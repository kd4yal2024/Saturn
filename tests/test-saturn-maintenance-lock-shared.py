#!/usr/bin/env python3
"""The lock tool's one admission to a release update's shared hold: saturn-go's own startup recovery.

A managed update holds `release` exclusively and `radio` and `repository` shared for its whole run (saturn-release-update-root.py). Saturn Go's startup runs
`restore:recover` on exactly `repository,radio` and exits when it cannot take them, so that one operation must be let in; every other request is exclusive and must be
refused (75) for as long as the update holds the locks. These tests use real flock holders, as the host does, and the real tool.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parents[1] / "update_manager/scripts/saturn-maintenance-lock.py"
RESOURCES = ("release", "repository", "disk", "fpga", "package", "network", "radio", "read-only")
BUSY = 75

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
        self.locks = Path(self.tmp.name)
        for name in RESOURCES:
            lock = self.locks / f"{name}.lock"
            lock.write_text("")
            lock.chmod(0o660)
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
        """What the update broker holds from its first lock to the end of its run."""
        return self.hold("release:ex", "radio:sh", "repository:sh")

    def tool(self, action, operation, resources, *extra):
        return subprocess.run([str(TOOL), "--lock-dir", str(self.locks), action, "--operation", operation, "--resources", resources, *extra],
                              capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=60)


class CapabilityTests(Rig):
    def test_it_says_it_can_share_with_the_servers_recovery_and_takes_no_lock(self):
        result = subprocess.run([str(TOOL), "--lock-dir", str(self.locks / "does-not-exist"), "capabilities"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        capabilities = json.loads(result.stdout)
        self.assertGreaterEqual(capabilities["shared_recovery"], 1)
        self.assertEqual(result.stderr, "")


class TheServersRecovery(Rig):
    def test_it_is_admitted_to_the_hold_of_a_running_update_in_every_mode_of_the_tool(self):
        self.transaction()
        for action, extra in (("probe", ()), ("hold", ()), ("run", ("--", "true"))):
            result = self.tool(action, "restore:recover", "repository,radio", *extra)
            self.assertEqual(result.returncode, 0, f"{action}: {result.stderr}")

    def test_it_is_admitted_with_the_resources_in_either_order(self):
        self.transaction()
        self.assertEqual(self.tool("probe", "restore:recover", "radio,repository").returncode, 0)

    def test_it_runs_its_command_while_the_update_holds_the_locks(self):
        self.transaction()
        marker = self.locks / "ran"
        result = self.tool("run", "restore:recover", "repository,radio", "--", "touch", str(marker))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(marker.exists())

    def test_without_an_update_it_takes_the_locks_exclusively_like_everything_else(self):
        first = subprocess.Popen([str(TOOL), "--lock-dir", str(self.locks), "hold", "--operation", "restore:recover", "--resources", "repository,radio"],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (first.stdin.close(), first.wait(timeout=10)))
        self.assertIn("locked", first.stdout.readline())
        for operation in ("restore:recover", "restore:settings"):
            self.assertEqual(self.tool("probe", operation, "repository,radio").returncode, BUSY, operation)
        self.assertEqual(self.tool("probe", "x", "release").returncode, 0, "looking for an update must not leave the release lock held")

    def test_looking_for_an_update_does_not_leave_the_release_lock_held(self):
        transaction = self.transaction()
        self.assertEqual(self.tool("probe", "restore:recover", "repository,radio").returncode, 0)
        self.stop(transaction)
        self.assertEqual(self.tool("probe", "x", "release,radio,repository").returncode, 0)

    def test_it_is_not_admitted_to_an_exclusive_hold(self):
        for held in ("radio", "repository"):
            exclusive = self.hold("release:ex", f"{held}:ex")
            self.assertEqual(self.tool("probe", "restore:recover", "repository,radio").returncode, BUSY, f"{held} held exclusively")
            self.stop(exclusive)

    def test_it_is_not_admitted_to_a_legacy_deployments_hold(self):
        deployment = self.hold("release:ex", "repository:ex", "package:ex", "radio:ex")
        self.assertEqual(self.tool("probe", "restore:recover", "repository,radio").returncode, BUSY)
        self.stop(deployment)

    def test_it_is_not_admitted_to_a_shared_hold_without_an_update_behind_it(self):
        self.hold("radio:sh", "repository:sh")
        self.assertEqual(self.tool("probe", "restore:recover", "repository,radio").returncode, BUSY)

    def test_an_exclusive_holder_of_one_lock_keeps_it_out_of_both(self):
        self.hold("release:ex", "repository:sh")
        outsider = self.hold("radio:ex")
        self.assertEqual(self.tool("probe", "restore:recover", "repository,radio").returncode, BUSY)
        self.stop(outsider)


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

    def test_a_recovery_that_is_running_keeps_everyone_else_out_even_after_the_update_has_ended(self):
        transaction = self.transaction()
        recovery = subprocess.Popen([str(TOOL), "--lock-dir", str(self.locks), "hold", "--operation", "restore:recover", "--resources", "repository,radio"],
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (recovery.stdin.close() if not recovery.stdin.closed else None, recovery.wait(timeout=10)))
        self.assertIn("locked", recovery.stdout.readline())
        self.stop(transaction)
        self.assertEqual(self.tool("probe", "flash_fpga.sh", "fpga,radio").returncode, BUSY, "the recovery's own hold must keep an exclusive request out")
        self.assertEqual(self.tool("probe", "x", "release").returncode, 0, "a running recovery must not keep the release lock: looking for an update takes it only for a moment")
        recovery.stdin.close()
        recovery.wait(timeout=10)
        self.assertEqual(self.tool("probe", "flash_fpga.sh", "fpga,radio").returncode, 0)


class OnlyThatOperation(Rig):
    """The rule is matched by name and by exactly those two resources: nothing near it is let in."""

    def test_names_that_merely_resemble_it_are_refused(self):
        self.transaction()
        for operation in ("restore:recover2", "xrestore:recover", "restore:recov", "Restore:Recover", "restore:recover ", "restore:", "recover", "restore:settings",
                          "restore:recover,repository", ""):
            self.assertEqual(self.tool("probe", operation, "repository,radio").returncode, BUSY, repr(operation))

    def test_other_resources_with_its_name_are_refused(self):
        self.transaction()
        for resources in ("radio", "repository", "release,radio,repository", "repository,radio,package", "fpga,radio", "repository,radio,fpga"):
            self.assertEqual(self.tool("probe", "restore:recover", resources).returncode, BUSY, resources)


if __name__ == "__main__":
    unittest.main(verbosity=2)
