# WDSP 2.11 Linux/ARM Integration

Saturn Bridge uses WDSP 2.11 by default in the Saturn Go installer. The native
build is reproducible and does not depend on cloud-init, a prebuilt piHPSDR
checkout, or files under another user's home directory.

## Installer-Owned Native Sources

`update_manager/scripts/install-saturn-bridge.sh` provisions sparse checkouts
at exact upstream commits under the bridge Cargo target directory:

- TAPR OpenHPSDR WDSP: `e69de50d1d702216098019013899ad87534c64e0` (Release Version 2.11)
- piHPSDR Linux port shim: `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`

Only `wdsp 2.11/Source` and `wdsp/linux_port.c` / `linux_port.h` are checked
out. The helper copies and patches those sources into an ignored build
directory; upstream checkouts remain unchanged. Changing either pin is a
native dependency update and requires the complete bridge test matrix.

The full Saturn Go install builds and installs the bridge unless explicitly
disabled:

```sh
sudo bash update_manager/install_saturn_go_nginx.sh
```

Set `SATURN_INSTALL_BRIDGE=0` to opt out. `SATURN_REQUIRE_BRIDGE` defaults to
the same value, so the normal installation fails rather than publishing a
Remote UI that has no matching backend.

## Build WDSP 2.11

The bridge installer is the supported entry point for a native build-only
artifact:

```sh
SATURN_BRIDGE_BUILD_ONLY=1 \
SATURN_BRIDGE_OUTPUT_BIN=/tmp/saturn-bridge \
bash update_manager/scripts/install-saturn-bridge.sh
```

After the pinned source cache exists, the lower-level archive helper can also
be run directly:

```sh
update_manager/saturn-bridge/scripts/build-wdsp2-linux-arm.sh
```

Default cache/build locations:

- WDSP 2.11 source: `target/native-src/OpenHPSDR-wdsp/wdsp 2.11/Source`
- piHPSDR Linux port shim: `target/native-src/pihpsdr/wdsp`
- output archive: `update_manager/saturn-bridge/target/wdsp2-linux-arm/libwdsp.a`

Override those paths with `WDSP2_SOURCE_DIR`, `PIHPSDR_WDSP_DIR`, and
`WDSP2_BUILD_DIR`.

## Build Saturn Bridge Against WDSP 2.11

```sh
cd update_manager/saturn-bridge
SATURN_WDSP_DIR=target/wdsp2-linux-arm cargo build --release
```

Set `SATURN_BRIDGE_WDSP_FLAVOR=pihpsdr` only for an intentional legacy build;
that mode still requires the external piHPSDR archives.

When `SATURN_WDSP_DIR` selects an archive with RNNR/SBNR support, the build
also links `librnnoise.a` and `libspecbleach.a`. By default they are resolved
from `rnnoise` and `libspecbleach` directories beside the selected WDSP
directory. Override those locations with `SATURN_RNNOISE_DIR` and
`SATURN_SPECBLEACH_DIR`.

## Saturn Remote Controls

The bridge and `/remote-next` client expose these WDSP 2.11 controls through
TCI and saved radio profiles:

- NNR Standard/Premium with voice/CW model selection and native model readback
- NR2 gain method: Gaussian, Gaussian Log, Gamma, or Trained
- NR2 noise estimate: OSMS, MMSE, or NSTAT
- NR2 psychoacoustic post filter
- TX phase rotator run state, corner frequency, and automatic corner optimizer
- Wideband FM stereo reception, stereo-lock telemetry, and selectable 75 us or
  50 us de-emphasis
- PureSignal 3.0 automatic calibration/correction, automatic or manual feedback
  attenuation, calibration reset, and live correction/feedback telemetry

Gamma, OSMS, and an enabled post filter preserve the previous NR2 behavior.
The phase rotator defaults to OFF at 338 Hz and is forced out of the active DSP
path in DIGU and DIGL modes. Builds against an older WDSP archive retain manual
phase-rotator run/corner control; the auto-optimizer call is enabled only when
`SetTXAPHROTAutoMode` is present in the selected archive.

WFM uses WDSP mode 12 with 192 kHz input and DSP rates, then returns 48 kHz
stereo audio to the bridge. Selecting WFM forces the Protocol 2 DDC to 192 kHz,
uses the 88-108 MHz FM broadcast band memory, and disables mono-oriented NR,
blanker, notch, AGC, and passband controls. The bridge refuses transmit while
WFM is active. WFM is advertised to clients only when the linked archive
exports both `SetRXAWBFMdmph` and `GetRXAWBFMStereoIndicator`.

