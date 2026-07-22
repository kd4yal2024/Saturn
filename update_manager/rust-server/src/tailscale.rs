use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    Json,
};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use crate::bounded_output::BoundedOutputSender;
use crate::maintenance_lock::{self, NETWORK_MAINTENANCE};
use crate::shutdown_controller::{self, ShutdownPolicy};

const HELPER_PATH: &str = "/usr/local/lib/saturn-go/scripts/saturn-tailscale.sh";
const HOSTNAME_RE_BYTES: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-";
const TAILSCALE_HELPER_DEADLINE_SECS: u64 = 10 * 60;

#[derive(Debug, Default, Deserialize)]
pub struct TailscaleUpRequest {
    #[serde(default)]
    pub auth_key: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub ssh: bool,
    #[serde(default)]
    pub accept_routes: bool,
    #[serde(default)]
    pub accept_dns: bool,
    #[serde(default)]
    pub reset: bool,
}

#[derive(Debug, Deserialize)]
pub struct TailscaleServeRequest {
    pub enable: bool,
    #[serde(default)]
    pub port: Option<u16>,
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, msg.to_string()).into_response()
}

fn validate_hostname(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 63 {
        return Err("hostname must be 1..63 chars");
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err("hostname must start with a letter or digit");
    }
    for &b in bytes {
        if !HOSTNAME_RE_BYTES.contains(&b) {
            return Err("hostname may only contain letters, digits, and hyphens");
        }
    }
    Ok(())
}

fn validate_auth_key(value: &str) -> Result<(), &'static str> {
    if !value.starts_with("tskey-") {
        return Err("auth key must begin with 'tskey-'");
    }
    if value.len() < 14 || value.len() > 262 {
        return Err("auth key length out of range");
    }
    for ch in value.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            return Err("auth key contains an invalid character");
        }
    }
    Ok(())
}

async fn stream_bounded_chunks<R>(mut reader: R, tx: BoundedOutputSender, prefix: &'static str)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 2048];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                let chunk = String::from_utf8_lossy(&buffer[..count]);
                for line in chunk.split(['\n', '\r']).filter(|line| !line.is_empty()) {
                    tx.try_send(format!("{prefix}{line}"));
                }
            }
            Err(error) => {
                tx.try_send(format!("{prefix}stream read error: {error}"));
                break;
            }
        }
    }
}

