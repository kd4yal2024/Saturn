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


# --- a window made to look as if it had been read through the owner collector (collector.mjs over owner_reader.py).
# This is an independent mirror of the collector's mapping; test_the_mirror_agrees_with_a_real_collector_run (when the real
# fixture exists) is what keeps the mirror honest.
BOOT = "5f1c0a52-3b6e-4c8a-9d21-0f7a9b3c6e11"
OTHER_HASH = "e" * 64
MIRROR_METRICS = {"iq": "iq", "audio": "audio", "connections": "connections", "iq_tci_frames_s": "iq_tci_frames_s", "rx_audio_frames_s": "rx_audio_frames_s",
                  "rows_written": "display_spectrum_rows_written", "spectrum_clients": "display_spectrum_clients", "audio_dropped_s": "audio_dropped_s",
                  "tcp_outq_hwm_bytes": "tcp_outq_hwm_bytes", "out_hwm_bytes": "out_hwm_bytes", "outbound_drops": "outbound_drops"}


def collector_answer(meta, b, pi_read, boot, ticks, nodelay, rxc1=("disabled", 0), fpga=("53460003", 1, 31)):
    metrics = {metric: b[field] for field, metric in MIRROR_METRICS.items() if field in b}
    metrics.update(pid=meta["bridgePid"], build_git_sha="abc1234", date_code_hex=fpga[0], firmware_major=fpga[1], firmware_minor=fpga[2])
    if rxc1 is not None:
        metrics["rx_counter_v31"] = {"schema": "rxc1-v1", "source_backend": "xdma", "status": rxc1[0], "sampled_at_ms": 0, "host_acquisition_failures": rxc1[1], "ddc": []}
    if nodelay is not None:
        metrics.update(tci_nodelay_enabled=nodelay[0], tci_nodelay_confirmed_total=nodelay[1], tci_nodelay_failed_total=nodelay[2])
    doc = {"schema_version": 1, "updated_at_ms": b["updatedAtMs"], "source": "saturn-bridge", "backend": "xdma", "metrics": metrics}
    return {"ok": True, "schema": "saturn-owner-reader-v1", "piReadAtMs": pi_read, "path": "/run/saturn-bridge/perf.json", "document": doc,
            "documentSha256": hashlib.sha256(json.dumps(doc).encode()).hexdigest(), "piSourceUpdatedAtMs": b["updatedAtMs"],
            "piSourceAgeMs": pi_read - b["updatedAtMs"], "fileMtimeMs": b["updatedAtMs"],
            "owner": {"pid": meta["bridgePid"], "alive": True, "startTicks": ticks, "clockTicksPerSecond": 100, "bootId": boot,
                      "exe": "/usr/local/bin/saturn-bridge", "exeSha256": meta["bridgeSha256"], "exeError": None, "mainPid": None, "mainPidMatches": None}}


def mapped_bridge(b, answer, entry, nodelay):
    """The sample.bridge record the collector builds from one answer (mirror of mapReading)."""
    o = answer["owner"]
    out = {k: v for k, v in b.items() if k in MIRROR_METRICS or k == "updatedAtMs"}
    out.update(piSourceAgeMs=answer["piSourceAgeMs"], piSourceUpdatedAtMs=answer["piSourceUpdatedAtMs"], piReadAtMs=answer["piReadAtMs"],
               documentSha256=answer["documentSha256"], ownerPid=o["pid"], ownerStartTicks=o["startTicks"], ownerAlive=o["alive"], bootId=o["bootId"],
               exeSha256=o["exeSha256"], exeError=o["exeError"], mainPidMatches=o["mainPidMatches"], buildGitSha=answer["document"]["metrics"].get("build_git_sha"),
               collectorSeq=entry["seq"], collectorSpawn=entry["spawn"], collectorReceivedAtMs=entry["receivedAtMs"], requestLatencyMs=entry["latencyMs"])
    if nodelay is not None:
        out.update(nodelayEnabled=nodelay[0], nodelayConfirmedTotal=nodelay[1], nodelayFailedTotal=nodelay[2])
    mm = answer["document"]["metrics"]
    out.update(fpgaBuildId=mm["date_code_hex"], firmwareMajor=mm["firmware_major"], firmwareMinor=mm["firmware_minor"])
    rx = mm.get("rx_counter_v31")
    if isinstance(rx, dict):
        out.update(rxc1Status=rx["status"], rxc1HostAcquisitionFailures=rx["host_acquisition_failures"])
    return out


def add_collector(w, window_path, *, pi_read=None, boot=BOOT, ticks=None, nodelay="auto", served=1, latency=25, recv_offset=30, warmup_failure=False,
                  sidecar=True, answer_hook=None, sample_hook=None, summary=True, rxc1=("disabled", 0), fpga=("53460003", 1, 31)):
    """Rewrite a window record as if it had been collected, and write its sidecar. Hooks alter the raw answer (so the sample and the
    sidecar agree on the alteration) or only the derived sample (so they disagree)."""
    meta = w["meta"]
    ticks = 5000 + 100 * meta["bridgeRestartNumber"] if ticks is None else ticks
    boot = boot(meta) if callable(boot) else boot
    ticks = ticks(meta) if callable(ticks) else ticks
    if nodelay == "auto":
        nodelay = (meta["bridgeNoDelay"], served if meta["bridgeNoDelay"] == 1 else 0, 0)
    pi_read = pi_read or (lambda i, s: s["page"]["t"] + 5)
    num = lambda v, i: v(i) if callable(v) else v
    t0 = w["samples"][0]["page"]["t"]
    entries = [{"kind": "start", "schema": "saturn-collector-v1", "startedAtMs": t0 - 20000, "command": ["python3", "owner_reader.py"], "timeoutMs": 2000, "spawnTimeoutMs": 10000, "source": "local"},
               {"kind": "spawn", "spawn": 1, "readerPid": 4321, "atMs": t0 - 20000}]
    seq, spawn_no, failures = 0, 1, 0

    def read(page_t, b, i, hook=True, fail=False):
        nonlocal seq, spawn_no, failures
        seq += 1
        recv, lat = page_t + num(recv_offset, i), num(latency, i)
        if fail:
            failures += 1
            spawn_no += 1
            entries.append({"kind": "read", "seq": seq, "spawn": spawn_no - 1, "spawned": False, "sentAtMs": recv - lat, "receivedAtMs": recv, "latencyMs": lat,
                            "error": "no response within 2000 ms", "raw": None})
            entries.append({"kind": "spawn", "spawn": spawn_no, "readerPid": 4322, "atMs": recv})
            return {"error": "no response within 2000 ms", "collectorSeq": seq, "collectorSpawn": spawn_no - 1, "collectorReceivedAtMs": recv, "requestLatencyMs": lat}
        answer = collector_answer(meta, b, pi_read(i, {"page": {"t": page_t}, "bridge": b}), boot, ticks, nodelay, rxc1, fpga)
        if hook and answer_hook:
            answer_hook(i, answer)
        entry = {"kind": "read", "seq": seq, "spawn": spawn_no, "spawned": False, "sentAtMs": recv - lat, "receivedAtMs": recv, "latencyMs": lat, "error": None,
                 "raw": json.dumps(answer, separators=(",", ":"))}
        entries.append(entry)
        out = mapped_bridge(b, answer, entry, nodelay)
        if hook and sample_hook:
            sample_hook(i, out)
        return out

    for name, dt, fail in (("atConnect", -8000, warmup_failure), ("atWarm", -5000, False), ("start", -1000, False)):
        rec = w["start"] if name == "start" else w[name]
        rec["bridge"] = read(t0 + dt, rec["bridge"], -1, hook=False, fail=fail)
    for i, s in enumerate(w["samples"]):
        s["bridge"] = read(s["page"]["t"], s["bridge"], i)
    if summary:
        w["collector"] = {"schema": "saturn-collector-v1", "reads": seq, "failures": failures, "spawns": spawn_no, "timeoutMs": 2000}
    if sidecar:
        with open(window_path[: -len(".json")] + ".collector.jsonl", "w") as fh:
            for e in entries:
                fh.write(json.dumps(e) + "\n")
    return entries


def collect_all(bench, skip=(), per_window=None, **kw):
    """add_collector to every window of a bench, counting how many windows each Bridge instance has served."""
    served = {}
    for path in window_files(bench.windows):
        w = read_json(path)
        n = w["meta"]["bridgeRestartNumber"]
        served[n] = served.get(n, 0) + 1
        if w["meta"]["index"] not in skip:
            opts = dict(kw, served=served[n])
            if per_window:
                opts.update(per_window(w["meta"]))
            add_collector(w, path, **opts)
        write_json(path, w)


