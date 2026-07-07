use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        ConnectInfo, OriginalUri, Path as AxumPath, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use sha2::Sha256;
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use tracing::{error, info, warn};

use crate::{
    delete_remote_profile, get_remote_profiles, get_remote_settings,
    middleware::csrf_protect,
    pages::{healthz, serve_page, REMOTE_NEXT_DEFAULT_QUERY},
    save_remote_profile, set_remote_profile_startup, set_remote_settings,
    state::{
        AppState, RemoteProfileDeleteRequest, RemoteProfileSaveRequest,
        RemoteProfileStartupRequest, RemoteSettings,
    },
};

type RemoteTlsResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_REMOTE_TLS_ADDR: &str = "0.0.0.0:8443";
const REMOTE_BASIC_AUTH_ENV: &str = "SATURN_REMOTE_BASIC_AUTH";
const REMOTE_DEV_INSECURE_ENV: &str = "SATURN_REMOTE_DEV_INSECURE";
const REMOTE_BASIC_AUTH_CHALLENGE: &str = "Basic realm=\"Saturn Remote\", charset=\"UTF-8\"";
const REMOTE_AUTH_COOKIE_NAME: &str = "saturn_remote_auth";
// "Remember this device": one password entry per browser per password. The
// cookie token is HMAC(persisted secret, current credential), so it survives
// restarts but every remembered device is signed out by a password change.
const REMOTE_AUTH_COOKIE_MAX_AGE_SECS: u64 = 365 * 24 * 60 * 60;
const REMOTE_AUTH_COOKIE_SECRET_FILE: &str = "remote-tls/cookie.secret";
const DEFAULT_STATE_DIR: &str = "/var/lib/saturn-state";
// Failed-credential tarpit: wrong Authorization guesses from one IP earn a
// growing delay. Two free failures keep it invisible to a fat-fingered human.
const TARPIT_FREE_FAILURES: u32 = 2;
const TARPIT_MAX_DELAY: Duration = Duration::from_secs(10);
const TARPIT_FORGET_AFTER: Duration = Duration::from_secs(15 * 60);
const TARPIT_MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BridgeProxyChannel {
    Legacy,
    Control,
    Media,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BridgeProxyMessageKind {
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

impl BridgeProxyChannel {
    fn label(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Control => "control",
            Self::Media => "media",
        }
    }
}

fn bridge_proxy_allows_message(channel: BridgeProxyChannel, kind: BridgeProxyMessageKind) -> bool {
    match (channel, kind) {
        (BridgeProxyChannel::Control, BridgeProxyMessageKind::Binary) => false,
        (BridgeProxyChannel::Media, BridgeProxyMessageKind::Text) => false,
        _ => true,
    }
}

/// Decision for whether the Saturn Remote TLS listener should bind at startup.
///
/// Pure function of `(auth_configured, dev_insecure_override)` so the policy is
/// trivially testable without touching process env. The caller in `main.rs`
/// reads the env once and passes the booleans in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTlsBindDecision {
    /// Saturn Remote auth is configured; bind the TLS listener normally.
    Bind,
    /// Auth is missing/malformed but `SATURN_REMOTE_DEV_INSECURE=1` is set.
    /// Bind the listener with no auth gate (development only).
    BindInsecure,
    /// Auth is missing and no override; refuse to bind. Admin HTTP is unaffected.
    Refuse,
}

pub fn remote_tls_bind_decision(
    auth_configured: bool,
    dev_insecure_override: bool,
) -> RemoteTlsBindDecision {
    match (auth_configured, dev_insecure_override) {
        (true, _) => RemoteTlsBindDecision::Bind,
        (false, true) => RemoteTlsBindDecision::BindInsecure,
        (false, false) => RemoteTlsBindDecision::Refuse,
    }
}

