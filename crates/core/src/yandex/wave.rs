//! «Моя волна»: сессии ротора Яндекса.
//!
//! - `POST /rotor/session/new` — открыть сессию, первая партия треков;
//! - `POST /rotor/session/{id}/tracks` — следующая партия; в `queue` передаются уже
//!   сыгранные треки (`trackId:albumId`), иначе волна повторяется;
//! - `POST /rotor/session/{id}/feedback` — отчёты (`radioStarted`, `trackStarted`,
//!   `trackFinished`, `skip`), по ним волна подстраивается.
//!
//! Режимы: «тихий» (`incognito`, отчёты не отправляются) и «обучаемый» (с отчётами).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::json;

use crate::model::Track;
use crate::Result;

use super::api::ApiClient;
use super::dto::YTrack;

/// Сколько последних сыгранных треков передавать в `queue`.
const QUEUE_LIMIT: usize = 30;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionNew {
    pub radio_session_id: String,
    pub batch_id: String,
    #[serde(default)]
    pub sequence: Vec<SequenceItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionTracks {
    pub batch_id: String,
    #[serde(default)]
    pub sequence: Vec<SequenceItem>,
    #[serde(default)]
    pub unknown_session: bool,
}

#[derive(Deserialize)]
pub(crate) struct SequenceItem {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub track: Option<YTrack>,
}

/// Событие для отчёта.
#[derive(Debug, Clone, PartialEq)]
pub enum WaveEvent {
    RadioStarted,
    TrackStarted { track: String },
    TrackFinished { track: String, played_secs: f64 },
    Skip { track: String, played_secs: f64 },
}

impl WaveEvent {
    fn to_json(&self, timestamp: &str) -> serde_json::Value {
        match self {
            WaveEvent::RadioStarted => json!({ "type": "radioStarted", "timestamp": timestamp }),
            WaveEvent::TrackStarted { track } => {
                json!({ "type": "trackStarted", "timestamp": timestamp, "trackId": track })
            }
            WaveEvent::TrackFinished { track, played_secs } => json!({
                "type": "trackFinished", "timestamp": timestamp,
                "trackId": track, "totalPlayedSeconds": played_secs,
            }),
            WaveEvent::Skip { track, played_secs } => json!({
                "type": "skip", "timestamp": timestamp,
                "trackId": track, "totalPlayedSeconds": played_secs,
            }),
        }
    }
}

/// Состояние одной сессии волны.
pub struct Wave {
    session_id: String,
    learning: bool,
    /// id трека → (`trackId:albumId`, batchId партии, в которой он пришёл).
    known: HashMap<String, (String, String)>,
    /// Сыгранные треки в формате `trackId:albumId`, по порядку.
    played: Vec<String>,
    played_set: HashSet<String>,
}

impl Wave {
    /// Открыть сессию. `learning = false` — тихий режим: incognito и без отчётов.
    pub async fn start(api: &ApiClient, learning: bool) -> Result<(Self, Vec<Track>)> {
        let body = json!({
            "seeds": ["user:onyourwave"],
            "includeTracksInResponse": true,
            "includeWaveModel": false,
            "interactive": true,
            "incognito": !learning,
        });
        let s: SessionNew = api.post_json("/rotor/session/new", &body).await?;
        let mut wave = Self {
            session_id: s.radio_session_id,
            learning,
            known: HashMap::new(),
            played: Vec::new(),
            played_set: HashSet::new(),
        };
        let tracks = wave.accept(&s.batch_id, s.sequence);
        wave.report(api, WaveEvent::RadioStarted).await?;
        Ok((wave, tracks))
    }

    pub fn learning(&self) -> bool {
        self.learning
    }

    /// Следующая партия (без уже выданных треков).
    pub async fn more(&mut self, api: &ApiClient) -> Result<Vec<Track>> {
        let body = json!({ "queue": self.queue_tail() });
        let path = format!("/rotor/session/{}/tracks", self.session_id);
        let r: SessionTracks = api.post_json(&path, &body).await?;
        if r.unknown_session {
            return Err(crate::Error::Unexpected(
                "сессия волны истекла, запустите волну заново".into(),
            ));
        }
        Ok(self.accept(&r.batch_id, r.sequence))
    }

    /// Трек начал играть.
    pub async fn track_started(&mut self, api: &ApiClient, track_id: &str) -> Result<()> {
        let Some((full, _)) = self.known.get(track_id).cloned() else {
            return Ok(());
        };
        if self.played_set.insert(full.clone()) {
            self.played.push(full.clone());
        }
        self.report_for(api, track_id, WaveEvent::TrackStarted { track: full })
            .await
    }

    /// Трек закончился: дослушан (`skipped = false`) или пропущен.
    pub async fn track_ended(
        &mut self,
        api: &ApiClient,
        track_id: &str,
        played_secs: f64,
        skipped: bool,
    ) -> Result<()> {
        let Some((full, _)) = self.known.get(track_id).cloned() else {
            return Ok(());
        };
        let played_secs = played_secs.max(0.0);
        let event = if skipped {
            WaveEvent::Skip { track: full, played_secs }
        } else {
            WaveEvent::TrackFinished { track: full, played_secs }
        };
        self.report_for(api, track_id, event).await
    }

    /// Принять партию: запомнить id и batchId, отбросить уже выданные треки.
    fn accept(&mut self, batch_id: &str, sequence: Vec<SequenceItem>) -> Vec<Track> {
        let mut out = Vec::new();
        for item in sequence {
            if item.kind.as_deref().is_some_and(|k| k != "track") {
                continue;
            }
            let Some(yt) = item.track else { continue };
            let track = Track::from(yt);
            if self.known.contains_key(&track.key.id) {
                continue;
            }
            let full = full_id(&track);
            self.known
                .insert(track.key.id.clone(), (full, batch_id.to_owned()));
            out.push(track);
        }
        out
    }

    fn queue_tail(&self) -> Vec<String> {
        let start = self.played.len().saturating_sub(QUEUE_LIMIT);
        self.played[start..].to_vec()
    }

    async fn report_for(&self, api: &ApiClient, track_id: &str, event: WaveEvent) -> Result<()> {
        let batch = self
            .known
            .get(track_id)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        self.send(api, &batch, event).await
    }

    async fn report(&self, api: &ApiClient, event: WaveEvent) -> Result<()> {
        let batch = self
            .known
            .values()
            .next()
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        self.send(api, &batch, event).await
    }

    async fn send(&self, api: &ApiClient, batch_id: &str, event: WaveEvent) -> Result<()> {
        // Тихий режим: на сервер ничего не уходит.
        if !self.learning {
            return Ok(());
        }
        let body = json!({ "event": event.to_json(&iso8601_now()), "batchId": batch_id });
        let path = format!("/rotor/session/{}/feedback", self.session_id);
        let _: serde_json::Value = api.post_json(&path, &body).await?;
        Ok(())
    }
}

fn full_id(t: &Track) -> String {
    match t.album.as_ref().and_then(|a| a.id.as_deref()) {
        Some(album) => format!("{}:{album}", t.key.id),
        None => t.key.id.clone(),
    }
}

fn iso8601_now() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    iso8601_utc(d.as_secs(), d.subsec_millis())
}

