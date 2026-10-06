//! Источник «Яндекс Музыка» (неофициальный API).

pub mod api;
pub mod download;
pub mod dto;
pub mod oauth;
pub mod wave;

use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::model::{
    Account, AlbumSummary, ArtistPage, Playlist, Quality, SearchResults, Source, StreamInfo,
    Track,
};
use crate::provider::Provider;
use crate::Result;

pub use api::{ApiClient, YandexConfig};
pub use oauth::{DeviceCode, OAuthClient};

pub struct YandexProvider {
    api: ApiClient,
    account: OnceCell<Account>,
}

impl YandexProvider {
    pub fn new(api: ApiClient) -> Self {
        Self {
            api,
            account: OnceCell::new(),
        }
    }

    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    /// Данные аккаунта; запрашиваются один раз за сессию.
    pub async fn account(&self) -> Result<&Account> {
        self.account
            .get_or_try_init(|| self.api.account_status())
            .await
    }

    async fn uid(&self) -> Result<String> {
        Ok(self.account().await?.uid.clone())
    }

    pub async fn liked_albums(&self) -> Result<Vec<AlbumSummary>> {
        let uid = self.uid().await?;
        self.api.liked_albums(&uid).await
    }

    pub async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.api.album_with_tracks(album_id).await?.1)
    }

    pub async fn search_all(&self, text: &str) -> Result<SearchResults> {
        self.api.search_all(text).await
    }

    pub async fn artist_page(&self, artist_id: &str) -> Result<ArtistPage> {
        self.api.artist_page(artist_id).await
    }

    pub async fn artist_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        self.api.artist_tracks(artist_id).await
    }

    /// Треки плейлиста; `owner_uid` — для чужих плейлистов (из поиска).
    pub async fn playlist_tracks_of(&self, playlist_id: &str, owner_uid: Option<&str>) -> Result<Vec<Track>> {
        let uid = match owner_uid {
            Some(uid) => uid.to_owned(),
            None => self.uid().await?,
        };
        self.api.playlist_tracks(&uid, playlist_id).await
    }
}

#[async_trait]
impl Provider for YandexProvider {
    fn source(&self) -> Source {
        Source::Yandex
    }

    async fn liked_tracks(&self) -> Result<Vec<Track>> {
        let uid = self.uid().await?;
        let ids = self.api.liked_track_ids(&uid).await?;
        self.api.tracks(&ids).await
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let uid = self.uid().await?;
        self.api.playlists(&uid).await
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let uid = self.uid().await?;
        self.api.playlist_tracks(&uid, playlist_id).await
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        self.api.search_tracks(query).await
    }

    async fn stream(&self, track_id: &str, quality: Quality) -> Result<StreamInfo> {
        self.api.stream(track_id, quality).await
    }
}
