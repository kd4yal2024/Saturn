# Saturn Performance Lab

The dedicated `Saturn Performance Lab` tab on `/saturn/telemetry` turns Saturn
optimization into a repeatable change-validation loop. Live service controls
and dashboards remain on the adjacent `Telemetry & Diagnostics` tab:

```text
CHANGE -> BENCHMARK -> MEASURE -> COMPARE -> ACCEPT / REVIEW / REJECT
```

It is deliberately a **no-regression benchmark**, not an RF instrument. An
`ACCEPT` verdict means that the candidate met the current software and data-path
gates under the same recorded workload. It does not prove an improvement in
sensitivity, RMDR, phase noise, IMD, or spectral purity.

## Run contract

1. Start the desired Protocol 2 client and establish a stable receive workload.
2. Keep band, sample rate, receiver count, wideband state, client display shape,
   and radio routing unchanged.
3. Give the run a useful name and record the implementation, FPGA image, or
   setting being tested.
4. Choose the warm-up and measurement window. The default is 6 seconds of
   warm-up followed by 60 seconds of measurement.
5. Record the operator observation, including subjective results such as
   `Sounded clean` or `tuning remained responsive`.
6. Save a baseline before the change, repeat the same workload after the change,
   and compare the candidate to the baseline.

The browser will not start until it can fingerprint the P2app executable. Saturn
Go reads `/proc/<pid>/exe` where host permissions allow it; hardened appliances
fall back to the effective systemd `ExecStart` path, restricted to Saturn's
managed deployment roots. It aborts a run if the service PID, application
telemetry, active-radio state, or workload identity changes. A minimum of five
valid samples is required.

## Stored evidence

Each run includes:

- timestamp, duration, sample interval, and sample count;
- Saturn Go build commit and the exact running P2app binary SHA-256 digest;
- application, backend, startup mode, panel mode, and workload key;
- change notes and operator observation;
- min, mean, p95, and max for host and radio data-path metrics;
- accumulated application, FIFO, network, and ADC-overflow events.

The persistent history is stored atomically at:

```text
/var/lib/saturn-state/performance_benchmarks.json
```

The file is limited to the newest 64 runs and is included in Saturn settings
backup/restore and managed-state migration.

## Current metrics

- process CPU as a percentage of one core;
- RSS memory;
- scheduler delay and context-switch rate;
- Ethernet packet and bit rates;
- XDMA interrupts per second and interrupts per MiB;
- DDC/DUC packet and DMA rates;
- average DDC/DUC DMA operation size;
- SoC temperature and CPU frequency;
- ADC1/ADC2 peak dBFS when available.

## Verdict rules

Runs must have the same backend, selected application, and workload key. A
mismatch is `INCOMPATIBLE`, not a pass or failure.

The initial gates are intentionally conservative:

- any candidate application, FIFO, network, or ADC-overflow event is `REJECT`;
- DDC packet throughput below 97% of baseline is `REJECT`, while 97-98.5% is
  `REVIEW`;
- CPU, scheduler delay, RSS, and XDMA IRQ/MiB use relative and absolute
  tolerances to avoid overreacting to measurement noise;
- scheduler-delay p95 is evaluated separately from its mean.

Every verdict includes the individual checks. `REVIEW` is used when a result is
not a hard regression but the tradeoff needs engineering judgment.

## Recommended test discipline

- Run at least two baselines to estimate normal variation before evaluating a
  change.
- Use 60 seconds for routine software changes and 5 minutes for scheduling,
  DMA, or sustained-throughput work.
- Keep the radio and client stationary during the measurement window.
- Compare like-for-like ambient and RF conditions when ADC peaks matter.
- Preserve bench-instrument results separately for RF specifications that Saturn
  Go cannot measure directly.
