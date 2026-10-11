#!/usr/bin/env python3
"""Write /etc/default/saturn-release-activate from the installer's defaults without losing what an administrator chose.

The installer used to rewrite that file from fixed text on every run, which silently dropped an administrator's browser memory allowance
(VALIDATION_MEMORY_MAX) and reset the activation policy. This tool keeps the settings an administrator owns and takes everything else
from the installer:

  * Installer-owned keys (paths, service names, URLs): always the installer's current values.
  * Administrator-owned keys (ACTIVATION_ENABLED, VALIDATION_ENABLED, VALIDATION_MEMORY_MAX, VALIDATION_TIMEOUT_SECONDS,
    VALIDATION_LEGACY_BRIDGE_RELEASES, VALIDATION_EXPECT_RXC1, READY_TIMEOUT_SECONDS): the existing value when it is valid, otherwise
    the installer's default (or none, for a key the defaults do not set). An invalid existing value is reported and dropped, never
    copied: the root activator refuses a malformed configuration, and refusing to activate is worse than using the default.
  * Anything else in the existing file (a key the activator does not know) is dropped and reported; the activator rejects it.

The activation policy is the one setting that is never changed by accident. With --managed-updates keep (the default) an existing
value, including an explicit 0, is preserved. Only --managed-updates enable or disable changes it, and that is an explicit choice of
whoever runs the installer. A fresh installation (no existing file) takes the installer's default.

The output is written atomically (a temporary file in the same directory, synced, then renamed) and is identical when the tool is run
again on its own output.
"""
from __future__ import annotations

import argparse
import os
import re
import sys
import tempfile

ADMIN_KEYS = {
    "ACTIVATION_ENABLED": lambda v: v in ("0", "1"),
    "VALIDATION_ENABLED": lambda v: v in ("0", "1"),
    "VALIDATION_MEMORY_MAX": lambda v: re.fullmatch(r"[1-9][0-9]*[MG]", v) is not None,
    "VALIDATION_TIMEOUT_SECONDS": lambda v: re.fullmatch(r"[1-9][0-9]*", v) is not None and int(v) <= 600,
    "VALIDATION_LEGACY_BRIDGE_RELEASES": lambda v: all(re.fullmatch(r"[0-9a-f]{40}", item) for item in v.split()),
    "VALIDATION_EXPECT_RXC1": lambda v: v in ("any", "valid", "unarmed", "disabled", "unsupported", "unavailable", "partial"),
    "READY_TIMEOUT_SECONDS": lambda v: re.fullmatch(r"[1-9][0-9]*", v) is not None and int(v) <= 300,
}

# Every key the root activator's loader accepts (load_config in saturn-release-activate-root.sh). A key outside this set makes the
# activator refuse the whole file.
KNOWN_KEYS = {
    "ACTIVATION_ENABLED", "SATURN_ROOT", "RELEASES_ROOT", "CURRENT_LINK", "TRANSACTION_FILE", "LOCK_FILE", "MANIFEST_TOOL", "COMPONENTS_FILE",
    "STATE_TOOL", "STATE_ROOT", "STATE_BACKUP_ROOT", "SYSTEMD_ROOT", "SATURN_GO_SERVICE", "BRIDGE_SERVICE", "P2APP_SERVICE", "SATURN_GO_READY_URL",
    "P23_PERF_URL", "BACKEND_STATUS_HELPER", "READY_TIMEOUT_SECONDS", "P2APP_PANEL_ENABLED", "TRANSACTION_GROUP", "VALIDATION_ENABLED", "VALIDATOR",
    "VALIDATOR_GUARD", "VALIDATION_TIMEOUT_SECONDS", "VALIDATION_EXPECT_RXC1", "VALIDATION_LEGACY_BRIDGE_RELEASES", "VALIDATION_LAUNCHER",
    "VALIDATION_BROWSER", "VALIDATION_URL", "VALIDATION_TLS_CERT", "VALIDATION_PERF_JSON", "VALIDATION_LOG_ROOT", "VALIDATION_MEMORY_MAX",
}

