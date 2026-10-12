#!/usr/bin/env python3
"""The Update action of the Saturn Go page with managed updates on: update-saturn-go.sh resolves the selected ref to one commit, builds that
commit into a bundle, and hands it to the REAL update broker, which runs the (stand-in) root installer and activator. What the page's script
refuses, what it builds, and what the status file says at each end are checked here against the real script and the real broker; only the
build itself and the root installer/activator are stand-ins."""
from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCRIPT = REPO / "update_manager/scripts/update-saturn-go.sh"

spec = importlib.util.spec_from_file_location("broker_tests", REPO / "tests/test-saturn-release-update.py")
B = importlib.util.module_from_spec(spec)
sys.modules["broker_tests"] = B
spec.loader.exec_module(B)

FAKE_BUILDER = r'''#!/usr/bin/env bash
echo "builder $*" >>"$FAKE_CALLS"
echo "builder-env cargo=$(command -v cargo || echo MISSING) cargo_home=${CARGO_HOME:-} rustup_home=${RUSTUP_HOME:-}" >>"$FAKE_CALLS"
echo "builder-inherited remote_auth=${SATURN_REMOTE_BASIC_AUTH-unset} addr=${SATURN_ADDR-unset} state_dir=${SATURN_STATE_DIR-unset} webroot=${SATURN_WEBROOT-unset} jobs=${SATURN_RELEASE_BUILD_JOBS-unset} swap=${SATURN_SATURNGO_BUILD_SWAP_MIB-unset} out=${SATURN_RELEASE_OUTPUT_ROOT-unset} repo=${SATURN_RELEASE_REPO_ROOT-unset} prompt=${GIT_TERMINAL_PROMPT-unset} home=${HOME-unset} lang=${LANG-unset} pkg=${PKG_CONFIG_PATH-unset}" >>"$FAKE_CALLS"
remote="" ref="" resolve=0 expect=""
while [ $# -gt 0 ]; do case "$1" in --source-remote) remote=$2; shift 2;; --source-ref) ref=$2; shift 2;; --resolve-only) resolve=1; shift;; --expect-commit) expect=$2; shift 2;; --output-root) shift 2;; *) shift;; esac; done
if [ -n "$remote" ]; then
  commit=$(git ls-remote "$remote" "refs/heads/$ref" | cut -f1)
  [ -n "$commit" ] || { echo "ref not found" >&2; exit 1; }
else
  commit=$(git -C "$SATURN_RELEASE_REPO_ROOT" rev-parse HEAD)
fi
if [ -n "$expect" ] && [ "$commit" != "$expect" ]; then
  echo "source ref moved since it was resolved: expected $expect, but $ref now resolves to $commit; nothing was built" >&2
  exit 1
fi
if [ "$resolve" = 1 ]; then
  printf 'source_remote=%s\nrequested_ref=%s\nresolved_ref=refs/heads/%s\nresolved_commit=%s\n' "$remote" "$ref" "$ref" "${FAKE_RESOLVED_COMMIT-$commit}"
  if [ -n "${FAKE_MOVE_AFTER_RESOLVE:-}" ]; then      # somebody pushes to the branch while the flow is between resolving and building
    git -C "$FAKE_WORK" commit -q --allow-empty -m moved && git -C "$FAKE_WORK" push -q "$remote" main
  fi
  exit 0
fi
[ "${FAKE_BUILD_RC:-0}" = 0 ] || { echo "compile error: boom" >&2; exit "$FAKE_BUILD_RC"; }
out="$SATURN_RELEASE_OUTPUT_ROOT/$commit"
mkdir -p "$out/scripts"
printf '{"source":{"commit":"%s","dirty":false,"repository":"%s","requested_ref":"%s"}}' "$commit" "$remote" "$ref" >"$out/release-manifest.json"
for h in saturn-release-activate-root.sh saturn-remote-validate.py saturn-remote-receive-guard.js; do printf '# saturn-helper-contract: %s\n' "${FAKE_BUNDLE_CONTRACT:-1}" >"$out/scripts/$h"; done
'''

RUN_THROUGH = r'''#!/usr/bin/env bash
# stands in for systemd-run: runs the unit's command at once, in the foreground, like the transient unit would
echo "systemd-run $*" >>"$FAKE_CALLS"
while [ $# -gt 0 ]; do case "$1" in --unit) shift 2;; --collect|--no-block) shift;; *) break;; esac; done
INVOCATION_ID=test "$@"
exit 0      # --no-block: the unit's own exit status is not systemd-run's
'''


