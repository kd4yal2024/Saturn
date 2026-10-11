#!/usr/bin/env python3
"""Root-owned broker for a managed update: install a built release, activate it, and say what happened.

The Update action of the Saturn Go page builds a release bundle as the service user and then calls this broker. It adds nothing of its own to
installing, activating or rolling back: it runs the existing root installer (saturn-release-install-root.sh) and the existing root activator
(saturn-release-activate-root.sh), whose transaction restarts the services, proves reception with the headless receive validation, and
restores the previous release if that fails. What this broker adds is the part that has to survive the web service restarting: it runs in
its own transient unit, holds the maintenance locks, keeps the status file the page reads up to date, and classifies the outcome.

Modes (the service user may run only the first two, through sudo; --run is for the transient unit and refuses to run from anything else):

  --preflight                 Policy and layout checks that need no bundle: managed updates enabled, a release layout (or an explicit
                              migration), the privileged helpers present. Changes nothing. Lets the page refuse before it spends hours building.
  --dispatch [--allow-legacy-migration] BUNDLE
                              Checks the bundle and everything --preflight checks, then starts --run in a detached transient unit and returns.
  --run [--allow-legacy-migration] BUNDLE
                              Does the update (as root, inside the unit).

The privileged helpers (activator, validator, guard) are never taken from a bundle: they are installed by the installer, which is the trust
anchor. A release that declares a newer helper contract than the installed helpers is refused before anything is changed, with
"installer update required".

Exit status: 0 the release is active and committed (or already was); 2 refused before any change; 70 the previous release could not be restored
(intervention needed); 71 the previous release was restored but reception on it is not verified; 1 any other failure after a rollback or
before the end (read the status file and the log).
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

DEFAULT_TOOLS_DIR = "/usr/local/lib/saturn-go/scripts"
DEFAULT_INSTALL_CONFIG = "/etc/default/saturn-release-install"
DEFAULT_ACTIVATE_CONFIG = "/etc/default/saturn-release-activate"
DEFAULT_STATUS_FILE = "/var/lib/saturn-state/saturngo_deploy_status.json"
DEFAULT_LOG_DIR = "/var/lib/saturn-state/release-update"
DEFAULT_PERF_JSON = "/run/saturn-bridge/perf.json"
DEFAULT_LOCK_DIR = "/run/lock/saturn-maintenance"

INSTALLER = "saturn-release-install-root.sh"
ACTIVATOR = "saturn-release-activate-root.sh"
LOCK_TOOL = "saturn-maintenance-lock.py"
STATE_WRITER = "saturn-state-write.py"
HELPERS = (ACTIVATOR, "saturn-remote-validate.py", "saturn-remote-receive-guard.js")
CONTRACT_MARK = re.compile(r"saturn-helper-contract:\s*([0-9]+)")

LOCK_RESOURCES = "release,radio"
LOCK_WAIT_SECONDS = 90
RUN_TIMEOUT_SECONDS = 2 * 3600
EXIT_REFUSED, EXIT_NOT_RESTORED, EXIT_UNVERIFIED, EXIT_CONFLICT = 2, 70, 71, 75

COMMIT = re.compile(r"[0-9a-f]{40}")
CONFIG_LINE = re.compile(r"^([A-Z][A-Z0-9_]*)=(.*)$")


class Refused(Exception):
    """Nothing was changed; the message is for the administrator."""

    def __init__(self, phase: str, message: str) -> None:
        super().__init__(message)
        self.phase, self.message = phase, message


def now_iso() -> str:
    return datetime.datetime.now().astimezone().isoformat(timespec="seconds")


def trim(value: str) -> str:
    for quote in ('"', "'"):
        if value.startswith(quote):
            value = value[1:]
        if value.endswith(quote):
            value = value[:-1]
    return value


def read_config(path: str, root_required: bool) -> dict[str, str]:
    """key=value lines, read as data. As root the file must be root-owned and not group or world writable, like every root-owned configuration."""
    values: dict[str, str] = {}
    if not os.path.exists(path):
        return values
    if root_required:
        info = os.stat(path)
        if info.st_uid != 0 or info.st_mode & 0o022:
            raise Refused("refused", f"{path} is not root-owned or is group/world writable")
    with open(path, encoding="utf-8") as handle:
        for line in handle.read().splitlines():
            match = CONFIG_LINE.match(line.strip()) if line.strip() and not line.lstrip().startswith("#") else None
            if match:
                values[match.group(1)] = trim(match.group(2))
    return values


class Environment:
    """Where everything is. Production values are fixed; a non-root caller (the tests) may move them with environment variables."""

    def __init__(self) -> None:
        self.root = os.geteuid() == 0
        movable = (lambda name, default: default) if self.root else (lambda name, default: os.environ.get(name, default))
        self.tools = movable("SATURN_RELEASE_UPDATE_TOOLS_DIR", DEFAULT_TOOLS_DIR)
        self.install_config = movable("SATURN_RELEASE_UPDATE_INSTALL_CONFIG", DEFAULT_INSTALL_CONFIG)
        self.activate_config = movable("SATURN_RELEASE_UPDATE_ACTIVATE_CONFIG", DEFAULT_ACTIVATE_CONFIG)
        self.status_file = movable("SATURN_RELEASE_UPDATE_STATUS_FILE", DEFAULT_STATUS_FILE)
        self.log_dir = movable("SATURN_RELEASE_UPDATE_LOG_DIR", DEFAULT_LOG_DIR)
        self.perf_json = movable("SATURN_RELEASE_UPDATE_PERF_JSON", DEFAULT_PERF_JSON)
        self.lock_dir = movable("SATURN_RELEASE_UPDATE_LOCK_DIR", DEFAULT_LOCK_DIR)
        self.systemd_run = movable("SATURN_RELEASE_UPDATE_SYSTEMD_RUN", "systemd-run")
        self.lock_wait = float(movable("SATURN_RELEASE_UPDATE_LOCK_WAIT", str(LOCK_WAIT_SECONDS)))
        self.self_path = movable("SATURN_RELEASE_UPDATE_SELF", os.path.abspath(__file__))
        install = read_config(self.install_config, self.root)
        activate = read_config(self.activate_config, self.root)
        self.run_user = install.get("RUN_USER", "pi")
        self.staging_root = install.get("STAGING_ROOT", "/var/lib/saturn-state/release-staging")
        self.releases_root = activate.get("RELEASES_ROOT", install.get("RELEASES_ROOT", "/opt/saturn/releases"))
        self.current_link = activate.get("CURRENT_LINK", "/opt/saturn/current")
        self.transaction_file = activate.get("TRANSACTION_FILE", "/var/lib/saturn-state/deployments/current.json")
        self.enabled = activate.get("ACTIVATION_ENABLED", "0") == "1"
        self.group = activate.get("TRANSACTION_GROUP", "pi")


def tool(env: Environment, name: str) -> str:
    return os.path.join(env.tools, name)


# ---- status -----------------------------------------------------------------------------------------------------------------------------
class Status:
    """The file the Saturn Go page polls. The fields the page already shows (status, phase, message, updated_at, unit_name) keep their meaning;
    the release details are an added object."""

    def __init__(self, env: Environment, unit: str | None) -> None:
        self.env, self.unit = env, unit
        self.started_at = now_iso()
        self.extra: dict = {}

    def write(self, status: str, phase: str, message: str, exit_code: int | None = None, finished: bool = False, **extra) -> None:
        self.extra.update(extra)
        document = {
            "status": status, "phase": phase, "message": message, "updated_at": now_iso(), "started_at": self.started_at,
            "finished_at": now_iso() if finished else None, "exit_code": exit_code, "unit_name": self.unit, "mode": "release", **self.extra,
        }
        text = json.dumps(document, indent=2, sort_keys=True) + "\n"
        target = self.env.status_file
        os.makedirs(os.path.dirname(target), mode=0o750, exist_ok=True)
        writer = tool(self.env, STATE_WRITER)
        if self.env.root and os.path.exists(writer):
            import pwd
            entry = pwd.getpwnam(self.env.run_user)
            subprocess.run([writer, "--path", target, "--mode", "0640", "--owner", str(entry.pw_uid), "--group", str(entry.pw_gid), "--last-good"],
                           input=text, text=True, check=True, timeout=30)
            return
        descriptor, temporary = tempfile.mkstemp(prefix=".status.", dir=os.path.dirname(target))
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(text)
        os.chmod(temporary, 0o640)
        os.replace(temporary, target)


# ---- checks -----------------------------------------------------------------------------------------------------------------------------
def current_layout(env: Environment) -> tuple[str, str | None]:
    """('release', commit) when the active pointer is a symbolic link to an installed release; ('legacy', None) otherwise."""
    link = env.current_link
    if os.path.islink(link):
        target = os.path.realpath(link)
        if os.path.dirname(target) == os.path.realpath(env.releases_root) and COMMIT.fullmatch(os.path.basename(target)):
            return "release", os.path.basename(target)
    return "legacy", None


def contract_of(path: str) -> int:
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            head = handle.read(4096)
    except OSError:
        return 0
    match = CONTRACT_MARK.search(head)
    return int(match.group(1)) if match else 0


def check_policy_and_layout(env: Environment, allow_migration: bool) -> tuple[str, str | None]:
    if not env.enabled:
        raise Refused("refused", "managed updates are disabled (ACTIVATION_ENABLED is not 1 in the root-owned activation configuration); "
                                 "enable them with SATURN_MANAGED_UPDATES=enable when running the installer")
    layout, commit = current_layout(env)
    if layout == "legacy" and not allow_migration:
        raise Refused("migration_required", "this installation uses the legacy layout (no release is active). Moving to managed releases is a one-way "
                                            "change and is never done by an ordinary update: run the update with --migrate-to-releases to do it deliberately")
    for name in (INSTALLER, ACTIVATOR, "saturn-remote-validate.py", "saturn-remote-receive-guard.js", LOCK_TOOL):
        if not os.path.exists(tool(env, name)):
            raise Refused("installer_update_required", f"the privileged helper {name} is not installed; run the installer")
    return layout, commit


def read_manifest(bundle: str) -> dict:
    try:
        with open(os.path.join(bundle, "release-manifest.json"), encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as error:
        raise Refused("refused", f"the bundle has no readable release-manifest.json: {error}") from error


def check_bundle(env: Environment, bundle: str) -> tuple[str, str, dict]:
    if os.path.islink(bundle):
        raise Refused("refused", "the bundle path is a symbolic link")
    real = os.path.realpath(bundle)
    if os.path.dirname(real) != os.path.realpath(env.staging_root):
        raise Refused("refused", f"the bundle is not directly inside the release staging directory {env.staging_root}")
    commit = os.path.basename(real)
    if not COMMIT.fullmatch(commit):
        raise Refused("refused", "the bundle directory is not named by a full lowercase commit")
    manifest = read_manifest(real)
    source = manifest.get("source") if isinstance(manifest.get("source"), dict) else {}
    if source.get("commit") != commit or source.get("dirty") is not False:
        raise Refused("refused", f"the bundle's manifest does not describe a clean tree of {commit}")
    return real, commit, manifest


def check_helper_contract(env: Environment, bundle: str) -> None:
    for name in HELPERS:
        wanted = contract_of(os.path.join(bundle, "scripts", name))
        have = contract_of(tool(env, name))
        if wanted > have:
            raise Refused("installer_update_required",
                          f"installer update required: this release needs {name} helper contract {wanted}, the installed one is {have}. "
                          "Run the installer from the new checkout (sudo ./install.sh); nothing was changed")


# ---- running ----------------------------------------------------------------------------------------------------------------------------
def radio_state(env: Environment) -> dict | None:
    """What the owner reports about the radio's reception settings, for the before/after comparison (never anything about transmit)."""
    try:
        with open(env.perf_json, encoding="utf-8") as handle:
            perf = json.load(handle)
    except (OSError, ValueError):
        return None
    metrics = perf.get("metrics") or {}
    keys = ("frequency_hz", "sample_rate_hz", "ddc", "adc", "processing_mode")
    state = {key: metrics.get(key) for key in keys}
    state.update(backend=perf.get("backend"), rf_safe=perf.get("rf_safe"), tx_keyed=metrics.get("tx_keyed"),
                 rxc1=(metrics.get("rx_counter_v31") or {}).get("status"))
    return state


