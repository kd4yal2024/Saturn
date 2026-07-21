use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, net::TcpStream, process::Command, time::timeout};

use crate::state::{AppState, CfgEntry, RemoteProfilesFile, RemoteSettings};
use crate::state_store::last_good_path;
use crate::update::UpdatePolicy;
use crate::util::{is_saturn_repo_root, parse_boolish};

pub const BUILD_COMMIT: &str = env!("SATURN_BUILD_COMMIT");
const DEFAULT_READY_MIN_FREE_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_READY_CONNECT_TIMEOUT_MS: u64 = 1500;

#[derive(Debug, Serialize)]
struct LiveResponse {
    status: &'static str,
    build_commit: &'static str,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReadyQuery {
    expected_commit: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    ready: bool,
    build_commit: &'static str,
    expected_commit: String,
    expected_commit_source: &'static str,
    components: BTreeMap<String, HealthComponent>,
}

#[derive(Debug, Serialize)]
struct HealthComponent {
    status: &'static str,
    required: bool,
    message: String,
    details: Value,
}

impl HealthComponent {
    fn ok(required: bool, message: impl Into<String>, details: Value) -> Self {
        Self {
            status: "ok",
            required,
            message: message.into(),
            details,
        }
    }

    fn warning(message: impl Into<String>, details: Value) -> Self {
        Self {
            status: "warning",
            required: false,
            message: message.into(),
            details,
        }
    }

    fn error(required: bool, message: impl Into<String>, details: Value) -> Self {
        Self {
            status: "error",
            required,
            message: message.into(),
            details,
        }
    }
}

pub async fn livez() -> Response {
    (
        StatusCode::OK,
        Json(LiveResponse {
            status: "alive",
            build_commit: BUILD_COMMIT,
        }),
    )
        .into_response()
}

/// Compatibility alias retained while installers and watchdogs migrate.
pub async fn healthz() -> Response {
    livez().await
}

pub async fn readyz(Query(query): Query<ReadyQuery>, State(state): State<AppState>) -> Response {
    let (expected_commit, expected_source) = expected_commit(query.expected_commit);
    let mut components = BTreeMap::new();

    components.insert(
        "release_identity".to_string(),
        release_identity_component(&expected_commit, expected_source),
    );
    components.insert("state".to_string(), state_component(&state).await);
    components.insert(
        "state_documents".to_string(),
        state_documents_component(&state).await,
    );
    components.insert("configuration".to_string(), config_component(&state).await);
    components.insert("disk".to_string(), disk_component(&state).await);
    components.insert("bridge".to_string(), bridge_component(&state).await);
    components.insert("p2app".to_string(), p2_component().await);
    components.insert("xdma".to_string(), xdma_component());

    let ready = required_components_ready(&components);
    let status = if ready { "ready" } else { "not_ready" };
    let http_status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        http_status,
        Json(ReadyResponse {
            status,
            ready,
            build_commit: BUILD_COMMIT,
            expected_commit,
            expected_commit_source: expected_source,
            components,
        }),
    )
        .into_response()
}

fn required_components_ready(components: &BTreeMap<String, HealthComponent>) -> bool {
    components
        .values()
        .all(|component| !component.required || component.status == "ok")
}

fn normalized_commit(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

fn expected_commit(query_commit: Option<String>) -> (String, &'static str) {
    if let Some(value) = query_commit {
        return (value.trim().to_ascii_lowercase(), "query");
    }
    if let Ok(value) = std::env::var("SATURN_EXPECTED_COMMIT") {
        if !value.trim().is_empty() {
            return (value.trim().to_ascii_lowercase(), "environment");
        }
    }
    (BUILD_COMMIT.to_string(), "embedded_build")
}

fn release_identity_component(expected: &str, source: &'static str) -> HealthComponent {
    let Some(build) = normalized_commit(BUILD_COMMIT) else {
        return HealthComponent::error(
            true,
            "running binary does not contain a full Git commit",
            json!({ "buildCommit": BUILD_COMMIT, "expectedSource": source }),
        );
    };
    let Some(expected) = normalized_commit(expected) else {
        return HealthComponent::error(
            true,
            "expected commit must be a full 40-character Git commit",
            json!({ "buildCommit": build, "expectedCommit": expected, "expectedSource": source }),
        );
    };
    if build != expected {
        return HealthComponent::error(
            true,
            "running release does not match the expected commit",
            json!({ "buildCommit": build, "expectedCommit": expected, "expectedSource": source }),
        );
    }
    HealthComponent::ok(
        true,
        "running release matches the expected commit",
        json!({ "buildCommit": build, "expectedCommit": expected, "expectedSource": source }),
    )
}

fn state_dir(state: &AppState) -> Option<PathBuf> {
    state.repo_root_file.parent().map(Path::to_path_buf)
}

async fn state_component(state: &AppState) -> HealthComponent {
    let Some(dir) = state_dir(state) else {
        return HealthComponent::error(true, "state path has no parent directory", json!({}));
    };
    match tokio::fs::metadata(&dir).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return HealthComponent::error(
                true,
                "mandatory state path is not a directory",
                json!({ "path": dir }),
            )
        }
        Err(error) => {
            return HealthComponent::error(
                true,
                format!("mandatory state directory is unavailable: {error}"),
                json!({ "path": dir }),
            )
        }
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let probe = dir.join(format!(".saturn-ready-{}-{nanos}", std::process::id()));
    let result = async {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .await?;
        file.write_all(b"ready\n").await?;
        file.sync_data().await
    }
    .await;
    let _ = tokio::fs::remove_file(&probe).await;

    match result {
        Ok(()) => HealthComponent::ok(
            true,
            "mandatory state directory is writable",
            json!({ "path": dir }),
        ),
        Err(error) => HealthComponent::error(
            true,
            format!("mandatory state directory is not writable: {error}"),
            json!({ "path": dir }),
        ),
    }
}

