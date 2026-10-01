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
