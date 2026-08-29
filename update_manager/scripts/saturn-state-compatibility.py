#!/usr/bin/env python3
"""Preflight, snapshot, migrate, and restore Saturn persistent application state."""

from __future__ import annotations

import argparse
import grp
import hashlib
import json
import os
import re
import shutil
import stat
import tempfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any

RELEASE_FORMAT = "saturn-application-release"
STATE_CONTRACT_FORMAT = "saturn-state-compatibility"
STATE_CONTRACT_SCHEMA_VERSION = 1
STATE_MARKER_FORMAT = "saturn-persistent-state"
STATE_MARKER_SCHEMA_VERSION = 1
BACKUP_FORMAT = "saturn-state-backup"
BACKUP_SCHEMA_VERSION = 1
STATE_MARKER_NAME = "state-schema.json"
BACKUP_MANIFEST_NAME = "backup-manifest.json"
MAX_MANAGED_FILE_BYTES = 16 * 1024 * 1024
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
LEGACY_MANAGED_PATHS = [
    "custom_scripts.json",
    "performance_benchmarks.json",
    "remote_profiles.json",
    "remote_settings.json",
    "repo_root.txt",
    "saturngo_update_policy.json",
    "update_policy.json",
]


class StateError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise StateError(message)


def load_json(path: Path) -> Any:
    try:
        with path.open(encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read JSON {path}: {error}")


def safe_managed_path(value: str) -> str:
    path = PurePosixPath(value)
    if (
        path.is_absolute()
        or len(path.parts) != 1
        or any(part in ("", ".", "..") for part in path.parts)
        or value == STATE_MARKER_NAME
        or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", value)
    ):
        fail(f"unsafe managed state path: {value!r}")
    return path.as_posix()


def validate_contract(value: Any, *, legacy: bool = False) -> dict[str, Any]:
    if legacy:
        return {
            "format": STATE_CONTRACT_FORMAT,
            "contract_schema_version": STATE_CONTRACT_SCHEMA_VERSION,
            "state_schema_version": 0,
            "readable_state_schema_versions": [0, 1],
            "migration": None,
            "managed_paths": LEGACY_MANAGED_PATHS,
            "legacy_manifest": True,
        }
    if not isinstance(value, dict):
        fail("release state compatibility contract must be an object")
    if (
        value.get("format") != STATE_CONTRACT_FORMAT
        or value.get("contract_schema_version") != STATE_CONTRACT_SCHEMA_VERSION
    ):
        fail("unsupported release state compatibility contract")
    state_version = value.get("state_schema_version")
    readable = value.get("readable_state_schema_versions")
    migration = value.get("migration")
    managed = value.get("managed_paths")
    if not isinstance(state_version, int) or isinstance(state_version, bool) or state_version < 1:
        fail("release state schema version must be a positive integer")
    if (
        not isinstance(readable, list)
        or any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in readable)
        or len(readable) != len(set(readable))
        or not {state_version - 1, state_version}.issubset(set(readable))
    ):
        fail("release must read its own and immediately preceding state schema")
    if not isinstance(migration, dict):
        fail("release migration contract must be an object")
    migration_from = migration.get("from_state_schema_versions")
    if (
        not isinstance(migration_from, list)
        or any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in migration_from)
        or len(migration_from) != len(set(migration_from))
        or state_version - 1 not in migration_from
    ):
        fail("release migration sources do not include the preceding state schema")
    if migration.get("kind") != "metadata-only":
        fail("only metadata-only migrations are currently supported")
    if not isinstance(migration.get("one_way"), bool):
        fail("release migration one_way flag must be boolean")
    documentation = migration.get("documentation")
    if not isinstance(documentation, str) or not documentation.strip():
        fail("release migration documentation is required")
    if not isinstance(managed, list) or not managed or any(not isinstance(item, str) for item in managed):
        fail("release managed state paths must be a non-empty array")
    managed_paths = sorted(safe_managed_path(item) for item in managed)
    if len(managed_paths) != len(set(managed_paths)):
        fail("release managed state paths must be unique")
    result = dict(value)
    result["readable_state_schema_versions"] = sorted(readable)
    result["managed_paths"] = managed_paths
    normalized_migration = dict(migration)
    normalized_migration["from_state_schema_versions"] = sorted(migration_from)
    normalized_migration["documentation"] = documentation.strip()
    result["migration"] = normalized_migration
    return result


