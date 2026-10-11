//! Receive settings that survive a restart of the Bridge.
//!
//! An update restarts the Bridge, and a restarted Bridge used to come back at its built-in defaults (14.2 MHz, USB, ...), so every update (and
//! every crash recovery) lost the operator's tuning. This module keeps the settings the Bridge has *accepted* from its clients, in a small
//! versioned file outside the release directory, and puts them back before reception starts.
//!
//! What is kept: the receive tuning (VFO A, VFO B, which one is active), mode and filter, antenna and attenuation, volume, AGC, noise
//! reduction, noise blanker, the WFM de-emphasis and the squelch. What is never kept, and never restored whatever a file says: anything about
//! transmitting (PTT, MOX, TX phase or enable, drive, tune, two-tone, PureSignal, ...) or about the running state. The writer has no such field;
//! the reader ignores any key it does not know and reports it.
//!
//! The file is `rx-state-v1.txt` in `SATURN_BRIDGE_STATE_DIR` (unset: the feature is off). Another version of the format gets another file
//! name, so a Bridge that predates this feature ignores it, a newer Bridge can still read this one, and a rollback can neither be confused by
//! nor destroy a newer Bridge's file. Writes are atomic (a temporary file in the same directory, synced, then renamed) and debounced: a value
//! must stay unchanged for [`DEBOUNCE`] before it is written, and a continuing series of changes still reaches the disk every [`MAX_DELAY`].

use std::fmt::{Display, Write as _};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::radio_model::{
    AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel, WbfmDeemphasis,
};
use crate::sync_ext::MutexExt;

pub const STATE_DIR_ENV: &str = "SATURN_BRIDGE_STATE_DIR";
pub const FORMAT: &str = "saturn-bridge-rx-state";
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "rx-state-v1.txt";

pub const POLL: Duration = Duration::from_millis(250);
pub const DEBOUNCE: Duration = Duration::from_secs(2);
pub const MAX_DELAY: Duration = Duration::from_secs(20);

const MAX_FILE_BYTES: u64 = 8 * 1024;
const MAX_FILTER_HZ: i32 = 200_000;

/// The receive settings that are kept. Nothing about transmitting is in here.
#[derive(Clone, Debug, PartialEq)]
pub struct RxSettings {
    pub vfo_a_hz: u32,
    pub vfo_b_hz: u32,
    pub active_vfo: u8,
    pub mode: DemodMode,
    pub filter_low_hz: i32,
    pub filter_high_hz: i32,
    pub rx_antenna: u8,
    pub rx_attenuation_db: u8,
    pub rx_volume_db: f64,
    pub agc_mode: AgcMode,
    pub agc_gain: f64,
    pub noise_reduction_mode: NoiseReductionMode,
    pub noise_reduction_level: f64,
    pub noise_blanker_mode: NoiseBlankerMode,
    pub noise_blanker_threshold: f64,
    pub wbfm_deemphasis: WbfmDeemphasis,
    pub squelch_enabled: bool,
    pub squelch_threshold: f64,
}

/// What a file said, field by field: a field that is absent, malformed or out of range is simply `None` and the default stays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartialRx {
    pub vfo_a_hz: Option<u32>,
    pub vfo_b_hz: Option<u32>,
    pub active_vfo: Option<u8>,
    pub mode: Option<DemodMode>,
    pub filter_low_hz: Option<i32>,
    pub filter_high_hz: Option<i32>,
    pub rx_antenna: Option<u8>,
    pub rx_attenuation_db: Option<u8>,
    pub rx_volume_db: Option<f64>,
    pub agc_mode: Option<AgcMode>,
    pub agc_gain: Option<f64>,
    pub noise_reduction_mode: Option<NoiseReductionMode>,
    pub noise_reduction_level: Option<f64>,
    pub noise_blanker_mode: Option<NoiseBlankerMode>,
    pub noise_blanker_threshold: Option<f64>,
    pub wbfm_deemphasis: Option<WbfmDeemphasis>,
    pub squelch_enabled: Option<bool>,
    pub squelch_threshold: Option<f64>,
}

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// The file is not this format at all.
    NotOurs,
    /// A format version this Bridge does not know. It is left alone.
    UnsupportedVersion(String),
}

impl Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotOurs => f.write_str("not a Saturn Bridge receive-state file"),
            Self::UnsupportedVersion(v) => {
                write!(f, "format version {v} is not supported by this Bridge")
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub fields: PartialRx,
    /// Keys the file had that this version does not know (a transmit key in a doctored file is reported here and never applied).
    pub ignored: Vec<String>,
    /// Known keys whose value was malformed or out of range, with the reason.
    pub rejected: Vec<(String, String)>,
}

pub fn capture(model: &RadioModel) -> RxSettings {
    let d = &model.desired;
    RxSettings {
        vfo_a_hz: d.vfo_a_hz,
        vfo_b_hz: d.vfo_b_hz,
        active_vfo: d.active_vfo,
        mode: d.mode,
        filter_low_hz: d.filter_low_hz,
        filter_high_hz: d.filter_high_hz,
        rx_antenna: d.rx_antenna,
        rx_attenuation_db: d.rx_attenuation_db,
        rx_volume_db: d.rx_volume_db,
        agc_mode: d.agc_mode,
        agc_gain: d.agc_gain,
        noise_reduction_mode: d.rx_noise_reduction_mode,
        noise_reduction_level: d.rx_noise_reduction_level,
        noise_blanker_mode: d.nb_mode,
        noise_blanker_threshold: d.nb_threshold,
        wbfm_deemphasis: d.rx_wbfm_deemphasis,
        squelch_enabled: d.rx_ssql_enabled,
        squelch_threshold: d.rx_ssql_threshold,
    }
}

