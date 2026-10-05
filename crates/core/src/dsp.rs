//! Эквалайзер и предусилитель: настройки → цепочка фильтров FFmpeg для
//! свойства `af` в mpv, импорт пресетов AutoEq и хранение на диске.
//!
//! Ограничения libmpv из media_kit (проверено на сборке 2023-09-24): в его
//! FFmpeg из звуковых фильтров есть только `equalizer`, нет `aresample`,
//! `volume`, `lowshelf`/`highshelf`. Поэтому:
//! - перед графом стоит встроенный фильтр mpv `format=format=floatp`, чтобы
//!   FFmpeg не пытался вставить отсутствующий `aresample`;
//! - полки имитируются широким пиковым фильтром (ширина в октавах);
//! - предусилитель применяется не фильтром, а громкостью плеера
//!   ([`EqSettings::effective_preamp_db`]).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub kind: FilterKind,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EqMode {
    Graphic10,
    Graphic18,
    Parametric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqSettings {
    pub enabled: bool,
    pub mode: EqMode,
    pub preamp_db: f32,
    pub bands: Vec<Band>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedPreset {
    pub name: String,
    pub settings: EqSettings,
}

/// Октавные полосы, как в классических 10-полосных эквалайзерах.
pub const GRAPHIC_10_HZ: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Полосы через пол-октавы.
pub const GRAPHIC_18_HZ: [f32; 18] = [
    45.0, 63.0, 90.0, 125.0, 180.0, 250.0, 355.0, 500.0, 710.0, 1000.0, 1400.0, 2000.0,
    2800.0, 4000.0, 5600.0, 8000.0, 11200.0, 16000.0,
];

/// Добротность полосы шириной в октаву и в пол-октавы.
const Q_OCTAVE: f32 = 1.414;
const Q_HALF_OCTAVE: f32 = 2.871;

pub const GAIN_LIMIT_DB: f32 = 24.0;
pub const MAX_PARAMETRIC_BANDS: usize = 20;

/// Порог, ниже которого усиление считается нулевым и фильтр не добавляется.
const NEGLIGIBLE_DB: f32 = 0.01;

impl EqMode {
    /// Частоты и добротность графического режима; `None` для параметрического.
    pub fn graphic_layout(self) -> Option<(&'static [f32], f32)> {
        match self {
            EqMode::Graphic10 => Some((&GRAPHIC_10_HZ, Q_OCTAVE)),
            EqMode::Graphic18 => Some((&GRAPHIC_18_HZ, Q_HALF_OCTAVE)),
            EqMode::Parametric => None,
        }
    }
}

impl EqSettings {
    /// Ровная АЧХ в заданном режиме.
    pub fn flat(mode: EqMode) -> Self {
        let bands = match mode.graphic_layout() {
            Some((freqs, q)) => freqs
                .iter()
                .map(|&freq_hz| Band {
                    kind: FilterKind::Peaking,
                    freq_hz,
                    gain_db: 0.0,
                    q,
                })
                .collect(),
            None => Vec::new(),
        };
        Self {
            enabled: true,
            mode,
            preamp_db: 0.0,
            bands,
        }
    }

    /// Графический пресет из кривой на 10 октавных полос.
    pub fn from_curve10(mode: EqMode, gains: &[f32; 10]) -> Self {
        let mut s = Self::flat(EqMode::Graphic10);
        for (band, &g) in s.bands.iter_mut().zip(gains) {
            band.gain_db = g;
        }
        s.preamp_db = s.auto_preamp_db();
        s.with_mode(mode)
    }

    /// Ничего не меняет в звуке: фильтры можно не включать вовсе.
    pub fn is_neutral(&self) -> bool {
        !self.enabled
            || (self.preamp_db.abs() < NEGLIGIBLE_DB
                && self.bands.iter().all(|b| b.gain_db.abs() < NEGLIGIBLE_DB))
    }

    /// Предусилитель, компенсирующий самый сильный подъём, чтобы не было перегрузки.
    pub fn auto_preamp_db(&self) -> f32 {
        let max_boost = self
            .bands
            .iter()
            .map(|b| b.gain_db)
            .fold(0.0_f32, f32::max);
        if max_boost > 0.0 {
            -max_boost
        } else {
            0.0
        }
    }

    /// Строка для свойства mpv `af` (только полосы, без предусилителя).
    /// Пустая строка отключает обработку, и mpv не тратит на неё процессор.
    pub fn to_mpv_af(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        let filters: Vec<String> = self.bands.iter().filter_map(band_filter).collect();
        if filters.is_empty() {
            return String::new();
        }
        format!("format=format=floatp,lavfi=[{}]", filters.join(","))
    }

    /// Итоговая АЧХ (полосы + предусилитель) в дБ на заданных частотах —
    /// то, что реально делает эквалайзер со звуком.
    pub fn response_db(&self, freqs: &[f32]) -> Vec<f32> {
        if !self.enabled {
            return vec![0.0; freqs.len()];
        }
        let filters: Vec<EffectiveFilter> = self.bands.iter().filter_map(effective_filter).collect();
        let preamp = self.effective_preamp_db();
        freqs
            .iter()
            .map(|&f| preamp + filters.iter().map(|flt| flt.response_db(f)).sum::<f32>())
            .collect()
    }

    /// Предусилитель, который нужно применить громкостью плеера.
    pub fn effective_preamp_db(&self) -> f32 {
        if self.enabled {
            clamp_gain(self.preamp_db)
        } else {
            0.0
        }
    }

    /// Переключение режима с сохранением формы кривой там, где это возможно.
    pub fn with_mode(&self, mode: EqMode) -> Self {
        if mode == self.mode {
            return self.clone();
        }
        let mut out = Self::flat(mode);
        out.enabled = self.enabled;
        out.preamp_db = self.preamp_db;

        match (self.mode.graphic_layout(), mode.graphic_layout()) {
            // Графический → графический: интерполяция по логарифму частоты.
            (Some(_), Some(_)) => {
                let points: Vec<(f32, f32)> =
                    self.bands.iter().map(|b| (b.freq_hz, b.gain_db)).collect();
                for band in &mut out.bands {
                    band.gain_db = interpolate_log(&points, band.freq_hz);
                }
            }
            // Графический → параметрический: те же полосы как пиковые фильтры.
            (Some(_), None) => {
                out.bands = self
                    .bands
                    .iter()
                    .copied()
                    .filter(|b| b.gain_db.abs() >= NEGLIGIBLE_DB)
                    .collect();
            }
            // Параметрический → графический: кривая не переносится однозначно.
            (None, Some(_)) => out.preamp_db = 0.0,
            (None, None) => unreachable!("режимы различаются"),
        }
        out
    }
}

fn clamp_gain(g: f32) -> f32 {
    if g.is_finite() {
        g.clamp(-GAIN_LIMIT_DB, GAIN_LIMIT_DB)
    } else {
        0.0
    }
}

/// Ширина пикового фильтра так, как её понимает `equalizer` в FFmpeg.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Width {
    Q(f32),
    Octaves(f32),
}

