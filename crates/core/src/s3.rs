//! «Моя музыка»: собственные mp3/flac в S3-совместимом хранилище
//! (Yandex Object Storage, Selectel, VK Cloud, Cloudflare R2, свой сервер…).
//!
//! Раскладка в бакете (всё под префиксом, по умолчанию `moth-amp/`):
//! - `tracks/<id>.mp3|flac` — файлы как есть, с тегами;
//! - `covers/<id>.<ext>` — обложка из тегов, для списков;
//! - `library.json` — индекс с метаданными, список открывается одним запросом.
//!
//! `id` — MD5 содержимого файла: повторная загрузка того же файла не дублирует его.
//! Запросы подписываются (AWS SigV4) временными ссылками; воспроизведение идёт
//! по ссылке напрямую, Range поддерживается самим S3.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusty_s3::actions::ListObjectsV2;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::{Deserialize, Serialize};

use crate::auth::now_unix;
use crate::cache::tags;
use crate::model::{StreamInfo, TrackMeta};
use crate::secrets::SecretStore;
use crate::{Error, Result};

/// Сколько живут подписанные ссылки.
const URL_TTL: Duration = Duration::from_secs(60 * 60);

fn default_prefix() -> String {
    "moth-amp/".into()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct S3Config {
    /// Например, `https://storage.yandexcloud.net`.
    pub endpoint: String,
    /// Например, `ru-central1`.
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    /// Адреса вида `endpoint/bucket/key` (нужно многим своим серверам).
    #[serde(default)]
    pub path_style: bool,
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

// Секретный ключ в логи не попадает.
impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .field("path_style", &self.path_style)
            .field("prefix", &self.prefix)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub id: String,
    /// Ключ файла в бакете.
    pub file: String,
    /// Ключ обложки в бакете.
    pub cover: Option<String>,
    pub size: u64,
    pub codec: String,
    pub meta: TrackMeta,
    pub added_at: u64,
}

/// Трек кэша в бакете (`cache/<источник>/<id>.<ext>`): общий кэш для
/// всех устройств пользователя. Для треков Яндекса действуют те же правила,
/// что и для локального кэша (подписка, удаление при выходе).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedObject {
    pub id: String,
    pub file: String,
    pub size: u64,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    pub meta: Option<TrackMeta>,
    pub added_at: u64,
}

