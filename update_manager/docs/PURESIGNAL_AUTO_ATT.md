# Shared-owner PureSignal startup Auto ATT

The `start31-owner-v1` capability implements startup feedback leveling on P2
and Direct XDMA. It does not change firmware, the XDMA driver or RXC1 polling.
It uses the existing acquisition owner, output fence and single native TX owner.

## Operator behavior

Auto ATT and its manual alternative are saved per band. An explicit saved manual
0 dB remains 0 dB; new manual preferences default to 31 dB. A capable shared host
reports its committed Auto ATT selection. The checkbox is usable while RX/off
(including completed fault cleanup); it is unchecked and unavailable for hosts
that do not advertise support. With PS off, Auto ATT is armed for a later PS
transmission; the manual baseline slider remains usable and displays the manual
value actually requested. With PS enabled and Auto ATT on, the slider displays
confirmed applied attenuation, independently of the
saved manual preset. A focused manual edit is not overwritten by panel refresh.
Neither changing a setting nor acknowledging a completed fault keys the radio.

For each fresh authorized PS transmission with Auto ATT enabled:

1. Apply 31 dB through the acquisition owner and wait for its matching completion
   before keying. A manual preset or previous leveling result cannot bypass this.
2. Stream ordinary uncorrected TX IQ while holding the native key receipt.
   Fitting and correction remain disabled. A build-matched, nonblocking native
   observer checks this state; contention skips that block without retiming it.
3. Wait 500 ms from the physical-key control completion, then accumulate active
   reference/RF energy over 40 ms observation windows. Reference samples below
   3% of the configured hardware reference peak do not drive gain decisions.
   Silence therefore holds the current gain without enabling fitting.
4. Estimate `256 * sqrt(sum(RF power) / sum(reference power))`, targeting 152.293
   with an accepted range of 140–165. This startup ratio estimate is not a WDSP
   readback or an RF power measurement. Change attenuation by at most 6 dB per
   step within 0–31 dB. Wait for the acquisition completion, then 30 ms of
   original observation time before measuring at the new gain. At most 12
   adjustment steps are permitted.
5. Require two in-range windows, or two valid above-target windows while already
   at maximum attenuation. The latter reports `limited`, retains 31 dB and does
   not claim that the level reached its target. Return the original key receipt
   and admit only
   observations after the new feedback-ready boundary plus ordinary settling.
   WDSP Single/Auto fitting may then begin. Hold gain for the rest of this TX;
   this version is not a continuous mid-correction attenuator servo.

The maximum-attenuation case is an actuator-limit notice, not a hardware overload
measurement. The startup RMS ratio differs from pinned WDSP 2.10's native
`info[4]`, which uses envelope extrapolation at normalized reference amplitude
one. A ratio above target at 31 dB therefore does not itself justify latching
`FeedbackLevel`. Both windows must pass the unchanged age, route, acquisition
revision, continuity, finite-sample, positive-overload and component-rail checks.
The normal controller then runs its own admission checks and WDSP decides
whether a fit is usable. Missing feedback and actual rail/overload faults remain
stop conditions. The lower attenuation limit and adjustment-step limit are not
relaxed. This does not qualify RF input headroom or amplifier ramp timing.

All gain changes preserve original sample timestamps. Each pair block carries
its acquisition-request revision, stamped before enqueue. Old-gain queued
blocks cannot count toward a new gain or refresh its watchdog. The original
physical-key timestamp remains unchanged; a separate admission floor anchors
post-leveling settling and freshness. Existing stale-feedback, source and
output watchdogs remain active during startup, including periodic service with
no arriving packets.

Sample-domain rail margin, nonfinite data, continuity faults and valid positive
overload evidence remain stop conditions. While fitting/correction are disabled,
RF rail hits below 31 dB can request more attenuation; a rail hit at 31 dB stops
with SevereOverdrive. Unknown overload coverage stays unknown. Release,
disconnect and configuration changes use the existing stop receipt, acquisition
restoration and native cleanup; no failure or settings completion auto-rekeys.
Manual operation and PS bypass do not run this servo.

## Wire and capture provenance

Negotiation: `saturn_ps_auto_att_caps:0,start31-owner-v1;`.
Committed setting: `tx_puresignal_auto_attenuate:0,true|false;`.
Settings use the existing ordered queued/terminal transaction contract.

Status:

```
saturn_ps_auto_att:0,<stop>,<config>,<frequencyHz>,<enabled>,<phase>,<requestedDb>,<appliedDb>,<ratioEstimate>,<acquisitionRevision>,<keyed>;
```

`appliedDb` and `ratioEstimate` use -1 when unknown. Phases are off, applying,
settling, leveling, stable, limited and fault. Messages are scoped to the current
stop
and committed configuration. `limited` means maximum attenuation with the
startup estimate still above target; it is neither `stable` nor proof of an
accepted native fit. Updated Remote clients display this distinction and label
the estimate as a startup ratio. Applied gain is acquisition-owner completion
information, not independently measured analog attenuation. Correction graphs
use that applied gain rather than the saved manual preset. Raw paired captures
include `acquisitionRevision`; original samples, scaling and arrival ages are
unchanged. Frozen startup fault evidence retains the requested/applied gain,
ratio estimate and reference/RF component peaks through completed cleanup.

## Validation and limits

The actual `shared_backend::run_inner` offline test exercises both acquisition
paths and actual TX adapters with real WDSP: Single and Auto fitting, two fresh
key-ups, maximum-31 startup, gain convergence, correction-disabled startup,
release, disconnect, missing feedback, reconfiguration and saturated RF at the
maximum attenuation. It checks stop/native completion, acquisition restoration
and owner join. Servo tests cover silence, envelope minima, delayed revisions,
stale/future data, gaps, nonfinite samples and positive ADC evidence. Browser
checks cover honest capability/selection, applied-gain display, per-band recall,
manual editing and fault recovery.

The maximum-attenuation regression reproduces a startup estimate of 258 and
requires two valid windows before admission. A subsequent component at the
unchanged 0.95 guard still faults. The real-WDSP runtime replay additionally
uses unclipped high-gain synthetic feedback through both actual backend adapters,
requires `limited` at 31 dB, accepted native correction, normal release, a fresh
authorized second generation, completed restoration and owner join. It uses the
same 0.95 component guard; these remain offline fixture results, not RF tests.

These are host/software results. The 500 ms settling interval is an explicit
host startup policy, not a measured amplifier-ready signal. This work does not
qualify the external sampler setting, amplifier output, ADC-overload coverage,
physical RF shutdown time or calibrated RF IMD. LP700 observations remain
separate evidence. The first deployed bench run must verify actual attenuation
changes and sustained RF/correction with the operator's amplifier setup.