async fn stream_helper(args: Vec<String>) -> Response {
    let action = args.first().map(String::as_str).unwrap_or("unknown");
    let operation = format!("tailscale:{action}");
    if let Err(error) = maintenance_lock::probe(&operation, NETWORK_MAINTENANCE).await {
        return (StatusCode::CONFLICT, error).into_response();
    }
    let operation_id = format!(
        "tailscale-{}-{}",
        action,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    let operation_guard = match shutdown_controller::register(
        operation_id,
        operation.clone(),
        ShutdownPolicy::Finish,
    ) {
        Ok(guard) => guard,
        Err(error) => return (StatusCode::SERVICE_UNAVAILABLE, error).into_response(),
    };
    let (tx, rx) = BoundedOutputSender::channel();

    let cmd_summary: String = std::iter::once("sudo".to_string())
        .chain(std::iter::once(HELPER_PATH.to_string()))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    tx.try_send(format!("$ {cmd_summary}"));

    let mut command_args = vec!["-n".to_string(), HELPER_PATH.to_string()];
    command_args.extend(args.iter().cloned());
    let mut command = maintenance_lock::wrapped_command(
        &operation,
        NETWORK_MAINTENANCE,
        Path::new("sudo"),
        &command_args,
    );
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            tx.send_terminal(format!("Error: failed to spawn helper: {e}"))
                .await;
            let stream = ReceiverStream::new(rx)
                .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
            return Sse::new(stream)
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
                .into_response();
        }
    };
    let Some(child_pid) = child.id() else {
        let _ = child.start_kill();
        tx.send_terminal("Error: maintenance helper did not report a PID".to_string())
            .await;
        let stream = ReceiverStream::new(rx)
            .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
        return Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
            .into_response();
    };
    if let Err(error) = operation_guard.set_process_group(child_pid as i32) {
        let _ = child.start_kill();
        tx.send_terminal(format!(
            "Error: failed to track maintenance helper: {error}"
        ))
        .await;
        let stream = ReceiverStream::new(rx)
            .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
        return Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
            .into_response();
    }

    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            stream_bounded_chunks(stdout, tx, "").await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            stream_bounded_chunks(stderr, tx, "ERR: ").await;
        });
    }

    tokio::spawn(async move {
        let _operation_guard = operation_guard;
        let terminal = tokio::time::timeout(
            Duration::from_secs(TAILSCALE_HELPER_DEADLINE_SECS),
            child.wait(),
        )
        .await;
        match terminal {
            Err(_) => {
                shutdown_controller::terminate_process_group(child_pid as i32).await;
                let _ = child.wait().await;
                tx.send_terminal(format!(
                    "Error: helper timed out after {TAILSCALE_HELPER_DEADLINE_SECS} seconds"
                ))
                .await;
            }
            Ok(Ok(status)) if status.success() => {
                tx.send_terminal("Done".to_string()).await;
            }
            Ok(Ok(status)) => {
                tx.send_terminal(format!("Error: helper exited with {status}"))
                    .await;
            }
            Ok(Err(e)) => {
                tx.send_terminal(format!("Error: wait failed: {e}")).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx)
        .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
    let mut resp = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    resp
}

pub async fn tailscale_install() -> Response {
    stream_helper(vec!["install".to_string()]).await
}

pub async fn tailscale_up(payload: Option<Json<TailscaleUpRequest>>) -> Response {
    let req = payload.map(|Json(p)| p).unwrap_or_default();
    let mut args = vec!["up".to_string()];

    if let Some(key) = req
        .auth_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Err(e) = validate_auth_key(key) {
            return bad_request(e);
        }
        args.push(format!("--auth-key={key}"));
    }
    if let Some(host) = req
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Err(e) = validate_hostname(host) {
            return bad_request(e);
        }
        args.push(format!("--hostname={host}"));
    }
    if req.ssh {
        args.push("--ssh".to_string());
    }
    if req.accept_routes {
        args.push("--accept-routes".to_string());
    }
    if req.accept_dns {
        args.push("--accept-dns".to_string());
    }
    if req.reset {
        args.push("--reset".to_string());
    }

    stream_helper(args).await
}

pub async fn tailscale_down() -> Response {
    stream_helper(vec!["down".to_string()]).await
}

pub async fn tailscale_logout() -> Response {
    stream_helper(vec!["logout".to_string()]).await
}

pub async fn tailscale_serve(Json(req): Json<TailscaleServeRequest>) -> Response {
    let mut args = if req.enable {
        vec!["serve-on".to_string()]
    } else {
        vec!["serve-off".to_string()]
    };
    if req.enable {
        if let Some(port) = req.port {
            if port == 0 {
                return bad_request("port must be 1..65535");
            }
            args.push(format!("--port={port}"));
        }
    }
    stream_helper(args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_accepts_simple() {
        assert!(validate_hostname("saturn-g2").is_ok());
        assert!(validate_hostname("kd4yal-saturn").is_ok());
        assert!(validate_hostname("a").is_ok());
    }

    #[test]
    fn hostname_rejects_bad() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("-leading").is_err());
        assert!(validate_hostname("has space").is_err());
        assert!(validate_hostname("has.dot").is_err());
        assert!(validate_hostname(&"x".repeat(64)).is_err());
    }

    #[test]
    fn auth_key_accepts_valid_prefix() {
        assert!(validate_auth_key("tskey-abcd_1234-EFGH").is_ok());
    }

    #[test]
    fn auth_key_rejects_wrong_prefix_or_chars() {
        assert!(validate_auth_key("not-a-key").is_err());
        assert!(validate_auth_key("tskey-bad char").is_err());
        assert!(validate_auth_key("tskey-bad/char").is_err());
        assert!(validate_auth_key("tskey-").is_err());
    }
}