PureSignal uses Saturn's synchronized 192 kHz paired feedback. The shared
lifecycle remains the single TX/native owner, with separate P2 and Direct XDMA
acquisition adapters. The managed startup remains an explicit host selection;
this library update preserves the deployed selection and operator settings.
Feedback level, delay, overload coverage and correction quality remain separate
measurements. A version string or native correction flag does not certify RF
IMD, physical shutdown timing or an unclipped capture.

The Linux/ARM build helper patches the WDSP 2.11 de-emphasis setter in its
staged source tree so a runtime 75 us/50 us change reaches both WFM audio
channels. The upstream source checkout is not modified.

## WDSP 2.11 changes and compatibility

Upstream 2.11 adds Standard/Premium CW NNR models (slots 2/3) beside the
speech models (slots 0/1), fixes FM, pre-emphasis, EMNR and neural processing,
and validates more file paths and buffer sizes. The bridge retains its current
operator controls; a library upgrade does not automatically select a new model.
The four model translation units are pure data and compile at `-O1`; all DSP
code remains `-O3`, without fast-math. The Premium models need measurement on
an actual CM4 before recommending them for continuous operation.

The Linux port now uses upstream `wdsp_align.h` for real aligned allocation.
The old Saturn pre-emphasis repair is present upstream: its corrected source
hash is verified instead of applying the patch twice. The existing resampler
index optimization and WFM stereo de-emphasis setter repair remain guarded by
source checks.

2.11 requires `SetChannelState(ch, 0, 1)` before closing a channel. The managed
owner begins OFF without blocking a lifecycle turn, continues zero-input DSP
exchanges after the output stop receipt, waits for flush completion and joins
the TX worker before freeing storage. Other close paths request the documented
bounded OFF transition. Native cleanup, observer layout, public header and
archive identities are pinned to this build; the update does not change the
PureSignal algorithm or its calibration/measurement qualification.

## Developer CPU profiling

Profiling was introduced upstream in 2.10 and remains available in 2.11.
Production builds keep it **off**. An isolated developer archive enables the
upstream NNR per-stage timers:

```sh
SATURN_WDSP_NNR_PROFILE=1 WDSP2_BUILD_DIR=/tmp/wdsp211-profile \
  bash update_manager/saturn-bridge/scripts/build-wdsp2-linux-arm.sh
SATURN_WDSP_DIR=/tmp/wdsp211-profile \
  SATURN_WDSP_PROFILE_OUTPUT=/tmp/wdsp211-reports \
  python3 tests/test-wdsp-211-contract.py
```

Run these commands from the repository root after provisioning the pinned
sources. The test uses synthetic audio, exercises all four models and exports
one `nnr-profile-model-N.txt` report per model. It opens no radio device and
keys no RF. Profiling writes files and is intentionally excluded from the
reviewed production archive whitelist. Exchange elapsed time includes worker
scheduling; it is not the same quantity as the upstream per-stage CPU timers.
For an x86 developer build additionally select `SATURN_WDSP_TARGET_ARCH=x86_64`
and `SATURN_WDSP_TARGET_CPU=x86-64`.

For normal archive validation:

```sh
SATURN_WDSP_DIR=/path/to/normal/wdsp211 \
  python3 tests/test-wdsp-211-contract.py
python3 tests/test-wdsp-channel-shutdown.py
```

Actual runtime fixtures with real WDSP must run serially in **fresh processes**.
An intentional unrecoverable restoration-failure test retains the native lease;
running later scenarios in that same process correctly refuses a second owner.
Use `tests/run-wdsp-runtime-isolated.py` after building the managed test binary
with the offline observer linked. Loopback/file-backed TX in these fixtures is
synthetic; it is not physical RF qualification.

WDSP 2.11 does not supply piHPSDR RNNR/SBNR exports. Saved NR3/NR4 requests
still use NR2 when those exports are absent. `rx_nr_capabilities` advertises the
available algorithms; `rx_nr_effective` identifies the implementation selected
for the saved request, and `rx_nnr_model` reports WDSP's model slot (or -1 when
NNR is inactive). The UI disables unsupported new selections while retaining
saved preferences and explicitly labels an NR2 fallback. No firmware, driver, RXC1 polling,
feedback limits or RF permission settings change with this update.

