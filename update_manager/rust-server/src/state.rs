use serde::{Deserialize, Serialize};
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
pub const CSRF_HEADER_NAME: &str = "x-saturn-csrf";
pub const CSRF_HEADER_VALUE: &str = "1";
pub const RUN_LOG_MAX_LINES: usize = 5000;
pub const RUN_LOG_FETCH_MAX_LINES: usize = 1000;
pub const MAX_COMPLETED_JOBS: usize = 20;
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
pub const P23_ADC_PEAK_TELEMETRY_JSON_FILE: &str =
    "/dev/shm/saturn_p23_adc_peak_telemetry.json";
pub const P23_APP_PERF_TELEMETRY_JSON_FILE: &str =
    "/dev/shm/saturn_p23_perf_stats.json";

#[derive(Clone)]
pub struct AppState {
    pub webroot: PathBuf,
    pub config_path: PathBuf,
    pub custom_scripts_file: PathBuf,
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

#[derive(Deserialize)]
pub struct PiImageStatusQuery {
    pub job_id: String,
}
