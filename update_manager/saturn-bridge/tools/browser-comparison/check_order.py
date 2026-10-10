#!/usr/bin/env python3
"""Fail-closed checker for browser-comparison measurement windows.

Reads the per-window records written by window.mjs (w<N>_<ARM>.json) and decides two
separate things for every window:

  EVIDENCE   Is this window a valid measurement of its arm? Telemetry present, fresh and
             sane at every sample; the browser stayed connected; the codec, rate and
             channels did not change; delivery matched the arm at every interval; counters
             never went backwards; the window had the planned length; and the Bridge's
             TCP_NODELAY state, checked against the retained Bridge logs, matches the label.
  PERFORMANCE  What the transport and the audio did: throughput, display stalls, slow or stopped
             delivery, underruns, overflows, drops, sequence gaps, queue lead. Reported, never used
             to reject a window. A display pause, a slow stream or a window full of underruns is
             exactly what is being measured, so once the subscription, the selected mode and the
             start are established it is a RESULT. Only a wrong transport, a failed setup or missing
             measurement evidence makes a window invalid. A transport that disconnects is kept and
             reported as its own outcome (it cannot support a matched comparison).

Nothing is averaged over what is left after discarding bad samples: a window with any invalid
sample is INVALID, and its record is kept exactly as it was (this tool only reads the input).

Exit status
  0  every window is valid and the TCP_NODELAY state of every window is VERIFIED
  1  invalid evidence: a failed window, a failed set-level rule, or a contradicted label
  2  usage error or unreadable input
  3  every window is valid but TCP_NODELAY (neither Bridge-reported evidence nor retained logs) or, in the live profile,
     telemetry freshness (no Pi-side source age) or the owner's process identity (no collector evidence) is UNVERIFIED
     for at least one window; 0 instead if --allow-unverified-nodelay / --allow-unverified-freshness /
     --allow-unverified-owner is given (the report still says UNVERIFIED)

Limits are fixed here, before any live window is evaluated, in PROFILES. `rehearsal` is the
30-second local rehearsal; `live` is the planned ten-minute window. Nothing is tuned per run.
"""
import argparse
import glob
import json
import math
import os
import re
import statistics
import sys

PROFILES = {
    "rehearsal": dict(duration_s=30, span_low=-3.0, span_high=2.0, min_samples=28, same_clock=True, require_rxc1=False, require_owner=False),
    "live": dict(duration_s=600, span_low=-5.0, span_high=5.0, min_samples=595, same_clock=False, require_rxc1=True, require_owner=True),
}
# Limits common to both profiles.
INTERVAL_MIN_S = 0.5            # sampling is nominally 1 Hz
INTERVAL_MAX_S = 2.0
HOLE_S = 3.0                    # any gap longer than this is a hole in the record
MAX_UNCHANGED_BRIDGE = 2        # at most this many consecutive samples with an identical Bridge timestamp
BRIDGE_AGE_MAX_MS = 3000        # same-clock profiles only: sample time minus the Bridge's update time
BRIDGE_AGE_MIN_MS = -1000
PI_AGE_MIN_MS = -50             # live profile: age computed on the Pi (read time minus the document's own update time)
COLLECTOR_LATENCY_MAX_MS = 2000  # one collector request, as bounded by the client (a slower one is a failure record anyway)
COLLECTOR_PAIR_MAX_MS = 3000    # the collector's receive time minus the page sample's time: both on the browser's computer
PI_CLOCK_TRACK_MS = 3000        # the Pi's read-time span vs the page's span over a window: durations only, the clocks are never compared
DISPLAY_FPS = (24.0, 36.0)      # expected display frames or rows per second (cap is 30): only a performance flag
AUDIO_NOMINAL_FPS = 46.9        # 1024-sample frames at 48 kHz
AUDIO_SLOW_FRACTION = 0.8       # below this share of nominal, audio is flagged slow: only a performance flag
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
C_DRAIN_MIN_MS = 3000           # arm C: IQ stopped at least this long before the first sample

ORDER = ["C", "A-off", "B-off", "B-on", "A-on", "A-on", "B-on", "B-off", "A-off", "C"]
ARMS = {
    "A-off": dict(kind="A", nodelay=0, override="iq", mode="normal"),
    "A-on": dict(kind="A", nodelay=1, override="iq", mode="normal"),
    "B-off": dict(kind="B", nodelay=0, override="spectrum", mode="normal"),
    "B-on": dict(kind="B", nodelay=1, override="spectrum", mode="normal"),
    "C": dict(kind="C", nodelay=1, override="iq", mode="c"),   # C is defined with TCP_NODELAY on
}

PAGE_NUM = ["t", "iq", "rxIq", "rows", "opusFrames", "pcmFrames", "audioPlayed", "lastAudioSeq", "audioGaps",
            "audioResyncs", "decodeErrors", "lateDrops", "underruns", "overflows", "drops", "rate", "channels", "queuedMs"]
PAGE_BOOL = ["connected", "iqStreaming"]
PAGE_STR = ["codec", "renderSource", "echoMode", "worklet", "override"]
PAGE_OPTIONAL_NUM = ["jitterP50", "jitterP95", "jitterP99"]   # may legitimately be null early on
# Cumulative counters. NOT outbound_drops: the Bridge clears it (tci/mod.rs: drop_count.swap(0)) every time it reports rx_drops to its client, so the
# published value is "drops not yet reported", which legitimately goes 1, 1, 0. The *_s fields are rates and are not monotonic either.
BRIDGE_CUMULATIVE = ["rows_written"]
PAGE_MONOTONIC = ["iq", "rxIq", "rows", "opusFrames", "pcmFrames", "audioPlayed", "lastAudioSeq", "audioGaps",
                  "audioResyncs", "decodeErrors", "lateDrops", "underruns", "overflows", "drops"]
BRIDGE_NUM = ["updatedAtMs", "iq", "audio", "connections", "iq_tci_frames_s", "rx_audio_frames_s", "rows_written",
              "spectrum_clients", "audio_dropped_s", "tcp_outq_hwm_bytes", "out_hwm_bytes", "outbound_drops"]
# The collector contract v1 (draft): what a sample's `bridge` carries when it was read through collector.mjs / owner_reader.py.
OWNER_INT_MIN = {"ownerPid": 1, "ownerStartTicks": 0, "collectorSeq": 1, "collectorSpawn": 1}
OWNER_NUM = ["piReadAtMs", "piSourceUpdatedAtMs", "collectorReceivedAtMs", "requestLatencyMs"]
OWNER_HASH = ["exeSha256", "documentSha256"]
OWNER_KEYS = list(OWNER_INT_MIN) + OWNER_NUM + OWNER_HASH + ["ownerAlive", "bootId", "fpgaBuildId", "firmwareMajor", "firmwareMinor"]
NODELAY_KEYS = ["nodelayEnabled", "nodelayConfirmedTotal", "nodelayFailedTotal"]
NODELAY_MAP = {"nodelayEnabled": "tci_nodelay_enabled", "nodelayConfirmedTotal": "tci_nodelay_confirmed_total", "nodelayFailedTotal": "tci_nodelay_failed_total"}
# sample.bridge field -> perf.json metric: the collector's mapping, verified against the raw documents in the sidecar
METRIC_MAP = {"iq": "iq", "audio": "audio", "connections": "connections", "iq_tci_frames_s": "iq_tci_frames_s", "rx_audio_frames_s": "rx_audio_frames_s",
              "rows_written": "display_spectrum_rows_written", "spectrum_clients": "display_spectrum_clients", "audio_dropped_s": "audio_dropped_s",
              "tcp_outq_hwm_bytes": "tcp_outq_hwm_bytes", "out_hwm_bytes": "out_hwm_bytes", "outbound_drops": "outbound_drops", "buildGitSha": "build_git_sha"}
FPGA_MAP = {"fpgaBuildId": "date_code_hex", "firmwareMajor": "firmware_major", "firmwareMinor": "firmware_minor"}   # the FPGA image the owner reports
SIDECAR_SCHEMA = "saturn-collector-v1"
META_KEYS = ["index", "arm", "bridgeNoDelay", "bridgePid", "bridgeSha256", "bridgeRestartNumber"]
TOP_KEYS = ["meta", "mode", "url", "atConnect", "atWarm", "start", "samples", "frames"]


