//! «Моя волна» для Flutter. Одновременно активна одна сессия волны.

use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use moth_core::yandex::wave::{self, Wave, WaveOption};
use tokio::sync::Mutex;

use super::yandex::{provider, run, track_dtos, TrackDto};

static WAVE: LazyLock<Mutex<Option<Wave>>> = LazyLock::new(|| Mutex::new(None));

/// Сбросить волну (например, при выходе из аккаунта).
pub(crate) async fn reset() {
    *WAVE.lock().await = None;
}

/// Вариант настройки волны.
pub struct WaveOptionDto {
    pub name: String,
    /// Зерно для `wave_start`.
    pub seed: String,
    /// Вариант «любое».
    pub is_default: bool,
}

pub struct WaveGroupDto {
    pub name: String,
    pub options: Vec<WaveOptionDto>,
}

/// Что можно настроить в волне: занятия и группы (настроение, характер, язык).
pub struct WaveSettingsDto {
    pub activities: Vec<WaveOptionDto>,
    pub groups: Vec<WaveGroupDto>,
}

fn option_dto(o: WaveOption) -> WaveOptionDto {
    WaveOptionDto {
        name: o.name,
        seed: o.seed,
        is_default: o.default,
    }
}

/// Варианты настроек волны (названия — от Яндекса).
pub async fn wave_settings() -> Result<WaveSettingsDto> {
    run(async {
        let s = wave::settings(provider()?.api()).await?;
        Ok(WaveSettingsDto {
            activities: s.activities.into_iter().map(option_dto).collect(),
            groups: s
                .groups
                .into_iter()
                .map(|g| WaveGroupDto {
                    name: g.name,
                    options: g.options.into_iter().map(option_dto).collect(),
                })
                .collect(),
        })
    })
    .await
}

/// Запустить волну. `learning = false` — тихий режим: incognito, отчёты не отправляются.
/// `seeds` — настройки (занятие, настроение…); пустой список — обычная волна.
pub async fn wave_start(learning: bool, seeds: Vec<String>) -> Result<Vec<TrackDto>> {
    run(async move {
        let p = provider()?;
        let (wave, tracks) = Wave::start(p.api(), learning, &seeds).await?;
        *WAVE.lock().await = Some(wave);
        Ok(track_dtos(tracks))
    })
    .await
}

/// Следующая партия треков текущей волны.
pub async fn wave_more() -> Result<Vec<TrackDto>> {
    run(async {
        let p = provider()?;
        let mut guard = WAVE.lock().await;
        let wave = guard.as_mut().ok_or_else(|| anyhow!("волна не запущена"))?;
        Ok(track_dtos(wave.more(p.api()).await?))
    })
    .await
}

pub async fn wave_track_started(track_id: String) -> Result<()> {
    run(async move {
        let p = provider()?;
        if let Some(wave) = WAVE.lock().await.as_mut() {
            wave.track_started(p.api(), &track_id).await?;
        }
        Ok(())
    })
    .await
}

/// `skipped = false` — трек дослушан до конца.
pub async fn wave_track_ended(track_id: String, played_secs: f64, skipped: bool) -> Result<()> {
    run(async move {
        let p = provider()?;
        if let Some(wave) = WAVE.lock().await.as_mut() {
            wave.track_ended(p.api(), &track_id, played_secs, skipped)
                .await?;
        }
        Ok(())
    })
    .await
}

pub async fn wave_stop() -> Result<()> {
    run(async {
        reset().await;
        Ok(())
    })
    .await
}
