# RXC1 1.31.002 release candidate — preparation, not deployment approval

The G2 must keep its working 1.31.001 image until an explicit combined
firmware/host deployment decision. The candidate primary-slot image was made
from the reviewed revision-4 *post-route* checkpoint with only
`BITSTREAM.CONFIG.USR_ACCESS` changed from `0x53460003` to `0x53460004` at
bitstream generation. No new synthesis, route, IQ packet format or DMA driver
change was made for this export.

| Evidence | SHA-256 / result |
| --- | --- |
| Reviewed revision-4 archive | `55e8e14ae336aac68124f87352da941982ee82284de560ace6b55b4ee11505aa` |
| Reviewed post-route checkpoint | `b196b332064cdc23dbfbab22c76306a84b893655502849a62a285ed217f76346` |
| Candidate BIT | `54a5ae585a12848bc287da214b8eac812a3e248c648fbcc9b0a14f5499d2b4c6` |
| Candidate primary-slot BIN | `e5ab5935385a3e6ec3c5f124a6751866f1c26ad8d675d86912601bc35ef7d2fa` |
| Working 1.31.001 rollback BIN | `5fe07f14c119766481afa1ceff46113bfc085af87ee91fab1fe42eb67c75172d` |
| Routed setup/hold | WNS `+0.078 ns`, WHS `+0.049 ns`; zero failing endpoints |

The candidate BIT/BIN are kept in the separate release evidence package, not
in the live `FPGA/` image directory. `verify-rxc1-image.py` checks the BIT
packet structure, unique embedded AXSS/USR_ACCESS write, exact BIN payload,
and primary-slot capacity. The BIN is 9,730,652 bytes. `bitgen-rxc1.tcl`
refuses an unexpected checkpoint identity/part and refuses to overwrite an
existing artifact. The revision-4 source archive and checkpoint hashes must
be checked before invoking Vivado.

The baseline XDMA `CDC-7` at `user_reset_int_reg/PRE` remains a critical-class
diagnostic; matching the verified 1.31.001 baseline is **not** a waiver. The
small positive setup margin also needs explicit engineering sign-off. Keep
both qualifications in the deployment record. The candidate has not been
programmed or tested on the radio.

## Host pairing

The reviewed acquisition-owner, recovery, Saturn Go and diagnostics-page
source is pinned at commit `0116138cf0697a796d52d05866f96b2788c1c3ef`.
The final candidate host commit is the commit recorded in the native ARM
release manifest; it adds the candidate identity and Bridge policy. Bridge
recognizes `0x53460004` for direct-XDMA runtime/receive, but does **not**
qualify its RF TX. P2 still accepts major-1 firmware and its existing TX
policy is unchanged: during initial P2 receive testing, do not key the radio.
Both owners probe the RXC1 register bank and report unsupported, failed,
reset, or stale acquisition as unavailable rather than zero loss.
Both owners accept `SATURN_RXC1_POLL_ENABLED=0` at process startup. This
suppresses only RXC1 register transactions; IQ acquisition and other
telemetry continue. Their RXC1 JSON and the existing diagnostics page show
`disabled` / `unavailable`, with null counters. Unset the variable (or use
`1`) and restart the selected owner to resume five-second RXC1 polling.

## Inactive build and deployment gate

On a clean native ARM checkout of the final candidate commit, use the
existing release builder with `SATURN_RELEASE_OUTPUT_ROOT` set to
`/var/lib/saturn-state/release-staging`. It runs the P2, Bridge stub-native,
Saturn Go and web tests, then builds a native ARM application release with a
pinned source commit and SHA-256 manifest. It **does not activate** that
release or restart services. Validate the completed bundle using
`saturn-release-manifest.py validate` and
`saturn-release-install-root.sh --validate` before any installation.

Do not flash or activate until all of these are reviewed: candidate image and
host hashes, the native ARM build manifest, baseline CDC-7 disposition,
routed margin, rollback copies, and a controlled first-boot plan. The release
activation helper is intentionally disabled by root-owned policy until its
rollback transaction has been appliance-tested. Do not bypass that policy as
part of preparation.

