#!/usr/bin/env python3
"""Fix the pinned WDSP 2.10 FM pre-emphasis filter resize use-after-free."""
import argparse
import hashlib
from pathlib import Path

PIN = "b02d5bac675dd2f33ec2bab2b339f79a597c47dd"
SOURCE_SHA256 = "7ca631d8cd9033f950aaf1613825e273f6d0380d1fa1c7ed33ef0d53d379b72a"
PATCHED_SHA256 = "7f48c82c5f3a28897bfe9302d14168a88d2aac46a04c1a4f3782388af8cdf0b0"
PATCH_ID = "emph-filter-rebuild-v1"

OLD = "\t\tbuild_fcimp(a->nc, a->wintype);\n"
NEW = "\t\ta->pfcimp = build_fcimp(a->nc, a->wintype);\n"


def digest(source):
    return hashlib.sha256(source.encode("utf-8")).hexdigest()


def candidate(source):
    # Only the pinned source is safe to transform. Normalize CRLF checkouts.
    source = source.replace("\r\n", "\n")
    if digest(source) != SOURCE_SHA256:
        raise ValueError("Unexpected WDSP emph.c source; refusing filter fix")
    if source.count(OLD) != 1:
        raise ValueError("Pinned FM pre-emphasis filter resize changed")
    patched = source.replace(OLD, NEW, 1)
    if digest(patched) != PATCHED_SHA256:
        raise ValueError("Unexpected patched emph.c digest; refusing output")
    return patched


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--check", action="store_true", help="Validate without writing")
    args = parser.parse_args()
    patched = candidate(args.source.read_bytes().decode("utf-8"))
    if not args.check:
        args.source.write_bytes(patched.encode("utf-8"))
    print(f"WDSP fix: {PATCH_ID} {'validated' if args.check else 'applied'} "
          f"source={SOURCE_SHA256} output={PATCHED_SHA256}")


if __name__ == "__main__":
    main()
