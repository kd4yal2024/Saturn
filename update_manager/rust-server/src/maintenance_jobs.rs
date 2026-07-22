use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;

use crate::maintenance_lock::{JOB_OUTPUT_MAX_BYTES, JOB_OUTPUT_MAX_LINES};
use crate::state_store::{write_json_atomic, AtomicWriteOptions};

const SCHEMA_VERSION: u32 = 1;
const JOB_DIR_NAME: &str = "maintenance-jobs";
static CONTROLLER_INSTANCE: OnceLock<String> = OnceLock::new();

fn controller_instance() -> &'static str {
    CONTROLLER_INSTANCE.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        format!("saturn-go-{}-{nanos}", std::process::id())
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobResult {
    pub outcome: String,
    pub message: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceJob {
    pub schema_version: u32,
    pub id: String,
    pub job_type: String,
    pub state: String,
    pub resources: Vec<String>,
    pub requester: String,
    #[serde(default)]
    pub controller_instance: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub child_scope: Option<String>,
    pub child_pid: Option<u32>,
    pub child_start_ticks: Option<u64>,
    pub output_path: String,
    pub result: Option<JobResult>,
    pub recovery_steps: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerResult {
    pub job_id: String,
    pub finished_at: String,
    pub exit_code: i32,
    #[serde(default)]
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileSummary {
    pub checked: usize,
    pub completed: Vec<String>,
    pub orphaned: Vec<String>,
    pub interrupted: Vec<String>,
}

pub fn jobs_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(JOB_DIR_NAME)
}

fn records_dir(state_dir: &Path) -> PathBuf {
    jobs_dir(state_dir).join("records")
}

fn output_dir(state_dir: &Path) -> PathBuf {
    jobs_dir(state_dir).join("output")
}

fn results_dir(state_dir: &Path) -> PathBuf {
    jobs_dir(state_dir).join("results")
}

pub fn record_path(state_dir: &Path, id: &str) -> PathBuf {
    records_dir(state_dir).join(format!("{id}.json"))
}

pub fn output_path(state_dir: &Path, id: &str) -> PathBuf {
    output_dir(state_dir).join(format!("{id}.log"))
}

pub fn broker_result_path(state_dir: &Path, id: &str) -> PathBuf {
    results_dir(state_dir).join(format!("{id}.json"))
}

pub async fn broker_timed_out(path: &Path) -> bool {
    tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BrokerResult>(&bytes).ok())
        .is_some_and(|result| result.timed_out)
}

pub async fn initialize(state_dir: &Path) -> Result<ReconcileSummary, String> {
    for path in [
        jobs_dir(state_dir),
        records_dir(state_dir),
        output_dir(state_dir),
        results_dir(state_dir),
    ] {
        tokio::fs::create_dir_all(&path).await.map_err(|error| {
            format!("failed to create job directory {}: {error}", path.display())
        })?;
    }
    reconcile(state_dir).await
}

pub fn new_job(
    state_dir: &Path,
    id: String,
    job_type: String,
    resources: &[&str],
    requester: &str,
    metadata: Value,
) -> MaintenanceJob {
    let now = Local::now().to_rfc3339();
    MaintenanceJob {
        schema_version: SCHEMA_VERSION,
        output_path: output_path(state_dir, &id).display().to_string(),
        id,
        job_type,
        state: "starting".to_string(),
        resources: resources.iter().map(|value| (*value).to_string()).collect(),
        requester: requester.to_string(),
        controller_instance: controller_instance().to_string(),
        created_at: now.clone(),
        started_at: None,
        updated_at: now,
        finished_at: None,
        child_scope: None,
        child_pid: None,
        child_start_ticks: None,
        result: None,
        recovery_steps: Vec::new(),
        metadata,
    }
}

pub async fn save(state_dir: &Path, job: &MaintenanceJob) -> Result<(), String> {
    write_json_atomic(
        &record_path(state_dir, &job.id),
        job,
        AtomicWriteOptions::state_file(),
    )
    .await
}

pub async fn append_output_line(path: &Path, line: &str) -> Result<(), String> {
    let existing = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(format!(
                "failed to inspect job output {}: {error}",
                path.display()
            ))
        }
    };
    let next_bytes = existing.len().saturating_add(line.len()).saturating_add(1);
    let existing_lines = existing.iter().filter(|byte| **byte == b'\n').count();
    if next_bytes > JOB_OUTPUT_MAX_BYTES as usize || existing_lines >= JOB_OUTPUT_MAX_LINES {
        return Ok(());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .open(path)
        .await
        .map_err(|error| format!("failed to open job output {}: {error}", path.display()))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|error| format!("failed to append job output {}: {error}", path.display()))?;
    file.write_all(b"\n")
        .await
        .map_err(|error| format!("failed to append job output {}: {error}", path.display()))?;
    file.sync_data()
        .await
        .map_err(|error| format!("failed to sync job output {}: {error}", path.display()))
}