pub fn render(settings: &RxSettings) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "format={FORMAT}");
    let _ = writeln!(text, "version={VERSION}");
    let _ = writeln!(text, "vfo_a_hz={}", settings.vfo_a_hz);
    let _ = writeln!(text, "vfo_b_hz={}", settings.vfo_b_hz);
    let _ = writeln!(text, "active_vfo={}", settings.active_vfo);
    let _ = writeln!(text, "mode={}", settings.mode);
    let _ = writeln!(text, "filter_low_hz={}", settings.filter_low_hz);
    let _ = writeln!(text, "filter_high_hz={}", settings.filter_high_hz);
    let _ = writeln!(text, "rx_antenna={}", settings.rx_antenna);
    let _ = writeln!(text, "rx_attenuation_db={}", settings.rx_attenuation_db);
    let _ = writeln!(text, "rx_volume_db={}", settings.rx_volume_db);
    let _ = writeln!(text, "agc_mode={}", settings.agc_mode);
    let _ = writeln!(text, "agc_gain={}", settings.agc_gain);
    let _ = writeln!(
        text,
        "noise_reduction_mode={}",
        settings.noise_reduction_mode
    );
    let _ = writeln!(
        text,
        "noise_reduction_level={}",
        settings.noise_reduction_level
    );
    let _ = writeln!(text, "noise_blanker_mode={}", settings.noise_blanker_mode);
    let _ = writeln!(
        text,
        "noise_blanker_threshold={}",
        settings.noise_blanker_threshold
    );
    let _ = writeln!(text, "wbfm_deemphasis={}", settings.wbfm_deemphasis);
    let _ = writeln!(
        text,
        "squelch_enabled={}",
        u8::from(settings.squelch_enabled)
    );
    let _ = writeln!(text, "squelch_threshold={}", settings.squelch_threshold);
    text
}

fn parse_choice<T: Copy + Display>(text: &str, all: &[T]) -> Option<T> {
    all.iter()
        .copied()
        .find(|candidate| candidate.to_string() == text)
}

const MODES: [DemodMode; 11] = [
    DemodMode::Usb,
    DemodMode::Lsb,
    DemodMode::Cwu,
    DemodMode::Cwl,
    DemodMode::Am,
    DemodMode::Sam,
    DemodMode::Dsb,
    DemodMode::Fm,
    DemodMode::Wfm,
    DemodMode::DigU,
    DemodMode::DigL,
];
const AGC_MODES: [AgcMode; 5] = [
    AgcMode::Off,
    AgcMode::Long,
    AgcMode::Slow,
    AgcMode::Medium,
    AgcMode::Fast,
];
const NR_MODES: [NoiseReductionMode; 5] = [
    NoiseReductionMode::Off,
    NoiseReductionMode::Nr1,
    NoiseReductionMode::Nr2,
    NoiseReductionMode::Nr3,
    NoiseReductionMode::Nr4,
];
const NB_MODES: [NoiseBlankerMode; 4] = [
    NoiseBlankerMode::Off,
    NoiseBlankerMode::Nb1,
    NoiseBlankerMode::Nb2,
    NoiseBlankerMode::Nb3,
];
const DEEMPHASIS: [WbfmDeemphasis; 3] = [
    WbfmDeemphasis::Off,
    WbfmDeemphasis::NorthAmerica,
    WbfmDeemphasis::Europe,
];

fn finite_in(text: &str, low: f64, high: f64) -> Result<f64, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number"))?;
    if !value.is_finite() || value < low || value > high {
        return Err(format!("{value} is outside {low}..={high}"));
    }
    Ok(value)
}

fn integer_in<T>(text: &str, low: i64, high: i64) -> Result<T, String>
where
    T: TryFrom<i64>,
{
    let value: i64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not an integer"))?;
    if value < low || value > high {
        return Err(format!("{value} is outside {low}..={high}"));
    }
    T::try_from(value).map_err(|_| format!("{value} does not fit"))
}

/// Read the text of a file. Errors only when it is not this format or version; everything else is reported field by field.
pub fn parse(text: &str) -> Result<Parsed, LoadError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    match lines.next().and_then(|line| line.split_once('=')) {
        Some(("format", value)) if value.trim() == FORMAT => {}
        _ => return Err(LoadError::NotOurs),
    }
    match lines.next().and_then(|line| line.split_once('=')) {
        Some(("version", value)) if value.trim() == VERSION.to_string() => {}
        Some(("version", value)) => {
            return Err(LoadError::UnsupportedVersion(value.trim().to_string()))
        }
        _ => return Err(LoadError::NotOurs),
    }
    let mut parsed = Parsed::default();
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            parsed.ignored.push(line.to_string());
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let fields = &mut parsed.fields;
        let outcome: Result<(), String> = match key {
            "vfo_a_hz" => {
                integer_in::<u32>(value, 1, 4_294_967_295).map(|v| fields.vfo_a_hz = Some(v))
            }
            "vfo_b_hz" => {
                integer_in::<u32>(value, 1, 4_294_967_295).map(|v| fields.vfo_b_hz = Some(v))
            }
            "active_vfo" => integer_in::<u8>(value, 0, 1).map(|v| fields.active_vfo = Some(v)),
            "mode" => parse_choice(value, &MODES)
                .map(|v| fields.mode = Some(v))
                .ok_or_else(|| format!("'{value}' is not a mode")),
            "filter_low_hz" => {
                integer_in::<i32>(value, -i64::from(MAX_FILTER_HZ), i64::from(MAX_FILTER_HZ))
                    .map(|v| fields.filter_low_hz = Some(v))
            }
            "filter_high_hz" => {
                integer_in::<i32>(value, -i64::from(MAX_FILTER_HZ), i64::from(MAX_FILTER_HZ))
                    .map(|v| fields.filter_high_hz = Some(v))
            }
            "rx_antenna" => integer_in::<u8>(value, 1, 3).map(|v| fields.rx_antenna = Some(v)),
            "rx_attenuation_db" => {
                integer_in::<u8>(value, 0, 31).map(|v| fields.rx_attenuation_db = Some(v))
            }
            "rx_volume_db" => finite_in(value, -40.0, 12.0).map(|v| fields.rx_volume_db = Some(v)),
            "agc_mode" => parse_choice(value, &AGC_MODES)
                .map(|v| fields.agc_mode = Some(v))
                .ok_or_else(|| format!("'{value}' is not an AGC mode")),
            "agc_gain" => finite_in(value, 0.0, 100.0).map(|v| fields.agc_gain = Some(v)),
            "noise_reduction_mode" => parse_choice(value, &NR_MODES)
                .map(|v| fields.noise_reduction_mode = Some(v))
                .ok_or_else(|| format!("'{value}' is not a noise reduction mode")),
            "noise_reduction_level" => {
                finite_in(value, 0.0, 100.0).map(|v| fields.noise_reduction_level = Some(v))
            }
            "noise_blanker_mode" => parse_choice(value, &NB_MODES)
                .map(|v| fields.noise_blanker_mode = Some(v))
                .ok_or_else(|| format!("'{value}' is not a noise blanker mode")),
            "noise_blanker_threshold" => {
                finite_in(value, 0.0, 100.0).map(|v| fields.noise_blanker_threshold = Some(v))
            }
            "wbfm_deemphasis" => parse_choice(value, &DEEMPHASIS)
                .map(|v| fields.wbfm_deemphasis = Some(v))
                .ok_or_else(|| format!("'{value}' is not a de-emphasis")),
            "squelch_enabled" => match value {
                "0" => Ok(fields.squelch_enabled = Some(false)),
                "1" => Ok(fields.squelch_enabled = Some(true)),
                _ => Err(format!("'{value}' is not 0 or 1")),
            },
            "squelch_threshold" => {
                finite_in(value, 0.0, 100.0).map(|v| fields.squelch_threshold = Some(v))
            }
            other => {
                parsed.ignored.push(other.to_string());
                continue;
            }
        };
        if let Err(reason) = outcome {
            parsed.rejected.push((key.to_string(), reason));
        }
    }
    Ok(parsed)
}