def is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def is_num(v):
    """A real, finite number. NaN and infinity (including 1e999) are not measurements."""
    return isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v)


def meta_problems(meta):
    """Type problems in a window's identity record (the keys that are present). Empty when it is usable for grouping and comparison."""
    if not isinstance(meta, dict):
        return ["meta is not an object"]
    bad = [k for k in ("index", "bridgePid", "bridgeRestartNumber") if k in meta and not is_int(meta[k])]
    bad += [k for k in ("arm",) if k in meta and not isinstance(meta[k], str)]
    if "bridgeNoDelay" in meta and not (is_int(meta["bridgeNoDelay"]) and meta["bridgeNoDelay"] in (0, 1)):
        bad.append("bridgeNoDelay")
    return bad


def hashable(v):
    """A hashable stand-in for any JSON value, so a malformed record is a failed check and never a crash."""
    return v if v is None or isinstance(v, (str, int, float, bool)) else json.dumps(v, sort_keys=True, default=str)


def non_finite_paths(obj, path="$", limit=5):
    """Paths of NaN/infinite floats anywhere in a record."""
    found = []

    def walk(o, p):
        if len(found) >= limit:
            return
        if isinstance(o, float) and not math.isfinite(o):
            found.append(p)
        elif isinstance(o, dict):
            for k, v in o.items():
                walk(v, f"{p}.{k}")
        elif isinstance(o, list):
            for i, v in enumerate(o):
                walk(v, f"{p}[{i}]")

    walk(obj, path)
    return found


def _reject_constant(name):
    raise ValueError(f"non-finite JSON constant {name} is not a measurement")


class Checks:
    """The checks of one window or of the set, each with an id, a verdict and a detail."""

    def __init__(self):
        self.items = []

    def add(self, check_id, ok, detail=""):
        self.items.append({"check": check_id, "ok": bool(ok), "detail": detail})
        return bool(ok)

    def failed(self):
        return [c for c in self.items if not c["ok"]]


def indices(xs, limit=8):
    xs = list(xs)
    return str(xs[:limit]) + (f" (+{len(xs) - limit} more)" if len(xs) > limit else "")


def validate_sample(s):
    """Problems with one sample's structure and types. Returns a list of strings."""
    problems = []
    page, bridge = s.get("page") if isinstance(s, dict) else None, s.get("bridge") if isinstance(s, dict) else None
    if not isinstance(page, dict):
        return ["page record missing"]
    if not isinstance(bridge, dict):
        return ["bridge record missing"]
    if "error" in bridge:
        problems.append(f"bridge telemetry read failed: {bridge.get('error')}")
    for k in PAGE_NUM:
        if not is_num(page.get(k)):
            problems.append(f"page.{k} missing or not a number")
    for k in PAGE_BOOL:
        if not isinstance(page.get(k), bool):
            problems.append(f"page.{k} missing or not a boolean")
    for k in PAGE_STR:
        if not isinstance(page.get(k), str):
            problems.append(f"page.{k} missing or not a string")
    for k in PAGE_OPTIONAL_NUM:
        if k not in page or not (page[k] is None or is_num(page[k])):
            problems.append(f"page.{k} missing")
    if "error" not in bridge:
        for k in BRIDGE_NUM:
            if not is_num(bridge.get(k)):
                problems.append(f"bridge.{k} missing or not a number")
        if "piSourceAgeMs" in bridge and not is_num(bridge["piSourceAgeMs"]):
            problems.append("bridge.piSourceAgeMs is not a finite number")
    return problems


