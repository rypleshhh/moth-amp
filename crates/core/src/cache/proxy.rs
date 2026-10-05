//! Локальный HTTP-прокси для первого воспроизведения.
//!
//! На каждый трек — одна загрузка с источника целиком во временный файл;
//! плееру данные отдаются из этого растущего файла с поддержкой `Range`
//! (mpv при открытии mp3 прыгает в конец за тегами и обратно — это отдельные
//! запросы). Так трафик не удваивается, а докачанный файл попадает в кэш.
//!
//! - Слушает только `127.0.0.1`; в пути случайный токен, чтобы другие
//!   программы на компьютере не могли играть через аккаунт пользователя.
//! - Если плеер не подключался дольше `grace` (трек пропущен), загрузка
//!   отменяется, недокачанное удаляется.
//! - Готовый файл переносится в кэш, когда его отпустили все читатели
//!   (на Windows открытый файл нельзя переименовать).

use std::collections::HashMap;
use std::future::Future;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::CONTENT_TYPE;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex};

use super::{is_safe_id, tags, Cache};
use crate::model::{StreamInfo, TrackMeta};
use crate::{Error, Result};

/// Ссылка на поток и метаданные для тегов файла кэша.
pub struct ResolvedTrack {
    pub stream: StreamInfo,
    pub meta: Option<TrackMeta>,
}

pub type ResolveFuture = Pin<Box<dyn Future<Output = Result<ResolvedTrack>> + Send>>;

/// Получить ссылку на поток (и метаданные) по id трека.
pub type Resolver = Arc<dyn Fn(String) -> ResolveFuture + Send + Sync>;

const MAX_HEAD_BYTES: usize = 16 * 1024;
const DEFAULT_GRACE: Duration = Duration::from_secs(15);
/// Сколько ждать, пока плеер отпустит докачанный файл.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, Default)]
struct Progress {
    written: u64,
    finished: bool,
    failed: bool,
}

struct Download {
    path: PathBuf,
    total: Option<u64>,
    content_type: String,
    progress: watch::Receiver<Progress>,
    clients: AtomicUsize,
    cancelled: AtomicBool,
    /// Скачивается вручную: уход плеера с трека загрузку не отменяет.
    pinned: AtomicBool,
}

struct Shared {
    cache: Arc<Cache>,
    resolver: Resolver,
    http: reqwest::Client,
    token: String,
    grace: Duration,
    downloads: Mutex<HashMap<String, Arc<Download>>>,
}

pub struct Proxy {
    port: u16,
    token: String,
    shared: Arc<Shared>,
}

impl Proxy {
    pub async fn start(cache: Arc<Cache>, resolver: Resolver, http: reqwest::Client) -> Result<Self> {
        Self::start_with_grace(cache, resolver, http, DEFAULT_GRACE).await
    }

    pub(crate) async fn start_with_grace(
        cache: Arc<Cache>,
        resolver: Resolver,
        http: reqwest::Client,
        grace: Duration,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let shared = Arc::new(Shared {
            cache,
            resolver,
            http,
            token: token.clone(),
            grace,
            downloads: Mutex::new(HashMap::new()),
        });
        let accept_shared = shared.clone();
        tokio::spawn(async move {
            let shared = accept_shared;
            loop {
                let sock = match listener.accept().await {
                    Ok((sock, _)) => sock,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let _ = handle(sock, shared).await;
                });
            }
        });
        Ok(Self {
            port,
            token,
            shared,
        })
    }

    /// Адрес, который отдаётся плееру.
    pub fn url(&self, track_id: &str) -> String {
        format!("http://127.0.0.1:{}/{}/{track_id}", self.port, self.token)
    }

    /// Скачать трек в кэш вручную (иконка загрузки). Если трек уже играет
    /// через прокси, используется та же загрузка. Завершается, когда файл в кэше.
    pub async fn download(&self, track_id: &str) -> Result<()> {
        if !is_safe_id(track_id) {
            return Err(Error::Unexpected("некорректный id трека".into()));
        }
        let cache = &self.shared.cache;
        if cache.contains(track_id) {
            return Ok(());
        }
        let dl = attach(&self.shared, track_id)
            .await
            .ok_or_else(|| Error::Unexpected("не удалось начать загрузку".into()))?;
        dl.pinned.store(true, Ordering::SeqCst);
        // Мы не читатель: отдаём «место» обратно.
        dl.clients.fetch_sub(1, Ordering::SeqCst);

        let mut rx = dl.progress.clone();
        loop {
            let p = *rx.borrow_and_update();
            if p.finished || p.failed || rx.changed().await.is_err() {
                break;
            }
        }
        // Перенос в кэш происходит, когда файл отпустят читатели.
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        while Instant::now() < deadline {
            if cache.contains(track_id) {
                return Ok(());
            }
            if !self.shared.downloads.lock().await.contains_key(track_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if cache.contains(track_id) {
            Ok(())
        } else {
            Err(Error::Unexpected(
                "трек не сохранён (без подписки доступно только превью, или сбой сети)".into(),
            ))
        }
    }
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..2u64)
        .map(|i| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(nanos());
            format!("{:016x}", h.finish())
        })
        .collect()
}