/// Фильтр в том виде, в каком он реально применяется в mpv. Из него строится
/// и строка `af`, и кривая АЧХ для экрана — они всегда совпадают.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EffectiveFilter {
    freq: f32,
    width: Width,
    gain: f32,
}

impl EffectiveFilter {
    fn to_af(self) -> String {
        let (freq, gain) = (self.freq, self.gain);
        match self.width {
            Width::Q(q) => format!("equalizer=f={freq:.1}:t=q:w={q:.3}:g={gain:.2}"),
            Width::Octaves(o) => format!("equalizer=f={freq:.1}:t=o:w={o:.2}:g={gain:.2}"),
        }
    }

    /// Усиление в дБ на частоте `f` (пиковый фильтр RBJ, как в FFmpeg).
    fn response_db(self, f: f32) -> f32 {
        use std::f64::consts::{LN_2, PI};
        let fs = RESPONSE_SAMPLE_RATE;
        let w0 = 2.0 * PI * f64::from(self.freq) / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = match self.width {
            Width::Q(q) => sin / (2.0 * f64::from(q)),
            Width::Octaves(bw) => sin * (LN_2 / 2.0 * f64::from(bw) * w0 / sin).sinh(),
        };
        let a = 10f64.powf(f64::from(self.gain) / 40.0);
        let (b0, b1, b2) = (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a);
        let (a0, a1, a2) = (1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a);

        // |H(e^jw)| = |b0 + b1 z^-1 + b2 z^-2| / |a0 + a1 z^-1 + a2 z^-2|
        let w = 2.0 * PI * f64::from(f) / fs;
        let mag = |c0: f64, c1: f64, c2: f64| {
            let re = c0 + c1 * w.cos() + c2 * (2.0 * w).cos();
            let im = -(c1 * w.sin() + c2 * (2.0 * w).sin());
            (re * re + im * im).sqrt()
        };
        (20.0 * (mag(b0, b1, b2) / mag(a0, a1, a2)).log10()) as f32
    }
}