def sidecar_path(bench, n):
    return bench.path(n)[: -len(".json")] + ".collector.jsonl"


def read_sidecar_lines(bench, n):
    with open(sidecar_path(bench, n)) as fh:
        return [json.loads(line) for line in fh if line.strip()]


def write_sidecar_lines(bench, n, entries):
    with open(sidecar_path(bench, n), "w") as fh:
        for e in entries:
            fh.write(json.dumps(e) + "\n")


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

    def test_audio_that_never_started_is_a_failed_setup(self):
        def never(d):
            for s in d["samples"]:
                for k in ("audioPlayed", "opusFrames", "lastAudioSeq"):
                    s["page"][k] = 0
        self.assertRejectedOnlyIn(4, {"audio.started"}, never)

    def test_audio_that_stops_during_the_window_is_a_result_not_a_rejection(self):
        def stops(d):
            frozen = {k: d["samples"][10]["page"][k] for k in ("audioPlayed", "opusFrames", "lastAudioSeq")}
            for s in d["samples"][10:]:
                s["page"].update(frozen)
        self.bench.alter(4, stops)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 4][0]["performance"]
        self.assertGreaterEqual(perf["stalled_seconds"], 18)
        self.assertTrue(perf["audio_slow"])

    def test_audio_at_a_fraction_of_the_rate_with_many_underruns_is_a_result(self):
        def slow(d):
            first = copy.deepcopy(d["samples"][0]["page"])
            for i, s in enumerate(d["samples"]):
                for key in ("opusFrames", "audioPlayed", "lastAudioSeq"):
                    s["page"][key] = first[key] + 5 * i
                s["page"]["underruns"] = i
        self.bench.alter(1, slow)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 1][0]["performance"]
        self.assertEqual(perf["underruns"], 29)
        self.assertTrue(perf["audio_slow"])
        self.assertLess(perf["played_per_s"], 10)

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

    def test_raw_iq_that_stalls_for_a_few_seconds_is_a_result_not_a_rejection(self):
        def stall(d):
            ref = d["samples"][10]["page"]["rxIq"]
            for s in d["samples"][10:13]:
                s["page"]["rxIq"] = ref
        self.bench.alter(2, stall)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 2][0]["performance"]
        self.assertEqual(perf["display_stalled_seconds"], 2)  # samples 11 and 12 repeat sample 10

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

    def test_a_raw_iq_rate_far_above_the_display_rate_is_flagged_not_rejected(self):
        def fast(d):
            base = d["samples"][0]["page"]["rxIq"]
            for s in d["samples"]:
                s["page"]["rxIq"] = base + 5 * (s["page"]["rxIq"] - base)
        self.bench.alter(2, fast)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 2][0]["performance"]
        self.assertTrue(perf["display_rate_outside_expected_band"])
        self.assertGreater(perf["display_per_s"], 100)

    def test_a_rows_window_whose_render_source_flips(self):
        self.assertRejectedOnlyIn(3, {"delivery.render_source"}, lambda d: d["samples"][20]["page"].update(echoMode="iq"))

    def test_a_row_rate_far_below_the_display_rate_is_flagged_not_rejected(self):
        def slow(d):
            base = d["samples"][0]["page"]["rows"]
            for k, s in enumerate(d["samples"]):
                s["page"]["rows"] = base + k * 5
        self.bench.alter(3, slow)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 3][0]["performance"]
        self.assertTrue(perf["display_rate_outside_expected_band"])
        self.assertLess(perf["display_per_s"], 10)

    def test_rows_that_stall_for_a_few_seconds_are_a_result_not_a_rejection(self):
        def stall(d):
            ref = d["samples"][10]["page"]["rows"]
            for s in d["samples"][10:13]:
                s["page"]["rows"] = ref
        self.bench.alter(3, stall)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 3][0]["performance"]
        self.assertEqual(perf["display_stalled_seconds"], 2)  # samples 11 and 12 repeat sample 10

    def test_a_display_pause_with_catch_up_is_a_result(self):
        # the reviewer's probe: one frozen interval, the next catching up; mode, subscriptions and telemetry all sound
        def pause(d):
            d["samples"][15]["page"]["rows"] = d["samples"][14]["page"]["rows"]
        self.bench.alter(3, pause)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertEqual([w for w in report["windows"] if w["n"] == 3][0]["performance"]["display_stalled_seconds"], 1)

    def test_a_display_that_delivered_nothing_with_the_subscription_in_place_is_a_result(self):
        def nothing(d):
            ref = d["samples"][0]["page"]["rows"]
            for s in d["samples"]:
                s["page"]["rows"] = ref
        self.bench.alter(3, nothing)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 3][0]["performance"]
        self.assertTrue(perf["display_delivered_nothing"])
        self.assertIn("display_delivered_nothing", out)

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


class DataValidation(Base):
    """Telemetry and identity that is malformed, contradictory or non-finite is never a measurement."""

    def test_a_nan_counter_in_the_record(self):
        # Python's json module writes and reads NaN, which is not JSON: the record is rejected as unreadable.
        self.bench.alter(1, lambda d: d["samples"][15]["page"].update(audioGaps=float("nan")))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("record.readable", failed_ids(report, 1))
        self.assertEqual(set(all_window_failures(report)), {1})

    def test_infinity_from_an_oversized_exponent_in_a_sample(self):
        # Valid JSON syntax that parses to infinity. A marker value is written, then replaced in the text.
        self.bench.alter(2, lambda d: d["samples"][15]["page"].update(audioGaps=123456789))
        path = self.bench.path(2)
        write_text(path, read_text(path).replace('"audioGaps": 123456789', '"audioGaps": 1e999', 1))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertTrue({"record.finite_numbers", "telemetry.every_sample_complete"} <= failed_ids(report, 2))
        self.assertEqual(set(all_window_failures(report)), {2})

    def test_a_non_finite_number_in_a_field_the_checker_does_not_otherwise_read(self):
        path = self.bench.path(3)
        data = read_json(path)
        data["atWarm"]["page"]["jitterP95"] = 1.0
        write_json(path, data)
        write_text(path, read_text(path).replace('"jitterP95": 1.0', '"jitterP95": 1e999', 1))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("record.finite_numbers", failed_ids(report, 3))

    def test_a_bridge_drop_count_that_the_bridge_clears_is_valid_and_reported(self):
        # tci/mod.rs clears drop_count (swap(0)) whenever the Bridge reports rx_drops to its client, so perf.json's outbound_drops is "drops not yet
        # reported". A real rehearsal window showed 0,...,1,1,0,...: a genuine drop event, not an invalid record.
        def real_pattern(d):
            for i, s in enumerate(d["samples"]):
                s["bridge"]["outbound_drops"] = 1 if i in (14, 15) else 0
        self.bench.alter(10, real_pattern)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 10][0]["performance"]
        self.assertEqual((perf["bridge_outbound_drops_max"], perf["bridge_samples_with_pending_drops"]), (1, 2))
        self.assertIn("bridge_samples_with_pending_drops 2", out, "the drop event is listed with the performance events")

    def test_a_larger_pending_drop_count_that_falls_back_to_zero_is_also_valid(self):
        def reset(d):
            for i, s in enumerate(d["samples"]):
                s["bridge"]["outbound_drops"] = 10 if i < 15 else 0
        self.bench.alter(1, reset)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertFalse(any(c["check"].startswith("counters.bridge_outbound") for w in report["windows"] for c in w["checks"]), "there is no monotonic rule for it")
        perf = [w for w in report["windows"] if w["n"] == 1][0]["performance"]
        self.assertEqual((perf["bridge_outbound_drops_max"], perf["bridge_samples_with_pending_drops"]), (10, 15))

    def test_a_cumulative_bridge_row_counter_that_decreases_is_still_rejected(self):
        def reset(d):
            for i, s in enumerate(d["samples"]):
                s["bridge"]["rows_written"] = 100 + 10 * i if i < 15 else 5
        self.assertRejectedOnlyIn(3, {"counters.bridge_rows_written_monotonic"}, reset)

    def test_genuine_bridge_drops_stay_valid_and_are_reported(self):
        def drops(d):
            for i, s in enumerate(d["samples"]):
                s["bridge"]["outbound_drops"] = i // 5
        self.bench.alter(5, drops)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        perf = [w for w in report["windows"] if w["n"] == 5][0]["performance"]
        self.assertEqual((perf["bridge_outbound_drops_max"], perf["bridge_samples_with_pending_drops"]), (5, 25))

    def test_per_second_bridge_rates_are_not_cumulative(self):
        def rates(d):
            for i, s in enumerate(d["samples"]):
                s["bridge"]["audio_dropped_s"] = 5 if i < 10 else 0
                s["bridge"]["rx_audio_frames_s"] = 47.0 - (i % 3)
        self.bench.alter(5, rates)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)

    def test_pcm_frames_advancing_in_a_window_declared_opus(self):
        def pcm(d):
            for i, s in enumerate(d["samples"]):
                s["page"]["pcmFrames"] = 50 * i
        self.assertRejectedOnlyIn(1, {"audio.no_frames_of_the_other_codec"}, pcm)

    def test_a_nonzero_pcm_startup_count_that_does_not_move_is_harmless(self):
        def startup(d):
            for s in d["samples"]:
                s["page"]["pcmFrames"] = 12
        self.bench.alter(1, startup)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)

    def test_opus_frames_advancing_in_a_window_declared_pcm(self):
        self.bench.alter(1, lambda d: None)
        code, out, report = self.bench.run("--expect-codec", "pcm")
        self.assertEqual(code, 1, out)
        self.assertTrue(all("audio.codec_every_sample" in failed_ids(report, n) for n in range(1, 11)))
        self.assertTrue(all("audio.no_frames_of_the_other_codec" in failed_ids(report, n) for n in range(1, 11)))

    def test_ten_empty_binary_hashes(self):
        for n in range(1, 11):
            self.bench.alter(n, lambda d: d["meta"].update(bridgeSha256=""))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertTrue(all("record.bridge_sha256_valid" in failed_ids(report, n) for n in range(1, 11)))
        self.assertIn("set.one_bridge_binary", set_failures(report))

    def test_ten_equal_but_malformed_binary_hashes(self):
        for n in range(1, 11):
            self.bench.alter(n, lambda d: d["meta"].update(bridgeSha256="bd541874" * 7))  # 56 chars: equal across the set, not a SHA-256
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.one_bridge_binary", set_failures(report))

    def test_an_uppercase_or_non_hex_hash_is_not_accepted_as_one(self):
        for bad in ("BD541874A1B1A1B928A3E141D3B0E972C1D35ECAE90A60F053F227393E738EB2", "z" * 64, 12345):
            with self.subTest(hash=bad):
                bench = Bench()
                self.addCleanup(bench.cleanup)
                bench.alter(4, lambda d, bad=bad: d["meta"].update(bridgeSha256=bad))
                code, out, report = bench.run()
                self.assertEqual(code, 1, out)
                self.assertIn("record.bridge_sha256_valid", failed_ids(report, 4))


