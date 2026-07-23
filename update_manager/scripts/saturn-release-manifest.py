#!/usr/bin/env python3
"""Create and validate Saturn application release manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import stat
import tempfile
import tomllib
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any

FORMAT = "saturn-application-release"
MANIFEST_SCHEMA_VERSION = 3
SUPPORTED_MANIFEST_SCHEMA_VERSIONS = {1, 2, MANIFEST_SCHEMA_VERSION}
DESCRIPTOR_SCHEMA_VERSION = 1
STATE_CONTRACT_FORMAT = "saturn-state-compatibility"
STATE_CONTRACT_SCHEMA_VERSION = 1
STATE_MARKER_NAME = "state-schema.json"
MANIFEST_NAME = "release-manifest.json"
CHECKSUM_NAME = "SHA256SUMS"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
LEGACY_MANAGED_STATE_PATHS = [
    "custom_scripts.json",
    "remote_profiles.json",
    "remote_settings.json",
    "repo_root.txt",
    "saturngo_update_policy.json",
    "update_policy.json",
]
LEGACY_MANIFEST_V1_BUILD_RESULTS = frozenset(
    {
        "rust-server-tests",
        "bridge-stub-tests",
        "remote-web-typecheck",
        "remote-web-template-seam",
        "remote-web-tests",
        "remote-web-production-bundle",
        "protocol2-boundary-tests",
        "saturn-go-release-build",
        "saturn-bridge-release-build",
        "native-release-build",
        "release-manifest-validation",
    }
)


class ManifestError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise ManifestError(message)


def safe_relative(value: str) -> str:
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(part in ("", ".", "..") for part in path.parts):
        fail(f"unsafe release path: {value!r}")
    return path.as_posix()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o644)
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def atomic_text(path: Path, text: str) -> None:
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o644)
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def load_json(path: Path) -> Any:
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read JSON {path}: {error}")


def package_version(repo_root: Path, source: dict[str, Any], commit: str) -> str:
    kind = source.get("type")
    if kind == "source_commit":
        return commit
    relative = safe_relative(str(source.get("path", "")))
    path = repo_root / relative
    if kind == "cargo":
        try:
            with path.open("rb") as handle:
                value = tomllib.load(handle)["package"]["version"]
        except (OSError, KeyError, tomllib.TOMLDecodeError) as error:
            fail(f"cannot resolve Cargo version from {path}: {error}")
        return str(value)
    if kind == "npm":
        value = load_json(path)
        try:
            return str(value["version"])
        except (KeyError, TypeError) as error:
            fail(f"cannot resolve npm version from {path}: {error}")
    fail(f"unsupported version source type: {kind!r}")


def regular_release_files(root: Path, include_metadata: bool = False) -> list[tuple[str, Path]]:
    files: list[tuple[str, Path]] = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            fail(f"symbolic links are not permitted in a release: {relative}")
        if path.is_dir():
            continue
        if not path.is_file():
            fail(f"non-regular release entry rejected: {relative}")
        if not include_metadata and relative in (MANIFEST_NAME, CHECKSUM_NAME):
            continue
        safe_relative(relative)
        files.append((relative, path))
    return files


def load_descriptor(path: Path) -> list[dict[str, Any]]:
    value = load_json(path)
    if not isinstance(value, dict) or value.get("schema_version") != DESCRIPTOR_SCHEMA_VERSION:
        fail(f"unsupported component descriptor schema: {path}")
    components = value.get("components")
    if not isinstance(components, list) or not components:
        fail(f"component descriptor is empty: {path}")
    names: set[str] = set()
    paths: set[str] = set()
    for component in components:
        if not isinstance(component, dict):
            fail("component descriptor entries must be objects")
        name = str(component.get("name", ""))
        relative = safe_relative(str(component.get("path", "")))
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", name):
            fail(f"unsafe component name: {name!r}")
        if name in names or relative in paths:
            fail(f"duplicate component name or path: {name} / {relative}")
        names.add(name)
        paths.add(relative)
    return components


def validate_state_contract(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail("state compatibility contract must be an object")
    if (
        value.get("format") != STATE_CONTRACT_FORMAT
        or value.get("contract_schema_version") != STATE_CONTRACT_SCHEMA_VERSION
    ):
        fail("unsupported state compatibility contract")
    state_version = value.get("state_schema_version")
    readable = value.get("readable_state_schema_versions")
    migration = value.get("migration")
    managed_paths = value.get("managed_paths")
    if not isinstance(state_version, int) or isinstance(state_version, bool) or state_version < 1:
        fail("state schema version must be a positive integer")
    if (
        not isinstance(readable, list)
        or any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in readable)
        or len(set(readable)) != len(readable)
    ):
        fail("readable state schema versions must be unique non-negative integers")
    required_readable = {state_version, state_version - 1}
    if not required_readable.issubset(set(readable)):
        fail("a release must read its own and immediately preceding state schema versions")
    if not isinstance(migration, dict):
        fail("state migration contract must be an object")
    migration_from = migration.get("from_state_schema_versions")
    if (
        not isinstance(migration_from, list)
        or any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in migration_from)
        or len(set(migration_from)) != len(migration_from)
    ):
        fail("migration source versions must be unique non-negative integers")
    if migration.get("kind") != "metadata-only":
        fail("only the audited metadata-only state migration is currently supported")
    if not isinstance(migration.get("one_way"), bool):
        fail("state migration one_way must be boolean")
    documentation = migration.get("documentation")
    if not isinstance(documentation, str) or not documentation.strip():
        fail("state migration documentation is required")
    if state_version - 1 not in migration_from:
        fail("state migration must support the immediately preceding schema version")
    if (
        not isinstance(managed_paths, list)
        or not managed_paths
        or any(not isinstance(item, str) for item in managed_paths)
    ):
        fail("managed state paths must be a non-empty string array")
    normalized_paths = [safe_relative(item) for item in managed_paths]
    if len(set(normalized_paths)) != len(normalized_paths):
        fail("managed state paths must be unique")
    if any(len(PurePosixPath(item).parts) != 1 for item in normalized_paths):
        fail("managed state paths must be direct files below the state root")
    if any(
        item == STATE_MARKER_NAME or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", item)
        for item in normalized_paths
    ):
        fail("managed state paths must be safe direct filenames and cannot include the schema marker")
    normalized = dict(value)
    normalized["readable_state_schema_versions"] = sorted(readable)
    normalized["managed_paths"] = sorted(normalized_paths)
    normalized_migration = dict(migration)
    normalized_migration["from_state_schema_versions"] = sorted(migration_from)
    normalized_migration["documentation"] = documentation.strip()
    normalized["migration"] = normalized_migration
    return normalized


def descriptor_state_contract(path: Path) -> dict[str, Any]:
    value = load_json(path)
    if not isinstance(value, dict):
        fail(f"component descriptor must be an object: {path}")
    return validate_state_contract(value.get("state_compatibility"))


def legacy_state_contract() -> dict[str, Any]:
    return {
        "format": STATE_CONTRACT_FORMAT,
        "contract_schema_version": STATE_CONTRACT_SCHEMA_VERSION,
        "state_schema_version": 0,
        "readable_state_schema_versions": [0, 1],
        "migration": None,
        "managed_paths": LEGACY_MANAGED_STATE_PATHS,
        "legacy_manifest": True,
    }


def required_build_results(path: Path) -> set[str]:
    value = load_json(path)
    results = value.get("required_build_results") if isinstance(value, dict) else None
    if not isinstance(results, list) or not results:
        fail(f"component descriptor has no required build results: {path}")
    names = {str(name) for name in results}
    if len(names) != len(results) or any(not re.fullmatch(r"[a-z0-9][a-z0-9-]*", name) for name in names):
        fail(f"component descriptor has invalid required build results: {path}")
    return names


def file_record(relative: str, path: Path) -> dict[str, Any]:
    mode = stat.S_IMODE(path.stat().st_mode)
    if mode & 0o022:
        fail(f"group/world-writable release file rejected: {relative}")
    return {
        "path": relative,
        "sha256": sha256_file(path),
        "size": path.stat().st_size,
        "mode": f"{mode:04o}",
    }


def create_manifest(args: argparse.Namespace) -> None:
    root = args.release_root.resolve()
    repo_root = args.repo_root.resolve()
    if not root.is_dir():
        fail(f"release root is not a directory: {root}")
    if not repo_root.is_dir():
        fail(f"repository root is not a directory: {repo_root}")
    commit = args.commit.strip().lower()
    if not COMMIT_RE.fullmatch(commit):
        fail("source commit must be one lowercase full Git commit")

    descriptor = load_descriptor(args.components)
    files = regular_release_files(root)
    file_records = {relative: file_record(relative, path) for relative, path in files}
    components: list[dict[str, Any]] = []
    for item in descriptor:
        relative = safe_relative(str(item["path"]))
        record = file_records.get(relative)
        if record is None:
            fail(f"required release component is missing: {relative}")
        executable = bool(item.get("executable"))
        mode = int(record["mode"], 8)
        if executable and not (mode & 0o111):
            fail(f"required component is not executable: {relative}")
        component = dict(record)
        component.update(
            {
                "name": item["name"],
                "role": item.get("role", "application-component"),
                "version": package_version(repo_root, item.get("version_source", {}), commit),
                "source_commit": commit,
            }
        )
        components.append(component)

    required_results = required_build_results(args.components)
    supplied_results = set(args.build_result)
    if supplied_results != required_results or len(args.build_result) != len(supplied_results):
        fail("build results do not exactly match the required release gates")
    build_results = [{"name": name, "status": "passed"} for name in sorted(supplied_results)]
    manifest = {
        "format": FORMAT,
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "source": {
            "commit": commit,
            "repository": args.repository,
            "requested_ref": args.requested_ref,
            "resolved_ref": args.resolved_ref,
            "dirty": False,
        },
        "build": {
            "created_at": args.created_at
            or datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
            "architecture": platform.machine(),
            "system": platform.system().lower(),
            "results": build_results,
        },
        "state_compatibility": descriptor_state_contract(args.components),
        "components": sorted(components, key=lambda item: item["name"]),
        "files": [file_records[name] for name in sorted(file_records)],
    }
    atomic_json(root / MANIFEST_NAME, manifest)
    checksum_lines = []
    for relative, path in regular_release_files(root, include_metadata=True):
        if relative == CHECKSUM_NAME:
            continue
        checksum_lines.append(f"{sha256_file(path)}  {relative}\n")
    atomic_text(root / CHECKSUM_NAME, "".join(checksum_lines))
    validate_release(root, args.components)


def validate_release(root: Path, descriptor_path: Path | None) -> dict[str, Any]:
    root = root.resolve()
    manifest_path = root / MANIFEST_NAME
    checksum_path = root / CHECKSUM_NAME
    manifest = load_json(manifest_path)
    if not isinstance(manifest, dict):
        fail("release manifest must be a JSON object")
    manifest_schema = manifest.get("schema_version")
    if manifest.get("format") != FORMAT or manifest_schema not in SUPPORTED_MANIFEST_SCHEMA_VERSIONS:
        fail("unsupported release manifest format or schema version")
    source = manifest.get("source")
    if not isinstance(source, dict) or not COMMIT_RE.fullmatch(str(source.get("commit", ""))):
        fail("release manifest contains an invalid source commit")
    if source.get("dirty") is not False:
        fail("release manifest must declare a clean source tree")
    source_identity_fields = ("repository", "requested_ref", "resolved_ref")
    if manifest_schema >= 3 and any(field not in source for field in source_identity_fields):
        fail("release manifest does not contain exact source selection provenance")
    for field in source_identity_fields:
        value = source.get(field)
        if value is not None and (
            not isinstance(value, str)
            or not value
            or len(value) > 2048
            or any(ord(character) < 32 for character in value)
        ):
            fail(f"release manifest contains invalid source {field}")
    if manifest_schema == 1:
        if "state_compatibility" in manifest:
            fail("legacy release manifests cannot carry a state compatibility contract")
    else:
        contract = validate_state_contract(manifest.get("state_compatibility"))
        if descriptor_path is not None and contract != descriptor_state_contract(descriptor_path):
            fail("release state compatibility does not match the trusted component policy")

    actual_files = {relative: file_record(relative, path) for relative, path in regular_release_files(root)}
    manifest_files = manifest.get("files")
    if not isinstance(manifest_files, list):
        fail("release manifest files must be an array")
    listed_files: dict[str, dict[str, Any]] = {}
    for record in manifest_files:
        if not isinstance(record, dict):
            fail("release file records must be objects")
        relative = safe_relative(str(record.get("path", "")))
        if relative in listed_files:
            fail(f"duplicate release file record: {relative}")
        listed_files[relative] = record
    if set(actual_files) != set(listed_files):
        fail("release manifest does not exactly cover the release files")
    for relative, actual in actual_files.items():
        listed = listed_files[relative]
        for key in ("sha256", "size", "mode"):
            if listed.get(key) != actual[key]:
                fail(f"release file {key} mismatch: {relative}")

    components = manifest.get("components")
    if not isinstance(components, list):
        fail("release manifest components must be an array")
    component_names: set[str] = set()
    component_paths: set[str] = set()
    for component in components:
        if not isinstance(component, dict):
            fail("release components must be objects")
        name = str(component.get("name", ""))
        relative = safe_relative(str(component.get("path", "")))
        if name in component_names or relative in component_paths:
            fail(f"duplicate manifest component: {name} / {relative}")
        component_names.add(name)
        component_paths.add(relative)
        file_record_value = actual_files.get(relative)
        if file_record_value is None:
            fail(f"component path is not a release file: {relative}")
        for key in ("sha256", "size", "mode"):
            if component.get(key) != file_record_value[key]:
                fail(f"component {key} mismatch: {name}")
        if component.get("source_commit") != source["commit"] or not component.get("version"):
            fail(f"component identity is incomplete: {name}")

    if descriptor_path is not None:
        expected = load_descriptor(descriptor_path)
        expected_names = {str(item["name"]) for item in expected}
        expected_paths = {safe_relative(str(item["path"])) for item in expected}
        if component_names != expected_names or component_paths != expected_paths:
            fail("release manifest does not contain the exact required component set")

    results = manifest.get("build", {}).get("results")
    if not isinstance(results, list) or not results:
        fail("release manifest contains no build/test results")
    if any(not isinstance(item, dict) or item.get("status") != "passed" for item in results):
        fail("release manifest contains an unsuccessful build/test result")
    result_names = {str(item.get("name", "")) for item in results}
    if descriptor_path is not None:
        expected_results = required_build_results(descriptor_path)
        if manifest_schema == 1:
            # Freeze the exact v1 gate set so later policy additions cannot
            # strand an already-installed rollback release.
            accepted_result_sets = {LEGACY_MANIFEST_V1_BUILD_RESULTS}
        else:
            accepted_result_sets = {frozenset(expected_results)}
        if frozenset(result_names) not in accepted_result_sets or len(result_names) != len(results):
            fail("release manifest does not contain an accepted exact build/test result set")

    expected_checksums = {
        relative: sha256_file(path)
        for relative, path in regular_release_files(root, include_metadata=True)
        if relative != CHECKSUM_NAME
    }
    try:
        lines = checksum_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read {CHECKSUM_NAME}: {error}")
    listed_checksums: dict[str, str] = {}
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if not match:
            fail(f"invalid checksum line: {line!r}")
        relative = safe_relative(match.group(2))
        if relative in listed_checksums:
            fail(f"duplicate checksum path: {relative}")
        listed_checksums[relative] = match.group(1)
    if listed_checksums != expected_checksums:
        fail("SHA256SUMS does not exactly match the release payload")
    return manifest


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)

    create = subparsers.add_parser("create", help="create and validate a manifest")
    create.add_argument("--release-root", type=Path, required=True)
    create.add_argument("--repo-root", type=Path, required=True)
    create.add_argument("--components", type=Path, required=True)
    create.add_argument("--commit", required=True)
    create.add_argument("--repository", required=True)
    create.add_argument("--requested-ref", required=True)
    create.add_argument("--resolved-ref", required=True)
    create.add_argument("--created-at", default="")
    create.add_argument("--build-result", action="append", default=[])

    validate = subparsers.add_parser("validate", help="validate an existing release")
    validate.add_argument("--release-root", type=Path, required=True)
    validate.add_argument("--components", type=Path)
    state_contract = subparsers.add_parser(
        "state-contract", help="validate a release and print its persistent-state contract"
    )
    state_contract.add_argument("--release-root", type=Path, required=True)
    state_contract.add_argument("--components", type=Path)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "create":
            if not args.build_result:
                fail("at least one --build-result is required")
            create_manifest(args)
        elif args.command == "validate":
            validate_release(args.release_root, args.components)
        else:
            manifest = validate_release(args.release_root, args.components)
            if manifest["schema_version"] == 1:
                contract = legacy_state_contract()
            else:
                contract = validate_state_contract(manifest.get("state_compatibility"))
            print(json.dumps(contract, sort_keys=True))
            return 0
    except ManifestError as error:
        print(f"saturn-release-manifest: ERROR: {error}", file=os.sys.stderr)
        return 1
    print(f"saturn-release-manifest: {args.command} passed: {args.release_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
