#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "wait_study", ROOT / "update_manager/scripts/saturn-xdma-wait-study.py")
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


def histogram(buckets=None, maximum=0):
    b = [0] * 32
    for index, count in (buckets or {}).items():
        b[index] = count
    return {"bucket_model": "ceil-us-pow2-last-open", "buckets": b,
            "count": sum(b), "max_us": maximum}


def sample(stamp=10000, n=0):
    return {"observedAtUnixMs": stamp, "satp": None, "owner": {
        "updated_at_ms": stamp, "backend": "xdma", "tx_owner": "shared",
        "phase": "shared-ready-off", "metrics": {
            "pid": 7, "build_git_sha": "a" * 40, "source_pin": "b" * 64,
            "tx_keyed": False, "tx_stream": False, "frequency_hz": 7200000,
            "connections": 0}, "shared_acquisition": {
                "schema": "xdma-acquisition-study-v1",
                "counters": dict.fromkeys(tool.COUNTERS + ("fifo_depth_hwm",), n),
                "latencies": {k: histogram({11: n}, 1500) for k in (
                    "dma_read", "dma_completion_gap", "host_queue_age", "parser")},
                "fifo_snapshot": {"valid": False}}}}


class WaitStudy(unittest.TestCase):
    def test_window_percentiles_use_bucket_deltas_not_session_percentiles_or_maxima(self):
        first = histogram({20: 900}, 1000000)
        last = histogram({20: 900, 5: 99, 6: 1}, 1000000)
        d = tool.histogram_delta(first, last)
        self.assertEqual(d["count"], 100)
        self.assertEqual(d["p99UpperUs"], 32)
        self.assertEqual(d["sessionMaxUsAtEnd"], 1000000)
        self.assertIsNone(d["windowMaxUs"])

    def test_empty_and_open_last_buckets_do_not_claim_zero_or_finite_tail(self):
        self.assertIsNone(tool.histogram_delta(histogram(), histogram())["p99UpperUs"])
        d = tool.histogram_delta(histogram(), histogram({31: 1}, 1 << 33))
        self.assertIsNone(d["p99UpperUs"])
        self.assertEqual(d["count"], 1)

    def test_regressions_and_invalid_counts_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "regression"):
            tool.histogram_delta(histogram({2: 1}), histogram())
        for bad in (-1, True, 1.0):
            with self.assertRaises(ValueError):
                tool.natural(bad)
        h = histogram({2: 1}); h["count"] = 0
        with self.assertRaisesRegex(ValueError, "mismatch"):
            tool.histogram_delta(h, histogram())

    def test_validate_rejects_stale_missing_wrong_owner_and_active_tx(self):
        s = sample()["owner"]
        self.assertEqual(tool.validate(s, 10001)[0], 7)
        with self.assertRaisesRegex(ValueError, "stale"):
            tool.validate(s, 14000)
        for key, value in [("backend", "p2"), ("tx_owner", "legacy"),
                           ("phase", "shared-keyed"), ("shared_acquisition", None)]:
            bad = copy.deepcopy(s); bad[key] = value
            with self.assertRaises(ValueError):
                tool.validate(bad, 10001)
        bad = copy.deepcopy(s); bad["metrics"]["tx_keyed"] = True
        with self.assertRaisesRegex(ValueError, "TX state"):
            tool.validate(bad, 10001)

    def test_summary_preserves_tuning_and_client_changes_and_counter_loss(self):
        a, b = sample(10000), sample(12000, 4)
        b["owner"]["metrics"]["frequency_hz"] = 7300000
        d = tool.summarize([a, b])
        self.assertFalse(d["stableRadioAndClients"])
        self.assertEqual(d["counterDeltas"]["host_buffer_drops"], 4)
        self.assertEqual(d["rates"]["dmaReadsPerSecond"], 2)
        b["owner"]["shared_acquisition"]["counters"]["dma_reads"] = -1
        with self.assertRaisesRegex(ValueError, "regressed"):
            tool.summarize([a, b])

    def test_satp_rates_use_same_window_endpoints_and_reject_resets(self):
        a, b = sample(10000), sample(12000, 4)
        a["satp"] = dict(updated_unix_ms=10000, packets_rx=100, invalid=200, frames_rx=300)
        b["satp"] = dict(updated_unix_ms=12000, packets_rx=100, invalid=240, frames_rx=300)
        self.assertEqual(tool.summarize([a,b])["satpWindow"]["perSecond"]["invalid"], 20)
        b["satp"]["invalid"] = 1
        self.assertIsNone(tool.summarize([a,b])["satpWindow"])

    def test_fifo_poll_counts_are_optional_for_older_release_baselines(self):
        a, b = sample(10000), sample(12000, 4)
        self.assertIsNone(tool.summarize([a, b])["fifoPolls"])
        a["owner"]["shared_acquisition"]["counters"]["fifo_polls"] = 10
        b["owner"]["shared_acquisition"]["counters"]["fifo_polls"] = 22
        self.assertEqual(tool.summarize([a, b])["fifoPolls"]["perCompletedRead"], 3)
        b["owner"]["shared_acquisition"]["counters"]["fifo_polls"] = 1
        with self.assertRaisesRegex(ValueError, "regression"):
            tool.summarize([a, b])

    def test_transient_counter_reset_is_rejected_before_a_later_value_can_hide_it(self):
        class Clock:
            now = 10.0
            def sleep(self, seconds): self.now += seconds
            def monotonic(self): return self.now
        clock = Clock()
        import json
        values = [json.dumps(sample(10000, 10)["owner"]), "{}",
                  json.dumps(sample(11000, 1)["owner"]),
                  json.dumps(sample(12000, 20)["owner"])]
        with patch.object(tool.time, "monotonic", clock.monotonic), \
             patch.object(tool.time, "time", clock.monotonic), \
             patch.object(tool.time, "sleep", clock.sleep), \
             patch.object(Path, "read_text", side_effect=values):
            report = tool.collect(Path("/unused"), Path("/satp"), 2, 1)
        self.assertFalse(report["passed"])
        self.assertIn("between publications", report["error"])
        self.assertEqual(len(report["samples"]), 1)

    def test_restart_preserves_partial_evidence_and_duplicate_publications_do_not_count(self):
        class Clock:
            now = 10.0
            def sleep(self, seconds): self.now += seconds
            def monotonic(self): return self.now
        clock = Clock()
        a, b = sample()["owner"], sample(11000)["owner"]
        b["metrics"]["pid"] = 8
        import json
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)/"perf.json"
            with patch.object(tool.time, "monotonic", clock.monotonic), \
                 patch.object(tool.time, "time", clock.monotonic), \
                 patch.object(tool.time, "sleep", clock.sleep), \
                 patch.object(Path, "read_text", side_effect=[json.dumps(a), "{}", json.dumps(a), json.dumps(b)]):
                r = tool.collect(p, Path(directory)/"satp.json", 2, 1)
        self.assertFalse(r["passed"])
        self.assertIn("PID/build/source changed", r["error"])
        self.assertEqual(len(r["samples"]), 1)


if __name__ == "__main__":
    unittest.main()
