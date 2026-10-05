//! Кэш аудио: обычные mp3/flac-файлы в папке приложения, LRU по объёму,
//! привязка к подписке.
//!
//! Файлы: `<dir>/tracks/<id>.<ext>`, недокачанные — `<dir>/tracks/<id>.part`,
//! индекс — `<dir>/index.json`.
//!
//! Кэш Яндекса играет, только если Плюс был подтверждён не раньше
//! [`OFFLINE_GRACE_SECS`] назад; при выходе из аккаунта кэш удаляется.

pub mod proxy;
pub mod tags;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::auth::now_unix;
use crate::fsutil::write_atomic;
use crate::model::TrackMeta;
use crate::Result;

/// Сколько кэш играет без подтверждения подписки (офлайн-льгота).
pub const OFFLINE_GRACE_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub file: String,
    pub size: u64,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    /// Unix-время последнего воспроизведения (для LRU).
    pub last_access: u64,
    /// Метаданные (они же вшиты в теги файла).
    #[serde(default)]
    pub meta: Option<TrackMeta>,
    /// Скачан вручную (иконкой загрузки), а не просто прослушан.
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct PlusMark {
    has_plus: bool,
    confirmed_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    entries: HashMap<String, Entry>,
    limit_bytes: u64,
    plus: Option<PlusMark>,
    /// Сохранять в кэш все прослушанные треки (иначе только скачанные вручную).
    #[serde(default = "default_true")]
    auto_cache: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Index {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            limit_bytes: 0,
            plus: None,
            auto_cache: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub tracks: usize,
}

pub struct Cache {
    dir: PathBuf,
    index: Mutex<Index>,
}

/// Трек Яндекса (или старая запись без метаданных) — играет только при
/// подтверждённом Плюсе. Собственные треки пользователя от подписки не зависят.
fn needs_plus(entry: &Entry) -> bool {
    entry.meta.as_ref().is_none_or(|m| m.source == "yandex")
}

/// Расширение файла по кодеку из API.
fn extension(codec: &str) -> &'static str {
    match codec {
        "flac" => "flac",
        "aac" | "he-aac" => "aac",
        _ => "mp3",
    }
}

