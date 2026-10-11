#!/usr/bin/env python3
"""The installer writes /etc/default/saturn-release-activate through saturn-release-config.py. What an administrator chose (the activation policy,
the browser memory allowance) must survive running the installer again; what the installer owns must be refreshed; and the result must be a file
the REAL root activator accepts."""
from __future__ import annotations

import importlib.util
import os
import re
import shutil
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

    def test_how_the_validation_reaches_its_browser_survives_the_installer(self):
        existing = ('VALIDATION_URL="https://127.0.0.1:9443/remote-next?validation=receive&transport=split"\nVALIDATION_TLS_CERT="/etc/saturn/remote.crt"\n'
                    'VALIDATION_BROWSER="/usr/bin/chromium-browser"\nVALIDATION_LAUNCHER="systemd"\nVALIDATION_PERF_JSON="/run/other/perf.json"\n'
                    'VALIDATION_LOG_ROOT="/var/log/saturn-validation"\n')
        got, notes = self.merge(existing=existing)
        for key, want in (("VALIDATION_URL", "https://127.0.0.1:9443/remote-next?validation=receive&transport=split"), ("VALIDATION_TLS_CERT", "/etc/saturn/remote.crt"),
                          ("VALIDATION_BROWSER", "/usr/bin/chromium-browser"), ("VALIDATION_LAUNCHER", "systemd"), ("VALIDATION_PERF_JSON", "/run/other/perf.json"),
                          ("VALIDATION_LOG_ROOT", "/var/log/saturn-validation")):
            self.assertEqual(got.get(key), want, f"{key} was lost by the installer")
        self.assertEqual([n for n in notes if "invalid" in n or "dropped" in n], [])

    def test_the_direct_launcher_is_only_kept_for_a_caller_that_is_not_root(self):
        original = C.os.geteuid
        try:
            C.os.geteuid = lambda: 0
            got, notes = self.merge(existing='VALIDATION_LAUNCHER="direct"\n')
            self.assertNotIn("VALIDATION_LAUNCHER", got, "the activator refuses the direct launcher as root, so it must not be written")
            self.assertTrue(any("invalid existing VALIDATION_LAUNCHER" in n for n in notes), notes)
            C.os.geteuid = lambda: 1000
            self.assertEqual(self.values(existing='VALIDATION_LAUNCHER="direct"\n')["VALIDATION_LAUNCHER"], "direct")
        finally:
            C.os.geteuid = original

    def test_an_invalid_override_of_those_is_reported_and_dropped(self):
        for key, bad in (("VALIDATION_URL", "http://127.0.0.1:8443/x"), ("VALIDATION_URL", "https://example.com:8443/x"), ("VALIDATION_URL", "https://127.0.0.1:8443/x y"),
                         ("VALIDATION_LAUNCHER", "docker"), ("VALIDATION_BROWSER", "chromium"), ("VALIDATION_TLS_CERT", "/etc/with space/c.crt"),
                         ("VALIDATION_PERF_JSON", "run/perf.json"), ("VALIDATION_LOG_ROOT", "")):
            got, notes = self.merge(existing=f'{key}="{bad}"\n')
            self.assertNotIn(key, got, f"{key}={bad!r} was kept")
            self.assertTrue(any(f"invalid existing {key}" in n for n in notes), (key, bad, notes))

    def test_every_key_the_activator_accepts_is_the_installers_or_the_administrators(self):
        block = INSTALLER.read_text()
        block = block[block.index('cat >"$release_activate_defaults" <<EOF'):]
        block = block[:block.index("\nEOF\n")]
        installer_keys = {line.split("=", 1)[0] for line in block.splitlines()[1:] if "=" in line and not line.startswith("#")}
        self.assertEqual(C.KNOWN_KEYS - installer_keys - set(C.ADMIN_KEYS), set(), "a known key would be lost silently on every installer run")

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


