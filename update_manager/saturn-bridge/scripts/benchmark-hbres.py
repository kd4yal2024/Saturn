#!/usr/bin/env python3
"""Isolated pinned-WDSP experiment; never modifies sources or installs an archive."""
import argparse
import hashlib
from pathlib import Path
import re
import shlex
import subprocess
import tempfile

PIN = "584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3"
SOURCE_SHA256 = "3cd128001a52c31c84151052e444fa956ea8d67b259e1fbb0424492bf988918c"


def candidate(source):
    # For 0 <= ring_ptr < N and 0 <= center +/- j <= N-1, each
    # unwrapped index lies in [-(N-1), N-1]. One conditional addition
    # therefore exactly replaces the double modulo, with no upper wrap.
    replacements = {
        "h_center_idx = (r->ring_ptr - center + r->N) % r->N;":
        "h_center_idx = r->ring_ptr - center;\n"
        "            if (h_center_idx < 0) h_center_idx += r->N;",
        "idx_left  = ((r->ring_ptr - (center - j)) % r->N + r->N) % r->N;":
        "idx_left = r->ring_ptr - (center - j);\n"
        "                if (idx_left < 0) idx_left += r->N;",
        "idx_right = ((r->ring_ptr - (center + j)) % r->N + r->N) % r->N;":
        "idx_right = r->ring_ptr - (center + j);\n"
        "                if (idx_right < 0) idx_right += r->N;",
    }
    for old, new in replacements.items():
        if source.count(old) != 1:
            raise ValueError("Pinned resampler index expression changed")
        source = source.replace(old, new)
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wdsp_repo", type=Path)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--cflags", default="-O3")
    args = parser.parse_args()
    def pinned(name):
        return subprocess.check_output([
            "git", "-C", str(args.wdsp_repo), "show", f"{PIN}:wdsp 2.00/Source/{name}"
        ])
    raw = pinned("reshb.c")
    if hashlib.sha256(raw).hexdigest() != SOURCE_SHA256:
        raise ValueError("Unexpected pinned resampler source")
    source = raw.decode()
    taps = sorted({int(n) for n in re.findall(r"taps\[\d+\] = (\d+);", source)})
    for n in taps:
        center = (n - 1) // 2
        for pointer in range(n):
            for offset in [center, *(center - j for j in range(1, center + 1, 2)),
                           *(center + j for j in range(1, center + 1, 2))]:
                index = pointer - offset
                wrapped = index + n if index < 0 else index
                if wrapped != index % n:
                    raise AssertionError((n, pointer, offset))
    print(f"PASS exhaustive ring-index equivalence for pinned tap counts {taps}", flush=True)
    # Both implementations use the same coefficient generation and allocation
    # shim. No fast-math flags: compare unchanged arithmetic bit for bit.
    flags = shlex.split(args.cflags)
    if any("fast" in flag or "Ofast" in flag or "unsafe" in flag or "NDEBUG" in flag for flag in flags):
        raise ValueError("Unsafe floating-point flags are not allowed")
    with tempfile.TemporaryDirectory(prefix="saturn-hbres-") as tmp:
        root = Path(tmp)
        (root / "reshb.h").write_bytes(pinned("reshb.h"))
        (root / "comm.h").write_text('''#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <math.h>
#include "reshb.h"
#define PI 3.1415926535897932
#define TWOPI 6.2831853071795864
#define _aligned_free free
static void *malloc0(size_t n) { void *p = calloc(1, n); assert(p); return p; }
''')
        names = re.findall(r"^(?:void|HBResampler) (\w+)\(", source, re.M)
        optimized = candidate(source)
        for name in names:
            optimized = re.sub(r"\b" + name + r"\b", "opt_" + name, optimized)
        (root / "reference.c").write_text(source)
        (root / "candidate.c").write_text(optimized)
        harness = Path(__file__).with_name("benchmark-hbres.c")
        executable = root / "benchmark"
        command = [args.cc, *flags, "-I", str(root), str(root / "reference.c"),
                   str(root / "candidate.c"), str(harness), "-lm", "-o", str(executable)]
        print(f"WDSP pin={PIN} source_sha256={SOURCE_SHA256}", flush=True)
        print("Compile:", shlex.join(command), flush=True)
        subprocess.run(command, check=True)
        subprocess.run([str(executable)], check=True)


if __name__ == "__main__":
    main()
