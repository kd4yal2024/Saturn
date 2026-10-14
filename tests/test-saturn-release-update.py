#!/usr/bin/env python3
"""The managed-update broker (saturn-release-update-root.py): what it refuses before it changes anything, what it hands to the existing root
installer and activator, how it reports every outcome to the page, and that it runs under the host maintenance locks.

The installer and activator are stand-ins that keep the real exit statuses (70 previous release not restored, 71 restored but reception
unverified) and write the real record fields; the maintenance-lock tool is the real one."""
from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from restore_fixtures import Host, tree_digest

REPO = Path(__file__).resolve().parent.parent
BROKER = REPO / "update_manager/scripts/saturn-release-update-root.py"
LOCK_TOOL = REPO / "update_manager/scripts/saturn-maintenance-lock.py"
HELPERS = ("saturn-release-activate-root.sh", "saturn-remote-validate.py", "saturn-remote-receive-guard.js")

OLD = "329f3109b17428ee04e3f14031da6cf25cb04810"
NEW = "cc09c61f91030d4c5a38d6c570a20c28b5d97e03"

FAKE_INSTALLER = r'''#!/usr/bin/env bash
echo "installer $*" >>"$FAKE_CALLS"
if [ "$1" = --validate ]; then exit "${FAKE_VALIDATE_RC:-0}"; fi
[ "${FAKE_INSTALL_RC:-0}" = 0 ] || exit "$FAKE_INSTALL_RC"
mkdir -p "$FAKE_RELEASES/$(basename "$1")"
'''

FAKE_ACTIVATOR = r'''#!/usr/bin/env python3
import json, os, subprocess, sys
calls = os.environ["FAKE_CALLS"]
open(calls, "a").write("activator " + " ".join(sys.argv[1:]) + "\n")
commit = sys.argv[-1]
outcome = os.environ.get("FAKE_OUTCOME", "success")
record_path, current, releases, perf = os.environ["FAKE_RECORD"], os.environ["FAKE_CURRENT"], os.environ["FAKE_RELEASES"], os.environ["FAKE_PERF"]

# What the real activation does to the services: it restarts them, and saturn-go's own startup (restore recovery, rust-server restore.rs) takes the repository
# and radio maintenance locks with the real lock tool and EXITS when either is busy. A held radio lock therefore means no server, a failed readiness
# check and a failed rollback: the first page-driven update on the G2 failed exactly like that. The call is built the way rust-server/src/maintenance_lock.rs
# wrapped_command and restore.rs recover_restore_transactions build it: the helper by its installed path, `recover --state-root <root>`, no deadline, no job options.
def saturn_go_recovery(operation="restore:recover", command=None):
    command = command or [os.environ["FAKE_RESTORE_HELPER"], "recover", "--state-root", os.environ["FAKE_STATE_ROOT"]]
    return subprocess.run([os.environ["FAKE_LOCK_TOOL"], "--lock-dir", os.environ["FAKE_LOCK_DIR"], "run", "--operation", operation,
                           "--resources", "repository,radio", "--", *command], capture_output=True)

if os.environ.get("FAKE_ACTIVATOR_LOG"):
    # what the host looks like from inside the activation: is the release lock still held (by the inherited descriptor), and is any Python process
    # of the broker still resident (each one is memory the headless browser's headroom check counts against it)?
    held = subprocess.run([os.environ["FAKE_LOCK_TOOL"], "--lock-dir", os.environ["FAKE_LOCK_DIR"], "probe", "--operation", "x", "--resources", "release"], capture_output=True)
    listing = subprocess.run(["ps", "-eo", "comm=,args="], capture_output=True, text=True).stdout.splitlines()
    # anything that names the broker, except the small shell that hands over to --finish (whose arguments carry the broker's path): by process name, not by
    # "python", because an interpreter may run under a loader of another name
    resident = [line for line in listing if os.environ["FAKE_BROKER"] in line and line.split(None, 1)[0] != "bash"]
    open(os.environ["FAKE_ACTIVATOR_LOG"], "a").write(f"release-lock-probe-rc={held.returncode}\nbroker-python-processes={len(resident)}\n")

# Competing maintenance that starts AFTER the broker's last check (this program runs after it): the operations saturn-go starts through the lock tool, with
# the resources rust-server/src/maintenance_lock.rs gives them, and a script that locks the radio file directly (the radio backend switch does). Recorded at the start of
# the activation and again when it ends, on every path (success, rollback, failure). `FAKE_COMPETE` names the log.
if os.environ.get("FAKE_COMPETE"):
    import atexit
    def compete(tag):
        tool, lock_dir = os.environ["FAKE_LOCK_TOOL"], os.environ["FAKE_LOCK_DIR"]
        def through_the_tool(action, operation, resources):
            return subprocess.run([tool, "--lock-dir", lock_dir, action, "--operation", operation, "--resources", resources], capture_output=True).returncode
        rows = [("flash-fpga", through_the_tool("probe", "flash_fpga.sh", "fpga,radio")),
                ("repository-restore", through_the_tool("probe", "restore:settings", "repository,radio")),
                ("pihpsdr-update", through_the_tool("probe", "update-pihpsdr.py", "repository,package,radio")),
                ("led-fix", through_the_tool("probe", "fix-led-power-button.sh", "radio")),
                ("hold-radio", subprocess.run(["timeout", "2", tool, "--lock-dir", lock_dir, "hold", "--operation", "flash_fpga.sh", "--resources", "fpga,radio"],
                                              stdin=subprocess.DEVNULL, capture_output=True).returncode),
                ("direct-flock-radio", subprocess.run(["flock", "-n", lock_dir + "/radio.lock", "true"]).returncode),
                ("direct-flock-repository", subprocess.run(["flock", "-n", lock_dir + "/repository.lock", "true"]).returncode),
                ("recovery-probe", through_the_tool("probe", "restore:recover", "repository,radio")),
                ("recovery-mislabeled-command", saturn_go_recovery(command=["true"]).returncode),
                ("recovery-another-helper-call", saturn_go_recovery(command=[os.environ["FAKE_RESTORE_HELPER"], "settings", "--state-root", os.environ["FAKE_STATE_ROOT"]]).returncode),
                ("recovery", saturn_go_recovery().returncode)]
        with open(os.environ["FAKE_COMPETE"], "a") as log:
            for name, rc in rows:
                log.write(f"{tag} {name} rc={rc}\n")
    compete("during")
    atexit.register(lambda: compete("end"))

if outcome != "refused":
    started = saturn_go_recovery()
    open(os.environ["FAKE_SERVER_LOG"], "a").write(f"server-start rc={started.returncode}\n")
    if started.returncode != 0:
        json.dump({"status": "rollback_failed", "phase": "rollback-failed", "target_commit": commit, "validation": {},
                   "activation_failure": {"phase": "readiness", "exit_status": 1, "command": "false"},
                   "rollback": {"status": "failed", "release_restored": False, "receive_verified": "not_attempted"}}, open(record_path, "w"))
        sys.exit(70)

def record(status, phase, **more):
    d = {"status": status, "phase": phase, "target_commit": commit, "validation": {}}
    d.update(more)
    json.dump(d, open(record_path, "w"))

def set_perf(frequency):
    if not os.path.exists(perf):
        return
    d = json.load(open(perf))
    d["metrics"]["frequency_hz"] = frequency
    json.dump(d, open(perf, "w"))

if outcome in ("success", "success-no-record"):
    if outcome == "success":
        record("committed", "commit", validation={"target": {"final_state": "done", "driver": "native", "role": "viewer", "viewer_role_policy": "required",
                                                        "legacy_fallback_used": False, "failed": None}})
    tmp = current + ".new"
    os.symlink(os.path.join(releases, commit), tmp)
    os.replace(tmp, current)
    set_perf(int(os.environ.get("FAKE_FREQUENCY_AFTER", "3902000")))
    sys.exit(0)
if outcome == "rolled-back":
    record("rolled_back", "rollback-complete", rollback={"status": "succeeded", "release_restored": True, "receive_verified": True},
           validation={"target": {"final_state": "failed", "failed": {"id": "audio.decoded_progress", "detail": "decoded audio frames played advanced by 0"}}},
           activation_failure={"phase": "receive-validation", "exit_status": 1})
    sys.exit(1)
if outcome == "rollback-failed":
    record("rollback_failed", "rollback-failed", rollback={"status": "failed", "release_restored": False})
    sys.exit(70)
if outcome == "rolled-back-unverified-exit-1":
    record("rolled_back", "rollback-complete", rollback={"status": "succeeded", "release_restored": True, "receive_verified": False})
    sys.exit(1)
if outcome == "unverified":
    record("rolled_back", "rollback-complete", rollback={"status": "succeeded", "release_restored": True, "receive_verified": False})
    sys.exit(71)
sys.exit(1)      # refused before the transaction began: the record is untouched
'''


