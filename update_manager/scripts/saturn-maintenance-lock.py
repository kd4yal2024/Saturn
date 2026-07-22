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
            sub.add_argument("command", nargs=argparse.REMAINDER)
    return result


def open_output_file(value: str | None):
    if not value:
        return None
    path = Path(value)
    if not path.is_absolute() or not path.parent.is_dir() or path.parent.is_symlink():
        fail(f"job output parent is missing or unsafe: {path.parent}", 2)
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0)
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


def write_result(path_value: str | None, job_id: str | None, exit_code: int) -> None:
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
    environment = os.environ.copy()
    environment["SATURN_MAINTENANCE_LOCK_HELD"] = "1"
    environment["SATURN_MAINTENANCE_LOCK_OPERATION"] = args.operation
    environment["SATURN_MAINTENANCE_LOCK_RESOURCES"] = ",".join(resources)
    output_stream = open_output_file(args.output_file)
    child = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE if output_stream else None,
        stderr=subprocess.STDOUT if output_stream else None,
        pass_fds=tuple(descriptors),
    )

    def forward(signum: int, _frame: object) -> None:
        if child.poll() is None:
            child.send_signal(signum)

    signal.signal(signal.SIGTERM, forward)
    signal.signal(signal.SIGINT, forward)
    stdout_available = True
    if output_stream and child.stdout:
        while True:
            chunk = child.stdout.read(65536)
            if not chunk:
                break
            output_stream.write(chunk)
            if stdout_available:
                try:
                    sys.stdout.buffer.write(chunk)
                    sys.stdout.buffer.flush()
                except BrokenPipeError:
                    stdout_available = False
        output_stream.flush()
        os.fsync(output_stream.fileno())
        output_stream.close()
    exit_code = child.wait()
    write_result(args.result_file, args.job_id, exit_code)
    return exit_code


def raise_exit() -> None:
    raise SystemExit(0)


if __name__ == "__main__":
    raise SystemExit(main())
