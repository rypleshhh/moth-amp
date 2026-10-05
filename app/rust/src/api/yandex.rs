//! Функции ядра, доступные из Flutter.
//!
//! Все вызовы выполняются на собственном tokio-рантайме: reqwest и таймеры
//! ядра требуют tokio, а исполнитель flutter_rust_bridge им не является.

use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use anyhow::{anyhow, Result};
use moth_core::auth::{KeyringTokenStore, TokenStore};
use moth_core::model::{Account, Playlist, Quality, StreamInfo, Track};
use moth_core::provider::Provider;
use moth_core::yandex::{ApiClient, DeviceCode, YandexConfig, YandexProvider};

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("не удалось запустить tokio")
});

/// Текущая сессия Яндекса. Пересоздаётся при входе и выходе, чтобы сбросить
/// закэшированные данные аккаунта.
static PROVIDER: RwLock<Option<Arc<YandexProvider>>> = RwLock::new(None);

/// Код подтверждения между `start_login` и `finish_login`.
static PENDING_LOGIN: Mutex<Option<DeviceCode>> = Mutex::new(None);

pub(crate) fn provider() -> Result<Arc<YandexProvider>> {
    if let Some(p) = PROVIDER.read().unwrap().as_ref() {
        return Ok(p.clone());
    }
    let mut slot = PROVIDER.write().unwrap();
    if let Some(p) = slot.as_ref() {
        return Ok(p.clone());
    }
    let store: Arc<dyn TokenStore> = Arc::new(KeyringTokenStore::new("moth-amp", "yandex"));
    let p = Arc::new(YandexProvider::new(ApiClient::new(YandexConfig::default(), store)?));
    *slot = Some(p.clone());
    Ok(p)
}

fn reset_provider() {
    *PROVIDER.write().unwrap() = None;
}

pub(crate) async fn run<T, F>(fut: F) -> Result<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    RUNTIME
        .spawn(fut)
        .await
        .map_err(|e| anyhow!("внутренняя ошибка: {e}"))?
}

// ---- типы для Dart ----

pub struct DeviceCodeDto {
    pub user_code: String,
    pub verification_url: String,
    /// Сколько секунд действует код.
    pub expires_in: u32,
}

pub struct AccountDto {
    pub uid: String,
    pub name: String,
    pub has_plus: bool,
}

pub struct TrackDto {
    pub id: String,
    /// Название вместе с версией.
    pub title: String,
    /// Исполнители одной строкой для показа.
    pub artists: String,
    /// Исполнители по отдельности (для тегов файла).
    pub artist_names: Vec<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub duration_ms: Option<u32>,
    pub available: bool,
    pub cover_url: Option<String>,
    /// Откуда трек: `yandex` или `s3` (собственная библиотека).
    pub source: String,
}

pub struct PlaylistDto {
    pub id: String,
    pub title: String,
    pub track_count: Option<u32>,
}

pub struct StreamDto {
    pub url: String,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    pub is_preview: bool,
}

fn account_dto(a: &Account) -> AccountDto {
    AccountDto {
        uid: a.uid.clone(),
        name: a
            .display_name
            .clone()
            .or_else(|| a.login.clone())
            .unwrap_or_else(|| a.uid.clone()),
        has_plus: a.has_plus,
    }
}

fn track_dto(t: Track) -> TrackDto {
    TrackDto {
        title: t.full_title(),
        artists: t.artist_line(),
        artist_names: t.artists.iter().map(|a| a.name.clone()).collect(),
        id: t.key.id,
        year: t.album.as_ref().and_then(|a| a.year),
        album: t.album.map(|a| a.title),
        duration_ms: t.duration_ms.and_then(|ms| u32::try_from(ms).ok()),
        available: t.available,
        cover_url: t.cover_url,
        source: "yandex".into(),
    }
}

pub(crate) fn track_dtos(tracks: Vec<Track>) -> Vec<TrackDto> {
    tracks.into_iter().map(track_dto).collect()
}

fn playlist_dto(p: Playlist) -> PlaylistDto {
    PlaylistDto {
        id: p.key.id,
        title: p.title,
        track_count: p.track_count,
    }
}

fn stream_dto(s: StreamInfo) -> StreamDto {
    StreamDto {
        url: s.url,
        codec: s.codec,
        bitrate_kbps: s.bitrate_kbps,
        is_preview: s.is_preview,
    }
}

// ---- API ----

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

pub async fn is_logged_in() -> Result<bool> {
    run(async { Ok(provider()?.api().is_logged_in().await?) }).await
}

/// Шаг 1 входа: получить код, который пользователь вводит на сайте Яндекса.
pub async fn start_login() -> Result<DeviceCodeDto> {
    run(async {
        let code = provider()?.api().oauth().request_device_code().await?;
        let dto = DeviceCodeDto {
            user_code: code.user_code.clone(),
            verification_url: code.verification_url.clone(),
            expires_in: u32::try_from(code.expires_in).unwrap_or(u32::MAX),
        };
        *PENDING_LOGIN.lock().unwrap() = Some(code);
        Ok(dto)
    })
    .await
}

/// Шаг 2 входа: дождаться подтверждения и сохранить токен.
pub async fn finish_login() -> Result<AccountDto> {
    run(async {
        let code = PENDING_LOGIN
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow!("вход не начат"))?;
        let p = provider()?;
        let tokens = p.api().oauth().wait_for_token(&code).await?;
        p.api().set_tokens(tokens).await?;
        reset_provider();
        let p = provider()?;
        let acc = p.account().await?;
        super::cache::confirm_plus(acc.has_plus);
        Ok(account_dto(acc))
    })
    .await
}

pub async fn logout() -> Result<()> {
    run(async {
        provider()?.api().logout().await?;
        super::wave::reset().await;
        super::cache::wipe();
        reset_provider();
        Ok(())
    })
    .await
}

pub async fn account() -> Result<AccountDto> {
    run(async {
        let p = provider()?;
        let acc = p.account().await?;
        // Каждый успешный запрос аккаунта продлевает офлайн-льготу кэша.
        super::cache::confirm_plus(acc.has_plus);
        Ok(account_dto(acc))
    })
    .await
}

pub async fn liked_tracks() -> Result<Vec<TrackDto>> {
    run(async { Ok(track_dtos(provider()?.liked_tracks().await?)) }).await
}

pub async fn playlists() -> Result<Vec<PlaylistDto>> {
    run(async {
        Ok(provider()?
            .playlists()
            .await?
            .into_iter()
            .map(playlist_dto)
            .collect())
    })
    .await
}

pub async fn playlist_tracks(id: String) -> Result<Vec<TrackDto>> {
    run(async move { Ok(track_dtos(provider()?.playlist_tracks(&id).await?)) }).await
}

pub async fn search(query: String) -> Result<Vec<TrackDto>> {
    run(async move { Ok(track_dtos(provider()?.search_tracks(&query).await?)) }).await
}

pub async fn stream_url(track_id: String, low_quality: bool) -> Result<StreamDto> {
    let quality = if low_quality { Quality::Low } else { Quality::High };
    run(async move { Ok(stream_dto(provider()?.stream(&track_id, quality).await?)) }).await
}