def settings_preserved(before: dict | None, after: dict | None) -> bool | None:
    if not before or not after:
        return None
    return all(before.get(key) == after.get(key) for key in ("backend", "frequency_hz", "sample_rate_hz", "ddc", "adc"))


def read_record(env: Environment) -> dict:
    try:
        with open(env.transaction_file, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return {}


def validation_summary(record: dict, commit: str) -> dict:
    if record.get("target_commit") != commit:
        return {}
    summary = {"record_status": record.get("status"), "record_phase": record.get("phase")}
    for label, entry in (record.get("validation") or {}).items():
        summary[label] = {key: entry.get(key) for key in ("final_state", "driver", "role", "viewer_role_policy", "legacy_fallback_used", "failed", "memory_enforcement")}
    rollback = record.get("rollback")
    if rollback:
        summary["rollback"] = {key: rollback.get(key) for key in ("status", "release_restored", "receive_verified")}
    failure = record.get("activation_failure")
    if failure:
        summary["activation_failure"] = failure
    return summary


def classify(env: Environment, commit: str, rc: int) -> tuple[str, str, str]:
    """(status, phase, message) from the activator's exit status and the transaction record it left."""
    record = read_record(env)
    mine = record.get("target_commit") == commit
    if rc == 0 and mine and record.get("status") == "committed":
        return "success", "done", f"Release {commit[:12]} is active, receives, and was verified by the headless receive validation"
    if rc == EXIT_NOT_RESTORED:
        return "error", "rollback_failed", "The update failed and the previous release could NOT be restored; the radio needs attention (see the log and the deployment record)"
    if rc == EXIT_UNVERIFIED:
        return "error", "rollback_unverified", "The update failed; the previous release was restored, but reception on it could not be verified (see the log)"
    rollback = (record.get("rollback") or {}) if mine else {}
    failed = ((record.get("validation") or {}).get("target") or {}).get("failed") if mine else None
    detail = f" ({failed.get('id')}: {failed.get('detail')})" if isinstance(failed, dict) and failed.get("id") else ""
    if mine and record.get("status") == "rolled_back" and rollback.get("receive_verified") is True:
        return "error", "rolled_back", f"The update was not kept: it failed validation{detail}. The previous release was restored and its reception was verified"
    if rc == 0:
        return "error", "unconfirmed", "The activator reported success but the deployment record does not say the release is committed"
    return "error", "failed", f"The update failed (exit {rc}){detail}; see the log and the deployment record"


def run_logged(command: list[str], log, env_extra: dict | None = None, timeout: float | None = None) -> int:
    log.write(f"$ {' '.join(command)}\n")
    log.flush()
    process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, env=dict(os.environ, **(env_extra or {})), timeout=timeout)
    log.write(f"[exit {process.returncode}]\n")
    log.flush()
    return process.returncode