#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub applied: Vec<&'static str>,
    pub skipped: Vec<(&'static str, String)>,
}

/// Put restored settings into the model's desired state. Only the receive fields above are written; the running state, TX enable, TX phase,
/// drive and every other transmit field are left exactly as they were. `frequency_ok` is the backend's own rule for a tuning it would accept.
pub fn apply(
    model: &mut RadioModel,
    fields: &PartialRx,
    frequency_ok: &dyn Fn(u32) -> bool,
) -> Applied {
    let mut applied = Applied::default();
    let d = &mut model.desired;
    let mut tuned = false;
    if let Some(hz) = fields.vfo_a_hz {
        if frequency_ok(hz) {
            d.vfo_a_hz = hz;
            applied.applied.push("vfo_a_hz");
            tuned = true;
        } else {
            applied.skipped.push((
                "vfo_a_hz",
                format!("{hz} Hz is not a frequency this receiver accepts"),
            ));
        }
    }
    if let Some(hz) = fields.vfo_b_hz {
        if frequency_ok(hz) {
            d.vfo_b_hz = hz;
            applied.applied.push("vfo_b_hz");
            tuned = true;
        } else {
            applied.skipped.push((
                "vfo_b_hz",
                format!("{hz} Hz is not a frequency this receiver accepts"),
            ));
        }
    }
    if let Some(active) = fields.active_vfo {
        d.active_vfo = active.min(1);
        applied.applied.push("active_vfo");
        tuned = true;
    }
    if let Some(mode) = fields.mode {
        d.mode = mode;
        (d.filter_low_hz, d.filter_high_hz) = mode.default_filter_band();
        applied.applied.push("mode");
    }
    match (fields.filter_low_hz, fields.filter_high_hz) {
        (Some(low), Some(high)) if low < high => {
            d.filter_low_hz = low;
            d.filter_high_hz = high;
            applied.applied.push("filter");
        }
        (Some(_), Some(_)) => applied.skipped.push((
            "filter",
            "the low edge is not below the high edge".to_string(),
        )),
        _ => {}
    }
    if let Some(v) = fields.rx_antenna {
        d.rx_antenna = v;
        applied.applied.push("rx_antenna");
    }
    if let Some(v) = fields.rx_attenuation_db {
        d.rx_attenuation_db = v;
        applied.applied.push("rx_attenuation_db");
    }
    if let Some(v) = fields.rx_volume_db {
        d.rx_volume_db = v;
        applied.applied.push("rx_volume_db");
    }
    if let Some(v) = fields.agc_mode {
        d.agc_mode = v;
        applied.applied.push("agc_mode");
    }
    if let Some(v) = fields.agc_gain {
        d.agc_gain = v;
        applied.applied.push("agc_gain");
    }
    if let Some(v) = fields.noise_reduction_mode {
        d.rx_noise_reduction_mode = v;
        applied.applied.push("noise_reduction_mode");
    }
    if let Some(v) = fields.noise_reduction_level {
        d.rx_noise_reduction_level = v;
        applied.applied.push("noise_reduction_level");
    }
    if let Some(v) = fields.noise_blanker_mode {
        d.nb_mode = v;
        applied.applied.push("noise_blanker_mode");
    }
    if let Some(v) = fields.noise_blanker_threshold {
        d.nb_threshold = v;
        applied.applied.push("noise_blanker_threshold");
    }
    if let Some(v) = fields.wbfm_deemphasis {
        d.rx_wbfm_deemphasis = v;
        applied.applied.push("wbfm_deemphasis");
    }
    if let Some(v) = fields.squelch_enabled {
        d.rx_ssql_enabled = v;
        applied.applied.push("squelch_enabled");
    }
    if let Some(v) = fields.squelch_threshold {
        d.rx_ssql_threshold = v;
        applied.applied.push("squelch_threshold");
    }
    if tuned {
        model.sync_vfo_routes();
    }
    applied
}

/// Where the state lives. `None` from [`RxStateStore::from_env`] means the feature is off.
#[derive(Clone, Debug)]
pub struct RxStateStore {
    dir: PathBuf,
}