class Outcomes(Base):
    def test_a_disconnect_is_kept_and_reported_as_its_own_outcome(self):
        self.bench.alter(2, lambda d: d["samples"][15]["page"].update(connected=False))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        window = [w for w in report["windows"] if w["n"] == 2][0]
        self.assertTrue(any("transport disconnected" in o for o in window["outcomes"]))
        self.assertIn("OUTCOME: transport disconnected", out)
        # the performance observed in the rest of the window is still reported
        self.assertIn("underruns", window["performance"])

    def test_no_outcome_is_reported_for_a_sound_window(self):
        code, out, report = self.bench.run()
        self.assertTrue(all(not w["outcomes"] for w in report["windows"]))
        self.assertNotIn("OUTCOME", out)


class LiveFreshness(unittest.TestCase):
    """The live profile, on synthetic 600 s windows built from the real window 1 (arm C).

    Advancing timestamps cannot prove a feed fresh across two computers (a clock offset and an old replay look the same),
    so freshness there rests only on an age computed on the Pi from its own clock: bridge.piSourceAgeMs.
    """

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="check-order-live-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.windows = os.path.join(self.root, "windows")
        os.makedirs(self.windows)

    def make(self, shift_ms=0, pi_age=None, partial=False, owner=None, rxc1_pid=None, **collector):
        """owner (default: when an age is given) makes the window look as if the owner collector had read it."""
        owner = (pi_age is not None) if owner is None else owner
        data = read_json(os.path.join(FIX, "windows", "w1_C.json"))
        first = copy.deepcopy(data["samples"][0])
        data["samples"] = []
        for i in range(600):
            s = copy.deepcopy(first)
            s["page"]["t"] += i * 1000
            s["bridge"]["updatedAtMs"] += i * 1000 + shift_ms
            for key in ("opusFrames", "audioPlayed", "lastAudioSeq"):
                s["page"][key] += 50 * i
            if pi_age is not None and not owner and not (partial and i % 2):
                s["bridge"]["piSourceAgeMs"] = pi_age(i) if callable(pi_age) else pi_age
            data["samples"].append(s)
        collector.setdefault("rxc1", ("valid", 0))
        data["meta"]["rxc1"] = {"state": "valid", "pid": data["meta"]["bridgePid"] if rxc1_pid is None else rxc1_pid, "identity": "1.31.002 / 0x53460004"}
        path = os.path.join(self.windows, "w1_C.json")
        if owner:
            age = (lambda i: pi_age(i) if callable(pi_age) else pi_age) if pi_age is not None else (lambda i: 400)
            add_collector(data, path, pi_read=lambda i, s: s["bridge"]["updatedAtMs"] + age(i), **collector)
            if partial:
                for i, s in enumerate(data["samples"]):
                    if i % 2:
                        del s["bridge"]["piSourceAgeMs"]
        write_json(path, data)

    def run_live(self, *extra):
        report = os.path.join(self.root, "report")
        proc = subprocess.run([sys.executable, CHECKER, self.windows, "--mode", "single", "--profile", "live", "--bridge-logs",
                               os.path.join(FIX, "bridge-logs"), "--report-dir", report, *extra], capture_output=True, text=True, timeout=60)
        return proc.returncode, proc.stdout, read_json(os.path.join(report, "order_check.json"))

    def test_a_fresh_pi_side_age_verifies_the_feed(self):
        self.make(pi_age=lambda i: 300 + (i % 400))
        code, out, report = self.run_live()
        self.assertEqual(code, 0, out)
        self.assertEqual(report["windows"][0]["freshness"]["status"], "VERIFIED")
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED")

    def test_a_stale_pi_side_age_is_rejected(self):
        self.make(pi_age=86_400_000)  # the cached document is a day old on the Pi's own clock
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.pi_source_age_every_sample", failed_ids(report, 1))
        self.assertEqual(report["windows"][0]["freshness"]["status"], "CONTRADICTED")

    def test_one_stale_reading_among_fresh_ones_is_rejected(self):
        self.make(pi_age=lambda i: 90_000 if i == 300 else 400)
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.pi_source_age_every_sample", failed_ids(report, 1))

    def test_a_clock_offset_with_a_fresh_pi_side_age_is_not_stale(self):
        # Bridge timestamps a day away from the browser's clock: a different computer's clock, not an old feed.
        self.make(shift_ms=-86_400_000, pi_age=400)
        code, out, report = self.run_live()
        self.assertEqual(code, 0, out)

    def test_an_advancing_but_day_old_feed_with_no_age_evidence_is_unverified_not_valid(self):
        self.make(shift_ms=-86_400_000)
        code, out, report = self.run_live()
        self.assertEqual(code, 3, out)
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED, FRESHNESS UNVERIFIED, OWNER IDENTITY UNVERIFIED")
        self.assertEqual(report["windows"][0]["freshness"]["status"], "UNVERIFIED")
        self.assertIn("telemetry freshness UNVERIFIED", out)

    def test_unverified_freshness_can_be_accepted_explicitly_and_is_still_reported(self):
        self.make()
        code, out, report = self.run_live("--allow-unverified-freshness")
        self.assertEqual(code, 3, "the owner identity is still unverified: each allowance is its own")
        code, out, report = self.run_live("--allow-unverified-freshness", "--allow-unverified-owner")
        self.assertEqual(code, 0, out)
        self.assertEqual(report["windows"][0]["freshness"]["status"], "UNVERIFIED")
        self.assertIn("FRESHNESS UNVERIFIED", report["verdict"])

    def test_age_evidence_present_in_only_some_samples_is_rejected(self):
        self.make(pi_age=400, partial=True)
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.pi_source_age_every_sample", failed_ids(report, 1))

    def test_a_non_finite_pi_side_age_is_rejected(self):
        self.make(pi_age=400)
        path = os.path.join(self.windows, "w1_C.json")
        write_text(path, read_text(path).replace('"piSourceAgeMs": 400', '"piSourceAgeMs": 1e999', 1))
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)

    def test_the_owner_allowance_does_not_cover_unverified_freshness(self):
        self.make()   # neither an age nor a collector
        code, out, report = self.run_live("--allow-unverified-owner")
        self.assertEqual(code, 3, "freshness is still unverified")
        self.assertEqual(report["windows"][0]["owner"]["status"], "UNVERIFIED")

    def test_an_rxc1_state_recorded_for_another_process_than_the_owner_is_rejected(self):
        self.make(pi_age=400, rxc1_pid=1)
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)
        self.assertEqual(failed_ids(report, 1), {"rxc1.pid_matches_owner"})

    def test_a_collected_live_window_whose_sidecar_was_not_kept_is_rejected(self):
        self.make(pi_age=400)
        os.remove(os.path.join(self.windows, "w1_C.collector.jsonl"))
        code, out, report = self.run_live()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_retained", failed_ids(report, 1))

    def test_a_collected_live_window_reports_the_owner_identity_it_verified(self):
        self.make(pi_age=lambda i: 300 + (i % 400))
        code, out, report = self.run_live()
        self.assertEqual(code, 0, out)
        owner = report["windows"][0]["owner"]
        self.assertEqual(owner["status"], "VERIFIED")
        self.assertIn("constant over 600 raw answers", owner["why"])

    def test_the_rehearsal_profile_still_uses_the_single_clock_age_rule(self):
        code, out, report = Bench().run()
        self.assertEqual(code, 0, out)
        self.assertTrue(all(w["freshness"]["status"] == "VERIFIED" for w in report["windows"]))


