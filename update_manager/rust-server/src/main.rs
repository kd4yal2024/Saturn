mod auth;
mod clone;
mod image;
mod middleware;
mod monitor;
mod pages;
mod remote_tls;
mod repair;
mod state;
mod sync_ext;
mod tailscale;
mod update;
mod util;
use crate::auth::{change_password, exit_server, kill_process};
use crate::clone::{pi_clone_cancel, pi_clone_start, pi_clone_status, pi_devices, pi_wipe_target};
use crate::image::{pi_image_cancel, pi_image_download, pi_image_start, pi_image_status};
use crate::middleware::csrf_protect;
use crate::monitor::{get_system_data, network_test};
use crate::pages::{
    backup_handler, custom_handler, deskhpsdr_handler, fallback_handler, fpga_handler, healthz,
    monitor_handler, p23test_handler, pihpsdr_handler, remote_handler, remote_next_handler,
    root_handler, saturngo_handler, tailscale_handler, update_handler,
};
use crate::remote_tls::{
    dev_insecure_override_set, ensure_self_signed_cert, load_remote_tls_config,
    remote_basic_auth_configured, remote_bridge_ws_handler, remote_tls_bind_decision,
    remote_tls_router, RemoteTlsBindDecision,
};
use crate::repair::{repair_pack, verify_system_config};
use crate::state::{
    AppState, CfgEntry, DefaultCustomScript, FlagsQuery, RemoteProfileDeleteRequest,
    RemoteProfileSaveRequest, RemoteProfileStartupRequest, RemoteProfilesFile, RemoteSettings,
    RunLogQuery, DEFAULT_CUSTOM_SCRIPT_CLEAN_BACKUPS, DEFAULT_CUSTOM_SCRIPT_CLEAN_LOGS,
    DEFAULT_CUSTOM_SCRIPT_FIX_LED_POWER_BUTTON, DEFAULT_CUSTOM_SCRIPT_SETUP_ETH_FALLBACK,
    DEFAULT_MAX_BODY_BYTES, DEFAULT_RESTORE_MAX_UPLOAD_BYTES, MAX_CUSTOM_SCRIPTS,
    MAX_CUSTOM_SCRIPTS_FILE_BYTES, MAX_REMOTE_PROFILES_FILE_BYTES, MAX_REMOTE_SETTINGS_FILE_BYTES,
    MAX_TAR_EXPANSION_FACTOR, P23_ADC_PEAK_TELEMETRY_ENABLE_FILE, P23_ADC_PEAK_TELEMETRY_JSON_FILE,
    P23_APP_PERF_TELEMETRY_JSON_FILE, RUN_LOG_FETCH_MAX_LINES, RUN_LOG_MAX_LINES,
};
use crate::sync_ext::MutexExt;
use crate::tailscale::{
    tailscale_down, tailscale_install, tailscale_logout, tailscale_serve, tailscale_up,
};
use crate::update::{
    begin_update_activity, expected_remote_url, get_repo_root, get_update_policy, list_repo_roots,
    load_update_policy, normalize_update_policy, set_repo_root, set_update_policy,
    update_policy_repo_configured, update_rollback, update_start, update_status, UpdatePolicy,
};
use crate::util::{
    backup_home_dir, current_repo_root, is_safe_backup_name_with_prefix,
    is_safe_custom_script_filename, is_safe_repo_part, is_safe_script_name, is_saturn_repo_root,
    json_error, output_error_text, parse_boolish, pihpsdr_repo_root, sanitize_custom_flags,
    validate_pihpsdr_repo_root, validate_saturn_repo_root,
};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Json, Response, Sse,
    },
    routing::{get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle as AxumServerHandle;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
struct P23AdcTelemetryRequest {
    enabled: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let addr = std::env::var("SATURN_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    if let Err(message) = validate_saturn_bind_addr(&addr) {
        error!("{message}");
        std::process::exit(1);
    }
    let webroot =
        std::env::var("SATURN_WEBROOT").unwrap_or_else(|_| "/var/lib/saturn-web".to_string());
    let config_path = std::env::var("SATURN_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{webroot}/config.json")));
    let scripts_dir = std::env::var("SATURN_SCRIPTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/opt/saturn-go/scripts"));
    let default_state_dir =
        std::env::var("SATURN_STATE_DIR").unwrap_or_else(|_| "/var/lib/saturn-state".to_string());
    let custom_scripts_file = std::env::var("SATURN_CUSTOM_SCRIPTS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/custom_scripts.json")));
    let remote_settings_file = std::env::var("SATURN_REMOTE_SETTINGS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/remote_settings.json")));
    let remote_profiles_file = std::env::var("SATURN_REMOTE_PROFILES_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/remote_profiles.json")));
    let default_repo_root = std::env::var("SATURN_REPO_ROOT").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/pi".to_string());
        format!("{home}/github/Saturn")
    });
    let repo_root_file = std::env::var("SATURN_REPO_ROOT_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/repo_root.txt")));
    let update_policy_file = std::env::var("SATURN_UPDATE_POLICY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/update_policy.json")));
    let saturngo_update_policy_file = std::env::var("SATURN_SATURNGO_UPDATE_POLICY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!("{default_state_dir}/saturngo_update_policy.json"))
        });
    let saturngo_deploy_status_file = std::env::var("SATURN_SATURNGO_DEPLOY_STATUS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!("{default_state_dir}/saturngo_deploy_status.json"))
        });
    let update_state_file = std::env::var("SATURN_UPDATE_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/update_state.json")));
    let snapshot_dir = std::env::var("SATURN_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/snapshots")));
    let staging_dir = std::env::var("SATURN_STAGING_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{default_state_dir}/repo-staging")));
    let restore_max_upload_bytes = std::env::var("SATURN_RESTORE_MAX_UPLOAD_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_RESTORE_MAX_UPLOAD_BYTES);
    let max_body_bytes = std::env::var("SATURN_MAX_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_BODY_BYTES)
        .min(usize::MAX as u64) as usize;
    let bridge_ws_url = std::env::var("SATURN_REMOTE_BRIDGE_WS")
        .unwrap_or_else(|_| "ws://127.0.0.1:50001".to_string());
    let remote_tls_config = load_remote_tls_config(&default_state_dir)
        .expect("invalid SATURN_REMOTE_TLS_ADDR/SATURN_REMOTE_TLS_CERT/SATURN_REMOTE_TLS_KEY");

    // Canonicalize both paths at startup to resolve symlinks before validation,
    // so a symlink planted in SATURN_REPO_ROOT or repo_root.txt cannot bypass
    // the is_saturn_repo_root() check.
    let mut repo_root = tokio::fs::canonicalize(&default_repo_root)
        .await
        .unwrap_or_else(|_| PathBuf::from(&default_repo_root));
    if let Ok(saved) = tokio::fs::read_to_string(&repo_root_file).await {
        let saved = saved.trim();
        if !saved.is_empty() {
            if let Ok(canonical) = tokio::fs::canonicalize(saved).await {
                if is_saturn_repo_root(&canonical) {
                    repo_root = canonical;
                }
            }
        }
    }
    if let Some(parent) = repo_root_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = update_policy_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = saturngo_update_policy_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = saturngo_deploy_status_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = update_state_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = custom_scripts_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = remote_settings_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Some(parent) = remote_profiles_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::create_dir_all(&scripts_dir).await;
    let _ = tokio::fs::create_dir_all(&snapshot_dir).await;
    let _ = tokio::fs::create_dir_all(&staging_dir).await;
    let _ = tokio::fs::write(&repo_root_file, format!("{}\n", repo_root.display())).await;

    let state = AppState {
        webroot: PathBuf::from(webroot),
        config_path,
        custom_scripts_file,
        remote_settings_file,
        remote_profiles_file,
        scripts_dir,
        saturn_addr: addr.clone(),
        bridge_ws_url: bridge_ws_url.clone(),
        repo_root: Arc::new(RwLock::new(repo_root)),
        repo_root_file,
        update_policy_file,
        saturngo_update_policy_file,
        saturngo_deploy_status_file,
        update_state_file,
        snapshot_dir,
        staging_dir,
        restore_max_upload_bytes,
    };

    if let Err(e) = ensure_default_custom_scripts(&state).await {
        error!("failed to initialize default custom scripts: {e}");
    }

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/custom", get(custom_handler))
        .route("/custom.html", get(custom_handler))
        .route("/index", get(custom_handler))
        .route("/index.html", get(custom_handler))
        .route("/backup", get(backup_handler))
        .route("/backup.html", get(backup_handler))
        .route("/update", get(update_handler))
        .route("/update.html", get(update_handler))
        .route("/saturngo", get(saturngo_handler))
        .route("/saturngo.html", get(saturngo_handler))
        .route("/saturn-go", get(saturngo_handler))
        .route("/saturn-go.html", get(saturngo_handler))
        .route("/p23test", get(p23test_handler))
        .route("/p23test.html", get(p23test_handler))
        .route("/fpga", get(fpga_handler))
        .route("/fpga.html", get(fpga_handler))
        .route("/pihpsdr", get(pihpsdr_handler))
        .route("/pihpsdr.html", get(pihpsdr_handler))
        .route("/deskhpsdr", get(deskhpsdr_handler))
        .route("/deskhpsdr.html", get(deskhpsdr_handler))
        .route("/remote", get(remote_handler))
        .route("/remote.html", get(remote_handler))
        .route("/remote-next", get(remote_next_handler))
        .route("/remote-next.html", get(remote_next_handler))
        .route("/saturn-remote", get(remote_handler))
        .route("/saturn-remote.html", get(remote_handler))
        .route("/tci", get(remote_bridge_ws_handler))
        .route("/monitor", get(monitor_handler))
        .route("/monitor.html", get(monitor_handler))
        .route("/healthz", get(healthz))
        .route("/get_versions", get(get_versions))
        .route("/get_scripts", get(get_scripts))
        .route("/get_flags", get(get_flags))
        .route("/custom_scripts", get(get_custom_scripts))
        .route("/custom_scripts", post(upsert_custom_script))
        .route("/custom_scripts_delete", post(delete_custom_script))
        .route("/remote_settings", get(get_remote_settings))
        .route("/remote_settings", post(set_remote_settings))
        .route("/remote_profiles", get(get_remote_profiles))
        .route("/remote_profiles/save", post(save_remote_profile))
        .route("/remote_profiles/delete", post(delete_remote_profile))
        .route("/remote_profiles/startup", post(set_remote_profile_startup))
        .route("/get_fpga_images", get(get_fpga_images))
        .route("/get_repo_root", get(get_repo_root))
        .route("/list_repo_roots", get(list_repo_roots))
        .route("/set_repo_root", post(set_repo_root))
        .route("/update_policy", get(get_update_policy))
        .route("/update_policy", post(set_update_policy))
        .route("/saturngo_policy", get(get_saturngo_policy))
        .route("/saturngo_policy", post(set_saturngo_policy))
        .route("/saturngo_deploy_status", get(get_saturngo_deploy_status))
        .route("/tailscale", get(tailscale_handler))
        .route("/tailscale.html", get(tailscale_handler))
        .route("/tailscale_status", get(get_tailscale_status))
        .route("/tailscale/install", post(tailscale_install))
        .route("/tailscale/up", post(tailscale_up))
        .route("/tailscale/down", post(tailscale_down))
        .route("/tailscale/logout", post(tailscale_logout))
        .route("/tailscale/serve", post(tailscale_serve))
        .route("/bridge_diag", get(get_bridge_diag))
        .route("/saturn/bridge_diag", get(get_bridge_diag))
        .route("/p23_status", get(get_p23_status))
        .route("/p23_perf", get(get_p23_perf))
        .route("/p23_adc_telemetry", post(set_p23_adc_telemetry))
        .route("/update_start", post(update_start))
        .route("/update_status", get(update_status))
        .route("/update_rollback", post(update_rollback))
        .route("/backup_full", get(backup_full))
        .route("/restore_full", post(restore_full))
        .route("/g2_backups", get(g2_backups))
        .route("/g2_restore", post(g2_restore))
        .route("/pihpsdr_backups", get(pihpsdr_backups))
        .route("/pihpsdr_restore", post(pihpsdr_restore))
        .route("/pi_image_start", post(pi_image_start))
        .route("/pi_image_status", get(pi_image_status))
        .route("/pi_image_cancel", post(pi_image_cancel))
        .route("/pi_image_download", get(pi_image_download))
        .route("/pi_devices", get(pi_devices))
        .route("/pi_wipe_target", post(pi_wipe_target))
        .route("/pi_clone_start", post(pi_clone_start))
        .route("/pi_clone_status", get(pi_clone_status))
        .route("/pi_clone_cancel", post(pi_clone_cancel))
        .route("/repair_pack", get(repair_pack))
        .route("/verify_system_config", get(verify_system_config))
        .route("/run", post(run_sse))
        .route("/run_log", get(get_run_log))
        .route("/backup_response", post(no_content))
        .route("/change_password", post(change_password))
        .route("/exit", post(exit_server))
        .route("/get_system_data", get(get_system_data))
        .route("/network_test", get(network_test))
        .route("/kill_process/{pid}", post(kill_process))
        .fallback(get(fallback_handler))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_protect,
        ))
        .layer(DefaultBodyLimit::max(max_body_bytes));

    let remote_tls_router = if let Some(remote_tls_addr) = remote_tls_config.addr {
        match ensure_self_signed_cert(&remote_tls_config.cert_path, &remote_tls_config.key_path)
            .await
        {
            Err(err) => {
                warn!("Saturn Remote TLS disabled (cert setup failed): {err}");
                None
            }
            Ok(()) => {
                match RustlsConfig::from_pem_file(
                    remote_tls_config.cert_path.clone(),
                    remote_tls_config.key_path.clone(),
                )
                .await
                {
                    Err(err) => {
                        warn!("Saturn Remote TLS disabled (rustls config failed): {err}");
                        None
                    }
                    Ok(rustls_config) => Some((
                        remote_tls_addr,
                        rustls_config,
                        remote_tls_router(state.clone()),
                    )),
                }
            }
        }
    } else {
        None
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    });

    info!("Saturn server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind failed");
    let http_server = serve_http(listener, app, shutdown_rx.clone());

    if let Some((remote_tls_addr, rustls_config, remote_tls_app)) = remote_tls_router {
        let auth_configured = remote_basic_auth_configured();
        let dev_insecure = dev_insecure_override_set();
        match remote_tls_bind_decision(auth_configured, dev_insecure) {
            RemoteTlsBindDecision::Refuse => {
                error!(
                    "Saturn Remote TLS listener refusing to start: SATURN_REMOTE_BASIC_AUTH is unset or malformed."
                );
                error!(
                    "Set Environment=SATURN_REMOTE_BASIC_AUTH=username:password in saturn-go.service (e.g. via `systemctl edit saturn-go.service`) and restart, or set SATURN_REMOTE_DEV_INSECURE=1 to override (NOT FOR PRODUCTION)."
                );
                error!("Saturn Go admin (port 8080) continues to start normally.");
            }
            RemoteTlsBindDecision::BindInsecure => {
                warn!(
                    "SATURN_REMOTE_DEV_INSECURE=1 — Saturn Remote TLS is starting WITHOUT basic auth on https://{remote_tls_addr}. Do not use in production."
                );
                info!("Saturn Remote TLS listening on https://{remote_tls_addr}");
                let tls_shutdown = shutdown_rx.clone();
                tokio::spawn(async move {
                    if let Err(err) = serve_remote_tls(
                        remote_tls_addr,
                        rustls_config,
                        remote_tls_app,
                        tls_shutdown,
                    )
                    .await
                    {
                        error!("Saturn Remote TLS server error: {err}");
                    }
                });
            }
            RemoteTlsBindDecision::Bind => {
                info!("Saturn Remote TLS listening on https://{remote_tls_addr}");
                let tls_shutdown = shutdown_rx.clone();
                tokio::spawn(async move {
                    if let Err(err) = serve_remote_tls(
                        remote_tls_addr,
                        rustls_config,
                        remote_tls_app,
                        tls_shutdown,
                    )
                    .await
                    {
                        error!("Saturn Remote TLS server error: {err}");
                    }
                });
            }
        }
    }
    http_server.await.expect("server failed");

    let _ = signal_task.await;
    info!("Saturn server shut down");
}

