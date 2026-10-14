#!/usr/bin/env python3
"""Tests for update_manager/scripts/saturn-remote-validate.py.

The validator is exercised as the update process runs it: as a separate program, started in the background, waited for, held, then stopped.
Everything it talks to is replaced by a fake that behaves like the real thing in the one way that matters here:

  * a fake Chromium that speaks DevTools over file descriptors 3 and 4 and follows a scripted scenario;
  * a fake owner: /readyz and /p23_perf over HTTP and a perf.json that is regenerated continuously. The owner's connection count follows
    whether the fake browser PROCESS IS ALIVE and connected, so "cleaned up" can only be true if the browser really went away;
  * fake services (real processes whose binaries live in the target release directory, one with the proxy credential in its environment);
  * a fake systemctl.
"""
import hashlib
import http.server
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
VALIDATOR = REPO / "update_manager/scripts/saturn-remote-validate.py"
GUARD = REPO / "update_manager/scripts/saturn-remote-receive-guard.js"
COMMIT = "f10a087fc9f949b5cf8eb31a781ce74d23512581"
OTHER_COMMIT = "329f3109b17428ee04e3f14031da6cf25cb04810"
SECRET = "saturn:Pw-7Hq2-zebra-TANGO-41"
SECRET_PASSWORD = SECRET.split(":", 1)[1]
FAST = {"ready_url_wait_s": 3, "page_load_s": 8, "connect_s": 5, "data_s": 6, "window_s": 1.0, "hold_stall_s": 2.0, "max_seconds": 40,
        "cleanup_s": 4, "call_s": 4, "poll_s": 0.2, "min_audio_played": 20, "min_opus_frames": 20, "min_display": 5,
        "owner_wait_s": 0.6, "headroom_wait_s": 0.6}

spec = importlib.util.spec_from_file_location("saturn_remote_validate", VALIDATOR)
V = importlib.util.module_from_spec(spec)
sys.modules["saturn_remote_validate"] = V
spec.loader.exec_module(V)

FAKE_BROWSER = r'''#!%(python)s
import json, os, signal, sys, threading, time
W = os.environ["FAKE_WORLD"]
SC = json.loads(os.environ.get("FAKE_SCENARIO", "{}"))
lock = threading.Lock()
def out(message):
    with lock:
        os.write(4, json.dumps(message).encode() + b"\0")
def put(name, value):
    with open(os.path.join(W, name), "w") as handle:
        json.dump(value, handle)
def session(connected):
    put("session.json", {"pid": os.getpid(), "connected": connected})
put("browser_argv.json", {"argv": sys.argv, "env": dict(os.environ)})
FAKE_PIDS, PARENTS = [], {}
T0 = time.time()
def usage_loop():
    """What a running browser's processes and the host would show, changing over time: memory (growing when the scenario says so), CPU ticks,
    the host's CPU, swap and fault counters."""
    while True:
        age = time.time() - T0
        pss_kb = SC.get("pss_kb", 40000) + int(SC.get("pss_growth_kb_s", 0) * age)
        for pid in list(FAKE_PIDS):
            d = os.path.join(W, "proc", str(pid))
            ticks = int(SC.get("cpu_ticks_per_s", 25) * age)
            with open(os.path.join(d, "stat"), "w") as h:
                h.write(f"{pid} (chromium) S {PARENTS.get(pid, 1)} 0 0 0 0 0 0 0 {int(age)} 0 {ticks} {ticks // 5} 0 0\n")
            if not SC.get("no_smaps") and not SC.get("no_usage"):
                with open(os.path.join(d, "smaps_rollup"), "w") as h: h.write(f"00400000-ffffffff ---p 00000000 00:00 0 [rollup]\nRss: 1 kB\nPss: {pss_kb} kB\n")
        host = os.path.join(W, "proc")
        with open(os.path.join(host, "stat"), "w") as h:
            h.write(f"cpu  {int(100 * age)} 0 {int(20 * age)} {int(180 * age)} {int(100 * age)} 0 0 0 0 0\n")
        with open(os.path.join(host, "vmstat"), "w") as h: h.write(f"pgmajfault {int(3 * age)}\npswpin {int(5 * age)}\npswpout {int(2 * age)}\n")
        with open(os.path.join(host, "meminfo"), "w") as h: h.write("MemTotal: 927000 kB\nMemAvailable: 500000 kB\nSwapTotal: 3000000 kB\nSwapFree: 2700000 kB\n")
        with open(os.path.join(host, "loadavg"), "w") as h: h.write("0.80 0.70 0.60 1/200 1234\n")
        time.sleep(0.2)
def fake_tree():
    """What /proc and the unit's cgroup would show for a real browser: a main process, GPU, and renderers (Seccomp: 2 when sandboxed)."""
    me = os.getpid()
    def entry(pid, ppid, kind, seccomp):
        d = os.path.join(W, "proc", str(pid)); os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "stat"), "w") as h: h.write(f"{pid} (chromium) S {ppid} 0 0\n")
        args = ["/usr/bin/chromium"] + ([f"--type={kind}"] if kind != "browser" else [])
        if "--no-sandbox" in sys.argv or SC.get("browser_no_sandbox"): args.append("--no-sandbox")
        # as on a real host: the browser process has NUL-separated arguments, the processes it forks one space-separated string
        with open(os.path.join(d, "cmdline"), "wb") as h:
            h.write(b"\0".join(a.encode() for a in args) + b"\0" if kind == "browser" else " ".join(args).encode() + b"\0")
        with open(os.path.join(d, "status"), "w") as h: h.write(f"Name:\tchromium\nSeccomp:\t{seccomp}\nNoNewPrivs:\t1\n" + ("" if SC.get("no_usage") else f"VmRSS:\t{SC.get('rss_kb', 50000)} kB\n"))
        parents[pid] = ppid
    parents = {}
    entry(me, os.getppid(), "browser", 0)
    pids = [me]
    for i in range(SC.get("renderers", 2)):
        entry(900001 + i, me, "renderer", SC.get("renderer_seccomp", 2)); pids.append(900001 + i)
    entry(900100, me, "gpu-process", 0); pids.append(900100)
    FAKE_PIDS.extend(pids); PARENTS.update(parents)
    cg = os.path.join(W, "cgroup", "system.slice", "fake.service"); os.makedirs(cg, exist_ok=True)
    with open(os.path.join(cg, "cgroup.procs"), "w") as h: h.write("\n".join(str(p) for p in pids) + "\n")
fake_tree()
usage_loop_thread = threading.Thread(target=usage_loop, daemon=True); usage_loop_thread.start()
session(False)
if SC.get("stubborn"):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
st = {"loaded": False, "connected_at": None, "snapshots": 0, "stopped": False, "challenge": None}
def elapsed():
    return 0.0 if st["connected_at"] is None else max(0.0, time.time() - st["connected_at"])
def frame(data, opcode=1):
    out({"method": "Network.webSocketFrameSent", "params": {"requestId": "ws1", "timestamp": time.time(), "response": {"opcode": opcode, "mask": True, "payloadData": data}}})
def snapshot():
    st["snapshots"] += 1
    if SC.get("hang_on_snapshot") and st["snapshots"] > SC["hang_on_snapshot"]:
        time.sleep(3600)
    e = elapsed(); up = st["connected_at"] is not None and time.time() >= st["connected_at"] and not st["stopped"]
    ed = max(0.0, e - SC.get("data_delay", 0.0))
    connected = up and SC.get("mode") != "no_connect"
    if SC.get("drop_after") is not None and connected and e > SC["drop_after"]:
        connected = False
    played = 0 if not connected else int(50 * ed)
    if SC.get("audio_stalled"): played = 3 if connected else 0
    if SC.get("audio_stall_after") is not None and connected: played = int(50 * min(e, SC["audio_stall_after"]))
    opus = 0 if (not connected or SC.get("no_audio_frames")) else int(50 * ed)
    rows = 0 if (not connected or SC.get("no_display")) else int(30 * ed)
    return {"mode": "legacy" if SC.get("legacy") else "receive", "at": int(time.time() * 1000), "path": "/remote-next",
            "connected": connected, "bridgeReady": connected and not SC.get("bridge_not_ready"), "connectPending": not connected and st["connected_at"] is not None,
            "role": SC.get("role", "viewer") if connected else None, "wsReadyState": 1 if connected else 3,
            "audioContextState": SC.get("audio_ctx", "running") if connected else "suspended",
            "codec": SC.get("codec", "opus") if connected else None, "opusFrames": opus, "pcmFrames": 0,
            "decodeErrors": int(5 * e) if SC.get("decode_errors") else 0, "lateDrops": 0, "audioFramesPlayed": played, "lastAudioSequence": played,
            "audioSeqGaps": 0, "underruns": 0, "overflows": 0, "queuedMs": 40, "renderSource": "server", "displayRows": rows, "iqFrames": 0, "rxIqFrames": 0,
            "blockedCounts": {}, "blockedExamples": [], "violations": SC.get("violations", []), "errors": SC.get("page_errors", [])}
def start():
    if SC.get("start_refused"):
        return json.dumps({"ok": False, "reason": "split transport required"})
    st["connected_at"] = time.time() + SC.get("connect_delay", 0.3)
    def go():
        time.sleep(SC.get("connect_delay", 0.3))
        if SC.get("mode") != "no_connect":
            session(True)
            if SC.get("radio_change"):
                put("radio_override.json", {"frequency_hz": 14074000})
            if SC.get("wire") != "none":
                frame("session_open:abc123,viewer;audio_samplerate:48000;audio_codec:opus;audio_start:0;iq_start:0;saturn_display:1;")
                if SC.get("wire") == "vfo": frame("vfo:0,0,14074000;")
                if SC.get("wire") == "binary": frame("\u0000\u0001", opcode=2)
                if SC.get("wire") == "tx": frame("trx:0,true;")
                if SC.get("wire") == "upper": frame("AUDIO_START:0;Saturn_Ping:1,2;")
                if SC.get("wire") == "vfo_late":
                    time.sleep(1.5)       # well after the session connected, while the measurement is under way
                    frame("vfo:0,0,14074000;")
            if SC.get("exception_after_start"):
                out({"method": "Runtime.exceptionThrown", "params": {"exceptionDetails": {"text": "Uncaught", "exception": {"description": "RangeError: late failure"}}}})
            if SC.get("leak_console"):
                password = open(os.path.join(W, "expected_auth.txt")).read().strip().partition(":")[2]
                out({"method": "Runtime.consoleAPICalled", "params": {"type": "error", "args": [{"type": "string", "value": "fetch failed: https://saturn:" + password + "@127.0.0.1/x"}]}})
            if SC.get("tx_change"):
                put("tx_override.json", {"tx_keyed": True})
            if SC.get("crash_after_start"):
                time.sleep(0.4); os._exit(9)
            if SC.get("crash_event"):
                out({"method": "Inspector.targetCrashed", "params": {}})
    threading.Thread(target=go, daemon=True).start()
    return json.dumps({"ok": True})
def stop():
    put("stop_called.json", {"at": time.time()})
    if not SC.get("ignore_stop"):
        st["stopped"] = True; session(False)
    return json.dumps({"ok": True})
def evaluate(expr):
    if expr == "document.visibilityState":
        return "visible" if SC.get("no_background_tab") else "hidden"
    if "receiveOnlyGuard" in expr:
        return json.dumps({"active": not SC.get("guard_inactive")})
    if "document.readyState" in expr:
        if SC.get("blank_race") and not st["loaded"]:
            return "undefined"   # the initial about:blank document: complete, and of course without a validation hook
        return ("undefined" if SC.get("legacy") else "object") if st["loaded"] else "loading"
    if "saturnValidation.start" in expr or "if (!b) return JSON.stringify" in expr:
        return start()
    if "saturnValidation.stop" in expr or "if (b) b.click()" in expr:
        return stop()
    if "saturnValidation.snapshot" in expr or 'mode: "legacy"' in expr:
        return json.dumps(snapshot())
    return None
def navigate(url):
    put("navigated.json", {"url": url})
    if SC.get("crash_on_load"):
        os._exit(9)
    status = SC.get("doc_status", 200)
    if SC.get("auth", True):
        # the request is paused by Fetch.enable; nothing proceeds until the client continues it, and only then is the login demanded
        st["challenge"] = "nav1"
        out({"method": "Fetch.requestPaused", "params": {"requestId": "req0", "request": {"url": url}}})
    else:
        finish_load(status)
def finish_load(status):
    out({"method": "Network.responseReceived", "params": {"type": "Document", "response": {"status": status, "url": "https://127.0.0.1/remote-next"}}})
    if SC.get("exception_on_load"):
        out({"method": "Runtime.exceptionThrown", "params": {"exceptionDetails": {"text": "Uncaught", "exception": {"description": "TypeError: x is not a function"}}}})
    if SC.get("leak_exception"):
        password = open(os.path.join(W, "expected_auth.txt")).read().strip().partition(":")[2]
        out({"method": "Runtime.exceptionThrown", "params": {"exceptionDetails": {"text": "Uncaught", "exception": {"description": "TypeError: cannot load https://saturn:" + password + "@127.0.0.1/"}}}})
    st["loaded"] = True
def handle(message):
    method, params, ident = message["method"], message.get("params", {}), message["id"]
    def reply(result=None, error=None):
        body = {"id": ident}
        if message.get("sessionId"): body["sessionId"] = message["sessionId"]
        if error: body["error"] = {"message": error}
        else: body["result"] = result or {}
        out(body)
    if method == "Target.getTargets": return reply({"targetInfos": [{"type": "page", "targetId": "T1"}]})
    if method == "Target.createTarget":
        put("created_target.json", params)
        if SC.get("no_background_tab"): return reply(error="Target.createTarget is not available")
        return reply({"targetId": "T2"})
    if method == "Target.attachToTarget":
        put("attached_target.json", params)
        return reply({"sessionId": "S1"})
    if method == "Page.addScriptToEvaluateOnNewDocument":
        put("injected.json", {"sha256": __import__("hashlib").sha256(params["source"].encode()).hexdigest(), "length": len(params["source"])})
        return reply({"identifier": "1"})
    if method == "Fetch.enable": put("fetch_enable.json", params); return reply()
    if method == "Performance.getMetrics" and SC.get("no_metrics"): return reply(error="Performance domain is not available")
    if method == "Performance.getMetrics":
        age = time.time() - T0
        return reply({"metrics": [{"name": "TaskDuration", "value": SC.get("page_busy", 0.4) * age}, {"name": "ScriptDuration", "value": 0.2 * age},
                                  {"name": "JSHeapUsedSize", "value": 30e6}, {"name": "Nodes", "value": 900}]})
    if method == "Fetch.continueRequest":
        reply()
        if params.get("requestId") == "req0" and st["challenge"]:
            out({"method": "Fetch.authRequired", "params": {"requestId": "nav1", "authChallenge": {"origin": "https://127.0.0.1", "scheme": "basic", "realm": "Saturn"}}})
        return
    if method == "Page.navigate": reply({"frameId": "F"}); return navigate(params["url"])
    if method == "Fetch.continueWithAuth":
        response = params["authChallengeResponse"]
        put("auth_seen.json", response)
        expected = open(os.path.join(W, "expected_auth.txt")).read().strip()
        user, _, password = expected.partition(":")
        ok = response.get("response") == "ProvideCredentials" and response.get("username") == user and response.get("password") == password
        reply()
        return finish_load(SC.get("doc_status", 200) if ok else 401)
    if method == "Runtime.evaluate":
        value = evaluate(params["expression"])
        return reply({"result": {"type": "string", "value": value}})
    if method == "Browser.close":
        reply()
        if SC.get("stubborn") or SC.get("ignore_close"):
            return
        session(False); os._exit(0)
    return reply()
buffer = b""
while True:
    chunk = os.read(3, 65536)
    if not chunk:
        break
    buffer += chunk
    while b"\0" in buffer:
        raw, buffer = buffer.split(b"\0", 1)
        if raw:
            handle(json.loads(raw))
'''

