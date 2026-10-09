#!/usr/bin/env python3
"""Tests for check_order.py.

Every alteration is embedded in a COMPLETE valid ten-window set (the real rehearsal of 2026-10-09,
fixtures/), so a rejection can only come from the specific rule under test, not from missing
windows. The positive controls prove the unaltered evidence, and evidence with real audio problems,
are accepted: validity is about the measurement, performance is reported separately.

Run:  python3 tools/browser-comparison/test_check_order.py
"""
import copy
import glob
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
CHECKER = os.environ.get("CHECK_ORDER_UNDER_TEST", os.path.join(HERE, "check_order.py"))  # a mutated copy when mutation-testing
FIX = os.path.join(HERE, "fixtures", "rehearsal-2026-10-09")


def read_json(path):
    with open(path) as fh:
        return json.load(fh)


def write_json(path, data):
    with open(path, "w") as fh:
        json.dump(data, fh)


def read_text(path):
    with open(path) as fh:
        return fh.read()


def write_text(path, text, mode="w"):
    with open(path, mode) as fh:
        fh.write(text)


def file_sha256(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def window_files(directory):
    return sorted(glob.glob(os.path.join(directory, "w*_*.json")), key=lambda f: int(os.path.basename(f).split("_")[0][1:]))


class Bench:
    """A temporary copy of the fixture set that a test can alter, then run the checker on."""

    def __init__(self, logs=True):
        self.root = tempfile.mkdtemp(prefix="check-order-test-")
        self.windows = os.path.join(self.root, "windows")
        self.logs = os.path.join(self.root, "bridge-logs")
        shutil.copytree(os.path.join(FIX, "windows"), self.windows)
        if logs:
            shutil.copytree(os.path.join(FIX, "bridge-logs"), self.logs)
        else:
            os.makedirs(self.logs)

    def path(self, n):
        return [f for f in window_files(self.windows) if os.path.basename(f).startswith(f"w{n}_")][0]

    def load(self, n):
        return read_json(self.path(n))

    def save(self, n, data):
        write_json(self.path(n), data)

    def alter(self, n, fn):
        data = self.load(n)
        fn(data)
        self.save(n, data)

    def run(self, *extra, mode="order", windows=None):
        report = os.path.join(self.root, "report")
        cmd = [sys.executable, CHECKER, windows or self.windows, "--mode", mode, "--report-dir", report]
        if os.path.isdir(self.logs):
            cmd += ["--bridge-logs", self.logs]
        proc = subprocess.run(cmd + list(extra), capture_output=True, text=True, timeout=60)
        data = None
        if os.path.exists(os.path.join(report, "order_check.json")):
            data = read_json(os.path.join(report, "order_check.json"))
        return proc.returncode, proc.stdout, data

    def cleanup(self):
        shutil.rmtree(self.root, ignore_errors=True)


def failed_ids(report, n):
    for w in report["windows"]:
        if w["n"] == n:
            return {c["check"] for c in w["checks"] if not c["ok"]}
    raise KeyError(n)


def all_window_failures(report):
    return {w["n"]: {c["check"] for c in w["checks"] if not c["ok"]} for w in report["windows"] if any(not c["ok"] for c in w["checks"])}


def set_failures(report):
    return {c["check"] for c in report["set_checks"] if not c["ok"]}


class Base(unittest.TestCase):
    def setUp(self):
        self.bench = Bench()
        self.addCleanup(self.bench.cleanup)

    def assertRejectedOnlyIn(self, n, expected_ids, alter, *extra):
        """The altered window is INVALID for (at least) the expected rules; every other window is untouched and valid."""
        self.bench.alter(n, alter)
        code, out, report = self.bench.run(*extra)
        self.assertEqual(code, 1, out)
        failures = all_window_failures(report)
        self.assertEqual(set(failures), {n}, f"only window {n} may fail, got {failures}")
        self.assertTrue(set(expected_ids) <= failures[n], f"expected {expected_ids} in {failures[n]}")
        return report


class PositiveControls(Base):
    def test_the_unaltered_ten_windows_are_valid_and_nodelay_is_verified(self):
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED")
        self.assertEqual(len(report["windows"]), 10)
        self.assertFalse(all_window_failures(report))
        self.assertFalse(set_failures(report))
        self.assertTrue(all(w["nodelay"]["status"] == "VERIFIED" for w in report["windows"]))

    def test_real_audio_problems_in_a_valid_window_are_results_not_rejections(self):
        def worse(d):
            for i, s in enumerate(d["samples"]):
                p = s["page"]
                step = i // 4  # underruns accumulate through the window
                p["underruns"] += step
                p["overflows"] += (1 if i > 10 else 0) + (1 if i > 20 else 0)
                p["audioGaps"] += 3 if i > 12 else 0
                p["audioResyncs"] += 1 if i > 25 else 0
        self.bench.alter(5, worse)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertFalse(all_window_failures(report))
        perf = [w for w in report["windows"] if w["n"] == 5][0]["performance"]
        self.assertGreater(perf["underruns"], 0)
        self.assertEqual(perf["overflows"], 2)
        self.assertEqual(perf["audio_gaps"], 3)
        self.assertIn("performance events in valid windows", out)

    def test_inputs_are_never_modified(self):
        self.bench.alter(1, lambda d: d["samples"][15].update(bridge={"error": "x"}))
        before = {f: file_sha256(f) for f in glob.glob(os.path.join(self.bench.root, "**", "*"), recursive=True) if os.path.isfile(f)}
        self.bench.run()
        for f, digest in before.items():
            self.assertEqual(file_sha256(f), digest, f)


class Telemetry(Base):
    def test_a_failed_bridge_read_invalidates_the_window(self):
        self.assertRejectedOnlyIn(1, {"telemetry.every_sample_complete"}, lambda d: d["samples"][15].update(bridge={"error": "simulated read failure"}))

    def test_bridge_telemetry_frozen_after_the_second_sample(self):
        def freeze(d):
            ref = d["samples"][1]["bridge"]["updatedAtMs"]
            for s in d["samples"][2:]:
                s["bridge"]["updatedAtMs"] = ref
        self.assertRejectedOnlyIn(1, {"bridge.fresh_every_sample", "bridge.timestamp_advances_with_window"}, freeze)

    def test_bridge_telemetry_that_is_old_at_every_sample(self):
        def old(d):
            for s in d["samples"]:
                s["bridge"]["updatedAtMs"] -= 10_000
        self.assertRejectedOnlyIn(3, {"bridge.age_every_sample"}, old)

    def test_the_age_rule_is_not_applied_across_machines_in_the_live_profile(self):
        def skew(d):
            for s in d["samples"]:
                s["bridge"]["updatedAtMs"] -= 600_000  # a clock ten minutes apart: advancing normally, so not stale
        self.bench.alter(3, skew)
        _, _, report = self.bench.run("--profile", "live")
        self.assertNotIn("bridge.age_every_sample", {c["check"] for w in report["windows"] if w["n"] == 3 for c in w["checks"]})
        self.assertNotIn("bridge.fresh_every_sample", failed_ids(report, 3))

    def test_a_missing_field_in_one_sample(self):
        self.assertRejectedOnlyIn(2, {"telemetry.every_sample_complete"}, lambda d: d["samples"][5]["page"].pop("codec"))

    def test_a_missing_meta_field(self):
        self.assertRejectedOnlyIn(2, {"record.meta_fields"}, lambda d: d["meta"].pop("bridgePid"))

    def test_a_hole_in_the_record(self):
        def hole(d):
            del d["samples"][10:15]
        self.assertRejectedOnlyIn(7, {"cadence.no_holes", "cadence.interval_in_band"}, hole)

    def test_a_window_that_is_too_short(self):
        self.assertRejectedOnlyIn(8, {"window.span", "window.sample_count"}, lambda d: d.update(samples=d["samples"][:20]))

    def test_the_live_profile_rejects_thirty_second_windows(self):
        code, out, report = self.bench.run("--profile", "live")
        self.assertEqual(code, 1, out)
        self.assertTrue(all("window.span" in failed_ids(report, n) for n in range(1, 11)))


class ConnectionAndAudioFormat(Base):
    def test_a_disconnect_in_the_middle(self):
        self.assertRejectedOnlyIn(2, {"page.connected_every_sample"}, lambda d: d["samples"][15]["page"].update(connected=False))

    def test_a_codec_change_in_the_middle(self):
        self.assertRejectedOnlyIn(4, {"audio.codec_every_sample"}, lambda d: d["samples"][15]["page"].update(codec="pcm"))

    def test_a_rate_change(self):
        def go(d):
            for s in d["samples"][15:]:
                s["page"]["rate"] = 44_100
        self.assertRejectedOnlyIn(5, {"audio.rate_constant"}, go)

    def test_a_channel_change(self):
        def go(d):
            for s in d["samples"][15:]:
                s["page"]["channels"] = 1
        self.assertRejectedOnlyIn(5, {"audio.channels_constant"}, go)

    def test_a_worklet_mode_change(self):
        def go(d):
            for s in d["samples"][15:]:
                s["page"]["worklet"] = "sab"
        self.assertRejectedOnlyIn(5, {"audio.worklet_constant"}, go)

    def test_a_different_display_override_than_the_arm(self):
        self.assertRejectedOnlyIn(3, {"page.override_matches_arm"}, lambda d: d["samples"][4]["page"].update(override="iq"))

    def test_audio_that_is_not_running_is_not_an_audio_measurement(self):
        def silent(d):
            ref = d["samples"][0]["page"]
            for s in d["samples"]:
                for k in ("audioPlayed", "opusFrames", "lastAudioSeq"):
                    s["page"][k] = ref[k]
        self.assertRejectedOnlyIn(4, {"audio.running"}, silent)


class Counters(Base):
    def test_a_counter_that_goes_backwards_means_a_reset(self):
        def reset(d):
            for s in d["samples"][15:]:
                s["page"]["audioPlayed"] -= 400
        self.assertRejectedOnlyIn(6, {"counters.audioPlayed_monotonic"}, reset)

    def test_rows_that_restart_from_zero(self):
        def reset(d):
            base = d["samples"][14]["page"]["rows"]
            for s in d["samples"][15:]:
                s["page"]["rows"] -= base
        self.assertRejectedOnlyIn(3, {"counters.rows_monotonic"}, reset)

    def test_the_bridge_rows_written_counter_going_backwards(self):
        def back(d):
            for s in d["samples"][15:]:
                s["bridge"]["rows_written"] -= 50
        self.assertRejectedOnlyIn(1, {"counters.bridge_rows_written_monotonic"}, back)


class Delivery(Base):
    def test_rows_arriving_in_a_raw_iq_window(self):
        def rows(d):
            for k, s in enumerate(d["samples"][15:], 1):
                s["page"]["rows"] += 5 * k
        self.assertRejectedOnlyIn(9, {"delivery.no_rows"}, rows)

    def test_raw_iq_arriving_in_an_audio_only_window(self):
        def iq(d):
            for k, s in enumerate(d["samples"][15:], 1):
                s["page"]["rxIq"] += 3 * k
                s["page"]["iq"] += 3 * k
        self.assertRejectedOnlyIn(1, {"delivery.no_raw_iq_or_rows"}, iq)

    def test_raw_iq_arriving_in_a_rows_window(self):
        def iq(d):
            for k, s in enumerate(d["samples"][15:], 1):
                s["page"]["rxIq"] += k
        self.assertRejectedOnlyIn(3, {"delivery.no_raw_iq"}, iq)

    def test_raw_iq_that_stalls_for_a_few_seconds(self):
        def stall(d):
            ref = d["samples"][10]["page"]["rxIq"]
            for k, s in enumerate(d["samples"][10:13]):
                s["page"]["rxIq"] = ref
            for s in d["samples"][13:]:
                s["page"]["rxIq"] -= 0  # later samples keep their values, so counters stay monotonic
        self.assertRejectedOnlyIn(2, {"delivery.raw_iq_advances_every_interval"}, stall)

    def test_the_bridge_still_holding_an_iq_subscription_in_audio_only(self):
        self.assertRejectedOnlyIn(10, {"delivery.bridge_iq_subscription_zero"}, lambda d: d["samples"][15]["bridge"].update(iq=1))

    def test_the_bridge_dropping_the_audio_subscription(self):
        self.assertRejectedOnlyIn(5, {"delivery.bridge_subscriptions"}, lambda d: d["samples"][20]["bridge"].update(audio=0))

    def test_the_page_reporting_iq_streaming_in_audio_only(self):
        self.assertRejectedOnlyIn(10, {"delivery.page_not_streaming_iq"}, lambda d: d["samples"][8]["page"].update(iqStreaming=True))

    def test_iq_restarted_after_the_stop_in_audio_only(self):
        def restart(d):
            d["frames"].append({"t": d["stoppedAt"] + 12_000, "p": "iq_start:0;"})
        self.assertRejectedOnlyIn(1, {"arm_c.no_restart_after_stop"}, restart)

    def test_audio_only_without_a_recorded_stop(self):
        self.assertRejectedOnlyIn(1, {"arm_c.single_iq_stop"}, lambda d: d.update(frames=[f for f in d["frames"] if "iq_stop" not in f["p"]]))

    def test_audio_only_without_the_drain(self):
        def nodrain(d):
            d["stoppedAt"] = d["samples"][0]["page"]["t"] - 500
            for f in d["frames"]:
                if "iq_stop" in f["p"]:
                    f["t"] = d["stoppedAt"]
        self.assertRejectedOnlyIn(1, {"arm_c.drained_before_first_sample"}, nodrain)

    def test_a_stop_sent_during_a_display_window(self):
        self.assertRejectedOnlyIn(2, {"delivery.no_iq_stop"}, lambda d: d["frames"].append({"t": d["samples"][5]["page"]["t"], "p": "iq_stop:0;"}))

    def test_a_window_recorded_in_the_wrong_mode(self):
        self.assertRejectedOnlyIn(2, {"record.mode_matches_arm"}, lambda d: d.update(mode="c"))


class NoDelay(Base):
    def test_a_wrong_label_for_the_arm(self):
        report = self.assertRejectedOnlyIn(1, {"nodelay.label_matches_arm", "nodelay.log_matches_label"}, lambda d: d["meta"].update(bridgeNoDelay=0))
        self.assertIn("nodelay", {k.split(".")[0] for k in failed_ids(report, 1)})

    def test_a_log_that_contradicts_the_label(self):
        path = os.path.join(self.bench.logs, "bridge_3.out")
        write_text(path, read_text(path).replace("TCP_NODELAY on accepted TCI sockets: on", "TCP_NODELAY on accepted TCI sockets: off"))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {4, 5, 6, 7})  # the four windows served by that Bridge instance
        self.assertTrue(all(failed_ids(report, n) >= {"nodelay.log_matches_label"} for n in (4, 5, 6, 7)))

    def test_a_log_that_is_consistent_but_disagrees_with_the_label(self):
        # Bridge instance 3 served B-on and A-on windows (labelled on). Make its retained logs consistently say
        # "off": the start-up line off and no socket calls. Nothing inside the logs contradicts itself; only the label does.
        out_path = os.path.join(self.bench.logs, "bridge_3.out")
        write_text(out_path, read_text(out_path).replace("TCP_NODELAY on accepted TCI sockets: on", "TCP_NODELAY on accepted TCI sockets: off"))
        err_path = os.path.join(self.bench.logs, "bridge_3.err")
        write_text(err_path, "\n".join(l for l in read_text(err_path).splitlines() if "Bridge setsockopt" not in l) + "\n")
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {4, 5, 6, 7})
        detail = [c["detail"] for w in report["windows"] if w["n"] == 4 for c in w["checks"] if c["check"] == "nodelay.log_matches_label"][0]
        self.assertIn("log says off, label says on", detail)

    def test_start_up_says_on_but_no_socket_was_set(self):
        path = os.path.join(self.bench.logs, "bridge_3.err")
        keep = [l for l in read_text(path).splitlines() if "Bridge setsockopt" not in l]
        write_text(path, "\n".join(keep) + "\n")
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {4, 5, 6, 7})

    def test_start_up_says_off_but_sockets_were_set(self):
        path = os.path.join(self.bench.logs, "bridge_2.err")
        write_text(path, "xdma_replay_shim: Bridge setsockopt(TCP_NODELAY=1) on fd 9 -> ok\n", "a")
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {2, 3})

    def test_a_failed_setsockopt_in_the_log(self):
        path = os.path.join(self.bench.logs, "bridge_5.err")
        write_text(path, "xdma_replay_shim: Bridge setsockopt(TCP_NODELAY=1) on fd 11 -> FAILED: Protocol not available\n", "a")
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {10})

    def test_missing_logs_are_unverified_not_passed(self):
        bench = Bench(logs=False)
        self.addCleanup(bench.cleanup)
        code, out, report = bench.run()
        self.assertEqual(code, 3, out)
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY UNVERIFIED")
        self.assertTrue(all(w["nodelay"]["status"] == "UNVERIFIED" for w in report["windows"]))
        self.assertFalse(all_window_failures(report))
        self.assertIn("UNVERIFIED", out)

    def test_unverified_can_be_accepted_explicitly_and_is_still_reported(self):
        bench = Bench(logs=False)
        self.addCleanup(bench.cleanup)
        code, out, report = bench.run("--allow-unverified-nodelay")
        self.assertEqual(code, 0, out)
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY UNVERIFIED")
        self.assertIn("UNVERIFIED", out)

    def test_a_log_without_the_start_up_line_is_unverified(self):
        path = os.path.join(self.bench.logs, "bridge_1.out")
        write_text(path, "saturn-bridge: nothing useful here\n")
        code, out, report = self.bench.run()
        self.assertEqual(code, 3, out)
        self.assertEqual([w["n"] for w in report["windows"] if w["nodelay"]["status"] == "UNVERIFIED"], [1])

    def test_invalid_evidence_wins_over_unverified(self):
        bench = Bench(logs=False)
        self.addCleanup(bench.cleanup)
        bench.alter(2, lambda d: d["samples"][15]["page"].update(connected=False))
        code, out, _ = bench.run()
        self.assertEqual(code, 1, out)