/// Reads `SATURN_REMOTE_DEV_INSECURE` from process env and returns `true` only
/// when the value is exactly `1` (with surrounding whitespace tolerated). Any
/// other value — empty, `0`, `true`, `false` — returns `false`.
pub fn dev_insecure_override_set() -> bool {
    match std::env::var(REMOTE_DEV_INSECURE_ENV) {
        Ok(raw) => raw.trim() == "1",
        Err(_) => false,
    }
}

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
        .route("/remote-next", get(remote_next_page_handler))
        .route("/remote-next.html", get(remote_next_page_handler))
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
        .route("/saturn/control", get(remote_bridge_control_ws_handler))
        .route("/saturn/media", get(remote_bridge_media_ws_handler))
        .route("/remote-assets/{asset}", get(remote_asset_handler))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(state, csrf_protect))
        .layer(axum::middleware::from_fn(remote_auth_tarpit))
}

async fn remote_page_handler(headers: HeaderMap, State(state): State<AppState>) -> Response {
    remote_page_response(headers, state, "saturn-remote.html").await
}

async fn remote_next_page_handler(
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    State(state): State<AppState>,
) -> Response {
    if uri.query().is_none_or(str::is_empty) {
        if let Err(rejection) = check_remote_auth(&headers) {
            return rejection;
        }
        let mut resp = Redirect::temporary(&format!("/remote-next?{REMOTE_NEXT_DEFAULT_QUERY}"))
            .into_response();
        attach_remote_auth_cookie(&mut resp);
        return resp;
    }
    remote_page_response(headers, state, "saturn-remote-next.html").await
}

async fn remote_page_response(headers: HeaderMap, state: AppState, page: &str) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    let mut resp = serve_page(&state.webroot, page).await;
    attach_remote_auth_cookie(&mut resp);
    // Enable SharedArrayBuffer for AudioWorklet ring-buffer audio pipeline.
    let hdrs = resp.headers_mut();
    hdrs.insert(
        header::HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    hdrs.insert(
        header::HeaderName::from_static("cross-origin-embedder-policy"),
        HeaderValue::from_static("credentialless"),
    );
    resp
}

async fn remote_asset_handler(
    headers: HeaderMap,
    AxumPath(asset): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }

    let filename = match asset.as_str() {
        "storage.js" => "saturn-remote-storage.js",
        "session.js" => "saturn-remote-session.js",
        "tci.js" => "saturn-remote-tci.js",
        "transport.js" => "saturn-remote-transport.js",
        "browser.js" => "saturn-remote-browser.js",
        "remote-next.js" => "saturn-remote-next.js",
        _ => return (StatusCode::NOT_FOUND, "asset not found").into_response(),
    };

    match tokio::fs::read(state.webroot.join(filename)).await {
        Ok(body) => (
            [
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                (
                    header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8",
                ),
            ],
            body,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

pub async fn remote_bridge_ws_handler(
    headers: HeaderMap,
    uri: axum::http::Uri,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    remote_bridge_ws_response(headers, uri, ws, state, BridgeProxyChannel::Legacy)
}

pub async fn remote_bridge_control_ws_handler(
    headers: HeaderMap,
    uri: axum::http::Uri,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    remote_bridge_ws_response(headers, uri, ws, state, BridgeProxyChannel::Control)
}

pub async fn remote_bridge_media_ws_handler(
    headers: HeaderMap,
    uri: axum::http::Uri,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    remote_bridge_ws_response(headers, uri, ws, state, BridgeProxyChannel::Media)
}

fn remote_bridge_ws_response(
    headers: HeaderMap,
    uri: axum::http::Uri,
    ws: WebSocketUpgrade,
    state: AppState,
    channel: BridgeProxyChannel,
) -> Response {
    if let Err(rejection) = check_ws_origin(&headers, &uri) {
        warn!(
            "remote TLS {} websocket rejected by origin check: host={:?} origin={:?} uri={uri}",
            channel.label(),
            header_value_for_log(&headers, header::HOST),
            header_value_for_log(&headers, header::ORIGIN),
        );
        return rejection;
    }
    let Some(source) = remote_auth_source(&headers) else {
        warn!(
            "remote TLS {} websocket rejected by auth check: host={:?} origin={:?} uri={uri}",
            channel.label(),
            header_value_for_log(&headers, header::HOST),
            header_value_for_log(&headers, header::ORIGIN),
        );
        return check_remote_basic_auth(&headers).err().unwrap_or_else(|| {
            (StatusCode::UNAUTHORIZED, "authentication required").into_response()
        });
    };
    info!(
        "remote TLS {} websocket accepted via {source} auth: host={:?} origin={:?} uri={uri}",
        channel.label(),
        header_value_for_log(&headers, header::HOST),
        header_value_for_log(&headers, header::ORIGIN),
    );
    let phase42_session_id = extract_phase42_session_from_uri(&uri);
    ws.on_upgrade(move |socket| {
        proxy_bridge_socket(socket, state.bridge_ws_url, channel, phase42_session_id)
    })
}

async fn remote_settings_get_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    get_remote_settings(State(state)).await
}

async fn remote_settings_post_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(settings): Json<RemoteSettings>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    set_remote_settings(State(state), Json(settings)).await
}

