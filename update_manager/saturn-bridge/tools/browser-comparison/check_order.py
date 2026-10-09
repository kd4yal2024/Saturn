#!/usr/bin/env python3
"""Fail-closed checker for browser-comparison measurement windows.

Reads the per-window records written by window.mjs (w<N>_<ARM>.json) and decides two
separate things for every window:

  EVIDENCE   Is this window a valid measurement of its arm? Telemetry present, fresh and
             sane at every sample; the browser stayed connected; the codec, rate and
             channels did not change; delivery matched the arm at every interval; counters
             never went backwards; the window had the planned length; and the Bridge's
             TCP_NODELAY state, checked against the retained Bridge logs, matches the label.
  PERFORMANCE  What the audio did: underruns, overflows, drops, sequence gaps, queue lead.
             This is reported, never used to reject a window. A valid window with real
             underruns is a result, not an error.

Nothing is averaged over what is left after discarding bad samples: a window with any invalid
sample is INVALID, and its record is kept exactly as it was (this tool only reads the input).

Exit status
  0  every window is valid and the TCP_NODELAY state of every window is VERIFIED
  1  invalid evidence: a failed window, a failed set-level rule, or a contradicted label
  2  usage error or unreadable input
  3  every window is valid but TCP_NODELAY is UNVERIFIED for at least one (logs not retained);
     0 instead if --allow-unverified-nodelay is given (the report still says UNVERIFIED)

Limits are fixed here, before any live window is evaluated, in PROFILES. `rehearsal` is the
30-second local rehearsal; `live` is the planned ten-minute window. Nothing is tuned per run.
"""
import argparse
import glob
import json
import os
import re
import statistics
import sys

PROFILES = {
    "rehearsal": dict(duration_s=30, span_low=-3.0, span_high=2.0, min_samples=28, same_clock=True, require_rxc1=False),
    "live": dict(duration_s=600, span_low=-5.0, span_high=5.0, min_samples=595, same_clock=False, require_rxc1=True),
}
# Limits common to both profiles.
INTERVAL_MIN_S = 0.5            # sampling is nominally 1 Hz
INTERVAL_MAX_S = 2.0
HOLE_S = 3.0                    # any gap longer than this is a hole in the record
MAX_UNCHANGED_BRIDGE = 2        # at most this many consecutive samples with an identical Bridge timestamp
BRIDGE_AGE_MAX_MS = 3000        # same-clock profiles only: sample time minus the Bridge's update time
BRIDGE_AGE_MIN_MS = -1000
DISPLAY_FPS = (24.0, 36.0)      # display frames or rows per second when the arm shows one (cap is 30)
AUDIO_FLOOR_FPS = 10.0          # below this audio is not running: not an audio measurement at all
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
PAGE_MONOTONIC = ["iq", "rxIq", "rows", "opusFrames", "pcmFrames", "audioPlayed", "lastAudioSeq", "audioGaps",
                  "audioResyncs", "decodeErrors", "lateDrops", "underruns", "overflows", "drops"]
BRIDGE_NUM = ["updatedAtMs", "iq", "audio", "connections", "iq_tci_frames_s", "rx_audio_frames_s", "rows_written",
              "spectrum_clients", "audio_dropped_s", "tcp_outq_hwm_bytes", "out_hwm_bytes", "outbound_drops"]
META_KEYS = ["index", "arm", "bridgeNoDelay", "bridgePid", "bridgeSha256", "bridgeRestartNumber"]
TOP_KEYS = ["meta", "mode", "url", "atConnect", "atWarm", "start", "samples", "frames"]


def is_num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


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
    return problems


