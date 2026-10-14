#!/usr/bin/env python3
"""Root-owned broker for a managed update: install a built release, activate it, and say what happened.

The Update action of the Saturn Go page builds a release bundle as the service user and then calls this broker. It adds nothing of its own to
installing, activating or rolling back: it runs the existing root installer (saturn-release-install-root.sh) and the existing root activator
(saturn-release-activate-root.sh), whose transaction restarts the services, proves reception with the headless receive validation, and
restores the previous release if that fails. What this broker adds is the part that has to survive the web service restarting: it runs in
its own transient unit, holds the maintenance locks for the whole run (the release lock exclusively, the radio and repository locks SHARED, so that nothing
else can start in between, while saturn-go's own startup recovery is still answered, by a read-only check), refuses to start while a restore transaction is still
pending, keeps the status file the page reads up to date, and classifies the outcome.

Modes (the service user may run only the first two, through sudo; --run is for the transient unit and refuses to run from anything else):

  --preflight                 Policy and layout checks that need no bundle: managed updates enabled, a release layout (or an explicit
                              migration), the privileged helpers present, no restore transaction pending. Changes nothing. Lets the page refuse before it spends hours building.
  --dispatch [--allow-legacy-migration] BUNDLE
                              Checks the bundle and everything --preflight checks, then starts --run in a detached transient unit and returns.
  --run [--allow-legacy-migration] BUNDLE
                              Does the update (as root, inside the unit): takes the maintenance locks, installs, then replaces itself with a small shell
                              that runs the activator, so that no Python process stays resident while the headless browser's memory headroom is
                              checked.
  --finish CONTEXT --activator-rc N
                              The last step, started by that shell (not for people): classifies the outcome and tells the page.

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
import fcntl
import json
import os
import re
import stat
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
DEFAULT_STATE_ROOT = "/var/lib/saturn-state"

INSTALLER = "saturn-release-install-root.sh"
ACTIVATOR = "saturn-release-activate-root.sh"
STATE_WRITER = "saturn-state-write.py"
HELPERS = (ACTIVATOR, "saturn-remote-validate.py", "saturn-remote-receive-guard.js")
CONTRACT_MARK = re.compile(r"saturn-helper-contract:\s*([0-9]+)")

# The maintenance locks are taken here, in this process, with flock on the same lock files the lock tool and saturn-go use: no lock-tool wrapper and no
# second copy of this program stay resident (each Python process costs memory that the headless browser's headroom check counts: the lock tool and the
# re-executed broker were two of the five Python processes that held it ~55 MiB below the 532 MiB the check needs).
#
# What is held, and why it is shaped like this. `release` is held EXCLUSIVELY for the whole run: a second deployment cannot start while this one does.
# `radio` and `repository` are held SHARED for the whole run, rollback included (the descriptors are inherited through the hand-over to the activator and
# the finish step). Every other maintenance operation asks for them exclusively, through the lock tool or with a plain flock (FPGA flashing, repository
# restore, the Pi-HPSDR updaters, the radio backend switch), so none of them can start at any point of the run, and the restarted web server being
# reachable before the validation has finished does not matter. The one caller that must get in is saturn-go itself: the activation restarts it, its startup
# runs a restore recovery under exactly those two locks (operation `restore:recover`) and EXITS when it cannot take them, so locks held exclusively here made
# the target's server unable to start (readiness failed) and the previous release's too (the rollback was recorded as failed): the first page-driven update
# on the G2 failed that way. The lock tool therefore answers that one call, exactly as saturn-go makes it, while this program holds the locks. An earlier
# version of this program only CHECKED radio and repository before the activation and let go of them; anything could take them the moment after the check.
#
# The answer is a READ-ONLY CHECK, never the recovery itself: the recovery proper writes (settings, the repository pointer, staging directories, the transaction
# records), and under a shared hold nothing would keep it from writing while this update does, or twice at once. The coordination is therefore: (1) a restore
# transaction that is still pending is not something an update may start with: this program asks the lock tool (`pending-restores`, read-only) before it
# begins and refuses, changing nothing, if one is; saturn-go's own startup completes it, under the exclusive locks, outside an update. (2) The question is asked
# again once the shared locks are held, because every writer needs those two locks exclusively: with them held no restore can start and none can be left
# pending, so (3) the check that saturn-go's restarted server meets during the activation, and during a rollback, finds nothing to do.
LOCK_RESOURCES = "release"
SHARED_RESOURCES = "radio,repository"
LOCK_TOOL = "saturn-maintenance-lock.py"
LOCK_WAIT_SECONDS = 90
RUN_TIMEOUT_SECONDS = 2 * 3600          # enforced by the transient unit (RuntimeMaxSec)
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
        self.state_root = movable("SATURN_RELEASE_UPDATE_STATE_ROOT", DEFAULT_STATE_ROOT)
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
    check_lock_tool(env)
    return layout, commit


def check_lock_tool(env: Environment) -> None:
    """This program holds radio and repository for the whole run and relies on the lock tool to answer saturn-go's own startup recovery while it does, with a
    read-only check that runs nothing and writes nothing. A tool that cannot would leave the restarted server unable to start (a failed rollback), and the tool
    of an earlier version ran whatever carried the recovery's name, writing, under the shared hold: refuse both, naming the installer, before anything is changed."""
    try:
        result = subprocess.run([tool(env, LOCK_TOOL), "capabilities"], capture_output=True, text=True, timeout=15)
        capabilities = json.loads(result.stdout) if result.returncode == 0 else {}
    except (OSError, ValueError, subprocess.TimeoutExpired):
        capabilities = {}
    check = capabilities.get("recovery_check") if isinstance(capabilities, dict) else None
    if not isinstance(check, int) or isinstance(check, bool) or check < 1 or "shared_recovery" in capabilities:
        raise Refused("installer_update_required",
                      f"installer update required: the installed {LOCK_TOOL} cannot answer saturn-go's own restore recovery with a read-only check while an update "
                      "holds the radio and repository locks (it either leaves the restarted server unable to start or lets the recovery write under the shared "
                      "locks). Run the Saturn Go installer from the new checkout; nothing was changed")