impl RxStateStore {
    pub fn from_env() -> Option<Self> {
        Self::from_value(std::env::var_os(STATE_DIR_ENV))
    }

    /// The store for the configured directory; none when the variable is unset or empty (the feature is off).
    pub fn from_value(value: Option<std::ffi::OsString>) -> Option<Self> {
        let value = value?;
        if value.is_empty() {
            return None;
        }
        Some(Self::new(PathBuf::from(value)))
    }

    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// The saved settings, if there are any this Bridge can read. A missing file is not an error.
    pub fn load(&self) -> Result<Option<Parsed>, LoadError> {
        let path = self.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(LoadError::NotOurs);
        }
        match fs::read_to_string(&path) {
            Ok(text) => parse(&text).map(Some),
            Err(_) => Err(LoadError::NotOurs),
        }
    }

    /// Restore into `model` before reception starts; says what it did on the Bridge's own log, and returns the settings now in force (the baseline
    /// the saver compares against, so an unchanged state is not rewritten).
    pub fn restore(
        &self,
        model: &mut RadioModel,
        frequency_ok: &dyn Fn(u32) -> bool,
    ) -> Option<RxSettings> {
        match self.load() {
            Ok(None) => {
                println!(
                    "saturn-bridge: no saved receive settings in {}; starting at the defaults",
                    self.dir.display()
                );
                None
            }
            Ok(Some(parsed)) => {
                let applied = apply(model, &parsed.fields, frequency_ok);
                println!(
                    "saturn-bridge: restored receive settings from {}: {} (VFO A {} Hz, VFO B {} Hz, active VFO {}, mode {})",
                    self.path().display(),
                    applied.applied.join(","),
                    model.desired.vfo_a_hz,
                    model.desired.vfo_b_hz,
                    model.desired.active_vfo,
                    model.desired.mode,
                );
                for key in &parsed.ignored {
                    eprintln!("saturn-bridge: saved receive settings: ignored '{key}' (not a setting this version restores)");
                }
                for (key, reason) in parsed
                    .rejected
                    .iter()
                    .map(|(k, r)| (k.as_str(), r.as_str()))
                    .chain(applied.skipped.iter().map(|(k, r)| (*k, r.as_str())))
                {
                    eprintln!("saturn-bridge: saved receive settings: kept the default for '{key}': {reason}");
                }
                Some(capture(model))
            }
            Err(error) => {
                eprintln!("saturn-bridge: saved receive settings at {} not used: {error}; starting at the defaults", self.path().display());
                None
            }
        }
    }

    /// Write the settings atomically: a temporary file in the same directory, synced, renamed over the old one, and the directory synced.
    pub fn save(&self, settings: &RxSettings) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let final_path = self.path();
        let temporary = self
            .dir
            .join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&temporary);
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(render(settings).as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &final_path)?;
            fs::File::open(&self.dir)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

/// Decides when a change is written: after it has stayed unchanged for `debounce`, or at the latest `max_delay` after it first differed from
/// the file. Pure (time is passed in) so it can be tested without sleeping.
#[derive(Debug)]
pub struct Debouncer {
    written: Option<RxSettings>,
    pending: Option<Pending>,
    debounce: Duration,
    max_delay: Duration,
}

#[derive(Debug)]
struct Pending {
    settings: RxSettings,
    first_differed: Instant,
    last_changed: Instant,
}

impl Debouncer {
    pub fn new(written: Option<RxSettings>, debounce: Duration, max_delay: Duration) -> Self {
        Self {
            written,
            pending: None,
            debounce,
            max_delay,
        }
    }

    /// Feed the current settings. Returns what should be written now, if anything.
    pub fn observe(&mut self, now: Instant, current: &RxSettings) -> Option<RxSettings> {
        if self.written.as_ref() == Some(current) {
            self.pending = None;
            return None;
        }
        let (first_differed, last_changed) = match self.pending.as_ref() {
            Some(pending) if pending.settings == *current => {
                (pending.first_differed, pending.last_changed)
            }
            Some(pending) => (pending.first_differed, now),
            None => (now, now),
        };
        self.pending = Some(Pending {
            settings: current.clone(),
            first_differed,
            last_changed,
        });
        if now.duration_since(last_changed) >= self.debounce
            || now.duration_since(first_differed) >= self.max_delay
        {
            return Some(current.clone());
        }
        None
    }

    pub fn wrote(&mut self, settings: RxSettings) {
        self.written = Some(settings);
        self.pending = None;
    }

    /// At shutdown: whatever differs from the file is written now, without waiting.
    pub fn flush(&mut self, current: &RxSettings) -> Option<RxSettings> {
        if self.written.as_ref() == Some(current) {
            None
        } else {
            Some(current.clone())
        }
    }
}

/// Watches the model on its own thread and keeps the file current. Dropping it writes any last change.
pub struct RxStateSaver {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RxStateSaver {
    pub fn spawn(
        store: RxStateStore,
        model: Arc<Mutex<RadioModel>>,
        baseline: Option<RxSettings>,
    ) -> Self {
        Self::spawn_with(store, model, baseline, POLL, DEBOUNCE, MAX_DELAY)
    }