FAKE_SYSTEMCTL = """#!/bin/sh
if [ "$1" = show ]; then
  case "$2" in
    --property=*) for unit; do :; done; echo "$unit" >> "$FAKE_PIDS/show-units.log"; cat "$FAKE_PIDS/unit-properties.txt" 2>/dev/null; exit 0 ;;
  esac
  cat "$FAKE_PIDS/$5" 2>/dev/null || echo 0; exit 0
fi
echo "$@" >> "$FAKE_PIDS/systemctl.log"
if [ "$1" = kill ]; then
  pid=$(sed -n 's/.*"pid": \\([0-9]*\\).*/\\1/p' "$FAKE_WORLD/session.json" 2>/dev/null)
  [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null
fi
exit 0
"""


FAKE_SYSTEMD_RUN = """#!/bin/sh
export FAKE_WORLD=%(world)s FAKE_PIDS=%(pids)s FAKE_SCENARIO='%(scenario)s'
: > "$FAKE_PIDS/systemd-run.args"
while [ $# -gt 0 ]; do
  if [ "$1" = -- ]; then shift; break; fi
  echo "$1" >> "$FAKE_PIDS/systemd-run.args"
  shift
done
exec "$@"
"""

DEFAULT_UNIT_PROPERTIES = {"ControlGroup": "/system.slice/fake.service", "MemoryMax": "367001600", "MemorySwapMax": "0",
                           "MemoryCurrent": "123456789", "MemoryPeak": "222222222", "ActiveState": "active", "OOMScoreAdjust": "1000"}
# this Raspberry Pi image: no memory controller, so the kernel reports nothing it could account
NO_MEMORY_CONTROLLER_PROPERTIES = {"MemoryMax": "infinity", "MemorySwapMax": "infinity", "MemoryCurrent": "[not set]", "MemoryPeak": "[not set]"}


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as handle:
            return handle.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return False


class Rig:
    """One complete fake environment, and the validator running against it."""

    def __init__(self, test, scenario=None, baseline=0, rxc1="unarmed", bridge_commit=COMMIT, ready_commit=COMMIT, freeze_owner=False,
                 sdr_never=False, limits=None, expect_rxc1="any", auth=True, ready_flag=True, bridge_pid_file=None, perf_pid=None,
                 freeze_on_session=False, owner_blind=False, p23_broken=False, sticky=False, isolation=None, unit_props=None, memory_controller=None,
                 mem_available_kb=500000, perf_after=None, old_pid_for=None, stale_for=None):
        self.test = test
        self.isolation = isolation          # None: not checked; "direct": a browser started directly; "systemd": in a (fake) transient unit
        self.unit_props = dict(DEFAULT_UNIT_PROPERTIES, **(unit_props or {}))
        self.memory_controller = memory_controller      # None: the kernel is not asked; True/False: cgroup.controllers lists (or lacks) memory
        self.root = Path(tempfile.mkdtemp(prefix="saturn-validate-test-"))
        self.world = self.root / "world"
        self.world.mkdir()
        self.ctrl = self.root / "ctrl"
        self.releases = self.root / "releases"
        self.pids = self.root / "pids"
        self.pids.mkdir()
        if isolation is not None:       # the host's memory figures exist before the browser does, as /proc/meminfo does
            (self.world / "proc").mkdir()
            (self.world / "proc" / "meminfo").write_text(f"MemTotal: 927000 kB\nMemAvailable: {mem_available_kb} kB\nSwapTotal: 3000000 kB\nSwapFree: 2700000 kB\n")
        if memory_controller is not None:
            (self.world / "cgroup").mkdir()
            (self.world / "cgroup" / "cgroup.controllers").write_text("cpuset cpu io memory pids\n" if memory_controller else "cpuset cpu io pids\n")
        self.scenario = scenario or {}
        self.scenario.setdefault("auth", auth)
        self.baseline, self.rxc1, self.ready_commit, self.freeze, self.sdr_never = baseline, rxc1, ready_commit, freeze_owner, sdr_never
        self.ready_flag = ready_flag
        self.perf_pid, self.freeze_on_session, self.owner_blind, self.p23_broken, self.sticky = perf_pid, freeze_on_session, owner_blind, p23_broken, sticky
        self.latched = False
        self.limits = dict(FAST, **(limits or {}))
        self.expect_rxc1 = expect_rxc1
        self.procs = []
        (self.world / "expected_auth.txt").write_text(SECRET + "\n")
        browser = self.root / "fake-chromium"
        browser.write_text(FAKE_BROWSER % {"python": sys.executable})
        browser.chmod(0o755)
        self.browser = browser
        systemctl = self.root / "systemctl"
        systemctl.write_text(FAKE_SYSTEMCTL)
        systemctl.chmod(0o755)
        self.systemctl = systemctl
        self.bridge_pid = self._service("saturn-bridge.service", bridge_commit, "saturn-bridge", {})
        if bridge_pid_file is not None:
            (self.pids / "saturn-bridge.service").write_text(bridge_pid_file)
        self._service("saturn-go.service", COMMIT, "saturn-go", {"SATURN_REMOTE_BASIC_AUTH": SECRET})
        self.perf = self.root / "perf.json"
        self.perf_after = perf_after        # a restarted Bridge writes its first telemetry this many seconds from now (None: it already has)
        self.old_pid_for = old_pid_for      # ... and until then the file there is the previous process's (another pid)
        self.stale_for = stale_for          # ... or an old one (its timestamp is a minute old)
        self.stop_world = threading.Event()
        self.started = time.time()
        self._write_perf(initial=True)
        self.world_thread = threading.Thread(target=self._world_loop, daemon=True)
        self.world_thread.start()
        rig = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                if self.path.startswith("/readyz"):
                    body = {"ready": rig.ready_flag and rig.ready_commit is not None, "build_commit": rig.ready_commit, "status": "ready"}
                elif self.path.startswith("/p23_perf") and rig.p23_broken:
                    self.send_response(500); self.end_headers(); return
                elif self.path.startswith("/p23_perf"):
                    metrics = rig._metrics()
                    body = {"perf": {"workload": {"selected_app": "xdma", "service_main_pid": rig.bridge_pid},
                                     "app_telemetry": {"snapshot_readable": True, "pid_matches_service": True, "age_seconds": rig._age(),
                                                       "current": {"app": "saturn-bridge", "pid": rig.bridge_pid,
                                                                   "state": {"sdr_active": rig._sdr(), "thread_error": False}}}}}
                else:
                    self.send_response(404); self.end_headers(); return
                data = json.dumps(body).encode()
                self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(data))); self.end_headers()
                self.wfile.write(data)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.validator = None

    # ---- the fake owner
    def _service(self, unit, commit, binary, env):
        directory = self.releases / commit / "bin"
        directory.mkdir(parents=True, exist_ok=True)
        target = directory / binary
        shutil.copy(shutil.which("sleep"), target)
        child_env = {"PATH": os.environ["PATH"], **env}
        process = subprocess.Popen([str(target), "3600"], env=child_env, start_new_session=True)
        self.procs.append(process)
        (self.pids / unit).write_text(f"{process.pid}\n")
        return process.pid

    def _session(self):
        """The browser process is alive and connected. An owner that keeps counting a dead client (sticky) stays latched on."""
        try:
            data = json.loads((self.world / "session.json").read_text())
            live = alive(data["pid"]) and data["connected"]
        except (OSError, ValueError, KeyError):
            live = False
        if live:
            self.latched = True
        return live or (self.sticky and self.latched)

    def _seen(self):
        """What the owner itself sees of the session: nothing at all if it is blind to it."""
        return self._session() and not self.owner_blind

    def _metrics(self):
        session = self._seen()
        frequency = 27385000
        try:
            if session:
                frequency = json.loads((self.world / "radio_override.json").read_text())["frequency_hz"]
        except (OSError, ValueError, KeyError):
            pass
        try:
            extra = int((self.world / "extra_connections").read_text())  # another person connecting or leaving while the validation runs
        except (OSError, ValueError):
            extra = 0
        tx_keyed = session and (self.world / "tx_override.json").exists()
        pid = self.perf_pid or self.bridge_pid
        if self.old_pid_for is not None and time.time() - self.started < self.old_pid_for:
            pid = 999999
        return {"pid": pid, "client": 1 if (self.baseline > 0 or session or extra > 0) else 0,
                "connections": self.baseline + extra + (2 if session else 0),
                "frequency_hz": frequency, "sample_rate_hz": 384000, "ddc": 6, "adc": "ADC1", "processing_mode": "audio" if (session or self.baseline > 0 or extra > 0) else "drain-only", "tx_keyed": tx_keyed,
                "tx_stream": False, "rx_counter_v31": {"status": self.rxc1}}

    def _age(self):
        return 60.0 if (self.freeze or (self.freeze_on_session and self._session())) else 0.0

    def _sdr(self):
        """What the real owner reports: a receive client is attached (a human's, or ours)."""
        return (not self.sdr_never) and (self._seen() or self.baseline > 0)

    def _write_perf(self, initial=False):
        if self.perf_after is not None and time.time() - self.started < self.perf_after:
            return
        stale = self.freeze or (self.freeze_on_session and self._session()) or (self.stale_for is not None and time.time() - self.started < self.stale_for)
        updated = int((time.time() - (60 if stale else 0)) * 1000)
        document = {"schema_version": 1, "updated_at_ms": updated, "source": "saturn-bridge", "backend": "xdma", "status": "ready", "rf_safe": True,
                    "error": None, "metrics": self._metrics()}
        temporary = self.perf.with_suffix(".tmp")
        temporary.write_text(json.dumps(document))
        os.replace(temporary, self.perf)

    def _world_loop(self):
        while not self.stop_world.is_set():
            if not self.freeze:
                self._write_perf()
            time.sleep(0.1)

    def connections(self):
        return self._metrics()["connections"]

    # ---- the validator
    def args(self, extra=()):
        return [sys.executable, str(VALIDATOR), "run", "--control-dir", str(self.ctrl), "--target-commit", COMMIT, "--backend", "xdma",
                "--expect-rxc1", self.expect_rxc1, "--url", "https://127.0.0.1:1/remote-next?validation=receive&transport=split",
                "--ready-url", f"http://127.0.0.1:{self.port}/readyz", "--perf-url", f"http://127.0.0.1:{self.port}/p23_perf",
                "--perf-json", str(self.perf), "--unit", "saturn-bridge.service", "--unit", "saturn-go.service",
                "--owner-unit", "saturn-bridge.service", "--auth-unit", "saturn-go.service", "--release-root", str(self.releases),
                "--tls-cert", "", "--browser", str(self.browser), "--launcher", "direct", "--guard-script", str(GUARD),
                "--systemctl", str(self.systemctl), "--limits", json.dumps(self.limits), *self.isolation_args(), *extra]

    def isolation_args(self):
        if self.isolation is None:
            return ["--isolation", "off"]
        args = ["--isolation", "required", "--proc-root", str(self.world / "proc"), "--cgroup-root", str(self.world / "cgroup")]
        if self.isolation == "systemd":
            args += ["--launcher", "systemd", "--systemd-run", str(self.root / "systemd-run")]
        return args

    def env(self):
        env = {"PATH": os.environ["PATH"], "FAKE_WORLD": str(self.world), "FAKE_PIDS": str(self.pids), "FAKE_SCENARIO": json.dumps(self.scenario)}
        return env

    def start(self, extra=()):
        (self.pids / "unit-properties.txt").write_text("".join(f"{k}={v}\n" for k, v in self.unit_props.items() if v is not None))
        stub = self.root / "systemd-run"
        stub.write_text(FAKE_SYSTEMD_RUN % {"world": self.world, "pids": self.pids, "scenario": json.dumps(self.scenario).replace("'", "'\\''")})
        stub.chmod(0o755)
        self.out = open(self.root / "validator.out", "w")
        self.validator = subprocess.Popen(self.args(extra), env=self.env(), stdout=self.out, stderr=subprocess.STDOUT, start_new_session=True)
        return self.validator

    def cli(self, command, *extra, timeout=60):
        result = subprocess.run([sys.executable, str(VALIDATOR), command, "--control-dir", str(self.ctrl), *extra], capture_output=True, text=True, timeout=timeout)
        try:
            return result.returncode, json.loads(result.stdout.strip().splitlines()[-1])
        except (ValueError, IndexError):
            return result.returncode, {"raw": result.stdout, "err": result.stderr}

    def wait_ready(self, timeout=30):
        return self.cli("wait", "--timeout", str(timeout), timeout=timeout + 30)

    def stop(self):
        return self.cli("stop", "--timeout", "40", timeout=80)

    def finish(self, timeout=40):
        return self.validator.wait(timeout=timeout)

    def status(self):
        return json.loads((self.ctrl / "status.json").read_text())

    def log_events(self):
        return [json.loads(line) for line in (self.ctrl / "validation.log").read_text().splitlines() if line.strip()]

    def browser_pid(self):
        try:
            return json.loads((self.world / "session.json").read_text())["pid"]
        except (OSError, ValueError, KeyError):
            return None

    def condition(self, identifier):
        return self.status()["conditions"].get(identifier)

    def read_world(self, name):
        return json.loads((self.world / name).read_text())

    def close(self):
        self.stop_world.set()
        if self.validator is not None and self.validator.poll() is None:
            try:
                os.killpg(self.validator.pid, signal.SIGKILL)
            except OSError:
                pass
        pid = self.browser_pid()
        if pid and alive(pid):
            try:
                os.killpg(os.getpgid(pid), signal.SIGKILL)
            except OSError:
                pass
        for process in self.procs:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except OSError:
                pass
            process.wait()
        self.server.shutdown()
        self.server.server_close()
        if getattr(self, "out", None):
            self.out.close()
        shutil.rmtree(self.root, ignore_errors=True)