class SetLevel(Base):
    def test_a_missing_window(self):
        os.remove(self.bench.path(5))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertTrue({"set.window_count", "set.indices_consecutive", "set.order"} <= set_failures(report))
        self.assertFalse(all_window_failures(report))

    def test_the_wrong_order(self):
        # Swap the POSITIONS of windows 2 (A-off) and 3 (B-off): each is still a self-consistent A-off or
        # B-off window with its own data; only the order in which they were run is wrong.
        a, b = self.bench.load(2), self.bench.load(3)
        a["meta"].update(index=3)
        b["meta"].update(index=2)
        pa, pb = self.bench.path(2), self.bench.path(3)
        os.remove(pa), os.remove(pb)
        write_json(os.path.join(self.bench.windows, "w3_A-off.json"), a)
        write_json(os.path.join(self.bench.windows, "w2_B-off.json"), b)
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.order", set_failures(report))

    def test_a_different_bridge_binary_in_one_window(self):
        self.bench.alter(6, lambda d: d["meta"].update(bridgeSha256="0" * 64))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.one_bridge_binary", set_failures(report))

    def test_a_different_audio_format_in_one_window(self):
        def fmt(d):
            for s in d["samples"]:
                s["page"]["channels"] = 1
        self.bench.alter(7, fmt)
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.same_audio_format_in_every_window", set_failures(report))

    def test_a_setting_that_changed_without_a_restart(self):
        self.bench.alter(2, lambda d: d["meta"].update(bridgeRestartNumber=1))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertTrue(set_failures(report))


