#!/usr/bin/env python3
"""Read-only P2 V51 / FPGA V29 RX soak collector. Uses only /p23_perf."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import shutil
import statistics
import sys
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_ENDPOINT = "http://127.0.0.1:8080/p23_perf"
DEFAULT_INTERVAL = 5.0
DEFAULT_DURATION = 1800.0
EXPECTED_BUILD_ID = 1446131968
FIFO_DEPTHS = {"ddc": 16384, "duc": 4096, "mic": 256, "speaker": 1024}
V29_CHANNELS = ("ddc", "duc", "mic", "speaker")
PROFILES = ("controlled", "antenna")
DEFAULT_DDC_EXPECTATIONS = ((2, 384, False),)

BASE_LOSS_COUNTERS = (
    "high_priority_send_errors", "mic_send_errors", "mic_dma_errors",
    "ddc_send_errors", "ddc_dma_errors", "ddc_partial_sends",
    "ddc_header_errors", "wideband_send_errors", "duc_recv_errors",
    "duc_dma_errors", "speaker_recv_errors", "speaker_dma_errors",
    "fifo_rx_ddc_over_events", "fifo_mic_over_events",
    "fifo_duc_under_events", "fifo_speaker_under_events",
)
CONTROLLED_ONLY_LOSS_COUNTERS = ("adc_overflow_events",)
HOST_ACTIVITY_COUNTERS = (
    "duc_queue_drop_events", "duc_queue_dropped_frames",
    "duc_gap_events", "duc_gap_dropped_frames", "speaker_gap_events",
    "speaker_gap_dropped_frames", "speaker_stall_events",
    "speaker_underrun_queue_empty_events", "speaker_underrun_queue_ready_events",
)
RATE_COUNTERS = {
    "ddc_dma_ops_per_sec": "ddc_dma_reads",
    "mic_dma_ops_per_sec": "mic_dma_reads",
    "duc_dma_ops_per_sec": "duc_dma_writes",
    "speaker_dma_ops_per_sec": "speaker_dma_writes",
}
NETWORK_ERROR_FIELDS = {
    "rx": ("drop", "errs", "fifo", "frame"),
    "tx": ("drop", "errs", "fifo", "carrier", "colls"),
}

ARTIFACT_NAMES = (
    "collect.py", "collection_status.json", "end.json", "samples.ndjson",
    "start.json", "summary.json", "summary.md",
)


def positive_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0:
        raise argparse.ArgumentTypeError("must be a finite number greater than zero")
    return parsed


def sample_interval(value: str) -> float:
    parsed = positive_float(value)
    if parsed < DEFAULT_INTERVAL:
        raise argparse.ArgumentTypeError(
            f"must be at least {DEFAULT_INTERVAL:g} seconds so V29 snapshot generations can advance"
        )
    return parsed


def ddc_expectation(value: str) -> tuple[int, int, bool]:
    parts = value.split(":")
    if len(parts) not in (2, 3):
        raise argparse.ArgumentTypeError("must be ID:RATE_KHZ[:interleaved|noninterleaved]")
    try:
        ddc_id = int(parts[0])
        sample_rate_khz = int(parts[1])
    except ValueError as exc:
        raise argparse.ArgumentTypeError("DDC ID and rate must be integers") from exc
    if not 0 <= ddc_id <= 9 or sample_rate_khz <= 0:
        raise argparse.ArgumentTypeError("DDC ID must be 0..9 and rate must be positive")
    mode = parts[2] if len(parts) == 3 else "noninterleaved"
    if mode not in ("interleaved", "noninterleaved"):
        raise argparse.ArgumentTypeError("mode must be interleaved or noninterleaved")
    return ddc_id, sample_rate_khz, mode == "interleaved"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Collect a read-only P2 V51 / FPGA V29 RX soak. The controlled "
            "profile gates ADC overflow; the antenna profile records it without "
            "assigning speaker-test causality."
        )
    )
    parser.add_argument("--profile", choices=PROFILES, default="controlled")
    parser.add_argument("--output", type=Path, required=True,
                        help="new or empty artifact directory")
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument("--duration", type=positive_float, default=DEFAULT_DURATION,
                        help="planned duration in seconds (default: 1800)")
    parser.add_argument("--interval", type=sample_interval, default=DEFAULT_INTERVAL,
                        help="sample interval in seconds, minimum/default: 5")
    parser.add_argument(
        "--expect-ddc", action="append", type=ddc_expectation, default=None,
        metavar="ID:RATE_KHZ[:MODE]",
        help=(
            "expected enabled receiver; repeat for multiple DDCs. MODE defaults "
            "to noninterleaved. Default: 2:384"
        ),
    )
    args = parser.parse_args(argv)
    args.expected_ddcs = tuple(args.expect_ddc or DEFAULT_DDC_EXPECTATIONS)
    if len({item[0] for item in args.expected_ddcs}) != len(args.expected_ddcs):
        parser.error("each --expect-ddc ID may be specified only once")
    return args


def gated_loss_counters(profile: str) -> tuple[str, ...]:
    if profile == "controlled":
        return BASE_LOSS_COUNTERS + CONTROLLED_ONLY_LOSS_COUNTERS
    if profile == "antenna":
        return BASE_LOSS_COUNTERS
    raise ValueError(f"unknown validation profile: {profile}")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def atomic_json(path: Path, value: object) -> None:
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temp, path)


def fetch(endpoint: str) -> dict:
    request = urllib.request.Request(endpoint, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=3.0) as response:
        return json.load(response)


def current(perf: dict) -> dict:
    return perf["app_telemetry"]["current"]


def deep_get(value: dict, *keys: str, default=None):
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            return default
        value = value[key]
    return value


def number(value, default=0):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else default


def is_number(value) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def counter_delta(before: dict, after: dict, name: str) -> int | float | None:
    a = before.get(name)
    b = after.get(name)
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        return None
    return b - a


def routing_is_required(app: dict, expected_ddcs: tuple[tuple[int, int, bool], ...]) -> bool:
    ddc = deep_get(app, "routing", "ddc", default=[])
    enabled = sorted(
        (
            entry.get("id"),
            entry.get("sample_rate_khz"),
            entry.get("interleaved"),
        )
        for entry in ddc if entry.get("enabled") is True
    )
    return (
        enabled == sorted(expected_ddcs)
        and deep_get(app, "routing", "wideband", "adc1_enabled") is False
        and deep_get(app, "routing", "wideband", "adc2_enabled") is False
    )


def ddc_expectations_text(expected_ddcs: tuple[tuple[int, int, bool], ...]) -> str:
    return ", ".join(
        f"DDC{ddc_id} {rate} ksps {'interleaved' if interleaved else 'non-interleaved'}"
        for ddc_id, rate, interleaved in expected_ddcs
    )


def gate_violations(
    perf: dict,
    baseline: dict | None,
    previous: dict | None,
    profile: str,
    expected_ddcs: tuple[tuple[int, int, bool], ...] = DEFAULT_DDC_EXPECTATIONS,
    endpoint_status: str | None = "ok",
) -> list[str]:
    problems: list[str] = []
    if endpoint_status != "ok":
        problems.append("p23_perf status is not ok")
    telemetry = perf.get("app_telemetry", {})
    app = telemetry.get("current")
    if telemetry.get("snapshot_readable") is not True or not isinstance(app, dict):
        return problems + ["P2 telemetry is unreadable"]
    fpga = app.get("fpga", {})
    state = app.get("state", {})
    v29 = deep_get(app, "gauges", "fpga_fifo_v29", default={})
    xdma = perf.get("xdma", {})
    workload = perf.get("workload", {})

    checks = (
        (app.get("app") == "p2", "active application is not p2"),
        (app.get("version") == 51, "P2 application version is not 51"),
        (state.get("tx_mode") is False, "tx_mode became true"),
        (state.get("pure_signal_enabled") is False, "PureSignal became active"),
        (state.get("exit_requested") is False, "P2 exit was requested"),
        (state.get("thread_error") is False, "P2 reported a thread error"),
        (fpga.get("firmware_version") == 29, "FPGA firmware is not V29"),
        (fpga.get("date_code_hex") == "09122026", "FPGA BIT date changed"),
        (fpga.get("fallback_config") is False, "FPGA fallback became active"),
        (fpga.get("all_clocks_present") is True, "an FPGA clock disappeared"),
        (
            routing_is_required(app, expected_ddcs),
            f"RX routing/workload is not {ddc_expectations_text(expected_ddcs)} with Wideband off",
        ),
        (workload.get("selected_app") == "p2", "workload selected_app changed"),
        (workload.get("panel_mode") == "off", "panel mode is not off"),
        (xdma.get("present") is True, "XDMA disappeared"),
        (deep_get(xdma, "pcie", "current_link_speed") == "5.0 GT/s PCIe", "PCIe speed changed"),
        (deep_get(xdma, "pcie", "current_link_width") == "1", "PCIe width changed"),
        (v29.get("available") is True and v29.get("status") == "available", "fpga_fifo_v29 is not available"),
        (v29.get("build_id") == EXPECTED_BUILD_ID, "fpga_fifo_v29 build ID changed"),
        (v29.get("snapshot_valid") is True, "V29 occupancy snapshot became invalid"),
        (
            is_number(app.get("counters", {}).get("adc_overflow_events")),
            "required ADC report counter is missing",
        ),
        (
            is_number(deep_get(app, "gauges", "adc", "overflow_bits")),
            "required ADC overflow bits gauge is missing",
        ),
        (
            is_number(deep_get(app, "gauges", "adc", "peak1"))
            and is_number(deep_get(app, "gauges", "adc", "peak2")),
            "required ADC peak gauges are missing",
        ),
    )
    problems.extend(message for ok, message in checks if not ok)

    if profile == "controlled" and number(deep_get(app, "gauges", "adc", "overflow_bits")) != 0:
        problems.append("ADC overflow bits became nonzero")

    if baseline is not None:
        base_app = current(baseline)
        base_perf = baseline
        if app.get("pid") != base_app.get("pid") or deep_get(perf, "service", "main_pid") != deep_get(base_perf, "service", "main_pid"):
            problems.append("P2 PID changed")
        if app.get("routing") != base_app.get("routing"):
            problems.append("routing changed from the first sample")
        for key in ("current_target", "panel_mode", "selected_app", "startup_mode", "workload_key"):
            if workload.get(key) != base_perf.get("workload", {}).get(key):
                problems.append(f"workload {key} changed from the first sample")
        timeout_delta = number(v29.get("snapshot_timeout_count")) - number(deep_get(base_app, "gauges", "fpga_fifo_v29", "snapshot_timeout_count"))
        if timeout_delta != 0:
            problems.append(f"V29 snapshot timeout count changed by {timeout_delta}")
        base_counters = base_app.get("counters", {})
        counters = app.get("counters", {})
        for name in gated_loss_counters(profile):
            delta = counter_delta(base_counters, counters, name)
            if delta is None:
                problems.append(f"required counter {name} is missing")
            elif delta != 0:
                problems.append(f"{name} changed by {delta}")
        if profile == "antenna":
            adc_delta = counter_delta(
                base_counters, counters, "adc_overflow_events"
            )
            if adc_delta is not None and adc_delta < 0:
                problems.append(
                    f"adc_overflow_events moved backward by {adc_delta}"
                )
        for interface, directions in base_perf.get("network", {}).items():
            current_interface = perf.get("network", {}).get(interface, {})
            for direction, fields in NETWORK_ERROR_FIELDS.items():
                for field in fields:
                    delta = number(deep_get(current_interface, direction, field)) - number(deep_get(directions, direction, field))
                    if delta != 0:
                        problems.append(f"network {interface}.{direction}.{field} changed by {delta}")

    if previous is not None:
        old_generation = number(deep_get(current(previous), "gauges", "fpga_fifo_v29", "snapshot_generation"))
        new_generation = number(v29.get("snapshot_generation"))
        advance = (int(new_generation) - int(old_generation)) & 0xFFFF
        if advance == 0 or advance >= 0x8000:
            problems.append(f"V29 occupancy snapshot generation did not advance ({old_generation} -> {new_generation})")
    return problems


def collect_metrics(records: list[dict]) -> dict:
    values: dict[str, list[float]] = {}

    def add(name: str, value) -> None:
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            values.setdefault(name, []).append(float(value))

    for record in records:
        perf = record["perf"]["perf"]
        app = current(perf)
        gauges = app.get("gauges", {})
        system = perf.get("system", {})
        add("soc_temp_c", deep_get(system, "hardware", "soc_temp_c"))
        add("fpga_temp_c", deep_get(app, "fpga", "die_temp_c"))
        add("rss_mib", number(deep_get(perf, "process", "stat", "rss_bytes")) / 1048576)
        add("memory_available_mib", number(deep_get(system, "memory", "available_bytes")) / 1048576)
        add("load_one", deep_get(system, "loadavg", "one"))
        add("adc1_peak", deep_get(gauges, "adc", "peak1"))
        add("adc2_peak", deep_get(gauges, "adc", "peak2"))
        for channel in V29_CHANNELS:
            add(f"v29_occupancy_{channel}_words", deep_get(gauges, "fpga_fifo_v29", "occupancy_words", channel))

    for before_record, after_record in zip(records, records[1:]):
        elapsed = after_record["elapsed_seconds"] - before_record["elapsed_seconds"]
        if elapsed <= 0:
            continue
        before = before_record["perf"]["perf"]
        after = after_record["perf"]["perf"]
        before_app = current(before)
        after_app = current(after)
        ticks = number(deep_get(after, "system", "clock_ticks_per_sec"), 100)
        cpu_ticks = (
            number(deep_get(after, "process", "stat", "utime_ticks"))
            + number(deep_get(after, "process", "stat", "stime_ticks"))
            - number(deep_get(before, "process", "stat", "utime_ticks"))
            - number(deep_get(before, "process", "stat", "stime_ticks"))
        )
        add("cpu_pct_one_core", cpu_ticks / ticks / elapsed * 100.0)
        delay = number(deep_get(after, "process", "schedstat", "run_delay_ns")) - number(deep_get(before, "process", "schedstat", "run_delay_ns"))
        add("scheduler_delay_ms_per_sec", delay / 1_000_000.0 / elapsed)
        contexts = (
            number(deep_get(after, "process", "status", "voluntary_ctxt_switches"))
            + number(deep_get(after, "process", "status", "nonvoluntary_ctxt_switches"))
            - number(deep_get(before, "process", "status", "voluntary_ctxt_switches"))
            - number(deep_get(before, "process", "status", "nonvoluntary_ctxt_switches"))
        )
        add("context_switches_per_sec", contexts / elapsed)
        irq = number(deep_get(after, "xdma", "interrupts_total")) - number(deep_get(before, "xdma", "interrupts_total"))
        add("xdma_interrupts_per_sec", irq / elapsed)
        for metric, counter_name in RATE_COUNTERS.items():
            delta = counter_delta(before_app.get("counters", {}), after_app.get("counters", {}), counter_name)
            if delta is not None:
                add(metric, delta / elapsed)

    return {
        name: {"min": min(series), "average": statistics.fmean(series), "max": max(series), "count": len(series)}
        for name, series in sorted(values.items()) if series
    }


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    out = args.output.expanduser().resolve()
    source_path = Path(__file__).resolve()
    out.mkdir(parents=True, exist_ok=True)
    allowed_existing = {source_path} if source_path.parent == out else set()
    unexpected = [path for path in out.iterdir() if path.resolve() not in allowed_existing]
    if unexpected:
        names = ", ".join(sorted(path.name for path in unexpected))
        raise SystemExit(f"output directory must be empty; found: {names}")
    artifact_script = out / "collect.py"
    if source_path != artifact_script.resolve():
        shutil.copy2(source_path, artifact_script)

    planned_samples = math.ceil(args.duration / args.interval) + 1
    records: list[dict] = []
    failures: list[dict] = []
    transition_increases: list[dict] = []
    activity_increases: list[dict] = []
    adc_report_clusters: list[dict] = []
    occupancy_at_or_beyond_depth: list[dict] = []
    start_mono = time.monotonic()
    raw_path = out / "samples.ndjson"

    atomic_json(out / "collection_status.json", {
        "status": "running", "started_utc": utc_now(), "planned_duration_seconds": args.duration,
        "sample_interval_seconds": args.interval, "endpoint": args.endpoint,
        "validation_profile": args.profile,
    })

    with raw_path.open("x", encoding="utf-8") as raw:
        previous: dict | None = None
        baseline: dict | None = None
        for index in range(planned_samples):
            target = start_mono + index * args.interval
            delay = target - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            timestamp = utc_now()
            try:
                payload = fetch(args.endpoint)
                perf = payload["perf"]
            except Exception as exc:
                failures.append({"utc_timestamp": timestamp, "reason": f"telemetry unreadable: {exc}"})
                break

            elapsed = time.monotonic() - start_mono
            record = {
                "sample_index": index, "utc_timestamp": timestamp,
                "elapsed_seconds": elapsed, "rate_eligible": index > 0,
                "perf": payload,
            }
            raw.write(json.dumps(record, separators=(",", ":")) + "\n")
            raw.flush()
            records.append(record)
            if baseline is None:
                baseline = perf
                atomic_json(out / "start.json", record)

            app = current(perf)
            v29 = deep_get(app, "gauges", "fpga_fifo_v29", default={})
            for channel, depth in FIFO_DEPTHS.items():
                occupancy = deep_get(v29, "occupancy_words", channel)
                if isinstance(occupancy, (int, float)) and occupancy >= depth:
                    occupancy_at_or_beyond_depth.append({
                        "utc_timestamp": timestamp, "sample_index": index,
                        "fifo": channel, "occupancy_words": occupancy, "configured_depth_words": depth,
                    })

            adc_bits = number(deep_get(app, "gauges", "adc", "overflow_bits"))
            if previous is None and adc_bits != 0:
                adc_reports = number(
                    app.get("counters", {}).get("adc_overflow_events")
                )
                adc_report_clusters.append({
                    "utc_timestamp": timestamp,
                    "sample_index": index,
                    "report_count_before": None,
                    "report_count_after": adc_reports,
                    "report_count_delta": None,
                    "sampled_overflow_bits": adc_bits,
                    "sampled_adc1_peak": deep_get(app, "gauges", "adc", "peak1"),
                    "sampled_adc2_peak": deep_get(app, "gauges", "adc", "peak2"),
                    "baseline_sample": True,
                })

            if previous is not None:
                prior_app = current(previous)
                changes = {}
                for channel in V29_CHANNELS:
                    old = number(deep_get(prior_app, "gauges", "fpga_fifo_v29", "event_transitions", channel))
                    new = number(deep_get(v29, "event_transitions", channel))
                    if new != old:
                        changes[channel] = {"before": old, "after": new, "delta": new - old}
                if changes:
                    transition_increases.append({"utc_timestamp": timestamp, "sample_index": index, "changes": changes})
                host_changes = {}
                for name in HOST_ACTIVITY_COUNTERS:
                    delta = counter_delta(prior_app.get("counters", {}), app.get("counters", {}), name)
                    if delta:
                        host_changes[name] = delta
                if host_changes:
                    activity_increases.append({"utc_timestamp": timestamp, "sample_index": index, "changes": host_changes})

                old_adc_reports = number(prior_app.get("counters", {}).get("adc_overflow_events"))
                new_adc_reports = number(app.get("counters", {}).get("adc_overflow_events"))
                adc_report_delta = new_adc_reports - old_adc_reports
                if adc_report_delta != 0 or adc_bits != 0:
                    adc_report_clusters.append({
                        "utc_timestamp": timestamp,
                        "sample_index": index,
                        "report_count_before": old_adc_reports,
                        "report_count_after": new_adc_reports,
                        "report_count_delta": adc_report_delta,
                        "sampled_overflow_bits": adc_bits,
                        "sampled_adc1_peak": deep_get(app, "gauges", "adc", "peak1"),
                        "sampled_adc2_peak": deep_get(app, "gauges", "adc", "peak2"),
                    })

            problems = gate_violations(
                perf, baseline, previous, args.profile, args.expected_ddcs,
                payload.get("status"),
            )
            if problems:
                failures.extend({"utc_timestamp": timestamp, "sample_index": index, "reason": reason} for reason in problems)
                break
            previous = perf
            atomic_json(out / "collection_status.json", {
                "status": "running", "started_utc": records[0]["utc_timestamp"],
                "last_sample_utc": timestamp, "sample_count": len(records),
                "elapsed_seconds": elapsed, "validation_profile": args.profile,
            })

    if not records:
        atomic_json(out / "collection_status.json", {
            "status": "failed", "failures": failures, "validation_profile": args.profile,
        })
        return 1

    atomic_json(out / "end.json", records[-1])
    first_perf = records[0]["perf"]["perf"]
    last_perf = records[-1]["perf"]["perf"]
    first_app = current(first_perf)
    last_app = current(last_perf)
    first_counters = first_app.get("counters", {})
    last_counters = last_app.get("counters", {})
    all_counter_deltas = {
        name: {"start": first_counters.get(name), "end": last_counters.get(name), "delta": counter_delta(first_counters, last_counters, name)}
        for name in sorted(set(first_counters) | set(last_counters))
    }
    first_v29 = deep_get(first_app, "gauges", "fpga_fifo_v29", default={})
    last_v29 = deep_get(last_app, "gauges", "fpga_fifo_v29", default={})
    transitions = {
        channel: {
            "start": deep_get(first_v29, "event_transitions", channel),
            "end": deep_get(last_v29, "event_transitions", channel),
            "delta": number(deep_get(last_v29, "event_transitions", channel)) - number(deep_get(first_v29, "event_transitions", channel)),
        } for channel in V29_CHANNELS
    }
    extrema = {
        channel: {
            "configured_depth_words": FIFO_DEPTHS[channel],
            "minimum_words": {"start": deep_get(first_v29, "minimum_words", channel), "end": deep_get(last_v29, "minimum_words", channel)},
            "maximum_words": {"start": deep_get(first_v29, "maximum_words", channel), "end": deep_get(last_v29, "maximum_words", channel)},
        } for channel in V29_CHANNELS
    }
    timeout_delta = number(last_v29.get("snapshot_timeout_count")) - number(first_v29.get("snapshot_timeout_count"))
    completed = not failures and len(records) == planned_samples and records[-1]["elapsed_seconds"] >= args.duration
    verdict = "PASS" if completed else "FAIL"
    summary = {
        "test": "P2 V51 FPGA V29 read-only RX soak",
        "scope": "RX qualification only; no raw XDMA access and no accumulator clear",
        "overall_verdict": verdict,
        "artifact_directory": str(out),
        "validation_profile": args.profile,
        "adc_gate_policy": "hard gate" if args.profile == "controlled" else "recorded, non-gating environmental signal",
        "expected_ddcs": [
            {"id": ddc_id, "sample_rate_khz": rate, "interleaved": interleaved}
            for ddc_id, rate, interleaved in args.expected_ddcs
        ],
        "start_utc": records[0]["utc_timestamp"],
        "end_utc": records[-1]["utc_timestamp"],
        "duration_seconds": records[-1]["elapsed_seconds"],
        "sample_interval_seconds": args.interval,
        "sample_count": len(records),
        "rate_interval_count": max(0, len(records) - 1),
        "first_sample_treatment": "initialization only, excluded from rate calculations",
        "identity": {
            "application": first_app.get("app"), "application_version": first_app.get("version"),
            "pid": first_app.get("pid"), "fpga": first_app.get("fpga"),
            "workload": first_perf.get("workload"), "routing": first_app.get("routing"),
            "xdma": {"present": deep_get(first_perf, "xdma", "present"), "pcie": deep_get(first_perf, "xdma", "pcie")},
        },
        "v29_snapshot": {
            "status_start": first_v29.get("status"), "status_end": last_v29.get("status"),
            "build_id_start": first_v29.get("build_id"), "build_id_end": last_v29.get("build_id"),
            "valid_start": first_v29.get("snapshot_valid"), "valid_end": last_v29.get("snapshot_valid"),
            "generation_start": first_v29.get("snapshot_generation"), "generation_end": last_v29.get("snapshot_generation"),
            "timeout_count_start": first_v29.get("snapshot_timeout_count"), "timeout_count_end": last_v29.get("snapshot_timeout_count"),
            "timeout_count_delta": timeout_delta,
            "generation_scope": "captured coherent occupancy only",
        },
        "v29_event_transitions": {
            "classification": "live boot-lifetime aggregate empty/full/almost-full transitions; not loss counters and not part of the coherent occupancy snapshot",
            "start_end_deltas": transitions,
            "increase_timestamps": transition_increases,
        },
        "boot_lifetime_extrema": {
            "classification": "live boot-lifetime accumulators; not cleared and not part of the coherent occupancy snapshot",
            "channels": extrema,
        },
        "captured_occupancy_at_or_beyond_depth": occupancy_at_or_beyond_depth,
        "counter_deltas": all_counter_deltas,
        "adc_observations": {
            "classification": "sampled report clusters; not independent physical clip episodes",
            "gating": args.profile == "controlled",
            "report_count": all_counter_deltas.get("adc_overflow_events"),
            "sampled_report_clusters": adc_report_clusters,
        },
        "host_activity_increase_timestamps": activity_increases,
        "network_start": first_perf.get("network"), "network_end": last_perf.get("network"),
        "metrics_min_average_max": collect_metrics(records),
        "failures": failures,
        "notes": [
            "The first sample establishes cumulative-counter baselines.",
            "V29 transition increases do not affect the verdict by themselves.",
            "Boot-lifetime extrema were observed without issuing FPGA clear control.",
            "No /dev/xdma0_user or raw register access was performed.",
        ],
    }
    atomic_json(out / "summary.json", summary)

    lines = [
        "# P2 V51 / FPGA V29 Read-Only RX Soak", "",
        f"Overall verdict: **{verdict}**", "",
        f"- Validation profile: {args.profile}",
        f"- ADC policy: {summary['adc_gate_policy']}",
        f"- Window: {summary['start_utc']} through {summary['end_utc']}",
        f"- Duration: {summary['duration_seconds']:.3f} seconds; {len(records)} samples, {max(0, len(records)-1)} rate intervals",
        f"- P2 V{first_app.get('version')} PID {first_app.get('pid')}; FPGA V{deep_get(first_app, 'fpga', 'firmware_version')} BIT {deep_get(first_app, 'fpga', 'date_code_hex')}",
        f"- V29 marker: {first_v29.get('build_id')}; occupancy generation {first_v29.get('snapshot_generation')} -> {last_v29.get('snapshot_generation')}",
        f"- Snapshot timeouts: {first_v29.get('snapshot_timeout_count')} -> {last_v29.get('snapshot_timeout_count')} (delta {timeout_delta})",
        f"- Frozen workload: RX-only, {ddc_expectations_text(args.expected_ddcs)}, Wideband/PureSignal off, headless/panel off",
        "", "## V29 aggregate transition counters", "",
        "These are live boot-lifetime empty/full/almost-full transition accumulators, not loss counters and not coherent snapshot fields.", "",
    ]
    for channel, item in transitions.items():
        lines.append(f"- {channel}: {item['start']} -> {item['end']} (delta {item['delta']})")
    lines += ["", "## Boot-lifetime extrema (not cleared)", ""]
    for channel, item in extrema.items():
        lines.append(
            f"- {channel}: min {item['minimum_words']['start']} -> {item['minimum_words']['end']}; "
            f"max {item['maximum_words']['start']} -> {item['maximum_words']['end']}; configured depth {item['configured_depth_words']} words"
        )
    lines += ["", "## Verdict gates", ""]
    for name in gated_loss_counters(args.profile):
        item = all_counter_deltas[name]
        lines.append(f"- {name}: {item['start']} -> {item['end']} (delta {item['delta']})")
    if args.profile == "antenna":
        adc_item = all_counter_deltas.get("adc_overflow_events", {})
        lines += [
            "", "## ADC observations (non-gating)", "",
            "These are sampled status-report clusters, not independent physical clip episodes.", "",
            f"- adc_overflow_events: {adc_item.get('start')} -> {adc_item.get('end')} (delta {adc_item.get('delta')})",
            f"- sampled report clusters: {len(adc_report_clusters)}",
        ]
        for item in adc_report_clusters:
            report_delta = item["report_count_delta"]
            report_text = "baseline" if report_delta is None else f"{report_delta:+}"
            lines.append(
                f"- {item['utc_timestamp']}: reports {report_text}; "
                f"bits {item['sampled_overflow_bits']}; peaks "
                f"ADC1={item['sampled_adc1_peak']}, ADC2={item['sampled_adc2_peak']}"
            )
    lines += ["", "## Captured occupancy at or beyond configured depth", ""]
    if occupancy_at_or_beyond_depth:
        for item in occupancy_at_or_beyond_depth:
            lines.append(f"- {item['utc_timestamp']}: {item['fifo']} {item['occupancy_words']} / {item['configured_depth_words']} words")
    else:
        lines.append("- None observed.")
    lines += ["", "## Failures", ""]
    if failures:
        lines.extend(f"- {item['utc_timestamp']}: {item['reason']}" for item in failures)
    else:
        lines.append("- None.")
    lines += ["", "No FPGA accumulators were cleared. No raw XDMA access was performed.", "", f"Artifact directory: `{out}`", ""]
    (out / "summary.md").write_text("\n".join(lines), encoding="utf-8")
    atomic_json(out / "collection_status.json", {
        "status": "complete" if completed else "failed", "overall_verdict": verdict,
        "started_utc": records[0]["utc_timestamp"], "ended_utc": records[-1]["utc_timestamp"],
        "sample_count": len(records), "duration_seconds": records[-1]["elapsed_seconds"],
        "failures": failures, "validation_profile": args.profile,
    })
    artifacts = [out / name for name in ARTIFACT_NAMES]
    with (out / "SHA256SUMS").open("w", encoding="utf-8") as hashes:
        for path in artifacts:
            hashes.write(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    print(json.dumps({
        "verdict": verdict, "validation_profile": args.profile,
        "artifact_directory": str(out), "sample_count": len(records), "failures": failures,
    }))
    return 0 if completed else 1


if __name__ == "__main__":
    sys.exit(main())
