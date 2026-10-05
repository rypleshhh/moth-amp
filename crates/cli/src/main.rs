//! Консольный клиент для проверки ядра без UI.

use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use moth_core::auth::KeyringTokenStore;
use moth_core::model::{Quality, Track};
use moth_core::provider::Provider;
use moth_core::yandex::{ApiClient, YandexConfig, YandexProvider};

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

    let store = Arc::new(KeyringTokenStore::new("moth-amp", "yandex"));
    let api = ApiClient::new(YandexConfig::default(), store)?;
    let yandex = YandexProvider::new(api);

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
                println!("{:>8}  {}  [{count}]", p.key.id, p.title);
            }
        }
        Command::Playlist { kind } => print_tracks(yandex.playlist_tracks(&kind).await?.iter()),
        Command::Search { query } => print_tracks(yandex.search_tracks(&query).await?.iter()),
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
        println!("{mark} {:>10}  {} — {}  [{dur}]", t.key.id, t.artist_line(), t.full_title());
    }
}