class SudoersTests(Rig):
    broker = "/usr/local/lib/saturn-go/scripts/saturn-release-update-root.py"

    def fragment(self, activation):
        config = self.path / "written"
        config.write_text(f'ACTIVATION_ENABLED="{activation}"\n')
        return C.sudoers_fragment("pi", self.broker, str(config))

    def test_enabled_managed_updates_authorize_exactly_the_broker_modes_the_page_needs(self):
        lines = self.fragment("1").splitlines()
        self.assertEqual(lines, [f"pi ALL=(root) NOPASSWD: {self.broker} --preflight", f"pi ALL=(root) NOPASSWD: {self.broker} --preflight --allow-legacy-migration",
                                 f"pi ALL=(root) NOPASSWD: {self.broker} --dispatch *"])
        self.assertTrue(all("--run" not in line for line in lines), "the service user must not be able to call --run directly")

    def test_disabled_or_unset_managed_updates_authorize_nothing(self):
        self.assertEqual(self.fragment("0"), "")
        self.assertEqual(C.sudoers_fragment("pi", self.broker, str(self.path / "does-not-exist")), "")

    def test_turning_managed_updates_off_removes_the_authorization_on_the_next_installer_run(self):
        self.merge(existing='ACTIVATION_ENABLED="1"\n', managed="keep")
        self.assertNotEqual(C.sudoers_fragment("pi", self.broker, str(self.out)), "")
        self.merge(existing=self.out.read_text(), managed="disable")
        self.assertEqual(C.sudoers_fragment("pi", self.broker, str(self.out)), "")

    def test_an_unusual_user_or_broker_path_is_refused(self):
        config = self.path / "written"
        config.write_text('ACTIVATION_ENABLED="1"\n')
        for user, broker in (("pi extra", self.broker), ("pi", "relative/path.py"), ("pi", "/usr/bin/../../tmp/x"), ("pi", "/path with space/x.py"), ("Pi", self.broker),
                             ("pi", "/usr/local/*/x.py"), ("pi,root", self.broker), ("pi", "/x\ny")):
            with self.assertRaises(SystemExit, msg=(user, broker)):
                C.sudoers_fragment(user, broker, str(config))

    def test_the_command_line_prints_the_fragment_and_the_settings(self):
        config = self.path / "written"
        config.write_text('ACTIVATION_ENABLED="1"\nVALIDATION_MEMORY_MAX="500M"\n')
        out = subprocess.run([sys.executable, str(TOOL), "sudoers", "--user", "pi", "--broker", self.broker, "--config", str(config)], capture_output=True, text=True)
        self.assertEqual((out.returncode, out.stdout.count("NOPASSWD")), (0, 3), out.stderr)
        got = subprocess.run([sys.executable, str(TOOL), "get", "--file", str(config), "--key", "VALIDATION_MEMORY_MAX"], capture_output=True, text=True)
        self.assertEqual(got.stdout.strip(), "500M")

    @unittest.skipUnless(shutil.which("visudo"), "visudo is not installed")
    def test_the_fragment_is_valid_sudoers_syntax(self):
        sudoers = self.path / "sudoers"
        sudoers.write_text("# test\n" + self.fragment("1"))
        sudoers.chmod(0o440)
        result = subprocess.run(["visudo", "-cf", str(sudoers)], capture_output=True, text=True)
        if "you do not exist" in result.stderr or "permission denied" in result.stderr.lower():
            self.skipTest("visudo cannot check as this user")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class LayoutTests(Rig):
    def test_a_pointer_into_the_releases_root_is_a_release_layout(self):
        releases = self.path / "releases"
        (releases / COMMIT).mkdir(parents=True)
        os.symlink(releases / COMMIT, self.path / "current")
        self.assertEqual(C.detect_layout(str(self.path / "current"), str(releases)), ("release", COMMIT))

    def test_anything_else_is_a_legacy_layout(self):
        releases = self.path / "releases"
        (releases / COMMIT).mkdir(parents=True)
        other = self.path / "elsewhere" / COMMIT
        other.mkdir(parents=True)
        (self.path / "plain").mkdir()
        os.symlink(other, self.path / "outside")
        os.symlink(releases, self.path / "root-itself")
        (releases / "cc09c61").mkdir()
        os.symlink(releases / "cc09c61", self.path / "short-name")
        for name in ("missing", "plain", "outside", "root-itself", "short-name"):
            self.assertEqual(C.detect_layout(str(self.path / name), str(releases)), ("legacy", None), name)
        # A real directory, even one that is a release, is not the `current` link: activation switches a symbolic link, so a layout without one is not
        # a release layout (the link test cannot be left to the "inside the release directory" test, which this directory passes).
        self.assertEqual(C.detect_layout(str(releases / COMMIT), str(releases)), ("legacy", None), "a real release directory used as the link")

    def test_the_application_rule(self):
        self.assertTrue(C.skip_application("release", "auto"))
        self.assertFalse(C.skip_application("legacy", "auto"))
        self.assertFalse(C.skip_application("release", "force"))
        self.assertTrue(C.skip_application("legacy", "skip"))
        self.assertTrue(C.skip_application("release", "skip"))
        self.assertFalse(C.skip_application("legacy", "force"))


