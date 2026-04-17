use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use axum::{
    extract::{
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use tracing::{error, info, warn};

use crate::{
    delete_remote_profile, get_remote_profiles, get_remote_settings,
    pages::{healthz, serve_page},
    save_remote_profile, set_remote_profile_startup, set_remote_settings,
    state::{
        AppState, RemoteProfileDeleteRequest, RemoteProfileSaveRequest,
        RemoteProfileStartupRequest, RemoteSettings,
    },
};

type RemoteTlsResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_REMOTE_TLS_ADDR: &str = "0.0.0.0:8443";
const REMOTE_BASIC_AUTH_ENV: &str = "SATURN_REMOTE_BASIC_AUTH";
const REMOTE_BASIC_AUTH_CHALLENGE: &str = "Basic realm=\"Saturn Remote\", charset=\"UTF-8\"";

#[derive(Clone, Debug, Eq, PartialEq)]
struct NormalizedAuthority {
    host: String,
    port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OriginAuthority {
    authority: NormalizedAuthority,
    default_port: u16,
}

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
        .unwrap_or_else(|_| {
            PathBuf::from(format!("{default_state_dir}/remote-tls/saturn-remote.crt"))
        });
    let key_path = std::env::var("SATURN_REMOTE_TLS_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!("{default_state_dir}/remote-tls/saturn-remote.key"))
        });

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
        .route("/", get(remote_page_handler))
        .route("/remote", get(remote_page_handler))
        .route("/remote.html", get(remote_page_handler))
        .route("/saturn-remote", get(remote_page_handler))
        .route("/saturn-remote.html", get(remote_page_handler))
        .route("/healthz", get(healthz))
        .route("/remote_settings", get(remote_settings_get_handler))
        .route("/remote_settings", post(remote_settings_post_handler))
        .route("/remote_profiles", get(remote_profiles_get_handler))
        .route("/remote_profiles/save", post(remote_profiles_save_handler))
        .route(
            "/remote_profiles/delete",
            post(remote_profiles_delete_handler),
        )
        .route(
            "/remote_profiles/startup",
            post(remote_profiles_startup_handler),
        )
        .route("/tci", get(remote_bridge_ws_handler))
        .with_state(state)
}

async fn remote_page_handler(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    serve_page(&state.webroot, "saturn-remote.html").await
}

pub async fn remote_bridge_ws_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    if let Err(rejection) = check_ws_origin(&headers) {
        return rejection;
    }
    ws.on_upgrade(move |socket| proxy_bridge_socket(socket, state.bridge_ws_url))
}

async fn remote_settings_get_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    get_remote_settings(State(state)).await
}

async fn remote_settings_post_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(settings): Json<RemoteSettings>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    set_remote_settings(State(state), Json(settings)).await
}

async fn remote_profiles_get_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    get_remote_profiles(State(state)).await
}

async fn remote_profiles_save_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileSaveRequest>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    save_remote_profile(State(state), Json(request)).await
}

async fn remote_profiles_delete_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileDeleteRequest>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    delete_remote_profile(State(state), Json(request)).await
}

async fn remote_profiles_startup_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileStartupRequest>,
) -> Response {
    if let Err(rejection) = check_remote_basic_auth(&headers) {
        return rejection;
    }
    set_remote_profile_startup(State(state), Json(request)).await
}

/// Reject WebSocket upgrades whose Origin authority differs from the request
/// Host authority. This is the browser-facing proxy path, so require an Origin
/// header and treat host+port mismatches as cross-origin.
fn check_ws_origin(headers: &HeaderMap) -> Result<(), Response> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Err((StatusCode::FORBIDDEN, "missing Origin header").into_response());
    };
    if origin.eq_ignore_ascii_case("null") {
        // Opaque origin (sandboxed iframe, data: URL, etc.) — reject.
        return Err((StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response());
    }
    let origin_authority = origin_authority_from_url(origin)
        .ok_or_else(|| (StatusCode::FORBIDDEN, "invalid Origin header").into_response())?;
    let request_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| authority_from_host_header(v, Some(origin_authority.default_port)))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing Host header").into_response())?;
    if origin_authority.authority != request_host {
        return Err((StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response());
    }
    Ok(())
}

