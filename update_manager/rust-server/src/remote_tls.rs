use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
};

use axum::{
    extract::{
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use tracing::{error, info, warn};

use crate::{
    pages::{healthz, remote_handler},
    state::AppState,
};

type RemoteTlsResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_REMOTE_TLS_ADDR: &str = "0.0.0.0:8443";

pub struct RemoteTlsConfig {
    pub addr: Option<SocketAddr>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

pub fn load_remote_tls_config(default_state_dir: &str) -> RemoteTlsResult<RemoteTlsConfig> {
    let addr = match std::env::var("SATURN_REMOTE_TLS_ADDR") {
        Ok(raw) => parse_remote_tls_addr(&raw)?,
        Err(_) => Some(DEFAULT_REMOTE_TLS_ADDR.parse()?),
    };

    let cert_path = std::env::var("SATURN_REMOTE_TLS_CERT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/remote-tls/saturn-remote.crt")));
    let key_path = std::env::var("SATURN_REMOTE_TLS_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/remote-tls/saturn-remote.key")));

    Ok(RemoteTlsConfig {
        addr,
        cert_path,
        key_path,
    })
}

pub async fn ensure_self_signed_cert(cert_path: &Path, key_path: &Path) -> RemoteTlsResult<()> {
    if cert_path.exists() && key_path.exists() {
        return Ok(());
    }

    if let Some(parent) = cert_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some(parent) = key_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let sans = collect_subject_alt_names();
    let mut params = CertificateParams::default();
    params.subject_alt_names = sans;
    params.is_ca = IsCa::NoCa;

    let common_name = preferred_common_name();
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, common_name);
    distinguished_name.push(DnType::OrganizationName, "Saturn Remote");
    params.distinguished_name = distinguished_name;

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    tokio::fs::write(cert_path, cert.pem()).await?;
    tokio::fs::write(key_path, key_pair.serialize_pem()).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(cert_path, std::fs::Permissions::from_mode(0o644))?;
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    info!(
        "generated Saturn Remote self-signed certificate at {}",
        cert_path.display()
    );
    Ok(())
}

pub fn remote_tls_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(remote_handler))
        .route("/remote", get(remote_handler))
        .route("/remote.html", get(remote_handler))
        .route("/saturn-remote", get(remote_handler))
        .route("/saturn-remote.html", get(remote_handler))
        .route("/healthz", get(healthz))
        .route("/tci", get(remote_bridge_ws_handler))
        .with_state(state)
}

async fn remote_bridge_ws_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_ws_origin(&headers) {
        return rejection;
    }
    ws.on_upgrade(move |socket| proxy_bridge_socket(socket, state.bridge_ws_url))
}

/// Reject WebSocket upgrades whose Origin header names a different host than
/// the request Host.  Browsers always send Origin on WebSocket handshakes, so
/// a mismatch indicates a cross-origin request from another page.
/// Native (non-browser) clients that omit Origin entirely are allowed through.
fn check_ws_origin(headers: &HeaderMap) -> Result<(), Response> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Ok(()); // no Origin header — native client, allow
    };
    if origin.eq_ignore_ascii_case("null") {
        // Opaque origin (sandboxed iframe, data: URL, etc.) — reject.
        return Err((StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response());
    }
    let origin_host = ws_host_from_url(origin)
        .ok_or_else(|| (StatusCode::FORBIDDEN, "invalid Origin header").into_response())?;
    let request_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(ws_host_from_authority)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing Host header").into_response())?;
    if origin_host != request_host {
        return Err((StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response());
    }
    Ok(())
}

fn ws_host_from_authority(value: &str) -> Option<String> {
    let authority = value.trim().rsplit('@').next().unwrap_or(value.trim()).trim();
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        // IPv6 literal — include the brackets, drop the port
        let end = authority.find(']')?;
        return Some(authority[..=end].to_ascii_lowercase());
    }
    let host = authority.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    if host.is_empty() { None } else { Some(host) }
}

fn ws_host_from_url(value: &str) -> Option<String> {
    let scheme_end = value.find("://")?;
    let rest = &value[scheme_end + 3..];
    let authority = rest.split('/').next().unwrap_or("");
    ws_host_from_authority(authority)
}

