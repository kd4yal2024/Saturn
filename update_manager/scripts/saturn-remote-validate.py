#!/usr/bin/env python3
# saturn-helper-contract: 1
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
The isolation is verified, not assumed (--isolation required, the default): the browser's own sandbox must be doing its job (no --no-sandbox
on any of its processes, and a seccomp filter confining every renderer, read from /proc) and the unit must carry the configured memory cap
and no swap (read from systemd). A sandbox that was silently switched off, or a cap that never applied, fails the validation.

Sized for a small appliance. The page runs in a background tab and the audio goes to Chrome's own fake output, which is what keeps a
software-rendering browser from using several cores (measured: about 350% of a core down to about 18%, the same frames received and played);
a browser is started only if the host has MemAvailable for the memory cap plus a reserve (host.memory_headroom). Where the kernel has no memory
controller (the Raspberry Pi image boots with cgroup_disable=memory) --memory-enforcement watchdog makes this process the cap: it reads the
browser's proportional memory four times a second and stops the unit above the cap, recorded as memory_enforcement=watchdog and never silent.
Every condition that does not depend on the audio and display counters is read even after one of those fails, and the evidence keeps where the
resources went (diagnostics): memory by process type, CPU, I/O wait, swap, the page's main-thread load, the owner's drop counters.

Restart timing: the services have usually just been restarted (an activation, or the rollback after one), and the first look at such a host is not
the state it settles into: the Bridge writes its first telemetry a few seconds late and the available memory reads low for a moment. The owner's
telemetry (present, fresh, the running service's own) and the memory headroom are therefore waited for within a bound (owner_wait_s and
headroom_wait_s, 30 s each); the wait, and what was seen first and at best, are part of the condition's detail, a stop request ends it at once,
and when the bound passes the validation fails with the reason.

Viewer role: the Bridge must assign the viewer role (--viewer-role required, the default). The one exception is a named compatibility
fallback (--viewer-role legacy-fallback), meant only for a release whose Bridge predates viewer-role enforcement (the release being rolled
back TO, or a running release that was installed before the fix). It is recorded as legacy_fallback_used and in its own condition, never
as an ordinary viewer session; the injected guard must be proven active before it is accepted, and the wire audit and the unchanged-radio
check apply as always.
"""
from __future__ import annotations

import argparse
import base64
import collections
import hashlib
import json
import os
import re
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

# The host must have this much memory available beyond the browser's cap before a browser is started: validating must never push the
# radio's own processes into swap.
HEADROOM_MARGIN_BYTES = 32 * 1024 ** 2       # the default reserve; an administrator may choose another with --memory-reserve (VALIDATION_MEMORY_RESERVE)
HEADROOM_MARGIN_FLOOR_BYTES = 8 * 1024 ** 2     # ... but never less than this

DEFAULT_LIMITS = {
    "ready_url_wait_s": 30.0,     # the services were just restarted: how long the target may take to report itself ready
    "owner_wait_s": 30.0,         # ... and the owner to write its first telemetry (a restarted Bridge creates perf.json a few seconds after it starts)
    "headroom_wait_s": 30.0,      # ... and the host's available memory to climb back to what the browser needs (it dips just after a restart)
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
        with self.lock:
            self.data.update(fields)
        self.write_status()

    def condition(self, identifier: str, ok: bool, detail: str = "") -> bool:
        """Record one verified fact. The first failure is the one reported as the failed condition."""
        with self.lock:
            self.data["conditions"][identifier] = {"ok": bool(ok), "detail": detail, "at": round(now() - self.started, 3)}
            if not ok and self.data["failed"] is None:
                self.data["failed"] = {"id": identifier, "detail": detail}
        self.log("condition", id=identifier, ok=bool(ok), detail=detail)
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
def parse_size(text: str) -> int:
    """systemd's reading of a size: plain bytes, or a number with a K, M, G or T suffix (powers of 1024)."""
    match = re.fullmatch(r"(\d+)([KMGT]?)", text.strip().upper())
    if match is None:
        raise ValueError(f"not a size: {text!r}")
    return int(match.group(1)) * 1024 ** ("KMGT".find(match.group(2)) + 1 if match.group(2) else 0)


def proc_snapshot(proc_root: Path, pid: int) -> dict[str, Any] | None:
    """What /proc says about one browser process: its kind (from its own command line) and whether a seccomp filter confines it."""
    try:
        command_line = (proc_root / str(pid) / "cmdline").read_bytes().decode("utf-8", "replace")
        status = (proc_root / str(pid) / "status").read_text()
    except OSError:
        return None
    # The browser process has one NUL-terminated word per argument; the processes it forks rewrite their title into a single string with the
    # arguments separated by spaces. Splitting on both reads either.
    arguments = [part for part in re.split(r"[\0\s]+", command_line) if part]
    fields = {key.strip(): value.strip() for key, _, value in (line.partition(":") for line in status.splitlines()) if value}
    kind = next((part.split("=", 1)[1] for part in arguments if part.startswith("--type=")), "browser")
    try:
        seccomp = int(fields.get("Seccomp", "-1"))
    except ValueError:
        seccomp = -1
    return {"pid": pid, "type": kind, "seccomp": seccomp, "no_new_privs": fields.get("NoNewPrivs"), "no_sandbox_flag": "--no-sandbox" in arguments}