fn validate_saturn_bind_addr(addr: &str) -> Result<(), String> {
    if parse_boolish(std::env::var("SATURN_ALLOW_NON_LOOPBACK_ADDR").ok()) {
        return Ok(());
    }
    if bind_addr_is_loopback(addr) {
        return Ok(());
    }
    Err(format!(
        "refusing non-loopback SATURN_ADDR={addr}; set SATURN_ALLOW_NON_LOOPBACK_ADDR=1 only if the backend has its own auth boundary"
    ))
}

fn bind_addr_is_loopback(addr: &str) -> bool {
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return socket_addr.ip().is_loopback();
    }

    let Some((host, _port)) = addr.rsplit_once(':') else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => { info!("received SIGINT, shutting down"); }
        _ = terminate => { info!("received SIGTERM, shutting down"); }
    }
}

async fn serve_http(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown_rx: watch::Receiver<bool>,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
        .await
}

async fn serve_remote_tls(
    addr: SocketAddr,
    rustls_config: RustlsConfig,
    app: Router,
    shutdown_rx: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let handle = AxumServerHandle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        wait_for_shutdown(shutdown_rx).await;
        shutdown_handle.graceful_shutdown(None);
    });

    axum_server::bind_rustls(addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn wait_for_shutdown(mut shutdown_rx: watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }
    let _ = shutdown_rx.changed().await;
}

fn stdbuf_binary() -> Option<&'static str> {
    static STDBUF_BIN: OnceLock<Option<&'static str>> = OnceLock::new();
    *STDBUF_BIN.get_or_init(|| {
        if Path::new("/usr/bin/stdbuf").exists() {
            Some("/usr/bin/stdbuf")
        } else if Path::new("/bin/stdbuf").exists() {
            Some("/bin/stdbuf")
        } else {
            None
        }
    })
}

fn build_script_command(script_path: &Path, flags: &[String]) -> Command {
    let is_python = script_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("py"))
        .unwrap_or(false);

    let mut cmd = if is_python {
        let mut cmd = if let Some(stdbuf) = stdbuf_binary() {
            let mut c = Command::new(stdbuf);
            c.arg("-oL")
                .arg("-eL")
                .arg("python3")
                .arg("-u")
                .arg(script_path)
                .args(flags);
            c
        } else {
            let mut c = Command::new("python3");
            c.arg("-u").arg(script_path).args(flags);
            c
        };
        cmd.env("PYTHONUNBUFFERED", "1");
        cmd.env("PYTHONIOENCODING", "UTF-8");
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("PYTHONPYCACHEPREFIX", "/var/cache/saturn-python");
        cmd
    } else if let Some(stdbuf) = stdbuf_binary() {
        let mut cmd = Command::new(stdbuf);
        cmd.arg("-oL").arg("-eL").arg(script_path).args(flags);
        cmd
    } else {
        let mut cmd = Command::new(script_path);
        cmd.args(flags);
        cmd
    };

    apply_build_subprocess_env(&mut cmd);
    cmd
}

// Saturn-go runs under systemd with a clipped environment. Without an
// explicit PATH and PKG_CONFIG_PATH, spawned build scripts can fail to
// find system binaries (cmake, pkg-config) and .pc files that are on
// disk and work from a normal interactive shell. Set both to the
// standard Debian/aarch64 defaults for every script we run.
fn apply_build_subprocess_env(cmd: &mut Command) {
    cmd.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    cmd.env(
        "PKG_CONFIG_PATH",
        "/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig",
    );
}

type RunLineSink = Arc<dyn Fn(String) + Send + Sync>;

fn emit_process_line(
    tx: &mpsc::UnboundedSender<String>,
    line_sink: Option<&RunLineSink>,
    line: String,
) {
    let _ = tx.send(line.clone());
    if let Some(sink) = line_sink {
        sink(line);
    }
}

async fn stream_process_output<R>(
    mut reader: R,
    tx: mpsc::UnboundedSender<String>,
    prefix: &'static str,
    line_sink: Option<RunLineSink>,
) where
    R: AsyncRead + Unpin,
{
    let mut buf = [0u8; 2048];
    let mut pending = String::new();

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => {
                if !pending.is_empty() {
                    let line = std::mem::take(&mut pending);
                    emit_process_line(&tx, line_sink.as_ref(), format!("{prefix}{line}"));
                }
                break;
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                let mut ended_with_delim = false;
                for ch in chunk.chars() {
                    if ch == '\n' || ch == '\r' {
                        ended_with_delim = true;
                        if !pending.is_empty() {
                            let line = std::mem::take(&mut pending);
                            emit_process_line(&tx, line_sink.as_ref(), format!("{prefix}{line}"));
                        }
                    } else {
                        ended_with_delim = false;
                        pending.push(ch);
                    }
                }
                if !ended_with_delim && !pending.is_empty() {
                    // Flush partial chunks too, so long-running commands update in near real-time.
                    let line = std::mem::take(&mut pending);
                    emit_process_line(&tx, line_sink.as_ref(), format!("{prefix}{line}"));
                }
            }
            Err(e) => {
                if !pending.is_empty() {
                    let line = std::mem::take(&mut pending);
                    emit_process_line(&tx, line_sink.as_ref(), format!("{prefix}{line}"));
                }
                emit_process_line(
                    &tx,
                    line_sink.as_ref(),
                    format!("{prefix}stream read error: {e}"),
                );
                break;
            }
        }
    }
}

async fn read_config(state: &AppState) -> Result<Vec<CfgEntry>, String> {
    let data = tokio::fs::read_to_string(&state.config_path)
        .await
        .map_err(|e| e.to_string())?;
    let entries: Vec<CfgEntry> = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    Ok(entries)
}

fn default_custom_scripts(state: &AppState) -> Vec<DefaultCustomScript> {
    let scripts_dir = state.scripts_dir.display().to_string();
    vec![
        DefaultCustomScript {
            entry: CfgEntry {
                filename: "cleanup-saturn-logs.sh".to_string(),
                name: Some("Cleanup Saturn Logs".to_string()),
                description: Some("Delete Saturn update logs (keep newer logs by default)".to_string()),
                directory: Some(scripts_dir.clone()),
                category: Some("Custom Scripts".to_string()),
                flags: Some(vec![
                    "--all".to_string(),
                    "--older-7".to_string(),
                    "--dry-run".to_string(),
                    "--verbose".to_string(),
                ]),
                version: Some("custom-default".to_string()),
            },
            content: DEFAULT_CUSTOM_SCRIPT_CLEAN_LOGS,
        },
        DefaultCustomScript {
            entry: CfgEntry {
                filename: "cleanup-saturn-backups.sh".to_string(),
                name: Some("Cleanup Saturn Backups".to_string()),
                description: Some("Prune Saturn/piHPSDR backup directories (keeps 2 newest by default)".to_string()),
                directory: Some(scripts_dir.clone()),
                category: Some("Custom Scripts".to_string()),
                flags: Some(vec![
                    "--saturn-only".to_string(),
                    "--pihpsdr-only".to_string(),
                    "--delete-all".to_string(),
                    "--dry-run".to_string(),
                    "--verbose".to_string(),
                ]),
                version: Some("custom-default".to_string()),
            },
            content: DEFAULT_CUSTOM_SCRIPT_CLEAN_BACKUPS,
        },
        DefaultCustomScript {
            entry: CfgEntry {
                filename: "fix-LED-power-button.sh".to_string(),
                name: Some("Fix LED Power Button".to_string()),
                description: Some(
                    "Install the BCM15 front-panel LED boot/shutdown handler and optional early boot default."
                        .to_string(),
                ),
                directory: Some(scripts_dir.clone()),
                category: Some("Custom Scripts".to_string()),
                flags: Some(vec![]),
                version: Some("custom-default".to_string()),
            },
            content: DEFAULT_CUSTOM_SCRIPT_FIX_LED_POWER_BUTTON,
        },
        DefaultCustomScript {
            entry: CfgEntry {
                filename: "setup-eth-fallback.sh".to_string(),
                name: Some("Setup Ethernet Fallback".to_string()),
                description: Some(
                    "Configure NetworkManager DHCP-to-APIPA fallback for direct Ethernet links."
                        .to_string(),
                ),
                directory: Some(scripts_dir),
                category: Some("Custom Scripts".to_string()),
                flags: Some(vec![]),
                version: Some("custom-default".to_string()),
            },
            content: DEFAULT_CUSTOM_SCRIPT_SETUP_ETH_FALLBACK,
        },
    ]
}

