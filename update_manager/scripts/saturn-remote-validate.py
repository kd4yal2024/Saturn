#!/usr/bin/env python3
"""Prove, with a temporary headless browser, that an installed Saturn Remote really receives.

The update process used to depend on a human keeping a browser connected: the activation helper's readiness gate requires an active receive
client (state.sdr_active), so an install on an idle radio failed, and so did its rollback. This program establishes and verifies reception
itself, in a way that cannot change the radio.

  saturn-remote-validate.py run  --control-dir DIR --target-commit COMMIT --backend xdma|p2 [options]
  saturn-remote-validate.py wait --control-dir DIR [--timeout SECONDS]       exit 0 once validation is READY, 1 if it failed or died
  saturn-remote-validate.py stop --control-dir DIR [--timeout SECONDS]       end the session, clean up, print the final status

`run` is started in the background by the activation helper. It stays alive after it reports READY, holding the receive session open until
the helper's own readiness check has finished (the helper then calls `stop`), and it always cleans up: on success, failure, timeout, a
signal, or a crash of the browser.

Receive-only, enforced independently:
  * the browser's session is opened as a VIEWER; the Bridge itself discards every command a viewer may not send;
  * saturn-remote-receive-guard.js is injected before any page script runs (so it protects a release whose page predates validation mode,
    such as the one being rolled back TO) and drops every command outside a deny-by-default allowlist, every binary uplink frame and every
    write request;
  * this program records the frames that really leave the browser, from outside the page, and fails on anything outside the allowlist;
  * the radio's tuning, sample rate, DDC, ADC and TX flags are read from the owner's telemetry before and after and must not differ.
It never starts a Bridge, never opens XDMA and never uses a saved profile.

Isolation: only the browser is unprivileged. In production it runs as a transient systemd service with a dynamic user, no sound device, no
network except loopback, a memory cap and a hard lifetime; the credential for the authenticated proxy is read here, from the saturn-go
service's own environment, and handed to the browser over the DevTools pipe. It is never put in an argument, a file or a log.
"""
from __future__ import annotations

import argparse
import base64
import collections
import hashlib
import json
import os
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable

SCHEMA = "saturn-remote-validation-v1"
EXIT_OK, EXIT_FAILED, EXIT_USAGE, EXIT_TIMEOUT = 0, 1, 2, 3

# Keep in step with saturn-remote-receive-guard.js; a test compares the two. The Bridge's own viewer list is the outer bound.
ALLOWED_COMMANDS = frozenset([
    "audio_codec", "audio_gain", "audio_samplerate", "audio_start", "audio_stop", "audio_stream_channels",
    "audio_stream_samples", "audio_stream_sample_type", "iq_start", "iq_stop", "saturn_display", "saturn_display_ack",
    "saturn_ping", "session_open",
])
# The radio's own state, as the owner publishes it. A receive-only session may not move any of it.
# processing_mode is deliberately absent: the Bridge derives it from client demand (drain-only with no receiver, audio with one), so it moves
# because reception started, not because anything was changed.
RADIO_METRICS = ("frequency_hz", "sample_rate_hz", "ddc", "adc", "tx_keyed", "tx_stream")
RADIO_TOP = ("rf_safe", "backend")

DEFAULT_LIMITS = {
    "ready_url_wait_s": 30.0,     # the services were just restarted: how long the target may take to report itself ready
    "page_load_s": 45.0,
    "connect_s": 45.0,
    "data_s": 45.0,               # how long, once connected, fresh data and decoded audio may take to show up
    "window_s": 3.0,              # the measured interval over which counters must advance
    "min_audio_played": 30,       # decoded audio frames that must be played in that interval (about 0.6 s of audio)
    "min_opus_frames": 30,
    "min_display": 8,             # display rows (or raw IQ frames) that must arrive in that interval
    "hold_stall_s": 6.0,          # once ready: how long audio may stand still before the held session counts as degraded
    "max_seconds": 300.0,         # hard bound on the whole session
    "cleanup_s": 15.0,            # how long the owner may take to drop the temporary client
    "call_s": 15.0,               # one DevTools call
    "poll_s": 0.5,
}


def now() -> float:
    return time.time()


# --------------------------------------------------------------------------------------------------------------------------------------
# secrets never reach a log, a status file or an argument
class Redactor:
    def __init__(self) -> None:
        self.secrets: set[str] = set()

    def add(self, credential: str | None) -> None:
        if not credential:
            return
        pieces = {credential, base64.b64encode(credential.encode()).decode()}
        if ":" in credential:
            pieces.add(credential.split(":", 1)[1])
        self.secrets.update(piece for piece in pieces if len(piece) >= 4)

    def clean(self, text: str) -> str:
        for secret in sorted(self.secrets, key=len, reverse=True):
            text = text.replace(secret, "<redacted>")
        return text


class Recorder:
    """The retained evidence: a JSON-lines log and an atomically replaced status file."""

    def __init__(self, directory: Path, redactor: Redactor) -> None:
        self.directory = directory
        self.redactor = redactor
        self.log_path = directory / "validation.log"
        self.status_path = directory / "status.json"
        self.lock = threading.Lock()
        self.started = now()
        self.data: dict[str, Any] = {
            "schema": SCHEMA, "state": "starting", "phase": "starting", "pid": os.getpid(), "started_at": self.started,
            "conditions": {}, "failed": None, "driver": None, "exit_code": None,
        }

    def log(self, event: str, **fields: Any) -> None:
        line = json.dumps({"t": round(now() - self.started, 3), "event": event, **fields}, default=str, sort_keys=True)
        with self.lock, open(self.log_path, "a", encoding="utf-8") as handle:
            handle.write(self.redactor.clean(line) + "\n")

    def write_status(self) -> None:
        with self.lock:
            text = self.redactor.clean(json.dumps(self.data, indent=2, sort_keys=True, default=str)) + "\n"
            temporary = self.directory / ".status.json.tmp"
            with open(temporary, "w", encoding="utf-8") as handle:
                handle.write(text)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, self.status_path)

    def update(self, **fields: Any) -> None:
        self.data.update(fields)
        self.write_status()

    def condition(self, identifier: str, ok: bool, detail: str = "") -> bool:
        """Record one verified fact. The first failure is the one reported as the failed condition."""
        self.data["conditions"][identifier] = {"ok": bool(ok), "detail": detail, "at": round(now() - self.started, 3)}
        self.log("condition", id=identifier, ok=bool(ok), detail=detail)
        if not ok and self.data["failed"] is None:
            self.data["failed"] = {"id": identifier, "detail": detail}
        self.write_status()
        return bool(ok)