# a lock tool that claims the capability and answers `pending-restores` with whatever a test needs
STUB_TOOL = """#!/usr/bin/env python3
import sys
if sys.argv[1:] == ["capabilities"]:
    print('{{"schema":1,"recovery_check":1}}')
elif sys.argv[1] == "pending-restores":
    print({pending!r})
    sys.exit({rc})
"""


def helper_text(name: str, contract: int) -> str:
    mark = f"saturn-helper-contract: {contract}"
    return f"// {mark}\n" if name.endswith(".js") else f"#!/usr/bin/env bash\n# {mark}\n"


class Rig(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="saturn-release-update-")
        self.addCleanup(self.tmp.cleanup)
        root = self.root = Path(self.tmp.name)
        self.tools, self.staging, self.releases = root / "tools", root / "staging", root / "releases"
        for directory in (self.tools, self.staging, self.releases, root / "locks", root / "logs", root / "run"):
            directory.mkdir()
        self.current = root / "current"
        self.status_file, self.perf, self.record, self.calls = root / "status.json", root / "run/perf.json", root / "record.json", root / "calls.log"
        self.calls.write_text("")
        # the host's restore state, and the restore helper as saturn-go starts it, here a stand-in that only records that it was started (the real one writes)
        self.host = Host(root / "host")
        self.state_root = self.host.state
        self.helper_log = root / "helper-started.log"
        self.restore_helper = root / "bin/saturn-restore-transaction.py"
        self.restore_helper.parent.mkdir()
        self.restore_helper.write_text(f'#!/bin/sh\necho "$@" >>"{self.helper_log}"\nprintf \'{{"status":"ok","recovered":[]}}\\n\'\n')
        self.restore_helper.chmod(0o755)
        (self.tools / "saturn-release-install-root.sh").write_text(FAKE_INSTALLER)
        (self.tools / "saturn-release-activate-root.sh").write_text(FAKE_ACTIVATOR)
        for name in HELPERS[1:]:
            (self.tools / name).write_text(helper_text(name, 1))
        (self.tools / "saturn-release-activate-root.sh").write_text("#!/usr/bin/env python3\n# saturn-helper-contract: 1\n" + FAKE_ACTIVATOR.split("\n", 1)[1])
        shutil.copy(LOCK_TOOL, self.tools / "saturn-maintenance-lock.py")
        for name in ("saturn-release-install-root.sh", "saturn-release-activate-root.sh", "saturn-remote-validate.py", "saturn-remote-receive-guard.js", "saturn-maintenance-lock.py"):
            (self.tools / name).chmod(0o755)
        for resource in ("release", "radio", "repository", "package", "fpga"):
            lock = root / "locks" / f"{resource}.lock"
            lock.write_text("")
            lock.chmod(0o660)
        (self.releases / OLD).mkdir()
        os.symlink(self.releases / OLD, self.current)
        self.perf.write_text(json.dumps({"backend": "xdma", "rf_safe": True, "metrics": {"frequency_hz": 3902000, "sample_rate_hz": 384000, "ddc": 6, "adc": "ADC1",
                                                                                     "processing_mode": "drain-only", "tx_keyed": False, "rx_counter_v31": {"status": "unarmed"}}}))
        self.set_config(enabled="1")
        self.fake_systemd_run = root / "systemd-run"
        self.fake_systemd_run.write_text(f'#!/usr/bin/env bash\necho "systemd-run $*" >>"{self.calls}"\nexit "${{FAKE_SYSTEMD_RUN_RC:-0}}"\n')
        self.fake_systemd_run.chmod(0o755)

    def set_config(self, enabled="1", current_link=None):
        (self.root / "install.conf").write_text(f'RUN_USER="pi"\nSTAGING_ROOT="{self.staging}"\nRELEASES_ROOT="{self.releases}"\n')
        (self.root / "activate.conf").write_text(
            f'ACTIVATION_ENABLED="{enabled}"\nRELEASES_ROOT="{self.releases}"\nCURRENT_LINK="{current_link or self.current}"\nTRANSACTION_FILE="{self.record}"\n')

    def bundle(self, commit=NEW, manifest_commit=None, dirty=False, helper_contract=1, name=None, requested_ref="goldmarsh/headless-validation"):
        path = self.staging / (name or commit)
        (path / "scripts").mkdir(parents=True)
        (path / "release-manifest.json").write_text(json.dumps({"source": {"commit": manifest_commit or commit, "dirty": dirty, "repository": "https://example.invalid/Saturn.git",
                                                                          "requested_ref": requested_ref}}))
        for helper in HELPERS:
            (path / "scripts" / helper).write_text(helper_text(helper, helper_contract))
        return path

    def env(self, **extra):
        env = dict(os.environ)
        env.update(SATURN_RELEASE_UPDATE_TOOLS_DIR=str(self.tools), SATURN_RELEASE_UPDATE_INSTALL_CONFIG=str(self.root / "install.conf"),
                   SATURN_RELEASE_UPDATE_ACTIVATE_CONFIG=str(self.root / "activate.conf"), SATURN_RELEASE_UPDATE_STATUS_FILE=str(self.status_file),
                   SATURN_RELEASE_UPDATE_LOG_DIR=str(self.root / "logs"), SATURN_RELEASE_UPDATE_PERF_JSON=str(self.perf),
                   SATURN_RELEASE_UPDATE_LOCK_DIR=str(self.root / "locks"), SATURN_RELEASE_UPDATE_SYSTEMD_RUN=str(self.fake_systemd_run),
                   SATURN_RELEASE_UPDATE_LOCK_WAIT="1", SATURN_RELEASE_UPDATE_SELF=str(BROKER), SATURN_RELEASE_UPDATE_STATE_ROOT=str(self.state_root),
                   FAKE_RESTORE_HELPER=str(self.restore_helper), FAKE_STATE_ROOT=str(self.state_root),
                   FAKE_CALLS=str(self.calls), FAKE_LOCK_TOOL=str(LOCK_TOOL), FAKE_LOCK_DIR=str(self.root / "locks"), FAKE_SERVER_LOG=str(self.root / "server.log"), FAKE_ACTIVATOR_LOG=str(self.root / "activator.log"), FAKE_BROKER=str(BROKER), FAKE_RELEASES=str(self.releases), FAKE_RECORD=str(self.record), FAKE_CURRENT=str(self.current), FAKE_PERF=str(self.perf))
        env.update(extra)
        return env

    def broker(self, *args, **env):
        return subprocess.run([sys.executable, str(BROKER), *args], capture_output=True, text=True, env=self.env(**env), timeout=120)

    def status(self):
        return json.loads(self.status_file.read_text())

    def calls_text(self):
        return self.calls.read_text()

    def active(self):
        return os.path.basename(os.path.realpath(self.current))