async fn ensure_default_custom_scripts(state: &AppState) -> Result<(), String> {
    let defaults = default_custom_scripts(state);
    for default in &defaults {
        let path = state.scripts_dir.join(&default.entry.filename);
        match tokio::fs::metadata(&path).await {
            Ok(meta) => {
                if !meta.is_file() {
                    return Err(format!(
                        "default script path is not a file: {}",
                        path.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::write(&path, default.content)
                    .await
                    .map_err(|err| {
                        format!("failed to write default script {}: {err}", path.display())
                    })?;
                tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .await
                    .map_err(|err| {
                        format!("failed to chmod default script {}: {err}", path.display())
                    })?;
            }
            Err(e) => {
                return Err(format!(
                    "failed to stat default script {}: {e}",
                    path.display()
                ))
            }
        }
    }

    let mut entries = load_custom_scripts(state).await?;
    let mut changed = false;
    for default in defaults {
        if entries.iter().all(|e| e.filename != default.entry.filename) {
            entries.push(default.entry);
            changed = true;
        }
    }
    if changed {
        save_custom_scripts(state, &entries).await?;
    }
    Ok(())
}

async fn load_custom_scripts(state: &AppState) -> Result<Vec<CfgEntry>, String> {
    // Reject oversized files before deserializing to bound memory cost.
    match tokio::fs::metadata(&state.custom_scripts_file).await {
        Ok(meta) if meta.len() > MAX_CUSTOM_SCRIPTS_FILE_BYTES => {
            return Err(format!(
                "custom_scripts.json exceeds size limit ({} bytes)",
                meta.len()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        _ => {}
    }
    let data = match tokio::fs::read_to_string(&state.custom_scripts_file).await {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("failed to read custom scripts: {e}")),
    };
    let mut entries = serde_json::from_str::<Vec<CfgEntry>>(&data)
        .map_err(|e| format!("invalid custom scripts json: {e}"))?;
    entries.truncate(MAX_CUSTOM_SCRIPTS);
    // Normalize flags on every loaded entry so oversized values already on disk
    // are clamped before they reach any handler.
    for entry in &mut entries {
        let sanitized = sanitize_custom_flags(entry.flags.take());
        entry.flags = if sanitized.is_empty() {
            None
        } else {
            Some(sanitized)
        };
    }
    Ok(entries)
}

async fn save_custom_scripts(state: &AppState, entries: &[CfgEntry]) -> Result<(), String> {
    if let Some(parent) = state.custom_scripts_file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create custom scripts dir: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(entries)
        .map_err(|e| format!("failed to serialize custom scripts: {e}"))?;
    tokio::fs::write(&state.custom_scripts_file, bytes)
        .await
        .map_err(|e| format!("failed to write custom scripts: {e}"))?;
    let _ = tokio::fs::set_permissions(
        &state.custom_scripts_file,
        std::fs::Permissions::from_mode(0o640),
    )
    .await;
    Ok(())
}

async fn read_all_script_entries(state: &AppState) -> Result<Vec<CfgEntry>, String> {
    let mut merged = Vec::new();
    if let Ok(mut builtins) = read_config(state).await {
        merged.append(&mut builtins);
    }
    let mut custom = load_custom_scripts(state).await?;
    merged.append(&mut custom);
    Ok(merged)
}

#[derive(Debug, Clone)]
struct ScriptRunLog {
    run_id: String,
    status: String, // running|done|error
    started_at: String,
    finished_at: Option<String>,
    line_offset: usize,
    lines: Vec<String>,
}

static SCRIPT_RUN_LOGS: OnceLock<Mutex<BTreeMap<String, ScriptRunLog>>> = OnceLock::new();

fn script_run_log_slot() -> &'static Mutex<BTreeMap<String, ScriptRunLog>> {
    SCRIPT_RUN_LOGS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn begin_script_run_log(script: &str, flags: &[String]) -> (String, String) {
    let run_id = format!(
        "run-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let start_line = format!("Running {} {}", script, flags.join(" "));
    let entry = ScriptRunLog {
        run_id: run_id.clone(),
        status: "running".to_string(),
        started_at: Local::now().to_rfc3339(),
        finished_at: None,
        line_offset: 0,
        lines: vec![start_line.clone()],
    };
    script_run_log_slot()
        .lock_unpoisoned()
        .insert(script.to_string(), entry);
    (run_id, start_line)
}

fn append_script_run_log_line(script: &str, run_id: &str, line: String) {
    let mut guard = script_run_log_slot().lock_unpoisoned();
    let Some(run) = guard.get_mut(script) else {
        return;
    };
    if run.run_id != run_id {
        return;
    }
    run.lines.push(line);
    if run.lines.len() > RUN_LOG_MAX_LINES {
        let excess = run.lines.len() - RUN_LOG_MAX_LINES;
        run.lines.drain(0..excess);
        run.line_offset += excess;
    }
}

fn finish_script_run_log(script: &str, run_id: &str, status: &str) {
    let mut guard = script_run_log_slot().lock_unpoisoned();
    let Some(run) = guard.get_mut(script) else {
        return;
    };
    if run.run_id != run_id {
        return;
    }
    run.status = status.to_string();
    run.finished_at = Some(Local::now().to_rfc3339());
}

fn is_g2_update_script(script: &str) -> bool {
    script.eq_ignore_ascii_case("update-G2.py") || script.eq_ignore_ascii_case("update-G2.sh")
}

fn is_saturngo_update_script(script: &str) -> bool {
    script.eq_ignore_ascii_case("update-saturn-go.sh")
}

fn has_flag(flags: &[String], wanted: &str) -> bool {
    flags.iter().any(|flag| flag == wanted)
}

fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
    let trimmed = remote_url.trim();
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("http://github.com/") {
        path
    } else {
        return None;
    };

    let mut parts = path.trim_end_matches(".git").splitn(3, '/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if parts.next().is_some() || !is_safe_repo_part(owner) || !is_safe_repo_part(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

async fn infer_saturngo_policy_repo_from_active_remote(
    state: &AppState,
) -> Option<(String, String)> {
    let repo_root = current_repo_root(state);
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8_lossy(&output.stdout);
    parse_github_owner_repo(&remote)
}

async fn normalize_saturngo_update_policy(policy: UpdatePolicy, state: &AppState) -> UpdatePolicy {
    let mut normalized = normalize_update_policy(policy, state);
    if !update_policy_repo_configured(&normalized) {
        if let Some((owner, repo)) = infer_saturngo_policy_repo_from_active_remote(state).await {
            normalized.owner = owner;
            normalized.repo = repo;
            normalized.repo_url_configured = true;
        }
    }
    normalized
}

async fn load_saturngo_update_policy(state: &AppState) -> Result<UpdatePolicy, String> {
    let policy = match tokio::fs::read_to_string(&state.saturngo_update_policy_file).await {
        Ok(data) => serde_json::from_str::<UpdatePolicy>(&data).unwrap_or_default(),
        Err(_) => UpdatePolicy::default(),
    };
    let normalized = normalize_saturngo_update_policy(policy, state).await;
    if let Err(e) = save_saturngo_update_policy(state, normalized.clone()).await {
        error!("failed to persist saturngo update policy: {e}");
    }
    Ok(normalized)
}

async fn save_saturngo_update_policy(
    state: &AppState,
    policy: UpdatePolicy,
) -> Result<UpdatePolicy, String> {
    let normalized = normalize_saturngo_update_policy(policy, state).await;
    if let Some(parent) = state.saturngo_update_policy_file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&normalized).map_err(|e| e.to_string())?;
    tokio::fs::write(&state.saturngo_update_policy_file, bytes)
        .await
        .map_err(|e| e.to_string())?;
    let _ = tokio::fs::set_permissions(
        &state.saturngo_update_policy_file,
        std::fs::Permissions::from_mode(0o640),
    )
    .await;
    Ok(normalized)
}

async fn get_saturngo_policy(State(state): State<AppState>) -> Response {
    match load_saturngo_update_policy(&state).await {
        Ok(policy) => Json(serde_json::json!({ "policy": policy })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn set_saturngo_policy(
    State(state): State<AppState>,
    Json(policy): Json<UpdatePolicy>,
) -> Response {
    match save_saturngo_update_policy(&state, policy).await {
        Ok(policy) => Json(serde_json::json!({ "status": "ok", "policy": policy })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn load_remote_settings_file(path: &Path) -> Result<RemoteSettings, String> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RemoteSettings::default()),
        Err(e) => return Err(format!("failed to stat remote settings file: {e}")),
    };
    if metadata.len() > MAX_REMOTE_SETTINGS_FILE_BYTES {
        return Err(format!(
            "remote settings file exceeds {} bytes",
            MAX_REMOTE_SETTINGS_FILE_BYTES
        ));
    }
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("failed to read remote settings file: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(RemoteSettings::default());
    }
    serde_json::from_str::<RemoteSettings>(&raw)
        .map_err(|e| format!("invalid remote settings file: {e}"))
}

async fn save_remote_settings_file(path: &Path, settings: &RemoteSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create remote settings dir: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize remote settings: {e}"))?;
    if bytes.len() as u64 > MAX_REMOTE_SETTINGS_FILE_BYTES {
        return Err(format!(
            "remote settings payload exceeds {} bytes",
            MAX_REMOTE_SETTINGS_FILE_BYTES
        ));
    }
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("failed to write remote settings file: {e}"))?;
    let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)).await;
    Ok(())
}

async fn load_remote_profiles_file(path: &Path) -> Result<RemoteProfilesFile, String> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemoteProfilesFile::default())
        }
        Err(e) => return Err(format!("failed to stat remote profiles file: {e}")),
    };
    if metadata.len() > MAX_REMOTE_PROFILES_FILE_BYTES {
        return Err(format!(
            "remote profiles file exceeds {} bytes",
            MAX_REMOTE_PROFILES_FILE_BYTES
        ));
    }
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("failed to read remote profiles file: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(RemoteProfilesFile::default());
    }
    serde_json::from_str::<RemoteProfilesFile>(&raw)
        .map_err(|e| format!("invalid remote profiles file: {e}"))
}

async fn save_remote_profiles_file(
    path: &Path,
    profiles: &RemoteProfilesFile,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create remote profiles dir: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(profiles)
        .map_err(|e| format!("failed to serialize remote profiles: {e}"))?;
    if bytes.len() as u64 > MAX_REMOTE_PROFILES_FILE_BYTES {
        return Err(format!(
            "remote profiles payload exceeds {} bytes",
            MAX_REMOTE_PROFILES_FILE_BYTES
        ));
    }
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("failed to write remote profiles file: {e}"))?;
    let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)).await;
    Ok(())
}

fn normalize_remote_profile_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > 64 {
        return None;
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.'))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub async fn get_remote_settings(State(state): State<AppState>) -> Response {
    match load_remote_settings_file(&state.remote_settings_file).await {
        Ok(settings) => {
            Json(serde_json::json!({ "status": "ok", "settings": settings })).into_response()
        }
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn set_remote_settings(
    State(state): State<AppState>,
    Json(settings): Json<RemoteSettings>,
) -> Response {
    match save_remote_settings_file(&state.remote_settings_file, &settings).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "settings": settings })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn get_remote_profiles(State(state): State<AppState>) -> Response {
    match load_remote_profiles_file(&state.remote_profiles_file).await {
        Ok(profiles) => {
            Json(serde_json::json!({ "status": "ok", "profiles": profiles })).into_response()
        }
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn save_remote_profile(
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileSaveRequest>,
) -> Response {
    let Some(name) = normalize_remote_profile_name(&request.name) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid remote profile name");
    };
    let mut profiles = match load_remote_profiles_file(&state.remote_profiles_file).await {
        Ok(profiles) => profiles,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    profiles.profiles.insert(name.clone(), request.settings);
    if request.make_startup {
        profiles.startup_profile = Some(name.clone());
    }
    match save_remote_profiles_file(&state.remote_profiles_file, &profiles).await {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "name": name,
            "profiles": profiles
        }))
        .into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn delete_remote_profile(
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileDeleteRequest>,
) -> Response {
    let Some(name) = normalize_remote_profile_name(&request.name) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid remote profile name");
    };
    let mut profiles = match load_remote_profiles_file(&state.remote_profiles_file).await {
        Ok(profiles) => profiles,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if profiles.profiles.remove(&name).is_none() {
        return json_error(StatusCode::NOT_FOUND, "remote profile not found");
    }
    if profiles.startup_profile.as_deref() == Some(name.as_str()) {
        profiles.startup_profile = None;
    }
    match save_remote_profiles_file(&state.remote_profiles_file, &profiles).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "profiles": profiles })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn set_remote_profile_startup(
    State(state): State<AppState>,
    Json(request): Json<RemoteProfileStartupRequest>,
) -> Response {
    let mut profiles = match load_remote_profiles_file(&state.remote_profiles_file).await {
        Ok(profiles) => profiles,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    match request.name.as_deref() {
        Some(name) => {
            let Some(name) = normalize_remote_profile_name(name) else {
                return json_error(StatusCode::BAD_REQUEST, "invalid remote profile name");
            };
            if !profiles.profiles.contains_key(&name) {
                return json_error(StatusCode::NOT_FOUND, "remote profile not found");
            }
            profiles.startup_profile = Some(name);
        }
        None => profiles.startup_profile = None,
    }
    match save_remote_profiles_file(&state.remote_profiles_file, &profiles).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "profiles": profiles })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn get_saturngo_deploy_status(State(state): State<AppState>) -> Response {
    match tokio::fs::read_to_string(&state.saturngo_deploy_status_file).await {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) => {
                Json(serde_json::json!({ "status": "ok", "deploy": value })).into_response()
            }
            Err(e) => json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid Saturn Go deploy status file: {e}"),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Json(serde_json::json!({
            "status": "ok",
            "deploy": {
                "status": "idle",
                "phase": "idle",
                "message": "No Saturn Go deploy recorded yet.",
                "updated_at": Local::now().to_rfc3339(),
            }
        }))
        .into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn command_text(program: &str, args: &[&str]) -> (bool, String) {
    match Command::new(program).args(args).output().await {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                text = output_error_text(&out);
            }
            (out.status.success(), text)
        }
        Err(e) => (false, format!("error: {e}")),
    }
}

fn strip_trailing_dns_dot(value: &str) -> String {
    value.trim().trim_end_matches('.').to_string()
}

fn tailscale_remote_url(dns_name: &str) -> Option<String> {
    let dns_name = strip_trailing_dns_dot(dns_name);
    if dns_name.is_empty() {
        None
    } else {
        Some(format!(
            "https://{dns_name}/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3"
        ))
    }
}

