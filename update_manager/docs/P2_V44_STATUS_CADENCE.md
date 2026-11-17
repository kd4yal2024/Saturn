# P2 v4.4 high-priority status cadence

Reference: TAPR **openHPSDR Ethernet Protocol v4.4**, source commit
[`09998920f4f6f1aff52261667b7efbdb46b34037`](https://github.com/TAPR/OpenHPSDR-Firmware/blob/09998920f4f6f1aff52261667b7efbdb46b34037/Protocol%202/Documentation/openHPSDR%20Ethernet%20Protocol%20v4.4.pdf).

## Sender change

The C P2 app previously requested 400 sleeps of 500 microseconds in RX, giving
a nominal 200 ms wait plus packet/diagnostic work. TX requested two sleeps,
giving 1 ms plus that work. The v4.4 periodic targets are 50 ms RX and 1 ms TX,
with earlier status on PTT/key input changes and ADC overflow.

The sender now uses CLOCK_MONOTONIC deadlines. Packet/diagnostic work consumes
the interval instead of being added to every following interval. Early input
events do not postpone the periodic deadline. When work misses periods, the
schedule advances to the next future deadline rather than bursting obsolete
status packets. Each session starts a fresh schedule.

Input polling retains requested sleeps of at most 500 microseconds. Each poll
also checks the committed atomic IsTXMode value, so client TX onset during an
RX wait switches cadence without waiting out the RX period. That atomic state
replaces an unsynchronized MOXAsserted read for cadence selection. A monotonic
clock failure follows the existing thread-error and socket-cleanup path.

Packet size, status fields, ADC peak accumulation, sequence and socket ownership
are unchanged. FPGA snapshots, temperature and JSON telemetry still run in the
same sender. This change establishes scheduling targets; it does not guarantee
wall-time deadlines, remove hardware/lock/scheduler delays or establish a CPU
saving. Moving slow diagnostics out of this sender is a separate performance
follow-up.

## Verification

The deterministic fixture compiles the actual OutHighPriority.c loop and
replaces hardware, monotonic clock, sleeps and send boundaries. It verifies
the actual packed packets for:

- 50 ms RX and 1 ms TX with nonzero diagnostic work.
- Early PTT/key and ADC events without shifting periodic cadence.
- Client TX onset during an RX wait.
- Recovery after slow work without a catch-up burst.
- Clock failure, thread error and cleanup.

Against the previous sender, that same fixture reproduced the RX defect:
the second packet arrived at synthetic 203 ms instead of the expected 50 ms
(200 ms sleeps plus 3 ms simulated work). This is a deterministic control-flow
regression, not a measured G2 latency result.

```sh
make -C sw_projects/P2_app test test-controller-lease-asan \
  test-protocol2-command-asan test-protocol2-high-priority-asan \
  test-protocol2-status-loop-asan
```

All ten normal C test groups and four sanitizer executions pass. The regular
control test depends on the status-loop fixture; CI also runs its sanitizer
target. The changed C files pass cppcheck performance/portability checks.
Evidence is retained in `Downloads/Saturn_P2_v44_Slice2_20261117` alongside
the full Rust and real-WDSP fixture results for the paired sequence repair.

No G2 service, FPGA, radio or live RF was accessed for this slice. An installed
check should retain received status timestamps, cadence distributions, CPU,
acquisition continuity and cleanup results before making performance claims.
The C discovery advertisement remains 4.3; accessory/VITA support is unchanged.