def check_restore_transactions(env: Environment) -> None:
    """Refuse, changing nothing, while a restore transaction is still pending (or cannot be read). The recovery that completes it writes, so it is done by
    saturn-go's startup under the exclusive locks and outside an update, never during one. Asked without the locks (preflight, dispatch) it is advisory: a
    restore that is running now shows as pending too. Asked with the shared locks held (the run) it is final, because no writer can start while they are."""
    try:
        result = subprocess.run([tool(env, LOCK_TOOL), "pending-restores", "--state-root", env.state_root], capture_output=True, text=True, timeout=30)
        answer = json.loads(result.stdout) if result.stdout.strip() else {}
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        raise Refused("restore_check_failed", f"the restore transactions could not be checked ({error!r}); nothing was changed") from error
    if not isinstance(answer, dict):
        answer = {}
    pending = answer.get("pending")
    if result.returncode == 0 and answer.get("status") == "ok" and pending == []:
        return
    if result.returncode == 1 and answer.get("status") == "pending" and isinstance(pending, list) and pending:
        raise Refused("restore_transaction_pending",
                      "a restore transaction is incomplete or still running (" + "; ".join(str(item) for item in pending[:3]) + "), so no update is started. Saturn Go's "
                      "startup completes it: wait for a running restore to finish, or restart the saturn-go service so its startup recovery rolls the "
                      "incomplete one back, then run the update again; nothing was changed")
    raise Refused("restore_check_failed", f"the restore transactions could not be checked (exit {result.returncode}: {(result.stderr or result.stdout).strip()[:200]}); "
                                          "nothing was changed")


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