class PreflightTests(Rig):
    def test_it_passes_when_managed_updates_are_on_and_a_release_is_active(self):
        result = self.broker("--preflight")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"active release {OLD}", result.stdout)

    def test_it_refuses_when_managed_updates_are_off(self):
        self.set_config(enabled="0")
        result = self.broker("--preflight")
        self.assertEqual(result.returncode, 2)
        self.assertIn("managed updates are disabled", result.stderr)

    def test_it_refuses_a_legacy_layout_unless_the_migration_is_explicit(self):
        self.current.unlink()
        refused = self.broker("--preflight")
        self.assertEqual(refused.returncode, 2)
        self.assertIn("legacy layout", refused.stderr)
        self.assertIn("--migrate-to-releases", refused.stderr)
        allowed = self.broker("--preflight", "--allow-legacy-migration")
        self.assertEqual(allowed.returncode, 0, allowed.stderr)
        self.assertIn("layout legacy", allowed.stdout)

    def test_a_pointer_that_is_not_an_installed_release_is_a_legacy_layout(self):
        self.current.unlink()
        os.symlink(self.root, self.current)
        self.assertEqual(self.broker("--preflight").returncode, 2)

    def test_it_refuses_when_a_privileged_helper_is_missing(self):
        (self.tools / "saturn-remote-validate.py").unlink()
        result = self.broker("--preflight")
        self.assertEqual(result.returncode, 2)
        self.assertIn("saturn-remote-validate.py is not installed", result.stderr)

    def test_it_takes_no_bundle(self):
        self.assertNotEqual(self.broker("--preflight", str(self.bundle())).returncode, 0)