/// Частота дискретизации для расчёта кривой (типичный вывод mpv).
const RESPONSE_SAMPLE_RATE: f64 = 48_000.0;

fn effective_filter(b: &Band) -> Option<EffectiveFilter> {
    let gain = clamp_gain(b.gain_db);
    if gain.abs() < NEGLIGIBLE_DB || !b.freq_hz.is_finite() || !b.q.is_finite() {
        return None;
    }
    let freq = b.freq_hz.clamp(10.0, 22000.0);
    match b.kind {
        FilterKind::Peaking => Some(EffectiveFilter {
            freq,
            width: Width::Q(b.q.clamp(0.1, 20.0)),
            gain,
        }),
        // Полка ≈ широкий пик, накрывающий диапазон от частоты среза до края
        // слышимого диапазона.
        FilterKind::LowShelf => shelf_as_peak(AUDIBLE_MIN_HZ, freq, gain),
        FilterKind::HighShelf => shelf_as_peak(freq, AUDIBLE_MAX_HZ, gain),
    }
}

fn band_filter(b: &Band) -> Option<String> {
    effective_filter(b).map(EffectiveFilter::to_af)
}

const AUDIBLE_MIN_HZ: f32 = 20.0;
const AUDIBLE_MAX_HZ: f32 = 20000.0;

fn shelf_as_peak(lo: f32, hi: f32, gain: f32) -> Option<EffectiveFilter> {
    if hi <= lo {
        return None;
    }
    Some(EffectiveFilter {
        freq: (lo * hi).sqrt(),
        width: Width::Octaves((hi / lo).log2().clamp(1.0, 6.0)),
        gain,
    })
}

/// `n` частот от 20 Гц до 20 кГц равномерно по логарифмической шкале.
pub fn log_frequencies(n: usize) -> Vec<f32> {
    let (lo, hi) = (AUDIBLE_MIN_HZ.ln(), AUDIBLE_MAX_HZ.ln());
    (0..n)
        .map(|i| {
            let t = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
            (lo + (hi - lo) * t).exp()
        })
        .collect()
}

/// Линейная интерполяция усиления по log2(частоты); за краями — крайние значения.
fn interpolate_log(points: &[(f32, f32)], freq: f32) -> f32 {
    let Some(&(first_f, first_g)) = points.first() else {
        return 0.0;
    };
    let &(last_f, last_g) = points.last().unwrap();
    if freq <= first_f {
        return first_g;
    }
    if freq >= last_f {
        return last_g;
    }
    for w in points.windows(2) {
        let ((f0, g0), (f1, g1)) = (w[0], w[1]);
        if freq >= f0 && freq <= f1 {
            let t = (freq.log2() - f0.log2()) / (f1.log2() - f0.log2());
            return g0 + (g1 - g0) * t;
        }
    }
    last_g
}

// ---- встроенные пресеты ----

const BUILTIN_CURVES: [(&str, [f32; 10]); 7] = [
    ("Ровно", [0.0; 10]),
    ("Больше баса", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Больше верхов", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Вокал", [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0]),
    ("Тихое прослушивание", [5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 0.0, 1.0, 3.0, 4.0]),
    ("Электроника", [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.0, 1.0, 4.0, 5.0]),
    ("Рок", [4.0, 3.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 4.0, 4.0]),
];

pub fn builtin_presets(mode: EqMode) -> Vec<NamedPreset> {
    BUILTIN_CURVES
        .iter()
        .map(|(name, curve)| NamedPreset {
            name: (*name).to_owned(),
            settings: EqSettings::from_curve10(mode, curve),
        })
        .collect()
}