    pub fn spawn_with(
        store: RxStateStore,
        model: Arc<Mutex<RadioModel>>,
        baseline: Option<RxSettings>,
        poll: Duration,
        debounce: Duration,
        max_delay: Duration,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::Builder::new()
            .name("rx-state-saver".to_string())
            .spawn(move || {
                let mut debouncer = Debouncer::new(baseline, debounce, max_delay);
                let mut last_error_at: Option<Instant> = None;
                let report = |error: &std::io::Error, last: &mut Option<Instant>| {
                    if last.map_or(true, |at| at.elapsed() >= Duration::from_secs(60)) {
                        eprintln!(
                            "saturn-bridge: could not save receive settings to {}: {error}",
                            store.path().display()
                        );
                        *last = Some(Instant::now());
                    }
                };
                while !flag.load(Ordering::Relaxed) {
                    thread::sleep(poll);
                    let current = capture(&model.lock_unpoisoned());
                    if let Some(settings) = debouncer.observe(Instant::now(), &current) {
                        match store.save(&settings) {
                            Ok(()) => debouncer.wrote(settings),
                            Err(error) => report(&error, &mut last_error_at),
                        }
                    }
                }
                let current = capture(&model.lock_unpoisoned());
                if let Some(settings) = debouncer.flush(&current) {
                    if let Err(error) = store.save(&settings) {
                        report(&error, &mut last_error_at);
                    }
                }
            })
            .ok();
        Self { stop, handle }
    }

