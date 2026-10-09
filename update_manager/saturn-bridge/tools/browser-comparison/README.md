# Browser comparison: window driver and fail-closed checker

Tools for the matched browser → proxy → AudioWorklet comparison of display transport and the Bridge's
`TCP_NODELAY`. Nothing here changes the Bridge, the page, the proxy or the firmware, and nothing here runs on a
radio host by itself. The local rehearsal stack (`order.sh`) uses the replay Bridge (`../replay/`), a scratch
build of the proxy and a headless Chrome, all on loopback.

| File | Purpose |
|---|---|
| `check_order.py` | Decides, for every window, whether the **evidence** is valid, separately from how the audio **performed**. |
| `test_check_order.py` | 86 tests: a complete valid ten-window set (the real rehearsal in `fixtures/`) with one alteration at a time. |
| `window.mjs`, `cdp.mjs` | One window in a real Chrome over the DevTools protocol. Writes its record on success **and on failure**. |
| `order_lib.sh` | Status bookkeeping: a failed window never stops the run but always fails it. |
| `order.sh` | The planned order on the local stack, then the checker. Needs the environment listed in its header. |
| `test_order_status.sh` | Tests for `order_lib.sh` and for `window.mjs`'s failure record. No browser needed. |
| `fixtures/rehearsal-2026-10-09/` | The ten preserved windows and the five Bridge logs they were taken with (copies of the rehearsal evidence). |

## Evidence is not performance

`check_order.py` answers two different questions and never mixes them.

**EVIDENCE: is this window a valid measurement of its arm?** A window is INVALID if any of these fails, and is never
repaired by dropping samples:
- every sample is complete: the browser's telemetry fields and the Bridge's, with a failed read of `perf.json` counting as a failure;
- the Bridge's telemetry is fresh at every sample: its own timestamp never goes back, does not freeze for more than two
  consecutive samples, advances with the window, and (same-clock profile only) is no older than 3 s at a sample;
- the browser stayed connected at every sample; the wire codec (`opus`), rate, channels and worklet mode did not change;
  the display override matches the arm;
- no counter ever went backwards (a reset or reload);
- delivery matched the arm **at every interval**, not only at the ends: raw IQ for A, rows for B, neither for C, with
  the Bridge's IQ/audio subscriptions as expected, no `iq_stop` in a display window, and for C a single `iq_stop`, a
  drain before the first sample and no restart after it;
- audio is running at all (at least 10 played and Opus frames per second);
- the sampling cadence held (no hole over 3 s, no interval outside 0.5–2 s) and the window had the planned length;
- `TCP_NODELAY` matches the arm (A/B-on and C on, A/B-off off) **and the retained Bridge logs** (see below);
- the record carries no `failure` from the driver.

**PERFORMANCE: what did the audio do?** Underruns, overflows, drops, sequence gaps, resyncs, decode errors, late drops,
stalled seconds, queue lead, jitter, Bridge-side drops. These are **reported, never used to reject a window**: a valid
window with real underruns is a result. Startup counters (what the page had already counted when the window began) are
shown separately from the window's deltas.

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
| 3 | all valid, `TCP_NODELAY` UNVERIFIED for at least one window |

`order.sh` combines this with its windows' own statuses: any failed window gives 1, even if the checker is content.
Failed records are kept; the checker only reads them and writes `check_report/order_check.json` and `order_summary.txt`.

## Limits, fixed before any live window is evaluated

`rehearsal` is the 30 s local rehearsal; `live` is the planned ten-minute window. They are constants in
`check_order.py` (`PROFILES`), not options:

| | rehearsal | live |
|---|---|---|
| window length | 30 s, span 27–32 s, ≥ 28 samples | 600 s, span 595–605 s, ≥ 595 samples |
| Bridge age check (needs one clock) | yes | no (browser and Pi clocks differ) |
| RXC1 state recorded and constant | no | yes |

Common: sampling 1 Hz (0.5–2 s allowed), holes over 3 s rejected, display 24–36 frames/s, audio floor 10 frames/s,
Bridge timestamp unchanged for at most 2 consecutive samples. The rehearsal limits were set from the design (1 Hz
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
`perf.json`; a live driver needs its own source), and it does not replace the acquisition owner's RXC1 procedure.