class RigCase(unittest.TestCase):
    def rig(self, **kwargs):
        rig = Rig(self, **kwargs)
        self.addCleanup(rig.close)
        return rig

    def assertCleanedUp(self, rig):
        """The browser process is gone, the owner holds no connection of ours, and the validator said so."""
        pid = rig.browser_pid()
        self.assertIsNotNone(pid, "the browser never started")
        self.assertFalse(alive(pid), "the temporary browser is still running")
        deadline = time.time() + 3
        while time.time() < deadline and rig.connections() != rig.baseline:
            time.sleep(0.1)
        self.assertEqual(rig.connections(), rig.baseline, "the owner still holds our connection")
        self.assertTrue(rig.condition("cleanup.browser_gone")["ok"])
        self.assertTrue(rig.condition("cleanup.client_released")["ok"])

    def assertNoSecret(self, rig):
        for path in rig.ctrl.rglob("*"):
            if path.is_file():
                self.assertNotIn(SECRET_PASSWORD, path.read_text(errors="replace"), f"the credential leaked into {path.name}")
        self.assertNotIn(SECRET_PASSWORD, (rig.root / "validator.out").read_text(errors="replace"))
        if (rig.world / "browser_argv.json").exists():  # a browser is only started once the target release is known to be running
            seen = rig.read_world("browser_argv.json")
            self.assertNotIn(SECRET_PASSWORD, json.dumps(seen["argv"]), "the credential was put in the browser's arguments")
            self.assertNotIn(SECRET_PASSWORD, json.dumps(seen["env"]), "the credential was put in the browser's environment")

    def run_to_failure(self, rig, identifier, expect_code=V.EXIT_FAILED, detail=None):
        rig.start()
        code = rig.finish()
        status = rig.status()
        self.assertEqual(code, expect_code, status.get("failed"))
        self.assertEqual(status["state"], "failed")
        self.assertEqual(status["failed"]["id"], identifier, status["failed"])
        if detail:
            self.assertRegex(status["failed"]["detail"], detail)
        return status


class SuccessTests(RigCase):
    def test_validates_holds_then_cleans_up_on_stop(self):
        rig = self.rig()
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        status = rig.status()
        self.assertEqual(status["state"], "ready")
        failed = [k for k, v in status["conditions"].items() if not v["ok"]]
        self.assertEqual(failed, [])
        for identifier in ("release.running", "owner.fresh_telemetry", "browser.started", "page.loaded", "session.started", "session.connected_as_viewer",
                           "data.display_fresh", "data.audio_frames_arriving", "audio.decoded_progress", "audio.context_running", "audio.no_decode_errors",
                           "owner.fresh_telemetry_with_session", "owner.session_registered", "owner.gate_equivalent", "rxc1.intended_state",
                           "radio.unchanged", "wire.only_allowed_commands", "page.no_fatal_errors"):
            self.assertIn(identifier, status["conditions"], f"{identifier} was never verified")
        # the session is HELD while the activation helper does its own check
        time.sleep(2.5)
        self.assertTrue(alive(rig.browser_pid()), "the session was not held")
        self.assertEqual(rig.connections(), rig.baseline + 2)
        self.assertEqual(rig.status()["state"], "ready")
        code, final = rig.stop()
        self.assertEqual((code, final["state"]), (0, "done"), final)
        self.assertEqual(rig.finish(), 0)
        self.assertCleanedUp(rig)
        self.assertEqual(rig.status()["exit_code"], 0)
        self.assertTrue((rig.world / "stop_called.json").exists(), "the page's own stop hook was never called")
        events = {e["event"] for e in rig.log_events()}
        self.assertNotIn("browser_terminate", events, "a well-behaved browser must close without being terminated")
        self.assertNotIn("browser_kill", events)

    def test_the_credential_reaches_the_browser_only_through_devtools(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.read_world("auth_seen.json"), {"response": "ProvideCredentials", "username": "saturn", "password": SECRET_PASSWORD})
        rig.stop(); rig.finish()
        self.assertNoSecret(rig)

    def test_the_guard_is_injected_before_the_page_and_the_session_is_receive_only(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        injected = rig.read_world("injected.json")
        self.assertEqual(injected["sha256"], hashlib.sha256(GUARD.read_bytes().decode().encode()).hexdigest())
        self.assertIn("validation=receive", rig.read_world("navigated.json")["url"])
        self.assertTrue(rig.read_world("fetch_enable.json")["handleAuthRequests"])
        rig.stop(); rig.finish()

    def test_data_that_starts_late_is_waited_for(self):
        rig = self.rig(scenario={"data_delay": 2.0})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        rig.stop(); rig.finish()

    def test_uppercase_commands_are_judged_like_the_bridge_judges_them(self):
        rig = self.rig(scenario={"wire": "upper"})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop(); rig.finish()

    def test_an_already_connected_human_browser_is_left_alone(self):
        rig = self.rig(baseline=2)
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.connections(), 4)
        rig.stop(); self.assertEqual(rig.finish(), 0)
        self.assertEqual(rig.connections(), 2)
        self.assertCleanedUp(rig)

    def test_no_human_browser_is_needed(self):
        """Nothing is connected to the owner before: this is the case that failed the first activation."""
        rig = self.rig(baseline=0)
        rig.start()
        self.assertEqual(rig.baseline, 0)
        self.assertEqual(rig.connections(), 0)
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertTrue(rig.condition("owner.gate_equivalent")["ok"])
        rig.stop(); rig.finish()

    def test_the_initial_blank_page_is_not_mistaken_for_a_loaded_page(self):
        rig = self.rig(scenario={"blank_race": True})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        self.assertEqual(rig.status()["driver"], "native", "about:blank was taken for a legacy page")
        rig.stop(); rig.finish()

    def test_a_release_page_without_the_validation_hook_is_driven_through_go_live(self):
        rig = self.rig(scenario={"legacy": True})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.status()["driver"], "legacy")
        rig.stop(); self.assertEqual(rig.finish(), 0)
        self.assertCleanedUp(rig)

    def test_a_person_connecting_during_validation_does_not_fail_cleanup(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        (rig.world / "extra_connections").write_text("2")  # a human browser connects while the session is held
        time.sleep(0.5)
        code, final = rig.stop()
        self.assertEqual((code, final["state"]), (0, "done"), final)
        self.assertTrue(rig.condition("cleanup.client_released")["ok"])
        self.assertFalse(alive(rig.browser_pid()))
        self.assertEqual(rig.connections(), 2, "the person's connection must be untouched")

    def test_sigterm_ends_the_session_and_cleans_up(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.validator.send_signal(signal.SIGTERM)
        rig.finish()
        self.assertCleanedUp(rig)

    def test_expected_rxc1_state_is_checked(self):
        rig = self.rig(rxc1="unarmed", expect_rxc1="unarmed")
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop(); rig.finish()
        rig = self.rig(rxc1="unarmed", expect_rxc1="valid")
        self.run_to_failure(rig, "rxc1.intended_state", detail="'unarmed'.*'valid'")
        self.assertCleanedUp(rig)


class FailureTests(RigCase):
    """Every way reception can be absent after an install has to be reported as that exact condition, and still clean up."""

    def failure(self, identifier, scenario=None, detail=None, **kwargs):
        rig = self.rig(scenario=scenario, **kwargs)
        status = self.run_to_failure(rig, identifier, detail=detail)
        self.assertCleanedUp(rig)
        self.assertNoSecret(rig)
        return rig, status

    def test_session_never_connects(self):
        self.failure("session.connected_as_viewer", {"mode": "no_connect"}, detail="not connected within")

    def test_the_bridge_never_reports_itself_ready(self):
        self.failure("session.connected_as_viewer", {"bridge_not_ready": True}, detail="bridgeReady=False")

    def test_a_tx_flag_changes_while_the_session_is_attached(self):
        self.failure("radio.unchanged", {"tx_change": True}, detail="tx_keyed")

    def test_an_uncaught_exception_during_the_session(self):
        self.failure("page.no_fatal_errors", {"exception_after_start": True}, detail="late failure")

    def test_the_owner_does_not_count_our_session(self):
        self.failure("owner.session_registered", owner_blind=True)

    def test_the_owner_counting_only_other_peoples_connections_is_not_registration(self):
        self.failure("owner.session_registered", baseline=2, owner_blind=True, detail="2 before it")

    def test_owner_telemetry_goes_stale_once_the_session_is_attached(self):
        self.failure("owner.fresh_telemetry_with_session", freeze_on_session=True, detail="age")

    def test_the_owner_telemetry_belongs_to_another_process(self):
        rig = self.rig(perf_pid=999999)
        self.run_to_failure(rig, "owner.fresh_telemetry", detail="telemetry pid 999999")
        self.assertIsNone(rig.browser_pid())

    def test_the_gate_data_cannot_be_read(self):
        self.failure("owner.gate_equivalent", p23_broken=True, detail="p23_perf is readable")

    def test_the_owner_keeps_counting_our_connection_after_cleanup(self):
        rig = self.rig(sticky=True, limits={"cleanup_s": 2})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop()
        rig.finish()
        self.assertFalse(rig.condition("cleanup.client_released")["ok"])
        self.assertEqual(rig.status()["state"], "failed")
        self.assertEqual(rig.status()["failed"]["id"], "cleanup.client_released")

    def test_credentials_in_page_messages_are_redacted_everywhere(self):
        rig = self.rig(scenario={"leak_exception": True})
        self.run_to_failure(rig, "page.loaded", detail="redacted")
        self.assertNoSecret(rig)
        self.assertIn("<redacted>", (rig.ctrl / "validation.log").read_text())
        self.assertIn("<redacted>", (rig.ctrl / "status.json").read_text())

    def test_credentials_in_console_output_are_redacted_in_the_status(self):
        rig = self.rig(scenario={"leak_console": True})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop(); rig.finish()
        self.assertNoSecret(rig)
        self.assertIn("<redacted>", (rig.ctrl / "status.json").read_text())

    def test_a_browser_that_will_not_close_but_obeys_terminate_is_not_killed(self):
        rig = self.rig(scenario={"ignore_close": True, "mode": "no_connect"})
        rig.start()
        rig.finish()
        events = [e["event"] for e in rig.log_events()]
        self.assertIn("browser_terminate", events)
        self.assertNotIn("browser_kill", events, "SIGKILL must be the last resort, not the first")
        self.assertCleanedUp(rig)

    def test_session_connects_but_not_as_a_viewer(self):
        self.failure("session.connected_as_viewer", {"role": "operator"}, detail="operator")

    def test_the_viewer_role_is_the_default_and_an_operator_session_is_refused(self):
        rig = self.rig(scenario={"role": "operator"})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "session.connected_as_viewer")
        self.assertEqual((status["role_policy"], status["legacy_fallback_used"]), ("required", False))
        self.assertNotIn("session.role_legacy_fallback", status["conditions"])
        self.assertCleanedUp(rig)

    def test_a_bridge_that_predates_viewer_enforcement_is_accepted_only_under_the_named_fallback(self):
        rig = self.rig(scenario={"role": "operator"})
        rig.start(("--viewer-role", "legacy-fallback"))
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        status = rig.status()
        self.assertEqual((status["role"], status["role_enforced_by_bridge"]), ("operator", False))
        self.assertEqual((status["role_policy"], status["legacy_fallback_used"]), ("legacy-fallback", True))
        self.assertTrue(status["conditions"]["guard.active"]["ok"])
        fallback = status["conditions"]["session.role_legacy_fallback"]
        self.assertTrue(fallback["ok"])
        self.assertIn("LEGACY BRIDGE FALLBACK USED", fallback["detail"])
        self.assertIn("predates viewer-role enforcement", fallback["detail"])
        self.assertNotIn("session.connected_as_viewer", status["conditions"], "an operator session must never be recorded as a viewer session")
        rig.stop(); self.assertEqual(rig.finish(), 0)

    def test_the_fallback_still_refuses_any_other_role(self):
        rig = self.rig(scenario={"role": "observer"})
        rig.start(("--viewer-role", "legacy-fallback"))
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "session.connected_as_viewer")
        self.assertFalse(status["legacy_fallback_used"])

    def test_the_fallback_is_not_used_when_the_bridge_enforces_the_viewer_role(self):
        rig = self.rig()
        rig.start(("--viewer-role", "legacy-fallback"))
        self.assertEqual(rig.wait_ready()[0], 0)
        status = rig.status()
        self.assertEqual((status["role"], status["role_enforced_by_bridge"]), ("viewer", True))
        self.assertEqual((status["role_policy"], status["legacy_fallback_used"]), ("legacy-fallback", False))
        self.assertNotIn("session.role_legacy_fallback", status["conditions"])
        rig.stop(); rig.finish()

    def test_a_viewer_is_recorded_as_enforced_by_the_bridge(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        status = rig.status()
        self.assertEqual((status["role"], status["role_enforced_by_bridge"]), ("viewer", True))
        self.assertEqual((status["role_policy"], status["legacy_fallback_used"]), ("required", False))
        rig.stop(); rig.finish()

    def test_the_fallback_does_not_waive_the_guard(self):
        rig = self.rig(scenario={"guard_inactive": True, "role": "operator"})
        rig.start(("--viewer-role", "legacy-fallback"))
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "guard.active")
        self.assertNotIn("session.started", status["conditions"])
        self.assertFalse(rig.read_world("session.json")["connected"], "a connection was made without the guard")

    def test_without_the_guard_nothing_is_started(self):
        rig = self.rig(scenario={"guard_inactive": True, "role": "operator"})
        status = self.run_to_failure(rig, "guard.active", detail="NOT active")
        self.assertNotIn("session.started", status["conditions"], "the session must not start without the guard")
        self.assertFalse(rig.read_world("session.json")["connected"], "a connection was made without the guard")
        self.assertCleanedUp(rig)

    def test_a_forbidden_frame_during_the_measurement_ends_it_at_once(self):
        started = time.time()
        rig = self.rig(scenario={"wire": "vfo_late", "data_delay": 100.0}, limits={"data_s": 40, "max_seconds": 80})
        status = self.run_to_failure(rig, "wire.only_allowed_commands", detail="vfo:0,0,14074000")
        self.assertLess(time.time() - started, 20)
        self.assertNotIn("data.display_fresh", status["conditions"])
        self.assertCleanedUp(rig)

    def test_the_first_forbidden_frame_ends_the_session_at_once(self):
        started = time.time()
        rig = self.rig(scenario={"wire": "vfo", "data_delay": 100.0}, limits={"data_s": 40, "max_seconds": 80})
        status = self.run_to_failure(rig, "wire.only_allowed_commands", detail="vfo:0,0,14074000")
        self.assertLess(time.time() - started, 20, "the audit waited for the measurement instead of acting on the frame")
        self.assertNotIn("data.display_fresh", status["conditions"])
        self.assertTrue(any(e["event"] == "wire_alarm" for e in rig.log_events()))
        self.assertCleanedUp(rig)

    def test_the_page_refuses_to_start(self):
        self.failure("session.started", {"start_refused": True}, detail="split transport")

    def test_no_display_data(self):
        self.failure("data.display_fresh", {"no_display": True})

    def test_no_audio_frames_arrive(self):
        self.failure("data.audio_frames_arriving", {"no_audio_frames": True})

    def test_audio_arrives_but_the_wrong_codec(self):
        self.failure("data.audio_frames_arriving", {"codec": "pcm"}, detail="'pcm'")

    def test_decoded_audio_does_not_progress(self):
        self.failure("audio.decoded_progress", {"audio_stalled": True})

    def test_audio_context_never_runs(self):
        self.failure("audio.context_running", {"audio_ctx": "suspended"})

    def test_decode_errors(self):
        self.failure("audio.no_decode_errors", {"decode_errors": True})

    def test_the_page_reports_a_guard_violation(self):
        self.failure("page.no_fatal_errors", {"violations": [{"kind": "fetch-POST", "detail": "/remote_settings"}]}, detail="violations")

    def test_the_page_reports_uncaught_errors(self):
        self.failure("page.no_fatal_errors", {"page_errors": [{"kind": "uncaught", "detail": "boom"}]}, detail="boom")

    def test_the_browser_crashes_event(self):
        self.failure("page.no_fatal_errors", {"crash_event": True}, detail="crashed")

    def test_a_tuning_command_leaves_the_browser(self):
        self.failure("wire.only_allowed_commands", {"wire": "vfo"}, detail="vfo:0,0,14074000")

    def test_a_transmit_command_leaves_the_browser(self):
        self.failure("wire.only_allowed_commands", {"wire": "tx"}, detail="trx")

    def test_a_binary_uplink_frame_leaves_the_browser(self):
        self.failure("wire.only_allowed_commands", {"wire": "binary"}, detail="binary")

    def test_nothing_observed_on_the_wire_proves_nothing(self):
        self.failure("wire.only_allowed_commands", {"wire": "none"}, detail="nothing could be verified")

    def test_the_radio_changes_while_the_session_is_attached(self):
        self.failure("radio.unchanged", {"radio_change": True}, detail="frequency_hz.*27385000.*14074000")

    def test_the_owner_does_not_see_receive_activity(self):
        self.failure("owner.gate_equivalent", sdr_never=True, detail="sdr_active")

    def test_fatal_error_while_loading(self):
        self.failure("page.loaded", {"exception_on_load": True}, detail="TypeError")

    def test_the_page_is_not_served(self):
        self.failure("page.loaded", {"doc_status": 500}, detail="HTTP 500")

    def test_the_proxy_rejects_the_credential(self):
        rig = self.rig()
        (rig.world / "expected_auth.txt").write_text("saturn:a-different-password\n")
        self.run_to_failure(rig, "page.loaded", detail="HTTP 401")
        self.assertNoSecret(rig)

    def test_the_browser_dies_during_the_session(self):
        rig = self.rig(scenario={"crash_after_start": True})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["state"], "failed")
        self.assertIsNotNone(status["failed"])
        self.assertCleanedUp(rig)

    def test_the_browser_dies_while_loading(self):
        rig = self.rig(scenario={"crash_on_load": True})
        rig.start()
        rig.finish()
        self.assertEqual(rig.status()["state"], "failed")
        self.assertCleanedUp(rig)

    def test_an_unresponsive_browser_is_bounded_and_killed(self):
        started = time.time()
        rig = self.rig(scenario={"hang_on_snapshot": 3, "stubborn": True}, limits={"call_s": 1.5})
        rig.start()
        rig.finish()
        self.assertLess(time.time() - started, 30)
        status = rig.status()
        self.assertEqual(status["state"], "failed")
        self.assertEqual(status["failed"]["id"], "browser.responsive")
        self.assertRegex(status["failed"]["detail"], "unresponsive|did not answer")
        self.assertCleanedUp(rig)

    def test_a_browser_that_ignores_close_and_term_is_killed(self):
        rig = self.rig(scenario={"stubborn": True, "mode": "no_connect"})
        rig.start()
        rig.finish()
        self.assertEqual(rig.status()["state"], "failed")
        self.assertCleanedUp(rig)
        events = [e["event"] for e in rig.log_events()]
        self.assertIn("browser_terminate", events, "the escalation path was not exercised")
        self.assertIn("browser_kill", events, "a browser that ignores SIGTERM must be killed")

    def test_the_overall_time_limit_is_enforced_and_reported_as_such(self):
        rig = self.rig(scenario={"mode": "no_connect"}, limits={"max_seconds": 3, "connect_s": 60})
        status = self.run_to_failure(rig, "validation.timeout", expect_code=V.EXIT_TIMEOUT, detail="3 s limit")
        self.assertCleanedUp(rig)

    def test_a_validator_nobody_stops_gives_up_and_reports_it(self):
        rig = self.rig(limits={"max_seconds": 6})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.finish(), V.EXIT_TIMEOUT)
        self.assertEqual(rig.status()["failed"]["id"], "hold.within_time_limit")
        self.assertCleanedUp(rig)

    def test_audio_stops_while_the_session_is_held(self):
        rig = self.rig(scenario={"audio_stall_after": 4.0}, limits={"hold_stall_s": 1.5})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual(code, 0, ready)
        rig.finish()
        self.assertEqual(rig.status()["failed"]["id"], "hold.audio_progressing")
        self.assertCleanedUp(rig)

    def test_the_connection_drops_while_the_session_is_held(self):
        rig = self.rig(scenario={"drop_after": 3.5})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.finish()
        self.assertEqual(rig.status()["failed"]["id"], "hold.session_alive")
        self.assertCleanedUp(rig)

    def test_wait_reports_the_exact_failed_condition(self):
        rig = self.rig(scenario={"audio_stalled": True})
        rig.start()
        code, result = rig.wait_ready()
        self.assertEqual(code, V.EXIT_FAILED)
        self.assertEqual(result["failed"]["id"], "audio.decoded_progress")
        rig.finish()


