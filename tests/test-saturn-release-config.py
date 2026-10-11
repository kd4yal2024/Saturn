#!/usr/bin/env python3
"""The installer writes /etc/default/saturn-release-activate through saturn-release-config.py. What an administrator chose (the activation policy,
the browser memory allowance) must survive running the installer again; what the installer owns must be refreshed; and the result must be a file
the REAL root activator accepts."""
from __future__ import annotations

import importlib.util
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TOOL = REPO / "update_manager/scripts/saturn-release-config.py"
ACTIVATOR = REPO / "update_manager/scripts/saturn-release-activate-root.sh"
INSTALLER = REPO / "update_manager/install_saturn_go_nginx.sh"

spec = importlib.util.spec_from_file_location("saturn_release_config", TOOL)
C = importlib.util.module_from_spec(spec)
sys.modules["saturn_release_config"] = C
spec.loader.exec_module(C)

COMMIT = "329f3109b17428ee04e3f14031da6cf25cb04810"


def defaults_text(activation="0", root="/opt/saturn", extra=""):
    return f'''# Managed by install_saturn_go_nginx.sh. Parsed as data by the root activator.
ACTIVATION_ENABLED="{activation}"
SATURN_ROOT="{root}"
RELEASES_ROOT="{root}/releases"
CURRENT_LINK="{root}/current"
TRANSACTION_FILE="/var/lib/saturn-state/deployments/current.json"
LOCK_FILE="/run/lock/saturn-release-activate.lock"
MANIFEST_TOOL="/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
COMPONENTS_FILE="/usr/local/lib/saturn-go/release/components-v1.json"
STATE_TOOL="/usr/local/lib/saturn-go/scripts/saturn-state-compatibility.py"
STATE_ROOT="/var/lib/saturn-state"
STATE_BACKUP_ROOT="/var/lib/saturn-state/deployments/state-backups"
SYSTEMD_ROOT="/etc/systemd/system"
READY_TIMEOUT_SECONDS="30"
TRANSACTION_GROUP="pi"
VALIDATION_ENABLED="1"
VALIDATOR="/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py"
VALIDATOR_GUARD="/usr/local/lib/saturn-go/scripts/saturn-remote-receive-guard.js"
{extra}'''


class Rig(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="saturn-release-config-")
        self.addCleanup(self.dir.cleanup)
        self.path = Path(self.dir.name)
        self.out = self.path / "saturn-release-activate"

    def merge(self, defaults="", existing=None, managed="keep", out=None, check=True):
        defaults_file = self.path / "defaults"
        defaults_file.write_text(defaults or defaults_text())
        argv = ["merge", "--defaults", str(defaults_file), "--out", str(out or self.out), "--managed-updates", managed]
        if existing is not None:
            existing_file = self.path / "existing"
            existing_file.write_text(existing)
            argv += ["--existing", str(existing_file)]
        notes = []
        original = C.sys.stderr
        import io
        C.sys.stderr = io.StringIO()
        try:
            code = C.main(argv)
            notes = C.sys.stderr.getvalue().splitlines()
        finally:
            C.sys.stderr = original
        self.assertEqual(code, 0)
        return C.parse(str(out or self.out)) and dict(C.parse(str(out or self.out))), notes

    def values(self, **kw):
        return self.merge(**kw)[0]