def git(*args, cwd):
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True,
                   env=dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t"))


class Flow(B.Rig):
    def setUp(self):
        super().setUp()
        self.remote = self.root / "remote.git"
        work = self.root / "work"
        work.mkdir()
        git("init", "-q", "-b", "main", cwd=work)
        (work / "a").write_text("1")
        git("add", "a", cwd=work)
        git("commit", "-q", "-m", "one", cwd=work)
        git("init", "-q", "--bare", str(self.remote), cwd=self.root)
        git("push", "-q", str(self.remote), "main", cwd=work)
        self.work = work
        self.head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=work, text=True).strip()
        self.checkout = self.root / "checkout"
        git("clone", "-q", "-b", "main", str(self.remote), str(self.checkout), cwd=self.root)
        self.builder = self.root / "builder.sh"
        self.builder.write_text(FAKE_BUILDER)
        self.builder.chmod(0o755)
        self.fake_systemd_run.write_text(RUN_THROUGH)

    def script(self, *flags, ref="main", at=SCRIPT, **env):
        environment = self.env(
            SATURN_RELEASE_ACTIVATE_CONFIG=str(self.root / "activate.conf"), SATURN_RELEASE_UPDATE_BROKER=str(B.BROKER), SATURN_RELEASE_SUDO="env",
            SATURN_RELEASE_STAGING_DIR=str(self.staging), SATURN_RELEASE_BUILDER=str(self.builder), SATURN_RELEASE_MANIFEST_TOOL=str(self.root / "no-such-tool"),
            SATURN_SATURNGO_DEPLOY_STATUS_FILE=str(self.status_file), SATURN_ACTIVE_REPO_ROOT=str(self.checkout), SATURN_SATURNGO_POLICY_URL=str(self.remote),
            SATURN_SATURNGO_POLICY_REF=ref, SATURN_SATURNGO_POLICY_REMOTE="origin", SATURN_RELEASE_REPO_ROOT=str(self.checkout),
            SATURN_RELEASE_BUILD_ENV_EXTRA="FAKE_[A-Z_]+|GIT_[A-Z_]+", FAKE_WORK=str(self.work), GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        environment.update(env)
        for name in [name for name, value in environment.items() if value is None]:      # None unsets
            del environment[name]
        return subprocess.run(["bash", str(at), *flags], capture_output=True, text=True, env=environment, timeout=180)

    def builds(self):
        return [line for line in self.calls_text().splitlines() if line.startswith("builder ") and "--resolve-only" not in line]


PAGE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"     # what the page gives every script it runs (rust-server apply_build_subprocess_env)


class UnderThePagesEnvironment(Flow):
    """The page starts scripts with a fixed system-only PATH: cargo and rustup, in the build user's ~/.cargo/bin, are not on it."""

    def home(self, with_cargo=True):
        home = self.root / "home"
        (home / ".cargo/bin").mkdir(parents=True, exist_ok=True)
        if with_cargo:
            cargo = home / ".cargo/bin/cargo"
            cargo.write_text("#!/bin/sh\necho cargo\n")
            cargo.chmod(0o755)
        return home

    def run_as_the_page(self, home, *flags, **env):
        # the page's variables only (nothing of a developer's PATH), HOME of the build user
        # (PYTHONHOME/PYTHONPATH emptied: the page has none, and a developer's Python distribution would leak one into the system python3)
        return self.script(*flags, PATH=PAGE_PATH, HOME=str(home), CARGO_HOME="", RUSTUP_HOME="", PYTHONHOME="", PYTHONPATH="", **env)

    def test_the_toolchain_is_found_and_handed_to_the_builder(self):
        home = self.home()
        result = self.run_as_the_page(home)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        envs = [line for line in self.calls_text().splitlines() if line.startswith("builder-env")]
        self.assertTrue(envs, "the builder was never run")
        for line in envs:
            self.assertIn(f"cargo={home}/.cargo/bin/cargo", line)
            self.assertIn(f"cargo_home={home}/.cargo", line)
            self.assertIn(f"rustup_home={home}/.rustup", line)
        self.assertEqual(self.active(), self.head)

    def test_without_a_toolchain_the_update_stops_first_and_says_so(self):
        # a build user whose home has no ~/.cargo (the resolver also tries /home/<build user>/.cargo, which a developer's machine may have)
        result = self.run_as_the_page(self.home(with_cargo=False), SATURN_SATURNGO_BUILD_USER="daemon")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cargo", result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertNotIn("builder", self.calls_text(), "nothing may be resolved or built without a toolchain")
        self.assertEqual(self.active(), B.OLD)
        status = self.status()
        self.assertEqual((status["status"], status["mode"]), ("error", "release"))
        self.assertIn("cargo", status["message"])

    def test_a_build_user_without_a_home_is_reported_in_the_status_not_only_in_the_output(self):
        result = self.run_as_the_page(self.home(), SATURN_SATURNGO_BUILD_USER="no-such-saturn-user")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.builds(), [])
        self.assertIn("home directory of the build user 'no-such-saturn-user'", self.status()["message"])

    def test_a_refusal_does_not_depend_on_the_toolchain(self):
        self.set_config(enabled="0")
        result = self.run_as_the_page(self.home(with_cargo=False), SATURN_SATURNGO_BUILD_USER="daemon")
        self.assertNotEqual(result.returncode, 0)
        status = self.status()
        self.assertEqual(status["phase"], "managed_updates_disabled")
        self.assertIn("managed updates are disabled", status["message"])


