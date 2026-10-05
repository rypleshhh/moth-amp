//! Структуры ответов API Яндекс Музыки и их преобразование в общие модели.
//!
//! API непоследовательно отдаёт идентификаторы то числами, то строками,
//! поэтому все id читаются через [`de_id`] / [`de_opt_id`].

use serde::{Deserialize, Deserializer};

use crate::model::{Account, AlbumRef, Artist, Playlist, PlaylistKey, Source, Track, TrackKey};

#[derive(Deserialize)]
#[serde(untagged)]
enum IdRepr {
    Str(String),
    Num(u64),
}

impl From<IdRepr> for String {
    fn from(v: IdRepr) -> Self {
        match v {
            IdRepr::Str(s) => s,
            IdRepr::Num(n) => n.to_string(),
        }
    }
}

pub(crate) fn de_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    IdRepr::deserialize(d).map(String::from)
}

pub(crate) fn de_opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<IdRepr>::deserialize(d).map(|o| o.map(String::from))
}

/// Обёртка всех ответов: `{"invocationInfo": …, "result": …}` или `{"error": …}`.
#[derive(Deserialize)]
pub(crate) struct Envelope<T> {
    pub result: Option<T>,
    pub error: Option<ApiErrorBody>,
}

#[derive(Deserialize)]
pub(crate) struct ApiErrorBody {
    pub name: Option<String>,
    pub message: Option<String>,
}

// ---- аккаунт ----