class OwnerEvidence(unittest.TestCase):
    """The owner's process identity, the collector's timing and the retained raw answers (rehearsal profile, --require-owner)."""

    def setUp(self):
        self.bench = Bench()
        self.addCleanup(self.bench.cleanup)

    def collect(self, **kw):
        collect_all(self.bench, **kw)

    def run_owner(self, *extra):
        return self.bench.run("--require-owner", *extra)

    def only_window(self, n, rules, report, exact=False):
        failures = all_window_failures(report)
        self.assertEqual(set(failures), {n}, f"only window {n} may fail, got {failures}")
        if exact:
            self.assertEqual(failures[n], set(rules))
        else:
            self.assertTrue(set(rules) <= failures[n], f"expected {rules} in {failures[n]}")

    # -- positive controls
    def test_a_collected_set_is_valid_with_owner_identity_verified(self):
        self.collect()
        code, out, report = self.run_owner()
        self.assertEqual(code, 0, out)
        self.assertFalse(all_window_failures(report)); self.assertFalse(set_failures(report))
        self.assertTrue(all(w["owner"]["status"] == "VERIFIED" for w in report["windows"]))
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED")

    def test_a_rehearsal_set_taken_without_the_collector_is_not_collected_and_still_valid(self):
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertTrue(all(w["owner"]["status"] == "NOT COLLECTED" for w in report["windows"]))
        self.assertNotIn("owner identity", out)

    def test_requiring_the_owner_when_none_was_collected_is_unverified_not_valid(self):
        code, out, report = self.run_owner()
        self.assertEqual(code, 3, out)
        self.assertTrue(all(w["owner"]["status"] == "UNVERIFIED" for w in report["windows"]))
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED, OWNER IDENTITY UNVERIFIED")
        self.assertIn("owner identity UNVERIFIED", out)
        code, out, report = self.run_owner("--allow-unverified-owner")
        self.assertEqual(code, 0, out)
        self.assertIn("OWNER IDENTITY UNVERIFIED", report["verdict"])

    def test_a_failed_warm_up_read_is_reported_but_does_not_invalidate_the_window(self):
        self.collect(per_window=lambda meta: {"warmup_failure": meta["index"] == 3})
        code, out, report = self.run_owner()
        self.assertEqual(code, 0, out)
        window = [w for w in report["windows"] if w["n"] == 3][0]
        self.assertTrue(any("collector: 1 failed read(s)" in o and "2 reader start(s)" in o for o in window["outcomes"]), window["outcomes"])
        self.assertIn("OUTCOME: collector: 1 failed read(s)", out)

    def test_a_collector_failure_inside_the_window_invalidates_it(self):
        self.collect()
        def fail(d):
            d["samples"][12]["bridge"] = {"error": "no response within 2000 ms", "collectorSeq": d["samples"][12]["bridge"]["collectorSeq"], "requestLatencyMs": 2000}
        self.bench.alter(4, fail)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("telemetry.every_sample_complete", failed_ids(report, 4))

    # -- fields
    def test_a_sample_missing_a_collector_field(self):
        self.collect()
        self.bench.alter(3, lambda d: d["samples"][5]["bridge"].pop("exeSha256"))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.fields_every_sample"}, report)
        self.assertEqual([w for w in report["windows"] if w["n"] == 3][0]["owner"]["status"], "CONTRADICTED")

    def test_malformed_collector_fields_are_each_rejected(self):
        self.collect()
        cases = {"ownerPid": 0, "ownerStartTicks": -1, "collectorSeq": 0, "collectorSpawn": True, "piReadAtMs": float("inf"), "requestLatencyMs": "25",
                 "exeSha256": "E" * 64, "documentSha256": "short", "ownerAlive": "yes", "bootId": ""}
        for key, bad in cases.items():
            with self.subTest(field=key):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench)
                bench.alter(3, lambda d, key=key, bad=bad: d["samples"][5]["bridge"].__setitem__(key, bad))
                text = read_text(bench.path(3)).replace("Infinity", "1e999")
                write_text(bench.path(3), text)
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("owner.fields_well_formed", failed_ids(report, 3))

    def test_an_executable_hash_the_reader_could_not_take_says_why(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(exeSha256=None, exeError="cannot read the running executable: [Errno 13] Permission denied") if i == 5 else None)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        detail = [c["detail"] for w in report["windows"] if w["n"] == 3 for c in w["checks"] if c["check"] == "owner.fields_well_formed"][0]
        self.assertIn("Permission denied", detail)
        self.assertIn("owner.fields_well_formed", failed_ids(report, 3))

    # -- identity (alterations made in the raw answer too, so only the rule under test can fail)
    def test_an_owner_that_is_not_alive(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(alive=False) if i == 15 else None)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.alive_every_sample"}, report, exact=True)

    def test_an_owner_that_restarts_inside_the_window(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(startTicks=a["owner"]["startTicks"] + 9) if i >= 15 else None)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.constant_in_window"}, report, exact=True)

    def test_a_pid_or_a_running_image_that_changes_inside_the_window_is_a_changed_owner(self):
        for label, hook in {"pid": lambda i, a: a["owner"].update(pid=a["owner"]["pid"] + 1) if i >= 15 else None,
                            "image": lambda i, a: a["owner"].update(exeSha256=OTHER_HASH) if i >= 15 else None}.items():
            with self.subTest(changed=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench, per_window=lambda meta, hook=hook: {"answer_hook": hook} if meta["index"] == 3 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("owner.constant_in_window", failed_ids(report, 3))

    def test_a_reused_pid_with_a_new_start_time_is_a_new_process(self):
        # every Bridge instance reports the same pid, but each started at a different time: five real restarts
        for n in range(1, 11):
            self.bench.alter(n, lambda d: d["meta"].update(bridgePid=111))
        self.collect()
        code, out, report = self.run_owner()
        self.assertEqual(code, 0, out)

    def test_a_new_pid_with_the_same_start_ticks_is_a_new_process(self):
        self.collect(ticks=4242)   # distinct pids per instance (the fixture's), identical start ticks
        code, out, report = self.run_owner()
        self.assertEqual(code, 0, out)

    def test_a_negative_request_latency_is_rejected(self):
        self.collect(per_window=lambda meta: {"latency": (lambda i: -5 if i == 10 else 25)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.latency_bounded", failed_ids(report, 3))

    def rewrite_raw(self, n, index, fn):
        entries = read_sidecar_lines(self.bench, n)
        victim = [e for e in entries if e["kind"] == "read"][index]
        victim["raw"] = fn(victim["raw"])
        write_sidecar_lines(self.bench, n, entries)

    def test_a_raw_answer_with_a_non_finite_token_is_not_a_usable_answer(self):
        self.collect()
        self.rewrite_raw(4, 10, lambda raw: raw.replace('"outbound_drops":0', '"outbound_drops":NaN', 1))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_entry_for_every_sample", failed_ids(report, 4))

    def test_a_raw_answer_that_says_it_failed_cannot_back_a_sample(self):
        self.collect()
        self.rewrite_raw(4, 10, lambda raw: raw.replace('"ok":true', '"ok":false', 1))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_entry_for_every_sample", failed_ids(report, 4))

    def test_a_sidecar_receive_time_that_differs_from_the_samples(self):
        self.collect()
        entries = read_sidecar_lines(self.bench, 4)
        [e for e in entries if e["kind"] == "read"][10]["receivedAtMs"] += 1
        write_sidecar_lines(self.bench, 4, entries)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 4))

    def test_an_owner_pid_that_is_not_the_windows_bridge(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(pid=999999))} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.pid_matches_meta"}, report, exact=True)

    def test_a_running_image_that_is_not_the_executable_the_window_names(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(exeSha256=OTHER_HASH))} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.exe_matches_meta"}, report, exact=True)

    def test_a_service_main_pid_that_differs_from_the_documents_pid(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(mainPid=1, mainPidMatches=False) if i == 7 else None)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"owner.service_main_pid"}, report, exact=True)

    def test_a_matching_service_main_pid_is_accepted(self):
        self.collect(per_window=lambda meta: {"answer_hook": (lambda i, a: a["owner"].update(mainPid=a["owner"]["pid"], mainPidMatches=True))})
        code, out, report = self.run_owner()
        self.assertEqual(code, 0, out)

    # -- timing
    def test_a_slow_collector_request(self):
        self.collect(per_window=lambda meta: {"latency": (lambda i: 2500 if i == 10 else 25)} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"collector.latency_bounded"}, report, exact=True)

    def test_a_collector_answer_that_arrives_long_after_its_page_sample_or_before_it(self):
        for offset in (4000, -5):
            with self.subTest(offset=offset):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench, per_window=lambda meta, offset=offset: {"recv_offset": (lambda i: offset if i == 10 else 30)} if meta["index"] == 3 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("collector.paired_with_page_sample", failed_ids(report, 3))

    def test_collector_sequence_numbers_that_repeat_or_go_back(self):
        self.collect()
        self.bench.alter(3, lambda d: d["samples"][6]["bridge"].update(collectorSeq=d["samples"][5]["bridge"]["collectorSeq"]))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.seq_strictly_increasing", failed_ids(report, 3))

    def test_a_pi_read_time_that_repeats_a_cached_answer(self):
        # the Pi's read time stands still for one interval (the same answer served twice)
        def stuck(i, s, seen={}):
            seen[i] = seen[8] if i == 9 else s["page"]["t"] + 5
            return seen[i]
        self.collect(per_window=lambda meta: {"pi_read": stuck} if meta["index"] == 3 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.only_window(3, {"collector.pi_read_time_advances"}, report, exact=True)

    def test_a_pi_clock_that_jumps_while_the_page_clock_does_not(self):
        bench = Bench(); self.addCleanup(bench.cleanup)
        collect_all(bench, per_window=lambda meta: {"pi_read": (lambda i, s: s["page"]["t"] + 5 + (60_000 if i >= 15 else 0))} if meta["index"] == 3 else {})
        code, out, report = bench.run("--require-owner")
        self.assertEqual(code, 1, out)
        self.assertIn("collector.pi_clock_tracks_window", failed_ids(report, 3))

    def test_the_pi_clock_may_differ_from_the_page_clock_by_a_constant(self):
        bench = Bench(); self.addCleanup(bench.cleanup)
        collect_all(bench, per_window=lambda meta: {"pi_read": (lambda i, s: s["page"]["t"] + 5 + 86_400_000)})
        code, out, report = bench.run("--require-owner")
        # the sample's updatedAtMs is on the page's clock in these windows, so the age is huge: that is a stale feed,
        # but the clock offset itself is not what is rejected
        self.assertNotIn("collector.pi_clock_tracks_window", failed_ids(report, 3))
        self.assertNotIn("collector.pi_read_time_advances", failed_ids(report, 3))

    def test_a_pi_age_that_is_not_read_time_minus_update_time(self):
        self.collect()
        self.bench.alter(3, lambda d: d["samples"][8]["bridge"].update(piSourceAgeMs=d["samples"][8]["bridge"]["piSourceAgeMs"] + 7))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.pi_age_arithmetic", failed_ids(report, 3))

    def test_a_pi_side_update_time_that_is_not_the_documents_own(self):
        self.collect()
        self.bench.alter(3, lambda d: d["samples"][8]["bridge"].update(piSourceUpdatedAtMs=d["samples"][8]["bridge"]["piSourceUpdatedAtMs"] + 1))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("bridge.pi_updated_matches_document", failed_ids(report, 3))

    # -- the retained raw answers
    def test_a_missing_sidecar(self):
        self.collect()
        os.remove(sidecar_path(self.bench, 4))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_retained", failed_ids(report, 4))

    def test_a_sidecar_that_is_not_strict_json_or_has_no_start_entry(self):
        cases = {
            "a line that is not JSON": lambda lines: lines + ["{ not json"],
            "a NaN token": lambda lines: lines + ['{"kind": "event", "x": NaN}'],
            "a line that is not an object": lambda lines: lines + ["[1, 2]"],
            "no start entry": lambda lines: lines[1:],
            "two start entries": lambda lines: [lines[0]] + lines,
            "a start entry with another schema": lambda lines: [lines[0].replace("saturn-collector-v1", "other")] + lines[1:],
        }
        for label, alter in cases.items():
            with self.subTest(case=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench)
                write_text(sidecar_path(bench, 4), "\n".join(alter(read_text(sidecar_path(bench, 4)).splitlines())) + "\n")
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("collector.sidecar_retained", failed_ids(report, 4))

    def test_a_sidecar_that_repeats_a_sequence_number(self):
        self.collect()
        entries = read_sidecar_lines(self.bench, 4)
        reads = [e for e in entries if e["kind"] == "read"]
        entries.append(dict(reads[-1]))
        write_sidecar_lines(self.bench, 4, entries)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_seq_unique", failed_ids(report, 4))

    def test_a_sidecar_with_a_missing_read(self):
        self.collect()
        entries = read_sidecar_lines(self.bench, 4)
        victim = [e for e in entries if e["kind"] == "read"][10]
        write_sidecar_lines(self.bench, 4, [e for e in entries if e is not victim])
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_entry_for_every_sample", failed_ids(report, 4))
        self.assertIn("collector.sidecar_complete", failed_ids(report, 4))

    def test_a_sidecar_read_that_failed_cannot_back_a_sample(self):
        self.collect()
        entries = read_sidecar_lines(self.bench, 4)
        victim = [e for e in entries if e["kind"] == "read"][10]
        victim["error"] = "no response within 2000 ms"
        write_sidecar_lines(self.bench, 4, entries)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_entry_for_every_sample", failed_ids(report, 4))

    def test_a_sidecar_whose_raw_answer_does_not_parse_cannot_back_a_sample(self):
        self.collect()
        entries = read_sidecar_lines(self.bench, 4)
        [e for e in entries if e["kind"] == "read"][10]["raw"] = "{ broken"
        write_sidecar_lines(self.bench, 4, entries)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_entry_for_every_sample", failed_ids(report, 4))

    def test_a_record_whose_collector_summary_disagrees_with_the_sidecar(self):
        self.collect()
        self.bench.alter(4, lambda d: d["collector"].update(reads=d["collector"]["reads"] + 1))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.sidecar_complete", failed_ids(report, 4))
        self.bench.alter(4, lambda d: (d["collector"].update(reads=d["collector"]["reads"] - 1), d["collector"].update(failures=3)))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.bench.alter(4, lambda d: d.pop("collector"))
        code, out, report = self.run_owner()
        self.assertIn("collector.sidecar_complete", failed_ids(report, 4))

    def test_a_sample_that_differs_from_its_raw_document(self):
        self.collect()
        for field, bad in (("iq", 0), ("outbound_drops", 7), ("exeSha256", OTHER_HASH), ("piSourceAgeMs", 1), ("nodelayConfirmedTotal", 9), ("requestLatencyMs", 24), ("buildGitSha", "zzz")):
            with self.subTest(field=field):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench, per_window=lambda meta, field=field, bad=bad: {"sample_hook": (lambda i, b: b.__setitem__(field, bad) if i == 12 else None)} if meta["index"] == 4 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 4))

    def test_a_sample_that_adds_a_field_the_raw_document_does_not_have(self):
        self.collect()
        self.bench.alter(4, lambda d: d["samples"][12]["bridge"].update(nodelayConfirmedTotal=5, nodelayEnabled=1, nodelayFailedTotal=0)
                         if False else d["samples"][12]["bridge"].update(documentSha256="a" * 64))
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 4))

    def test_a_sidecar_without_collector_fields_in_the_samples_is_contradictory(self):
        self.collect(skip=(4,))
        entries = [{"kind": "start", "schema": "saturn-collector-v1"}]
        write_sidecar_lines(self.bench, 4, entries)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("owner.fields_every_sample", failed_ids(report, 4))

    # -- set level
    def test_owner_evidence_in_only_some_windows(self):
        self.collect(skip=(6,))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("set.owner_identity_every_window", set_failures(report))

    def test_two_processes_serving_one_bridge_instance(self):
        self.collect(per_window=lambda meta: {"ticks": 777777} if meta["index"] == 6 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("set.owner_identity_per_bridge_instance", set_failures(report))

    def test_a_label_restart_that_did_not_start_a_new_process(self):
        # restart numbers advance, but every instance was served by the very same process (same pid, same start time)
        for n in range(1, 11):
            self.bench.alter(n, lambda d: d["meta"].update(bridgePid=111))
        self.collect(ticks=4242)
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("set.owner_new_process_per_restart", set_failures(report))

    def test_a_reboot_during_the_comparison(self):
        self.collect(per_window=lambda meta: {"boot": "0a0a0a0a-1111-2222-3333-444444444444"} if meta["index"] >= 9 else {})
        code, out, report = self.run_owner()
        self.assertEqual(code, 1, out)
        self.assertIn("set.owner_same_boot", set_failures(report))


class NodelayFromBridge(unittest.TestCase):
    """TCP_NODELAY as the Bridge itself reports it (setting, and sockets set and read back): sample fields first, logs second."""

    def setUp(self):
        self.bench = Bench()
        self.addCleanup(self.bench.cleanup)

    def nodelay_of(self, report, n):
        return [w for w in report["windows"] if w["n"] == n][0]["nodelay"]

    def test_bridge_reported_evidence_verifies_the_setting_without_any_retained_log(self):
        bench = Bench(logs=False); self.addCleanup(bench.cleanup)
        code, out, report = bench.run()
        self.assertEqual(code, 3, "without logs and without Bridge-reported fields it stays UNVERIFIED")
        collect_all(bench)
        code, out, report = bench.run()
        self.assertEqual(code, 0, out)
        self.assertTrue(all(w["nodelay"]["status"] == "VERIFIED" and w["nodelay"]["source"] == "Bridge-reported (perf.json)" for w in report["windows"]))
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED")

    def test_with_both_sources_the_bridge_reported_one_leads_and_the_logs_agree(self):
        collect_all(self.bench)
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.nodelay_of(report, 4)["source"], "Bridge-reported (perf.json)")
        self.assertEqual(self.nodelay_of(report, 4)["log_state"]["status"], "VERIFIED")

    def test_the_logs_alone_still_work_when_the_samples_carry_no_such_fields(self):
        code, out, report = self.bench.run()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.nodelay_of(report, 4)["source"], "retained Bridge logs")

    def alter_all(self, n, fn):
        collect_all(self.bench)
        self.bench.alter(n, lambda d: [fn(i, s["bridge"]) for i, s in enumerate(d["samples"])])
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        return report

    def test_a_bridge_that_reports_the_opposite_setting(self):
        report = self.alter_all(4, lambda i, b: b.update(nodelayEnabled=0, nodelayConfirmedTotal=0))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))
        self.assertEqual(self.nodelay_of(report, 4)["status"], "CONTRADICTED")

    def test_a_bridge_whose_setting_is_the_opposite_while_its_counters_look_right(self):
        # label 0 (window 2): the Bridge says the setting is ON though no socket was confirmed; label 1 (window 4): it says OFF with sockets confirmed
        report = self.alter_all(2, lambda i, b: b.update(nodelayEnabled=1))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 2))
        self.setUp()
        report = self.alter_all(4, lambda i, b: b.update(nodelayEnabled=0))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))

    def test_a_socket_whose_option_could_not_be_set_or_read_back(self):
        report = self.alter_all(4, lambda i, b: b.update(nodelayFailedTotal=1 if i >= 10 else 0))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))

    def test_a_confirmed_counter_that_goes_backwards(self):
        report = self.alter_all(4, lambda i, b: b.update(nodelayConfirmedTotal=b["nodelayConfirmedTotal"] + (5 if i < 15 else 0)))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))

    def test_no_confirmed_socket_for_a_window_served_with_the_setting_on(self):
        report = self.alter_all(4, lambda i, b: b.update(nodelayConfirmedTotal=0))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))

    def test_a_confirmed_socket_while_the_setting_is_off(self):
        report = self.alter_all(2, lambda i, b: b.update(nodelayConfirmedTotal=1))
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 2))

    def test_bridge_reported_fields_in_only_some_samples_or_not_counters(self):
        report = self.alter_all(4, lambda i, b: b.pop("nodelayFailedTotal") if i == 5 else None)
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))
        self.setUp()
        report = self.alter_all(4, lambda i, b: b.update(nodelayConfirmedTotal=-1) if i == 5 else None)
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))
        self.setUp()
        report = self.alter_all(4, lambda i, b: b.update(nodelayEnabled=True) if i == 5 else None)
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 4))

    def test_the_first_window_of_an_instance_needs_one_confirmed_socket_the_second_needs_two(self):
        # windows 5 and 6 are served by the same Bridge instance (setting on): the second must show two sockets by its first sample
        collect_all(self.bench)
        self.bench.alter(6, lambda d: [s["bridge"].update(nodelayConfirmedTotal=1) for s in d["samples"]])
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 6))

    def test_a_label_of_the_wrong_type_is_a_failed_check_not_a_crash(self):
        for label in ([1], {"on": 1}, "1", 2, None, True, 1.0):
            with self.subTest(label=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench)
                bench.alter(4, lambda d, label=label: d["meta"].update(bridgeNoDelay=label))
                code, out, report = bench.run()
                self.assertIsNotNone(report, out)
                self.assertEqual(code, 1, out)
                self.assertIn("record.meta_types", failed_ids(report, 4))

    def test_retained_logs_that_contradict_a_verifying_bridge(self):
        collect_all(self.bench)
        out_log = os.path.join(self.bench.logs, "bridge_3.out")
        write_text(out_log, read_text(out_log).replace("TCP_NODELAY on accepted TCI sockets: on", "TCP_NODELAY on accepted TCI sockets: off"))
        code, out, report = self.bench.run()
        self.assertEqual(code, 1, out)
        self.assertIn("nodelay.log_matches_label", failed_ids(report, 4))
        self.assertEqual(self.nodelay_of(report, 4)["status"], "CONTRADICTED")


