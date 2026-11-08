# PureSignal review disposition — 8 November 2026

The independent review correctly identifies a significant lifetime defect:
the managed owner destroys the correction curve on ordinary release. Accept
that finding, the per-key Single-mode request finding, and the need for runtime
feedback leveling. Retained bench results demonstrate that correction can work;
they do not establish reliable short-SSB operation or a guaranteed −60 dBc result.

Reviewed base: `8b5d8402e30536e18a0b1167c91196165e2e87d7`, plus the
uncommitted `fix/thetis-runtime-auto-att-20261108` changes. The supplied review
was read from the local archive's `review/PURESIGNAL_REVIEW_20261108.md`.
This disposition distinguishes source findings, existing reports and new offline
execution. No RF was keyed and no G2 setting, service, firmware or driver was changed.

## Installed configuration checked directly

A read-only whitelist of the running Bridge process environment returned:

| Setting | Running value |
| --- | --- |
| TX owner | `shared` |
| PS admission | `operational` |
| Hardware peak | `0.67` |
| Requested TX delay | `0` seconds |
| Host settling | `25000000` ns = 25 ms |

The installed release remains `8b5d8402`. An earlier read-only TCI snapshot
reported `tx_pa_calibration:0,legacy;`. These observations establish configured
values, not their suitability for the current amplifier/coupler operating point.
The strict-admission default is therefore a deployment/usability issue, **not
the active blocker on this G2**. No replacement RXC1 counter source is proposed.

## Finding-by-finding disposition

| Review finding | Disposition |
| --- | --- |
| F0a PA calibration | Confirmed legacy profile. The source can produce drive 41 versus 68 at 40 commanded W near 27 MHz. The saved Thetis table is a starting profile, not a newly measured PA calibration. Verify LP average/PEP mode and actual RF before declaring the target achieved or increasing drive into the amplifier. |
| F0b pre-key attenuation | Confirmed historical component-rail trip and startup-only leveling. Last-good gain needs an operating-context identity and invalidation on power/routing changes; a band number alone cannot guarantee safety when an external amplifier changes. Avoid a blanket ban on 0 dB: it may be valid for a different sampler. Use conservative first-use Auto ATT and explicit manual operation with truthful level feedback. |
| F1/F11 curve lifetime | Confirmed. `begin_cleanup` always leads to reset and channel recreation; controller cleanup requires native correction off. Separate retained passive correction from active fitting/output authority. |
| F2/F6 mode per key | Confirmed for Single: a new manual request in LSTAYON drives reset. Remove per-key mode writes when introducing retained correction. Reasserting `automode` does not by itself force an already-Auto LWAIT/LSETUP state through LRESET; the existing unconditional cleanup is the clear reason both modes lose their channel. |
| F3 strict admission | Missing counter brackets cannot pass strict admission. Running G2 is already operational. Preserve unknown measurement coverage; do not invent counter brackets or increase polling. |
| F4 MOX delay | Confirmed managed delay 0 versus Legacy 0.2 seconds. Align native timing with a documented settling policy and test sample-count behavior. Do not silently turn the installed 25 ms host setting into 250 ms or mistake either timer for measured amplifier readiness. |
| F5 reference peak | Measurement required. Current configured peak is 0.67, not 0.6121. A component peak is not the complex-envelope maximum. Use current-build `GetPSMaxTX` and paired evidence before choosing a replacement. |
| F7 runtime ATT | Confirmed startup-only design. Local implementation now uses native fit-attempt reports and reset → quiet proof → acquisition gain acknowledgment → original-observation settling → resume, within the existing owner. It still needs retained-gain/curve integration across overs. |
| F8 P2 gaps | Current controller faults on discontinuity. A recoverable PS UDP gap needs explicit partial-buffer/collector recovery and bounded loss handling. Do not apply this change indiscriminately to Direct DMA framing loss, old mic/IQ, source timeout or revoked TX authority. |
| F9 native status | Confirmed for pinned WDSP 2.10: `info[6]` solution/collector bits are calibration notices. Local correction preserves actual component-limit and positive-overload faults. Collector-reset alone does not establish that RX attenuation should increase; a TX reference/peak problem can also cause it. |
| F10 delay | Partly correct. Requested delay lacks a per-band calibration/default, but WDSP's returned delay is actually used, checked for finite/range validity, stored in controller status and reapplied/compared during recreation. Actual TX/RF delay readback exists. Consistent operator publication and measured signed-delay qualification remain open. |
| F12 persisted curves | Not implemented. Defer restart/disk persistence until normal release retention and configuration invalidation work. A saved curve must carry backend, band/route, drive/PA profile, peak/delay and provenance. |
| F13 observability | Partly present: controller carries all 16 native words and actual delay; correction monitor exports raw native info; applied attenuation is reported. Add consistent compact level/max-reference/notice telemetry and a matched A/B summary. Snapshot time is not calibration age. |
| F14 analog path | Rail-level feedback cannot certify useful correction. The record establishes clipped sample components; it does not locate the exact analog clipping stage. Additional filtering depends on actual sampler/ADC/DDC response and unwanted-signal evidence; it is not a proven missing prerequisite on this bench. |

## Corrections to the proposed implementation

- The inspected Thetis `PSForm.cs` at
  `02db47d55ea5ffc9ca8e93a2c4bc98baa9e0743f` accepts **129–181**; piHPSDR
  `ps_menu.c` at `82aa8c7c5d3108dba1f507109346ff89a1ec541b` accepts
  **140–165**. Both use the 152.293 target. A tighter band is an explicit policy
  choice, not a correction needed to match Thetis.
