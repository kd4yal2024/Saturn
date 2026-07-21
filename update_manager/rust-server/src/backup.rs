use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncBufReadExt, io::BufReader, process::Command};
use tokio_util::io::ReaderStream;
use tracing::error;

use crate::health::BUILD_COMMIT;
use crate::state::{AppState, CfgEntry, MAX_CUSTOM_SCRIPTS_FILE_BYTES};
use crate::util::{
    backup_home_dir, current_repo_root, is_safe_custom_script_filename, json_error,
    pihpsdr_repo_root, validate_saturn_repo_root,
};

const SETTINGS_BACKUP_FORMAT: &str = "saturn-settings-backup";
const SETTINGS_BACKUP_SCHEMA_VERSION: u32 = 1;
const SETTINGS_ARCHIVE_ROOT: &str = "saturn-settings-v1";
const MAX_SETTINGS_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SETTINGS_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_RELEASES_ROOT: &str = "/opt/saturn/releases";
const DEFAULT_CURRENT_RELEASE: &str = "/opt/saturn/current";
const RELEASE_MANIFEST_NAME: &str = "release-manifest.json";

#[derive(Debug, Clone)]
struct SettingsBackupSources {
    state_schema_file: PathBuf,
    custom_scripts_file: PathBuf,
    remote_settings_file: PathBuf,
    remote_profiles_file: PathBuf,
    repo_root_file: PathBuf,
    update_policy_file: PathBuf,
    saturngo_update_policy_file: PathBuf,
    scripts_dir: PathBuf,
    pihpsdr_root: PathBuf,
    deskhpsdr_config_root: PathBuf,
}

impl SettingsBackupSources {
    fn from_state(state: &AppState) -> Result<Self, String> {
        let state_root = state
            .repo_root_file
            .parent()
            .ok_or_else(|| "Saturn state root is unavailable".to_string())?;
        Ok(Self {
            state_schema_file: state_root.join("state-schema.json"),
            custom_scripts_file: state.custom_scripts_file.clone(),
            remote_settings_file: state.remote_settings_file.clone(),
            remote_profiles_file: state.remote_profiles_file.clone(),
            repo_root_file: state.repo_root_file.clone(),
            update_policy_file: state.update_policy_file.clone(),
            saturngo_update_policy_file: state.saturngo_update_policy_file.clone(),
            scripts_dir: state.scripts_dir.clone(),
            pihpsdr_root: pihpsdr_repo_root(),
            deskhpsdr_config_root: backup_home_dir().join(".config/deskhpsdr"),
        })
    }
}

#[derive(Debug, Serialize)]
struct SettingsBackupManifest {
    format: &'static str,
    schema_version: u32,
    created_at: String,
    saturn_go_build_commit: &'static str,
    portability: &'static str,
    sensitivity: &'static str,
    files: Vec<SettingsBackupFile>,
    omitted: Vec<&'static str>,
    restore_policy: &'static str,
}

#[derive(Debug, Serialize)]
struct SettingsBackupFile {
    inventory_id: &'static str,
    archive_path: String,
    size: u64,
    mode: u32,
    sha256: String,
}

