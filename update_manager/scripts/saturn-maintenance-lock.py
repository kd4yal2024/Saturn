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
            sub.add_argument("command", nargs=argparse.REMAINDER)
    return result


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
    child = subprocess.Popen(command, env=environment)

    def forward(signum: int, _frame: object) -> None:
        if child.poll() is None:
            child.send_signal(signum)

    signal.signal(signal.SIGTERM, forward)
    signal.signal(signal.SIGINT, forward)
    return child.wait()


def raise_exit() -> None:
    raise SystemExit(0)


if __name__ == "__main__":
    raise SystemExit(main())
