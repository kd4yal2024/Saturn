#!/usr/bin/env python3
"""Read-only M1 checks against Git blobs, not potentially modified WDSP files.

Usage: python3 verify-source-contract.py /path/to/OpenHPSDR-wdsp
No native library is executed, no radio is accessed, no source is changed.
"""
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

PIN = "b02d5bac675dd2f33ec2bab2b339f79a597c47dd"
SATURN_BASE = "861f0fbc79e0e36c68a5c8ddc157e010b8be20df"
HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]


def blob(repo, ref, name):
    return subprocess.check_output(["git", "-C", str(repo), "show", f"{ref}:{name}"])


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    upstream = Path(sys.argv[1])
    sources = {name: blob(upstream, PIN, f"wdsp 2.10/Source/{name}")
               for name in ("wdsp.h", "calcc.c", "TXA.c", "delay.c", "version.c")}
    calcc = sources["calcc.c"].decode()
    # Check allocation bounds from the actual pinned implementation, not folklore.
    assert re.search(r"#define\s+DISP_PTS\s+512\b", calcc)
    assert re.search(r"#define\s+SAMPLE_NBUCKS\s+16\b", calcc)
    collection = calcc[calcc.index("static PSCOLLECTION build_collection()"):
                       calcc.index("static void teardown_collection")]
    buckets = re.findall(r"collect->tpb\[\s*(\d+)\s*\]\s*=\s*(\d+)", collection)
    assert len(buckets) == 16 and sum(int(n) for _, n in buckets) == 4096
    assert "16 * sizeof(int)" in calcc[calcc.index("void GetPSInfo"):][:400]
    assert "const int version = 210;" in sources["version.c"].decode()
    # The upstream public header is standalone on Linux. Our declarations and
    # function-pointer type assertions must agree with it under a real C compiler.
    translation_unit = sources["wdsp.h"] + b"\n" + (HERE / "native-abi.h").read_bytes()
    subprocess.run(["gcc", "-std=c11", "-Werror", "-Wall", "-Wextra",
                    "-fsyntax-only", "-x", "c", "-"], input=translation_unit, check=True)
    incompatible_header = re.sub(
        rb"WDSP_API void GetPSDisp\s*\([^;]+;",
        b"WDSP_API void GetPSDisp(int channel, double *x, double *ym, "
        b"double *yc, double *ys, double *cm, double *cc, double *cs);",
        sources["wdsp.h"], count=1)
    assert incompatible_header != sources["wdsp.h"]
    negative = subprocess.run(
        ["gcc", "-std=c11", "-Werror", "-fsyntax-only", "-x", "c", "-"],
        input=incompatible_header + b"\n" + (HERE / "native-abi.h").read_bytes(),
        capture_output=True)
    assert negative.returncode != 0 and b"GetPSDisp" in negative.stderr
    rollback = blob(REPO, SATURN_BASE, "FPGA/saturnprimary2026v31.bin")
    assert hashlib.sha256(rollback).hexdigest() == (
        "5fe07f14c119766481afa1ceff46113bfc085af87ee91fab1fe42eb67c75172d")
    print(json.dumps({
        "scope": "offline-source-and-header-only",
        "saturn_base": SATURN_BASE,
        "wdsp_source_commit": PIN,
        "source_sha256": {k: hashlib.sha256(v).hexdigest() for k, v in sources.items()},
        "wdsp_version_expected": 210,
        "get_ps_info_ints": 16,
        "get_ps_disp_capacities": {"measured_each": 4096, "correction_each": 512},
        "c_signature_check": "passed",
        "incompatible_seven_array_header": "rejected-as-required",
        "repo_v31_bin": "verified-rollback-1.31.001-not-RXC1",
        "installed_native_archive_identity": "unknown-not-inspected",
        "native_execution": False,
        "radio_access": False
    }, indent=2))


if __name__ == "__main__":
    main()