fn private_temp_dir(prefix: &str) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..64u32 {
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique settings-backup directory",
    ))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn copy_settings_file(
    source: &Path,
    archive_root: &Path,
    relative: &Path,
    inventory_id: &'static str,
    files: &mut Vec<SettingsBackupFile>,
) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect {}: {error}", source.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "settings source must be a regular file: {}",
            source.display()
        ));
    }
    if metadata.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(format!(
            "settings source exceeds {} bytes: {}",
            MAX_SETTINGS_FILE_BYTES,
            source.display()
        ));
    }
    let current_total = files.iter().map(|file| file.size).sum::<u64>();
    let remaining_total = MAX_SETTINGS_TOTAL_BYTES.saturating_sub(current_total);
    if metadata.len() > remaining_total {
        return Err(format!(
            "settings backup exceeds {} bytes",
            MAX_SETTINGS_TOTAL_BYTES
        ));
    }
    let input = fs::File::open(source)
        .map_err(|error| format!("cannot open {}: {error}", source.display()))?;
    let opened_metadata = input
        .metadata()
        .map_err(|error| format!("cannot inspect open file {}: {error}", source.display()))?;
    if !opened_metadata.is_file()
        || opened_metadata.dev() != metadata.dev()
        || opened_metadata.ino() != metadata.ino()
    {
        return Err(format!(
            "settings source changed while it was being opened: {}",
            source.display()
        ));
    }
    let destination = archive_root.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot secure {}: {error}", parent.display()))?;
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let copy_limit = MAX_SETTINGS_FILE_BYTES.min(remaining_total);
    let mut limited_input = input.take(copy_limit + 1);
    let copied = io::copy(&mut limited_input, &mut output).map_err(|error| {
        format!(
            "cannot copy settings file {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    if copied > copy_limit {
        drop(output);
        let _ = fs::remove_file(&destination);
        return Err(format!(
            "settings source grew beyond the backup limit while being copied: {}",
            source.display()
        ));
    }
    output
        .sync_all()
        .map_err(|error| format!("cannot flush {}: {error}", destination.display()))?;
    files.push(SettingsBackupFile {
        inventory_id,
        archive_path: relative.to_string_lossy().to_string(),
        size: copied,
        mode: metadata.permissions().mode() & 0o7777,
        sha256: sha256_file(&destination)
            .map_err(|error| format!("cannot hash {}: {error}", destination.display()))?,
    });
    Ok(())
}

fn regular_props(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot list {}: {error}", root.display())),
    };
    let mut props = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", root.display()))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("props") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "radio property source must be a regular file: {}",
                path.display()
            ));
        }
        props.push(path);
    }
    props.sort();
    Ok(props)
}