fn check_remote_basic_auth(headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = configured_basic_auth_header() else {
        return Ok(());
    };
    let actual = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    if actual == Some(expected) {
        return Ok(());
    }

    let mut response = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(REMOTE_BASIC_AUTH_CHALLENGE),
    );
    Err(response)
}

fn configured_basic_auth_header() -> Option<&'static str> {
    static AUTH_HEADER: OnceLock<Option<String>> = OnceLock::new();
    AUTH_HEADER
        .get_or_init(|| match std::env::var(REMOTE_BASIC_AUTH_ENV) {
            Ok(raw) => build_basic_auth_header(&raw).or_else(|| {
                warn!("{REMOTE_BASIC_AUTH_ENV} is set but must use the format username:password");
                None
            }),
            Err(_) => None,
        })
        .as_deref()
}

fn build_basic_auth_header(spec: &str) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (username, password) = trimmed.split_once(':')?;
    Some(format!(
        "Basic {}",
        BASE64.encode(format!("{username}:{password}"))
    ))
}

fn authority_from_host_header(
    value: &str,
    default_port: Option<u16>,
) -> Option<NormalizedAuthority> {
    let authority = value
        .trim()
        .rsplit('@')
        .next()
        .unwrap_or(value.trim())
        .trim();
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = authority[..=end].to_ascii_lowercase();
        let remainder = authority[end + 1..].trim();
        let port = if remainder.is_empty() {
            default_port?
        } else if let Some(port_text) = remainder.strip_prefix(':') {
            port_text.parse::<u16>().ok()?
        } else {
            return None;
        };
        return Some(NormalizedAuthority { host, port });
    }
    let (host_text, port) = match authority.rsplit_once(':') {
        Some((host_text, port_text))
            if !host_text.contains(':')
                && !port_text.is_empty()
                && port_text.chars().all(|ch| ch.is_ascii_digit()) =>
        {
            (host_text.trim(), port_text.parse::<u16>().ok()?)
        }
        _ => (authority, default_port?),
    };
    let host = host_text.trim().to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(NormalizedAuthority { host, port })
    }
}

fn origin_authority_from_url(value: &str) -> Option<OriginAuthority> {
    let scheme_end = value.find("://")?;
    let scheme = value[..scheme_end].trim().to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" | "ws" => 80,
        "https" | "wss" => 443,
        _ => return None,
    };
    let rest = &value[scheme_end + 3..];
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority_from_host_header(authority, Some(default_port))?;
    Some(OriginAuthority {
        authority,
        default_port,
    })
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
        for value in extra
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_basic_auth_header_from_username_and_password() {
        assert_eq!(
            build_basic_auth_header("admin:secret").as_deref(),
            Some("Basic YWRtaW46c2VjcmV0")
        );
    }

    #[test]
    fn rejects_malformed_basic_auth_spec() {
        assert_eq!(build_basic_auth_header("missing-delimiter"), None);
        assert_eq!(build_basic_auth_header(""), None);
    }

    #[test]
    fn parses_origin_authority_with_explicit_port() {
        assert_eq!(
            origin_authority_from_url("https://radio.local:8443/tci"),
            Some(OriginAuthority {
                authority: NormalizedAuthority {
                    host: "radio.local".to_string(),
                    port: 8443,
                },
                default_port: 443,
            })
        );
    }

    #[test]
    fn parses_origin_authority_with_default_https_port() {
        assert_eq!(
            origin_authority_from_url("https://radio.local/tci"),
            Some(OriginAuthority {
                authority: NormalizedAuthority {
                    host: "radio.local".to_string(),
                    port: 443,
                },
                default_port: 443,
            })
        );
    }

    #[test]
    fn parses_host_authority_with_ipv6_and_port() {
        assert_eq!(
            authority_from_host_header("[::1]:8443", Some(443)),
            Some(NormalizedAuthority {
                host: "[::1]".to_string(),
                port: 8443,
            })
        );
    }

    #[test]
    fn check_ws_origin_rejects_cross_origin_port_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://radio.local:3000"),
        );
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers).is_err());
    }

    #[test]
    fn check_ws_origin_accepts_same_origin_authority() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://radio.local:8443"),
        );
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers).is_ok());
    }

    #[test]
    fn check_ws_origin_requires_origin_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers).is_err());
    }
}