class RealCollectorRun(unittest.TestCase):
    """Two windows taken on 2026-10-09 through the real collector (collector.mjs over owner_reader.py) against the replay Bridge on
    loopback: the contract test between the producer and this checker. No G2 was involved."""
    REAL = os.path.join(HERE, "fixtures", "collector-run-2026-10-09")

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="check-order-real-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.windows = os.path.join(self.root, "windows")
        shutil.copytree(os.path.join(self.REAL, "windows"), self.windows)

    def run_checker(self, *extra, logs=True):
        report = os.path.join(self.root, "report")
        cmd = [sys.executable, CHECKER, self.windows, "--mode", "single", "--require-owner", "--report-dir", report]
        if logs:
            cmd += ["--bridge-logs", os.path.join(self.REAL, "bridge-logs")]
        proc = subprocess.run(cmd + list(extra), capture_output=True, text=True, timeout=60)
        return proc.returncode, proc.stdout, read_json(os.path.join(report, "order_check.json"))

    def sidecar(self, name):
        with open(os.path.join(self.windows, name + ".collector.jsonl")) as fh:
            return [json.loads(line) for line in fh if line.strip()]

    def test_the_real_collector_output_is_valid_with_owner_identity_and_nodelay_verified(self):
        code, out, report = self.run_checker()
        self.assertEqual(code, 0, out)
        self.assertEqual([w["owner"]["status"] for w in report["windows"]], ["VERIFIED", "VERIFIED"])
        self.assertEqual([w["nodelay"]["source"] for w in report["windows"]], ["Bridge-reported (perf.json)"] * 2)
        self.assertEqual(report["verdict"], "VALID, TCP_NODELAY VERIFIED")

    def test_the_real_collector_output_satisfies_the_planned_rxc1_state_and_fpga_image(self):
        code, out, report = self.run_checker("--expect-rxc1", "disabled", "--expect-fpga-build", "53460003")
        self.assertEqual(code, 0, out)
        code, out, report = self.run_checker("--expect-rxc1", "valid")
        self.assertEqual(code, 1, out)
        self.assertIn("rxc1.state_is_the_planned_one", failed_ids(report, 1))
        code, out, report = self.run_checker("--expect-fpga-build", "53460004")
        self.assertEqual(code, 1, out)
        self.assertIn("owner.fpga_image_is_the_planned_one", failed_ids(report, 2))

    def test_the_bridges_own_evidence_needs_no_retained_log(self):
        code, out, report = self.run_checker(logs=False)
        self.assertEqual(code, 0, out)
        self.assertTrue(all(w["nodelay"]["log_state"]["status"] == "UNVERIFIED" and w["nodelay"]["status"] == "VERIFIED" for w in report["windows"]))

    def test_the_mirror_used_by_the_other_tests_agrees_with_the_real_producer(self):
        for name in ("w1_C", "w2_A-off"):
            w = read_json(os.path.join(self.windows, name + ".json"))
            reads = {e["seq"]: e for e in self.sidecar(name) if e["kind"] == "read"}
            for i, s in enumerate(w["samples"]):
                entry = reads[s["bridge"]["collectorSeq"]]
                answer = json.loads(entry["raw"])
                metrics = answer["document"]["metrics"]
                nodelay = (metrics["tci_nodelay_enabled"], metrics["tci_nodelay_confirmed_total"], metrics["tci_nodelay_failed_total"])
                self.assertEqual(mapped_bridge(s["bridge"], answer, entry, nodelay), s["bridge"], f"{name} sample {i}")
                # and the mirror's raw answer has the real answer's shape
                rx = metrics["rx_counter_v31"]
                ours = collector_answer(w["meta"], s["bridge"], answer["piReadAtMs"], answer["owner"]["bootId"], answer["owner"]["startTicks"], nodelay,
                                        (rx["status"], rx["host_acquisition_failures"]), (metrics["date_code_hex"], metrics["firmware_major"], metrics["firmware_minor"]))
                self.assertEqual(set(ours), set(answer))
                self.assertEqual(set(ours["owner"]), set(answer["owner"]))
                self.assertTrue(set(ours["document"]) <= set(answer["document"]))
                self.assertTrue(set(ours["document"]["metrics"]) <= set(metrics), set(ours["document"]["metrics"]) - set(metrics))
                self.assertTrue(all(metric in metrics for metric in MIRROR_METRICS.values()))

    def test_a_raw_document_edited_after_the_fact_no_longer_backs_its_sample(self):
        entries = self.sidecar("w2_A-off")
        victim = [e for e in entries if e["kind"] == "read"][10]
        answer = json.loads(victim["raw"])
        answer["document"]["metrics"]["outbound_drops"] += 5
        victim["raw"] = json.dumps(answer, separators=(",", ":"))
        write_text(os.path.join(self.windows, "w2_A-off.collector.jsonl"), "".join(json.dumps(e) + "\n" for e in entries))
        code, out, report = self.run_checker()
        self.assertEqual(code, 1, out)
        self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 2))

    def test_a_label_that_the_bridge_contradicts(self):
        w = read_json(os.path.join(self.windows, "w1_C.json"))
        w["meta"]["bridgeNoDelay"] = 0
        write_json(os.path.join(self.windows, "w1_C.json"), w)
        code, out, report = self.run_checker()
        self.assertEqual(code, 1, out)
        self.assertIn("nodelay.bridge_reported_matches_label", failed_ids(report, 1))

    def test_a_sample_whose_owner_is_another_process(self):
        w = read_json(os.path.join(self.windows, "w2_A-off.json"))
        w["meta"]["bridgePid"] += 1
        write_json(os.path.join(self.windows, "w2_A-off.json"), w)
        code, out, report = self.run_checker()
        self.assertEqual(code, 1, out)
        self.assertIn("owner.pid_matches_meta", failed_ids(report, 2))


