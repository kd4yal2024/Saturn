//! The replay harness's own checks: how it judges a shutdown, and that it
//! neither blocks on a child's output nor throws away evidence. No Bridge, no
//! WDSP and no radio: the children here are small shell scripts.

mod common;

use std::os::unix::process::ExitStatusExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use common::*;

const LONG: Duration = Duration::from_secs(20);

fn shell(script: &str) -> Replay {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    Replay::spawn_command(command)
}

fn status(raw: i32) -> ExitStatus {
    ExitStatus::from_raw(raw)
}

#[test]
fn only_a_requested_clean_shutdown_passes() {
    // Linux wait statuses: exit code in bits 8..15, terminating signal in the low bits.
    assert!(check_stop(&Stop::Stopped(status(0))).is_ok());

    for (name, raw) in [
        ("nonzero exit", 23 << 8),
        ("killed by SIGTERM (no handler)", 15),
        ("SIGSEGV", 11),
        ("SIGABRT", 6),
        ("SIGKILL", 9),
        ("SIGSEGV with a core dump", 11 | 0x80),
    ] {
        let verdict = check_stop(&Stop::Stopped(status(raw)));
        assert!(verdict.is_err(), "{name} was accepted as a clean shutdown");
    }
    for (name, raw) in [("exit 0", 0), ("SIGSEGV", 11), ("exit 23", 23 << 8)] {
        let verdict = check_stop(&Stop::ExitedBeforeStop(status(raw)));
        assert!(verdict.is_err(), "an exit before the stop request ({name}) was accepted");
    }
    let message = check_stop(&Stop::Stopped(status(11 | 0x80))).unwrap_err();
    assert!(message.contains("signal 11") && message.contains("core dumped"), "{message}");
}

#[test]
fn a_child_that_handles_sigterm_and_exits_zero_is_a_clean_shutdown() {
    let mut replay = shell("trap 'exit 0' TERM; echo ready; while :; do sleep 0.05; done");
    assert!(replay.wait_for_log("stdout.log", "ready", LONG), "the child never became ready");
    let stop = replay.stop();
    assert!(matches!(stop, Stop::Stopped(_)), "{stop:?}");
    assert!(check_stop(&stop).is_ok(), "{stop:?}");
}

#[test]
fn a_child_that_dies_of_sigterm_or_exits_nonzero_is_not_clean() {
    let mut dies = shell("echo ready; while :; do sleep 0.05; done");
    assert!(dies.wait_for_log("stdout.log", "ready", LONG));
    let stop = dies.stop();
    assert!(matches!(stop, Stop::Stopped(_)));
    assert!(check_stop(&stop).is_err(), "death by SIGTERM was accepted: {stop:?}");

    let mut nonzero = shell("trap 'exit 3' TERM; echo ready; while :; do sleep 0.05; done");
    assert!(nonzero.wait_for_log("stdout.log", "ready", LONG));
    let stop = nonzero.stop();
    assert!(check_stop(&stop).is_err(), "exit 3 after SIGTERM was accepted: {stop:?}");
}

#[test]
fn a_crash_before_the_stop_request_is_reported_as_such() {
    for script in ["kill -SEGV $$", "kill -ABRT $$", "kill -KILL $$", "exit 23", "exit 0"] {
        let mut replay = shell(script);
        assert!(replay.wait_for_exit(LONG).is_some(), "{script}: the child did not exit");
        let stop = replay.stop();
        assert!(matches!(stop, Stop::ExitedBeforeStop(_)), "{script}: {stop:?}");
        assert!(check_stop(&stop).is_err(), "{script}: accepted");
    }
}

