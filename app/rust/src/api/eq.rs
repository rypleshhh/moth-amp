//! Эквалайзер для Flutter: настройки, пресеты, AutoEq и строка фильтров mpv.

use std::path::Path;

use anyhow::Result;
use moth_core::auth::now_unix;
use moth_core::dsp::{self, Band, EqMode, EqSettings, EqStore, FilterKind, NamedPreset};

pub enum EqModeDto {
    Graphic10,
    Graphic18,
    Parametric,
}

pub enum FilterKindDto {
    Peaking,
    LowShelf,
    HighShelf,
}

pub struct EqBandDto {
    pub kind: FilterKindDto,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

pub struct EqSettingsDto {
    pub enabled: bool,
    pub mode: EqModeDto,
    pub preamp_db: f32,
    pub bands: Vec<EqBandDto>,
}

pub struct EqPresetDto {
    pub name: String,
    pub settings: EqSettingsDto,
}

pub struct EqStateDto {
    pub current: EqSettingsDto,
    pub user_presets: Vec<EqPresetDto>,
}

// ---- преобразования ----

fn mode_from(m: EqModeDto) -> EqMode {
    match m {
        EqModeDto::Graphic10 => EqMode::Graphic10,
        EqModeDto::Graphic18 => EqMode::Graphic18,
        EqModeDto::Parametric => EqMode::Parametric,
    }
}

fn mode_to(m: EqMode) -> EqModeDto {
    match m {
        EqMode::Graphic10 => EqModeDto::Graphic10,
        EqMode::Graphic18 => EqModeDto::Graphic18,
        EqMode::Parametric => EqModeDto::Parametric,
    }
}

fn settings_from(s: EqSettingsDto) -> EqSettings {
    EqSettings {
        enabled: s.enabled,
        mode: mode_from(s.mode),
        preamp_db: s.preamp_db,
        bands: s
            .bands
            .into_iter()
            .map(|b| Band {
                kind: match b.kind {
                    FilterKindDto::Peaking => FilterKind::Peaking,
                    FilterKindDto::LowShelf => FilterKind::LowShelf,
                    FilterKindDto::HighShelf => FilterKind::HighShelf,
                },
                freq_hz: b.freq_hz,
                gain_db: b.gain_db,
                q: b.q,
            })
            .collect(),
    }
}

fn settings_to(s: EqSettings) -> EqSettingsDto {
    EqSettingsDto {
        enabled: s.enabled,
        mode: mode_to(s.mode),
        preamp_db: s.preamp_db,
        bands: s
            .bands
            .into_iter()
            .map(|b| EqBandDto {
                kind: match b.kind {
                    FilterKind::Peaking => FilterKindDto::Peaking,
                    FilterKind::LowShelf => FilterKindDto::LowShelf,
                    FilterKind::HighShelf => FilterKindDto::HighShelf,
                },
                freq_hz: b.freq_hz,
                gain_db: b.gain_db,
                q: b.q,
            })
            .collect(),
    }
}

fn preset_to(p: NamedPreset) -> EqPresetDto {
    EqPresetDto {
        name: p.name,
        settings: settings_to(p.settings),
    }
}

// ---- API ----

#[flutter_rust_bridge::frb(sync)]
pub fn eq_flat(mode: EqModeDto) -> EqSettingsDto {
    settings_to(EqSettings::flat(mode_from(mode)))
}

#[flutter_rust_bridge::frb(sync)]
pub fn eq_with_mode(settings: EqSettingsDto, mode: EqModeDto) -> EqSettingsDto {
    settings_to(settings_from(settings).with_mode(mode_from(mode)))
}

/// Строка для свойства mpv `af`; пустая — обработка выключена.
#[flutter_rust_bridge::frb(sync)]
pub fn eq_to_filter(settings: EqSettingsDto) -> String {
    settings_from(settings).to_mpv_af()
}

/// Предусилитель для громкости плеера (0, если эквалайзер выключен).
#[flutter_rust_bridge::frb(sync)]
pub fn eq_effective_preamp(settings: EqSettingsDto) -> f32 {
    settings_from(settings).effective_preamp_db()
}

#[flutter_rust_bridge::frb(sync)]
pub fn eq_auto_preamp(settings: EqSettingsDto) -> f32 {
    settings_from(settings).auto_preamp_db()
}

#[flutter_rust_bridge::frb(sync)]
pub fn eq_builtin_presets(mode: EqModeDto) -> Vec<EqPresetDto> {
    dsp::builtin_presets(mode_from(mode))
        .into_iter()
        .map(preset_to)
        .collect()
}

/// Импорт пресета AutoEq / Equalizer APO (текст файла `ParametricEQ.txt`).
#[flutter_rust_bridge::frb(sync)]
pub fn eq_parse_autoeq(text: String) -> Result<EqSettingsDto> {
    Ok(settings_to(dsp::parse_autoeq(&text)?))
}

fn state_to(store: EqStore) -> EqStateDto {
    EqStateDto {
        current: settings_to(
            store
                .current
                .unwrap_or_else(|| EqSettings::flat(EqMode::Graphic10)),
        ),
        user_presets: store.presets.into_iter().map(preset_to).collect(),
    }
}

pub fn eq_load(path: String) -> Result<EqStateDto> {
    Ok(state_to(dsp::load_store(Path::new(&path))?))
}

/// Настройки с учётом S3: берётся более свежая копия (локальная или из
/// бакета), и она же записывается на другую сторону. Без S3 — только локально.
pub async fn eq_sync(path: String) -> Result<EqStateDto> {
    super::yandex::run(async move {
        let local = dsp::load_store(Path::new(&path))?;
        let Some(lib) = super::s3::library()? else {
            return Ok(state_to(local));
        };
        let remote: Option<EqStore> = match lib.get_setting(EQ_SETTING).await? {
            Some(bytes) => serde_json::from_slice(&bytes).ok(),
            None => None,
        };
        let store = match remote {
            Some(remote) if remote.updated_at > local.updated_at => {
                dsp::save_store(Path::new(&path), &remote)?;
                remote
            }
            _ => {
                if local.updated_at > 0 {
                    lib.put_setting(EQ_SETTING, serde_json::to_vec_pretty(&local)?)
                        .await?;
                }
                local
            }
        };
        Ok(state_to(store))
    })
    .await
}

const EQ_SETTING: &str = "equalizer";

/// Сохранить локально и (если подключено) в S3. Сбой S3 не мешает
/// локальному сохранению.
pub async fn eq_save(path: String, state: EqStateDto) -> Result<()> {
    let store = EqStore {
        current: Some(settings_from(state.current)),
        presets: state
            .user_presets
            .into_iter()
            .map(|p| NamedPreset {
                name: p.name,
                settings: settings_from(p.settings),
            })
            .collect(),
        updated_at: now_unix(),
    };
    dsp::save_store(Path::new(&path), &store)?;
    super::yandex::run(async move {
        if let Ok(Some(lib)) = super::s3::library() {
            if let Ok(json) = serde_json::to_vec_pretty(&store) {
                let _ = lib.put_setting(EQ_SETTING, json).await;
            }
        }
        Ok(())
    })
    .await
}

/// Кривая АЧХ (дБ) в `points` точках от 20 Гц до 20 кГц — для дисплея плеера.
#[flutter_rust_bridge::frb(sync)]
pub fn eq_response(settings: EqSettingsDto, points: u32) -> Vec<f32> {
    let freqs = dsp::log_frequencies(points as usize);
    settings_from(settings).response_db(&freqs)
}
