# PureSignal keying investigation — November 6, 2026

The G2 was running release `a1b9f39b`, shared Direct XDMA, firmware
1.31.002 / 0x53460004 and WDSP 2.10. No RF action, setting change, service restart
or deployment was performed by this investigation. Operator attempts and existing
captures supplied the live evidence.

## Two distinct stops

1. The browser diagnostic at 23:56:16 UTC reported `Local settings queue limit;
   unsent settings cancelled`, while the host was clean `ReadyOff`, committed
   Single, with no host fault. This browser fault blocks both two-tone and the
   browser-mediated rear-PTT action. Reconnecting clears this local session fault.
2. The operator's new attempt at 18:57:45 EST reached host `Active`, then stopped
   with `SevereOverdrive`. The 23:58:22 UTC diagnostic confirmed that host fault
   latched after cleanup. Reconnection alone does not acknowledge that host fault.
   The existing completed-fault acknowledgment remains a separate RX-only action;
   it never keys or automatically retries TX.

The latter digital archive records a Key at 442894103146 ns and a Stop at
442925958331 ns: about 31.9 ms apart on the owner clock. These records do not
measure physical RF onset/cessation or LP700 response. Commanded power was 16 W;
this is not a measured RF output value. RF permission was enabled. This was an
explicit stop after admission, rather than a missing two-tone capability.

## Browser repair

A deterministic 1000-edit power burst reproduced the local queue fault. Previously
every power edit occupied a FIFO entry behind one transmitted transaction; the
33rd unsent message faulted the entire managed session. Consecutive **unsent,
valid absolute power targets** now retain the latest target. An already-sent
transaction still needs its real queued/terminal receipts and committed owner
publication. PS/source/other settings remain ordering barriers. Release and
disconnect discard unsent targets. No completion creates fresh PTT authority.
Other settings retain their existing FIFO and bounds; this is not a blanket
suppression of queue errors. The PS diagnostic copy also includes managed phase,
pending-settings count and the last settings result.

## Why the current overdrive label is insufficient

Two controller branches produce `SevereOverdrive`:

- A reference or RF sample component reaches the configured limit (0.95 in the
  observed deployment). This is a sample-domain rail margin, not an ADC-overrange
  counter or calibrated RF power/SWR measurement.
- WDSP reports status mask `0x2` in `GetPSInfo()[6]`. In the pinned `calcc.c`, that flag requests
  reset when insufficient useful top-bucket correction range is detected. It
  must not automatically be described as ADC clipping.

Both branches previously called `latch_fault()`, whose buffer clearing zeroed the
controller's native info. Native cleanup then recreated the engine. Consequently,
post-stop state 0, feedback 0 and calibration count 0 did not retain the trigger.
Existing rate-limited sample archives also need not include the exact fault block.

A fixed-size frozen snapshot now preserves the origin, original fence/interval,
oldest Bridge-ingress timestamp and native info. For a pre-native component trip,
it also preserves the reference/RF component peaks and configured limit. It does
not label pscc-mutated post-delay arrays as original input samples. This evidence
survives stop/native cleanup. For the native flag, the interval identifies the
submission at which status was observed, not the asynchronous fitter's exact
source interval. The evidence is logged at the existing lifecycle transition
**after completed cleanup**, preserving stop-before-diagnostic ordering. It clears
when a new controller configuration is installed. The existing stop and fault
latch, limits, native numerical processing and re-arm requirements are unchanged.

The exact branch and signal that triggered the live 16 W attempt remain unknown
from the old executable's evidence. Installing this diagnostic and observing a
fresh operator-authorized attempt is needed to resolve that distinction. Raising
the guard threshold is not a demonstrated repair.

## Offline recorded-pair positive control

The existing `window-10.pairs` from run `2722792` was copied with its metadata and
hash verified. It contains 16384 original reference/RF pairs from the earlier
5-second 10-commanded-W run. Its reference component peak is about 0.639,
reference envelope peak about 0.6425 and RF component peak about 0.431. These
samples did not reach the observed 0.95 component limit.

Replaying these paired samples into the reviewed x86 WDSP 2.10 archive at
192 kS/s, HWPeak 0.67 and zero commanded delay produced a fit: calibration count 1,
correction-active readback 1, status bits 0. This is evidence that this retained
steady window can fit; it does not qualify the live startup, installed ARM library
execution, delay across other captures, physical RF correction or calibrated IMD.
The source `calcc.c` hash matches the installed ARM build's staged source. The
x86 archive identity is separate from the installed ARM archive identity.

Evidence is retained in `Downloads/Saturn_PS_Investigation_20261106`, including
original logs, the before/after queue reproducer, raw paired window and hashes,
replay source/results and test logs. An initial native-test link omitted the test
observer, and an initial parallel native run was stopped; both logs are retained.
The corrected native run linked the explicit test observer and ran serially.

## Receive tuning after a completed TX fault

A TX fault remains latched on the Bridge across browser disconnect/reconnect.
Previously, managed browser routing silently discarded VFO/DDS setters in the
fault phase, and the owner rejected every configuration while faulted. RX kept
streaming at its old frequency even though the browser optimistically moved its
VFO. Reconnecting could not repair this host condition.