#[test]
fn stop_clean_fails_the_test_and_keeps_the_evidence() {
    let mut replay = shell("echo about-to-crash >&2; kill -SEGV $$");
    assert!(replay.wait_for_exit(LONG).is_some());
    let root = replay.root().to_path_buf();
    let outcome = catch_unwind(AssertUnwindSafe(|| replay.stop_clean()));
    assert!(outcome.is_err(), "stop_clean accepted a crashed child");
    drop(replay);
    assert!(root.join("logs/stderr.log").exists(), "the log was deleted with the failure");
    let log = std::fs::read_to_string(root.join("logs/stderr.log")).unwrap();
    assert!(log.contains("about-to-crash"), "{log:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_verbose_child_is_never_blocked_by_its_output() {
    // 8 MiB on each stream: far past a 64 KiB pipe buffer. With undrained pipes
    // this child would block in write(2) and never exit.
    let mut replay = shell(
        "head -c 8388608 /dev/zero | tr '\\0' 'x' ; head -c 8388608 /dev/zero | tr '\\0' 'y' >&2",
    );
    let status = replay.wait_for_exit(LONG).expect("the verbose child blocked on its output");
    assert!(status.success(), "{status}");
    let size = |name: &str| std::fs::metadata(replay.log_path(name)).unwrap().len();
    assert_eq!(size("stdout.log"), 8 * 1024 * 1024);
    assert_eq!(size("stderr.log"), 8 * 1024 * 1024);
}

#[test]
fn evidence_is_removed_after_a_pass_and_kept_when_asked() {
    let mut passed = shell("exit 0");
    assert!(passed.wait_for_exit(LONG).is_some());
    let root = passed.root().to_path_buf();
    drop(passed);
    assert!(!root.exists(), "a passing run left its directory behind");

    let mut kept = shell("echo measured");
    assert!(kept.wait_for_exit(LONG).is_some());
    kept.keep();
    let root = kept.root().to_path_buf();
    drop(kept);
    assert!(root.join("logs/stdout.log").exists(), "keep() did not keep the logs");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn every_run_gets_its_own_directory() {
    let first = shell("exit 0");
    let second = shell("exit 0");
    assert_ne!(first.root(), second.root());
}

// ---- the records a published comparison rests on are required, not best effort

use std::path::{Path, PathBuf};

/// A unique scratch directory, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("saturn-records-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn results(&self) -> PathBuf {
        self.0.join("results")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PERF: &[u8] = b"{\n  \"metrics\": {\n    \"ddc_s\": 844.0\n  }\n}\n";
/// SHA-256 of the three bytes "abc" (FIPS 180-2 test vector).
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const INTERVALS: &[f64] = &[21.3, 21.4, 42.9];

fn save(scratch: &Scratch, perf: &Path, bridge: &Path, table: &str, intervals: &[(&str, &[f64])]) -> Result<(), String> {
    save_required_records(&scratch.results(), perf, bridge, "settings\n", table, intervals)
}

#[test]
fn all_required_records_are_saved_with_the_right_hash() {
    let scratch = Scratch::new();
    let perf = scratch.file("perf.json", PERF);
    let bridge = scratch.file("saturn-bridge", b"abc");
    save(&scratch, &perf, &bridge, "table\n", &[("A1", INTERVALS), ("B", INTERVALS)]).unwrap();
    let results = scratch.results();
    assert_eq!(std::fs::read(results.join("perf-final.json")).unwrap(), PERF);
    let info = std::fs::read_to_string(results.join("run-info.txt")).unwrap();
    assert!(info.contains(&format!("bridge sha256: {ABC_SHA256}")), "{info}");
    assert!(info.contains("settings"), "{info}");
    assert_eq!(std::fs::read_to_string(results.join("transport-screen.txt")).unwrap(), "table\n");
    let rows = std::fs::read_to_string(results.join("arm-B-audio-intervals-ms.txt")).unwrap();
    assert_eq!(rows, "21.300\n21.400\n42.900\n");
}

#[test]
fn a_missing_final_perf_json_is_an_error_not_a_skipped_record() {
    let scratch = Scratch::new();
    let bridge = scratch.file("saturn-bridge", b"abc");
    let missing = scratch.0.join("perf.json");
    let why = save(&scratch, &missing, &bridge, "table\n", &[("A1", INTERVALS)]).unwrap_err();
    assert!(why.contains("perf.json") && why.contains("required"), "{why}");
    assert!(!scratch.results().join("perf-final.json").exists());
}

#[test]
fn an_empty_or_foreign_perf_json_is_an_error() {
    let scratch = Scratch::new();
    let bridge = scratch.file("saturn-bridge", b"abc");
    let empty = scratch.file("empty.json", b"");
    assert!(save(&scratch, &empty, &bridge, "t\n", &[("A1", INTERVALS)]).unwrap_err().contains("perf"));
    let foreign = scratch.file("foreign.json", b"{\"hello\": 1}\n");
    let why = save(&scratch, &foreign, &bridge, "t\n", &[("A1", INTERVALS)]).unwrap_err();
    assert!(why.contains("not a perf document"), "{why}");
}

#[test]
fn a_missing_or_empty_bridge_executable_is_an_error_not_a_blank_hash() {
    let scratch = Scratch::new();
    let perf = scratch.file("perf.json", PERF);
    let missing = scratch.0.join("no-such-bridge");
    let why = save(&scratch, &perf, &missing, "t\n", &[("A1", INTERVALS)]).unwrap_err();
    assert!(why.contains("Bridge executable hash is required"), "{why}");
    let empty = scratch.file("empty-bridge", b"");
    let why = save(&scratch, &perf, &empty, "t\n", &[("A1", INTERVALS)]).unwrap_err();
    assert!(why.contains("Bridge executable hash is required") && why.contains("empty"), "{why}");
    // Neither failure may leave a run-info that looks complete.
    assert!(!scratch.results().join("run-info.txt").exists());
}

#[test]
fn an_empty_table_or_an_arm_with_no_intervals_is_an_error() {
    let scratch = Scratch::new();
    let perf = scratch.file("perf.json", PERF);
    let bridge = scratch.file("saturn-bridge", b"abc");
    let why = save(&scratch, &perf, &bridge, "", &[("A1", INTERVALS)]).unwrap_err();
    assert!(why.contains("transport-screen.txt") && why.contains("empty"), "{why}");
    let why = save(&scratch, &perf, &bridge, "t\n", &[("A1", INTERVALS), ("C", &[])]).unwrap_err();
    assert!(why.contains("arm C") && why.contains("no audio intervals"), "{why}");
}

#[test]
fn an_unwritable_results_directory_is_an_error() {
    let scratch = Scratch::new();
    let perf = scratch.file("perf.json", PERF);
    let bridge = scratch.file("saturn-bridge", b"abc");
    let blocker = scratch.file("a-file", b"x");
    let why = save_required_records(&blocker.join("results"), &perf, &bridge, "i\n", "t\n", &[("A1", INTERVALS)])
        .unwrap_err();
    assert!(why.contains("cannot create"), "{why}");
}