/// id трека в имени файла: только цифры, буквы, `-` и `_` (защита от `..` и т.п.).
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Cache {
    /// Открыть кэш: прочитать индекс, выбросить записи без файлов
    /// и недокачанные остатки прошлых запусков.
    pub fn open(dir: &Path, default_limit_bytes: u64) -> Result<Self> {
        let tracks = dir.join("tracks");
        fs::create_dir_all(&tracks)?;

        let mut index: Index = match fs::read(dir.join("index.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(e.into()),
        };
        if index.limit_bytes == 0 {
            index.limit_bytes = default_limit_bytes;
        }
        index
            .entries
            .retain(|_, e| tracks.join(&e.file).is_file());

        for item in fs::read_dir(&tracks)?.flatten() {
            let path = item.path();
            if path.extension().is_some_and(|e| e == "part") {
                let _ = fs::remove_file(path);
            }
        }

        let cache = Self {
            dir: dir.to_owned(),
            index: Mutex::new(index),
        };
        cache.save()?;
        Ok(cache)
    }

    fn tracks_dir(&self) -> PathBuf {
        self.dir.join("tracks")
    }

    fn save(&self) -> Result<()> {
        let json = serde_json::to_vec_pretty(&*self.index.lock().unwrap())?;
        write_atomic(&self.dir.join("index.json"), &json)
    }

    /// Можно ли играть из кэша: Плюс подтверждён и льгота не истекла.
    pub fn playback_allowed(&self) -> bool {
        self.playback_allowed_at(now_unix())
    }

    fn playback_allowed_at(&self, now: u64) -> bool {
        self.index.lock().unwrap().plus.is_some_and(|p| {
            p.has_plus && now.saturating_sub(p.confirmed_at) <= OFFLINE_GRACE_SECS
        })
    }

    /// Отметить свежий статус подписки (после успешного запроса аккаунта).
    pub fn confirm_plus(&self, has_plus: bool) -> Result<()> {
        self.index.lock().unwrap().plus = Some(PlusMark {
            has_plus,
            confirmed_at: now_unix(),
        });
        self.save()
    }

    /// Путь к файлу трека, если он в кэше и играть из кэша разрешено.
    /// Обновляет время последнего доступа.
    pub fn lookup(&self, track_id: &str) -> Option<(PathBuf, Entry)> {
        let plus_ok = self.playback_allowed();
        let found = {
            let mut index = self.index.lock().unwrap();
            let entry = index.entries.get_mut(track_id)?;
            if needs_plus(entry) && !plus_ok {
                return None;
            }
            entry.last_access = now_unix();
            entry.clone()
        };
        let path = self.tracks_dir().join(&found.file);
        if !path.is_file() {
            self.index.lock().unwrap().entries.remove(track_id);
            let _ = self.save();
            return None;
        }
        let _ = self.save();
        Some((path, found))
    }

    pub fn contains(&self, track_id: &str) -> bool {
        self.index.lock().unwrap().entries.contains_key(track_id)
    }

    /// Куда писать недокачанный файл.
    pub fn part_path(&self, track_id: &str) -> PathBuf {
        self.tracks_dir().join(format!("{track_id}.part"))
    }

    /// Принять докачанный файл в кэш и освободить место при переполнении.
    /// `pinned` — скачан вручную.
    pub fn commit(
        &self,
        track_id: &str,
        part: &Path,
        codec: &str,
        bitrate_kbps: Option<u32>,
        meta: Option<TrackMeta>,
        pinned: bool,
    ) -> Result<()> {
        let file = format!("{track_id}.{}", extension(codec));
        let target = self.tracks_dir().join(&file);
        fs::rename(part, &target)?;
        let size = fs::metadata(&target)?.len();
        self.index.lock().unwrap().entries.insert(
            track_id.to_owned(),
            Entry {
                file,
                size,
                codec: codec.to_owned(),
                bitrate_kbps,
                last_access: now_unix(),
                meta,
                pinned,
            },
        );
        self.evict(Some(track_id));
        self.save()
    }

    /// Удалять самые давно игравшие треки, пока объём не уложится в лимит.
    /// `keep` — трек, который нельзя удалять (например, только что добавленный).
    fn evict(&self, keep: Option<&str>) {
        let mut index = self.index.lock().unwrap();
        let mut used: u64 = index.entries.values().map(|e| e.size).sum();
        if used <= index.limit_bytes {
            return;
        }
        let mut by_age: Vec<(String, u64, u64)> = index
            .entries
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != keep)
            .map(|(id, e)| (id.clone(), e.last_access, e.size))
            .collect();
        by_age.sort_by_key(|(_, at, _)| *at);
        for (id, _, size) in by_age {
            if used <= index.limit_bytes {
                break;
            }
            if let Some(e) = index.entries.remove(&id) {
                let _ = fs::remove_file(self.tracks_dir().join(e.file));
                used = used.saturating_sub(size);
            }
        }
    }

    /// id всех треков в кэше (для отметок «скачан» в списках).
    pub fn cached_ids(&self) -> Vec<String> {
        self.index.lock().unwrap().entries.keys().cloned().collect()
    }

    /// Метаданные треков в кэше — для офлайн-списка «Скачанное».
    pub fn cached_tracks(&self) -> Vec<(String, Entry)> {
        let mut list: Vec<(String, Entry)> = self
            .index
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|(id, e)| (id.clone(), e.clone()))
            .collect();
        list.sort_by_key(|(_, e)| std::cmp::Reverse(e.last_access));
        list
    }

    /// id треков, попавших в кэш без метаданных (до появления тегов).
    pub fn missing_meta(&self) -> Vec<String> {
        self.index
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|(_, e)| e.meta.is_none())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Дописать метаданные: теги в файл и запись в индекс.
    pub fn attach_meta(&self, track_id: &str, meta: TrackMeta, cover_jpeg: Option<&[u8]>) -> Result<()> {
        let Some(file) = self
            .index
            .lock()
            .unwrap()
            .entries
            .get(track_id)
            .map(|e| e.file.clone())
        else {
            return Ok(());
        };
        let path = self.tracks_dir().join(file);
        // Теги не критичны: если формат не поддержан, метаданные всё равно в индексе.
        let _ = tags::write_tags(&path, &meta, cover_jpeg);
        let size = fs::metadata(&path).map(|m| m.len()).ok();
        if let Some(entry) = self.index.lock().unwrap().entries.get_mut(track_id) {
            entry.meta = Some(meta);
            if let Some(size) = size {
                entry.size = size;
            }
        }
        self.save()
    }

    /// Папка с файлами треков.
    pub fn folder(&self) -> PathBuf {
        self.tracks_dir()
    }

    pub fn auto_cache(&self) -> bool {
        self.index.lock().unwrap().auto_cache
    }

    pub fn set_auto_cache(&self, on: bool) -> Result<()> {
        self.index.lock().unwrap().auto_cache = on;
        self.save()
    }

    pub fn stats(&self) -> CacheStats {
        let index = self.index.lock().unwrap();
        CacheStats {
            used_bytes: index.entries.values().map(|e| e.size).sum(),
            limit_bytes: index.limit_bytes,
            tracks: index.entries.len(),
        }
    }

    pub fn set_limit(&self, bytes: u64) -> Result<()> {
        self.index.lock().unwrap().limit_bytes = bytes;
        self.evict(None);
        self.save()
    }

    /// Удалить все треки (настройки и отметка о подписке остаются).
    pub fn clear(&self) -> Result<()> {
        let files: Vec<String> = self
            .index
            .lock()
            .unwrap()
            .entries
            .drain()
            .map(|(_, e)| e.file)
            .collect();
        for file in files {
            let _ = fs::remove_file(self.tracks_dir().join(file));
        }
        self.save()
    }

    /// Выход из аккаунта Яндекса: удалить треки Яндекса и отметку о подписке.
    /// Собственные треки пользователя остаются.
    pub fn wipe_account(&self) -> Result<()> {
        let files: Vec<String> = {
            let mut index = self.index.lock().unwrap();
            index.plus = None;
            let yandex: Vec<String> = index
                .entries
                .iter()
                .filter(|(_, e)| needs_plus(e))
                .map(|(id, _)| id.clone())
                .collect();
            yandex
                .iter()
                .filter_map(|id| index.entries.remove(id))
                .map(|e| e.file)
                .collect()
        };
        for file in files {
            let _ = fs::remove_file(self.tracks_dir().join(file));
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "moth-cache-{name}-{}-{}",
                std::process::id(),
                now_unix()
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn add(cache: &Cache, id: &str, bytes: usize) {
        let part = cache.part_path(id);
        fs::write(&part, vec![0u8; bytes]).unwrap();
        cache.commit(id, &part, "mp3", Some(320), None, false).unwrap();
    }

    #[test]
    fn requires_plus_for_playback() {
        let dir = TempDir::new("plus");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        add(&cache, "1", 10);
        assert!(cache.contains("1"));
        assert!(cache.lookup("1").is_none(), "без подтверждения Плюса кэш не играет");
        cache.confirm_plus(true).unwrap();
        let (path, entry) = cache.lookup("1").unwrap();
        assert!(path.ends_with("1.mp3"));
        assert_eq!(entry.size, 10);
        cache.confirm_plus(false).unwrap();
        assert!(cache.lookup("1").is_none());
    }

    #[test]
    fn grace_period() {
        let dir = TempDir::new("grace");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        cache.confirm_plus(true).unwrap();
        let now = now_unix();
        assert!(cache.playback_allowed_at(now + OFFLINE_GRACE_SECS - 10));
        assert!(!cache.playback_allowed_at(now + OFFLINE_GRACE_SECS + 10));
    }

    #[test]
    fn lru_eviction() {
        let dir = TempDir::new("lru");
        let cache = Cache::open(&dir.0, 25).unwrap();
        cache.confirm_plus(true).unwrap();
        add(&cache, "a", 10);
        add(&cache, "b", 10);
        // «a» слушали позже «b» — вытеснен должен быть «b».
        cache.index.lock().unwrap().entries.get_mut("a").unwrap().last_access = now_unix() + 100;
        add(&cache, "c", 10);
        assert!(cache.contains("a"));
        assert!(!cache.contains("b"));
        assert!(cache.contains("c"));
        assert!(!dir.0.join("tracks").join("b.mp3").exists());
        assert_eq!(cache.stats().used_bytes, 20);
    }

    #[test]
    fn reopen_keeps_index_and_drops_parts() {
        let dir = TempDir::new("reopen");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            add(&cache, "1", 5);
            fs::write(cache.part_path("2"), b"half").unwrap();
        }
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        assert!(cache.contains("1"));
        assert!(!cache.part_path("2").exists());
        assert_eq!(cache.stats().tracks, 1);
    }

    #[test]
    fn wipe_on_logout() {
        let dir = TempDir::new("wipe");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        cache.confirm_plus(true).unwrap();
        add(&cache, "1", 5);
        cache.wipe_account().unwrap();
        assert_eq!(cache.stats().tracks, 0);
        assert!(!cache.playback_allowed());
        assert!(!dir.0.join("tracks").join("1.mp3").exists());
    }

    #[test]
    fn own_tracks_do_not_depend_on_subscription() {
        let dir = TempDir::new("own");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        add(&cache, "ya", 5);
        let part = cache.part_path("mine");
        fs::write(&part, b"x").unwrap();
        let meta = TrackMeta {
            source: "s3".into(),
            id: "mine".into(),
            ..Default::default()
        };
        cache.commit("mine", &part, "flac", None, Some(meta), true).unwrap();

        // Без Плюса: свой трек играет, трек Яндекса — нет.
        assert!(cache.lookup("mine").is_some());
        assert!(cache.lookup("ya").is_none());

        // Выход из аккаунта стирает только треки Яндекса.
        cache.confirm_plus(true).unwrap();
        cache.wipe_account().unwrap();
        assert!(cache.contains("mine"));
        assert!(!cache.contains("ya"));
        assert!(dir.0.join("tracks").join("mine.flac").exists());
    }

    #[test]
    fn auto_cache_setting_persists() {
        let dir = TempDir::new("auto");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            assert!(cache.auto_cache(), "по умолчанию включено");
            cache.set_auto_cache(false).unwrap();
        }
        assert!(!Cache::open(&dir.0, 1_000).unwrap().auto_cache());
    }

    #[test]
    fn meta_is_stored() {
        let dir = TempDir::new("meta");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        let part = cache.part_path("7");
        fs::write(&part, b"x").unwrap();
        let meta = TrackMeta {
            source: "yandex".into(),
            id: "7".into(),
            title: "T".into(),
            ..Default::default()
        };
        cache.commit("7", &part, "flac", None, Some(meta.clone()), true).unwrap();
        let list = cache.cached_tracks();
        assert_eq!(list[0].1.meta.as_ref(), Some(&meta));
        assert!(list[0].1.pinned);
        assert!(list[0].1.file.ends_with(".flac"));
        assert_eq!(cache.cached_ids(), vec!["7".to_owned()]);
    }

    #[test]
    fn safe_ids() {
        assert!(is_safe_id("123456"));
        assert!(!is_safe_id("../etc"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("a/b"));
    }
}