impl CachedObject {
    /// Трек Яндекса (или без метаданных) — нужен подтверждённый Плюс.
    pub fn needs_plus(&self) -> bool {
        self.meta.as_ref().is_none_or(|m| m.source == "yandex")
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheIndex {
    #[serde(default)]
    tracks: Vec<CachedObject>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LibraryIndex {
    #[serde(default)]
    tracks: Vec<LibraryEntry>,
}

pub struct S3Library {
    bucket: Bucket,
    creds: Credentials,
    prefix: String,
    http: reqwest::Client,
}

fn s3_error(context: &str, status: reqwest::StatusCode, body: &str) -> Error {
    // В ответе S3 код ошибки в <Code>…</Code>.
    let code = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(code, _)| code)
        .unwrap_or("");
    Error::Unexpected(format!("S3 {context}: HTTP {status} {code}"))
}

impl S3Library {
    pub fn new(config: &S3Config, http: reqwest::Client) -> Result<Self> {
        let endpoint = url::Url::parse(config.endpoint.trim())
            .map_err(|e| Error::Unexpected(format!("адрес S3: {e}")))?;
        let style = if config.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket = Bucket::new(
            endpoint,
            style,
            config.bucket.trim().to_owned(),
            config.region.trim().to_owned(),
        )
        .map_err(|e| Error::Unexpected(format!("бакет S3: {e:?}")))?;
        let mut prefix = config.prefix.trim().trim_start_matches('/').to_owned();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Ok(Self {
            bucket,
            creds: Credentials::new(config.access_key.trim(), config.secret_key.trim()),
            prefix,
            http,
        })
    }

    fn key(&self, rel: &str) -> String {
        format!("{}{rel}", self.prefix)
    }

    /// Подписанная ссылка на объект (для плеера и обложек).
    pub fn object_url(&self, key: &str) -> String {
        self.bucket
            .get_object(Some(&self.creds), key)
            .sign(URL_TTL)
            .to_string()
    }

    async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let resp = self.http.get(self.object_url(key)).send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(s3_error("чтение", status, &body));
        }
        Ok(Some(resp.bytes().await?.to_vec()))
    }

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
        let url = self.bucket.put_object(Some(&self.creds), key).sign(URL_TTL);
        let resp = self
            .http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(s3_error("запись", status, &body));
        }
        Ok(())
    }

    async fn delete_key(&self, key: &str) -> Result<()> {
        let url = self.bucket.delete_object(Some(&self.creds), key).sign(URL_TTL);
        let resp = self.http.delete(url).send().await?;
        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(s3_error("удаление", status, &body));
        }
        Ok(())
    }

    async fn read_index(&self) -> Result<LibraryIndex> {
        match self.get_bytes(&self.key("library.json")).await? {
            Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
            None => Ok(LibraryIndex::default()),
        }
    }

    async fn write_index(&self, index: &LibraryIndex) -> Result<()> {
        let json = serde_json::to_vec_pretty(index)?;
        self.put_bytes(&self.key("library.json"), json, "application/json")
            .await
    }

    /// Проверка доступа: ключи, бакет, права на чтение. Возвращает число треков.
    pub async fn check(&self) -> Result<usize> {
        let mut list: ListObjectsV2<'_> = self.bucket.list_objects_v2(Some(&self.creds));
        list.with_prefix(self.prefix.as_str());
        list.with_max_keys(1);
        let resp = self.http.get(list.sign(URL_TTL)).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(s3_error("доступ к бакету", status, &body));
        }
        ListObjectsV2::parse_response(&body)
            .map_err(|e| Error::Unexpected(format!("S3: непонятный ответ на список: {e}")))?;
        Ok(self.read_index().await?.tracks.len())
    }

    /// Все треки библиотеки, сначала недавно добавленные.
    pub async fn tracks(&self) -> Result<Vec<LibraryEntry>> {
        let mut tracks = self.read_index().await?.tracks;
        tracks.sort_by_key(|t| std::cmp::Reverse(t.added_at));
        Ok(tracks)
    }

    pub async fn entry(&self, id: &str) -> Result<Option<LibraryEntry>> {
        Ok(self.read_index().await?.tracks.into_iter().find(|t| t.id == id))
    }

    /// Ссылка на поток для плеера.
    pub fn stream_info(&self, entry: &LibraryEntry) -> StreamInfo {
        StreamInfo {
            url: self.object_url(&entry.file),
            codec: entry.codec.clone(),
            bitrate_kbps: bitrate_kbps(entry),
            is_preview: false,
        }
    }

    /// Метаданные со свежей ссылкой на обложку.
    pub fn meta_with_cover(&self, entry: &LibraryEntry) -> TrackMeta {
        let mut meta = entry.meta.clone();
        meta.cover_url = entry.cover.as_deref().map(|k| self.object_url(k));
        meta
    }

    /// Загрузить mp3/flac. Теги, длительность и обложка берутся из файла.
    pub async fn upload(&self, path: &Path) -> Result<LibraryEntry> {
        let info = {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || tags::read_file_info(&path))
                .await
                .map_err(|e| Error::Unexpected(e.to_string()))??
        };
        let bytes = tokio::fs::read(path).await?;
        let id = format!("{:x}", md5::compute(&bytes));
        let size = bytes.len() as u64;

        let file = self.key(&format!("tracks/{id}.{}", info.codec));
        let content_type = if info.codec == "flac" { "audio/flac" } else { "audio/mpeg" };
        self.put_bytes(&file, bytes, content_type).await?;

        let cover = match info.cover {
            Some((data, mime)) => {
                let ext = if mime.contains("png") { "png" } else { "jpg" };
                let key = self.key(&format!("covers/{id}.{ext}"));
                self.put_bytes(&key, data, &mime).await?;
                Some(key)
            }
            None => None,
        };

        let title = info.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| id.clone())
        });
        let entry = LibraryEntry {
            id: id.clone(),
            file,
            cover,
            size,
            codec: info.codec.to_owned(),
            meta: TrackMeta {
                source: "s3".into(),
                id: id.clone(),
                title,
                artists: info.artists,
                album: info.album,
                year: info.year,
                cover_url: None,
                duration_ms: Some(info.duration_ms),
            },
            added_at: now_unix(),
        };

        let mut index = self.read_index().await?;
        index.tracks.retain(|t| t.id != id);
        index.tracks.push(entry.clone());
        self.write_index(&index).await?;
        Ok(entry)
    }

    fn setting_key(&self, name: &str) -> Result<String> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(Error::Unexpected(format!("недопустимое имя настройки: {name}")));
        }
        Ok(self.key(&format!("settings/{name}.json")))
    }

    /// Прочитать настройку (`settings/<name>.json`), `None` — её ещё нет.
    pub async fn get_setting(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.get_bytes(&self.setting_key(name)?).await
    }

    /// Записать настройку (`settings/<name>.json`).
    pub async fn put_setting(&self, name: &str, json: Vec<u8>) -> Result<()> {
        self.put_bytes(&self.setting_key(name)?, json, "application/json")
            .await
    }

    // ---- кэш в бакете ----

    async fn read_cache_index(&self) -> Result<CacheIndex> {
        match self.get_bytes(&self.key("cache/index.json")).await? {
            Some(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            None => Ok(CacheIndex::default()),
        }
    }

    async fn write_cache_index(&self, index: &CacheIndex) -> Result<()> {
        let json = serde_json::to_vec_pretty(index)?;
        self.put_bytes(&self.key("cache/index.json"), json, "application/json")
            .await
    }

    /// Все треки кэша в бакете.
    pub async fn cache_list(&self) -> Result<Vec<CachedObject>> {
        Ok(self.read_cache_index().await?.tracks)
    }

    pub async fn cache_get(&self, id: &str) -> Result<Option<CachedObject>> {
        Ok(self.read_cache_index().await?.tracks.into_iter().find(|t| t.id == id))
    }

    /// Положить файл из локального кэша в бакет (если его там ещё нет).
    pub async fn cache_put(
        &self,
        id: &str,
        path: &Path,
        codec: &str,
        bitrate_kbps: Option<u32>,
        meta: Option<TrackMeta>,
    ) -> Result<()> {
        if !crate::cache::is_safe_id(id) {
            return Err(Error::Unexpected("некорректный id трека".into()));
        }
        let mut index = self.read_cache_index().await?;
        if index.tracks.iter().any(|t| t.id == id) {
            return Ok(());
        }
        let source = meta.as_ref().map_or("yandex", |m| m.source.as_str()).to_owned();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp3").to_owned();
        let file = self.key(&format!("cache/{source}/{id}.{ext}"));
        let bytes = tokio::fs::read(path).await?;
        let size = bytes.len() as u64;
        let content_type = if ext == "flac" { "audio/flac" } else { "audio/mpeg" };
        self.put_bytes(&file, bytes, content_type).await?;
        index.tracks.retain(|t| t.id != id);
        index.tracks.push(CachedObject {
            id: id.to_owned(),
            file,
            size,
            codec: codec.to_owned(),
            bitrate_kbps,
            meta,
            added_at: now_unix(),
        });
        self.write_cache_index(&index).await
    }

    /// Ссылка на поток трека из кэша в бакете.
    pub fn cache_stream_info(&self, obj: &CachedObject) -> StreamInfo {
        StreamInfo {
            url: self.object_url(&obj.file),
            codec: obj.codec.clone(),
            bitrate_kbps: obj.bitrate_kbps,
            is_preview: false,
        }
    }

    /// Удалить из бакета кэш треков Яндекса (выход из аккаунта).
    /// Возвращает число удалённых треков.
    pub async fn cache_remove_yandex(&self) -> Result<usize> {
        let mut index = self.read_cache_index().await?;
        let (gone, keep): (Vec<CachedObject>, Vec<CachedObject>) =
            index.tracks.into_iter().partition(CachedObject::needs_plus);
        index.tracks = keep;
        self.write_cache_index(&index).await?;
        for obj in &gone {
            self.delete_key(&obj.file).await?;
        }
        Ok(gone.len())
    }

    /// Удалить трек из библиотеки (файл, обложку, запись в индексе).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let mut index = self.read_index().await?;
        let Some(pos) = index.tracks.iter().position(|t| t.id == id) else {
            return Ok(());
        };
        let entry = index.tracks.remove(pos);
        self.write_index(&index).await?;
        self.delete_key(&entry.file).await?;
        if let Some(cover) = &entry.cover {
            self.delete_key(cover).await?;
        }
        Ok(())
    }
}

