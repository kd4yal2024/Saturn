use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use std::path::Path;

use crate::state::AppState;

pub async fn root_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "overview.html").await
}

pub async fn overview_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "overview.html").await
}

pub async fn custom_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "index.html").await
}

pub async fn backup_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "backup.html").await
}

pub async fn update_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "update.html").await
}

pub async fn saturngo_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "saturngo.html").await
}

pub async fn p23test_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "p23test.html").await
}

pub async fn fpga_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "fpga.html").await
}

pub async fn pihpsdr_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "pihpsdr.html").await
}

pub async fn deskhpsdr_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "deskhpsdr.html").await
}

pub async fn monitor_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "monitor.html").await
}

pub async fn tailscale_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "tailscale.html").await
}

fn remote_host_without_port(host: &str) -> &str {
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            return &host[..=end];
        }
        return host;
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.chars().all(|ch| ch.is_ascii_digit()) => {
            name
        }
        _ => host,
    }
}

pub const REMOTE_NEXT_DEFAULT_QUERY: &str = "transport=split&tx_opus=1&tx_cfc=1";

fn remote_next_https_url(host: &str) -> String {
    format!(
        "https://{}:8443/remote-next?{}",
        remote_host_without_port(host),
        REMOTE_NEXT_DEFAULT_QUERY
    )
}

fn request_host(headers: &HeaderMap) -> &str {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
}

pub async fn remote_next_handler(headers: HeaderMap) -> impl IntoResponse {
    Redirect::temporary(&remote_next_https_url(request_host(&headers)))
}

pub fn route_to_page(path: &str) -> Option<&'static str> {
    match path {
        "/" | "/saturn" | "/saturn/" => Some("overview.html"),
        "/overview"
        | "/overview/"
        | "/overview.html"
        | "/saturn/overview"
        | "/saturn/overview/"
        | "/saturn/overview.html" => Some("overview.html"),
        "/custom"
        | "/custom/"
        | "/custom.html"
        | "/index"
        | "/index.html"
        | "/saturn/custom"
        | "/saturn/custom/"
        | "/saturn/custom.html"
        | "/saturn/index"
        | "/saturn/index.html" => Some("index.html"),
        "/backup"
        | "/backup/"
        | "/backup.html"
        | "/saturn/backup"
        | "/saturn/backup/"
        | "/saturn/backup.html" => Some("backup.html"),
        "/update"
        | "/update/"
        | "/update.html"
        | "/saturn/update"
        | "/saturn/update/"
        | "/saturn/update.html" => Some("update.html"),
        "/saturngo"
        | "/saturngo/"
        | "/saturngo.html"
        | "/saturn-go"
        | "/saturn-go/"
        | "/saturn-go.html"
        | "/saturn/saturngo"
        | "/saturn/saturngo/"
        | "/saturn/saturngo.html"
        | "/saturn/saturn-go"
        | "/saturn/saturn-go/"
        | "/saturn/saturn-go.html" => Some("saturngo.html"),
        "/telemetry"
        | "/telemetry/"
        | "/telemetry.html"
        | "/saturn/telemetry"
        | "/saturn/telemetry/"
        | "/saturn/telemetry.html"
        | "/p23test"
        | "/p23test/"
        | "/p23test.html"
        | "/saturn/p23test"
        | "/saturn/p23test/"
        | "/saturn/p23test.html" => Some("p23test.html"),
        "/fpga" | "/fpga/" | "/fpga.html" | "/saturn/fpga" | "/saturn/fpga/"
        | "/saturn/fpga.html" => Some("fpga.html"),
        "/pihpsdr"
        | "/pihpsdr/"
        | "/pihpsdr.html"
        | "/saturn/pihpsdr"
        | "/saturn/pihpsdr/"
        | "/saturn/pihpsdr.html" => Some("pihpsdr.html"),
        "/deskhpsdr"
        | "/deskhpsdr/"
        | "/deskhpsdr.html"
        | "/saturn/deskhpsdr"
        | "/saturn/deskhpsdr/"
        | "/saturn/deskhpsdr.html" => Some("deskhpsdr.html"),
        "/remote"
        | "/remote/"
        | "/remote.html"
        | "/saturn-remote"
        | "/saturn-remote/"
        | "/saturn-remote.html"
        | "/saturn/remote"
        | "/saturn/remote/"
        | "/saturn/remote.html"
        | "/saturn/saturn-remote"
        | "/saturn/saturn-remote/"
        | "/saturn/saturn-remote.html"
        | "/remote-next"
        | "/remote-next/"
        | "/remote-next.html"
        | "/saturn/remote-next"
        | "/saturn/remote-next/"
        | "/saturn/remote-next.html" => Some("saturn-remote-next.html"),
        "/monitor"
        | "/monitor/"
        | "/monitor.html"
        | "/saturn/monitor"
        | "/saturn/monitor/"
        | "/saturn/monitor.html" => Some("monitor.html"),
        "/tailscale"
        | "/tailscale/"
        | "/tailscale.html"
        | "/saturn/tailscale"
        | "/saturn/tailscale/"
        | "/saturn/tailscale.html" => Some("tailscale.html"),
        _ => None,
    }
}

