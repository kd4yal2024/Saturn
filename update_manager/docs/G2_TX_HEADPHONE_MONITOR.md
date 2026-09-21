# G2 TX headphone monitor

Implementation status (September 20, 2026): the operator deployed the hardware
audio-mute correction and confirmed audible TX MON at G2's headphone jack.
**Playback continuity remains unresolved:** the operator reports choppiness.
The latest TX diagnostics show zero microphone underruns, backlog drops and
TX FIFO faults, while MON's separate FIFO frequently reads empty. This points
to MON buffering/pacing but does not establish clean over-the-air audio.

The MON commit includes the UI/protocol integration, processed-IQ monitor,
non-owning register access and hardware-mute correction. Separate RX recovery,
RX recorder and TX ingress-latency edits remain outside that commit. Historical
deployment/test records below describe the combined working-tree builds.

Commit-isolated verification: exported the staged MON-only tree to a temporary
directory; 289 bridge stub-native tests passed (1 ignored), 487 web tests passed,
and TypeScript, production bundle and template seam checks passed. No hardware
or installed service was changed by these checks.

MON is a local, processed transmit-audio monitor, not an off-air/RF monitor.
It is supported by the primary PCB2 direct-XDMA backend (AIC23B codec), not P2
or PCB3/AIC3204. Unsupported bridges leave the browser control disabled.

- `tx_monitor:0,true|false;` selects MON; default off, not persisted by browser.
- `tx_monitor_level:0,<dB>;` controls digital level, -60 to -6 dB, default -30.
- `tx_monitor_supported:0,true|false;` advertises capability/fault availability.
- All commands require the existing operator role. Neither command arms or keys RF.
- MON plays only actual keyed TX IQ. RX, two-tone, CW, unsupported modes and
  RF-inhibited TX produce no monitor audio. Unkey invalidates queued audio;
  operator disconnect clears MON. MON faults disable only MON and are published.

The processed WDSP output is copied **after** successful DUC delivery into a
bounded best-effort queue. A separate worker demodulates SSB/digital sideband,
AM or FM, applies a 127-tap anti-alias filter and 4:1 decimation (192 to 48 kHz),
DC removal and independent attenuation. FM level is referenced to 5 kHz
deviation; it is not a calibrated modulation measurement. CW sidetone is outside
this feature. Requested mode changes deferred by TX DSP do not change MON's
decoder prematurely; each frame carries the actual DSP mode.

The worker owns H2C1 codec DMA, rejecting audio older than 30 ms and dropping
backlog instead of delaying RF output. It initializes the AIC23B when MON is
selected, asserts hardware audio mute while idle, disables analog bypass/sidetone,
uses 16-bit stereo I2S, and writes small bounded blocks only with FIFO headroom.
Both channels carry the same audio. Headphone analog attenuation is fixed at
-12 dB in addition to the displayed digital MON level. The AIC23B shares its DAC
with line outputs: this does not promise electrical isolation from line-out.
During valid MON playback, the hardware audio mute is released, matching
Saturn/P2's playback path. It is reasserted on silence, stop and cleanup. This
does not guarantee headphone-only routing: keep headphones connected and
external speakers off for qualification.

