//! «Моя музыка» в S3 для Flutter.

use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, Result};
use moth_core::model::{StreamInfo, TrackMeta};
use moth_core::s3::{self, LibraryEntry, S3Config, S3Library};

use super::yandex::{run, secrets, TrackDto};

static LIBRARY: RwLock<Option<Arc<S3Library>>> = RwLock::new(None);

pub struct S3ConfigDto {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
}

/// Текущие настройки без секретного ключа.
pub struct S3StatusDto {
    pub connected: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub path_style: bool,
}

pub struct S3UploadResultDto {
    pub uploaded: u32,
    /// «файл: причина» для не загрузившихся.
    pub failed: Vec<String>,
}

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("moth-amp/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Подключённая библиотека (настройки читаются из системного хранилища).
pub(crate) fn library() -> Result<Option<Arc<S3Library>>> {
    if let Some(lib) = LIBRARY.read().unwrap().as_ref() {
        return Ok(Some(lib.clone()));
    }
    let Some(config) = s3::load_config(&*secrets()?)? else {
        return Ok(None);
    };
    let lib = Arc::new(S3Library::new(&config, http()?)?);
    *LIBRARY.write().unwrap() = Some(lib.clone());
    Ok(Some(lib))
}

fn require() -> Result<Arc<S3Library>> {
    library()?.ok_or_else(|| anyhow!("хранилище S3 не подключено"))
}

fn entry_dto(lib: &S3Library, e: &LibraryEntry) -> TrackDto {
    let meta = lib.meta_with_cover(e);
    TrackDto {
        id: e.id.clone(),
        title: meta.title,
        artists: meta.artists.join(", "),
        artist_names: meta.artists,
        album: meta.album,
        year: meta.year,
        duration_ms: meta.duration_ms.and_then(|ms| u32::try_from(ms).ok()),
        available: true,
        cover_url: meta.cover_url,
        source: "s3".into(),
    }
}

/// Проверить доступ и сохранить настройки. Возвращает число треков в библиотеке.
pub async fn s3_connect(config: S3ConfigDto) -> Result<u32> {
    run(async move {
        let config = S3Config {
            endpoint: config.endpoint,
            region: config.region,
            bucket: config.bucket,
            access_key: config.access_key,
            secret_key: config.secret_key,
            path_style: config.path_style,
            prefix: "moth-amp/".into(),
        };
        let lib = S3Library::new(&config, http()?)?;
        let count = lib.check().await?;
        s3::save_config(&*secrets()?, &config)?;
        *LIBRARY.write().unwrap() = Some(Arc::new(lib));
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    })
    .await
}

pub fn s3_status() -> Result<S3StatusDto> {
    Ok(match s3::load_config(&*secrets()?)? {
        Some(c) => S3StatusDto {
            connected: true,
            endpoint: c.endpoint,
            region: c.region,
            bucket: c.bucket,
            access_key: c.access_key,
            path_style: c.path_style,
        },
        None => S3StatusDto {
            connected: false,
            endpoint: String::new(),
            region: String::new(),
            bucket: String::new(),
            access_key: String::new(),
            path_style: false,
        },
    })
}

/// Забыть настройки и ключи. Данные в бакете не трогаются.
pub fn s3_disconnect() -> Result<()> {
    s3::delete_config(&*secrets()?)?;
    *LIBRARY.write().unwrap() = None;
    Ok(())
}

pub async fn s3_tracks() -> Result<Vec<TrackDto>> {
    run(async {
        let lib = require()?;
        Ok(lib.tracks().await?.iter().map(|e| entry_dto(&lib, e)).collect())
    })
    .await
}

/// Загрузить файлы mp3/flac. Ошибки по отдельным файлам не прерывают остальные.
pub async fn s3_upload(paths: Vec<String>) -> Result<S3UploadResultDto> {
    run(async move {
        let lib = require()?;
        let mut result = S3UploadResultDto {
            uploaded: 0,
            failed: Vec::new(),
        };
        for path in paths {
            let p = Path::new(&path);
            match lib.upload(p).await {
                Ok(_) => result.uploaded += 1,
                Err(e) => {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone());
                    result.failed.push(format!("{name}: {e}"));
                }
            }
        }
        Ok(result)
    })
    .await
}

pub async fn s3_delete(track_id: String) -> Result<()> {
    run(async move {
        require()?.delete(&track_id).await?;
        Ok(())
    })
    .await
}

/// Поток и метаданные трека из S3 (для плеера, прокси и кэша).
pub(crate) async fn resolve(track_id: &str) -> Result<(StreamInfo, TrackMeta)> {
    let lib = require()?;
    let entry = lib
        .entry(track_id)
        .await?
        .ok_or_else(|| anyhow!("трека нет в библиотеке S3"))?;
    Ok((lib.stream_info(&entry), lib.meta_with_cover(&entry)))
}
