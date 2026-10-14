#!/usr/bin/env python3
"""The lock tool and a managed update's shared hold: what Saturn Go's startup recovery gets, and what nothing else does.

A managed update holds `release` exclusively and `radio` and `repository` shared for its whole run (saturn-release-update-root.py). Saturn Go's startup runs
`saturn-lock run --operation restore:recover --resources repository,radio -- saturn-restore-transaction.py recover --state-root <root>` and exits when it cannot
take those locks, so that one call must be answered while the update runs. It is answered by a READ-ONLY CHECK that the lock tool performs itself: the recovery
proper writes (it restores or deletes settings, moves the repository pointer, removes staging directories, rewrites the transaction records), and under a shared
hold nothing would keep two of them, or one of them and the update, from writing at once. So during an update the tool never runs what it was handed; it
serializes callers; and a restore transaction that is still pending is reported, not completed (the update refuses to start with one pending). Every other request
is exclusive and is refused (75) for as long as the update holds the locks. These tests use real flock holders, as the host does, the real tool and the real
restore helper, with real pending restore transactions.
"""
import fcntl
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from restore_fixtures import HELPER, Host, tree_digest

SCRIPTS = Path(__file__).resolve().parents[1] / "update_manager/scripts"
TOOL = SCRIPTS / "saturn-maintenance-lock.py"
RESOURCES = ("release", "repository", "disk", "fpga", "package", "network", "radio", "read-only")
BUSY = 75
PENDING_EXIT = 1

HOLDER = r'''
import fcntl, os, sys
lock_dir = sys.argv[1]
for spec in sys.argv[2:]:
    name, mode = spec.split(":")
    fd = os.open(f"{lock_dir}/{name}.lock", os.O_RDWR)
    fcntl.flock(fd, fcntl.LOCK_EX if mode == "ex" else fcntl.LOCK_SH)
print("ready", flush=True)
sys.stdin.read()
'''