## Integration and performance follow-up (2026-11-11)

NNR selects slots 0/1 for Standard/Premium voice and 2/3 for CWU/CWL. Quality
and level use the existing saved radio/profile/band preference paths. The level
slider sets WDSP's mask floor from 0 to -60 dB. The host requires all four NNR
control/readback exports before advertising it. Premium remains an explicit
operator choice; x86 tests do not qualify CM4 CPU capacity.

Ordinary managed PTT release now retains an existing PS curve after the
matching backend stop receipt, MOX-off, fitter/IQC quiescence and completed
native audio/IQ flush. Loaded IQC run state is distinguished from asynchronous
IQC work. CALCC delay lines and external DEXP history are also cleared. The next
fresh authorized generation reuses the curve and its last applied feedback ATT;
mode flags are requested once per configuration, not on each PTT. New output
is still fenced until that generation's acquisition preparation completes.
A fault, reconfiguration or shutdown uses cold cleanup and discards the curve;
a cold request can supersede an in-progress warm flush. This is in-process
retention, not correction-file persistence or permission to reuse a curve
across a band/route/source/configuration change. RF freshness and rail guards
remain enforced.

The second-over replay covers Single and Auto through authenticated TCI/SATP,
the shared controller, real WDSP and both actual owner adapters. A separate
native test seeds real audio-filter history, proves zero output on the next
zero-input over, and escalates an active warm flush to cold shutdown. Synthetic
replay confirms host/native contracts, not RF improvement.

Shared receive runs the normal WDSP passband/meter chain for connected
control-only clients. With no audio subscription or local headphone consumer,
it bypasses NR, ANF and RX EQ, skips audio conversion/packetization and clears
partial packets. Enabling audio restores the saved profile. With no clients,
IQ subscription or headphone consumer, the receive channel is suspended while
acquisition continues to drain. Full TX/native output, owner mic blocks, IQ
packet submission and SATP output use fixed/reused buffers in the scheduling
loop. Retained SATP timestamps and FIR-history provenance are unchanged.

The shared performance JSON adds fixed-storage count, p95/p99 bucket upper
bounds and observed maxima for RX exchanges, active TX turns, feedback pumping,
rendering, backend output submission/gaps, source age and notification-to-stop
receipt. These are session totals. Exchange includes native-worker waiting;
source age starts at Bridge ingress; a backend receipt does not measure physical
RF-off latency. Formatting is in the existing low-rate publisher. Use an actual
CM4 before claiming CPU or RF timing improvements. No audio-buffer reduction,
firmware/driver change or scheduling-policy change is part of this slice.

## Pin manifest

