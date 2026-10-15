//! End-to-end: the role a client is last told is the role it holds, even when it declares a viewer session the instant its socket opens.
//!
//! Starts the real `saturn-bridge` through the replay runner (no XDMA hardware, RF TX inhibited, see `replay_e2e.rs`) and connects to its control path the way the
//! receive-only validation's browser does: the handshake, then `session_open ... viewer` at once, before the connection thread has had a chance to write its
//! snapshot. The Bridge registers the first client as the operator, announces that in the snapshot, and releases the slot when it reads the viewer declaration
//! (`release_operator_for_viewer_session`). A release notice that is written BEFORE the snapshot's older `operator` announcement leaves the client holding a role
//! the Bridge has already taken away, which failed three of the ten receive validations the G2 recorded on 2026-10-14 (a target at 06:15, then the target and the rollback of the 19:46 update).
//!
//! Needs only the stub native layer (no real WDSP: it looks at roles, not audio), so it runs in CI. Competing busy threads make the connection thread late, as a
//! busy host does; the unfixed Bridge loses the race a measurable fraction of the time, the fixed one never does.
//!
//!   SATURN_BRIDGE_STUB_NATIVE=1 cargo test --locked --test replay_role_order -- --nocapture

mod common;

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

/// The `remote_client_role:0,<role>,<id>;` notices in a stream of TCI text, in the order they arrived.
fn role_notices(text: &str) -> Vec<String> {
    text.split(';')
        .map(str::trim)
        .filter(|command| command.starts_with("remote_client_role:"))
        .map(|command| command.rsplit(',').nth(1).unwrap_or("?").to_string())
        .collect()
}

/// The Bridge locks its receive buffers in memory and does not start when it cannot ("could not lock aligned XDMA buffer in memory"); its service unit is
/// given `LimitMEMLOCK=16M` for that. A CI runner's default is 8 MiB or less, and a developer's shell may be lower.
const BRIDGE_MEMLOCK_BYTES: libc::rlim_t = 16 * 1024 * 1024;

/// True when the Bridge this test starts can lock its buffers: the soft limit is raised to the requirement where the hard limit allows it (the runner inherits it).
fn memlock_is_enough() -> bool {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit for the duration of the call.
    if unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut limit) } != 0 {
        return false;
    }
    if limit.rlim_cur >= BRIDGE_MEMLOCK_BYTES {
        return true;
    }
    if limit.rlim_max < BRIDGE_MEMLOCK_BYTES {
        return false;
    }
    limit.rlim_cur = BRIDGE_MEMLOCK_BYTES;
    // SAFETY: `limit` is a valid rlimit whose soft limit does not exceed its hard limit.
    unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) == 0 }
}

fn connect_control(port: u16) -> Socket {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the Bridge");
    let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/control"), stream)
        .expect("websocket handshake");
    socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    socket
}

/// One session: connect, declare a viewer session at once, and return the roles the Bridge told this client, in order.
fn viewer_session_roles(port: u16, index: usize) -> Vec<String> {
    let mut socket = connect_control(port);
    send(
        &mut socket,
        &format!("session_open:role-order-{index},viewer;"),
    );
    let mut text = String::new();
    collect(&mut socket, Duration::from_millis(200), |_, item| {
        if let Item::Text(t) = item {
            text.push_str(&t);
        }
    });
    let _ = socket.close(None);
    role_notices(&text)
}

#[test]
fn a_viewer_session_declared_at_once_is_last_told_viewer() {
    // The replay runner never runs next to a real radio (it would conflict with the live Bridge), and the release build's test gate runs on the appliance
    // itself: there this test steps aside, and the unit tests above carry the rule.
    if std::path::Path::new("/dev/xdma0_user").exists() {
        eprintln!("skipped: /dev/xdma0_user exists, so this is a radio host and the replay runner refuses to start here");
        return;
    }
    if !memlock_is_enough() {
        // CI must run this test (its workflow step lifts the limit); a developer's shell with a low hard limit may step aside, loudly.
        let message = format!(
            "the locked-memory limit (RLIMIT_MEMLOCK) is below the {} MiB the Bridge needs to start, and its hard limit does not allow raising it; \
             run `ulimit -l 16384` first, or start the shell with a higher limit",
            BRIDGE_MEMLOCK_BYTES / (1024 * 1024)
        );
        assert!(std::env::var_os("CI").is_none(), "{message}");
        eprintln!("skipped: {message}");
        return;
    }
    let mut replay = Replay::start();
    replay.wait_ready(Duration::from_secs(120));

    let stop = Arc::new(AtomicBool::new(false));
    let busy: Vec<_> = (0..std::thread::available_parallelism()
        .map(|n| n.get() * 2)
        .unwrap_or(8))
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut x = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    x = x
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    std::hint::black_box(x);
                }
            })
        })
        .collect();

    let sessions = 60;
    let started = Instant::now();
    let mut stale = Vec::new();
    for index in 0..sessions {
        let roles = viewer_session_roles(replay.port, index);
        assert!(
            !roles.is_empty(),
            "session {index}: the Bridge told the client no role at all"
        );
        if roles.last().map(String::as_str) != Some("viewer") {
            stale.push((index, roles));
        }
    }
    stop.store(true, Ordering::Relaxed);
    for thread in busy {
        let _ = thread.join();
    }
    eprintln!(
        "{sessions} viewer sessions in {:.1} s, {} left holding a stale role: {:?}",
        started.elapsed().as_secs_f64(),
        stale.len(),
        stale.iter().take(3).collect::<Vec<_>>()
    );
    assert!(
        stale.is_empty(),
        "{} of {sessions} viewer sessions ended up told a role other than viewer last (session, roles in arrival order): {:?}",
        stale.len(),
        stale.iter().take(5).collect::<Vec<_>>()
    );
}