class IsolationTests(RigCase):
    """The browser's sandbox and memory cap are verified from the host while the session runs, not assumed from the flags that were passed."""

    def isolated(self, **kwargs):
        return self.rig(isolation="systemd", **kwargs)

    def failure(self, rig, identifier, detail):
        status = self.run_to_failure(rig, identifier, detail=detail)
        self.assertCleanedUp(rig)       # a failed isolation check still ends the session and releases the owner
        return status

    def test_a_sandboxed_browser_with_its_cap_is_verified_and_the_evidence_kept(self):
        rig = self.isolated()
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        status = rig.status()
        sandbox, memory = status["conditions"]["browser.sandbox_intact"], status["conditions"]["browser.memory_limit_applied"]
        self.assertTrue(sandbox["ok"] and memory["ok"], (sandbox, memory))
        self.assertIn("Seccomp: 2", sandbox["detail"])
        self.assertIn("367001600", memory["detail"])
        kinds = sorted(p["type"] for p in status["isolation"]["processes"])
        self.assertEqual(kinds, ["browser", "gpu-process", "renderer", "renderer"])
        self.assertTrue(all(p["seccomp"] == 2 for p in status["isolation"]["processes"] if p["type"] == "renderer"))
        self.assertEqual(status["isolation"]["memory"]["max_bytes"], "367001600")
        self.assertTrue(status["sandbox_requested"])
        rig.stop(); self.assertEqual(rig.finish(), 0)
        status = rig.status()
        self.assertEqual(status["isolation"]["memory"]["peak_bytes"], "222222222", "how close the browser came to its cap is kept")
        self.assertCleanedUp(rig)

    def test_the_unit_is_started_with_the_cap_and_the_hardening(self):
        rig = self.isolated()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop(); rig.finish()
        given = (rig.pids / "systemd-run.args").read_text().split()
        for needed in ("MemoryMax=350M", "MemorySwapMax=0", "NoNewPrivileges=yes", "DynamicUser=yes", "IPAddressDeny=any", "IPAddressAllow=localhost"):
            self.assertIn(needed, given)
        self.assertNotIn("--no-sandbox", rig.read_world("browser_argv.json")["argv"])
        unit = rig.status()["unit"] + ".service"
        self.assertIn(f"stop {unit}", (rig.pids / "systemctl.log").read_text())
        queried = set((rig.pids / "show-units.log").read_text().split())
        self.assertEqual(queried, {unit}, "the cap and the cgroup must be read from the browser's own unit, not from any unit")

    def test_a_renderer_that_is_not_confined_fails_the_run(self):
        rig = self.isolated(scenario={"renderer_seccomp": 0})
        status = self.failure(rig, "browser.sandbox_intact", r"Seccomp:0")
        self.assertFalse(status["conditions"]["browser.sandbox_intact"]["ok"])

    def test_a_browser_started_without_its_sandbox_fails_even_if_it_looks_confined(self):
        rig = self.isolated(scenario={"browser_no_sandbox": True})
        self.failure(rig, "browser.sandbox_intact", r"--no-sandbox")

    def test_the_no_sandbox_option_cannot_pass_the_check(self):
        rig = self.isolated()
        rig.start(("--no-sandbox",))
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "browser.sandbox_intact")
        self.assertFalse(status["sandbox_requested"])
        self.assertIn("--no-sandbox", rig.read_world("browser_argv.json")["argv"])
        self.assertCleanedUp(rig)

    def test_a_browser_with_no_renderer_to_inspect_is_not_assumed_sandboxed(self):
        rig = self.isolated(scenario={"renderers": 0})
        self.failure(rig, "browser.sandbox_intact", r"no renderer process")

    def test_a_missing_memory_cap_fails_the_run(self):
        rig = self.isolated(unit_props={"MemoryMax": "infinity"})
        self.failure(rig, "browser.memory_limit_applied", r"no memory cap")

    def test_a_different_memory_cap_fails_the_run(self):
        rig = self.isolated(unit_props={"MemoryMax": str(2 * 1024 ** 3)})
        self.failure(rig, "browser.memory_limit_applied", r"MemoryMax is 2147483648 bytes but 350M")

    def test_swap_is_not_allowed(self):
        rig = self.isolated(unit_props={"MemorySwapMax": "infinity"})
        self.failure(rig, "browser.memory_limit_applied", r"swap")

    def test_an_unreadable_unit_fails_the_run(self):
        rig = self.isolated(unit_props={"MemoryMax": None, "MemorySwapMax": None, "ControlGroup": None})
        status = self.run_to_failure(rig, "browser.sandbox_intact")        # no cgroup, so no process could be inspected either
        self.assertFalse(status["conditions"]["browser.memory_limit_applied"]["ok"])
        self.assertCleanedUp(rig)

    def test_a_browser_started_directly_has_no_cap_and_fails_a_required_check(self):
        rig = self.rig(isolation="direct")
        status = self.run_to_failure(rig, "browser.memory_limit_applied", detail="not started in a systemd unit")
        self.assertTrue(status["conditions"]["browser.sandbox_intact"]["ok"], "the sandbox evidence is gathered the same way for a direct child")
        self.assertCleanedUp(rig)

    def test_isolation_is_checked_before_the_session_is_held(self):
        rig = self.isolated(scenario={"renderer_seccomp": 0})
        rig.start()
        rig.finish()
        self.assertIsNone(rig.status().get("ready_at"), "a run whose isolation failed must never report ready")

    def test_the_sandbox_only_mode_checks_the_sandbox_and_not_the_cap(self):
        rig = self.rig(isolation="direct")
        rig.start(("--isolation", "sandbox"))
        self.assertEqual(rig.wait_ready()[0], 0)
        conditions = rig.status()["conditions"]
        self.assertTrue(conditions["browser.sandbox_intact"]["ok"])
        self.assertNotIn("browser.memory_limit_applied", conditions)
        rig.stop(); rig.finish()

    def test_the_sandbox_only_mode_still_fails_an_unconfined_renderer(self):
        rig = self.rig(isolation="direct", scenario={"renderer_seccomp": 0})
        rig.start(("--isolation", "sandbox"))
        rig.finish()
        self.assertEqual(rig.status()["failed"]["id"], "browser.sandbox_intact")
        self.assertCleanedUp(rig)

    def test_with_isolation_off_nothing_is_inspected(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertNotIn("browser.sandbox_intact", rig.status()["conditions"])
        rig.stop(); rig.finish()


class EverythingIsReadTests(RigCase):
    """One run says everything it can: the sandbox, the memory cap, the owner's view, the wire and the radio are read even when the audio or
    the display failed, and the first failure is still the one reported."""

    ALSO_READ = ("browser.sandbox_intact", "browser.memory_limit_applied", "owner.fresh_telemetry_with_session", "owner.session_registered",
                 "owner.gate_equivalent", "radio.unchanged", "wire.only_allowed_commands", "page.no_fatal_errors")

    def test_stalled_audio_still_reports_the_sandbox_the_cap_the_owner_the_wire_and_the_radio(self):
        rig = self.rig(isolation="systemd", scenario={"audio_stalled": True, "legacy": True})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "audio.decoded_progress", "the first failure stays the headline")
        for identifier in self.ALSO_READ:
            self.assertIn(identifier, status["conditions"], f"{identifier} was never read although the browser was running")
            self.assertTrue(status["conditions"][identifier]["ok"], (identifier, status["conditions"][identifier]))
        self.assertTrue(status["conditions"]["data.display_fresh"]["ok"])
        self.assertCleanedUp(rig)

    def test_a_failure_in_the_data_does_not_hide_a_failure_in_the_isolation(self):
        rig = self.rig(isolation="systemd", scenario={"no_display": True, "renderer_seccomp": 0})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "data.display_fresh")
        self.assertFalse(status["conditions"]["browser.sandbox_intact"]["ok"], "an unconfined renderer must be reported even though the display failed first")
        self.assertCleanedUp(rig)

    def test_a_session_that_never_connected_still_gets_its_isolation_read(self):
        rig = self.rig(isolation="systemd", scenario={"mode": "no_connect"})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "session.connected_as_viewer")
        self.assertTrue(status["conditions"]["browser.sandbox_intact"]["ok"])
        self.assertCleanedUp(rig)

    def test_a_run_that_is_stopped_does_not_go_on_reading(self):
        rig = self.rig(isolation="systemd", scenario={"audio_stalled": True}, limits={"data_s": 30})
        rig.start()
        time.sleep(2.5)
        rig.cli("stop", "--timeout", "40", timeout=80)
        rig.finish()
        self.assertNotIn("browser.sandbox_intact", rig.status()["conditions"], "a validation that was told to stop must not start further checks")


