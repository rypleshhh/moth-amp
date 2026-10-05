//! Источник «Яндекс Музыка» (неофициальный API).

pub mod api;
pub mod download;
pub mod dto;
pub mod oauth;
pub mod wave;

use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::model::{Account, Playlist, Quality, Source, StreamInfo, Track};
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
