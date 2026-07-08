use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::state::{PiImageStatusQuery, MAX_COMPLETED_JOBS};
use crate::sync_ext::MutexExt;
use crate::util::json_error;

const PRIVILEGED_PI_CLONE_HELPER: &str = "/usr/local/lib/saturn-go/scripts/clone_pi_to_device.sh";
const PRIVILEGED_PI_WIPE_HELPER: &str = "/usr/local/lib/saturn-go/scripts/saturn-pi-wipe-target.sh";

#[derive(Debug, Clone, Serialize)]
pub struct PiCloneJob {
    id: String,
    status: String,
    progress: u8,
    message: String,
    pid: Option<u32>,
    log: Vec<String>,
}

static PI_CLONE_JOBS: OnceLock<Mutex<std::collections::HashMap<String, PiCloneJob>>> =
    OnceLock::new();

fn clone_jobs_map() -> &'static Mutex<std::collections::HashMap<String, PiCloneJob>> {
    PI_CLONE_JOBS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn set_clone_job(job: PiCloneJob) {
    let mut map = clone_jobs_map().lock_unpoisoned();
    map.insert(job.id.clone(), job);
    prune_completed_clone_jobs(&mut map);
}

fn prune_completed_clone_jobs(map: &mut std::collections::HashMap<String, PiCloneJob>) {
    let completed: Vec<String> = map
        .iter()
        .filter(|(_, j)| j.status != "running")
        .map(|(id, _)| id.clone())
        .collect();
    if completed.len() > MAX_COMPLETED_JOBS {
        let excess = completed.len() - MAX_COMPLETED_JOBS;
        for id in completed.into_iter().take(excess) {
            map.remove(&id);
        }
    }
}

fn update_clone_job(id: &str, f: impl FnOnce(&mut PiCloneJob)) {
    let mut map = clone_jobs_map().lock_unpoisoned();
    if let Some(j) = map.get_mut(id) {
        f(j);
    }
}

fn get_clone_job(id: &str) -> Option<PiCloneJob> {
    let map = clone_jobs_map().lock_unpoisoned();
    map.get(id).cloned()
}

fn append_clone_log(id: &str, line: String) {
    update_clone_job(id, |j| {
        j.log.push(line);
        if j.log.len() > 200 {
            let excess = j.log.len() - 200;
            j.log.drain(0..excess);
        }
    });
}

#[derive(Serialize)]
pub struct PiDeviceInfo {
    name: String,
    path: String,
    size_bytes: u64,
    model: String,
}

fn pi_clone_device_allowed(name: &str, sys_block_path: &std::path::Path) -> bool {
    // Skip virtual/non-target block devices and the running system source disk.
    if name == "mmcblk0"
        || name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("zram")
        || name.starts_with("dm-")
        || name.starts_with("md")
        || name.starts_with("nbd")
        || name.starts_with("sr")
    {
        return false;
    }

    let removable = fs::read_to_string(sys_block_path.join("removable"))
        .ok()
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    if removable {
        return true;
    }

    // Many USB SD readers report removable=0; allow USB-attached disks too.
    fs::canonicalize(sys_block_path.join("device"))
        .ok()
        .map(|p| p.to_string_lossy().contains("/usb"))
        .unwrap_or(false)
}

pub async fn pi_devices() -> impl IntoResponse {
    let mut devices: Vec<PiDeviceInfo> = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if !pi_clone_device_allowed(&name, &ent.path()) {
                continue;
            }
            let size_path = ent.path().join("size");
            let sectors = fs::read_to_string(&size_path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let size_bytes = sectors.saturating_mul(512);
            let model_path = ent.path().join("device").join("model");
            let model = fs::read_to_string(&model_path)
                .ok()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let path = format!("/dev/{name}");
            devices.push(PiDeviceInfo {
                name,
                path,
                size_bytes,
                model,
            });
        }
    }
    Json(serde_json::json!({ "devices": devices }))
}