@unittest.skipIf(os.geteuid() == 0, "the plan is checked as an ordinary user, like an administrator looking before sudo")
class InstallerPlan(Rig):
    """The REAL installer in its plan mode: it says what it would do on this layout, and changes nothing."""

    def plan(self, layout="release", config='ACTIVATION_ENABLED="0"\nVALIDATION_MEMORY_MAX="500M"\n', **extra):
        releases = self.path / "releases"
        (releases / COMMIT).mkdir(parents=True, exist_ok=True)
        link = self.path / "current"
        if layout == "release" and not link.exists():
            os.symlink(releases / COMMIT, link)
        (self.path / "conf").write_text(config)
        env = dict(os.environ, SATURN_INSTALL_PLAN="1", SATURN_SERVICE_USER=subprocess.check_output(["id", "-un"], text=True).strip(),
                   SATURN_RELEASE_CURRENT_LINK=str(link), SATURN_RELEASES_ROOT=str(releases), SATURN_RELEASE_ACTIVATE_CONFIG=str(self.path / "conf"))
        env.update(extra)
        before = (self.path / "conf").read_text()
        result = subprocess.run(["bash", str(INSTALLER)], capture_output=True, text=True, env=env, timeout=60)
        self.assertEqual((self.path / "conf").read_text(), before, "the plan changed the configuration")
        return result

    def test_a_release_layout_leaves_the_application_alone(self):
        result = self.plan()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"application layout: release (active release {COMMIT})", result.stdout)
        self.assertIn("application payload: NOT deployed", result.stdout)
        self.assertIn("no service restart", result.stdout)
        self.assertIn(f"readiness check after the run: the running release {COMMIT}", result.stdout)
        self.assertIn("administrator settings kept: VALIDATION_MEMORY_MAX=500M", result.stdout)
        self.assertIn("ACTIVATION_ENABLED): 0 -> 0 (SATURN_MANAGED_UPDATES=keep)", result.stdout)

    def test_the_policy_changes_only_on_an_explicit_choice(self):
        self.assertIn("0 -> 1 (SATURN_MANAGED_UPDATES=enable)", self.plan(SATURN_MANAGED_UPDATES="enable").stdout)
        self.assertIn("1 -> 0 (SATURN_MANAGED_UPDATES=disable)", self.plan(config='ACTIVATION_ENABLED="1"\n', SATURN_MANAGED_UPDATES="disable").stdout)
        self.assertIn("1 -> 1 (SATURN_MANAGED_UPDATES=keep)", self.plan(config='ACTIVATION_ENABLED="1"\n').stdout)

    def test_a_legacy_layout_still_deploys_and_points_at_the_explicit_migration(self):
        result = self.plan(layout="legacy", SATURN_MANAGED_UPDATES="enable")
        self.assertIn("application layout: legacy", result.stdout)
        self.assertIn("application payload: deployed", result.stdout)
        self.assertIn("the commit of this checkout", result.stdout)
        self.assertIn("--migrate-to-releases", result.stdout)

    def test_forcing_the_application_on_a_release_layout_is_warned_about(self):
        result = self.plan(SATURN_INSTALL_APPLICATION="force")
        self.assertIn("application payload: deployed", result.stdout)
        self.assertIn("WARNING", result.stdout)

    def test_skipping_the_application_is_honoured_anywhere(self):
        result = self.plan(layout="legacy", SATURN_INSTALL_APPLICATION="skip")
        self.assertIn("NOT deployed (SATURN_INSTALL_APPLICATION=skip)", result.stdout)

    def test_an_unknown_choice_is_refused(self):
        for key, value in (("SATURN_INSTALL_APPLICATION", "maybe"), ("SATURN_MANAGED_UPDATES", "maybe")):
            result = self.plan(**{key: value})
            self.assertNotEqual(result.returncode, 0, key)
            self.assertIn(f"{key} must be", result.stderr)

    def test_without_the_plan_mode_it_still_needs_root(self):
        result = self.plan(SATURN_INSTALL_PLAN="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Run as root", result.stderr)