class MemoryWatchdogTests(RigCase):
    """Where the kernel has no memory controller (cgroup_disable=memory, as on the appliance's Raspberry Pi image) the validator is the cap, and
    says so."""

    def pi(self, **kwargs):
        props = dict(NO_MEMORY_CONTROLLER_PROPERTIES, **kwargs.pop("unit_props", {}))
        return self.rig(isolation="systemd", memory_controller=False, unit_props=props, **kwargs)

    def test_a_browser_within_the_cap_is_enforced_by_the_watchdog_and_the_evidence_says_so(self):
        rig = self.pi()
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        status = rig.status()
        self.assertEqual(status["memory_enforcement"], "watchdog")
        applied = status["conditions"]["browser.memory_limit_applied"]
        self.assertTrue(applied["ok"], applied)
        self.assertIn("ENFORCED BY THE VALIDATOR'S WATCHDOG", applied["detail"])
        self.assertIn("not by the kernel", applied["detail"])
        self.assertEqual(status["isolation"]["memory"]["enforcement"], "watchdog")
        self.assertEqual(status["isolation"]["memory"]["cap_bytes"], 350 * 1024 ** 2)
        rig.stop()
        self.assertEqual(rig.finish(), 0)
        status = rig.status()
        within = status["conditions"]["browser.memory_within_cap"]
        self.assertTrue(within["ok"], within)
        memory = status["isolation"]["memory"]
        self.assertEqual(memory["peak_source"], "validator sampling (proportional set size)", "the kernel reports no peak here, so the measured one is kept and named")
        self.assertAlmostEqual(int(memory["peak_bytes"]) / 1048576, 4 * 40000 / 1024, delta=2)      # four processes, 40000 kB each
        self.assertCleanedUp(rig)

    def test_the_unit_still_carries_the_cap_and_the_oom_setting_so_a_kernel_with_a_controller_applies_it(self):
        rig = self.pi()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        rig.stop(); rig.finish()
        given = (rig.pids / "systemd-run.args").read_text().split()
        for needed in ("MemoryMax=350M", "MemorySwapMax=0", "OOMScoreAdjust=1000"):
            self.assertIn(needed, given)

    def test_a_browser_that_outgrows_the_cap_is_stopped_and_that_is_the_first_failure(self):
        rig = self.pi(scenario={"pss_growth_kb_s": 100000}, limits={"data_s": 20, "max_seconds": 60})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "browser.memory_within_cap", status["failed"])
        self.assertIn("MEMORY CAP EXCEEDED, enforced by the watchdog", status["failed"]["detail"])
        self.assertRegex(status["failed"]["detail"], r"use \d+ MiB \(proportional set size\) against the 350M cap")
        self.assertIn("kill --signal=SIGKILL " + status["unit"] + ".service", (rig.pids / "systemctl.log").read_text(), "the unit itself is what is stopped")
        self.assertIsNone(status.get("ready_at"), "a browser over the cap never reports ready")
        self.assertFalse(status["conditions"]["browser.memory_within_cap"]["ok"], "the evidence kept at the end must still say the cap was exceeded")
        failing = sorted(k for k, v in status["conditions"].items() if not v["ok"])
        self.assertEqual(failing, ["browser.memory_within_cap"], "a browser stopped on purpose must not leave a trail of unrelated failures about its death")
        self.assertCleanedUp(rig)

    def test_a_browser_that_outgrows_the_cap_while_the_session_is_held_ends_the_hold_as_a_failure(self):
        rig = self.pi(scenario={"pss_growth_kb_s": 12000}, limits={"data_s": 20, "max_seconds": 60})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), "it was within the cap when it was declared ready")
        rig.finish()
        status = rig.status()
        self.assertEqual(status["state"], "failed")
        self.assertEqual(status["failed"]["id"], "browser.memory_within_cap", status["failed"])
        self.assertFalse(status["conditions"]["browser.memory_within_cap"]["ok"], "the evidence kept at the end must still say the cap was exceeded")
        failing = sorted(k for k, v in status["conditions"].items() if not v["ok"])
        self.assertEqual(failing, ["browser.memory_within_cap"], "a browser stopped on purpose must not leave a trail of unrelated failures about its death")
        self.assertCleanedUp(rig)

    def test_a_browser_stopped_for_its_memory_while_the_validator_waits_on_it_is_not_reported_as_unresponsive(self):
        # the second snapshot never answers, so the validator is inside a DevTools call when the watchdog stops the browser: the call then fails
        # because the pipe closed, and that failure is a consequence of the memory stop, not a finding of its own
        rig = self.pi(scenario={"pss_growth_kb_s": 15000, "hang_on_snapshot": 1}, limits={"data_s": 30, "max_seconds": 60, "call_s": 8})
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "browser.memory_within_cap", status["failed"])
        failing = sorted(k for k, v in status["conditions"].items() if not v["ok"])
        self.assertEqual(failing, ["browser.memory_within_cap"], failing)
        self.assertGreaterEqual(rig.read_world("session.json")["pid"], 1)
        self.assertCleanedUp(rig)

    def test_a_unit_that_is_not_the_oom_killers_first_choice_does_not_count_as_capped(self):
        rig = self.pi(unit_props={"OOMScoreAdjust": "0"})
        status = self.run_to_failure(rig, "browser.memory_limit_applied", detail="first choice")
        self.assertCleanedUp(rig)

    def test_without_proportional_memory_the_resident_size_is_read_instead(self):
        rig = self.pi(scenario={"no_smaps": True})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual(code, 0, ready)
        rig.stop(); rig.finish()
        self.assertAlmostEqual(int(rig.status()["isolation"]["memory"]["peak_bytes"]) / 1048576, 4 * 50000 / 1024, delta=2)

    def test_memory_that_cannot_be_read_at_all_is_not_a_cap(self):
        rig = self.pi(scenario={"no_usage": True})
        status = self.run_to_failure(rig, "browser.memory_limit_applied", detail="could not be measured")
        self.assertCleanedUp(rig)

    def test_a_browser_that_outgrows_the_resident_size_cap_is_stopped_too(self):
        rig = self.pi(scenario={"no_smaps": True, "rss_kb": 120000}, limits={"data_s": 20, "max_seconds": 60})        # 4 x 120000 kB = 469 MiB resident
        rig.start()
        rig.finish()
        status = rig.status()
        self.assertEqual(status["failed"]["id"], "browser.memory_within_cap", status["failed"])
        self.assertCleanedUp(rig)

    def test_a_kernel_with_the_controller_keeps_the_cgroup_enforcement_and_asks_for_no_watchdog(self):
        rig = self.rig(isolation="systemd", memory_controller=True)
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        status = rig.status()
        self.assertEqual(status["memory_enforcement"], "cgroup")
        self.assertIn("MemoryMax 367001600 bytes", status["conditions"]["browser.memory_limit_applied"]["detail"])
        rig.stop(); rig.finish()
        self.assertNotIn("browser.memory_within_cap", rig.status()["conditions"], "the kernel enforces it; there is no second opinion to record")

    def test_with_a_memory_controller_the_validator_leaves_enforcement_to_the_kernel(self):
        rig = self.rig(isolation="systemd", memory_controller=True, scenario={"pss_growth_kb_s": 100000}, limits={"hold_stall_s": 30})
        rig.start()
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), "the cap is the kernel's job here; the validator does not stop a browser the kernel is trusted to bound")
        time.sleep(1.5)
        status = rig.status()
        self.assertEqual(status["state"], "ready")
        self.assertNotIn("browser.memory_within_cap", status["conditions"])
        log = rig.pids / "systemctl.log"
        self.assertNotIn("kill", log.read_text() if log.exists() else "")
        rig.stop(); rig.finish()

    def test_an_explicit_watchdog_is_honoured_even_where_the_kernel_could_do_it(self):
        rig = self.rig(isolation="systemd", memory_controller=True)
        rig.start(("--memory-enforcement", "watchdog"))
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.status()["memory_enforcement"], "watchdog")
        rig.stop(); rig.finish()


class DiagnosticsTests(RigCase):
    """What the next hardware run needs in order to say WHY a browser on the appliance was too slow: kept in the evidence, never able to fail a run."""

    def test_the_evidence_keeps_where_the_cpu_the_memory_and_the_disk_went(self):
        rig = self.rig(isolation="systemd", memory_controller=True, scenario={"page_busy": 0.5})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        time.sleep(2.5)
        rig.stop()
        self.assertEqual(rig.finish(), 0)
        diagnostics = rig.status()["diagnostics"]
        summary = diagnostics["summary"]
        self.assertGreaterEqual(summary["samples"], 4)
        self.assertAlmostEqual(summary["browser_pss_mb"]["max"], 4 * 40000 / 1024, delta=2)
        kinds = summary["pss_by_process_type_mb_max"]
        self.assertEqual({k: v["processes"] for k, v in kinds.items()}, {"browser": 1, "renderer": 2, "gpu-process": 1})
        self.assertAlmostEqual(kinds["renderer"]["pss_mb"], 2 * 40000 / 1024, delta=1)
        self.assertGreater(summary["browser_cpu_pct"]["max"], 50)               # four processes, 25 ticks a second each, plus system time
        self.assertLess(summary["browser_cpu_pct"]["max"], 200)
        self.assertGreater(summary["host_iowait_pct"]["mean"], 10)              # the fake host spends a quarter of its time waiting for I/O
        self.assertLess(summary["host_iowait_pct"]["mean"], 40)
        self.assertEqual(summary["mem_available_mb_min"], round(500000 / 1024))
        self.assertGreater(summary["swap_in_pages_total"], 0)
        self.assertGreater(summary["page_main_thread_busy"], 0.3)
        self.assertLess(summary["page_main_thread_busy"], 0.7)
        self.assertEqual(summary["owner_counters"]["connections"]["last"], 2, "the owner's own counters are sampled with the browser's")
        first = diagnostics["samples"][0]
        self.assertIn("pss_mb", first)
        self.assertEqual(diagnostics["watch"]["errors"], 0)
        self.assertLess(diagnostics["watch"]["longest_gap_s"], 1.5)

    def test_the_legacy_page_s_decoder_counters_are_in_the_snapshot_that_is_kept(self):
        self.assertIn("decodedFrames: rxOpus.decodedFrames", V.LEGACY_SNAPSHOT)
        self.assertIn("decoderFramesOutput: rxOpus.decoderFramesOutput", V.LEGACY_SNAPSHOT)

    def test_a_page_that_cannot_answer_for_its_metrics_does_not_fail_the_run(self):
        rig = self.rig(scenario={"legacy": True, "no_metrics": True})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        time.sleep(1.5)
        rig.stop(); self.assertEqual(rig.finish(), 0)
        self.assertNotIn("page_main_thread_busy", rig.status()["diagnostics"]["summary"], "no figure is invented for a page that gave none")