// ---- HTTP ----

struct Request {
    method: String,
    path: String,
    range: Option<String>,
}

async fn read_head(sock: &mut TcpStream) -> Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Err(Error::Unexpected("соединение закрыто до заголовков".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(end);
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        if buf.len() > MAX_HEAD_BYTES {
            return Err(Error::Unexpected("слишком длинные заголовки".into()));
        }
    }
}

fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let range = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("range")
            .then(|| value.trim().to_owned())
    });
    Some(Request { method, path, range })
}

/// `bytes=S-` или `bytes=S-E` → (S, Some(E)); остальное не поддерживается.
fn parse_range(range: &str) -> Option<(u64, Option<u64>)> {
    let spec = range.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        e => Some(e.parse().ok()?),
    };
    Some((start, end))
}

async fn respond_status(sock: &mut TcpStream, code: u16, reason: &str) -> Result<()> {
    let head = format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    sock.write_all(head.as_bytes()).await?;
    Ok(())
}

async fn handle(mut sock: TcpStream, shared: Arc<Shared>) -> Result<()> {
    let head = read_head(&mut sock).await?;
    let Some(req) = parse_request(&head) else {
        return respond_status(&mut sock, 400, "Bad Request").await;
    };
    let id = match req.path.strip_prefix('/').and_then(|p| p.split_once('/')) {
        Some((t, id)) if t == shared.token && is_safe_id(id) => id.to_owned(),
        _ => return respond_status(&mut sock, 404, "Not Found").await,
    };
    if req.method != "GET" && req.method != "HEAD" {
        return respond_status(&mut sock, 405, "Method Not Allowed").await;
    }

    let Some(dl) = attach(&shared, &id).await else {
        return respond_status(&mut sock, 502, "Bad Gateway").await;
    };
    // Счётчик читателей; при уходе последнего — отложенная отмена загрузки.
    let _client = ClientGuard {
        dl: dl.clone(),
        grace: shared.grace,
    };
    serve(&mut sock, &dl, req.range.as_deref(), req.method == "HEAD").await
}

/// Найти идущую загрузку, файл в кэше или начать новую загрузку.
async fn attach(shared: &Arc<Shared>, id: &str) -> Option<Arc<Download>> {
    let mut downloads = shared.downloads.lock().await;
    if let Some(dl) = downloads.get(id) {
        dl.clients.fetch_add(1, Ordering::SeqCst);
        return Some(dl.clone());
    }
    if let Some((path, _)) = shared.cache.lookup(id) {
        return finished_file(path).await;
    }

    let resolved = (shared.resolver)(id.to_owned()).await.ok()?;
    let resp = shared.http.get(&resolved.stream.url).send().await.ok()?;
    if resp.status() != reqwest::StatusCode::OK {
        return None;
    }
    let total = resp.content_length();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_owned();
    let path = shared.cache.part_path(&format!("{id}-{}", nanos()));
    let file = tokio::fs::File::create(&path).await.ok()?;

    let (tx, rx) = watch::channel(Progress::default());
    let dl = Arc::new(Download {
        path,
        total,
        content_type,
        progress: rx,
        clients: AtomicUsize::new(1),
        cancelled: AtomicBool::new(false),
        pinned: AtomicBool::new(false),
    });
    downloads.insert(id.to_owned(), dl.clone());
    tokio::spawn(write_task(shared.clone(), id.to_owned(), dl.clone(), resp, file, tx, resolved));
    Some(dl)
}

