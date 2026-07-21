#!/usr/bin/env python3
"""Crash-recoverable Saturn settings and repository restore transactions."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import sys
import tempfile
import time
from pathlib import Path, PurePosixPath
from typing import Any


TRANSACTION_FORMAT = "saturn-restore-transaction"
TRANSACTION_SCHEMA_VERSION = 1
SETTINGS_FORMAT = "saturn-settings-backup"
SETTINGS_SCHEMA_VERSION = 1
STATE_SCHEMA_FORMAT = "saturn-persistent-state"
STATE_MARKER_SCHEMA_VERSION = 1
READABLE_STATE_SCHEMA_VERSIONS = {0, 1}
MAX_SETTINGS_FILE_BYTES = 16 * 1024 * 1024
MAX_SETTINGS_TOTAL_BYTES = 128 * 1024 * 1024
SETTINGS_SPACE_RESERVE_BYTES = 32 * 1024 * 1024
SOURCE_SPACE_RESERVE_BYTES = 512 * 1024 * 1024
SAFE_NAME_RE = re.compile(r"[A-Za-z0-9._-]+")
HOST_POLICY_PATHS = {
    "saturn-state/repo_root.txt",
    "saturn-state/update_policy.json",
    "saturn-state/saturngo_update_policy.json",
}
STATE_PATHS = {
    "saturn-state/state-schema.json": "state-schema.json",
    "saturn-state/custom_scripts.json": "custom_scripts.json",
    "saturn-state/remote_settings.json": "remote_settings.json",
    "saturn-state/remote_profiles.json": "remote_profiles.json",
    "saturn-state/repo_root.txt": "repo_root.txt",
    "saturn-state/update_policy.json": "update_policy.json",
    "saturn-state/saturngo_update_policy.json": "saturngo_update_policy.json",
}


class RestoreError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise RestoreError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def ensure_real_directory(path: Path, mode: int = 0o750) -> Path:
    if path.exists() and (path.is_symlink() or not path.is_dir()):
        fail(f"directory must be a real directory: {path}")
    path.mkdir(parents=True, exist_ok=True, mode=mode)
    if path.is_symlink() or not path.is_dir():
        fail(f"directory must be a real directory: {path}")
    return path.resolve()


def validate_owned_path(path: Path, description: str) -> None:
    metadata = path.stat()
    if metadata.st_uid != os.geteuid():
        fail(
            f"{description} must be owned by uid {os.geteuid()}, "
            f"but uid {metadata.st_uid} owns {path}"
        )


def atomic_write(path: Path, content: bytes, mode: int) -> None:
    parent = ensure_real_directory(path.parent)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
        fsync_directory(parent)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_json(path: Path, value: dict[str, Any], mode: int = 0o640) -> None:
    content = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    atomic_write(path, content, mode)


def atomic_copy(source: Path, destination: Path, mode: int) -> None:
    if source.is_symlink() or not source.is_file():
        fail(f"transaction payload must be a regular file: {source}")
    with source.open("rb") as handle:
        content = handle.read(MAX_SETTINGS_FILE_BYTES + 1)
    if len(content) > MAX_SETTINGS_FILE_BYTES:
        fail(f"transaction payload exceeds {MAX_SETTINGS_FILE_BYTES} bytes: {source}")
    atomic_write(destination, content, mode)


def atomic_text(path: Path, value: str, mode: int = 0o640) -> None:
    atomic_write(path, f"{value.rstrip()}\n".encode("utf-8"), mode)


def load_json(path: Path, maximum: int = MAX_SETTINGS_FILE_BYTES) -> Any:
    if path.is_symlink() or not path.is_file():
        fail(f"JSON input must be a regular file: {path}")
    if path.stat().st_size > maximum:
        fail(f"JSON input exceeds {maximum} bytes: {path}")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON in {path}: {error}")


def safe_relative(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if not value or path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail(f"unsafe settings archive path: {value!r}")
    return path


def safe_name(value: str) -> str:
    if not SAFE_NAME_RE.fullmatch(value) or ".." in value:
        fail(f"unsafe settings filename: {value!r}")
    return value


def validate_saturn_repo(path: Path) -> Path:
    if path.is_symlink() or not path.is_dir():
        fail(f"Saturn repository must be a real directory: {path}")
    resolved = path.resolve()
    if not resolved.joinpath(".git").exists():
        fail(f"Saturn repository is not a Git checkout: {resolved}")
    if not resolved.joinpath("update_manager").is_dir():
        fail(f"Saturn repository does not contain update_manager: {resolved}")
    return resolved


def transaction_root(state_root: Path) -> Path:
    return ensure_real_directory(state_root / "restore-transactions", 0o750)


def new_transaction(state_root: Path, kind: str) -> tuple[Path, dict[str, Any]]:
    root = transaction_root(state_root)
    identifier = f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-{os.getpid()}-{os.urandom(4).hex()}"
    directory = root / identifier
    directory.mkdir(mode=0o750)
    fsync_directory(root)
    value: dict[str, Any] = {
        "format": TRANSACTION_FORMAT,
        "schema_version": TRANSACTION_SCHEMA_VERSION,
        "id": identifier,
        "kind": kind,
        "status": "staging",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "message": "restore transaction is staging",
    }
    atomic_json(directory / "transaction.json", value)
    return directory, value


def update_transaction(directory: Path, value: dict[str, Any], status: str, message: str) -> None:
    value["status"] = status
    value["message"] = message
    value["updated_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    atomic_json(directory / "transaction.json", value)


def maybe_crash(name: str) -> None:
    if os.environ.get("SATURN_RESTORE_FAILPOINT") == name:
        os._exit(97)


def available_bytes(path: Path) -> int:
    override = os.environ.get("SATURN_RESTORE_TEST_AVAILABLE_BYTES")
    if override is not None:
        try:
            return int(override)
        except ValueError:
            fail("SATURN_RESTORE_TEST_AVAILABLE_BYTES must be an integer")
    return shutil.disk_usage(path).free


def file_tree_bytes(path: Path) -> int:
    total = 0
    for parent, directories, files in os.walk(path, followlinks=False):
        parent_path = Path(parent)
        for name in [*directories, *files]:
            candidate = parent_path / name
            metadata = candidate.lstat()
            if not stat.S_ISLNK(metadata.st_mode) and metadata.st_uid != os.geteuid():
                fail(f"repository entry is not owned by uid {os.geteuid()}: {candidate}")
            if metadata.st_mode & (stat.S_ISUID | stat.S_ISGID | stat.S_ISVTX):
                fail(f"repository entry has unsupported special permission bits: {candidate}")
            if stat.S_ISREG(metadata.st_mode):
                total += metadata.st_size
            elif stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(candidate)
                target_path = PurePosixPath(target)
                if target_path.is_absolute() or ".." in target_path.parts:
                    fail(f"repository contains unsafe symlink target: {candidate} -> {target}")
            elif not stat.S_ISDIR(metadata.st_mode):
                fail(f"repository contains an unsupported special file: {candidate}")
    return total


def fsync_tree(root: Path) -> None:
    directories: list[Path] = []
    for parent, child_dirs, files in os.walk(root, followlinks=False):
        parent_path = Path(parent)
        directories.append(parent_path)
        for name in files:
            path = parent_path / name
            if path.is_symlink():
                continue
            with path.open("rb") as handle:
                os.fsync(handle.fileno())
        for name in child_dirs:
            path = parent_path / name
            if path.is_symlink():
                continue
    for directory in reversed(directories):
        fsync_directory(directory)


def settings_destination(
    relative: str,
    state_root: Path,
    scripts_root: Path,
    pihpsdr_root: Path,
    deskhpsdr_root: Path,
) -> tuple[Path, int]:
    if relative in STATE_PATHS:
        return state_root / STATE_PATHS[relative], 0o640
    path = safe_relative(relative)
    if len(path.parts) == 2 and path.parts[0] == "custom-scripts":
        return scripts_root / safe_name(path.parts[1]), 0o755
    if len(path.parts) == 3 and path.parts[:2] == ("clients", "pihpsdr"):
        name = safe_name(path.parts[2])
        if not name.endswith(".props"):
            fail(f"piHPSDR restore accepts only direct .props files: {relative}")
        return pihpsdr_root / name, 0o600
    if len(path.parts) == 3 and path.parts[:2] == ("clients", "deskhpsdr"):
        name = safe_name(path.parts[2])
        if not name.endswith(".props"):
            fail(f"deskHPSDR restore accepts only direct .props files: {relative}")
        return deskhpsdr_root / name, 0o600
    fail(f"settings manifest contains an unsupported destination: {relative}")


def validate_settings_semantics(archive_root: Path, records: dict[str, dict[str, Any]], include_host_policy: bool) -> None:
    def record_path(relative: str) -> Path | None:
        record = records.get(relative)
        return archive_root / relative if record is not None else None

    marker_path = record_path("saturn-state/state-schema.json")
    if marker_path is not None:
        marker = load_json(marker_path)
        if not isinstance(marker, dict):
            fail("settings state-schema.json must contain a JSON object")
        marker_format = marker.get("format", STATE_SCHEMA_FORMAT)
        if marker_format != STATE_SCHEMA_FORMAT:
            fail(f"unsupported Saturn state schema format: {marker_format!r}")
        marker_schema = marker.get("schema_version", STATE_MARKER_SCHEMA_VERSION)
        if marker_schema != STATE_MARKER_SCHEMA_VERSION:
            fail(f"unsupported Saturn state marker schema: {marker_schema!r}")
        version = marker.get("state_schema_version", 0)
        if not isinstance(version, int) or version not in READABLE_STATE_SCHEMA_VERSIONS:
            fail(f"unsupported Saturn state schema version: {version!r}")

    for relative in (
        "saturn-state/remote_settings.json",
        "saturn-state/remote_profiles.json",
        "saturn-state/update_policy.json",
        "saturn-state/saturngo_update_policy.json",
    ):
        path = record_path(relative)
        if path is not None and not isinstance(load_json(path), dict):
            fail(f"{relative} must contain a JSON object")

    registry_path = record_path("saturn-state/custom_scripts.json")
    if registry_path is not None:
        registry = load_json(registry_path)
        if not isinstance(registry, list):
            fail("custom_scripts.json must contain a JSON array")
        required_scripts: set[str] = set()
        for entry in registry:
            if not isinstance(entry, dict) or not isinstance(entry.get("filename"), str):
                fail("custom_scripts.json contains an invalid entry")
            filename = safe_name(entry["filename"])
            if entry.get("version") != "custom-default":
                required_scripts.add(f"custom-scripts/{filename}")
        present_scripts = {path for path in records if path.startswith("custom-scripts/")}
        if required_scripts != present_scripts:
            fail("operator script content does not exactly match the custom-script registry")

    repo_path = record_path("saturn-state/repo_root.txt")
    if include_host_policy and repo_path is not None:
        try:
            requested = Path(repo_path.read_text(encoding="utf-8").strip())
        except (OSError, UnicodeDecodeError) as error:
            fail(f"cannot read restored repository policy: {error}")
        if not requested.is_absolute():
            fail("restored repository root must be absolute")
        requested = validate_saturn_repo(requested)
        validate_owned_path(requested, "restored repository root")


def prepare_settings_plan(args: argparse.Namespace) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    archive_root = validate_real_archive_root(Path(args.archive_root))
    manifest = load_json(archive_root / "manifest.json")
    if (
        not isinstance(manifest, dict)
        or manifest.get("format") != SETTINGS_FORMAT
        or manifest.get("schema_version") != SETTINGS_SCHEMA_VERSION
        or not isinstance(manifest.get("files"), list)
    ):
        fail("archive is not a supported Saturn settings backup")

    records: dict[str, dict[str, Any]] = {}
    total = 0
    for record in manifest["files"]:
        if not isinstance(record, dict) or not isinstance(record.get("archive_path"), str):
            fail("settings manifest contains an invalid file record")
        relative = record["archive_path"]
        safe_relative(relative)
        if relative in records:
            fail(f"settings manifest contains a duplicate path: {relative}")
        size = record.get("size")
        digest = record.get("sha256")
        source_mode = record.get("mode")
        if not isinstance(size, int) or size < 0 or size > MAX_SETTINGS_FILE_BYTES:
            fail(f"settings manifest contains an invalid size: {relative}")
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            fail(f"settings manifest contains an invalid SHA-256: {relative}")
        if (
            not isinstance(source_mode, int)
            or isinstance(source_mode, bool)
            or source_mode < 0
            or source_mode > 0o777
        ):
            fail(f"settings manifest contains invalid permissions: {relative}")
        source = archive_root / relative
        if source.is_symlink() or not source.is_file():
            fail(f"settings payload must be a regular file: {relative}")
        if source.stat().st_size != size or sha256_file(source) != digest:
            fail(f"settings payload checksum mismatch: {relative}")
        total += size
        if total > MAX_SETTINGS_TOTAL_BYTES:
            fail(f"settings payload exceeds {MAX_SETTINGS_TOTAL_BYTES} bytes")
        records[relative] = record

    actual_files: set[str] = set()
    for path in archive_root.rglob("*"):
        if path.is_symlink():
            fail(f"settings archive contains a symlink: {path.relative_to(archive_root)}")
        if path.is_file():
            actual_files.add(path.relative_to(archive_root).as_posix())
    if actual_files != {"manifest.json", *records.keys()}:
        fail("settings archive contains undeclared or missing files")

    state_root = ensure_real_directory(Path(args.state_root))
    scripts_root = ensure_real_directory(Path(args.scripts_root), 0o775)
    pihpsdr_root = ensure_real_directory(Path(args.pihpsdr_root), 0o755)
    deskhpsdr_root = ensure_real_directory(Path(args.deskhpsdr_root), 0o700)
    for root, description in (
        (state_root, "Saturn state root"),
        (scripts_root, "Saturn scripts root"),
        (pihpsdr_root, "piHPSDR settings root"),
        (deskhpsdr_root, "deskHPSDR settings root"),
    ):
        validate_owned_path(root, description)
    include_host_policy = bool(args.include_host_policy)
    validate_settings_semantics(archive_root, records, include_host_policy)

    plan: list[dict[str, Any]] = []
    skipped: list[str] = []
    for relative in sorted(records):
        if relative in HOST_POLICY_PATHS and not include_host_policy:
            skipped.append(relative)
            continue
        destination, mode = settings_destination(
            relative, state_root, scripts_root, pihpsdr_root, deskhpsdr_root
        )
        parent = ensure_real_directory(destination.parent)
        if destination.parent.resolve() != parent:
            fail(f"settings destination parent changed unexpectedly: {destination}")
        if destination.exists() or destination.is_symlink():
            if destination.is_symlink() or not destination.is_file():
                fail(f"live settings destination must be a regular file: {destination}")
            validate_owned_path(destination, "live settings destination")
            if destination.stat().st_size > MAX_SETTINGS_FILE_BYTES:
                fail(f"live settings destination exceeds rollback size limit: {destination}")
        plan.append(
            {
                "archive_path": relative,
                "source": str(archive_root / relative),
                "destination": str(destination),
                "mode": mode,
                "sha256": records[relative]["sha256"],
                "size": records[relative]["size"],
                "old_size": destination.stat().st_size if destination.exists() else 0,
            }
        )
    summary = {
        "format": SETTINGS_FORMAT,
        "schema_version": SETTINGS_SCHEMA_VERSION,
        "files": len(plan),
        "bytes": sum(entry["size"] for entry in plan),
        "skipped_host_policy": skipped,
        "include_host_policy": include_host_policy,
    }
    return summary, plan


def validate_real_archive_root(path: Path) -> Path:
    if path.is_symlink() or not path.is_dir():
        fail(f"archive root must be a real directory: {path}")
    return path.resolve()


def rollback_settings(directory: Path, transaction: dict[str, Any]) -> None:
    entries = transaction.get("entries", [])
    if not isinstance(entries, list):
        fail("settings transaction entries are invalid")
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("destination"), str):
            fail("settings transaction entry is invalid")
        destination = Path(entry["destination"])
        if not destination.is_absolute():
            fail("settings rollback destination is not absolute")
        if entry.get("old_present") is True:
            old_relative = entry.get("old_file")
            old_digest = entry.get("old_sha256")
            if not isinstance(old_relative, str) or not isinstance(old_digest, str):
                fail("settings rollback payload metadata is incomplete")
            old_source = directory / old_relative
            if sha256_file(old_source) != old_digest:
                fail(f"settings rollback payload checksum mismatch: {destination}")
            atomic_copy(old_source, destination, int(entry["old_mode"]))
        else:
            destination.unlink(missing_ok=True)
            fsync_directory(destination.parent)


def settings_restore(args: argparse.Namespace) -> dict[str, Any]:
    summary, plan = prepare_settings_plan(args)
    state_root = ensure_real_directory(Path(args.state_root))
    rollback_bytes = sum(int(item["old_size"]) for item in plan)
    required = summary["bytes"] * 2 + rollback_bytes + SETTINGS_SPACE_RESERVE_BYTES
    if available_bytes(state_root) < required:
        fail(
            f"insufficient space for transactional settings restore: need {required} bytes including reserve"
        )
    if args.dry_run:
        return {"status": "ok", "dry_run": True, **summary}

    directory, transaction = new_transaction(state_root, "settings")
    old_root = ensure_real_directory(directory / "old", 0o750)
    new_root = ensure_real_directory(directory / "new", 0o750)
    entries: list[dict[str, Any]] = []
    try:
        for index, item in enumerate(plan):
            source = Path(item["source"])
            destination = Path(item["destination"])
            new_file = new_root / f"{index:03d}.data"
            shutil.copyfile(source, new_file)
            os.chmod(new_file, 0o640)
            with new_file.open("rb") as handle:
                os.fsync(handle.fileno())
            entry: dict[str, Any] = {
                "archive_path": item["archive_path"],
                "destination": str(destination),
                "new_file": str(new_file.relative_to(directory)),
                "new_sha256": item["sha256"],
                "new_mode": item["mode"],
                "old_present": False,
            }
            if destination.exists() or destination.is_symlink():
                if destination.is_symlink() or not destination.is_file():
                    fail(f"live settings destination must be a regular file: {destination}")
                old_file = old_root / f"{index:03d}.data"
                shutil.copyfile(destination, old_file)
                os.chmod(old_file, 0o640)
                with old_file.open("rb") as handle:
                    os.fsync(handle.fileno())
                metadata = destination.stat()
                entry.update(
                    {
                        "old_present": True,
                        "old_file": str(old_file.relative_to(directory)),
                        "old_sha256": sha256_file(old_file),
                        "old_mode": stat.S_IMODE(metadata.st_mode),
                    }
                )
            entries.append(entry)
        fsync_directory(old_root)
        fsync_directory(new_root)
        transaction["entries"] = entries
        transaction["summary"] = summary
        update_transaction(directory, transaction, "prepared", "settings payload and rollback data are durable")
        maybe_crash("settings_after_prepare")
        update_transaction(directory, transaction, "applying", "settings activation is in progress")
        for index, entry in enumerate(entries):
            new_source = directory / entry["new_file"]
            if sha256_file(new_source) != entry["new_sha256"]:
                fail(f"staged settings checksum mismatch: {entry['archive_path']}")
            atomic_copy(new_source, Path(entry["destination"]), int(entry["new_mode"]))
            maybe_crash(f"settings_after_{index + 1}")
        for entry in entries:
            destination = Path(entry["destination"])
            if destination.is_symlink() or not destination.is_file():
                fail(f"restored settings destination is unavailable: {destination}")
            if sha256_file(destination) != entry["new_sha256"]:
                fail(f"restored settings checksum mismatch: {destination}")
        update_transaction(directory, transaction, "committed", "settings restore committed and verified")
        return {
            "status": "ok",
            "dry_run": False,
            "transaction_id": transaction["id"],
            **summary,
        }
    except Exception as error:
        try:
            transaction["entries"] = entries
            rollback_settings(directory, transaction)
            update_transaction(directory, transaction, "rolled_back", f"settings restore rolled back: {error}")
        except Exception as rollback_error:
            update_transaction(
                directory,
                transaction,
                "recovery_required",
                f"settings restore failed: {error}; rollback failed: {rollback_error}",
            )
            fail(f"settings restore failed and rollback requires recovery: {error}; {rollback_error}")
        raise


def source_restore(args: argparse.Namespace) -> dict[str, Any]:
    source = validate_saturn_repo(Path(args.source_root))
    current = validate_saturn_repo(Path(args.current_repo_root))
    state_root = ensure_real_directory(Path(args.state_root))
    validate_owned_path(source, "source restore tree")
    validate_owned_path(current, "active Saturn repository")
    validate_owned_path(state_root, "Saturn state root")
    repo_root_file = Path(args.repo_root_file)
    if not repo_root_file.is_absolute() or repo_root_file.parent.resolve() != state_root:
        fail("repository pointer must be a direct file in the Saturn state root")
    bytes_total = file_tree_bytes(source)
    generation_root = ensure_real_directory(state_root / "repository-restores", 0o750)
    required = bytes_total + SOURCE_SPACE_RESERVE_BYTES
    if available_bytes(generation_root) < required:
        fail(
            f"insufficient space for transactional source restore: need {required} bytes including reserve"
        )
    if args.dry_run:
        return {
            "status": "ok",
            "dry_run": True,
            "source_root": str(source),
            "previous_repo_root": str(current),
            "bytes": bytes_total,
        }

    directory, transaction = new_transaction(state_root, "source")
    identifier = transaction["id"]
    staging = generation_root / f".{identifier}.staging"
    final = generation_root / identifier
    transaction.update(
        {
            "previous_repo_root": str(current),
            "new_repo_root": str(final),
            "staging_repo_root": str(staging),
            "repo_root_file": str(repo_root_file),
            "bytes": bytes_total,
        }
    )
    update_transaction(directory, transaction, "staging", "repository generation copy is in progress")
    try:
        shutil.copytree(source, staging, symlinks=True, copy_function=shutil.copy2)
        validate_saturn_repo(staging)
        staged_bytes = file_tree_bytes(staging)
        if staged_bytes != bytes_total:
            fail(
                "repository generation byte count changed while staging: "
                f"expected {bytes_total}, copied {staged_bytes}"
            )
        maybe_crash("source_after_copy")
        fsync_tree(staging)
        os.rename(staging, final)
        fsync_directory(generation_root)
        update_transaction(directory, transaction, "prepared", "repository generation is durable")
        maybe_crash("source_after_generation")
        update_transaction(directory, transaction, "applying", "repository pointer activation is in progress")
        atomic_text(repo_root_file, str(final))
        maybe_crash("source_after_pointer")
        selected = Path(repo_root_file.read_text(encoding="utf-8").strip()).resolve()
        if selected != final.resolve():
            fail("repository pointer verification failed")
        validate_saturn_repo(selected)
        update_transaction(directory, transaction, "committed", "repository restore committed and verified")
        return {
            "status": "ok",
            "dry_run": False,
            "transaction_id": identifier,
            "previous_repo_root": str(current),
            "new_repo_root": str(final),
            "bytes": bytes_total,
        }
    except Exception as error:
        try:
            atomic_text(repo_root_file, str(current))
            cleanup_source_staging(directory, transaction)
            update_transaction(directory, transaction, "rolled_back", f"source restore rolled back: {error}")
        except Exception as rollback_error:
            update_transaction(
                directory,
                transaction,
                "recovery_required",
                f"source restore failed: {error}; rollback failed: {rollback_error}",
            )
            fail(f"source restore failed and rollback requires recovery: {error}; {rollback_error}")
        raise


def cleanup_source_staging(directory: Path, transaction: dict[str, Any]) -> None:
    staging_value = transaction.get("staging_repo_root")
    identifier = transaction.get("id")
    if not isinstance(staging_value, str) or not isinstance(identifier, str):
        return
    staging = Path(staging_value)
    state_root = directory.parent.parent.resolve()
    generation_root = (state_root / "repository-restores").resolve()
    if (
        not staging.is_absolute()
        or staging.parent.resolve() != generation_root
        or staging.name != f".{identifier}.staging"
    ):
        fail("source transaction contains an unsafe staging path")
    if staging.is_symlink():
        fail("source transaction staging path became a symlink")
    if staging.exists():
        if not staging.is_dir():
            fail("source transaction staging path is not a directory")
        shutil.rmtree(staging)
        fsync_directory(generation_root)


def recover_transaction(directory: Path, transaction: dict[str, Any]) -> dict[str, Any]:
    if transaction.get("status") in {"committed", "rolled_back"}:
        return transaction
    kind = transaction.get("kind")
    if kind == "settings":
        rollback_settings(directory, transaction)
    elif kind == "source":
        previous = transaction.get("previous_repo_root")
        pointer = transaction.get("repo_root_file")
        if isinstance(previous, str) and isinstance(pointer, str):
            validate_saturn_repo(Path(previous))
            atomic_text(Path(pointer), previous)
        cleanup_source_staging(directory, transaction)
    else:
        fail(f"unknown restore transaction kind in {directory}: {kind!r}")
    update_transaction(directory, transaction, "rolled_back", "incomplete restore rolled back during startup recovery")
    return transaction


def recover(args: argparse.Namespace) -> dict[str, Any]:
    state_root = ensure_real_directory(Path(args.state_root))
    root = transaction_root(state_root)
    recovered: list[str] = []
    for directory in sorted(root.iterdir()):
        if directory.is_symlink() or not directory.is_dir():
            fail(f"restore transaction entry must be a real directory: {directory}")
        transaction = load_json(directory / "transaction.json")
        if (
            not isinstance(transaction, dict)
            or transaction.get("format") != TRANSACTION_FORMAT
            or transaction.get("schema_version") != TRANSACTION_SCHEMA_VERSION
            or transaction.get("id") != directory.name
        ):
            fail(f"invalid restore transaction record: {directory}")
        prior = transaction.get("status")
        recover_transaction(directory, transaction)
        if prior not in {"committed", "rolled_back"}:
            recovered.append(directory.name)
    return {"status": "ok", "recovered": recovered}


def status(args: argparse.Namespace) -> dict[str, Any]:
    state_root = ensure_real_directory(Path(args.state_root))
    root = transaction_root(state_root)
    transactions = []
    for directory in sorted(root.iterdir(), reverse=True):
        if directory.is_symlink() or not directory.is_dir():
            continue
        try:
            value = load_json(directory / "transaction.json")
        except RestoreError:
            continue
        transactions.append(value)
    return {"status": "ok", "transactions": transactions[:20]}


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)

    settings_parser = subparsers.add_parser("settings")
    settings_parser.add_argument("--state-root", type=Path, required=True)
    settings_parser.add_argument("--archive-root", type=Path, required=True)
    settings_parser.add_argument("--scripts-root", type=Path, required=True)
    settings_parser.add_argument("--pihpsdr-root", type=Path, required=True)
    settings_parser.add_argument("--deskhpsdr-root", type=Path, required=True)
    settings_parser.add_argument("--include-host-policy", action="store_true")
    settings_parser.add_argument("--dry-run", action="store_true")

    source_parser = subparsers.add_parser("source")
    source_parser.add_argument("--state-root", type=Path, required=True)
    source_parser.add_argument("--source-root", type=Path, required=True)
    source_parser.add_argument("--current-repo-root", type=Path, required=True)
    source_parser.add_argument("--repo-root-file", type=Path, required=True)
    source_parser.add_argument("--dry-run", action="store_true")

    recover_parser = subparsers.add_parser("recover")
    recover_parser.add_argument("--state-root", type=Path, required=True)

    status_parser = subparsers.add_parser("status")
    status_parser.add_argument("--state-root", type=Path, required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "settings":
            value = settings_restore(args)
        elif args.command == "source":
            value = source_restore(args)
        elif args.command == "recover":
            value = recover(args)
        else:
            value = status(args)
        print(json.dumps(value, sort_keys=True))
        return 0
    except RestoreError as error:
        print(f"saturn-restore-transaction: ERROR: {error}", file=sys.stderr)
        return 1
    except Exception as error:  # Keep an actionable durable failure boundary.
        print(f"saturn-restore-transaction: ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
