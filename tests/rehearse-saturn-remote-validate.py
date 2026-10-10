#!/usr/bin/env python3
"""Rehearse the headless receive validation against the REAL page, proxy, Bridge and Chrome, all on loopback.

Real:   the replay Bridge (the shipped saturn-bridge, driven by synthetic samples), the saturn-go proxy with its TLS listener and basic
        authentication, the page being installed, headless Chrome, and the validator exactly as the update process runs it.
Shimmed: only what the host's service manager would provide: /readyz and /p23_perf (derived from the real Bridge's own perf.json, with the
        Bridge's own definition of sdr_active), and `systemctl show -p MainPID`, which reports the real process ids.

Required environment (nothing here is a default for a particular machine):
  SATURN_REH_BRIDGE_BIN   saturn-bridge built with the real WDSP, started through tools/replay/run_replay_bridge.sh
  SATURN_REH_PROXY_BIN    the saturn-go proxy binary
  SATURN_REH_WEBROOT      page directory for the NEW release (templates plus the built saturn-remote-next.js)
  SATURN_REH_OLD_WEBROOT  page directory of the release a rollback returns to (its page predates validation mode)
  SATURN_REH_OUT          empty or new output directory (evidence is kept here)
Optional: SATURN_REH_WISDOM (FFTW wisdom file), SATURN_REH_CHROME (default google-chrome),
          SATURN_REH_OLD_BRIDGE_BIN (a saturn-bridge built WITHOUT the viewer-role fix: adds the "oldbridge" cases, which are the rollback target's
          situation: the Bridge assigns the operator role and only the injected guard, the live wire audit and the unchanged-radio check hold the line).

Usage: rehearse-saturn-remote-validate.py [new|legacy|stalled|nosandbox|oldbridge|all]
Exit status 0 only if every requested case behaved as required.
"""
import http.server
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
VALIDATOR = REPO / "update_manager/scripts/saturn-remote-validate.py"
GUARD = REPO / "update_manager/scripts/saturn-remote-receive-guard.js"
REPLAY = REPO / "update_manager/saturn-bridge/tools/replay/run_replay_bridge.sh"
BRIDGE_PORT, TLS_PORT, SHIM_PORT = 50141, 18443, 18081

ENV = {name: os.environ.get(name) for name in ("SATURN_REH_BRIDGE_BIN", "SATURN_REH_PROXY_BIN", "SATURN_REH_WEBROOT", "SATURN_REH_OLD_WEBROOT", "SATURN_REH_OUT")}
for name, value in ENV.items():
    if not value:
        sys.exit(f"set {name} (see the header of this file)")
OUT = Path(ENV["SATURN_REH_OUT"])
CHROME = os.environ.get("SATURN_REH_CHROME", "google-chrome")
results = []


def say(message):
    print(f"[rehearse] {message}", flush=True)


def check(condition, message):
    results.append((bool(condition), message))
    print(("  ok   " if condition else "  FAIL ") + message, flush=True)
    return bool(condition)


def wait_port(port, seconds=30):
    deadline = time.time() + seconds
    while time.time() < deadline:
        with socket.socket() as probe:
            probe.settimeout(0.3)
            if probe.connect_ex(("127.0.0.1", port)) == 0:
                return True
        time.sleep(0.3)
    return False