/// Уже докачанный файл из кэша — как «завершённая загрузка».
async fn finished_file(path: PathBuf) -> Option<Arc<Download>> {
    let len = tokio::fs::metadata(&path).await.ok()?.len();
    let (_tx, rx) = watch::channel(Progress {
        written: len,
        finished: true,
        failed: false,
    });
    Some(Arc::new(Download {
        content_type: content_type_for(&path).to_owned(),
        path,
        total: Some(len),
        progress: rx,
        clients: AtomicUsize::new(1),
        cancelled: AtomicBool::new(false),
        pinned: AtomicBool::new(false),
    }))
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        _ => "audio/mpeg",
    }
}

async fn write_task(
    shared: Arc<Shared>,
    id: String,
    dl: Arc<Download>,
    mut resp: reqwest::Response,
    mut file: tokio::fs::File,
    tx: watch::Sender<Progress>,
    resolved: ResolvedTrack,
) {
    let info = &resolved.stream;
    let mut ok = true;
    loop {
        if dl.cancelled.load(Ordering::SeqCst) {
            ok = false;
            break;
        }
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if file.write_all(&chunk).await.is_err() {
                    ok = false;
                    break;
                }
                // Читатели видят только записанное на диск.
                let _ = file.flush().await;
                tx.send_modify(|p| p.written += chunk.len() as u64);
            }
            Ok(None) => break,
            Err(_) => {
                ok = false;
                break;
            }
        }
    }
    let _ = file.flush().await;
    drop(file);
    let written = tx.borrow().written;
    let complete = ok && dl.total.is_none_or(|t| t == written);
    tx.send_modify(|p| {
        if complete {
            p.finished = true;
        } else {
            p.failed = true;
        }
    });

    let keep = complete && !info.is_preview;
    // Обложку качаем заранее, пока плеер ещё может читать файл.
    let cover = match (&resolved.meta, keep) {
        (Some(meta), true) => fetch_cover(&shared.http, meta).await,
        _ => None,
    };

    // Переносим в кэш (или удаляем), когда файл никто не читает.
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    loop {
        {
            let mut downloads = shared.downloads.lock().await;
            if dl.clients.load(Ordering::SeqCst) == 0 || Instant::now() > deadline {
                if let (true, Some(meta)) = (keep, &resolved.meta) {
                    // Теги не критичны: без них файл всё равно играет.
                    let _ = tags::write_tags(&dl.path, meta, cover.as_deref());
                }
                let committed = keep
                    && shared
                        .cache
                        .commit(
                            &id,
                            &dl.path,
                            &info.codec,
                            info.bitrate_kbps,
                            resolved.meta.clone(),
                            dl.pinned.load(Ordering::SeqCst),
                        )
                        .is_ok();
                if !committed {
                    let _ = tokio::fs::remove_file(&dl.path).await;
                }
                downloads.remove(&id);
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Обложка покрупнее для тегов (в списках — 100×100, тут 600×600).
pub async fn fetch_cover(http: &reqwest::Client, meta: &TrackMeta) -> Option<Vec<u8>> {
    let url = meta.cover_url.as_deref()?;
    let url = match url.rsplit_once('/') {
        Some((base, size)) if size.contains('x') => format!("{base}/600x600"),
        _ => url.to_owned(),
    };
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.bytes().await.ok().map(|b| b.to_vec())
}

struct ClientGuard {
    dl: Arc<Download>,
    grace: Duration,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if self.dl.clients.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        let p = *self.dl.progress.borrow();
        if p.finished || p.failed || self.dl.pinned.load(Ordering::SeqCst) {
            return;
        }
        // Последний читатель ушёл посреди загрузки: если за `grace` никто
        // не вернулся (перемотка переподключается быстрее), трек пропущен.
        let dl = self.dl.clone();
        let grace = self.grace;
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            if dl.clients.load(Ordering::SeqCst) == 0 && !dl.pinned.load(Ordering::SeqCst) {
                dl.cancelled.store(true, Ordering::SeqCst);
            }
        });
    }
}