#[derive(Deserialize)]
pub struct PiCloneStartReq {
    pub target: String,
    #[serde(default)]
    pub verify_compare: bool,
}

fn classify_clone_stderr_line(line: &str) -> String {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("ERR:") {
        return trimmed.to_string();
    }
    let is_dd_progress = trimmed.contains(" bytes ") && trimmed.contains(" copied");
    let is_dd_summary = trimmed.ends_with("records in") || trimmed.ends_with("records out");
    if is_dd_progress || is_dd_summary {
        trimmed.to_string()
    } else {
        format!("stderr: {trimmed}")
    }
}

pub async fn pi_clone_start(
    axum::extract::Json(req): axum::extract::Json<PiCloneStartReq>,
) -> Response {
    let PiCloneStartReq {
        target,
        verify_compare,
    } = req;
    if !target.starts_with("/dev/") {
        return json_error(StatusCode::BAD_REQUEST, "target must be a /dev path");
    }
    if target == "/dev/mmcblk0" {
        return json_error(StatusCode::BAD_REQUEST, "target cannot be source device");
    }

    let name = target.trim_start_matches("/dev/");
    let sys_block_path = std::path::Path::new("/sys/block").join(name);
    if !sys_block_path.exists() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "target device not found in /sys/block",
        );
    }
    if !pi_clone_device_allowed(name, &sys_block_path) {
        return json_error(StatusCode::BAD_REQUEST, "target device is not removable");
    }

    let id = format!(
        "piclone-{}-{}",
        std::process::id(),
        Local::now().format("%Y%m%d%H%M%S")
    );
    let job = PiCloneJob {
        id: id.clone(),
        status: "running".to_string(),
        progress: 0,
        message: "starting".to_string(),
        pid: None,
        log: Vec::new(),
    };
    set_clone_job(job.clone());

    tokio::spawn(async move {
        let mut cmd = Command::new("sudo");
        cmd.arg("-n").arg(PRIVILEGED_PI_CLONE_HELPER);
        cmd.arg("--target").arg(&target);
        if verify_compare {
            cmd.arg("--verify-compare");
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                update_clone_job(&id, |j| {
                    j.status = "error".to_string();
                    j.message = e.to_string();
                });
                return;
            }
        };

        update_clone_job(&id, |j| j.pid = child.id());

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(out) = stdout {
            let id2 = id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(p) = line.strip_prefix("Progress: ") {
                        if let Ok(v) = p.trim_end_matches('%').trim().parse::<u8>() {
                            update_clone_job(&id2, |j| j.progress = v);
                        }
                    }
                    update_clone_job(&id2, |j| j.message = line.clone());
                    append_clone_log(&id2, line);
                }
            });
        }

        if let Some(err) = stderr {
            let id2 = id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let msg = classify_clone_stderr_line(&line);
                    if msg.is_empty() {
                        continue;
                    }
                    update_clone_job(&id2, |j| j.message = msg.clone());
                    append_clone_log(&id2, msg);
                }
            });
        }

        let status = child.wait().await;
        match status {
            Ok(s) if s.success() => update_clone_job(&id, |j| {
                j.status = "done".to_string();
                j.progress = 100;
                j.message = "done".to_string();
                j.pid = None;
            }),
            Ok(s) => update_clone_job(&id, |j| {
                j.status = "error".to_string();
                j.message = format!("clone failed: {s}");
                j.pid = None;
            }),
            Err(e) => update_clone_job(&id, |j| {
                j.status = "error".to_string();
                j.message = format!("clone failed: {e}");
                j.pid = None;
            }),
        }
    });

    Json(serde_json::json!({ "job_id": job.id })).into_response()
}

pub async fn pi_clone_status(Query(q): Query<PiImageStatusQuery>) -> impl IntoResponse {
    if let Some(job) = get_clone_job(&q.job_id) {
        Json(job).into_response()
    } else {
        json_error(StatusCode::NOT_FOUND, "job not found")
    }
}