class ApplianceInstallerGuard(Rig):
    """The whole-appliance installer (install.sh) on an appliance that runs releases: it refuses before it changes anything, and says what to run."""
    WRAPPER = REPO / "scripts/install-saturn-appliance.sh"

    def run_wrapper(self, layout="release", *args, **extra):
        releases = self.path / "releases"
        (releases / COMMIT).mkdir(parents=True, exist_ok=True)
        link = self.path / "current"
        if layout == "release" and not link.exists():
            os.symlink(releases / COMMIT, link)
        env = dict(os.environ, SATURN_USER=subprocess.check_output(["id", "-un"], text=True).strip(),
                   SATURN_RELEASE_CURRENT_LINK=str(link), SATURN_RELEASES_ROOT=str(releases))
        env.pop("SATURN_INSTALL_APPLICATION", None)
        env.update(extra)
        return subprocess.run(["bash", str(self.WRAPPER), "--dry-run", *args], capture_output=True, text=True, env=env, timeout=60)

    def test_a_release_layout_is_refused_before_anything_else_happens(self):
        result = self.run_wrapper()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"this appliance runs managed releases (active release {COMMIT})", result.stderr)
        self.assertIn("Nothing was changed", result.stderr)
        self.assertIn("update_manager/install_saturn_go_nginx.sh", result.stderr, "it must name the command to run instead")
        self.assertIn("SATURN_INSTALL_APPLICATION=force", result.stderr)
        # refused FIRST: none of the later checks or the provisioner hand-over was reached
        for later in ("install source:", "runtime user:", "would execute shared provisioner", "invalid Saturn checkout", "user does not exist"):
            self.assertNotIn(later, result.stdout + result.stderr)

    def test_the_explicit_skip_choice_does_not_make_it_safe(self):
        self.assertEqual(self.run_wrapper(SATURN_INSTALL_APPLICATION="skip").returncode, 1)

    def test_force_goes_ahead_and_says_what_that_means(self):
        result = self.run_wrapper(SATURN_INSTALL_APPLICATION="force")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("would execute shared provisioner", result.stdout)
        self.assertIn("SATURN_INSTALL_APPLICATION=force on an appliance that runs managed releases", result.stderr)
        self.assertIn("--skip-saturn-go", result.stderr)

    def test_a_legacy_layout_or_a_fresh_machine_is_not_affected(self):
        """No release link at all is both a fresh machine and the legacy layout."""
        result = self.run_wrapper("legacy")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("would execute shared provisioner", result.stdout)
        self.assertNotIn("managed releases", result.stderr)

    def test_an_unknown_choice_is_refused_even_on_a_legacy_layout(self):
        result = self.run_wrapper("legacy", SATURN_INSTALL_APPLICATION="maybe")
        self.assertEqual(result.returncode, 1)
        self.assertIn("SATURN_INSTALL_APPLICATION must be auto, force or skip", result.stderr)

    def test_when_the_layout_cannot_be_determined_it_refuses_rather_than_assuming_a_fresh_machine(self):
        """Fail closed: a broken layout tool must not turn into 'nothing to protect'."""
        tree = self.path / "checkout"
        tool = tree / "update_manager/scripts/saturn-release-config.py"
        tool.parent.mkdir(parents=True)
        tool.write_text("import sys\nsys.exit(2)\n")
        result = self.run_wrapper("legacy", SATURN_REPO_DIR=str(tree))
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("could not determine whether this appliance runs managed releases", result.stderr)
        self.assertNotIn("invalid Saturn checkout", result.stderr, "the layout question must be settled before any later check")

    def test_help_still_works_on_a_release_layout(self):
        releases = self.path / "releases"
        (releases / COMMIT).mkdir(parents=True)
        os.symlink(releases / COMMIT, self.path / "current")
        env = dict(os.environ, SATURN_RELEASE_CURRENT_LINK=str(self.path / "current"), SATURN_RELEASES_ROOT=str(releases))
        result = subprocess.run(["bash", str(self.WRAPPER), "--help"], capture_output=True, text=True, env=env, timeout=30)
        self.assertEqual(result.returncode, 0)
        self.assertIn("Usage: sudo ./install.sh", result.stdout)

    def test_the_guard_runs_before_every_phase_in_the_script(self):
        text = self.WRAPPER.read_text()
        call = text.index("\nrefuse_on_release_layout\n")
        for later in ("check_source_checkout\n", 'getent passwd "$SATURN_USER"', "(( EUID == 0 ))", 'exec bash "$PROVISIONER"'):
            self.assertLess(call, text.index(later, call), later)
        self.assertIn("update_manager/scripts/saturn-release-config.py", text, "one layout rule for both installers")


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

    def test_the_installer_installs_the_broker_and_derives_the_sudoers_rule_from_the_written_policy(self):
        self.assertIn('"$SOURCE_DIR/scripts/$SATURN_RELEASE_UPDATE_BROKER_NAME"', self.text)
        self.assertIn('sudoers --user "$SERVICE_USER"', self.text)
        self.assertIn('--config "$SATURN_RELEASE_ACTIVATE_CONFIG"', self.text)
        self.assertIn("${managed_updates_sudoers}", self.text)
        self.assertIn('cat >"$SUDOERS_FILE" <<EOF', self.text, "the sudoers file must be written from scratch so a disabled policy removes the rule")
        self.assertLess(self.text.index('merge --defaults "$release_activate_defaults"'), self.text.index('sudoers --user "$SERVICE_USER"'),
                        "the rule must be derived from the policy as written, so the policy is written first")

    def test_on_a_release_layout_nothing_of_the_application_is_built_copied_or_restarted(self):
        text = self.text
        gate = "if (( SKIP_APPLICATION )); then\n"
        # Each application step is the ELSE branch of exactly this test (not a negated or loosened one), so a release layout does not run it.
        for skipped, marker in (("  info \"Web assets are part of the application; left as they are", 'else\ninfo "Copying web assets..."'),
                                ("  info \"Rust server: not built or installed", 'else\ninfo "Building Rust server..."')):
            self.assertRegex(text, re.escape(gate + skipped) + r"[^\n]*\n(?:  [^\n]*\n)*" + re.escape(marker), marker)
        restart_block = text[text.index("systemctl enable saturn-go-watchdog.timer"):text.index("Service and watchdog enabled")]
        self.assertRegex(restart_block, re.compile(re.escape(gate) + r"  # The unit files are refreshed.*?\nelif systemctl is-active --quiet saturn-go\.service; then\n  systemctl restart saturn-go\.service\n", re.S))
        self.assertNotRegex(restart_block.split("elif")[0], r"systemctl restart saturn-go")
        self.assertIn('RUST_BUILD_COMMIT="$ACTIVE_RELEASE_COMMIT"', text, "readiness must be checked against the running release")
        self.assertIn('SATURN_BRIDGE_UNIT_ONLY="$SKIP_APPLICATION"', text)
        self.assertEqual(text.count(gate), 7, "an application gate was added or lost without this test being told")

    def test_what_the_installer_says_for_each_layout_and_choice_before_it_changes_anything(self):
        """The announcement block of the real installer, run with stand-ins for info and warn: plan mode exits before it, so it is checked here."""
        start = self.text.index("if (( SKIP_APPLICATION )); then\n  if [[ \"$SATURN_APP_LAYOUT\" == \"release\" ]]; then\n    info \"Release layout")
        end = self.text.index("\nfi\n", start) + len("\nfi\n")
        block = self.text[start:end]
        script = 'info(){ echo "INFO $*"; }\nwarn(){ echo "WARN $*"; }\nACTIVE_RELEASE_COMMIT=' + COMMIT + '\nSATURN_ROOT=/opt/saturn-go\n' + block
        cases = (  # (SKIP_APPLICATION, layout) -> the line that is said
            (1, "release", f"INFO Release layout (active release {COMMIT}): the application is managed by releases"),
            (1, "legacy", "INFO SATURN_INSTALL_APPLICATION=skip: the application payload is not deployed."),
            (0, "release", "WARN SATURN_INSTALL_APPLICATION=force on a release layout"),
            (0, "legacy", ""),
        )
        for skip, layout, expected in cases:
            result = subprocess.run(["bash", "-c", f"SKIP_APPLICATION={skip}; SATURN_APP_LAYOUT={layout}\n{script}"], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            if expected:
                self.assertTrue(result.stdout.startswith(expected), (skip, layout, result.stdout))
            else:
                self.assertEqual(result.stdout, "", (skip, layout))

    def test_the_bridge_installer_has_a_mode_that_touches_neither_the_binary_nor_the_service(self):
        bridge = (REPO / "update_manager/scripts/install-saturn-bridge.sh").read_text()
        self.assertIn('SATURN_BRIDGE_UNIT_ONLY="${SATURN_BRIDGE_UNIT_ONLY:-0}"', bridge)
        unit_mode = bridge[bridge.index('  if flag_enabled "$SATURN_BRIDGE_UNIT_ONLY"; then\n    capture_selected_backend'):]
        unit_mode = unit_mode[:unit_mode.index("return 0")]
        for forbidden in ("build_bridge", "install_binary", "ensure_apt_packages", "systemctl restart", "BACKEND_SWITCH_HELPER"):
            self.assertNotIn(forbidden, unit_mode)
        service = bridge[bridge.index("install_service() {"):bridge.index("require_service_limit() {")]
        before_switch = service[:service.index("The backend transaction owns")]
        self.assertIn('flag_enabled "$SATURN_BRIDGE_UNIT_ONLY"', before_switch, "unit-only must return before the backend switch and the restart")

    def test_a_legacy_layout_is_pointed_at_the_explicit_migration(self):
        self.assertIn("--migrate-to-releases", self.text)

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
