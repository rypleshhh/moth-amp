//! Кэш аудио и локальный прокси для Flutter.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use moth_core::cache::proxy::{fetch_cover, CommitHook, Proxy, ResolvedTrack, Resolver};
use moth_core::cache::Cache;
use moth_core::model::{Quality, StreamInfo, TrackMeta};
use moth_core::provider::Provider;

use super::yandex::{provider, run, TrackDto};

struct CacheState {
    cache: Arc<Cache>,
    proxy: Proxy,
}

static STATE: OnceLock<CacheState> = OnceLock::new();

/// Ссылки, полученные в `play_source`, чтобы прокси не запрашивал их повторно.
static PENDING: LazyLock<Mutex<HashMap<String, StreamInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Метаданные треков для тегов файлов кэша.
static META: LazyLock<Mutex<HashMap<String, TrackMeta>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const MB: u64 = 1024 * 1024;

/// Загрузки в бакет по одной: индекс кэша в S3 правится чтением-записью.
static MIRROR_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Скопировать трек из локального кэша в S3 (если S3 подключено).
/// Свои треки из библиотеки S3 не копируются — они и так там.
async fn mirror_one(id: &str) -> Result<bool> {
    let Some(lib) = super::s3::library()? else {
        return Ok(false);
    };
    let Some(state) = STATE.get() else {
        return Ok(false);
    };
    let Some((path, entry)) = state.cache.entry(id) else {
        return Ok(false);
    };
    if entry.meta.as_ref().is_some_and(|m| m.source == "s3") {
        return Ok(false);
    }
    let _guard = MIRROR_LOCK.lock().await;
    lib.cache_put(id, &path, &entry.codec, entry.bitrate_kbps, entry.meta)
        .await?;
    Ok(true)
}

/// Трек Яндекса из кэша в S3, если он там есть и подписка подтверждена.
async fn s3_cached_stream(id: &str) -> Option<(StreamInfo, Option<TrackMeta>)> {
    let state = STATE.get()?;
    let lib = super::s3::library().ok()??;
    let obj = lib.cache_get(id).await.ok()??;
    if obj.needs_plus() && !state.cache.playback_allowed() {
        return None;
    }
    Some((lib.cache_stream_info(&obj), obj.meta))
}

pub struct PlaySourceDto {
    /// Путь к файлу кэша, адрес локального прокси или прямая ссылка.
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
    /// Сохранять все прослушанные треки.
    pub auto_cache: bool,
    /// Папка с файлами треков.
    pub folder: String,
}

fn meta_from(t: &TrackDto) -> TrackMeta {
    TrackMeta {
        source: t.source.clone(),
        id: t.id.clone(),
        title: t.title.clone(),
        artists: t.artist_names.clone(),
        album: t.album.clone(),
        year: t.year,
        cover_url: t.cover_url.clone(),
        duration_ms: t.duration_ms.map(u64::from),
    }
}

fn remember_meta(t: &TrackDto) {
    META.lock().unwrap().insert(t.id.clone(), meta_from(t));
}