class Stack:
    """The replay Bridge and the proxy, each started from a copy placed in a release-shaped directory."""

    def __init__(self, webroot, label, bridge_bin=None):
        self.label = label
        self.bridge_bin = bridge_bin or ENV["SATURN_REH_BRIDGE_BIN"]
        self.dir = OUT / label
        self.dir.mkdir(parents=True)
        self.webroot = webroot
        self.work = self.dir / "work"
        self.state = self.dir / "state"
        self.state.mkdir()
        (self.dir / "login.txt").write_text("rehearsal:" + os.urandom(12).hex())
        os.chmod(self.dir / "login.txt", 0o600)
        self.login = (self.dir / "login.txt").read_text()
        self.procs = {}
        self.commit = None

    def popen(self, name, argv, env, **kwargs):
        out = open(self.dir / f"{name}.out", "w")
        proc = subprocess.Popen(argv, env=env, stdout=out, stderr=subprocess.STDOUT, start_new_session=True, **kwargs)
        self.procs[name] = proc
        return proc

    def start(self):
        env = dict(os.environ, SATURN_BRIDGE_TCI_NODELAY="0", SATURN_BRIDGE_WDSP_FLAVOR="wdsp2-2.10")
        proxy_env = {"PATH": os.environ["PATH"], "SATURN_WEBROOT": str(self.webroot), "SATURN_STATE_DIR": str(self.state), "SATURN_REPO_ROOT": str(REPO),
                     "SATURN_ADDR": "127.0.0.1:18080", "SATURN_REMOTE_TLS_ADDR": f"127.0.0.1:{TLS_PORT}", "SATURN_REMOTE_BASIC_AUTH": self.login,
                     "SATURN_REMOTE_BRIDGE_WS": f"ws://127.0.0.1:{BRIDGE_PORT}"}
        # The proxy's build commit names the release directory it must run from, so learn it first.
        probe = self.popen("proxy-probe", [ENV["SATURN_REH_PROXY_BIN"]], proxy_env)
        assert wait_port(18080), "proxy did not start"
        import urllib.error, urllib.request
        try:
            body = urllib.request.urlopen("http://127.0.0.1:18080/readyz", timeout=5)
        except urllib.error.HTTPError as error:  # a scratch proxy is "not_ready" (no systemd), but still names its build
            body = error
        self.commit = json.load(body)["build_commit"]
        os.killpg(probe.pid, signal.SIGTERM)
        probe.wait()
        time.sleep(1)
        release = self.dir / "releases" / self.commit / "bin"
        release.mkdir(parents=True)
        shutil.copy(self.bridge_bin, release / "saturn-bridge")
        shutil.copy(ENV["SATURN_REH_PROXY_BIN"], release / "saturn-go")
        self.release_root = self.dir / "releases"
        argv = ["bash", str(REPLAY), "--bridge", str(release / "saturn-bridge"), "--work", str(self.work), "--port", str(BRIDGE_PORT), "--tones", "1500:-30"]
        if os.environ.get("SATURN_REH_WISDOM"):
            argv += ["--wisdom", os.environ["SATURN_REH_WISDOM"]]
        self.popen("bridge", argv, env)
        deadline = time.time() + 90
        while time.time() < deadline:
            try:
                if json.loads((self.work / "perf.json").read_text())["metrics"].get("ddc_s", 0) > 800:
                    break
            except (OSError, ValueError, KeyError):
                pass
            time.sleep(0.5)
        else:
            raise RuntimeError("the replay Bridge never started receiving")
        self.popen("proxy", [str(release / "saturn-go")], proxy_env)
        assert wait_port(TLS_PORT), "the TLS listener did not start"
        self.shim()

    def perf(self):
        return json.loads((self.work / "perf.json").read_text())

    def bridge_pid(self):
        return self.perf()["metrics"]["pid"]

    def proxy_pid(self):
        return self.procs["proxy"].pid

    def shim(self):
        stack = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                perf = stack.perf()
                m = perf["metrics"]
                pid = m["pid"]
                if self.path.startswith("/readyz"):
                    body = {"ready": True, "build_commit": stack.commit, "status": "ready"}
                elif self.path.startswith("/p23_perf"):
                    age = max(0.0, time.time() - perf["updated_at_ms"] / 1000.0)
                    # the Bridge's own definition (rust-server main.rs): pid match, a client, an active stream, and samples flowing
                    sdr = m.get("client", 0) >= 1 and (m.get("iq", 0) >= 1 or m.get("audio", 0) >= 1) and m.get("ddc_s", 0) > 0
                    body = {"perf": {"workload": {"selected_app": "xdma", "service_main_pid": pid},
                                     "app_telemetry": {"snapshot_readable": True, "pid_matches_service": True, "age_seconds": age,
                                                       "current": {"app": "saturn-bridge", "pid": pid, "state": {"sdr_active": bool(sdr), "thread_error": False}}}}}
                else:
                    self.send_response(404); self.end_headers(); return
                data = json.dumps(body).encode()
                self.send_response(200); self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", SHIM_PORT), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        systemctl = self.dir / "systemctl"
        systemctl.write_text(f"""#!/bin/sh
if [ "$1" = show ]; then
  case "$5" in
    saturn-bridge.service) python3 -c "import json;print(json.load(open('{self.work}/perf.json'))['metrics']['pid'])" ;;
    saturn-go.service) echo {self.proxy_pid()} ;;
    *) echo 0 ;;
  esac
  exit 0
fi
exit 0
""")
        systemctl.chmod(0o755)
        self.systemctl = systemctl

    def stop(self):
        try:
            self.server.shutdown()
            self.server.server_close()
        except Exception:
            pass
        for name in ("proxy", "bridge", "proxy-probe"):
            proc = self.procs.get(name)
            if proc and proc.poll() is None:
                try:
                    os.killpg(proc.pid, signal.SIGCONT)
                    os.killpg(proc.pid, signal.SIGTERM)
                except OSError:
                    pass
        time.sleep(2)
        for proc in self.procs.values():
            if proc.poll() is None:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except OSError:
                    pass
        for proc in self.procs.values():
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass


def validator(stack, name, extra=(), limits=None, no_sandbox=False):
    ctrl = stack.dir / f"ctrl-{name}"
    argv = [sys.executable, str(VALIDATOR), "run", "--control-dir", str(ctrl), "--target-commit", stack.commit, "--backend", "xdma",
            "--url", f"https://127.0.0.1:{TLS_PORT}/remote-next?validation=receive&transport=split&display_transport=spectrum",
            "--ready-url", f"http://127.0.0.1:{SHIM_PORT}/readyz", "--perf-url", f"http://127.0.0.1:{SHIM_PORT}/p23_perf",
            "--perf-json", str(stack.work / "perf.json"), "--release-root", str(stack.release_root),
            "--tls-cert", str(stack.state / "remote-tls/saturn-remote.crt"), "--browser", CHROME, "--launcher", "direct",
            # the real Chrome runs WITH its sandbox; this host has no transient unit, so only the sandbox is verified here (the memory cap is
            # verified on the appliance, where the production launcher applies it)
            "--isolation", "sandbox", *(["--no-sandbox"] if no_sandbox else []),
            "--guard-script", str(GUARD), "--systemctl", str(stack.systemctl), "--limits", json.dumps(limits or {}), *extra]
    process = subprocess.Popen(argv, stdout=open(stack.dir / f"validator-{name}.out", "w"), stderr=subprocess.STDOUT, start_new_session=True)
    return ctrl, process


