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
import json, os, sys
calls = os.environ["FAKE_CALLS"]
open(calls, "a").write("activator " + " ".join(sys.argv[1:]) + "\n")
commit = sys.argv[-1]
outcome = os.environ.get("FAKE_OUTCOME", "success")
record_path, current, releases, perf = os.environ["FAKE_RECORD"], os.environ["FAKE_CURRENT"], os.environ["FAKE_RELEASES"], os.environ["FAKE_PERF"]

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
        (self.tools / "saturn-release-install-root.sh").write_text(FAKE_INSTALLER)
        (self.tools / "saturn-release-activate-root.sh").write_text(FAKE_ACTIVATOR)
        for name in HELPERS[1:]:
            (self.tools / name).write_text(helper_text(name, 1))
        (self.tools / "saturn-release-activate-root.sh").write_text("#!/usr/bin/env python3\n# saturn-helper-contract: 1\n" + FAKE_ACTIVATOR.split("\n", 1)[1])
        shutil.copy(LOCK_TOOL, self.tools / "saturn-maintenance-lock.py")
        for name in ("saturn-release-install-root.sh", "saturn-release-activate-root.sh", "saturn-remote-validate.py", "saturn-remote-receive-guard.js", "saturn-maintenance-lock.py"):
            (self.tools / name).chmod(0o755)
        for resource in ("release", "radio", "repository", "package"):
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
                   SATURN_RELEASE_UPDATE_LOCK_WAIT="1", SATURN_RELEASE_UPDATE_SELF=str(BROKER),
                   FAKE_CALLS=str(self.calls), FAKE_RELEASES=str(self.releases), FAKE_RECORD=str(self.record), FAKE_CURRENT=str(self.current), FAKE_PERF=str(self.perf))
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
        self.assertRegex(line, rf"--unit saturn-release-update-[0-9]{{14}} --collect --no-block {BROKER} --run {bundle}$")
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

    def test_a_lock_that_is_released_in_time_is_waited_for(self):
        import threading, time
        held = open(self.root / "locks" / "release.lock", "r+")
        fcntl.flock(held, fcntl.LOCK_EX)
        threading.Timer(0.6, lambda: (fcntl.flock(held, fcntl.LOCK_UN), held.close())).start()
        result = self.run_update(SATURN_RELEASE_UPDATE_LOCK_WAIT="20")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(self.active(), NEW)

    def test_the_work_holds_the_locks_while_it_runs(self):
        probe = self.root / "probe.txt"
        (self.tools / "saturn-release-install-root.sh").write_text(
            f'#!/usr/bin/env bash\necho "installer $*" >>"$FAKE_CALLS"\n[ "$1" = --validate ] && {{ "{LOCK_TOOL}" --lock-dir "{self.root}/locks" probe --operation x --resources release,radio >"{probe}" 2>&1; echo "rc=$?" >>"{probe}"; exit 0; }}\n'
            'mkdir -p "$FAKE_RELEASES/$(basename "$1")"\n')
        self.assertEqual(self.run_update().returncode, 0)
        self.assertIn("rc=75", probe.read_text(), "the locks were not held while the installer ran")

    def test_every_step_is_in_the_log(self):
        self.run_update(FAKE_OUTCOME="rolled-back")
        log = Path(self.status()["log"]).read_text()
        for expected in ("saturn-release-install-root.sh --validate", f"saturn-release-install-root.sh {self.staging / NEW}", f"saturn-release-activate-root.sh {NEW}", "[exit 1]"):
            self.assertIn(expected, log)

    def test_locked_belongs_to_run(self):
        self.assertNotEqual(self.broker("--dispatch", "--locked", str(self.bundle())).returncode, 0)

    def test_one_mode_at_a_time(self):
        self.assertNotEqual(self.broker("--preflight", "--run").returncode, 0)
        self.assertNotEqual(self.broker().returncode, 0)


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