# ---- the locks ----------------------------------------------------------------------------------------------------------------------------
def open_lock(env: Environment, name: str) -> int:
    path = os.path.join(env.lock_dir, f"{name}.lock")
    try:
        descriptor = os.open(path, os.O_RDWR | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise Refused("lock", f"cannot open the maintenance lock file {path}: {error.strerror}") from error
    if not stat.S_ISREG(os.fstat(descriptor).st_mode):
        os.close(descriptor)
        raise Refused("lock", f"the maintenance lock path is not a regular file: {path}")
    return descriptor


def hold_release_lock(env: Environment, status: Status, release: dict) -> int | None:
    """Take the release lock (a second deployment cannot start while this one runs), retrying briefly while the page's own script, which holds the
    application-deployment locks until it exits, finishes. The descriptor is inheritable: it stays held across the hand-over to the activator."""
    descriptor = open_lock(env, LOCK_RESOURCES)
    deadline = time.monotonic() + env.lock_wait
    announced = False
    while True:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            os.set_inheritable(descriptor, True)
            return descriptor
        except BlockingIOError:
            if time.monotonic() >= deadline:
                os.close(descriptor)
                return None
            if not announced:
                status.write("running", "lock", "Waiting for the other maintenance operations to finish", release=release)
                announced = True
            time.sleep(min(2.0, max(0.05, env.lock_wait / 20)))


def hold_shared_locks(env: Environment, status: Status, release: dict) -> list[int] | None:
    """Take the radio and repository locks SHARED (see SHARED_RESOURCES), all or none. An exclusive holder (a restore, a flash, a backend switch) means
    "busy"; the wait is as long as the release lock's, because the page's own script lets go of its locks one after the other as it exits. The descriptors
    are inheritable: they stay held across the hand-over to the activator and the finish step."""
    descriptors: list[int] = []
    try:
        for name in SHARED_RESOURCES.split(","):
            descriptors.append(open_lock(env, name))
    except Refused:
        for descriptor in descriptors:
            os.close(descriptor)
        raise
    deadline = time.monotonic() + env.lock_wait
    announced = False
    while True:
        try:
            for descriptor in descriptors:
                fcntl.flock(descriptor, fcntl.LOCK_SH | fcntl.LOCK_NB)
            for descriptor in descriptors:
                os.set_inheritable(descriptor, True)
            return descriptors
        except BlockingIOError:
            for descriptor in descriptors:
                fcntl.flock(descriptor, fcntl.LOCK_UN)       # all or none
            if time.monotonic() >= deadline:
                for descriptor in descriptors:
                    os.close(descriptor)
                return None
            if not announced:
                status.write("running", "lock", "Waiting for the other maintenance operations to finish", release=release)
                announced = True
            time.sleep(min(2.0, max(0.05, env.lock_wait / 20)))


# ---- the run ----------------------------------------------------------------------------------------------------------------------------
# While the activator runs (the services restart and the headless browser is validated) nothing of this program may stay resident: the validation
# refuses to start a browser unless the host has the memory for it, and a resident Python process is memory it does not have. So after the
# preparation this process REPLACES ITSELF with a small shell that runs the activator, appends its output to the log, and then starts this program
# again in --finish mode to classify the outcome. The descriptors of the release, radio and repository locks are inherited through all of it.
SUPERVISOR = r"""log="$3"
printf '$ %s %s\n' "$1" "$2" >>"$log"
"$1" "$2" >>"$log" 2>&1
rc=$?
printf '[exit %s]\n' "$rc" >>"$log"
exec "$4" "$5" --finish "$6" --activator-rc "$rc"
"""


def do_run(env: Environment, bundle_arg: str, allow_migration: bool) -> int:
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
        lock = hold_release_lock(env, status, release)
        if lock is None:
            status.write("error", "busy", "Another deployment holds the release lock; nothing was changed. Try again when it has finished", EXIT_REFUSED, True, release=release)
            return EXIT_REFUSED
        shared = hold_shared_locks(env, status, release)
        if shared is None:
            status.write("error", "busy", "Another maintenance operation holds the radio or repository lock; nothing was changed. Try again when it has finished", EXIT_REFUSED, True, release=release)
            return EXIT_REFUSED
        check_restore_transactions(env)           # final: with radio and repository held no restore can start, and none can be left pending from here on
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
        context_path = log_path + ".context.json"
        descriptor = os.open(context_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump({"commit": commit, "unit": unit, "started_at": status.started_at, "extra": status.extra, "before": before}, handle)
        log.close()
        log = None
        os.execv("/bin/bash", ["bash", "-c", SUPERVISOR, "saturn-release-update", tool(env, ACTIVATOR), commit, log_path, sys.executable, env.self_path, context_path])
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


def do_finish(env: Environment, context_path: str, rc: int) -> int:
    """The last step of a run, started by the shell that ran the activator: read what the run knew, compare the radio before and after, classify the
    outcome from the activator's exit status and the deployment record, and tell the page."""
    try:
        with open(context_path, encoding="utf-8") as handle:
            context = json.load(handle)
        commit, before = context["commit"], context.get("before")
        status = Status(env, unit_name_from_cgroup() or context.get("unit"))
        status.started_at = context["started_at"]
        status.extra.update(context["extra"])
        release = status.extra["release"]
    except (OSError, ValueError, KeyError) as error:
        print(f"[saturn-release-update] cannot finish: the run's context {context_path} is unreadable: {error!r}", file=sys.stderr)
        return EXIT_REFUSED
    try:
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
    except Exception as error:  # noqa: BLE001  (the page must hear about it)
        status.write("error", "failed", f"The update failed unexpectedly: {error!r}", 1, True)
        raise
    finally:
        try:
            os.unlink(context_path)
        except OSError:
            pass


def do_dispatch(env: Environment, bundle_arg: str, allow_migration: bool) -> int:
    status = Status(env, None)
    try:
        layout, _ = check_policy_and_layout(env, allow_migration)
        bundle, commit, manifest = check_bundle(env, bundle_arg)
        check_helper_contract(env, bundle)
        check_restore_transactions(env)           # advisory here (the run asks again under the locks), so that the page hears it before a unit is started
    except Refused as refusal:
        status.write("error", refusal.phase, refusal.message, EXIT_REFUSED, True)
        print(f"[saturn-release-update] REFUSED ({refusal.phase}): {refusal.message}", file=sys.stderr)
        return EXIT_REFUSED
    unit = f"saturn-release-update-{time.strftime('%Y%m%d%H%M%S')}"
    command = [env.systemd_run, "--unit", unit, "--collect", "--no-block", f"--property=RuntimeMaxSec={RUN_TIMEOUT_SECONDS}", env.self_path, "--run"]
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
        check_restore_transactions(env)
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
    group.add_argument("--finish", metavar="CONTEXT", help=argparse.SUPPRESS)
    parser.add_argument("--activator-rc", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--allow-legacy-migration", action="store_true")
    parser.add_argument("bundle", nargs="?")
    args = parser.parse_args(argv)
    env = Environment()
    if args.finish is not None:
        if args.bundle or args.activator_rc is None:
            parser.error("--finish takes a context file and --activator-rc, nothing else")
        if env.root and not os.environ.get("INVOCATION_ID"):
            print("[saturn-release-update] --finish belongs to the transient update unit", file=sys.stderr)
            return EXIT_REFUSED
        return do_finish(env, args.finish, args.activator_rc)
    if args.activator_rc is not None:
        parser.error("--activator-rc belongs to --finish")
    if args.preflight:
        if args.bundle:
            parser.error("--preflight takes no bundle")
        return do_preflight(env, args.allow_legacy_migration)
    if not args.bundle:
        parser.error("a bundle directory is required")
    if args.dispatch:
        return do_dispatch(env, args.bundle, args.allow_legacy_migration)
    if env.root and not os.environ.get("INVOCATION_ID"):
        print("[saturn-release-update] --run is for the transient update unit (a terminal session would end with the connection); use --dispatch", file=sys.stderr)
        return EXIT_REFUSED
    return do_run(env, args.bundle, args.allow_legacy_migration)


if __name__ == "__main__":
    sys.exit(main())