def check_window(w, prof, nodelay_state, sidecar=None):
    """All checks of one window. Returns (Checks, performance, startup counters, delivery, info), where info holds
    the window's reported outcomes (for example a disconnect) and the status of its freshness and owner evidence."""
    r = Checks()
    perf, startup, delivery = {}, {}, {}
    info = {"outcomes": [], "freshness": {"status": "UNVERIFIED", "why": "not evaluated"},
            "owner": {"status": "UNVERIFIED" if prof.get("require_owner") else "NOT COLLECTED", "why": "not evaluated"}, "owner_ident": None, "fpga_ident": None}
    # A record whose driver failed carries a `failure`: whatever it recorded is kept, but it is never a measurement.
    r.add("record.no_failure", "failure" not in w, f"the window driver failed: {str(w.get('failure', {}).get('message', ''))[:200]}" if "failure" in w else "")
    bad_numbers = non_finite_paths(w)
    r.add("record.finite_numbers", not bad_numbers, f"non-finite numbers at {bad_numbers}" if bad_numbers else "")
    missing = [k for k in TOP_KEYS if k not in w]
    if not r.add("record.top_level_fields", not missing, f"missing: {missing}" if missing else ""):
        return r, perf, startup, delivery, info
    meta = w["meta"]
    missing_meta = [k for k in META_KEYS if k not in meta]
    if not r.add("record.meta_fields", not missing_meta, f"missing: {missing_meta}" if missing_meta else ""):
        return r, perf, startup, delivery, info
    bad_types = meta_problems(meta)
    if not r.add("record.meta_types", not bad_types, f"wrong types in meta: {bad_types}" if bad_types else ""):
        return r, perf, startup, delivery, info
    r.add("record.bridge_sha256_valid", isinstance(meta["bridgeSha256"], str) and SHA256_RE.match(meta["bridgeSha256"]) is not None,
          f"bridgeSha256 {meta['bridgeSha256']!r} is not a 64-hex SHA-256")
    arm = ARMS.get(meta["arm"])
    if not r.add("record.known_arm", arm is not None, f"arm {meta['arm']!r}"):
        return r, perf, startup, delivery, info
    if prof.get("require_rxc1"):
        # The acquisition owner's record of the actual RXC1 polling state for this window: not the environment flag.
        rx = meta.get("rxc1")
        have = isinstance(rx, dict) and all(rx.get(k) not in (None, "", {}, []) for k in ("state", "pid", "identity"))
        r.add("rxc1.recorded", have, "meta.rxc1 must be an object with non-empty state, pid and identity" if not have else "")
        if prof.get("expect_rxc1") is not None:
            r.add("rxc1.state_is_the_planned_one", have and rx["state"] == prof["expect_rxc1"],
                  f"RXC1 state {rx.get('state') if isinstance(rx, dict) else None!r}, planned {prof['expect_rxc1']!r}")
    samples = w["samples"]
    if not r.add("record.samples_present", isinstance(samples, list) and len(samples) >= 2,
                 f"{len(samples) if isinstance(samples, list) else 'no'} samples"):
        return r, perf, startup, delivery, info

    # --- every sample, structurally
    bad = {}
    for i, s in enumerate(samples):
        for p in validate_sample(s):
            bad.setdefault(i, []).append(p)
    r.add("telemetry.every_sample_complete", not bad,
          "; ".join(f"sample {i}: {ps[0]}" for i, ps in list(bad.items())[:5]) + (f" (+{len(bad) - 5} more samples)" if len(bad) > 5 else ""))
    if bad:
        # Keep checking what can still be checked: use only structurally sound samples for numeric rules,
        # but the window is already invalid and stays so.
        pass
    good = [(i, s) for i, s in enumerate(samples) if i not in bad]
    if len(good) < 2:
        return r, perf, startup, delivery, info
    pages = [(i, s["page"]) for i, s in good]
    bridges = [(i, s["bridge"]) for i, s in good]

    # --- cadence, span, length (over every sample that carries a timestamp, sound or not)
    stamped = [(i, s["page"]["t"]) for i, s in enumerate(samples)
               if isinstance(s, dict) and isinstance(s.get("page"), dict) and is_num(s["page"].get("t"))]
    ts = [t_ for _, t_ in stamped]
    gaps = [(stamped[k + 1][0], (ts[k + 1] - ts[k]) / 1000.0) for k in range(len(ts) - 1)]
    out_of_band = [i for i, g in gaps if not (INTERVAL_MIN_S <= g <= INTERVAL_MAX_S)]
    holes = [i for i, g in gaps if g > HOLE_S]
    r.add("cadence.no_holes", not holes, f"gaps over {HOLE_S}s before samples {indices(holes)}")
    r.add("cadence.interval_in_band", not out_of_band,
          f"intervals outside {INTERVAL_MIN_S}-{INTERVAL_MAX_S}s before samples {indices(out_of_band)}")
    r.add("cadence.timestamps_increase", all(b > a for a, b in zip(ts, ts[1:])), "page timestamps do not strictly increase")
    span = (ts[-1] - ts[0]) / 1000.0
    lo, hi = prof["duration_s"] + prof["span_low"], prof["duration_s"] + prof["span_high"]
    r.add("window.span", lo <= span <= hi, f"span {span:.1f}s, required {lo:.0f}-{hi:.0f}s for a {prof['duration_s']}s window")
    r.add("window.sample_count", len(samples) >= prof["min_samples"], f"{len(samples)} samples, required at least {prof['min_samples']}")

    # --- Bridge telemetry freshness, per sample
    ups = [b["updatedAtMs"] for _, b in bridges if "updatedAtMs" in b]
    if len(ups) == len(bridges):
        r.add("bridge.timestamp_never_goes_back", all(b >= a for a, b in zip(ups, ups[1:])), "Bridge updatedAtMs decreased")
        run = longest = 0
        for a, b in zip(ups, ups[1:]):
            run = run + 1 if a == b else 0
            longest = max(longest, run)
        r.add("bridge.fresh_every_sample", longest <= MAX_UNCHANGED_BRIDGE,
              f"Bridge timestamp unchanged for {longest} consecutive samples (limit {MAX_UNCHANGED_BRIDGE})")
        up_span = (ups[-1] - ups[0]) / 1000.0
        r.add("bridge.timestamp_advances_with_window", up_span >= 0.9 * span, f"Bridge timestamp advanced {up_span:.1f}s over a {span:.1f}s window")
        if prof["same_clock"]:
            ages = [(i, p["t"] - b["updatedAtMs"]) for (i, p), (_, b) in zip(pages, bridges)]
            old = [i for i, a in ages if not (BRIDGE_AGE_MIN_MS <= a <= BRIDGE_AGE_MAX_MS)]
            r.add("bridge.age_every_sample", not old, f"Bridge telemetry age outside {BRIDGE_AGE_MIN_MS}..{BRIDGE_AGE_MAX_MS} ms at samples {indices(old)}")
            info["freshness"] = {"status": "VERIFIED", "why": "one clock: sample time minus the Bridge's update time"}
        else:
            # Two computers: advancing timestamps cannot prove a feed fresh (a clock offset and an old replay look the same).
            # Only an age computed on the Pi, from its own clock and the cached document's own update time, can.
            pi_ages = [b.get("piSourceAgeMs") for _, b in bridges]
            have = [a is not None for a in pi_ages]
            if all(have):
                stale = [i for (i, _), a in zip(bridges, pi_ages) if not (PI_AGE_MIN_MS <= a <= BRIDGE_AGE_MAX_MS)]
                r.add("bridge.pi_source_age_every_sample", not stale, f"Pi-side source age outside {PI_AGE_MIN_MS}..{BRIDGE_AGE_MAX_MS} ms at samples {indices(stale)}")
                info["freshness"] = {"status": "VERIFIED" if not stale else "CONTRADICTED",
                                     "why": f"Pi-side source age {min(pi_ages):.0f}..{max(pi_ages):.0f} ms" if not stale else f"stale at samples {indices(stale)}"}
            elif any(have):
                r.add("bridge.pi_source_age_every_sample", False, "Pi-side source age present in only some samples")
                info["freshness"] = {"status": "CONTRADICTED", "why": "Pi-side source age present in only some samples"}
            else:
                info["freshness"] = {"status": "UNVERIFIED", "why": "no Pi-side source-age evidence (bridge.piSourceAgeMs): advancing timestamps alone cannot prove freshness across two computers"}

    # --- the owner's process identity, the collector's timing and the retained raw answers
    check_owner(r, info, w, prof, pages, bridges, sidecar)

    # --- browser connection and the audio format, per sample
    gone = [i for i, p in pages if not p["connected"]]
    r.add("page.connected_every_sample", not gone, f"disconnected at samples {indices(gone)}")
    if gone:
        info["outcomes"].append(f"transport disconnected at samples {indices(gone)}: kept and reported, not usable for a matched comparison")
    expect_codec = prof.get("expect_codec", "opus")
    r.add("audio.codec_every_sample", all(p["codec"] == expect_codec for _, p in pages),
          f"wire codec not {expect_codec!r} at samples {indices(i for i, p in pages if p['codec'] != expect_codec)}")
    for key in ("rate", "channels", "worklet"):
        vals = {p[key] for _, p in pages}
        r.add(f"audio.{key}_constant", len(vals) == 1, f"{key} took values {sorted(map(str, vals))}")
    r.add("page.override_matches_arm", all(p["override"] == arm["override"] for _, p in pages),
          f"display override not {arm['override']!r} at samples {indices(i for i, p in pages if p['override'] != arm['override'])}")

    # --- counters never go backwards (a reset or reload invalidates the window)
    for key in PAGE_MONOTONIC:
        series = [p[key] for _, p in pages]
        back = [pages[k + 1][0] for k in range(len(series) - 1) if series[k + 1] < series[k]]
        r.add(f"counters.{key}_monotonic", not back, f"{key} decreased before samples {indices(back)}")
    for key in BRIDGE_CUMULATIVE:
        series = [b[key] for _, b in bridges if key in b]
        if len(series) == len(bridges):
            back = [k + 1 for k in range(len(series) - 1) if series[k + 1] < series[k]]
            r.add(f"counters.bridge_{key}_monotonic", not back, f"Bridge {key} decreased before samples {indices(back)}")
    rw = [b["rows_written"] for _, b in bridges if "rows_written" in b]

    # --- the transport is the arm's: the right mode and subscriptions at every sample, and nothing of the wrong kind.
    # How much of the right kind arrived (throughput, stalls) is PERFORMANCE and is reported below, never judged here.
    def deltas(key):
        return [pages[k + 1][1][key] - pages[k][1][key] for k in range(len(pages) - 1)]

    secs = span if span > 0 else 1.0
    d_rx, d_iq, d_rows = deltas("rxIq"), deltas("iq"), deltas("rows")
    kind = arm["kind"]
    display_deltas = None
    if kind == "A":
        r.add("delivery.render_source", all(p["renderSource"] == "iq" and p["echoMode"] == "iq" for _, p in pages), "render source/echo not iq")
        display_deltas = d_rx
        r.add("delivery.no_rows", all(d == 0 for d in d_rows), "spectrum rows arrived in a raw IQ window")
        r.add("delivery.page_streaming", all(p["iqStreaming"] for _, p in pages), "page not streaming IQ")
        r.add("delivery.bridge_subscriptions", all(b["iq"] == 1 and b["audio"] == 1 for _, b in bridges if "iq" in b), "Bridge IQ/audio subscription not 1/1")
    elif kind == "B":
        r.add("delivery.render_source", all(p["renderSource"] == "server" and p["echoMode"] == "spectrum" for _, p in pages), "render source/echo not server/spectrum")
        display_deltas = d_rows
        r.add("delivery.no_raw_iq", all(d == 0 for d in d_rx) and all(d == 0 for d in d_iq), "raw IQ arrived in a rows window")
        r.add("delivery.page_streaming", all(p["iqStreaming"] for _, p in pages), "page not streaming")
        r.add("delivery.bridge_subscriptions", all(b["iq"] == 1 and b["audio"] == 1 for _, b in bridges if "iq" in b), "Bridge IQ/audio subscription not 1/1")
    else:  # C: audio only
        display_deltas = None
        r.add("delivery.no_raw_iq_or_rows", all(d == 0 for d in d_rx) and all(d == 0 for d in d_iq) and all(d == 0 for d in d_rows),
              "raw RX IQ, IQ or spectrum rows arrived in an audio-only window")
        r.add("delivery.page_not_streaming_iq", all(not p["iqStreaming"] for _, p in pages), "page reports IQ streaming")
        r.add("delivery.bridge_iq_subscription_zero", all(b["iq"] == 0 and b["iq_tci_frames_s"] == 0 for _, b in bridges if "iq" in b),
              "Bridge IQ subscription or IQ frame rate not zero")
        r.add("delivery.bridge_audio_subscription_one", all(b["audio"] == 1 for _, b in bridges if "audio" in b), "Bridge audio subscription not 1")
        r.add("delivery.bridge_rows_written_unchanged", len(set(rw)) == 1 if rw else False, "Bridge spectrum rows written changed")
        frames = w["frames"]
        stops = [f for f in frames if "iq_stop" in f.get("p", "")]
        stopped = w.get("stoppedAt")
        r.add("arm_c.single_iq_stop", len(stops) == 1 and is_num(stopped) and abs(stops[0]["t"] - stopped) <= 1500,
              f"{len(stops)} iq_stop frame(s), stoppedAt={stopped}")
        if is_num(stopped):
            after = [f["p"] for f in frames if f["t"] > stopped + 1500]
            r.add("arm_c.no_restart_after_stop", not after, f"commands sent after the stop: {after[:3]}")
            r.add("arm_c.drained_before_first_sample", ts[0] - stopped >= C_DRAIN_MIN_MS, f"first sample {ts[0] - stopped} ms after the stop, required {C_DRAIN_MIN_MS}")
    if kind != "C":
        r.add("delivery.no_iq_stop", not any("iq_stop" in f.get("p", "") for f in w["frames"]), "an iq_stop was sent during a display window")
    r.add("record.mode_matches_arm", w["mode"] == arm["mode"], f"mode {w['mode']!r}, expected {arm['mode']!r}")
    r.add("record.url_names_display_transport", f"display_transport={arm['override']}" in w["url"], f"url {w['url']!r}")

    # --- audio was started (cumulative frames since page load). How fast it then played is performance, not validity:
    # a window whose audio slowed or stopped is a result, but audio that never started is a failed setup.
    played = (pages[-1][1]["audioPlayed"] - pages[0][1]["audioPlayed"]) / secs
    opus = (pages[-1][1]["opusFrames"] - pages[0][1]["opusFrames"]) / secs
    wire_key, other_key = ("opusFrames", "pcmFrames") if expect_codec == "opus" else ("pcmFrames", "opusFrames")
    r.add("audio.started", pages[-1][1][wire_key] > 0 and pages[-1][1]["audioPlayed"] > 0,
          f"no {expect_codec} frames and no played audio were ever counted: audio never started")
    # The decoded-frame counters must agree with the declared wire codec: a startup count of the other codec is harmless,
    # but new frames of the other codec in a measured interval contradict the label.
    contra = [pages[k + 1][0] for k in range(len(pages) - 1) if pages[k + 1][1][other_key] > pages[k][1][other_key]]
    r.add("audio.no_frames_of_the_other_codec", not contra,
          f"{other_key} advanced in a window declared {expect_codec} (intervals ending at samples {indices(contra)})")
    delivery["played"] = played

    # --- TCP_NODELAY: label, arm, and the retained Bridge log must all agree
    label = meta["bridgeNoDelay"]
    r.add("nodelay.label_matches_arm", label == arm["nodelay"], f"label {label}, arm {meta['arm']} requires {arm['nodelay']}")
    # UNVERIFIED is not a failed check: it is reported in the window's nodelay field and decides exit status 3.
    # Two sources, each compared with the label in one place: the Bridge's own socket read-back counters carried in the
    # samples (nodelay_from_samples) and the retained Bridge logs (read_nodelay_evidence).
    log_state = nodelay_state.get("log_state", nodelay_state)
    sample_state = nodelay_state.get("sample_state")
    if log_state["status"] != "UNVERIFIED":
        r.add("nodelay.log_matches_label", log_state["status"] == "VERIFIED", log_state["why"])
    if sample_state is not None:
        r.add("nodelay.bridge_reported_matches_label", sample_state["status"] == "VERIFIED", sample_state["why"])

    # --- performance: reported, never a reason to reject
    first, last = pages[0][1], pages[-1][1]
    q = [p["queuedMs"] for _, p in pages if is_num(p["queuedMs"])]
    perf = {
        "underruns": last["underruns"] - first["underruns"], "overflows": last["overflows"] - first["overflows"],
        "drops": last["drops"] - first["drops"], "audio_gaps": last["audioGaps"] - first["audioGaps"],
        "audio_resyncs": last["audioResyncs"] - first["audioResyncs"], "decode_errors": last["decodeErrors"] - first["decodeErrors"],
        "late_drops": last["lateDrops"] - first["lateDrops"],
        "played_per_s": round(played, 1),
        "stalled_seconds": sum(1 for d in deltas("audioPlayed") if d == 0),
        "audio_slow": played < AUDIO_SLOW_FRACTION * AUDIO_NOMINAL_FPS,
        "opus_per_s": round(opus, 1),
        "queue_ms_min_med_max": [round(min(q)), round(statistics.median(q)), round(max(q))] if q else None,
        "bridge_audio_dropped_s_max": max((b["audio_dropped_s"] for _, b in bridges if "audio_dropped_s" in b), default=None),
        # outbound_drops is a per-report count (cleared when the Bridge reports it), so only these two are meaningful; it is never a total.
        "bridge_outbound_drops_max": max((b["outbound_drops"] for _, b in bridges if "outbound_drops" in b), default=None),
        "bridge_samples_with_pending_drops": sum(1 for _, b in bridges if b.get("outbound_drops", 0) > 0),
        "jitter_p95_ms_last": last.get("jitterP95"),
    }
    if display_deltas is not None:
        total = sum(display_deltas)
        rate = total / secs
        perf["display_per_s"] = round(rate, 1)
        perf["display_stalled_seconds"] = sum(1 for d in display_deltas if d <= 0)
        perf["display_delivered_nothing"] = total == 0
        perf["display_rate_outside_expected_band"] = not (DISPLAY_FPS[0] <= rate <= DISPLAY_FPS[1])
    sp = w["start"].get("page", {}) if isinstance(w.get("start"), dict) else {}
    startup = {k: sp.get(k) for k in ("underruns", "overflows", "drops")}
    delivery.update(render=last["renderSource"], codec=last["codec"], format=[last["codec"], last["rate"], last["channels"], last["worklet"]], rows_s=round((last["rows"] - first["rows"]) / secs, 1),
                    rx_iq_s=round((last["rxIq"] - first["rxIq"]) / secs, 1))
    delivery["bridge_iq_audio"] = f'{bridges[-1][1].get("iq")}/{bridges[-1][1].get("audio")}'
    return r, perf, startup, delivery, info