fn operator_custom_script_names(registry: &Path) -> Result<Vec<String>, String> {
    let metadata = match fs::symlink_metadata(registry) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot inspect {}: {error}", registry.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "custom script registry must be a regular file: {}",
            registry.display()
        ));
    }
    if metadata.len() > MAX_CUSTOM_SCRIPTS_FILE_BYTES {
        return Err(format!(
            "custom script registry exceeds {} bytes",
            MAX_CUSTOM_SCRIPTS_FILE_BYTES
        ));
    }
    let raw = fs::read_to_string(registry)
        .map_err(|error| format!("cannot read {}: {error}", registry.display()))?;
    let entries: Vec<CfgEntry> = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid custom script registry: {error}"))?;
    let mut names = Vec::new();
    for entry in entries {
        if entry.version.as_deref() == Some("custom-default") {
            continue;
        }
        if !is_safe_custom_script_filename(&entry.filename) {
            return Err(format!(
                "custom script registry contains an unsafe filename: {}",
                entry.filename
            ));
        }
        names.push(entry.filename);
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn build_settings_backup_tree(
    sources: &SettingsBackupSources,
) -> Result<(PathBuf, PathBuf), String> {
    let temp_root = private_temp_dir("saturn-settings-backup")
        .map_err(|error| format!("cannot create settings-backup staging directory: {error}"))?;
    let archive_root = temp_root.join(SETTINGS_ARCHIVE_ROOT);
    fs::create_dir(&archive_root)
        .map_err(|error| format!("cannot create {}: {error}", archive_root.display()))?;
    fs::set_permissions(&archive_root, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("cannot secure {}: {error}", archive_root.display()))?;

    let result = (|| {
        let mut files = Vec::new();
        let state_files = [
            (
                &sources.state_schema_file,
                "saturn-state/state-schema.json",
                "saturn-go-state-schema",
            ),
            (
                &sources.custom_scripts_file,
                "saturn-state/custom_scripts.json",
                "custom-script-registry",
            ),
            (
                &sources.remote_settings_file,
                "saturn-state/remote_settings.json",
                "remote-radio-settings",
            ),
            (
                &sources.remote_profiles_file,
                "saturn-state/remote_profiles.json",
                "remote-radio-profiles",
            ),
            (
                &sources.repo_root_file,
                "saturn-state/repo_root.txt",
                "update-and-repository-policy",
            ),
            (
                &sources.update_policy_file,
                "saturn-state/update_policy.json",
                "update-and-repository-policy",
            ),
            (
                &sources.saturngo_update_policy_file,
                "saturn-state/saturngo_update_policy.json",
                "update-and-repository-policy",
            ),
        ];
        for (source, relative, inventory_id) in state_files {
            copy_settings_file(
                source,
                &archive_root,
                Path::new(relative),
                inventory_id,
                &mut files,
            )?;
        }

        for source in regular_props(&sources.pihpsdr_root)? {
            let name = source
                .file_name()
                .ok_or_else(|| format!("invalid piHPSDR property path: {}", source.display()))?;
            copy_settings_file(
                &source,
                &archive_root,
                &Path::new("clients/pihpsdr").join(name),
                "pihpsdr-radio-properties",
                &mut files,
            )?;
        }
        for source in regular_props(&sources.deskhpsdr_config_root)? {
            let name = source
                .file_name()
                .ok_or_else(|| format!("invalid deskHPSDR property path: {}", source.display()))?;
            copy_settings_file(
                &source,
                &archive_root,
                &Path::new("clients/deskhpsdr").join(name),
                "deskhpsdr-radio-properties",
                &mut files,
            )?;
        }

        for name in operator_custom_script_names(&sources.custom_scripts_file)? {
            let source = sources.scripts_dir.join(&name);
            if !source.exists() {
                return Err(format!(
                    "registered operator script is missing: {}",
                    source.display()
                ));
            }
            copy_settings_file(
                &source,
                &archive_root,
                &Path::new("custom-scripts").join(&name),
                "custom-script-content",
                &mut files,
            )?;
        }
        files.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));

        let manifest = SettingsBackupManifest {
            format: SETTINGS_BACKUP_FORMAT,
            schema_version: SETTINGS_BACKUP_SCHEMA_VERSION,
            created_at: Utc::now().to_rfc3339(),
            saturn_go_build_commit: BUILD_COMMIT,
            portability: "portable-with-radio-and-path-review",
            sensitivity: "private-operator-backup; not a support bundle",
            files,
            omitted: vec![
                "administrator and Linux credentials",
                "initial-login recovery material",
                "remembered-device cookie secret",
                "TLS and SSH private keys",
                "Tailscale and NetworkManager/Wi-Fi identity",
                "machine identity and hostname",
                "boot, LCD, front-panel, and device provisioning state",
                "deployment transactions and logs",
                "source trees, installed releases, caches, and staging",
                "FPGA hardware contents",
            ],
            restore_policy:
                "Import only through Saturn Go transactional settings restore; do not unpack over live state",
        };
        let manifest_path = archive_root.join("manifest.json");
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("cannot serialize settings manifest: {error}"))?;
        let mut manifest_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&manifest_path)
            .map_err(|error| format!("cannot create {}: {error}", manifest_path.display()))?;
        manifest_file
            .write_all(&bytes)
            .and_then(|_| manifest_file.write_all(b"\n"))
            .map_err(|error| format!("cannot write {}: {error}", manifest_path.display()))?;
        manifest_file
            .sync_all()
            .map_err(|error| format!("cannot flush {}: {error}", manifest_path.display()))?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temp_root);
        return Err(error);
    }
    Ok((temp_root, archive_root))
}

fn attachment_headers(filename: &str, backup_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    headers.insert(
        HeaderName::from_static("x-saturn-backup-type"),
        HeaderValue::from_static(backup_type),
    );
    headers
}

