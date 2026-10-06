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
use std::sync::{Mutex, RwLock};

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
    /// Дополнительные копии в папках плейлистов (`Плейлист/имя.mp3`):
    /// жёсткие ссылки на тот же файл, а где их нельзя — копии.
    #[serde(default)]
    pub links: Vec<String>,
}

impl Entry {
    /// Основной файл и все копии (пути относительно папки треков).
    fn all_files(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.file).chain(self.links.iter())
    }
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
    /// Папка с файлами треков, выбранная пользователем; `None` — по умолчанию.
    #[serde(default)]
    tracks_folder: Option<String>,
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
            tracks_folder: None,
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
    /// Папка с файлами треков (отдельная блокировка: читается и под замком индекса).
    folder: RwLock<PathBuf>,
    index: Mutex<Index>,
}

/// Обложка папки плейлиста/альбома: её показывают Проводник и многие плееры.
const FOLDER_COVER: &str = "folder.jpg";

/// Удалить папку файла, если в ней не осталось ничего, кроме обложки,
/// и это не корневая папка треков.
fn remove_empty_parent(root: &Path, file: &Path) {
    let Some(dir) = file.parent() else { return };
    if dir == root || !dir.starts_with(root) {
        return;
    }
    let only_cover = fs::read_dir(dir).is_ok_and(|items| {
        items
            .flatten()
            .all(|i| i.file_name().to_string_lossy() == FOLDER_COVER)
    });
    if only_cover {
        let _ = fs::remove_file(dir.join(FOLDER_COVER));
        let _ = fs::remove_dir(dir);
    }
}

/// Переместить файл; между дисками — копированием.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
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

/// Сколько символов имени отдаётся под «исполнитель — название».
const READABLE_NAME_CHARS: usize = 120;

/// Убрать из имени то, что нельзя в файлах Windows/Android и ключах S3.
pub fn sanitize_name(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(READABLE_NAME_CHARS).collect();
    // Windows не любит точки и пробелы в конце имени.
    truncated.trim_end_matches(['.', ' ']).trim().to_owned()
}