pub async fn pi_clone_cancel(Query(q): Query<PiImageStatusQuery>) -> impl IntoResponse {
    let job = match get_clone_job(&q.job_id) {
        Some(j) => j,
        None => return json_error(StatusCode::NOT_FOUND, "job not found"),
    };
    if job.status != "running" {
        return json_error(StatusCode::BAD_REQUEST, "job not running");
    }
    if let Some(pid) = job.pid {
        let _ = Command::new("kill")
            .arg("-15")
            .arg(pid.to_string())
            .status()
            .await;
    }
    update_clone_job(&q.job_id, |j| {
        j.status = "cancelled".to_string();
        j.message = "cancelled".to_string();
        j.pid = None;
    });
    Json(serde_json::json!({ "status": "cancelled" })).into_response()
}

pub async fn pi_wipe_target(
    axum::extract::Json(req): axum::extract::Json<PiCloneStartReq>,
) -> Response {
    let target = req.target;
    if !target.starts_with("/dev/") {
        return json_error(StatusCode::BAD_REQUEST, "target must be a /dev path");
    }
    if target == "/dev/mmcblk0" {
        return json_error(StatusCode::BAD_REQUEST, "target cannot be source device");
    }

    let name = target.trim_start_matches("/dev/");
    let sys_block_path = Path::new("/sys/block").join(name);
    if !sys_block_path.exists() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "target device not found in /sys/block",
        );
    }
    if !pi_clone_device_allowed(name, &sys_block_path) {
        return json_error(StatusCode::BAD_REQUEST, "target device is not removable");
    }

    let out = match Command::new("sudo")
        .arg("-n")
        .arg(PRIVILEGED_PI_WIPE_HELPER)
        .arg(&target)
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("wipe helper failed: {e}"),
            )
        }
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let msg = if stderr.is_empty() { stdout } else { stderr };
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("wipe helper failed: {msg}"),
        );
    }
    let log: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| line.to_string())
        .collect();

    Json(serde_json::json!({
        "status": "ok",
        "message": "Target wiped (signatures/partition metadata cleared).",
        "log": log
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- classify_clone_stderr_line ---

    #[test]
    fn test_classify_empty_line() {
        assert_eq!(classify_clone_stderr_line(""), "");
        assert_eq!(classify_clone_stderr_line("   \t"), "");
    }

    #[test]
    fn test_classify_err_prefix_passthrough() {
        let line = "ERR: something bad happened";
        assert_eq!(classify_clone_stderr_line(line), line);
    }

    #[test]
    fn test_classify_dd_progress() {
        let line = "1073741824 bytes (1.1 GB, 1.0 GiB) copied, 5.12 s, 210 MB/s";
        let result = classify_clone_stderr_line(line);
        assert_eq!(result, line.trim_end());
    }

    #[test]
    fn test_classify_dd_records() {
        assert_eq!(
            classify_clone_stderr_line("16+0 records in"),
            "16+0 records in"
        );
        assert_eq!(
            classify_clone_stderr_line("16+0 records out"),
            "16+0 records out"
        );
    }

    #[test]
    fn test_classify_generic_stderr_prefixed() {
        let result = classify_clone_stderr_line("some warning message");
        assert_eq!(result, "stderr: some warning message");
    }

    // --- pi_clone_device_allowed ---

    #[test]
    fn test_mmcblk0_always_blocked() {
        let path = std::path::Path::new("/sys/block/mmcblk0");
        assert!(!pi_clone_device_allowed("mmcblk0", path));
    }

    #[test]
    fn test_loop_device_blocked() {
        let path = std::path::Path::new("/sys/block/loop0");
        assert!(!pi_clone_device_allowed("loop0", path));
    }

    #[test]
    fn test_zram_blocked() {
        let path = std::path::Path::new("/sys/block/zram0");
        assert!(!pi_clone_device_allowed("zram0", path));
    }
}
