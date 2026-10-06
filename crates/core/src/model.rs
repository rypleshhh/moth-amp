//! Модели, общие для всех источников музыки.

use serde::{Deserialize, Serialize};

/// Откуда пришли данные о треке или плейлисте.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Yandex,
    Subsonic,
    Local,
}

/// Уникальный ключ трека: источник + идентификатор внутри источника.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackKey {
    pub source: Source,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: Option<String>,
    pub title: String,
    pub year: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub key: TrackKey,
    pub title: String,
    /// Подзаголовок версии: «Remastered», «Live» и т.п.
    pub version: Option<String>,
    pub artists: Vec<Artist>,
    pub album: Option<AlbumRef>,
    pub duration_ms: Option<u64>,
    /// `false`, если источник сообщает, что трек недоступен (удалён, регион и т.п.).
    pub available: bool,
    pub cover_url: Option<String>,
    pub isrc: Option<String>,
}

impl Track {
    /// Название вместе с версией: «Song (Live)».
    pub fn full_title(&self) -> String {
        match &self.version {
            Some(v) if !v.is_empty() => format!("{} ({v})", self.title),
            _ => self.title.clone(),
        }
    }

    pub fn artist_line(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlaylistKey {
    pub source: Source,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub key: PlaylistKey,
    pub title: String,
    pub track_count: Option<u32>,
    pub cover_url: Option<String>,
    /// uid владельца: нужен, чтобы открыть чужой плейлист (из поиска).
    pub owner_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
}

/// Страница исполнителя.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistPage {
    pub artist: ArtistSummary,
    pub popular_tracks: Vec<Track>,
    /// Свои альбомы.
    pub albums: Vec<AlbumSummary>,
    /// Сборники и альбомы с участием.
    pub also_albums: Vec<AlbumSummary>,
}

/// Результаты общего поиска по разделам.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResults {
    /// Тип лучшего совпадения: `artist`, `album`, `track`, `playlist`.
    pub best: Option<String>,
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumSummary>,
    pub playlists: Vec<Playlist>,
    pub tracks: Vec<Track>,
}

/// Альбом в списке (лайкнутые альбомы и т.п.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub id: String,
    pub title: String,
    pub artists: Vec<Artist>,
    pub year: Option<u32>,
    pub cover_url: Option<String>,
    pub track_count: Option<u32>,
}

impl AlbumSummary {
    pub fn artist_line(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Имя папки для скачанного альбома: «Исполнитель — Альбом (год)».
    pub fn folder_name(&self) -> String {
        let artists = self.artist_line();
        let base = if artists.is_empty() {
            self.title.clone()
        } else {
            format!("{artists} — {}", self.title)
        };
        match self.year {
            Some(y) => format!("{base} ({y})"),
            None => base,
        }
    }
}

/// Желаемое качество потока.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// Экономия трафика (мобильная сеть).
    Low,
    /// Лучший lossy-вариант (mp3 320).
    High,
    /// FLAC, если доступен; иначе лучший lossy.
    Lossless,
}

/// Готовая ссылка на аудиопоток.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    /// `true` — это 30-секундное превью (нет подписки).
    pub is_preview: bool,
}

/// Метаданные, которые вшиваются в файл кэша и хранятся в индексе.
/// Тот же набор полей читается из собственных mp3/flac пользователя.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    /// Источник: `yandex`, `local`, …
    pub source: String,
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub cover_url: Option<String>,
    pub duration_ms: Option<u64>,
}

impl TrackMeta {
    pub fn from_track(t: &Track) -> Self {
        let source = match t.key.source {
            Source::Yandex => "yandex",
            Source::Subsonic => "subsonic",
            Source::Local => "local",
        };
        Self {
            source: source.into(),
            id: t.key.id.clone(),
            title: t.full_title(),
            artists: t.artists.iter().map(|a| a.name.clone()).collect(),
            album: t.album.as_ref().map(|a| a.title.clone()),
            year: t.album.as_ref().and_then(|a| a.year),
            cover_url: t.cover_url.clone(),
            duration_ms: t.duration_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub uid: String,
    pub login: Option<String>,
    pub display_name: Option<String>,
    pub has_plus: bool,
}