Register references: `sw_projects/common/saturnregisters.c` (`CodecInitialise`,
`SetSpkrMute`), `src/xdma_audio.rs` (codec DMA/FIFO geometry), and
[TI AIC23B datasheet](https://www.ti.com/lit/ds/symlink/tlv320aic23b.pdf).
No codec reset, microphone capture, DUC address, RF enable or drive write is
performed by the MON worker. Shared FPGA read/modify/write operations are
serialized so codec FIFO and speaker-mute updates preserve unrelated TX bits.

## Supervised acceptance required

1. Build/deploy bridge and web assets together with rollback copies. Do not
   enable MON automatically during deployment. Confirm MON defaults off, correct
   backend capability, RX unchanged, and no spontaneous TX.
2. With suitable dummy load and operator-controlled PTT, enable MON at -30 dB;
   start with headphones off-ear. Verify both headphone channels, low starting
   level and no speaker output. Increase gradually. Confirm line-out behavior.
3. Verify voice in USB/LSB, AM and FM; compare TX EQ/processor changes between
   transmissions. Confirm MON volume does not change mic/RF meters.
4. Verify MON off, unkey, operator disconnect, source-stall watchdog and service
   stop silence the headphones without stale audio on the next transmission.
5. Check DUC/RX FIFO telemetry, CPU usage and audible monitor continuity under
   normal display load. DMA blocking/driver failures and actual codec routing
   require hardware validation; stub-native unit tests cannot establish these.

No RF keying or live codec writes are part of automated unit tests.

## Software verification (2026-09-20)

- Bridge stub-native suite: 286 passed, one existing ignored benchmark.
- MON tests linked with local WDSP 2.00: 7 passed; native bridge build passed.
- Web suite: 492 passed; TypeScript, production bundle and template seam passed.
- Chrome layout validation: all 24 scenarios passed.
- Strict Clippy is not clean repository-wide (existing warnings); the new
  MON module has no Clippy diagnostics. No unrelated lint cleanup was performed.

At the initial software-verification stage, the existing RX underrun-recorder
edits were preserved. No live service,
firmware, codec register, transmit setting or installed web asset was changed.

## TX latency and peripheral ownership repair (September 20)

- Timestamp audio at bounded ingress; reject frames queued before native TX
  setup completed or older than the configured microphone queue budget. Start
  DSP/DUC pacing clocks after setup, not before the potentially slow rebuild.
- Limit the microphone sample queue to configured prefill plus two 512-sample
  blocks. Default: 3072 samples / 64 ms, instead of 48000 samples / one second.
  Preserve configurable prefill (up to 250 ms), RF qualification and watchdogs.
  This bounds bridge queueing, not end-to-end latency or native arm duration.
- Open MON's registers without taking RF ownership: peripheral open/drop no
  longer calls the owning handle's force-safe-RX routine. RF shutdown remains
  the main session owner's responsibility.
- Log MON DAC unmute/mute, DMA write counts, quantized PCM peak and FIFO words.
  Nonzero DMA writes alone do not verify analog headphone playback.
- No Opus negotiation changes: use `tx_opus=0` for qualification of this repair.

`build-tx-mon-staged.sh` builds and runs targeted native tests in the isolated
G2 staging directory. `deploy-tx-mon-staged.sh --check` only validates preflight.
Running the deploy script without that flag requires sudo, saves a rollback
copy, installs only the bridge binary and automatically restores the previous
binary if the restart/health check fails. No web assets or service settings are
changed. Release PTT/MOX and lock TX before deployment.

Repair verification: stub-native suite 289 passed / 1 ignored; G2 ARM release
build succeeded with real WDSP2; 32 targeted native tests passed (7 MON,
23 microphone/routing/buffering, 1 arm epoch and 1 peripheral ownership).
Both staging scripts pass `bash -n` and ShellCheck. Clippy completed with
existing repository warnings; physical headphone output remains unverified.

## Hardware audio-mute correction (September 20)

The operator reports the same G2 headphones work with Thetis. The deployed
latency repair produces nonzero MON PCM and a consuming codec FIFO, but no
audible MON. Unlike P2 playback, the original MON implementation held RF GPIO
bit 4 (`SetSpkrMute`) asserted at all times. The assumption that this control
could remain asserted independently of the chassis headphone path is unproven.

The candidate correction releases only that bit after a valid MON DMA write
and DAC-unmute request, then checks the playback generation again. Silence
asserts hardware mute before attempting all codec mutes and audio FIFO reset.
The existing codec configuration, conservative gains, TX safety gates, RX
fixes and latency repair are unchanged. No codec reset is introduced.

File-backed tests exercise the actual output method: nonzero stereo bytes,
hardware mute/re-mute, preservation of unrelated RF GPIO bits, and stop races
before DMA, after DMA and during unmute. These cannot qualify analog audio;
operator-controlled headphone testing remains necessary. The playback log
now reports a DAC-unmute *request*, not a verified physical DAC state.

### Previous latency-repair artifact (already deployed)

The following artifact predates the hardware-mute correction. Do not redeploy
it over the correction.

Tested artifact: `/home/pi/saturn-tx-mon-fix.guNjTn/saturn-bridge`

SHA-256: `a42023d00743a6a97f438dc4cd5ccecc87ae3f6002f9f1cf7d443ab4ea65232f`

Operator commands (PowerShell):

```powershell
ssh pi@192.168.0.139 "bash /home/pi/saturn-tx-mon-fix.guNjTn/deploy.sh --check"
ssh -t pi@192.168.0.139 "sudo bash /home/pi/saturn-tx-mon-fix.guNjTn/deploy.sh"
```

The second command interrupts RX and asks for the sudo password in the terminal.
Reconnect using `https://192.168.0.139:8443/remote-next?transport=split&tx_opus=0&tx_cfc=1`.
MON starts off. After a supervised TX test, inspect `MON output` and `TX diag`
in the service journal for PCM peaks, FIFO state and the bounded microphone queue.

### Hardware-mute correction: deployed; audible output confirmed

Artifact: `/home/pi/saturn-mon-mute-fix.72V400/saturn-bridge`

SHA-256: `17cebacbc40bb480186798eaead9b46d223696139fb748685c8db90f265fb636`

Verification: 291 local stub-native tests passed / 1 ignored; native ARM WDSP2
build and 34 targeted native tests passed. Clippy completed with existing
warnings, none in the MON module. Script syntax, ShellCheck and diff whitespace
checks passed. No installed binary, running service or live codec was changed.
During testing the operator switched to P2/Thetis; deployment must wait until
the operator closes Thetis and selects **Use XDMA / TCI** again. The preflight
explicitly rejects P2 ownership or a stopped bridge.

With PTT/MOX released, TX locked and the XDMA bridge ready, run in PowerShell:

```powershell
ssh pi@192.168.0.139 "bash /home/pi/saturn-mon-mute-fix.72V400/deploy.sh --check"
ssh -t pi@192.168.0.139 "sudo bash /home/pi/saturn-mon-mute-fix.72V400/deploy.sh"
```

The installer expects the previous latency-repair binary, backs it up, and
rolls back if readiness fails. Reconnect with the PCM URL above. With headphones
connected but initially off-ear and external speakers off, start MON at -30 dB
and use only operator-controlled voice PTT with a suitable load. Look for
`MON playback enabled` followed by nonzero `MON output` peaks. Confirm audible
voice and silence after unkey/MON off. The operator subsequently confirmed
audible headphone output but reported choppiness; do not treat this as complete
playback-quality or off-air qualification. Rollback from this deployment is
`/opt/saturn-go/tx-mon-backup.lbuDAE/saturn-bridge`.
