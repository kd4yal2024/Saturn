#!/usr/bin/env python3
"""Host-level resource locks for Saturn maintenance operations.

The lock owner is this process.  In ``run`` mode it starts and waits for the
maintenance command, so the locks remain held if the Saturn Go process that
launched it restarts.
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


def open_lock_files(lock_dir: Path, resources: list[str], create: bool) -> list[int]:
    if not lock_dir.is_dir() or lock_dir.is_symlink():
        fail(f"lock directory is missing or unsafe: {lock_dir}")

    descriptors: list[int] = []
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
                os.close(fd)
                fail(f"resource is busy: {resource}", EXIT_CONFLICT)
            os.set_inheritable(fd, True)
            descriptors.append(fd)
    except BaseException:
        for fd in descriptors:
            os.close(fd)
        raise
    return descriptors


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--lock-dir",
        default=os.environ.get("SATURN_MAINTENANCE_LOCK_DIR", "/run/lock/saturn-maintenance"),
    )
    result.add_argument("--create", action="store_true", help=argparse.SUPPRESS)
    subparsers = result.add_subparsers(dest="action", required=True)
    for action in ("probe", "hold", "run"):
        sub = subparsers.add_parser(action)
        sub.add_argument("--operation", required=True)
        sub.add_argument("--resources", required=True)
        if action == "run":
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
    resources = parse_resources(args.resources)
    descriptors = open_lock_files(Path(args.lock_dir), resources, args.create)
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
