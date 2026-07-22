use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::shutdown_controller::{self, ShutdownPolicy};
use crate::util::output_error_text;

#[derive(Deserialize)]
pub struct PasswordForm {
    new_password: String,
}

/// Privileged helper that updates BOTH auth backends together (nginx
/// htpasswd + the Saturn Remote TLS drop-in) so they cannot drift, then
/// schedules a deferred saturn-go restart to apply the TLS-side change.
const PASSWORD_HELPER: &str = "/usr/local/lib/saturn-go/scripts/saturn-admin-password.sh";
const PASSWORD_MIN_LEN: usize = 5;

fn validate_new_password(new_password: &str) -> Result<(), &'static str> {
    if new_password.chars().count() < PASSWORD_MIN_LEN {
        return Err("password must be at least 5 characters");
    }
    if new_password.chars().any(char::is_control) {
        return Err("password must not contain control characters");
    }
    Ok(())
}

async fn run_password_helper(new_password: &str) -> Result<std::process::Output, std::io::Error> {
    let mut cmd = Command::new("sudo");
    cmd.arg("-n")
        .arg(PASSWORD_HELPER)
        .arg("set")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(new_password.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
    }
    child.wait_with_output().await
}

pub async fn change_password(
    axum::extract::Form(form): axum::extract::Form<PasswordForm>,
) -> impl IntoResponse {
    if let Err(message) = validate_new_password(&form.new_password) {
        return Json(serde_json::json!({
            "status":"error",
            "message": message
        }));
    }

    let operation_id = format!(
        "password-change-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    let _operation_guard = match shutdown_controller::register(
        operation_id,
        "password-change",
        ShutdownPolicy::Finish,
    ) {
        Ok(guard) => guard,
        Err(error) => {
            return Json(serde_json::json!({ "status": "error", "message": error }));
        }
    };

    match run_password_helper(&form.new_password).await {
        Ok(out) if out.status.success() => Json(serde_json::json!({
            "status":"success",
            "message":"Password updated for LAN and remote. Remote (TLS) sessions reconnect in a few seconds."
        })),
        Ok(out) => {
            let detail = output_error_text(&out);
            let msg = if detail.contains("a password is required")
                || detail.contains("no tty present")
                || detail.contains("is not allowed to execute")
                || detail.contains("command not found")
                || detail.contains("No such file")
            {
                "Password change requires the saturn-admin-password.sh helper and its sudoers entry (rerun the Saturn Go installer or update).".to_string()
            } else {
                format!("password helper failed: {detail}")
            };
            Json(serde_json::json!({ "status":"error", "message": msg }))
        }
        Err(e) => Json(serde_json::json!({ "status":"error", "message": e.to_string() })),
    }
}

pub async fn exit_server(headers: HeaderMap) -> impl IntoResponse {
    let remote = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let started = shutdown_controller::request_shutdown(format!("admin API from {remote}"));
    Json(serde_json::json!({
        "status": "shutting_down",
        "started": started,
        "message": if started {
            "Maintenance admission is closed; Saturn Go will stop after active jobs reach their declared shutdown boundary."
        } else {
            "Graceful shutdown is already in progress."
        }
    }))
}

#[derive(Deserialize)]
pub struct KillQuery {
    sig: Option<String>, // term|kill
}

pub async fn kill_process(
    axum::extract::Path(pid): axum::extract::Path<i32>,
    Query(kq): Query<KillQuery>,
) -> impl IntoResponse {
    if pid <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "message": "bad pid" })),
        )
            .into_response();
    }
    if is_protected_pid(pid) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "message": "Protected process" })),
        )
            .into_response();
    }
    let sig = kq.sig.as_deref().unwrap_or("term");
    let signal = match sig {
        "kill" => "-9",
        "term" => "-15",
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "message": "invalid signal" })),
            )
                .into_response()
        }
    };

    let output = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            (StatusCode::OK, Json(serde_json::json!({ "message": "OK" }))).into_response()
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let stderr_lc = stderr.to_lowercase();
            let status = if stderr_lc.contains("no such process") {
                StatusCode::NOT_FOUND
            } else if stderr_lc.contains("operation not permitted") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            let msg = if stderr.is_empty() {
                format!("Failed: {}", o.status)
            } else {
                format!("Failed: {stderr}")
            };
            (status, Json(serde_json::json!({ "message": msg }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": format!("Failed: {e}") })),
        )
            .into_response(),
    }
}

fn is_protected_pid(pid: i32) -> bool {
    if pid <= 2 {
        return true;
    }
    if let Ok(data) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        return uid_is_root_from_status(&data);
    }
    false
}

/// Parse a /proc/PID/status blob and return true if the real UID (Uid: field,
/// first value) is 0. Extracted so it can be unit-tested without /proc access.
pub(crate) fn uid_is_root_from_status(status: &str) -> bool {
    for line in status.lines() {
        if line.starts_with("Uid:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1] == "0";
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use tower::ServiceExt;

    fn kill_router() -> axum::Router {
        axum::Router::new().route("/kill_process/{pid}", post(kill_process))
    }

    // --- change_password validation ---

    /// Passwords shorter than 5 characters must be rejected before any
    /// helper call is made (no external process needed for this path).
    #[tokio::test]
    async fn test_change_password_rejects_short() {
        let app = axum::Router::new().route("/change_password", post(change_password));
        let req = Request::builder()
            .method("POST")
            .uri("/change_password")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("new_password=abc"))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK); // handler returns 200 with error JSON
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "error");
        assert!(json["message"].as_str().unwrap().contains("at least 5"));
    }

    #[test]
    fn test_change_password_accepts_minimum_and_longer_values() {
        assert!(validate_new_password("abc12").is_ok());
        assert!(validate_new_password("a-longer-password").is_ok());
    }

    #[test]
    fn test_change_password_rejects_control_characters() {
        assert_eq!(
            validate_new_password("abc12\n"),
            Err("password must not contain control characters")
        );
    }

    // --- kill_process validation ---

    /// PID 0 must return 400.
    #[tokio::test]
    async fn test_kill_process_rejects_zero_pid() {
        let app = kill_router();
        let req = Request::builder()
            .method("POST")
            .uri("/kill_process/0")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// An unknown signal name must return 400.
    #[tokio::test]
    async fn test_kill_process_rejects_bad_signal() {
        let app = kill_router();
        let req = Request::builder()
            .method("POST")
            .uri("/kill_process/99999?sig=hup") // valid PID range, invalid sig name
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    // --- is_protected_pid / uid_is_root_from_status ---

    /// pid <= 2 is always protected regardless of /proc content.
    #[test]
    fn test_protected_pid_low_values() {
        assert!(is_protected_pid(0));
        assert!(is_protected_pid(1));
        assert!(is_protected_pid(2));
    }

    #[test]
    fn test_uid_is_root_true() {
        let status = "Name:\tinit\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n";
        assert!(uid_is_root_from_status(status));
    }

    #[test]
    fn test_uid_is_root_false_for_nonroot() {
        let status = "Name:\tpi\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert!(!uid_is_root_from_status(status));
    }

    #[test]
    fn test_uid_is_root_false_for_missing_uid_line() {
        let status = "Name:\tpi\nGid:\t1000\t1000\t1000\t1000\n";
        assert!(!uid_is_root_from_status(status));
    }
}