/// Отдать данные из (растущего) файла загрузки с учётом диапазона.
async fn serve(sock: &mut TcpStream, dl: &Download, range: Option<&str>, head_only: bool) -> Result<()> {
    let requested = range.and_then(parse_range);
    let (status, start, end_excl) = match (requested, dl.total) {
        (Some((start, _)), Some(total)) if start >= total => {
            let head = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).await?;
            return Ok(());
        }
        (Some((start, end)), Some(total)) => {
            let end_excl = end.map_or(total, |e| (e + 1).min(total));
            (206, start, Some(end_excl))
        }
        // Без известной длины диапазоны не поддерживаем — отдаём всё с начала.
        (_, total) => (200, 0, total),
    };

    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        if status == 206 { "Partial Content" } else { "OK" },
        dl.content_type,
    );
    if let Some(end) = end_excl {
        head.push_str(&format!("Content-Length: {}\r\n", end - start));
    }
    if status == 206 {
        let total = dl.total.unwrap_or(0);
        head.push_str(&format!(
            "Content-Range: bytes {start}-{}/{total}\r\n",
            end_excl.unwrap_or(total).saturating_sub(1)
        ));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes()).await?;
    if head_only {
        return Ok(());
    }

    let mut file = tokio::fs::File::open(&dl.path).await?;
    file.seek(SeekFrom::Start(start)).await?;
    let end_excl = end_excl.unwrap_or(u64::MAX);
    let mut pos = start;
    let mut rx = dl.progress.clone();
    let mut buf = vec![0u8; 64 * 1024];
    while pos < end_excl {
        let p = *rx.borrow_and_update();
        if pos < p.written {
            let want = (p.written.min(end_excl) - pos).min(buf.len() as u64) as usize;
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            if sock.write_all(&buf[..n]).await.is_err() {
                return Ok(());
            }
            pos += n as u64;
            continue;
        }
        if p.finished || p.failed {
            break;
        }
        if rx.changed().await.is_err() {
            // Загрузка завершилась и отпустила канал — дочитываем остаток.
            if pos >= rx.borrow().written {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{CONTENT_RANGE, RANGE};

    const BODY_LEN: usize = 100_000;

    fn body() -> Vec<u8> {
        (0..BODY_LEN).map(|i| (i % 251) as u8).collect()
    }

    /// Поддельный CDN: отдаёт body(); путь `/slow` — медленно, десятью кусками.
    async fn fake_upstream() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let head = read_head(&mut sock).await.unwrap();
                    let req = parse_request(&head).unwrap();
                    let data = body();
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: audio/mpeg\r\n\r\n",
                        data.len()
                    );
                    sock.write_all(head.as_bytes()).await.unwrap();
                    if req.path == "/slow" {
                        for part in data.chunks(BODY_LEN / 10) {
                            if sock.write_all(part).await.is_err() {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    } else {
                        let _ = sock.write_all(&data).await;
                    }
                });
            }
        });
        port
    }

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("moth-proxy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    async fn setup(name: &str, preview: bool, path: &str) -> (Arc<Cache>, Proxy, PathBuf) {
        let port = fake_upstream().await;
        let url = format!("http://127.0.0.1:{port}{path}");
        let dir = temp_dir(name);
        let cache = Arc::new(Cache::open(&dir, 10_000_000).unwrap());
        cache.confirm_plus(true).unwrap();
        let resolver: Resolver = Arc::new(move |_id| {
            let url = url.clone();
            Box::pin(async move {
                Ok(ResolvedTrack {
                    stream: StreamInfo {
                        url,
                        codec: "mp3".into(),
                        bitrate_kbps: Some(320),
                        is_preview: preview,
                    },
                    meta: Some(TrackMeta {
                        source: "yandex".into(),
                        title: "Тест".into(),
                        ..Default::default()
                    }),
                })
            })
        });
        let proxy = Proxy::start_with_grace(
            cache.clone(),
            resolver,
            reqwest::Client::new(),
            Duration::from_millis(200),
        )
        .await
        .unwrap();
        (cache, proxy, dir)
    }

    async fn wait_cached(cache: &Cache, id: &str) -> bool {
        for _ in 0..100 {
            if cache.contains(id) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    fn part_files(dir: &Path) -> usize {
        std::fs::read_dir(dir.join("tracks"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "part"))
            .count()
    }

    #[tokio::test]
    async fn full_stream_is_served_and_cached() {
        let (cache, proxy, dir) = setup("full", false, "/t.mp3").await;
        let got = reqwest::get(proxy.url("42")).await.unwrap().bytes().await.unwrap();
        assert_eq!(got.as_ref(), body().as_slice());
        assert!(wait_cached(&cache, "42").await);
        let (path, entry) = cache.lookup("42").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body());
        assert_eq!(entry.size, BODY_LEN as u64);

        // Повторный запрос с диапазоном отдаётся уже из кэша.
        let resp = reqwest::Client::new()
            .get(proxy.url("42"))
            .header(RANGE, "bytes=10-19")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 206);
        assert_eq!(resp.bytes().await.unwrap().as_ref(), &body()[10..20]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn range_requests_share_one_download() {
        let (cache, proxy, dir) = setup("range", false, "/t.mp3").await;
        // Как mpv: начало, конец (теги), снова начало.
        let client = reqwest::Client::new();
        for (range, slice) in [("bytes=0-", 0..BODY_LEN), ("bytes=99000-", 99_000..BODY_LEN), ("bytes=500-", 500..BODY_LEN)] {
            let resp = client.get(proxy.url("43")).header(RANGE, range).send().await.unwrap();
            assert_eq!(resp.status(), 206);
            assert!(resp.headers().get(CONTENT_RANGE).is_some());
            assert_eq!(resp.bytes().await.unwrap().as_ref(), &body()[slice]);
        }
        assert!(wait_cached(&cache, "43").await);
        assert_eq!(part_files(&dir), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn preview_is_not_cached() {
        let (cache, proxy, dir) = setup("preview", true, "/t.mp3").await;
        let got = reqwest::get(proxy.url("44")).await.unwrap().bytes().await.unwrap();
        assert_eq!(got.len(), BODY_LEN);
        assert!(!wait_cached(&cache, "44").await);
        assert_eq!(part_files(&dir), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn skipped_track_download_is_cancelled() {
        let (cache, proxy, dir) = setup("skip", false, "/slow").await;
        let mut resp = reqwest::get(proxy.url("46")).await.unwrap();
        let first = resp.chunk().await.unwrap();
        assert!(first.is_some());
        drop(resp); // плеер ушёл с трека
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!cache.contains("46"));
        assert_eq!(part_files(&dir), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn manual_download_is_pinned_and_survives_skip() {
        let (cache, proxy, dir) = setup("manual", false, "/slow").await;
        // Плеер начал и ушёл с трека, но пользователь нажал «скачать».
        let mut resp = reqwest::get(proxy.url("47")).await.unwrap();
        let _ = resp.chunk().await.unwrap();
        let download = proxy.download("47");
        drop(resp);
        download.await.unwrap();
        let entry = cache
            .cached_tracks()
            .into_iter()
            .find(|(id, _)| id == "47")
            .unwrap()
            .1;
        assert!(entry.pinned);
        assert_eq!(entry.meta.unwrap().title, "Тест");
        assert_eq!(part_files(&dir), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn manual_download_of_preview_fails() {
        let (cache, proxy, dir) = setup("manual-preview", true, "/t.mp3").await;
        assert!(proxy.download("48").await.is_err());
        assert!(!cache.contains("48"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn wrong_token_or_bad_id_is_404() {
        let (_cache, proxy, dir) = setup("token", false, "/t.mp3").await;
        let wrong = proxy.url("45").replace(&proxy.token, "0000");
        assert_eq!(reqwest::get(wrong).await.unwrap().status(), 404);
        let bad = format!("http://127.0.0.1:{}/{}/..%2Fx", proxy.port, proxy.token);
        assert_eq!(reqwest::get(bad).await.unwrap().status(), 404);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn request_and_range_parsing() {
        let r = parse_request("GET /t/1 HTTP/1.1\r\nHost: x\r\nrange: bytes=10-").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/t/1");
        assert_eq!(r.range.as_deref(), Some("bytes=10-"));
        assert!(parse_request("").is_none());
        assert_eq!(parse_range("bytes=10-"), Some((10, None)));
        assert_eq!(parse_range("bytes=10-19"), Some((10, Some(19))));
        assert_eq!(parse_range("bytes=-500"), None);
        assert_eq!(parse_range("items=1-2"), None);
    }
}