pub async fn mark_running(
    state_dir: &Path,
    job: &mut MaintenanceJob,
    pid: u32,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    job.state = "running".to_string();
    job.started_at = Some(now.clone());
    job.updated_at = now;
    job.child_scope = Some(format!("process-group:{pid}"));
    job.child_pid = Some(pid);
    job.child_start_ticks = process_identity(pid).map(|identity| identity.0);
    save(state_dir, job).await
}

pub async fn mark_in_process_running(
    state_dir: &Path,
    job: &mut MaintenanceJob,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    let pid = std::process::id();
    job.state = "running".to_string();
    job.started_at = Some(now.clone());
    job.updated_at = now;
    job.child_scope = Some(format!("saturn-go-task:{pid}:{}", job.id));
    job.child_pid = Some(pid);
    job.child_start_ticks = process_identity(pid).map(|identity| identity.0);
    save(state_dir, job).await
}

pub async fn finish(
    state_dir: &Path,
    job: &mut MaintenanceJob,
    state: &str,
    result: JobResult,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    job.state = state.to_string();
    job.updated_at = now.clone();
    job.finished_at = Some(now);
    job.result = Some(result);
    job.recovery_steps.clear();
    save(state_dir, job).await
}

#[cfg(test)]
async fn load(state_dir: &Path, id: &str) -> Result<MaintenanceJob, String> {
    read_job(&record_path(state_dir, id)).await
}

async fn read_job(path: &Path) -> Result<MaintenanceJob, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| format!("failed to read job record {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid job record {}: {error}", path.display()))
}

pub async fn list(state_dir: &Path) -> Result<Vec<MaintenanceJob>, String> {
    let mut paths = Vec::new();
    let mut entries = match tokio::fs::read_dir(records_dir(state_dir)).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to list job records: {error}")),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("failed to list job records: {error}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".last-good"))
        {
            paths.push(path);
        }
    }
    let mut jobs = Vec::new();
    for path in paths {
        jobs.push(read_job(&path).await?);
    }
    jobs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(jobs)
}

pub async fn latest_for_type(
    state_dir: &Path,
    job_type: &str,
) -> Result<Option<MaintenanceJob>, String> {
    Ok(list(state_dir)
        .await?
        .into_iter()
        .find(|job| job.job_type == job_type))
}

fn incomplete(state: &str) -> bool {
    matches!(state, "starting" | "running" | "orphaned")
}

pub async fn reconcile(state_dir: &Path) -> Result<ReconcileSummary, String> {
    let mut summary = ReconcileSummary {
        checked: 0,
        completed: Vec::new(),
        orphaned: Vec::new(),
        interrupted: Vec::new(),
    };
    for mut job in list(state_dir).await? {
        if !incomplete(&job.state) {
            continue;
        }
        let result_path = broker_result_path(state_dir, &job.id);
        if let Ok(bytes) = tokio::fs::read(&result_path).await {
            summary.checked += 1;
            let broker: BrokerResult = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid broker result {}: {error}", result_path.display())
            })?;
            if broker.job_id != job.id {
                return Err(format!(
                    "broker result job ID mismatch for {}: expected {}, got {}",
                    result_path.display(),
                    job.id,
                    broker.job_id
                ));
            }
            let success = broker.exit_code == 0 && !broker.timed_out;
            job.state = if broker.timed_out {
                "timed_out"
            } else if success {
                "completed"
            } else {
                "failed"
            }
            .to_string();
            job.updated_at = broker.finished_at.clone();
            job.finished_at = Some(broker.finished_at);
            job.result = Some(JobResult {
                outcome: if broker.timed_out {
                    "timeout"
                } else if success {
                    "success"
                } else {
                    "failure"
                }
                .to_string(),
                message: if broker.timed_out {
                    format!(
                        "maintenance child exceeded its deadline and exited with code {}",
                        broker.exit_code
                    )
                } else {
                    format!("maintenance child exited with code {}", broker.exit_code)
                },
                exit_code: Some(broker.exit_code),
            });
            job.recovery_steps.clear();
            save(state_dir, &job).await?;
            summary.completed.push(job.id);
            continue;
        }
        if job.controller_instance == controller_instance() {
            continue;
        }
        summary.checked += 1;

        let alive = job
            .child_pid
            .zip(job.child_start_ticks)
            .and_then(|(pid, start)| process_identity(pid).map(|identity| (pid, start, identity)))
            .is_some_and(|(pid, start, (actual_start, pgrp))| start == actual_start && pgrp == pid);
        let now = Local::now().to_rfc3339();
        job.updated_at = now.clone();
        if alive {
            job.state = "orphaned".to_string();
            job.result = Some(JobResult {
                outcome: "running-detached".to_string(),
                message: "maintenance child survived the Saturn Go restart".to_string(),
                exit_code: None,
            });
            job.recovery_steps = vec![
                "Do not start a conflicting maintenance job; host resource locks remain held."
                    .to_string(),
                format!("Monitor durable output at {}.", job.output_path),
                "Refresh maintenance job status after the child exits.".to_string(),
            ];
            summary.orphaned.push(job.id.clone());
        } else {
            job.state = "interrupted".to_string();
            job.finished_at = Some(now);
            job.result = Some(JobResult {
                outcome: "interrupted".to_string(),
                message: "maintenance child is no longer running and wrote no completion result"
                    .to_string(),
                exit_code: None,
            });
            job.recovery_steps = vec![
                format!("Review durable output at {}.", job.output_path),
                "Verify the affected subsystem before retrying the operation.".to_string(),
                "Retry only after the maintenance lock probe reports the resources available."
                    .to_string(),
            ];
            summary.interrupted.push(job.id.clone());
        }
        save(state_dir, &job).await?;
    }
    Ok(summary)
}

