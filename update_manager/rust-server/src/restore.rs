use axum::{
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs, io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncWriteExt, process::Command};

use crate::maintenance_lock::{self, READ_ONLY_MAINTENANCE, REPOSITORY_RESTORE};
use crate::state::{AppState, MAX_TAR_EXPANSION_FACTOR};
use crate::state_store::{write_atomic, AtomicWriteOptions};
use crate::update::begin_update_activity;
use crate::util::{
    backup_home_dir, current_repo_root, is_saturn_repo_root, json_error, parse_boolish,
};

const DEFAULT_TRANSACTION_TOOL: &str =
    "/usr/local/lib/saturn-go/scripts/saturn-restore-transaction.py";

#[derive(Debug, Deserialize, Default)]
pub struct RestoreQuery {
    dry_run: Option<String>,
    include_host_policy: Option<String>,
}

fn transaction_tool() -> PathBuf {
    std::env::var("SATURN_RESTORE_TRANSACTION_TOOL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_TRANSACTION_TOOL))
}

fn state_root(state: &AppState) -> Result<PathBuf, String> {
    state
        .repo_root_file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Saturn state root is unavailable".to_string())
}

fn secure_tmp_path(prefix: &str, attempt: u32) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{nanos}-{}-{attempt}", std::process::id()))
}

fn create_secure_temp_upload_file() -> io::Result<(PathBuf, tokio::fs::File)> {
    for attempt in 0..64 {
        let path = secure_tmp_path("saturn-upload", attempt);
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, tokio::fs::File::from_std(file))),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate unique Saturn upload temp file",
    ))
}

fn create_secure_temp_extract_dir() -> io::Result<PathBuf> {
    for attempt in 0..64 {
        let path = secure_tmp_path("saturn-restore", attempt);
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
        "could not allocate unique Saturn restore temp directory",
    ))
}

fn has_unsafe_path_component(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|component| component == "..")
}

fn parse_tar_verbose_line(line: &str) -> Option<(&str, u64)> {
    let mut rest = line;
    for _ in 0..5 {
        rest = rest.trim_start_matches(|character: char| character.is_ascii_whitespace());
        if rest.is_empty() {
            return None;
        }
        let token_end = rest
            .find(|character: char| character.is_ascii_whitespace())
            .unwrap_or(rest.len());
        rest = &rest[token_end..];
    }
    let path = rest.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if path.is_empty() {
        return None;
    }
    let size = line.split_ascii_whitespace().nth(2)?.parse::<u64>().ok()?;
    Some((path, size))
}

async fn available_bytes_at(path: &Path) -> Option<u64> {
    let output = Command::new("df")
        .arg("-B1")
        .arg("--output=avail")
        .arg(path)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)?
        .trim()
        .parse::<u64>()
        .ok()
}

async fn receive_archive(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<(PathBuf, u64, Option<String>), Response> {
    let mut upload_path = None;
    let mut upload_bytes = 0u64;
    let mut confirm = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "confirm" {
            confirm = Some(field.text().await.unwrap_or_default().trim().to_string());
            continue;
        }
        if name != "file" {
            continue;
        }
        if let Some(existing) = upload_path.as_ref() {
            let _ = tokio::fs::remove_file(existing).await;
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "only one restore archive is accepted",
            ));
        }
        let (path, mut file) = create_secure_temp_upload_file()
            .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()))?;
        let mut field = field;
        while let Ok(Some(chunk)) = field.chunk().await {
            upload_bytes = upload_bytes.saturating_add(chunk.len() as u64);
            if upload_bytes > state.restore_max_upload_bytes {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(json_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    &format!(
                        "archive too large (limit {} MB)",
                        state.restore_max_upload_bytes / 1024 / 1024
                    ),
                ));
            }
            if let Err(error) = file.write_all(&chunk).await {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(json_error(
                    StatusCode::INSUFFICIENT_STORAGE,
                    &format!("cannot store restore upload: {error}"),
                ));
            }
        }
        if let Err(error) = file.flush().await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(json_error(
                StatusCode::INSUFFICIENT_STORAGE,
                &format!("cannot flush restore upload: {error}"),
            ));
        }
        if let Err(error) = file.sync_all().await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(json_error(
                StatusCode::INSUFFICIENT_STORAGE,
                &format!("cannot make restore upload durable: {error}"),
            ));
        }
        upload_path = Some(path);
    }

    upload_path
        .map(|path| (path, upload_bytes, confirm))
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "missing file"))
}

