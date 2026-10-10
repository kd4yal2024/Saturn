# Browser comparison: window driver and fail-closed checker

Tools for the matched browser → proxy → AudioWorklet comparison of display transport and the Bridge's
`TCP_NODELAY`. Nothing here changes the Bridge, the page, the proxy or the firmware, and nothing here runs on a
radio host by itself. The local rehearsal stack (`order.sh`) uses the replay Bridge (`../replay/`), a scratch
build of the proxy and a headless Chrome, all on loopback.

| File | Purpose |
|---|---|
| `check_order.py` | Decides, for every window, whether the **evidence** is valid, separately from how the audio **performed**. |
| `test_check_order.py` | 182 tests: a complete valid ten-window set (the real rehearsal in `fixtures/`) with one alteration at a time, plus collected sets and synthetic live windows. |
| `owner_reader.py`, `test_owner_reader.py` | The read-only reader of the acquisition owner's cached telemetry, meant to run **on the Pi** (22 tests). |
| `collector.mjs`, `test_collector.mjs` | The bounded client of that reader: timeouts, respawn, a sidecar that keeps every raw answer (28 tests). |
| `window.mjs`, `cdp.mjs` | One window in a real Chrome over the DevTools protocol. Writes its record on success **and on failure**. |
| `order_lib.sh` | Status bookkeeping: a failed window never stops the run but always fails it. |
| `order.sh` | The planned order on the local stack, then the checker. Needs the environment listed in its header. |
| `test_order_status.sh` | 26 checks for `order_lib.sh` and for `window.mjs`'s failure record and collector switch. No browser needed. |
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

`piSourceAgeMs` is produced by the owner reader (below) and enforced here; the field names are a **draft** for the collector's
owner to confirm or rename (one constant each in `check_order.py` and `collector.mjs`).

## The owner collector (contract v1, draft)

For a live comparison the Bridge's telemetry has to come from the **existing acquisition owner's cached document**, read on the
Pi, with the Pi's own clock and the owner's process identity. Nothing here starts, restarts or writes to the owner.