async fn stream_tar(
    parent: PathBuf,
    base: String,
    filename: String,
    backup_type: &'static str,
    cleanup: Option<PathBuf>,
) -> Result<Response, Response> {
    let mut command = Command::new("tar");
    command
        .arg("-C")
        .arg(&parent)
        .arg("-czf")
        .arg("-")
        .arg("--")
        .arg(&base)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some(cleanup) = cleanup.as_ref() {
                let _ = tokio::fs::remove_dir_all(cleanup).await;
            }
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &error.to_string(),
            ));
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            if let Some(cleanup) = cleanup.as_ref() {
                let _ = tokio::fs::remove_dir_all(cleanup).await;
            }
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "backup tar stdout is unavailable",
            ));
        }
    };
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        if let Some(stderr) = stderr {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                error!("backup tar stderr: {line}");
            }
        }
        match child.wait().await {
            Ok(status) if !status.success() => error!("backup tar exited with status {status}"),
            Err(error) => error!("backup tar wait failed: {error}"),
            _ => {}
        }
        if let Some(cleanup) = cleanup {
            if let Err(error) = tokio::fs::remove_dir_all(&cleanup).await {
                error!(
                    "cannot remove backup staging {}: {error}",
                    cleanup.display()
                );
            }
        }
    });
    let body = Body::from_stream(ReaderStream::new(stdout));
    Ok((attachment_headers(&filename, backup_type), body).into_response())
}

pub async fn backup_settings(State(state): State<AppState>) -> Result<Response, Response> {
    let sources = SettingsBackupSources::from_state(&state)
        .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error))?;
    let (temp_root, archive_root) =
        tokio::task::spawn_blocking(move || build_settings_backup_tree(&sources))
            .await
            .map_err(|error| {
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("settings backup staging task failed: {error}"),
                )
            })?
            .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error))?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    stream_tar(
        temp_root.clone(),
        archive_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(SETTINGS_ARCHIVE_ROOT)
            .to_string(),
        format!("saturn-settings-{timestamp}.tar.gz"),
        "settings-v1",
        Some(temp_root),
    )
    .await
}

pub async fn backup_source(State(state): State<AppState>) -> Result<Response, Response> {
    let repo_root = current_repo_root(&state);
    validate_saturn_repo_root(&repo_root)
        .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?;
    let metadata = fs::symlink_metadata(&repo_root).map_err(|error| {
        json_error(
            StatusCode::BAD_REQUEST,
            &format!("cannot inspect active repository: {error}"),
        )
    })?;
    if !metadata.file_type().is_dir() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "active repository must be a real directory",
        ));
    }
    let parent = repo_root.parent().unwrap_or(Path::new("/")).to_path_buf();
    let base = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Saturn")
        .to_string();
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    stream_tar(
        parent,
        base,
        format!("saturn-source-{timestamp}.tar.gz"),
        "source-repository",
        None,
    )
    .await
}