async fn remote_profiles_get_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    get_remote_profiles(State(state)).await
}

async fn remote_profiles_save_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileSaveRequest>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    save_remote_profile(State(state), Json(request)).await
}

async fn remote_profiles_delete_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileDeleteRequest>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    delete_remote_profile(State(state), Json(request)).await
}

async fn remote_profiles_startup_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileStartupRequest>,
) -> Response {
    if let Err(rejection) = check_remote_auth(&headers) {
        return rejection;
    }
    set_remote_profile_startup(State(state), Json(request)).await
}

/// Reject WebSocket upgrades whose Origin authority differs from the request
/// Host authority. This is the browser-facing proxy path, so require an Origin
/// header and treat host+port mismatches as cross-origin.
fn check_ws_origin(headers: &HeaderMap, uri: &axum::http::Uri) -> Result<(), Response> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Err((StatusCode::FORBIDDEN, "missing Origin header").into_response());
    };
    if origin.eq_ignore_ascii_case("null") {
        // Opaque origin (sandboxed iframe, data: URL, etc.) — reject.
        return Err((StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response());
    }
    let origin_authority = origin_authority_from_url(origin)
        .ok_or_else(|| (StatusCode::FORBIDDEN, "invalid Origin header").into_response())?;
    // Try Host header first, fall back to URI authority for HTTP/2 which
    // uses the :authority pseudo-header instead of Host.
    let request_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| authority_from_host_header(v, Some(origin_authority.default_port)))
        .or_else(|| {
            uri.authority().and_then(|a| {
                authority_from_host_header(a.as_str(), Some(origin_authority.default_port))
            })
        })
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
    if actual.is_some_and(|a| ct_eq(a.as_bytes(), expected.as_bytes())) {
        return Ok(());
    }

    let mut response = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(REMOTE_BASIC_AUTH_CHALLENGE),
    );
    Err(response)
}

fn check_remote_auth(headers: &HeaderMap) -> Result<(), Response> {
    if remote_auth_source(headers).is_some() {
        return Ok(());
    }
    check_remote_basic_auth(headers)
}

fn remote_auth_source(headers: &HeaderMap) -> Option<&'static str> {
    let Some(expected) = configured_basic_auth_header() else {
        return Some("disabled");
    };
    let actual = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    if actual.is_some_and(|a| ct_eq(a.as_bytes(), expected.as_bytes())) {
        return Some("basic");
    }
    if remote_auth_cookie_matches(headers) {
        return Some("cookie");
    }
    None
}

fn header_value_for_log(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(160).collect())
}

fn attach_remote_auth_cookie(response: &mut Response) {
    if configured_basic_auth_header().is_none() {
        return;
    }
    let value = format!(
        "{REMOTE_AUTH_COOKIE_NAME}={}; Path=/; Max-Age={REMOTE_AUTH_COOKIE_MAX_AGE_SECS}; \
         Secure; HttpOnly; SameSite=Strict",
        remote_auth_cookie_value()
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
}

fn remote_auth_cookie_matches(headers: &HeaderMap) -> bool {
    if configured_basic_auth_header().is_none() {
        return false;
    }
    let expected = remote_auth_cookie_value();
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|header| {
            header.split(';').any(|part| {
                let (name, value) = part.trim().split_once('=').unwrap_or(("", ""));
                name == REMOTE_AUTH_COOKIE_NAME && ct_eq(value.as_bytes(), expected.as_bytes())
            })
        })
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

fn remote_auth_cookie_value() -> &'static str {
    static COOKIE_VALUE: OnceLock<String> = OnceLock::new();
    COOKIE_VALUE
        .get_or_init(|| {
            if let (Some(secret), Some(credential)) =
                (persisted_cookie_secret(), configured_basic_auth_header())
            {
                return derive_cookie_token(&secret, credential);
            }
            warn!(
                "remote auth cookie falling back to a per-process token; \
                 remembered devices will not survive a saturn-go restart"
            );
            per_process_cookie_token()
        })
        .as_str()
}

