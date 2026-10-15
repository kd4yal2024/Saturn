# PureSignal milestone 2 — production admission policy proposal

SilverForge, 2026-10-12. **Unreviewed proposal, offline execution only.**
This separates operating admission from measurement qualification; it supersedes
the previous report's recommendation that every adaptation block must have three
complete counter brackets, **only if this replacement policy is approved**.

## Outcome and unchanged boundary

Recommendation: admit verified, fresh, continuous paired feedback without requiring
ADC/DDC clear brackets on every adaptation block. Missing or uncertain overload
telemetry means **unclipped measurement qualification unavailable**, never clear.
Positive overload evidence and known continuity/freshness failures stop adaptation
and reset native correction. The native fitter still decides whether excitation
and fit quality are sufficient; admission is not a successful-correction claim.

The proposal is implemented behind the explicit, non-default Cargo feature
`offline-puresignal-policy-proposal`. It is exercised through both real adapters,
the shared controller, and pinned real WDSP. `Controller::new` retains the old
strict gate even with that feature compiled. Only the separately named
`new_offline_admission_proposal` selects the candidate; production has no caller,
UI enable control, native port, or acquisition-owner wiring for it. This is not
a production unlock. Default builds do not compile the alternative gate.

No G2 connection, RF testing, register/DMA access, deployment, FPGA, kernel-driver,
or polling change occurred. Keep firmware **1.31.002 / 0x53460004** installed.
Installed WDSP identity and exact RXC1 paired-path qualification remain open.
GoldMarsh's original worktree was untouched. Work is uncommitted in the isolated
`silverforge/puresignal-milestone1` worktree, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`; nothing was pushed.

## Decision table

| Input/condition | Proposed operating decision | Measurement treatment |
| --- | --- | --- |
| Verified ABI/topology, Bridge owns native channel, approved enable and subsequent authorized TX epoch; correct route/rate/format; fresh continuous finite paired data after settling | Admit to bounded queue; accumulate exact 1,024-pair native windows. Automatic or single calibration remains a native decision. | Qualify unclipped only from exact, valid ADC + both DDC covering evidence. Operating admission alone says nothing about clipping. |
| Missing telemetry, counter width/saturation/overflow unknown, invalid/resetting counters, clear instantaneous status, or bracket not covering capture | Do not invent a hit or clear bracket. May admit otherwise eligible samples under the explicit candidate policy. | Qualification unavailable with the exact reason (`ClipStatusUnknown`, `CounterInexact`, `CounterReset`, `ObservationIncomplete`, etc.). One unqualified fragment invalidates qualification for the entire assembled capture. |
| Fresh silence/under-excitation | Keep sample/DSP clocks advancing. Native collector skips zero TX/RF and reference below its source-defined collection floor; do not synthesize excitation or force success. Existing same-epoch correction can remain applied. | No new calibration/IMD success claim. Silence is not missing transport. A previously triggered fit can finish during silence; it must not be described as a fit produced by silence. |
| Active reference but RF effectively absent, using reviewed path-specific level limits | Stop/reset with `FeedbackLevel`; do not feed the bad native window. | Capture unavailable; not an ADC-overload claim. |
| Raw IQ component at the configured rail guard | Stop/reset with `SevereOverdrive` before native submission. | Suspected clipping, not proof of exact ADC/DDC event time or physical watts. Below-rail samples do not certify unclipped. |
| Valid positive ADC overflow report or DDC clamp increment associated conservatively with current owner session/epoch | Stop/reset immediately when delivered, even if the positive report is older than the feedback freshness deadline. A wide counter interval cannot locate the event, so stop conservatively. | Never certify a clear capture. Do not retrospectively assign an exact sample interval or silently leave a prior result qualified. |
| P2 gap/duplicate/reorder; malformed DMA/header or known FIFO/acquisition loss; bounded handoff overflow; wrong route/rate/format; nonfinite data | Poison/retire affected acquisition and stop/reset correction. Old-generation callbacks cannot restore it. | Invalidate affected captures; no trimming/interpolation used as fictitious continuity. |
| Oldest sample is stale, or queued/partial samples become stale before native consumption | Stop/reset; a newly completed DMA frame or browser receipt cannot rejuvenate it. | Stale/unavailable. Existing browser transit-delay/freshness contract still applies. |
| Before settling boundary, no authorized key edge, or old-generation callback | Reject without admitting/arming. A configuration change resets and waits for a new supervised TX epoch. | No current measurement. |
| WDSP fit-rejection/severe-overdrive bits, native call/quiescence failure | Stop/reset; keep fault latched. Treat failed quiescence as native failure, not bypass success. | No valid corrected-state attribution or fresh measurement. |

For faults: first stop admitting feedback and clear pending captures, set native
PS MOX false, drain/reset asynchronous work **while TX DSP continues advancing**,
and verify correction is off. This is not RF unkey. Future live integration must
ask the existing TX authority for a controlled unkey/envelope transition; changing
predistortion while keyed can change RF output. Do not automatically increase
drive, transmit uncorrected at the previous corrected-power setting, or auto-rekey.
An unkey/rekey alone does not clear a fault. Recovery requires acknowledged valid
reconfiguration and a later authorized TX epoch. The offline core proves native
cleanup, not this missing physical TX-authority integration.

## Existing-interface mapping and limitations

| Backend/source | Use for operating admission | What it cannot establish |
| --- | --- | --- |
| [P2 paired adapter](../../saturn-bridge/src/puresignal/adapters.rs), DDC0 paired 24-bit feedback/reference packet | Strict format/order, wrapping outbound sequence and oldest host arrival; owner-generation/config/TX fences. Propagate adapter faults to controller. | Network sequence does not establish lossless FPGA acquisition. Reviewed packetizer has no usable RF sample timestamp. |
| [P2 high-priority status](../../saturn-bridge/src/p2/packets.rs) `HighPriorityFromSdr.adc_overflows`, packet byte 5 | Positive selected-ADC bit is a hazard report. No second reader, new register, or invented DDC clamp counter. | Zero bit is not clear interval coverage. Cross-stream status has no exact paired-sample or TX-epoch association. |
| [Direct XDMA adapter](../../saturn-bridge/src/puresignal/adapters.rs), paired mux with optional retained DDC6 | Strict header/rate/pair order; oldest partial-word arrival; existing sole owner propagates framing, FIFO/read/handoff loss and resets. | No hardware sequence/timestamp in this frame. Locally continuous ordinals do not prove absence of silent whole-frame loss or exact paired atomicity. Exact frozen RXC1 path remains unverified. |
| [Existing XDMA ADC status](../../saturn-bridge/src/xdma_rx.rs) | Selected-ADC positive basic overflow bit, or valid new episode/activity evidence from coherent extended telemetry, can generate a conservative hazard notification. | Coherent snapshot does not equal capture coverage. Do not infer source timing, wrap/saturation knowledge or RF power calibration from host receipt. |
| [RXC1 poller](../../saturn-bridge/src/rx_counter_v31.rs) | If already armed, compare valid same-source/session/config counter snapshots. A new `accepted_clamped_iq_components` increment for the relevant DDC stops conservatively. Positive deltas are events, not clear brackets. Refused pairs/framing anomalies warrant continuity fault handling after source/path relevance is established. | It is opt-in and internally limited to **5 s**; snapshots are per DDC, not atomic with post-FIR capture ordinals. Do not arm it, extend its arm, accelerate it, or create sample-aligned brackets to make this proposal work. |
| [XDMA worker cadence](../../saturn-bridge/src/xdma_backend.rs) | Preserve existing 1 s performance section and 5 s basic-status section; RXC1 still applies its own 5 s period. | No 250 ms overload-detection guarantee. The 250 ms replay feedback age limit is not a telemetry latency limit. |

The operational policy is deliberately weaker than a proof of unclipped acquisition.
It relies on verified pair topology plus available known-fault monitoring. It does
not prove detection of every overload or silent loss. Accepting potentially delayed
ADC/DDC hazard detection is an explicit review decision, not a benefit proved by
the replay. Rejecting that risk does not, by itself, justify faster polling or new
firmware; the alternative is to leave live adaptation blocked pending a separately
reviewed capability requirement.

For positive status without RF-time attribution, the owner decision fence identifies
where to stop now, **not a newly inferred hardware observation epoch**. Preserve the
oldest host arrival and producer/source validity. A report that might belong to the
current session must stop conservatively, rather than be stamped as clear or ignored
because it lacks capture coverage. Only demonstrably obsolete source generations
may be discarded. Invalid/absent telemetry never clears an existing fault.

## Feedback level policy and reviewable limits

All level decisions use normalized signed-24-bit IQ (`sample / 8,388,608`), never
FPGA detector watts or LP-700 readings. The combined replay uses verified fixture
`hardware_peak=0.6121`, active-reference complex RMS fraction **0.1**, missing-RF
complex RMS floor **1e-6**, and component guard **8,388,607 / 8,388,608**.
These are explicit fixture parameters in `DraftLimits`, **not qualified production
defaults**. Evaluate RMS over 1,024 paired samples, not individual four-pair DMA
fragments or every instantaneous speech/two-tone envelope zero. A component guard
is separate from complex-envelope magnitude. The maximum accumulation interval is
about 5.33 ms at 192 kHz, plus bounded host scheduling; no real-time claim is made.

Pinned WDSP [calcc.c](https://github.com/TAPR/OpenHPSDR-wdsp/blob/b02d5bac675dd2f33ec2bab2b339f79a597c47dd/calcc.c)
defines `SAMPLE_MIN_X=0.0050`; `putSample` excludes effectively zero TX/RF and
out-of-collection-range reference. Collection needs all amplitude buckets; reference
level or admitted samples alone do not mean a fit succeeded. Raw `info[4]` is a
fitter-derived feedback quantity, not an absolute RF power or instantaneous clipping
measurement. Preserve all 16 native info integers; use successful calibration count,
applied flag and current fence together. `info[6]` rejection/overdrive handling stays
unchanged. No attenuator/drive auto-adjustment is proposed here.

Before live admission, review source scaling/peak, path gain/noise floor and the
active-reference/missing-RF thresholds for each backend. A calibrated noise-level
policy may supersede the fixture's simple floor; do not present this near-zero test
as protection against every weak, noisy, mistuned or uncorrelated feedback path.
Delay is configured rate-first through the verified native API; topology/config
changes invalidate the solution and delay assumptions. This replay does not prove
physical synchronization or choose the G2's feedback delay.

## Proposed interface responsibilities

- Shared controller: eligibility/fences, bounded continuity/freshness checks,
  window-level guards, native fit status, fault latch and quiescent reset.
- Existing acquisition owner: paired blocks with true oldest host arrival and
  generation/config; explicit known-loss callbacks; positive hazard observations
  with source, validity and preserved host timestamp. No independent register poller.
- Analyzer: raw pre-native-delay RF capture plus exact capture interval and a
  **separate** `Result<(), Fault>` unclipped qualification. Counter evidence must
  cover every contributing fragment. Numerical/provisional spectrum may remain
  displayable when qualification is unavailable; never label it qualified IMD.
- Future publication: include policy ID/review state, operating state/reason,
  native applied/calibration status, telemetry availability/age, and independent
  measurement qualification/reason. Preserve existing boot/config/sequence/clock
  bounds. “Correction applied” and “unclipped measurement” must not share a green flag.

The prototype interfaces are [DraftLimits, HazardObservation and measurement
certification](../../saturn-bridge/src/puresignal/admission_proposal.rs) plus
`Controller::new_offline_admission_proposal`, `observe_hazard`,
`uses_offline_admission_proposal`, and `raw_unclipped_qualification` in the
[shared controller](../../saturn-bridge/src/puresignal/controller.rs).
`HazardObservation` is positive-only; there is intentionally no method for an
instantaneous zero status to certify clear. The underlying `CounterBracket::certify`
contract is unchanged. These prototype methods are not a delivered server/browser
telemetry protocol or production TX-authority adapter.

## Offline execution evidence

[Runner](../../saturn-bridge/tests/run-puresignal-offline-native.sh) independently
checks the declared archive/headers, builds the strict suite and opt-in suite with
`rustc`, then runs [both adapters through shared controller and real NativePs](../../saturn-bridge/tests/support/puresignal-controller-native-replay.rs).
No fabricated clear brackets are supplied to the missing-telemetry scenarios.
Separate explicitly synthetic complete-clear evidence is used only to test the
positive measurement qualification path, followed by missing/inexact evidence.

Executed here on x86-64 WSL Linux, Rust 1.98.1, GCC 11.4, FFTW 3.3.8:

- **24/24** original default-gate tests; **32/32** feature-enabled tests, including
  an explicit regression that default construction still rejects absent brackets.
- Real-native strict replay passes on P2 and Direct XDMA: unkey during fitting,
  config reset, fresh-epoch fit, stale reset and destruction with DSP advancing.
- Real-native **automatic-mode candidate replay** passes on both: fresh silence
  produces no new fit; normal samples without brackets fit while measurement
  qualification stays unavailable; inexact counters do not certify clear; explicit
  synthetic coverage qualifies only its modeled window; delayed ADC hazard and
  positive counter delta reset; missing RF and rail guards reset an applied curve;
  P2 sequence loss / simulated XDMA owner FIFO-loss callback and stale feedback
  also reset the applied curve. Fault does not clear merely by unkey/rekey.
- Native cleanup checks IQC run/busy **both zero**, DSP exchanges increase through
  reset, and channel closes. Observer reads the existing internal flags; no native
  algorithm/archive change. Application-only timing and sample counts vary with
  asynchronous fitting; they are not a throughput/latency benchmark.
- Standalone native numerical replay also passes: 4,096 measured / 512 correction
  points, canaries intact, modeled normalized RMS error **0.102006762 → 0.00147866877**.
  This is not measured RF IMD, closed-loop radio correction or a TX safety test.
- Full stub-native Bridge suite: **377 passed, 2 ignored**, plus 32 library tests.
  Feature-enabled library Clippy with warnings denied, source/header ABI check,
  runner ShellCheck, browser typecheck and **699/699 browser tests** pass.

WDSP source remains `b02d5bac675dd2f33ec2bab2b339f79a597c47dd` (version 210/PS3),
Linux port `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`.
Unchanged offline archive SHA-256:
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
This identifies this offline build, **not the G2 installed library**.

## Review decisions before any live wiring

1. Approve/reject operating with unavailable per-capture ADC/DDC qualification and
   existing delayed hazard detection; keep measurement qualification independent.
2. Approve path-specific level/scaling limits and what status constitutes a credible
   positive event, including unassignable hardware timing and poller discontinuity.
3. Approve controlled-unkey behavior when correction is removed; no automatic
   uncorrected-power continuation or drive increase.

After policy review, owner integration still must prove actual paired topology,
native-library identity, bounded handoff/fault propagation and supervised TX lifecycle.
None is established by this offline replay. **Live admission stays blocked.**
