use axum::{
    body::Body,
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::state::{AppState, CSRF_HEADER_NAME, CSRF_HEADER_VALUE};
use crate::util::json_error;

fn is_safe_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

fn parse_host_from_authority(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let authority = trimmed.rsplit('@').next().unwrap_or(trimmed).trim();
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        return Some(authority[..=end].to_ascii_lowercase());
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn parse_host_from_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let scheme_pos = trimmed.find("://")?;
    let rest = &trimmed[scheme_pos + 3..];
    let authority = rest.split('/').next().unwrap_or("");
    parse_host_from_authority(authority)
}

fn get_request_host(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_host_from_authority)
}

fn get_source_host(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if !origin.eq_ignore_ascii_case("null") {
            if let Some(host) = parse_host_from_url(origin) {
                return Some(host);
            }
        }
    }
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_host_from_url)
}

pub async fn csrf_protect(
    axum::extract::State(_state): axum::extract::State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if is_safe_method(req.method()) {
        return next.run(req).await;
    }

    let headers = req.headers();
    let csrf = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    if csrf != Some(CSRF_HEADER_VALUE) {
        return json_error(StatusCode::FORBIDDEN, "missing or invalid CSRF header");
    }

    let req_host = match get_request_host(headers) {
        Some(v) => v,
        None => return json_error(StatusCode::BAD_REQUEST, "missing Host header"),
    };
    match get_source_host(headers) {
        Some(source_host) => {
            if source_host != req_host {
                return json_error(StatusCode::FORBIDDEN, "Origin/Referer host mismatch");
            }
        }
        None => {
            return json_error(StatusCode::FORBIDDEN, "missing Origin or Referer header");
        }
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use tower::ServiceExt;

    use crate::state::AppState;

    fn test_state() -> AppState {
        let tmp = std::path::PathBuf::from("/tmp/saturn-test-middleware");
        AppState {
            webroot: tmp.clone(),
            config_path: tmp.join("config.json"),
            custom_scripts_file: tmp.join("custom_scripts.json"),
            remote_settings_file: tmp.join("remote_settings.json"),
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

    /// A minimal router that wraps a no-op POST handler with the CSRF middleware.
    fn csrf_router() -> axum::Router {
        async fn ok_handler() -> &'static str {
            "ok"
        }
        let state = test_state();
        axum::Router::new()
            .route("/test", post(ok_handler))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                csrf_protect,
            ))
            .with_state(state)
    }

    // --- parse_host_from_authority ---

    #[test]
    fn test_parse_authority_simple_host() {
        assert_eq!(
            parse_host_from_authority("example.com"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_parse_authority_strips_port() {
        assert_eq!(
            parse_host_from_authority("example.com:8080"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_parse_authority_strips_userinfo() {
        assert_eq!(
            parse_host_from_authority("user@example.com:8080"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_parse_authority_ipv6() {
        assert_eq!(
            parse_host_from_authority("[::1]:8080"),
            Some("[::1]".to_string())
        );
    }

    #[test]
    fn test_parse_authority_empty() {
        assert_eq!(parse_host_from_authority(""), None);
    }

    // --- parse_host_from_url ---

    #[test]
    fn test_parse_url_http() {
        assert_eq!(
            parse_host_from_url("http://example.com:8080/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_parse_url_no_scheme_returns_none() {
        assert_eq!(parse_host_from_url("example.com/path"), None);
    }

    // --- csrf_protect HTTP integration ---

    /// GET requests must bypass the CSRF check entirely (safe method).
    /// This test uses GET on a POST-only route; the middleware passes it to the
    /// router which returns 405 (Method Not Allowed) — not a CSRF rejection.
    #[tokio::test]
    async fn test_csrf_get_bypasses_check() {
        let app = csrf_router();
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        // Not 403: the CSRF middleware let it through; router returns 405.
        assert_ne!(res.status(), StatusCode::FORBIDDEN);
    }

    /// POST without the CSRF header must be rejected with 403.
    #[tokio::test]
    async fn test_csrf_post_missing_header_returns_403() {
        let app = csrf_router();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("host", "example.com")
            .header("origin", "http://example.com")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// POST with the correct CSRF header but a mismatched Origin host must be
    /// rejected with 403 (cross-origin check).
    #[tokio::test]
    async fn test_csrf_post_origin_mismatch_returns_403() {
        let app = csrf_router();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("host", "example.com")
            .header("origin", "http://evil.com")
            .header(
                crate::state::CSRF_HEADER_NAME,
                crate::state::CSRF_HEADER_VALUE,
            )
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// POST with the correct CSRF header and matching Origin host must reach the
    /// handler (200 OK).
    #[tokio::test]
    async fn test_csrf_post_valid_passes() {
        let app = csrf_router();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("host", "example.com")
            .header("origin", "http://example.com")
            .header(
                crate::state::CSRF_HEADER_NAME,
                crate::state::CSRF_HEADER_VALUE,
            )
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
