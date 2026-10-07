# FPGA image identity readiness

## Host change

The P2 app and direct XDMA bridge read FPGA register `0x4004` through their existing register-access paths. The Saturn Go `/p23_perf` response compares a fresh reading with `release/fpga-image-identity-v1.json`. The App / Firmware Info display, Saturn Go identity log, RX Measure status, and downloaded RX spectrum JSON show the resulting firmware version, RX filter, and raw build ID. RX Measure no longer accepts a manually entered firmware label.

The manifest contains two image identities verified against the generated BIN and BIT files in the earlier preparation bundle `silverforge_identity_readiness_20260929T2158EDT`. Its source manifest SHA256 is `75c866b6616627af33d12a5f847e5f77a5b1a73bc6899a1d6e8353c3f08f2e6f`. The actual BIN SHA256 values, embedded AXSS/USR_ACCESS IDs, and BIN/BIT payload equivalence were checked independently:

| Embedded ID | Firmware | RX filter | BIN SHA256 |
| --- | --- | --- | --- |
| `0x53460001` | `1.30.001` | `18/Q20` | `7bc6aea0659b81e30c5b19ad547d4946eb56b4d2c41d237fd0606c730fbbd1f4` |
| `0x53460002` | `1.30.002` | `22/Q24` | `8d472d4f1484d6edd8300cdebe1802795bf65ffaac21f0d7c4939baa5be9d31d` |

An older or unrecognized ID remains visible as a raw hex value and is labeled `1.30 — build unidentified` when the major/minor register values are 1/30; the RX filter remains unidentified. Stale or unavailable telemetry cannot produce an identified label.

## G2 observation and isolated host verification

The established SSH connection to `pi@192.168.0.139` reached the active XDMA bridge. A read-only `pread` of `/dev/xdma0_user` at `0x4004` returned `0x53460002` at `2026-10-01T02:41:19.426806Z`. Fresh bridge telemetry agreed, reported major/minor `1.30`, and identified the active backend as `xdma`.

The changed Saturn Go manager compiled on the G2's native ARM system and ran from `/home/pi/silverforge-identity-verify` on loopback ports `18080` and `18443`. It did not replace the production manager or restart the bridge. The isolated manager's fresh telemetry, App / Firmware Info script, and identity log all reported `1.30.002`, `22/Q24`, and `0x53460002`.

A new 30-second RX Measure capture through that isolated manager and the live bridge started at `2026-10-01T02:55:35.548Z`. It contains 230 raw IQ spectra with good coverage. Its automatically generated `firmwareLabel` and structured `fpgaIdentity` both report `1.30.002`, `22/Q24`, and `0x53460002`. The browser's socket allowlist admitted only read-only status/display requests and per-client IQ start; tuning, sample-rate, audio, and RX setting commands were blocked. Bridge frequency remained 3,899,000 Hz, sample rate 384,000 Hz, and IQ/audio active before and after. The temporary client was closed and the bridge connection count returned to its starting value. Full evidence is in `silverforge_identity_host_verification_20261001T023106Z/capture_verification.json` and `new_capture.json`.

These checks validate the host change against the running G2 hardware in an isolated manager. The production manager and browser have not been updated; their deployed display and capture paths remain to be verified after an authorized host deployment. No FPGA image change is needed for the current live ID.

## Board check before any future image change

1. On the running Saturn host, open **Saturn App / Firmware Info** and record its `Build ID` line. This is the live read from register `0x4004` through the active P2 or XDMA backend after the host update.
2. Confirm the service telemetry is fresh and belongs to the active service. The RX Measure capture button performs this check automatically.
3. Compare the exact raw ID and register major/minor version with the verified image manifest. The G2's active XDMA bridge reported `0x53460002` with major/minor `1.30` on 2026-10-01 UTC, so this is a host labeling change. If a later ID differs, report the exact value and stop before proposing an FPGA image change.
4. Make one short RX Measure capture and inspect `fpgaIdentity` in its JSON. For the identified Q24 image it must contain firmware `1.30.002`, subversion `2`, RX filter `22/Q24`, and build ID `0x53460002`. An unknown ID must have a null subversion and RX filter.

## Local validation

- `cargo test fpga_image_identity --manifest-path update_manager/rust-server/Cargo.toml`
- `npm run typecheck`, `npm test -- --run tests/rx-spectrum-measure.test.ts`, and `npm run check:seam` in `update_manager/remote-web`
- `bash -n update_manager/scripts/g2-version-info.sh`

No FPGA image is selected or programmed by this change. Production host deployment and a post-deployment display/capture check remain separate actions.

## Firmware 1.31.001 offline-verified host readiness (2026-10-04)

