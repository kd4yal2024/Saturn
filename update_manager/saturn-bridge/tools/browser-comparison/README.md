# Browser comparison: window driver and fail-closed checker

Tools for the matched browser → proxy → AudioWorklet comparison of display transport and the Bridge's
`TCP_NODELAY`. Nothing here changes the Bridge, the page, the proxy or the firmware, and nothing here runs on a
radio host by itself. The local rehearsal stack (`order.sh`) uses the replay Bridge (`../replay/`), a scratch
build of the proxy and a headless Chrome, all on loopback.

| File | Purpose |
|---|---|
| `check_order.py` | Decides, for every window, whether the **evidence** is valid, separately from how the audio **performed**. |
| `test_check_order.py` | 113 tests: a complete valid ten-window set (the real rehearsal in `fixtures/`) with one alteration at a time. |
| `window.mjs`, `cdp.mjs` | One window in a real Chrome over the DevTools protocol. Writes its record on success **and on failure**. |
| `order_lib.sh` | Status bookkeeping: a failed window never stops the run but always fails it. |
| `order.sh` | The planned order on the local stack, then the checker. Needs the environment listed in its header. |
| `test_order_status.sh` | Tests for `order_lib.sh` and for `window.mjs`'s failure record. No browser needed. |
| `fixtures/rehearsal-2026-10-09/` | The ten preserved windows and the five Bridge logs they were taken with (copies of the rehearsal evidence). |

## Evidence is not performance

`check_order.py` answers two different questions and never mixes them.

**EVIDENCE: is this window a valid measurement of its arm?** It is INVALID only if the *measurement* failed, never because
the thing measured went badly. A window is INVALID if any of these fails, and is never repaired by dropping samples:
- every sample is complete and every number is **finite** (NaN and infinity are rejected, including `1e999`; the JSON is
  loaded strictly, so a `NaN` token makes the record unreadable); a failed read of `perf.json` is a failure;
- the Bridge's telemetry is fresh at every sample (see "Freshness" below);
- the browser stayed connected; the wire codec (`opus`), rate, channels and worklet mode did not change; the display
  override matches the arm; the decoded-frame counters agree with the codec (frames of the other codec must not advance);
- no cumulative counter ever went backwards: the browser's counters and the Bridge's `rows_written` and `outbound_drops`
  (the Bridge's `*_s` fields are rates and are not monotonic);
- the **transport is the arm's**: the right render source and Bridge subscriptions at every sample and nothing of the wrong
  kind (rows in a raw IQ window, raw IQ in a rows window, IQ or rows in an audio-only window); arm C has one `iq_stop`, a
  drain before the first sample and no restart after it; audio was **started** (cumulative frames counted);
- the sampling cadence held and the window had the planned length; the planned order and one Bridge binary for the set;
- every window carries a valid 64-hex lowercase SHA-256 for the Bridge executable (empty or malformed is invalid, even if
  all ten are equal);
- `TCP_NODELAY` matches the arm and the retained Bridge logs (see below); the driver recorded no failure.

**PERFORMANCE: what did the transport and the audio do?** Throughput and stalls (display frames or rows per second, seconds
with none, a display that delivered nothing, audio rate, stalled audio seconds, "slow" audio), underruns, overflows, drops,
sequence gaps, resyncs, decode errors, late drops, queue lead, jitter, Bridge-side drops, startup counters. **Reported, never
used to reject a window.** A display pause, a stream that slows or stops, or a window full of underruns is a result: once the
subscription, the selected mode and the start are established, bad delivery is exactly what is being measured. Genuine
nonzero fault counts stay valid when the record is otherwise sound. The expected display rate (24 to 36 per second) and a
nominal audio rate are only used to set flags in the performance section.

**A disconnect is kept and reported as its own outcome.** The page being disconnected at a sample makes the window
INVALID for a matched comparison, but the window is preserved and the report lists `OUTCOME: transport disconnected at
samples ...` for it, with the performance seen in the rest of the window.

## Freshness

Same-clock profile (the local rehearsal): the Bridge's timestamp may not be older than 3 s at a sample, may not go back,
may not repeat more than twice in a row, and must advance with the window.

Live profile (browser and Pi are different computers): advancing timestamps cannot prove a feed fresh (a clock offset and an
old replay look identical), and the two clocks are never compared. Freshness rests only on **an age computed on the Pi, from
the Pi's own clock and the cached document's own update time**, carried in every sample as `bridge.piSourceAgeMs`:
- present in every sample and within -50 to 3000 ms: freshness VERIFIED;
- stale, or present in only some samples: the window is INVALID (`bridge.pi_source_age_every_sample`);
- absent: freshness is **UNVERIFIED**, never VALID: exit status 3 (`--allow-unverified-freshness` accepts it for the exit
  status; the report still says UNVERIFIED).

`piSourceAgeMs` is the one field the checker enforces; its name is a draft that the live collector's owner may rename (it is
a single constant to change). The rest of the collector's contract is not enforced here and belongs to whoever builds it:
the original cached document and its update time, the Pi-side read time, the collector's receive time and request latency,
the owner's PID and start identity, the executable hash, the firmware identity and receiver/audio settings, and the
actual RXC1 state tied to that process; an unavailable or stale read must be a failure record, never a zero or a refreshed
timestamp, and a blocked request must be bounded.