#[derive(Debug, Deserialize, Default)]
pub struct ReleaseBackupQuery {
    commit: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReleaseBackupEntry {
    commit: String,
    active: bool,
    manifest_present: bool,
}

fn normalized_full_commit(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

fn releases_root() -> PathBuf {
    std::env::var("SATURN_RELEASES_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_RELEASES_ROOT))
}

fn current_release_link() -> PathBuf {
    std::env::var("SATURN_RELEASE_CURRENT_LINK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CURRENT_RELEASE))
}

fn active_release_commit(root: &Path) -> Option<String> {
    let target = fs::read_link(current_release_link()).ok()?;
    let absolute = if target.is_absolute() {
        target
    } else {
        current_release_link().parent()?.join(target)
    };
    let canonical_root = fs::canonicalize(root).ok()?;
    let canonical_target = fs::canonicalize(absolute).ok()?;
    if canonical_target.parent()? != canonical_root {
        return None;
    }
    normalized_full_commit(canonical_target.file_name()?.to_str()?)
}

fn resolve_installed_release(root: &Path, commit: &str) -> Result<PathBuf, String> {
    let commit = normalized_full_commit(commit).ok_or_else(|| {
        "release commit must be a full 40-character hexadecimal commit".to_string()
    })?;
    let root = fs::canonicalize(root)
        .map_err(|error| format!("installed releases root is unavailable: {error}"))?;
    let candidate = root.join(&commit);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("installed release is unavailable: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err("installed release must be a real directory".to_string());
    }
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("cannot resolve installed release: {error}"))?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("installed release escapes the releases root".to_string());
    }
    if !canonical.join(RELEASE_MANIFEST_NAME).is_file() {
        return Err("installed release manifest is missing".to_string());
    }
    Ok(canonical)
}

pub async fn backup_releases() -> Response {
    let root = releases_root();
    let active = active_release_commit(&root);
    let mut releases = Vec::new();
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().and_then(normalized_full_commit) else {
                continue;
            };
            let path = entry.path();
            let is_real_dir = fs::symlink_metadata(&path)
                .map(|metadata| metadata.file_type().is_dir())
                .unwrap_or(false);
            if !is_real_dir {
                continue;
            }
            releases.push(ReleaseBackupEntry {
                active: active.as_deref() == Some(name.as_str()),
                manifest_present: path.join(RELEASE_MANIFEST_NAME).is_file(),
                commit: name,
            });
        }
    }
    releases.sort_by(|left, right| right.commit.cmp(&left.commit));
    Json(serde_json::json!({
        "format": "saturn-installed-release-list",
        "active_commit": active,
        "releases": releases,
    }))
    .into_response()
}