def unit_name_from_cgroup() -> str | None:
    try:
        with open("/proc/self/cgroup", encoding="utf-8") as handle:
            for line in handle.read().splitlines():
                name = line.rsplit("/", 1)[-1]
                if name.endswith(".service"):
                    return name[:-len(".service")]
    except OSError:
        pass
    return None


def do_run(env: Environment, bundle_arg: str, allow_migration: bool, locked: bool) -> int:
    unit = unit_name_from_cgroup() or os.environ.get("SATURN_RELEASE_UPDATE_UNIT")
    status = Status(env, unit)
    log = None
    try:
        layout, previous = check_policy_and_layout(env, allow_migration)
        bundle, commit, manifest = check_bundle(env, bundle_arg)
        check_helper_contract(env, bundle)
        source = manifest.get("source", {})
        release = {"commit": commit, "previous_commit": previous, "layout_before": layout, "requested_ref": source.get("requested_ref"),
                   "repository": source.get("repository")}
        if previous == commit:
            status.write("success", "done", f"Release {commit[:12]} is already the active release; nothing to do", 0, True, release=release)
            return 0
        if not locked:
            return run_under_locks(env, bundle, allow_migration, status, release)
        os.makedirs(env.log_dir, mode=0o750, exist_ok=True)
        log_path = os.path.join(env.log_dir, f"{time.strftime('%Y%m%dT%H%M%S')}-{commit[:12]}.log")
        log = open(log_path, "w", encoding="utf-8")
        os.chmod(log_path, 0o640)
        status.extra["log"] = log_path
        before = radio_state(env)
        release["radio_before"] = before
        installed = os.path.isdir(os.path.join(env.releases_root, commit))
        status.write("running", "validate", "Validating the built release", release=release)
        if not installed and run_logged([tool(env, INSTALLER), "--validate", bundle], log):
            status.write("error", "refused", "The root installer refused the built release (see the log); nothing was changed", EXIT_REFUSED, True, release=release)
            return EXIT_REFUSED
        if not installed:
            status.write("running", "install", "Installing the release (inactive)", release=release)
            if run_logged([tool(env, INSTALLER), bundle], log):
                status.write("error", "refused", "The release could not be installed (see the log); the active release was not changed", EXIT_REFUSED, True, release=release)
                return EXIT_REFUSED
        status.write("running", "activate", "Activating: the services restart, reception is verified by a headless session, and the previous release is restored if that fails", release=release)
        rc = run_logged([tool(env, ACTIVATOR), commit], log)
        after = radio_state(env)
        release["radio_after"] = after
        release["settings_preserved"] = settings_preserved(before, after)
        release["rxc1_after"] = (after or {}).get("rxc1")
        release["active_commit"] = current_layout(env)[1]
        outcome, phase, message = classify(env, commit, rc)
        release["result"] = validation_summary(read_record(env), commit)
        exit_status = 0 if outcome == "success" else (rc or 1)       # never report a failure with exit status 0
        status.write(outcome, phase, message, exit_status, True, release=release)
        return exit_status
    except Refused as refusal:
        status.write("error", refusal.phase, refusal.message, EXIT_REFUSED, True)
        print(f"[saturn-release-update] {refusal.message}", file=sys.stderr)
        return EXIT_REFUSED
    except subprocess.TimeoutExpired as error:
        status.write("error", "failed", f"A step did not finish in time: {error.cmd}", 1, True)
        return 1
    except Exception as error:  # noqa: BLE001  (the page must hear about it)
        status.write("error", "failed", f"The update failed unexpectedly: {error!r}", 1, True)
        raise
    finally:
        if log:
            log.close()