def check_owner(r, info, w, prof, pages, bridges, sidecar):
    """The owner's process identity, the collector's timing and the retained raw answers.

    Evidence is in play when any sample carries a collector field or a sidecar exists, and then every rule applies. With
    none of it, the live profile (or --require-owner) reports UNVERIFIED; other runs report NOT COLLECTED."""
    meta = w["meta"]
    sidecar_present = bool(sidecar and sidecar.get("present"))
    if not any(k in b for _, b in bridges for k in OWNER_KEYS) and not sidecar_present:
        if prof.get("require_owner"):
            info["owner"] = {"status": "UNVERIFIED", "why": "no owner-identity evidence (the collector's ownerPid, start ticks, boot id and running-image hash)"}
        else:
            info["owner"] = {"status": "NOT COLLECTED", "why": "this window was taken without the owner collector"}
        return
    failed_before = len(r.failed())
    missing_at = [i for i, b in bridges if not all(k in b for k in OWNER_KEYS)]
    r.add("owner.fields_every_sample", not missing_at, f"owner/collector fields missing at samples {indices(missing_at)}")
    well_formed = False
    if not missing_at:
        bad_at = []
        for i, b in bridges:
            problems = [k for k, low in OWNER_INT_MIN.items() if not (is_int(b[k]) and b[k] >= low)]
            problems += [k for k in OWNER_NUM if not is_num(b[k])]
            problems += [k for k in OWNER_HASH if not (isinstance(b[k], str) and SHA256_RE.match(b[k]))]
            problems += [k for k in ("ownerAlive",) if not isinstance(b[k], bool)]
            problems += [k for k in ("bootId",) if not (isinstance(b[k], str) and b[k])]
            problems += [k for k in ("fpgaBuildId",) if not (isinstance(b[k], str) and b[k])]
            problems += [k for k in ("firmwareMajor", "firmwareMinor") if not (is_int(b[k]) and b[k] >= 0)]
            problems += [k for k in ("rxc1Status",) if k in b and not isinstance(b[k], str)]
            problems += [k for k in ("rxc1HostAcquisitionFailures",) if k in b and not (is_int(b[k]) and b[k] >= 0)]
            if "exeSha256" in problems and b.get("exeError"):
                problems.append(f"exeError: {b['exeError']}")   # typically: the reader may not read another user's /proc/<pid>/exe
            if problems:
                bad_at.append((i, problems))
        well_formed = r.add("owner.fields_well_formed", not bad_at, "; ".join(f"sample {i}: {ps}" for i, ps in bad_at[:3]))
    if well_formed:
        ident = {(b["ownerPid"], b["ownerStartTicks"], b["bootId"], b["exeSha256"]) for _, b in bridges}
        r.add("owner.alive_every_sample", all(b["ownerAlive"] for _, b in bridges), "the owner process was not alive at some sample")
        r.add("owner.constant_in_window", len(ident) == 1, f"the owner changed during the window: {sorted(map(str, ident))[:3]}")
        r.add("owner.pid_matches_meta", all(b["ownerPid"] == meta["bridgePid"] for _, b in bridges),
              f"owner pid {sorted({b['ownerPid'] for _, b in bridges})} but the window's meta says bridgePid {meta['bridgePid']}")
        r.add("owner.exe_matches_meta", all(b["exeSha256"] == meta["bridgeSha256"] for _, b in bridges),
              "the running executable's hash is not the one the window's meta names")
        bad_main = [i for i, b in bridges if not (b.get("mainPidMatches") is None or b.get("mainPidMatches") is True)]
        r.add("owner.service_main_pid", not bad_main, f"the service's MainPID differs from the document's pid at samples {indices(bad_main)}")
        late = [i for i, b in bridges if not (0 <= b["requestLatencyMs"] <= COLLECTOR_LATENCY_MAX_MS)]
        r.add("collector.latency_bounded", not late, f"request latency outside 0..{COLLECTOR_LATENCY_MAX_MS} ms at samples {indices(late)}")
        unpaired = [i for (i, p), (_, b) in zip(pages, bridges) if not (0 <= b["collectorReceivedAtMs"] - p["t"] <= COLLECTOR_PAIR_MAX_MS)]
        r.add("collector.paired_with_page_sample", not unpaired,
              f"collector answer not received within 0..{COLLECTOR_PAIR_MAX_MS} ms after its page sample, at samples {indices(unpaired)}")
        seqs = [b["collectorSeq"] for _, b in bridges]
        r.add("collector.seq_strictly_increasing", all(y > x for x, y in zip(seqs, seqs[1:])), "collector sequence numbers do not strictly increase")
        reads = [b["piReadAtMs"] for _, b in bridges]
        r.add("collector.pi_read_time_advances", all(y > x for x, y in zip(reads, reads[1:])), "the Pi's read time does not strictly increase (a repeated answer or a clock step)")
        d_pi, d_page = reads[-1] - reads[0], pages[-1][1]["t"] - pages[0][1]["t"]
        r.add("collector.pi_clock_tracks_window", abs(d_pi - d_page) <= PI_CLOCK_TRACK_MS,
              f"the Pi's clock advanced {d_pi / 1000:.1f}s while the page's advanced {d_page / 1000:.1f}s (limit {PI_CLOCK_TRACK_MS / 1000:.0f}s)")
        aged = [b for _, b in bridges if is_num(b.get("piSourceAgeMs"))]
        r.add("bridge.pi_age_arithmetic", all(b["piSourceAgeMs"] == b["piReadAtMs"] - b["piSourceUpdatedAtMs"] for b in aged),
              "piSourceAgeMs is not the Pi's read time minus the document's own update time")
        r.add("bridge.pi_updated_matches_document", all(b["piSourceUpdatedAtMs"] == b["updatedAtMs"] for _, b in bridges),
              "the Pi-side update time is not the document's updatedAtMs")
        rx = meta.get("rxc1")
        if prof.get("require_rxc1") and isinstance(rx, dict) and "pid" in rx:
            r.add("rxc1.pid_matches_owner", all(b["ownerPid"] == rx["pid"] for _, b in bridges), f"the RXC1 state is recorded for pid {rx['pid']!r}, not the owner process")
        fpga = {(b["fpgaBuildId"], b["firmwareMajor"], b["firmwareMinor"]) for _, b in bridges}
        r.add("owner.fpga_image_constant_in_window", len(fpga) == 1, f"the FPGA image changed during the window: {sorted(map(str, fpga))[:3]}")
        if prof.get("expect_fpga_build") is not None:
            r.add("owner.fpga_image_is_the_planned_one", {b["fpgaBuildId"] for _, b in bridges} == {prof["expect_fpga_build"]},
                  f"the owner reports FPGA build {sorted({b['fpgaBuildId'] for _, b in bridges})}, planned {prof['expect_fpga_build']!r}")
        if len(fpga) == 1:
            info["fpga_ident"] = next(iter(fpga))
        # RXC1 polling state, from each sample's own telemetry (metrics.rx_counter_v31.status), never from what a person typed.
        carries = ["rxc1Status" in b for _, b in bridges]
        r.add("rxc1.telemetry_all_or_none", all(carries) or not any(carries), "the owner's RXC1 status is present in only some samples")
        if prof.get("require_rxc1") and isinstance(rx, dict) and "state" in rx:
            mismatched = [i for i, b in bridges if b.get("rxc1Status") != rx["state"]]
            r.add("rxc1.status_every_sample_matches_state", not mismatched,   # a sample without the status counts as a mismatch
                  f"the owner's RXC1 status differs from the recorded state {rx['state']!r} at samples {indices(mismatched)}" if any(carries)
                  else "no sample carries the owner's RXC1 status, so the recorded RXC1 state cannot be verified")
        if len(ident) == 1:
            info["owner_ident"] = next(iter(ident))

    # --- the retained raw answers
    retained = sidecar_present and sidecar.get("error") is None and sidecar.get("starts") == 1 and sidecar["entries"][0].get("kind") == "start" \
        and sidecar["entries"][0].get("schema") == SIDECAR_SCHEMA
    r.add("collector.sidecar_retained", retained,
          "no collector sidecar was retained for this window" if not sidecar_present else f"the sidecar is unusable: {sidecar.get('error') or 'no single start entry'}")
    if retained:
        entries = sidecar["entries"]
        reads = [e for e in entries if e.get("kind") == "read"]
        seqs = [e.get("seq") for e in reads]
        r.add("collector.sidecar_seq_unique", len(set(map(str, seqs))) == len(seqs), "the sidecar repeats a sequence number")
        failures = sum(1 for e in reads if e.get("error"))
        summary = w.get("collector")
        r.add("collector.sidecar_complete", isinstance(summary, dict) and summary.get("reads") == len(reads) and summary.get("failures") == failures,
              f"the record's collector summary {summary!r} does not match the sidecar ({len(reads)} reads, {failures} failed)")
        if well_formed:
            by_seq = {e["seq"]: e for e in reads if is_int(e.get("seq"))}
            unmatched, mismatched = [], []
            for i, b in bridges:
                e = by_seq.get(b["collectorSeq"])
                answer = None
                if e is not None and not e.get("error") and isinstance(e.get("raw"), str):
                    try:
                        answer = json.loads(e["raw"], parse_constant=_reject_constant)
                    except ValueError:
                        answer = None
                if not isinstance(answer, dict) or answer.get("ok") is not True:
                    unmatched.append(i)
                    continue
                diffs = raw_differences(b, e, answer)
                if diffs:
                    mismatched.append((i, diffs))
            r.add("collector.sidecar_entry_for_every_sample", not unmatched, f"no usable raw answer in the sidecar for samples {indices(unmatched)}")
            r.add("collector.samples_match_raw_documents", not mismatched, "; ".join(f"sample {i}: {d[:4]}" for i, d in mismatched[:3]))
        spawns = [e for e in entries if e.get("kind") == "spawn"]
        events = [e for e in entries if e.get("kind") == "event"]
        if failures or events or len(spawns) > 1:
            info["outcomes"].append(f"collector: {failures} failed read(s), {len(events)} reader event(s), {len(spawns)} reader start(s) recorded in the sidecar")
    info["owner"] = {"status": "CONTRADICTED" if len(r.failed()) > failed_before else "VERIFIED", "why": "see the failed owner./collector. checks"}
    if info["owner"]["status"] == "VERIFIED":
        pid, ticks, boot, exe = info["owner_ident"]
        info["owner"]["why"] = f"pid {pid}, start ticks {ticks}, boot {boot[:8]}, running image {exe[:12]} constant over {len(bridges)} raw answers"


