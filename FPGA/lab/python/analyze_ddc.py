#!/usr/bin/env python3
"""Analyze complex I/Q output produced by the Saturn RX DDC testbench."""

from __future__ import annotations

import argparse
from pathlib import Path

from sfdr import emit_metrics, load_numeric, measure_tone


def analyze(path: Path, sample_rate: float, bits: int, window: str) -> dict[str, object]:
    data = load_numeric(path, 2)
    iq = data[:, 0] + 1j * data[:, 1]
    metrics = measure_tone(iq, sample_rate, float((1 << (bits - 1)) - 1), window=window)
    metrics.update({"kind": "ddc", "source": str(path), "sample_bits": bits})
    return metrics


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=Path)
    parser.add_argument("--sample-rate", type=float, default=1_536_000.0)
    parser.add_argument("--bits", type=int, default=24)
    parser.add_argument("--window", choices=("blackman", "hann", "rectangular"), default="blackman")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    emit_metrics(analyze(args.samples, args.sample_rate, args.bits, args.window), args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