const MAX_READY_STATE_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

async fn validate_json_document<T: DeserializeOwned>(path: &Path) -> Result<(), String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "state document is not a regular file: {}",
            path.display()
        ));
    }
    if metadata.len() > MAX_READY_STATE_DOCUMENT_BYTES {
        return Err(format!(
            "state document exceeds {} bytes: {}",
            MAX_READY_STATE_DOCUMENT_BYTES,
            path.display()
        ));
    }
    let raw = tokio::fs::read(path)
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice::<T>(&raw)
        .map(|_| ())
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

async fn validate_optional_json_document<T: DeserializeOwned>(path: &Path) -> Result<bool, String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => validate_json_document::<T>(path).await.map(|_| true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

async fn validate_optional_json_object(path: &Path) -> Result<bool, String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => {
            let metadata = tokio::fs::symlink_metadata(path)
                .await
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "state document is not a regular file: {}",
                    path.display()
                ));
            }
            if metadata.len() > MAX_READY_STATE_DOCUMENT_BYTES {
                return Err(format!(
                    "state document exceeds {} bytes: {}",
                    MAX_READY_STATE_DOCUMENT_BYTES,
                    path.display()
                ));
            }
            let raw = tokio::fs::read(path)
                .await
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let value = serde_json::from_slice::<Value>(&raw)
                .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
            if !value.is_object() {
                return Err(format!(
                    "state document must contain a JSON object: {}",
                    path.display()
                ));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

async fn state_documents_component(state: &AppState) -> HealthComponent {
    let mut checked = Vec::new();
    let mut absent_optional = Vec::new();
    let mut errors = Vec::new();
    let mut last_good = Vec::new();

    let required_json = [
        (&state.custom_scripts_file, "custom_scripts", 0_u8),
        (&state.update_policy_file, "update_policy", 1_u8),
        (
            &state.saturngo_update_policy_file,
            "saturngo_update_policy",
            1_u8,
        ),
    ];
    for (path, name, kind) in required_json {
        let result = match kind {
            0 => validate_json_document::<Vec<CfgEntry>>(path).await,
            _ => validate_json_document::<UpdatePolicy>(path).await,
        };
        match result {
            Ok(()) => checked.push(name),
            Err(error) => errors.push(error),
        }
        if last_good_path(path).is_ok_and(|candidate| candidate.is_file()) {
            last_good.push(name);
        }
    }

    match tokio::fs::read_to_string(&state.repo_root_file).await {
        Ok(raw) if !raw.trim().is_empty() && Path::new(raw.trim()).is_absolute() => {
            match tokio::fs::canonicalize(raw.trim()).await {
                Ok(canonical) if is_saturn_repo_root(&canonical) => checked.push("repo_root"),
                _ => errors.push(format!(
                    "repository pointer does not identify a Saturn checkout: {}",
                    state.repo_root_file.display()
                )),
            }
        }
        Ok(_) => errors.push(format!(
            "repository pointer must contain one absolute path: {}",
            state.repo_root_file.display()
        )),
        Err(error) => errors.push(format!(
            "cannot read repository pointer {}: {error}",
            state.repo_root_file.display()
        )),
    }
    if last_good_path(&state.repo_root_file).is_ok_and(|candidate| candidate.is_file()) {
        last_good.push("repo_root");
    }

    macro_rules! optional_json {
        ($path:expr, $name:literal, $kind:ty) => {
            match validate_optional_json_document::<$kind>($path).await {
                Ok(true) => checked.push($name),
                Ok(false) => absent_optional.push($name),
                Err(error) => errors.push(error),
            }
        };
    }
    optional_json!(
        &state.remote_settings_file,
        "remote_settings",
        RemoteSettings
    );
    optional_json!(
        &state.remote_profiles_file,
        "remote_profiles",
        RemoteProfilesFile
    );
    for (path, name) in [
        (&state.update_state_file, "update_history"),
        (&state.saturngo_deploy_status_file, "saturngo_deploy_status"),
    ] {
        match validate_optional_json_object(path).await {
            Ok(true) => checked.push(name),
            Ok(false) => absent_optional.push(name),
            Err(error) => errors.push(error),
        }
    }
    if let Some(root) = state_dir(state) {
        let schema = root.join("state-schema.json");
        match validate_optional_json_object(&schema).await {
            Ok(true) => checked.push("state_schema"),
            Ok(false) => absent_optional.push("state_schema"),
            Err(error) => errors.push(error),
        }
    }

    let details = json!({
        "checked": checked,
        "absentOptional": absent_optional,
        "lastKnownGood": last_good,
        "errors": errors,
    });
    if errors.is_empty() {
        HealthComponent::ok(
            true,
            "mandatory state documents parsed successfully",
            details,
        )
    } else {
        HealthComponent::error(
            true,
            "mandatory state contains missing or malformed documents",
            details,
        )
    }
}

async fn config_component(state: &AppState) -> HealthComponent {
    let raw = match tokio::fs::read_to_string(&state.config_path).await {
        Ok(raw) => raw,
        Err(error) => {
            return HealthComponent::error(
                true,
                format!("required configuration cannot be read: {error}"),
                json!({ "path": state.config_path }),
            )
        }
    };
    match serde_json::from_str::<Vec<CfgEntry>>(&raw) {
        Ok(entries) => HealthComponent::ok(
            true,
            "required configuration parsed successfully",
            json!({ "path": state.config_path, "entries": entries.len() }),
        ),
        Err(error) => HealthComponent::error(
            true,
            format!("required configuration is invalid: {error}"),
            json!({ "path": state.config_path }),
        ),
    }
}

fn ready_min_free_bytes() -> Result<u64, String> {
    match std::env::var("SATURN_READY_MIN_FREE_BYTES") {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            format!("SATURN_READY_MIN_FREE_BYTES must be an unsigned integer, got: {value}")
        }),
        Err(_) => Ok(DEFAULT_READY_MIN_FREE_BYTES),
    }
}