# --------------------------------------------------------------------------------------------------------------------------------------
# what the owner (the Bridge and the server) says; separate from what the browser says
class Owner:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args

    def http_json(self, url: str, timeout: float = 3.0) -> Any:
        with urllib.request.urlopen(url, timeout=timeout) as response:  # noqa: S310  (loopback URLs from root-owned configuration)
            return json.loads(response.read().decode("utf-8"))

    def readyz(self) -> Any:
        return self.http_json(self.args.ready_url)

    def p23(self) -> Any:
        return self.http_json(self.args.perf_url)

    def perf(self) -> dict[str, Any]:
        with open(self.args.perf_json, encoding="utf-8") as handle:
            return json.load(handle)

    def main_pid(self, unit: str) -> int | None:
        result = subprocess.run([self.args.systemctl, "show", "-p", "MainPID", "--value", unit], capture_output=True, text=True, timeout=10)
        try:
            pid = int(result.stdout.strip())
        except ValueError:
            return None
        return pid if pid > 0 else None

    def exe(self, pid: int) -> str:
        return os.path.realpath(f"/proc/{pid}/exe")

    def credential(self) -> str | None:
        """SATURN_REMOTE_BASIC_AUTH from the saturn-go service's own environment, or None when the proxy runs without one."""
        pid = self.main_pid(self.args.auth_unit)
        if pid is None:
            raise RuntimeError(f"{self.args.auth_unit} has no main process, so its proxy credential cannot be read")
        with open(f"/proc/{pid}/environ", "rb") as handle:
            for entry in handle.read().split(b"\0"):
                if entry.startswith(b"SATURN_REMOTE_BASIC_AUTH="):
                    return entry.split(b"=", 1)[1].decode("utf-8") or None
        return None


def gate_conditions(p23: Any, backend: str) -> list[tuple[str, Any, bool]]:
    """The activation helper's own receive-readiness conditions (selected_owner_receiving), one by one."""
    try:
        perf = p23["perf"]
        workload, telemetry = perf["workload"], perf["app_telemetry"]
        current = telemetry["current"]
        state = current["state"]
    except (KeyError, TypeError):
        return [("p23_perf has the expected structure", None, False)]
    pid, age = workload.get("service_main_pid"), telemetry.get("age_seconds")
    expected_app = "p2" if backend == "p2" else "saturn-bridge"
    return [
        (f"workload.selected_app == {backend}", workload.get("selected_app"), workload.get("selected_app") == backend),
        (f"current.app == {expected_app}", current.get("app"), current.get("app") == expected_app),
        ("service_main_pid is positive and equals current.pid", (pid, current.get("pid")), isinstance(pid, int) and pid > 0 and current.get("pid") == pid),
        ("snapshot_readable and pid_matches_service", (telemetry.get("snapshot_readable"), telemetry.get("pid_matches_service")),
         telemetry.get("snapshot_readable") is True and telemetry.get("pid_matches_service") is True),
        ("0 <= age_seconds <= 5", age, isinstance(age, (int, float)) and 0 <= age <= 5),
        ("state.sdr_active is true (a receive client is attached)", state.get("sdr_active"), state.get("sdr_active") is True),
        ("state.thread_error is not true", state.get("thread_error"), state.get("thread_error") is not True),
    ]


def frame_offenders(frame: dict[str, Any]) -> list[str]:
    """What, if anything, a frame that left the browser may not carry."""
    if frame.get("opcode") == 2:
        return ["binary frame"]
    offenders = []
    for segment in str(frame.get("data", "")).split(";"):
        command = segment.strip()
        if command and command.split(":", 1)[0].strip().lower() not in ALLOWED_COMMANDS:
            offenders.append(command[:80])
    return offenders


def radio_state(perf: dict[str, Any]) -> dict[str, Any]:
    metrics = perf.get("metrics", {})
    state = {key: metrics.get(key) for key in RADIO_METRICS}
    state.update({key: perf.get(key) for key in RADIO_TOP})
    return state


# --------------------------------------------------------------------------------------------------------------------------------------
# the browser
def spki_pin(certificate: Path) -> str:
    """base64(sha256(SubjectPublicKeyInfo)) of the proxy's own certificate: pin exactly it instead of ignoring every certificate error."""
    key = subprocess.run(["openssl", "x509", "-in", str(certificate), "-pubkey", "-noout"], capture_output=True, check=True, timeout=15).stdout
    der = subprocess.run(["openssl", "pkey", "-pubin", "-outform", "der"], input=key, capture_output=True, check=True, timeout=15).stdout
    return base64.b64encode(hashlib.sha256(der).digest()).decode()


def browser_flags(user_data_dir: str, spki: str | None, sandbox: bool) -> list[str]:
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={user_data_dir}", "--disable-gpu", "--no-first-run",
        "--no-default-browser-check", "--disable-extensions", "--disable-background-networking", "--disable-sync",
        "--disable-component-update", "--disable-dev-shm-usage", "--autoplay-policy=no-user-gesture-required",
        "--window-size=800,500", "--force-device-scale-factor=1", "--alsa-output-device=null", "--js-flags=--max-old-space-size=160",
    ]
    if spki:
        flags.append(f"--ignore-certificate-errors-spki-list={spki}")
    if not sandbox:
        flags.append("--no-sandbox")
    return flags + ["about:blank"]


# The DevTools pipe is file descriptors 3 (commands in) and 4 (replies out). Both launchers hand the browser a pair of ordinary pipes as its
# stdin/stdout and map them to 3 and 4 here, so the same code path serves a transient unit and a direct child.
PIPE_WRAPPER = 'exec "$@" 3<&0 4>&1 0</dev/null 1>&2'


def build_launch_command(mode: str, browser: str, flags: list[str], unit: str, max_seconds: float, memory_max: str) -> list[str]:
    inner = ["/bin/sh", "-c", PIPE_WRAPPER, "sh", browser, *flags]
    if mode == "direct":
        return inner
    if mode != "systemd":
        raise ValueError(f"unknown launcher {mode!r}")
    properties = [
        "DynamicUser=yes", "PrivateTmp=yes", "PrivateDevices=yes", "ProtectSystem=strict", "ProtectHome=yes", "NoNewPrivileges=yes",
        "ProtectKernelTunables=yes", "ProtectControlGroups=yes", "RestrictSUIDSGID=yes", "LockPersonality=yes",
        "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK", "IPAddressDeny=any", "IPAddressAllow=localhost",
        f"MemoryMax={memory_max}", "MemorySwapMax=0", "TasksMax=300", "CPUWeight=20", "Nice=10", "IOSchedulingClass=idle",
        f"RuntimeMaxSec={int(max_seconds) + 30}", "Environment=HOME=/tmp", "Environment=PULSE_SERVER=none", "WorkingDirectory=/tmp",
    ]
    command = ["systemd-run", "--quiet", "--collect", "--pipe", f"--unit={unit}"]
    for prop in properties:
        command += ["-p", prop]
    return command + ["--", *inner]


