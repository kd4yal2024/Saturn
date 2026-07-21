#!/usr/bin/env python3
"""Validate and atomically replace a bounded Saturn JSON state document."""

from __future__ import annotations

import argparse
import json
import os
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any


MAX_JSON_BYTES = 16 * 1024 * 1024


class StateWriteError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise StateWriteError(message)


def parse_mode(value: str) -> int:
    try:
        mode = int(value, 8)
    except ValueError:
        raise argparse.ArgumentTypeError("mode must be an octal value") from None
    if mode < 0 or mode > 0o7777:
        raise argparse.ArgumentTypeError("mode must be between 0000 and 7777")
    return mode


def ensure_real_parent(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o750)
    metadata = path.parent.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        fail(f"state parent must be a real directory: {path.parent}")
    return path.parent.resolve()


def target_identity(path: Path, owner: int | None, group: int | None) -> tuple[int, int]:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return (
            os.geteuid() if owner is None else owner,
            os.getegid() if group is None else group,
        )
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"state target must be a regular file: {path}")
    return (
        metadata.st_uid if owner is None else owner,
        metadata.st_gid if group is None else group,
    )


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def atomic_replace(
    path: Path,
    content: bytes,
    mode: int,
    owner: int | None,
    group: int | None,
    fault: str,
) -> None:
    parent = ensure_real_parent(path)
    uid, gid = target_identity(path, owner, group)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=parent)
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, mode)
        if os.fstat(descriptor).st_uid != uid or os.fstat(descriptor).st_gid != gid:
            os.fchown(descriptor, uid, gid)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        metadata = temporary.stat()
        if stat.S_IMODE(metadata.st_mode) != mode or metadata.st_uid != uid or metadata.st_gid != gid:
            fail(f"temporary state identity verification failed: {temporary}")
        if fault == "before-rename":
            fail("injected state-write failure before rename")
        os.replace(temporary, path)
        fsync_directory(parent)
        if fault == "after-rename":
            fail("injected state-write failure after rename")
    finally:
        temporary.unlink(missing_ok=True)


def canonical_json(raw: bytes) -> bytes:
    if len(raw) > MAX_JSON_BYTES:
        fail(f"JSON state document exceeds {MAX_JSON_BYTES} bytes")
    try:
        value: Any = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON state document: {error}")
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--path", required=True, type=Path)
    parser.add_argument("--mode", default=0o640, type=parse_mode)
    parser.add_argument("--owner", type=int)
    parser.add_argument("--group", type=int)
    parser.add_argument("--last-good", action="store_true")
    parser.add_argument(
        "--fault",
        choices=("none", "before-rename", "after-rename"),
        default="none",
        help=argparse.SUPPRESS,
    )
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    content = canonical_json(sys.stdin.buffer.read(MAX_JSON_BYTES + 1))
    if arguments.last_good:
        last_good = arguments.path.with_name(f"{arguments.path.name}.last-good")
        atomic_replace(
            last_good,
            content,
            arguments.mode,
            arguments.owner,
            arguments.group,
            "none",
        )
    atomic_replace(
        arguments.path,
        content,
        arguments.mode,
        arguments.owner,
        arguments.group,
        arguments.fault,
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except StateWriteError as error:
        print(f"saturn-state-write: ERROR: {error}", file=sys.stderr)
        raise SystemExit(1) from error