## TCP_NODELAY comes from retained logs, or is UNVERIFIED

For each window the checker reads `bridge_<N>.out` and `bridge_<N>.err` for the window's Bridge instance
(`meta.bridgeRestartNumber`) from `--bridge-logs`, the windows directory, or `../bridge-logs`. The `.out` must contain
the Bridge's start-up line `TCP_NODELAY on accepted TCI sockets: on|off`; for "on" the `.err` must hold at least one
`Bridge setsockopt(TCP_NODELAY=1) on fd N -> ok` per window served and no failure; for "off" no such call. Anything
inconsistent is CONTRADICTED (invalid). Missing or incomplete logs are **UNVERIFIED**, never a pass: exit status 3,
unless `--allow-unverified-nodelay` (which still prints UNVERIFIED).

## Exit status

| Status | Meaning |
|---|---|
| 0 | every window valid, `TCP_NODELAY` VERIFIED for all |
| 1 | invalid evidence (a window, the set, or a contradicted label) |
| 2 | usage error or unreadable input |
| 3 | all valid, but `TCP_NODELAY` (logs not retained) or, in the live profile, telemetry freshness (no Pi-side age) is UNVERIFIED for at least one window |

`order.sh` combines this with its windows' own statuses: any failed window gives 1, even if the checker is content.
Failed records are kept; the checker only reads them and writes `check_report/order_check.json` and `order_summary.txt`.

## Limits, fixed before any live window is evaluated

`rehearsal` is the 30 s local rehearsal; `live` is the planned ten-minute window. They are constants in
`check_order.py` (`PROFILES`), not options:

| | rehearsal | live |
|---|---|---|
| window length | 30 s, span 27–32 s, ≥ 28 samples | 600 s, span 595–605 s, ≥ 595 samples |
| Bridge age check | one clock: sample time minus the Bridge's update time | the Pi-side `piSourceAgeMs` only (two clocks are never compared) |
| RXC1 state recorded and constant | no | yes |

Common: sampling 1 Hz (0.5–2 s allowed), holes over 3 s rejected, Bridge timestamp unchanged for at most 2 consecutive
samples. Performance flags use 24–36 display frames/s and 80% of the nominal 46.9 audio frames/s; they flag, they never reject. The rehearsal limits were set from the design (1 Hz
sampling, 1 Hz telemetry) and then checked against the preserved windows, which they accept; they were looked at, not
fitted to pass.

**RXC1.** For the live profile (or `--require-rxc1`) each window's `meta.rxc1` must be an object with non-empty
`state`, `pid` and `identity`, recorded by the acquisition owner (the *actual* armed/valid-poll status, not the
environment flag), and `state` must be identical in every window.

## Running the tests

```sh
python3 tools/browser-comparison/test_check_order.py
bash tools/browser-comparison/test_order_status.sh
```

Re-check the preserved rehearsal:

```sh
python3 tools/browser-comparison/check_order.py tools/browser-comparison/fixtures/rehearsal-2026-10-09/windows \
  --bridge-logs tools/browser-comparison/fixtures/rehearsal-2026-10-09/bridge-logs --report-dir /tmp/check-report
```

## What this does not do

It does not judge whether anything improved, it does not know the G2's live telemetry (the driver reads a local
`perf.json`; a live driver needs its own source), and it does not replace the acquisition owner's RXC1 procedure. Its
`TCP_NODELAY` evidence reader recognizes the replay shim's log lines; production confirmation (the running build, the setting
and an actual socket confirmation retained with the owner's session) needs its own evidence adapter, and until one exists
the result for a live run is UNVERIFIED, which is the honest answer.