class EveryRuleHasATest(Base):
    """One alteration per remaining rule, so that disabling any single rule makes a test fail."""

    COUNTERS = ["iq", "rxIq", "rows", "opusFrames", "pcmFrames", "audioPlayed", "lastAudioSeq", "audioGaps",
                "audioResyncs", "decodeErrors", "lateDrops", "underruns", "overflows", "drops"]

    def test_each_page_counter_going_backwards_is_caught_by_its_own_rule(self):
        for key in self.COUNTERS:
            with self.subTest(counter=key):
                bench = Bench()
                self.addCleanup(bench.cleanup)
                bench.alter(6, lambda d, key=key: d["samples"][15]["page"].update({key: d["samples"][15]["page"][key] - 100_000}))
                code, out, report = bench.run()
                self.assertEqual(code, 1, out)
                self.assertIn(f"counters.{key}_monotonic", failed_ids(report, 6))

    def test_bridge_timestamp_going_backwards(self):
        self.assertRejectedOnlyIn(3, {"bridge.timestamp_never_goes_back"}, lambda d: d["samples"][15]["bridge"].update(updatedAtMs=d["samples"][15]["bridge"]["updatedAtMs"] - 5000))

    def test_page_timestamps_that_do_not_increase(self):
        def swap(d):
            a, b = d["samples"][10]["page"], d["samples"][11]["page"]
            a["t"], b["t"] = b["t"], a["t"]
        self.assertRejectedOnlyIn(4, {"cadence.timestamps_increase"}, swap)

    def test_a_raw_iq_window_whose_render_source_flips(self):
        self.assertRejectedOnlyIn(9, {"delivery.render_source"}, lambda d: d["samples"][20]["page"].update(renderSource="server"))

    def test_a_raw_iq_window_whose_page_stops_streaming(self):
        self.assertRejectedOnlyIn(9, {"delivery.page_streaming"}, lambda d: d["samples"][20]["page"].update(iqStreaming=False))

    def test_a_raw_iq_rate_far_above_the_display_rate(self):
        def fast(d):
            base = d["samples"][0]["page"]["rxIq"]
            for s in d["samples"]:
                s["page"]["rxIq"] = base + 5 * (s["page"]["rxIq"] - base)
        self.assertRejectedOnlyIn(2, {"delivery.raw_iq_rate"}, fast)

    def test_a_rows_window_whose_render_source_flips(self):
        self.assertRejectedOnlyIn(3, {"delivery.render_source"}, lambda d: d["samples"][20]["page"].update(echoMode="iq"))

    def test_a_row_rate_far_below_the_display_rate(self):
        def slow(d):
            base = d["samples"][0]["page"]["rows"]
            for k, s in enumerate(d["samples"]):
                s["page"]["rows"] = base + k * 5  # 5 rows/s: advancing every interval, but far too slow
        self.assertRejectedOnlyIn(3, {"delivery.row_rate"}, slow)

    def test_rows_that_stall_for_a_few_seconds(self):
        def stall(d):
            ref = d["samples"][10]["page"]["rows"]
            for s in d["samples"][10:13]:
                s["page"]["rows"] = ref
        self.assertRejectedOnlyIn(3, {"delivery.rows_advance_every_interval"}, stall)

    def test_the_bridge_audio_subscription_dropping_in_audio_only(self):
        self.assertRejectedOnlyIn(1, {"delivery.bridge_audio_subscription_one"}, lambda d: d["samples"][15]["bridge"].update(audio=0))

    def test_the_bridge_writing_rows_during_audio_only(self):
        def rows(d):
            for k, s in enumerate(d["samples"][15:], 1):
                s["bridge"]["rows_written"] += k
        self.assertRejectedOnlyIn(10, {"delivery.bridge_rows_written_unchanged"}, rows)

    def test_a_record_whose_driver_failed_is_never_a_measurement(self):
        self.assertRejectedOnlyIn(6, {"record.no_failure"}, lambda d: d.update(failure={"message": "page.evaluate: target closed", "stack": ""}))

    def test_a_record_with_a_missing_top_level_field(self):
        self.assertRejectedOnlyIn(2, {"record.top_level_fields"}, lambda d: d.pop("frames"))

    def test_a_record_with_an_unknown_arm(self):
        report = self.assertRejectedOnlyIn(2, {"record.known_arm"}, lambda d: d["meta"].update(arm="Z-turbo"))
        self.assertIn("record.known_arm", failed_ids(report, 2))

    def test_a_record_with_no_samples(self):
        self.assertRejectedOnlyIn(2, {"record.samples_present"}, lambda d: d.update(samples=[]))

    def test_a_record_whose_url_does_not_name_the_display_transport(self):
        self.assertRejectedOnlyIn(2, {"record.url_names_display_transport"}, lambda d: d.update(url="https://127.0.0.1:18443/remote-next?transport=split"))

    def test_a_file_name_that_disagrees_with_its_meta(self):
        os.rename(self.bench.path(4), os.path.join(self.bench.windows, "w4_B-off.json"))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("record.file_name_matches_meta", failed_ids(report, 4))

    def test_an_unreadable_record(self):
        write_text(self.bench.path(3), "{ this is not json")
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("record.readable", failed_ids(report, 3))

    def test_arm_c_labelled_nodelay_off_at_both_ends(self):
        for n in (1, 10):
            self.bench.alter(n, lambda d: d["meta"].update(bridgeNoDelay=0))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.arm_c_has_nodelay_on_at_both_ends", set_failures(report))

    def test_a_missing_window_is_named_as_missing(self):
        os.remove(self.bench.path(5))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.no_missing_windows", set_failures(report))

    def test_two_pids_in_one_bridge_instance(self):
        self.bench.alter(5, lambda d: d["meta"].update(bridgePid=1))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.one_state_and_pid_per_bridge_instance", set_failures(report))

    def test_a_restart_with_no_change_of_setting(self):
        self.bench.alter(6, lambda d: d["meta"].update(bridgeRestartNumber=4))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.restart_exactly_when_nodelay_changes", set_failures(report))

    def test_restart_numbers_that_go_backwards(self):
        self.bench.alter(5, lambda d: d["meta"].update(bridgeRestartNumber=2))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.restart_numbers_nondecreasing", set_failures(report))


