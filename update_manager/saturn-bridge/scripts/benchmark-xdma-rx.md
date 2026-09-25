# Direct-XDMA RX performance candidate — 2026-09-22

## Scope and status

Candidate based on workstation `main` at
`7a0a6701b24f9db5b6014222da1e5d00667330a5`.
Local parser benchmark and stub-linked tests passed. This is **not** a G2
end-to-end qualification, a native WDSP release build, or a deployed update.
No FPGA image, P2 behavior, TX qualification, watchdog, tuning, or RF gate was
changed. No service was restarted and no raw XDMA device was opened on G2.

The running G2 binary was preserved before editing:

- Telemetry timestamp: `2026-09-22T23:59:05.934Z`, PID `206128`, V30, audio + IQ.
- Reported build: `a33debf004d6cf6ab6891a7c1c911c44aaf125b1`, dirty source.
- Installed binary and `/proc/206128/exe` both verified SHA-256
  `1a1b80a417338e7c9868f37d1eda2b379cc5ceccf23ece9cbbb457ba3965c8b7`.
- Local baseline capture: `/tmp/saturn-xdma-perf.ciuf0r/g2-before.json` and
  `g2-before.bin`. These temporary artifacts should be archived before deployment.

Follow-up reconciliation found the matching deployed source package at
`/home/pi/saturn-satp-preflight.YnkHgp`: its executable matches the installed
hash and all source-manifest entries verify. The baseline and optimization
merge are preserved at `/home/pi/saturn-xdma-rx-perf.IvETtd`. After normalizing
formatting, the merged Bridge `src/` matches the workstation candidate exactly;
Cargo.toml, Cargo.lock and build.rs also match. Existing staged Bridge features
are therefore represented in main. The ordinary G2 checkout is an older
checkpoint branch, not the source used for the running executable. Publish
the optimization commit and build from that exact GitHub revision before
deployment. This reconciliation did not build or install a native candidate.

## Changes

1. Parse by offset into the pending buffer; compact once on return, including
   error returns. Preserve exact signed-24-bit conversion, framing checks,
   power arithmetic, initial synchronization, partial tails, and discontinuity
   recovery. This removes repeated tail copies, not sample detail.
2. Give the dedicated C2H reader its own non-owning BAR register handle. Its
   `0x9000` reads do not acquire the control/snapshot mutex. The old shared lock
   covered up to 50 snapshot polls with 100-us sleeps between them, in addition
   to register I/O and scheduling. The coherent snapshot implementation is
   unchanged. This removes a software lock dependency, not all possible PCIe
   or kernel contention. There is still exactly one C2H reader. The owning
   control handle remains responsible for RF-safe cleanup.
3. Add fixed-storage timing histograms and a completion timestamp per host
   buffer. Recording does not allocate or log on the reader path.
4. Add `SATURN_BRIDGE_XDMA_RX_MIN_READ_BYTES`, accepting only 4096 (default),
   8192, or 16384. Larger reads remain opt-in. Invalid values fail before the
   RX session opens hardware. Catch-up can still read up to 32768 bytes, never
   more than the observed FIFO occupancy. Probe/P2 read policies are unchanged.

## Local before/after benchmark

Intel Core i7-13700, x86-64 WSL, Rust/Cargo 1.98.1, release profile. Same
benchmark fixture before/after: 37,748,736 input bytes, 524,288 complete frames,
4,194,304 IQ pairs per trial; median of three trials. Baseline is the source
commit above plus the benchmark test only. Candidate measurements completed
by `2026-09-23T00:08:08Z` (September 22 local time).

| Read chunk | Decode before | Decode after | Decode time reduction | Discard before | Discard after |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 31.508 ms | 28.398 ms | 9.9% | 13.056 ms | 6.808 ms |
| 8 KiB | 38.978 ms | 28.743 ms | 26.3% | 19.463 ms | 6.686 ms |
| 16 KiB | 52.121 ms | 29.147 ms | 44.1% | 30.640 ms | 6.741 ms |
| 32 KiB | 73.912 ms | 29.544 ms | 60.0% | 54.998 ms | 6.818 ms |

All trials are preserved in
[the raw TSV](benchmark-xdma-rx-20260922.tsv). The full local output logs are
`/tmp/saturn-xdma-perf.ciuf0r/parser-before.log` and `parser-after.log`.
These are parser CPU measurements, not ARM, driver, WDSP, audio-latency, or
total-Bridge CPU results. The benchmark does not include operational timing
instrumentation overhead. There was no statistical confidence study or CPU
affinity isolation. Do not choose a hardware batch size from this table alone.

Run from `update_manager/saturn-bridge` on an idle development machine:

```bash
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --all-targets
SATURN_BRIDGE_STUB_NATIVE=1 cargo check --all-targets
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --release benchmark_ddc_parser -- --ignored --nocapture
```

Native stubs are only for tests. **Never install a stub-linked executable on
the radio.** Do not run this CPU benchmark during a live quality soak.

