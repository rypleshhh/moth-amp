//! Общий интерфейс источника музыки (Яндекс, Subsonic, локальные файлы).

use async_trait::async_trait;

use crate::model::{Playlist, Quality, Source, StreamInfo, Track};
use crate::Result;

#[async_trait]
pub trait Provider: Send + Sync {
    fn source(&self) -> Source;

    async fn liked_tracks(&self) -> Result<Vec<Track>>;

    async fn playlists(&self) -> Result<Vec<Playlist>>;

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>>;

    async fn stream(&self, track_id: &str, quality: Quality) -> Result<StreamInfo>;
}
