#!/usr/bin/env python3
"""Stand-in for saturn-remote-validate.py with the same command-line contract the activation helper relies on.

run   --control-dir DIR --target-commit C ...   attaches a "receive client" (a marker file the fake owner reports as sdr_active), reports ready, and
                                                holds until `stop`, then detaches. Scenarios come from the environment:
                                                  SATURN_TEST_VALIDATOR_FAIL_COMMITS   commits for which reception cannot be established
                                                  SATURN_TEST_VALIDATOR_HANG_COMMITS   commits for which it never becomes ready
                                                  SATURN_TEST_VALIDATOR_STOP_FAIL_COMMITS  commits whose cleanup fails
                                                  SATURN_TEST_VALIDATOR_STALL_COMMITS  commits whose session attaches (the owner reports a receive
                                                                                       client) but whose audio never progresses
wait  --control-dir DIR [--timeout S]           exit 0 on ready; 1 on failure; 3 on timeout; prints one JSON line
stop  --control-dir DIR [--timeout S]           ends the session; exit 0 only if it finished cleanly; prints one JSON line
"""
import json
import os
import sys
import time

MARKER = os.environ.get("SATURN_TEST_CLIENT_MARKER", "")
SHARED_LOG = os.environ.get("SATURN_TEST_SYSTEMCTL_LOG", "")
ARGS_LOG = os.environ.get("SATURN_TEST_VALIDATOR_ARGS_LOG", "")


def note(line):
    if SHARED_LOG:
        with open(SHARED_LOG, "a") as handle:
            handle.write(line + "\n")


def listed(name, commit):
    return commit in os.environ.get(name, "").split()


def option(argv, flag, default=None):
    return argv[argv.index(flag) + 1] if flag in argv else default


def write_status(directory, **fields):
    path = os.path.join(directory, "status.json")
    try:
        with open(path) as handle:
            status = json.load(handle)
    except (OSError, ValueError):
        status = {"conditions": {}}
    status.update(fields)
    temporary = path + ".tmp"
    with open(temporary, "w") as handle:
        json.dump(status, handle)
    os.replace(temporary, path)


def read_status(directory):
    try:
        with open(os.path.join(directory, "status.json")) as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return None


def run(argv):
    directory, commit = option(argv, "--control-dir"), option(argv, "--target-commit")
    os.makedirs(directory, exist_ok=True)
    if ARGS_LOG:
        with open(ARGS_LOG, "a") as handle:
            handle.write(json.dumps(["run"] + argv) + "\n")
    started = time.time()
    write_status(directory, state="starting", started_at=started, driver=None, failed=None, exit_code=None)
    note(f"validator-run {commit}")
    if listed("SATURN_TEST_VALIDATOR_HANG_COMMITS", commit):
        while not os.path.exists(os.path.join(directory, "stop")):
            time.sleep(0.1)
        write_status(directory, state="failed", failed={"id": "validation.timeout", "detail": "never ready"}, exit_code=3, finished_at=time.time())
        return 3
    time.sleep(0.3)
    if listed("SATURN_TEST_VALIDATOR_STALL_COMMITS", commit):
        # The session is attached, so the owner's own gate would pass. Only the validator's verdict can stop the commit.
        if MARKER:
            open(MARKER, "w").close()
        write_status(directory, state="failed", exit_code=1, finished_at=time.time(),
                     failed={"id": "audio.decoded_progress", "detail": "decoded audio frames played advanced by 0 (need 30) over 3.0 s"},
                     conditions={"session.connected_as_viewer": {"ok": True}, "audio.decoded_progress": {"ok": False}})
        note(f"validator-stalled {commit}")
        while not os.path.exists(os.path.join(directory, "stop")):
            time.sleep(0.05)
        if MARKER and os.path.exists(MARKER):
            os.unlink(MARKER)
        return 1
    if listed("SATURN_TEST_VALIDATOR_FAIL_COMMITS", commit):
        write_status(directory, state="failed", exit_code=1, finished_at=time.time(),
                     failed={"id": "data.audio_frames_arriving", "detail": "Opus frames advanced by 0 (need 30), codec 'opus' over 3.0 s"},
                     conditions={"session.connected_as_viewer": {"ok": True}, "data.audio_frames_arriving": {"ok": False}})
        note(f"validator-failed {commit}")
        return 1
    if MARKER:
        open(MARKER, "w").close()
    write_status(directory, state="ready", driver="native", ready_at=time.time(),
                 conditions={"session.connected_as_viewer": {"ok": True}, "data.audio_frames_arriving": {"ok": True}, "audio.decoded_progress": {"ok": True}})
    note(f"validator-ready {commit}")
    while not os.path.exists(os.path.join(directory, "stop")):
        time.sleep(0.05)
    if MARKER and os.path.exists(MARKER):
        os.unlink(MARKER)
    if listed("SATURN_TEST_VALIDATOR_STOP_FAIL_COMMITS", commit):
        write_status(directory, state="failed", exit_code=1, finished_at=time.time(),
                     failed={"id": "cleanup.client_released", "detail": "the owner still counts our connection"})
        note(f"validator-stopped-dirty {commit}")
        return 1
    write_status(directory, state="done", exit_code=0, finished_at=time.time())
    note(f"validator-stopped {commit}")
    return 0


def wait(argv):
    directory, timeout = option(argv, "--control-dir"), float(option(argv, "--timeout", "150"))
    deadline = time.time() + timeout
    while time.time() < deadline:
        status = read_status(directory)
        if status and status.get("state") == "ready":
            print(json.dumps({"state": "ready", "driver": status.get("driver")}))
            return 0
        if status and status.get("state") in ("failed", "done"):
            print(json.dumps({"state": status["state"], "failed": status.get("failed")}))
            return 1
        time.sleep(0.1)
    status = read_status(directory) or {}
    print(json.dumps({"state": "timeout", "failed": status.get("failed")}))
    return 3


def stop(argv):
    directory, timeout = option(argv, "--control-dir"), float(option(argv, "--timeout", "60"))
    open(os.path.join(directory, "stop"), "w").close()
    deadline = time.time() + timeout
    while time.time() < deadline:
        status = read_status(directory)
        if status and status.get("state") in ("failed", "done"):
            break
        time.sleep(0.05)
    status = read_status(directory) or {"state": "unknown"}
    print(json.dumps({"state": status.get("state"), "failed": status.get("failed"), "exit_code": status.get("exit_code"), "forced": False}))
    return 0 if status.get("state") == "done" else 1


if __name__ == "__main__":
    command = sys.argv[1]
    sys.exit({"run": run, "wait": wait, "stop": stop}[command](sys.argv[2:]))