Local checks: 305 tests passed, zero failed, two intentionally ignored
benchmarks. The parser benchmark was run explicitly and passed separately.
All-target compilation, formatting of changed Rust files, and `git diff
--check` passed. Clippy completed with existing repository warnings; it is
not a warning-clean build. Regression coverage includes signed sample edges,
varied DMA chunk boundaries, power/peak accounting, discard/decode behavior,
fatal headers after valid frames, incomplete frames, discontinuity recovery,
default read-policy equivalence, histogram bounds, independent register
access under a held control lock, and the existing firmware/RF safety tests.

## Timing metrics

Each prefix below exports `_count`, `_max_us`, and `_p99_upper_us` under
`perf.json`'s `metrics`. Durations round up to microseconds. Histograms use
power-of-two buckets: `p99_upper_us` is an upper bound, not an exact percentile.
Zero count means no observations, not a zero-latency measurement.

| Prefix | Scope | Measurement |
| --- | --- | --- |
| `rx_dma_read_session` | RX session | Successful DMA syscall duration |
| `rx_dma_completion_gap_session` | RX session | Between successful C2H completions; includes polling, fill time, scheduling, locks, and DMA |
| `rx_host_queue_age_session` | RX session | DMA completion to consumer dequeue; not end-to-end RF/audio latency |
| `rx_parser_session` | RX session | Per-buffer decode or discard parser call |
| `rx_iq_publish_interval` | Reporting interval | IQ packetizer + local TCI enqueue per input block, including blocks not yet emitting a frame; not socket delivery |
| `rx_dsp_audio_interval` | Reporting interval | WDSP processing + local audio enqueue per input block; excludes resume flush and socket delivery |
| `rx_control_batch_interval` | Reporting interval | Nonempty control batches, including model synchronization/publication; excludes the subsequent slow-batch log |
| `rx_snapshot_interval` | Reporting interval | Extended FIFO/ADC sampling, including failed attempts; excludes telemetry file writes |

Session histograms reset only with a new RX session. Interval histograms reset
after each existing performance report (normally one second). Retain every
report when comparing bursts; a later quiet interval can otherwise hide a
control spike. `rx_dma_min_read_bytes` records the effective threshold.
Queue-age samples cover consumed buffers only; consult drop/discontinuity
counters as well, because reclaimed buffers are not represented in that
histogram. Session histograms span all processing modes used in that session.

Current coherent `fifo_v29_occupancy_*` must remain distinct from FPGA
boot-lifetime minima/maxima/aggregate event transitions and host session
high-water marks. Compare counter deltas; an old FPGA maximum is not proof of
a new failure. Existing fault handling is not suppressed or cleared here.

## G2 comparison still required

At the current DDC6 384-ksps packing, each frame contains eight IQ pairs plus
one header, each using an 8-byte word: 3,456,000 bytes/s. Nominal values are:

| Minimum read | Steady reads/s | Block fill time |
| --- | ---: | ---: |
| 4096 | 843.75 | 1.185 ms |
| 8192 | 421.875 | 2.370 ms |
| 16384 | 210.9375 | 4.741 ms |

These are calculations, not measured syscall or latency improvements.
The 16,384-word FIFO holds about 37.93 ms at this packing/rate. Larger minimum
reads trade fewer syscalls for additional buffering latency. The 8-MiB host
pool is allocated capacity: 256 queued 4-KiB payloads contain only 1 MiB of
actual data, not 8 MiB. Measure age, not just buffer count.

1. Preserve the installed binary, source provenance, service environment,
   native WDSP archive hash, profile, and initial counters. Identify the dirty
   running build's source before replacing it. Use matching source/dependency
   baselines so unrelated changes cannot masquerade as a performance gain.
2. Build a real pinned-WDSP candidate separately. Build with the same native
   dependencies and target CPU flags for both comparison binaries.
3. During an agreed RX-only test window, use the existing transactional radio
   owner/service path. Do not launch a second Bridge or raw-device reader.
   Test baseline and candidate with the same firmware, antenna/input, full
   saved client profile, viewport, FFT/rates, and audio/IQ demand. No PTT/TX.
4. Start with candidate default 4096. Then test 8192 and 16384 by changing only
   `SATURN_BRIDGE_XDMA_RX_MIN_READ_BYTES` in the existing owner's environment.
   Confirm `rx_dma_min_read_bytes` in telemetry after each fresh RX session.
   Retain default 4096 unless CPU and latency results justify another value.
5. For each variant capture at least 60 seconds idle, 10 cold Remote connects,
   tuning/mode/filter/AGC/audio start-stop/reconnect exercises, then a 30-minute
   384-ksps RX soak for the chosen candidate. Record CPU/thread load, read rate,
   DMA/completion-gap/queue-age tails, control and DSP timings, IQ/audio delivery
   counters, current occupancy, FIFO-event deltas, host drops/discontinuities,
   DMA/header errors, exits, and WebSocket disconnects. Use owner telemetry;
   add owner-side higher-cadence occupancy capture if sub-second snapshots are
   needed. Do not poll read-to-clear registers from an external script.
6. Require no new FIFO overflow/underflow, DMA/framing failure, host drop,
   discontinuity, Bridge exit, or unexpected connectivity loss, continuously
   advancing IQ/audio, unchanged identity/routing, and acceptable audio latency.
   Parser speed alone does not constitute PASS. Keep the baseline available
   for recovery. TX validation remains a separate authorized dummy-load test.
