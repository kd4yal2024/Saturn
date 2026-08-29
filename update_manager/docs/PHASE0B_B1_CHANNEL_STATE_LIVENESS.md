# Phase 0B — B1: WDSP Channel-State Restoration and TX Liveness Separation

**Status:** Design spec — ready for implementation
**Authority:** `Saturn_Precision_SDR_Architecture_v1.1` §67 (defect B1 + mandatory acceptance test), §70 Phase 0B, §111
**Scope:** `update_manager/saturn-bridge` only. No FPGA, codec, or WDSP-source change.
**Verified against:** branch `phase0b-bridge-correctness` @ `e1cd7ab` (= origin/main `b8e70c1` + XDMA hardening/P3 retirement)

---

## 1. Verified current behavior

| # | Behavior | Location |
|---|---|---|
| C1 | RX WDSP channel starved during TX: `push_iq` is skipped entirely while `tx_active` | `src/rx_thread.rs:207` |
| C2 | TX WDSP channel is never transitioned to state 0; `set_active(false)` only clears buffers. Comment documents the workaround for the "zero TX IQ after first MOX" bug | `src/wdsp.rs:1597–1602` |
| C3 | Unkey ordering is `set_puresignal_mox(false)` → `set_active(false)` — correct `SetPSMox(0)`-first ordering already exists in the one unkey path | `src/tx_thread.rs:1302` (`do_unkey`) |
| C4 | Mic-silence keepalive fills zeros **unboundedly**: after `TX_SILENCE_GAP` (250 ms) the block filler substitutes 0.0 forever; WDSP keeps producing IQ; the DUC FIFO keeps being serviced | `src/tx_thread.rs:846–864, 886–890` |
| C5 | Watchdog (a): control liveness on `saturn_ping`, `TX_CONTROL_WATCHDOG_SPLIT_LIMIT` = 1500 ms | `src/main.rs:57` |
| C6 | Watchdog (b): maximum-transmit, wall clock since `tx_armed_at`, `SATURN_BRIDGE_TX_WATCHDOG` 3–180 s | `src/tx_thread.rs:808` |
| C7 | Watchdog (c): FPGA FIFO-service watchdog (FW V20) cancels TX after ~2 s without DUC FIFO service | FPGA firmware, not bridge code |