def raw_differences(b, entry, answer):
    """Fields of a sample's bridge record that differ from the raw answer the collector retained for it."""
    doc = answer.get("document") if isinstance(answer.get("document"), dict) else {}
    metrics = doc.get("metrics") if isinstance(doc.get("metrics"), dict) else {}
    owner = answer.get("owner") if isinstance(answer.get("owner"), dict) else {}
    pairs = [("documentSha256", answer.get("documentSha256")), ("piSourceAgeMs", answer.get("piSourceAgeMs")), ("piReadAtMs", answer.get("piReadAtMs")),
             ("piSourceUpdatedAtMs", answer.get("piSourceUpdatedAtMs")), ("updatedAtMs", doc.get("updated_at_ms")),
             ("ownerPid", owner.get("pid")), ("ownerStartTicks", owner.get("startTicks")), ("ownerAlive", owner.get("alive")), ("bootId", owner.get("bootId")),
             ("exeSha256", owner.get("exeSha256")), ("exeError", owner.get("exeError")), ("mainPidMatches", owner.get("mainPidMatches")),
             ("collectorReceivedAtMs", entry.get("receivedAtMs")), ("requestLatencyMs", entry.get("latencyMs")), ("collectorSpawn", entry.get("spawn"))]
    pairs += [(field, metrics.get(metric)) for field, metric in {**METRIC_MAP, **NODELAY_MAP, **FPGA_MAP}.items()]
    rx = metrics.get("rx_counter_v31") if isinstance(metrics.get("rx_counter_v31"), dict) else {}
    pairs += [("rxc1Status", rx.get("status")), ("rxc1HostAcquisitionFailures", rx.get("host_acquisition_failures"))]
    return [field for field, want in pairs if b.get(field) != want]   # absent on both sides is equal