GoldMarsh built the 1.31.001 test image from the 1.30.002 source with the ten DDC final 27-to-24-bit output conversions changed from wrap to saturating clamp (`-8,388,608..+8,388,607`). The 22/Q24 RX FIR, 24-bit I/Q transport, channel ordering, register map, FIFO telemetry ABIs, DDS, and TX path are intended to remain unchanged. Those properties passed GoldMarsh's offline acceptance. GoldMarsh reports that Jerry subsequently programmed the image on the G2; readback showed build ID `0x53460003`, major/minor `1.31`, primary configuration, and good clocks. This is an identity check, **not** an RX or TX validation. No clamp-event counter is exported.

The built image has firmware major `1`, minor `31`, subversion `1`, and USR_ACCESS at `0x4004` equal to `0x53460003`. The version register `0xC000` is field-encoded: the expected value is `0x024001F0 | clock_mon`, not the integer 31. FIFO_Monitor BUILD_ID `0x56323900` and ADC_V30 BUILD_ID `0x56333000` identify telemetry register ABIs and are not image IDs. The offline-verified entry in `fpga-image-identity-v1.json` is backed by GoldMarsh's `goldmarsh_fw131_20261004T130938Z/REPORT.md` and identity manifest SHA-256 `04bf23010e618b2dbee8b2658d054bab88e509976b4d60f18b8fe24ff75f9cc0`. The original top-level manifest provenance still refers to the two 1.30 images; the 1.31 entry carries its own provenance fields.

The 1.31 BIT SHA-256 is `1c8d19a9f253cdc8b36a7ec99bc5d5290f5843680e1435c42d5bcb8fccfb7af7`; the BIN SHA-256 is `5fe07f14c119766481afa1ceff46113bfc085af87ee91fab1fe42eb67c75172d`. I independently hashed the local BIT, BIN, source identity manifest, and packaged 1.30.002 restoration BIT/BIN; all matched GoldMarsh's values and the previous 1.30.002 record. The BIN also compares byte-for-byte with the BIT configuration payload. This verifies the local handoff files, not hardware behavior. Fresh matching host telemetry will now label the image `1.31.001` with `22/Q24 saturated`; unknown or stale IDs remain unidentified, and the telemetry screen captures the raw identity.

| Host path | 1.30.002 baseline | 1.31.001 test image |
| --- | --- | --- |
| P2 app | Major-1 acceptance; existing FIFO/ADC ABI probes | Major-1 Protocol 2 and TX interface remains accepted on minor 31; a regression test prevents a minor-version TX hold |
| Bridge direct XDMA | Primary PCB2 firmware minors 27–30 remain accepted and RF-TX-permitted under the existing host policy | Primary PCB2 minor 31 is RX- and RF-TX-permitted only when USR_ACCESS is `0x53460003`; other minor-31 IDs remain rejected |
| Saturn Go identity | Exact verified match gives `1.30.002`, `22/Q24` | Exact `0x53460003` and major/minor 1/31 match gives `1.31.001`, `22/Q24 saturated` |
| XDMA kernel module | Generic register/DMA transport | No driver version gate or wire-format change; no driver edit justified by this firmware delta |

Offline checks for this staged host set: Bridge unit tests cover existing 1.30 acceptance and exact-ID 1.31 RF-TX permission while rejecting unknown minor-31 IDs; P2 tests verify major-1 compatibility on minor 31 and exercise the unchanged FIFO and ADC marker contracts; Saturn Go tests identify 1.31 only for the exact ID and version pair; web tests distinguish identified 1.30.002, identified 1.31.001, and unknown images. This removes the Bridge host's minor-31 TX hold; it does **not** validate RF output or qualify the physical radio. The P2 path was already able to transmit on major-1/minor-31 firmware; the G2 ran P2 when this section was written. The updated Bridge, identity manifest, and telemetry display were not yet deployed there at that time. Before a radio TX test, obtain the combined firmware/host review, use a supervised dummy-load setup and bounded low-power keying procedure, and keep the exact 1.30.002 restoration image available. No radio access or RF transmission was performed for this code change.

## RXC1 1.31.002 release preparation (2026-10-07)

The isolated revision-4 RXC1 post-route checkpoint has been exported to a
candidate BIT and primary-slot BIN with a new `0x53460004` USR_ACCESS identity.
The resulting BIN is **not installed** and is deliberately kept outside the
live `FPGA/` image directory. The manifest's exact-ID 1.31.002 entry uses the
same 22/Q24 saturating receive filter description as 1.31.001; it does not
claim that counter acquisition has succeeded on hardware. The Bridge allows
the new identity for direct-XDMA runtime/receive but keeps RF TX unqualified
for this candidate. P2's major-1 compatibility and TX policy are unchanged,
so initial P2 qualification must be receive-only by operating procedure.

The export and rollback hashes, routed timing and baseline CDC caveat, native
ARM build gate, and separate P2/Bridge receive plan are recorded in
`update_manager/release/rxc1/README.md`. The G2's working 1.31.001 image
and host services remain unchanged pending a combined deployment review.
