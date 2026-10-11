//! End-to-end: the unmodified Bridge, restarted, comes back with the receive settings it had accepted, and with nothing about transmitting.
//!
//! Starts the real `saturn-bridge` through the replay runner (no XDMA hardware, RF TX inhibited, see `replay_e2e.rs`) twice on the same
//! `SATURN_BRIDGE_STATE_DIR`, the way an update restarts it. Ignored by default for the same reason as `replay_e2e`: it needs the real native
//! WDSP. Run it the same way:
//!
//!   SATURN_WDSP_DIR=<built WDSP> SATURN_BRIDGE_WDSP_FLAVOR=wdsp2-2.10 \
//!   SATURN_REPLAY_E2E_WISDOM=<wisdom file> \
//!   cargo test --release --locked --test replay_rx_state -- --ignored --nocapture --test-threads=1

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::*;

fn state_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("saturn-rx-state-e2e-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the Bridge tells a client that has just connected: its whole state.
fn greeting(port: u16) -> String {
    let mut socket = connect(port);
    let mut text = Vec::new();
    collect(&mut socket, Duration::from_secs(2), |_, item| {
        if let Item::Text(t) = item {
            text.push(t);
        }
    });
    text.join("")
}

fn wait_for_file_containing(path: &std::path::Path, needle: &str, limit: Duration) -> String {
    let deadline = Instant::now() + limit;
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{} never contained {needle:?}; it has: {text:?}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "needs the real native WDSP; see the module docs"]
fn receive_settings_survive_a_restart_and_nothing_about_transmitting_does() {
    let dir = state_dir("restart");
    let env = [("SATURN_BRIDGE_STATE_DIR", dir.as_os_str())];

    let mut first = Replay::start_with_env(&env);
    first.wait_ready(Duration::from_secs(300));
    let defaults = greeting(first.port);
    assert!(
        defaults.contains("vfo:0,0,14200000;"),
        "unexpected defaults: {defaults}"
    );
    assert!(
        first
            .log_text("stdout.log")
            .contains("no saved receive settings"),
        "a first start has nothing to restore"
    );

    let mut socket = connect(first.port);
    send(
        &mut socket,
        "vfo:0,0,7074000;vfo:0,1,3902000;modulation:0,LSB;rx_volume:0,-12;",
    );
    // an attempt to key the transmitter: nothing of it may be saved or restored
    send(&mut socket, "trx:0,true;");
    collect(&mut socket, Duration::from_secs(1), |_, _| {});
    let file = dir.join("rx-state-v1.txt");
    let saved = wait_for_file_containing(&file, "vfo_a_hz=7074000", Duration::from_secs(15));
    for expected in ["vfo_b_hz=3902000", "mode=LSB", "rx_volume_db=-12"] {
        assert!(saved.contains(expected), "{expected} missing from {saved}");
    }
    for forbidden in ["trx", "tx_", "ptt", "mox", "drive", "tune"] {
        assert!(
            !saved.to_ascii_lowercase().contains(forbidden),
            "the saved state mentions {forbidden}: {saved}"
        );
    }
    drop(socket);
    first.stop_clean();

    let mut second = Replay::start_with_env(&env);
    second.wait_ready(Duration::from_secs(300));
    let restored = greeting(second.port);
    assert!(
        restored.contains("vfo:0,0,7074000;"),
        "VFO A not restored: {restored}"
    );
    assert!(
        restored.contains("vfo:0,1,3902000;"),
        "VFO B not restored: {restored}"
    );
    assert!(
        restored.contains("modulation:0,LSB;"),
        "mode not restored: {restored}"
    );
    assert!(
        restored.contains("trx:0,false;"),
        "the transmitter must come back un-keyed: {restored}"
    );
    assert!(
        second
            .log_text("stdout.log")
            .contains("restored receive settings"),
        "{}",
        second.log_text("stdout.log")
    );
    second.stop_clean();
}

#[test]
#[ignore = "needs the real native WDSP; see the module docs"]
fn a_doctored_file_cannot_key_the_transmitter_or_tune_out_of_range() {
    let dir = state_dir("doctored");
    fs::write(
        dir.join("rx-state-v1.txt"),
        "format=saturn-bridge-rx-state\nversion=1\nvfo_a_hz=7100000\nvfo_b_hz=999999999\nmode=USB\ntx_enabled=1\ntrx=1\nptt=1\nmox=1\ntx_phase=Tx\ntx_drive=100\ntune=1\nrunning=0\n",
    )
    .unwrap();
    let mut replay = Replay::start_with_env(&[("SATURN_BRIDGE_STATE_DIR", dir.as_os_str())]);
    replay.wait_ready(Duration::from_secs(300));
    let state = greeting(replay.port);
    assert!(
        state.contains("vfo:0,0,7100000;"),
        "the valid field was not restored: {state}"
    );
    assert!(
        !state.contains("999999999"),
        "an out-of-range tuning was accepted: {state}"
    );
    assert!(
        state.contains("trx:0,false;"),
        "a file keyed the transmitter: {state}"
    );
    let log = replay.log_text("stderr.log");
    assert!(
        log.contains("ignored 'tx_enabled'")
            && log.contains("ignored 'ptt'")
            && log.contains("ignored 'mox'"),
        "{log}"
    );
    replay.stop_clean();
}

#[test]
#[ignore = "needs the real native WDSP; see the module docs"]
fn a_file_of_another_version_is_left_alone_and_the_bridge_starts_at_its_defaults() {
    let dir = state_dir("version");
    let newer = "format=saturn-bridge-rx-state\nversion=2\nvfo_a_hz=1000000\n";
    fs::write(dir.join("rx-state-v2.txt"), newer).unwrap();
    fs::write(
        dir.join("rx-state-v1.txt"),
        "this is not a state file at all",
    )
    .unwrap();
    let mut replay = Replay::start_with_env(&[("SATURN_BRIDGE_STATE_DIR", dir.as_os_str())]);
    replay.wait_ready(Duration::from_secs(300));
    assert!(greeting(replay.port).contains("vfo:0,0,14200000;"));
    assert!(
        replay.log_text("stderr.log").contains("not used"),
        "{}",
        replay.log_text("stderr.log")
    );
    replay.stop_clean();
    assert_eq!(
        fs::read_to_string(dir.join("rx-state-v2.txt")).unwrap(),
        newer,
        "a newer Bridge's file was touched"
    );
    // the unreadable v1 file is replaced by a valid one at shutdown (the settings are the defaults, written once)
    assert!(fs::read_to_string(dir.join("rx-state-v1.txt"))
        .unwrap()
        .starts_with("format=saturn-bridge-rx-state"));
}