class Rxc1FromTelemetry(unittest.TestCase):
    """With RXC1 polling ON the recorded state must be the one the owner itself publishes (metrics.rx_counter_v31.status) in every sample."""

    def setUp(self):
        self.bench = Bench()
        self.addCleanup(self.bench.cleanup)

    def prepare(self, state="valid", **kw):
        for path in window_files(self.bench.windows):
            w = read_json(path)
            w["meta"]["rxc1"] = {"state": state, "pid": w["meta"]["bridgePid"], "identity": "fw 1.31.002 / 0x53460004"}
            write_json(path, w)
        collect_all(self.bench, rxc1=(state, 0), **kw)

    def run_planned(self, expect="valid", *extra):
        return self.bench.run("--require-owner", "--expect-rxc1", expect, *extra)

    def test_recorded_state_that_the_owner_publishes_in_every_sample_is_valid(self):
        self.prepare()
        code, out, report = self.run_planned()
        self.assertEqual(code, 0, out)
        self.assertFalse(all_window_failures(report)); self.assertFalse(set_failures(report))

    def test_the_planned_state_is_enforced_on_every_window(self):
        self.prepare()
        code, out, report = self.run_planned("unarmed")
        self.assertEqual(code, 1, out)
        self.assertTrue(all("rxc1.state_is_the_planned_one" in ids for ids in all_window_failures(report).values()))
        self.assertEqual(len(all_window_failures(report)), 10)

    def test_expecting_a_state_implies_that_each_window_records_one(self):
        code, out, report = self.bench.run("--expect-rxc1", "valid")        # the fixture records the string "not applicable (replay)"
        self.assertEqual(code, 1, out)
        self.assertIn("rxc1.recorded", failed_ids(report, 4))

    def test_a_window_whose_owner_says_something_else_than_was_recorded(self):
        self.prepare(per_window=lambda meta: {"rxc1": ("unarmed", 0)} if meta["index"] == 4 else {})
        code, out, report = self.run_planned()
        self.assertEqual(code, 1, out)
        self.assertEqual(failed_ids(report, 4), {"rxc1.status_every_sample_matches_state"})

    def test_polling_that_drops_out_in_the_middle_of_a_window(self):
        hook = lambda i, a: a["document"]["metrics"]["rx_counter_v31"].update(status="unavailable") if i >= 15 else None
        self.prepare(per_window=lambda meta: {"answer_hook": hook} if meta["index"] == 4 else {})
        code, out, report = self.run_planned()
        self.assertEqual(code, 1, out)
        self.assertEqual(failed_ids(report, 4), {"rxc1.status_every_sample_matches_state"})

    def test_a_bridge_that_publishes_no_rxc1_state_cannot_back_a_recorded_one(self):
        self.prepare(per_window=lambda meta: {"rxc1": None} if meta["index"] == 4 else {})
        code, out, report = self.run_planned()
        self.assertEqual(code, 1, out)
        self.assertEqual(failed_ids(report, 4), {"rxc1.status_every_sample_matches_state"})
        detail = [c["detail"] for w in report["windows"] if w["n"] == 4 for c in w["checks"] if c["check"] == "rxc1.status_every_sample_matches_state"][0]
        self.assertIn("cannot be verified", detail)

    def test_the_state_present_in_only_some_samples(self):
        hook = lambda i, a: a["document"]["metrics"].pop("rx_counter_v31") if i == 5 else None
        self.prepare(per_window=lambda meta: {"answer_hook": hook} if meta["index"] == 4 else {})
        code, out, report = self.run_planned()
        self.assertEqual(code, 1, out)
        self.assertIn("rxc1.telemetry_all_or_none", failed_ids(report, 4))

    def test_a_status_that_is_not_text_or_a_failure_count_that_is_not_a_counter(self):
        for label, hook in {"status": lambda i, a: a["document"]["metrics"]["rx_counter_v31"].update(status=5) if i == 5 else None,
                            "failures": lambda i, a: a["document"]["metrics"]["rx_counter_v31"].update(host_acquisition_failures=-1) if i == 5 else None}.items():
            with self.subTest(field=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                for path in window_files(bench.windows):
                    w = read_json(path); w["meta"]["rxc1"] = {"state": "valid", "pid": w["meta"]["bridgePid"], "identity": "x"}; write_json(path, w)
                collect_all(bench, rxc1=("valid", 0), per_window=lambda meta, hook=hook: {"answer_hook": hook} if meta["index"] == 4 else {})
                code, out, report = bench.run("--require-owner", "--expect-rxc1", "valid")
                self.assertEqual(code, 1, out)
                self.assertIn("owner.fields_well_formed", failed_ids(report, 4))

    def test_a_sample_that_differs_from_its_raw_answer_in_the_rxc1_fields(self):
        for field, bad in (("rxc1Status", "disabled"), ("rxc1HostAcquisitionFailures", 9)):
            with self.subTest(field=field):
                bench = Bench(); self.addCleanup(bench.cleanup)
                for path in window_files(bench.windows):
                    w = read_json(path); w["meta"]["rxc1"] = {"state": "valid", "pid": w["meta"]["bridgePid"], "identity": "x"}; write_json(path, w)
                collect_all(bench, rxc1=("valid", 0), per_window=lambda meta, field=field, bad=bad: {"sample_hook": (lambda i, b: b.__setitem__(field, bad) if i == 12 else None)} if meta["index"] == 4 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 4))

    def test_states_that_differ_between_windows_are_a_set_level_failure(self):
        self.prepare(per_window=lambda meta: {"rxc1": ("partial", 0)} if meta["index"] == 6 else {})
        for path in window_files(self.bench.windows):
            w = read_json(path)
            if w["meta"]["index"] == 6:
                w["meta"]["rxc1"]["state"] = "partial"
                write_json(path, w)
        code, out, report = self.bench.run("--require-owner", "--require-rxc1")
        self.assertEqual(code, 1, out)
        self.assertIn("set.rxc1_state_constant", set_failures(report))
        self.assertFalse(all_window_failures(report), "each window is internally consistent; only the set disagrees")