`scripts/wdsp-pin-manifest.py` records exactly which WDSP a build contains, so
two builds (a developer PC, a cross build, the G2's staged build) can be
compared without trusting a version string. It reads the commits the installer
pins, the commits actually checked out, a SHA-256 for every upstream source file
and every patched file the archive was compiled from, the compile options,
compiler and FFTW version, and `libwdsp.a` with each object's hash, its machine
type, and whether every symbol the installer requires is defined. It exits 1 on
a pin mismatch or a missing symbol.

```sh
python3 scripts/wdsp-pin-manifest.py \
  --native-src target/native-src --build-dir target/wdsp2-linux-arm \
  --bridge target/release/saturn-bridge --output wdsp-pin-manifest.json
```

Source hashes are comparable between any two builds. The archive and object
hashes only match between builds made with the same compiler and flags, so a G2
build is compared with another G2 build, not with a cross build. A compiler that
emits debug information also embeds the build directory, so the build path must
match as well. GCC without `-g` gave bit-identical objects and archive from two
different directories.

## Follow-up to the 15-finding source review (2026-11-12)

Baseline: deployed source `94b7ad82bd957147ba0a5bcb0659658ee830d4e7`.
The supplied review covered the earlier `91a4c773..fdcbe15d` range. Review
and tests below are offline; the running G2 release/configuration is unchanged.

| Finding | Disposition |
| --- | --- |
| Band-profile drive bypasses the Legacy endpoint | Intentional calibration policy, clarified in `pa_calibration.rs`. `legacy_drive_at_100w` is a Legacy calibration endpoint, not a measured DAC hardware ceiling. A band profile replaces that curve. Existing tests cover monotonicity, zero drive, 100-W request saturation and encoded-byte bounds. Reinstating a universal byte-68 cap would undo the requested 10/11-m PA calibration. No claim that a commanded watt value measures RF power. |
| RF clipping evaluated before acquisition/gain settling | Fixed: validate timestamps, continuity, known overload, finiteness and reference rail first; interpret RF level only after acquisition completion and its original-observation settling boundary. Added old-gain/new-revision rail regressions. |
| Readiness survives a gain request; completed status regresses | Fixed: a gain request clears readiness/applied readback/info. Held-key completion also requires preparation, matching applied gain and Stable/Limited state. Completed startup leveling retains its status. |
| Native try-lock contention discards a feedback block | Fixed: retain exactly one deferred block; retry it before dequeuing another, preserve age and recycle/discard it on cancellation. Actual scheduling tests cover recovery without SequenceGap, release, staleness and completed cleanup. |
| Owner/audio-drop and Auto ATT telemetry consumes receipt FIFO | Fixed: latest owner/ATT/drop snapshots use coalesced control slots; audio-drop changes no longer advance owner-state serials. Actual media-authority edges and settings receipts retain their bounded FIFO and revocation policy. Flood tests preserve authorization; receipt overflow still revokes. |
| Band recall persists local hardware outputs / re-arms MON | Fixed: new band entries omit headphone/MON settings and old entries cannot override global device choices. Reconnect explicitly leaves MON off; global headphone choice and monitor level remain available. |
| Client reconnect/band restore overwrites host PA profile | Fixed: generic preference restoration never sends a PA profile. Only explicit PA Apply/Reset changes it. Tests cover absent and stale client profiles. |
| Band-edge tuning fills settings queue | Fixed: adjacent unsent absolute navigation and derived band snapshots coalesce together. A later fine tune retains its pending snapshot. Explicit settings remain barriers, sent transactions retain receipts, and release cancels unsent navigation. |
| A historical SWL selection captures overlapping Ham tuning | Fixed: ordinary jumps resolve Ham first; tuning within an explicitly selected SWL band retains that context. Absolute keypad entry resolves independently. Explicit Ham/SWL button memories remain distinct. |
| ARM32 rejected / x86 inherits Cortex CPU | Fixed: armv7l/armv8l and ARM64 use Cortex-A72 defaults; x86_64/amd64 use x86-64. Explicit CPU overrides remain supported. Target validation precedes build-directory mutation. Entry-point tests cover defaults, overrides and invalid targets; no ARM32 executable was run. |
| Default mic gain changed from -12 to 0 dB | Intentional operator-requested unity default. Saved gain values remain honored; no automatic +12 dB adjustment is applied to existing profiles. |
| Channel destruction waits for its own missing exchanges | Fixed for RX/Legacy destruction: begin OFF without blocking, advance zero-input exchanges, then wait for build-matched flush completion before close. Managed lifecycle already polls native completion cooperatively. Real WDSP regression verifies RX and active Legacy TX flushes; delayed-worker test retains join-before-free. Unsupported native builds retain the existing conservative flush fallback. |
| One rear-PTT read error terminates RX runtime | Fixed in both Direct XDMA loops: invalidate the input and cancel its claimed press, retain receive operation and log counted failures. A valid released observation is required before a new press. Unrelated acquisition failures remain errors. |
| Explicit WDSP 2.00/2.10 silently labeled 2.11 | Fixed: explicit older-version selectors fail before provisioning. Generic `wdsp2` and explicit `2.11` select the pinned build. Managed archive/header digest checks already reject unreviewed archives independently of the label. |
| Repeated full preference normalization | Fixed: RX/monitor scalar clamps and PS-mode normalization are shared helpers; applying a few fields no longer constructs two complete normalized profiles. |

The existing cancellation-boundary fixture also now uses its deterministic
adapter key clock, matching the virtual milliseconds advanced by the owner.
This corrects a test clock mismatch; production timing rules are unchanged.

Validation: managed stub suite (85 library, 725 binary, 15 active integration
tests); full web suite (1,072 tests); TypeScript, template seam, UI parity,
migration, production asset build; Chrome Ham/SWL (20) and settings (47) checks.
Nine focused real-WDSP cases run in fresh processes, including normal
release/re-arm retention and actual RX/Legacy close. The 22-case production runtime
replay passed using both backend fixtures. A first run hit an output-age
watchdog while the separate NNR profiling test competed for CPU; its failure
and completed cleanup are retained alongside the isolated successful rerun. These results do not establish physical RF
shutdown timing, RF power calibration, overload coverage or RF IMD.