## Controlled installation plan after approval only

1. Record the current backend, live build ID/version, service states, active
   binary hashes and web asset hashes. Verify off-device copies of the exact
   installed P2, Bridge and Saturn Go binaries, `p23test.html`, the remote web
   bundle and the 1.31.001 primary-slot BIN. Keep the rollback files outside
   the release install target and verify them again immediately before use.
2. Install the validated ARM release *inactive*. Rehearse rollback from the
   present legacy `/opt/saturn-go` layout on a spare appliance before
   enabling `saturn-release-activate-root.sh`; there is no pre-existing
   `/opt/saturn/current` release pointer on this G2. The activation helper's
   automatic failure rollback restores prior unit drop-ins, but successful
   first activation has no old immutable release to select. The legacy
   binary/web backups are therefore required for an operator-directed return.
3. With 1.31.001 (`0x53460003`) still installed, unkeyed and receiving,
   activate the compatible new ARM host through the approved transaction
   helper. Keep an auto-reconnecting receive client attached: activation
   commits only when the selected P2 **or** direct-XDMA owner (never both)
   reports fresh, PID-matched, active receive telemetry in addition to the
   Saturn Go commit. Confirm the old image ID and normal receive after this
   host change. If activation fails, let its automatic host rollback finish;
   do not flash the candidate.
4. Stage the candidate BIN under a dedicated path, verify its exact SHA-256
   and run `saturn-flash-fpga.sh --image <candidate> --primary --verify
   --dry-run --confirm e5ab59`. Only after the separate deployment approval,
   repeat without `--dry-run`. Never target the fallback slot. Follow the
   controlled reload/power-cycle procedure, then verify major/minor `1.31`,
   build ID `0x53460004`, RXC1 magic and normal clocks.
5. Qualify P2 and Bridge **separately** as the selected acquisition owner.
   Use the transactional `saturn-radio-backend-switch-root.sh` helper; never
   run P2 and direct XDMA together. In each mode verify all DDC identities,
   nonzero session/configuration/serial/token, freshness age, and the
   diagnostics page's unavailable-versus-zero distinction. Restart the
   owner with an old snapshot pending, exercise bounded request-error
   recovery, and check reset/configuration changes invalidate reads.
6. Compare receive timing with RXC1 polling enabled and disabled at the same
   sample rate and settings: DMA gaps, FIFO state, host drops, audio
   underruns, CPU/scheduler wait and counter freshness. Do not infer FPGA
   loss solely from a browser pause. Keep TX disabled during this receive
   gate, and do not claim RF TX qualification from the identity allowlist.

For that comparison, put `Environment=SATURN_RXC1_POLL_ENABLED=0` in a
separate root-owned systemd drop-in for **only the selected owner**
(`saturn-bridge.service` for `xdma`, `p2app.service` for `p2`), reload systemd
and restart that owner through its controlled owner path. Verify the other
owner remains inactive, IQ reception resumes, non-RXC1 telemetry continues,
and RXC1 is explicitly `disabled` / `unavailable` with null counters. Remove
the drop-in, reload and restart the same owner to re-enable polling. Allow
startup to settle before comparing equal-length receive windows; record
backend, rate, DDC, data width, firmware ID and owner PID for each window.

If host activation fails before flashing, verify the activation helper's
automatic rollback while 1.31.001 remains installed. If rollback is needed
*after* flashing, stop the selected owner and manager, restore the exact
1.31.001 primary-slot BIN with `--verify`, perform the controlled reload,
and verify live ID `0x53460003` **before** restoring/restarting the legacy
owner and Saturn Go. Restore the backed-up legacy binaries/web assets and
unit configuration using the rehearsed legacy-layout procedure; never start
the old Bridge against `0x53460004`. Finally verify the previous selected
backend, normal receive and RXC1 **unsupported/unavailable**, never zero
losses. Preserve failed-candidate logs and counters off-device.