def read_sidecar(window_file):
    """The collector's sidecar next to a window record (w<N>_<ARM>.collector.jsonl), strictly parsed, or {"present": False}."""
    if not window_file.endswith(".json"):
        return {"present": False}
    path = window_file[: -len(".json")] + ".collector.jsonl"
    if not os.path.exists(path):
        return {"present": False}
    entries, error = [], None
    try:
        with open(path) as fh:
            for line in fh:
                if line.strip():
                    entries.append(json.loads(line, parse_constant=_reject_constant))
    except (OSError, ValueError) as e:
        error = f"{os.path.basename(path)}: {e}"
    if error is None and not all(isinstance(e, dict) for e in entries):
        error = f"{os.path.basename(path)}: a line is not a JSON object"
    entries = [e for e in entries if isinstance(e, dict)]
    return {"present": True, "entries": entries, "error": error, "starts": sum(1 for e in entries if e.get("kind") == "start")}


def nodelay_from_samples(w, label, windows_served):
    """TCP_NODELAY as the Bridge itself reports it in the samples (perf.json: the setting, and the sockets whose option it set
    and then read back with getsockopt). None when the samples carry no such fields.
    `windows_served`: how many windows this Bridge instance has served, this one included."""
    samples = w.get("samples") if isinstance(w, dict) else None
    if not isinstance(samples, list):
        return None
    bridges = [s["bridge"] for s in samples if isinstance(s, dict) and isinstance(s.get("bridge"), dict) and "error" not in s["bridge"]]
    if not any(k in b for b in bridges for k in NODELAY_KEYS):
        return None

    def verdict(status, why):
        return {"status": status, "why": why, "logged": None}

    if not all(all(k in b for k in NODELAY_KEYS) for b in bridges):
        return verdict("CONTRADICTED", "the Bridge-reported TCP_NODELAY fields are present in only some samples")
    if not all(is_int(b[k]) and b[k] >= 0 for b in bridges for k in NODELAY_KEYS):
        return verdict("CONTRADICTED", "the Bridge-reported TCP_NODELAY fields are not non-negative counters")
    enabled = {b["nodelayEnabled"] for b in bridges}
    confirmed = [b["nodelayConfirmedTotal"] for b in bridges]
    failed = [b["nodelayFailedTotal"] for b in bridges]
    if enabled != {label}:
        return verdict("CONTRADICTED", f"the Bridge reports tci_nodelay_enabled {sorted(enabled)}, the label says {label!r}")
    if any(failed):
        return verdict("CONTRADICTED", f"the Bridge counted {max(failed)} socket(s) whose TCP_NODELAY could not be set or read back")
    if any(y < x for x, y in zip(confirmed, confirmed[1:])):
        return verdict("CONTRADICTED", "the Bridge's confirmed-socket counter went backwards (a restart inside the window)")
    if label == 1 and confirmed[0] < windows_served:
        return verdict("CONTRADICTED", f"only {confirmed[0]} socket(s) confirmed at the first sample, but this Bridge instance has served {windows_served} window(s)")
    if label == 0 and confirmed[-1] != 0:
        return verdict("CONTRADICTED", f"{confirmed[-1]} socket(s) confirmed with TCP_NODELAY off")
    what = f"{confirmed[0]}..{confirmed[-1]} accepted socket(s) set and read back as on, none failed" if label == 1 else "setting off, no socket changed"
    return verdict("VERIFIED", f"the Bridge reports tci_nodelay_enabled={label}: {what}")


def combine_nodelay(log_state, sample_state):
    """One state for the report and the exit status: the Bridge-reported evidence first, the retained logs second, else UNVERIFIED.
    A source that contradicts the other makes the window CONTRADICTED."""
    out = dict(log_state, log_state=log_state, sample_state=sample_state, source="retained Bridge logs")
    if sample_state is None:
        return out
    out.update(status=sample_state["status"], why=sample_state["why"], logged=sample_state.get("logged"), source="Bridge-reported (perf.json)")
    if sample_state["status"] == "VERIFIED" and log_state["status"] == "CONTRADICTED":
        out.update(status="CONTRADICTED", why=f"the Bridge reports it verified, but the retained logs contradict: {log_state['why']}")
    return out


def read_nodelay_evidence(restart_number, windows_served, label, log_dirs):
    """TCP_NODELAY state of one Bridge instance from its retained logs: status VERIFIED / UNVERIFIED / CONTRADICTED."""
    out_path = err_path = None
    for d in log_dirs:
        o, e = os.path.join(d, f"bridge_{restart_number}.out"), os.path.join(d, f"bridge_{restart_number}.err")
        if os.path.exists(o) and out_path is None:
            out_path = o
        if os.path.exists(e) and err_path is None:
            err_path = e
    if out_path is None:
        return {"status": "UNVERIFIED", "why": f"no retained Bridge log bridge_{restart_number}.out", "logged": None}
    text = open(out_path, errors="replace").read()
    states = re.findall(r"TCP_NODELAY on accepted TCI sockets: (on|off)", text)
    if not states:
        return {"status": "UNVERIFIED", "why": f"bridge_{restart_number}.out has no TCP_NODELAY start-up line", "logged": None}
    if len(set(states)) != 1:
        return {"status": "CONTRADICTED", "why": f"bridge_{restart_number}.out reports both on and off", "logged": None}
    logged = 1 if states[0] == "on" else 0
    if err_path is None:
        return {"status": "UNVERIFIED", "why": f"no retained socket log bridge_{restart_number}.err (start-up line says {states[0]})", "logged": logged}
    err = open(err_path, errors="replace").read()
    calls_ok = len(re.findall(r"Bridge setsockopt\(TCP_NODELAY=1\) on fd \d+ -> ok", err))
    calls_any = len(re.findall(r"Bridge setsockopt\(TCP_NODELAY", err))
    failures = len(re.findall(r"TCP_NODELAY FAILED|Bridge setsockopt\(TCP_NODELAY=\d\) on fd \d+ -> FAILED", err))
    if failures:
        return {"status": "CONTRADICTED", "why": f"{failures} TCP_NODELAY call(s) failed in bridge_{restart_number}.err", "logged": logged}
    if logged == 1 and calls_ok < windows_served:
        return {"status": "CONTRADICTED", "why": f"start-up says on, but only {calls_ok} socket(s) set for {windows_served} window(s)", "logged": logged}
    if logged == 0 and calls_any:
        return {"status": "CONTRADICTED", "why": f"start-up says off, but {calls_any} socket call(s) were logged", "logged": logged}
    if logged != label:
        return {"status": "CONTRADICTED", "why": f"log says {states[0]}, label says {'on' if label else 'off'}", "logged": logged}
    return {"status": "VERIFIED", "why": f"start-up line '{states[0]}' and {calls_ok} socket call(s) retained", "logged": logged}