pub fn process_identity(pid: u32) -> Option<(u64, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(close + 2..)?.split_whitespace().collect();
    let pgrp = fields.get(2)?.parse::<u32>().ok()?;
    let start_ticks = fields.get(19)?.parse::<u64>().ok()?;
    Some((start_ticks, pgrp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "saturn-maintenance-job-{name}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[tokio::test]
    async fn incomplete_job_without_child_is_marked_interrupted() {
        let root = temp_dir("interrupted");
        initialize(&root).await.unwrap();
        let job = new_job(
            &root,
            "job-1".to_string(),
            "script:test.sh".to_string(),
            &["radio"],
            "tester",
            serde_json::json!({}),
        );
        let mut job = job;
        job.controller_instance = "previous-controller".to_string();
        save(&root, &job).await.unwrap();
        let summary = reconcile(&root).await.unwrap();
        assert_eq!(summary.interrupted, vec!["job-1"]);
        let recovered = load(&root, "job-1").await.unwrap();
        assert_eq!(recovered.state, "interrupted");
        assert!(!recovered.recovery_steps.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn broker_result_deterministically_completes_job() {
        let root = temp_dir("result");
        initialize(&root).await.unwrap();
        let job = new_job(
            &root,
            "job-2".to_string(),
            "script:test.sh".to_string(),
            &["radio"],
            "tester",
            serde_json::json!({}),
        );
        let mut job = job;
        job.controller_instance = "previous-controller".to_string();
        save(&root, &job).await.unwrap();
        write_json_atomic(
            &broker_result_path(&root, "job-2"),
            &BrokerResult {
                job_id: "job-2".to_string(),
                finished_at: "2026-07-21T12:00:00-04:00".to_string(),
                exit_code: 0,
                timed_out: false,
            },
            AtomicWriteOptions::state_file(),
        )
        .await
        .unwrap();
        let summary = reconcile(&root).await.unwrap();
        assert_eq!(summary.completed, vec!["job-2"]);
        assert_eq!(load(&root, "job-2").await.unwrap().state, "completed");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn broker_timeout_is_reconciled_as_timed_out() {
        let root = temp_dir("timeout-result");
        initialize(&root).await.unwrap();
        let mut job = new_job(
            &root,
            "job-timeout".to_string(),
            "script:test.sh".to_string(),
            &["radio"],
            "tester",
            serde_json::json!({"deadline_seconds": 1}),
        );
        job.controller_instance = "previous-controller".to_string();
        save(&root, &job).await.unwrap();
        write_json_atomic(
            &broker_result_path(&root, "job-timeout"),
            &BrokerResult {
                job_id: "job-timeout".to_string(),
                finished_at: "2026-07-21T12:00:00-04:00".to_string(),
                exit_code: -15,
                timed_out: true,
            },
            AtomicWriteOptions::state_file(),
        )
        .await
        .unwrap();
        reconcile(&root).await.unwrap();
        let recovered = load(&root, "job-timeout").await.unwrap();
        assert_eq!(recovered.state, "timed_out");
        assert_eq!(recovered.result.unwrap().outcome, "timeout");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn surviving_process_group_is_marked_orphaned() {
        let root = temp_dir("orphaned");
        initialize(&root).await.unwrap();
        let mut child = std::process::Command::new("sleep");
        child.arg("30").process_group(0);
        let mut child = child.spawn().unwrap();
        let pid = child.id();
        let (start_ticks, pgrp) = process_identity(pid).unwrap();
        assert_eq!(pgrp, pid);

        let mut job = new_job(
            &root,
            "job-3".to_string(),
            "script:test.sh".to_string(),
            &["radio"],
            "tester",
            serde_json::json!({}),
        );
        job.controller_instance = "previous-controller".to_string();
        job.state = "running".to_string();
        job.child_pid = Some(pid);
        job.child_start_ticks = Some(start_ticks);
        job.child_scope = Some(format!("process-group:{pid}"));
        save(&root, &job).await.unwrap();

        let summary = reconcile(&root).await.unwrap();
        assert_eq!(summary.orphaned, vec!["job-3"]);
        let recovered = load(&root, "job-3").await.unwrap();
        assert_eq!(recovered.state, "orphaned");
        assert!(!recovered.recovery_steps.is_empty());

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn current_process_identity_is_stable_and_grouped() {
        let first = process_identity(std::process::id()).unwrap();
        let second = process_identity(std::process::id()).unwrap();
        assert_eq!(first, second);
        assert!(first.0 > 0);
    }
}