class Rig(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="saturn-lock-shared-")
        self.addCleanup(self.tmp.cleanup)
        base = Path(self.tmp.name)
        self.locks = base / "locks"
        self.locks.mkdir()
        for name in RESOURCES:
            lock = self.locks / f"{name}.lock"
            lock.write_text("")
            lock.chmod(0o660)
        self.host = Host(base / "host")
        self.state = self.host.state
        # Saturn Go starts the helper by its installed path; here that is a stand-in that runs the real one with this interpreter...
        self.real_helper = base / "bin/saturn-restore-transaction.py"
        self.real_helper.parent.mkdir()
        self.real_helper.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{HELPER}" "$@"\n')
        self.real_helper.chmod(0o755)
        # ...and this one only records that it was started
        self.marker = base / "started"
        self.recording_helper = base / "recording/saturn-restore-transaction.py"
        self.recording_helper.parent.mkdir()
        self.recording_helper.write_text(f'#!/bin/sh\necho "$@" >>"{self.marker}"\nprintf \'{{"status":"ok","recovered":[]}}\\n\'\n')
        self.recording_helper.chmod(0o755)
        self.holders = []
        self.addCleanup(self.release_all)

    def release_all(self):
        for holder in self.holders:
            self.stop(holder)

    def hold(self, *specs):
        """Take locks the way another process of the host does, e.g. hold("release:ex", "radio:sh")."""
        holder = subprocess.Popen([sys.executable, "-c", HOLDER, str(self.locks), *specs], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.assertEqual(holder.stdout.readline().strip(), "ready")
        self.holders.append(holder)
        return holder

    def stop(self, holder):
        if holder.poll() is None:
            holder.stdin.close()
            holder.wait(timeout=10)

    def transaction(self):
        """What the update broker holds from its first lock to the end of its run (one update at a time: asking again returns the one that is running)."""
        if getattr(self, "update", None) is None or self.update.poll() is not None:
            self.update = self.hold("release:ex", "radio:sh", "repository:sh")
        return self.update

    def tool(self, action, operation, resources, *extra):
        return subprocess.run([sys.executable, str(TOOL), "--lock-dir", str(self.locks), action, "--operation", operation, "--resources", resources, *extra],
                              capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=60)

    def recovery(self, helper=None, *, state=None, gate_wait="0.3", resources="repository,radio", operation="restore:recover", before=(), after=()):
        """Saturn Go's startup recovery, exactly as rust-server/src/maintenance_lock.rs and restore.rs build it."""
        command = [str(helper or self.real_helper), "recover", "--state-root", str(state or self.state), *after]
        return self.tool("run", operation, resources, "--recovery-gate-wait", gate_wait, *before, "--", *command)

    def started(self):
        return self.marker.read_text() if self.marker.exists() else ""


class CapabilityTests(Rig):
    def test_it_says_it_can_check_recovery_for_the_update_and_takes_no_lock(self):
        result = subprocess.run([sys.executable, str(TOOL), "--lock-dir", str(self.locks / "does-not-exist"), "capabilities"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        capabilities = json.loads(result.stdout)
        self.assertGreaterEqual(capabilities["recovery_check"], 1)
        self.assertNotIn("shared_recovery", capabilities, "the name of the earlier, broader admission must not be claimed by this tool")
        self.assertEqual(result.stderr, "")


class TheServersRecoveryDuringAnUpdate(Rig):
    def test_it_is_answered_by_a_check_and_the_update_keeps_its_locks(self):
        transaction = self.transaction()
        result = self.recovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        answer = json.loads(result.stdout)
        self.assertEqual((answer["status"], answer["recovered"]), ("ok", []), "saturn-go reads these two keys")
        self.assertTrue(answer["checked_only"])
        self.assertIsNone(transaction.poll())
        self.assertEqual(self.tool("probe", "flash_fpga.sh", "fpga,radio").returncode, BUSY, "the update's hold must outlive the recovery")

    def test_it_is_answered_with_the_resources_in_either_order(self):
        self.transaction()
        self.assertEqual(self.recovery(resources="radio,repository").returncode, 0)

    def test_it_never_runs_what_it_was_handed(self):
        """The reviewer's mislabeled-command probe, and the stronger form of it: even the right helper is not started under the shared hold."""
        self.transaction()
        result = self.recovery(self.recording_helper)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.started(), "", "the helper was started under the update's shared hold")

    def test_it_reads_and_writes_nothing(self):
        self.transaction()
        self.host.write_record("20261014T000000Z-1-aaaaaaaa", status="rolled_back")
        before = tree_digest(self.host.root)
        self.assertEqual(self.recovery().returncode, 0)
        self.assertEqual(tree_digest(self.host.root), before)

    def test_a_missing_transaction_directory_is_nothing_to_recover_and_stays_missing(self):
        self.transaction()
        self.assertFalse((self.state / "restore-transactions").exists())
        self.assertEqual(self.recovery().returncode, 0)
        self.assertFalse((self.state / "restore-transactions").exists(), "the check must not create what the recovery proper creates")

    def test_a_pending_settings_restore_is_reported_and_left_exactly_as_it_was(self):
        """The recovery proper rolls the settings back and rewrites the record. During an update that is exactly what must not happen."""
        ids = self.host.pending_settings()
        self.assertEqual((self.state / "remote_settings.json").read_text().strip(), '{"value":"new"}', "the crash left the restore half-applied")
        self.transaction()
        before = tree_digest(self.host.root)
        result = self.recovery()
        self.assertEqual(result.returncode, PENDING_EXIT, result.stdout + result.stderr)
        self.assertIn(ids[0], result.stderr)
        self.assertEqual(result.stdout, "", "saturn-go must not be told that anything was recovered")
        self.assertEqual(tree_digest(self.host.root), before, "a pending restore was touched under the update's shared hold")
        self.assertEqual(self.host.statuses(), ["applying"])

    def test_a_pending_source_restore_is_reported_and_left_exactly_as_it_was(self):
        ids = self.host.pending_source()
        staging = list((self.state / "repository-restores").glob(".*.staging"))
        self.assertTrue(staging, "the crash left a staging directory")
        self.transaction()
        before = tree_digest(self.host.root)
        result = self.recovery()
        self.assertEqual(result.returncode, PENDING_EXIT, result.stdout + result.stderr)
        self.assertIn(ids[0], result.stderr)
        self.assertEqual(tree_digest(self.host.root), before, "the staging directory or the repository pointer was touched under the shared hold")
        self.assertTrue(all(path.exists() for path in staging))

    def test_once_the_update_has_ended_the_recovery_proper_completes_it_under_exclusive_locks(self):
        ids = self.host.pending_settings()
        transaction = self.transaction()
        self.assertEqual(self.recovery().returncode, PENDING_EXIT)
        self.stop(transaction)
        result = self.recovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["recovered"], ids)
        self.assertEqual((self.state / "remote_settings.json").read_text().strip(), '{"value":"old"}')
        self.assertEqual(self.host.statuses(), ["rolled_back"])

    def test_two_recoveries_at_once_are_serialized(self):
        """The first caller is held inside its check (it owns the gate); the second must not be inside it too."""
        self.transaction()
        directory = self.state / "restore-transactions"
        directory.mkdir()
        gate = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        self.addCleanup(lambda: os.close(gate) if gate is not None else None)
        fcntl.flock(gate, fcntl.LOCK_EX)
        second = self.recovery(gate_wait="0.4")
        self.assertEqual(second.returncode, BUSY, second.stdout + second.stderr)
        self.assertIn("another", second.stderr)
        fcntl.flock(gate, fcntl.LOCK_UN)
        self.assertEqual(self.recovery().returncode, 0, "the gate must be free again once the first caller is done")
        # asked exclusively: even a caller that only holds it shared (two checks inside it at once would be that) keeps the next one out
        fcntl.flock(gate, fcntl.LOCK_SH)
        third = self.recovery(gate_wait="0.4")
        self.assertEqual(third.returncode, BUSY, "the gate is requested shared, so two recoveries can be inside it together: " + third.stdout + third.stderr)
        fcntl.flock(gate, fcntl.LOCK_UN)

    def test_a_caller_that_waits_for_the_first_gets_in_when_the_first_is_done(self):
        self.transaction()
        directory = self.state / "restore-transactions"
        directory.mkdir()
        gate = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        self.addCleanup(os.close, gate)
        fcntl.flock(gate, fcntl.LOCK_EX)
        waiting = subprocess.Popen([sys.executable, str(TOOL), "--lock-dir", str(self.locks), "run", "--operation", "restore:recover", "--resources", "repository,radio",
                                    "--recovery-gate-wait", "20", "--", str(self.real_helper), "recover", "--state-root", str(self.state)],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: waiting.poll() is None and waiting.kill())
        with self.assertRaises(subprocess.TimeoutExpired):
            waiting.communicate(timeout=0.6)
        fcntl.flock(gate, fcntl.LOCK_UN)
        out, err = waiting.communicate(timeout=20)
        self.assertEqual(waiting.returncode, 0, err)
        self.assertEqual(json.loads(out)["recovered"], [])

    def test_probe_and_hold_under_the_recovery_name_are_not_admitted(self):
        """Only the call that Saturn Go makes is answered; a lock held or probed under that name proves nothing and would hold the update's locks open."""
        self.transaction()
        for action in ("probe", "hold"):
            self.assertEqual(self.tool(action, "restore:recover", "repository,radio").returncode, BUSY, action)

    def test_it_is_not_admitted_to_an_exclusive_hold(self):
        for held in ("radio", "repository"):
            exclusive = self.hold("release:ex", f"{held}:ex")
            self.assertEqual(self.recovery().returncode, BUSY, f"{held} held exclusively")
            self.stop(exclusive)

    def test_it_is_not_admitted_to_a_legacy_deployments_hold(self):
        deployment = self.hold("release:ex", "repository:ex", "package:ex", "radio:ex")
        self.assertEqual(self.recovery().returncode, BUSY)
        self.stop(deployment)

    def test_it_is_not_admitted_to_a_shared_hold_without_an_update_behind_it(self):
        self.hold("radio:sh", "repository:sh")
        self.assertEqual(self.recovery().returncode, BUSY)

    def test_an_exclusive_holder_of_one_lock_keeps_it_out_of_both(self):
        self.hold("release:ex", "repository:sh")
        outsider = self.hold("radio:ex")
        self.assertEqual(self.recovery().returncode, BUSY)
        self.stop(outsider)

    def test_looking_for_an_update_does_not_leave_the_release_lock_held(self):
        transaction = self.transaction()
        self.assertEqual(self.recovery().returncode, 0)
        self.stop(transaction)
        self.assertEqual(self.tool("probe", "x", "release,radio,repository").returncode, 0)


class TheServersRecoveryWithoutAnUpdate(Rig):
    """Outside an update nothing changes: the recovery proper runs under the exclusive locks, as it always did."""

    def test_the_helper_is_started_under_exclusive_locks(self):
        result = self.recovery(self.recording_helper)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.started().split(), ["recover", "--state-root", str(self.state)])

    def test_a_pending_restore_is_rolled_back_there(self):
        ids = self.host.pending_settings()
        result = self.recovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["recovered"], ids)
        self.assertEqual((self.state / "remote_settings.json").read_text().strip(), '{"value":"old"}')

    def test_it_takes_the_locks_exclusively_like_everything_else(self):
        first = subprocess.Popen([sys.executable, str(TOOL), "--lock-dir", str(self.locks), "hold", "--operation", "restore:recover", "--resources", "repository,radio"],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (first.stdin.close(), first.wait(timeout=10)))
        self.assertIn("locked", first.stdout.readline())
        for operation in ("restore:recover", "restore:settings"):
            self.assertEqual(self.tool("probe", operation, "repository,radio").returncode, BUSY, operation)
        self.assertEqual(self.recovery().returncode, BUSY, "a second recovery while the first one holds the locks")
        self.assertEqual(self.tool("probe", "x", "release").returncode, 0)


