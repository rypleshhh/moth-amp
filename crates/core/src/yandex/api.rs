//! HTTP-клиент API Яндекс Музыки.
//!
//! Отправляется только необходимое: токен и параметры запроса. Заголовок
//! `X-Yandex-Music-Client` по умолчанию не передаётся; если на этапе
//! исследования выяснится, что он обязателен, его можно задать в конфиге.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::AUTHORIZATION;
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::auth::{TokenSet, TokenStore};
use crate::model::{Account, Playlist, Quality, StreamInfo, Track};
use crate::{Error, Result};

use super::download::{build_direct_url, parse_download_xml, pick_variant};
use super::dto::{
    AccountStatus, DownloadInfo, Envelope, LikesResult, SearchResult, TrackShort, YPlaylist,
    YTrack,
};
use super::oauth::{OAuthClient, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET};

pub const API_BASE: &str = "https://api.music.yandex.net";

/// Обновлять токен, если до истечения осталось меньше суток.
const REFRESH_MARGIN_SECS: u64 = 24 * 60 * 60;

/// Сколько треков запрашивать за один вызов `/tracks`.
const TRACKS_BATCH: usize = 200;

#[derive(Debug, Clone)]
pub struct YandexConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Значение `X-Yandex-Music-Client`; `None` — заголовок не отправляется.
    pub client_header: Option<String>,
    /// Нейтральный User-Agent без сведений об устройстве.
    pub user_agent: String,
}

impl Default for YandexConfig {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_owned(),
            client_secret: DEFAULT_CLIENT_SECRET.to_owned(),
            client_header: None,
            user_agent: concat!("moth-amp/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    oauth: OAuthClient,
    store: Arc<dyn TokenStore>,
    token: Mutex<Option<TokenSet>>,
    client_header: Option<String>,
}

impl ApiClient {
    pub fn new(config: YandexConfig, store: Arc<dyn TokenStore>) -> Result<Self> {
        // Короткий таймаут соединения: при сбое сети (или нерабочем IPv6)
        // ошибка приходит за секунды, а не через ~20 с системного таймаута.
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent)
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        let oauth = OAuthClient::new(http.clone(), config.client_id, config.client_secret);
        Ok(Self {
            http,
            base: API_BASE.to_owned(),
            oauth,
            store,
            token: Mutex::new(None),
            client_header: config.client_header,
        })
    }

    pub fn oauth(&self) -> &OAuthClient {
        &self.oauth
    }

    /// Сохраняет токены после входа.
    pub async fn set_tokens(&self, tokens: TokenSet) -> Result<()> {
        self.store.save(&tokens)?;
        *self.token.lock().await = Some(tokens);
        Ok(())
    }

    /// Выход: токен удаляется из хранилища и из памяти.
    pub async fn logout(&self) -> Result<()> {
        self.store.clear()?;
        *self.token.lock().await = None;
        Ok(())
    }

    pub async fn is_logged_in(&self) -> Result<bool> {
        let mut guard = self.token.lock().await;
        if guard.is_none() {
            *guard = self.store.load()?;
        }
        Ok(guard.is_some())
    }

    async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if guard.is_none() {
            *guard = self.store.load()?;
        }
        let current = guard.as_ref().ok_or(Error::Unauthorized)?;