A matched host now advertises `saturn_rx_tuning_caps:0,fault-latched-v1`.
Following completed stop/native cleanup (the matching
`saturn_tx_fault_ack_state:0,<stop>,true`), the operator can send pure absolute
`vfo`/`dds` transactions while the TX fault remains latched. Normal ordered
queued/applied/rejected receipts and unsent-target coalescing still apply. Mixed
batches containing drive, PS, source or other settings do not receive this
exception. Older hosts retain their existing contract.

The sole owner changes the receive model and configuration identity, updates
its stopped backend model, then publishes committed state before replying
applied. This operation neither configures nor renders native DSP. It preserves
the controller fault, frozen overdrive evidence, stop receipt, output fence and
fault acknowledgment requirement. A new PTT action still cannot key until the
operator separately acknowledges the completed fault. Tuning is not an implicit
acknowledgment. Release/disconnect revoke pending transactions as before.

Regression coverage uses the actual TCI dispatch and both P2/Direct XDMA output
adapters with offline devices. It checks ordered tuning, mixed TX rejection,
release/disconnect, retained fault, zero output and zero native configuration
or processing. Browser tests cover reconnect, 1,000 tuning edits, old hosts,
incomplete cleanup, lost operator authority and TX commands. The frozen native
fault-evidence test also retunes without losing the original observation.

Release arriving during the faulted retag cancels the transaction. Its retired
configuration identity is published with the previous receive settings, so a
fresh request cannot reuse that identity or inherit a rejected frequency. A
deterministic native-information hook exercises this boundary and a subsequent
successful fresh tune on both backend fixtures.

## Acquisition completion and amplifier feedback follow-up

The first faulted-tuning repair committed VFO/DDS settings in the owner but the
Direct acquisition loop still gated hardware programming on `startup_ready()`.
That predicate deliberately rejects a latched primary fault. The result was a
changed software/readout frequency with an unchanged receiver NCO. The new
`receive_control_ready()` checks completed authority/native cleanup, stop
receipt evidence, a closed output fence and current cancellation epoch, while
retaining the primary TX fault. It is used only for receive control application;
startup/TX readiness remains separate.

The actual `shared_backend::run_inner` regression reproduces the previous
failure after a fault: an applied transaction with an unchanged file-backed
DDC6 phase register. It now checks real acquisition-register changes for three
frequencies, retaining the fault and producing no transmit samples. It also
sets manual feedback attenuation while faulted, separately acknowledges the
fault, and verifies the next independently authorized preparation programs the
ADC TX attenuation bits to the selected value. No microphone/IQ input is
provided, so the fixture remains awaiting key until release.

A separate `saturn_ps_fault_attenuation_caps:0,latched-rx-off-v1` contract permits
only a validated manual feedback-attenuation setter through completed fault
cleanup. It retains normal settings receipts, permissions, cancellation and
fault acknowledgment. A mixed TX batch cannot acquire this exception. Both
capabilities survive fragmented Auto negotiation. The UI keeps other PS/TX
actions unavailable while faulted and exposes receive-tuning/attenuation
readiness in copied diagnostics.

The installed G2 event at 2026-11-06 23:35:53 EST recorded origin
`SampleComponentLimit`, reference component peak 0.64124596118927, RF component
peak 1.0 and component limit 0.95. Native flags were zero. This establishes the
sample guard's reason, not independently observed ADC overload coverage or
physical RF measurements. The protection threshold is unchanged.

The operator confirmed a Coaxial Dynamics 7999 adjustable sampler after the
amplifier, connected directly from its sample port to RX BYPASS. The saved
ANAN-G2 Thetis profile (modified 2026-11-01) has PS Auto ATT and Quick Attenuate
enabled; its saved peak setting is 0.6121. The SDR-VST3 ANAN-G2 profile also has
Auto ATT enabled. Saturn's shared path currently provides manual attenuation,
not the Legacy P2 owner's automatic loop. Its observed command was 0 dB.
A saved 2026-09-25 80 m calibration log changed from 31 dB at PS enable to 12 dB
at save; that historical setting is not a measured value for the current 40 m
amplifier trial. Do not copy it as a calibration or claim this manual-control
repair implements Thetis Auto ATT.

Manufacturer reference: https://www.coaxial.com/products/7998_7999.htm
The sampler's actual coupling setting, amplifier output and input return level
must be accounted for before a high-power correction claim. No live keying or
RF level-setting was performed during this source repair.

### Per-band PureSignal preferences

Band memory retains `pureSignalMode` (Off/Single/Auto adaptation),
`pureSignalAutoAttenuate` (Auto ATT preference) and
`pureSignalAttenuationDb` (requested manual feedback attenuation). The manual
slider saves its validated request immediately to the current band, browser
fallback and the normal server-settings save path, before waiting for an echo.
A saved request is not proof of host acceptance or an automatic level result.

Round-trip tests exercise distinct 40 m / 20 m choices, reload the saved memories
into a fresh page, and verify repeated recall including false and 0 dB values.
The actual slider handler is tested without a host echo and rejects invalid or
unavailable edits without rewriting memory. Recall does not key RF or enable
two-tone. Unsupported shared Auto ATT commands remain omitted; storing the
preference does not implement or advertise the automatic feedback-level loop.