class FpgaImage(unittest.TestCase):
    """The FPGA image the owner reports (build id and firmware version) is part of what a window ran on."""

    def setUp(self):
        self.bench = Bench()
        self.addCleanup(self.bench.cleanup)

    def test_the_planned_image_is_accepted_and_another_is_rejected_on_every_window(self):
        collect_all(self.bench)
        code, out, report = self.bench.run("--require-owner", "--expect-fpga-build", "53460003")
        self.assertEqual(code, 0, out)
        code, out, report = self.bench.run("--require-owner", "--expect-fpga-build", "53460004")
        self.assertEqual(code, 1, out)
        self.assertEqual(len(all_window_failures(report)), 10)
        self.assertTrue(all(ids == {"owner.fpga_image_is_the_planned_one"} for ids in all_window_failures(report).values()))

    def test_an_image_that_changes_inside_a_window(self):
        hook = lambda i, a: a["document"]["metrics"].update(date_code_hex="53460004") if i >= 15 else None
        collect_all(self.bench, per_window=lambda meta: {"answer_hook": hook} if meta["index"] == 4 else {})
        code, out, report = self.bench.run("--require-owner")
        self.assertEqual(code, 1, out)
        self.assertEqual(failed_ids(report, 4), {"owner.fpga_image_constant_in_window"})

    def test_the_planned_image_must_hold_for_every_sample_not_just_some(self):
        hook = lambda i, a: a["document"]["metrics"].update(date_code_hex="53460004") if i >= 15 else None
        collect_all(self.bench, per_window=lambda meta: {"answer_hook": hook} if meta["index"] == 4 else {})
        code, out, report = self.bench.run("--require-owner", "--expect-fpga-build", "53460003")
        self.assertEqual(code, 1, out)
        self.assertIn("owner.fpga_image_is_the_planned_one", failed_ids(report, 4))

    def test_a_firmware_version_that_changes_inside_a_window(self):
        hook = lambda i, a: a["document"]["metrics"].update(firmware_minor=32) if i == 7 else None
        collect_all(self.bench, per_window=lambda meta: {"answer_hook": hook} if meta["index"] == 4 else {})
        code, out, report = self.bench.run("--require-owner")
        self.assertEqual(code, 1, out)
        self.assertIn("owner.fpga_image_constant_in_window", failed_ids(report, 4))

    def test_a_different_image_in_one_window_of_the_set(self):
        collect_all(self.bench, per_window=lambda meta: {"fpga": ("53460004", 1, 31)} if meta["index"] == 6 else {})
        code, out, report = self.bench.run("--require-owner")
        self.assertEqual(code, 1, out)
        self.assertIn("set.fpga_image_same_in_every_window", set_failures(report))
        self.assertFalse(all_window_failures(report), "each window is internally consistent; only the set disagrees")

    def test_a_malformed_or_missing_image_identity(self):
        cases = {"empty build id": lambda i, a: a["document"]["metrics"].update(date_code_hex="") if i == 5 else None,
                 "negative firmware": lambda i, a: a["document"]["metrics"].update(firmware_major=-1) if i == 5 else None,
                 "build id not text": lambda i, a: a["document"]["metrics"].update(date_code_hex=53460003) if i == 5 else None}
        for label, hook in cases.items():
            with self.subTest(case=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench, per_window=lambda meta, hook=hook: {"answer_hook": hook} if meta["index"] == 4 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("owner.fields_well_formed", failed_ids(report, 4))
        bench = Bench(); self.addCleanup(bench.cleanup)
        collect_all(bench)
        bench.alter(4, lambda d: d["samples"][5]["bridge"].pop("fpgaBuildId"))
        code, out, report = bench.run("--require-owner")
        self.assertEqual(code, 1, out)
        self.assertIn("owner.fields_every_sample", failed_ids(report, 4))

    def test_a_sample_whose_image_differs_from_its_raw_answer(self):
        for field, bad in (("fpgaBuildId", "53460004"), ("firmwareMajor", 2), ("firmwareMinor", 99)):
            with self.subTest(field=field):
                bench = Bench(); self.addCleanup(bench.cleanup)
                collect_all(bench, per_window=lambda meta, field=field, bad=bad: {"sample_hook": (lambda i, b: b.__setitem__(field, bad) if i == 12 else None)} if meta["index"] == 4 else {})
                code, out, report = bench.run("--require-owner")
                self.assertEqual(code, 1, out)
                self.assertIn("collector.samples_match_raw_documents", failed_ids(report, 4))


class MalformedRecords(Base):
    """A record the checker cannot use is an INVALID window with a reason, and the report is still written: never a crash."""

    def test_wrong_types_in_the_windows_identity_are_failed_checks(self):
        cases = {"bridgeNoDelay": [[1], {"a": 1}, "1", 2], "bridgeRestartNumber": [[3], "3", 2.5, None], "bridgePid": [[1], "7", 1.5],
                 "arm": [["C"], 4, None], "index": ["4", [4], 4.5]}
        for key, values in cases.items():
            for value in values:
                with self.subTest(key=key, value=value):
                    bench = Bench(); self.addCleanup(bench.cleanup)
                    bench.alter(4, lambda d, key=key, value=value: d["meta"].update({key: value}))
                    code, out, report = bench.run()
                    self.assertIsNotNone(report, out)
                    self.assertEqual(code, 1, out)
                    self.assertTrue(any(w["n"] == 4 for w in report["windows"]))
                    self.assertIn("record.meta_types", failed_ids(report, 4))

    def test_a_sha_that_is_not_even_hashable_is_a_failed_check(self):
        self.bench.alter(4, lambda d: d["meta"].update(bridgeSha256=["x"]))
        code, out, report = self.bench.run()
        self.assertIsNotNone(report, out)
        self.assertEqual(code, 1, out)
        self.assertIn("record.bridge_sha256_valid", failed_ids(report, 4))
        self.assertIn("set.one_bridge_binary", set_failures(report))

    def test_a_record_with_an_uninterpretable_structure_is_invalid_with_a_reason(self):
        for label, alter in {"url is not a string": lambda d: d.update(url=12345), "frames hold a non-object": lambda d: d.update(frames=[5, "x"])}.items():
            with self.subTest(case=label):
                bench = Bench(); self.addCleanup(bench.cleanup)
                bench.alter(1, alter)
                code, out, report = bench.run()
                self.assertIsNotNone(report, out)
                self.assertEqual(code, 1, out)
                self.assertIn("record.interpretable", failed_ids(report, 1))
                detail = [c["detail"] for c in [w for w in report["windows"] if w["n"] == 1][0]["checks"] if c["check"] == "record.interpretable"][0]
                self.assertIn("could not interpret", detail)
                self.assertEqual(len(report["windows"]), 10, "the other windows are still checked and reported")


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