class CdpError(RuntimeError):
    pass


class Cdp:
    """A DevTools client over the pipe pair: NUL-terminated JSON messages."""

    def __init__(self, process: subprocess.Popen, recorder: Recorder, call_timeout: float) -> None:
        self.process = process
        self.recorder = recorder
        self.call_timeout = call_timeout
        self.next_id = 0
        self.pending: dict[int, dict[str, Any]] = {}
        self.handlers: dict[str, list[Callable[[dict[str, Any], str | None], None]]] = collections.defaultdict(list)
        self.lock = threading.Lock()
        self.write_lock = threading.Lock()
        self.closed = threading.Event()
        self.unresponsive = False
        threading.Thread(target=self._read, daemon=True).start()

    def on(self, method: str, handler: Callable[[dict[str, Any], str | None], None]) -> None:
        self.handlers[method].append(handler)

    def _read(self) -> None:
        buffer = b""
        stream = self.process.stdout
        try:
            while True:
                chunk = os.read(stream.fileno(), 65536)
                if not chunk:
                    break
                buffer += chunk
                while b"\0" in buffer:
                    raw, buffer = buffer.split(b"\0", 1)
                    if raw:
                        self._dispatch(json.loads(raw.decode("utf-8", "replace")))
        except (OSError, ValueError) as error:
            self.recorder.log("cdp_read_error", error=str(error))
        finally:
            self.closed.set()
            with self.lock:
                for slot in self.pending.values():
                    slot["event"].set()

    def _dispatch(self, message: dict[str, Any]) -> None:
        if "id" in message:
            with self.lock:
                slot = self.pending.get(message["id"])
            if slot is not None:
                slot["message"] = message
                slot["event"].set()
            return
        for handler in list(self.handlers.get(message.get("method", ""), [])):
            try:
                handler(message.get("params", {}), message.get("sessionId"))
            except Exception as error:  # a handler bug must not kill the reader
                self.recorder.log("cdp_handler_error", method=message.get("method"), error=repr(error))

    def _write(self, method: str, params: dict[str, Any] | None, session: str | None, slot: dict[str, Any] | None) -> int:
        with self.lock:
            self.next_id += 1
            identifier = self.next_id
            if slot is not None:
                self.pending[identifier] = slot
        message: dict[str, Any] = {"id": identifier, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        try:
            with self.write_lock:
                self.process.stdin.write(json.dumps(message).encode() + b"\0")
                self.process.stdin.flush()
        except (OSError, ValueError) as error:
            with self.lock:
                self.pending.pop(identifier, None)
            raise CdpError(f"cannot write to the browser: {error}") from error
        return identifier

    def send(self, method: str, params: dict[str, Any] | None = None, session: str | None = None) -> None:
        """Fire and forget. Event handlers run on the reader thread, which must never wait for a reply that only it can deliver."""
        if not self.closed.is_set():
            try:
                self._write(method, params, session, None)
            except CdpError as error:
                self.recorder.log("cdp_send_failed", method=method, error=str(error))

    def call(self, method: str, params: dict[str, Any] | None = None, session: str | None = None, timeout: float | None = None) -> Any:
        if self.closed.is_set():
            raise CdpError("the browser's DevTools pipe is closed")
        slot: dict[str, Any] = {"event": threading.Event(), "message": None}
        identifier = self._write(method, params, session, slot)
        if not slot["event"].wait(timeout if timeout is not None else self.call_timeout):
            with self.lock:
                self.pending.pop(identifier, None)
            self.unresponsive = True
            raise CdpError(f"{method} did not answer within {timeout if timeout is not None else self.call_timeout:g} s (browser unresponsive)")
        with self.lock:
            self.pending.pop(identifier, None)
        reply = slot["message"]
        if reply is None:
            raise CdpError("the browser's DevTools pipe closed while waiting for " + method)
        if "error" in reply:
            raise CdpError(f"{method}: {reply['error'].get('message', reply['error'])}")
        return reply.get("result", {})


# --------------------------------------------------------------------------------------------------------------------------------------
LEGACY_SNAPSHOT = """JSON.stringify((() => ({
  mode: "legacy", at: Date.now(), path: location.pathname,
  connected: state.connected === true, bridgeReady: state.bridgeReady === true, connectPending: state.connectPending === true,
  role: state.remoteClientRole, wsReadyState: state.ws ? state.ws.readyState : null,
  audioContextState: state.audioCtx ? state.audioCtx.state : null,
  codec: rxAudioCodecSession.accepted, opusFrames: rxOpus.opusFrames, pcmFrames: rxOpus.pcmFrames, decodeErrors: rxOpus.decodeErrors, lateDrops: rxOpus.lateDrops,
  audioFramesPlayed: state.audioFramesPlayed, lastAudioSequence: state.lastAudioSequence, audioSeqGaps: state.audioSeqGapCount,
  underruns: state.rxWorkletUnderruns, overflows: state.rxWorkletOverflows, queuedMs: state.rxWorkletQueuedMs,
  renderSource: state.displayRenderSource, displayRows: state.displayServerRowsReceived, iqFrames: state.iqFrameVersion, rxIqFrames: state.rxIqFrameVersion,
  blockedCounts: (window.saturnReceiveGuardRecorder || {}).blockedCounts || {}, blockedExamples: (window.saturnReceiveGuardRecorder || {}).blockedExamples || [],
  violations: (window.saturnReceiveGuardRecorder || {}).violations || [], errors: (window.saturnReceiveGuardRecorder || {}).errors || [],
}))())"""
# Proof that the injected receive-only guard is really installed in the page, whatever page it is.
GUARD_PROBE = ("JSON.stringify({active: !!(window.saturnReceiveGuardRecorder && WebSocket.prototype.send && WebSocket.prototype.send.receiveOnlyGuard === true"
               " && window.fetch && window.fetch.receiveOnlyGuard === true)})")
NATIVE_SNAPSHOT = "JSON.stringify(window.saturnValidation.snapshot())"
LEGACY_START = "(() => { const b = document.getElementById('go-live-btn'); if (!b) return JSON.stringify({ok: false, reason: 'no go-live button'}); b.click(); return JSON.stringify({ok: true}); })()"
LEGACY_STOP = "(() => { const b = document.getElementById('go-live-btn'); if (b) b.click(); return JSON.stringify({ok: true}); })()"
NATIVE_START = "window.saturnValidation.start().then((r) => JSON.stringify(r))"
NATIVE_STOP = "window.saturnValidation.stop().then((r) => JSON.stringify(r))"


class Validator:
    def __init__(self, args: argparse.Namespace, recorder: Recorder, redactor: Redactor, owner: Owner | None = None) -> None:
        self.args = args
        self.recorder = recorder
        self.redactor = redactor
        self.owner = owner or Owner(args)
        self.limits = dict(DEFAULT_LIMITS)
        self.limits.update({k: v for k, v in (json.loads(args.limits) if args.limits else {}).items() if k in DEFAULT_LIMITS})
        self.stop_requested = threading.Event()
        self.timed_out = threading.Event()
        self.deadline = now() + self.limits["max_seconds"]
        self.process: subprocess.Popen | None = None
        self.cdp: Cdp | None = None
        self.session: str | None = None
        self.unit = f"saturn-remote-validator-{os.getpid()}-{int(now()) % 100000}"
        self.profile_dir: str | None = None
        self.credential: str | None = None
        self.driver = "native"
        # evidence gathered from the browser, outside the page's own code
        self.wire: list[dict[str, Any]] = []
        self.console_errors: list[str] = []
        self.exceptions: list[str] = []
        self.net_failures: list[str] = []
        self.document_status: int | None = None
        self.crashed = threading.Event()
        self.wire_alarm = threading.Event()
        self.baseline_radio: dict[str, Any] = {}
        self.baseline_connections = 0
        self.first_failure_time: float | None = None

    # ---- small helpers
    def failed(self) -> bool:
        return self.recorder.data["failed"] is not None

    def remaining(self) -> float:
        return self.deadline - now()

    def check_deadline(self) -> bool:
        """True once the overall time limit has passed; the first caller marks it, so no path can end early and still look like success."""
        if self.remaining() <= 0 and not self.timed_out.is_set():
            self.recorder.log("deadline_reached")
            self.timed_out.set()
            self.stop_requested.set()
        return self.timed_out.is_set()

    def sleep(self, seconds: float) -> None:
        self.stop_requested.wait(min(seconds, max(0.0, self.remaining())))

    def fail(self, identifier: str, detail: str) -> None:
        self.recorder.condition(identifier, False, detail)

    def interrupted(self, identifier: str, doing: str) -> bool:
        """A wait ended because the validation was told to stop. Say which: the overall time limit, or a request."""
        if self.check_deadline():
            return self.recorder.condition("validation.timeout", False, f"the {self.limits['max_seconds']:g} s limit for the whole validation was reached while {doing}")
        return self.recorder.condition(identifier, False, f"the validation was stopped while {doing}")

    # ---- phase 1: the services
    def check_release(self) -> bool:
        deadline = now() + self.limits["ready_url_wait_s"]
        last = ""
        while now() < deadline and not self.stop_requested.is_set():
            try:
                ready = self.owner.readyz()
                commit = ready.get("build_commit")
                if ready.get("ready") is True and commit == self.args.target_commit:
                    break
                last = f"readyz reports ready={ready.get('ready')} build_commit={commit}, wanted {self.args.target_commit}"
            except (OSError, ValueError, urllib.error.URLError) as error:
                last = f"readyz not answering: {error}"
            self.sleep(1.0)
        else:
            if self.stop_requested.is_set():
                return self.interrupted("release.running", "waiting for the target release to report ready")
            return self.recorder.condition("release.running", False, last or "the target release did not report ready in time")
        problems = []
        root = os.path.realpath(os.path.join(self.args.release_root, self.args.target_commit)) + os.sep
        for unit in self.args.unit:
            pid = self.owner.main_pid(unit)
            if pid is None:
                problems.append(f"{unit} has no main process")
                continue
            exe = self.owner.exe(pid)
            if not self.args.no_exe_check and not exe.startswith(root):
                problems.append(f"{unit} (pid {pid}) runs {exe}, not from {root}")
        where = "readyz reports it" if self.args.no_exe_check else "readyz reports it and every service runs from its release directory"
        return self.recorder.condition("release.running", not problems, "; ".join(problems) or f"{self.args.target_commit} is running ({where}; {', '.join(self.args.unit)})")

    def read_owner(self) -> dict[str, Any] | None:
        """One coherent look at the owner: its telemetry, and whether that telemetry belongs to the running service."""
        try:
            perf = self.owner.perf()
        except (OSError, ValueError) as error:
            self.fail("owner.fresh_telemetry", f"cannot read {self.args.perf_json}: {error}")
            return None
        metrics = perf.get("metrics", {})
        age = (now() * 1000 - perf.get("updated_at_ms", 0)) / 1000.0
        pid = self.owner.main_pid(self.args.owner_unit)
        return {"perf": perf, "metrics": metrics, "age_s": age, "pid_matches": pid is not None and metrics.get("pid") == pid, "service_pid": pid}

    def baseline(self) -> bool:
        view = self.read_owner()
        if view is None:
            return False
        ok = 0 <= view["age_s"] <= 5 and view["pid_matches"]
        self.recorder.condition("owner.fresh_telemetry", ok, f"before the session: age {view['age_s']:.2f} s, telemetry pid {view['metrics'].get('pid')}, service pid {view['service_pid']}")
        self.baseline_radio = radio_state(view["perf"])
        self.baseline_connections = int(view["metrics"].get("connections") or 0)
        self.recorder.log("baseline", radio=self.baseline_radio, connections=self.baseline_connections, rxc1=(view["metrics"].get("rx_counter_v31") or {}).get("status"))
        return ok

    # ---- phase 2: the browser
    def launch(self) -> bool:
        spki = spki_pin(Path(self.args.tls_cert)) if self.args.tls_cert else None
        if self.args.launcher == "direct":
            import tempfile
            self.profile_dir = tempfile.mkdtemp(prefix="saturn-validate-")
            profile = self.profile_dir
        else:
            profile = "/tmp/profile"
        flags = browser_flags(profile, spki, sandbox=not self.args.no_sandbox)
        command = build_launch_command(self.args.launcher, self.args.browser, flags, self.unit, self.limits["max_seconds"], self.args.memory_max)
        self.recorder.log("launch", launcher=self.args.launcher, unit=self.unit, argv=command)
        self.recorder.update(launcher=self.args.launcher, unit=self.unit)
        env = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": self.profile_dir or "/tmp", "PULSE_SERVER": "none", "LANG": "C"}
        if self.args.launcher == "direct":
            env.update({key: value for key, value in os.environ.items() if key.startswith("FAKE_")})
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, start_new_session=True)
        threading.Thread(target=self._drain_stderr, daemon=True).start()
        self.cdp = Cdp(self.process, self.recorder, self.limits["call_s"])
        cdp = self.cdp
        cdp.on("Target.targetCrashed", lambda p, s: self.crashed.set())
        cdp.on("Inspector.targetCrashed", lambda p, s: self.crashed.set())
        cdp.on("Runtime.exceptionThrown", self._on_exception)
        cdp.on("Runtime.consoleAPICalled", self._on_console)
        cdp.on("Network.responseReceived", self._on_response)
        cdp.on("Network.loadingFailed", self._on_loading_failed)
        cdp.on("Network.webSocketFrameSent", self._on_frame_sent)
        cdp.on("Fetch.authRequired", self._on_auth)
        cdp.on("Fetch.requestPaused", self._on_paused)
        cdp.on("Page.javascriptDialogOpening", self._on_dialog)
        return True

    def _drain_stderr(self) -> None:
        tail = collections.deque(maxlen=60)
        try:
            for raw in iter(self.process.stderr.readline, b""):
                tail.append(raw.decode("utf-8", "replace").rstrip())
        finally:
            self.stderr_tail = list(tail)

    stderr_tail: list[str] = []

    def _on_exception(self, params: dict[str, Any], _s: str | None) -> None:
        detail = params.get("exceptionDetails", {})
        text = (detail.get("exception") or {}).get("description") or detail.get("text") or "exception"
        if len(self.exceptions) < 30:
            self.exceptions.append(str(text)[:300])

    def _on_console(self, params: dict[str, Any], _s: str | None) -> None:
        if params.get("type") == "error" and len(self.console_errors) < 30:
            self.console_errors.append(" ".join(str(a.get("value", a.get("description", ""))) for a in params.get("args", []))[:300])

    def _on_response(self, params: dict[str, Any], _s: str | None) -> None:
        if params.get("type") == "Document":
            self.document_status = (params.get("response") or {}).get("status")

    def _on_loading_failed(self, params: dict[str, Any], _s: str | None) -> None:
        if not params.get("canceled") and len(self.net_failures) < 30:
            self.net_failures.append(f"{params.get('type')}: {params.get('errorText')}")

    def _on_frame_sent(self, params: dict[str, Any], _s: str | None) -> None:
        frame = params.get("response") or {}
        record = {"opcode": frame.get("opcode"), "data": str(frame.get("payloadData", ""))[:200]}
        if len(self.wire) < 4000:
            self.wire.append(record)
        if frame_offenders(record) and not self.wire_alarm.is_set():
            # Judged the moment it leaves, not after the measurement: the session ends at the first command it may not send.
            self.recorder.log("wire_alarm", frame=record)
            self.wire_alarm.set()

    def _on_auth(self, params: dict[str, Any], session: str | None) -> None:
        user, _, password = (self.credential or ":").partition(":")
        answer = {"response": "ProvideCredentials", "username": user, "password": password} if self.credential else {"response": "CancelAuth"}
        self.cdp.send("Fetch.continueWithAuth", {"requestId": params["requestId"], "authChallengeResponse": answer}, session=session)

    def _on_paused(self, params: dict[str, Any], session: str | None) -> None:
        self.cdp.send("Fetch.continueRequest", {"requestId": params["requestId"]}, session=session)

    def _on_dialog(self, params: dict[str, Any], session: str | None) -> None:
        self.cdp.send("Page.handleJavaScriptDialog", {"accept": True}, session=session)

    def evaluate(self, expression: str, await_promise: bool = False, timeout: float | None = None) -> Any:
        result = self.cdp.call("Runtime.evaluate", {"expression": expression, "returnByValue": True, "awaitPromise": await_promise, "userGesture": True},
                               session=self.session, timeout=timeout)
        if "exceptionDetails" in result:
            raise CdpError("page evaluation failed: " + str((result["exceptionDetails"].get("exception") or {}).get("description") or result["exceptionDetails"].get("text")))
        return (result.get("result") or {}).get("value")

    def attach_and_navigate(self) -> bool:
        cdp = self.cdp
        targets = cdp.call("Target.getTargets")["targetInfos"]
        page = next((t for t in targets if t.get("type") == "page"), None)
        if page is None:
            return self.recorder.condition("browser.started", False, "the browser has no page target")
        self.session = cdp.call("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True})["sessionId"]
        for domain in ("Page", "Runtime", "Network", "Log"):
            cdp.call(f"{domain}.enable", session=self.session)
        cdp.call("Fetch.enable", {"handleAuthRequests": True}, session=self.session)
        guard = Path(self.args.guard_script).read_text(encoding="utf-8")
        cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": guard}, session=self.session)
        self.recorder.condition("browser.started", True, "headless browser attached; receive-only guard injected before any page script")
        self.recorder.update(phase="navigating")
        url = self.args.url
        cdp.call("Page.navigate", {"url": url}, session=self.session)
        deadline = now() + self.limits["page_load_s"]
        while now() < deadline and not self.stop_requested.is_set():
            if self.cdp.closed.is_set():
                return self.recorder.condition("browser.alive", False, "the browser exited while the page was loading")
            try:
                present = self.evaluate("(document.readyState === 'complete' && location.protocol !== 'about:') ? (typeof window.saturnValidation) : 'loading'", timeout=5)
            except CdpError:
                present = "loading"
            # Right after Page.navigate the previous document (about:blank) is still current and already "complete"; only the main document's
            # own response proves the page being validated has replaced it.
            if present in ("object", "undefined") and self.document_status is not None:
                self.driver = "native" if present == "object" else "legacy"
                self.recorder.update(driver=self.driver)
                ok = self.document_status == 200 and not self.exceptions
                detail = f"main document HTTP {self.document_status}; page driver {self.driver}; {len(self.exceptions)} uncaught exception(s)"
                if self.exceptions:
                    detail += ": " + self.exceptions[0]
                return self.recorder.condition("page.loaded", ok, detail)
            self.sleep(0.25)
        if self.stop_requested.is_set():
            return self.interrupted("page.loaded", "the page was loading")
        return self.recorder.condition("page.loaded", False, f"the page did not finish loading within {self.limits['page_load_s']:g} s (document HTTP {self.document_status})")

    def snapshot(self) -> dict[str, Any]:
        return json.loads(self.evaluate(NATIVE_SNAPSHOT if self.driver == "native" else LEGACY_SNAPSHOT, timeout=8))

    # ---- phase 3: connect, then prove data and decoded audio
    def start_session(self) -> bool:
        self.recorder.update(phase="connecting")
        started = json.loads(self.evaluate(NATIVE_START if self.driver == "native" else LEGACY_START, await_promise=self.driver == "native", timeout=20))
        if not started.get("ok"):
            return self.recorder.condition("session.started", False, str(started.get("reason", started)))
        self.recorder.condition("session.started", True, f"{self.driver} start accepted")
        deadline = now() + self.limits["connect_s"]
        snap: dict[str, Any] = {}
        while now() < deadline and not self.stop_requested.is_set():
            snap = self.snapshot()
            if self.alarm():
                return False
            if snap.get("connected") and snap.get("bridgeReady"):
                role = snap.get("role")
                self.recorder.update(role=role, role_enforced_by_bridge=role == "viewer")
                if role == "viewer":
                    return self.recorder.condition("session.connected_as_viewer", True, "connected; the Bridge assigned the viewer role and enforces it")
                if role == "operator" and self.args.viewer_role == "preferred":
                    return self.recorder.condition(
                        "session.connected_as_viewer", True,
                        "connected; the Bridge assigned role 'operator' because it predates viewer enforcement: receive-only rests on the injected guard, "
                        "the live wire audit and the unchanged-radio check")
                return self.recorder.condition("session.connected_as_viewer", False, f"connected; the Bridge assigned role {role!r} (a validation session must be a viewer)")
            self.sleep(self.limits["poll_s"])
        if self.stop_requested.is_set():
            return self.interrupted("session.connected_as_viewer", "the session was connecting")
        return self.recorder.condition("session.connected_as_viewer", False,
                                       f"not connected within {self.limits['connect_s']:g} s (connected={snap.get('connected')}, bridgeReady={snap.get('bridgeReady')}, pending={snap.get('connectPending')})")

    def measure(self) -> bool:
        self.recorder.update(phase="measuring")
        base = self.snapshot()
        base_t = now()
        last = base
        deadline = base_t + self.limits["data_s"]
        while not self.stop_requested.is_set():
            self.sleep(self.limits["poll_s"])
            if self.alarm():
                return False
            last = self.snapshot()
            elapsed = now() - base_t
            if elapsed >= self.limits["window_s"]:
                checks = self.data_checks(base, last)
                if all(ok for _, ok, _ in checks) or now() >= deadline:
                    for identifier, ok, detail in checks:
                        self.recorder.condition(identifier, ok, f"{detail} over {elapsed:.1f} s")
                    self.last_snapshot = last
                    return all(ok for _, ok, _ in checks)
            if now() >= deadline or self.remaining() <= 0:
                break
        if self.stop_requested.is_set() or self.remaining() <= 0:
            self.interrupted("data.measured", "measuring receive data")
        else:
            self.recorder.condition("data.measured", False, f"no measurement window completed within {self.limits['data_s']:g} s")
        return False

    def data_checks(self, a: dict[str, Any], b: dict[str, Any]) -> list[tuple[str, bool, str]]:
        def delta(key: str) -> int:
            return int((b.get(key) or 0) - (a.get(key) or 0))
        display = max(delta("displayRows"), delta("iqFrames"), delta("rxIqFrames"))
        return [
            ("data.display_fresh", display >= self.limits["min_display"], f"display rows/IQ frames advanced by {display} (need {self.limits['min_display']})"),
            ("data.audio_frames_arriving", delta("opusFrames") >= self.limits["min_opus_frames"] and b.get("codec") == "opus",
             f"Opus frames advanced by {delta('opusFrames')} (need {self.limits['min_opus_frames']}), codec {b.get('codec')!r}"),
            ("audio.decoded_progress", delta("audioFramesPlayed") >= self.limits["min_audio_played"], f"decoded audio frames played advanced by {delta('audioFramesPlayed')} (need {self.limits['min_audio_played']})"),
            ("audio.context_running", b.get("audioContextState") == "running", f"AudioContext state {b.get('audioContextState')!r}"),
            ("audio.no_decode_errors", delta("decodeErrors") == 0, f"decode errors advanced by {delta('decodeErrors')}"),
        ]

    # ---- phase 4: everything the owner and the wire say, checked independently of the page
    def check_owner_and_wire(self) -> bool:
        ok = True
        view = self.read_owner()
        if view is None:
            return False
        ok &= self.recorder.condition("owner.fresh_telemetry_with_session", 0 <= view["age_s"] <= 5 and view["pid_matches"],
                                      f"age {view['age_s']:.2f} s, telemetry pid {view['metrics'].get('pid')}, service pid {view['service_pid']}")
        connections = int(view["metrics"].get("connections") or 0)
        ok &= self.recorder.condition("owner.session_registered", connections > self.baseline_connections and int(view["metrics"].get("client") or 0) == 1,
                                      f"the owner counts {connections} connection(s) with the session attached, {self.baseline_connections} before it; client flag {view['metrics'].get('client')}")
        try:
            gate = gate_conditions(self.owner.p23(), self.args.backend)
        except (OSError, ValueError, urllib.error.URLError) as error:
            gate = [("p23_perf is readable", str(error), False)]
        failed_gate = [f"{name}: {value!r}" for name, value, passed in gate if not passed]
        ok &= self.recorder.condition("owner.gate_equivalent", not failed_gate, "; ".join(failed_gate) or "every condition of the activation helper's receive-readiness gate holds with this session attached")
        rx = (view["metrics"].get("rx_counter_v31") or {}).get("status")
        if self.args.expect_rxc1 == "any":
            ok &= self.recorder.condition("rxc1.intended_state", True, f"RXC1 status {rx!r} recorded (no particular state required)")
        else:
            ok &= self.recorder.condition("rxc1.intended_state", rx == self.args.expect_rxc1, f"RXC1 status {rx!r}, intended {self.args.expect_rxc1!r}")
        ok &= self.check_radio_unchanged(view)
        ok &= self.check_wire()
        snap = self.last_snapshot
        problems = []
        if snap.get("violations"):
            problems.append(f"the page guard recorded violations {snap['violations'][:3]}")
        if snap.get("errors"):
            problems.append(f"uncaught page errors {snap['errors'][:3]}")
        if self.exceptions:
            problems.append(f"uncaught exceptions {self.exceptions[:2]}")
        if self.crashed.is_set():
            problems.append("the page crashed")
        ok &= self.recorder.condition("page.no_fatal_errors", not problems, "; ".join(problems) or f"no uncaught exception or guard violation ({len(self.console_errors)} console error message(s) recorded)")
        return bool(ok)

    def check_radio_unchanged(self, view: dict[str, Any]) -> bool:
        now_state = radio_state(view["perf"])
        differences = {key: (self.baseline_radio.get(key), now_state.get(key)) for key in self.baseline_radio if now_state.get(key) != self.baseline_radio.get(key)}
        return self.recorder.condition("radio.unchanged", not differences,
                                       "tuning, rate, input and TX flags are as they were: " + json.dumps(now_state, sort_keys=True) if not differences
                                       else f"the radio changed while the session was attached (before, after): {differences}; a person may have used the radio meanwhile")

    def alarm(self) -> bool:
        """True (after recording the exact offending frames as the failed condition) once a frame left the browser that may not have."""
        if self.wire_alarm.is_set():
            self.check_wire()
            return True
        return False

    def check_guard(self) -> bool:
        """Receive-only rests on the injected guard whatever role the Bridge assigns, so its presence is proven, not assumed."""
        try:
            active = json.loads(self.evaluate(GUARD_PROBE, timeout=8)).get("active") is True
        except (CdpError, ValueError) as error:
            return self.recorder.condition("guard.active", False, f"the guard could not be probed: {error}")
        return self.recorder.condition("guard.active", active, "the receive-only guard is installed in the page before it can send anything"
                                       if active else "the receive-only guard is NOT active in the page; nothing was sent")

    def check_wire(self) -> bool:
        offenders = []
        for frame in self.wire:
            offenders.extend(frame_offenders(frame))
        sent = sum(1 for f in self.wire if f["opcode"] != 2)
        return self.recorder.condition("wire.only_allowed_commands", not offenders and sent > 0,
                                       f"{len(self.wire)} frame(s) left the browser, every command inside the allowlist" if not offenders and sent > 0
                                       else ("no frame was observed leaving the browser, so nothing could be verified" if not offenders else f"frames outside the allowlist left the browser: {offenders[:5]}"))

    last_snapshot: dict[str, Any] = {}
    connections_before_stop: int | None = None

    # ---- phase 5: hold the session until the helper is done, then clean up
    def hold(self) -> None:
        self.recorder.update(state="ready", phase="holding", ready_at=now())
        last_progress = now()
        last_played = self.last_snapshot.get("audioFramesPlayed") or 0
        while not self.stop_requested.is_set() and self.remaining() > 0:
            self.sleep(1.0)
            if self.alarm():
                return
            if self.cdp.closed.is_set() or self.process.poll() is not None:
                self.recorder.condition("hold.session_alive", False, "the browser exited while the session was being held")
                return
            try:
                snap = self.snapshot()
            except CdpError as error:
                self.recorder.condition("hold.session_alive", False, str(error))
                return
            self.last_snapshot = snap
            if not snap.get("connected"):
                self.recorder.condition("hold.session_alive", False, "the held session lost its connection")
                return
            if (snap.get("audioFramesPlayed") or 0) > last_played:
                last_played, last_progress = snap["audioFramesPlayed"], now()
            elif now() - last_progress > self.limits["hold_stall_s"]:
                self.recorder.condition("hold.audio_progressing", False, f"decoded audio stood still for more than {self.limits['hold_stall_s']:g} s while the session was held")
                return
        if self.check_deadline():
            self.recorder.condition("hold.within_time_limit", False, f"the session was still held after {self.limits['max_seconds']:g} s: nothing asked it to stop")

    def shutdown(self) -> None:
        self.recorder.update(phase="stopping")
        before = self.read_owner_quiet()
        self.connections_before_stop = int(before["metrics"].get("connections") or 0) if before else None
        if self.cdp is not None and not self.cdp.closed.is_set() and not self.cdp.unresponsive and self.session is not None:
            try:
                self.evaluate(NATIVE_STOP if self.driver == "native" else LEGACY_STOP, await_promise=self.driver == "native", timeout=8)
                self.sleep(1.0)
            except (CdpError, ValueError) as error:
                self.recorder.log("stop_hook_failed", error=str(error))
            try:
                self.cdp.call("Browser.close", timeout=5)
            except CdpError:
                pass
        if self.process is not None:
            try:
                self.process.wait(timeout=6)
            except subprocess.TimeoutExpired:
                self.recorder.log("browser_terminate")
                self._kill_group(signal.SIGTERM)
                try:
                    self.process.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    self.recorder.log("browser_kill")
                    self._kill_group(signal.SIGKILL)
                    self.process.wait(timeout=4)
        if self.args.launcher == "systemd":
            subprocess.run([self.args.systemctl, "stop", self.unit + ".service"], capture_output=True, timeout=20)
        if self.profile_dir:
            import shutil
            shutil.rmtree(self.profile_dir, ignore_errors=True)
        self.recorder.condition("cleanup.browser_gone", self.process is None or self.process.poll() is not None, "the temporary browser and its profile are gone")
        deadline = now() + self.limits["cleanup_s"]
        released, last = False, ""
        while now() < deadline:
            view = self.read_owner_quiet()
            if view is not None:
                connections = int(view["metrics"].get("connections") or 0)
                # Someone else may have connected or left meanwhile, so this session's release is proven by either measure: back to the count
                # from before the session, or below the count from just before it was closed.
                stop_count = self.connections_before_stop
                last = (f"the owner counts {connections} connection(s); {self.baseline_connections} before the session"
                        + (f", {stop_count} just before it was closed" if stop_count is not None else ""))
                if connections <= self.baseline_connections or (stop_count is not None and connections < stop_count):
                    released = True
                    break
            time.sleep(0.5)
        self.recorder.condition("cleanup.client_released", released, last or "the owner's telemetry could not be read")

    def read_owner_quiet(self) -> dict[str, Any] | None:
        try:
            perf = self.owner.perf()
        except (OSError, ValueError):
            return None
        return {"perf": perf, "metrics": perf.get("metrics", {})}

    def _kill_group(self, sig: int) -> None:
        try:
            os.killpg(os.getpgid(self.process.pid), sig)
        except (ProcessLookupError, PermissionError):
            pass

    # ---- the whole run
    def run(self) -> int:
        recorder = self.recorder
        code = EXIT_FAILED
        try:
            try:
                self.credential = self.owner.credential()
                self.redactor.add(self.credential)
            except (RuntimeError, OSError) as error:
                recorder.condition("proxy.credential", False, str(error))
                raise _Abort
            recorder.log("credential", configured=bool(self.credential))
            if not self.check_release() or not self.baseline():
                raise _Abort
            self.launch()
            if not self.attach_and_navigate() or not self.check_guard() or not self.start_session() or not self.measure():
                raise _Abort
            if not self.check_owner_and_wire():
                raise _Abort
            self.hold()
        except _Abort:
            pass
        except CdpError as error:
            recorder.condition("browser.responsive", False, str(error))
        except Exception as error:  # report the exact failure, then clean up
            recorder.condition("validator.internal", False, repr(error))
        finally:
            try:
                self.shutdown()
            except Exception as error:
                recorder.condition("cleanup.completed", False, repr(error))
        if self.check_deadline() or self.timed_out.is_set():
            code = EXIT_TIMEOUT
        elif not self.failed():
            code = EXIT_OK
        recorder.update(state="done" if code == EXIT_OK else "failed", phase="finished", exit_code=code, finished_at=now(),
                        browser_stderr_tail=self.stderr_tail[-12:], console_errors=self.console_errors[:10], net_failures=self.net_failures[:10],
                        wire_frames=len(self.wire), last_snapshot=self.last_snapshot)
        return code


class _Abort(Exception):
    pass


# --------------------------------------------------------------------------------------------------------------------------------------
def command_run(args: argparse.Namespace) -> int:
    directory = Path(args.control_dir)
    directory.mkdir(mode=0o750, parents=True, exist_ok=True)
    for stale in ("stop", "status.json", "validation.log"):
        (directory / stale).unlink(missing_ok=True)
    redactor = Redactor()
    recorder = Recorder(directory, redactor)
    (directory / "pid").write_text(str(os.getpid()) + "\n")
    recorder.write_status()
    validator = Validator(args, recorder, redactor)

    def request_stop(signum: int, _frame: Any) -> None:
        recorder.log("signal", number=signum)
        validator.stop_requested.set()

    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)

    def watch_for_stop_file() -> None:
        while not validator.stop_requested.is_set():
            if (directory / "stop").exists():
                recorder.log("stop_requested")
                validator.stop_requested.set()
            time.sleep(0.2)

    threading.Thread(target=watch_for_stop_file, daemon=True).start()

    def watchdog() -> None:
        while not validator.stop_requested.wait(0.25):
            if validator.check_deadline():
                return

    threading.Thread(target=watchdog, daemon=True).start()
    return validator.run()