class OnlyTheCallSaturnGoMakes(Rig):
    """The reviewer's third probe: an unrelated command under the recovery's name. The admission is for one exact call, not for a label."""

    def refused(self, command, *before, label=""):
        self.transaction()
        result = self.tool("run", "restore:recover", "repository,radio", *before, "--", *command)
        self.assertEqual(result.returncode, BUSY, f"{label or command}: was let in: {result.stdout}{result.stderr}")
        self.assertFalse(self.marker.exists(), f"{label or command}: ran under the update's shared hold")

    def test_unrelated_commands_are_refused_and_never_run(self):
        mark = f"echo ran >>{self.marker}"
        for command in (["touch", str(self.marker)], ["true"], ["/bin/sh", "-c", mark], [sys.executable, "-c", f"open({str(self.marker)!r}, 'w')"],
                        ["sh", "-c", f"{self.real_helper} recover --state-root {self.state}; {mark}"]):
            self.refused(command)

    def test_the_helper_with_another_verb_or_other_arguments_is_refused(self):
        state = str(self.state)
        for command in ([self.recording_helper, "settings", "--state-root", state],
                        [self.recording_helper, "source", "--state-root", state],
                        [self.recording_helper, "status", "--state-root", state],
                        [self.recording_helper, "recover", "--state-root", state, "--dry-run"],
                        [self.recording_helper, "recover", "--state-root", state, "--state-root", state],
                        [self.recording_helper, "recover", "--state-root"],
                        [self.recording_helper, "recover", "--state-roots", state],
                        [self.recording_helper, "recover", "--root", state],
                        [self.recording_helper, "recover", "--dry-run", state],
                        [self.recording_helper, "recover", "-s", state],
                        [self.recording_helper, "recover", state],
                        [self.recording_helper, "recover"],
                        [self.recording_helper, "--state-root", state, "recover"],
                        [self.recording_helper, "recover", "--state-root", "relative/state"],
                        [self.recording_helper, "recover", "--state-root", ""],
                        [self.recording_helper]):
            self.refused([str(part) for part in command])

    def test_another_program_with_the_same_arguments_is_refused(self):
        other = Path(self.tmp.name) / "other-tool"
        other.write_text(f'#!/bin/sh\necho "$@" >>"{self.marker}"\n')
        other.chmod(0o755)
        for program in (str(other), "/bin/true", "saturn-restore-transaction.py.sh", str(self.recording_helper) + "x", "/bin/sh"):
            self.refused([program, "recover", "--state-root", str(self.state)], label=program)

    def test_the_call_with_job_options_or_a_deadline_is_not_saturn_gos_startup_call(self):
        for before in (("--timeout-seconds", "5"), ("--job-id", "j1"), ("--output-file", str(Path(self.tmp.name) / "out")),
                       ("--result-file", str(Path(self.tmp.name) / "result"))):
            self.refused([str(self.recording_helper), "recover", "--state-root", str(self.state)], *before, label=str(before))

    def test_names_that_merely_resemble_it_are_refused(self):
        self.transaction()
        for operation in ("restore:recover2", "xrestore:recover", "restore:recov", "Restore:Recover", "restore:recover ", "restore:", "recover", "restore:settings",
                          "restore:recover,repository", ""):
            result = self.recovery(self.recording_helper, operation=operation)
            self.assertEqual(result.returncode, BUSY, repr(operation))
        self.assertEqual(self.started(), "")

    def test_other_resources_with_its_name_are_refused(self):
        self.transaction()
        for resources in ("radio", "repository", "release,radio,repository", "repository,radio,package", "fpga,radio", "repository,radio,fpga"):
            self.assertEqual(self.recovery(self.recording_helper, resources=resources).returncode, BUSY, resources)
        self.assertEqual(self.started(), "")