/// Deterministic remember-device token: stable across restarts for the same
/// (secret, credential) pair, different as soon as the password changes.
fn derive_cookie_token(secret: &[u8], credential: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).expect("HMAC-SHA256 accepts keys of any length");
    mac.update(b"saturn-remote-auth-cookie-v1:");
    mac.update(credential.as_bytes());
    BASE64.encode(mac.finalize().into_bytes())
}

/// Random 32-byte secret persisted under the state dir (0600). Created on
/// first use; returns None if it can neither be read nor created.
fn persisted_cookie_secret() -> Option<[u8; 32]> {
    let state_dir =
        std::env::var("SATURN_STATE_DIR").unwrap_or_else(|_| DEFAULT_STATE_DIR.to_string());
    let path = Path::new(&state_dir).join(REMOTE_AUTH_COOKIE_SECRET_FILE);

    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() >= 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes[..32]);
            return Some(secret);
        }
        warn!(
            "cookie secret {} is malformed; regenerating",
            path.display()
        );
    }

    let mut secret = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut secret))
        .ok()?;
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            warn!("cannot create {}: {err}", parent.display());
            return None;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        let write_result = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(&secret));
        if let Err(err) = write_result {
            warn!("cannot persist cookie secret {}: {err}", path.display());
            return None;
        }
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        if let Err(err) = std::fs::write(&path, secret) {
            warn!("cannot persist cookie secret {}: {err}", path.display());
            return None;
        }
    }
    info!("generated remote auth cookie secret at {}", path.display());
    Some(secret)
}

fn per_process_cookie_token() -> String {
    let mut bytes = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let fallback = format!(
            "{}:{}:{}",
            std::process::id(),
            now,
            configured_basic_auth_header().unwrap_or("")
        );
        return BASE64.encode(fallback.as_bytes());
    }
    BASE64.encode(bytes)
}

/// Constant-time equality for credential material.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(left ^ right);
    }
    diff == 0
}

struct AuthFailureState {
    consecutive: u32,
    last: Instant,
    blocked_until: Instant,
}

fn auth_failure_registry() -> &'static Mutex<HashMap<IpAddr, AuthFailureState>> {
    static REGISTRY: OnceLock<Mutex<HashMap<IpAddr, AuthFailureState>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn tarpit_delay(consecutive: u32) -> Duration {
    if consecutive <= TARPIT_FREE_FAILURES {
        return Duration::ZERO;
    }
    let exponent = (consecutive - TARPIT_FREE_FAILURES - 1).min(6);
    Duration::from_secs(1u64 << exponent).min(TARPIT_MAX_DELAY)
}