def read_status(directory: Path) -> dict[str, Any] | None:
    try:
        return json.loads((directory / "status.json").read_text())
    except (OSError, ValueError):
        return None


def pid_alive(directory: Path) -> bool:
    """The recorded process is running and is the validator: a zombie that nobody has reaped yet is not alive, and a reused pid is not ours."""
    try:
        pid = int((directory / "pid").read_text().strip())
        with open(f"/proc/{pid}/stat") as handle:
            if handle.read().rsplit(")", 1)[1].split()[0] == "Z":
                return False
        with open(f"/proc/{pid}/cmdline", "rb") as handle:
            return b"saturn-remote-validate" in handle.read()
    except (OSError, ValueError, IndexError):
        return False


def command_wait(args: argparse.Namespace) -> int:
    directory = Path(args.control_dir)
    deadline = now() + args.timeout
    while now() < deadline:
        status = read_status(directory)
        if status is not None:
            state = status["state"]
            if state in ("failed", "done"):
                print(json.dumps({"state": state, "failed": status.get("failed")}))
                return EXIT_FAILED
            if not pid_alive(directory):
                died = status.get("failed") or {"id": "validator.died", "detail": f"the validator exited in state {state!r} without a final status"}
                print(json.dumps({"state": "died", "failed": died}))
                return EXIT_FAILED
            if state == "ready":
                print(json.dumps({"state": "ready", "driver": status.get("driver")}))
                return EXIT_OK
        time.sleep(0.25)
    print(json.dumps({"state": "timeout", "failed": (read_status(directory) or {}).get("failed")}))
    return EXIT_TIMEOUT