SERVICE_ENVIRONMENT = dict(     # what the saturn-go service's own environment holds (names as on the G2); the credential here is a made-up one
    SATURN_REMOTE_BASIC_AUTH="user:made-up-secret", SATURN_ADDR="127.0.0.1:8080", SATURN_STATE_DIR="/var/lib/saturn-state",
    SATURN_WEBROOT="/opt/saturn/current/webroot", SATURN_CONFIG="/var/lib/saturn-web/config.json", SATURN_REPO_ROOT="/elsewhere")


class TheBuildsEnvironment(Flow):
    """A release is built in a clean environment: no service configuration, no credential; the toolchain and the build knobs still arrive."""

    def inherited(self):
        return [line for line in self.calls_text().splitlines() if line.startswith("builder-inherited")]

    def test_the_services_configuration_and_credential_never_reach_the_builder(self):
        result = self.script(**SERVICE_ENVIRONMENT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = self.inherited()
        self.assertEqual(len(lines), 2, "the resolve and the build both run the builder")
        for line in lines:
            for leaked in ("made-up-secret", "127.0.0.1:8080", "/var/lib/saturn-state", "/opt/saturn/current/webroot", "/elsewhere"):
                self.assertNotIn(leaked, line)
            self.assertIn("remote_auth=unset addr=unset state_dir=unset webroot=unset", line)
        self.assertNotIn("made-up-secret", result.stdout + result.stderr + self.calls_text())

    def test_what_a_build_needs_still_arrives(self):
        result = self.script(SATURN_RELEASE_BUILD_JOBS="2", SATURN_SATURNGO_BUILD_SWAP_MIB="1024", LANG="en_GB.UTF-8", PKG_CONFIG_PATH="/x/pkgconfig", **SERVICE_ENVIRONMENT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        resolve, build = self.inherited()
        for line in (resolve, build):
            self.assertIn("jobs=2 swap=1024", line)
            self.assertIn(f"repo={self.checkout}", line)
            self.assertIn("prompt=0", line)
            self.assertIn("lang=en_GB.UTF-8 pkg=/x/pkgconfig", line)
            self.assertNotIn("home=unset", line)
        self.assertIn(f"out={self.staging}", build)

    def test_a_utf8_locale_is_the_default_when_the_service_has_none(self):
        result = self.script(LANG=None, LC_ALL=None, LC_CTYPE=None)       # the saturn-go service has no LANG at all
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for line in self.inherited():
            self.assertIn("lang=C.UTF-8", line)

    def test_the_build_settings_are_the_flows_not_the_callers(self):
        # a caller's own value for a variable the flow sets is replaced, not trusted
        result = self.script(SATURN_RELEASE_OUTPUT_ROOT="/somewhere/else", GIT_TERMINAL_PROMPT="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        resolve, build = self.inherited()
        self.assertIn(f"out={self.staging}", build)
        self.assertIn("prompt=0", resolve + build)
        self.assertNotIn("/somewhere/else", build)


class HappyPath(Flow):
    def test_the_update_action_builds_the_resolved_commit_and_the_broker_installs_it(self):
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"resolves to commit {self.head}", result.stdout)
        self.assertEqual(len(self.builds()), 1)
        self.assertIn(f"--source-remote {self.remote} --source-ref main", self.builds()[0])
        self.assertTrue((self.staging / self.head / "release-manifest.json").is_file())
        calls = self.calls_text()
        self.assertIn("systemd-run", calls)
        self.assertRegex(calls, rf"activator {self.head}")
        status = self.status()
        self.assertEqual((status["status"], status["phase"], status["mode"]), ("success", "done", "release"))
        self.assertEqual(status["release"]["commit"], self.head)
        self.assertEqual(status["release"]["previous_commit"], B.OLD)
        self.assertEqual(self.active(), self.head)

    def test_the_builder_is_the_installer_managed_one_beside_the_updater_not_the_checkouts(self):
        """On the appliance the updater runs from the installer-managed scripts directory while the working checkout can be many commits old."""
        scripts = self.root / "installed-scripts"
        shutil.copytree(SCRIPT.parent, scripts)
        for directory, tag in ((scripts, "installed"), (self.checkout / "update_manager/scripts", "checkout")):
            directory.mkdir(parents=True, exist_ok=True)
            builder = directory / "saturn-release-build.sh"
            builder.write_text(FAKE_BUILDER.replace('echo "builder $*"', f'echo "builder[{tag}] $*"'))
            builder.chmod(0o755)
        result = self.script(at=scripts / "update-saturn-go.sh", SATURN_RELEASE_BUILDER="")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        used = [line.split()[0] for line in self.calls_text().splitlines() if line.startswith("builder[")]
        self.assertTrue(used and set(used) == {"builder[installed]"}, used)
        self.assertEqual(self.active(), self.head)

    def test_without_an_installed_builder_the_update_stops_and_names_the_installer(self):
        scripts = self.root / "installed-scripts"
        shutil.copytree(SCRIPT.parent, scripts)
        (scripts / "saturn-release-build.sh").unlink()
        result = self.script(at=scripts / "update-saturn-go.sh", SATURN_RELEASE_BUILDER="")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("builder is not installed", result.stdout + result.stderr)
        self.assertIn("install_saturn_go_nginx.sh", result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertEqual(self.active(), B.OLD)

    def test_the_commit_is_resolved_once_and_that_commit_is_what_is_activated(self):
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        resolves = [line for line in self.calls_text().splitlines() if "--resolve-only" in line]
        self.assertEqual(len(resolves), 1)
        # a later push to the same ref does not change what this run installs
        (self.work / "a").write_text("2")
        git("commit", "-q", "-am", "two", cwd=self.work)
        git("push", "-q", str(self.remote), "main", cwd=self.work)
        self.assertEqual(self.active(), self.head)

    def test_a_release_that_is_already_active_is_not_built_or_dispatched(self):
        self.current.unlink()
        os.symlink(self.releases / self.head, self.current)
        (self.releases / self.head).mkdir()
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertIn("already active", result.stdout)
        self.assertEqual(self.status()["status"], "success")

    def test_an_existing_bundle_of_that_commit_is_reused_not_rebuilt(self):
        self.script("--stage-only")
        self.assertEqual(len(self.builds()), 1)
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.builds()), 1, "the bundle was built again")
        self.assertEqual(self.active(), self.head)

    def test_skip_git_builds_the_checkout_as_it_is(self):
        result = self.script("--skip-git")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("--source-remote", self.builds()[0])
        self.assertEqual(self.active(), self.head)


class Pinning(Flow):
    def test_the_build_is_told_which_commit_to_build(self):
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"--expect-commit {self.head}", self.builds()[0])

    def test_a_skipped_git_update_is_pinned_too(self):
        result = self.script("--skip-git")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"--expect-commit {self.head}", self.builds()[0])

    def test_a_branch_that_moves_between_resolving_and_building_builds_nothing(self):
        result = self.script(FAKE_MOVE_AFTER_RESOLVE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("moved since it was resolved", result.stdout + result.stderr)
        self.assertEqual(list(self.staging.iterdir()), [], "a bundle of some commit was built although the ref had moved")
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertEqual(self.active(), B.OLD)
        self.assertEqual(self.status()["status"], "error")

    def test_the_commit_that_was_resolved_is_the_one_installed_when_the_branch_does_not_move(self):
        self.assertEqual(self.script().returncode, 0)
        self.assertEqual(self.active(), self.head)


class Routing(Flow):
    """Which updater runs is decided by the layout, not only by the authorization."""

    def test_a_release_installation_with_managed_updates_disabled_refuses_and_never_falls_back_to_the_legacy_deployment(self):
        self.set_config(enabled="0")
        for flags in ((), ("--dry-run",), ("--migrate-to-releases",), ("--skip-git",), ("--stage-only",)):
            with self.subTest(flags=flags):
                result = self.script(*flags)
                self.assertNotEqual(result.returncode, 0)
                text = result.stdout + result.stderr
                self.assertIn("runs managed releases", text)
                self.assertIn("SATURN_MANAGED_UPDATES=enable", text)
                self.assertNotIn("Cargo", text)
                self.assertNotIn("Building saturn-go", text)
                status = self.status()
                self.assertEqual((status["status"], status["phase"], status["mode"]), ("error", "managed_updates_disabled", "release"))
                self.assertEqual(self.builds(), [])
                self.assertNotIn("systemd-run", self.calls_text())
                self.assertEqual(self.active(), B.OLD)

    def test_the_refusal_names_the_active_release(self):
        self.set_config(enabled="0")
        self.script()
        self.assertIn(B.OLD[:12], self.status()["message"])

    def test_a_release_installation_whose_configuration_is_missing_also_refuses(self):
        (self.root / "activate.conf").unlink()
        result = self.script(SATURN_RELEASE_ACTIVATE_CONFIG=str(self.root / "activate.conf"))
        # no configuration: the default pointer (/opt/saturn/current) is not this test's, so this is a legacy layout and is not entered as a release
        self.assertNotIn("runs managed releases", result.stdout + result.stderr)

    def test_a_legacy_installation_with_managed_updates_disabled_keeps_the_legacy_updater(self):
        self.current.unlink()
        self.set_config(enabled="0")
        result = self.script("--dry-run")
        text = result.stdout + result.stderr
        self.assertNotIn("Managed release update", text)
        self.assertNotIn("runs managed releases", text)
        self.assertEqual(self.builds(), [])
        self.assertNotIn("systemd-run", self.calls_text())


class Options(Flow):
    def test_dry_run_resolves_and_changes_nothing(self):
        result = self.script("--dry-run")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertEqual(self.active(), B.OLD)
        self.assertEqual(self.status()["phase"], "dry-run")

    def test_stage_only_builds_without_installing(self):
        result = self.script("--stage-only")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.builds()), 1)
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertEqual(self.active(), B.OLD)
        self.assertEqual(self.status()["phase"], "stage")

    def test_skip_deploy_builds_without_installing(self):
        self.assertEqual(self.script("--skip-deploy").returncode, 0)
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertEqual(self.active(), B.OLD)

    def test_skip_build_needs_an_existing_bundle(self):
        result = self.script("--skip-build")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no built bundle", result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertEqual(self.status()["status"], "error")


