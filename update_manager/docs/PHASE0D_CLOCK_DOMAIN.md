# Phase 0D — Clock-Domain Hardening

**Authority:** `Saturn_Precision_SDR_Architecture_v1.1` §89 (D10), §70 Phase 0D, PR 01c.
**Scope:** host/bridge/web software only (no FPGA builds are possible on this project).
**Verified against:** pinned WDSP 2.00 @ `584e8ac` (`rmatch.c/h`), branch tip after Phase 0B.
**Implementation status:** host-side Items 1–3 implemented; hardware soak acceptance remains open.

Policy (D10): paths crossing independent clocks are explicitly rate-matched; the local
codec path and the PureSignal loop are synchronous and stay SRC-free.

## Item 1 — Browser mic → WDSP TX: `rmatch` on ingest

**Problem:** browser mic frames arrive on the browser's WebAudio clock; the TX thread
clocks 512-sample blocks into WDSP from the Pi's monotonic clock. A constant ppm offset
drains or grows the jitter queue until underrun-fill or drop-oldest — no FIFO depth fixes
a rate mismatch (§89.3).

**Design (verified from `rmatch.c`):**

- `create_rmatchV(in_size=64, out_size=512, 48000, 48000, ringsize, var=1.0)` between
  mic-frame arrival and the WDSP block filler. Mono is fed as complex `[s, 0]`; the I
  channel is taken from the output.
- **Ring:** initialized half-full with silence and servo-target is half-full
  (`n_ring = rsize/2`, `deviation = n_ring − rsize/2`), so added latency ≈ `ringsize/2`.
  `ringsize = 2 × tx_mic_prefill_samples` keeps the operator's existing prefill knob as
  the latency control (default 2048 → ring 4096 ≈ 85 ms, ~43 ms nominal latency).
- **Underflow is graceful:** `xrmatchOUT` always yields a full block — it slews to
  silence (`dslew`), counts an underflow, and up-slews when data returns. The legacy
  hold-last/zero fill is therefore skipped in rmatch mode; WDSP's slews replace it.
- **Adaptation startup:** the 3 s `startup_delay` gates only the control loop
  (`control_flag`); audio flows immediately at ratio 1.0. Drift is ppm-scale, so 3 s of
  nominal ratio is inconsequential.
- **Re-arm flush:** there is no flush API, but `setRMatchRingsize(ptr, same_size)` stops
  the matcher, runs `decalc_rmatch`/`calc_rmatch` (full state rebuild, ring re-primed
  half-full), and restarts — a complete reset in ~10 ms. Called on every Arm, plus
  `resetRMatchDiags`.
- **Prefill gate bypassed in rmatch mode:** the pre-primed half-ring of silence is the
  warm-up; `mic_output_started` is set at Arm.
- **Hardware status:** controlled A/B testing on direct XDMA showed materially more DUC
  FIFO underflows with rmatch enabled. The implementation remains available for further
  diagnosis but is opt-in; the accepted legacy queue stays the production default.

**Unchanged invariants:** the TX source-stall watchdog stays keyed to mic-frame
*arrival* (B1 INV-4 — rmatch output never refreshes liveness); two-tone stays
self-sourced; the PS loop and DUC path gain no SRC.

**Diagnostics (§89.3 item 4):** nominal rates, measured `var` ratio, `nring`/ringsize,
underflow/overflow counts exposed in the periodic TX diag line and telemetry.

The installed bridge defaults to the legacy queue. For a controlled rmatch A/B run:

```bash
sudo systemctl edit saturn-bridge.service
```

Add this drop-in, then restart the bridge through the normal backend owner switch:

```ini
[Service]
Environment=SATURN_BRIDGE_TX_MIC_RMATCH=1
```

Remove the drop-in to return to the production legacy path. A live rmatch diagnostic
looks like:

```text
mic_rmatch=1 rmatch_underflows=0 rmatch_overflows=0 rmatch_var=1.000000000 rmatch_ring=2048/4096
```

## Item 2 — Bridge → DUC FIFO: occupancy-driven pacing — VERIFIED PRESENT

The direct-XDMA backend already paces by measured FIFO occupancy: occupancy-seeded
prefill with fault detection (`xdma_tx_radio.rs`, "production DUC occupancy prefill"),
largest-safe-partial-batch writes, and FIFO snapshot decoding (`xdma_duc.rs`). The
P2-network backend intentionally paces by wall clock — that is the P2 protocol's model
(remote clients do the same), and the receiving p2app owns the hardware FIFO. No code
change; this item is discharged by audit.

Hardware A/B testing exposed a host-side starvation source adjacent to that pacing:
each browser mic media frame was treated like a model-changing TCI control and caused a
full radio-state publication. The 200 ms keyed watchdog heartbeat also produced a full
snapshot after its dedicated pong. Those batches commonly occupied about 7 ms and
exceeded 10 ms before an observed DUC FIFO underflow. Direct XDMA now forwards mic
media without taking the radio-model lock or publishing state, and heartbeat pings send
only their dedicated pong. The intentional one-second S-meter/full-state convergence
request and batches containing actual control commands retain the existing behavior.

## Item 3 — Bridge → browser RX audio: adaptive playback resampler