class DispatchTests(Rig):
    def refused(self, bundle, text, phase="refused", pointer_exists=True, **kwargs):
        result = self.broker("--dispatch", str(bundle), **kwargs)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn(text, result.stderr)
        self.assertEqual(self.status()["status"], "error")
        self.assertEqual(self.status()["phase"], phase)
        self.assertNotIn("systemd-run", self.calls_text(), "a unit was started although the bundle was refused")
        if pointer_exists:
            self.assertEqual(self.active(), OLD)
        self.assertFalse((self.releases / NEW).exists(), "something was installed although the bundle was refused")

    def test_a_good_bundle_is_handed_to_a_detached_unit(self):
        bundle = self.bundle()
        result = self.broker("--dispatch", str(bundle))
        self.assertEqual(result.returncode, 0, result.stderr)
        line = next(l for l in self.calls_text().splitlines() if l.startswith("systemd-run"))
        self.assertRegex(line, rf"--unit saturn-release-update-[0-9]{{14}} --collect --no-block --property=RuntimeMaxSec=7200 {BROKER} --run {bundle}$")
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("dispatched", "dispatch"))
        self.assertRegex(status["unit_name"], r"^saturn-release-update-[0-9]{14}$")
        self.assertEqual(status["release"]["commit"], NEW)

    def test_the_migration_flag_is_passed_to_the_unit_only_when_given(self):
        self.current.unlink()
        self.assertEqual(self.broker("--dispatch", "--allow-legacy-migration", str(self.bundle())).returncode, 0)
        self.assertIn("--run --allow-legacy-migration", self.calls_text())

    def test_it_refuses_when_managed_updates_are_off(self):
        self.set_config(enabled="0")
        self.refused(self.bundle(), "managed updates are disabled")

    def test_it_refuses_a_legacy_layout_without_the_explicit_migration(self):
        self.current.unlink()
        self.refused(self.bundle(), "legacy layout", phase="migration_required", pointer_exists=False)

    def test_it_refuses_a_bundle_outside_the_staging_directory(self):
        outside = self.root / "elsewhere"
        outside.mkdir()
        shutil.copytree(self.bundle(), outside / NEW)
        self.refused(outside / NEW, "not directly inside the release staging directory")

    def test_it_refuses_a_path_that_climbs_out_of_the_staging_directory(self):
        self.bundle()
        self.refused(self.staging / ".." / "staging" / ".." / "elsewhere", "not directly inside")

    def test_it_refuses_a_symbolic_link(self):
        real = self.bundle()
        link = self.staging / ("1" * 40)
        os.symlink(real, link)
        self.refused(link, "symbolic link")

    def test_it_refuses_a_directory_not_named_by_a_full_commit(self):
        self.refused(self.bundle(name="cc09c61"), "not named by a full lowercase commit")

    def test_it_refuses_a_manifest_of_another_commit(self):
        self.refused(self.bundle(manifest_commit=OLD), "does not describe a clean tree")

    def test_it_refuses_a_dirty_tree(self):
        self.refused(self.bundle(dirty=True), "does not describe a clean tree")

    def test_it_refuses_a_bundle_without_a_manifest(self):
        bundle = self.bundle()
        (bundle / "release-manifest.json").unlink()
        self.refused(bundle, "no readable release-manifest.json")

    def test_a_release_that_needs_newer_helpers_is_refused_before_anything_changes(self):
        for helper in HELPERS:
            with self.subTest(helper=helper):
                shutil.rmtree(self.staging / NEW, ignore_errors=True)
                bundle = self.bundle()
                (bundle / "scripts" / helper).write_text(helper_text(helper, 2))
                self.refused(bundle, f"installer update required: this release needs {helper} helper contract 2, the installed one is 1", phase="installer_update_required")

    def test_a_release_with_the_same_or_older_helper_contract_is_accepted(self):
        (self.tools / "saturn-remote-validate.py").write_text(helper_text("saturn-remote-validate.py", 3))
        self.assertEqual(self.broker("--dispatch", str(self.bundle(helper_contract=1))).returncode, 0, "the same contract was refused")
        shutil.rmtree(self.staging / NEW)
        bundle = self.bundle(helper_contract=1)
        (bundle / "scripts" / "saturn-remote-validate.py").write_text(helper_text("saturn-remote-validate.py", 2))
        self.assertEqual(self.broker("--dispatch", str(bundle)).returncode, 0, "an older contract than the installed one was refused")

    def test_a_unit_that_cannot_be_started_is_reported(self):
        result = self.broker("--dispatch", str(self.bundle()), FAKE_SYSTEMD_RUN_RC="1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual((self.status()["status"], self.status()["phase"]), ("error", "dispatch"))

    def test_extra_arguments_are_not_accepted(self):
        bundle = self.bundle()
        self.assertNotEqual(self.broker("--dispatch", str(bundle), "--property=ExecStartPre=/bin/sh").returncode, 0)
        self.assertNotEqual(self.broker("--dispatch", str(bundle), str(bundle)).returncode, 0)
        self.assertNotIn("systemd-run", self.calls_text())


class RunTests(Rig):
    def run_update(self, bundle=None, **env):
        return self.broker("--run", str(bundle or self.bundle()), **env)

    def test_a_good_update_validates_installs_activates_and_reports_success(self):
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        calls = self.calls_text().splitlines()
        self.assertEqual([c.split()[0] + " " + c.split()[1] for c in calls], ["installer --validate", f"installer {self.staging / NEW}", f"activator {NEW}"][:3])
        self.assertEqual(self.active(), NEW)
        status = self.status()
        self.assertEqual((status["status"], status["phase"], status["exit_code"]), ("success", "done", 0))
        self.assertIsNotNone(status["finished_at"])
        release = status["release"]
        self.assertEqual((release["commit"], release["previous_commit"], release["active_commit"], release["layout_before"]), (NEW, OLD, NEW, "release"))
        self.assertEqual(release["requested_ref"], "goldmarsh/headless-validation")
        self.assertEqual(release["result"]["target"]["role"], "viewer")
        self.assertTrue(Path(status["log"]).is_file())
        self.assertIn(f"saturn-release-activate-root.sh {NEW}", Path(status["log"]).read_text())

    def test_the_status_says_whether_the_radio_settings_survived(self):
        self.assertEqual(self.run_update(FAKE_FREQUENCY_AFTER="3902000").returncode, 0)
        release = self.status()["release"]
        self.assertTrue(release["settings_preserved"])
        self.assertEqual((release["radio_before"]["frequency_hz"], release["radio_after"]["frequency_hz"]), (3902000, 3902000))
        self.assertEqual(release["rxc1_after"], "unarmed")

    def test_settings_that_did_not_survive_are_reported_not_hidden(self):
        self.assertEqual(self.run_update(FAKE_FREQUENCY_AFTER="14200000").returncode, 0)
        release = self.status()["release"]
        self.assertIs(release["settings_preserved"], False)
        self.assertEqual((release["radio_before"]["frequency_hz"], release["radio_after"]["frequency_hz"]), (3902000, 14200000))

    def test_an_unreadable_radio_state_is_unknown_not_preserved(self):
        self.perf.unlink()
        self.assertEqual(self.run_update().returncode, 0)
        self.assertIsNone(self.status()["release"]["settings_preserved"])

    def test_a_release_that_is_already_active_changes_nothing(self):
        bundle = self.bundle(commit=OLD)
        result = self.run_update(bundle)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.calls_text(), "")
        self.assertIn("already the active release", self.status()["message"])

    def test_an_installed_release_is_not_installed_again(self):
        (self.releases / NEW).mkdir()
        self.assertEqual(self.run_update().returncode, 0)
        self.assertEqual([c.split()[0] for c in self.calls_text().splitlines()], ["activator"])

    def test_a_failed_validation_is_a_rollback_and_says_why(self):
        result = self.run_update(FAKE_OUTCOME="rolled-back")
        self.assertEqual(result.returncode, 1)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "rolled_back"))
        self.assertIn("audio.decoded_progress", status["message"])
        self.assertIn("previous release was restored and its reception was verified", status["message"])
        self.assertEqual(status["release"]["result"]["rollback"]["receive_verified"], True)
        self.assertEqual(self.active(), OLD)

    def test_a_previous_release_that_could_not_be_restored_is_loud(self):
        result = self.run_update(FAKE_OUTCOME="rollback-failed")
        self.assertEqual(result.returncode, 70)
        status = self.status()
        self.assertEqual((status["status"], status["phase"], status["exit_code"]), ("error", "rollback_failed", 70))
        self.assertIn("could NOT be restored", status["message"])

    def test_a_restored_release_whose_reception_is_unverified_is_not_called_a_success(self):
        result = self.run_update(FAKE_OUTCOME="unverified")
        self.assertEqual(result.returncode, 71)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "rollback_unverified"))
        self.assertIn("could not be verified", status["message"])

    def test_an_activator_that_refuses_before_its_transaction_is_a_failure(self):
        result = self.run_update(FAKE_OUTCOME="refused")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.status()["phase"], "failed")
        self.assertEqual(self.active(), OLD)

    def test_a_success_status_needs_the_committed_record(self):
        result = self.run_update(FAKE_OUTCOME="success-no-record")
        self.assertNotEqual(result.returncode, 0)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "unconfirmed"))

    def test_a_record_of_another_release_is_not_mistaken_for_ours(self):
        self.record.write_text(json.dumps({"status": "committed", "phase": "commit", "target_commit": OLD}))
        result = self.run_update(FAKE_OUTCOME="refused")
        self.assertEqual(self.status()["phase"], "failed")

    def test_a_stale_committed_record_of_another_release_never_makes_a_success(self):
        self.record.write_text(json.dumps({"status": "committed", "phase": "commit", "target_commit": OLD}))
        result = self.run_update(FAKE_OUTCOME="success-no-record")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.status()["phase"], "unconfirmed")

    def test_a_stale_rolled_back_record_of_another_release_is_not_this_runs_rollback(self):
        self.record.write_text(json.dumps({"status": "rolled_back", "phase": "rollback-complete", "target_commit": OLD,
                                           "rollback": {"status": "succeeded", "release_restored": True, "receive_verified": True}}))
        self.run_update(FAKE_OUTCOME="refused")
        status = self.status()
        self.assertEqual(status["phase"], "failed", "a rollback verified for an earlier attempt was reported for this one")
        self.assertNotIn("reception was verified", status["message"])

    def test_a_rollback_is_called_verified_only_when_the_record_says_so(self):
        result = self.run_update(FAKE_OUTCOME="rolled-back-unverified-exit-1")
        self.assertEqual(result.returncode, 1)
        status = self.status()
        self.assertEqual(status["phase"], "failed")
        self.assertNotIn("reception was verified", status["message"])

    def test_the_installer_refusing_the_bundle_stops_everything(self):
        result = self.run_update(FAKE_VALIDATE_RC="1")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.status()["phase"], "refused")
        self.assertNotIn("activator", self.calls_text())
        self.assertEqual(self.active(), OLD)

    def test_an_install_that_fails_stops_before_activation(self):
        result = self.run_update(FAKE_INSTALL_RC="1")
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("activator", self.calls_text())
        self.assertIn("could not be installed", self.status()["message"])

    def test_the_run_refuses_what_dispatch_refuses(self):
        self.set_config(enabled="0")
        self.assertEqual(self.run_update().returncode, 2)
        self.assertEqual(self.calls_text(), "")
        self.set_config(enabled="1")
        shutil.rmtree(self.staging / NEW, ignore_errors=True)
        bundle = self.bundle(helper_contract=9)
        self.assertEqual(self.run_update(bundle).returncode, 2)
        self.assertEqual(self.status()["phase"], "installer_update_required")
        self.assertEqual(self.calls_text(), "")

    def test_a_busy_release_lock_means_nothing_is_changed(self):
        with open(self.root / "locks" / "release.lock", "r+") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            result = self.run_update()
        self.assertEqual(result.returncode, 2)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "busy"))
        self.assertEqual(self.calls_text(), "")
        self.assertEqual(self.active(), OLD)

    def test_a_busy_radio_lock_means_nothing_is_changed(self):
        with open(self.root / "locks" / "radio.lock", "r+") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            result = self.run_update()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.calls_text(), "")

    def test_a_busy_repository_lock_means_nothing_is_changed(self):
        with open(self.root / "locks" / "repository.lock", "r+") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            result = self.run_update()
        self.assertEqual(result.returncode, 2)
        self.assertEqual((self.status()["status"], self.status()["phase"]), ("error", "busy"))
        self.assertEqual(self.calls_text(), "")
        self.assertEqual(self.active(), OLD)

    def test_a_lock_that_is_released_in_time_is_waited_for(self):
        import threading, time
        held = open(self.root / "locks" / "release.lock", "r+")
        fcntl.flock(held, fcntl.LOCK_EX)
        threading.Timer(0.6, lambda: (fcntl.flock(held, fcntl.LOCK_UN), held.close())).start()
        result = self.run_update(SATURN_RELEASE_UPDATE_LOCK_WAIT="20")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(self.active(), NEW)

    def test_the_release_radio_and_repository_locks_are_all_held_while_it_runs(self):
        probe = self.root / "probe.txt"
        (self.tools / "saturn-release-install-root.sh").write_text(
            f'#!/usr/bin/env bash\necho "installer $*" >>"$FAKE_CALLS"\n[ "$1" = --validate ] && {{ '
            f'"{LOCK_TOOL}" --lock-dir "{self.root}/locks" probe --operation x --resources release >/dev/null 2>&1; echo "release rc=$?" >>"{probe}"; '
            f'"{LOCK_TOOL}" --lock-dir "{self.root}/locks" probe --operation x --resources radio >/dev/null 2>&1; echo "radio rc=$?" >>"{probe}"; '
            f'"{LOCK_TOOL}" --lock-dir "{self.root}/locks" probe --operation x --resources repository >/dev/null 2>&1; echo "repository rc=$?" >>"{probe}"; exit 0; }}\n'
            'mkdir -p "$FAKE_RELEASES/$(basename "$1")"\n')
        self.assertEqual(self.run_update().returncode, 0)
        self.assertEqual(probe.read_text().split("\n")[:3], ["release rc=75", "radio rc=75", "repository rc=75"],
                         "nothing else may take any of the three while the installer runs: only saturn-go's own restore recovery is answered, and only that exact call")

    def competition(self, **env):
        log = self.root / "compete.log"
        result = self.run_update(FAKE_COMPETE=str(log), **env)
        rows = {}
        for line in log.read_text().splitlines():
            tag, name, rc = line.split()
            rows[(tag, name)] = int(rc.split("=")[1])
        return result, rows

    COMPETITORS = ("flash-fpga", "repository-restore", "pihpsdr-update", "led-fix", "hold-radio")

    def assertNoOneGetsIn(self, rows):
        for tag in ("during", "end"):
            for name in self.COMPETITORS:
                self.assertEqual(rows[(tag, name)], 75, f"{name} was let in {tag} the activation: {rows}")
            for name in ("direct-flock-radio", "direct-flock-repository"):
                self.assertEqual(rows[(tag, name)], 1, f"a script that locks the file directly got in {tag} the activation: {rows}")
            self.assertEqual(rows[(tag, "recovery")], 0, f"the server's own restore recovery must be answered {tag} the activation (its startup needs it): {rows}")
            for name in ("recovery-probe", "recovery-mislabeled-command", "recovery-another-helper-call"):
                self.assertEqual(rows[(tag, name)], 75, f"{name} was let in {tag} the activation: {rows}")

    def test_competing_maintenance_started_after_the_last_check_cannot_proceed(self):
        """The reviewer's finding: the broker checked radio and repository and let go of them, so FPGA flashing, a repository restore or the radio backend
        switch could take the radio lock the moment after the check, while saturn-go (restarted, and reachable) and the activator went on."""
        result, rows = self.competition()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNoOneGetsIn(rows)
        self.assertFalse(self.helper_log.exists(), "the restore helper was started under the update's shared locks")

    def test_competing_maintenance_cannot_proceed_while_a_failed_update_is_rolled_back(self):
        result, rows = self.competition(FAKE_OUTCOME="rolled-back")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.status()["phase"], "rolled_back")
        self.assertNoOneGetsIn(rows)

    def test_competing_maintenance_cannot_proceed_while_a_rollback_is_unverified(self):
        result, rows = self.competition(FAKE_OUTCOME="unverified")
        self.assertEqual(result.returncode, 71, result.stdout + result.stderr)
        self.assertNoOneGetsIn(rows)

    def test_competing_maintenance_cannot_proceed_while_a_rollback_fails(self):
        result, rows = self.competition(FAKE_OUTCOME="rollback-failed")
        self.assertEqual(result.returncode, 70, result.stdout + result.stderr)
        self.assertNoOneGetsIn(rows)

    def test_competing_maintenance_is_admitted_again_when_the_run_has_ended(self):
        result, rows = self.competition()
        self.assertEqual(result.returncode, 0)
        for name, resources in (("flash_fpga.sh", "fpga,radio"), ("restore:settings", "repository,radio"), ("update-pihpsdr.py", "repository,package,radio"), ("x", "release")):
            probe = subprocess.run([str(LOCK_TOOL), "--lock-dir", str(self.root / "locks"), "probe", "--operation", name, "--resources", resources], capture_output=True)
            self.assertEqual(probe.returncode, 0, f"{name} still cannot run after the update has ended")
        self.assertEqual(subprocess.run(["flock", "-n", str(self.root / "locks" / "radio.lock"), "true"]).returncode, 0)

    def test_the_locks_are_held_from_the_start_not_only_once_the_activator_runs(self):
        """Competitors that come in while the release is being installed (minutes on a slow card) are excluded too."""
        probe = self.root / "during-install.txt"
        (self.tools / "saturn-release-install-root.sh").write_text(
            f'#!/usr/bin/env bash\necho "installer $*" >>"$FAKE_CALLS"\n[ "$1" = --validate ] && exit 0\n'
            f'"{LOCK_TOOL}" --lock-dir "{self.root}/locks" probe --operation flash_fpga.sh --resources fpga,radio >/dev/null 2>&1; echo "rc=$?" >>"{probe}"\n'
            'mkdir -p "$FAKE_RELEASES/$(basename "$1")"\n')
        self.assertEqual(self.run_update().returncode, 0)
        self.assertEqual(probe.read_text().split(), ["rc=75"])

    def test_the_server_can_start_while_the_activation_runs(self):
        """The regression of the first page-driven update: the services restart inside the activation, and saturn-go's startup takes radio+repository."""
        self.assertEqual(self.run_update().returncode, 0)
        self.assertEqual((self.root / "server.log").read_text().split(), ["server-start", "rc=0"][:0] + ["server-start", "rc=0"])
        self.assertEqual(self.status()["status"], "success")
        self.assertEqual(self.active(), NEW)

    def test_a_server_that_could_not_start_is_reported_as_a_failed_rollback_not_as_success(self):
        # a stand-in for the failure itself: the server's startup takes radio and repository, finds them held and is not answered (here: another operation name),
        # as happened on the first page-driven update, and exits; the activation must then be a failed rollback, never a success
        activator = (self.tools / "saturn-release-activate-root.sh").read_text()
        self.assertEqual(activator.count("started = saturn_go_recovery()"), 1)
        (self.tools / "saturn-release-activate-root.sh").write_text(activator.replace(
            "started = saturn_go_recovery()", 'started = saturn_go_recovery(operation="restore:something-else")'))
        result = self.run_update()
        self.assertEqual(result.returncode, 70, result.stdout + result.stderr)
        self.assertEqual(self.status()["phase"], "rollback_failed")
        self.assertEqual((self.root / "server.log").read_text().split(), ["server-start", "rc=75"])

    def test_a_radio_lock_that_is_released_a_moment_after_the_release_lock_is_waited_for(self):
        # the page's own script holds release, repository, package and radio and lets go in that order as it exits
        import threading
        held = open(self.root / "locks" / "radio.lock", "r+")
        fcntl.flock(held, fcntl.LOCK_EX)
        threading.Timer(0.6, lambda: (fcntl.flock(held, fcntl.LOCK_UN), held.close())).start()
        result = self.run_update(SATURN_RELEASE_UPDATE_LOCK_WAIT="20")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(self.active(), NEW)

    def test_a_lock_tool_that_cannot_answer_the_servers_recovery_is_refused_before_anything_changes(self):
        """The broker holds radio and repository for the whole run and relies on the lock tool to answer the server's own recovery with a read-only check. With
        an older tool the server could not start (the failure of the first page-driven update), so it refuses, naming the installer, before it changes anything."""
        (self.tools / "saturn-maintenance-lock.py").write_text("#!/usr/bin/env python3\nimport sys\nsys.exit(2)\n")
        result = self.run_update()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "installer_update_required"))
        self.assertIn("installer", status["message"])
        self.assertEqual(self.calls_text(), "")
        self.assertEqual(self.active(), OLD)
        self.assertEqual(self.broker("--preflight").returncode, 2, "the page must hear it before it builds for hours")

    def test_a_lock_tool_that_does_not_claim_the_capability_properly_is_refused(self):
        for output in ("", "not json", "{}", '{"recovery_check": 0}', '{"recovery_check": false}', '{"recovery_check": true}', '{"recovery_check": "1"}', "[1]", "null",
                       '{"shared_recovery": 1}',                      # the earlier tool: it ran whatever carried the recovery's name, writing, under the shared locks
                       '{"recovery_check": 1, "shared_recovery": 1}'):
            (self.tools / "saturn-maintenance-lock.py").write_text(f"#!/usr/bin/env python3\nprint({output!r})\n")
            result = self.broker("--preflight")
            self.assertEqual(result.returncode, 2, f"a tool answering {output!r} was accepted: {result.stdout}{result.stderr}")
            self.assertIn("installer update required", result.stderr)
        (self.tools / "saturn-maintenance-lock.py").write_text(STUB_TOOL.format(pending='{"status":"ok","pending":[]}', rc=0))
        self.assertEqual(self.broker("--preflight").returncode, 0, "a tool that claims the capability was refused")

    def test_a_missing_lock_tool_is_refused_with_the_installer_message(self):
        (self.tools / "saturn-maintenance-lock.py").unlink()
        result = self.broker("--preflight")
        self.assertEqual(result.returncode, 2)
        self.assertIn("saturn-maintenance-lock.py is not installed", result.stderr)

    def test_a_failed_lock_check_refuses_instead_of_going_ahead(self):
        lock = self.root / "locks" / "radio.lock"
        lock.unlink()
        lock.mkdir()          # cannot be opened as a lock file
        result = self.run_update()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "lock"))
        self.assertIn("cannot open the maintenance lock file", status["message"])
        self.assertEqual(self.calls_text(), "")
        self.assertEqual(self.active(), OLD)

    def test_no_python_process_of_the_broker_stays_resident_while_the_activator_runs(self):
        self.assertEqual(self.run_update().returncode, 0)
        lines = (self.root / "activator.log").read_text().split()
        self.assertIn("broker-python-processes=0", lines, "a resident Python process is memory the browser's headroom check counts against it")

    def test_the_release_lock_stays_held_through_the_hand_over_to_the_activator(self):
        self.assertEqual(self.run_update().returncode, 0)
        self.assertIn("release-lock-probe-rc=75", (self.root / "activator.log").read_text().split())

    def test_nothing_is_left_behind_when_the_run_ends(self):
        self.assertEqual(self.run_update().returncode, 0)
        probe = subprocess.run([str(LOCK_TOOL), "--lock-dir", str(self.root / "locks"), "probe", "--operation", "x", "--resources", "release,radio,repository"], capture_output=True)
        self.assertEqual(probe.returncode, 0, "the release lock is still held after the run")
        self.assertEqual(list((self.root / "logs").glob("*.context.json")), [], "the hand-over file was not removed")
        self.assertEqual(len(list((self.root / "logs").glob("*.log"))), 1)

    def test_finish_with_a_missing_context_says_so_and_changes_nothing(self):
        result = self.broker("--finish", str(self.root / "no-such-context.json"), "--activator-rc", "0")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unreadable", result.stderr)
        self.assertFalse(self.status_file.exists())

    def test_finish_needs_the_activator_status_and_nothing_else(self):
        bundle = str(self.bundle())
        for arguments, message in (((("--finish", str(self.root / "x"))), "takes a context file and --activator-rc"),
                                   (("--finish", str(self.root / "x"), "--activator-rc", "0", bundle), "takes a context file and --activator-rc"),
                                   (("--run", "--activator-rc", "0", bundle), "--activator-rc belongs to --finish")):
            result = self.broker(*arguments)
            self.assertEqual(result.returncode, 2, arguments)
            self.assertIn(message, result.stderr, arguments)

    def test_every_step_is_in_the_log(self):
        self.run_update(FAKE_OUTCOME="rolled-back")
        log = Path(self.status()["log"]).read_text()
        for expected in ("saturn-release-install-root.sh --validate", f"saturn-release-install-root.sh {self.staging / NEW}", f"saturn-release-activate-root.sh {NEW}", "[exit 1]"):
            self.assertIn(expected, log)

    def test_one_mode_at_a_time(self):
        self.assertNotEqual(self.broker("--preflight", "--run").returncode, 0)
        self.assertNotEqual(self.broker().returncode, 0)