class Rxc1(Base):
    """The live profile requires the acquisition owner's RXC1 state, recorded per window and held constant."""
    GOOD = {"state": "armed, valid poll", "pid": 4242, "identity": "fw 1.31.002 / g2"}

    def with_rxc1(self, change=None):
        for n in range(1, 11):
            def go(d, n=n):
                d["meta"]["rxc1"] = copy.deepcopy(self.GOOD)
                if change:
                    change(n, d)
            self.bench.alter(n, go)

    def test_the_rehearsal_does_not_require_it(self):
        code, out, _ = self.bench.run()
        self.assertEqual(code, 0, out)

    def test_a_placeholder_is_rejected_when_required(self):
        code, out, report = self.bench.run("--require-rxc1")
        self.assertEqual(code, 1, out)
        self.assertTrue(all("rxc1.recorded" in failed_ids(report, n) for n in range(1, 11)))

    def test_a_recorded_constant_state_is_accepted(self):
        self.with_rxc1()
        code, out, report = self.bench.run("--require-rxc1")
        self.assertEqual(code, 0, out)

    def test_a_window_with_no_recorded_state(self):
        self.with_rxc1(lambda n, d: d["meta"]["rxc1"].pop("state") if n == 4 else None)
        code, out, report = self.bench.run("--require-rxc1")
        self.assertEqual(code, 1, out)
        self.assertEqual(set(all_window_failures(report)), {4})
        self.assertIn("rxc1.recorded", failed_ids(report, 4))

    def test_a_state_that_changes_between_windows(self):
        self.with_rxc1(lambda n, d: d["meta"]["rxc1"].update(state="armed, no valid poll") if n == 7 else None)
        code, out, report = self.bench.run("--require-rxc1")
        self.assertEqual(code, 1, out)
        self.assertIn("set.rxc1_state_constant", set_failures(report))
        self.assertFalse(all_window_failures(report))