def release_contract(release_root: Path) -> dict[str, Any]:
    manifest = load_json(release_root / "release-manifest.json")
    if not isinstance(manifest, dict) or manifest.get("format") != RELEASE_FORMAT:
        fail(f"invalid Saturn release manifest: {release_root}")
    schema = manifest.get("schema_version")
    if schema == 1:
        return validate_contract(None, legacy=True)
    if schema in (2, 3):
        return validate_contract(manifest.get("state_compatibility"))
    fail(f"unsupported release manifest schema: {schema!r}")


def state_root(path: Path) -> Path:
    if not path.is_absolute():
        fail("state root must be absolute")
    if path.is_symlink() or not path.is_dir():
        fail(f"state root must be a real directory: {path}")
    return path.resolve()


def current_state_version(root: Path) -> int:
    marker = root / STATE_MARKER_NAME
    if not marker.exists() and not marker.is_symlink():
        return 0
    if marker.is_symlink() or not marker.is_file():
        fail(f"state schema marker must be a regular file: {marker}")
    value = load_json(marker)
    if (
        not isinstance(value, dict)
        or value.get("format") != STATE_MARKER_FORMAT
        or value.get("schema_version") != STATE_MARKER_SCHEMA_VERSION
        or not isinstance(value.get("state_schema_version"), int)
        or isinstance(value.get("state_schema_version"), bool)
        or value["state_schema_version"] < 1
    ):
        fail(f"invalid state schema marker: {marker}")
    return value["state_schema_version"]


def compatibility_plan(
    root: Path,
    target_release: Path,
    previous_release: Path | None,
    approve_one_way: bool,
) -> dict[str, Any]:
    current = current_state_version(root)
    target = release_contract(target_release)
    target_version = target["state_schema_version"]
    if current not in target["readable_state_schema_versions"]:
        fail(f"target release cannot read current state schema {current}")
    migration_required = target_version > current
    if migration_required:
        if target_version != current + 1:
            fail("state migrations must advance exactly one schema version")
        migration = target["migration"]
        if current not in migration["from_state_schema_versions"]:
            fail(f"target release has no migration from state schema {current}")
    previous = release_contract(previous_release) if previous_release else validate_contract(None, legacy=True)
    rollback_safe = (not migration_required) or target_version in previous["readable_state_schema_versions"]
    one_way = bool(migration_required and not rollback_safe)
    if one_way:
        migration = target["migration"]
        if not migration["one_way"]:
            fail("deployment would make rollback unsafe and is not documented as one-way")
        if not approve_one_way:
            fail("documented one-way state migration requires explicit operator approval")
    elif approve_one_way:
        fail("one-way approval was supplied for a rollback-compatible migration")
    return {
        "format": "saturn-state-compatibility-plan",
        "schema_version": 1,
        "current_state_schema_version": current,
        "target_state_schema_version": target_version,
        "target_readable_state_schema_versions": target["readable_state_schema_versions"],
        "previous_readable_state_schema_versions": previous["readable_state_schema_versions"],
        "migration_required": migration_required,
        "migration_kind": target["migration"]["kind"] if migration_required else None,
        "rollback_safe": rollback_safe,
        "one_way_approved": one_way and approve_one_way,
        "migration_documentation": target["migration"]["documentation"] if migration_required else None,
        "managed_paths": target["managed_paths"],
    }


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def fsync_directory(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_json(path: Path, value: Any, mode: int, uid: int, gid: int) -> None:
    fd, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        if os.geteuid() == 0:
            os.chown(temporary, uid, gid)
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_copy(source: Path, destination: Path, mode: int, uid: int, gid: int) -> None:
    fd, temporary_name = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
    os.close(fd)
    temporary = Path(temporary_name)
    try:
        shutil.copyfile(source, temporary)
        with temporary.open("rb") as handle:
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        if os.geteuid() == 0:
            os.chown(temporary, uid, gid)
        os.replace(temporary, destination)
        fsync_directory(destination.parent)
    finally:
        temporary.unlink(missing_ok=True)


def group_gid(name: str) -> int:
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name):
        fail(f"unsafe state group: {name!r}")
    try:
        return grp.getgrnam(name).gr_gid
    except KeyError:
        fail(f"state group does not exist: {name}")


def safe_existing_file(path: Path) -> os.stat_result | None:
    if not path.exists() and not path.is_symlink():
        return None
    if path.is_symlink() or not path.is_file():
        fail(f"managed state entry must be a regular file: {path}")
    metadata = path.stat()
    if metadata.st_size > MAX_MANAGED_FILE_BYTES:
        fail(f"managed state file exceeds {MAX_MANAGED_FILE_BYTES} bytes: {path}")
    return metadata