async fn get_tailscale_status() -> Response {
    let (version_ok, version_text) = command_text("tailscale", &["version"]).await;
    let installed = version_ok || !version_text.starts_with("error:");
    let version = installed
        .then(|| version_text.lines().next().unwrap_or("").trim().to_string())
        .filter(|value| !value.is_empty());

    let (active_ok, active) = command_text("systemctl", &["is-active", "tailscaled.service"]).await;
    let (_, enabled) = command_text("systemctl", &["is-enabled", "tailscaled.service"]).await;

    let mut backend_state = String::new();
    let mut hostname = String::new();
    let mut dns_name = String::new();
    let mut tailnet_name = String::new();
    let mut tailscale_ips = Vec::<String>::new();
    let mut status_json = serde_json::Value::Null;
    let mut status_error = serde_json::Value::Null;

    if installed {
        let (status_ok, status_text) = command_text("tailscale", &["status", "--json"]).await;
        if status_ok {
            match serde_json::from_str::<serde_json::Value>(&status_text) {
                Ok(value) => {
                    backend_state = value
                        .get("BackendState")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    hostname = value
                        .get("Self")
                        .and_then(|v| v.get("HostName"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    dns_name = value
                        .get("Self")
                        .and_then(|v| v.get("DNSName"))
                        .and_then(|v| v.as_str())
                        .map(strip_trailing_dns_dot)
                        .unwrap_or_default();
                    tailnet_name = value
                        .get("CurrentTailnet")
                        .and_then(|v| v.get("Name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(values) = value
                        .get("Self")
                        .and_then(|v| v.get("TailscaleIPs"))
                        .and_then(|v| v.as_array())
                    {
                        tailscale_ips = values
                            .iter()
                            .filter_map(|value| value.as_str().map(|s| s.to_string()))
                            .collect();
                    }
                    status_json = value;
                }
                Err(e) => {
                    status_error = serde_json::json!(format!("invalid tailscale status JSON: {e}"));
                }
            }
        } else {
            status_error = serde_json::json!(status_text);
        }
    }

    let mut serve_text = String::new();
    let mut serve_json = serde_json::Value::Null;
    let mut serve_error = serde_json::Value::Null;
    let mut serve_configured = false;
    if installed {
        let (serve_ok, raw_serve_text) =
            command_text("tailscale", &["serve", "status", "--json"]).await;
        if serve_ok {
            serve_text = raw_serve_text.clone();
            serve_configured = !raw_serve_text.trim().is_empty()
                && raw_serve_text.trim() != "{}"
                && raw_serve_text.trim() != "null";
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_serve_text) {
                serve_json = value;
            }
        } else {
            serve_error = serde_json::json!(raw_serve_text);
        }
    }

    let level = if !installed {
        "bad"
    } else if !active_ok || active.trim() != "active" {
        "warn"
    } else if backend_state == "Running" {
        "ok"
    } else if backend_state.is_empty() {
        "unknown"
    } else {
        "warn"
    };

    let summary = if !installed {
        "Tailscale CLI is not installed.".to_string()
    } else if !active_ok || active.trim() != "active" {
        format!("tailscaled.service is {}.", active.trim())
    } else if backend_state == "Running" {
        "Tailscale is running.".to_string()
    } else if backend_state.is_empty() {
        "Tailscale status is unknown.".to_string()
    } else {
        format!("Tailscale backend state is {backend_state}.")
    };

    Json(serde_json::json!({
        "status": "ok",
        "level": level,
        "summary": summary,
        "installed": installed,
        "version": version,
        "service": {
            "active": active.trim(),
            "enabled": enabled.trim(),
        },
        "tailscale": {
            "backend_state": backend_state,
            "hostname": hostname,
            "dns_name": dns_name,
            "tailnet": tailnet_name,
            "ips": tailscale_ips,
            "raw_status": status_json,
            "status_error": status_error,
        },
        "serve": {
            "configured": serve_configured,
            "raw_status": serve_json,
            "text": serve_text,
            "error": serve_error,
        },
        "remote_url": tailscale_remote_url(&dns_name),
    }))
    .into_response()
}

async fn get_bridge_diag() -> Response {
    fn strip_journal_prefix(line: &str) -> &str {
        line.split_once("saturn-bridge")
            .map(|(_, rest)| {
                rest.trim_start_matches([
                    '[', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ']',
                ])
            })
            .and_then(|rest| rest.split_once(": ").map(|(_, msg)| msg))
            .unwrap_or(line)
    }

    fn parse_diag_fields(line: &str) -> BTreeMap<String, serde_json::Value> {
        let mut fields = BTreeMap::<String, serde_json::Value>::new();
        let Some((_, rest)) = line.split_once("diag ") else {
            return fields;
        };
        for token in rest.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            let json_value = if value == "-" {
                serde_json::Value::Null
            } else if let Ok(v) = value.parse::<i64>() {
                serde_json::json!(v)
            } else if let Ok(v) = value.parse::<f64>() {
                serde_json::json!(v)
            } else {
                serde_json::json!(value)
            };
            fields.insert(key.to_string(), json_value);
        }
        fields
    }

    let service_name = "saturn-bridge.service";
    let (active_ok, active) = command_text("systemctl", &["is-active", service_name]).await;
    let (_, enabled) = command_text("systemctl", &["is-enabled", service_name]).await;
    let (_, main_pid_text) = command_text(
        "systemctl",
        &["show", "-p", "MainPID", "--value", service_name],
    )
    .await;
    let (_, environment) = command_text(
        "systemctl",
        &["show", "-p", "Environment", "--value", service_name],
    )
    .await;
    let (journal_ok, journal) = command_text(
        "journalctl",
        &[
            "-u",
            service_name,
            "-n",
            "180",
            "--no-pager",
            "-o",
            "short-iso",
        ],
    )
    .await;
    let main_pid = main_pid_text.trim().parse::<u32>().ok().filter(|v| *v > 0);
    let running_exe = main_pid.and_then(|pid| {
        fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.display().to_string())
    });

    let lines: Vec<&str> = journal.lines().collect();
    let diag_lines: Vec<String> = lines
        .iter()
        .filter(|line| line.contains("saturn-bridge: diag "))
        .rev()
        .take(20)
        .map(|line| strip_journal_prefix(line).to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let status_lines: Vec<String> = lines
        .iter()
        .filter(|line| {
            line.contains("saturn-bridge: vfoA=")
                || line.contains("saturn-bridge: TX ")
                || line.contains("saturn-bridge: remote TX")
                || line.contains("saturn-bridge: clamping remote TX")
        })
        .rev()
        .take(20)
        .map(|line| strip_journal_prefix(line).to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let latest_diag = diag_lines.last().map(|line| {
        serde_json::json!({
            "line": line,
            "fields": parse_diag_fields(line),
        })
    });
    let latest_status = status_lines.last().cloned();

    Json(serde_json::json!({
        "status": "ok",
        "bridge": {
            "service": {
                "name": service_name,
                "active": active,
                "active_ok": active_ok,
                "enabled": enabled,
                "main_pid": main_pid_text.trim(),
                "running_exe": running_exe,
                "environment": environment,
            },
            "journal": {
                "ok": journal_ok,
                "source": "journalctl -u saturn-bridge.service -n 180 --no-pager -o short-iso",
                "latest_diag": latest_diag,
                "latest_status": latest_status,
                "diag_lines": diag_lines,
                "status_lines": status_lines,
            }
        }
    }))
    .into_response()
}

async fn get_p23_status(State(state): State<AppState>) -> Response {
    fn adc_peak_telemetry_info(service_main_pid: Option<u32>) -> serde_json::Value {
        let control = PathBuf::from(P23_ADC_PEAK_TELEMETRY_ENABLE_FILE);
        let snapshot = PathBuf::from(P23_ADC_PEAK_TELEMETRY_JSON_FILE);
        let enabled = control.exists();
        let metadata = fs::metadata(&snapshot).ok();
        let snapshot_exists = metadata.is_some();
        let modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<Local>::from(t).to_rfc3339());
        let (current, read_error, parse_error) = if snapshot_exists {
            match fs::read_to_string(&snapshot) {
                Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(value) => (Some(value), None, None),
                    Err(error) => (None, None, Some(error.to_string())),
                },
                Err(error) => (None, Some(error.to_string()), None),
            }
        } else {
            (None, None, None)
        };
        let snapshot_pid = current
            .as_ref()
            .and_then(|value| value.get("pid"))
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok());
        let pid_matches_service = matches!(
            (snapshot_pid, service_main_pid),
            (Some(snapshot_pid), Some(service_pid)) if snapshot_pid == service_pid
        );
        let snapshot_timestamp = current
            .as_ref()
            .and_then(|value| value.get("timestamp_epoch"))
            .and_then(|value| value.as_u64());
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let age_seconds = snapshot_timestamp.map(|timestamp| now_epoch.saturating_sub(timestamp));
        let state = if !enabled {
            "disabled"
        } else if !snapshot_exists {
            "waiting_for_radio"
        } else if read_error.is_some() {
            "unreadable"
        } else if parse_error.is_some() {
            "invalid"
        } else if !pid_matches_service {
            "stale_process"
        } else if age_seconds.is_some_and(|age| age > 5) {
            "stale"
        } else {
            "live"
        };

        serde_json::json!({
            "state": state,
            "enabled": enabled,
            "control_file": control.display().to_string(),
            "snapshot_file": snapshot.display().to_string(),
            "snapshot_exists": snapshot_exists,
            "snapshot_readable": current.is_some() && read_error.is_none() && parse_error.is_none(),
            "modified": modified,
            "age_seconds": age_seconds,
            "snapshot_pid": snapshot_pid,
            "pid_matches_service": pid_matches_service,
            "read_error": read_error,
            "parse_error": parse_error,
            "current": current,
        })
    }

    fn file_info(path: &Path) -> serde_json::Value {
        match fs::metadata(path) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .map(|t| chrono::DateTime::<Local>::from(t).to_rfc3339());
                serde_json::json!({
                    "path": path.display().to_string(),
                    "exists": true,
                    "is_file": meta.is_file(),
                    "size_bytes": meta.len(),
                    "modified": modified,
                })
            }
            Err(_) => serde_json::json!({
                "path": path.display().to_string(),
                "exists": false,
            }),
        }
    }

    fn dir_info(path: &Path) -> serde_json::Value {
        serde_json::json!({
            "path": path.display().to_string(),
            "exists": path.is_dir(),
        })
    }

    fn systemd_env_value(env_text: &str, key: &str) -> Option<String> {
        let prefix = format!("{key}=");
        env_text
            .split_whitespace()
            .find_map(|token| token.strip_prefix(&prefix).map(str::to_string))
    }

    fn parse_boolish(value: Option<&str>) -> Option<bool> {
        match value.map(|v| v.trim().to_ascii_lowercase()) {
            Some(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => Some(true),
            Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => Some(false),
            _ => None,
        }
    }

    async fn systemctl_text(args: &[&str]) -> String {
        match Command::new("systemctl").args(args).output().await {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !text.is_empty() {
                    text
                } else {
                    output_error_text(&out)
                }
            }
            Err(e) => format!("error: {e}"),
        }
    }

    let repo_root = current_repo_root(&state);
    let p2_dir = repo_root.join("sw_projects/P2_app");
    let p3_dir = repo_root.join("sw_projects/P3_app");
    let p2_bin = p2_dir.join("p2app");
    let p3_bin = p3_dir.join("p3app");

    let deploy_root = PathBuf::from("/opt/saturn-go/p23-apps");
    let deploy_p2 = deploy_root.join("p2app");
    let deploy_p3 = deploy_root.join("p3app");
    let current_link = deploy_root.join("current");
    let override_file =
        PathBuf::from("/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf");
    let service_name = "p2app.service";

    let symlink_meta = fs::symlink_metadata(&current_link).ok();
    let current_target = fs::read_link(&current_link)
        .ok()
        .map(|p| p.display().to_string());
    let current_target_abs = fs::canonicalize(&current_link)
        .ok()
        .map(|p| p.display().to_string());
    let selected_app = match current_target_abs.as_deref() {
        Some(v) if v == deploy_p2.display().to_string() => Some("p2"),
        Some(v) if v == deploy_p3.display().to_string() => Some("p3"),
        _ => None,
    };

    let active = systemctl_text(&["is-active", service_name]).await;
    let enabled = systemctl_text(&["is-enabled", service_name]).await;
    let main_pid_text = systemctl_text(&["show", "-p", "MainPID", "--value", service_name]).await;
    let service_environment_text =
        systemctl_text(&["show", "-p", "Environment", "--value", service_name]).await;
    let main_pid = main_pid_text.trim().parse::<u32>().ok().filter(|v| *v > 0);
    let running_exe = main_pid.and_then(|pid| {
        fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.display().to_string())
    });

    let override_contents = fs::read_to_string(&override_file).ok();
    let override_execstart = override_contents.as_deref().and_then(|text| {
        text.lines()
            .map(str::trim)
            .find(|line| line.starts_with("ExecStart=") && *line != "ExecStart=")
            .map(|s| s.to_string())
    });
    let override_panel_mode = override_contents.as_deref().and_then(|text| {
        text.lines().map(str::trim).find_map(|line| {
            line.strip_prefix("Environment=SATURN_FRONT_PANEL_MODE=")
                .map(str::to_string)
        })
    });
    let override_saturn_metadata = override_contents.as_deref().and_then(|text| {
        text.lines().map(str::trim).find_map(|line| {
            let meta = line.strip_prefix("# saturn-p23 ")?;
            let mut mode = None::<String>;
            let mut panel = None::<String>;
            for token in meta.split_whitespace() {
                if let Some(v) = token.strip_prefix("mode=") {
                    mode = Some(v.to_string());
                } else if let Some(v) = token.strip_prefix("panel=") {
                    panel = Some(v.to_string());
                }
            }
            Some(serde_json::json!({
                "mode": mode,
                "panel": panel,
            }))
        })
    });
    let service_environment_error = service_environment_text
        .strip_prefix("error: ")
        .map(str::to_string);
    let service_panel_mode =
        systemd_env_value(&service_environment_text, "SATURN_FRONT_PANEL_MODE");
    let rt_enable_raw = systemd_env_value(&service_environment_text, "SATURN_P3_RT_AUDIO_ENABLE");
    let rt_policy = systemd_env_value(&service_environment_text, "SATURN_P3_RT_AUDIO_POLICY");
    let rt_priority_raw =
        systemd_env_value(&service_environment_text, "SATURN_P3_RT_AUDIO_PRIORITY");
    let rt_priority = rt_priority_raw
        .as_deref()
        .and_then(|value| value.parse::<i32>().ok());
    let rt_cpus = systemd_env_value(&service_environment_text, "SATURN_P3_RT_AUDIO_CPUS");
    let rt_enabled = parse_boolish(rt_enable_raw.as_deref()).unwrap_or(false);
    let rt_configured = rt_enable_raw.is_some()
        || rt_policy.is_some()
        || rt_priority_raw.is_some()
        || rt_cpus.is_some();
    let mut relevant_environment = BTreeMap::<String, String>::new();
    if let Some(value) = service_panel_mode.clone() {
        relevant_environment.insert("SATURN_FRONT_PANEL_MODE".to_string(), value);
    }
    if let Some(value) = rt_enable_raw.clone() {
        relevant_environment.insert("SATURN_P3_RT_AUDIO_ENABLE".to_string(), value);
    }
    if let Some(value) = rt_policy.clone() {
        relevant_environment.insert("SATURN_P3_RT_AUDIO_POLICY".to_string(), value);
    }
    if let Some(value) = rt_priority_raw.clone() {
        relevant_environment.insert("SATURN_P3_RT_AUDIO_PRIORITY".to_string(), value);
    }
    if let Some(value) = rt_cpus.clone() {
        relevant_environment.insert("SATURN_P3_RT_AUDIO_CPUS".to_string(), value);
    }

    Json(serde_json::json!({
        "status": "ok",
        "p23": {
            "repo_root": repo_root.display().to_string(),
            "service": {
                "name": service_name,
                "active": active,
                "enabled": enabled,
                "main_pid": main_pid,
                "running_exe": running_exe,
            },
            "sources": {
                "p2_dir": dir_info(&p2_dir),
                "p3_dir": dir_info(&p3_dir),
                "p2_bin": file_info(&p2_bin),
                "p3_bin": file_info(&p3_bin),
            },
            "deployed": {
                "deploy_root": dir_info(&deploy_root),
                "p2_bin": file_info(&deploy_p2),
                "p3_bin": file_info(&deploy_p3),
                "current": {
                    "path": current_link.display().to_string(),
                    "exists": symlink_meta.is_some(),
                    "is_symlink": symlink_meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false),
                    "target": current_target,
                    "target_abs": current_target_abs,
                    "selected_app": selected_app,
                }
            },
            "override": {
                "path": override_file.display().to_string(),
                "exists": override_file.exists(),
                "exec_start": override_execstart,
                "panel_mode": override_panel_mode,
                "panel_mode_effective": service_panel_mode.clone().or_else(|| override_panel_mode.clone()),
                "saturn_meta": override_saturn_metadata,
                "contents": override_contents,
            },
            "service_runtime": {
                "source": "systemctl show -p Environment --value",
                "error": service_environment_error,
                "front_panel_mode": service_panel_mode.clone().or_else(|| override_panel_mode.clone()),
                "environment": relevant_environment,
                "audio_rt": {
                    "configured": rt_configured,
                    "enabled": rt_enabled,
                    "enable_raw": rt_enable_raw,
                    "policy": rt_policy,
                    "priority": rt_priority,
                    "priority_raw": rt_priority_raw,
                    "cpus": rt_cpus,
                }
            },
            "adc_peak_telemetry": adc_peak_telemetry_info(main_pid),
        }
    }))
    .into_response()
}

