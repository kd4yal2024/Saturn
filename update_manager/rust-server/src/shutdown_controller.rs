use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{watch, Notify};

static ACCEPTING_JOBS: AtomicBool = AtomicBool::new(true);
static SHUTDOWN_TX: OnceLock<watch::Sender<bool>> = OnceLock::new();
static ACTIVE: OnceLock<Mutex<BTreeMap<String, ActiveOperation>>> = OnceLock::new();
static CANCEL_REQUESTED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
static ACTIVE_CHANGED: OnceLock<Notify> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownPolicy {
    Finish,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveOperation {
    pub id: String,
    pub job_type: String,
    pub policy: ShutdownPolicy,
    pub process_group: Option<i32>,
}

pub struct ActiveOperationGuard {
    id: String,
}

impl ActiveOperationGuard {
    pub fn set_process_group(&self, process_group: i32) -> Result<(), String> {
        if process_group <= 0 {
            return Err("process group must be positive".to_string());
        }
        let policy = {
            let mut active = active_slot()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let operation = active
                .get_mut(&self.id)
                .ok_or_else(|| format!("active operation disappeared: {}", self.id))?;
            operation.process_group = Some(process_group);
            operation.policy
        };
        if is_shutting_down() && policy == ShutdownPolicy::Cancel {
            cancel_process_group(self.id.clone(), process_group);
        }
        Ok(())
    }
}

impl Drop for ActiveOperationGuard {
    fn drop(&mut self) {
        active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        cancel_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        active_changed().notify_waiters();
    }
}

fn active_slot() -> &'static Mutex<BTreeMap<String, ActiveOperation>> {
    ACTIVE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn cancel_slot() -> &'static Mutex<BTreeSet<String>> {
    CANCEL_REQUESTED.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn active_changed() -> &'static Notify {
    ACTIVE_CHANGED.get_or_init(Notify::new)
}

pub fn initialize(sender: watch::Sender<bool>) -> Result<(), String> {
    SHUTDOWN_TX
        .set(sender)
        .map_err(|_| "shutdown controller is already initialized".to_string())
}

pub fn is_shutting_down() -> bool {
    !ACCEPTING_JOBS.load(Ordering::Acquire)
}

pub fn ensure_accepting_jobs() -> Result<(), String> {
    if is_shutting_down() {
        Err("Saturn Go is shutting down; new maintenance jobs are not accepted".to_string())
    } else {
        Ok(())
    }
}

pub fn register(
    id: impl Into<String>,
    job_type: impl Into<String>,
    policy: ShutdownPolicy,
) -> Result<ActiveOperationGuard, String> {
    ensure_accepting_jobs()?;
    let id = id.into();
    let operation = ActiveOperation {
        id: id.clone(),
        job_type: job_type.into(),
        policy,
        process_group: None,
    };
    let mut active = active_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if is_shutting_down() {
        return Err(
            "Saturn Go is shutting down; new maintenance jobs are not accepted".to_string(),
        );
    }
    if active.contains_key(&id) {
        return Err(format!("maintenance operation ID is already active: {id}"));
    }
    active.insert(id.clone(), operation);
    Ok(ActiveOperationGuard { id })
}

pub fn cancel_requested(id: &str) -> bool {
    cancel_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(id)
}

fn mark_cancel_requested(id: &str) -> bool {
    cancel_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(id.to_string())
}

pub fn script_policy(script: &str) -> ShutdownPolicy {
    match script.to_ascii_lowercase().as_str() {
        // These installed maintenance helpers have no transactional commit
        // boundary and are explicitly safe to interrupt. An operator who
        // replaces one of these named scripts assumes the same contract.
        "cleanup-saturn-logs.sh"
        | "cleanup-saturn-backups.sh"
        | "log_cleaner.sh"
        | "g2-version-info.sh" => ShutdownPolicy::Cancel,
        // Updates, restores, deploys, flashes, network changes, and unknown
        // operator scripts conservatively finish. Unknown work must never be
        // called cancel-safe merely because the controller cannot classify it.
        _ => ShutdownPolicy::Finish,
    }
}

pub fn request_shutdown(source: impl Into<String>) -> bool {
    if ACCEPTING_JOBS
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let source = source.into();
    tracing::warn!("graceful shutdown requested by {source}; maintenance admission is closed");
    tokio::spawn(async move {
        drain_for_shutdown().await;
        if let Some(sender) = SHUTDOWN_TX.get() {
            let _ = sender.send(true);
        } else {
            tracing::error!("shutdown controller has no server signal sender");
        }
    });
    true
}

async fn drain_for_shutdown() {
    for (id, group) in cancel_groups() {
        cancel_process_group(id, group);
    }

    loop {
        let notified = active_changed().notified();
        let remaining = active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        if remaining == 0 {
            break;
        }
        tracing::info!("graceful shutdown waiting for {remaining} maintenance operation(s)");
        notified.await;
    }
}

fn cancel_process_group(id: String, process_group: i32) {
    cancel_process_group_after(id, process_group, cancel_grace());
}