def create_backup(
    root: Path,
    backup_root: Path,
    target_commit: str,
    plan: dict[str, Any],
    gid: int,
) -> Path:
    if not COMMIT_RE.fullmatch(target_commit):
        fail("target commit must be one lowercase full Git commit")
    if not backup_root.is_absolute() or backup_root.is_symlink():
        fail("state backup root must be an absolute real directory")
    backup_root.mkdir(parents=True, exist_ok=True, mode=0o750)
    if backup_root.is_symlink() or not backup_root.is_dir():
        fail("state backup root must be an absolute real directory")
    backup_root = backup_root.resolve()
    os.chmod(backup_root, 0o750)
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    backup = backup_root / f"{timestamp}-{target_commit}"
    backup.mkdir(mode=0o750)
    if os.geteuid() == 0:
        os.chown(backup_root, 0, gid)
        os.chown(backup, 0, gid)
    try:
        entries: dict[str, Any] = {}
        paths = [STATE_MARKER_NAME, *plan["managed_paths"]]
        for index, relative in enumerate(paths):
            source = root / relative
            metadata = safe_existing_file(source)
            record: dict[str, Any] = {"present": metadata is not None}
            if metadata is not None:
                backup_name = f"{index:03d}.data"
                destination = backup / backup_name
                shutil.copyfile(source, destination)
                os.chmod(destination, 0o640)
                if os.geteuid() == 0:
                    os.chown(destination, 0, gid)
                with destination.open("rb") as handle:
                    os.fsync(handle.fileno())
                record.update(
                    {
                        "backup_file": backup_name,
                        "sha256": sha256_file(destination),
                        "size": destination.stat().st_size,
                        "mode": stat.S_IMODE(metadata.st_mode),
                        "uid": metadata.st_uid,
                        "gid": metadata.st_gid,
                    }
                )
            entries[relative] = record
        manifest = {
            "format": BACKUP_FORMAT,
            "schema_version": BACKUP_SCHEMA_VERSION,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "state_root": str(root),
            "target_commit": target_commit,
            "from_state_schema_version": plan["current_state_schema_version"],
            "to_state_schema_version": plan["target_state_schema_version"],
            "managed_paths": plan["managed_paths"],
            "entries": entries,
        }
        atomic_json(backup / BACKUP_MANIFEST_NAME, manifest, 0o640, 0, gid)
        fsync_directory(backup)
        fsync_directory(backup_root)
        return backup
    except Exception:
        shutil.rmtree(backup, ignore_errors=True)
        fsync_directory(backup_root)
        raise


def migrate(args: argparse.Namespace) -> dict[str, Any]:
    root = state_root(args.state_root)
    previous = args.previous_release.resolve() if args.previous_release else None
    plan = compatibility_plan(root, args.target_release.resolve(), previous, args.approve_one_way)
    if not plan["migration_required"]:
        return {"plan": plan, "backup_directory": None, "migrated": False}
    gid = group_gid(args.state_group)
    backup = create_backup(root, args.backup_root, args.target_commit, plan, gid)
    marker = {
        "format": STATE_MARKER_FORMAT,
        "schema_version": STATE_MARKER_SCHEMA_VERSION,
        "state_schema_version": plan["target_state_schema_version"],
        "migrated_from_state_schema_version": plan["current_state_schema_version"],
        "migration_kind": plan["migration_kind"],
        "migration_documentation": plan["migration_documentation"],
        "release_commit": args.target_commit,
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }
    try:
        atomic_json(root / STATE_MARKER_NAME, marker, 0o640, 0, gid)
    except Exception as migration_error:
        restore_args = argparse.Namespace(
            state_root=root,
            backup_root=args.backup_root,
            backup_directory=backup,
        )
        try:
            restore(restore_args)
        except Exception as restore_error:
            fail(
                "state migration failed and its backup could not be restored: "
                f"{migration_error}; restore error: {restore_error}"
            )
        fail(f"state migration failed; the pre-migration backup was restored: {migration_error}")
    return {"plan": plan, "backup_directory": str(backup), "migrated": True}