async fn available_bytes_at(path: &Path) -> Result<u64, String> {
    let output = Command::new("df")
        .arg("-B1")
        .arg("--output=avail")
        .arg(path)
        .output()
        .await
        .map_err(|error| format!("failed to run df: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)
        .ok_or_else(|| "df did not return available bytes".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid available-byte value from df: {error}"))
}

async fn disk_component(state: &AppState) -> HealthComponent {
    let Some(dir) = state_dir(state) else {
        return HealthComponent::error(true, "state path has no parent directory", json!({}));
    };
    let minimum = match ready_min_free_bytes() {
        Ok(value) => value,
        Err(error) => return HealthComponent::error(true, error, json!({ "path": dir })),
    };
    match available_bytes_at(&dir).await {
        Ok(available) if available >= minimum => HealthComponent::ok(
            true,
            "free disk space exceeds the readiness threshold",
            json!({ "path": dir, "availableBytes": available, "minimumBytes": minimum }),
        ),
        Ok(available) => HealthComponent::error(
            true,
            "free disk space is below the readiness threshold",
            json!({ "path": dir, "availableBytes": available, "minimumBytes": minimum }),
        ),
        Err(error) => HealthComponent::error(
            true,
            format!("free disk space could not be measured: {error}"),
            json!({ "path": dir, "minimumBytes": minimum }),
        ),
    }
}

fn bridge_authority(value: &str) -> Option<(String, u16)> {
    let (remainder, default_port) = value
        .strip_prefix("ws://")
        .map(|value| (value, 80))
        .or_else(|| value.strip_prefix("wss://").map(|value| (value, 443)))?;
    let authority = remainder.split('/').next()?.trim();
    if authority.is_empty() {
        return None;
    }
    if let Some(remainder) = authority.strip_prefix('[') {
        let end = remainder.find(']')?;
        let host = remainder[..end].to_string();
        let suffix = &remainder[end + 1..];
        let port = if suffix.is_empty() {
            default_port
        } else {
            suffix.strip_prefix(':')?.parse::<u16>().ok()?
        };
        return Some((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            Some((host.to_string(), port.parse::<u16>().ok()?))
        }
        _ => Some((authority.to_string(), default_port)),
    }
}

fn ready_connect_timeout() -> Result<Duration, String> {
    let milliseconds = match std::env::var("SATURN_READY_CONNECT_TIMEOUT_MS") {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            format!("SATURN_READY_CONNECT_TIMEOUT_MS must be an unsigned integer, got: {value}")
        })?,
        Err(_) => DEFAULT_READY_CONNECT_TIMEOUT_MS,
    };
    Ok(Duration::from_millis(milliseconds.max(1)))
}

fn bridge_required(value: Option<String>) -> bool {
    match value {
        Some(value) => parse_boolish(Some(value)),
        None => true,
    }
}

async fn bridge_component(state: &AppState) -> HealthComponent {
    let required = bridge_required(std::env::var("SATURN_READY_REQUIRE_BRIDGE").ok());
    let Some((host, port)) = bridge_authority(&state.bridge_ws_url) else {
        return HealthComponent::error(
            required,
            "bridge WebSocket URL is invalid",
            json!({ "url": state.bridge_ws_url }),
        );
    };
    let connect_timeout = match ready_connect_timeout() {
        Ok(value) => value,
        Err(error) => {
            return HealthComponent::error(required, error, json!({ "host": host, "port": port }))
        }
    };
    match timeout(connect_timeout, TcpStream::connect((host.as_str(), port))).await {
        Ok(Ok(_stream)) => HealthComponent::ok(
            required,
            "Saturn Bridge TCP listener is reachable",
            json!({ "host": host, "port": port }),
        ),
        Ok(Err(error)) => HealthComponent::error(
            required,
            format!("Saturn Bridge TCP listener is unavailable: {error}"),
            json!({ "host": host, "port": port }),
        ),
        Err(_) => HealthComponent::error(
            required,
            "Saturn Bridge TCP connection timed out",
            json!({ "host": host, "port": port, "timeoutMs": connect_timeout.as_millis() }),
        ),
    }
}

async fn p2_component() -> HealthComponent {
    let required = parse_boolish(std::env::var("SATURN_READY_REQUIRE_P2").ok());
    let result = timeout(
        Duration::from_secs(2),
        Command::new("systemctl")
            .args(["is-active", "p2app.service"])
            .output(),
    )
    .await;
    match result {
        Ok(Ok(output)) if output.status.success() => HealthComponent::ok(
            required,
            "p2app.service is active",
            json!({ "service": "p2app.service" }),
        ),
        Ok(Ok(output)) => {
            let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if required {
                HealthComponent::error(
                    true,
                    "p2app.service is not active",
                    json!({ "service": "p2app.service", "state": state }),
                )
            } else {
                HealthComponent::warning(
                    "p2app.service is not active; radio state does not block application deployment",
                    json!({ "service": "p2app.service", "state": state }),
                )
            }
        }
        Ok(Err(error)) => {
            let message = format!("p2app.service state could not be queried: {error}");
            if required {
                HealthComponent::error(true, message, json!({ "service": "p2app.service" }))
            } else {
                HealthComponent::warning(message, json!({ "service": "p2app.service" }))
            }
        }
        Err(_) => {
            if required {
                HealthComponent::error(
                    true,
                    "p2app.service state query timed out",
                    json!({ "service": "p2app.service" }),
                )
            } else {
                HealthComponent::warning(
                    "p2app.service state query timed out",
                    json!({ "service": "p2app.service" }),
                )
            }
        }
    }
}

fn xdma_component() -> HealthComponent {
    const DEVICES: [&str; 2] = ["/dev/xdma0_user", "/dev/xdma0_control"];
    let present: Vec<&str> = DEVICES
        .iter()
        .copied()
        .filter(|path| Path::new(path).exists())
        .collect();
    if present.is_empty() {
        HealthComponent::warning(
            "XDMA device is not present; hardware state does not block application deployment",
            json!({ "checked": DEVICES }),
        )
    } else {
        HealthComponent::ok(
            false,
            "XDMA device is present",
            json!({ "present": present }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        routing::get,
        Router,
    };
    use std::sync::{Arc, RwLock};
    use tower::ServiceExt;

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "saturn-health-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn test_state(tmp: &Path, bridge_ws_url: String) -> AppState {
        AppState {
            webroot: tmp.to_path_buf(),
            config_path: tmp.join("config.json"),
            custom_scripts_file: tmp.join("custom_scripts.json"),
            remote_settings_file: tmp.join("remote_settings.json"),
            remote_profiles_file: tmp.join("remote_profiles.json"),
            scripts_dir: tmp.join("scripts"),
            saturn_addr: "127.0.0.1:8080".to_string(),
            bridge_ws_url,
            repo_root: Arc::new(RwLock::new(tmp.to_path_buf())),
            repo_root_file: tmp.join("repo_root"),
            update_policy_file: tmp.join("update_policy.json"),
            saturngo_update_policy_file: tmp.join("saturngo_policy.json"),
            saturngo_deploy_status_file: tmp.join("saturngo_deploy.json"),
            update_state_file: tmp.join("update_state.json"),
            snapshot_dir: tmp.join("snapshots"),
            staging_dir: tmp.join("staging"),
            restore_max_upload_bytes: 2 * 1024 * 1024 * 1024,
        }
    }

    #[test]
    fn parses_bridge_authorities() {
        assert_eq!(
            bridge_authority("ws://127.0.0.1:50001/control"),
            Some(("127.0.0.1".to_string(), 50001))
        );
        assert_eq!(
            bridge_authority("wss://[::1]:8443/socket"),
            Some(("::1".to_string(), 8443))
        );
        assert_eq!(
            bridge_authority("ws://localhost/socket"),
            Some(("localhost".to_string(), 80))
        );
        assert_eq!(bridge_authority("http://127.0.0.1:50001"), None);
    }

    #[test]
    fn bridge_is_required_by_default_and_can_be_explicitly_optional() {
        assert!(bridge_required(None));
        assert!(bridge_required(Some("1".to_string())));
        assert!(bridge_required(Some("true".to_string())));
        assert!(!bridge_required(Some("0".to_string())));
        assert!(!bridge_required(Some("false".to_string())));
    }

    #[test]
    fn release_identity_requires_full_matching_commit() {
        assert_eq!(
            release_identity_component(BUILD_COMMIT, "test").status,
            "ok"
        );
        assert_eq!(release_identity_component("abc", "test").status, "error");
        assert_eq!(
            release_identity_component("0000000000000000000000000000000000000000", "test").status,
            "error"
        );
    }

    #[tokio::test]
    async fn livez_is_independent_of_dependencies() {
        let app = Router::new().route("/livez", get(livez));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/livez")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "alive");
        assert_eq!(value["build_commit"], BUILD_COMMIT);
    }

    #[test]
    fn only_required_component_failures_block_readiness() {
        let mut components = BTreeMap::new();
        components.insert(
            "release".to_string(),
            HealthComponent::ok(true, "matched", json!({})),
        );
        components.insert(
            "hardware".to_string(),
            HealthComponent::warning("temporarily absent", json!({})),
        );
        assert!(required_components_ready(&components));

        components.insert(
            "bridge".to_string(),
            HealthComponent::error(true, "unreachable", json!({})),
        );
        assert!(!required_components_ready(&components));
    }

    #[tokio::test]
    async fn readyz_rejects_wrong_commit_even_when_dependencies_are_up() {
        let tmp = test_dir("wrong-commit");
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        tokio::fs::write(tmp.join("config.json"), b"[]\n")
            .await
            .unwrap();
        let state = test_state(&tmp, "ws://127.0.0.1:1".to_string());
        let app = Router::new()
            .route("/readyz", get(readyz))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/readyz?expected_commit=0000000000000000000000000000000000000000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["components"]["release_identity"]["status"], "error");
        let _ = tokio::fs::remove_dir_all(tmp).await;
    }

    #[tokio::test]
    async fn malformed_mandatory_state_blocks_readiness_without_replacing_it() {
        let tmp = test_dir("malformed-state");
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        let state = test_state(&tmp, "ws://127.0.0.1:1".to_string());
        tokio::fs::write(&state.repo_root_file, b"/tmp\n")
            .await
            .unwrap();
        tokio::fs::write(&state.custom_scripts_file, b"[]\n")
            .await
            .unwrap();
        let policy = serde_json::to_vec_pretty(&UpdatePolicy::default()).unwrap();
        tokio::fs::write(&state.update_policy_file, b"{broken-json\n")
            .await
            .unwrap();
        tokio::fs::write(&state.saturngo_update_policy_file, policy)
            .await
            .unwrap();

        let component = state_documents_component(&state).await;
        assert_eq!(component.status, "error");
        assert!(component.required);
        assert!(component.details["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error.as_str().unwrap().contains("update_policy.json")));
        assert_eq!(
            tokio::fs::read(&state.update_policy_file).await.unwrap(),
            b"{broken-json\n"
        );
        let _ = tokio::fs::remove_dir_all(tmp).await;
    }
}
