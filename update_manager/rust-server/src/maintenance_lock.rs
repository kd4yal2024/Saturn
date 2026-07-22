use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

const DEFAULT_TOOL: &str = "/usr/local/lib/saturn-go/scripts/saturn-maintenance-lock.py";
const DEFAULT_LOCK_DIR: &str = "/run/lock/saturn-maintenance";

pub const RELEASE: &str = "release";
pub const REPOSITORY: &str = "repository";
pub const DISK: &str = "disk";
pub const FPGA: &str = "fpga";
pub const PACKAGE: &str = "package";
pub const NETWORK: &str = "network";
pub const RADIO: &str = "radio";
pub const READ_ONLY: &str = "read-only";

pub const APPLICATION_DEPLOYMENT: &[&str] = &[RELEASE, REPOSITORY, PACKAGE, RADIO];
pub const REPOSITORY_RESTORE: &[&str] = &[REPOSITORY, RADIO];
pub const NETWORK_MAINTENANCE: &[&str] = &[NETWORK];
pub const READ_ONLY_MAINTENANCE: &[&str] = &[READ_ONLY];
const UNKNOWN_CUSTOM_SCRIPT: &[&str] = &[RELEASE, REPOSITORY, DISK, FPGA, PACKAGE, NETWORK, RADIO];

fn tool() -> PathBuf {
    std::env::var("SATURN_MAINTENANCE_LOCK_TOOL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_TOOL))
}

fn lock_dir() -> PathBuf {
    std::env::var("SATURN_MAINTENANCE_LOCK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_LOCK_DIR))
}

fn resource_argument(resources: &[&str]) -> String {
    resources.join(",")
}

fn base_command(action: &str, operation: &str, resources: &[&str]) -> Command {
    let mut command = Command::new(tool());
    command
        .arg("--lock-dir")
        .arg(lock_dir())
        .arg(action)
        .arg("--operation")
        .arg(operation)
        .arg("--resources")
        .arg(resource_argument(resources));
    command
}

fn command_error(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("maintenance lock helper exited with {}", output.status)
    }
}

pub async fn probe(operation: &str, resources: &[&str]) -> Result<(), String> {
    let output = base_command("probe", operation, resources)
        .output()
        .await
        .map_err(|error| format!("failed to start maintenance lock helper: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(&output))
    }
}

pub fn wrapped_command(
    operation: &str,
    resources: &[&str],
    program: &Path,
    arguments: &[String],
) -> Command {
    let mut command = base_command("run", operation, resources);
    command.arg("--").arg(program).args(arguments);
    command.as_std_mut().process_group(0);
    command
}

pub fn wrapped_job_command(
    operation: &str,
    resources: &[&str],
    program: &Path,
    arguments: &[String],
    job_id: &str,
    output_path: &Path,
    result_path: &Path,
) -> Command {
    let mut command = base_command("run", operation, resources);
    command
        .arg("--job-id")
        .arg(job_id)
        .arg("--output-file")
        .arg(output_path)
        .arg("--result-file")
        .arg(result_path)
        .arg("--")
        .arg(program)
        .args(arguments);
    // The lock broker becomes the process-group leader and its maintenance
    // child inherits that group. The group survives a Saturn Go restart and
    // gives startup reconciliation an identity independent of the server PID.
    command.as_std_mut().process_group(0);
    command
}

pub struct HostLockGuard {
    child: Child,
    _stdin: ChildStdin,
}

impl HostLockGuard {
    pub async fn acquire(operation: &str, resources: &[&str]) -> Result<Self, String> {
        let mut child = base_command("hold", operation, resources);
        child
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = child
            .spawn()
            .map_err(|error| format!("failed to start maintenance lock holder: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "maintenance lock holder has no stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "maintenance lock holder has no stdout".to_string())?;
        let mut lines = BufReader::new(stdout).lines();
        let ready = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .map_err(|_| "maintenance lock holder timed out".to_string())?
            .map_err(|error| format!("maintenance lock holder output failed: {error}"))?;
        match ready {
            Some(line) if line.contains("\"status\":\"locked\"") => Ok(Self {
                child,
                _stdin: stdin,
            }),
            Some(line) => {
                let status = child.wait().await.ok();
                Err(format!(
                    "maintenance lock holder failed{}: {line}",
                    status
                        .map(|value| format!(" ({value})"))
                        .unwrap_or_default()
                ))
            }
            None => {
                let output = child.wait_with_output().await.map_err(|error| {
                    format!("maintenance lock holder ended without output: {error}")
                })?;
                Err(command_error(&output))
            }
        }
    }
}

impl Drop for HostLockGuard {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub fn script_resources(script: &str) -> &'static [&'static str] {
    match script.to_ascii_lowercase().as_str() {
        "update-g2.py" | "update-g2.sh" | "update-saturn-go.sh" => APPLICATION_DEPLOYMENT,
        "update-pihpsdr.py" | "update-deskhpsdr.py" => &[REPOSITORY, PACKAGE, RADIO],
        "flash_fpga.sh" => &[FPGA, RADIO],
        "restore-backup.sh" | "cleanup-saturn-backups.sh" => REPOSITORY_RESTORE,
        "setup-eth-fallback.sh" | "saturn-tailscale.sh" => NETWORK_MAINTENANCE,
        "log_cleaner.sh" | "cleanup-saturn-logs.sh" => READ_ONLY_MAINTENANCE,
        "fix-led-power-button.sh" => &[RADIO],
        _ => UNKNOWN_CUSTOM_SCRIPT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_deployment_does_not_claim_disk_or_fpga() {
        let resources = script_resources("update-G2.py");
        assert!(!resources.contains(&DISK));
        assert!(!resources.contains(&FPGA));
        assert!(resources.contains(&RELEASE));
        assert!(resources.contains(&REPOSITORY));
    }

    #[test]
    fn destructive_unknown_custom_scripts_are_conservative() {
        let resources = script_resources("operator-maintenance.sh");
        assert!(resources.contains(&DISK));
        assert!(resources.contains(&FPGA));
        assert!(resources.contains(&NETWORK));
        assert!(resources.contains(&RADIO));
    }

    #[test]
    fn dedicated_operations_have_separate_resources() {
        assert_eq!(script_resources("flash_fpga.sh"), &[FPGA, RADIO]);
        assert_eq!(script_resources("setup-eth-fallback.sh"), &[NETWORK]);
        assert_eq!(script_resources("cleanup-saturn-logs.sh"), &[READ_ONLY]);
    }
}