def restore(args: argparse.Namespace) -> dict[str, Any]:
    root = state_root(args.state_root)
    if (
        not args.backup_root.is_absolute()
        or args.backup_root.is_symlink()
        or not args.backup_root.is_dir()
    ):
        fail("state backup root must be an absolute real directory")
    if args.backup_directory.is_symlink() or not args.backup_directory.is_dir():
        fail("state backup must be a direct real directory below the configured backup root")
    backup_root = args.backup_root.resolve()
    backup = args.backup_directory.resolve()
    if backup.parent != backup_root:
        fail("state backup must be a direct real directory below the configured backup root")
    manifest_path = backup / BACKUP_MANIFEST_NAME
    if manifest_path.is_symlink() or not manifest_path.is_file():
        fail("state backup manifest must be a regular file")
    manifest = load_json(manifest_path)
    if (
        not isinstance(manifest, dict)
        or manifest.get("format") != BACKUP_FORMAT
        or manifest.get("schema_version") != BACKUP_SCHEMA_VERSION
        or manifest.get("state_root") != str(root)
        or not isinstance(manifest.get("managed_paths"), list)
        or not isinstance(manifest.get("entries"), dict)
    ):
        fail("invalid state backup manifest")
    managed_paths = manifest["managed_paths"]
    if any(not isinstance(item, str) for item in managed_paths):
        fail("invalid managed state paths in backup manifest")
    normalized_paths = [safe_managed_path(item) for item in managed_paths]
    if len(normalized_paths) != len(set(normalized_paths)):
        fail("duplicate managed state paths in backup manifest")
    entries = manifest["entries"]
    if set(entries) != {STATE_MARKER_NAME, *normalized_paths}:
        fail("state backup manifest does not contain the exact managed state set")
    prepared: list[
        tuple[Path, dict[str, Any], Path | None, int | None, int | None, int | None]
    ] = []
    payload_names: set[str] = set()
    for relative, record in entries.items():
        if relative != STATE_MARKER_NAME:
            safe_managed_path(relative)
        if not isinstance(record, dict) or not isinstance(record.get("present"), bool):
            fail(f"invalid state backup entry: {relative}")
        destination = root / relative
        safe_existing_file(destination)
        if record["present"]:
            backup_name = record.get("backup_file")
            if not isinstance(backup_name, str) or not re.fullmatch(r"[0-9]{3}\.data", backup_name):
                fail(f"invalid state backup payload name: {relative}")
            source = backup / backup_name
            if source.is_symlink() or not source.is_file():
                fail(f"state backup payload is missing: {source}")
            if backup_name in payload_names:
                fail(f"state backup payload is reused: {backup_name}")
            payload_names.add(backup_name)
            if source.stat().st_size > MAX_MANAGED_FILE_BYTES:
                fail(f"state backup payload exceeds {MAX_MANAGED_FILE_BYTES} bytes: {source}")
            if source.stat().st_size != record.get("size") or sha256_file(source) != record.get("sha256"):
                fail(f"state backup payload checksum mismatch: {relative}")
            mode = record.get("mode")
            uid = record.get("uid")
            gid = record.get("gid")
            if (
                not all(
                    isinstance(value, int) and not isinstance(value, bool)
                    for value in (mode, uid, gid)
                )
                or not 0 <= mode <= 0o7777
                or uid < 0
                or gid < 0
            ):
                fail(f"invalid state backup metadata: {relative}")
            prepared.append((destination, record, source, mode, uid, gid))
        else:
            prepared.append((destination, record, None, None, None, None))

    # Validate every payload and destination before changing any live state.
    for destination, record, source, mode, uid, gid in prepared:
        if record["present"]:
            if source is None or mode is None or uid is None or gid is None:
                fail("validated state restore plan is incomplete")
            atomic_copy(source, destination, mode, uid, gid)
        else:
            destination.unlink(missing_ok=True)
            fsync_directory(root)
    return {
        "restored": True,
        "backup_directory": str(backup),
        "state_schema_version": current_state_version(root),
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)
    for name in ("preflight", "migrate"):
        command = subparsers.add_parser(name)
        command.add_argument("--state-root", type=Path, required=True)
        command.add_argument("--target-release", type=Path, required=True)
        command.add_argument("--previous-release", type=Path)
        command.add_argument("--approve-one-way", action="store_true")
        if name == "migrate":
            command.add_argument("--backup-root", type=Path, required=True)
            command.add_argument("--target-commit", required=True)
            command.add_argument("--state-group", required=True)
    restore_parser = subparsers.add_parser("restore")
    restore_parser.add_argument("--state-root", type=Path, required=True)
    restore_parser.add_argument("--backup-root", type=Path, required=True)
    restore_parser.add_argument("--backup-directory", type=Path, required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "preflight":
            root = state_root(args.state_root)
            previous = args.previous_release.resolve() if args.previous_release else None
            value = compatibility_plan(
                root, args.target_release.resolve(), previous, args.approve_one_way
            )
        elif args.command == "migrate":
            value = migrate(args)
        else:
            value = restore(args)
    except (OSError, StateError) as error:
        print(f"saturn-state-compatibility: ERROR: {error}", file=os.sys.stderr)
        return 1
    print(json.dumps(value, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
