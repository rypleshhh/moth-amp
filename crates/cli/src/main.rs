//! Консольный клиент для проверки ядра без UI.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use moth_core::cache::proxy::{Proxy, ResolvedTrack, Resolver};
use moth_core::cache::{tags, Cache};
use moth_core::model::{Quality, Track, TrackMeta};
use moth_core::secrets::{KeyringSecretStore, SecretTokenStore};
use moth_core::yandex::wave::Wave;
use moth_core::yandex::{ApiClient, YandexProvider};

#[derive(Parser)]
#[command(name = "moth", about = "moth-amp: консольная проверка ядра")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Вход в аккаунт Яндекса по коду подтверждения
    Login,
    /// Выход: удалить токен из системного хранилища
    Logout,
    /// Данные аккаунта и наличие Плюса
    Status,
    /// Лайкнутые треки
    Likes {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Список плейлистов
    Playlists,
    /// Треки плейлиста
    Playlist { kind: String },
    /// Поиск треков
    Search { query: String },
    /// «Моя волна»: первые партии треков (по умолчанию тихий режим, без отчётов)
    Wave {
        /// Обучаемый режим: отправлять отчёты, волна подстраивается
        #[arg(long)]
        learning: bool,
        /// Сколько партий получить
        #[arg(long, default_value_t = 2)]
        batches: usize,
    },
    /// Скачать трек в кэш с тегами (проверка загрузки без UI)
    Download {
        track_id: String,
        /// Папка кэша (по умолчанию — временная)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Прямая ссылка на поток трека
    Url {
        track_id: String,
        #[arg(long, value_enum, default_value_t = QualityArg::High)]
        quality: QualityArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Low,
    High,
}

impl From<QualityArg> for Quality {
    fn from(q: QualityArg) -> Self {
        match q {
            QualityArg::Low => Quality::Low,
            QualityArg::High => Quality::High,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // То же хранилище, что у приложения: войти можно один раз.
    let store = Arc::new(SecretTokenStore::new(
        Arc::new(KeyringSecretStore::new("moth-amp")),
        "yandex",
    ));
    let api = ApiClient::new(store)?;
    let yandex = Arc::new(YandexProvider::new(api));

    match cli.command {
        Command::Login => login(&yandex).await?,
        Command::Logout => {
            yandex.api().logout().await?;
            println!("Токен удалён.");
        }
        Command::Status => {
            let acc = yandex.account().await?;
            println!("uid:      {}", acc.uid);
            if let Some(login) = &acc.login {
                println!("логин:    {login}");
            }
            println!("Плюс:     {}", if acc.has_plus { "есть" } else { "нет" });
        }
        Command::Likes { limit } => {
            let tracks = yandex.liked_tracks().await?;
            println!("Лайков: {}", tracks.len());
            print_tracks(tracks.iter().take(limit));
        }
        Command::Playlists => {
            for p in yandex.playlists().await? {
                let count = p.track_count.map(|c| c.to_string()).unwrap_or_default();
                println!("{:>8}  {}  [{count}]", p.id, p.title);
            }
        }
        Command::Playlist { kind } => {
            print_tracks(yandex.playlist_tracks(&kind, None).await?.iter())
        }
        Command::Search { query } => print_tracks(yandex.search_all(&query).await?.tracks.iter()),
        Command::Wave { learning, batches } => {
            let (mut wave, first) = Wave::start(yandex.api(), learning, &[]).await?;
            println!("Режим: {}", if learning { "обучаемый" } else { "тихий (incognito)" });
            print_tracks(first.iter());
            for _ in 1..batches {
                print_tracks(wave.more(yandex.api()).await?.iter());
            }
        }
        Command::Download { track_id, dir } => {
            let dir = dir.unwrap_or_else(|| std::env::temp_dir().join("moth-cli-cache"));
            let cache = Arc::new(Cache::open(&dir, 2048 * 1024 * 1024)?);
            cache.confirm_plus(yandex.account().await?.has_plus)?;

            let t = yandex
                .api()
                .tracks(std::slice::from_ref(&track_id))
                .await?
                .into_iter()
                .next()
                .context("трек не найден")?;
            let meta = TrackMeta::from_track(&t);

            let provider = yandex.clone();
            let resolver: Resolver = Arc::new(move |id: String| {
                let provider = provider.clone();
                let meta = meta.clone();
                Box::pin(async move {
                    Ok(ResolvedTrack {
                        stream: provider.stream(&id, Quality::High).await?,
                        meta: Some(meta),
                    })
                })
            });
            let proxy = Proxy::start(cache.clone(), resolver, reqwest::Client::new()).await?;
            proxy.download(&track_id).await?;

            let (path, entry) = cache.lookup(&track_id).context("трек не попал в кэш")?;
            let info = tags::read_file_info(&path)?;
            println!("файл:   {} ({:.2} МБ, {})", path.display(), entry.size as f64 / 1048576.0, entry.codec);
            println!("теги:   {} — {}", info.artists.join(", "), info.title.unwrap_or_default());
            println!("обложка: {}", if info.cover.is_some() { "есть" } else { "нет" });
        }
        Command::Url { track_id, quality } => {
            let s = yandex.stream(&track_id, quality.into()).await?;
            let kbps = s.bitrate_kbps.map(|b| format!(" {b} кбит/с")).unwrap_or_default();
            eprintln!(
                "{}{kbps}{}",
                s.codec,
                if s.is_preview { " (превью 30 с, нет Плюса)" } else { "" }
            );
            println!("{}", s.url);
        }
    }
    Ok(())
}

async fn login(yandex: &YandexProvider) -> Result<()> {
    let oauth = yandex.api().oauth();
    let code = oauth.request_device_code().await?;
    println!("Откройте {} и введите код: {}", code.verification_url, code.user_code);
    println!("Код действует {} мин. Жду подтверждения…", code.expires_in / 60);

    let tokens = oauth.wait_for_token(&code).await?;
    yandex.api().set_tokens(tokens).await?;
    println!("Вход подтверждён, токен сохранён.");

    let acc = match yandex.account().await {
        Ok(acc) => acc,
        Err(e) => {
            println!("Не удалось получить данные аккаунта: {e}");
            println!("Проверьте сеть и повторите `moth status`; входить заново не нужно.");
            return Ok(());
        }
    };
    let name = acc.display_name.as_deref().or(acc.login.as_deref()).unwrap_or(&acc.uid);
    println!("Вход выполнен: {name}. Плюс: {}", if acc.has_plus { "есть" } else { "нет" });
    Ok(())
}

fn print_tracks<'a>(tracks: impl Iterator<Item = &'a Track>) {
    for t in tracks {
        let dur = t
            .duration_ms
            .map(|ms| format!("{}:{:02}", ms / 60_000, (ms / 1000) % 60))
            .unwrap_or_default();
        let mark = if t.available { " " } else { "✗" };
        println!("{mark} {:>10}  {} — {}  [{dur}]", t.id, t.artist_line(), t.full_title());
    }
}