def cli(command, ctrl, *extra, timeout=120):
    result = subprocess.run([sys.executable, str(VALIDATOR), command, "--control-dir", str(ctrl), *extra], capture_output=True, text=True, timeout=timeout)
    try:
        return result.returncode, json.loads(result.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return result.returncode, {"raw": result.stdout, "err": result.stderr}


def chrome_running(stack):
    result = subprocess.run(["pgrep", "-f", f"saturn-validate-.*|{stack.dir}"], capture_output=True, text=True)
    return [p for p in result.stdout.split() if p and int(p) != os.getpid() and "pgrep" not in p and
            "--remote-debugging-pipe" in Path(f"/proc/{p}/cmdline").read_text(errors="replace")] if result.stdout else []


def case(webroot, label, expect_driver, stall=False, bridge_bin=None, viewer_role="required", expect_role="viewer", expect_refusal=False, no_sandbox=False):
    say(f"case {label}: page from {webroot}")
    stack = Stack(webroot, label, bridge_bin)
    try:
        stack.start()
        before = stack.perf()["metrics"]
        check(before.get("connections") == 0 and before.get("client") == 0, f"{label}: nothing is connected to the Bridge before validation (connections={before.get('connections')}, client={before.get('client')})")
        radio_before = {k: before.get(k) for k in ("frequency_hz", "sample_rate_hz", "ddc", "adc", "tx_keyed", "tx_stream")}
        ctrl, process = validator(stack, label, extra=("--viewer-role", viewer_role),
                                  limits={"max_seconds": 150, "data_s": 40, "connect_s": 40, "page_load_s": 40} if stall else {"max_seconds": 150},
                                  no_sandbox=no_sandbox)
        if no_sandbox:
            process.wait(timeout=140)
            status = json.loads((ctrl / "status.json").read_text())
            failed = status.get("failed") or {}
            check(status["state"] == "failed" and failed.get("id") == "browser.sandbox_intact",
                  f"{label}: a real Chrome started with --no-sandbox is refused: {failed.get('detail')}")
            check("Seccomp:0" in failed.get("detail", "") or "--no-sandbox" in failed.get("detail", ""), f"{label}: the refusal names what /proc showed")
            check(not chrome_running(stack), f"{label}: the temporary Chrome is gone after the refusal")
            time.sleep(2)
            check(stack.perf()["metrics"].get("connections") == 0, f"{label}: nothing of ours remains at the Bridge")
            return
        if expect_refusal:
            process.wait(timeout=140)
            status = json.loads((ctrl / "status.json").read_text())
            failed = (status.get("failed") or {}).get("id")
            check(status["state"] == "failed" and failed == "session.connected_as_viewer",
                  f"{label}: --viewer-role {viewer_role} refuses a Bridge that assigns {status.get('role')!r}: {(status.get('failed') or {}).get('detail')}")
            check(not chrome_running(stack), f"{label}: the temporary Chrome is gone after the refusal")
            time.sleep(2)
            check(stack.perf()["metrics"].get("connections") == 0, f"{label}: nothing of ours remains at the Bridge")
            return
        if stall:
            # stop the Bridge's data the moment the session is measuring: the Bridge accepted the connection, then media delivery fails
            deadline = time.time() + 60
            while time.time() < deadline:
                try:
                    if json.loads((ctrl / "status.json").read_text()).get("phase") == "measuring":
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(0.1)
            os.killpg(stack.procs["bridge"].pid, signal.SIGSTOP)
            say("Bridge stopped (media delivery failure injected)")
            process.wait(timeout=140)
            os.killpg(stack.procs["bridge"].pid, signal.SIGCONT)
            status = json.loads((ctrl / "status.json").read_text())
            check(status["state"] == "failed" and status["failed"] is not None, f"{label}: the failure is reported (state={status['state']})")
            failed = status["failed"]["id"] if status["failed"] else None
            check(failed in ("data.display_fresh", "data.audio_frames_arriving", "audio.decoded_progress", "owner.fresh_telemetry_with_session", "session.connected_as_viewer", "data.measured"),
                  f"{label}: the exact failed condition is retained: {failed}: {(status['failed'] or {}).get('detail')}")
            check(not chrome_running(stack), f"{label}: the temporary Chrome is gone after the failure")
            time.sleep(2)
            check(stack.perf()["metrics"].get("connections") == 0, f"{label}: after the failure no connection of ours remains at the Bridge (connections={stack.perf()['metrics'].get('connections')})")
            return
        code, ready = cli("wait", ctrl, "--timeout", "120")
        check(code == 0 and ready.get("state") == "ready", f"{label}: validation reached READY: {ready}")
        status = json.loads((ctrl / "status.json").read_text())
        for cid, c in sorted(status["conditions"].items()):
            print(f"      {'ok  ' if c['ok'] else 'FAIL'} {cid}: {c['detail']}")
        check(status.get("driver") == expect_driver, f"{label}: page driver is {expect_driver} (was {status.get('driver')})")
        check(status.get("role") == expect_role and status.get("role_enforced_by_bridge") == (expect_role == "viewer"),
              f"{label}: the Bridge assigned role {status.get('role')!r}, enforced by the Bridge: {status.get('role_enforced_by_bridge')}")
        check(status["conditions"].get("guard.active", {}).get("ok") is True, f"{label}: the receive-only guard is proven active in the page")
        check(status.get("legacy_fallback_used") == (expect_role == "operator") and status.get("role_policy") == viewer_role,
              f"{label}: role policy {status.get('role_policy')!r}, legacy fallback used: {status.get('legacy_fallback_used')}")
        sandbox = status["conditions"].get("browser.sandbox_intact", {})
        renderers = [p for p in (status.get("isolation") or {}).get("processes", []) if p["type"] == "renderer"]
        check(sandbox.get("ok") is True and renderers and all(p["seccomp"] == 2 for p in renderers),
              f"{label}: the real Chrome's sandbox is verified from /proc: {sandbox.get('detail')}")
        check(all(c["ok"] for c in status["conditions"].values()), f"{label}: every condition holds")
        held = stack.perf()["metrics"]
        check(held.get("client") == 1 and held.get("connections", 0) >= 2, f"{label}: while held, the Bridge sees the session (client={held.get('client')}, connections={held.get('connections')})")
        check(held.get("split_paired", 0) >= 1, f"{label}: the split pair is registered at the Bridge (split_paired={held.get('split_paired')})")
        sdr = held.get("client", 0) >= 1 and (held.get("iq", 0) >= 1 or held.get("audio", 0) >= 1) and held.get("ddc_s", 0) > 0
        check(sdr, f"{label}: the Bridge's own sdr_active definition holds with only our session attached")
        time.sleep(3)
        radio_held = {k: stack.perf()["metrics"].get(k) for k in radio_before}
        check(radio_held == radio_before, f"{label}: the radio did not change while validating: {radio_held}")
        code, final = cli("stop", ctrl, "--timeout", "60")
        check(code == 0 and final.get("state") == "done", f"{label}: stop returned done: {final}")
        process.wait(timeout=30)
        time.sleep(2.5)
        after = stack.perf()["metrics"]
        check(after.get("connections") == 0 and after.get("client") == 0, f"{label}: after stop nothing is connected (connections={after.get('connections')}, client={after.get('client')})")
        check(not chrome_running(stack), f"{label}: the temporary Chrome and its profile are gone")
        radio_after = {k: after.get(k) for k in radio_before}
        check(radio_after == radio_before, f"{label}: the radio is as it was after the session: {radio_after}")
        log = (ctrl / "validation.log").read_text() + (ctrl / "status.json").read_text()
        check(stack.login.split(":", 1)[1] not in log, f"{label}: the proxy credential is not in the validator's log or status")
        frames = [json.loads(l) for l in log.splitlines() if l.startswith("{") and '"event": "condition"' in l]
        wire = status["conditions"].get("wire.only_allowed_commands", {})
        say(f"{label}: wire audit: {wire.get('detail')}")
    finally:
        stack.stop()


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    OUT.mkdir(parents=True, exist_ok=True)
    cases = {"new": lambda: case(Path(ENV["SATURN_REH_WEBROOT"]), "new", "native"),
             "legacy": lambda: case(Path(ENV["SATURN_REH_OLD_WEBROOT"]), "legacy", "legacy"),
             "stalled": lambda: case(Path(ENV["SATURN_REH_WEBROOT"]), "stalled", "native", stall=True),
             "oldbridge": lambda: (
                 case(Path(ENV["SATURN_REH_OLD_WEBROOT"]), "oldbridge", "legacy", bridge_bin=os.environ["SATURN_REH_OLD_BRIDGE_BIN"],
                      viewer_role="legacy-fallback", expect_role="operator"),
                 case(Path(ENV["SATURN_REH_OLD_WEBROOT"]), "oldbridge-required", "legacy", bridge_bin=os.environ["SATURN_REH_OLD_BRIDGE_BIN"],
                      viewer_role="required", expect_role="operator", expect_refusal=True)),
             "nosandbox": lambda: case(Path(ENV["SATURN_REH_WEBROOT"]), "nosandbox", "native", no_sandbox=True)}
    for name in (["new", "legacy", "stalled", "nosandbox"] + (["oldbridge"] if os.environ.get("SATURN_REH_OLD_BRIDGE_BIN") else []) if which == "all" else [which]):
        try:
            cases[name]()
        except Exception as error:  # a rehearsal that cannot run is a failure, not a pass
            check(False, f"{name}: the rehearsal itself failed: {error!r}")
    failed = [m for ok, m in results if not ok]
    print(f"\n{len(results) - len(failed)} of {len(results)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
