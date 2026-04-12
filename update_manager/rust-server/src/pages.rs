use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use std::path::Path;

use crate::state::AppState;

pub async fn root_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "update.html").await
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

pub async fn remote_handler(State(state): State<AppState>) -> impl IntoResponse {
    serve_page(&state.webroot, "saturn-remote.html").await
}

pub async fn healthz() -> impl IntoResponse {
    StatusCode::OK
}

pub fn route_to_page(path: &str) -> Option<&'static str> {
    match path {
        "/" | "/saturn" | "/saturn/" => Some("update.html"),
        "/custom" | "/custom/" | "/custom.html" | "/index" | "/index.html" | "/saturn/custom"
        | "/saturn/custom/" | "/saturn/custom.html" | "/saturn/index"
        | "/saturn/index.html" => Some("index.html"),
        "/backup" | "/backup/" | "/backup.html" | "/saturn/backup" | "/saturn/backup/"
        | "/saturn/backup.html" => Some("backup.html"),
        "/update" | "/update/" | "/update.html" | "/saturn/update" | "/saturn/update/"
        | "/saturn/update.html" => Some("update.html"),
        "/saturngo" | "/saturngo/" | "/saturngo.html" | "/saturn-go" | "/saturn-go/"
        | "/saturn-go.html" | "/saturn/saturngo" | "/saturn/saturngo/"
        | "/saturn/saturngo.html" | "/saturn/saturn-go" | "/saturn/saturn-go/"
        | "/saturn/saturn-go.html" => Some("saturngo.html"),
        "/p23test" | "/p23test/" | "/p23test.html" | "/saturn/p23test" | "/saturn/p23test/"
        | "/saturn/p23test.html" => Some("p23test.html"),
        "/fpga" | "/fpga/" | "/fpga.html" | "/saturn/fpga" | "/saturn/fpga/"
        | "/saturn/fpga.html" => Some("fpga.html"),
        "/pihpsdr" | "/pihpsdr/" | "/pihpsdr.html" | "/saturn/pihpsdr" | "/saturn/pihpsdr/"
        | "/saturn/pihpsdr.html" => Some("pihpsdr.html"),
        "/deskhpsdr" | "/deskhpsdr/" | "/deskhpsdr.html" | "/saturn/deskhpsdr"
        | "/saturn/deskhpsdr/" | "/saturn/deskhpsdr.html" => Some("deskhpsdr.html"),
        "/remote" | "/remote/" | "/remote.html" | "/saturn-remote" | "/saturn-remote/"
        | "/saturn-remote.html" | "/saturn/remote" | "/saturn/remote/"
        | "/saturn/remote.html" | "/saturn/saturn-remote" | "/saturn/saturn-remote/"
        | "/saturn/saturn-remote.html" => Some("saturn-remote.html"),
        "/monitor" | "/monitor/" | "/monitor.html" | "/saturn/monitor" | "/saturn/monitor/"
        | "/saturn/monitor.html" => Some("monitor.html"),
        _ => None,
    }
}

pub async fn fallback_handler(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> impl IntoResponse {
    if let Some(page) = route_to_page(uri.path()) {
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
    fn test_root_routes_to_update() {
        assert_eq!(route_to_page("/"), Some("update.html"));
        assert_eq!(route_to_page("/saturn"), Some("update.html"));
        assert_eq!(route_to_page("/update"), Some("update.html"));
        assert_eq!(route_to_page("/update.html"), Some("update.html"));
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
        assert_eq!(route_to_page("/saturn/saturngo.html"), Some("saturngo.html"));
    }

    #[test]
    fn test_remote_aliases() {
        assert_eq!(route_to_page("/remote"), Some("saturn-remote.html"));
        assert_eq!(route_to_page("/saturn-remote"), Some("saturn-remote.html"));
        assert_eq!(route_to_page("/saturn/remote.html"), Some("saturn-remote.html"));
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
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
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
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// healthz must return 200 regardless of filesystem state.
    #[tokio::test]
    async fn test_healthz_returns_200() {
        let state = test_state();
        let app = axum::Router::new()
            .route("/healthz", get(healthz))
            .with_state(state);
        let req = Request::builder()
            .method("GET")
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