async fn validate_and_extract_archive(
    upload_path: &Path,
    upload_bytes: u64,
) -> Result<(PathBuf, PathBuf), Response> {
    let listing = Command::new("tar")
        .arg("-tzvf")
        .arg(upload_path)
        .output()
        .await
        .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error.to_string()))?;
    if !listing.status.success() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "tar list failed: {}",
                String::from_utf8_lossy(&listing.stderr).trim()
            ),
        ));
    }

    let mut uncompressed_bytes = 0u64;
    for line in String::from_utf8_lossy(&listing.stdout).lines() {
        let Some((path_field, size)) = parse_tar_verbose_line(line) else {
            continue;
        };
        let (link_name, link_target) = match path_field.find(" -> ") {
            Some(position) => (&path_field[..position], Some(&path_field[position + 4..])),
            None => (path_field, None),
        };
        if has_unsafe_path_component(link_name)
            || link_target.is_some_and(has_unsafe_path_component)
        {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "archive contains an unsafe path or symlink target",
            ));
        }
        uncompressed_bytes = uncompressed_bytes.saturating_add(size);
    }
    if uncompressed_bytes > upload_bytes.saturating_mul(MAX_TAR_EXPANSION_FACTOR) {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "archive expansion ratio exceeds the restore safety limit",
        ));
    }
    let temp_root = std::env::temp_dir();
    if let Some(available) = available_bytes_at(&temp_root).await {
        if uncompressed_bytes > available {
            return Err(json_error(
                StatusCode::INSUFFICIENT_STORAGE,
                "insufficient temporary space to extract the restore archive",
            ));
        }
    }

    let extract_dir = create_secure_temp_extract_dir()
        .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()))?;
    let status = match Command::new("tar")
        .arg("-xzf")
        .arg(upload_path)
        .arg("-C")
        .arg(&extract_dir)
        .status()
        .await
    {
        Ok(status) => status,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &error.to_string(),
            ));
        }
    };
    if !status.success() {
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "archive extraction failed",
        ));
    }

    let mut entries = match tokio::fs::read_dir(&extract_dir).await {
        Ok(entries) => entries,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &error.to_string(),
            ));
        }
    };
    let mut top_level = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => top_level.push(entry),
            Ok(None) => break,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&extract_dir).await;
                return Err(json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &error.to_string(),
                ));
            }
        }
    }
    if top_level.len() != 1 {
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "archive must contain exactly one top-level directory",
        ));
    }
    let entry = top_level.remove(0);
    let metadata = entry.metadata().await;
    let file_type = entry.file_type().await;
    if metadata
        .as_ref()
        .map(|value| !value.is_dir())
        .unwrap_or(true)
        || file_type.map(|value| value.is_symlink()).unwrap_or(true)
    {
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "archive top-level entry must be a real directory",
        ));
    }
    Ok((extract_dir, entry.path()))
}

async fn run_tool(
    arguments: &[String],
    operation: &str,
    resources: &[&str],
) -> Result<Value, String> {
    let tool = transaction_tool();
    if !tool.is_file() {
        return Err(format!(
            "restore transaction helper is not installed: {}",
            tool.display()
        ));
    }
    let output = maintenance_lock::wrapped_command(operation, resources, &tool, arguments)
        .output()
        .await
        .map_err(|error| format!("failed to start restore transaction helper: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() { stderr } else { stdout });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("restore helper returned invalid JSON: {error}"))
}

fn update_in_memory_repo_root(state: &AppState, value: &Value) -> Result<(), String> {
    let path = value
        .get("new_repo_root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            fs::read_to_string(&state.repo_root_file)
                .ok()
                .map(|value| PathBuf::from(value.trim()))
        })
        .ok_or_else(|| "restore completed without a repository pointer".to_string())?;
    let canonical = fs::canonicalize(&path)
        .map_err(|error| format!("cannot resolve restored repository pointer: {error}"))?;
    if !is_saturn_repo_root(&canonical) {
        return Err("restored repository pointer is not a Saturn checkout".to_string());
    }
    *state
        .repo_root
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = canonical;
    Ok(())
}

async fn restore_archive(
    state: AppState,
    query: RestoreQuery,
    multipart: Multipart,
    kind: &str,
) -> Result<Response, Response> {
    let dry_run = parse_boolish(query.dry_run);
    let include_host_policy = parse_boolish(query.include_host_policy);
    let _activity_guard = if dry_run {
        None
    } else {
        let detail = if kind == "settings" {
            "portable settings restore"
        } else {
            "source repository restore"
        };
        Some(
            begin_update_activity(&format!("saturn-{kind}-restore"), detail)
                .map_err(|error| json_error(StatusCode::CONFLICT, &error))?,
        )
    };
    let lock_operation = format!("restore:{kind}");
    let lock_resources = if dry_run {
        READ_ONLY_MAINTENANCE
    } else {
        REPOSITORY_RESTORE
    };
    maintenance_lock::probe(&lock_operation, lock_resources)
        .await
        .map_err(|error| json_error(StatusCode::CONFLICT, &error))?;

    let (upload_path, upload_bytes, confirm) = receive_archive(&state, multipart).await?;
    if !dry_run && confirm.as_deref() != Some("RESTORE") {
        let _ = tokio::fs::remove_file(&upload_path).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "confirm token required",
        ));
    }
    let extracted = validate_and_extract_archive(&upload_path, upload_bytes).await;
    let (extract_dir, archive_root) = match extracted {
        Ok(value) => value,
        Err(error) => {
            let _ = tokio::fs::remove_file(&upload_path).await;
            return Err(error);
        }
    };

    let state_root = state_root(&state)
        .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error))?;
    let mut arguments = vec![
        kind.to_string(),
        "--state-root".to_string(),
        state_root.display().to_string(),
    ];
    if kind == "settings" {
        arguments.extend([
            "--archive-root".to_string(),
            archive_root.display().to_string(),
            "--scripts-root".to_string(),
            state.scripts_dir.display().to_string(),
            "--pihpsdr-root".to_string(),
            crate::util::pihpsdr_repo_root().display().to_string(),
            "--deskhpsdr-root".to_string(),
            backup_home_dir()
                .join(".config/deskhpsdr")
                .display()
                .to_string(),
        ]);
        if include_host_policy {
            arguments.push("--include-host-policy".to_string());
        }
    } else {
        if !archive_root.join(".git").exists() && archive_root.join("manifest.json").is_file() {
            let _ = tokio::fs::remove_file(&upload_path).await;
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "this appears to be a Settings Backup; use Settings Restore",
            ));
        }
        arguments.extend([
            "--source-root".to_string(),
            archive_root.display().to_string(),
            "--current-repo-root".to_string(),
            current_repo_root(&state).display().to_string(),
            "--repo-root-file".to_string(),
            state.repo_root_file.display().to_string(),
        ]);
    }
    if dry_run {
        arguments.push("--dry-run".to_string());
    }

    let result = run_tool(&arguments, &lock_operation, lock_resources).await;
    let _ = tokio::fs::remove_file(&upload_path).await;
    let _ = tokio::fs::remove_dir_all(&extract_dir).await;
    let value = result.map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?;
    if !dry_run && (kind == "source" || include_host_policy) {
        update_in_memory_repo_root(&state, &value)
            .map_err(|error| json_error(StatusCode::INTERNAL_SERVER_ERROR, &error))?;
    }
    Ok(Json(value).into_response())
}

