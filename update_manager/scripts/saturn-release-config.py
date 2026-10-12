#!/usr/bin/env python3
"""Write /etc/default/saturn-release-activate from the installer's defaults without losing what an administrator chose.

The installer used to rewrite that file from fixed text on every run, which silently dropped an administrator's browser memory allowance
(VALIDATION_MEMORY_MAX) and reset the activation policy. This tool keeps the settings an administrator owns and takes everything else
from the installer:

  * Installer-owned keys (paths, service names, URLs): always the installer's current values.
  * Administrator-owned keys (ACTIVATION_ENABLED, VALIDATION_ENABLED, VALIDATION_MEMORY_MAX, VALIDATION_MEMORY_RESERVE, VALIDATION_TIMEOUT_SECONDS,
    VALIDATION_LEGACY_BRIDGE_RELEASES, VALIDATION_EXPECT_RXC1, READY_TIMEOUT_SECONDS, VALIDATION_LAUNCHER, VALIDATION_URL,
    VALIDATION_BROWSER, VALIDATION_TLS_CERT, VALIDATION_PERF_JSON, VALIDATION_LOG_ROOT): the existing value when it is valid,
    otherwise the installer's default (or none, for a key the defaults do not set). Every key the activator accepts is either one of
    these or one the installer sets; none can be lost without a report. An invalid existing value is reported and dropped, never
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

def _safe_path(v: str) -> bool:
    """The activator's rule for every configured path: absolute, and no space, tab or line break."""
    return v.startswith("/") and re.search(r"[\t\r\n ]", v) is None


ADMIN_KEYS = {
    "ACTIVATION_ENABLED": lambda v: v in ("0", "1"),
    "VALIDATION_ENABLED": lambda v: v in ("0", "1"),
    "VALIDATION_MEMORY_MAX": lambda v: re.fullmatch(r"[1-9][0-9]*[MG]", v) is not None,
    # memory kept free beyond the browser's cap (default 32M in the activator; at least 8 MiB, the activator's floor)
    "VALIDATION_MEMORY_RESERVE": lambda v: re.fullmatch(r"[1-9][0-9]*[MG]", v) is not None and int(v[:-1]) * (1024 if v.endswith("G") else 1) >= 8,
    "VALIDATION_TIMEOUT_SECONDS": lambda v: re.fullmatch(r"[1-9][0-9]*", v) is not None and int(v) <= 600,
    "VALIDATION_LEGACY_BRIDGE_RELEASES": lambda v: all(re.fullmatch(r"[0-9a-f]{40}", item) for item in v.split()),
    "VALIDATION_EXPECT_RXC1": lambda v: v in ("any", "valid", "unarmed", "disabled", "unsupported", "unavailable", "partial"),
    "READY_TIMEOUT_SECONDS": lambda v: re.fullmatch(r"[1-9][0-9]*", v) is not None and int(v) <= 300,
    # How the validation reaches and runs the browser; the installer does not set them, an administrator may (a custom HTTPS port or
    # certificate, another browser or perf file). The rules are the activator's (validate_configuration).
    # "direct" runs the browser as the caller; the activator refuses it as root, and this runs as root, so it is only valid for a caller that is not.
    "VALIDATION_LAUNCHER": lambda v: v == "systemd" or (v == "direct" and os.geteuid() != 0),
    "VALIDATION_URL": lambda v: re.fullmatch(r"https://127\.0\.0\.1:[0-9]+/[A-Za-z0-9_./-]+(\?[A-Za-z0-9_=&.-]*)?", v) is not None,
    "VALIDATION_BROWSER": _safe_path,
    "VALIDATION_TLS_CERT": _safe_path,
    "VALIDATION_PERF_JSON": _safe_path,
    "VALIDATION_LOG_ROOT": _safe_path,
}

