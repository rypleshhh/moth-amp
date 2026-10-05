//! «Моя волна» для Flutter. Одновременно активна одна сессия волны.

use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use moth_core::yandex::wave::Wave;
use tokio::sync::Mutex;

use super::yandex::{provider, run, track_dtos, TrackDto};

static WAVE: LazyLock<Mutex<Option<Wave>>> = LazyLock::new(|| Mutex::new(None));

/// Сбросить волну (например, при выходе из аккаунта).
pub(crate) async fn reset() {
    *WAVE.lock().await = None;
}

/// Запустить волну. `learning = false` — тихий режим: incognito, отчёты не отправляются.
pub async fn wave_start(learning: bool) -> Result<Vec<TrackDto>> {
    run(async move {
        let p = provider()?;
        let (wave, tracks) = Wave::start(p.api(), learning).await?;
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