def command_stop(args: argparse.Namespace) -> int:
    """Ask the validator to end its session and clean up; if it cannot, finish the job from here so nothing is left running."""
    directory = Path(args.control_dir)
    (directory / "stop").touch()
    deadline = now() + args.timeout
    while now() < deadline and pid_alive(directory):
        time.sleep(0.2)
    forced = False
    if pid_alive(directory):
        forced = True
        pid = int((directory / "pid").read_text().strip())
        for sig, wait in ((signal.SIGTERM, 25.0), (signal.SIGKILL, 5.0)):
            try:
                os.kill(pid, sig)
            except OSError:
                break
            limit = now() + wait
            while now() < limit and pid_alive(directory):
                time.sleep(0.2)
            if not pid_alive(directory):
                break
    status = read_status(directory) or {"state": "unknown"}
    unfinished = status.get("state") not in ("done", "failed")
    if unfinished and status.get("launcher") == "systemd" and status.get("unit"):
        subprocess.run([args.systemctl, "stop", str(status["unit"]) + ".service"], capture_output=True, timeout=30)
        forced = True
    print(json.dumps({"state": "failed" if unfinished and status.get("state") != "unknown" else status.get("state"), "failed": status.get("failed"),
                      "exit_code": status.get("exit_code"), "forced": forced}))
    return EXIT_OK if status.get("state") == "done" else EXIT_FAILED