# Every key the root activator's loader accepts (load_config in saturn-release-activate-root.sh). A key outside this set makes the
# activator refuse the whole file.
KNOWN_KEYS = {
    "ACTIVATION_ENABLED", "SATURN_ROOT", "RELEASES_ROOT", "CURRENT_LINK", "TRANSACTION_FILE", "LOCK_FILE", "MANIFEST_TOOL", "COMPONENTS_FILE",
    "STATE_TOOL", "STATE_ROOT", "STATE_BACKUP_ROOT", "SYSTEMD_ROOT", "SATURN_GO_SERVICE", "BRIDGE_SERVICE", "P2APP_SERVICE", "SATURN_GO_READY_URL",
    "P23_PERF_URL", "BACKEND_STATUS_HELPER", "READY_TIMEOUT_SECONDS", "P2APP_PANEL_ENABLED", "TRANSACTION_GROUP", "VALIDATION_ENABLED", "VALIDATOR",
    "VALIDATOR_GUARD", "VALIDATION_TIMEOUT_SECONDS", "VALIDATION_EXPECT_RXC1", "VALIDATION_LEGACY_BRIDGE_RELEASES", "VALIDATION_LAUNCHER",
    "VALIDATION_BROWSER", "VALIDATION_URL", "VALIDATION_TLS_CERT", "VALIDATION_PERF_JSON", "VALIDATION_LOG_ROOT", "VALIDATION_MEMORY_MAX",
    "VALIDATION_MEMORY_RESERVE",
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


def sudoers_fragment(user: str, broker: str, config: str) -> str:
    """The sudoers lines that let the service user call the managed-update broker, and nothing when managed updates are off. The installer writes
    the sudoers file from scratch on every run, so turning managed updates off removes the authorization with it. The broker accepts only
    --preflight and --dispatch from this user and validates everything else itself; the wildcard only lets the dispatch carry its one bundle path."""
    if not re.fullmatch(r"[a-z_][a-z0-9_-]*", user):
        raise SystemExit(f"refusing to write a sudoers rule for the unusual user name {user!r}")
    if not re.fullmatch(r"/[A-Za-z0-9_./-]+", broker) or ".." in broker.split("/"):
        raise SystemExit(f"refusing to write a sudoers rule for the unusual broker path {broker!r}")
    values = dict(parse(config))
    if values.get("ACTIVATION_ENABLED") != "1":
        return ""
    return (f"{user} ALL=(root) NOPASSWD: {broker} --preflight\n"
            f"{user} ALL=(root) NOPASSWD: {broker} --preflight --allow-legacy-migration\n"
            f"{user} ALL=(root) NOPASSWD: {broker} --dispatch *\n")


COMMIT_NAME = re.compile(r"[0-9a-f]{40}")


def detect_layout(current_link: str, releases_root: str) -> tuple[str, str | None]:
    """('release', commit) when the active pointer is a symbolic link to an installed release; ('legacy', None) otherwise (a legacy deployment,
    or a machine with nothing deployed yet)."""
    if os.path.islink(current_link):
        target = os.path.realpath(current_link)
        if os.path.dirname(target) == os.path.realpath(releases_root) and COMMIT_NAME.fullmatch(os.path.basename(target)):
            return "release", os.path.basename(target)
    return "legacy", None


def skip_application(layout: str, mode: str) -> bool:
    """Whether the installer leaves the application (server binary, web assets, Bridge binary, the services' running state) alone. On a release
    layout the application is the release's business: an install there deploys nothing into /opt/saturn-go that the services would not run.
    auto: skip on a release layout; force: deploy regardless; skip: never deploy."""
    return mode == "skip" or (mode == "auto" and layout == "release")


def plan_text(config: str, current_link: str, releases_root: str, managed: str, application: str, default_enabled: str) -> str:
    layout, commit = detect_layout(current_link, releases_root)
    skip = skip_application(layout, application)
    existing = dict(parse(config)) if os.path.exists(config) else {}
    before = existing.get("ACTIVATION_ENABLED") if ADMIN_KEYS["ACTIVATION_ENABLED"](existing.get("ACTIVATION_ENABLED", "")) else None
    after = {"enable": "1", "disable": "0"}.get(managed, before if before is not None else default_enabled)
    kept = [f"{k}={existing[k]}" for k in ADMIN_KEYS if k in existing and k != "ACTIVATION_ENABLED" and ADMIN_KEYS[k](existing[k])]
    lines = ["installer plan (nothing is changed)",
             f"  application layout: {layout}" + (f" (active release {commit})" if commit else ""),
             "  application payload: " + ("NOT deployed (" + ("the releases own the application" if layout == "release" else "SATURN_INSTALL_APPLICATION=skip") + "): no server or Bridge build, no web assets, no service restart"
                                          if skip else "deployed (server and Bridge are built and installed, services restarted)"),
             f"  managed updates (ACTIVATION_ENABLED): {before if before is not None else 'unset'} -> {after} (SATURN_MANAGED_UPDATES={managed})",
             "  administrator settings kept: " + (", ".join(kept) if kept else "none set"),
             "  readiness check after the run: " + (f"the running release {commit}" if skip and commit else "the commit of this checkout")]
    if layout == "legacy" and after == "1":
        lines.append("  NOTE: managed updates are on but no release is active yet; the first update must be run with --migrate-to-releases")
    if layout == "release" and not skip:
        lines.append("  WARNING: SATURN_INSTALL_APPLICATION=force deploys a legacy payload into /opt/saturn-go that the active release overrides")
    return "\n".join(lines) + "\n"


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
    g = sub.add_parser("get", help="print one setting of a configuration file (empty when unset)")
    g.add_argument("--file", required=True)
    g.add_argument("--key", required=True)
    u = sub.add_parser("sudoers", help="print the sudoers lines for the managed-update broker (nothing when managed updates are off)")
    u.add_argument("--user", required=True)
    u.add_argument("--broker", required=True)
    u.add_argument("--config", required=True, help="the written activation configuration; ACTIVATION_ENABLED decides")
    lo = sub.add_parser("layout", help="print '<layout> <active commit or -> <skip application 0|1>'")
    lo.add_argument("--current-link", required=True)
    lo.add_argument("--releases-root", required=True)
    lo.add_argument("--application", choices=("auto", "force", "skip"), default="auto")
    pl = sub.add_parser("plan", help="print what the installer would do, changing nothing")
    pl.add_argument("--config", required=True)
    pl.add_argument("--current-link", required=True)
    pl.add_argument("--releases-root", required=True)
    pl.add_argument("--managed-updates", choices=("keep", "enable", "disable"), default="keep")
    pl.add_argument("--application", choices=("auto", "force", "skip"), default="auto")
    pl.add_argument("--default-enabled", choices=("0", "1"), default="0")
    args = parser.parse_args(argv)
    if args.command == "layout":
        layout, commit = detect_layout(args.current_link, args.releases_root)
        print(layout, commit or "-", int(skip_application(layout, args.application)))
        return 0
    if args.command == "plan":
        sys.stdout.write(plan_text(args.config, args.current_link, args.releases_root, args.managed_updates, args.application, args.default_enabled))
        return 0
    if args.command == "get":
        print(dict(parse(args.file)).get(args.key, ""))
        return 0
    if args.command == "sudoers":
        sys.stdout.write(sudoers_fragment(args.user, args.broker, args.config))
        return 0
    notes: list[str] = []
    text = merge(args.defaults, args.existing, args.managed_updates, notes)
    write_atomic(args.out, text, int(args.mode, 8))
    for note in notes:
        print(f"[saturn-release-config] {note}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
