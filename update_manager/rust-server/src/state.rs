use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

pub const DEFAULT_MAX_BODY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_RESTORE_MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_UPDATE_HEALTH_TIMEOUT_SECS: u64 = 8;
pub const DEFAULT_UPDATE_HEALTH_RETRIES: u32 = 2;
pub const DEFAULT_UPDATE_HEALTH_INITIAL_DELAY_SECS: u64 = 0;
pub const HEALTH_CHECK_RETRY_INTERVAL_SECS: u64 = 2;
pub const DEFAULT_UPDATE_KEEP_SNAPSHOTS: usize = 5;
pub const DEFAULT_STAGE_WORKTREE_KEEP: usize = 6;
pub const MAX_CUSTOM_SCRIPTS: usize = 64;
pub const MAX_SCRIPT_FLAG_LEN: usize = 256;
pub const MAX_SCRIPT_FLAGS: usize = 32;
pub const MAX_TAR_EXPANSION_FACTOR: u64 = 10;
// Upper bound on the raw JSON file to guard deserialization cost.
// Derived from MAX_CUSTOM_SCRIPTS * (MAX_SCRIPT_FLAGS * MAX_SCRIPT_FLAG_LEN + per-entry overhead).
pub const MAX_CUSTOM_SCRIPTS_FILE_BYTES: u64 = 1_048_576; // 1 MiB
pub const MAX_REMOTE_SETTINGS_FILE_BYTES: u64 = 1_048_576; // 1 MiB
pub const MAX_REMOTE_PROFILES_FILE_BYTES: u64 = 1_048_576; // 1 MiB
pub const CSRF_HEADER_NAME: &str = "x-saturn-csrf";
pub const CSRF_HEADER_VALUE: &str = "1";
pub const RUN_LOG_MAX_LINES: usize = 5000;
pub const RUN_LOG_FETCH_MAX_LINES: usize = 1000;
pub const DEFAULT_CUSTOM_SCRIPT_CLEAN_LOGS: &str =
    include_str!("../../scripts/cleanup-saturn-logs.sh");
pub const DEFAULT_CUSTOM_SCRIPT_CLEAN_BACKUPS: &str =
    include_str!("../../scripts/cleanup-saturn-backups.sh");
pub const DEFAULT_CUSTOM_SCRIPT_FIX_LED_POWER_BUTTON: &str =
    include_str!("../../../scripts/fix-LED-power-button.sh");
pub const DEFAULT_CUSTOM_SCRIPT_SETUP_ETH_FALLBACK: &str =
    include_str!("../../../scripts/setup-eth-fallback.sh");
pub const P23_ADC_PEAK_TELEMETRY_ENABLE_FILE: &str =
    "/dev/shm/saturn_p23_adc_peak_telemetry.enabled";
pub const P23_ADC_PEAK_TELEMETRY_JSON_FILE: &str = "/dev/shm/saturn_p23_adc_peak_telemetry.json";
pub const P23_APP_PERF_TELEMETRY_JSON_FILE: &str = "/dev/shm/saturn_p23_perf_stats.json";