LINE = re.compile(r"^([A-Z][A-Z0-9_]*)=(.*)$")


def trim(value: str) -> str:
    """The activator's trim_config_value: one pair of surrounding quotes, nothing else."""
    if value.startswith('"'):
        value = value[1:]
    if value.endswith('"'):
        value = value[:-1]
    if value.startswith("'"):
        value = value[1:]
    if value.endswith("'"):
        value = value[:-1]
    return value


def parse(path: str | None) -> list[tuple[str, str]]:
    """The key=value lines of a configuration file, in order. Comments and blank lines are ignored."""
    if not path or not os.path.exists(path):
        return []
    pairs = []
    with open(path, encoding="utf-8") as handle:
        for raw in handle.read().splitlines():
            if not raw.strip() or raw.lstrip().startswith("#"):
                continue
            match = LINE.match(raw)
            if match:
                pairs.append((match.group(1), trim(match.group(2))))
    return pairs


def render(pairs: list[tuple[str, str]]) -> str:
    return "".join(f'{key}="{value}"\n' for key, value in pairs)


def merge(defaults_path: str, existing_path: str | None, managed: str, notes: list[str]) -> str:
    defaults_text = open(defaults_path, encoding="utf-8").read()
    comments = [line for line in defaults_text.splitlines() if line.strip().startswith("#")]
    default_pairs = parse(defaults_path)
    default_keys = {key for key, _ in default_pairs}
    existing = {}
    for key, value in parse(existing_path):
        existing[key] = value            # a later line wins, as in the activator's loader

    for key in existing:
        if key not in KNOWN_KEYS:
            notes.append(f"dropped {key}: the activator does not know it")

    chosen: dict[str, str] = {}
    for key, check in ADMIN_KEYS.items():
        if key in existing:
            if check(existing[key]):
                chosen[key] = existing[key]
            else:
                notes.append(f"ignored the invalid existing {key}={existing[key]!r}")
    if managed == "enable":
        chosen["ACTIVATION_ENABLED"] = "1"
    elif managed == "disable":
        chosen["ACTIVATION_ENABLED"] = "0"

    out_pairs: list[tuple[str, str]] = []
    for key, value in default_pairs:
        out_pairs.append((key, chosen.get(key, value)))
    for key, value in chosen.items():
        if key not in default_keys:
            out_pairs.append((key, value))

    for key, value in chosen.items():
        before = existing.get(key)
        if key == "ACTIVATION_ENABLED" and managed in ("enable", "disable"):
            notes.append(f"managed updates {'enabled' if value == '1' else 'disabled'} by the installer's explicit choice (was {before!r})")
        elif before == value:
            notes.append(f"kept {key}={value}")
    return "".join(line + "\n" for line in comments) + render(out_pairs)


def write_atomic(path: str, text: str, mode: int) -> None:
    if os.path.islink(path):
        raise SystemExit(f"refusing to write through the symbolic link {path}")
    directory = os.path.dirname(os.path.abspath(path))
    descriptor, temporary = tempfile.mkstemp(prefix=".saturn-release-config.", dir=directory)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        if os.path.exists(temporary):
            os.unlink(temporary)
        raise
    directory_fd = os.open(directory, os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    m = sub.add_parser("merge", help="write the activation configuration from the installer's defaults and the existing file")
    m.add_argument("--defaults", required=True, help="the installer's rendered defaults (key=value lines)")
    m.add_argument("--existing", help="the current /etc/default/saturn-release-activate, if there is one")
    m.add_argument("--out", required=True)
    m.add_argument("--managed-updates", choices=("keep", "enable", "disable"), default="keep",
                   help="keep (default): never change an existing activation policy; enable/disable: set it, as an explicit choice")
    m.add_argument("--mode", default="0644")
    args = parser.parse_args(argv)
    notes: list[str] = []
    text = merge(args.defaults, args.existing, args.managed_updates, notes)
    write_atomic(args.out, text, int(args.mode, 8))
    for note in notes:
        print(f"[saturn-release-config] {note}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