fn cancel_process_group_after(id: String, process_group: i32, grace: Duration) {
    if !mark_cancel_requested(&id) {
        return;
    }
    signal_process_group(process_group, "TERM");
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        let still_active = active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
            .map(|operation| {
                operation.policy == ShutdownPolicy::Cancel
                    && operation.process_group == Some(process_group)
            })
            .unwrap_or(false);
        if still_active {
            signal_process_group(process_group, "KILL");
        }
    });
}

fn cancel_groups() -> Vec<(String, i32)> {
    active_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .filter(|operation| operation.policy == ShutdownPolicy::Cancel)
        .filter_map(|operation| {
            operation
                .process_group
                .map(|group| (operation.id.clone(), group))
        })
        .collect()
}

fn cancel_grace() -> Duration {
    let seconds = std::env::var("SATURN_JOB_CANCEL_GRACE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5)
        .clamp(1, 30);
    Duration::from_secs(seconds)
}

fn signal_process_group(process_group: i32, signal: &str) {
    if process_group <= 0 || !matches!(signal, "TERM" | "KILL") {
        return;
    }
    match std::process::Command::new("kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{process_group}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            tracing::warn!("failed to send SIG{signal} to process group {process_group}: {status}")
        }
        Err(error) => tracing::warn!(
            "failed to execute process-group SIG{signal} for {process_group}: {error}"
        ),
    }
}

pub async fn terminate_process_group(process_group: i32) {
    terminate_process_group_after(process_group, cancel_grace()).await;
}

async fn terminate_process_group_after(process_group: i32, grace: Duration) {
    if process_group <= 0 {
        return;
    }
    signal_process_group(process_group, "TERM");
    tokio::time::sleep(grace).await;
    if process_group_exists(process_group) {
        signal_process_group(process_group, "KILL");
    }
}

fn process_group_exists(process_group: i32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg("--")
        .arg(format!("-{process_group}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn status() -> serde_json::Value {
    let active = active_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .cloned()
        .collect::<Vec<_>>();
    serde_json::json!({
        "status": if is_shutting_down() { "shutting_down" } else { "accepting_jobs" },
        "accepting_jobs": !is_shutting_down(),
        "active_operations": active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;

    #[test]
    fn cleanup_scripts_cancel_and_mutating_or_unknown_scripts_finish() {
        assert_eq!(script_policy("update-G2.py"), ShutdownPolicy::Finish);
        assert_eq!(script_policy("flash_fpga.sh"), ShutdownPolicy::Finish);
        assert_eq!(script_policy("operator-custom.sh"), ShutdownPolicy::Finish);
        assert_eq!(
            script_policy("cleanup-saturn-logs.sh"),
            ShutdownPolicy::Cancel
        );
    }

    #[test]
    fn duplicate_registration_does_not_replace_the_original_operation() {
        let id = format!("duplicate-test-{}", std::process::id());
        let guard = register(id.clone(), "original", ShutdownPolicy::Finish).unwrap();
        let duplicate = register(id.clone(), "replacement", ShutdownPolicy::Cancel);
        assert!(duplicate.is_err());
        let active = active_slot().lock().unwrap();
        assert_eq!(active.get(&id).unwrap().job_type, "original");
        assert_eq!(active.get(&id).unwrap().policy, ShutdownPolicy::Finish);
        drop(active);
        drop(guard);
    }

    #[test]
    fn process_group_signal_terminates_complete_test_group() {
        let mut command = std::process::Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo $!; wait"])
            .stdout(std::process::Stdio::piped())
            .process_group(0);
        let mut child = command.spawn().unwrap();
        let process_group = child.id() as i32;
        let mut background_pid = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut background_pid)
            .unwrap();
        assert!(background_pid.trim().parse::<u32>().is_ok());
        signal_process_group(process_group, "TERM");
        let status = child.wait().unwrap();
        assert!(!status.success());

        for _ in 0..50 {
            let group_exists = std::process::Command::new("kill")
                .args(["-0", "--", &format!("-{process_group}")])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if !group_exists {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("process group {process_group} survived SIGTERM");
    }

    #[tokio::test]
    async fn cancellation_escalates_for_a_term_resistant_process_group() {
        let id = format!("cancel-escalation-test-{}", std::process::id());
        let guard = register(id.clone(), "cancel-test", ShutdownPolicy::Cancel).unwrap();
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", "trap '' TERM; while :; do sleep 30; done"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().unwrap();
        let process_group = child.id().unwrap() as i32;
        guard.set_process_group(process_group).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;

        cancel_process_group_after(id.clone(), process_group, Duration::from_millis(30));
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .expect("SIGKILL escalation timed out")
            .unwrap();
        assert!(!status.success());
        assert!(cancel_requested(&id));
        drop(guard);
        assert!(!cancel_requested(&id));
    }

    #[tokio::test]
    async fn deadline_termination_escalates_for_a_term_resistant_process_group() {
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", "trap '' TERM; while :; do sleep 30; done"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().unwrap();
        let process_group = child.id().unwrap() as i32;
        tokio::time::sleep(Duration::from_millis(30)).await;

        terminate_process_group_after(process_group, Duration::from_millis(30)).await;
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .expect("deadline SIGKILL escalation timed out")
            .unwrap();
        assert!(!status.success());
    }
}