// ---- импорт AutoEq / Equalizer APO ----

/// Разбор текстового пресета в формате Equalizer APO (его же выдаёт AutoEq,
/// файл `… ParametricEQ.txt`):
///
/// ```text
/// Preamp: -6.2 dB
/// Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
/// Filter 2: ON PK Fc 210 Hz Gain -3.1 dB Q 0.88
/// ```
///
/// Неподдерживаемые типы фильтров (ФНЧ, ФВЧ и т.п.) пропускаются.
pub fn parse_autoeq(text: &str) -> Result<EqSettings> {
    let mut settings = EqSettings::flat(EqMode::Parametric);

    for line in text.lines() {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("preamp:") {
            settings.preamp_db = first_number(rest)
                .ok_or_else(|| Error::InvalidPreset(format!("не разобран предусилитель: {line}")))?;
            continue;
        }
        if !lower.starts_with("filter") {
            continue;
        }
        if let Some(band) = parse_filter_line(line)? {
            if settings.bands.len() >= MAX_PARAMETRIC_BANDS {
                return Err(Error::InvalidPreset(format!(
                    "слишком много полос (больше {MAX_PARAMETRIC_BANDS})"
                )));
            }
            settings.bands.push(band);
        }
    }

    if settings.bands.is_empty() {
        return Err(Error::InvalidPreset(
            "не найдено ни одной полосы (ожидается формат Equalizer APO / AutoEq)".into(),
        ));
    }
    Ok(settings)
}

fn first_number(s: &str) -> Option<f32> {
    s.split_whitespace().find_map(|t| t.parse().ok())
}

fn parse_filter_line(line: &str) -> Result<Option<Band>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let Some(state_idx) = tokens
        .iter()
        .position(|t| t.eq_ignore_ascii_case("on") || t.eq_ignore_ascii_case("off"))
    else {
        return Ok(None);
    };
    if tokens[state_idx].eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    let Some(kind_token) = tokens.get(state_idx + 1) else {
        return Ok(None);
    };
    let kind = match kind_token.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" | "MODAL" => FilterKind::Peaking,
        "LS" | "LSC" | "LSQ" => FilterKind::LowShelf,
        "HS" | "HSC" | "HSQ" => FilterKind::HighShelf,
        _ => return Ok(None),
    };

    let value_after = |key: &str| {
        tokens
            .iter()
            .position(|t| t.eq_ignore_ascii_case(key))
            .and_then(|i| tokens.get(i + 1))
            .and_then(|v| v.parse::<f32>().ok())
    };
    let freq_hz = value_after("fc")
        .ok_or_else(|| Error::InvalidPreset(format!("нет частоты Fc: {line}")))?;
    let gain_db = value_after("gain").unwrap_or(0.0);
    // У полок без Q в Equalizer APO наклон по умолчанию соответствует Q ≈ 0.71.
    let q = value_after("q").unwrap_or(0.707);

    Ok(Some(Band {
        kind,
        freq_hz,
        gain_db,
        q,
    }))
}

// ---- хранение ----

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EqStore {
    pub current: Option<EqSettings>,
    #[serde(default)]
    pub presets: Vec<NamedPreset>,
    /// Unix-время последнего изменения (синхронизация между устройствами).
    #[serde(default)]
    pub updated_at: u64,
}

/// Читает настройки; отсутствующий файл — это пустые настройки.
pub fn load_store(path: &Path) -> Result<EqStore> {
    match fs::read_to_string(path) {
        Ok(json) => Ok(serde_json::from_str(&json)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EqStore::default()),
        Err(e) => Err(e.into()),
    }
}