    pub fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RxStateSaver {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The frequencies the receiver accepts when the backend has no rule of its own to offer: the DDC's tuning range.
pub fn generic_frequency_ok(hz: u32) -> bool {
    (1..=61_440_000).contains(&hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> RadioModel {
        RadioModel::new(6, 14_200_000, 0, 384, 24, 2048, false, 2048, false)
    }

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "saturn-rx-state-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tuned() -> RadioModel {
        let mut model = model();
        model.desired.vfo_a_hz = 3_902_000;
        model.desired.vfo_b_hz = 7_074_000;
        model.desired.active_vfo = 0;
        model.sync_vfo_routes();
        model.desired.mode = DemodMode::Lsb;
        model.desired.filter_low_hz = -2800;
        model.desired.filter_high_hz = -200;
        model.desired.rx_antenna = 2;
        model.desired.rx_attenuation_db = 12;
        model.desired.rx_volume_db = -6.5;
        model.desired.agc_mode = AgcMode::Fast;
        model.desired.agc_gain = 72.5;
        model.desired.rx_noise_reduction_mode = NoiseReductionMode::Nr2;
        model.desired.rx_noise_reduction_level = 40.0;
        model.desired.nb_mode = NoiseBlankerMode::Nb2;
        model.desired.nb_threshold = 25.0;
        model.desired.rx_wbfm_deemphasis = WbfmDeemphasis::Europe;
        model.desired.rx_ssql_enabled = true;
        model.desired.rx_ssql_threshold = 33.0;
        model
    }

    fn all_ok(_: u32) -> bool {
        true
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        let model = tuned();
        let settings = capture(&model);
        let parsed = parse(&render(&settings)).expect("parses");
        assert!(
            parsed.ignored.is_empty() && parsed.rejected.is_empty(),
            "{parsed:?}"
        );
        let mut fresh = self::model();
        let applied = apply(&mut fresh, &parsed.fields, &all_ok);
        assert!(applied.skipped.is_empty(), "{applied:?}");
        assert_eq!(capture(&fresh), settings);
        assert_eq!(
            fresh.desired.iq_center_hz, 3_902_000,
            "the receive center follows the active VFO"
        );
        assert_eq!(fresh.desired.tx_frequency_hz, 3_902_000);
    }

    #[test]
    fn the_second_vfo_active_restores_its_center() {
        let mut model = tuned();
        model.desired.active_vfo = 1;
        model.sync_vfo_routes();
        let mut fresh = self::model();
        apply(
            &mut fresh,
            &parse(&render(&capture(&model))).unwrap().fields,
            &all_ok,
        );
        assert_eq!(
            (fresh.desired.active_vfo, fresh.desired.iq_center_hz),
            (1, 7_074_000)
        );
    }

    #[test]
    fn nothing_about_transmitting_is_ever_written_or_restored() {
        let text = render(&capture(&tuned()));
        for forbidden in [
            "tx",
            "ptt",
            "mox",
            "tune",
            "drive",
            "two_tone",
            "puresignal",
            "pure_signal",
            "running",
            "split",
            "phase",
        ] {
            assert!(
                !text.to_ascii_lowercase().contains(forbidden),
                "the file mentions {forbidden}: {text}"
            );
        }
        let doctored = format!("{text}tx_enabled=1\nptt=1\nmox=1\ntx_phase=Tx\ntx_drive=100\ntune=1\ntwo_tone_enabled=1\nrunning=0\nsplit_enabled=1\npure_signal_enabled=1\n");
        let parsed = parse(&doctored).unwrap();
        assert_eq!(parsed.ignored.len(), 10, "{:?}", parsed.ignored);
        let mut fresh = self::model();
        fresh.desired.running = true;
        let reference = self::model();
        let before = format!("{:?}", fresh.desired);
        apply(&mut fresh, &parsed.fields, &all_ok);
        let after = &fresh.desired;
        assert!(
            !after.tx_enabled
                && after.tx_phase == crate::radio_model::TxPhase::Rx
                && !after.two_tone_enabled
                && !after.pure_signal_enabled
                && !after.split_enabled
        );
        assert!(after.running, "the running state was changed by a file");
        assert_eq!(
            after.tx_drive, reference.desired.tx_drive,
            "drive was changed by a file"
        );
        assert_eq!(
            after.tx_frequency_hz, after.vfo_a_hz,
            "the transmit frequency must follow the receive VFO only"
        );
        assert_ne!(
            before,
            format!("{after:?}"),
            "the receive fields were not applied"
        );
    }

    #[test]
    fn only_receive_fields_change_in_the_model() {
        let mut fresh = self::model();
        let baseline = format!("{:?}", fresh.desired);
        apply(
            &mut fresh,
            &parse(&render(&capture(&tuned()))).unwrap().fields,
            &all_ok,
        );
        let mut expected = tuned();
        // everything the restore may write is receive; compare the whole desired state with a model that had only those set
        expected.desired.tx_enabled = false;
        assert_ne!(baseline, format!("{:?}", fresh.desired));
        assert_eq!(
            format!("{:?}", fresh.desired),
            format!("{:?}", expected.desired)
        );
    }

    #[test]
    fn a_frequency_the_backend_would_refuse_is_not_restored() {
        let mut fresh = self::model();
        let parsed = parse("format=saturn-bridge-rx-state\nversion=1\nvfo_a_hz=99999999\nvfo_b_hz=7074000\nactive_vfo=0\n").unwrap();
        let applied = apply(&mut fresh, &parsed.fields, &generic_frequency_ok);
        assert_eq!(fresh.desired.vfo_a_hz, 14_200_000);
        assert_eq!(fresh.desired.vfo_b_hz, 7_074_000);
        assert_eq!(applied.skipped.len(), 1);
        assert_eq!(
            fresh.desired.iq_center_hz, 14_200_000,
            "the receive center must stay at a frequency that was accepted"
        );
    }

    #[test]
    fn malformed_and_out_of_range_fields_keep_their_defaults_and_are_reported() {
        let text = "format=saturn-bridge-rx-state\nversion=1\nvfo_a_hz=abc\nvfo_b_hz=-5\nactive_vfo=2\nmode=BOGUS\nmode_x=1\nfilter_low_hz=900000\n\
                    rx_antenna=0\nrx_attenuation_db=99\nrx_volume_db=NaN\nagc_mode=turbo\nagc_gain=101\nnoise_reduction_mode=NR9\nnoise_reduction_level=-1\n\
                    noise_blanker_mode=NB9\nnoise_blanker_threshold=inf\nwbfm_deemphasis=x\nsquelch_enabled=yes\nsquelch_threshold=1e9\n";
        let parsed = parse(text).unwrap();
        assert_eq!(
            parsed.fields,
            PartialRx::default(),
            "something malformed was accepted: {:?}",
            parsed.fields
        );
        assert_eq!(parsed.rejected.len(), 17, "{:?}", parsed.rejected);
        assert_eq!(parsed.ignored, vec!["mode_x".to_string()]);
        let mut fresh = self::model();
        let before = format!("{:?}", fresh.desired);
        apply(&mut fresh, &parsed.fields, &all_ok);
        assert_eq!(before, format!("{:?}", fresh.desired));
    }

    #[test]
    fn values_just_outside_a_range_are_rejected_and_values_on_the_edge_are_accepted() {
        let head = "format=saturn-bridge-rx-state\nversion=1\n";
        for rejected in [
            "rx_volume_db=12.5",
            "rx_volume_db=-40.5",
            "vfo_a_hz=0",
            "vfo_b_hz=0",
            "agc_gain=100.5",
            "noise_reduction_level=100.5",
            "noise_blanker_threshold=-0.5",
            "squelch_threshold=100.5",
            "rx_antenna=4",
            "rx_attenuation_db=32",
            "filter_high_hz=200001",
            "filter_low_hz=-200001",
        ] {
            let parsed = parse(&format!("{head}{rejected}\n")).unwrap();
            assert_eq!(
                parsed.fields,
                PartialRx::default(),
                "{rejected} was accepted"
            );
            assert_eq!(parsed.rejected.len(), 1, "{rejected}");
        }
        for accepted in [
            "rx_volume_db=12",
            "rx_volume_db=-40",
            "vfo_a_hz=1",
            "agc_gain=100",
            "agc_gain=0",
            "rx_antenna=1",
            "rx_antenna=3",
            "rx_attenuation_db=31",
            "rx_attenuation_db=0",
            "filter_high_hz=200000",
            "filter_low_hz=-200000",
            "active_vfo=0",
            "active_vfo=1",
        ] {
            let parsed = parse(&format!("{head}{accepted}\n")).unwrap();
            assert!(
                parsed.rejected.is_empty(),
                "{accepted} was rejected: {:?}",
                parsed.rejected
            );
            assert_ne!(
                parsed.fields,
                PartialRx::default(),
                "{accepted} was not read"
            );
        }
    }

    #[test]
    fn the_second_vfo_is_held_to_the_same_frequency_rule() {
        let mut fresh = self::model();
        let parsed = parse(
            "format=saturn-bridge-rx-state\nversion=1\nvfo_a_hz=7100000\nvfo_b_hz=99999999\n",
        )
        .unwrap();
        let applied = apply(&mut fresh, &parsed.fields, &generic_frequency_ok);
        assert_eq!(
            (fresh.desired.vfo_a_hz, fresh.desired.vfo_b_hz),
            (7_100_000, 14_200_000)
        );
        assert_eq!(applied.skipped.len(), 1);
        assert_eq!(applied.skipped[0].0, "vfo_b_hz");
    }

    #[test]
    fn a_filter_whose_edges_are_inverted_is_not_restored() {
        let mut fresh = self::model();
        let parsed = parse("format=saturn-bridge-rx-state\nversion=1\nmode=USB\nfilter_low_hz=3000\nfilter_high_hz=300\n").unwrap();
        let applied = apply(&mut fresh, &parsed.fields, &all_ok);
        assert_eq!(
            (fresh.desired.filter_low_hz, fresh.desired.filter_high_hz),
            DemodMode::Usb.default_filter_band()
        );
        assert_eq!(applied.skipped.len(), 1);
    }

    #[test]
    fn a_mode_without_a_filter_takes_the_modes_default_filter() {
        let mut fresh = self::model();
        apply(
            &mut fresh,
            &parse("format=saturn-bridge-rx-state\nversion=1\nmode=CWU\n")
                .unwrap()
                .fields,
            &all_ok,
        );
        assert_eq!(
            (
                fresh.desired.mode,
                fresh.desired.filter_low_hz,
                fresh.desired.filter_high_hz
            ),
            (DemodMode::Cwu, 200, 800)
        );
    }

    #[test]
    fn a_file_that_is_not_ours_or_is_another_version_is_not_used() {
        assert_eq!(parse("hello"), Err(LoadError::NotOurs));
        assert_eq!(parse(""), Err(LoadError::NotOurs));
        assert_eq!(
            parse("format=something-else\nversion=1\n"),
            Err(LoadError::NotOurs)
        );
        assert_eq!(
            parse("format=saturn-bridge-rx-state\n"),
            Err(LoadError::NotOurs)
        );
        assert_eq!(
            parse("format=saturn-bridge-rx-state\nversion=2\nvfo_a_hz=1000000\n"),
            Err(LoadError::UnsupportedVersion("2".to_string()))
        );
        assert_eq!(
            parse("version=1\nformat=saturn-bridge-rx-state\n"),
            Err(LoadError::NotOurs)
        );
    }

    #[test]
    fn unknown_keys_of_a_newer_minor_addition_are_ignored_not_fatal() {
        let parsed =
            parse("format=saturn-bridge-rx-state\nversion=1\nvfo_a_hz=7100000\nfuture_setting=1\n")
                .unwrap();
        assert_eq!(parsed.fields.vfo_a_hz, Some(7_100_000));
        assert_eq!(parsed.ignored, vec!["future_setting".to_string()]);
    }

    #[test]
    fn the_file_is_private_atomic_and_leaves_no_temporary_behind() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("write");
        let store = RxStateStore::new(dir.clone());
        store.save(&capture(&tuned())).unwrap();
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()]);
        store.save(&capture(&model())).unwrap();
        assert_eq!(
            parse(&fs::read_to_string(store.path()).unwrap())
                .unwrap()
                .fields
                .vfo_a_hz,
            Some(14_200_000)
        );
    }