**Problem:** `remote-web/src/audio/resample.ts` uses a static ratio; over long sessions
the browser's playback clock drifts against the bridge's 48 kHz, accumulating buffer
growth (latency creep) or starvation (dropouts).

**Design:** mirror rmatch's servo in TypeScript at the playback buffer: measure queued
audio (occupancy), target a setpoint (half the nominal buffer), and trim the resample
ratio with a small proportional feedback (rmatch uses 4.0e-06 per-sample gain and
0.003 s slews — the same order applies), slew-limited so corrections are inaudible.
Expose occupancy, applied ratio, underruns, and correction count through the existing
`rx-telemetry.ts` reporting. Static behavior must remain available as a fallback flag.

The browser implementation uses a 42 ms queue target, ±2,000 ppm correction bound,
and a 500 ppm/s slew limit. Fractional output frames carry between packets so small
clock corrections are not rounded away. Open Saturn Remote with
`?rx_audio_adaptive=0` to run the unchanged static-ratio path for an A/B comparison.
The copied **RX Latency** diagnostic and Performance Lab snapshots include the current
ratio, signed ppm correction, correction count, target queue, underruns, and overflows.

## TX hardware result — accepted 2026-08-29

Production bridge PID `3451260` started with `mic_rmatch=0`. Hardware validation on a
dummy load completed ten short keyed cycles followed by one continuous transmission
from 22:50:25 through the 180-second safety limit at 22:53:24. The bridge logged
`TX watchdog timeout (180s), auto-unkeying` and returned to receive normally.

- All 45 sampled mic diagnostics reported `underruns=0`; after startup the legacy mic
  queue stayed bounded at approximately 8,256–9,152 samples.
- DUC output advanced continuously through packet 142,917. The final keyed XDMA status
  reported `tx_fifo_faults=0`, TX FIFO low/high watermarks 647/3,620, RX header errors
  and resyncs both zero, SWR 1.00, and no RF-safety output fault.
- No `DUC IQ send error`, TX source stall, output fault, or forced-receive event occurred
  during the short-cycle or continuous-TX tests. Operator audio was reported clean and
  more responsive during the controlled hardware validation.

The production TX portion of Phase 0D is accepted. Adaptive browser RX soak acceptance
remains open.

## RX hardware result — rejected 2026-08-30

A 53-minute direct-XDMA browser RX soak was rejected rather than counted toward Item 3
acceptance. Saturn Remote had abnormally low receive gain, while the same radio and
antenna returned to normal receive level under P2/Thetis. Source audit found that the
direct backend wrote the RX-state ALEX word at `0xB000` without any ANT1/ANT2/ANT3
selection bits and did not program the RX band-pass-filter word at `0xB004`.

The source-only correction now:

- programs the P2-compatible RX BPF at the 1.5/2.1/5.5/11/22/35 MHz boundaries;
- maps the Saturn Remote RX selector to distinct ANT1/ANT2/ANT3 relay bits and preserves
  that selection across retunes and TX/RX transitions; and
- keeps the independently validated transmit path locked to ANT1.

Hardware validation of this correction is still pending. Four Pi resets occurred during
the investigation: the first while switching ownership from direct XDMA to P2, and later
resets while attempting raw XDMA register snapshots. The prior-boot journal was not
retained (`Storage=volatile`), so no reset cause is claimed. Do not repeat raw
`/dev/xdma0_user` snapshots or count a new RX soak until the ownership transition can be
observed with persistent crash evidence.

## Acceptance

- TX rmatch remains rejected as a production default until a later A/B soak can run
  without DUC FIFO faults. Its diagnostics and explicit opt-in remain for investigation.
- Production legacy mic path, with mic-only state publication removed: at least ten
  short keyed cycles and one continuous voice transmission through the production
  180-second TX-watchdog limit without a DUC FIFO fault. The watchdog deliberately
  caps TX at 180 seconds, so a five-minute continuous test is not possible without
  weakening an RF-safety invariant.
- Browser: overnight RX soak shows bounded playback occupancy, no latency creep, no
  audible artifacts at correction points.

## Hardware acceptance procedure

1. Select the Saturn Bridge backend and confirm the journal startup line contains
   `mic_rmatch=0`.
2. With a dummy load, perform at least ten short MOX/PTT cycles and one continuous
   voice transmission until the production 180-second watchdog safely auto-unkeys.
   Confirm normal key/dekey behavior and no new RF safety fault.
3. During TX, capture the periodic diagnostic fields:

   ```bash
   sudo journalctl -u saturn-bridge.service --since "10 minutes ago" --no-pager \
     | grep -E 'TX thread started|TX state|TX diag mic|TX watchdog timeout|xdma status|DUC IQ send error|TX source stall|TX output fault|forced receive'
   ```

4. Leave RX audio active for at least 30 minutes, then use **Operations → RX
   Latency → Copy**. Queue occupancy should remain bounded around the target and the
   ratio must remain inside 0.998–1.002.
5. Only for a controlled diagnostic A/B, set `SATURN_BRIDGE_TX_MIC_RMATCH=1` and repeat
   the short cycles. Do not accept it as the default unless it is at least as reliable
   as the production path. The browser `?rx_audio_adaptive=0` flag independently tests
   RX playback and does not control TX rmatch.

No step in this phase changes or rebuilds FPGA firmware; direct-XDMA DUC pacing is an
audit of the existing host implementation only.