/// `2026-10-05T19:00:00.123Z` без внешних зависимостей (алгоритм civil-from-days).
fn iso8601_utc(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, album: u64) -> SequenceItem {
        let json = format!(
            r#"{{"type":"track","track":{{"id":"{id}","title":"T{id}","albums":[{{"id":{album}}}]}}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    fn wave(learning: bool) -> Wave {
        Wave {
            session_id: "s".into(),
            learning,
            known: HashMap::new(),
            played: Vec::new(),
            played_set: HashSet::new(),
        }
    }

    #[test]
    fn accept_dedupes_and_remembers_batch() {
        let mut w = wave(false);
        let first = w.accept("b1", vec![item(1, 10), item(2, 20)]);
        assert_eq!(first.len(), 2);
        let second = w.accept("b2", vec![item(2, 20), item(3, 30)]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].key.id, "3");
        assert_eq!(w.known["1"], ("1:10".to_owned(), "b1".to_owned()));
        assert_eq!(w.known["3"].1, "b2");
    }

    #[test]
    fn non_track_items_skipped() {
        let mut w = wave(false);
        let ad: SequenceItem = serde_json::from_str(r#"{"type":"ad"}"#).unwrap();
        assert!(w.accept("b", vec![ad]).is_empty());
    }

    #[test]
    fn queue_tail_is_limited() {
        let mut w = wave(false);
        for i in 0..40 {
            w.played.push(format!("{i}:1"));
        }
        let q = w.queue_tail();
        assert_eq!(q.len(), QUEUE_LIMIT);
        assert_eq!(q[0], "10:1");
        assert_eq!(q.last().unwrap(), "39:1");
    }

    #[test]
    fn session_response_parsing() {
        let json = r#"{"radioSessionId":"r1","batchId":"b1","pumpkin":false,"terminated":false,
            "sequence":[{"type":"track","liked":false,"track":{"id":"5","title":"A","albums":[{"id":7}]},
            "trackParameters":{"bpm":120}}]}"#;
        let s: SessionNew = serde_json::from_str(json).unwrap();
        assert_eq!(s.radio_session_id, "r1");
        assert_eq!(s.sequence.len(), 1);
    }

    #[test]
    fn event_json() {
        let e = WaveEvent::Skip { track: "1:2".into(), played_secs: 3.5 };
        let v = e.to_json("2026-01-01T00:00:00.000Z");
        assert_eq!(v["type"], "skip");
        assert_eq!(v["trackId"], "1:2");
        assert_eq!(v["totalPlayedSeconds"], 3.5);
        assert_eq!(WaveEvent::RadioStarted.to_json("t")["type"], "radioStarted");
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601_utc(0, 0), "1970-01-01T00:00:00.000Z");
        // 2000-02-29 (високосный) 12:34:56
        assert_eq!(iso8601_utc(951_827_696, 7), "2000-02-29T12:34:56.007Z");
        assert_eq!(iso8601_utc(1_791_226_800, 0), "2026-10-05T19:00:00.000Z");
    }
}
