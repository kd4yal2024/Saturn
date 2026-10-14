#!/usr/bin/env python3
"""Host-level resource locks for Saturn maintenance operations.

The lock owner is this process.  In ``run`` mode it starts and waits for the
maintenance command, so the locks remain held if the Saturn Go process that
launched it restarts.

One call is answered while a release update holds locks SHARED: Saturn Go's
own startup recovery.  A managed update (saturn-release-update-root.py) holds
``release`` exclusively and ``radio`` and ``repository`` shared for its whole
run, rollback included, so that no other maintenance can start in between; the
activation restarts Saturn Go, whose startup runs

    run --operation restore:recover --resources repository,radio --
        saturn-restore-transaction.py recover --state-root <root>

and exits when it cannot take those locks.  The recovery proper WRITES (it
restores or deletes settings, moves the repository pointer, removes staging
directories and rewrites the transaction records), and under a shared hold
nothing keeps two of them, or one of them and the update, from writing at once.
So while an update holds the locks this tool does not run that command, or any
other: it answers that exact call (the operation, those two resources, that
helper's name, ``recover``, ``--state-root`` and an absolute path, nothing
else) with a READ-ONLY CHECK of its own, one caller at a time, that reports
whether any restore transaction is still incomplete.  A pending transaction is
reported (exit 1) and left as it is: the update broker refuses to start while
one is pending, so that the recovery proper only ever runs under the exclusive
locks, outside an update.  Every other request is exclusive and is refused (75)
while the update holds them.  ``capabilities`` tells the update broker whether
this tool does that; ``pending-restores`` is the broker's own read-only
question.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path


RESOURCE_ORDER = (
    "release",
    "repository",
    "disk",
    "fpga",
    "package",
    "network",
    "radio",
    "read-only",
)
EXIT_CONFLICT = 75
PENDING_EXIT = 1
# Saturn Go's startup recovery (rust-server/src/restore.rs: recover_restore_transactions) and the only locks it asks for.  While a release update holds
# them SHARED that exact call is answered by a read-only check (see the module docstring); nothing it carries is run.
RECOVERY_OPERATION = "restore:recover"
RECOVERY_RESOURCES = frozenset({"repository", "radio"})
RESTORE_HELPER_NAME = "saturn-restore-transaction.py"
RECOVERY_GATE_WAIT_SECONDS = 10.0
# What saturn-restore-transaction.py writes and what its `recover` leaves alone (recover_transaction() returns early for these two statuses).  A test holds
# these equal to the helper's, and holds this module's verdict equal to the helper's on every kind of leftover.
TRANSACTION_FORMAT = "saturn-restore-transaction"
TRANSACTION_SCHEMA_VERSION = 1
FINISHED_TRANSACTIONS = frozenset({"committed", "rolled_back"})
MAX_TRANSACTION_RECORD_BYTES = 1024 * 1024
CAPABILITIES = {"schema": 1, "recovery_check": 1}
DEFAULT_OUTPUT_MAX_BYTES = 4 * 1024 * 1024
DEFAULT_OUTPUT_MAX_LINES = 5000
MAX_TIMEOUT_SECONDS = 6 * 60 * 60
TIMEOUT_TERM_GRACE_SECONDS = 5
OUTPUT_TRUNCATION_MARKER = (
    b"[output truncated: durable log byte/line limit reached; further output omitted]\n"
)


def fail(message: str, status: int = 1) -> None:
    print(f"saturn-maintenance-lock: {message}", file=sys.stderr)
    raise SystemExit(status)


def parse_resources(value: str) -> list[str]:
    requested = {part.strip().lower() for part in value.split(",") if part.strip()}
    unknown = requested.difference(RESOURCE_ORDER)
    if unknown:
        fail(f"unknown resource class: {sorted(unknown)[0]}", 2)
    if not requested:
        fail("at least one resource class is required", 2)
    return [resource for resource in RESOURCE_ORDER if resource in requested]


def release_run_is_active(lock_dir: Path) -> bool:
    """True while some process holds the release lock (a managed update holds it for its whole run).  Looked at with a shared lock that is let go at once."""
    try:
        fd = os.open(lock_dir / "release.lock", os.O_RDWR | getattr(os, "O_NOFOLLOW", 0))
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
    except BlockingIOError:
        return True
    finally:
        os.close(fd)
    return False


def open_lock_files(lock_dir: Path, resources: list[str], create: bool, recovery: bool = False) -> tuple[list[int], bool]:
    """Take the locks (exclusive, except ``read-only``).  Returns the descriptors and whether any was only JOINED, shared: that happens for ``recovery`` (the one
    call described in the module docstring) while a release update holds the locks, and the caller then performs the read-only check instead of its command."""
    if not lock_dir.is_dir() or lock_dir.is_symlink():
        fail(f"lock directory is missing or unsafe: {lock_dir}")

    descriptors: list[int] = []
    joined = False
    flags = os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
    if create:
        flags |= os.O_CREAT
    try:
        for resource in resources:
            path = lock_dir / f"{resource}.lock"
            try:
                fd = os.open(path, flags, 0o660)
            except OSError as error:
                fail(f"cannot open lock file {path}: {error.strerror}")
            metadata = os.fstat(fd)
            if not stat.S_ISREG(metadata.st_mode):
                os.close(fd)
                fail(f"lock path is not a regular file: {path}")
            operation = fcntl.LOCK_SH if resource == "read-only" else fcntl.LOCK_EX
            try:
                fcntl.flock(fd, operation | fcntl.LOCK_NB)
            except BlockingIOError:
                # Saturn Go's startup recovery may look at a release update's SHARED hold (never an exclusive one: a shared lock fails against any exclusive holder)
                shared = False
                if recovery and operation == fcntl.LOCK_EX and release_run_is_active(lock_dir):
                    try:
                        fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
                        shared = True
                    except BlockingIOError:
                        pass
                if not shared:
                    os.close(fd)
                    fail(f"resource is busy: {resource}", EXIT_CONFLICT)
                joined = True
            os.set_inheritable(fd, True)
            descriptors.append(fd)
    except BaseException:
        for fd in descriptors:
            os.close(fd)
        raise
    return descriptors, joined


def recovery_request(args: argparse.Namespace, resources: list[str]) -> Path | None:
    """The state root of Saturn Go's startup recovery if, and only if, this IS that call, exactly as rust-server/src/maintenance_lock.rs wrapped_command builds
    it for restore.rs: no job options, no deadline, and the command is the restore helper (by name) with ``recover --state-root <absolute path>`` and nothing
    else.  The command is never run on the strength of this: it only decides whether the call may be answered by the check."""
    if args.action != "run" or args.operation != RECOVERY_OPERATION or set(resources) != RECOVERY_RESOURCES:
        return None
    if args.job_id or args.output_file or args.result_file or args.timeout_seconds:
        return None
    command = list(args.command)
    if command and command[0] == "--":
        command.pop(0)
    if len(command) != 4:
        return None
    helper, verb, flag, root = command
    if os.path.basename(helper) != RESTORE_HELPER_NAME or verb != "recover" or flag != "--state-root":
        return None
    if not os.path.isabs(root) or "\0" in root:
        return None
    return Path(root)


def transaction_problem(path: Path, name: str) -> str | None:
    """Why the recovery proper would have to act on this entry (or could not read it), or None when it would leave it alone.  Mirrors recover() and
    recover_transaction() of saturn-restore-transaction.py, and reads only."""
    try:
        info = os.lstat(path)
    except OSError as error:
        return f"unreadable ({error.strerror})"
    if not stat.S_ISDIR(info.st_mode):
        return "not a real directory"
    try:
        fd = os.open(path / "transaction.json", os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        return f"no readable transaction record ({error.strerror})"
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            return "transaction record is not a regular file"
        raw = os.read(fd, MAX_TRANSACTION_RECORD_BYTES + 1)
    except OSError as error:
        return f"unreadable transaction record ({error.strerror})"
    finally:
        os.close(fd)
    if len(raw) > MAX_TRANSACTION_RECORD_BYTES:
        return "transaction record is too large"
    try:
        record = json.loads(raw)
    except ValueError:
        return "transaction record is not valid JSON"
    if (not isinstance(record, dict) or record.get("format") != TRANSACTION_FORMAT
            or record.get("schema_version") != TRANSACTION_SCHEMA_VERSION or record.get("id") != name):
        return "invalid transaction record"
    status = record.get("status")
    if isinstance(status, str) and status in FINISHED_TRANSACTIONS:
        return None
    return f"incomplete (status {str(status)[:40]})"


def pending_restore_transactions(state_root: Path) -> list[str]:
    """One line per restore transaction under ``state_root`` that is not finished (or not readable); empty when the recovery proper has nothing to do.  Read-only:
    it creates, locks and writes nothing."""
    directory = state_root / "restore-transactions"
    try:
        info = os.lstat(directory)
    except FileNotFoundError:
        return []
    except OSError as error:
        return [f"{directory}: {error.strerror}"]
    if not stat.S_ISDIR(info.st_mode):
        return [f"{directory}: not a real directory"]
    try:
        names = sorted(os.listdir(directory))
    except OSError as error:
        return [f"{directory}: {error.strerror}"]
    problems = []
    for name in names:
        problem = transaction_problem(directory / name, name)
        if problem:
            problems.append(f"{name}: {problem}")
    return problems


def report_pending_restores(state_root: str) -> int:
    if not os.path.isabs(state_root):
        fail("pending-restores needs an absolute --state-root", 2)
    problems = pending_restore_transactions(Path(state_root))
    print(json.dumps({"status": "pending" if problems else "ok", "pending": problems}, separators=(",", ":")))
    return PENDING_EXIT if problems else 0


def recovery_check(state_root: Path, wait: float) -> int:
    """Answer Saturn Go's startup recovery while a release update holds the locks: one caller at a time (the gate is an exclusive flock on the transaction
    directory, which only this check uses), and read-only.  Exit 0 and the JSON Saturn Go reads when nothing is pending; exit 1 and nothing on stdout when
    something is (Saturn Go then does not start, which the update's own refusal to begin with a pending transaction makes unreachable); 75 when another
    check does not finish within ``wait`` seconds."""
    directory = state_root / "restore-transactions"
    gate = None
    try:
        gate = os.open(directory, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0))
    except OSError:
        pass        # missing: nothing to recover and nothing to serialize; anything else that is not a real directory is reported by the listing below
    try:
        if gate is not None:
            deadline = time.monotonic() + max(0.0, wait)
            while True:
                try:
                    fcntl.flock(gate, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        fail("another restore recovery check is running; try again", EXIT_CONFLICT)
                    time.sleep(0.05)
        problems = pending_restore_transactions(state_root)
    finally:
        if gate is not None:
            os.close(gate)
    if problems:
        fail("a release update is running and these restore transactions are incomplete: " + "; ".join(problems) + ".  Recovery writes, so it is not run under the "
             "update's shared locks and nothing was changed; it is completed by Saturn Go's startup outside an update", PENDING_EXIT)
    print(json.dumps({"status": "ok", "recovered": [], "checked_only": True,
                      "note": "a release update holds the locks; nothing is pending and nothing was changed"}, separators=(",", ":")), flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--lock-dir",
        default=os.environ.get("SATURN_MAINTENANCE_LOCK_DIR", "/run/lock/saturn-maintenance"),
    )
    result.add_argument("--create", action="store_true", help=argparse.SUPPRESS)
    subparsers = result.add_subparsers(dest="action", required=True)
    subparsers.add_parser("capabilities", help="print what this tool can do, as JSON, and take no lock")
    pending = subparsers.add_parser("pending-restores", help="list the restore transactions that are not finished (read-only, takes no lock); exit 1 if any")
    pending.add_argument("--state-root", required=True)
    for action in ("probe", "hold", "run"):
        sub = subparsers.add_parser(action)
        sub.add_argument("--operation", required=True)
        sub.add_argument("--resources", required=True)
        if action == "run":
            sub.add_argument("--recovery-gate-wait", type=float, default=RECOVERY_GATE_WAIT_SECONDS, help=argparse.SUPPRESS)
            sub.add_argument("--job-id")
            sub.add_argument("--output-file")
            sub.add_argument("--result-file")
            sub.add_argument("--output-max-bytes", type=int, default=DEFAULT_OUTPUT_MAX_BYTES)
            sub.add_argument("--output-max-lines", type=int, default=DEFAULT_OUTPUT_MAX_LINES)
            sub.add_argument("--timeout-seconds", type=int, default=0)
            sub.add_argument("command", nargs=argparse.REMAINDER)
    return result


def open_output_file(value: str | None):
    if not value:
        return None
    path = Path(value)
    if not path.is_absolute() or not path.parent.is_dir() or path.parent.is_symlink():
        fail(f"job output parent is missing or unsafe: {path.parent}", 2)
    flags = os.O_RDWR | os.O_CREAT | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags, 0o640)
    except OSError as error:
        fail(f"cannot open job output {path}: {error.strerror}")
    metadata = os.fstat(fd)
    if not stat.S_ISREG(metadata.st_mode):
        os.close(fd)
        fail(f"job output is not a regular file: {path}")
    os.fchmod(fd, 0o640)
    return os.fdopen(fd, "ab", buffering=0)


class BoundedOutput:
    def __init__(self, stream, max_bytes: int, max_lines: int) -> None:
        if max_bytes < len(OUTPUT_TRUNCATION_MARKER) + 1:
            fail("output byte limit is too small", 2)
        if max_lines < 1:
            fail("output line limit must be positive", 2)
        self.stream = stream
        self.max_payload_bytes = max_bytes - len(OUTPUT_TRUNCATION_MARKER)
        self.max_payload_lines = max_lines - 1
        existing_size = os.fstat(stream.fileno()).st_size
        existing = os.pread(stream.fileno(), min(existing_size, max_bytes), 0)
        self.retained_bytes = existing_size
        self.retained_lines = existing.count(b"\n")
        self.truncated = OUTPUT_TRUNCATION_MARKER.strip() in existing

    def write(self, chunk: bytes) -> None:
        if self.truncated:
            return
        retained = bytearray()
        retained_newlines = 0
        for value in chunk:
            if (
                self.retained_bytes + len(retained) >= self.max_payload_bytes
                or self.retained_lines + retained_newlines >= self.max_payload_lines
            ):
                self._mark_truncated(retained, retained_newlines)
                return
            retained.append(value)
            if value == 0x0A:
                retained_newlines += 1
        if retained:
            self.stream.write(retained)
            self.retained_bytes += len(retained)
            self.retained_lines += retained_newlines

    def _mark_truncated(self, retained: bytearray, retained_newlines: int) -> None:
        if retained:
            self.stream.write(retained)
            self.retained_bytes += len(retained)
            self.retained_lines += retained_newlines
        self.stream.write(OUTPUT_TRUNCATION_MARKER)
        self.truncated = True

    def close(self) -> None:
        self.stream.flush()
        os.fsync(self.stream.fileno())
        self.stream.close()


def write_result(
    path_value: str | None, job_id: str | None, exit_code: int, timed_out: bool = False
) -> None:
    if not path_value:
        return
    path = Path(path_value)
    if not path.is_absolute() or not path.parent.is_dir() or path.parent.is_symlink():
        fail(f"job result parent is missing or unsafe: {path.parent}", 2)
    if path.exists() and (path.is_symlink() or not path.is_file()):
        fail(f"job result path is unsafe: {path}")
    payload = {
        "job_id": job_id or "",
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "exit_code": exit_code,
        "timed_out": timed_out,
    }
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        os.fchmod(fd, 0o640)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(payload, stream, separators=(",", ":"))
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def main() -> int:
    args = parser().parse_args()
    if args.action == "capabilities":
        print(json.dumps(CAPABILITIES, separators=(",", ":")))
        return 0
    if args.action == "pending-restores":
        return report_pending_restores(args.state_root)
    resources = parse_resources(args.resources)
    state_root = recovery_request(args, resources)
    descriptors, joined = open_lock_files(Path(args.lock_dir), resources, args.create, state_root is not None)
    if joined:
        return recovery_check(state_root, args.recovery_gate_wait)
    payload = {
        "status": "locked",
        "operation": args.operation,
        "resources": resources,
        "pid": os.getpid(),
    }

    if args.action == "probe":
        print(json.dumps(payload, separators=(",", ":")), flush=True)
        return 0

    if args.action == "hold":
        print(json.dumps(payload, separators=(",", ":")), flush=True)
        signal.signal(signal.SIGTERM, lambda _signum, _frame: raise_exit())
        while sys.stdin.buffer.read(65536):
            pass
        return 0

    command = list(args.command)
    if command and command[0] == "--":
        command.pop(0)
    if not command:
        fail("run requires a command after --", 2)
    if args.timeout_seconds < 0 or args.timeout_seconds > MAX_TIMEOUT_SECONDS:
        fail(f"timeout must be between 0 and {MAX_TIMEOUT_SECONDS} seconds", 2)
    environment = os.environ.copy()
    environment["SATURN_MAINTENANCE_LOCK_HELD"] = "1"
    environment["SATURN_MAINTENANCE_LOCK_OPERATION"] = args.operation
    environment["SATURN_MAINTENANCE_LOCK_RESOURCES"] = ",".join(resources)
    output_stream = open_output_file(args.output_file)
    bounded_output = (
        BoundedOutput(
            output_stream,
            args.output_max_bytes,
            args.output_max_lines,
        )
        if output_stream and args.output_file
        else None
    )
    child = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE if output_stream else None,
        stderr=subprocess.STDOUT if output_stream else None,
        pass_fds=tuple(descriptors),
    )

    def forward(signum: int, _frame: object) -> None:
        if child.poll() is None:
            try:
                child.send_signal(signum)
            except ProcessLookupError:
                pass

    signal.signal(signal.SIGTERM, forward)
    signal.signal(signal.SIGINT, forward)
    timed_out = threading.Event()

    def expire() -> None:
        if child.poll() is not None:
            return
        timed_out.set()
        try:
            os.killpg(os.getpgrp(), signal.SIGTERM)
        except ProcessLookupError:
            return
        time.sleep(TIMEOUT_TERM_GRACE_SECONDS)
        if child.poll() is None:
            kill_group_members_except_self(os.getpgrp())

    deadline_timer = None
    if args.timeout_seconds:
        deadline_timer = threading.Timer(args.timeout_seconds, expire)
        deadline_timer.daemon = True
        deadline_timer.start()
    stdout_available = True
    if output_stream and child.stdout:
        while True:
            chunk = child.stdout.read(65536)
            if not chunk:
                break
            bounded_output.write(chunk)
            if stdout_available:
                try:
                    sys.stdout.buffer.write(chunk)
                    sys.stdout.buffer.flush()
                except BrokenPipeError:
                    stdout_available = False
        bounded_output.close()
    exit_code = child.wait()
    if deadline_timer:
        deadline_timer.cancel()
    write_result(args.result_file, args.job_id, exit_code, timed_out.is_set())
    return exit_code


def kill_group_members_except_self(process_group: int) -> None:
    own_pid = os.getpid()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid == own_pid:
            continue
        try:
            stat_fields = (entry / "stat").read_text(encoding="utf-8").rsplit(")", 1)[1].split()
            member_group = int(stat_fields[2])
        except (FileNotFoundError, IndexError, PermissionError, ValueError):
            continue
        if member_group == process_group:
            try:
                os.kill(pid, signal.SIGKILL)
            except (PermissionError, ProcessLookupError):
                pass


def raise_exit() -> None:
    raise SystemExit(0)


if __name__ == "__main__":
    raise SystemExit(main())