async fn set_p23_adc_telemetry(Json(req): Json<P23AdcTelemetryRequest>) -> Response {
    let control = PathBuf::from(P23_ADC_PEAK_TELEMETRY_ENABLE_FILE);
    let snapshot = PathBuf::from(P23_ADC_PEAK_TELEMETRY_JSON_FILE);

    let result = if req.enabled {
        fs::write(&control, b"1\n")
    } else {
        let _ = fs::remove_file(&control);
        let _ = fs::remove_file(&snapshot);
        Ok(())
    };

    match result {
        Ok(_) => Json(serde_json::json!({
            "status": "ok",
            "enabled": req.enabled,
            "control_file": control.display().to_string(),
            "snapshot_file": snapshot.display().to_string(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("failed to update ADC telemetry toggle: {e}")
            })),
        )
            .into_response(),
    }
}

async fn get_p23_perf(State(_state): State<AppState>) -> Response {
    fn parse_system_cpu() -> Option<(u64, u64, u64)> {
        let raw = fs::read_to_string("/proc/stat").ok()?;
        let line = raw.lines().find(|l| l.starts_with("cpu "))?;
        let mut nums = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse::<u64>().ok());
        let user = nums.next()?;
        let nice = nums.next()?;
        let system = nums.next()?;
        let idle = nums.next()?;
        let iowait = nums.next().unwrap_or(0);
        let irq = nums.next().unwrap_or(0);
        let softirq = nums.next().unwrap_or(0);
        let steal = nums.next().unwrap_or(0);
        let _guest = nums.next().unwrap_or(0);
        let _guest_nice = nums.next().unwrap_or(0);
        let total = user + nice + system + idle + iowait + irq + softirq + steal;
        Some((total, idle, iowait))
    }

    fn parse_meminfo() -> (Option<u64>, Option<u64>) {
        let raw = match fs::read_to_string("/proc/meminfo") {
            Ok(v) => v,
            Err(_) => return (None, None),
        };
        let mut total = None::<u64>;
        let mut avail = None::<u64>;
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|kb| kb * 1024);
            } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                avail = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|kb| kb * 1024);
            }
        }
        (total, avail)
    }

    fn parse_loadavg() -> (Option<f64>, Option<f64>, Option<f64>) {
        let raw = match fs::read_to_string("/proc/loadavg") {
            Ok(v) => v,
            Err(_) => return (None, None, None),
        };
        let mut it = raw.split_whitespace();
        let one = it.next().and_then(|s| s.parse::<f64>().ok());
        let five = it.next().and_then(|s| s.parse::<f64>().ok());
        let fifteen = it.next().and_then(|s| s.parse::<f64>().ok());
        (one, five, fifteen)
    }

    fn parse_netdev_interface(name: &str) -> serde_json::Value {
        let raw = match fs::read_to_string("/proc/net/dev") {
            Ok(v) => v,
            Err(_) => {
                return serde_json::json!({
                    "name": name,
                    "present": false,
                })
            }
        };
        for line in raw.lines().skip(2) {
            let (iface_raw, stats_raw) = match line.split_once(':') {
                Some(v) => v,
                None => continue,
            };
            let iface = iface_raw.trim();
            if iface != name {
                continue;
            }
            let nums: Vec<u64> = stats_raw
                .split_whitespace()
                .filter_map(|s| s.parse::<u64>().ok())
                .collect();
            if nums.len() < 16 {
                return serde_json::json!({
                    "name": name,
                    "present": true,
                    "parse_error": true,
                });
            }
            return serde_json::json!({
                "name": name,
                "present": true,
                "rx": {
                    "bytes": nums[0],
                    "packets": nums[1],
                    "errs": nums[2],
                    "drop": nums[3],
                    "fifo": nums[4],
                    "frame": nums[5],
                    "compressed": nums[6],
                    "multicast": nums[7],
                },
                "tx": {
                    "bytes": nums[8],
                    "packets": nums[9],
                    "errs": nums[10],
                    "drop": nums[11],
                    "fifo": nums[12],
                    "colls": nums[13],
                    "carrier": nums[14],
                    "compressed": nums[15],
                }
            });
        }
        serde_json::json!({
            "name": name,
            "present": false,
        })
    }

    fn read_trimmed(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    fn parse_xdma_interrupts() -> serde_json::Value {
        let raw = match fs::read_to_string("/proc/interrupts") {
            Ok(v) => v,
            Err(_) => return serde_json::json!({ "present": false }),
        };

        let mut lines_json = Vec::<serde_json::Value>::new();
        let mut total_count: u64 = 0;
        let mut found = false;

        for line in raw.lines() {
            if !line.to_ascii_lowercase().contains("xdma") {
                continue;
            }
            let (irq, rest) = match line.split_once(':') {
                Some(v) => v,
                None => continue,
            };
            let mut line_count: u64 = 0;
            for tok in rest.split_whitespace() {
                match tok.parse::<u64>() {
                    Ok(v) => line_count = line_count.saturating_add(v),
                    Err(_) => break,
                }
            }
            total_count = total_count.saturating_add(line_count);
            found = true;
            lines_json.push(serde_json::json!({
                "irq": irq.trim(),
                "count": line_count,
                "raw": line.trim(),
            }));
        }

        let pcie_speed = read_trimmed("/sys/class/xdma/xdma0_control/device/current_link_speed")
            .or_else(|| read_trimmed("/sys/class/xdma/xdma0_h2c_0/device/current_link_speed"));
        let pcie_width = read_trimmed("/sys/class/xdma/xdma0_control/device/current_link_width")
            .or_else(|| read_trimmed("/sys/class/xdma/xdma0_h2c_0/device/current_link_width"));

        serde_json::json!({
            "present": found || pcie_speed.is_some() || pcie_width.is_some(),
            "interrupts_total": if found { Some(total_count) } else { None::<u64> },
            "interrupt_lines": lines_json,
            "pcie": {
                "current_link_speed": pcie_speed,
                "current_link_width": pcie_width,
            }
        })
    }

    fn parse_proc_status(pid: u32) -> serde_json::Value {
        let path = format!("/proc/{pid}/status");
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return serde_json::json!({}),
        };
        let mut vmrss = None::<u64>;
        let mut threads = None::<u64>;
        let mut vctx = None::<u64>;
        let mut nvctx = None::<u64>;
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                vmrss = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|kb| kb * 1024);
            } else if let Some(v) = line.strip_prefix("Threads:") {
                threads = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("voluntary_ctxt_switches:") {
                vctx = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
                nvctx = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            }
        }
        serde_json::json!({
            "vmrss_bytes": vmrss,
            "threads": threads,
            "voluntary_ctxt_switches": vctx,
            "nonvoluntary_ctxt_switches": nvctx,
        })
    }

    fn parse_proc_io(pid: u32) -> serde_json::Value {
        let path = format!("/proc/{pid}/io");
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return serde_json::json!({}),
        };
        let mut rchar = None::<u64>;
        let mut wchar = None::<u64>;
        let mut read_bytes = None::<u64>;
        let mut write_bytes = None::<u64>;
        let mut cancelled_write_bytes = None::<u64>;
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("rchar:") {
                rchar = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("wchar:") {
                wchar = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("read_bytes:") {
                read_bytes = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("write_bytes:") {
                write_bytes = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            } else if let Some(v) = line.strip_prefix("cancelled_write_bytes:") {
                cancelled_write_bytes = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok());
            }
        }
        serde_json::json!({
            "rchar": rchar,
            "wchar": wchar,
            "read_bytes": read_bytes,
            "write_bytes": write_bytes,
            "cancelled_write_bytes": cancelled_write_bytes,
            "source": "procfs",
        })
    }

    fn netdev_bytes(interface: &serde_json::Value) -> (Option<u64>, Option<u64>) {
        let rx_bytes = interface
            .get("rx")
            .and_then(|rx| rx.get("bytes"))
            .and_then(|v| v.as_u64());
        let tx_bytes = interface
            .get("tx")
            .and_then(|tx| tx.get("bytes"))
            .and_then(|v| v.as_u64());
        (rx_bytes, tx_bytes)
    }

    fn parse_proc_schedstat(pid: u32) -> serde_json::Value {
        let path = format!("/proc/{pid}/schedstat");
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return serde_json::json!({}),
        };
        let mut it = raw.split_whitespace();
        let runtime_ns = it.next().and_then(|s| s.parse::<u64>().ok());
        let run_delay_ns = it.next().and_then(|s| s.parse::<u64>().ok());
        let timeslices = it.next().and_then(|s| s.parse::<u64>().ok());
        serde_json::json!({
            "runtime_ns": runtime_ns,
            "run_delay_ns": run_delay_ns,
            "timeslices": timeslices,
        })
    }

    fn parse_proc_stat(pid: u32, page_size: u64) -> serde_json::Value {
        let path = format!("/proc/{pid}/stat");
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return serde_json::json!({}),
        };
        let l = match raw.find('(') {
            Some(v) => v,
            None => return serde_json::json!({}),
        };
        let r = match raw.rfind(')') {
            Some(v) => v,
            None => return serde_json::json!({}),
        };
        let comm = raw.get(l + 1..r).unwrap_or("").to_string();
        let tail = raw.get(r + 2..).unwrap_or("");
        let fields: Vec<&str> = tail.split_whitespace().collect();
        if fields.len() < 22 {
            return serde_json::json!({ "comm": comm });
        }
        let state = fields.first().copied().map(str::to_string);
        let minflt = fields.get(7).and_then(|s| s.parse::<u64>().ok());
        let majflt = fields.get(9).and_then(|s| s.parse::<u64>().ok());
        let utime_ticks = fields.get(11).and_then(|s| s.parse::<u64>().ok());
        let stime_ticks = fields.get(12).and_then(|s| s.parse::<u64>().ok());
        let num_threads = fields.get(17).and_then(|s| s.parse::<u64>().ok());
        let starttime_ticks = fields.get(19).and_then(|s| s.parse::<u64>().ok());
        let vsize_bytes = fields.get(20).and_then(|s| s.parse::<u64>().ok());
        let rss_pages = fields.get(21).and_then(|s| s.parse::<i64>().ok());
        let rss_bytes = rss_pages.and_then(|p| {
            if p < 0 {
                None
            } else {
                Some((p as u64).saturating_mul(page_size))
            }
        });
        serde_json::json!({
            "comm": comm,
            "state": state,
            "minflt": minflt,
            "majflt": majflt,
            "utime_ticks": utime_ticks,
            "stime_ticks": stime_ticks,
            "num_threads": num_threads,
            "starttime_ticks": starttime_ticks,
            "vsize_bytes": vsize_bytes,
            "rss_pages": rss_pages,
            "rss_bytes": rss_bytes,
        })
    }

    fn cmdline_for_pid(pid: u32) -> Option<Vec<String>> {
        let bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let mut parts = Vec::new();
        for part in bytes.split(|b| *b == 0) {
            if part.is_empty() {
                continue;
            }
            parts.push(String::from_utf8_lossy(part).to_string());
        }
        Some(parts)
    }

    fn exe_for_pid(pid: u32) -> Option<String> {
        fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.display().to_string())
    }

    fn fd_count_for_pid(pid: u32) -> Option<u64> {
        let it = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
        Some(it.filter_map(Result::ok).count() as u64)
    }

    fn page_size_bytes() -> u64 {
        // Linux on Raspberry Pi uses 4 KiB pages; keep this lightweight and dependency-free.
        4096
    }

    fn clock_ticks_per_sec() -> u64 {
        // Linux procfs CPU accounting is typically USER_HZ=100 on Raspberry Pi.
        100
    }

    async fn systemctl_value(args: &[&str]) -> Option<String> {
        let out = Command::new("systemctl").args(args).output().await.ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    fn p23_workload_info(main_pid: Option<u32>) -> serde_json::Value {
        let deploy_root = PathBuf::from("/opt/saturn-go/p23-apps");
        let deploy_p2 = deploy_root.join("p2app");
        let deploy_p3 = deploy_root.join("p3app");
        let current_link = deploy_root.join("current");
        let override_file =
            PathBuf::from("/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf");

        let current_target = fs::read_link(&current_link)
            .ok()
            .map(|p| p.display().to_string());
        let current_target_abs = fs::canonicalize(&current_link)
            .ok()
            .map(|p| p.display().to_string());
        let service_cmdline = main_pid.and_then(cmdline_for_pid);
        let inferred_app = service_cmdline.as_ref().and_then(|parts| {
            parts.first().and_then(|binary| {
                let name = Path::new(binary).file_name()?.to_str()?;
                match name {
                    "p2app" => Some("p2".to_string()),
                    "p3app" => Some("p3".to_string()),
                    _ => None,
                }
            })
        });
        let selected_app = match current_target_abs.as_deref() {
            Some(v) if v == deploy_p2.display().to_string() => Some("p2".to_string()),
            Some(v) if v == deploy_p3.display().to_string() => Some("p3".to_string()),
            _ => inferred_app,
        };

        let override_contents = fs::read_to_string(&override_file).ok();
        let panel_mode = override_contents.as_deref().and_then(|text| {
            text.lines().map(str::trim).find_map(|line| {
                line.strip_prefix("Environment=SATURN_FRONT_PANEL_MODE=")
                    .map(str::to_string)
            })
        });
        let saturn_meta = override_contents.as_deref().and_then(|text| {
            text.lines().map(str::trim).find_map(|line| {
                let meta = line.strip_prefix("# saturn-p23 ")?;
                let mut mode = None::<String>;
                let mut panel = None::<String>;
                for token in meta.split_whitespace() {
                    if let Some(v) = token.strip_prefix("mode=") {
                        mode = Some(v.to_string());
                    } else if let Some(v) = token.strip_prefix("panel=") {
                        panel = Some(v.to_string());
                    }
                }
                Some(serde_json::json!({
                    "mode": mode,
                    "panel": panel,
                }))
            })
        });

        let mode = saturn_meta
            .as_ref()
            .and_then(|meta| meta.get("mode"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| selected_app.as_ref().map(|_| "service-default".to_string()));
        let panel = panel_mode
            .clone()
            .or_else(|| {
                saturn_meta
                    .as_ref()
                    .and_then(|meta| meta.get("panel"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .or_else(|| {
                service_cmdline.as_ref().map(|parts| {
                    if parts.iter().any(|arg| arg == "-p") {
                        "enabled".to_string()
                    } else {
                        "off".to_string()
                    }
                })
            });
        let selected = selected_app.as_deref().unwrap_or("unknown");
        let mode_key = mode.clone().unwrap_or_else(|| "n/a".to_string());
        let panel_key = panel.clone().unwrap_or_else(|| "n/a".to_string());
        let source = if current_target_abs.is_some() {
            "deployment-slot"
        } else if selected_app.is_some() {
            "service-cmdline"
        } else {
            "unknown"
        };

        serde_json::json!({
            "selected_app": selected_app,
            "current_target": current_target,
            "current_target_abs": current_target_abs,
            "startup_mode": mode,
            "panel_mode": panel,
            "saturn_meta": saturn_meta,
            "source": source,
            "service_cmdline": service_cmdline,
            "service_main_pid": main_pid,
            "workload_key": format!("{selected}|mode={mode_key}|panel={panel_key}"),
        })
    }

    fn p23_app_perf_telemetry(main_pid: Option<u32>) -> serde_json::Value {
        let snapshot = PathBuf::from(P23_APP_PERF_TELEMETRY_JSON_FILE);
        let meta = fs::metadata(&snapshot).ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<Local>::from(t).to_rfc3339());
        let snapshot_text = fs::read_to_string(&snapshot);
        let read_error = snapshot_text.as_ref().err().map(ToString::to_string);
        let (current, parse_error) = match snapshot_text {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => (Some(value), None),
                Err(error) => (None, Some(error.to_string())),
            },
            Err(_) => (None, None),
        };
        let snapshot_pid = current
            .as_ref()
            .and_then(|v| v.get("pid"))
            .and_then(|v| v.as_u64());
        let pid_matches_service = match (main_pid, snapshot_pid) {
            (Some(service_pid), Some(snapshot_pid)) => snapshot_pid == service_pid as u64,
            _ => false,
        };
        let age_seconds = current
            .as_ref()
            .and_then(|v| v.get("timestamp_epoch"))
            .and_then(|v| v.as_i64())
            .and_then(|ts| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|now| now.as_secs() as i64 - ts)
            });

        serde_json::json!({
            "snapshot_file": snapshot.display().to_string(),
            "snapshot_exists": snapshot.exists(),
            "snapshot_readable": read_error.is_none() && current.is_some(),
            "read_error": read_error,
            "parse_error": parse_error,
            "modified": modified,
            "pid_matches_service": pid_matches_service,
            "snapshot_pid": snapshot_pid,
            "age_seconds": age_seconds,
            "current": current,
        })
    }

    let service_name = "p2app.service";
    let main_pid = systemctl_value(&["show", "-p", "MainPID", "--value", service_name])
        .await
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|v| *v > 0);

    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let clk_tck = clock_ticks_per_sec();
    let page_size = page_size_bytes();
    let (cpu_total_ticks, cpu_idle_ticks, cpu_iowait_ticks) =
        parse_system_cpu().unwrap_or((0, 0, 0));
    let (mem_total_bytes, mem_available_bytes) = parse_meminfo();
    let (load_1, load_5, load_15) = parse_loadavg();
    let soc_temp_c = read_trimmed("/sys/class/thermal/thermal_zone0/temp")
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value / 1000.0);
    let cpu_frequency_mhz = read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .or_else(|| read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_cur_freq"))
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value / 1000.0);
    let cpu_frequency_max_mhz =
        read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq")
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value / 1000.0);

    let eth0 = parse_netdev_interface("eth0");
    let wlan0 = parse_netdev_interface("wlan0");

    let process = main_pid.and_then(|pid| {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return None;
        }
        let mut io = parse_proc_io(pid);
        let io_empty = io.as_object().map(|obj| obj.is_empty()).unwrap_or(true);
        if io_empty {
            let (rx_bytes, tx_bytes) = netdev_bytes(&eth0);
            if rx_bytes.is_some() || tx_bytes.is_some() {
                io = serde_json::json!({
                    "rchar": rx_bytes,
                    "wchar": tx_bytes,
                    "read_bytes": None::<u64>,
                    "write_bytes": None::<u64>,
                    "cancelled_write_bytes": None::<u64>,
                    "source": "eth0_netdev_proxy",
                });
            }
        }
        Some(serde_json::json!({
            "pid": pid,
            "exe": exe_for_pid(pid),
            "cmdline": cmdline_for_pid(pid),
            "fd_count": fd_count_for_pid(pid),
            "stat": parse_proc_stat(pid, page_size),
            "status": parse_proc_status(pid),
            "io": io,
            "schedstat": parse_proc_schedstat(pid),
        }))
    });

    let collected_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Json(serde_json::json!({
        "status": "ok",
        "perf": {
            "collected_at_ms": collected_at_ms,
            "service": {
                "name": service_name,
                "main_pid": main_pid,
            },
            "system": {
                "cpu_count": cpu_count,
                "clock_ticks_per_sec": clk_tck,
                "page_size_bytes": page_size,
                "cpu": {
                    "total_ticks": cpu_total_ticks,
                    "idle_ticks": cpu_idle_ticks,
                    "iowait_ticks": cpu_iowait_ticks,
                },
                "memory": {
                    "total_bytes": mem_total_bytes,
                    "available_bytes": mem_available_bytes,
                },
                "loadavg": {
                    "one": load_1,
                    "five": load_5,
                    "fifteen": load_15,
                },
                "hardware": {
                    "soc_temp_c": soc_temp_c,
                    "cpu_frequency_mhz": cpu_frequency_mhz,
                    "cpu_frequency_max_mhz": cpu_frequency_max_mhz,
                }
            },
            "network": {
                "eth0": eth0,
                "wlan0": wlan0,
            },
            "xdma": parse_xdma_interrupts(),
            "workload": p23_workload_info(main_pid),
            "app_telemetry": p23_app_perf_telemetry(main_pid),
            "process": process,
        }
    }))
    .into_response()
}