class MemoryHeadroomTests(RigCase):
    """On a 905 MB appliance a browser is not started unless the host has the memory to give it; the radio's processes never pay for a validation."""

    def test_a_host_with_the_memory_free_starts_the_browser(self):
        rig = self.rig(isolation="systemd", mem_available_kb=450 * 1024)      # 350M cap + 32 MiB reserve = 382 MiB needed
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        detail = rig.condition("host.memory_headroom")
        self.assertTrue(detail["ok"], detail)
        self.assertIn("450 MiB available", detail["detail"])
        rig.stop(); rig.finish()

    def test_a_host_without_the_memory_is_refused_and_no_browser_is_started(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024)
        status = self.run_to_failure(rig, "host.memory_headroom", detail=r"300 MiB available.*382 MiB needed\): no browser was started")
        self.assertIsNone(rig.browser_pid(), "a browser was started although the host cannot spare the memory")
        self.assertFalse((rig.world / "browser_argv.json").exists())

    def test_the_reserve_is_part_of_the_requirement(self):
        rig = self.rig(isolation="systemd", mem_available_kb=360 * 1024)      # more than the cap, less than cap + reserve
        self.run_to_failure(rig, "host.memory_headroom", detail="382 MiB needed")

    def test_available_memory_that_cannot_be_read_is_not_assumed(self):
        rig = self.rig(isolation="systemd")
        (rig.world / "proc" / "meminfo").unlink()
        self.run_to_failure(rig, "host.memory_headroom", detail="could not be read")
        self.assertIsNone(rig.browser_pid())

    def test_a_smaller_reserve_lowers_the_requirement(self):
        rig = self.rig(isolation="systemd", mem_available_kb=370 * 1024)      # 350M cap + 32 MiB = 382 needed: refused by default ...
        rig.start(("--memory-reserve", "8M"))                                   # ... 350M + 8 MiB = 358: enough
        self.assertEqual(rig.wait_ready()[0], 0)
        detail = rig.condition("host.memory_headroom")
        self.assertTrue(detail["ok"], detail)
        self.assertIn("8 MiB are kept in reserve", detail["detail"])
        rig.stop(); rig.finish()

    def test_a_larger_reserve_needs_more(self):
        rig = self.rig(isolation="systemd", mem_available_kb=450 * 1024)
        rig.start(("--memory-reserve", "200M"))                                 # 350M + 200M = 550 MiB needed
        rig.finish()
        failed = rig.status()["failed"]
        self.assertEqual(failed["id"], "host.memory_headroom")
        self.assertIn("550 MiB needed", failed["detail"])
        self.assertIsNone(rig.browser_pid())

    def test_a_reserve_below_the_floor_cannot_be_chosen(self):
        rig = self.rig(isolation="systemd", mem_available_kb=900 * 1024)
        rig.start(("--memory-reserve", "4M"))
        rig.finish()
        failed = rig.status()["failed"]
        self.assertEqual(failed["id"], "host.memory_headroom")
        self.assertIn("below the 8 MiB minimum", failed["detail"])
        self.assertIsNone(rig.browser_pid())

    def test_the_default_reserve_is_thirty_two_mebibytes(self):
        self.assertEqual(V.HEADROOM_MARGIN_BYTES, 32 * 1024 ** 2)
        self.assertEqual(V.HEADROOM_MARGIN_FLOOR_BYTES, 8 * 1024 ** 2)
        rig = self.rig(isolation="systemd", mem_available_kb=381 * 1024)      # one MiB short of 350M + 32 MiB when no reserve is named
        rig.start()
        rig.finish()
        self.assertIn("382 MiB needed", rig.status()["failed"]["detail"])

    def own_process(self, rig, pid, anon_kb, argv=("python3", "/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py", "run"), status=True):
        directory = rig.world / "proc" / str(pid)
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "cmdline").write_bytes(b"\0".join(part.encode() for part in argv) + b"\0")
        if status:
            (directory / "status").write_text(f"Name:\tpython3\nRssFile:\t9000 kB\nRssAnon:\t{anon_kb} kB\n")

    def test_the_validators_own_memory_is_added_back_and_said(self):
        rig = self.rig(isolation="systemd", mem_available_kb=360 * 1024)      # 350M cap + 32 MiB = 382 needed; 360 alone is not enough ...
        self.own_process(rig, 4101, 14000)                                     # ... the controller (about 14 MiB) and the waiter (about 21 MiB) are 35 MiB
        self.own_process(rig, 4102, 21500, argv=("python3", "/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py", "wait"))
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        detail = rig.condition("host.memory_headroom")
        self.assertTrue(detail["ok"], detail)
        self.assertRegex(detail["detail"], r"395 MiB available \(counting the 35 MiB that the validator's own processes use")
        rig.stop(); rig.finish()

    def test_adding_the_validators_memory_back_cannot_make_a_host_without_the_memory_pass(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024)
        self.own_process(rig, 4101, 36000)
        self.run_to_failure(rig, "host.memory_headroom", detail=r"335 MiB available \(counting the 35 MiB.*382 MiB needed\): no browser was started")
        self.assertIsNone(rig.browser_pid())

    def test_only_this_programs_processes_are_counted(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024)
        self.own_process(rig, 4101, 36000, argv=("grep", "-r", "saturn-remote-validate.py"))                           # names the script, is not it
        self.own_process(rig, 4102, 36000, argv=("vim", "/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py"))
        self.own_process(rig, 4103, 36000, argv=("python3", "/usr/local/lib/saturn-go/scripts/some-other-script.py"))
        self.run_to_failure(rig, "host.memory_headroom", detail=r"300 MiB available; the browser may take up to 350M.*382 MiB needed\)")
        self.assertNotIn("counting", rig.status()["failed"]["detail"])

    def test_the_memory_added_back_is_bounded(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024)
        self.own_process(rig, 4101, 500 * 1024)           # a runaway reading must not be able to buy a pass
        self.run_to_failure(rig, "host.memory_headroom", detail=r"364 MiB available \(counting the 64 MiB.*382 MiB needed\)")

    def test_an_unreadable_process_entry_counts_as_nothing(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024)
        self.own_process(rig, 4101, 0, status=False)       # a process that vanished between the two reads
        (rig.world / "proc" / "4102").mkdir()              # no cmdline at all
        self.run_to_failure(rig, "host.memory_headroom", detail=r"300 MiB available; the browser may take up to 350M.*382 MiB needed\)")

    def test_a_larger_measurement_cap_needs_the_larger_headroom(self):
        rig = self.rig(isolation="systemd", mem_available_kb=450 * 1024)
        rig.start(("--memory-max", "500M"))
        rig.finish()
        self.assertEqual(rig.status()["failed"]["id"], "host.memory_headroom")

    def test_without_isolation_checks_there_is_nothing_to_protect(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertNotIn("host.memory_headroom", rig.status()["conditions"])
        rig.stop(); rig.finish()

    def test_the_lean_browser_switches_are_in_the_flags(self):
        flags = V.browser_flags("/tmp/profile", None, sandbox=True)
        for needed in ("--enable-low-end-device-mode", "--disable-software-rasterizer", "--num-raster-threads=1", "--disable-audio-output"):
            self.assertIn(needed, flags)
        for forbidden in ("--no-sandbox", "--single-process", "--no-zygote", "--disable-gpu-sandbox", "--in-process-gpu"):
            self.assertNotIn(forbidden, flags, "no switch that weakens the browser's sandbox may be added for the sake of memory")


class RestartTimingTests(RigCase):
    """Right after an activation, or the rollback after one, the services have just been restarted. The first look at the host is not the
    state it settles into, so the validator waits, within a bound, for the owner's first telemetry and for the memory to be back, and says so."""

    def test_telemetry_that_appears_a_moment_after_the_start_is_waited_for(self):
        rig = self.rig(perf_after=1.5, limits={"owner_wait_s": 6})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0, rig.status().get("failed"))
        detail = rig.condition("owner.fresh_telemetry")
        self.assertTrue(detail["ok"], detail)
        self.assertRegex(detail["detail"], r"after waiting [0-9.]+ s for the restarted owner's telemetry")
        rig.stop(); rig.finish()

    def test_telemetry_that_never_appears_fails_after_the_bound_and_says_how_long_it_waited(self):
        rig = self.rig(perf_after=60, limits={"owner_wait_s": 1.0})
        self.run_to_failure(rig, "owner.fresh_telemetry", detail=r"cannot read .*perf\.json.*\(after waiting [0-9.]+ s for the restarted owner's telemetry\)")
        self.assertIsNone(rig.browser_pid(), "a browser was started although the owner never wrote telemetry")

    def test_telemetry_that_is_there_and_right_is_not_waited_for(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertNotIn("after waiting", rig.condition("owner.fresh_telemetry")["detail"])
        rig.stop(); rig.finish()

    def test_telemetry_that_stays_stale_still_fails(self):
        rig = self.rig(freeze_owner=True, limits={"owner_wait_s": 1.0})
        self.run_to_failure(rig, "owner.fresh_telemetry", detail=r"age 6[0-9]\.[0-9]+ s.*after waiting")

    def test_telemetry_of_another_process_still_fails(self):
        rig = self.rig(perf_pid=999999, limits={"owner_wait_s": 1.0})
        self.run_to_failure(rig, "owner.fresh_telemetry", detail="telemetry pid 999999")

    def test_telemetry_of_the_previous_process_is_waited_out(self):
        rig = self.rig(old_pid_for=1.5, limits={"owner_wait_s": 6})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0, rig.status().get("failed"))
        detail = rig.condition("owner.fresh_telemetry")
        self.assertTrue(detail["ok"], detail)
        self.assertIn("after waiting", detail["detail"])
        rig.stop(); rig.finish()

    def test_telemetry_that_is_at_first_a_minute_old_is_waited_out(self):
        rig = self.rig(stale_for=1.5, limits={"owner_wait_s": 6})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0, rig.status().get("failed"))
        detail = rig.condition("owner.fresh_telemetry")
        self.assertTrue(detail["ok"], detail)
        self.assertIn("after waiting", detail["detail"])
        rig.stop(); rig.finish()

    def test_memory_that_rises_and_falls_back_is_judged_by_the_last_reading_and_reports_the_best(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024, limits={"headroom_wait_s": 2.5})
        meminfo = rig.world / "proc" / "meminfo"
        text = "MemTotal: 927000 kB\nMemAvailable: %d kB\nSwapTotal: 3000000 kB\nSwapFree: 2700000 kB\n"
        threading.Timer(0.8, lambda: meminfo.write_text(text % (360 * 1024))).start()      # more than before, still less than the 382 MiB needed
        threading.Timer(1.4, lambda: meminfo.write_text(text % (320 * 1024))).start()
        self.run_to_failure(rig, "host.memory_headroom", detail=r"320 MiB available.*300 MiB at first, 360 MiB at the most.*382 MiB needed\): no browser was started")
        self.assertIsNone(rig.browser_pid())

    def test_memory_that_becomes_unreadable_while_waiting_is_not_assumed(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024, limits={"headroom_wait_s": 4})
        meminfo = rig.world / "proc" / "meminfo"
        threading.Timer(1.0, lambda: meminfo.unlink()).start()
        self.run_to_failure(rig, "host.memory_headroom", detail="could not be read")
        self.assertIsNone(rig.browser_pid())

    def test_a_stop_request_ends_the_wait_for_telemetry_at_once(self):
        rig = self.rig(perf_after=60, limits={"owner_wait_s": 30})
        rig.start()
        time.sleep(1.5)
        began = time.time()
        rig.stop()
        code = rig.finish()
        self.assertLess(time.time() - began, 8, "the stop request did not end the wait")
        self.assertNotEqual(code, 0)
        self.assertEqual(rig.status()["failed"]["id"], "owner.fresh_telemetry")
        self.assertIn("stopped while waiting for the owner's telemetry", rig.status()["failed"]["detail"])

    def test_memory_that_climbs_back_is_waited_for_and_the_browser_starts_only_then(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024, limits={"headroom_wait_s": 8})
        meminfo = rig.world / "proc" / "meminfo"
        threading.Timer(1.2, lambda: meminfo.write_text("MemTotal: 927000 kB\nMemAvailable: 450000 kB\nSwapTotal: 3000000 kB\nSwapFree: 2700000 kB\n")).start()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0, rig.status().get("failed"))
        detail = rig.condition("host.memory_headroom")
        self.assertTrue(detail["ok"], detail)
        self.assertRegex(detail["detail"], r"439 MiB available.*after waiting [0-9.]+ s: 300 MiB at first")
        rig.stop(); rig.finish()

    def test_memory_that_never_climbs_back_is_refused_after_the_bound_with_what_was_seen(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024, limits={"headroom_wait_s": 1.0})
        self.run_to_failure(rig, "host.memory_headroom", detail=r"300 MiB available.*after waiting [0-9.]+ s: 300 MiB at first, 300 MiB at the most.*382 MiB needed\): no browser was started")
        self.assertIsNone(rig.browser_pid())
        self.assertFalse((rig.world / "browser_argv.json").exists())

    def test_memory_that_is_there_at_once_is_not_waited_for(self):
        rig = self.rig(isolation="systemd", mem_available_kb=450 * 1024)
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertNotIn("after waiting", rig.condition("host.memory_headroom")["detail"])
        rig.stop(); rig.finish()

    def test_a_stop_request_ends_the_wait_for_memory_at_once(self):
        rig = self.rig(isolation="systemd", mem_available_kb=300 * 1024, limits={"headroom_wait_s": 30})
        rig.start()
        time.sleep(1.5)
        began = time.time()
        rig.stop()
        code = rig.finish()
        self.assertLess(time.time() - began, 8, "the stop request did not end the wait")
        self.assertNotEqual(code, 0)
        self.assertEqual(rig.status()["failed"]["id"], "host.memory_headroom")
        self.assertIn("stopped while waiting for the host to have the memory", rig.status()["failed"]["detail"])
        self.assertIsNone(rig.browser_pid())


class LeanBrowserTests(RigCase):
    """The browser on the appliance is a small Raspberry Pi: the page is validated in a background tab and the audio goes to Chrome's own fake
    sink, which is what takes a software-rendering browser from several cores to a fraction of one. Both are verified, not assumed."""

    def test_the_page_is_opened_in_a_background_tab_and_that_is_the_tab_that_is_validated(self):
        rig = self.rig()
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.read_world("created_target.json"), {"url": "about:blank", "background": True})
        self.assertEqual(rig.read_world("attached_target.json")["targetId"], "T2", "the validator must work on the background tab, not on the one in front")
        self.assertEqual(rig.status()["page_visibility"], "hidden", "the evidence says whether the page really was hidden")
        rig.stop(); self.assertEqual(rig.finish(), 0)

    def test_a_browser_that_cannot_open_a_background_tab_is_validated_in_the_tab_it_has(self):
        rig = self.rig(scenario={"no_background_tab": True})
        rig.start()
        self.assertEqual(rig.wait_ready()[0], 0)
        self.assertEqual(rig.read_world("attached_target.json")["targetId"], "T1")
        self.assertEqual(rig.status()["page_visibility"], "visible")
        self.assertIn("background_tab_unavailable", [e["event"] for e in rig.log_events()])
        rig.stop(); self.assertEqual(rig.finish(), 0)

    def test_the_audio_sink_is_chromes_fake_one_and_the_sandbox_stays_on(self):
        flags = V.browser_flags("/tmp/profile", "pin=", sandbox=True)
        self.assertIn("--disable-audio-output", flags)
        self.assertNotIn("--alsa-output-device=null", flags, "an ALSA null device is rendered against flat out; the fake output is paced by a timer")
        self.assertNotIn("--no-sandbox", flags)
        self.assertIn("--no-sandbox", V.browser_flags("/tmp/profile", None, sandbox=False))