class MergeTests(Rig):
    def test_a_fresh_installation_takes_the_installers_defaults(self):
        got = self.values()
        self.assertEqual(got["ACTIVATION_ENABLED"], "0")
        self.assertNotIn("VALIDATION_MEMORY_MAX", got)

    def test_a_fresh_installation_can_enable_managed_updates_explicitly(self):
        self.assertEqual(self.values(managed="enable")["ACTIVATION_ENABLED"], "1")

    def test_the_memory_allowance_survives_the_installer(self):
        got = self.values(existing='ACTIVATION_ENABLED="0"\nVALIDATION_MEMORY_MAX="500M"\n')
        self.assertEqual(got["VALIDATION_MEMORY_MAX"], "500M")

    def test_an_explicitly_disabled_policy_survives_the_installer(self):
        got = self.values(defaults=defaults_text(activation="1"), existing='ACTIVATION_ENABLED="0"\n')
        self.assertEqual(got["ACTIVATION_ENABLED"], "0", "the installer's default must not override an administrator's explicit 0")

    def test_an_enabled_policy_survives_the_installer(self):
        got = self.values(defaults=defaults_text(activation="0"), existing='ACTIVATION_ENABLED="1"\n')
        self.assertEqual(got["ACTIVATION_ENABLED"], "1", "running the installer again must not switch managed updates off")

    def test_enabling_on_an_existing_installation_is_an_explicit_choice(self):
        got = self.values(existing='ACTIVATION_ENABLED="0"\nVALIDATION_MEMORY_MAX="500M"\n', managed="enable")
        self.assertEqual((got["ACTIVATION_ENABLED"], got["VALIDATION_MEMORY_MAX"]), ("1", "500M"))

    def test_disabling_is_an_explicit_choice_too(self):
        self.assertEqual(self.values(existing='ACTIVATION_ENABLED="1"\n', managed="disable")["ACTIVATION_ENABLED"], "0")

    def test_the_installers_own_paths_are_refreshed(self):
        got = self.values(defaults=defaults_text(root="/opt/saturn2"), existing='SATURN_ROOT="/opt/saturn"\nCURRENT_LINK="/opt/saturn/current"\nACTIVATION_ENABLED="1"\n')
        self.assertEqual((got["SATURN_ROOT"], got["CURRENT_LINK"]), ("/opt/saturn2", "/opt/saturn2/current"))
        self.assertEqual(got["ACTIVATION_ENABLED"], "1")

    def test_every_administrator_setting_is_kept(self):
        existing = (f'ACTIVATION_ENABLED="1"\nVALIDATION_ENABLED="0"\nVALIDATION_MEMORY_MAX="600M"\nVALIDATION_TIMEOUT_SECONDS="200"\n'
                    f'VALIDATION_LEGACY_BRIDGE_RELEASES="{COMMIT}"\nVALIDATION_EXPECT_RXC1="valid"\nREADY_TIMEOUT_SECONDS="45"\n')
        got = self.values(existing=existing)
        for key, want in (("ACTIVATION_ENABLED", "1"), ("VALIDATION_ENABLED", "0"), ("VALIDATION_MEMORY_MAX", "600M"), ("VALIDATION_TIMEOUT_SECONDS", "200"),
                          ("VALIDATION_LEGACY_BRIDGE_RELEASES", COMMIT), ("VALIDATION_EXPECT_RXC1", "valid"), ("READY_TIMEOUT_SECONDS", "45")):
            self.assertEqual(got[key], want, key)

    def test_an_invalid_setting_is_dropped_not_copied(self):
        for key, bad in (("VALIDATION_MEMORY_MAX", "lots"), ("VALIDATION_MEMORY_MAX", "0M"), ("VALIDATION_MEMORY_MAX", "500"), ("ACTIVATION_ENABLED", "2"),
                         ("VALIDATION_TIMEOUT_SECONDS", "601"), ("VALIDATION_EXPECT_RXC1", "sometimes"), ("READY_TIMEOUT_SECONDS", "301"),
                         ("VALIDATION_LEGACY_BRIDGE_RELEASES", "329f3109")):
            got, notes = self.merge(existing=f'{key}="{bad}"\n')
            self.assertEqual(got.get(key), {"ACTIVATION_ENABLED": "0", "VALIDATION_ENABLED": "1", "READY_TIMEOUT_SECONDS": "30"}.get(key), f"{key}={bad} was kept")
            self.assertTrue(any(f"invalid existing {key}" in n for n in notes), (key, bad, notes))

    def test_a_key_the_activator_does_not_know_is_dropped_and_reported(self):
        got, notes = self.merge(existing='BOGUS_KEY="1"\nVALIDATION_MEMORY_MAX="500M"\n')
        self.assertNotIn("BOGUS_KEY", got)
        self.assertTrue(any("dropped BOGUS_KEY" in n for n in notes), notes)
        self.assertEqual(got["VALIDATION_MEMORY_MAX"], "500M")

    def test_a_later_line_wins_as_in_the_activators_loader(self):
        self.assertEqual(self.values(existing='VALIDATION_MEMORY_MAX="400M"\nVALIDATION_MEMORY_MAX="500M"\n')["VALIDATION_MEMORY_MAX"], "500M")

    def test_quotes_are_handled_like_the_activators_loader(self):
        self.assertEqual(self.values(existing="VALIDATION_MEMORY_MAX='500M'\nACTIVATION_ENABLED=1\n")["VALIDATION_MEMORY_MAX"], "500M")

    def test_running_it_again_changes_nothing(self):
        existing = 'ACTIVATION_ENABLED="1"\nVALIDATION_MEMORY_MAX="500M"\n'
        self.merge(existing=existing)
        first = self.out.read_text()
        self.merge(existing=first)
        self.assertEqual(self.out.read_text(), first, "a second run is not identical to the first")
        self.merge(existing=first)
        self.assertEqual(self.out.read_text(), first)

    def test_the_file_is_written_atomically_with_the_requested_mode(self):
        self.merge(existing='VALIDATION_MEMORY_MAX="500M"\n')
        self.assertEqual(stat.S_IMODE(self.out.stat().st_mode), 0o644)
        self.assertEqual([p.name for p in self.path.iterdir() if p.name.startswith(".saturn-release-config.")], [])

    def test_a_failed_write_leaves_the_existing_file_untouched_and_no_temporary_file(self):
        self.out.write_text('ACTIVATION_ENABLED="1"\nVALIDATION_MEMORY_MAX="500M"\n')
        before = self.out.read_text()
        original = C.os.replace
        C.os.replace = lambda *a, **k: (_ for _ in ()).throw(OSError("disk full"))
        try:
            with self.assertRaises(OSError):
                self.merge(existing=before)
        finally:
            C.os.replace = original
        self.assertEqual(self.out.read_text(), before)
        self.assertEqual([p.name for p in self.path.iterdir() if p.name.startswith(".saturn-release-config.")], [])

    def test_a_symbolic_link_is_not_written_through(self):
        target = self.path / "target"
        target.write_text("keep\n")
        link = self.path / "link"
        link.symlink_to(target)
        with self.assertRaises(SystemExit):
            self.merge(out=link)
        self.assertEqual(target.read_text(), "keep\n")

    def test_comments_of_the_defaults_are_kept_and_nothing_else_is_invented(self):
        self.merge(existing='VALIDATION_MEMORY_MAX="500M"\n')
        text = self.out.read_text()
        self.assertTrue(text.startswith("# Managed by install_saturn_go_nginx.sh"))
        keys = [line.split("=", 1)[0] for line in text.splitlines() if "=" in line]
        self.assertEqual(len(keys), len(set(keys)), "a key is written twice")
        self.assertTrue(set(keys) <= C.KNOWN_KEYS, set(keys) - C.KNOWN_KEYS)