fn tree_stats_sync(root: &Path) -> (u64, u64, u64) {
    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        dirs += 1;
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            match ent.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(ft) if ft.is_file() => {
                    files += 1;
                    if let Ok(meta) = ent.metadata() {
                        bytes += meta.len();
                    }
                }
                _ => {}
            }
        }
    }
    (files, dirs, bytes)
}

async fn tree_stats(root: PathBuf) -> (u64, u64, u64) {
    tokio::task::spawn_blocking(move || tree_stats_sync(&root))
        .await
        .unwrap_or((0, 0, 0))
}

async fn get_versions(State(state): State<AppState>) -> impl IntoResponse {
    let entries = read_all_script_entries(&state).await.unwrap_or_default();
    let mut versions = BTreeMap::new();
    for e in entries {
        let v = e.version.unwrap_or_else(|| "unknown".to_string());
        versions.insert(e.filename, v);
    }
    Json(serde_json::json!({ "versions": versions }))
}

async fn backup_full(State(state): State<AppState>) -> Result<Response, Response> {
    let repo_root = current_repo_root(&state);
    if !repo_root.is_dir() {
        return Err((StatusCode::BAD_REQUEST, "repo_root is not a directory").into_response());
    }

    let parent = repo_root.parent().unwrap_or(Path::new("/")).to_path_buf();
    let base = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Saturn");

    let ts = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("{base}.bak-{ts}.tar.gz");

    let mut cmd = Command::new("tar");
    cmd.arg("-C")
        .arg(&parent)
        .arg("-czf")
        .arg("-")
        .arg(base)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()),
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "tar stdout missing").into_response())
        }
    };
    let stderr = child.stderr.take();

    tokio::spawn(async move {
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                error!("tar stderr: {line}");
            }
        }
        if let Ok(status) = child.wait().await {
            if !status.success() {
                error!("tar exited with status {status}");
            }
        }
    });

    let stream = ReaderStream::new(stdout);
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/gzip"));
    headers.insert(
        "Content-Disposition",
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );

    Ok((headers, body).into_response())
}

#[derive(Deserialize, Default)]
struct RestoreQuery {
    dry_run: Option<String>,
}

#[derive(Serialize)]
struct G2BackupEntry {
    name: String,
    path: String,
    files: u64,
    dirs: u64,
    bytes: u64,
    modified_epoch: u64,
}

#[derive(Deserialize)]
struct G2RestoreReq {
    backup_name: String,
    dry_run: Option<bool>,
    confirm: Option<String>,
}

async fn resolve_backup_path(name: &str, prefix: &str) -> Result<PathBuf, String> {
    if !is_safe_backup_name_with_prefix(name, prefix) {
        return Err("invalid backup name".to_string());
    }
    let home = backup_home_dir();
    let home_canon = tokio::fs::canonicalize(&home)
        .await
        .map_err(|e| format!("cannot resolve backup home: {e}"))?;
    let candidate = home.join(name);
    let candidate_canon = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|e| format!("backup not found: {e}"))?;
    if !candidate_canon.starts_with(&home_canon) {
        return Err("backup path escapes home directory".to_string());
    }
    let meta = tokio::fs::metadata(&candidate_canon)
        .await
        .map_err(|e| format!("cannot read backup metadata: {e}"))?;
    if !meta.is_dir() {
        return Err("backup path is not a directory".to_string());
    }
    Ok(candidate_canon)
}

async fn list_backups_with_prefix(prefix: &str) -> Response {
    let home = backup_home_dir();
    let mut rows: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut read_dir = match tokio::fs::read_dir(&home).await {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to list backups: {e}"),
            )
        }
    };

    while let Ok(Some(ent)) = read_dir.next_entry().await {
        let name = match ent.file_name().to_str() {
            Some(v) => v.to_string(),
            None => continue,
        };
        if !is_safe_backup_name_with_prefix(&name, prefix) {
            continue;
        }
        let path = ent.path();
        let meta = match ent.metadata().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        let modified_epoch = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        rows.push((name, path, modified_epoch));
    }

    rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

    let mut backups = Vec::new();
    for (name, path, modified_epoch) in rows {
        let (files, dirs, bytes) = tree_stats(path.clone()).await;
        backups.push(G2BackupEntry {
            name,
            path: path.display().to_string(),
            files,
            dirs,
            bytes,
            modified_epoch,
        });
    }

    Json(serde_json::json!({
        "home": home,
        "backups": backups,
    }))
    .into_response()
}

async fn g2_backups() -> Response {
    list_backups_with_prefix("saturn-backup-").await
}

async fn pihpsdr_backups() -> Response {
    list_backups_with_prefix("pihpsdr-backup-").await
}

fn has_unsafe_path_component(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|c| c == "..")
}