#[derive(Deserialize)]
pub(crate) struct AccountStatus {
    pub account: AccountInfo,
    pub plus: Option<PlusInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountInfo {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub uid: Option<String>,
    pub login: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlusInfo {
    #[serde(default)]
    pub has_plus: bool,
}

impl AccountStatus {
    pub fn into_account(self) -> Option<Account> {
        Some(Account {
            uid: self.account.uid?,
            login: self.account.login,
            display_name: self.account.display_name,
            has_plus: self.plus.is_some_and(|p| p.has_plus),
        })
    }
}

// ---- треки ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YTrack {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    pub title: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub artists: Vec<YArtist>,
    #[serde(default)]
    pub albums: Vec<YAlbum>,
    pub duration_ms: Option<u64>,
    pub available: Option<bool>,
    pub cover_uri: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct YArtist {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct YAlbum {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    pub title: Option<String>,
    pub year: Option<u32>,
}

/// `avatars.yandex.net/get-music-content/…/%%` → `https://…/400x400`.
pub(crate) fn cover_url(uri: &str, size: &str) -> String {
    format!("https://{}", uri.replace("%%", size))
}

impl From<YTrack> for Track {
    fn from(t: YTrack) -> Self {
        let album = t.albums.into_iter().next().map(|a| AlbumRef {
            id: a.id,
            title: a.title.unwrap_or_default(),
            year: a.year,
        });
        Track {
            key: TrackKey {
                source: Source::Yandex,
                id: t.id,
            },
            title: t.title.unwrap_or_default(),
            version: t.version,
            artists: t
                .artists
                .into_iter()
                .filter_map(|a| {
                    Some(Artist {
                        id: a.id,
                        name: a.name?,
                    })
                })
                .collect(),
            album,
            duration_ms: t.duration_ms,
            available: t.available.unwrap_or(true),
            cover_url: t.cover_uri.as_deref().map(|u| cover_url(u, "400x400")),
            isrc: None,
        }
    }
}

// ---- лайки ----

#[derive(Deserialize)]
pub(crate) struct LikesResult {
    pub library: LikesLibrary,
}

#[derive(Deserialize)]
pub(crate) struct LikesLibrary {
    #[serde(default)]
    pub tracks: Vec<TrackShort>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackShort {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub album_id: Option<String>,
}

impl TrackShort {
    /// Формат, который принимает `/tracks`: `trackId:albumId`.
    pub fn full_id(&self) -> String {
        match &self.album_id {
            Some(album) => format!("{}:{album}", self.id),
            None => self.id.clone(),
        }
    }
}

// ---- плейлисты ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YPlaylist {
    #[serde(deserialize_with = "de_id")]
    pub kind: String,
    pub title: Option<String>,
    pub track_count: Option<u32>,
    #[serde(default)]
    pub tracks: Vec<YPlaylistItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YPlaylistItem {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub album_id: Option<String>,
    pub track: Option<YTrack>,
}

impl YPlaylist {
    pub fn summary(&self) -> Playlist {
        Playlist {
            key: PlaylistKey {
                source: Source::Yandex,
                id: self.kind.clone(),
            },
            title: self.title.clone().unwrap_or_default(),
            track_count: self.track_count,
        }
    }
}

// ---- поиск ----

#[derive(Deserialize)]
pub(crate) struct SearchResult {
    pub tracks: Option<SearchBlock>,
}

#[derive(Deserialize)]
pub(crate) struct SearchBlock {
    #[serde(default)]
    pub results: Vec<YTrack>,
}

// ---- загрузка ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadInfo {
    pub codec: String,
    pub bitrate_in_kbps: Option<u32>,
    pub download_info_url: String,
    #[serde(default)]
    pub preview: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_status_numeric_uid() {
        let json = r#"{"result":{"account":{"uid":12345,"login":"user","displayName":"User"},"plus":{"hasPlus":true}}}"#;
        let env: Envelope<AccountStatus> = serde_json::from_str(json).unwrap();
        let acc = env.result.unwrap().into_account().unwrap();
        assert_eq!(acc.uid, "12345");
        assert!(acc.has_plus);
    }

    #[test]
    fn account_without_plus_block() {
        let json = r#"{"account":{"uid":"1"}}"#;
        let st: AccountStatus = serde_json::from_str(json).unwrap();
        assert!(!st.into_account().unwrap().has_plus);
    }

    #[test]
    fn track_conversion() {
        let json = r#"{
            "id": "42", "title": "Song", "version": "Live",
            "artists": [{"id": 7, "name": "Band"}, {"various": true}],
            "albums": [{"id": 99, "title": "Album", "year": 2001}],
            "durationMs": 180000, "available": false,
            "coverUri": "avatars.yandex.net/get-music-content/1/2/%%"
        }"#;
        let t: Track = serde_json::from_str::<YTrack>(json).unwrap().into();
        assert_eq!(t.key.id, "42");
        assert_eq!(t.full_title(), "Song (Live)");
        assert_eq!(t.artist_line(), "Band");
        assert_eq!(t.album.as_ref().unwrap().id.as_deref(), Some("99"));
        assert!(!t.available);
        assert_eq!(
            t.cover_url.as_deref(),
            Some("https://avatars.yandex.net/get-music-content/1/2/400x400")
        );
    }

    #[test]
    fn likes_full_ids() {
        let json = r#"{"library":{"uid":1,"revision":5,"tracks":[
            {"id":"10","albumId":"20","timestamp":"2024-01-01T00:00:00+00:00"},
            {"id":11,"timestamp":"2024-01-01T00:00:00+00:00"}
        ]}}"#;
        let r: LikesResult = serde_json::from_str(json).unwrap();
        let ids: Vec<_> = r.library.tracks.iter().map(TrackShort::full_id).collect();
        assert_eq!(ids, ["10:20", "11"]);
    }

    #[test]
    fn download_info_list() {
        let json = r#"[
            {"codec":"mp3","bitrateInKbps":320,"downloadInfoUrl":"https://x/1","direct":false,"preview":false,"gain":false},
            {"codec":"aac","bitrateInKbps":64,"downloadInfoUrl":"https://x/2"}
        ]"#;
        let v: Vec<DownloadInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(v[0].bitrate_in_kbps, Some(320));
        assert!(!v[1].preview);
    }
}