class EveryoneElse(Rig):
    def test_every_other_operation_is_refused_for_as_long_as_the_update_runs(self):
        self.transaction()
        # the operations saturn-go starts, with the resources rust-server/src/maintenance_lock.rs gives them
        for operation, resources in (("flash_fpga.sh", "fpga,radio"), ("restore:settings", "repository,radio"), ("restore:source", "repository,radio"),
                                     ("update-pihpsdr.py", "repository,package,radio"), ("fix-led-power-button.sh", "radio"),
                                     ("appliance-update", "release,repository,package,radio"), ("operator-maintenance.sh", "release,repository,disk,fpga,package,network,radio"),
                                     ("anything", "repository"), ("anything", "radio")):
            for action, extra in (("probe", ()), ("hold", ()), ("run", ("--", "true"))):
                result = self.tool(action, operation, resources, *extra)
                self.assertEqual(result.returncode, BUSY, f"{operation} ({action}) on {resources} was let in: {result.stderr}")

    def test_a_script_that_locks_the_radio_file_directly_is_refused_too(self):
        self.transaction()
        for name in ("radio", "repository"):
            direct = subprocess.run(["flock", "-n", str(self.locks / f"{name}.lock"), "true"])
            self.assertEqual(direct.returncode, 1, f"{name}.lock was taken by a plain flock")

    def test_resources_the_update_does_not_hold_stay_available(self):
        self.transaction()
        for resources in ("network", "read-only", "disk", "fpga"):
            self.assertEqual(self.tool("probe", "x", resources).returncode, 0, resources)