class Usage(Base):
    def test_a_missing_directory_is_a_usage_error(self):
        code, _, _ = self.bench.run(windows=os.path.join(self.bench.root, "nope"))
        self.assertEqual(code, 2)

    def test_an_empty_directory_is_a_usage_error(self):
        empty = os.path.join(self.bench.root, "empty")
        os.makedirs(empty)
        code, _, _ = self.bench.run(windows=empty)
        self.assertEqual(code, 2)


class ReviewerCases(unittest.TestCase):
    """The seven altered single-window copies from DarkOverLord's review of the arm C package,
    on window 1 (arm C), single-window mode, with the retained Bridge log."""

    def run_case(self, alter, extra=()):
        root = tempfile.mkdtemp(prefix="check-order-reviewer-")
        self.addCleanup(shutil.rmtree, root, True)
        windows = os.path.join(root, "windows")
        os.makedirs(windows)
        data = read_json(os.path.join(FIX, "windows", "w1_C.json"))
        alter(data)
        write_json(os.path.join(windows, "w1_C.json"), data)
        proc = subprocess.run([sys.executable, CHECKER, windows, "--mode", "single", "--bridge-logs", os.path.join(FIX, "bridge-logs"),
                               "--report-dir", os.path.join(root, "report"), *extra], capture_output=True, text=True, timeout=60)
        return proc.returncode, proc.stdout, read_json(os.path.join(root, "report", "order_check.json"))

    def test_valid_control_passes(self):
        code, out, report = self.run_case(lambda d: None)
        self.assertEqual(code, 0, out)

    def test_bridge_read_error_is_rejected(self):
        code, out, report = self.run_case(lambda d: d["samples"].__setitem__(15, {**d["samples"][15], "bridge": {"error": "simulated telemetry read failure"}}))
        self.assertEqual(code, 1, out)
        self.assertIn("telemetry.every_sample_complete", failed_ids(report, 1))

    def test_stale_bridge_after_the_second_sample_is_rejected(self):
        def stale(d):
            ref = d["samples"][1]["bridge"]["updatedAtMs"]
            for s in d["samples"][2:]:
                s["bridge"]["updatedAtMs"] = ref
        code, out, report = self.run_case(stale)
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.fresh_every_sample", failed_ids(report, 1))

    def test_midwindow_disconnect_is_rejected(self):
        code, out, report = self.run_case(lambda d: d["samples"][15]["page"].update(connected=False))
        self.assertEqual(code, 1, out)
        self.assertIn("page.connected_every_sample", failed_ids(report, 1))

    def test_midwindow_codec_change_is_rejected(self):
        code, out, report = self.run_case(lambda d: d["samples"][15]["page"].update(codec="pcm"))
        self.assertEqual(code, 1, out)
        self.assertIn("audio.codec_every_sample", failed_ids(report, 1))

    def test_wrong_nodelay_label_is_rejected(self):
        code, out, report = self.run_case(lambda d: d["meta"].update(bridgeNoDelay=0))
        self.assertEqual(code, 1, out)
        self.assertIn("nodelay.log_matches_label", failed_ids(report, 1))

    def test_a_genuine_audio_gap_is_kept_as_valid_bad_performance(self):
        code, out, report = self.run_case(lambda d: d["samples"][-1]["page"].update(audioGaps=d["samples"][-1]["page"]["audioGaps"] + 1))
        self.assertEqual(code, 0, out)
        self.assertEqual([w for w in report["windows"] if w["n"] == 1][0]["performance"]["audio_gaps"], 1)


if __name__ == "__main__":
    unittest.main(verbosity=1)