def run_under_locks(env: Environment, bundle: str, allow_migration: bool, status: Status, release: dict) -> int:
    """Run the rest of the update under the host maintenance locks (the same ones the other maintenance operations take), retrying briefly while
    the page's own script, which holds them until it exits, finishes."""
    command = [tool(env, LOCK_TOOL), "--lock-dir", env.lock_dir, "run", "--operation", "release-update", "--resources", LOCK_RESOURCES,
               "--timeout-seconds", str(RUN_TIMEOUT_SECONDS), "--", sys.executable, env.self_path, "--run", "--locked"]
    if allow_migration:
        command.append("--allow-legacy-migration")
    command.append(bundle)
    deadline = time.monotonic() + env.lock_wait
    status.write("running", "lock", "Waiting for the other maintenance operations to finish", release=release)
    while True:
        process = subprocess.run(command, env=dict(os.environ, SATURN_RELEASE_UPDATE_UNIT=status.unit or ""))
        if process.returncode != EXIT_CONFLICT or time.monotonic() >= deadline:
            break
        time.sleep(2)
    if process.returncode == EXIT_CONFLICT:
        status.write("error", "busy", "Another maintenance operation holds the release or radio lock; nothing was changed. Try again when it has finished", EXIT_REFUSED, True, release=release)
        return EXIT_REFUSED
    return process.returncode


