use std::time::Duration;

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    Json,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

const HELPER_PATH: &str = "/usr/local/lib/saturn-go/scripts/saturn-tailscale.sh";
const HOSTNAME_RE_BYTES: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-";

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

fn stream_helper(args: Vec<String>) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<String>();

    let cmd_summary: String = std::iter::once("sudo".to_string())
        .chain(std::iter::once(HELPER_PATH.to_string()))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let _ = tx.send(format!("$ {cmd_summary}"));

    let mut command = Command::new("sudo");
    command
        .arg("-n")
        .arg(HELPER_PATH)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(format!("Error: failed to spawn helper: {e}"));
            let stream = UnboundedReceiverStream::new(rx)
                .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
            return Sse::new(stream)
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
                .into_response();
        }
    };

    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(format!("ERR: {line}"));
            }
        });
    }

    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) if status.success() => {
                let _ = tx.send("Done".to_string());
            }
            Ok(status) => {
                let _ = tx.send(format!("Error: helper exited with {status}"));
            }
            Err(e) => {
                let _ = tx.send(format!("Error: wait failed: {e}"));
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx)
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
    stream_helper(vec!["install".to_string()])
}

pub async fn tailscale_up(payload: Option<Json<TailscaleUpRequest>>) -> Response {
    let req = payload.map(|Json(p)| p).unwrap_or_default();
    let mut args = vec!["up".to_string()];

    if let Some(key) = req.auth_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Err(e) = validate_auth_key(key) {
            return bad_request(e);
        }
        args.push(format!("--auth-key={key}"));
    }
    if let Some(host) = req.hostname.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
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

    stream_helper(args)
}

pub async fn tailscale_down() -> Response {
    stream_helper(vec!["down".to_string()])
}

pub async fn tailscale_logout() -> Response {
    stream_helper(vec!["logout".to_string()])
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
    stream_helper(args)
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