class Refusals(Flow):
    def test_a_failed_build_installs_nothing_and_says_why(self):
        result = self.script(FAKE_BUILD_RC="3")
        self.assertNotEqual(result.returncode, 0)
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "build"))
        self.assertIn("build failed", status["message"])
        self.assertNotIn("systemd-run", self.calls_text())
        self.assertEqual(self.active(), B.OLD)

    def test_a_ref_that_does_not_exist_is_refused_before_building(self):
        result = self.script(ref="no-such-branch")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.builds(), [])
        self.assertEqual(self.status()["status"], "error")

    def test_a_ref_that_does_not_resolve_to_a_full_commit_is_refused(self):
        for bad in ("zzzz", "cc09c61", self.head.upper(), self.head[:39], ""):
            with self.subTest(resolved=bad):
                result = self.script(FAKE_RESOLVED_COMMIT=bad)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("did not resolve to a full commit", result.stdout + result.stderr)
                self.assertEqual(self.builds(), [])
                self.assertNotIn("systemd-run", self.calls_text())

    def test_the_migration_flag_means_nothing_to_a_legacy_installation_that_has_managed_updates_off(self):
        self.current.unlink()
        self.set_config(enabled="0")
        result = self.script("--migrate-to-releases")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("applies only when managed updates are enabled", result.stderr)
        self.assertEqual(self.builds(), [])

    def test_a_legacy_layout_is_refused_before_anything_is_built(self):
        self.current.unlink()
        result = self.script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("legacy layout", result.stdout + result.stderr)
        self.assertEqual(self.builds(), [])
        self.assertNotIn("systemd-run", self.calls_text())

    def test_the_migration_is_explicit(self):
        self.current.unlink()
        result = self.script("--migrate-to-releases")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--allow-legacy-migration", self.calls_text().split("systemd-run", 1)[1].splitlines()[0])
        self.assertEqual(self.active(), self.head)
        self.assertEqual(self.status()["release"]["layout_before"], "legacy")

    def test_a_release_that_needs_newer_helpers_is_refused_with_the_brokers_reason(self):
        result = self.script(FAKE_BUNDLE_CONTRACT="2")
        self.assertNotEqual(result.returncode, 0)
        status = self.status()
        self.assertEqual(status["phase"], "installer_update_required", "the script overwrote the broker's reason")
        self.assertIn("installer update required", status["message"])
        self.assertNotIn("activator", self.calls_text())
        self.assertEqual(self.active(), B.OLD)

    def test_a_missing_broker_says_to_run_the_installer(self):
        result = self.script(SATURN_RELEASE_UPDATE_BROKER=str(self.root / "missing"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Re-run the installer", result.stderr)
        self.assertEqual(self.builds(), [])

    def test_a_preflight_refusal_comes_before_any_build_time_is_spent(self):
        self.set_config(enabled="1")
        (self.tools / "saturn-remote-validate.py").unlink()
        result = self.script()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.builds(), [])
        self.assertIn("saturn-remote-validate.py is not installed", result.stdout + result.stderr)

    def test_a_failed_validation_ends_as_a_rollback_the_page_can_read(self):
        result = self.script(FAKE_OUTCOME="rolled-back")
        status = self.status()
        self.assertEqual((status["status"], status["phase"]), ("error", "rolled_back"))
        self.assertEqual(self.active(), B.OLD)
        self.assertIn("audio.decoded_progress", status["message"])


class SourceChecks(unittest.TestCase):
    text = SCRIPT.read_text()
    flow = (REPO / "update_manager/scripts/saturn-release-update-flow.sh").read_text()

    def test_the_flow_runs_before_the_legacy_build_and_the_legacy_path_is_untouched(self):
        self.assertLess(self.text.index("run_release_flow"), self.text.index("ensure_build_swap\n  info"))
        self.assertIn("dispatch detached root deploy helper", self.text.replace("Dispatching", "dispatch"))

    def test_both_deployments_prepare_the_toolchain_the_same_way_before_they_build(self):
        self.assertEqual(self.text.count("prepare_build_toolchain_env\n"), 2, "the release branch and the legacy deployment both call it (the definition is the third mention)")
        self.assertLess(self.text.index("prepare_build_toolchain_env\n  run_release_flow"), self.text.index("exit 0\nfi\n(( ! MIGRATE_TO_RELEASES ))"))
        legacy = self.text[self.text.index("(( ! MIGRATE_TO_RELEASES )) || die"):]
        self.assertLess(legacy.index("prepare_build_toolchain_env"), legacy.index('"$CARGO_BIN"'), "the legacy deployment builds with CARGO_BIN, so it must be prepared first")
        definition = self.text[self.text.index("prepare_build_toolchain_env(){"):]
        definition = definition[:definition.index("\n}\n")]
        for needed in ('export CARGO_HOME=', 'export RUSTUP_HOME=', 'export PATH="$CARGO_BIN_DIR:$PATH"'):
            self.assertIn(needed, definition)

    def test_the_builder_is_started_only_through_the_clean_environment(self):
        invocations = [line.strip() for line in self.flow.splitlines() if '"$builder"' in line and not line.strip().startswith("#")]
        self.assertEqual(len(invocations), 2, invocations)      # the definition's own env -i call, and the [[ -x ]] check
        self.assertTrue(any(line.startswith('env -i LANG=C.UTF-8 "${clean_env[@]}" "${settings[@]}" "$builder"') for line in invocations))
        self.assertEqual(self.flow.count("run_release_builder "), 3, "the definition, the resolve and the build")
        allow = self.flow[self.flow.index("RELEASE_BUILD_ENV_ALLOW="):].splitlines()[0]
        for secret_or_config in ("SATURN_REMOTE", "SATURN_ADDR", "SATURN_STATE", "SATURN_WEBROOT", "SATURN_CONFIG", "SATURN_[A-Z]"):
            self.assertNotIn(secret_or_config, allow)

    def test_the_flow_installs_and_restarts_nothing_itself(self):
        for forbidden in ("systemctl", "systemd-run", "ln -s", "cp -", "install -m"):
            self.assertNotIn(forbidden, self.flow, forbidden)

    def test_the_flow_hands_over_through_the_broker_only(self):
        self.assertIn('"$broker" --dispatch', self.flow)
        self.assertIn('"$broker" --preflight', self.flow)


if __name__ == "__main__":
    unittest.main(verbosity=2)