def do_dispatch(env: Environment, bundle_arg: str, allow_migration: bool) -> int:
    status = Status(env, None)
    try:
        layout, _ = check_policy_and_layout(env, allow_migration)
        bundle, commit, manifest = check_bundle(env, bundle_arg)
        check_helper_contract(env, bundle)
    except Refused as refusal:
        status.write("error", refusal.phase, refusal.message, EXIT_REFUSED, True)
        print(f"[saturn-release-update] REFUSED ({refusal.phase}): {refusal.message}", file=sys.stderr)
        return EXIT_REFUSED
    unit = f"saturn-release-update-{time.strftime('%Y%m%d%H%M%S')}"
    command = [env.systemd_run, "--unit", unit, "--collect", "--no-block", env.self_path, "--run"]
    if allow_migration:
        command.append("--allow-legacy-migration")
    command.append(bundle)
    status = Status(env, unit)
    status.write("dispatched", "dispatch", f"Release {commit[:12]} handed to a detached update unit; the services will restart",
                 release={"commit": commit, "layout_before": layout})
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        status.write("error", "dispatch", f"The update unit could not be started: {result.stderr.strip()[:300]}", 1, True)
        print(result.stderr, file=sys.stderr)
        return 1
    print(f"[saturn-release-update] dispatched {commit} as {unit}")
    return 0


def do_preflight(env: Environment, allow_migration: bool) -> int:
    try:
        layout, commit = check_policy_and_layout(env, allow_migration)
    except Refused as refusal:
        print(f"[saturn-release-update] REFUSED ({refusal.phase}): {refusal.message}", file=sys.stderr)
        return EXIT_REFUSED
    print(f"[saturn-release-update] managed updates are enabled; layout {layout}" + (f", active release {commit}" if commit else ""))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--preflight", action="store_true")
    group.add_argument("--dispatch", action="store_true")
    group.add_argument("--run", action="store_true")
    parser.add_argument("--locked", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--allow-legacy-migration", action="store_true")
    parser.add_argument("bundle", nargs="?")
    args = parser.parse_args(argv)
    env = Environment()
    if args.preflight:
        if args.bundle or args.locked:
            parser.error("--preflight takes no bundle")
        return do_preflight(env, args.allow_legacy_migration)
    if not args.bundle:
        parser.error("a bundle directory is required")
    if args.locked and not args.run:
        parser.error("--locked belongs to --run")
    if args.dispatch:
        return do_dispatch(env, args.bundle, args.allow_legacy_migration)
    if env.root and not os.environ.get("INVOCATION_ID"):
        print("[saturn-release-update] --run is for the transient update unit (a terminal session would end with the connection); use --dispatch", file=sys.stderr)
        return EXIT_REFUSED
    return do_run(env, args.bundle, args.allow_legacy_migration, args.locked)


if __name__ == "__main__":
    sys.exit(main())