/// Parses one line of GNU tar verbose (`-v`) output.
/// Format: "PERMS OWNER SIZE DATE TIME PATH[ -> TARGET]"
/// Returns (full_path_field, uncompressed_size_bytes).
/// Skips exactly five whitespace-delimited tokens, then treats the remainder as
/// the path — this correctly handles filenames that contain spaces.
fn parse_tar_verbose_line(line: &str) -> Option<(&str, u64)> {
    let mut rest = line;
    for _ in 0..5 {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if rest.is_empty() {
            return None;
        }
        let token_end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        rest = &rest[token_end..];
    }
    let path = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if path.is_empty() {
        return None;
    }
    // Size is the third whitespace-separated token (index 2).
    let size = line.split_ascii_whitespace().nth(2)?.parse::<u64>().ok()?;
    Some((path, size))
}

async fn available_bytes_at(path: &str) -> Option<u64> {
    let out = Command::new("df")
        .arg("-B1")
        .arg("--output=avail")
        .arg(path)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Output: "       Avail\n  12345678\n"
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .nth(1)?
        .trim()
        .parse::<u64>()
        .ok()
}

fn secure_tmp_path(prefix: &str, attempt: u32) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!(
        "/tmp/{prefix}-{nanos}-{}-{attempt}",
        std::process::id()
    ))
}

fn create_secure_temp_upload_file() -> io::Result<(PathBuf, tokio::fs::File)> {
    for attempt in 0..64 {
        let path = secure_tmp_path("saturn-upload", attempt);
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
                return Ok((path, tokio::fs::File::from_std(file)));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate unique Saturn upload temp file",
    ))
}

fn create_secure_temp_extract_dir() -> io::Result<PathBuf> {
    for attempt in 0..64 {
        let path = secure_tmp_path("saturn-restore", attempt);
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate unique Saturn restore temp directory",
    ))
}

async fn restore_backup_by_kind(state: &AppState, req: G2RestoreReq, kind: &str) -> Response {
    let (prefix, target_label) = match kind {
        "saturn" => ("saturn-backup-", "saturn"),
        "pihpsdr" => ("pihpsdr-backup-", "pihpsdr"),
        _ => return json_error(StatusCode::BAD_REQUEST, "invalid backup kind"),
    };
    let dry_run = req.dry_run.unwrap_or(false);
    if !dry_run && req.confirm.as_deref() != Some("RESTORE") {
        return json_error(StatusCode::BAD_REQUEST, "confirm token required");
    }

    let _activity_guard = if dry_run {
        None
    } else {
        match begin_update_activity(
            &format!("{target_label}-backup-restore"),
            format!("backup={}", req.backup_name),
        ) {
            Ok(g) => Some(g),
            Err(e) => return json_error(StatusCode::CONFLICT, &e),
        }
    };

    let backup_root = match resolve_backup_path(req.backup_name.trim(), prefix).await {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
    };

    let repo_root = if kind == "saturn" {
        if let Err(e) = validate_saturn_repo_root(&backup_root) {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("backup is not a Saturn repo snapshot: {e}"),
            );
        }
        let root = current_repo_root(state);
        if let Err(e) = validate_saturn_repo_root(&root) {
            return json_error(StatusCode::BAD_REQUEST, &e);
        }
        root
    } else {
        if let Err(e) = validate_pihpsdr_repo_root(&backup_root) {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("backup is not a piHPSDR repo snapshot: {e}"),
            );
        }
        let root = pihpsdr_repo_root();
        if let Err(e) = validate_pihpsdr_repo_root(&root) {
            return json_error(StatusCode::BAD_REQUEST, &e);
        }
        root
    };

    let (files, dirs, bytes) = tree_stats(backup_root.clone()).await;
    if dry_run {
        return Json(serde_json::json!({
            "status": "ok",
            "dry_run": true,
            "backup_root": backup_root,
            "repo_root": repo_root,
            "files": files,
            "dirs": dirs,
            "bytes": bytes,
        }))
        .into_response();
    }

    let status = match Command::new("rsync")
        .arg("-a")
        .arg("--delete")
        .arg(format!("{}/", backup_root.display()))
        .arg(format!("{}/", repo_root.display()))
        .status()
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to run rsync: {e}"),
            )
        }
    };
    if !status.success() {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "rsync failed");
    }

    Json(serde_json::json!({
        "status": "ok",
        "kind": target_label,
        "backup_root": backup_root,
        "repo_root": repo_root,
        "files": files,
        "dirs": dirs,
        "bytes": bytes,
    }))
    .into_response()
}

async fn g2_restore(State(state): State<AppState>, Json(req): Json<G2RestoreReq>) -> Response {
    restore_backup_by_kind(&state, req, "saturn").await
}

async fn pihpsdr_restore(State(state): State<AppState>, Json(req): Json<G2RestoreReq>) -> Response {
    restore_backup_by_kind(&state, req, "pihpsdr").await
}

async fn restore_full(
    State(state): State<AppState>,
    Query(q): Query<RestoreQuery>,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let dry_run = parse_boolish(q.dry_run);
    let _activity_guard = if dry_run {
        None
    } else {
        match begin_update_activity("saturn-full-restore", "full archive restore") {
            Ok(g) => Some(g),
            Err(e) => return Err(json_error(StatusCode::CONFLICT, &e)),
        }
    };
    let repo_root = current_repo_root(&state);

    let mut upload_path: Option<PathBuf> = None;
    let mut confirm: Option<String> = None;
    let mut upload_bytes = 0u64;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        if name == "confirm" {
            let text = field.text().await.unwrap_or_default();
            confirm = Some(text.trim().to_string());
            continue;
        }
        if name != "file" {
            continue;
        }

        let (path, mut file) = create_secure_temp_upload_file()
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        let mut field = field;
        while let Ok(Some(chunk)) = field.chunk().await {
            upload_bytes = upload_bytes.saturating_add(chunk.len() as u64);
            if upload_bytes > state.restore_max_upload_bytes {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(json_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    &format!(
                        "archive too large (limit {} MB)",
                        state.restore_max_upload_bytes / 1024 / 1024
                    ),
                ));
            }
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        }
        upload_path = Some(path);
    }

    let upload_path = match upload_path {
        Some(p) => p,
        None => return Err(json_error(StatusCode::BAD_REQUEST, "missing file")),
    };

    if !dry_run {
        if confirm.as_deref() != Some("RESTORE") {
            let _ = tokio::fs::remove_file(&upload_path).await;
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "confirm token required",
            ));
        }
    }

    if let Err(e) = validate_saturn_repo_root(&repo_root) {
        let _ = tokio::fs::remove_file(&upload_path).await;
        return Err(json_error(StatusCode::BAD_REQUEST, &e));
    }

    // Pre-validate tar contents: traversal paths, sparse-bomb expansion, and available space.
    // -tzvf gives verbose output with sizes so we can measure uncompressed bytes in one pass.
    let list_out = Command::new("tar")
        .arg("-tzvf")
        .arg(&upload_path)
        .output()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?;
    if !list_out.status.success() {
        let msg = String::from_utf8_lossy(&list_out.stderr).trim().to_string();
        let _ = tokio::fs::remove_file(&upload_path).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!("tar list failed: {msg}"),
        ));
    }
    let mut uncompressed_bytes: u64 = 0;
    for line in String::from_utf8_lossy(&list_out.stdout).lines() {
        let Some((path_field, size)) = parse_tar_verbose_line(line) else {
            continue;
        };
        // Symlink entries look like "linkname -> target" — check both sides.
        let (link_name, link_target) = match path_field.find(" -> ") {
            Some(pos) => (&path_field[..pos], Some(&path_field[pos + 4..])),
            None => (path_field, None),
        };
        if has_unsafe_path_component(link_name) {
            let _ = tokio::fs::remove_file(&upload_path).await;
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "archive contains unsafe paths",
            ));
        }
        if let Some(target) = link_target {
            if has_unsafe_path_component(target) {
                let _ = tokio::fs::remove_file(&upload_path).await;
                return Err(json_error(
                    StatusCode::BAD_REQUEST,
                    "archive contains unsafe symlink target",
                ));
            }
        }
        uncompressed_bytes = uncompressed_bytes.saturating_add(size);
    }
    // Sparse-archive bomb check: reject if expansion ratio exceeds the safety limit.
    if uncompressed_bytes > upload_bytes.saturating_mul(MAX_TAR_EXPANSION_FACTOR) {
        let _ = tokio::fs::remove_file(&upload_path).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "archive expansion ratio too large ({} MB uncompressed from {} MB compressed)",
                uncompressed_bytes / 1024 / 1024,
                upload_bytes / 1024 / 1024
            ),
        ));
    }
    // Available-space check: ensure /tmp has enough room to extract.
    if let Some(avail) = available_bytes_at("/tmp").await {
        if uncompressed_bytes > avail {
            let _ = tokio::fs::remove_file(&upload_path).await;
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!(
                    "insufficient space in /tmp: need {} MB, have {} MB",
                    uncompressed_bytes / 1024 / 1024,
                    avail / 1024 / 1024
                ),
            ));
        }
    }

    let extract_dir = create_secure_temp_extract_dir()
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&upload_path)
        .arg("-C")
        .arg(&extract_dir)
        .status()
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !status.success() {
        let _ = tokio::fs::remove_file(&upload_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(StatusCode::BAD_REQUEST, "extract failed"));
    }

    let mut entries = tokio::fs::read_dir(&extract_dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let mut top_dirs: Vec<PathBuf> = Vec::new();
    while let Ok(Some(ent)) = entries.next_entry().await {
        let path = ent.path();
        if path.is_dir() {
            top_dirs.push(path);
        }
    }
    if top_dirs.len() != 1 {
        let _ = tokio::fs::remove_file(&upload_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "archive must contain a single top-level directory",
        ));
    }
    let extracted_root = top_dirs.remove(0);
    if let Err(e) = validate_saturn_repo_root(&extracted_root) {
        let _ = tokio::fs::remove_file(&upload_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!("archive root is not a Saturn repo snapshot: {e}"),
        ));
    }

    if dry_run {
        let (files, dirs, bytes) = tree_stats(extracted_root.clone()).await;
        let _ = tokio::fs::remove_file(&upload_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        return Ok(Json(serde_json::json!({
            "status": "ok",
            "dry_run": true,
            "extracted_root": extracted_root,
            "files": files,
            "dirs": dirs,
            "bytes": bytes
        }))
        .into_response());
    }

    tokio::fs::create_dir_all(&repo_root)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if let Some(avail) = available_bytes_at(repo_root.to_string_lossy().as_ref()).await {
        if uncompressed_bytes > avail {
            let _ = tokio::fs::remove_file(&upload_path).await;
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!(
                    "insufficient space in repo target: need {} MB, have {} MB",
                    uncompressed_bytes / 1024 / 1024,
                    avail / 1024 / 1024
                ),
            ));
        }
    }

    let rsync_status = Command::new("rsync")
        .arg("-a")
        .arg("--delete")
        .arg(format!("{}/", extracted_root.display()))
        .arg(format!("{}/", repo_root.display()))
        .status()
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let _ = tokio::fs::remove_file(&upload_path).await;
    let _ = tokio::fs::remove_dir_all(&extract_dir).await;

    if !rsync_status.success() {
        return Err(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "rsync failed",
        ));
    }

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

#[derive(Debug, Deserialize)]
struct CustomScriptUpsertReq {
    filename: String,
    name: Option<String>,
    description: Option<String>,
    flags: Option<Vec<String>>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CustomScriptDeleteReq {
    filename: String,
    delete_file: Option<bool>,
}

async fn get_custom_scripts(State(state): State<AppState>) -> Response {
    match load_custom_scripts(&state).await {
        Ok(entries) => Json(serde_json::json!({ "scripts": entries })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn upsert_custom_script(
    State(state): State<AppState>,
    Json(req): Json<CustomScriptUpsertReq>,
) -> Response {
    let filename = req.filename.trim();
    if !is_safe_custom_script_filename(filename) {
        return json_error(StatusCode::BAD_REQUEST, "invalid filename");
    }

    let script_path = state.scripts_dir.join(filename);
    if let Some(content) = req.content.as_deref() {
        let normalized = content.replace("\r\n", "\n");
        if normalized.trim().is_empty() {
            return json_error(StatusCode::BAD_REQUEST, "content is empty");
        }
        if let Some(parent) = script_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("failed to create script dir: {e}"),
                );
            }
        }
        if let Err(e) = tokio::fs::write(&script_path, normalized).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to write script: {e}"),
            );
        }
        let _ =
            tokio::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).await;
    }

    let meta = match tokio::fs::metadata(&script_path).await {
        Ok(v) => v,
        Err(_) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "script file not found in scripts directory",
            )
        }
    };
    if !meta.is_file() {
        return json_error(StatusCode::BAD_REQUEST, "script path is not a file");
    }

    let mut scripts = match load_custom_scripts(&state).await {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    let entry = CfgEntry {
        filename: filename.to_string(),
        name: Some(req.name.as_deref().unwrap_or(filename).trim().to_string()),
        description: req.description.map(|s| s.trim().to_string()),
        directory: Some(state.scripts_dir.display().to_string()),
        category: Some("Custom Scripts".to_string()),
        flags: Some(sanitize_custom_flags(req.flags)),
        version: Some("custom".to_string()),
    };

    if let Some(existing) = scripts.iter_mut().find(|s| s.filename == filename) {
        *existing = entry.clone();
    } else {
        if scripts.len() >= MAX_CUSTOM_SCRIPTS {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("custom script limit ({MAX_CUSTOM_SCRIPTS}) reached"),
            );
        }
        scripts.push(entry.clone());
    }

    if let Err(e) = save_custom_scripts(&state, &scripts).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }

    Json(serde_json::json!({
        "status": "ok",
        "script": entry
    }))
    .into_response()
}