/// Slow repeated wrong-credential guesses per source IP. Only requests that
/// carried an Authorization header and got a 401 count as failures — a bare
/// first visit answered with the auth challenge stays instant. Requests
/// without ConnectInfo (unit tests) bypass the tarpit.
async fn remote_auth_tarpit(req: Request, next: Next) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let guessed = req.headers().contains_key(header::AUTHORIZATION);
    let authenticated = guessed && remote_auth_source(req.headers()).is_some();

    if let (Some(ip), true, false) = (ip, guessed, authenticated) {
        let wait = {
            let registry = auth_failure_registry().lock().unwrap();
            registry
                .get(&ip)
                .map(|s| s.blocked_until.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::ZERO)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    let response = next.run(req).await;

    if let (Some(ip), true) = (ip, guessed) {
        if response.status() == StatusCode::UNAUTHORIZED {
            let delay = {
                let mut registry = auth_failure_registry().lock().unwrap();
                if registry.len() >= TARPIT_MAX_ENTRIES {
                    registry.retain(|_, s| s.last.elapsed() < TARPIT_FORGET_AFTER);
                    if registry.len() >= TARPIT_MAX_ENTRIES {
                        registry.clear();
                    }
                }
                let now = Instant::now();
                let entry = registry.entry(ip).or_insert(AuthFailureState {
                    consecutive: 0,
                    last: now,
                    blocked_until: now,
                });
                if entry.last.elapsed() >= TARPIT_FORGET_AFTER {
                    entry.consecutive = 0;
                }
                entry.consecutive = entry.consecutive.saturating_add(1);
                entry.last = now;
                let delay = tarpit_delay(entry.consecutive);
                entry.blocked_until = now + delay;
                if entry.consecutive == TARPIT_FREE_FAILURES + 1 {
                    warn!("repeated basic-auth failures from {ip}; tarpitting responses");
                }
                delay
            };
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
        } else if authenticated {
            auth_failure_registry().lock().unwrap().remove(&ip);
        }
    }

    response
}

pub fn remote_basic_auth_configured() -> bool {
    configured_basic_auth_header().is_some()
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

fn extract_phase42_session_from_uri(uri: &axum::http::Uri) -> Option<String> {
    let query = uri.query()?;
    for part in query.split('&') {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        if name == "session" {
            let value = decode_query_component(value)?;
            return sanitize_phase42_session_id(&value);
        }
    }
    None
}

fn decode_query_component(value: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                decoded.push(b' ');
                i += 1;
            }
            b'%' => {
                let hi = bytes.get(i + 1).copied().and_then(hex_value)?;
                let lo = bytes.get(i + 2).copied().and_then(hex_value)?;
                decoded.push((hi << 4) | lo);
                i += 3;
            }
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn sanitize_phase42_session_id(value: &str) -> Option<String> {
    let session_id: String = value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .take(64)
        .collect();
    if session_id.is_empty() {
        None
    } else {
        Some(session_id)
    }
}

fn phase42_proxy_lane_message(
    channel: BridgeProxyChannel,
    session_id: Option<&str>,
) -> Option<String> {
    let session_id = sanitize_phase42_session_id(session_id?)?;
    match channel {
        BridgeProxyChannel::Legacy => None,
        BridgeProxyChannel::Control => Some(format!("session_lane:{session_id},control;")),
        BridgeProxyChannel::Media => Some(format!("session_lane:{session_id},media;")),
    }
}

async fn proxy_bridge_socket(
    client: WebSocket,
    bridge_ws_url: String,
    channel: BridgeProxyChannel,
    phase42_session_id: Option<String>,
) {
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

    if let Some(message) = phase42_proxy_lane_message(channel, phase42_session_id.as_deref()) {
        if let Err(err) = bridge_tx
            .send(TungsteniteMessage::Text(message.into()))
            .await
        {
            warn!("remote TLS bridge proxy failed to send Phase 42 lane marker: {err}");
            let _ = client_tx.send(AxumMessage::Close(None)).await;
            return;
        }
    }

    let client_to_bridge = async {
        while let Some(message) = client_rx.next().await {
            let message = message.map_err(|err| format!("client websocket error: {err}"))?;
            match message {
                AxumMessage::Text(text) => {
                    if !bridge_proxy_allows_message(channel, BridgeProxyMessageKind::Text) {
                        warn!(
                            "remote TLS {} websocket proxy dropped client text frame",
                            channel.label()
                        );
                        continue;
                    }
                    bridge_tx
                        .send(TungsteniteMessage::Text(text.to_string().into()))
                        .await
                        .map_err(|err| format!("bridge websocket send failed: {err}"))?;
                }
                AxumMessage::Binary(bytes) => {
                    if !bridge_proxy_allows_message(channel, BridgeProxyMessageKind::Binary) {
                        warn!(
                            "remote TLS {} websocket proxy dropped client binary frame",
                            channel.label()
                        );
                        continue;
                    }
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
                    if !bridge_proxy_allows_message(channel, BridgeProxyMessageKind::Text) {
                        warn!(
                            "remote TLS {} websocket proxy dropped bridge text frame",
                            channel.label()
                        );
                        continue;
                    }
                    client_tx
                        .send(AxumMessage::Text(text.to_string().into()))
                        .await
                        .map_err(|err| format!("client websocket send failed: {err}"))?;
                }
                TungsteniteMessage::Binary(bytes) => {
                    if !bridge_proxy_allows_message(channel, BridgeProxyMessageKind::Binary) {
                        warn!(
                            "remote TLS {} websocket proxy dropped bridge binary frame",
                            channel.label()
                        );
                        continue;
                    }
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
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn test_state(name: &str) -> AppState {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("saturn-test-remote-tls-{name}-{pid}-{nanos}"));
        AppState {
            webroot: tmp.clone(),
            config_path: tmp.join("config.json"),
            custom_scripts_file: tmp.join("custom_scripts.json"),
            remote_settings_file: tmp.join("remote_settings.json"),
            remote_profiles_file: tmp.join("remote_profiles.json"),
            scripts_dir: tmp.join("scripts"),
            saturn_addr: "127.0.0.1:8080".to_string(),
            bridge_ws_url: "ws://127.0.0.1:50001".to_string(),
            repo_root: std::sync::Arc::new(std::sync::RwLock::new(tmp.clone())),
            repo_root_file: tmp.join("repo_root"),
            update_policy_file: tmp.join("update_policy.json"),
            saturngo_update_policy_file: tmp.join("saturngo_policy.json"),
            saturngo_deploy_status_file: tmp.join("saturngo_deploy.json"),
            update_state_file: tmp.join("update_state.json"),
            snapshot_dir: tmp.join("snapshots"),
            staging_dir: tmp.join("staging"),
            restore_max_upload_bytes: 2 * 1024 * 1024 * 1024,
        }
    }

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
    fn cookie_token_is_stable_for_same_secret_and_credential() {
        let secret = [7u8; 32];
        assert_eq!(
            derive_cookie_token(&secret, "Basic YWRtaW46b2xk"),
            derive_cookie_token(&secret, "Basic YWRtaW46b2xk")
        );
    }

    #[test]
    fn cookie_token_changes_with_credential_or_secret() {
        let secret = [7u8; 32];
        let token = derive_cookie_token(&secret, "Basic YWRtaW46b2xk");
        assert_ne!(token, derive_cookie_token(&secret, "Basic YWRtaW46bmV3"));
        assert_ne!(token, derive_cookie_token(&[8u8; 32], "Basic YWRtaW46b2xk"));
    }

    #[test]
    fn tarpit_delay_is_free_then_doubles_then_caps() {
        assert_eq!(tarpit_delay(1), Duration::ZERO);
        assert_eq!(tarpit_delay(2), Duration::ZERO);
        assert_eq!(tarpit_delay(3), Duration::from_secs(1));
        assert_eq!(tarpit_delay(4), Duration::from_secs(2));
        assert_eq!(tarpit_delay(5), Duration::from_secs(4));
        assert_eq!(tarpit_delay(6), Duration::from_secs(8));
        assert_eq!(tarpit_delay(7), TARPIT_MAX_DELAY);
        assert_eq!(tarpit_delay(u32::MAX), TARPIT_MAX_DELAY);
    }

    #[test]
    fn ct_eq_matches_only_identical_bytes() {
        assert!(ct_eq(b"same", b"same"));
        assert!(!ct_eq(b"same", b"sane"));
        assert!(!ct_eq(b"short", b"longer"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn remote_tls_bind_decision_binds_when_auth_configured() {
        assert_eq!(
            remote_tls_bind_decision(true, false),
            RemoteTlsBindDecision::Bind
        );
        assert_eq!(
            remote_tls_bind_decision(true, true),
            RemoteTlsBindDecision::Bind
        );
    }

    #[test]
    fn remote_tls_bind_decision_refuses_when_auth_missing_and_no_override() {
        assert_eq!(
            remote_tls_bind_decision(false, false),
            RemoteTlsBindDecision::Refuse
        );
    }

    #[test]
    fn remote_tls_bind_decision_binds_insecure_when_override_set_and_no_auth() {
        assert_eq!(
            remote_tls_bind_decision(false, true),
            RemoteTlsBindDecision::BindInsecure
        );
    }

    #[test]
    fn phase42_proxy_channels_enforce_control_media_separation() {
        assert!(bridge_proxy_allows_message(
            BridgeProxyChannel::Legacy,
            BridgeProxyMessageKind::Text
        ));
        assert!(bridge_proxy_allows_message(
            BridgeProxyChannel::Legacy,
            BridgeProxyMessageKind::Binary
        ));
        assert!(bridge_proxy_allows_message(
            BridgeProxyChannel::Control,
            BridgeProxyMessageKind::Text
        ));
        assert!(!bridge_proxy_allows_message(
            BridgeProxyChannel::Control,
            BridgeProxyMessageKind::Binary
        ));
        assert!(bridge_proxy_allows_message(
            BridgeProxyChannel::Media,
            BridgeProxyMessageKind::Binary
        ));
        assert!(!bridge_proxy_allows_message(
            BridgeProxyChannel::Media,
            BridgeProxyMessageKind::Text
        ));
    }

    #[test]
    fn phase42_proxy_channels_keep_websocket_control_frames_allowed() {
        for channel in [
            BridgeProxyChannel::Legacy,
            BridgeProxyChannel::Control,
            BridgeProxyChannel::Media,
        ] {
            assert!(bridge_proxy_allows_message(
                channel,
                BridgeProxyMessageKind::Ping
            ));
            assert!(bridge_proxy_allows_message(
                channel,
                BridgeProxyMessageKind::Pong
            ));
            assert!(bridge_proxy_allows_message(
                channel,
                BridgeProxyMessageKind::Close
            ));
        }
    }

    #[test]
    fn phase42_proxy_extracts_sanitized_session_from_uri() {
        let uri: axum::http::Uri = "/saturn/control?x=1&session=phase.42_1-operator"
            .parse()
            .unwrap();
        assert_eq!(
            extract_phase42_session_from_uri(&uri),
            Some("phase.42_1-operator".to_string())
        );

        let encoded_uri: axum::http::Uri =
            "/saturn/media?session=phase%3A42+operator".parse().unwrap();
        assert_eq!(
            extract_phase42_session_from_uri(&encoded_uri),
            Some("phase42operator".to_string())
        );

        let invalid_uri: axum::http::Uri = "/saturn/media?session=%zz".parse().unwrap();
        assert_eq!(extract_phase42_session_from_uri(&invalid_uri), None);
    }

    #[test]
    fn phase42_proxy_lane_messages_mark_split_channels() {
        assert_eq!(
            phase42_proxy_lane_message(BridgeProxyChannel::Legacy, Some("phase-42")),
            None
        );
        assert_eq!(
            phase42_proxy_lane_message(BridgeProxyChannel::Control, Some("phase-42")),
            Some("session_lane:phase-42,control;".to_string())
        );
        assert_eq!(
            phase42_proxy_lane_message(BridgeProxyChannel::Media, Some("phase:42")),
            Some("session_lane:phase42,media;".to_string())
        );
        assert_eq!(
            phase42_proxy_lane_message(BridgeProxyChannel::Media, Some("   ")),
            None
        );
    }

    #[test]
    fn dev_insecure_override_recognizes_only_one() {
        // Save+restore env so the test does not bleed into other cases.
        // SATURN_REMOTE_DEV_INSECURE is intentionally not in OnceLock — each
        // call reads the current env directly.
        let prev = std::env::var(REMOTE_DEV_INSECURE_ENV).ok();

        std::env::remove_var(REMOTE_DEV_INSECURE_ENV);
        assert!(!dev_insecure_override_set(), "unset should be false");

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "");
        assert!(!dev_insecure_override_set(), "empty should be false");

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "0");
        assert!(!dev_insecure_override_set(), "0 should be false");

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "true");
        assert!(!dev_insecure_override_set(), "'true' should be false");

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "1");
        assert!(dev_insecure_override_set(), "1 should be true");

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "  1  ");
        assert!(
            dev_insecure_override_set(),
            "whitespace-padded 1 should be true"
        );

        std::env::set_var(REMOTE_DEV_INSECURE_ENV, "11");
        assert!(!dev_insecure_override_set(), "11 should be false");

        match prev {
            Some(v) => std::env::set_var(REMOTE_DEV_INSECURE_ENV, v),
            None => std::env::remove_var(REMOTE_DEV_INSECURE_ENV),
        }
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

    fn empty_uri() -> axum::http::Uri {
        "/tci".parse().unwrap()
    }

    #[test]
    fn check_ws_origin_rejects_cross_origin_port_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://radio.local:3000"),
        );
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers, &empty_uri()).is_err());
    }

    #[test]
    fn check_ws_origin_accepts_same_origin_authority() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://radio.local:8443"),
        );
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers, &empty_uri()).is_ok());
    }

    #[test]
    fn check_ws_origin_requires_origin_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("radio.local:8443"));
        assert!(check_ws_origin(&headers, &empty_uri()).is_err());
    }

    #[test]
    fn check_ws_origin_accepts_uri_authority_fallback() {
        // HTTP/2: no Host header, authority comes from URI
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://radio.local:8443"),
        );
        let uri: axum::http::Uri = "https://radio.local:8443/tci".parse().unwrap();
        assert!(check_ws_origin(&headers, &uri).is_ok());
    }

    #[tokio::test]
    async fn remote_asset_route_serves_known_asset() {
        let state = test_state("asset-ok");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-storage.js"),
            "window.testStorage = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/storage.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testStorage = true;"[..]);
    }

    #[tokio::test]
    async fn remote_page_route_serves_stable_remote_page() {
        let state = test_state("remote-page");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(state.webroot.join("saturn-remote.html"), "stable remote")
            .await
            .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::HeaderName::from_static(
                    "cross-origin-opener-policy"
                ))
                .unwrap(),
            "same-origin"
        );
        assert_eq!(
            res.headers()
                .get(header::HeaderName::from_static(
                    "cross-origin-embedder-policy"
                ))
                .unwrap(),
            "credentialless"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"stable remote"[..]);
    }

    #[tokio::test]
    async fn remote_next_route_redirects_to_default_query_when_missing() {
        let state = test_state("remote-next-default-redirect");
        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-next")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3"
        );
    }

    #[tokio::test]
    async fn remote_next_route_serves_dev_remote_page_when_query_present() {
        let state = test_state("remote-next-page");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(state.webroot.join("saturn-remote.html"), "stable remote")
            .await
            .unwrap();
        tokio::fs::write(state.webroot.join("saturn-remote-next.html"), "dev remote")
            .await
            .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::HeaderName::from_static(
                    "cross-origin-opener-policy"
                ))
                .unwrap(),
            "same-origin"
        );
        assert_eq!(
            res.headers()
                .get(header::HeaderName::from_static(
                    "cross-origin-embedder-policy"
                ))
                .unwrap(),
            "credentialless"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"dev remote"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_serves_session_asset() {
        let state = test_state("asset-session");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-session.js"),
            "window.testSession = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/session.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testSession = true;"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_serves_tci_asset() {
        let state = test_state("asset-tci");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-tci.js"),
            "window.testTci = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/tci.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testTci = true;"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_serves_transport_asset() {
        let state = test_state("asset-transport");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-transport.js"),
            "window.testTransport = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/transport.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testTransport = true;"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_serves_browser_asset() {
        let state = test_state("asset-browser");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-browser.js"),
            "window.testBrowser = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/browser.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testBrowser = true;"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_serves_remote_next_bundle() {
        let state = test_state("asset-remote-next-bundle");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();
        tokio::fs::write(
            state.webroot.join("saturn-remote-next.js"),
            "window.testRemoteNext = true;",
        )
        .await
        .unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/remote-next.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body, &b"window.testRemoteNext = true;"[..]);
    }

    #[tokio::test]
    async fn remote_asset_route_rejects_unknown_asset() {
        let state = test_state("asset-missing");
        tokio::fs::create_dir_all(&state.webroot).await.unwrap();

        let app = remote_tls_router(state);
        let req = Request::builder()
            .uri("/remote-assets/nope.js")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