class PreconditionTests(RigCase):
    def no_browser(self, rig):
        self.assertIsNone(rig.browser_pid(), "a browser was started although the target is not running")

    def test_stale_owner_telemetry(self):
        rig = self.rig(freeze_owner=True)
        self.run_to_failure(rig, "owner.fresh_telemetry", detail="age 6")
        self.no_browser(rig)

    def test_the_wrong_release_is_running(self):
        rig = self.rig(bridge_commit=OTHER_COMMIT)
        self.run_to_failure(rig, "release.running", detail=OTHER_COMMIT)
        self.no_browser(rig)

    def test_readyz_reports_another_commit(self):
        rig = self.rig(ready_commit=OTHER_COMMIT, limits={"ready_url_wait_s": 1.5})
        self.run_to_failure(rig, "release.running", detail=f"wanted {COMMIT}")
        self.no_browser(rig)

    def test_a_legacy_install_outside_the_release_tree_is_accepted_only_when_asked(self):
        rig = self.rig(bridge_commit=OTHER_COMMIT)
        rig.start(("--no-exe-check",))
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        self.assertIn("readyz reports it;", rig.condition("release.running")["detail"])
        rig.stop(); rig.finish()

    def test_readyz_says_not_ready_even_for_the_right_commit(self):
        rig = self.rig(ready_flag=False, limits={"ready_url_wait_s": 1.5})
        self.run_to_failure(rig, "release.running", detail="ready=False")
        self.no_browser(rig)

    def test_a_service_without_a_main_process(self):
        rig = self.rig(bridge_pid_file="0\n")
        self.run_to_failure(rig, "release.running", detail="saturn-bridge.service has no main process")
        self.no_browser(rig)

    def test_readyz_not_ready(self):
        rig = self.rig(ready_commit=None, limits={"ready_url_wait_s": 1.5})
        self.run_to_failure(rig, "release.running")
        self.no_browser(rig)

    def test_the_status_file_never_exposes_the_credential_on_early_failure(self):
        rig = self.rig(bridge_commit=OTHER_COMMIT)
        self.run_to_failure(rig, "release.running")
        self.assertNoSecret(rig)


