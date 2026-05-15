# Saturn Remote Phase 37 Analysis Gate

Phase 37 is adaptive raw-IQ rate selection, but it must not guess thresholds.
The implementation gate is the Phase 36 measurement matrix.

## Required Inputs

Before adaptive-rate code is written, capture:

- LAN normal client at 48, 96, and 192 kHz.
- VPN normal client at 48, 96, and 192 kHz.
- LAN 192 kHz with one deliberately slow client.
- VPN 192 kHz with one deliberately slow client.

Each session needs both:

- Browser `SaturnRemotePerf` summary or sample JSON.
- Host capture log from `saturn-remote-phase36-capture.sh`.

## Analyzer

Run:

```bash
/home/pi/github/Saturn/update_manager/scripts/saturn-remote-phase37-analyze.mjs \
  ~/Documents/perf-captures/phase36 \
  --out ~/Documents/perf-captures/phase36/phase37-analysis.md
```

Use strict mode in CI or before declaring Phase 37 ready:

```bash
/home/pi/github/Saturn/update_manager/scripts/saturn-remote-phase37-analyze.mjs \
  ~/Documents/perf-captures/phase36 \
  --out ~/Documents/perf-captures/phase36/phase37-analysis.md \
  --strict
```

Strict mode exits non-zero when the matrix is incomplete or any required bar
fails.

## Pass/Fail Bars

The analyzer enforces:

- Safety enqueue-to-write p99 <= 5 ms.
- Control enqueue-to-write p99 <= 20 ms.
- Safety queue depth overflow count equals 0.
- Slow-client bridge IQ processing rate remains within +/-2% of the matching
  no-slow-client 192 kHz baseline.

It also reports, but does not automatically fail:

- Display replacements and drops.
- Display frames skipped by the bridge FPS cap.
- Audio drops and panic drains.
- Browser and bridge audio sequence gaps.
- Send-blocked milliseconds.
- Outbound high-watermark bytes.
- TCP out-queue high-watermark bytes from bridge host logs.
- Bridge RTT maxima.

## Threshold Derivation

Once the matrix is complete and passing, Phase 37 can set adaptive-rate
thresholds from the report:

- Downstep only after sustained bad control p99, RTT, send-blocked time, or
  display replacement pressure.
- Upstep only after at least twice the clean duration.
- Keep safety/control thresholds as hard invariants, not tuning knobs.
- Publish the active display transport state separately from the control/TX
  path.

Until the analyzer says the matrix is complete, Phase 37 remains blocked on
data collection.