/// Средний битрейт по размеру и длительности (для отображения).
fn bitrate_kbps(entry: &LibraryEntry) -> Option<u32> {
    let ms = entry.meta.duration_ms.filter(|&ms| ms > 0)?;
    u32::try_from(entry.size * 8 / ms).ok()
}

// ---- хранение настроек ----

const CONFIG_SECRET: &str = "s3";

/// Настройки S3 (вместе с ключами) из хранилища секретов.
pub fn load_config(store: &dyn SecretStore) -> Result<Option<S3Config>> {
    match store.get(CONFIG_SECRET)? {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

pub fn save_config(store: &dyn SecretStore, config: &S3Config) -> Result<()> {
    store.set(CONFIG_SECRET, &serde_json::to_string(config)?)
}

pub fn delete_config(store: &dyn SecretStore) -> Result<()> {
    store.delete(CONFIG_SECRET)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(endpoint: &str) -> S3Config {
        S3Config {
            endpoint: endpoint.into(),
            region: "ru-central1".into(),
            bucket: "music".into(),
            access_key: "AK".into(),
            secret_key: "SECRET".into(),
            path_style: true,
            prefix: "moth-amp".into(),
        }
    }

    #[test]
    fn debug_hides_secret() {
        let s = format!("{:?}", config("https://storage.yandexcloud.net"));
        assert!(!s.contains("SECRET"));
        assert!(s.contains("AK"));
    }

    #[test]
    fn presigned_url_shape() {
        let lib = S3Library::new(&config("https://storage.yandexcloud.net"), reqwest::Client::new()).unwrap();
        let url = lib.object_url(&lib.key("tracks/x.mp3"));
        assert!(url.starts_with("https://storage.yandexcloud.net/music/moth-amp/tracks/x.mp3?"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(url.contains("X-Amz-Expires=3600"));
        assert!(!url.contains("SECRET"));
    }

    #[test]
    fn bad_endpoint() {
        assert!(S3Library::new(&config("not a url"), reqwest::Client::new()).is_err());
    }

    #[test]
    fn bitrate_estimate() {
        let entry = LibraryEntry {
            id: "1".into(),
            file: "f".into(),
            cover: None,
            size: 8_000_000,
            codec: "mp3".into(),
            meta: TrackMeta {
                duration_ms: Some(200_000),
                ..Default::default()
            },
            added_at: 0,
        };
        assert_eq!(bitrate_kbps(&entry), Some(320));
    }

    #[test]
    fn s3_error_code() {
        let e = s3_error("x", reqwest::StatusCode::FORBIDDEN, "<Error><Code>AccessDenied</Code></Error>");
        assert!(e.to_string().contains("AccessDenied"));
    }

    /// Сквозная проверка с настоящим S3-совместимым сервером. Запуск:
    /// `MOTH_S3_TEST=http://127.0.0.1:9000 cargo test -p moth-core s3_roundtrip -- --ignored`
    /// (бакет `music`, ключи `moth`/`mothtest123`).
    #[tokio::test]
    #[ignore]
    async fn s3_roundtrip() {
        let endpoint = std::env::var("MOTH_S3_TEST").expect("MOTH_S3_TEST");
        let mut cfg = config(&endpoint);
        cfg.access_key = "moth".into();
        cfg.secret_key = "mothtest123".into();
        cfg.region = "us-east-1".into();
        let lib = S3Library::new(&cfg, reqwest::Client::new()).unwrap();
        // Бакет может уже существовать — ошибку создания игнорируем.
        let _ = reqwest::Client::new()
            .put(lib.bucket.create_bucket(&lib.creds).sign(URL_TTL))
            .send()
            .await;

        let dir = std::env::temp_dir().join(format!("moth-s3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Мой трек.mp3");
        std::fs::write(&path, crate::cache::tags::tests_support::tiny_mp3()).unwrap();
        let meta = TrackMeta {
            title: "Моя песня".into(),
            artists: vec!["Я".into()],
            ..Default::default()
        };
        tags::write_tags(&path, &meta, Some(&[0xFF, 0xD8, 0xFF, 0xD9])).unwrap();

        let before = lib.check().await.unwrap();
        let entry = lib.upload(&path).await.unwrap();
        assert_eq!(entry.meta.title, "Моя песня");
        assert_eq!(entry.meta.artists, vec!["Я".to_owned()]);
        assert!(entry.cover.is_some());
        assert_eq!(lib.check().await.unwrap(), before + 1);

        // Повторная загрузка того же файла не дублирует.
        lib.upload(&path).await.unwrap();
        assert_eq!(lib.tracks().await.unwrap().len(), before + 1);

        // Поток читается по подписанной ссылке, с Range.
        let info = lib.stream_info(&entry);
        let resp = reqwest::Client::new()
            .get(&info.url)
            .header(reqwest::header::RANGE, "bytes=0-9")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 206);
        assert_eq!(resp.bytes().await.unwrap().len(), 10);

        lib.delete(&entry.id).await.unwrap();
        assert_eq!(lib.tracks().await.unwrap().len(), before);

        // Кэш в бакете: положить, найти, прочитать, стереть при выходе.
        let ya = TrackMeta {
            source: "yandex".into(),
            id: "777".into(),
            title: "Трек".into(),
            ..Default::default()
        };
        lib.cache_put("777", &path, "mp3", Some(320), Some(ya)).await.unwrap();
        lib.cache_put("777", &path, "mp3", Some(320), None).await.unwrap();
        let obj = lib.cache_get("777").await.unwrap().unwrap();
        assert!(obj.needs_plus());
        let resp = reqwest::get(lib.cache_stream_info(&obj).url).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(lib.cache_remove_yandex().await.unwrap(), 1);
        assert!(lib.cache_get("777").await.unwrap().is_none());

        lib.put_setting("test_eq", b"{\"a\":1}".to_vec()).await.unwrap();
        assert_eq!(lib.get_setting("test_eq").await.unwrap().unwrap(), b"{\"a\":1}");
        assert!(lib.get_setting("missing").await.unwrap().is_none());
        assert!(lib.get_setting("../x").await.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