class WaitStopTests(unittest.TestCase):
    def test_wait_times_out_with_no_status(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([sys.executable, str(VALIDATOR), "wait", "--control-dir", directory, "--timeout", "1"], capture_output=True, text=True)
            self.assertEqual(result.returncode, V.EXIT_TIMEOUT)
            self.assertEqual(json.loads(result.stdout)["state"], "timeout")

    def test_wait_notices_a_dead_validator(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "status.json").write_text(json.dumps({"state": "ready", "failed": None}))
            dead = subprocess.Popen(["true"]); dead.wait()
            Path(directory, "pid").write_text(f"{dead.pid}\n")
            result = subprocess.run([sys.executable, str(VALIDATOR), "wait", "--control-dir", directory, "--timeout", "3"], capture_output=True, text=True)
            self.assertEqual(result.returncode, V.EXIT_FAILED)
            output = json.loads(result.stdout)
            self.assertEqual((output["state"], output["failed"]["id"]), ("died", "validator.died"))

    def stubborn_validator(self, directory, state):
        """A live process the validator's commands will recognise as the validator, which ignores the stop file."""
        process = subprocess.Popen([sys.executable, "-c", "import signal, time; time.sleep(120)", "saturn-remote-validate"],
                                   start_new_session=True)
        self.addCleanup(lambda: (process.kill(), process.wait()))
        Path(directory, "pid").write_text(f"{process.pid}\n")
        Path(directory, "status.json").write_text(json.dumps({"state": state, "failed": {"id": "x.y", "detail": "z"}}))
        return process

    def test_wait_reports_a_failed_validator_at_once_even_while_it_is_still_exiting(self):
        with tempfile.TemporaryDirectory() as directory:
            self.stubborn_validator(directory, "failed")
            started = time.time()
            result = subprocess.run([sys.executable, str(VALIDATOR), "wait", "--control-dir", directory, "--timeout", "20"], capture_output=True, text=True)
            self.assertLess(time.time() - started, 10)
            self.assertEqual(result.returncode, V.EXIT_FAILED)
            self.assertEqual(json.loads(result.stdout)["state"], "failed")

    def test_stop_ends_a_validator_that_ignores_it(self):
        with tempfile.TemporaryDirectory() as directory:
            process = self.stubborn_validator(directory, "ready")
            result = subprocess.run([sys.executable, str(VALIDATOR), "stop", "--control-dir", directory, "--timeout", "1"], capture_output=True, text=True, timeout=60)
            self.assertEqual(json.loads(result.stdout)["forced"], True)
            self.assertEqual(result.returncode, V.EXIT_FAILED)
            deadline = time.time() + 5
            while time.time() < deadline and alive(process.pid):
                time.sleep(0.1)
            self.assertFalse(alive(process.pid), "a validator that ignores stop must be ended")

    def test_stop_on_a_missing_validator_does_not_hang(self):
        with tempfile.TemporaryDirectory() as directory:
            started = time.time()
            result = subprocess.run([sys.executable, str(VALIDATOR), "stop", "--control-dir", directory, "--timeout", "2"], capture_output=True, text=True)
            self.assertLess(time.time() - started, 10)
            self.assertEqual(result.returncode, V.EXIT_FAILED)


class UnitTests(unittest.TestCase):
    def test_allowlist_matches_the_injected_guard(self):
        guard = GUARD.read_text()
        listed = re.search(r"ALLOWED_COMMANDS = Object\.freeze\(\[(.*?)\]\)", guard, re.S).group(1)
        self.assertEqual(set(re.findall(r'"([a-z_]+)"', listed)), set(V.ALLOWED_COMMANDS))

    def test_the_wire_audit_allows_nothing_that_changes_the_radio(self):
        for command in ("vfo", "dds", "modulation", "trx", "tx_enable", "drive", "rx_filter_band", "start", "stop", "iq_samplerate"):
            self.assertNotIn(command, V.ALLOWED_COMMANDS)

    def test_redactor_masks_the_password_the_whole_credential_and_its_base64(self):
        import base64
        red = V.Redactor()
        red.add(SECRET)
        text = f"x {SECRET} y {SECRET_PASSWORD} z Authorization: Basic {base64.b64encode(SECRET.encode()).decode()}"
        cleaned = red.clean(text)
        for needle in (SECRET_PASSWORD, base64.b64encode(SECRET.encode()).decode()):
            self.assertNotIn(needle, cleaned)

    def test_recorder_keeps_the_first_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            rec = V.Recorder(Path(directory), V.Redactor())
            rec.condition("a", True, "fine")
            rec.condition("b", False, "first")
            rec.condition("c", False, "second")
            self.assertEqual(rec.data["failed"], {"id": "b", "detail": "first"})
            self.assertEqual(json.loads((Path(directory) / "status.json").read_text())["failed"]["id"], "b")

    def test_systemd_launch_isolates_the_browser(self):
        command = V.build_launch_command("systemd", "/usr/bin/chromium", ["--headless=new"], "saturn-remote-validator-1", 300, "350M")
        text = " ".join(command)
        for needed in ("DynamicUser=yes", "IPAddressDeny=any", "IPAddressAllow=localhost", "MemoryMax=350M", "NoNewPrivileges=yes", "PrivateDevices=yes",
                       "ProtectHome=yes", "ProtectSystem=strict", "RuntimeMaxSec=330", "--pipe", "--unit=saturn-remote-validator-1", "TasksMax=",
                       "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK"):
            self.assertIn(needed, text)
        self.assertEqual(command[0], "systemd-run")
        self.assertNotIn("User=root", text)

    def test_the_defaults_are_the_strict_ones(self):
        args = V.parser().parse_args(["run", "--control-dir", "/x", "--target-commit", "c"])
        self.assertEqual((args.viewer_role, args.isolation, args.memory_max, args.no_sandbox), ("required", "required", "350M", False))

    def test_parse_size_reads_what_systemd_reads(self):
        self.assertEqual(V.parse_size("350M"), 350 * 1024 ** 2)
        self.assertEqual(V.parse_size("1g"), 1024 ** 3)
        self.assertEqual(V.parse_size("512"), 512)
        self.assertEqual(V.parse_size("2K"), 2048)
        for bad in ("", "M", "1.5G", "-1", "350 MB", "infinity"):
            with self.assertRaises(ValueError, msg=bad):
                V.parse_size(bad)

    def test_sandbox_verdict(self):
        def proc(kind, seccomp, flag=False, pid=1):
            return {"pid": pid, "type": kind, "seccomp": seccomp, "no_new_privs": "1", "no_sandbox_flag": flag}
        good = [proc("browser", 0), proc("gpu-process", 0), proc("renderer", 2, pid=2), proc("renderer", 2, pid=3)]
        self.assertTrue(V.sandbox_verdict(good)[0])
        self.assertFalse(V.sandbox_verdict([])[0])
        self.assertFalse(V.sandbox_verdict([proc("browser", 0)])[0])
        self.assertFalse(V.sandbox_verdict(good + [proc("renderer", 0, pid=4)])[0], "one loose renderer is enough")
        self.assertFalse(V.sandbox_verdict(good + [proc("renderer", 1, pid=4)])[0], "strict mode (1) is not the filter the sandbox installs")
        self.assertFalse(V.sandbox_verdict([proc("browser", 0, flag=True), proc("renderer", 2)])[0])
        self.assertFalse(V.sandbox_verdict([proc("browser", 0), proc("renderer", 2, flag=True)])[0])
        self.assertFalse(V.sandbox_verdict([proc("browser", 0), proc("renderer", -1)])[0], "an unreadable Seccomp field proves nothing")

    def test_memory_verdict(self):
        props = {"MemoryMax": str(350 * 1024 ** 2), "MemorySwapMax": "0", "MemoryCurrent": "5"}
        self.assertTrue(V.memory_verdict(props, "350M")[0])
        self.assertFalse(V.memory_verdict(props, "351M")[0])
        self.assertFalse(V.memory_verdict(props, "350")[0])
        self.assertFalse(V.memory_verdict(dict(props, MemoryMax="infinity"), "350M")[0])
        self.assertFalse(V.memory_verdict({k: v for k, v in props.items() if k != "MemoryMax"}, "350M")[0])
        self.assertFalse(V.memory_verdict(dict(props, MemorySwapMax="infinity"), "350M")[0])
        self.assertFalse(V.memory_verdict({k: v for k, v in props.items() if k != "MemorySwapMax"}, "350M")[0])

    def test_the_restart_waits_are_long_enough_for_a_restarted_service_and_bounded(self):
        for key in ("owner_wait_s", "headroom_wait_s"):
            self.assertGreaterEqual(V.DEFAULT_LIMITS[key], 10, f"{key} is too short for a Bridge that has just been restarted")
            self.assertLessEqual(V.DEFAULT_LIMITS[key], 60, f"{key} is not bounded well inside the helper's own validation time limit")

    def test_the_unit_is_the_oom_killers_first_choice(self):
        command = V.build_launch_command("systemd", "/usr/bin/chromium", ["--headless=new"], "saturn-remote-validator-1", 300, "350M")
        self.assertIn("OOMScoreAdjust=1000", command)

    def test_the_memory_enforcement_defaults_to_asking_the_kernel(self):
        args = V.parser().parse_args(["run", "--control-dir", "/x", "--target-commit", "c"])
        self.assertEqual(args.memory_enforcement, "auto")

    def validator_for(self, cgroup_root, enforcement="auto"):
        args = V.parser().parse_args(["run", "--control-dir", "/x", "--target-commit", "c", "--memory-enforcement", enforcement, "--cgroup-root", str(cgroup_root)])
        with tempfile.TemporaryDirectory() as directory:
            return V.Validator(args, V.Recorder(Path(directory), V.Redactor()), V.Redactor())

    def test_the_memory_enforcement_follows_the_kernels_memory_controller(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(self.validator_for(root).memory_enforcement, "cgroup", "a kernel that cannot be asked is assumed to apply the cap")
            (root / "cgroup.controllers").write_text("cpuset cpu io memory pids\n")
            self.assertEqual(self.validator_for(root).memory_enforcement, "cgroup")
            (root / "cgroup.controllers").write_text("cpuset cpu io pids\n")
            self.assertEqual(self.validator_for(root).memory_enforcement, "watchdog", "the Raspberry Pi image: cgroup_disable=memory")
            (root / "cgroup.controllers").write_text("cpuset cpu io memorypressure pids\n")
            self.assertEqual(self.validator_for(root).memory_enforcement, "watchdog", "only the controller named exactly 'memory' counts")
            self.assertEqual(self.validator_for(root, "cgroup").memory_enforcement, "cgroup", "an explicit choice is never second-guessed")
            (root / "cgroup.controllers").write_text("cpuset cpu io memory pids\n")
            self.assertEqual(self.validator_for(root, "watchdog").memory_enforcement, "watchdog")

    def test_the_watchdog_verdict_needs_the_oom_setting_readings_and_a_browser_within_the_cap(self):
        props = {"OOMScoreAdjust": "1000"}
        watch = {"steps": 20, "peak": 200 * 1024 ** 2, "max_gap": 0.4, "age": 0.3, "exceeded": False, "detail": ""}
        ok, detail = V.watchdog_memory_verdict(props, "350M", watch)
        self.assertTrue(ok, detail)
        self.assertIn("ENFORCED BY THE VALIDATOR'S WATCHDOG", detail)
        self.assertIn("200 MiB", detail)
        for name, bad_props, bad_watch, wanted in (
                ("oom setting missing", {}, watch, "first choice"),
                ("oom setting 0", {"OOMScoreAdjust": "0"}, watch, "first choice"),
                ("exceeded", props, dict(watch, exceeded=True, detail="400 MiB"), "exceeded its memory cap"),
                ("no readings", props, dict(watch, steps=0, peak=0), "could not be measured"),
                ("one reading", props, dict(watch, steps=1), "could not be measured"),
                ("nothing read", props, dict(watch, peak=0), "could not be measured"),
                ("stopped reading", props, dict(watch, age=2.5), "stopped reading"),
                ("gap", props, dict(watch, max_gap=3.5), "without a reading"),
                ("over the cap but not tripped", props, dict(watch, peak=351 * 1024 ** 2), "against the 350M cap")):
            ok, detail = V.watchdog_memory_verdict(bad_props, "350M", bad_watch)
            self.assertFalse(ok, name)
            self.assertIn(wanted, detail, name)
        self.assertTrue(V.watchdog_memory_verdict(props, "350M", dict(watch, peak=350 * 1024 ** 2))[0], "exactly the cap is within it")

    def test_process_usage_reads_memory_and_cpu_and_survives_a_missing_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "7").mkdir()
            (root / "7" / "stat").write_text("7 (chromium (renderer)) S 1 0 0 0 0 0 0 0 11 0 300 60 0 0\n")      # a name with spaces and a parenthesis
            (root / "7" / "status").write_text("Name:\tx\nVmRSS:\t 2048 kB\n")
            (root / "7" / "smaps_rollup").write_text("Rss: 2048 kB\nPss:               1024 kB\n")
            self.assertEqual(V.process_usage(root, 7), {"majflt": 11, "ticks": 360, "rss": 2048 * 1024, "pss": 1024 * 1024})
            (root / "7" / "smaps_rollup").unlink()
            self.assertEqual(V.process_usage(root, 7)["pss"], 2048 * 1024, "without Pss the resident size is the (larger) stand-in")
            (root / "7" / "status").unlink()
            self.assertIsNone(V.process_usage(root, 7), "no memory figure at all means the process is gone")
            self.assertIsNone(V.process_usage(root, 8))

    def test_host_counters_read_what_the_kernel_provides_and_skip_the_rest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(V.host_counters(root), {}, "this Pi has no pressure-stall information and a tool must not need it")
            (root / "stat").write_text("cpu  100 5 20 180 100 2 3 4 0 0\ncpu0 1 1 1 1 1 1 1 1 0 0\n")
            (root / "vmstat").write_text("nr_free_pages 5\npswpin 7\npswpout 9\npgmajfault 11\n")
            (root / "meminfo").write_text("MemTotal: 927000 kB\nMemAvailable: 500000 kB\nSwapTotal: 3000000 kB\nSwapFree: 2700000 kB\n")
            (root / "loadavg").write_text("1.25 0.5 0.3 1/2 3\n")
            self.assertEqual(V.host_counters(root), {"cpu_user": 105, "cpu_system": 25, "cpu_idle": 180, "cpu_iowait": 100, "cpu_steal": 4,
                                                     "pswpin": 7, "pswpout": 9, "pgmajfault": 11, "MemAvailable": 500000 * 1024,
                                                     "SwapTotal": 3000000 * 1024, "SwapFree": 2700000 * 1024, "load1_x100": 125})

    def test_the_diagnostics_summary_says_where_the_resources_went(self):
        samples = [{"by_type": {"renderer": [1, 60.0], "browser": [1, 40.0]}, "pss_mb": 100.0, "rss_mb": 150.0, "browser_cpu_pct": 80, "host_busy_pct": 30, "host_iowait_pct": 20, "load1": 1.0, "mem_avail_mb": 400,
                    "swap_in_pages": 5, "swap_out_pages": 1, "browser_major_faults": 2, "owner": {"outbound_drops": 0, "iq_tci_drop_queue_overflow_total": 10, "connections": 0}},
                   {"by_type": {"renderer": [3, 140.0], "browser": [1, 60.0]}, "pss_mb": 200.0, "rss_mb": 260.0, "browser_cpu_pct": 120, "host_busy_pct": 50, "host_iowait_pct": 40, "load1": 2.0, "mem_avail_mb": 300,
                    "swap_in_pages": 7, "swap_out_pages": 2, "browser_major_faults": 3, "owner": {"outbound_drops": 0, "iq_tci_drop_queue_overflow_total": 40, "connections": 2}}]
        page = [(100.0, {"TaskDuration": 1.0, "ScriptDuration": 0.5, "JSHeapUsedSize": 20e6}), (110.0, {"TaskDuration": 9.0, "ScriptDuration": 3.5, "JSHeapUsedSize": 60e6})]
        s = V.summarize_diagnostics(samples, page)
        self.assertEqual(s["browser_pss_mb"], {"max": 200.0, "mean": 150.0})
        self.assertEqual(s["browser_cpu_pct"], {"max": 120.0, "mean": 100.0})
        self.assertEqual(s["host_iowait_pct"], {"max": 40.0, "mean": 30.0})
        self.assertEqual((s["mem_available_mb_min"], s["swap_in_pages_total"], s["swap_out_pages_total"], s["browser_major_faults_total"]), (300, 12, 3, 5))
        self.assertEqual(s["owner_counters"]["iq_tci_drop_queue_overflow_total"], {"first": 10, "last": 40, "delta": 30})
        self.assertEqual(s["owner_counters"]["connections"]["delta"], 2)
        self.assertEqual(s["pss_by_process_type_mb_max"], {"browser": {"processes": 1, "pss_mb": 60.0}, "renderer": {"processes": 3, "pss_mb": 140.0}},
                         "where the memory went: the most each kind of process held")
        self.assertEqual(s["page_main_thread_busy"], 0.8)
        self.assertEqual(s["page_script_busy"], 0.3)
        self.assertEqual(s["page_js_heap_used_mb_max"], round(60e6 / 1048576, 1))
        self.assertEqual(V.summarize_diagnostics([], []), {"samples": 0}, "a run that measured nothing says so and invents nothing")

    def test_descendants_follow_the_parent_pids(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for pid, parent in ((10, 1), (11, 10), (12, 11), (13, 10), (20, 1), (21, 20)):
                (root / str(pid)).mkdir()
                (root / str(pid) / "stat").write_text(f"{pid} (a b) c) S {parent} 0 0\n")        # a command name with spaces and a parenthesis
            (root / "self").mkdir()
            self.assertEqual(sorted(V.descendants(root, 10)), [10, 11, 12, 13])
            self.assertEqual(V.descendants(root, 99), [99])

    def test_proc_snapshot_reads_the_kind_and_the_filter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "7").mkdir()
            (root / "7" / "cmdline").write_bytes(b"/usr/bin/chromium\0--type=renderer\0--lang=en\0")
            (root / "7" / "status").write_text("Name:\tchromium\nSeccomp:\t2\nNoNewPrivs:\t1\n")
            snap = V.proc_snapshot(root, 7)
            self.assertEqual((snap["type"], snap["seccomp"], snap["no_sandbox_flag"]), ("renderer", 2, False))
            (root / "7" / "cmdline").write_bytes(b"/usr/bin/chromium\0--no-sandbox\0")
            snap = V.proc_snapshot(root, 7)
            self.assertEqual((snap["type"], snap["no_sandbox_flag"]), ("browser", True))
            # a process forked by the browser retitles itself: one string, arguments separated by spaces (found by running a real Chrome)
            (root / "7" / "cmdline").write_bytes(b"/opt/google/chrome/chrome --type=renderer --no-sandbox --lang=en-US\0\0\0")
            snap = V.proc_snapshot(root, 7)
            self.assertEqual((snap["type"], snap["no_sandbox_flag"]), ("renderer", True))
            (root / "7" / "cmdline").write_bytes(b"/opt/google/chrome/chrome --type=renderer --lang=en-US\0\0\0")
            self.assertEqual(V.proc_snapshot(root, 7)["type"], "renderer")
            self.assertIsNone(V.proc_snapshot(root, 8))

    def test_direct_launch_maps_the_pipe_to_descriptors_3_and_4(self):
        command = V.build_launch_command("direct", "/b", ["--x"], "u", 1, "1M")
        self.assertEqual(command[:2], ["/bin/sh", "-c"])
        self.assertIn("3<&0 4>&1", command[2])
        self.assertEqual(command[-2:], ["/b", "--x"])
        with self.assertRaises(ValueError):
            V.build_launch_command("bogus", "/b", [], "u", 1, "1M")

    def test_browser_flags_pin_the_certificate_instead_of_ignoring_errors(self):
        flags = V.browser_flags("/p", "ABC=", sandbox=True)
        self.assertIn("--remote-debugging-pipe", flags)
        self.assertIn("--ignore-certificate-errors-spki-list=ABC=", flags)
        self.assertNotIn("--ignore-certificate-errors", flags)
        self.assertNotIn("--no-sandbox", flags)
        self.assertIn("--no-sandbox", V.browser_flags("/p", None, sandbox=False))
        self.assertFalse([f for f in V.browser_flags("/p", None, True) if "spki" in f])

    @unittest.skipUnless(shutil.which("openssl"), "openssl is needed")
    def test_spki_pin_matches_the_standard_pipeline(self):
        with tempfile.TemporaryDirectory() as directory:
            key, cert = Path(directory, "k.pem"), Path(directory, "c.pem")
            subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", str(key), "-out", str(cert),
                            "-subj", "/CN=127.0.0.1", "-days", "2"], check=True, capture_output=True)
            expected = subprocess.run(f"openssl x509 -in {cert} -pubkey -noout | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | base64",
                                      shell=True, capture_output=True, text=True, check=True).stdout.strip()
            self.assertEqual(V.spki_pin(cert), expected)

    def good_p23(self, **state):
        base = {"sdr_active": True, "thread_error": False}
        base.update(state)
        return {"perf": {"workload": {"selected_app": "xdma", "service_main_pid": 77},
                         "app_telemetry": {"snapshot_readable": True, "pid_matches_service": True, "age_seconds": 0.4,
                                           "current": {"app": "saturn-bridge", "pid": 77, "state": base}}}}

    def failing(self, p23):
        return [name for name, _, ok in V.gate_conditions(p23, "xdma") if not ok]

    def test_gate_conditions_mirror_the_activation_helpers_gate(self):
        self.assertEqual(self.failing(self.good_p23()), [])
        self.assertEqual(len(V.gate_conditions(self.good_p23(), "xdma")), 7)
        self.assertEqual(self.failing(self.good_p23(sdr_active=False)), ["state.sdr_active is true (a receive client is attached)"])
        self.assertEqual(self.failing(self.good_p23(thread_error=True)), ["state.thread_error is not true"])
        cases = {
            "selected_app": lambda d: d["perf"]["workload"].update(selected_app="p2"),
            "app": lambda d: d["perf"]["app_telemetry"]["current"].update(app="p2app"),
            "pid": lambda d: d["perf"]["app_telemetry"]["current"].update(pid=78),
            "readable": lambda d: d["perf"]["app_telemetry"].update(snapshot_readable=False),
            "pid_matches": lambda d: d["perf"]["app_telemetry"].update(pid_matches_service=False),
            "old": lambda d: d["perf"]["app_telemetry"].update(age_seconds=5.5),
            "negative age": lambda d: d["perf"]["app_telemetry"].update(age_seconds=-1),
            "no age": lambda d: d["perf"]["app_telemetry"].update(age_seconds=None),
            "zero pid": lambda d: (d["perf"]["workload"].update(service_main_pid=0), d["perf"]["app_telemetry"]["current"].update(pid=0)),
        }
        for name, mutate in cases.items():
            doc = self.good_p23()
            mutate(doc)
            self.assertTrue(self.failing(doc), f"{name} did not fail the gate")
        self.assertEqual(self.failing({}), ["p23_perf has the expected structure"])
        self.assertEqual(self.failing(None), ["p23_perf has the expected structure"])

    def checker(self, **limits):
        validator = V.Validator.__new__(V.Validator)
        validator.limits = dict(V.DEFAULT_LIMITS, **limits)
        return validator

    def test_data_checks_need_real_progress_in_every_stream(self):
        validator = self.checker()
        a = {"opusFrames": 0, "audioFramesPlayed": 0, "displayRows": 0, "iqFrames": 0, "rxIqFrames": 0, "decodeErrors": 0, "codec": "opus", "audioContextState": "running"}
        b = dict(a, opusFrames=150, audioFramesPlayed=150, displayRows=90)
        self.assertTrue(all(ok for _, ok, _ in validator.data_checks(a, b)))
        for key in ("opusFrames", "audioFramesPlayed", "displayRows"):
            c = dict(b); c[key] = 0
            self.assertEqual([i for i, ok, _ in validator.data_checks(a, c) if not ok], {"opusFrames": ["data.audio_frames_arriving"], "audioFramesPlayed": ["audio.decoded_progress"], "displayRows": ["data.display_fresh"]}[key])
        raw_iq = dict(b, displayRows=0, iqFrames=60)
        self.assertTrue(all(ok for _, ok, _ in validator.data_checks(a, raw_iq)), "raw IQ frames also count as fresh display data")
        self.assertFalse(all(ok for _, ok, _ in validator.data_checks(a, dict(b, decodeErrors=1))))
        self.assertFalse(all(ok for _, ok, _ in validator.data_checks(a, dict(b, audioContextState="suspended"))))
        self.assertFalse(all(ok for _, ok, _ in validator.data_checks(a, dict(b, codec="pcm"))))
        # counters that are merely high, but not advancing, are not progress
        high = dict(a, opusFrames=9999, audioFramesPlayed=9999, displayRows=9999)
        self.assertFalse(all(ok for _, ok, _ in validator.data_checks(high, dict(high))))

    def test_a_receiver_attaching_changes_the_processing_mode_but_that_is_not_a_radio_change(self):
        self.assertNotIn("processing_mode", V.RADIO_METRICS)

    def test_frame_offenders_judges_one_frame(self):
        judge = V.frame_offenders
        self.assertEqual(judge({"opcode": 1, "data": "audio_start:0;iq_start:0;"}), [])
        self.assertEqual(judge({"opcode": 1, "data": "AUDIO_START:0;Saturn_Ping:1,2;"}), [])
        self.assertEqual(judge({"opcode": 1, "data": "audio_start:0;vfo:0,0,1;trx:0,true;"}), ["vfo:0,0,1", "trx:0,true"])
        self.assertEqual(judge({"opcode": 2, "data": "\x00"}), ["binary frame"])
        self.assertEqual(judge({"opcode": 1, "data": ""}), [])
        self.assertEqual(judge({"opcode": 1, "data": ";;  ;"}), [])

    def test_radio_state_covers_tuning_rate_input_and_tx(self):
        perf = {"backend": "xdma", "rf_safe": True, "metrics": {"frequency_hz": 1, "sample_rate_hz": 2, "ddc": 3, "adc": "ADC1", "processing_mode": "audio",
                                                              "tx_keyed": False, "tx_stream": False, "client": 1, "connections": 9}}
        state = V.radio_state(perf)
        self.assertEqual(set(state), set(V.RADIO_METRICS) | set(V.RADIO_TOP))
        self.assertNotIn("connections", state)


def run_parallel(jobs):
    """Every scenario owns its fake environment, so they can run side by side. The slowest are the ones that wait out a bounded timeout."""
    import concurrent.futures
    loader = unittest.TestLoader()
    tests = [t for group in loader.loadTestsFromModule(sys.modules[__name__]) for sub in group for t in (sub if isinstance(sub, unittest.TestSuite) else [sub])]
    results = []

    def one(test):
        started = time.time()
        result = unittest.TestResult()
        test.run(result)
        return test, result, time.time() - started

    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        for test, result, took in pool.map(one, tests):
            bad = result.errors + result.failures
            results.append((took, test, bad, result.skipped))
            print(("FAIL " if bad else "ok   ") + f"{took:6.1f}s  {test.id().split('.', 1)[-1]}", flush=True)
            for _, trace in bad:
                print(textwrap.indent(trace, "    "), flush=True)
    failed = [r for r in results if r[2]]
    print(f"\nRan {len(results)} tests, {len(failed)} failed, slowest {max(r[0] for r in results):.1f}s")
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--parallel":
        sys.exit(run_parallel(int(sys.argv[2]) if len(sys.argv) > 2 else 8))
    unittest.main(verbosity=2)