        if current.expires_within(REFRESH_MARGIN_SECS) {
            if let Some(refresh) = current.refresh_token.clone() {
                let fresh = self.oauth.refresh(&refresh).await?;
                self.store.save(&fresh)?;
                *guard = Some(fresh);
            }
        }
        Ok(guard.as_ref().map(|t| t.access_token.clone()).unwrap_or_default())
    }

    async fn authorized(&self, req: RequestBuilder) -> Result<RequestBuilder> {
        let token = self.access_token().await?;
        let mut req = req.header(AUTHORIZATION, format!("OAuth {token}"));
        if let Some(client) = &self.client_header {
            req = req.header("X-Yandex-Music-Client", client);
        }
        Ok(req)
    }

    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = self.authorized(req).await?.send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        parse_envelope(status, &body)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        self.send(self.http.get(format!("{}{path}", self.base)).query(query))
            .await
    }

    async fn post_form<T: DeserializeOwned>(&self, path: &str, form: &[(&str, &str)]) -> Result<T> {
        self.send(self.http.post(format!("{}{path}", self.base)).form(form))
            .await
    }

    // ---- методы API ----

    pub async fn account_status(&self) -> Result<Account> {
        let st: AccountStatus = self.get("/account/status", &[]).await?;
        st.into_account().ok_or(Error::Unauthorized)
    }

    /// Идентификаторы лайкнутых треков в формате `trackId:albumId`.
    pub async fn liked_track_ids(&self, uid: &str) -> Result<Vec<String>> {
        let r: LikesResult = self
            .get(&format!("/users/{uid}/likes/tracks"), &[])
            .await?;
        Ok(r.library.tracks.iter().map(TrackShort::full_id).collect())
    }

    /// Полные данные треков по списку id (пачками).
    pub async fn tracks(&self, ids: &[String]) -> Result<Vec<Track>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(TRACKS_BATCH) {
            let joined = chunk.join(",");
            let batch: Vec<YTrack> = self
                .post_form("/tracks", &[("track-ids", joined.as_str()), ("with-positions", "false")])
                .await?;
            out.extend(batch.into_iter().map(Track::from));
        }
        Ok(out)
    }

    pub async fn playlists(&self, uid: &str) -> Result<Vec<Playlist>> {
        let list: Vec<YPlaylist> = self
            .get(&format!("/users/{uid}/playlists/list"), &[])
            .await?;
        Ok(list.iter().map(YPlaylist::summary).collect())
    }

    pub async fn playlist_tracks(&self, uid: &str, kind: &str) -> Result<Vec<Track>> {
        let pl: YPlaylist = self
            .get(&format!("/users/{uid}/playlists/{kind}"), &[])
            .await?;

        // Обычно треки приходят целиком; если нет — догружаем по id.
        let mut full = Vec::new();
        let mut missing = Vec::new();
        for item in pl.tracks {
            match (item.track, item.id) {
                (Some(t), _) => full.push(Track::from(t)),
                (None, Some(id)) => missing.push(match item.album_id {
                    Some(album) => format!("{id}:{album}"),
                    None => id,
                }),
                (None, None) => {}
            }
        }
        if !missing.is_empty() {
            full.extend(self.tracks(&missing).await?);
        }
        Ok(full)
    }

    pub async fn search_tracks(&self, text: &str) -> Result<Vec<Track>> {
        let r: SearchResult = self
            .get(
                "/search",
                &[("text", text), ("type", "track"), ("page", "0"), ("nocorrect", "false")],
            )
            .await?;
        Ok(r
            .tracks
            .map(|b| b.results.into_iter().map(Track::from).collect())
            .unwrap_or_default())
    }

    /// Прямая ссылка на поток трека.
    pub async fn stream(&self, track_id: &str, quality: Quality) -> Result<StreamInfo> {
        let variants: Vec<DownloadInfo> = self
            .get(&format!("/tracks/{track_id}/download-info"), &[])
            .await?;
        let chosen = pick_variant(&variants, quality)
            .ok_or_else(|| Error::NoStream(track_id.to_owned()))?;

        let resp = self
            .authorized(self.http.get(&chosen.download_info_url))
            .await?
            .send()
            .await?
            .error_for_status()?;
        let xml = resp.text().await?;
        let url = build_direct_url(&parse_download_xml(&xml)?);

        Ok(StreamInfo {
            url,
            codec: chosen.codec.clone(),
            bitrate_kbps: chosen.bitrate_in_kbps,
            is_preview: chosen.preview,
        })
    }
}

fn parse_envelope<T: DeserializeOwned>(status: StatusCode, body: &str) -> Result<T> {
    if status == StatusCode::UNAUTHORIZED {
        return Err(Error::Unauthorized);
    }
    // Сначала читаем в serde_json::Value: Яндекс иногда отдаёт объекты с
    // повторяющимся ключом (например, `albums` у треков в поиске). Строгий
    // derive-разбор на этом падает, а Value просто берёт последнее значение.
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) if !status.is_success() => {
            return Err(Error::Unexpected(format!("HTTP {status}")));
        }
        Err(e) => return Err(e.into()),
    };
    let env: Envelope<T> = serde_json::from_value(value)?;
    if let Some(e) = env.error {
        return Err(Error::Api {
            name: e.name.unwrap_or_else(|| status.to_string()),
            message: e.message.unwrap_or_default(),
        });
    }
    env.result
        .ok_or_else(|| Error::Unexpected(format!("нет поля result (HTTP {status})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_ok() {
        let v: Vec<u32> = parse_envelope(StatusCode::OK, r#"{"invocationInfo":{},"result":[1,2]}"#).unwrap();
        assert_eq!(v, [1, 2]);
    }

    #[test]
    fn envelope_tolerates_duplicate_keys() {
        // Так выглядят треки в ответе /search: ключ albums встречается дважды.
        let body = r#"{"result":[{"id":1,"title":"T","albums":[{"id":1}],"durationMs":1000,"albums":[{"id":2,"title":"A"}]}]}"#;
        let v: Vec<super::YTrack> = parse_envelope(StatusCode::OK, body).unwrap();
        let t = Track::from(v.into_iter().next().unwrap());
        assert_eq!(t.album.unwrap().id.as_deref(), Some("2"));
    }

    #[test]
    fn envelope_api_error() {
        let err = parse_envelope::<Vec<u32>>(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"name":"validate","message":"bad id"}}"#,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Api { ref name, .. } if name == "validate"));
    }

    #[test]
    fn envelope_unauthorized() {
        let err = parse_envelope::<Vec<u32>>(StatusCode::UNAUTHORIZED, "").unwrap_err();
        assert!(matches!(err, Error::Unauthorized));
    }

    #[test]
    fn envelope_non_json_error() {
        let err = parse_envelope::<Vec<u32>>(StatusCode::BAD_GATEWAY, "<html>").unwrap_err();
        assert!(matches!(err, Error::Unexpected(_)));
    }

    #[tokio::test]
    async fn no_token_means_unauthorized() {
        let store = Arc::new(crate::auth::MemoryTokenStore::default());
        let api = ApiClient::new(YandexConfig::default(), store).unwrap();
        assert!(!api.is_logged_in().await.unwrap());
        assert!(matches!(api.access_token().await, Err(Error::Unauthorized)));
    }
}