/// Имя файла трека: `Исполнитель — Название (id).mp3`. id в скобках остаётся,
/// чтобы имена не совпадали; без метаданных — просто `id.mp3`.
pub fn readable_file_name(id: &str, meta: Option<&TrackMeta>, ext: &str) -> String {
    let base = meta
        .filter(|m| !m.title.trim().is_empty())
        .map(|m| {
            let artists = m.artists.join(", ");
            if artists.trim().is_empty() {
                m.title.clone()
            } else {
                format!("{artists} — {}", m.title)
            }
        })
        .map(|b| sanitize_name(&b))
        .unwrap_or_default();
    if base.is_empty() {
        format!("{id}.{ext}")
    } else {
        format!("{base} ({id}).{ext}")
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
        fs::create_dir_all(dir)?;
        let mut index: Index = match fs::read(dir.join("index.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(e.into()),
        };
        if index.limit_bytes == 0 {
            index.limit_bytes = default_limit_bytes;
        }

        let tracks = index
            .tracks_folder
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join("tracks"));
        // Своя папка может быть недоступна (отключённый диск). Тогда индекс
        // не трогаем: файлы найдутся, когда диск вернётся.
        if fs::create_dir_all(&tracks).is_ok() && tracks.is_dir() {
            index
                .entries
                .retain(|_, e| tracks.join(&e.file).is_file());
            for entry in index.entries.values_mut() {
                entry.links.retain(|l| tracks.join(l).is_file());
            }
            for item in fs::read_dir(&tracks)?.flatten() {
                let path = item.path();
                if path.extension().is_some_and(|e| e == "part") {
                    let _ = fs::remove_file(path);
                }
            }
        }

        let cache = Self {
            dir: dir.to_owned(),
            folder: RwLock::new(tracks),
            index: Mutex::new(index),
        };
        let ids: Vec<String> = cache.index.lock().unwrap().entries.keys().cloned().collect();
        for id in ids {
            cache.rename_readable(&id);
        }
        cache.save()?;
        Ok(cache)
    }

    /// Переименовать файл трека в читаемое имя по его метаданным. Если файл
    /// занят (играет) или имя уже занято — оставить как есть.
    fn rename_readable(&self, track_id: &str) {
        let mut index = self.index.lock().unwrap();
        let Some(entry) = index.entries.get_mut(track_id) else {
            return;
        };
        let ext = Path::new(&entry.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp3")
            .to_owned();
        let wanted = readable_file_name(track_id, entry.meta.as_ref(), &ext);
        if wanted == entry.file {
            return;
        }
        let from = self.tracks_dir().join(&entry.file);
        let to = self.tracks_dir().join(&wanted);
        if !to.exists() && fs::rename(&from, &to).is_ok() {
            entry.file = wanted;
        }
    }

    fn tracks_dir(&self) -> PathBuf {
        self.folder.read().unwrap().clone()
    }

    /// Выбрать папку для файлов треков (`None` — по умолчанию). Уже скачанные
    /// файлы переезжают; при сбое перенос откатывается. Возвращает число
    /// перенесённых файлов.
    pub fn set_folder(&self, folder: Option<&Path>) -> Result<usize> {
        let target = folder
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.dir.join("tracks"));
        fs::create_dir_all(&target)?;
        let current = self.tracks_dir();
        if fs::canonicalize(&current).ok() == fs::canonicalize(&target).ok() {
            self.index.lock().unwrap().tracks_folder =
                folder.map(|p| p.to_string_lossy().into_owned());
            self.save()?;
            return Ok(0);
        }

        let mut index = self.index.lock().unwrap();
        let mut moved: Vec<String> = Vec::new();
        let files: Vec<String> = index
            .entries
            .values()
            .flat_map(|e| e.all_files().cloned())
            .collect();
        for file in files {
            let from = current.join(&file);
            let to = target.join(&file);
            if !from.is_file() || to.exists() {
                continue;
            }
            let result = to
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| move_file(&from, &to));
            if let Err(e) = result {
                for file in &moved {
                    let _ = move_file(&target.join(file), &current.join(file));
                }
                return Err(e.into());
            }
            moved.push(file);
        }
        // Обложки папок плейлистов переезжают следом; пустые папки — убрать.
        for file in &moved {
            if let Some(sub) = Path::new(file).parent().filter(|p| !p.as_os_str().is_empty()) {
                let cover = current.join(sub).join(FOLDER_COVER);
                if cover.is_file() {
                    let _ = move_file(&cover, &target.join(sub).join(FOLDER_COVER));
                }
            }
            remove_empty_parent(&current, &current.join(file));
        }
        *self.folder.write().unwrap() = target;
        index.tracks_folder = folder.map(|p| p.to_string_lossy().into_owned());
        drop(index);
        self.save()?;
        Ok(moved.len())
    }

    /// Выбрана ли своя папка (а не папка по умолчанию).
    pub fn custom_folder(&self) -> bool {
        self.index.lock().unwrap().tracks_folder.is_some()
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

    /// Файл и запись без проверки подписки — для копирования в своё хранилище.
    pub fn entry(&self, track_id: &str) -> Option<(PathBuf, Entry)> {
        let entry = self.index.lock().unwrap().entries.get(track_id).cloned()?;
        let path = self.tracks_dir().join(&entry.file);
        path.is_file().then_some((path, entry))
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
        let file = readable_file_name(track_id, meta.as_ref(), extension(codec));
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
                links: Vec::new(),
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
                self.remove_files(&e);
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
        self.rename_readable(track_id);
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
        let entries: Vec<Entry> = self
            .index
            .lock()
            .unwrap()
            .entries
            .drain()
            .map(|(_, e)| e)
            .collect();
        for entry in &entries {
            self.remove_files(entry);
        }
        self.save()
    }

    /// Удалить файл трека и его копии в папках плейлистов (и опустевшие папки).
    fn remove_files(&self, entry: &Entry) {
        let root = self.tracks_dir();
        for file in entry.all_files() {
            let path = root.join(file);
            let _ = fs::remove_file(&path);
            remove_empty_parent(&root, &path);
        }
    }

    /// Обложка папки плейлиста или альбома (`folder.jpg`).
    pub fn set_folder_cover(&self, folder: &str, jpeg: &[u8]) -> Result<()> {
        let sub = sanitize_name(folder);
        if sub.is_empty() {
            return Err(crate::Error::Storage("пустое имя папки".into()));
        }
        let dir = self.tracks_dir().join(sub);
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(FOLDER_COVER), jpeg)
    }

    /// Положить трек ещё и в папку плейлиста: `<папка треков>/<плейлист>/`.
    /// Жёсткая ссылка не занимает лишнего места; на другом диске — копия.
    pub fn place_in_folder(&self, track_id: &str, folder: &str) -> Result<()> {
        let sub = sanitize_name(folder);
        if sub.is_empty() {
            return Err(crate::Error::Storage("пустое имя папки".into()));
        }
        let root = self.tracks_dir();
        let mut index = self.index.lock().unwrap();
        let entry = index
            .entries
            .get_mut(track_id)
            .ok_or_else(|| crate::Error::Storage("трека нет в кэше".into()))?;
        let name = Path::new(&entry.file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.file.clone());
        let rel = format!("{sub}/{name}");
        if entry.file == rel || entry.links.contains(&rel) {
            return Ok(());
        }
        let from = root.join(&entry.file);
        let to = root.join(&rel);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir)?;
        }
        if !to.exists() {
            fs::hard_link(&from, &to).or_else(|_| fs::copy(&from, &to).map(|_| ()))?;
        }
        entry.links.push(rel);
        drop(index);
        self.save()
    }

    /// Выход из аккаунта Яндекса: удалить треки Яндекса и отметку о подписке.
    /// Собственные треки пользователя остаются.
    pub fn wipe_account(&self) -> Result<()> {
        let files: Vec<Entry> = {
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
                .collect::<Vec<Entry>>()
        };
        for entry in &files {
            self.remove_files(entry);
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
    fn readable_names() {
        let meta = TrackMeta {
            id: "42".into(),
            title: "Us And Them (2011 - Remaster)".into(),
            artists: vec!["Pink Floyd".into()],
            ..Default::default()
        };
        assert_eq!(
            readable_file_name("42", Some(&meta), "mp3"),
            "Pink Floyd — Us And Them (2011 - Remaster) (42).mp3"
        );
        // Запрещённые символы и хвостовые точки.
        let bad = TrackMeta {
            title: "What?/Why: \"yes\"...".into(),
            artists: vec!["AC/DC".into(), "Гость".into()],
            ..Default::default()
        };
        assert_eq!(
            readable_file_name("7", Some(&bad), "flac"),
            "AC DC, Гость — What Why yes (7).flac"
        );
        // Без названия или метаданных — просто id.
        assert_eq!(readable_file_name("7", None, "mp3"), "7.mp3");
        assert_eq!(readable_file_name("7", Some(&TrackMeta::default()), "mp3"), "7.mp3");
        // Длинное имя обрезается.
        let long = TrackMeta {
            title: "x".repeat(500),
            ..Default::default()
        };
        assert!(readable_file_name("7", Some(&long), "mp3").chars().count() < 140);
    }

    #[test]
    fn old_files_are_renamed_on_open() {
        let dir = TempDir::new("rename");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            add(&cache, "5", 3);
            assert!(dir.0.join("tracks").join("5.mp3").exists());
            let meta = TrackMeta {
                source: "yandex".into(),
                id: "5".into(),
                title: "Песня".into(),
                artists: vec!["Группа".into()],
                ..Default::default()
            };
            // Как у записей, сохранённых до появления метаданных: метаданные
            // есть, а файл ещё со старым именем.
            cache.index.lock().unwrap().entries.get_mut("5").unwrap().meta = Some(meta);
            cache.save().unwrap();
        }
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        let (path, _) = cache.entry("5").unwrap();
        assert!(path.ends_with("Группа — Песня (5).mp3"));
        assert!(!dir.0.join("tracks").join("5.mp3").exists());
    }

    #[test]
    fn custom_folder_moves_files_and_persists() {
        let dir = TempDir::new("folder");
        let other = TempDir::new("folder-target");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            cache.confirm_plus(true).unwrap();
            add(&cache, "1", 4);
            assert_eq!(cache.set_folder(Some(&other.0)).unwrap(), 1);
            assert!(other.0.join("1.mp3").exists());
            assert!(!dir.0.join("tracks").join("1.mp3").exists());
            assert!(cache.lookup("1").unwrap().0.starts_with(&other.0));
            assert!(cache.custom_folder());
        }
        // Выбор сохраняется между запусками.
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        assert!(cache.folder().starts_with(&other.0));
        assert!(cache.contains("1"));
        // Обратно — в папку по умолчанию.
        assert_eq!(cache.set_folder(None).unwrap(), 1);
        assert!(dir.0.join("tracks").join("1.mp3").exists());
        assert!(!cache.custom_folder());
    }

    #[test]
    fn unavailable_folder_keeps_index() {
        let dir = TempDir::new("gone");
        let other = TempDir::new("gone-target");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            add(&cache, "1", 4);
            cache.set_folder(Some(&other.0)).unwrap();
        }
        // «Диск отключили»: на месте папки — файл, создать папку нельзя.
        fs::remove_dir_all(&other.0).unwrap();
        fs::write(&other.0, b"not a dir").unwrap();
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        assert!(cache.contains("1"), "запись не должна пропасть");
        fs::remove_file(&other.0).unwrap();
        fs::create_dir_all(&other.0).unwrap();
    }

    #[test]
    fn playlist_folders() {
        let dir = TempDir::new("pl");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        add(&cache, "1", 4);
        let tracks = dir.0.join("tracks");

        cache.place_in_folder("1", "Дорога: ночь?").unwrap();
        cache.place_in_folder("1", "Дорога: ночь?").unwrap(); // повтор — без дублей
        cache.place_in_folder("1", "Утро").unwrap();
        assert!(tracks.join("Дорога ночь").join("1.mp3").is_file());
        assert!(tracks.join("Утро").join("1.mp3").is_file());
        assert_eq!(cache.entry("1").unwrap().1.links.len(), 2);
        assert!(cache.place_in_folder("2", "Утро").is_err(), "трека нет в кэше");
        assert!(cache.place_in_folder("1", "  ").is_err());

        // Объём считается один раз.
        assert_eq!(cache.stats().used_bytes, 4);

        // Смена папки переносит и папки плейлистов.
        let other = TempDir::new("pl-target");
        cache.set_folder(Some(&other.0)).unwrap();
        assert!(other.0.join("Утро").join("1.mp3").is_file());
        assert!(!tracks.join("Утро").exists(), "пустая папка в старом месте убрана");

        // Очистка убирает все копии и папки плейлистов.
        cache.clear().unwrap();
        assert!(!other.0.join("Утро").exists());
        assert!(!other.0.join("Дорога ночь").exists());
        assert!(!other.0.join("1.mp3").exists());
    }

    #[test]
    fn folder_cover_follows_folder() {
        let dir = TempDir::new("cover");
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        add(&cache, "1", 4);
        cache.place_in_folder("1", "Альбом").unwrap();
        cache.set_folder_cover("Альбом", b"jpg").unwrap();
        assert!(dir.0.join("tracks").join("Альбом").join("folder.jpg").is_file());

        let other = TempDir::new("cover-target");
        cache.set_folder(Some(&other.0)).unwrap();
        assert!(other.0.join("Альбом").join("folder.jpg").is_file());
        assert!(!dir.0.join("tracks").join("Альбом").exists());

        // Очистка: в папке остаётся только обложка — папка убирается целиком.
        cache.clear().unwrap();
        assert!(!other.0.join("Альбом").exists());
    }

    #[test]
    fn manually_deleted_copies_are_forgotten() {
        let dir = TempDir::new("pl-gone");
        {
            let cache = Cache::open(&dir.0, 1_000).unwrap();
            add(&cache, "1", 4);
            cache.place_in_folder("1", "Утро").unwrap();
        }
        fs::remove_file(dir.0.join("tracks").join("Утро").join("1.mp3")).unwrap();
        let cache = Cache::open(&dir.0, 1_000).unwrap();
        assert!(cache.entry("1").unwrap().1.links.is_empty());
    }

    #[test]
    fn safe_ids() {
        assert!(is_safe_id("123456"));
        assert!(!is_safe_id("../etc"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("a/b"));
    }
}