async fn delete_custom_script(
    State(state): State<AppState>,
    Json(req): Json<CustomScriptDeleteReq>,
) -> Response {
    let filename = req.filename.trim();
    if !is_safe_custom_script_filename(filename) {
        return json_error(StatusCode::BAD_REQUEST, "invalid filename");
    }

    let mut scripts = match load_custom_scripts(&state).await {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let before = scripts.len();
    scripts.retain(|s| s.filename != filename);
    if scripts.len() == before {
        return json_error(StatusCode::NOT_FOUND, "custom script not found");
    }
    if let Err(e) = save_custom_scripts(&state, &scripts).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }

    if req.delete_file.unwrap_or(false) {
        let path = state.scripts_dir.join(filename);
        let _ = tokio::fs::remove_file(path).await;
    }

    Json(serde_json::json!({ "status": "ok" })).into_response()
}

async fn get_scripts(State(state): State<AppState>) -> impl IntoResponse {
    let entries = read_all_script_entries(&state).await.unwrap_or_default();
    if entries.is_empty() {
        return Json(serde_json::json!({
            "scripts": {
                "System": [
                    { "filename":"echo-hello.sh", "name":"Echo Hello", "description":"Demo script" }
                ]
            },
            "warnings": ["config.json missing or invalid; showing demo"]
        }));
    }

    let mut grouped: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for e in entries {
        let cat = e.category.clone().unwrap_or_else(|| "Scripts".to_string());
        grouped.entry(cat).or_default().push(serde_json::json!({
            "filename": e.filename,
            "name": e.name.unwrap_or_default(),
            "description": e.description.unwrap_or_default(),
        }));
    }

    Json(serde_json::json!({ "scripts": grouped, "warnings": [] }))
}

async fn get_flags(
    State(state): State<AppState>,
    Query(q): Query<FlagsQuery>,
) -> impl IntoResponse {
    let script = q.script.unwrap_or_default();
    let entries = match read_all_script_entries(&state).await {
        Ok(v) => v,
        Err(_) => {
            return Json(serde_json::json!({
                "flags": [],
                "error": "config.json not found or invalid",
                "warning": "Using empty flags"
            }));
        }
    };

    for e in entries.into_iter().rev() {
        if e.filename == script {
            return Json(serde_json::json!({ "flags": e.flags.unwrap_or_default() }));
        }
    }
    Json(serde_json::json!({ "flags": [] }))
}

async fn get_fpga_images(State(state): State<AppState>) -> impl IntoResponse {
    fn list_images(dir: &Path) -> Vec<(String, u64)> {
        let mut images: Vec<(String, u64)> = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("bin") {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            let modified_epoch = entry
                                .metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            images.push((name.to_string(), modified_epoch));
                        }
                    }
                }
            }
        }
        images.sort_by(|a, b| a.0.cmp(&b.0));
        images
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("SATURN_FPGA_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(root) = std::env::var("SATURN_ACTIVE_REPO_ROOT") {
        candidates.push(PathBuf::from(root).join("FPGA"));
    }
    if let Ok(root) = std::env::var("SATURN_REPO_ROOT") {
        candidates.push(PathBuf::from(root).join("FPGA"));
    }
    candidates.push(current_repo_root(&state).join("FPGA"));

    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join("github/Saturn/FPGA"));
        candidates.push(PathBuf::from(&home).join("github/saturn/FPGA"));
    }

    if let Ok(home_entries) = fs::read_dir("/home") {
        for entry in home_entries.flatten() {
            candidates.push(entry.path().join("github/Saturn/FPGA"));
            candidates.push(entry.path().join("github/saturn/FPGA"));
        }
    }

    candidates.push(PathBuf::from("/opt/saturn-go/FPGA"));

    let mut selected: Option<PathBuf> = None;
    let mut images: Vec<String> = Vec::new();
    let mut latest_image: Option<String> = None;
    let mut checked: Vec<String> = Vec::new();

    for dir in candidates {
        let dir_str = dir.to_string_lossy().to_string();
        if checked.iter().any(|d| d == &dir_str) {
            continue;
        }
        checked.push(dir_str);
        if dir.is_dir() {
            let listed = list_images(&dir);
            let listed_names: Vec<String> = listed.iter().map(|(name, _)| name.clone()).collect();
            let listed_latest = listed
                .iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
                .map(|(name, _)| name.clone());
            if selected.is_none() {
                selected = Some(dir.clone());
                images = listed_names.clone();
                latest_image = listed_latest.clone();
            }
            if !listed.is_empty() {
                selected = Some(dir);
                images = listed_names;
                latest_image = listed_latest;
                break;
            }
        }
    }

    let warning = if selected.is_none() {
        Some(
            "No FPGA directory found (set SATURN_FPGA_DIR or place images in ~/github/Saturn/FPGA)"
                .to_string(),
        )
    } else if images.is_empty() {
        Some("FPGA directory found but no .bin images were found".to_string())
    } else {
        None
    };

    Json(serde_json::json!({
        "dir": selected,
        "images": images,
        "latest_image": latest_image,
        "checked": checked,
        "warning": warning
    }))
}

async fn get_run_log(Query(q): Query<RunLogQuery>) -> Response {
    let script = q.script.unwrap_or_default();
    if !is_safe_script_name(&script) {
        return json_error(StatusCode::BAD_REQUEST, "invalid script");
    }
    let from = q.from.unwrap_or(0);
    let limit = q.limit.unwrap_or(300).clamp(1, RUN_LOG_FETCH_MAX_LINES);

    let guard = script_run_log_slot().lock_unpoisoned();
    let Some(run) = guard.get(&script) else {
        return Json(serde_json::json!({
            "script": script,
            "run_id": serde_json::Value::Null,
            "status": "idle",
            "running": false,
            "started_at": serde_json::Value::Null,
            "finished_at": serde_json::Value::Null,
            "from": from,
            "next_from": from,
            "total_lines": 0,
            "lines": Vec::<String>::new(),
        }))
        .into_response();
    };

    let total = run.line_offset + run.lines.len();
    let start = from.max(run.line_offset).min(total);
    let start_idx = start.saturating_sub(run.line_offset);
    let end_idx = (start_idx + limit).min(run.lines.len());
    let end = run.line_offset + end_idx;
    let lines = run.lines[start_idx..end_idx].to_vec();
    Json(serde_json::json!({
        "script": script,
        "run_id": run.run_id,
        "status": run.status,
        "running": run.status == "running",
        "started_at": run.started_at,
        "finished_at": run.finished_at,
        "from": start,
        "next_from": end,
        "total_lines": total,
        "lines": lines,
    }))
    .into_response()
}

async fn run_sse(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Response, Response> {
    let (script, flags) = match parse_multipart(multipart).await {
        Ok(v) => v,
        Err(resp) => return Err(resp),
    };

    if !is_safe_script_name(&script) {
        return Err((StatusCode::BAD_REQUEST, "invalid script").into_response());
    }

    let script_path = state.scripts_dir.join(&script);
    if tokio::fs::metadata(&script_path).await.is_err() {
        return Err((StatusCode::NOT_FOUND, "script not found").into_response());
    }
    let repo_root = current_repo_root(&state);
    let script_is_python = script_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("py"))
        .unwrap_or(false);
    if script_is_python {
        let script_resolved = tokio::fs::canonicalize(&script_path)
            .await
            .unwrap_or_else(|_| script_path.clone());
        let repo_root_resolved = tokio::fs::canonicalize(&repo_root)
            .await
            .unwrap_or_else(|_| repo_root.clone());
        if script_resolved.starts_with(&repo_root_resolved) {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "Refusing to execute Python scripts from repo tree. Use installed scripts in /opt/saturn-go/scripts.",
            ));
        }
    }
    let repo_root_display = repo_root.display().to_string();
    let g2_script = is_g2_update_script(&script);
    let saturngo_script = is_saturngo_update_script(&script);
    let saturngo_skip_git = saturngo_script && has_flag(&flags, "--skip-git");
    let g2_policy = if g2_script {
        let policy = load_update_policy(&state)
            .await
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
        if !update_policy_repo_configured(&policy) {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "Appliance Update repo URL is not configured. Save a GitHub repo URL first.",
            ));
        }
        Some(policy)
    } else {
        None
    };
    let saturngo_policy = if saturngo_script {
        let policy = load_saturngo_update_policy(&state)
            .await
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
        if !saturngo_skip_git && !update_policy_repo_configured(&policy) {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "Saturn Go repo URL is not configured. Save a GitHub repo URL first.",
            ));
        }
        if update_policy_repo_configured(&policy) {
            Some(policy)
        } else {
            None
        }
    } else {
        None
    };
    let update_activity_guard = if g2_script || saturngo_script {
        let kind = if g2_script {
            "update-g2"
        } else {
            "saturngo-update"
        };
        Some(
            begin_update_activity(
                kind,
                format!("script={} repo_root={repo_root_display}", script),
            )
            .map_err(|e| json_error(StatusCode::CONFLICT, &e))?,
        )
    } else {
        None
    };

    let (tx, rx) = mpsc::unbounded_channel::<String>();

    let (run_id, start_line) = begin_script_run_log(&script, &flags);
    let _ = tx.send(start_line);

    let mut cmd = build_script_command(&script_path, &flags);
    cmd.env("SATURN_REPO_ROOT", &repo_root_display);
    cmd.env("SATURN_DIR", &repo_root_display);
    cmd.env("SATURN_ACTIVE_REPO_ROOT", &repo_root_display);
    if let Some(policy) = &g2_policy {
        let target_ref = policy
            .custom_ref
            .clone()
            .unwrap_or_else(|| policy.stable_ref.clone());
        cmd.env("SATURN_UPDATE_POLICY_OWNER", policy.owner.trim());
        cmd.env("SATURN_UPDATE_POLICY_REPO", policy.repo.trim());
        cmd.env("SATURN_UPDATE_POLICY_REMOTE", policy.remote.trim());
        cmd.env("SATURN_UPDATE_POLICY_REF", target_ref.trim());
        cmd.env("SATURN_UPDATE_POLICY_URL", expected_remote_url(policy));
    }
    if let Some(policy) = &saturngo_policy {
        let target_ref = policy
            .custom_ref
            .clone()
            .unwrap_or_else(|| policy.stable_ref.clone());
        cmd.env("SATURN_SATURNGO_POLICY_OWNER", policy.owner.trim());
        cmd.env("SATURN_SATURNGO_POLICY_REPO", policy.repo.trim());
        cmd.env("SATURN_SATURNGO_POLICY_REMOTE", policy.remote.trim());
        cmd.env("SATURN_SATURNGO_POLICY_REF", target_ref.trim());
        cmd.env("SATURN_SATURNGO_POLICY_URL", expected_remote_url(policy));
        cmd.env(
            "SATURN_SATURNGO_DEPLOY_STATUS_FILE",
            state.saturngo_deploy_status_file.display().to_string(),
        );
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Error: {e}");
            append_script_run_log_line(&script, &run_id, msg);
            finish_script_run_log(&script, &run_id, "error");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
        }
    };

    if let Some(stdout) = child.stdout.take() {
        let tx_out = tx.clone();
        let script_out = script.clone();
        let run_id_out = run_id.clone();
        let line_sink: RunLineSink = Arc::new(move |line: String| {
            append_script_run_log_line(&script_out, &run_id_out, line);
        });
        tokio::spawn(async move {
            stream_process_output(stdout, tx_out, "", Some(line_sink)).await;
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx_err = tx.clone();
        let script_err = script.clone();
        let run_id_err = run_id.clone();
        let line_sink: RunLineSink = Arc::new(move |line: String| {
            append_script_run_log_line(&script_err, &run_id_err, line);
        });
        tokio::spawn(async move {
            stream_process_output(stderr, tx_err, "ERR: ", Some(line_sink)).await;
        });
    }

    let script_wait = script.clone();
    let run_id_wait = run_id.clone();
    tokio::spawn(async move {
        let _update_activity_guard = update_activity_guard;
        match child.wait().await {
            Ok(status) if status.success() => {
                let line = "Done".to_string();
                let _ = tx.send(line.clone());
                append_script_run_log_line(&script_wait, &run_id_wait, line);
                finish_script_run_log(&script_wait, &run_id_wait, "done");
            }
            Ok(status) => {
                let line = format!("Error: {status}");
                let _ = tx.send(line.clone());
                append_script_run_log_line(&script_wait, &run_id_wait, line);
                finish_script_run_log(&script_wait, &run_id_wait, "error");
            }
            Err(e) => {
                let line = format!("Error: {e}");
                let _ = tx.send(line.clone());
                append_script_run_log_line(&script_wait, &run_id_wait, line);
                finish_script_run_log(&script_wait, &run_id_wait, "error");
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx)
        .map(|line| Ok::<Event, std::convert::Infallible>(Event::default().data(line)));
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5)));
    let mut resp = sse.into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(resp)
}

async fn parse_multipart(mut multipart: Multipart) -> Result<(String, Vec<String>), Response> {
    let mut script = String::new();
    let mut flags = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        let text = field.text().await.unwrap_or_default();
        if name == "script" {
            script = text;
        } else if name == "flags" {
            flags.push(text);
        }
    }

    if script.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing script").into_response());
    }

    Ok((script, flags))
}

async fn no_content() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::bind_addr_is_loopback;

    #[test]
    fn bind_addr_loopback_guard_accepts_only_local_hosts() {
        assert!(bind_addr_is_loopback("127.0.0.1:8080"));
        assert!(bind_addr_is_loopback("127.42.0.1:8080"));
        assert!(bind_addr_is_loopback("[::1]:8080"));
        assert!(bind_addr_is_loopback("localhost:8080"));

        assert!(!bind_addr_is_loopback("0.0.0.0:8080"));
        assert!(!bind_addr_is_loopback("192.168.0.139:8080"));
        assert!(!bind_addr_is_loopback("[::]:8080"));
    }
}