async fn proxy_bridge_socket(client: WebSocket, bridge_ws_url: String) {
    let (bridge, _) = match connect_async(&bridge_ws_url).await {
        Ok(connection) => connection,
        Err(err) => {
            error!("remote TLS bridge proxy connect failed: {err}");
            let mut client = client;
            let _ = client.send(AxumMessage::Close(None)).await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client.split();
    let (mut bridge_tx, mut bridge_rx) = bridge.split();

    let client_to_bridge = async {
        while let Some(message) = client_rx.next().await {
            let message = message.map_err(|err| format!("client websocket error: {err}"))?;
            match message {
                AxumMessage::Text(text) => {
                    bridge_tx
                        .send(TungsteniteMessage::Text(text.to_string().into()))
                        .await
                        .map_err(|err| format!("bridge websocket send failed: {err}"))?;
                }
                AxumMessage::Binary(bytes) => {
                    bridge_tx
                        .send(TungsteniteMessage::Binary(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("bridge websocket send failed: {err}"))?;
                }
                AxumMessage::Ping(bytes) => {
                    bridge_tx
                        .send(TungsteniteMessage::Ping(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("bridge websocket ping failed: {err}"))?;
                }
                AxumMessage::Pong(bytes) => {
                    bridge_tx
                        .send(TungsteniteMessage::Pong(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("bridge websocket pong failed: {err}"))?;
                }
                AxumMessage::Close(_) => {
                    let _ = bridge_tx.send(TungsteniteMessage::Close(None)).await;
                    break;
                }
            }
        }
        Ok::<(), String>(())
    };

    let bridge_to_client = async {
        while let Some(message) = bridge_rx.next().await {
            let message = message.map_err(|err| format!("bridge websocket error: {err}"))?;
            match message {
                TungsteniteMessage::Text(text) => {
                    client_tx
                        .send(AxumMessage::Text(text.to_string().into()))
                        .await
                        .map_err(|err| format!("client websocket send failed: {err}"))?;
                }
                TungsteniteMessage::Binary(bytes) => {
                    client_tx
                        .send(AxumMessage::Binary(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("client websocket send failed: {err}"))?;
                }
                TungsteniteMessage::Ping(bytes) => {
                    client_tx
                        .send(AxumMessage::Ping(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("client websocket ping failed: {err}"))?;
                }
                TungsteniteMessage::Pong(bytes) => {
                    client_tx
                        .send(AxumMessage::Pong(bytes.to_vec().into()))
                        .await
                        .map_err(|err| format!("client websocket pong failed: {err}"))?;
                }
                TungsteniteMessage::Close(_) => {
                    let _ = client_tx.send(AxumMessage::Close(None)).await;
                    break;
                }
                TungsteniteMessage::Frame(_) => {}
            }
        }
        Ok::<(), String>(())
    };

    let result = tokio::select! {
        result = client_to_bridge => result,
        result = bridge_to_client => result,
    };

    if let Err(err) = result {
        warn!("remote TLS websocket proxy ended with error: {err}");
    }
}

fn parse_remote_tls_addr(raw: &str) -> RemoteTlsResult<Option<SocketAddr>> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("disabled")
        || trimmed == "0"
    {
        return Ok(None);
    }
    Ok(Some(trimmed.parse()?))
}

fn preferred_common_name() -> String {
    if let Ok(hostname) = hostname::get() {
        let hostname = hostname.to_string_lossy().trim().to_string();
        if !hostname.is_empty() {
            return hostname;
        }
    }
    "saturn-remote".to_string()
}

fn collect_subject_alt_names() -> Vec<SanType> {
    let mut dns_names = BTreeSet::new();
    let mut ip_addrs = BTreeSet::new();

    dns_names.insert("localhost".to_string());
    ip_addrs.insert(IpAddr::V4(Ipv4Addr::LOCALHOST));
    ip_addrs.insert(IpAddr::V6(Ipv6Addr::LOCALHOST));

    if let Ok(hostname) = hostname::get() {
        let hostname = hostname.to_string_lossy().trim().to_string();
        if !hostname.is_empty() {
            dns_names.insert(hostname);
        }
    }

    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            let ip = iface.ip();
            if !ip.is_unspecified() {
                ip_addrs.insert(ip);
            }
        }
    }

    if let Ok(extra) = std::env::var("SATURN_REMOTE_TLS_EXTRA_SANS") {
        for value in extra.split(',').map(str::trim).filter(|value| !value.is_empty()) {
            if let Ok(ip) = value.parse::<IpAddr>() {
                ip_addrs.insert(ip);
            } else {
                dns_names.insert(value.to_string());
            }
        }
    }

    let mut sans = Vec::with_capacity(dns_names.len() + ip_addrs.len());
    for name in dns_names {
        if let Ok(name) = name.try_into() {
            sans.push(SanType::DnsName(name));
        }
    }
    for ip in ip_addrs {
        sans.push(SanType::IpAddress(ip));
    }
    sans
}
