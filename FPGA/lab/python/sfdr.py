#!/usr/bin/env python3
"""Frequency-domain measurements for Saturn simulation and capture data."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np


_FLOOR = np.finfo(np.float64).tiny


def db20(value: float | np.ndarray) -> float | np.ndarray:
    """Convert a linear voltage/amplitude ratio to decibels."""
    result = 20.0 * np.log10(np.maximum(np.abs(value), _FLOOR))
    return float(result) if np.ndim(result) == 0 else result


def load_numeric(path: str | Path, columns: int) -> np.ndarray:
    """Load comma- or whitespace-separated numeric samples."""
    sample_path = Path(path)
    try:
        data = np.loadtxt(sample_path, delimiter=",", ndmin=2)
    except ValueError:
        data = np.loadtxt(sample_path, ndmin=2)
    if data.shape[1] < columns:
        raise ValueError(f"{sample_path} has {data.shape[1]} column(s); expected {columns}")
    if data.shape[0] < 16:
        raise ValueError(f"{sample_path} has only {data.shape[0]} samples; expected at least 16")
    return data


def _window(name: str, length: int) -> np.ndarray:
    windows = {
        "blackman": np.blackman,
        "hann": np.hanning,
        "rectangular": np.ones,
    }
    try:
        return np.asarray(windows[name](length), dtype=np.float64)
    except KeyError as exc:
        raise ValueError(f"unsupported window: {name}") from exc


def amplitude_spectrum(
    samples: np.ndarray,
    sample_rate: float,
    *,
    window: str = "blackman",
) -> tuple[np.ndarray, np.ndarray]:
    """Return frequency bins and coherent-gain-corrected peak amplitudes."""
    values = np.asarray(samples)
    weights = _window(window, values.size)
    coherent_gain = float(np.sum(weights))
    if np.iscomplexobj(values):
        spectrum = np.fft.fftshift(np.fft.fft(values * weights))
        frequencies = np.fft.fftshift(np.fft.fftfreq(values.size, 1.0 / sample_rate))
        amplitudes = np.abs(spectrum) / coherent_gain
    else:
        spectrum = np.fft.rfft(values * weights)
        frequencies = np.fft.rfftfreq(values.size, 1.0 / sample_rate)
        amplitudes = np.abs(spectrum) / coherent_gain
        if amplitudes.size > 2:
            amplitudes[1:-1] *= 2.0
    return frequencies, amplitudes


def measure_tone(
    samples: np.ndarray,
    sample_rate: float,
    full_scale: float,
    *,
    window: str = "blackman",
    guard_bins: int = 4,
) -> dict[str, Any]:
    """Measure the dominant tone, DC, RMS level, and largest non-tone spur."""
    values = np.asarray(samples)
    if values.ndim != 1:
        raise ValueError("samples must be one-dimensional")
    if values.size < 16:
        raise ValueError("at least 16 samples are required")
    if sample_rate <= 0 or full_scale <= 0:
        raise ValueError("sample_rate and full_scale must be positive")

    dc = np.mean(values)
    centered = values - dc
    frequencies, amplitudes = amplitude_spectrum(centered, sample_rate, window=window)

    usable = np.ones(amplitudes.size, dtype=bool)
    bin_width = sample_rate / values.size
    usable[np.abs(frequencies) < 0.5 * bin_width] = False
    if not np.any(usable):
        raise ValueError("no non-DC FFT bins are available")

    search = np.where(usable, amplitudes, -1.0)
    fundamental_index = int(np.argmax(search))
    fundamental = float(amplitudes[fundamental_index])

    spur_mask = usable.copy()
    lo = max(0, fundamental_index - guard_bins)
    hi = min(spur_mask.size, fundamental_index + guard_bins + 1)
    spur_mask[lo:hi] = False
    spur = float(np.max(amplitudes[spur_mask])) if np.any(spur_mask) else 0.0

    rms = float(np.sqrt(np.mean(np.abs(centered) ** 2)))
    dc_level = float(np.abs(dc))
    return {
        "samples": int(values.size),
        "sample_rate_hz": float(sample_rate),
        "bin_width_hz": float(bin_width),
        "fundamental_hz": float(frequencies[fundamental_index]),
        "fundamental_dbfs": float(db20(fundamental / full_scale)),
        "rms_dbfs": float(db20(rms / full_scale)),
        "dc_dbfs": float(db20(dc_level / full_scale)),
        "largest_spur_dbfs": float(db20(spur / full_scale)),
        "sfdr_dbc": float(db20(fundamental / max(spur, _FLOOR))),
        "window": window,
    }


def emit_metrics(metrics: dict[str, Any], output: str | Path | None) -> None:
    rendered = json.dumps(metrics, indent=2, sort_keys=True) + "\n"
    if output is None:
        print(rendered, end="")
    else:
        Path(output).write_text(rendered, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=Path, help="one-column real-valued sample file")
    parser.add_argument("--sample-rate", type=float, required=True, help="sample rate in Hz")
    parser.add_argument("--full-scale", type=float, required=True, help="positive peak full scale")
    parser.add_argument("--window", choices=("blackman", "hann", "rectangular"), default="blackman")
    parser.add_argument("--output", type=Path, help="write JSON metrics to this path")
    args = parser.parse_args()

    data = load_numeric(args.samples, 1)[:, 0]
    emit_metrics(
        measure_tone(data, args.sample_rate, args.full_scale, window=args.window),
        args.output,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
