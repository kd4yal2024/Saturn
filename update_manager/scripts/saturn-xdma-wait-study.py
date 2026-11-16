#!/usr/bin/env python3
"""Bounded read-only study of the existing acquisition owner's published data.

No device access, service control, PTT, register reads, or packet transmission.
"""
import argparse
import datetime
import json
import math
from pathlib import Path
import time


COUNTERS = ("dma_reads", "dma_bytes", "frames", "samples", "header_resyncs",
            "header_errors", "fifo_almost_full", "fifo_overflows",
            "fifo_over_threshold", "fifo_empty_observations", "host_buffer_drops",
            "host_buffer_drop_bytes", "host_discontinuities", "host_pool_starvations")


def natural(value):
    if type(value) is not int or value < 0:
        raise ValueError("expected a nonnegative integer counter")
    return value


def histogram_delta(first, last):
    """Return window quantile bucket bounds; never subtract percentiles/maxima."""
    for h in (first, last):
        if h.get("bucket_model") != "ceil-us-pow2-last-open" or len(h["buckets"]) != 32:
            raise ValueError("unknown histogram geometry")
        if sum(natural(x) for x in h["buckets"]) != natural(h["count"]):
            raise ValueError("histogram count mismatch")
        natural(h["max_us"])
    buckets = [b - a for a, b in zip(first["buckets"], last["buckets"])]
    if any(b < 0 for b in buckets):
        raise ValueError("histogram reset or regression")
    count = sum(buckets)

    def upper(percent):
        if not count:
            return None
        rank, cumulative = (count * percent + 99) // 100, 0
        for index, n in enumerate(buckets):
            cumulative += n
            if cumulative >= rank:
                # Bucket 31 includes all values greater than 2**30 us.
                return None if index == 31 else 1 << index
        raise ValueError("invalid histogram")

    return {"count": count, "buckets": buckets, "p95UpperUs": upper(95),
            "p99UpperUs": upper(99), "windowMaxUs": None,
            "sessionMaxUsAtEnd": last["max_us"],
            "scope": "window bucket deltas; open last bucket has no finite bound"}


def validate(snapshot, now_ms):
    if (snapshot.get("backend"), snapshot.get("tx_owner"), snapshot.get("phase")) != (
            "xdma", "shared", "shared-ready-off"):
        raise ValueError("study needs the shared Direct XDMA owner in ReadyOff")
    m = snapshot["metrics"]
    if m.get("tx_keyed") is not False or m.get("tx_stream") is not False:
        raise ValueError("TX state is active or unknown; study stopped without controlling the radio")
    if not 0 <= now_ms - natural(snapshot["updated_at_ms"]) <= 3000:
        raise ValueError("owner telemetry is stale or from a future clock")
    a = snapshot.get("shared_acquisition")
    if not a or a.get("schema") != "xdma-acquisition-study-v1":
        raise ValueError("installed owner does not publish acquisition study data")
    for key in COUNTERS:
        natural(a["counters"][key])
    return (natural(m["pid"]), m["build_git_sha"], m["source_pin"])


