//! Кэш аудио и локальный прокси для Flutter.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use moth_core::cache::proxy::{Proxy, Resolver};
use moth_core::cache::Cache;
use moth_core::model::{Quality, StreamInfo};
use moth_core::provider::Provider;

use super::yandex::{provider, run};

struct CacheState {
    cache: Arc<Cache>,
    proxy: Proxy,
}

static STATE: OnceLock<CacheState> = OnceLock::new();

/// Ссылки, полученные в `play_source`, чтобы прокси не запрашивал их повторно.
static PENDING: LazyLock<Mutex<HashMap<String, StreamInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const MB: u64 = 1024 * 1024;

pub struct PlaySourceDto {
    /// Путь к файлу кэша или адрес локального прокси.
    pub url: String,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    pub is_preview: bool,
    pub cached: bool,
}

pub struct CacheStatsDto {
    pub used_mb: f64,
    pub limit_mb: u32,
    pub tracks: u32,
}

/// Открыть кэш в `dir` и запустить прокси. `default_limit_mb` применяется,
/// если лимит ещё не сохранён.
pub async fn cache_init(dir: String, default_limit_mb: u32) -> Result<()> {
    run(async move {
        if STATE.get().is_some() {
            return Ok(());
        }
        let cache = Arc::new(Cache::open(Path::new(&dir), u64::from(default_limit_mb) * MB)?);
        let resolver: Resolver = Arc::new(|id: String| {
            Box::pin(async move {
                if let Some(info) = PENDING.lock().unwrap().remove(&id) {
                    return Ok(info);
                }
                let p = provider().map_err(|e| moth_core::Error::Unexpected(e.to_string()))?;
                p.stream(&id, Quality::High).await
            })
        });
        let http = reqwest_client()?;
        let proxy = Proxy::start(cache.clone(), resolver, http).await?;
        let _ = STATE.set(CacheState { cache, proxy });
        Ok(())
    })
    .await
}

fn reqwest_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("moth-amp/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Откуда играть трек: файл из кэша или поток через прокси (с записью в кэш).
pub async fn play_source(track_id: String) -> Result<PlaySourceDto> {
    run(async move {
        if let Some(state) = STATE.get() {
            if let Some((path, entry)) = state.cache.lookup(&track_id) {
                return Ok(PlaySourceDto {
                    url: path.to_string_lossy().into_owned(),
                    codec: entry.codec,
                    bitrate_kbps: entry.bitrate_kbps,
                    is_preview: false,
                    cached: true,
                });
            }
        }
        let info = provider()?.stream(&track_id, Quality::High).await?;
        let url = match STATE.get() {
            Some(state) => {
                let url = state.proxy.url(&track_id);
                PENDING
                    .lock()
                    .unwrap()
                    .insert(track_id.clone(), info.clone());
                url
            }
            // Кэш не открылся — играем напрямую, без записи.
            None => info.url.clone(),
        };
        Ok(PlaySourceDto {
            url,
            codec: info.codec,
            bitrate_kbps: info.bitrate_kbps,
            is_preview: info.is_preview,
            cached: false,
        })
    })
    .await
}

fn state() -> Result<&'static CacheState> {
    STATE.get().ok_or_else(|| anyhow!("кэш не инициализирован"))
}

pub fn cache_stats() -> Result<CacheStatsDto> {
    let s = state()?.cache.stats();
    Ok(CacheStatsDto {
        used_mb: s.used_bytes as f64 / MB as f64,
        limit_mb: u32::try_from(s.limit_bytes / MB).unwrap_or(u32::MAX),
        tracks: u32::try_from(s.tracks).unwrap_or(u32::MAX),
    })
}

pub fn cache_set_limit(limit_mb: u32) -> Result<()> {
    state()?.cache.set_limit(u64::from(limit_mb) * MB)?;
    Ok(())
}

pub fn cache_clear() -> Result<()> {
    state()?.cache.clear()?;
    Ok(())
}

/// Свежий статус подписки (после успешного запроса аккаунта).
pub(crate) fn confirm_plus(has_plus: bool) {
    if let Some(s) = STATE.get() {
        let _ = s.cache.confirm_plus(has_plus);
    }
}

/// Выход из аккаунта: кэш Яндекса удаляется.
pub(crate) fn wipe() {
    if let Some(s) = STATE.get() {
        let _ = s.cache.wipe_account();
    }
}