**Consequence of C4:** if the browser mic stream dies while MOX is held and the control
WebSocket stays alive, nothing dekeys the radio until watchdog (b) — up to 180 s of
zero-modulation carrier-capable TX. C1/C2 violate WDSP Guide §3.3 (continuous data flow,
`SetChannelState` transitions); C2's workaround exists because a state-0 down-slew was
starved of samples (the Guide's documented ~200 ms wait/timeout failure mode).

## 2. Design

### 2.1 TX channel lifecycle (`wdsp.rs::set_active`)

**Arm (unchanged):** clear buffers → `SetChannelState(ch, 1, 0)`.

**Unkey (new):**

```text
SetPSMox(ch, 0)                        (caller already does this; set_active(false)
                                        re-asserts it defensively — INV-1)
SetChannelState(ch, 0, 0)              (state 0, no wait)
feed SLEW_FLUSH_BLOCKS zero-input fexchange0 blocks at normal cadence
discard all output produced by those blocks (INV-2)
channel quiescent until next arm
```

- **`dmode = 1` (blocking wait) is forbidden** — verified in §3: single-threaded it
  always times out (~100 ms) and takes WDSP's force-reset path, which skips the buffer
  flush and is the mechanical cause of the C2 bug.
- `SLEW_FLUSH_BLOCKS` values are derived in §3 item 4 from the verified slew times and
  block sizes: TX = 8 blocks, RX = 16 frames, with sentinel-verified completion and
  `recreate_channel` fallback.
- This removes the C2 workaround **and** the bug it papered over: the down-slew now
  completes because it is fed, so the next `SetChannelState(ch, 1, 0)` starts clean.

### 2.2 RX channel during MOX (`rx_thread.rs`)

Replace the C1 skip with explicit state transitions:

```text
MOX assert:   SetChannelState(rx_ch, 0, 0)
              keep feeding arriving DDC frames through fexchange0 for the
              down-slew window; discard audio output
MOX release:  SetChannelState(rx_ch, 1, 0)
              resume live IQ feeding and audio publication
```

- Preserves the intent of the existing comment (rx_thread.rs:204–206): TX energy never
  drives RX AGC/audio — the channel is *stopped*, not fed TX energy. Zero-feeding a
  running channel is rejected: long MOX would slew AGC gain to maximum and pop on unkey.
- TCI display IQ publication (`rx_thread.rs:203`) is independent of WDSP and unaffected (INV-3).
- The S-meter (`smeter_dbm`) will hold its last value during MOX; acceptable.

### 2.3 Liveness separation (the safety half)

**INV-4 (from arch doc §67 B1, verbatim intent):** WDSP block cadence is not a TX-liveness
signal. Zero blocks, slew-flush blocks, settle blocks (`settle_wdsp_tx`), keepalive fill,
and routine `fexchange0` calls must never refresh any liveness signal or watchdog.

Liveness inputs are exactly:

| Signal | Watchdog | Refreshed by | Cannot be refreshed by |
|---|---|---|---|
| `saturn_ping` control progress | (a) 1500 ms | validated control messages | WDSP cadence — different subsystem, safe by construction |
| wall clock since `tx_armed_at` | (b) 3–180 s | nothing (monotonic) | anything, safe by construction |
| DUC FIFO service | (c) FPGA ~2 s | real DUC writes | *currently defeated by C4* — fixed by the stall watchdog below |
| **NEW: real mic frame arrival** | **(d) TX source stall** | arrival of browser mic frames (arrival, not audio level) | zero-fill, keepalive, two-tone, any synthetic block |

**New watchdog (d) — TX source stall:**

- Track `last_real_mic_frame_at` = wall time of last mic-frame **arrival** in the
  `TxCommand` mic path. Silence with frames still arriving does NOT trip it (an operator
  pausing speech keeps the stream alive); only transport stall does.
- If `state != Idle && !two_tone && last_real_mic_frame_at.elapsed() > TX_SOURCE_STALL_LIMIT`
  → run the existing `do_unkey` path, log reason `TX_SOURCE_STALL`, bump telemetry counter.
- Default `TX_SOURCE_STALL_LIMIT` = 2000 ms (same order as FPGA watchdog (c)), env
  override `SATURN_BRIDGE_TX_SOURCE_STALL_MS`, clamped to [500, 10000].
- Two-tone/tune mode is exempt (self-sourced, no mic dependency); the exemption is a
  deliberate, documented carve-out — watchdogs (a)–(c) still bound it.
- `TX_SILENCE_GAP` (250 ms) keepalive is retained as the **only** deliberate
  silence-bridging behavior, now bounded above by (d).

**Watchdogs (a), (b), (c) are not modified in any way.** Their limits, trigger sources,
and dekey paths are unchanged; the acceptance tests prove it.

### 2.4 Invariants (testable)

- **INV-1:** `SetPSMox(ch, 0)` strictly precedes every `SetChannelState(ch, 0, _)` (WDSP Guide §6.3.20 NOTE).
- **INV-2:** No `fexchange0` output produced while the bridge TX state is `Idle` — including slew-flush output — is ever forwarded to the DUC. (Existing Idle gating at `tx_thread.rs:898–908` retained; flush output explicitly discarded in `wdsp.rs`.)
- **INV-3:** Display IQ and HP packet handling are unaffected by RX channel state.
- **INV-4:** No synthetic WDSP input/output refreshes any liveness signal (grep-auditable: the four liveness variables are written only from validated-frame / control / clock sources).
- **INV-5:** Every dekey path (a)–(d) lands in the existing `do_unkey` → HP `tx_enabled=false` burst → RX DDC restore sequence. No new dekey mechanism is invented.

## 3. Pre-implementation verification — COMPLETED (pinned WDSP 2.00 @ `584e8ac`, checkout `/home/pi/github/OpenHPSDR-wdsp`)

**V1 findings (`channel.c:259–297`, `iobuffs.c:400–517`):**

1. **State-0 mechanism:** `SetChannelState(ch, 0, dmode)` sets `slew.downflag` +
   `flushflag` and returns. Subsequent `fexchange0` calls run `downslew0` on the output;
   when the down-slew completes, `fexchange0` itself clears the channel `exchange` flag and
   releases `Sem_Flush` (`iobuffs.c:498–503`). A dedicated per-channel **`flushChannel`
   thread** (spawned in `create_iobuffs`, `iobuffs.c:422`) then takes both critical
   sections, runs `flush_iobuffs` (zeroes both rings, resets every index/counter,
   recreates `Sem_OutReady`) + `flush_main`, sets `exec_bypass`, and clears `flushflag`.
   **The graceful path leaves the channel fully reset by WDSP itself.**
2. **`dmode=1` is always degraded here:** the wait loop (`Sleep(1)` × 100, ~100 ms+) can
   only be satisfied by `fexchange0` calls the caller itself would make. Single-threaded,
   it always times out and takes the force-reset path (`channel.c:280–286`), which clears
   `exchange`/`flushflag`/`downflag` **without** running `flush_iobuffs` — ring indexes and
   `r2_havesamps` accounting are left mid-stream, and `Sem_OutReady` counts go stale. This
   is the mechanical cause of the C2 "zero TX IQ after first MOX" bug. `dmode=0` without
   continued feeding is equally broken: `downflag` stays set, so the next state-1's output
   keeps running `downslew0` to zero. **Conclusion: `dmode=0` + fed flush is the only
   correct pattern in this architecture. `dmode=1` is forbidden.**
3. **Stopped channel:** `fexchange0` with `exchange` clear is a complete no-op — it
   returns `error = 0` **without touching the output buffer** (`iobuffs.c:471`). The
   bridge must therefore never consume the output buffer after the flush completes
   (stale data, no error indication); detection below.
4. **Flush block counts** (OpenChannel args verified in `wdsp.rs`: `tdelaydown = 0.0`,
   `tslewdown = 0.010`, `bfo = 1` on both channels):
   - **TX:** down-slew = 0.010 × 192 000 = 1920 output samples; out_size per call =
     512 mic in → 2048 IQ out, so the slew completes within ~1 call; use
     `SLEW_FLUSH_BLOCKS_TX = 8` (pipeline depth `DSP_MULT` margin included, cap bounded).
   - **RX:** down-slew = 0.010 × 48 000 = 480 audio samples; out_size ≈ 64 per call →
     8 calls; use `SLEW_FLUSH_BLOCKS_RX = 16` frames of arriving DDC IQ.
   - `bfo = 1` means flush-phase calls self-pace on `Sem_OutReady` — no artificial
     real-time pacing needed; the flush completes in milliseconds of wall time. Cap the
     feed loop and fall back to `recreate_channel` (existing code path) if the sentinel
     check below fails.
5. **Completion detection:** pre-fill the output buffer with a sentinel before each
   flush-phase call; when a call returns with the buffer untouched and `error == 0`, the
   exchange flag has cleared → flush signaled. Log and hard-recover via
   `recreate_channel` if this never happens within the capped block count.

**V2 finding:** after the flush, `exec_bypass` is set and the buffers are pristine;
`SetChannelState(ch, 1, 0)` clears `exec_bypass`, sets `upflag`, and re-enables exchange —
no priming needed. The up-slew (`tdelayup = 0.010`, `tslewup = 0.025`) ramps the first
~35 ms of output automatically.

**Build-time check (implementation task):** verify the staged piHPSDR `linux_port.h`
(pinned `974acba`, staged by `scripts/build-wdsp2-linux-arm.sh`) maps `_beginthread` →
pthread so the `flushChannel` thread exists on ARM. If it does not, the graceful flush
never completes and this design must be revisited before merge.

## 4. Acceptance tests (arch doc §67 B1 — all mandatory before merge)

All RF-inhibited (`rf_enabled=false`) or into dummy load. Results recorded in the Performance Lab.

| # | Test | Expected |
|---|---|---|
| T1 | MOX keyed, control WS alive, mic stream killed (mute/suspend the browser track) | Bridge dekeys at `TX_SOURCE_STALL_LIMIT` (~2 s), reason `TX_SOURCE_STALL`, radio in RX |
| T2 | MOX keyed, control channel killed | Watchdog (a) fires at unchanged 1500 ms |
| T3 | MOX held past `SATURN_BRIDGE_TX_WATCHDOG` | Watchdog (b) fires at unchanged limit |
| T4 | `SIGSTOP` the bridge while keyed | FPGA watchdog (c) cancels TX at ~2 s |
| T5 | ≥20 consecutive MOX key/unkey cycles | Every cycle produces non-zero TX IQ within the recorded first-IQ latency — regression guard for the C2 "zero TX IQ after first MOX" bug |
| T6 | Unkey after 60 s MOX | RX audio resumes promptly; no AGC pop/slow recovery (validates §2.2 stop-not-starve choice) |

T1 is the doc's mandatory "B1 stall test": continuous zero feeding of WDSP must not
extend any of (a)/(b)/(c), and each must fire at its unchanged limit.

## 5. Telemetry additions

`tx_source_stall_count`, `tx_source_stall_limit_ms`, `tx_slew_flush_blocks`,
`rx_wdsp_channel_state`, dekey reason string in the existing TX diag events.

## 6. Out of scope

B2–B8 (§67 table — separate work package), FPGA changes, PureSignal internals, codec
paths, mic-prefill/DUC-pacing retune (done after B2's CFIR removal, on hardware).