def check_window(w, prof, nodelay_state):
    """All checks of one window. Returns (Checks, performance dict, startup dict, delivery dict)."""
    r = Checks()
    perf, startup, delivery = {}, {}, {}
    # A record whose driver failed carries a `failure`: whatever it recorded is kept, but it is never a measurement.
    r.add("record.no_failure", "failure" not in w, f"the window driver failed: {str(w.get('failure', {}).get('message', ''))[:200]}" if "failure" in w else "")
    missing = [k for k in TOP_KEYS if k not in w]
    if not r.add("record.top_level_fields", not missing, f"missing: {missing}" if missing else ""):
        return r, perf, startup, delivery
    meta = w["meta"]
    missing_meta = [k for k in META_KEYS if k not in meta]
    if not r.add("record.meta_fields", not missing_meta, f"missing: {missing_meta}" if missing_meta else ""):
        return r, perf, startup, delivery
    arm = ARMS.get(meta["arm"])
    if not r.add("record.known_arm", arm is not None, f"arm {meta['arm']!r}"):
        return r, perf, startup, delivery
    if prof.get("require_rxc1"):
        # The acquisition owner's record of the actual RXC1 polling state for this window: not the environment flag.
        rx = meta.get("rxc1")
        have = isinstance(rx, dict) and all(rx.get(k) not in (None, "", {}, []) for k in ("state", "pid", "identity"))
        r.add("rxc1.recorded", have, "meta.rxc1 must be an object with non-empty state, pid and identity" if not have else "")
    samples = w["samples"]
    if not r.add("record.samples_present", isinstance(samples, list) and len(samples) >= 2,
                 f"{len(samples) if isinstance(samples, list) else 'no'} samples"):
        return r, perf, startup, delivery

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
        return r, perf, startup, delivery
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

    # --- browser connection and the audio format, per sample
    r.add("page.connected_every_sample", all(p["connected"] for _, p in pages),
          f"disconnected at samples {indices(i for i, p in pages if not p['connected'])}")
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
    rw = [b["rows_written"] for _, b in bridges if "rows_written" in b]
    if len(rw) == len(bridges):
        back = [k + 1 for k in range(len(rw) - 1) if rw[k + 1] < rw[k]]
        r.add("counters.bridge_rows_written_monotonic", not back, f"decreased before samples {indices(back)}")

    # --- delivery matched the arm at every interval, not just at the ends
    def deltas(key):
        return [pages[k + 1][1][key] - pages[k][1][key] for k in range(len(pages) - 1)]

    secs = span if span > 0 else 1.0
    d_rx, d_iq, d_rows = deltas("rxIq"), deltas("iq"), deltas("rows")
    kind = arm["kind"]
    if kind == "A":
        r.add("delivery.render_source", all(p["renderSource"] == "iq" and p["echoMode"] == "iq" for _, p in pages), "render source/echo not iq")
        r.add("delivery.raw_iq_advances_every_interval", all(d > 0 for d in d_rx), f"no raw RX IQ in intervals {indices(i for i, d in enumerate(d_rx, 1) if d <= 0)}")
        rate = (pages[-1][1]["rxIq"] - pages[0][1]["rxIq"]) / secs
        r.add("delivery.raw_iq_rate", DISPLAY_FPS[0] <= rate <= DISPLAY_FPS[1], f"{rate:.1f} frames/s, required {DISPLAY_FPS[0]:.0f}-{DISPLAY_FPS[1]:.0f}")
        r.add("delivery.no_rows", all(d == 0 for d in d_rows), "spectrum rows arrived in a raw IQ window")
        r.add("delivery.page_streaming", all(p["iqStreaming"] for _, p in pages), "page not streaming IQ")
        r.add("delivery.bridge_subscriptions", all(b["iq"] == 1 and b["audio"] == 1 for _, b in bridges if "iq" in b), "Bridge IQ/audio subscription not 1/1")
        delivery["rate"] = rate
    elif kind == "B":
        r.add("delivery.render_source", all(p["renderSource"] == "server" and p["echoMode"] == "spectrum" for _, p in pages), "render source/echo not server/spectrum")
        r.add("delivery.rows_advance_every_interval", all(d > 0 for d in d_rows), f"no rows in intervals {indices(i for i, d in enumerate(d_rows, 1) if d <= 0)}")
        rate = (pages[-1][1]["rows"] - pages[0][1]["rows"]) / secs
        r.add("delivery.row_rate", DISPLAY_FPS[0] <= rate <= DISPLAY_FPS[1], f"{rate:.1f} rows/s, required {DISPLAY_FPS[0]:.0f}-{DISPLAY_FPS[1]:.0f}")
        r.add("delivery.no_raw_iq", all(d == 0 for d in d_rx) and all(d == 0 for d in d_iq), "raw IQ arrived in a rows window")
        r.add("delivery.page_streaming", all(p["iqStreaming"] for _, p in pages), "page not streaming")
        r.add("delivery.bridge_subscriptions", all(b["iq"] == 1 and b["audio"] == 1 for _, b in bridges if "iq" in b), "Bridge IQ/audio subscription not 1/1")
        delivery["rate"] = rate
    else:  # C: audio only
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

    # --- audio is running at all (a window with no audio is not an audio measurement)
    played = (pages[-1][1]["audioPlayed"] - pages[0][1]["audioPlayed"]) / secs
    opus = (pages[-1][1]["opusFrames"] - pages[0][1]["opusFrames"]) / secs
    r.add("audio.running", played >= AUDIO_FLOOR_FPS and opus >= AUDIO_FLOOR_FPS,
          f"played {played:.1f}/s, Opus frames {opus:.1f}/s, required at least {AUDIO_FLOOR_FPS:.0f}/s")
    delivery["played"] = played

    # --- TCP_NODELAY: label, arm, and the retained Bridge log must all agree
    label = meta["bridgeNoDelay"]
    r.add("nodelay.label_matches_arm", label == arm["nodelay"], f"label {label}, arm {meta['arm']} requires {arm['nodelay']}")
    # UNVERIFIED is not a failed check: it is reported in the window's nodelay field and decides exit status 3.
    if nodelay_state["status"] != "UNVERIFIED":
        # The label/log comparison itself lives in read_nodelay_evidence (one place, not two).
        r.add("nodelay.log_matches_label", nodelay_state["status"] == "VERIFIED", nodelay_state["why"])

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
        "queue_ms_min_med_max": [round(min(q)), round(statistics.median(q)), round(max(q))] if q else None,
        "bridge_audio_dropped_s_max": max((b["audio_dropped_s"] for _, b in bridges if "audio_dropped_s" in b), default=None),
        "bridge_outbound_drops_delta": (bridges[-1][1].get("outbound_drops", 0) - bridges[0][1].get("outbound_drops", 0)) if "outbound_drops" in bridges[0][1] else None,
        "jitter_p95_ms_last": last.get("jitterP95"),
    }
    sp = w["start"].get("page", {}) if isinstance(w.get("start"), dict) else {}
    startup = {k: sp.get(k) for k in ("underruns", "overflows", "drops")}
    delivery.update(render=last["renderSource"], codec=last["codec"], format=[last["codec"], last["rate"], last["channels"], last["worklet"]], rows_s=round((last["rows"] - first["rows"]) / secs, 1),
                    rx_iq_s=round((last["rxIq"] - first["rxIq"]) / secs, 1))
    delivery["bridge_iq_audio"] = f'{bridges[-1][1].get("iq")}/{bridges[-1][1].get("audio")}'
    return r, perf, startup, delivery


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
            out.append((int(m.group(1)), m.group(2), f, json.load(open(f))))
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
    ap.add_argument("--expect-codec", default="opus")
    ap.add_argument("--require-rxc1", action="store_true", help="require a recorded RXC1 state per window (always on in the live profile)")
    ap.add_argument("--order", default=",".join(ORDER), help="comma-separated planned arm order (order mode)")
    args = ap.parse_args(argv)
    if not os.path.isdir(args.windows_dir):
        print(f"check_order: not a directory: {args.windows_dir}", file=sys.stderr)
        return 2
    prof = dict(PROFILES[args.profile])
    prof["expect_codec"] = args.expect_codec
    if args.require_rxc1:
        prof["require_rxc1"] = True
    windows = load_windows(args.windows_dir)
    if not windows:
        print("check_order: no w<N>_<ARM>.json records found", file=sys.stderr)
        return 2
    log_dirs = args.bridge_logs + [args.windows_dir, os.path.join(args.windows_dir, "..", "bridge-logs")]

    results, set_checks = [], Checks()
    served = {}
    for _, _, _, w in windows:
        n = w.get("meta", {}).get("bridgeRestartNumber") if isinstance(w, dict) else None
        served[n] = served.get(n, 0) + 1
    for idx, arm_name, path, w in windows:
        if "_unreadable" in w:
            c = Checks()
            c.add("record.readable", False, w["_unreadable"])
            results.append(dict(n=idx, arm=arm_name, file=os.path.basename(path), checks=c, perf={}, startup={}, delivery={}, nodelay={"status": "UNVERIFIED", "why": "unreadable record"}))
            continue
        meta = w.get("meta", {})
        nd = read_nodelay_evidence(meta.get("bridgeRestartNumber"), served.get(meta.get("bridgeRestartNumber"), 1), meta.get("bridgeNoDelay"), log_dirs) \
            if all(k in meta for k in ("bridgeRestartNumber", "bridgeNoDelay")) else {"status": "UNVERIFIED", "why": "meta lacks restart number or label", "logged": None}
        c, perf, startup, delivery = check_window(w, prof, nd)
        c.add("record.file_name_matches_meta", meta.get("arm") == arm_name and meta.get("index") == idx, f"file w{idx}_{arm_name} vs meta {meta.get('index')}/{meta.get('arm')}")
        results.append(dict(n=idx, arm=arm_name, file=os.path.basename(path), checks=c, perf=perf, startup=startup, delivery=delivery, nodelay=nd, meta=meta))

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
        shas = {m.get("bridgeSha256") for m in metas}
        set_checks.add("set.one_bridge_binary", len(shas) == 1 and None not in shas, f"Bridge binary hashes {sorted(map(str, shas))}")
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
        c_nd = {r["meta"].get("bridgeNoDelay") for r in results if r["arm"] == "C" and r.get("meta")}
        set_checks.add("set.arm_c_has_nodelay_on_at_both_ends", c_nd == {1} if c_nd else False, f"arm C labels {c_nd}")

    any_invalid = bool(set_checks.failed()) or any(r["checks"].failed() for r in results)
    any_unverified = any(r["nodelay"]["status"] == "UNVERIFIED" for r in results)

    # --- report
    rows = []
    for r in results:
        failed = r["checks"].failed()
        p, d, st = r["perf"], r["delivery"], r["startup"]
        rows.append({
            "n": r["n"], "arm": r["arm"], "evidence": "INVALID" if failed else "valid", "nodelay": r["nodelay"]["status"],
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
    for c in set_checks.failed():
        notes.append(f"SET FAILED {c['check']}: {c['detail']}")
    verdict = "INVALID EVIDENCE" if any_invalid else ("VALID, TCP_NODELAY UNVERIFIED" if any_unverified else "VALID, TCP_NODELAY VERIFIED")
    perf_flags = [f"window {r['n']} ({r['arm']}): " + ", ".join(f"{k} {v}" for k, v in r["perf"].items() if k in ("underruns", "overflows", "drops", "audio_gaps", "audio_resyncs", "decode_errors", "late_drops") and v)
                  for r in results if r["perf"] and any(r["perf"].get(k) for k in ("underruns", "overflows", "drops", "audio_gaps", "audio_resyncs", "decode_errors", "late_drops"))]
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
                                "performance": r["perf"], "startup_counters": r["startup"], "delivery": r["delivery"]} for r in results]},
                  fh, indent=1, default=str)
    if any_invalid:
        return 1
    if any_unverified and not args.allow_unverified_nodelay:
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