#[derive(Clone)]
pub struct AppState {
    pub webroot: PathBuf,
    pub config_path: PathBuf,
    pub custom_scripts_file: PathBuf,
    pub remote_settings_file: PathBuf,
    pub remote_profiles_file: PathBuf,
    pub scripts_dir: PathBuf,
    pub saturn_addr: String,
    pub bridge_ws_url: String,
    pub repo_root: Arc<RwLock<PathBuf>>,
    pub repo_root_file: PathBuf,
    pub update_policy_file: PathBuf,
    pub saturngo_update_policy_file: PathBuf,
    pub saturngo_deploy_status_file: PathBuf,
    pub update_state_file: PathBuf,
    pub snapshot_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub restore_max_upload_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CfgEntry {
    pub filename: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub directory: Option<String>,
    pub category: Option<String>,
    pub flags: Option<Vec<String>>,
    pub version: Option<String>,
}

#[derive(Clone)]
pub struct DefaultCustomScript {
    pub entry: CfgEntry,
    pub content: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct FlagsQuery {
    pub script: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunLogQuery {
    pub script: Option<String>,
    pub from: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteRadioPrefs {
    pub sample_rate: Option<u32>,
    pub rx_adc: Option<u8>,
    pub rx_antenna: Option<u8>,
    pub mode: Option<String>,
    pub rx_volume_db: Option<f64>,
    pub rx_noise_reduction_mode: Option<String>,
    pub rx_noise_reduction_level: Option<u8>,
    pub rx_nb_mode: Option<String>,
    pub rx_nb_threshold: Option<f64>,
    pub rx_anr_taps: Option<u16>,
    pub rx_anr_delay: Option<u16>,
    pub rx_anr_gain: Option<f64>,
    pub rx_anr_leakage: Option<f64>,
    pub anf_enabled: Option<bool>,
    pub rx_anf_taps: Option<u16>,
    pub rx_anf_delay: Option<u16>,
    pub rx_anf_gain: Option<f64>,
    pub rx_anf_leakage: Option<f64>,
    pub agc_mode: Option<String>,
    pub agc_gain: Option<u8>,
    pub filter_low: Option<i32>,
    pub filter_high: Option<i32>,
    pub tx_drive: Option<u8>,
    pub tx_mic_gain_db: Option<f64>,
    pub tx_filter_low: Option<i32>,
    pub tx_filter_high: Option<i32>,
    pub rx_eq_enabled: Option<bool>,
    pub rx_eq_bands: Vec<i32>,
    pub tx_eq_enabled: Option<bool>,
    pub tx_eq_bands: Vec<i32>,
    pub cfc_enabled: Option<bool>,
    pub cfc_precomp: Option<f64>,
    pub cfc_bands: Vec<f64>,
    pub tx_meter_mode: Option<String>,
    pub two_tone_enabled: Option<bool>,
    pub tx_two_tone_freq1: Option<f64>,
    pub tx_two_tone_freq2: Option<f64>,
    pub tx_two_tone_level_db: Option<f64>,
    pub tx_two_tone_invert_lsb: Option<bool>,
    pub tx_two_tone_delay_ms: Option<u16>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteDisplayPrefs {
    pub spectrum_auto_range: Option<bool>,
    pub spectrum_floor_db: Option<f64>,
    pub spectrum_ceiling_db: Option<f64>,
    pub waterfall_auto_range: Option<bool>,
    pub waterfall_floor_db: Option<f64>,
    pub waterfall_ceiling_db: Option<f64>,
    pub spectrum_average: Option<u8>,
    pub waterfall_speed: Option<u8>,
    pub waterfall_palette: Option<String>,
    pub spectrum_peak_hold: Option<bool>,
    pub show_grid: Option<bool>,
    pub show_center_line: Option<bool>,
    pub show_band_edges: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteProfileData {
    pub ws_url: Option<String>,
    pub display_zoom: Option<f64>,
    pub frequency_lock: Option<bool>,
    pub keep_screen_awake: Option<bool>,
    pub layout_mode: Option<String>,
    pub theme: Option<String>,
    pub phone_panels: BTreeMap<String, bool>,
    pub display_prefs: RemoteDisplayPrefs,
    pub radio_prefs: RemoteRadioPrefs,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteSettings {
    pub active_profile: Option<String>,
    pub ws_url: Option<String>,
    pub display_zoom: Option<f64>,
    pub frequency_lock: Option<bool>,
    pub keep_screen_awake: Option<bool>,
    pub layout_mode: Option<String>,
    pub theme: Option<String>,
    pub phone_panels: BTreeMap<String, bool>,
    pub display_prefs: RemoteDisplayPrefs,
    pub radio_prefs: RemoteRadioPrefs,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteProfilesFile {
    pub startup_profile: Option<String>,
    pub profiles: BTreeMap<String, RemoteProfileData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProfileSaveRequest {
    pub name: String,
    pub settings: RemoteProfileData,
    #[serde(default)]
    pub make_startup: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProfileDeleteRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteProfileStartupRequest {
    pub name: Option<String>,
}
