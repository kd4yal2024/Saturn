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
        "cleanup_s": 4, "call_s": 4, "poll_s": 0.2, "min_audio_played": 20, "min_opus_frames": 20, "min_display": 5}

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
    if method == "Target.attachToTarget": return reply({"sessionId": "S1"})
    if method == "Page.addScriptToEvaluateOnNewDocument":
        put("injected.json", {"sha256": __import__("hashlib").sha256(params["source"].encode()).hexdigest(), "length": len(params["source"])})
        return reply({"identifier": "1"})
    if method == "Fetch.enable": put("fetch_enable.json", params); return reply()
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
if [ "$1" = show ]; then cat "$FAKE_PIDS/$5" 2>/dev/null || echo 0; exit 0; fi
echo "$@" >> "$FAKE_PIDS/systemctl.log"
exit 0
"""


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
                 freeze_on_session=False, owner_blind=False, p23_broken=False, sticky=False):
        self.test = test
        self.root = Path(tempfile.mkdtemp(prefix="saturn-validate-test-"))
        self.world = self.root / "world"
        self.world.mkdir()
        self.ctrl = self.root / "ctrl"
        self.releases = self.root / "releases"
        self.pids = self.root / "pids"
        self.pids.mkdir()
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
        return {"pid": self.perf_pid or self.bridge_pid, "client": 1 if (self.baseline > 0 or session or extra > 0) else 0,
                "connections": self.baseline + extra + (2 if session else 0),
                "frequency_hz": frequency, "sample_rate_hz": 384000, "ddc": 6, "adc": "ADC1", "processing_mode": "audio" if (session or self.baseline > 0 or extra > 0) else "drain-only", "tx_keyed": tx_keyed,
                "tx_stream": False, "rx_counter_v31": {"status": self.rxc1}}

    def _age(self):
        return 60.0 if (self.freeze or (self.freeze_on_session and self._session())) else 0.0

    def _sdr(self):
        """What the real owner reports: a receive client is attached (a human's, or ours)."""
        return (not self.sdr_never) and (self._seen() or self.baseline > 0)

    def _write_perf(self, initial=False):
        stale = self.freeze or (self.freeze_on_session and self._session())
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
                "--systemctl", str(self.systemctl), "--limits", json.dumps(self.limits), *extra]

    def env(self):
        env = {"PATH": os.environ["PATH"], "FAKE_WORLD": str(self.world), "FAKE_PIDS": str(self.pids), "FAKE_SCENARIO": json.dumps(self.scenario)}
        return env

    def start(self, extra=()):
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

    def test_a_bridge_that_predates_viewer_enforcement_is_accepted_when_the_guard_is_proven(self):
        rig = self.rig(scenario={"role": "operator"})
        rig.start(("--viewer-role", "preferred"))
        code, ready = rig.wait_ready()
        self.assertEqual((code, ready["state"]), (0, "ready"), ready)
        status = rig.status()
        self.assertEqual((status["role"], status["role_enforced_by_bridge"]), ("operator", False))
        self.assertTrue(status["conditions"]["guard.active"]["ok"])
        self.assertIn("predates viewer enforcement", status["conditions"]["session.connected_as_viewer"]["detail"])
        rig.stop(); self.assertEqual(rig.finish(), 0)

    def test_preferred_still_refuses_any_other_role(self):
        rig = self.rig(scenario={"role": "observer"})
        rig.start(("--viewer-role", "preferred"))
        rig.finish()
        self.assertEqual(rig.status()["failed"]["id"], "session.connected_as_viewer")

    def test_a_viewer_is_recorded_as_enforced_by_the_bridge(self):
        rig = self.rig()
        rig.start(("--viewer-role", "preferred"))
        self.assertEqual(rig.wait_ready()[0], 0)
        status = rig.status()
        self.assertEqual((status["role"], status["role_enforced_by_bridge"]), ("viewer", True))
        rig.stop(); rig.finish()

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
                       "ProtectHome=yes", "ProtectSystem=strict", "RuntimeMaxSec=330", "--pipe", "--unit=saturn-remote-validator-1", "TasksMax="):
            self.assertIn(needed, text)
        self.assertEqual(command[0], "systemd-run")
        self.assertNotIn("User=root", text)

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