class PendingRestores(Rig):
    """`pending-restores`: what the update broker asks while it holds the locks, and what the check above is made of."""

    def ask(self, state=None):
        return subprocess.run([sys.executable, str(TOOL), "pending-restores", "--state-root", str(state or self.state)], capture_output=True, text=True, timeout=30)

    def test_nothing_pending_is_exit_0_and_an_empty_list(self):
        for build in (lambda: None, lambda: (self.state / "restore-transactions").mkdir(),
                      lambda: self.host.write_record("a", status="committed"), lambda: self.host.write_record("b", status="rolled_back")):
            build()
        result = self.ask()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"status": "ok", "pending": []})

    def test_a_real_pending_transaction_is_exit_1_and_named(self):
        ids = self.host.pending_settings()
        result = self.ask()
        self.assertEqual(result.returncode, PENDING_EXIT)
        answer = json.loads(result.stdout)
        self.assertEqual(answer["status"], "pending")
        self.assertTrue(answer["pending"][0].startswith(ids[0]), answer)
        self.assertIn("applying", answer["pending"][0])

    def test_it_takes_no_lock_and_writes_nothing(self):
        self.host.pending_settings()
        holder = self.hold("release:ex", "radio:ex", "repository:ex")
        before = tree_digest(self.host.root)
        self.assertEqual(self.ask().returncode, PENDING_EXIT)
        self.assertEqual(tree_digest(self.host.root), before)
        self.assertIsNone(holder.poll())

    def test_it_needs_an_absolute_state_root(self):
        result = self.ask("relative/state")
        self.assertNotEqual(result.returncode, 0)

    def test_what_it_calls_clean_is_exactly_what_the_recovery_proper_leaves_alone(self):
        """Conformance: for every kind of leftover, the check says "clean" if and only if the real `recover` succeeds having recovered nothing."""
        def pending_real(host):
            host.pending_settings()

        def build(*records, files=()):
            def go(host):
                for name, fields in records:
                    host.write_record(name, **fields)
                for name in files:
                    (host.state / "restore-transactions").mkdir(exist_ok=True)
                    (host.state / "restore-transactions" / name).write_text("")
            return go

        def broken_json(host):
            (host.state / "restore-transactions/j").mkdir(parents=True)
            (host.state / "restore-transactions/j/transaction.json").write_text("{")

        def no_record(host):
            (host.state / "restore-transactions/n").mkdir(parents=True)

        def a_link(host):
            (host.state / "restore-transactions").mkdir()
            real = host.root / "real-transaction"
            real.mkdir()
            (real / "transaction.json").write_text(json.dumps({"format": "saturn-restore-transaction", "schema_version": 1, "id": "l", "status": "committed"}))
            os.symlink(real, host.state / "restore-transactions/l")

        def directory_is_a_file(host):
            (host.state / "restore-transactions").write_text("")

        def directory_is_a_link(host):
            real = host.root / "elsewhere"
            real.mkdir()
            os.symlink(real, host.state / "restore-transactions")

        cases = [
            ("no directory", lambda host: None, True),
            ("the directory is a file", directory_is_a_file, False),
            ("the directory is a link", directory_is_a_link, False),
            ("empty directory", lambda host: (host.state / "restore-transactions").mkdir(), True),
            ("committed", build(("a", {"status": "committed"})), True),
            ("rolled back", build(("a", {"status": "rolled_back"})), True),
            ("committed and rolled back", build(("a", {"status": "committed"}), ("b", {"status": "rolled_back"})), True),
            ("a real pending settings restore", pending_real, False),
            ("staging", build(("a", {"status": "staging"})), False),
            ("recovery required", build(("a", {"status": "recovery_required"})), False),
            ("an unknown status", build(("a", {"status": "something-new"})), False),
            ("no status", build(("a", {"status": None})), False),
            ("one pending among finished ones", build(("a", {"status": "committed"}), ("b", {"status": "staging"})), False),
            ("an unknown kind that is pending", build(("a", {"status": "staging", "kind": "other"})), False),
            ("a record that is not JSON", broken_json, False),
            ("a directory without a record", no_record, False),
            ("an id that is not the directory's", build(("a", {"id": "b"})), False),
            ("another format", build(("a", {"format": "something-else"})), False),
            ("another schema version", build(("a", {"schema_version": 2})), False),
            ("a file where a transaction should be", build(files=("stray",)), False),
            ("a link to a transaction", a_link, False),
        ]
        for name, build_case, clean in cases:
            with self.subTest(case=name):
                with tempfile.TemporaryDirectory(prefix="saturn-lock-conformance-") as scratch:
                    host = Host(Path(scratch) / "host")
                    build_case(host)
                    verdict = subprocess.run([sys.executable, str(TOOL), "pending-restores", "--state-root", str(host.state)], capture_output=True, text=True).returncode
                    copy = Path(scratch) / "copy"
                    shutil.copytree(host.root, copy, symlinks=True)
                    real = subprocess.run([sys.executable, str(HELPER), "recover", "--state-root", str(copy / "state")], capture_output=True, text=True)
                    acted = real.returncode != 0 or bool(json.loads(real.stdout)["recovered"])
                    self.assertEqual(verdict == 0, clean, f"the check's verdict for {name}")
                    self.assertEqual(verdict == 0, not acted, f"the check and the recovery proper disagree about {name}: {real.stdout}{real.stderr}")

    def test_the_constants_it_mirrors_are_the_helpers(self):
        def load(path, name):
            spec = importlib.util.spec_from_file_location(name, path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            return module
        helper, tool = load(HELPER, "restore_helper"), load(TOOL, "lock_tool")
        self.assertEqual(tool.TRANSACTION_FORMAT, helper.TRANSACTION_FORMAT)
        self.assertEqual(tool.TRANSACTION_SCHEMA_VERSION, helper.TRANSACTION_SCHEMA_VERSION)
        self.assertEqual(tool.RESTORE_HELPER_NAME, HELPER.name)
        source = HELPER.read_text()
        for status in tool.FINISHED_TRANSACTIONS:
            self.assertIn(f'"{status}"', source)
        self.assertEqual(set(tool.FINISHED_TRANSACTIONS), {"committed", "rolled_back"}, "recover_transaction() returns early for exactly these")


if __name__ == "__main__":
    unittest.main(verbosity=2)
