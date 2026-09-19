#!/usr/bin/env python3
"""Apply the benchmarked index-only optimization to a copied WDSP source file."""
import argparse
import hashlib
from pathlib import Path

PIN = "584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3"
SOURCE_SHA256 = "3cd128001a52c31c84151052e444fa956ea8d67b259e1fbb0424492bf988918c"
OPTIMIZED_SHA256 = "45462ec55fcf5c8b241351ecbb2d8857b9248a04fb641d589c2796f808b6f255"
PATCH_ID = "hbres-index-wrap-v1"

REPLACEMENTS = {
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


def digest(source):
    return hashlib.sha256(source.encode("utf-8")).hexdigest()


def candidate(source):
    # Normalize only CRLF checkout line endings; reject all other source drift.
    source = source.replace("\r\n", "\n")
    if digest(source) != SOURCE_SHA256:
        raise ValueError("Unexpected WDSP reshb.c source; refusing index optimization")
    # All unwrapped indices lie in [-(N-1), N-1]. One addition when negative
    # is exactly equivalent to the original modulo, without division.
    for old, new in REPLACEMENTS.items():
        if source.count(old) != 1:
            raise ValueError("Pinned resampler index expression changed")
        source = source.replace(old, new)
    if digest(source) != OPTIMIZED_SHA256:
        raise ValueError("Unexpected optimized reshb.c digest; refusing output")
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--check", action="store_true", help="Validate without writing")
    args = parser.parse_args()
    # Read bytes so newline normalization is deliberate and independently tested.
    optimized = candidate(args.source.read_bytes().decode("utf-8"))
    if not args.check:
        args.source.write_bytes(optimized.encode("utf-8"))
    print(f"WDSP optimization: {PATCH_ID} {'validated' if args.check else 'applied'} "
          f"source={SOURCE_SHA256} output={OPTIMIZED_SHA256}")


if __name__ == "__main__":
    main()