class ActivatorAcceptsIt(Rig):
    """The REAL root activator, in its own non-root test mode on temporary paths, loads the merged file: its policy gate answers from it, and it
    does not reject a key."""

    def run_activator(self, activation, managed="keep", existing=None):
        root = self.path / "world"
        for sub in ("deployments", "saturn/releases", "systemd", "state"):
            (root / sub).mkdir(parents=True, exist_ok=True)
        extra = ""
        defaults = defaults_text(activation=activation, root=str(root / "saturn")).replace("/var/lib/saturn-state/deployments", str(root / "deployments"))
        defaults = defaults.replace("/run/lock/saturn-release-activate.lock", str(root / "lock")).replace("/etc/systemd/system", str(root / "systemd"))
        defaults = defaults.replace('STATE_ROOT="/var/lib/saturn-state"', f'STATE_ROOT="{root / "state"}"')
        defaults = defaults.replace('TRANSACTION_GROUP="pi"', f'TRANSACTION_GROUP="{subprocess.check_output(["id", "-gn"], text=True).strip()}"')
        for name, rel in (("MANIFEST_TOOL", "update_manager/scripts/saturn-release-manifest.py"), ("STATE_TOOL", "update_manager/scripts/saturn-state-compatibility.py"),
                          ("COMPONENTS_FILE", "update_manager/release/components-v1.json")):
            defaults = "\n".join(f'{name}="{REPO / rel}"' if line.startswith(name + "=") else line for line in defaults.splitlines()) + "\n"
        defaults += 'BACKEND_STATUS_HELPER="/bin/true"\n'
        out = self.path / "merged"
        self.merge(defaults=defaults, existing=existing, managed=managed, out=out)
        env = dict(os.environ, SATURN_RELEASE_ACTIVATE_TEST_MODE="1", SATURN_RELEASE_ACTIVATE_CONFIG=str(out))
        result = subprocess.run(["bash", str(ACTIVATOR), "--reconcile-rollback"], env=env, capture_output=True, text=True, timeout=60)
        return result.stdout + result.stderr

    def test_the_activator_reads_the_merged_file_and_its_gate_follows_the_policy(self):
        kept = 'VALIDATION_MEMORY_MAX="500M"\nVALIDATION_TIMEOUT_SECONDS="200"\nVALIDATION_EXPECT_RXC1="valid"\n'
        off = self.run_activator("0", existing='ACTIVATION_ENABLED="0"\n' + kept)
        self.assertIn("reconciliation is disabled by root-owned policy", off, off)
        self.assertNotIn("unsupported activation config key", off)
        on = self.run_activator("0", existing='ACTIVATION_ENABLED="1"\n' + kept)
        self.assertIn("nothing to reconcile", on, on)
        self.assertNotIn("disabled by root-owned policy", on)

    def test_an_explicit_choice_reaches_the_activator(self):
        self.assertIn("nothing to reconcile", self.run_activator("0", managed="enable", existing='ACTIVATION_ENABLED="0"\n'))
        self.assertIn("disabled by root-owned policy", self.run_activator("1", managed="disable", existing='ACTIVATION_ENABLED="1"\n'))


