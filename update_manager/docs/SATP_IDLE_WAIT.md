# SATP idle receiver wait

The receiver waits on UDP readiness when media is missing/lost and no SATP
output is authorized. It rechecks controls, lease state and status within a
10 ms poll interval, plus scheduler delay. An arriving datagram wakes the wait
immediately. Authentication, packet ownership and the receive timestamp remain
in the single existing receiver loop.

An eventfd allocated at startup provides a separate shutdown wakeup. Runtime
drop sets stop, signals that descriptor, and joins the receiver. The descriptor
is nonblocking and closes with its owned handle. Disabled SATP allocates no
wakeup descriptor. Readiness errors are logged and use a bounded 10 ms fallback.
This is an idle-wakeup change: active playout keeps the existing 250 us poll,
512-frame default jitter target, rate control and audio-loss/dekey handling.

## Checks and measurements

Unit checks cover UDP readiness without consuming/authenticating a packet,
bounded waits with no input, and coalesced shutdown notifications independent
of network traffic. Existing authenticated replay/loss handling remains covered.
Real-WDSP production-runtime fixtures exercise SATP pairing/renewal/source
switching, startup failure cleanup, and release/re-arm/active shutdown through
P2 and Direct XDMA file/loopback backends.

The ignored `satp::tests::actual_receiver_idle_and_restart_profile` fixture runs
the production SATP receiver loop and runtime destructor. It measures thread CPU
during three seconds of idle, then receives three authenticated 120-packet
bursts at 48 kHz with deliberate 150 ms gaps. It records first-packet observation,
steady-input missing samples, total missing samples, overruns, resyncs and joined
shutdown time. The deliberate gaps include synthesized silence before the
100 ms fixture loss timeout; they are separate from steady-input loss. No output
is authorized and no RF, native microphone or physical device is used.

Run the fixture serially after compilation, with no overlapping build/profile:

```sh
cd update_manager/saturn-bridge
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --locked --bin saturn-bridge --no-run
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --locked --bin saturn-bridge \
  actual_receiver_idle_and_restart_profile -- --ignored --nocapture --test-threads=1
```

Stub-fixture CPU/timing is an offline comparison, not a production latency
qualification. Actual G2 standby or receive CPU is measured separately with the
committed `saturn-thread-profile.py` v2 collector, retaining workload transitions,
priority, affinity and before/after counters. Its cumulative process-lifetime
histograms are not measurement-window p99 values. Compare matched workloads and
retain unresolved audio/continuity/deadline/cleanup observations explicitly.

## ARM loopback evidence, 14 November 2026

Four interleaved runs per version used normal CFS priority and the same default
unoptimized test harness on the G2. Each version received 1,440 authenticated
packets in 12 bursts, with zero steady-input missing samples, overruns or resyncs.

| Measurement | Baseline | Idle readiness wait |
| --- | --- | --- |
| Idle thread CPU, median (one core = 100%) | 9.71% | 0.58% |
| Idle thread CPU, observed range | 5.32–9.73% | 0.33–0.61% |
| First packet observation, median / sample max | 1.384 / 1.670 ms | 1.257 / 1.758 ms |
| Joined shutdown, median / sample max | 0.600 / 0.661 ms | 0.396 / 0.456 ms |

There are only 12 first-packet observations and four shutdown observations per
version. These are sample medians/maxima, not p99 or physical key/release times.
The candidate's largest first-packet observation is 0.088 ms above the baseline
maximum; this experiment does not establish a tail-latency improvement. CPU
frequency was not fixed and the observed variation is retained.

One earlier low-priority baseline fixture timed out on its first packet. Its
output is preserved alongside the successful normal-priority repeats; its cause
is unestablished. A passing repeat does not identify that cause. This slice also
does not close the display review's server-interactive/worker-2D underruns or
worker-3D p99 observation, and hosted CI availability remains separate work.

The evidence package is `Downloads/Saturn_SATP_Idle_Wait_20261114`. Installed
release standby/receive measurements and deployment results live there; they
are distinct from this debug-fixture comparison.
