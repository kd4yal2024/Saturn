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