pub async fn backup_release(Query(query): Query<ReleaseBackupQuery>) -> Result<Response, Response> {
    let root = releases_root();
    let commit = match query.commit {
        Some(commit) => normalized_full_commit(&commit).ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "release commit must be a full 40-character hexadecimal commit",
            )
        })?,
        None => active_release_commit(&root).ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "no active immutable release; select an installed release commit",
            )
        })?,
    };
    let release = resolve_installed_release(&root, &commit)
        .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?;
    let parent = release
        .parent()
        .ok_or_else(|| json_error(StatusCode::INTERNAL_SERVER_ERROR, "release parent missing"))?
        .to_path_buf();
    stream_tar(
        parent,
        commit.clone(),
        format!("saturn-release-{commit}.tar.gz"),
        "immutable-release",
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "saturn-backup-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn settings_tree_contains_only_declared_portable_files() {
        let root = test_root("settings");
        let state = root.join("state");
        let scripts = root.join("scripts");
        let pihpsdr = root.join("pihpsdr");
        let deskhpsdr = root.join("deskhpsdr");
        for directory in [&state, &scripts, &pihpsdr, &deskhpsdr] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(
            state.join("state-schema.json"),
            b"{\"state_schema_version\":1}\n",
        )
        .unwrap();
        fs::write(
            state.join("remote_settings.json"),
            b"{\"theme\":\"dark\"}\n",
        )
        .unwrap();
        fs::write(state.join("remote-tls.key"), b"must-not-copy\n").unwrap();
        fs::write(
            state.join("custom_scripts.json"),
            br#"[
              {"filename":"default.sh","version":"custom-default"},
              {"filename":"operator.sh","version":"operator"}
            ]"#,
        )
        .unwrap();
        fs::write(scripts.join("default.sh"), b"default\n").unwrap();
        fs::write(scripts.join("operator.sh"), b"operator\n").unwrap();
        fs::write(pihpsdr.join("saturn.xdma.props"), b"pihpsdr\n").unwrap();
        fs::write(pihpsdr.join("binary"), b"omit\n").unwrap();
        fs::write(deskhpsdr.join("startup_config.props"), b"deskhpsdr\n").unwrap();
        fs::write(deskhpsdr.join("deskhpsdr.log"), b"omit\n").unwrap();

        let missing = root.join("missing");
        let sources = SettingsBackupSources {
            state_schema_file: state.join("state-schema.json"),
            custom_scripts_file: state.join("custom_scripts.json"),
            remote_settings_file: state.join("remote_settings.json"),
            remote_profiles_file: missing.join("remote_profiles.json"),
            repo_root_file: missing.join("repo_root.txt"),
            update_policy_file: missing.join("update_policy.json"),
            saturngo_update_policy_file: missing.join("saturngo_update_policy.json"),
            scripts_dir: scripts,
            pihpsdr_root: pihpsdr,
            deskhpsdr_config_root: deskhpsdr,
        };
        let (temp, archive) = build_settings_backup_tree(&sources).unwrap();
        assert!(archive.join("manifest.json").is_file());
        assert!(archive.join("saturn-state/remote_settings.json").is_file());
        assert!(archive.join("custom-scripts/operator.sh").is_file());
        assert!(!archive.join("custom-scripts/default.sh").exists());
        assert!(archive.join("clients/pihpsdr/saturn.xdma.props").is_file());
        assert!(archive
            .join("clients/deskhpsdr/startup_config.props")
            .is_file());
        assert!(!archive.join("clients/pihpsdr/binary").exists());
        assert!(!archive.join("clients/deskhpsdr/deskhpsdr.log").exists());
        assert!(!archive.join("saturn-state/remote-tls.key").exists());

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(archive.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["format"], SETTINGS_BACKUP_FORMAT);
        assert_eq!(manifest["schema_version"], SETTINGS_BACKUP_SCHEMA_VERSION);
        assert!(manifest["omitted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("credentials")));
        let paths: Vec<&str> = manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["archive_path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"custom-scripts/operator.sh"));
        assert!(!paths.iter().any(|path| path.contains("default.sh")));
        fs::remove_dir_all(temp).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn settings_tree_rejects_selected_script_symlink() {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink");
        let state = root.join("state");
        let scripts = root.join("scripts");
        fs::create_dir(&state).unwrap();
        fs::create_dir(&scripts).unwrap();
        fs::write(
            state.join("custom_scripts.json"),
            br#"[{"filename":"operator.sh","version":"operator"}]"#,
        )
        .unwrap();
        symlink("/etc/passwd", scripts.join("operator.sh")).unwrap();
        let missing = root.join("missing");
        let sources = SettingsBackupSources {
            state_schema_file: missing.join("state-schema.json"),
            custom_scripts_file: state.join("custom_scripts.json"),
            remote_settings_file: missing.join("remote_settings.json"),
            remote_profiles_file: missing.join("remote_profiles.json"),
            repo_root_file: missing.join("repo_root.txt"),
            update_policy_file: missing.join("update_policy.json"),
            saturngo_update_policy_file: missing.join("saturngo_update_policy.json"),
            scripts_dir: scripts,
            pihpsdr_root: missing.join("pihpsdr"),
            deskhpsdr_config_root: missing.join("deskhpsdr"),
        };
        let error = build_settings_backup_tree(&sources).unwrap_err();
        assert!(error.contains("regular file"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_commit_validation_is_strict() {
        assert!(normalized_full_commit(&"a".repeat(40)).is_some());
        assert!(normalized_full_commit("main").is_none());
        assert!(normalized_full_commit(&format!("{}../", "a".repeat(36))).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn installed_release_must_be_a_manifest_bearing_real_child() {
        use std::os::unix::fs::symlink;

        let root = test_root("release");
        let commit = "a".repeat(40);
        let release = root.join(&commit);
        fs::create_dir(&release).unwrap();
        fs::write(release.join(RELEASE_MANIFEST_NAME), b"{}\n").unwrap();
        assert_eq!(resolve_installed_release(&root, &commit).unwrap(), release);

        let missing_manifest = "b".repeat(40);
        fs::create_dir(root.join(&missing_manifest)).unwrap();
        assert!(resolve_installed_release(&root, &missing_manifest)
            .unwrap_err()
            .contains("manifest is missing"));

        let outside = test_root("release-outside");
        let symlink_commit = "c".repeat(40);
        symlink(&outside, root.join(&symlink_commit)).unwrap();
        assert!(resolve_installed_release(&root, &symlink_commit)
            .unwrap_err()
            .contains("real directory"));

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
