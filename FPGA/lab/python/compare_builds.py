#!/usr/bin/env python3
"""Compare two Saturn lab JSON measurement files."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


DEFAULT_FIELDS = (
    "fundamental_hz",
    "fundamental_dbfs",
    "rms_dbfs",
    "dc_dbfs",
    "largest_spur_dbfs",
    "sfdr_dbc",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    candidate = json.loads(args.candidate.read_text(encoding="utf-8"))
    comparison: dict[str, object] = {
        "baseline": str(args.baseline),
        "candidate": str(args.candidate),
        "delta_candidate_minus_baseline": {},
    }
    deltas = comparison["delta_candidate_minus_baseline"]
    assert isinstance(deltas, dict)
    for field in DEFAULT_FIELDS:
        if field in baseline and field in candidate:
            deltas[field] = float(candidate[field]) - float(baseline[field])

    rendered = json.dumps(comparison, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