- `info[7]` counts attempted fits; `info[5]` counts accepted fits. Only use a
  fresh valid level/extrapolation report, not every repeated status snapshot.
- At 192,000 paired samples/s and 119 pairs/datagram, P2 feedback is about
  **1,613 datagrams/s**. 187.5/s is the cadence of 1,024-pair native blocks,
  not the wire-packet rate.
- A physical/sample-rail guard remains independent of calibration notices.
  Existing historical `SampleComponentLimit` trips with RF peak 1.0 are not
  explained away by changing the meaning of native `info[6]`.
- A fixed 150 ns is a reference-client starting value, not a measured delay
  for every backend/route. Do not change delay or increase generator amplitude
  to compensate for an unverified PA profile.
- Lower power does not inherently make PS ineffective. Noise, clipping,
  reference scaling, envelope coverage and measurement limits determine the
  achievable result. The remaining 2–3 dB cannot be promised without a matched
  measurement at a confirmed operating point.
- Keep the owner responsive during native work. Do not copy blocking
  `SetChannelState(..., dmode=1)` or timer sleeps into its scheduling loop.

## Required retained-release implementation

Normal authorized release should close output immediately, cancel pending arms
and discard queued audio/IQ. Native cleanup still waits for the matching complete
backend stop receipt. It then needs a nonblocking proof of MOX off, fitter idle,
no pending IQC transition and complete native input/output-history flushing.
The loaded passive IQC curve may remain. Retaining it must never retain SATP FIR
history, microphone buffers, old output samples or authorization.

Set Single/Auto control on actual mode/configuration/recalibrate actions;
ordinary PTT changes MOX only. Same-context re-arm may use a retained curve and
matching gain. Fault, PS disable, incompatible configuration, backend switch and
shutdown must still invalidate/clean the native channel. Shutdown must force cold
cleanup even if the preceding normal release left a passive curve.

Two-key acceptance: fit Single, release with completed receipt/flush, re-arm from
a fresh operator action, then prove native correction is already available before
another fit. Include no-old-audio assertions, configuration-change invalidation,
fault and shutdown controls. Test both backend adapters through actual runtime
orchestration. A native MOX-only experiment is supporting evidence, not proof
that this production lifetime change has been implemented.

## Local runtime slice and execution evidence

The local runtime ATT slice retains the current first-use startup behavior;
it does not yet solve F1/F2 or add per-band retained-curve storage. Native controls
are nonblocking and serialized with the existing WDSP owner. No second reader,
DSP owner, RF drive writer or analysis thread is introduced into TX control.

Focused policy/core tests cover new fit reports, hysteresis/limits, ordered
gain acknowledgments, old acquisition revisions, original observation age,
no manufactured freshness and partial-buffer invalidation. Native status tests
distinguish fit notices from real clipping and unknown status bits.

The actual-runtime gain up/down test uses real WDSP, loopback P2 and ordinary
Direct-XDMA fixture files. Both fixtures increased and decreased attenuation and
refitted without changing the TX stop/generation identity. The first fixture
attempt lacked its correction-monitor opt-in; another assumed acquisition could
never be pending while Active. Those harness defects were corrected rather than
increasing the timeout or changing production freshness limits.

A separate real-WDSP synthetic test reached Single LSTAYON with an accepted fit
and IQC running. Eight zero-input blocks after native MOX off, and another eight
after MOX on, retained the calibration count, state 8 and active IQC without a
new mode request. Calling the current full native cleanup then recreated the
channel and cleared correction. This confirms the native lifetime distinction;
it does not claim that the production owner has acquired warm-release behavior.

Runtime-transition release, missing feedback and reconfiguration also passed
through both actual backend fixtures. The first two harness attempts waited
for a rate-limited UI phase snapshot and omitted the gain-ramp injection for new
negative cases. The final tests observe the acquisition request directly and
inject the same ramp in every runtime case. Existing freshness limits were not
changed. Complete stop receipts, native cleanup and acquisition restoration are
required. Missing feedback remains a watchdog stop in this slice.

Further execution results belong in the accompanying validation report. These
offline tests establish host behavior only; they do not establish physical RF,
the LP meter's power convention, amplifier settling or calibrated RF IMD.

Aggregate validation at this checkpoint: managed stub 90 library / 715 binary
passes; default stub 53 library / 528 binary passes. The native library suite
passed 90 tests. The native binary suite passed 701 tests (63 ignored) under GDB,
both with and without normal Rust output capture. Two ordinary aggregate runs
stalled at the combined TCI/SATP native test and were terminated; that test
passed in isolation and after the receive-tuning fixture under GDB. This
timing/order-dependent native-suite behavior remains unresolved. The passing
debugger run does not cancel the failures or establish a generally clean native
validation. Preserve the logs and resolve this before promotion/deployment.

The runtime slice is a local checkpoint, not a merge to main or a G2 deployment.
The deployed radio remains on the prior release with its normal operation
unchanged. Normal-release curve retention, context-bound last-good gain and
the corresponding two-key production acceptance are still required.

Next bench comparison must record G2 output and amplifier output separately,
meter mode, actual drive/PA profile, applied ATT, reference envelope/peak, native
fit/correction status, and matched PS-off/on RF spectra. Retained low-power
success does not qualify high-power operation, and the converse also applies.