def descendants(proc_root: Path, root_pid: int) -> list[int]:
    """root_pid and everything below it, from the parent pids in /proc (a browser started directly rather than in a unit)."""
    children: dict[int, list[int]] = {}
    try:
        entries = list(proc_root.iterdir())
    except OSError:
        return [root_pid]
    for entry in entries:
        if not entry.name.isdigit():
            continue
        try:
            parent = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            continue
        children.setdefault(parent, []).append(int(entry.name))
    found, queue = [root_pid], [root_pid]
    while queue:
        for child in children.get(queue.pop(), []):
            found.append(child)
            queue.append(child)
    return found


def host_counters(proc_root: Path) -> dict[str, int]:
    """System-wide numbers from /proc: CPU ticks (busy and waiting for I/O), swap traffic, major faults, free memory and the load. Whatever
    the kernel does not provide is left out (this Pi has no pressure-stall information)."""
    counters: dict[str, int] = {}
    try:
        cpu = [int(value) for value in (proc_root / "stat").read_text().splitlines()[0].split()[1:9]]
        cpu += [0] * (8 - len(cpu))
        counters.update(cpu_user=cpu[0] + cpu[1], cpu_system=cpu[2] + cpu[5] + cpu[6], cpu_idle=cpu[3], cpu_iowait=cpu[4], cpu_steal=cpu[7])
    except (OSError, ValueError, IndexError):
        pass
    try:
        for line in (proc_root / "vmstat").read_text().splitlines():
            key, _, value = line.partition(" ")
            if key in ("pswpin", "pswpout", "pgmajfault"):
                counters[key] = int(value)
    except (OSError, ValueError):
        pass
    try:
        for line in (proc_root / "meminfo").read_text().splitlines():
            key, _, rest = line.partition(":")
            if key in ("MemAvailable", "SwapFree", "SwapTotal"):
                counters[key] = int(rest.split()[0]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    try:
        counters["load1_x100"] = int(float((proc_root / "loadavg").read_text().split()[0]) * 100)
    except (OSError, ValueError, IndexError):
        pass
    return counters


def process_usage(proc_root: Path, pid: int) -> dict[str, int] | None:
    """One browser process: proportional (Pss) and resident memory in bytes, CPU ticks and major page faults. None once it is gone."""
    base = proc_root / str(pid)
    usage: dict[str, int] = {}
    try:
        fields = (base / "stat").read_text().rsplit(")", 1)[1].split()
        usage.update(majflt=int(fields[9]), ticks=int(fields[11]) + int(fields[12]))
    except (OSError, ValueError, IndexError):
        pass
    resident = pss = None
    try:
        for line in (base / "status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                resident = int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    try:
        for line in (base / "smaps_rollup").read_text().splitlines():
            if line.startswith("Pss:"):
                pss = int(line.split()[1]) * 1024
                break
    except (OSError, ValueError, IndexError):
        pass
    if resident is None and pss is None:
        return None
    usage["rss"] = resident if resident is not None else int(pss or 0)
    usage["pss"] = pss if pss is not None else usage["rss"]
    return usage


# What the owner counts when it has to drop something for a client that cannot keep up; read each second while the session is held.
OWNER_DROP_COUNTERS = ("iq_tci_drop_queue_overflow_total", "iq_tci_queue_dropped_deliveries_total", "outbound_drops", "audio_dropped_s",
                       "display_dropped_s", "host_buffer_drops", "connections")
# What the page itself says about its main thread (DevTools Performance.getMetrics; durations are cumulative seconds).
PAGE_METRICS = ("TaskDuration", "ScriptDuration", "LayoutDuration", "RecalcStyleDuration", "JSHeapUsedSize", "JSHeapTotalSize")


def summarize_diagnostics(samples: list[dict[str, Any]], page: list[tuple[float, dict[str, float]]]) -> dict[str, Any]:
    """Where the resources went while the browser ran: peaks, means and totals from the one-second samples and the page's own metrics."""
    def column(key: str) -> list[float]:
        return [float(s[key]) for s in samples if isinstance(s.get(key), (int, float))]

    summary: dict[str, Any] = {"samples": len(samples)}
    for key, name in (("pss_mb", "browser_pss_mb"), ("rss_mb", "browser_rss_mb"), ("browser_cpu_pct", "browser_cpu_pct"),
                      ("host_busy_pct", "host_busy_pct"), ("host_iowait_pct", "host_iowait_pct"), ("load1", "load1")):
        values = column(key)
        if values:
            summary[name] = {"max": round(max(values), 1), "mean": round(sum(values) / len(values), 1)}
    for key, name in (("mem_avail_mb", "mem_available_mb_min"),):
        values = column(key)
        if values:
            summary[name] = round(min(values))
    for key in ("swap_in_pages", "swap_out_pages", "browser_major_faults"):
        values = column(key)
        if values:
            summary[key + "_total"] = int(sum(values))
    heaviest: dict[str, list[float]] = {}
    for s in samples:
        for kind, counted in (s.get("by_type") or {}).items():
            if kind not in heaviest or counted[1] > heaviest[kind][1]:
                heaviest[kind] = [counted[0], counted[1]]
    if heaviest:
        summary["pss_by_process_type_mb_max"] = {kind: {"processes": int(n), "pss_mb": mb} for kind, (n, mb) in sorted(heaviest.items())}
    drops: dict[str, Any] = {}
    firsts = next((s["owner"] for s in samples if isinstance(s.get("owner"), dict)), None)
    lasts = next((s["owner"] for s in reversed(samples) if isinstance(s.get("owner"), dict)), None)
    if firsts and lasts:
        for key in OWNER_DROP_COUNTERS:
            if isinstance(firsts.get(key), (int, float)) and isinstance(lasts.get(key), (int, float)):
                drops[key] = {"first": firsts[key], "last": lasts[key], "delta": lasts[key] - firsts[key]}
    if drops:
        summary["owner_counters"] = drops
    if len(page) >= 2:
        (t0, m0), (t1, m1) = page[0], page[-1]
        wall = t1 - t0
        if wall > 0:
            for key, name in (("TaskDuration", "page_main_thread_busy"), ("ScriptDuration", "page_script_busy")):
                if key in m0 and key in m1:
                    summary[name] = round((m1[key] - m0[key]) / wall, 2)
        if "JSHeapUsedSize" in m1:
            summary["page_js_heap_used_mb_max"] = round(max(m.get("JSHeapUsedSize", 0) for _, m in page) / 1048576, 1)
    return summary


class BrowserWatch(threading.Thread):
    """Four times a second, reads what the browser's processes use. Where the kernel cannot apply a memory cap (the memory cgroup controller
    is disabled on the Raspberry Pi image), this is the cap: the browser's proportional memory is compared with it and the unit is stopped
    above it. Either way it keeps the diagnostics: where the CPU, the memory and the disk went while the session was held."""

    def __init__(self, validator: "Validator", interval: float = 0.25) -> None:
        super().__init__(daemon=True)
        self.validator = validator
        self.interval = interval
        self.halt = threading.Event()
        self.exceeded = threading.Event()
        self.exceeded_detail = ""
        self.cap = parse_size(validator.args.memory_max)
        self.group: str | None = None
        self.samples: list[dict[str, Any]] = []
        self.peak_pss = 0
        self.peak_rss = 0
        self.steps = 0
        self.errors = 0
        self.first_at: float | None = None
        self.last_at: float | None = None
        self.max_gap = 0.0
        self._previous: tuple[float, dict[str, int], int, int] | None = None

    def run(self) -> None:
        while not self.halt.wait(self.interval):
            try:
                self.step()
            except Exception:  # noqa: BLE001  (watching must never take the validation down; the count is part of the evidence)
                self.errors += 1

    def pids(self) -> list[int]:
        validator = self.validator
        if validator.args.launcher != "systemd":
            return validator.browser_pids({})
        if self.group is None:
            self.group = validator.unit_properties(("ControlGroup",)).get("ControlGroup") or None
            if self.group is None:
                return []
        return validator.browser_pids({"ControlGroup": self.group})

    def step(self) -> None:
        validator = self.validator
        moment = time.monotonic()
        proc_root = Path(validator.args.proc_root)
        pairs = [(pid, process_usage(proc_root, pid)) for pid in self.pids()]
        pairs = [(pid, u) for pid, u in pairs if u]
        usage = [u for _, u in pairs]
        if not usage:
            return
        pss, rss = sum(u["pss"] for u in usage), sum(u["rss"] for u in usage)
        self.peak_pss, self.peak_rss = max(self.peak_pss, pss), max(self.peak_rss, rss)
        if self.first_at is None:
            self.first_at = moment
        if self.last_at is not None:
            self.max_gap = max(self.max_gap, moment - self.last_at)
        self.last_at = moment
        self.steps += 1
        if validator.memory_enforcement == "watchdog" and pss > self.cap and not self.exceeded.is_set():
            self.exceeded_detail = f"the browser's processes use {pss / 1048576:.0f} MiB (proportional set size) against the {validator.args.memory_max} cap"
            validator.recorder.condition("browser.memory_within_cap", False, "MEMORY CAP EXCEEDED, enforced by the watchdog: " + self.exceeded_detail)
            self.exceeded.set()
            validator.kill_browser()
        if self.steps % max(1, round(1.0 / self.interval)) == 0:
            self.sample(moment, pairs, pss, rss)

    def sample(self, moment: float, pairs: list[tuple[int, dict[str, int]]], pss: int, rss: int) -> None:
        validator = self.validator
        proc_root = Path(validator.args.proc_root)
        host = host_counters(proc_root)
        usage = [u for _, u in pairs]
        ticks, majflt = sum(u.get("ticks", 0) for u in usage), sum(u.get("majflt", 0) for u in usage)
        kinds: dict[str, list[float]] = {}
        for pid, u in pairs:
            snapshot = proc_snapshot(proc_root, pid)
            kind = snapshot["type"] if snapshot else "unknown"
            counted = kinds.setdefault(kind, [0, 0.0])
            counted[0] += 1
            counted[1] += u["pss"] / 1048576
        entry: dict[str, Any] = {"t": round(now() - validator.recorder.started, 1), "procs": len(usage),
                                 "pss_mb": round(pss / 1048576, 1), "rss_mb": round(rss / 1048576, 1),
                                 "by_type": {kind: [int(n), round(mb, 1)] for kind, (n, mb) in sorted(kinds.items())}}
        if "MemAvailable" in host:
            entry["mem_avail_mb"] = round(host["MemAvailable"] / 1048576)
        if "SwapTotal" in host and "SwapFree" in host:
            entry["swap_used_mb"] = round((host["SwapTotal"] - host["SwapFree"]) / 1048576)
        if "load1_x100" in host:
            entry["load1"] = host["load1_x100"] / 100
        if self._previous is not None:
            before_at, before, before_ticks, before_majflt = self._previous
            seconds = moment - before_at
            if seconds > 0 and ticks >= before_ticks:
                entry["browser_cpu_pct"] = round(100 * (ticks - before_ticks) / os.sysconf("SC_CLK_TCK") / seconds)
            names = ("cpu_user", "cpu_system", "cpu_idle", "cpu_iowait", "cpu_steal")
            if all(name in host and name in before for name in names):
                total = sum(host[name] - before[name] for name in names)
                if total > 0:
                    entry["host_busy_pct"] = round(100 * (total - (host["cpu_idle"] - before["cpu_idle"]) - (host["cpu_iowait"] - before["cpu_iowait"])) / total)
                    entry["host_iowait_pct"] = round(100 * (host["cpu_iowait"] - before["cpu_iowait"]) / total)
            for key, name in (("pswpin", "swap_in_pages"), ("pswpout", "swap_out_pages")):
                if key in host and key in before:
                    entry[name] = host[key] - before[key]
            entry["browser_major_faults"] = max(0, majflt - before_majflt)
        self._previous = (moment, host, ticks, majflt)
        try:
            metrics = validator.owner.perf().get("metrics", {})
            entry["owner"] = {key: metrics[key] for key in OWNER_DROP_COUNTERS if isinstance(metrics.get(key), (int, float))}
        except (OSError, ValueError):
            pass
        if len(self.samples) < 400:
            self.samples.append(entry)


def sandbox_verdict(processes: list[dict[str, Any]]) -> tuple[bool, str]:
    """The browser's sandbox is intact when no process was started without it and every renderer is confined by a seccomp filter
    (Seccomp: 2). With --no-sandbox the renderers show Seccomp: 0, which is what makes this evidence rather than a flag check."""
    if any(p["no_sandbox_flag"] for p in processes):
        return False, "the browser is running with --no-sandbox"
    renderers = [p for p in processes if p["type"] == "renderer"]
    if not renderers:
        return False, f"no renderer process was found to inspect among {len(processes)} browser process(es)"
    loose = [f"pid {p['pid']} Seccomp:{p['seccomp']}" for p in renderers if p["seccomp"] != 2]
    if loose:
        return False, "renderer(s) not confined by a seccomp filter: " + ", ".join(loose)
    return True, f"no process runs with --no-sandbox and all {len(renderers)} renderer process(es) are confined by a seccomp filter (Seccomp: 2)"


def memory_verdict(props: dict[str, str], configured: str) -> tuple[bool, str]:
    """The unit carries the configured memory cap and no swap."""
    wanted = parse_size(configured)
    limit, swap = props.get("MemoryMax", ""), props.get("MemorySwapMax", "")
    if not limit.isdigit():
        return False, f"the browser has no memory cap (MemoryMax={limit or 'unreadable'!r}); {configured} = {wanted} bytes was configured"
    if int(limit) != wanted:
        return False, f"MemoryMax is {limit} bytes but {configured} = {wanted} bytes was configured"
    if swap != "0":
        return False, f"the browser's unit may use swap (MemorySwapMax={swap or 'unreadable'!r}), which would hide memory pressure on this host"
    return True, f"MemoryMax {limit} bytes ({configured}) and MemorySwapMax 0 are applied to the browser's unit; {props.get('MemoryCurrent', 'unknown')} bytes in use"


def watchdog_memory_verdict(props: dict[str, str], configured: str, watch: dict[str, Any]) -> tuple[bool, str]:
    """Where the kernel has no memory controller the cap is enforced by this process. That counts as applied only while the unit is the OOM
    killer's first choice and the browser's memory is really being read, without gaps, and has not exceeded the cap."""
    wanted = parse_size(configured)
    if props.get("OOMScoreAdjust") != "1000":
        return False, f"the browser's unit is not the OOM killer's first choice (OOMScoreAdjust={props.get('OOMScoreAdjust', 'unreadable')!r}, wanted 1000)"
    if watch["exceeded"]:
        return False, "the browser exceeded its memory cap and was stopped: " + str(watch.get("detail", ""))
    if watch["steps"] < 2 or watch["peak"] <= 0:
        return False, f"the browser's memory could not be measured ({watch['steps']} reading(s)), so the cap was not being enforced"
    if watch["age"] > 2.0:
        return False, f"the memory watchdog stopped reading {watch['age']:.1f} s ago"
    if watch["max_gap"] > 3.0:
        return False, f"the memory watchdog went {watch['max_gap']:.1f} s without a reading"
    if watch["peak"] > wanted:
        return False, f"the browser used {watch['peak'] / 1048576:.0f} MiB against the {configured} cap"
    return True, (f"MEMORY CAP ENFORCED BY THE VALIDATOR'S WATCHDOG, not by the kernel (the memory cgroup controller is not enabled here): the browser's "
                  f"proportional memory is read {int(1 / 0.25)} times a second and its unit is stopped above {configured}; OOMScoreAdjust=1000 makes it the OOM "
                  f"killer's first choice; peak so far {watch['peak'] / 1048576:.0f} MiB over {watch['steps']} readings")


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
        "--window-size=800,500", "--force-device-scale-factor=1", "--disable-audio-output", "--js-flags=--max-old-space-size=160",
        # a small appliance: fewer, lighter rendering threads and no software 3D rasteriser (measured: half the CPU and about 50 MiB less)
        "--enable-low-end-device-mode", "--disable-software-rasterizer", "--num-raster-threads=1",
    ]
    if spki:
        flags.append(f"--ignore-certificate-errors-spki-list={spki}")
    if not sandbox:
        flags.append("--no-sandbox")
    return flags + ["about:blank"]


# The DevTools pipe is file descriptors 3 (commands in) and 4 (replies out). Both launchers hand the browser a pair of ordinary pipes as its
# stdin/stdout and map them to 3 and 4 here, so the same code path serves a transient unit and a direct child.
PIPE_WRAPPER = 'exec "$@" 3<&0 4>&1 0</dev/null 1>&2'


def build_launch_command(mode: str, browser: str, flags: list[str], unit: str, max_seconds: float, memory_max: str,
                         systemd_run: str = "systemd-run") -> list[str]:
    inner = ["/bin/sh", "-c", PIPE_WRAPPER, "sh", browser, *flags]
    if mode == "direct":
        return inner
    if mode != "systemd":
        raise ValueError(f"unknown launcher {mode!r}")
    properties = [
        "DynamicUser=yes", "PrivateTmp=yes", "PrivateDevices=yes", "ProtectSystem=strict", "ProtectHome=yes", "NoNewPrivileges=yes",
        "ProtectKernelTunables=yes", "ProtectControlGroups=yes", "RestrictSUIDSGID=yes", "LockPersonality=yes",
        "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK", "IPAddressDeny=any", "IPAddressAllow=localhost",
        f"MemoryMax={memory_max}", "MemorySwapMax=0", "OOMScoreAdjust=1000", "TasksMax=300", "CPUWeight=20", "Nice=10", "IOSchedulingClass=idle",
        f"RuntimeMaxSec={int(max_seconds) + 30}", "Environment=HOME=/tmp", "Environment=PULSE_SERVER=none", "WorkingDirectory=/tmp",
    ]
    command = [systemd_run, "--quiet", "--collect", "--pipe", f"--unit={unit}"]
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
  decodedFrames: rxOpus.decodedFrames, decoderFramesOutput: rxOpus.decoderFramesOutput, malformedFrames: rxOpus.malformedFrames, resyncs: rxOpus.resyncs,
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
        self.memory_enforcement = self.choose_memory_enforcement()
        self.watch: BrowserWatch | None = None
        self.page_metrics: list[tuple[float, dict[str, float]]] = []
        self._page_metrics_at = 0.0

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

    def choose_memory_enforcement(self) -> str:
        """cgroup: the kernel applies MemoryMax. watchdog: it cannot (the memory controller is not enabled on this kernel), so this process
        enforces the cap itself and says so. Explicit on the command line; 'auto' asks the kernel and assumes cgroup when it cannot tell."""
        choice = getattr(self.args, "memory_enforcement", "auto")
        if choice != "auto":
            return choice
        try:
            controllers = (Path(self.args.cgroup_root) / "cgroup.controllers").read_text().split()
        except OSError:
            return "cgroup"
        return "cgroup" if "memory" in controllers else "watchdog"

    def kill_browser(self) -> None:
        """Stop the browser's unit (or process group) at once. Used by the memory watchdog from its own thread."""
        try:
            if self.args.launcher == "systemd":
                subprocess.run([self.args.systemctl, "kill", "--signal=SIGKILL", self.unit + ".service"], capture_output=True, timeout=10)
            else:
                self._kill_group(signal.SIGKILL)
        except (OSError, subprocess.SubprocessError):
            pass
        self.recorder.log("browser_killed_by_memory_watchdog")

    def note_page_metrics(self) -> None:
        """Once a second at most: the page's own account of its main thread. Diagnostic only; it can never fail a validation."""
        if self.cdp is None or self.session is None or self.cdp.closed.is_set() or self.cdp.unresponsive or time.monotonic() - self._page_metrics_at < 1.0:
            return
        self._page_metrics_at = time.monotonic()
        try:
            reply = self.cdp.call("Performance.getMetrics", session=self.session, timeout=3)
        except CdpError:
            return
        wanted = {m["name"]: float(m["value"]) for m in reply.get("metrics", []) if m.get("name") in PAGE_METRICS and isinstance(m.get("value"), (int, float))}
        if wanted and len(self.page_metrics) < 400:
            self.page_metrics.append((time.monotonic(), wanted))

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

    def owner_view(self) -> tuple[dict[str, Any] | None, str]:
        """One coherent look at the owner: its telemetry, and whether that telemetry belongs to the running service. Records nothing; when
        the telemetry cannot be read, the second item says why."""
        try:
            perf = self.owner.perf()
        except (OSError, ValueError) as error:
            return None, f"cannot read {self.args.perf_json}: {error}"
        metrics = perf.get("metrics", {})
        age = (now() * 1000 - perf.get("updated_at_ms", 0)) / 1000.0
        pid = self.owner.main_pid(self.args.owner_unit)
        return {"perf": perf, "metrics": metrics, "age_s": age, "pid_matches": pid is not None and metrics.get("pid") == pid, "service_pid": pid}, ""

    def read_owner(self) -> dict[str, Any] | None:
        view, problem = self.owner_view()
        if view is None:
            self.fail("owner.fresh_telemetry", problem)
        return view

    def check_memory_headroom(self) -> bool:
        """Before the browser is started: the host has the memory free that the cap lets the browser take, plus a margin. Where it has not, the
        validation is refused rather than started, so the radio's processes are never the ones that pay for it."""
        if self.args.isolation != "required":
            return True
        reserve = parse_size(self.args.memory_reserve)
        if reserve < HEADROOM_MARGIN_FLOOR_BYTES:
            return self.recorder.condition("host.memory_headroom", False, f"the memory reserve {self.args.memory_reserve} is below the {HEADROOM_MARGIN_FLOOR_BYTES // 1048576} MiB minimum, so the check cannot be trusted")
        wanted = parse_size(self.args.memory_max) + reserve
        unreadable = "the host's available memory could not be read, so it cannot be shown that the browser fits"
        available = first = best = host_counters(Path(self.args.proc_root)).get("MemAvailable")
        if available is None:
            return self.recorder.condition("host.memory_headroom", False, unreadable)
        # Just after the services were restarted (an activation, or the rollback after one) the host reads lower than it does at rest, and climbs
        # back within seconds: wait, within a bound, for the memory instead of refusing at the first reading. Never longer than the bound, and
        # a browser is not started until the memory is there.
        started, waited_for = now(), False
        while available < wanted and now() - started < self.limits["headroom_wait_s"] and not self.stop_requested.is_set():
            waited_for = True
            self.sleep(self.limits["poll_s"])
            available = host_counters(Path(self.args.proc_root)).get("MemAvailable")
            if available is None:
                return self.recorder.condition("host.memory_headroom", False, unreadable)
            best = max(best, available)
        if available < wanted and self.stop_requested.is_set():
            return self.interrupted("host.memory_headroom", "waiting for the host to have the memory the browser needs")
        text = f"{available / 1048576:.0f} MiB available; the browser may take up to {self.args.memory_max} and {reserve // 1048576} MiB are kept in reserve"
        if waited_for:
            text += (f" (after waiting {now() - started:.1f} s: {first / 1048576:.0f} MiB at first"
                     + ("" if available >= wanted else f", {best / 1048576:.0f} MiB at the most") + ")")
        if available < wanted:
            text += f" ({wanted / 1048576:.0f} MiB needed): no browser was started"
        return self.recorder.condition("host.memory_headroom", available >= wanted, text)

    def baseline(self) -> bool:
        """The owner's telemetry before the session: there, fresh, and the running service's own. The services were just restarted (an activation,
        or the rollback after one) and a restarted Bridge writes its first telemetry a few seconds after it starts, so this waits, within a bound,
        instead of failing at the first look. If it never comes right the condition says what was seen last and how long it waited."""
        started = now()
        polls = 0
        while True:
            view, problem = self.owner_view()
            if view is not None and 0 <= view["age_s"] <= 5 and view["pid_matches"]:
                break
            if now() - started >= self.limits["owner_wait_s"] or self.stop_requested.is_set():
                break
            polls += 1
            self.sleep(self.limits["poll_s"])
        if self.stop_requested.is_set() and not (view is not None and 0 <= view["age_s"] <= 5 and view["pid_matches"]):
            return self.interrupted("owner.fresh_telemetry", "waiting for the owner's telemetry")
        waited = f" (after waiting {now() - started:.1f} s for the restarted owner's telemetry)" if polls else ""
        if view is None:
            self.recorder.condition("owner.fresh_telemetry", False, problem + waited)
            return False
        ok = 0 <= view["age_s"] <= 5 and view["pid_matches"]
        self.recorder.condition("owner.fresh_telemetry", ok, f"before the session: age {view['age_s']:.2f} s, telemetry pid {view['metrics'].get('pid')}, service pid {view['service_pid']}{waited}")
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
        command = build_launch_command(self.args.launcher, self.args.browser, flags, self.unit, self.limits["max_seconds"], self.args.memory_max,
                                       self.args.systemd_run)
        self.recorder.log("launch", launcher=self.args.launcher, unit=self.unit, argv=command)
        self.recorder.update(launcher=self.args.launcher, unit=self.unit, role_policy=self.args.viewer_role, legacy_fallback_used=False,
                             sandbox_requested=not self.args.no_sandbox, memory_max_configured=self.args.memory_max)
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
        self.recorder.update(memory_enforcement=self.memory_enforcement)
        self.watch = BrowserWatch(self)
        self.watch.start()
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
        target_id = page["targetId"]
        # The page being validated runs in a background tab; the tab the browser opened stays in front. A tab nobody can see is not painted or
        # composited, which is where a software-rendering browser spends its CPU (and the Pi has little). The page's data, its decoder and its
        # audio are the page's own and run as before; the page_visibility recorded below shows it really was hidden.
        try:
            target_id = cdp.call("Target.createTarget", {"url": "about:blank", "background": True})["targetId"]
        except (CdpError, KeyError) as error:
            self.recorder.log("background_tab_unavailable", error=str(error))
        self.session = cdp.call("Target.attachToTarget", {"targetId": target_id, "flatten": True})["sessionId"]
        for domain in ("Page", "Runtime", "Network", "Log"):
            cdp.call(f"{domain}.enable", session=self.session)
        try:
            cdp.call("Performance.enable", session=self.session)
        except CdpError:
            pass  # diagnostics only
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
                try:
                    visibility = self.evaluate("document.visibilityState", timeout=5)
                except CdpError:
                    visibility = None
                self.recorder.update(driver=self.driver, page_visibility=visibility)
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
                if role == "operator" and self.args.viewer_role == "legacy-fallback":
                    # check_guard() has already proven the guard active: a run whose guard is not never gets as far as starting a session
                    self.recorder.update(legacy_fallback_used=True)
                    return self.recorder.condition(
                        "session.role_legacy_fallback", True,
                        "LEGACY BRIDGE FALLBACK USED: the Bridge assigned the role 'operator' because this release predates viewer-role enforcement. "
                        "Receive-only rests on the injected guard (proven active), the live wire audit and the unchanged-radio check, not on the Bridge")
                return self.recorder.condition("session.connected_as_viewer", False, f"connected; the Bridge assigned role {role!r} (a validation session must be a viewer)")
            self.sleep(self.limits["poll_s"])
        if self.stop_requested.is_set():
            return self.interrupted("session.connected_as_viewer", "the session was connecting")
        return self.recorder.condition("session.connected_as_viewer", False,
                                       f"not connected within {self.limits['connect_s']:g} s (connected={snap.get('connected')}, bridgeReady={snap.get('bridgeReady')}, pending={snap.get('connectPending')})")

    def unit_properties(self, names: tuple[str, ...]) -> dict[str, str]:
        result = subprocess.run([self.args.systemctl, "show", *[f"--property={name}" for name in names], self.unit + ".service"],
                                capture_output=True, text=True, timeout=10)
        return {key: value for key, separator, value in (line.partition("=") for line in result.stdout.splitlines()) if separator}

    def browser_pids(self, props: dict[str, str]) -> list[int]:
        """Every process of the browser: the unit's cgroup when it runs in a unit, otherwise the tree below the process we started."""
        proc_root = Path(self.args.proc_root)
        if self.args.launcher == "systemd":
            group = props.get("ControlGroup", "")
            if not group:
                return []
            try:
                text = (Path(self.args.cgroup_root) / group.lstrip("/") / "cgroup.procs").read_text()
            except OSError:
                return []
            return [int(line) for line in text.split() if line.isdigit()]
        return descendants(proc_root, self.process.pid) if self.process is not None else []

    def check_isolation(self) -> bool:
        """Read, from the host, that the browser's sandbox and memory cap are really in force while the session is running."""
        if self.args.isolation == "off":
            self.recorder.log("isolation_not_checked")
            return True
        self.recorder.update(phase="isolation")
        props: dict[str, str] = {}
        if self.args.launcher == "systemd":
            try:
                props = self.unit_properties(("ControlGroup", "MemoryMax", "MemorySwapMax", "MemoryCurrent", "ActiveState", "OOMScoreAdjust"))
            except (OSError, subprocess.SubprocessError) as error:
                self.recorder.log("unit_properties_failed", error=str(error))
        pids = self.browser_pids(props)
        processes = [p for p in (proc_snapshot(Path(self.args.proc_root), pid) for pid in pids) if p]
        sandbox_ok, sandbox_detail = sandbox_verdict(processes)
        if self.args.launcher == "systemd" and self.memory_enforcement == "watchdog":
            memory_ok, memory_detail = watchdog_memory_verdict(props, self.args.memory_max, self.watch_state())
        elif self.args.launcher == "systemd":
            memory_ok, memory_detail = memory_verdict(props, self.args.memory_max)
        else:
            memory_ok, memory_detail = False, "the browser was not started in a systemd unit, so no memory cap is applied to it"
        self.recorder.update(isolation={
            "processes": [{k: p[k] for k in ("pid", "type", "seccomp", "no_new_privs")} for p in processes],
            "memory": {"configured": self.args.memory_max, "cap_bytes": parse_size(self.args.memory_max), "enforcement": self.memory_enforcement,
                       "max_bytes": props.get("MemoryMax"), "swap_max": props.get("MemorySwapMax"), "current_bytes": props.get("MemoryCurrent"),
                       "oom_score_adjust": props.get("OOMScoreAdjust")},
        })
        ok = self.recorder.condition("browser.sandbox_intact", sandbox_ok, sandbox_detail)
        if self.args.isolation == "required":
            ok &= self.recorder.condition("browser.memory_limit_applied", memory_ok, memory_detail)
        return bool(ok)

    def watch_state(self) -> dict[str, Any]:
        watch = self.watch
        if watch is None or watch.last_at is None:
            return {"steps": 0, "max_gap": 0.0, "age": float("inf"), "peak": 0, "exceeded": False, "errors": 0}
        return {"steps": watch.steps, "max_gap": watch.max_gap, "age": time.monotonic() - watch.last_at, "peak": watch.peak_pss,
                "exceeded": watch.exceeded.is_set(), "detail": watch.exceeded_detail, "errors": watch.errors}

    def record_memory_peak(self) -> None:
        """How close the browser came to its cap. The kernel's figure when it keeps one (systemd 255 and a memory controller); otherwise what
        this process measured (proportional set size), and which of the two it is."""
        if self.args.isolation == "off":
            return
        props: dict[str, str] = {}
        if self.args.launcher == "systemd":
            try:
                props = self.unit_properties(("MemoryPeak", "MemoryCurrent"))
            except (OSError, subprocess.SubprocessError):
                pass
        isolation = dict(self.recorder.data.get("isolation") or {})
        memory = dict(isolation.get("memory") or {})
        peak = props.get("MemoryPeak", "")
        if peak.isdigit():
            memory.update(peak_bytes=peak, peak_source="systemd MemoryPeak")
        elif self.watch is not None and self.watch.peak_pss:
            memory.update(peak_bytes=str(self.watch.peak_pss), peak_source="validator sampling (proportional set size)")
        else:
            memory.update(peak_bytes=peak or None)
        memory["current_bytes_at_end"] = props.get("MemoryCurrent")
        isolation["memory"] = memory
        self.recorder.update(isolation=isolation)

    def finish_watch(self) -> None:
        """Stop reading, keep what was read, and (where the validator is the memory cap) say whether the browser stayed within it."""
        watch = self.watch
        if watch is None:
            return
        watch.halt.set()
        watch.join(timeout=3)
        self.recorder.update(diagnostics={
            "summary": summarize_diagnostics(watch.samples, self.page_metrics), "samples": watch.samples,
            "watch": {"readings": watch.steps, "errors": watch.errors, "longest_gap_s": round(watch.max_gap, 2),
                      "peak_pss_bytes": watch.peak_pss, "peak_rss_bytes": watch.peak_rss}})
        if self.memory_enforcement == "watchdog" and self.args.launcher == "systemd" and self.args.isolation == "required":
            within = not watch.exceeded.is_set() and watch.peak_pss <= watch.cap and watch.steps >= 2
            self.recorder.condition(
                "browser.memory_within_cap", within,
                f"the browser's peak was {watch.peak_pss / 1048576:.0f} MiB (proportional) against the {self.args.memory_max} cap, over {watch.steps} readings"
                if within else (watch.exceeded_detail or f"not enough readings ({watch.steps}) to show the browser stayed within {self.args.memory_max}"))

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
            self.note_page_metrics()
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
        if self.watch is not None and self.watch.exceeded.is_set():
            return True
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
            self.note_page_metrics()
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
        self.record_memory_peak()
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
        self.finish_watch()
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

    def browser_still_running(self) -> bool:
        return (self.process is not None and self.process.poll() is None and self.cdp is not None and not self.cdp.closed.is_set()
                and not self.stop_requested.is_set() and not self.timed_out.is_set()
                and not (self.watch is not None and self.watch.exceeded.is_set()))        # a browser stopped for its memory is not read further

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
            if not self.check_release() or not self.baseline() or not self.check_memory_headroom():
                raise _Abort
            self.launch()
            data_ok = self.attach_and_navigate() and self.check_guard() and self.start_session() and self.measure()
            if not data_ok and not self.browser_still_running():
                raise _Abort
            # The sandbox, the memory cap, the owner's view, the wire and the radio do not depend on the audio and display counters: read them
            # even when one of those failed, so one run says everything it can. The first failure is still the one reported as the failed condition.
            isolated = self.check_isolation()
            owned = self.check_owner_and_wire()
            if not (data_ok and isolated and owned):
                raise _Abort
            self.hold()
        except _Abort:
            pass
        except CdpError as error:
            if not self.alarm():
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
    run.add_argument("--viewer-role", choices=("required", "legacy-fallback"), default="required",
                     help="required: the Bridge must assign the viewer role. legacy-fallback: ALSO accept the operator role, only for a release whose "
                          "Bridge predates viewer-role enforcement; recorded as legacy_fallback_used, and the injected guard must be proven active")
    run.add_argument("--no-exe-check", action="store_true", help="a deployment that predates release directories: trust /readyz alone for the running commit")
    run.add_argument("--tls-cert", default="/var/lib/saturn-state/remote-tls/saturn-remote.crt")
    run.add_argument("--browser", default="/usr/bin/chromium")
    run.add_argument("--launcher", choices=("systemd", "direct"), default="systemd")
    run.add_argument("--memory-max", default="350M")
    run.add_argument("--memory-reserve", default="32M", help="memory kept free beyond the browser's cap before a browser is started (at least 8M)")
    run.add_argument("--memory-enforcement", choices=("auto", "cgroup", "watchdog"), default="auto",
                     help="cgroup: the kernel applies MemoryMax. watchdog: the kernel cannot (no memory controller, e.g. cgroup_disable=memory on the "
                          "Raspberry Pi image), so this process reads the browser's proportional memory and stops its unit above the cap; recorded as "
                          "such. auto: cgroup when the kernel offers a memory controller, watchdog when it plainly does not")
    run.add_argument("--no-sandbox", action="store_true", help="test rigs only: under --isolation required it fails browser.sandbox_intact")
    run.add_argument("--isolation", choices=("required", "sandbox", "off"), default="required",
                     help="required: verify from the host that the browser's sandbox is in force and its unit carries the memory cap (the update "
                          "process always uses this). sandbox: the sandbox only, for a rig that runs a real browser outside a unit. off: only for "
                          "rigs that run a stand-in browser")
    run.add_argument("--systemd-run", default="systemd-run", help=argparse.SUPPRESS)
    run.add_argument("--proc-root", default="/proc", help=argparse.SUPPRESS)
    run.add_argument("--cgroup-root", default="/sys/fs/cgroup", help=argparse.SUPPRESS)
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