def summarize(samples):
    if len(samples) < 2:
        raise ValueError("need at least two distinct published observations")
    first, last = samples[0]["owner"], samples[-1]["owner"]
    elapsed = (last["updated_at_ms"] - first["updated_at_ms"]) / 1000
    if elapsed <= 0:
        raise ValueError("publication clock did not advance")
    a, b = first["shared_acquisition"], last["shared_acquisition"]
    counters = {k: b["counters"][k] - a["counters"][k] for k in COUNTERS}
    if any(value < 0 for value in counters.values()):
        raise ValueError("acquisition counters reset or regressed")
    latencies = {k: histogram_delta(a["latencies"][k], b["latencies"][k])
                 for k in ("dma_read", "dma_completion_gap", "host_queue_age", "parser")}
    keys = ("frequency_hz", "sample_rate_hz", "adc", "ddc", "connections", "iq", "audio")
    states = [{k: s["owner"]["metrics"].get(k) for k in keys} for s in samples]
    stable = all(state == states[0] for state in states)
    fifo_polls = None
    if "fifo_polls" in a["counters"] and "fifo_polls" in b["counters"]:
        delta = natural(b["counters"]["fifo_polls"]) - natural(a["counters"]["fifo_polls"])
        if delta < 0:
            raise ValueError("FIFO poll counter reset or regression")
        fifo_polls = {"delta": delta, "perSecond": delta / elapsed,
                      "perCompletedRead": delta / counters["dma_reads"]
                      if counters["dma_reads"] else None}
    rates = {"dmaReadsPerSecond": counters["dma_reads"] / elapsed,
             "dmaBytesPerSecond": counters["dma_bytes"] / elapsed,
             "averageReadBytes": counters["dma_bytes"] / counters["dma_reads"]
             if counters["dma_reads"] else None}
    # SATP counters are session cumulative too. Compare only fresh endpoints
    # captured within this same owner-PID window, not across service restarts.
    satp = None
    x, y = samples[0].get("satp"), samples[-1].get("satp")
    if x and y:
        delta = {k: natural(y[k]) - natural(x[k])
                 for k in ("packets_rx", "invalid", "frames_rx")}
        dt = (y["updated_unix_ms"] - x["updated_unix_ms"]) / 1000
        if dt > 0 and all(v >= 0 for v in delta.values()):
            satp = {"seconds": dt, "counterDeltas": delta,
                    "perSecond": {k: v / dt for k, v in delta.items()}}
    return {"publishedWindowSeconds": elapsed, "observations": len(samples),
            "stableRadioAndClients": stable, "radioAtStart": states[0],
            "radioAtEnd": states[-1], "counterDeltas": counters, "rates": rates,
            "latencies": latencies, "satpWindow": satp, "fifoPolls": fifo_polls,
            "waitPolicyAtEnd": {k: b.get(k) for k in ("fifo_poll_us", "reader_wait_mode",
                "rx_fifo_poll_cap_us", "feedback_fifo_poll_us", "paused_wait_cap_us")},
            "fifoSessionHighWaterWords": b["counters"]["fifo_depth_hwm"],
            "fifoSnapshotAtEnd": b["fifo_snapshot"],
            "limits": "FIFO high-water and latency maxima are session values. FPGA min/max/events are boot lifetime. No sub-second FIFO occupancy coverage, cancellation deadline, loss-free proof or longer-wait approval follows from this report."}


def collect(perf_path, satp_path, seconds, interval):
    report = {"format": "saturn-xdma-wait-study-v1",
              "startedAtUtc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "readOnly": True, "rfKeyedByTool": False, "samples": [], "passed": False}
    deadline, identity, last_stamp = time.monotonic() + seconds, None, None
    try:
        while True:
            now_ms = int(time.time() * 1000)
            owner = json.loads(perf_path.read_text())
            current = validate(owner, now_ms)
            if identity is not None and identity != current:
                raise ValueError("owner PID/build/source changed during study")
            identity = current
            stamp = owner["updated_at_ms"]
            if last_stamp is not None and stamp < last_stamp:
                raise ValueError("owner publication clock regressed")
            if stamp != last_stamp:
                if report["samples"]:
                    previous = report["samples"][-1]["owner"]["shared_acquisition"]
                    current_stats = owner["shared_acquisition"]
                    keys = COUNTERS + (("fifo_polls",) if "fifo_polls" in previous["counters"] else ())
                    if any(natural(current_stats["counters"][k]) < previous["counters"][k] for k in keys):
                        raise ValueError("acquisition counter reset or regression between publications")
                    for key in ("dma_read", "dma_completion_gap", "host_queue_age", "parser"):
                        histogram_delta(previous["latencies"][key], current_stats["latencies"][key])
                satp = None
                try:
                    s = json.loads(satp_path.read_text())
                    if 0 <= now_ms - s["updated_unix_ms"] <= 3000:
                        satp = {k: s.get(k) for k in ("running", "enabled", "packets_rx",
                                                     "invalid", "frames_rx", "updated_unix_ms")}
                except (OSError, ValueError, KeyError):
                    pass
                report["samples"].append({"observedAtUnixMs": now_ms, "owner": owner, "satp": satp})
                last_stamp = stamp
            if time.monotonic() >= deadline:
                break
            time.sleep(min(interval, max(0, deadline - time.monotonic())))
        report["summary"] = summarize(report["samples"])
        report["passed"] = True
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    report["finishedAtUtc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--perf-path", type=Path, default=Path("/run/saturn-bridge/perf.json"))
    p.add_argument("--satp-path", type=Path, default=Path("/run/saturn-bridge/satp-status.json"))
    p.add_argument("--seconds", type=float, default=60)
    p.add_argument("--interval", type=float, default=1)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    if not math.isfinite(args.seconds) or not 2 <= args.seconds <= 600:
        p.error("--seconds must be between 2 and 600")
    if not math.isfinite(args.interval) or not .25 <= args.interval <= 5:
        p.error("--interval must be between 0.25 and 5")
    if args.output.resolve() in (args.perf_path.resolve(), args.satp_path.resolve()):
        p.error("output must not overwrite an input")
    report = collect(args.perf_path, args.satp_path, args.seconds, args.interval)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ("passed", "summary", "error") if k in report}, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