class RestoreTransactionTests(Rig):
    """A restore transaction that is still pending is completed by saturn-go's startup recovery, which WRITES, under the exclusive locks and outside an update.
    The reviewer's point: allowing that recovery under the update's shared locks removed the protection its writes had. So an update never starts with one pending,
    and the recovery that saturn-go's restarted server meets during the update (and the rollback) is a check that finds nothing to do."""

    def run_update(self, bundle=None, **env):
        if bundle is None:
            if not hasattr(self, "the_bundle"):
                self.the_bundle = self.bundle()
            bundle = self.the_bundle
        return self.broker("--run", str(bundle), **env)

    def assertNothingChanged(self, digest):
        self.assertEqual(self.calls_text(), "", "the installer or activator was called although a restore is pending")
        self.assertEqual(self.active(), OLD)
        self.assertFalse((self.releases / NEW).exists())
        self.assertEqual(tree_digest(self.host.root), digest, "the pending restore, or anything else under the state root, was touched")
        self.assertFalse(self.helper_log.exists(), "the restore helper was started")

    def test_a_pending_settings_restore_stops_the_run_and_is_left_exactly_as_it_was(self):
        ids = self.host.pending_settings()
        digest = tree_digest(self.host.root)
        result = self.run_update()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "restore_transaction_pending"))
        self.assertIn(ids[0], status["message"])
        self.assertIn("nothing was changed", status["message"])
        self.assertIn("restart the saturn-go service", status["message"])
        self.assertNothingChanged(digest)

    def test_a_pending_repository_restore_stops_the_run_too(self):
        ids = self.host.pending_source()
        digest = tree_digest(self.host.root)
        self.assertEqual(self.run_update().returncode, 2)
        self.assertEqual(self.status()["phase"], "restore_transaction_pending")
        self.assertIn(ids[0], self.status()["message"])
        self.assertNothingChanged(digest)

    def test_the_page_hears_it_at_preflight_and_at_dispatch_before_a_unit_is_started(self):
        self.host.pending_settings()
        digest = tree_digest(self.host.root)
        preflight = self.broker("--preflight")
        self.assertEqual(preflight.returncode, 2)
        self.assertIn("restore_transaction_pending", preflight.stderr)
        dispatch = self.broker("--dispatch", str(self.bundle()))
        self.assertEqual(dispatch.returncode, 2, dispatch.stderr)
        self.assertEqual(self.status()["phase"], "restore_transaction_pending")
        self.assertNotIn("systemd-run", self.calls_text(), "a unit was started although a restore is pending")
        self.assertEqual(tree_digest(self.host.root), digest)

    def test_a_restore_left_pending_after_the_dispatch_still_stops_the_run(self):
        """The check at dispatch is advisory; the run asks again once it holds the locks."""
        bundle = self.bundle()
        self.assertEqual(self.broker("--dispatch", str(bundle)).returncode, 0)
        self.calls.write_text("")
        self.host.pending_settings()
        digest = tree_digest(self.host.root)
        self.assertEqual(self.run_update(bundle).returncode, 2)
        self.assertEqual(self.status()["phase"], "restore_transaction_pending")
        self.assertNothingChanged(digest)

    def test_the_run_asks_with_the_locks_held(self):
        """Asked without them the answer could be stale the moment after: a stand-in tool shows what the lock state is when the broker asks during the run."""
        record = self.root / "asked-with.txt"
        (self.tools / "saturn-maintenance-lock.py").write_text(
            "#!/usr/bin/env python3\nimport subprocess, sys\nreal = %r\n" % str(LOCK_TOOL) +
            "if sys.argv[1] == 'pending-restores':\n"
            "    rows = []\n"
            "    for name in ('release', 'radio', 'repository'):\n"
            "        rc = subprocess.run([sys.executable, real, '--lock-dir', %r, 'probe', '--operation', 'x', '--resources', name], capture_output=True).returncode\n" % str(self.root / "locks") +
            "        rows.append(f'{name}={rc}')\n"
            "    open(%r, 'a').write(' '.join(rows) + '\\n')\n" % str(record) +
            "sys.exit(subprocess.run([sys.executable, real] + sys.argv[1:]).returncode)\n")
        self.assertEqual(self.run_update().returncode, 0)
        lines = record.read_text().splitlines()
        self.assertIn("release=75 radio=75 repository=75", lines, "during the run the question must be asked with all three locks held: " + repr(lines))

    def test_once_saturn_gos_startup_has_completed_it_the_same_update_goes_through(self):
        ids = self.host.pending_settings()
        self.assertEqual(self.run_update().returncode, 2)
        recovered = self.host.helper("recover", "--state-root", str(self.state_root))
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertEqual(json.loads(recovered.stdout)["recovered"], ids)
        self.calls.write_text("")
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.active(), NEW)
        self.assertEqual(self.status()["status"], "success")

    def test_finished_transactions_do_not_stop_it(self):
        self.host.pending_settings()
        self.assertEqual(self.host.helper("recover", "--state-root", str(self.state_root)).returncode, 0)
        self.host.write_record("20261014T000000Z-2-bbbbbbbb", status="committed")
        self.assertCountEqual(self.host.statuses(), ["rolled_back", "committed"])
        self.assertEqual(self.run_update().returncode, 0)

    def test_the_servers_recovery_during_the_update_changes_nothing_and_starts_nothing(self):
        """What saturn-go's restarted server meets: the same call as always, answered, with the state exactly as it was and the helper never started."""
        self.host.write_record("20261014T000000Z-3-cccccccc", status="rolled_back")
        digest = tree_digest(self.host.root)
        self.assertEqual(self.run_update().returncode, 0)
        self.assertEqual((self.root / "server.log").read_text().split(), ["server-start", "rc=0"])
        self.assertEqual(tree_digest(self.host.root), digest)
        self.assertFalse(self.helper_log.exists())

    def test_a_server_start_during_a_rollback_is_answered_the_same_way(self):
        self.host.write_record("20261014T000000Z-4-dddddddd", status="committed")
        digest = tree_digest(self.host.root)
        result = self.run_update(FAKE_OUTCOME="rolled-back")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.status()["phase"], "rolled_back")
        self.assertEqual((self.root / "server.log").read_text().split(), ["server-start", "rc=0"])
        self.assertEqual(tree_digest(self.host.root), digest)

    def test_a_question_that_cannot_be_answered_is_a_refusal_not_a_go_ahead(self):
        for rc, output in ((3, ""), (0, "not json"), (0, "{}"), (0, '{"status":"ok","pending":null}'), (1, '{"status":"ok","pending":[]}'),
                           (0, '{"status":"pending","pending":["x"]}'), (1, '{"status":"pending","pending":[]}'), (1, ""),
                           (0, '{"pending":[]}'), (0, '{"status":"pending","pending":[]}'), (0, '{"status":"unknown","pending":[]}'), (0, '[]'), (0, 'null')):
            with self.subTest(rc=rc, output=output):
                (self.tools / "saturn-maintenance-lock.py").write_text(STUB_TOOL.format(pending=output, rc=rc))
                result = self.run_update()
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn(self.status()["phase"], ("restore_check_failed", "restore_transaction_pending"))
                self.assertEqual(self.calls_text(), "")
                self.assertEqual(self.active(), OLD)

    def test_a_stand_in_that_never_answers_is_a_refusal_too(self):
        (self.tools / "saturn-maintenance-lock.py").write_text(
            "#!/usr/bin/env python3\nimport sys\nif sys.argv[1:] == ['capabilities']:\n    print('{\"schema\":1,\"recovery_check\":1}')\nelse:\n    sys.exit(7)\n")
        self.assertEqual(self.run_update().returncode, 2)
        self.assertEqual(self.status()["phase"], "restore_check_failed")
        self.assertEqual(self.calls_text(), "")


class SourceTests(unittest.TestCase):
    text = BROKER.read_text()

    def test_it_runs_the_existing_installer_and_activator_not_a_second_implementation(self):
        self.assertIn('INSTALLER = "saturn-release-install-root.sh"', self.text)
        self.assertIn('ACTIVATOR = "saturn-release-activate-root.sh"', self.text)
        for forbidden in ("systemctl", "ln -s", "os.symlink", "os.rename(", "shutil.copytree"):
            self.assertNotIn(forbidden, self.text, f"the broker touches {forbidden} itself instead of leaving it to the activator")

    def test_the_helpers_it_trusts_are_the_installed_ones(self):
        self.assertIn("def tool(env: Environment, name: str)", self.text)
        self.assertNotIn('os.path.join(bundle, "scripts", ACTIVATOR)', self.text)

    def test_every_privileged_helper_carries_a_contract_marker(self):
        for name in HELPERS:
            head = (REPO / "update_manager/scripts" / name).read_text()[:4096]
            self.assertRegex(head, r"saturn-helper-contract:\s*[0-9]+", name)


if __name__ == "__main__":
    unittest.main(verbosity=2)