def load_windows(directory):
    files = glob.glob(os.path.join(directory, "w*_*.json"))
    out = []
    for f in files:
        m = re.match(r"w(\d+)_(.+)\.json$", os.path.basename(f))
        if not m:
            continue
        try:
            with open(f) as fh:
                out.append((int(m.group(1)), m.group(2), f, json.load(fh, parse_constant=_reject_constant)))
        except (OSError, ValueError) as e:
            out.append((int(m.group(1)), m.group(2), f, {"_unreadable": str(e)}))
    return sorted(out, key=lambda x: x[0])


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("windows_dir", help="directory holding w<N>_<ARM>.json")
    ap.add_argument("--mode", choices=["order", "single"], default="order", help="order: the full planned set; single: per-window rules only")
    ap.add_argument("--profile", choices=sorted(PROFILES), default="rehearsal")
    ap.add_argument("--bridge-logs", action="append", default=[], help="directory with bridge_<N>.out/.err (repeatable)")
    ap.add_argument("--report-dir", help="where to write order_check.json and order_summary.txt (default: <windows_dir>/check_report)")
    ap.add_argument("--allow-unverified-nodelay", action="store_true", help="exit 0 when only TCP_NODELAY is UNVERIFIED (still reported)")
    ap.add_argument("--allow-unverified-freshness", action="store_true", help="exit 0 when only live telemetry freshness is UNVERIFIED (still reported)")
    ap.add_argument("--allow-unverified-owner", action="store_true", help="exit 0 when only the owner's process identity is UNVERIFIED (still reported)")
    ap.add_argument("--require-owner", action="store_true", help="require the owner collector's evidence per window (always on in the live profile)")
    ap.add_argument("--expect-codec", default="opus")
    ap.add_argument("--require-rxc1", action="store_true", help="require a recorded RXC1 state per window (always on in the live profile)")
    ap.add_argument("--expect-fpga-build", help="the planned FPGA build id as the owner reports it (date_code_hex, for example 53460004); every collected window must show it")
    ap.add_argument("--expect-rxc1", help="the planned RXC1 state (the owner's rx_counter_v31.status, for example valid); every window must record it. Implies --require-rxc1")
    ap.add_argument("--order", default=",".join(ORDER), help="comma-separated planned arm order (order mode)")
    args = ap.parse_args(argv)
    if not os.path.isdir(args.windows_dir):
        print(f"check_order: not a directory: {args.windows_dir}", file=sys.stderr)
        return 2
    prof = dict(PROFILES[args.profile])
    prof["expect_codec"] = args.expect_codec
    if args.require_rxc1 or args.expect_rxc1 is not None:
        prof["require_rxc1"] = True
    if args.expect_rxc1 is not None:
        prof["expect_rxc1"] = args.expect_rxc1
    if args.expect_fpga_build is not None:
        prof["expect_fpga_build"] = args.expect_fpga_build
    if args.require_owner:
        prof["require_owner"] = True
    windows = load_windows(args.windows_dir)
    if not windows:
        print("check_order: no w<N>_<ARM>.json records found", file=sys.stderr)
        return 2
    log_dirs = args.bridge_logs + [args.windows_dir, os.path.join(args.windows_dir, "..", "bridge-logs")]

    results, set_checks = [], Checks()
    served, seen = {}, {}
    for _, _, _, w in windows:
        m = w.get("meta") if isinstance(w, dict) and not meta_problems(w.get("meta")) else {}
        served[m.get("bridgeRestartNumber")] = served.get(m.get("bridgeRestartNumber"), 0) + 1
    for idx, arm_name, path, w in windows:
        if "_unreadable" in w:
            c = Checks()
            c.add("record.readable", False, w["_unreadable"])
            results.append(dict(n=idx, arm=arm_name, file=os.path.basename(path), checks=c, perf={}, startup={}, delivery={}, nodelay={"status": "UNVERIFIED", "why": "unreadable record"},
                                info={"outcomes": [], "freshness": {"status": "UNVERIFIED", "why": "unreadable record"}, "owner": {"status": "UNVERIFIED", "why": "unreadable record"}, "owner_ident": None}))
            continue
        raw_meta = w.get("meta") if isinstance(w.get("meta"), dict) else {}
        meta = raw_meta if not meta_problems(raw_meta) else {}   # a malformed identity is excluded from grouping; check_window fails it
        seen[meta.get("bridgeRestartNumber")] = seen.get(meta.get("bridgeRestartNumber"), 0) + 1
        has_label = all(k in meta for k in ("bridgeRestartNumber", "bridgeNoDelay"))
        try:
            log_nd = read_nodelay_evidence(meta.get("bridgeRestartNumber"), served.get(meta.get("bridgeRestartNumber"), 1), meta.get("bridgeNoDelay"), log_dirs) \
                if has_label else {"status": "UNVERIFIED", "why": "meta lacks restart number or label", "logged": None}
            sample_nd = nodelay_from_samples(w, meta.get("bridgeNoDelay"), seen[meta.get("bridgeRestartNumber")]) if has_label else None
            nd = combine_nodelay(log_nd, sample_nd)
            c, perf, startup, delivery, info = check_window(w, prof, nd, read_sidecar(path))
        except Exception as error:   # a record the checker cannot interpret is INVALID with a reason: never a crash, never a pass
            c, perf, startup, delivery = Checks(), {}, {}, {}
            c.add("record.interpretable", False, f"the checker could not interpret this record: {type(error).__name__}: {error}")
            nd = {"status": "UNVERIFIED", "why": "record not interpretable", "logged": None}
            info = {"outcomes": [], "freshness": {"status": "UNVERIFIED", "why": "record not interpretable"},
                    "owner": {"status": "UNVERIFIED", "why": "record not interpretable"}, "owner_ident": None}
        c.add("record.file_name_matches_meta", raw_meta.get("arm") == arm_name and raw_meta.get("index") == idx, f"file w{idx}_{arm_name} vs meta {raw_meta.get('index')}/{raw_meta.get('arm')}")
        results.append(dict(n=idx, arm=arm_name, file=os.path.basename(path), checks=c, perf=perf, startup=startup, delivery=delivery, nodelay=nd, info=info, meta=meta))

    if args.mode == "order":
        planned = [a.strip() for a in args.order.split(",") if a.strip()]
        got = [r["arm"] for r in results]
        idxs = [r["n"] for r in results]
        set_checks.add("set.window_count", len(results) == len(planned), f"{len(results)} windows, planned {len(planned)}")
        set_checks.add("set.indices_consecutive", idxs == list(range(1, len(idxs) + 1)), f"indices {idxs}")
        set_checks.add("set.order", got == planned, f"order {got}, planned {planned}")
        missing = [i + 1 for i, a in enumerate(planned) if i >= len(got)]
        set_checks.add("set.no_missing_windows", not missing, f"missing windows {missing}")
        metas = [r.get("meta", {}) for r in results if r.get("meta")]
        shas = {hashable(m.get("bridgeSha256")) for m in metas}
        set_checks.add("set.one_bridge_binary", len(shas) == 1 and all(isinstance(s, str) and SHA256_RE.match(s) for s in shas),
                       f"Bridge binary hashes {sorted(map(str, shas))} (one valid 64-hex SHA-256 required)")
        restarts = [m.get("bridgeRestartNumber") for m in metas]
        set_checks.add("set.restart_numbers_nondecreasing", all(b >= a for a, b in zip(restarts, restarts[1:])) if restarts else False, f"restart numbers {restarts}")
        by_inst = {}
        for m in metas:
            by_inst.setdefault(m.get("bridgeRestartNumber"), set()).add((m.get("bridgeNoDelay"), m.get("bridgePid")))
        set_checks.add("set.one_state_and_pid_per_bridge_instance", all(len(v) == 1 for v in by_inst.values()), f"instances {by_inst}")
        changes = [(a.get("bridgeNoDelay"), b.get("bridgeNoDelay"), a.get("bridgeRestartNumber"), b.get("bridgeRestartNumber")) for a, b in zip(metas, metas[1:])]
        set_checks.add("set.restart_exactly_when_nodelay_changes", all((x != y) == (rx != ry) for x, y, rx, ry in changes), "a setting changed without a restart, or a restart without a change")
        fmts = {tuple(r["delivery"]["format"]) for r in results if r["delivery"].get("format")}
        set_checks.add("set.same_audio_format_in_every_window", len(fmts) == 1, f"codec/rate/channels/worklet seen: {sorted(map(str, fmts))}")
        if prof.get("require_rxc1"):
            states = {json.dumps(m.get("rxc1", {}).get("state") if isinstance(m.get("rxc1"), dict) else None, sort_keys=True) for m in metas}
            set_checks.add("set.rxc1_state_constant", len(states) == 1 and "null" not in states, f"RXC1 states seen: {sorted(states)}")
        collected = [r["info"]["owner"]["status"] in ("VERIFIED", "CONTRADICTED") for r in results]
        if any(collected):
            # The same process serves every window of a Bridge instance, and a restart is a new process on the same boot.
            set_checks.add("set.owner_identity_every_window", all(collected), f"owner evidence missing in windows {[r['n'] for r, c_ in zip(results, collected) if not c_]}")
            by_inst = {}
            for r in results:
                if r["info"].get("owner_ident") is not None:
                    by_inst.setdefault(r.get("meta", {}).get("bridgeRestartNumber"), set()).add(r["info"]["owner_ident"])
            set_checks.add("set.owner_identity_per_bridge_instance", all(len(v) == 1 for v in by_inst.values()), f"identities per Bridge instance {by_inst}")
            processes = [next(iter(v))[:2] for v in by_inst.values() if len(v) == 1]
            set_checks.add("set.owner_new_process_per_restart", len(set(processes)) == len(processes), "two Bridge instances were served by the same process")
            set_checks.add("set.fpga_image_same_in_every_window", len({r["info"]["fpga_ident"] for r in results if r["info"].get("fpga_ident")}) <= 1, "the FPGA image differs between windows")
            set_checks.add("set.owner_same_boot", len({i[2] for v in by_inst.values() for i in v}) <= 1, "the Pi rebooted during the comparison")
        c_nd = {r["meta"].get("bridgeNoDelay") for r in results if r["arm"] == "C" and r.get("meta")}
        set_checks.add("set.arm_c_has_nodelay_on_at_both_ends", c_nd == {1} if c_nd else False, f"arm C labels {c_nd}")

    any_invalid = bool(set_checks.failed()) or any(r["checks"].failed() for r in results)
    nodelay_unverified = any(r["nodelay"]["status"] == "UNVERIFIED" for r in results)
    fresh_unverified = any(r["info"]["freshness"]["status"] == "UNVERIFIED" for r in results)
    owner_unverified = any(r["info"]["owner"]["status"] == "UNVERIFIED" for r in results)
    any_unverified = (nodelay_unverified and not args.allow_unverified_nodelay) or (fresh_unverified and not args.allow_unverified_freshness) \
        or (owner_unverified and not args.allow_unverified_owner)

    # --- report
    rows = []
    for r in results:
        failed = r["checks"].failed()
        p, d, st = r["perf"], r["delivery"], r["startup"]
        rows.append({
            "n": r["n"], "arm": r["arm"], "evidence": "INVALID" if failed else "valid", "nodelay": r["nodelay"]["status"], "fresh": r["info"]["freshness"]["status"], "owner": r["info"]["owner"]["status"],
            "render": d.get("render", "-"), "rows/s": d.get("rows_s", "-"), "rxIQ/s": d.get("rx_iq_s", "-"), "codec": d.get("codec", "-"),
            "played/s": p.get("played_per_s", "-"),
            "underruns": p.get("underruns", "-"), "overflows": p.get("overflows", "-"), "drops": p.get("drops", "-"), "gaps": p.get("audio_gaps", "-"),
            "startup u/o/d": "/".join(str(st.get(k, "-")) for k in ("underruns", "overflows", "drops")) if st else "-",
            "queue ms min/med/max": "/".join(map(str, p["queue_ms_min_med_max"])) if p.get("queue_ms_min_med_max") else "-",
            "bridge iq/audio": d.get("bridge_iq_audio", "-"),
        })
    cols = list(rows[0].keys())
    width = {c: max(len(c), *(len(str(x[c])) for x in rows)) for c in cols}
    lines = [" ".join(c.ljust(width[c]) for c in cols)] + [" ".join(str(x[c]).ljust(width[c]) for c in cols) for x in rows]
    text = "\n".join(lines)
    notes = []
    for r in results:
        for c in r["checks"].failed():
            notes.append(f"window {r['n']} ({r['arm']}) FAILED {c['check']}: {c['detail']}")
        if r["nodelay"]["status"] != "VERIFIED":
            notes.append(f"window {r['n']} ({r['arm']}) TCP_NODELAY {r['nodelay']['status']}: {r['nodelay']['why']}")
        if r["info"]["freshness"]["status"] != "VERIFIED":
            notes.append(f"window {r['n']} ({r['arm']}) telemetry freshness {r['info']['freshness']['status']}: {r['info']['freshness']['why']}")
        if r["info"]["owner"]["status"] not in ("VERIFIED", "NOT COLLECTED"):
            notes.append(f"window {r['n']} ({r['arm']}) owner identity {r['info']['owner']['status']}: {r['info']['owner']['why']}")
        for o in r["info"]["outcomes"]:
            notes.append(f"window {r['n']} ({r['arm']}) OUTCOME: {o}")
    for c in set_checks.failed():
        notes.append(f"SET FAILED {c['check']}: {c['detail']}")
    verdict = "INVALID EVIDENCE" if any_invalid else ("VALID, TCP_NODELAY UNVERIFIED" if nodelay_unverified else "VALID, TCP_NODELAY VERIFIED")
    if not any_invalid and fresh_unverified:
        verdict += ", FRESHNESS UNVERIFIED"
    if not any_invalid and owner_unverified:
        verdict += ", OWNER IDENTITY UNVERIFIED"
    event_keys = ("underruns", "overflows", "drops", "audio_gaps", "audio_resyncs", "decode_errors", "late_drops", "stalled_seconds", "bridge_samples_with_pending_drops",
                  "display_stalled_seconds", "display_delivered_nothing", "audio_slow", "display_rate_outside_expected_band")
    perf_flags = [f"window {r['n']} ({r['arm']}): " + ", ".join(f"{k} {v}" for k, v in r["perf"].items() if k in event_keys and v)
                  for r in results if r["perf"] and any(r["perf"].get(k) for k in event_keys)]
    out_text = text + "\n\n" + "\n".join(notes) + ("\n" if notes else "") + \
        ("performance events in valid windows (reported, not a reason to reject): " + "; ".join(perf_flags) + "\n" if perf_flags else "") + \
        f"profile {args.profile}, mode {args.mode}: {verdict}\n"
    print(out_text)
    report_dir = args.report_dir or os.path.join(args.windows_dir, "check_report")
    os.makedirs(report_dir, exist_ok=True)
    with open(os.path.join(report_dir, "order_summary.txt"), "w") as fh:
        fh.write(out_text)
    with open(os.path.join(report_dir, "order_check.json"), "w") as fh:
        json.dump({"verdict": verdict, "profile": args.profile, "mode": args.mode, "limits": {k: v for k, v in prof.items()},
                   "set_checks": set_checks.items,
                   "windows": [{"n": r["n"], "arm": r["arm"], "file": r["file"], "checks": r["checks"].items, "nodelay": r["nodelay"],
                                "performance": r["perf"], "startup_counters": r["startup"], "delivery": r["delivery"],
                                "outcomes": r["info"]["outcomes"], "freshness": r["info"]["freshness"], "owner": r["info"]["owner"]} for r in results]},
                  fh, indent=1, default=str)
    if any_invalid:
        return 1
    if any_unverified:
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