def parser() -> argparse.ArgumentParser:
    top = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = top.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="validate reception and hold the session until `stop`")
    run.add_argument("--control-dir", required=True)
    run.add_argument("--target-commit", required=True)
    run.add_argument("--backend", choices=("xdma", "p2"), default="xdma")
    run.add_argument("--expect-rxc1", default="any", help="the RXC1 status the owner is intended to report (valid, unarmed, disabled, unsupported) or any")
    run.add_argument("--url", default="https://127.0.0.1:8443/remote-next?validation=receive&transport=split&display_transport=spectrum")
    run.add_argument("--ready-url", default="http://127.0.0.1:8080/readyz")
    run.add_argument("--perf-url", default="http://127.0.0.1:8080/p23_perf")
    run.add_argument("--perf-json", default="/run/saturn-bridge/perf.json")
    run.add_argument("--unit", action="append", help="a service whose running binary must belong to the target release (repeatable)")
    run.add_argument("--owner-unit", default="saturn-bridge.service", help="the service that owns the FPGA and publishes --perf-json")
    run.add_argument("--auth-unit", default="saturn-go.service", help="the service whose environment holds the proxy credential")
    run.add_argument("--release-root", default="/opt/saturn/releases")
    run.add_argument("--viewer-role", choices=("required", "preferred"), default="required",
                     help="required: the Bridge must assign the viewer role. preferred: also accept the operator role from a Bridge that predates viewer "
                          "enforcement, because the injected guard, the live wire audit and the unchanged-radio check hold the line")
    run.add_argument("--no-exe-check", action="store_true", help="a deployment that predates release directories: trust /readyz alone for the running commit")
    run.add_argument("--tls-cert", default="/var/lib/saturn-state/remote-tls/saturn-remote.crt")
    run.add_argument("--browser", default="/usr/bin/chromium")
    run.add_argument("--launcher", choices=("systemd", "direct"), default="systemd")
    run.add_argument("--memory-max", default="350M")
    run.add_argument("--no-sandbox", action="store_true", help="only inside the sandboxed unit, if the browser's own sandbox cannot start there")
    run.add_argument("--guard-script", default=str(Path(__file__).resolve().with_name("saturn-remote-receive-guard.js")))
    run.add_argument("--systemctl", default="systemctl")
    run.add_argument("--limits", help="JSON object overriding DEFAULT_LIMITS (tests)")
    wait = sub.add_parser("wait")
    wait.add_argument("--control-dir", required=True)
    wait.add_argument("--timeout", type=float, default=150.0)
    stop = sub.add_parser("stop")
    stop.add_argument("--control-dir", required=True)
    stop.add_argument("--timeout", type=float, default=60.0)
    stop.add_argument("--systemctl", default="systemctl")
    return top


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.command == "run":
        args.unit = args.unit or ["saturn-bridge.service", "saturn-go.service"]
        return command_run(args)
    return command_wait(args) if args.command == "wait" else command_stop(args)


if __name__ == "__main__":
    sys.exit(main())