/// Открыть кэш в `dir` и запустить прокси. `default_limit_mb` применяется,
/// если лимит ещё не сохранён.
pub async fn cache_init(dir: String, default_limit_mb: u32) -> Result<()> {
    run(async move {
        if STATE.get().is_some() {
            return Ok(());
        }
        let cache = Arc::new(Cache::open(
            Path::new(&dir),
            u64::from(default_limit_mb) * MB,
        )?);
        let resolver: Resolver = Arc::new(|id: String| {
            Box::pin(async move {
                let mut meta = META.lock().unwrap().get(&id).cloned();
                let pending = PENDING.lock().unwrap().remove(&id);
                let stream = match pending {
                    Some(info) => info,
                    None if meta.as_ref().is_some_and(|m| m.source == "s3") => {
                        let (info, fresh) = super::s3::resolve(&id)
                            .await
                            .map_err(|e| moth_core::Error::Unexpected(e.to_string()))?;
                        meta = Some(fresh);
                        info
                    }
                    None => match s3_cached_stream(&id).await {
                        Some((info, s3_meta)) => {
                            meta = meta.or(s3_meta);
                            info
                        }
                        None => {
                            let p = provider()
                                .map_err(|e| moth_core::Error::Unexpected(e.to_string()))?;
                            p.stream(&id, Quality::High).await?
                        }
                    },
                };
                Ok(ResolvedTrack { stream, meta })
            })
        });
        let http = reqwest::Client::builder()
            .user_agent(concat!("moth-amp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let proxy = Proxy::start(cache.clone(), resolver, http).await?;
        let hook: CommitHook = Arc::new(|id: String| {
            tokio::spawn(async move {
                let _ = mirror_one(&id).await;
            });
        });
        proxy.set_on_commit(hook);
        let _ = STATE.set(CacheState { cache, proxy });
        Ok(())
    })
    .await
}

/// Откуда играть трек: файл из кэша, поток через прокси (с записью в кэш)
/// или напрямую, если автосохранение выключено.
pub async fn play_source(track: TrackDto) -> Result<PlaySourceDto> {
    run(async move {
        let state = STATE.get();
        if let Some(state) = state {
            if let Some((path, entry)) = state.cache.lookup(&track.id) {
                return Ok(PlaySourceDto {
                    url: path.to_string_lossy().into_owned(),
                    codec: entry.codec,
                    bitrate_kbps: entry.bitrate_kbps,
                    is_preview: false,
                    cached: true,
                });
            }
        }
        let info = if track.source == "s3" {
            let (info, meta) = super::s3::resolve(&track.id).await?;
            META.lock().unwrap().insert(track.id.clone(), meta);
            info
        } else {
            remember_meta(&track);
            match s3_cached_stream(&track.id).await {
                Some((info, _)) => info,
                None => provider()?.stream(&track.id, Quality::High).await?,
            }
        };
        let url = match state {
            Some(state) if state.cache.auto_cache() => {
                PENDING
                    .lock()
                    .unwrap()
                    .insert(track.id.clone(), info.clone());
                state.proxy.url(&track.id)
            }
            // Автосохранение выключено (или кэш не открылся) — напрямую, без записи.
            _ => info.url.clone(),
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

/// Скачать трек в кэш вручную. Завершается, когда файл сохранён.
pub async fn cache_download(track: TrackDto) -> Result<()> {
    run(async move {
        let state = state()?;
        remember_meta(&track);
        state.proxy.download(&track.id).await?;
        Ok(())
    })
    .await
}

fn state() -> Result<&'static CacheState> {
    STATE.get().ok_or_else(|| anyhow!("кэш не инициализирован"))
}

/// id всех треков в кэше (для отметок в списках).
pub fn cached_ids() -> Result<Vec<String>> {
    Ok(state()?.cache.cached_ids())
}

pub fn cache_stats() -> Result<CacheStatsDto> {
    let cache = &state()?.cache;
    let s = cache.stats();
    Ok(CacheStatsDto {
        used_mb: s.used_bytes as f64 / MB as f64,
        limit_mb: u32::try_from(s.limit_bytes / MB).unwrap_or(u32::MAX),
        tracks: u32::try_from(s.tracks).unwrap_or(u32::MAX),
        auto_cache: cache.auto_cache(),
        folder: cache.folder().to_string_lossy().into_owned(),
    })
}

pub fn cache_set_limit(limit_mb: u32) -> Result<()> {
    state()?.cache.set_limit(u64::from(limit_mb) * MB)?;
    Ok(())
}

pub fn cache_set_auto(enabled: bool) -> Result<()> {
    state()?.cache.set_auto_cache(enabled)?;
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
pub(crate) async fn wipe() {
    if let Some(s) = STATE.get() {
        let _ = s.cache.wipe_account();
    }
    if let Ok(Some(lib)) = super::s3::library() {
        let _guard = MIRROR_LOCK.lock().await;
        let _ = lib.cache_remove_yandex().await;
    }
}

/// Скопировать в S3 треки локального кэша, которых там ещё нет.
/// Возвращает число скопированных. Без подключённого S3 — 0.
pub async fn cache_mirror_to_s3() -> Result<u32> {
    run(async {
        let state = state()?;
        let Some(lib) = super::s3::library()? else {
            return Ok(0);
        };
        let remote: Vec<String> = lib.cache_list().await?.into_iter().map(|o| o.id).collect();
        let mut done = 0;
        for id in state.cache.cached_ids() {
            if !remote.contains(&id) && mirror_one(&id).await? {
                done += 1;
            }
        }
        Ok(done)
    })
    .await
}

/// Дописать метаданные и теги трекам, попавшим в кэш без них.
/// Возвращает, скольким трекам дописано. Нужна сеть.
pub async fn cache_backfill_meta() -> Result<u32> {
    run(async {
        let state = state()?;
        let ids = state.cache.missing_meta();
        if ids.is_empty() {
            return Ok(0);
        }
        let tracks = provider()?.api().tracks(&ids).await?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("moth-amp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut done = 0;
        for t in tracks {
            let meta = TrackMeta::from_track(&t);
            let cover = fetch_cover(&http, &meta).await;
            state.cache.attach_meta(&t.key.id, meta, cover.as_deref())?;
            done += 1;
        }
        Ok(done)
    })
    .await
}

/// Треки в кэше с метаданными — список «Скачанное», работает без сети.
/// Сначала недавно игравшие.
pub async fn cached_tracks() -> Result<Vec<TrackDto>> {
    run(async {
        let state = state()?;
        let mut list: Vec<TrackDto> = state
            .cache
            .cached_tracks()
            .into_iter()
            .map(|(id, entry)| track_from_meta(id, entry.meta))
            .collect();
        // Треки из кэша в S3, которых нет на этом устройстве.
        if let Ok(Some(lib)) = super::s3::library() {
            if let Ok(remote) = lib.cache_list().await {
                let plus_ok = state.cache.playback_allowed();
                for obj in remote {
                    if (plus_ok || !obj.needs_plus()) && !list.iter().any(|t| t.id == obj.id) {
                        list.push(track_from_meta(obj.id, obj.meta));
                    }
                }
            }
        }
        Ok(list)
    })
    .await
}

fn track_from_meta(id: String, meta: Option<TrackMeta>) -> TrackDto {
    let meta = meta.unwrap_or_default();
    TrackDto {
        source: if meta.source.is_empty() {
            "yandex".into()
        } else {
            meta.source.clone()
        },
        title: if meta.title.is_empty() {
            id.clone()
        } else {
            meta.title
        },
        artists: meta.artists.join(", "),
        artist_names: meta.artists,
        album: meta.album,
        year: meta.year,
        duration_ms: meta.duration_ms.and_then(|ms| u32::try_from(ms).ok()),
        available: true,
        cover_url: meta.cover_url,
        id,
    }
}