pub async fn restore_settings(
    State(state): State<AppState>,
    Query(query): Query<RestoreQuery>,
    multipart: Multipart,
) -> Result<Response, Response> {
    restore_archive(state, query, multipart, "settings").await
}

pub async fn restore_source(
    State(state): State<AppState>,
    Query(query): Query<RestoreQuery>,
    multipart: Multipart,
) -> Result<Response, Response> {
    restore_archive(state, query, multipart, "source").await
}

pub async fn transactional_source_restore_directory(
    state: &AppState,
    source: &Path,
    dry_run: bool,
) -> Result<Value, String> {
    let root = state_root(state)?;
    let mut arguments = vec![
        "source".to_string(),
        "--state-root".to_string(),
        root.display().to_string(),
        "--source-root".to_string(),
        source.display().to_string(),
        "--current-repo-root".to_string(),
        current_repo_root(state).display().to_string(),
        "--repo-root-file".to_string(),
        state.repo_root_file.display().to_string(),
    ];
    if dry_run {
        arguments.push("--dry-run".to_string());
    }
    let resources = if dry_run {
        READ_ONLY_MAINTENANCE
    } else {
        REPOSITORY_RESTORE
    };
    maintenance_lock::probe("restore:source-directory", resources).await?;
    let value = run_tool(&arguments, "restore:source-directory", resources).await?;
    if !dry_run {
        update_in_memory_repo_root(state, &value)?;
    }
    Ok(value)
}

pub async fn restore_status(State(state): State<AppState>) -> Response {
    let root = match state_root(&state) {
        Ok(value) => value,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    match run_tool(
        &[
            "status".to_string(),
            "--state-root".to_string(),
            root.display().to_string(),
        ],
        "restore:status",
        READ_ONLY_MAINTENANCE,
    )
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

pub async fn recover_restore_transactions(state_root: &Path) -> Result<Value, String> {
    let transactions = state_root.join("restore-transactions");
    if !transaction_tool().is_file() && !transactions.exists() {
        return Ok(serde_json::json!({"status": "ok", "recovered": []}));
    }
    run_tool(
        &[
            "recover".to_string(),
            "--state-root".to_string(),
            state_root.display().to_string(),
        ],
        "restore:recover",
        REPOSITORY_RESTORE,
    )
    .await
}

pub async fn persist_repo_root_atomic(path: &Path, repo_root: &Path) -> Result<(), String> {
    let content = format!("{}\n", repo_root.display()).into_bytes();
    write_atomic(path, content, AtomicWriteOptions::state_file()).await
}