class InstallerUsesIt(unittest.TestCase):
    text = INSTALLER.read_text()

    def test_the_installer_no_longer_overwrites_the_file_from_fixed_text(self):
        self.assertNotIn('cat >"$SATURN_RELEASE_ACTIVATE_CONFIG"', self.text)
        self.assertIn('saturn-release-config.py', self.text)
        self.assertIn('merge --defaults "$release_activate_defaults"', self.text)
        self.assertIn('--existing "$SATURN_RELEASE_ACTIVATE_CONFIG"', self.text)
        self.assertIn('--managed-updates "$SATURN_MANAGED_UPDATES"', self.text)

    def test_the_default_is_to_keep_and_an_explicit_choice_is_validated(self):
        self.assertIn('SATURN_MANAGED_UPDATES=keep', self.text)
        self.assertIn('SATURN_MANAGED_UPDATES must be keep, enable or disable', self.text)

    def test_every_installer_default_key_is_one_the_activator_knows(self):
        block = self.text[self.text.index('cat >"$release_activate_defaults" <<EOF'):]
        block = block[:block.index("\nEOF\n")]
        keys = {line.split("=", 1)[0] for line in block.splitlines()[1:] if "=" in line and not line.startswith("#")}
        self.assertTrue(keys <= C.KNOWN_KEYS, keys - C.KNOWN_KEYS)
        activator = ACTIVATOR.read_text()
        for key in C.KNOWN_KEYS:
            self.assertIn(f"{key}) {key}=", activator, f"{key} is not accepted by the activator's loader")


if __name__ == "__main__":
    unittest.main(verbosity=2)