    #[test]
    fn a_write_that_fails_after_the_temporary_file_exists_removes_it() {
        let dir = temp_dir("leftover");
        let store = RxStateStore::new(dir.clone());
        // the final name is a non-empty directory: the file is written, then the rename fails
        fs::create_dir(store.path()).unwrap();
        fs::write(store.path().join("keep"), "x").unwrap();
        assert!(store.save(&capture(&model())).is_err());
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec![FILE_NAME.to_string()],
            "a temporary file was left behind: {names:?}"
        );
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_whole() {
        let dir = temp_dir("fail");
        let store = RxStateStore::new(dir.clone());
        store.save(&capture(&tuned())).unwrap();
        let before = fs::read_to_string(store.path()).unwrap();
        // a directory sits where the temporary file must be created: the write fails before the rename
        fs::create_dir(dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()))).unwrap();
        assert!(store.save(&capture(&model())).is_err());
        assert_eq!(fs::read_to_string(store.path()).unwrap(), before);
    }

    #[test]
    fn a_symbolic_link_or_oversized_file_is_not_read() {
        let dir = temp_dir("link");
        let store = RxStateStore::new(dir.clone());
        fs::write(dir.join("target"), render(&capture(&tuned()))).unwrap();
        std::os::unix::fs::symlink(dir.join("target"), store.path()).unwrap();
        assert_eq!(store.load(), Err(LoadError::NotOurs));
        fs::remove_file(store.path()).unwrap();
        fs::write(
            store.path(),
            "format=saturn-bridge-rx-state\nversion=1\n".to_string() + &"x=1\n".repeat(5000),
        )
        .unwrap();
        assert_eq!(store.load(), Err(LoadError::NotOurs));
    }

    #[test]
    fn a_missing_file_is_simply_the_defaults() {
        let store = RxStateStore::new(temp_dir("none"));
        let mut fresh = self::model();
        let before = format!("{:?}", fresh.desired);
        assert_eq!(store.restore(&mut fresh, &all_ok), None);
        assert_eq!(before, format!("{:?}", fresh.desired));
    }

    #[test]
    fn a_newer_bridges_file_is_left_alone_and_never_overwritten() {
        let dir = temp_dir("v2");
        let newer = dir.join("rx-state-v2.txt");
        fs::write(
            &newer,
            "format=saturn-bridge-rx-state\nversion=2\nvfo_a_hz=1000000\n",
        )
        .unwrap();
        let store = RxStateStore::new(dir.clone());
        let mut fresh = self::model();
        assert_eq!(
            store.restore(&mut fresh, &all_ok),
            None,
            "a v1 reader must not read the v2 file"
        );
        store.save(&capture(&tuned())).unwrap();
        assert_eq!(
            fs::read_to_string(&newer).unwrap(),
            "format=saturn-bridge-rx-state\nversion=2\nvfo_a_hz=1000000\n"
        );
        // and a v2 text under the v1 name is refused, not half-read
        fs::write(
            store.path(),
            "format=saturn-bridge-rx-state\nversion=2\nvfo_a_hz=1000000\n",
        )
        .unwrap();
        assert_eq!(
            store.load(),
            Err(LoadError::UnsupportedVersion("2".to_string()))
        );
    }

    #[test]
    fn a_restart_comes_back_with_the_settings_it_had() {
        let dir = temp_dir("restart");
        let store = RxStateStore::new(dir);
        let shared = Arc::new(Mutex::new(tuned()));
        let mut saver = RxStateSaver::spawn_with(
            store.clone(),
            shared.clone(),
            None,
            Duration::from_millis(10),
            Duration::from_millis(40),
            Duration::from_secs(5),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !store.path().exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        saver.finish();
        let mut restarted = self::model();
        let restored = store.restore(&mut restarted, &all_ok).expect("restored");
        assert_eq!(restored, capture(&shared.lock_unpoisoned()));
        assert_eq!(restarted.desired.iq_center_hz, 3_902_000);
        assert_eq!(restarted.desired.mode, DemodMode::Lsb);
    }

    #[test]
    fn dropping_the_saver_writes_a_change_that_had_not_waited_out_the_debounce() {
        let dir = temp_dir("flush");
        let store = RxStateStore::new(dir);
        let shared = Arc::new(Mutex::new(model()));
        {
            let _saver = RxStateSaver::spawn_with(
                store.clone(),
                shared.clone(),
                Some(capture(&model())),
                Duration::from_millis(10),
                Duration::from_secs(60),
                Duration::from_secs(600),
            );
            thread::sleep(Duration::from_millis(60));
            assert!(!store.path().exists(), "nothing had changed");
            shared.lock_unpoisoned().desired.vfo_a_hz = 5_357_000;
            shared.lock_unpoisoned().sync_vfo_routes();
            thread::sleep(Duration::from_millis(100));
            assert!(!store.path().exists(), "written before the debounce passed");
        }
        assert_eq!(
            store.load().unwrap().unwrap().fields.vfo_a_hz,
            Some(5_357_000)
        );
    }

    #[test]
    fn restoring_makes_the_unchanged_state_the_baseline_so_it_is_not_rewritten() {
        let dir = temp_dir("baseline");
        let store = RxStateStore::new(dir);
        store.save(&capture(&tuned())).unwrap();
        let before = fs::metadata(store.path()).unwrap().modified().unwrap();
        let mut fresh = self::model();
        let baseline = store.restore(&mut fresh, &all_ok);
        let shared = Arc::new(Mutex::new(fresh));
        {
            let _saver = RxStateSaver::spawn_with(
                store.clone(),
                shared,
                baseline,
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(100),
            );
            thread::sleep(Duration::from_millis(150));
        }
        assert_eq!(
            fs::metadata(store.path()).unwrap().modified().unwrap(),
            before,
            "an unchanged state was written again"
        );
    }

    fn settings(hz: u32) -> RxSettings {
        let mut m = model();
        m.desired.vfo_a_hz = hz;
        capture(&m)
    }

    #[test]
    fn a_change_is_written_only_after_it_has_stayed_still_for_the_debounce() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(
            Some(settings(1)),
            Duration::from_secs(2),
            Duration::from_secs(20),
        );
        assert_eq!(d.observe(t0, &settings(1)), None, "nothing changed");
        assert_eq!(d.observe(t0, &settings(2)), None);
        assert_eq!(
            d.observe(t0 + Duration::from_millis(1900), &settings(2)),
            None
        );
        assert_eq!(
            d.observe(t0 + Duration::from_millis(2000), &settings(2)),
            Some(settings(2))
        );
        d.wrote(settings(2));
        assert_eq!(
            d.observe(t0 + Duration::from_secs(3), &settings(2)),
            None,
            "an unchanged value is not written twice"
        );
    }

    #[test]
    fn a_value_that_keeps_changing_restarts_the_wait_but_not_past_the_maximum() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(
            Some(settings(0)),
            Duration::from_secs(2),
            Duration::from_secs(20),
        );
        let mut written = None;
        for second in 0..40u32 {
            // tuning knob turning: a new value every second
            if let Some(w) = d.observe(
                t0 + Duration::from_secs(u64::from(second)),
                &settings(second + 1),
            ) {
                written = Some((second, w));
                break;
            }
        }
        let (second, w) = written.expect("a continuously changing value must still reach the disk");
        assert_eq!(second, 20, "the write comes at the maximum delay");
        assert_eq!(w, settings(21));
    }

    #[test]
    fn a_change_that_is_undone_before_the_debounce_is_never_written() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(
            Some(settings(1)),
            Duration::from_secs(2),
            Duration::from_secs(20),
        );
        assert_eq!(d.observe(t0, &settings(2)), None);
        assert_eq!(d.observe(t0 + Duration::from_secs(1), &settings(1)), None);
        assert_eq!(d.observe(t0 + Duration::from_secs(10), &settings(1)), None);
        // changed again later: this is a new change with its own wait, not a continuation of the undone one
        assert_eq!(
            d.observe(t0 + Duration::from_secs(11), &settings(2)),
            None,
            "the undone change's age was carried over"
        );
        assert_eq!(
            d.observe(t0 + Duration::from_millis(12_900), &settings(2)),
            None
        );
        assert_eq!(
            d.observe(t0 + Duration::from_secs(13), &settings(2)),
            Some(settings(2))
        );
    }

    #[test]
    fn flush_writes_only_what_differs() {
        let mut d = Debouncer::new(
            Some(settings(1)),
            Duration::from_secs(2),
            Duration::from_secs(20),
        );
        assert_eq!(d.flush(&settings(1)), None);
        assert_eq!(d.flush(&settings(3)), Some(settings(3)));
        let mut empty = Debouncer::new(None, Duration::from_secs(2), Duration::from_secs(20));
        assert_eq!(empty.flush(&settings(3)), Some(settings(3)));
    }

    #[test]
    fn a_written_state_is_not_written_again_while_it_stays_the_same() {
        let dir = temp_dir("once");
        let store = RxStateStore::new(dir);
        let shared = Arc::new(Mutex::new(tuned()));
        let mut saver = RxStateSaver::spawn_with(
            store.clone(),
            shared,
            None,
            Duration::from_millis(10),
            Duration::from_millis(30),
            Duration::from_secs(5),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !store.path().exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(60));
        let written = fs::metadata(store.path()).unwrap().modified().unwrap();
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            fs::metadata(store.path()).unwrap().modified().unwrap(),
            written,
            "the same state was written over and over"
        );
        saver.finish();
    }

    #[test]
    fn the_feature_is_off_unless_a_directory_is_configured() {
        assert!(RxStateStore::from_value(None).is_none());
        assert!(
            RxStateStore::from_value(Some(std::ffi::OsString::new())).is_none(),
            "an empty value must not mean the current directory"
        );
        assert_eq!(
            RxStateStore::from_value(Some("/var/lib/saturn-bridge".into()))
                .unwrap()
                .path(),
            PathBuf::from("/var/lib/saturn-bridge/rx-state-v1.txt")
        );
    }

    #[test]
    fn the_file_name_and_format_are_the_published_contract() {
        // A Bridge of another version looks for its own file name; changing these is a format change and needs a new name and a new version.
        assert_eq!(
            (FORMAT, VERSION, FILE_NAME),
            ("saturn-bridge-rx-state", 1, "rx-state-v1.txt")
        );
        assert!(FILE_NAME.contains(&format!("v{VERSION}")));
    }
}