pub async fn fallback_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> impl IntoResponse {
    let host = request_host(&headers);
    let path = uri.path();
    if path.len() > 1 && path != "/saturn/" && path.ends_with('/') {
        let canonical_path = path.trim_end_matches('/');
        let canonical = match uri.query() {
            Some(query) => format!("{canonical_path}?{query}"),
            None => canonical_path.to_string(),
        };
        return Redirect::permanent(&canonical).into_response();
    }
    if let Some(page) = route_to_page(uri.path()) {
        if page == "saturn-remote-next.html" {
            return Redirect::temporary(&remote_next_https_url(host)).into_response();
        }
        return serve_page(&state.webroot, page).await;
    }
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

pub async fn serve_page(webroot: &Path, page: &str) -> Response {
    let page_path = webroot.join(page);
    match tokio::fs::read_to_string(&page_path).await {
        Ok(body) => (
            [(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")],
            Html(body),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "page not found").into_response(),
    }
}

/// Rejects any path with a `..`, empty, or otherwise non-literal component so
/// `/assets/{*path}` can never escape `webroot/assets`.
pub fn is_safe_asset_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn asset_content_type(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

pub async fn asset_handler(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    if !is_safe_asset_path(&path) {
        return (StatusCode::NOT_FOUND, "page not found").into_response();
    }
    let asset_path = state.webroot.join("assets").join(&path);
    match tokio::fs::read(&asset_path).await {
        Ok(bytes) => {
            let content_type = asset_content_type(&path);
            // Asset filenames are stable rather than content-hashed, so every
            // class of asset must revalidate after an appliance update.
            let cache_control = "no-cache, no-store, must-revalidate";
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (header::CACHE_CONTROL, cache_control),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "page not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    use crate::state::AppState;

    fn test_state() -> AppState {
        let tmp = std::path::PathBuf::from("/tmp/saturn-test-pages");
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

    // --- route_to_page ---

    #[test]
    fn test_root_routes_to_overview() {
        assert_eq!(route_to_page("/"), Some("overview.html"));
        assert_eq!(route_to_page("/saturn"), Some("overview.html"));
        assert_eq!(route_to_page("/overview"), Some("overview.html"));
        assert_eq!(
            route_to_page("/saturn/overview.html"),
            Some("overview.html")
        );
        assert_eq!(route_to_page("/update"), Some("update.html"));
        assert_eq!(route_to_page("/update.html"), Some("update.html"));
        assert_eq!(route_to_page("/telemetry"), Some("p23test.html"));
        assert_eq!(route_to_page("/saturn/telemetry"), Some("p23test.html"));
        assert_eq!(route_to_page("/p23test"), Some("p23test.html"));
    }

    #[test]
    fn test_custom_routes_to_index() {
        assert_eq!(route_to_page("/custom"), Some("index.html"));
        assert_eq!(route_to_page("/index.html"), Some("index.html"));
        assert_eq!(route_to_page("/saturn/custom"), Some("index.html"));
    }

    #[test]
    fn test_saturngo_aliases() {
        assert_eq!(route_to_page("/saturngo"), Some("saturngo.html"));
        assert_eq!(route_to_page("/saturn-go"), Some("saturngo.html"));
        assert_eq!(
            route_to_page("/saturn/saturngo.html"),
            Some("saturngo.html")
        );
    }

    #[test]
    fn test_remote_aliases() {
        assert_eq!(route_to_page("/remote"), Some("saturn-remote-next.html"));
        assert_eq!(
            route_to_page("/saturn-remote"),
            Some("saturn-remote-next.html")
        );
        assert_eq!(
            route_to_page("/saturn/remote.html"),
            Some("saturn-remote-next.html")
        );
        assert_eq!(
            route_to_page("/remote-next"),
            Some("saturn-remote-next.html")
        );
        assert_eq!(
            route_to_page("/saturn/remote-next.html"),
            Some("saturn-remote-next.html")
        );
    }

    #[test]
    fn test_unknown_path_returns_none() {
        assert_eq!(route_to_page("/unknown"), None);
        assert_eq!(route_to_page("/api/something"), None);
        assert_eq!(route_to_page(""), None);
    }

    // --- fallback_handler ---

    /// A request for an unknown path must return 404.
    #[tokio::test]
    async fn test_fallback_unknown_path_returns_404() {
        let state = test_state();
        let app = axum::Router::new()
            .fallback(get(fallback_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/no-such-page")
            .header("host", "127.0.0.1:8080")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_fallback_redirects_trailing_slash_to_canonical_page() {
        let state = test_state();
        let app = axum::Router::new()
            .fallback(get(fallback_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/saturn/monitor/?view=cpu")
            .header("host", "127.0.0.1:8080")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/saturn/monitor?view=cpu"
        );
    }

    /// A request for a known route alias whose HTML file does not exist on disk
    /// must return 404 (serve_page falls back when the file is missing).
    #[tokio::test]
    async fn test_fallback_known_route_missing_file_returns_404() {
        // /tmp/saturn-test-pages/update.html does not exist, so serving fails.
        let state = test_state();
        let app = axum::Router::new()
            .fallback(get(fallback_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/update")
            .header("host", "127.0.0.1:8080")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_remote_handler_redirects_to_tls_remote_next() {
        let state = test_state();
        let app = axum::Router::new()
            .route("/remote", get(remote_next_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/remote")
            .header("host", "192.168.0.139")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            &format!("https://192.168.0.139:8443/remote-next?{REMOTE_NEXT_DEFAULT_QUERY}")
        );
    }

    #[tokio::test]
    async fn test_fallback_remote_alias_redirects_to_tls_remote_next() {
        let state = test_state();
        let app = axum::Router::new()
            .fallback(get(fallback_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/saturn/remote")
            .header("host", "192.168.0.139:8080")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            &format!("https://192.168.0.139:8443/remote-next?{REMOTE_NEXT_DEFAULT_QUERY}")
        );
    }

    #[tokio::test]
    async fn test_fallback_remote_next_alias_redirects_to_tls_remote_next() {
        let state = test_state();
        let app = axum::Router::new()
            .fallback(get(fallback_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/saturn/remote-next")
            .header("host", "192.168.0.139:8080")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "https://192.168.0.139:8443/remote-next?transport=split&tx_opus=1&tx_cfc=1"
        );
    }

    // --- asset_handler / is_safe_asset_path ---

    #[test]
    fn test_safe_asset_path_rejects_traversal() {
        assert!(!is_safe_asset_path("../../etc/passwd"));
        assert!(!is_safe_asset_path("css/../../../etc/passwd"));
        assert!(!is_safe_asset_path(""));
        assert!(!is_safe_asset_path("/etc/passwd"));
    }

    #[test]
    fn test_safe_asset_path_accepts_normal_paths() {
        assert!(is_safe_asset_path("css/saturn-ui.css"));
        assert!(is_safe_asset_path("js/saturn-shell.js"));
        assert!(is_safe_asset_path("vendor/tailwind.js"));
    }

    #[tokio::test]
    async fn test_asset_handler_serves_existing_file_with_content_type() {
        let state = test_state();
        let assets_dir = state.webroot.join("assets/css");
        tokio::fs::create_dir_all(&assets_dir).await.unwrap();
        tokio::fs::write(assets_dir.join("saturn-ui.css"), b"body{color:red}")
            .await
            .unwrap();

        let app = axum::Router::new()
            .route("/assets/{*path}", get(asset_handler))
            .with_state(state.clone());
        let req = Request::builder()
            .method("GET")
            .uri("/assets/css/saturn-ui.css")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache, no-store, must-revalidate"
        );

        tokio::fs::remove_dir_all(state.webroot.join("assets"))
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_asset_handler_rejects_traversal_with_404() {
        let state = test_state();
        let app = axum::Router::new()
            .route("/assets/{*path}", get(asset_handler))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/assets/..%2f..%2fetc%2fpasswd")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