- `owner_reader.py` runs on the Pi (standard library only; sent as one program argument over SSH, it installs nothing). It reads the
  cached `perf.json` (default `/run/saturn-bridge/perf.json`; **the active path must be verified on the host**) and `/proc`. It never
  writes, never opens an XDMA device, and its only subprocess is an optional `systemctl show ... -p MainPID`. It returns the
  document exactly as parsed (never edited, never refreshed), its SHA-256, the document's own update time, the Pi-side age
  (`piReadAtMs - piSourceUpdatedAtMs`, both on the Pi's clock), and the owner's identity: pid, process start ticks, boot id, the
  SHA-256 of the **running** executable image (read through `/proc/<pid>/exe`, so a replaced file cannot pass for it), and whether
  the service's MainPID matches. An unreadable, stale or malformed document is reported as it is: unavailable, or its true age.
  Python 3.7 or newer. **Precondition to verify on the host:** the reader hashes the owner's running image through
  `/proc/<pid>/exe`, which Linux lets only the owner's user (or root) read. If the Bridge runs as a different user than the account
  the reader runs under, `exeSha256` comes back empty with an `exeError` and the window is INVALID
  (`owner.fields_well_formed` says why); the fix is to run the reader as that user, not to weaken the check. Nothing here has run on a G2,
  so this has not been checked there.
- `collector.mjs` keeps one reader process per window, asks it once per sample, bounds every request (2 s; 10 s when the request had
  to start the reader), kills and replaces a hung or dead reader, and appends every answer **and every failure** to
  `OUT.collector.jsonl` the moment it arrives, so the raw evidence survives a timeout, a crash or a closed browser. It maps an answer
  into the sample's `bridge` record with the same function for every arm and every source. A failure is an error record; nothing is
  ever replaced by zero. Switch: `SATURN_CMP_COLLECTOR=local` (the same file, on this machine) or `ssh` (with
  `SATURN_CMP_COLLECTOR_HOST`, optional `_PATH` and `_SERVICE`); unset keeps the original direct file read.

**What the checker requires** (rules in `check_order.py`; evidence is in play when any sample carries a collector field or a sidecar
exists, and then **all** apply; with none of it the live profile, or `--require-owner`, reports **UNVERIFIED**, other runs
report `NOT COLLECTED`):

| Rule | Meaning |
|---|---|
| `owner.fields_every_sample`, `owner.fields_well_formed` | every collector field present and well-typed (hashes 64-hex lowercase, counters non-negative integers, finite numbers) |
| `owner.alive_every_sample`, `owner.constant_in_window` | the owner was alive, and one `(pid, start ticks, boot id, running-image hash)` for the whole window |
| `owner.pid_matches_meta`, `owner.exe_matches_meta` | the owner is the window's `bridgePid`, and its running image is the executable the window names |
| `owner.service_main_pid` | when the reader could ask systemd, the service's MainPID is the document's pid |
| `collector.latency_bounded`, `collector.paired_with_page_sample` | each request took at most 2 s, and its answer arrived within 3 s after its page sample (both on the browser's computer) |
| `collector.seq_strictly_increasing`, `collector.pi_read_time_advances` | every sample is a fresh read: sequence numbers and the Pi's read time strictly increase |
| `collector.pi_clock_tracks_window` | over the window the Pi's clock advanced as far as the page's (within 3 s). Durations only: the two clocks are never compared |
| `bridge.pi_age_arithmetic`, `bridge.pi_updated_matches_document` | the age is the Pi's read time minus the document's own update time, and that update time is the sample's `updatedAtMs` |
| `rxc1.pid_matches_owner` | (live) the RXC1 state in `meta.rxc1` was recorded for this owner process (`meta.rxc1.pid` is defined as the owner's pid) |
| `collector.sidecar_retained`, `collector.sidecar_seq_unique`, `collector.sidecar_complete` | the sidecar exists, is strict JSON with exactly one start entry, repeats no sequence number, and agrees with the record's own read and failure counts |
| `collector.sidecar_entry_for_every_sample`, `collector.samples_match_raw_documents` | every sample is backed by a successful raw answer, and every field of the sample equals what that raw answer says |
| `set.owner_identity_every_window`, `set.owner_identity_per_bridge_instance`, `set.owner_new_process_per_restart`, `set.owner_same_boot` | all windows are collected or none; one process serves one Bridge instance; each restart is a different process; no reboot during the comparison |

A record the checker cannot interpret (wrong types in `meta`, a URL that is not a string, frames that are not objects, ...) is an INVALID
window with the reason (`record.meta_types`, `record.interpretable`) and the report is still written; it is never a crash.

A failed read inside the window is an error record and makes the window INVALID (`telemetry.every_sample_complete`); a failed
warm-up read, a respawn or a stray reader event is kept and reported as an `OUTCOME` without invalidating the window.

## TCP_NODELAY: Bridge-reported evidence first, retained logs second, else UNVERIFIED

The Bridge itself reports, in `perf.json`, `tci_nodelay_enabled` (the setting), `tci_nodelay_confirmed_total` (accepted sockets whose
option was set **and then read back as on** with `getsockopt`) and `tci_nodelay_failed_total` (sockets where either step failed or
read back off). The collector carries them as `nodelayEnabled`, `nodelayConfirmedTotal` and `nodelayFailedTotal`; a Bridge that
predates them yields no such fields (absent, never zero). The checker then requires: the setting equals the label; no failed
socket; the confirmed counter never goes backwards; with the setting on, at least as many confirmed sockets by the first sample
as windows this Bridge instance has served; with it off, none. Fields in only some samples, or not counters, contradict. If the
retained logs below also exist and contradict a Bridge that says it is verified, the window is CONTRADICTED.

## TCP_NODELAY from retained logs

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
| 3 | all valid, but `TCP_NODELAY` (neither Bridge-reported evidence nor retained logs) or, in the live profile, telemetry freshness (no Pi-side age) or the owner's process identity (no collector evidence) is UNVERIFIED for at least one window; each has its own `--allow-unverified-nodelay` / `-freshness` / `-owner` |

`order.sh` combines this with its windows' own statuses: any failed window gives 1, even if the checker is content.
Failed records are kept; the checker only reads them and writes `check_report/order_check.json` and `order_summary.txt`.

## Limits, fixed before any live window is evaluated

`rehearsal` is the 30 s local rehearsal; `live` is the planned ten-minute window. They are constants in
`check_order.py` (`PROFILES`), not options:

| | rehearsal | live |
|---|---|---|
| window length | 30 s, span 27–32 s, ≥ 28 samples | 600 s, span 595–605 s, ≥ 595 samples |
| Bridge age check | one clock: sample time minus the Bridge's update time | the Pi-side `piSourceAgeMs` only (two clocks are never compared) |
| RXC1 state recorded and constant (and recorded for the owner's pid) | no | yes |
| owner collector evidence | `NOT COLLECTED` unless present or `--require-owner` | required (UNVERIFIED without it) |

Common: sampling 1 Hz (0.5–2 s allowed), holes over 3 s rejected, Bridge timestamp unchanged for at most 2 consecutive
samples; collector request at most 2 s, answer within 3 s of its page sample, Pi clock within 3 s of the page's over a window. Performance flags use 24–36 display frames/s and 80% of the nominal 46.9 audio frames/s; they flag, they never reject. The rehearsal limits were set from the design (1 Hz
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

It does not judge whether anything improved and it does not replace the acquisition owner's RXC1 procedure. The owner reader
and collector have been exercised only against the replay Bridge on loopback: **nothing here has run against a G2**, the active
`perf.json` path on the host is unverified, and the installed firmware's Bridge predates the `tci_nodelay_*` metrics, so a live
run against it reports `TCP_NODELAY` UNVERIFIED unless the retained logs say otherwise. That is the honest answer, not a defect.