/// Пишет через временный файл, чтобы сбой посреди записи не испортил настройки.
pub fn save_store(path: &Path, store: &EqStore) -> Result<()> {
    crate::fsutil::write_atomic(path, &serde_json::to_vec_pretty(store)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_layouts() {
        assert_eq!(EqSettings::flat(EqMode::Graphic10).bands.len(), 10);
        assert_eq!(EqSettings::flat(EqMode::Graphic18).bands.len(), 18);
        assert!(EqSettings::flat(EqMode::Parametric).bands.is_empty());
    }

    #[test]
    fn neutral_gives_empty_filter() {
        assert_eq!(EqSettings::flat(EqMode::Graphic10).to_mpv_af(), "");
        let mut s = EqSettings::flat(EqMode::Graphic10);
        s.bands[0].gain_db = 6.0;
        s.enabled = false;
        assert_eq!(s.to_mpv_af(), "");
    }

    #[test]
    fn filter_string() {
        let mut s = EqSettings::flat(EqMode::Graphic10);
        s.preamp_db = -3.0;
        s.bands[0].gain_db = 3.0;
        s.bands[9].gain_db = -2.5;
        assert_eq!(
            s.to_mpv_af(),
            "format=format=floatp,lavfi=[equalizer=f=31.0:t=q:w=1.414:g=3.00,equalizer=f=16000.0:t=q:w=1.414:g=-2.50]"
        );
        assert_eq!(s.effective_preamp_db(), -3.0);
    }

    #[test]
    fn preamp_only_has_no_filter() {
        let mut s = EqSettings::flat(EqMode::Graphic10);
        s.preamp_db = -4.0;
        assert_eq!(s.to_mpv_af(), "");
        assert_eq!(s.effective_preamp_db(), -4.0);
        s.enabled = false;
        assert_eq!(s.effective_preamp_db(), 0.0);
    }

    #[test]
    fn shelves_and_clamping() {
        let s = EqSettings {
            enabled: true,
            mode: EqMode::Parametric,
            preamp_db: 0.0,
            bands: vec![
                Band { kind: FilterKind::LowShelf, freq_hz: 105.0, gain_db: 40.0, q: 0.7 },
                Band { kind: FilterKind::HighShelf, freq_hz: 9000.0, gain_db: -1.0, q: 0.0 },
                // Полка ниже края слышимого диапазона ничего не делает.
                Band { kind: FilterKind::LowShelf, freq_hz: 5.0, gain_db: 3.0, q: 0.7 },
                Band { kind: FilterKind::Peaking, freq_hz: f32::NAN, gain_db: 3.0, q: 1.0 },
                Band { kind: FilterKind::Peaking, freq_hz: 1000.0, gain_db: 2.0, q: 50.0 },
            ],
        };
        assert_eq!(
            s.to_mpv_af(),
            "format=format=floatp,lavfi=[\
             equalizer=f=45.8:t=o:w=2.39:g=24.00,\
             equalizer=f=13416.4:t=o:w=1.15:g=-1.00,\
             equalizer=f=1000.0:t=q:w=20.000:g=2.00]"
        );
    }

    #[test]
    fn response_curve() {
        let freqs = [20.0, 1000.0, 10000.0];
        assert_eq!(EqSettings::flat(EqMode::Graphic10).response_db(&freqs), vec![0.0; 3]);

        let mut s = EqSettings::flat(EqMode::Parametric);
        s.bands.push(Band { kind: FilterKind::Peaking, freq_hz: 1000.0, gain_db: 6.0, q: 1.414 });
        let r = s.response_db(&freqs);
        assert!((r[1] - 6.0).abs() < 0.05, "в центре полосы ровно её усиление: {}", r[1]);
        assert!(r[0].abs() < 0.2 && r[2].abs() < 0.5, "вдали от полосы почти 0: {r:?}");

        s.preamp_db = -6.0;
        let r = s.response_db(&freqs);
        assert!(r[1].abs() < 0.05 && (r[0] + 6.0).abs() < 0.2);

        s.enabled = false;
        assert_eq!(s.response_db(&freqs), vec![0.0; 3]);
    }

    #[test]
    fn shelf_emulation_curve() {
        let mut s = EqSettings::flat(EqMode::Parametric);
        s.bands.push(Band { kind: FilterKind::LowShelf, freq_hz: 105.0, gain_db: 5.0, q: 0.7 });
        let r = s.response_db(&[40.0, 5000.0]);
        assert!(r[0] > 4.0, "ниже среза — подъём: {}", r[0]);
        assert!(r[1].abs() < 0.3, "далеко выше среза — ровно: {}", r[1]);
    }

    #[test]
    fn log_frequency_grid() {
        let f = log_frequencies(3);
        assert!((f[0] - 20.0).abs() < 1e-3 && (f[2] - 20000.0).abs() < 1.0);
        assert!((f[1] - 632.5).abs() < 1.0);
    }

    #[test]
    fn auto_preamp() {
        let mut s = EqSettings::flat(EqMode::Graphic10);
        assert_eq!(s.auto_preamp_db(), 0.0);
        s.bands[2].gain_db = 4.5;
        s.bands[3].gain_db = -6.0;
        assert_eq!(s.auto_preamp_db(), -4.5);
    }

    #[test]
    fn mode_switch_keeps_curve() {
        let s = EqSettings::from_curve10(EqMode::Graphic10, &[6.0, 6.0, 6.0, 6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let s18 = s.with_mode(EqMode::Graphic18);
        assert_eq!(s18.bands.len(), 18);
        // 125 Гц есть в обеих сетках.
        let b125 = s18.bands.iter().find(|b| b.freq_hz == 125.0).unwrap();
        assert!((b125.gain_db - 6.0).abs() < 1e-4);
        // 8 кГц — ровно.
        let b8k = s18.bands.iter().find(|b| b.freq_hz == 8000.0).unwrap();
        assert!(b8k.gain_db.abs() < 1e-4);
        // 710 Гц — между 500 (6 дБ) и 1000 (0 дБ).
        let b710 = s18.bands.iter().find(|b| b.freq_hz == 710.0).unwrap();
        assert!(b710.gain_db > 0.0 && b710.gain_db < 6.0);

        let p = s.with_mode(EqMode::Parametric);
        assert_eq!(p.bands.len(), 5);
        assert_eq!(p.preamp_db, s.preamp_db);
    }

    #[test]
    fn builtin_presets_cover_modes() {
        for mode in [EqMode::Graphic10, EqMode::Graphic18, EqMode::Parametric] {
            let presets = builtin_presets(mode);
            assert_eq!(presets.len(), BUILTIN_CURVES.len());
            assert!(presets.iter().all(|p| p.settings.mode == mode));
            // Пресеты с подъёмом сразу идут с компенсирующим предусилителем.
            let bass = &presets[1].settings;
            assert!(bass.preamp_db <= -5.9);
        }
    }

    #[test]
    fn autoeq_parsing() {
        let text = "\
Preamp: -6.4 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
Filter 2: ON PK Fc 210 Hz Gain -3.1 dB Q 0.88
Filter 3: OFF PK Fc 400 Hz Gain 2.0 dB Q 1.00
Filter 4: ON HP Fc 20 Hz
Filter 5: ON HSC Fc 10000 Hz Gain -2.0 dB Q 0.70
Filter 6: ON LS Fc 80 Hz Gain 1.0 dB
";
        let s = parse_autoeq(text).unwrap();
        assert_eq!(s.mode, EqMode::Parametric);
        assert!((s.preamp_db + 6.4).abs() < 1e-4);
        assert_eq!(s.bands.len(), 4);
        assert_eq!(s.bands[0].kind, FilterKind::LowShelf);
        assert_eq!(s.bands[1].kind, FilterKind::Peaking);
        assert!((s.bands[1].gain_db + 3.1).abs() < 1e-4);
        assert_eq!(s.bands[2].kind, FilterKind::HighShelf);
        assert!((s.bands[3].q - 0.707).abs() < 1e-4);
    }

    #[test]
    fn autoeq_rejects_garbage() {
        assert!(parse_autoeq("hello world").is_err());
        assert!(parse_autoeq("Filter 1: ON PK Gain 3 dB Q 1").is_err());
    }

    #[test]
    fn store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("moth-eq-test-{}", std::process::id()));
        let path = dir.join("eq.json");
        assert_eq!(load_store(&path).unwrap(), EqStore::default());

        let store = EqStore {
            current: Some(EqSettings::flat(EqMode::Graphic18)),
            presets: vec![NamedPreset {
                name: "Мои наушники".into(),
                settings: EqSettings::flat(EqMode::Parametric),
            }],
            updated_at: 1_700_000_000,
        };
        save_store(&path, &store).unwrap();
        assert_eq!(load_store(&path).unwrap(), store);
        fs::remove_dir_all(&dir).unwrap();
    }
}
