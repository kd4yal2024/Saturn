# Phase 0B Acceptance Runbook — T1–T6 Hardware Pass

**Operator guide.** Spec: `PHASE0B_B1_CHANNEL_STATE_LIVENESS.md` §4. Everything here is
RF-inhibited or into a dummy load — never an antenna.

## 0. Prerequisites

### 0.1 Deploy the branch build

The deployed `/opt/saturn-go/bin/saturn-bridge` predates the Phase 0B branch. Deploy the
branch build first (from `update_manager/saturn-bridge/`):

```bash
cargo build --release                        # links target-local libwdsp.a
sudo systemctl stop saturn-bridge
sudo cp /opt/saturn-go/bin/saturn-bridge /opt/saturn-go/bin/saturn-bridge.pre-phase0b
sudo cp target/release/saturn-bridge /opt/saturn-go/bin/saturn-bridge
sudo systemctl start saturn-bridge
```

Rollback at any time: copy the `.pre-phase0b` file back and restart.

### 0.2 Environment

- Radio Backend selector → the bridge stack (Saturn Remote), not plain p2app.
- Dummy load connected, or keep RF disabled in the client for every test except where noted.
- Two terminals on the Pi:
  - **A (watch):** `journalctl -u saturn-bridge -f`
  - **B (commands):** for the steps below.
- Saturn Remote open in the browser with mic permission granted and MOX working.

New knobs (systemd override via `sudo systemctl edit saturn-bridge` if needed):
`SATURN_BRIDGE_TX_SOURCE_STALL_MS` (500–10000, default 2000),
`SATURN_REMOTE_TX_WATCHDOG_SECS` (3–180, default 180),
`SATURN_BRIDGE_SMETER_CAL_DB` (±60, default 0).

## T1 — Mic-source stall (the mandatory B1 stall test)

Proves synthetic zero-fill cannot hold TX when the real mic source dies.

1. Key MOX with live mic audio; confirm TX spectrum moves.
2. **Kill the mic without touching the control link:** mute the mic at the OS level,
   revoke the tab's mic permission, or unplug a USB mic. Do NOT close the tab or drop
   the network — that is T2's stimulus.
3. Watch terminal A.

**PASS:** within ~2.5 s of mic loss:
```
saturn-bridge: TX source stall (2000ms without mic frames), auto-unkeying (TX_SOURCE_STALL, count=1)
saturn-bridge: TX state -> OFF
```
Radio lands in RX and stays there. **FAIL:** TX held on zero-fill beyond ~3 s.

Note: silence into a *live* mic must NOT trip this — speak nothing for 10 s while keyed
first and confirm TX stays up (keepalive fill is the intended behavior); only frame
*arrival* stopping trips the stall.

## T2 — Control-link stall

1. Key MOX with live mic.
2. Kill the control path abruptly: disconnect the client machine's network, or
   `kill -9` the browser process. (This also stops mic frames — the control watchdog's
   1.5 s limit is shorter than the 2 s stall limit, so control fires first; seeing the
   control line and not the stall line is itself evidence of correct ordering.)

**PASS:** within ~2 s:
```
saturn-bridge: TX control watchdog <n>ms > 1500ms; forcing RX
```
Radio in RX. Limit unchanged at 1500 ms (split-lane).

## T3 — Maximum-transmit watchdog

1. `sudo systemctl edit saturn-bridge` → add `Environment=SATURN_REMOTE_TX_WATCHDOG_SECS=10`,
   then `sudo systemctl restart saturn-bridge`.
2. Key MOX with continuous live audio and hold past 10 s.

**PASS:** at 10 s:
```
saturn-bridge: TX watchdog timeout (10s), auto-unkeying
```
3. Remove the override (`sudo systemctl edit saturn-bridge`, delete the line) and restart.

## T4 — FPGA FIFO-service watchdog (RF-INHIBITED MANDATORY)

Proves the hardware backstop below the bridge still fires.

1. Confirm RF is disabled in the client (bridge stages DUC data but no RF keying).
2. Key MOX. In terminal B: `sudo kill -STOP $(pgrep -x saturn-bridge)`
3. Wait 5 s: the FPGA (FW V20) must cancel TX ~2 s after FIFO service stops — verify on
   the radio's state/front panel that it dropped out of TX.
4. `sudo kill -CONT $(pgrep -x saturn-bridge)` — expect watchdog/unkey messages as the
   bridge catches up; the session must recover to a working RX (re-key once to confirm).

**PASS:** hardware dekey ~2 s into the freeze; clean recovery after CONT.

## T5 — Repeated-key regression (guards the original zero-TX-IQ bug)

The P2 backend does NOT recreate the WDSP channel per arm, so every cycle exercises the
new fed-flush stop path — this is the direct regression test for the bug the old code
worked around.

1. 20 cycles: key with speech 3–5 s → unkey 2 s.
2. Every keyed period must show live TX IQ (moving TX spectrum / output peak in the
   diag lines), including cycles 2–20.

**PASS:** 20/20 cycles transmit real IQ; the journal shows NO occurrences of:
```
saturn-bridge: TX channel down-slew flush did not complete
```
(one occurrence means the fallback saved the cycle but the flush needs investigation —
record it as a finding either way). **FAIL:** any keyed period with dead/zero TX IQ.

## T6 — Unkey recovery

1. Key MOX for 60 s (speech or silence), then unkey.
2. RX audio must return promptly (≤ ~1 s), at normal level, without an AGC surge, pop,
   or slow fade-in. (B1 stops the RX channel across MOX instead of starving it; this
   test catches both regressions.)

**PASS:** clean, prompt RX recovery after a long transmission.

## Results

Record per test in the Performance Lab notes: date, bridge binary (git commit + file
date), backend mode, pass/fail, measured times, and the exact journal lines. Phase 0B's
exit criterion (arch doc §67) additionally wants the repeated-key dummy-load harness and
the RF-inhibited five-cycle acceptance recorded, plus mic-prefill re-measurement
(`SATURN_BRIDGE_TX_MIC_PREFILL_MS`) since CFIR removal changed TX latency.

| Test | Pass | Time observed | Journal line captured | Notes |
|---|---|---|---|---|
| T1 |  |  |  |  |
| T1-silence (no trip) |  |  |  |  |
| T2 |  |  |  |  |
| T3 |  |  |  |  |
| T4 |  |  |  |  |
| T5 (n/20) |  |  |  |  |
| T6 |  |  |  |  |
