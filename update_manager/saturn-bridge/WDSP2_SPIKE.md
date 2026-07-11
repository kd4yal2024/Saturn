# WDSP 2.00 Linux/ARM Integration

Saturn Bridge uses WDSP 2.00 by default in the Saturn Go installer. The native
build is reproducible and does not depend on cloud-init, a prebuilt piHPSDR
checkout, or files under another user's home directory.

## Installer-Owned Native Sources

`update_manager/scripts/install-saturn-bridge.sh` provisions sparse checkouts
at exact upstream commits under the bridge Cargo target directory:

- TAPR OpenHPSDR WDSP: `584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3`
- piHPSDR Linux port shim: `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`

Only `wdsp 2.00/Source` and `wdsp/linux_port.c` / `linux_port.h` are checked
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

## Build WDSP 2.00

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

- WDSP 2.00 source: `target/native-src/OpenHPSDR-wdsp/wdsp 2.00/Source`
- piHPSDR Linux port shim: `target/native-src/pihpsdr/wdsp`
- output archive: `update_manager/saturn-bridge/target/wdsp2-linux-arm/libwdsp.a`

Override those paths with `WDSP2_SOURCE_DIR`, `PIHPSDR_WDSP_DIR`, and
`WDSP2_BUILD_DIR`.

## Build Saturn Bridge Against WDSP 2.00

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

The bridge and `/remote-next` client expose these WDSP 2.00 controls through
TCI and saved radio profiles:

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

PureSignal uses Saturn's synchronized 192 kHz feedback path: DDC0 carries the
ADC0 PA feedback samples and DDC1 carries the TX-DAC reference samples. The
bridge deinterleaves those samples into WDSP `pscc` blocks, tunes both feedback
DDCs to the transmit frequency, and applies the Saturn-specific hardware peak
and delay values. Feedback attenuation defaults to automatic control and is
constrained to 0-31 dB. Enable, automatic-control, and manual-attenuation
changes are rejected while TX is armed. A feedback outage bypasses correction
without interrupting uncorrected TX, reports a fault to the UI, and restarts
calibration when synchronized feedback returns.

The Linux/ARM build helper patches the WDSP 2.00 de-emphasis setter in its
staged source tree so a runtime 75 us/50 us change reaches both WFM audio
channels. The upstream source checkout is not modified.

## Current Limitation

WDSP 2.00 does not include piHPSDR's `RNNR` / `SBNR` symbols. Saturn Bridge now
detects those symbols at build time. If they are absent, NR3 and NR4 requests
fall back to EMNR so the bridge can link and run, but NR3/NR4 are not equivalent
to the current piHPSDR WDSP 1.29 behavior.

PureSignal has native-library and packet-path test coverage, but its feedback
level, automatic attenuation, correction quality, and RF delay values still
require controlled low-power validation on each Saturn hardware path before it
is treated as production calibrated. CESSB remains deferred until the bridge
can coordinate its compressor, linear-phase filter, and transmit-level
safeguards as one state transition.

The first controlled Saturn test produced feedback levels of 155-159 with 4 dB
feedback attenuation, zero gaps across more than 10,000 synchronized feedback
packets, and `maxTx` near the configured 0.6121 hardware peak. The ADC overflow
status remained clear. This validates the tested hardware path but does not
replace per-radio dummy-load validation after installation.
