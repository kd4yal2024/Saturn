#!/usr/bin/env python3
"""Read-only reader of the acquisition owner's cached telemetry, meant to run ON the Pi.

It reads the Bridge's cached perf.json and /proc, and nothing else. It never writes a file, never opens an XDMA
device, never reads an FPGA register and never starts a Bridge. Standard library only, so it can be sent over an SSH
session as a program argument and leave nothing installed on the host.

Protocol (one long-lived process per measurement window, so one SSH session serves the whole window):
  stdin line "read"  ->  one line of JSON on stdout
  stdin line "quit", or end of input  ->  exit 0

Every answer is a single JSON object:

  {"ok": true, "schema": "saturn-owner-reader-v1",
   "piReadAtMs": ...,              this machine's clock when the file was read
   "document": {...},              the cached document exactly as parsed (the original, never edited or refreshed)
   "documentSha256": "...",        of the raw bytes that were read
   "piSourceUpdatedAtMs": ...,     the document's OWN update time (the Bridge writes it with the Pi's clock)
   "piSourceAgeMs": ...,           piReadAtMs - piSourceUpdatedAtMs, both on the Pi's clock
   "fileMtimeMs": ...,
   "owner": {"pid": ..., "alive": ..., "startTicks": ..., "clockTicksPerSecond": ..., "bootId": "...",
             "exe": "...", "exeSha256": "...", "exeError": null, "mainPid": ..., "mainPidMatches": ...}}

or {"ok": false, "error": "...", "piReadAtMs": ...} when the document cannot be read or parsed. An unavailable
read is reported as unavailable; it is never replaced by zeros, and the document's timestamp is never refreshed.
The reader reports the age; it does not judge it. Rejecting a stale read is the checker's job, so that a stale
document is kept as evidence rather than silently dropped.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time

SCHEMA = "saturn-owner-reader-v1"
DEFAULT_PATH = "/run/saturn-bridge/perf.json"   # the source's default; the active configured path must be verified per host
_EXE_CACHE = {}


def now_ms():
    return int(time.time() * 1000)


def _reject_constant(name):
    raise ValueError(f"non-finite JSON constant {name}")


def read_document(path):
    with open(path, "rb") as fh:
        raw = fh.read()
    st = os.stat(path)
    document = json.loads(raw.decode("utf-8"), parse_constant=_reject_constant)
    return raw, document, st


def proc_start_ticks(pid):
    """Field 22 of /proc/<pid>/stat: the process start time in clock ticks since boot."""
    with open(f"/proc/{pid}/stat", "rb") as fh:
        stat = fh.read().decode("utf-8", "replace")
    rest = stat[stat.rindex(")") + 2:].split()   # the command name may contain spaces and parentheses
    return int(rest[19])


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def owner_identity(pid, service):
    info = {"pid": pid, "alive": False, "startTicks": None, "clockTicksPerSecond": os.sysconf("SC_CLK_TCK"),
            "bootId": None, "exe": None, "exeSha256": None, "exeError": None, "mainPid": None, "mainPidMatches": None}
    try:
        with open("/proc/sys/kernel/random/boot_id") as fh:
            info["bootId"] = fh.read().strip()
    except OSError:
        pass
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        info["exeError"] = "the document names no process id"
        return info
    try:
        info["startTicks"] = proc_start_ticks(pid)
        info["alive"] = True
    except (OSError, ValueError, IndexError):
        info["exeError"] = f"process {pid} is not running"
        return info
    try:
        exe = os.readlink(f"/proc/{pid}/exe")
        info["exe"] = exe[: -len(" (deleted)")] if exe.endswith(" (deleted)") else exe   # (no str.removesuffix: older Pi images)
        key = (pid, info["startTicks"])
        if key not in _EXE_CACHE:
            # Hash the running image through /proc, not the path on disk: the file may have been replaced since.
            _EXE_CACHE[key] = sha256_file(f"/proc/{pid}/exe")
        info["exeSha256"] = _EXE_CACHE[key]
    except OSError as error:
        info["exeError"] = f"cannot read the running executable: {error}"
    if service:
        try:
            out = subprocess.run(["systemctl", "show", service, "-p", "MainPID", "--value"], capture_output=True, text=True, timeout=2)
            main_pid = int(out.stdout.strip())
            info["mainPid"] = main_pid
            info["mainPidMatches"] = main_pid == pid
        except (OSError, ValueError, subprocess.SubprocessError):
            pass
    return info


def read_once(path, service=None):
    read_at = now_ms()
    try:
        raw, document, st = read_document(path)
    except (OSError, ValueError) as error:
        return {"ok": False, "schema": SCHEMA, "piReadAtMs": read_at, "path": path, "error": f"cannot read {path}: {error}"}
    updated = document.get("updated_at_ms") if isinstance(document, dict) else None
    metrics = document.get("metrics") if isinstance(document, dict) else None
    if isinstance(updated, bool) or not isinstance(updated, int) or not isinstance(metrics, dict):
        return {"ok": False, "schema": SCHEMA, "piReadAtMs": read_at, "path": path,
                "error": "the document has no integer updated_at_ms or no metrics object"}
    return {
        "ok": True, "schema": SCHEMA, "piReadAtMs": read_at, "path": path,
        "document": document, "documentSha256": hashlib.sha256(raw).hexdigest(),
        "piSourceUpdatedAtMs": updated, "piSourceAgeMs": read_at - updated, "fileMtimeMs": int(st.st_mtime * 1000),
        "owner": owner_identity(metrics.get("pid"), service),
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--path", default=DEFAULT_PATH, help=f"the owner's cached perf.json (default {DEFAULT_PATH}; verify the host's active path)")
    ap.add_argument("--service", help="optional systemd unit whose MainPID must match the document's pid")
    ap.add_argument("--once", action="store_true", help="answer one read and exit (no protocol)")
    args = ap.parse_args(argv)
    if args.once:
        print(json.dumps(read_once(args.path, args.service), separators=(",", ":")))
        return 0
    for line in sys.stdin:
        command = line.strip()
        if command == "quit":
            return 0
        if command == "read":
            sys.stdout.write(json.dumps(read_once(args.path, args.service), separators=(",", ":")) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
