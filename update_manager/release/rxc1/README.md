# RXC1 1.31.002 release candidate — preparation, not deployment approval

This is the historical pre-deployment plan. The later operator TX qualification
decision is recorded in [tx-qualification-20261009.md](tx-qualification-20261009.md).

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
Both owners leave RXC1 polling disabled by default. Neither probes the RXC1
register bank on the working `0x53460003` image, even if polling is requested:
an absent AXI-Lite register cannot be safely discovered by reading it. On an
identified `0x53460004` candidate only, RXC1 still requires both
`SATURN_RXC1_POLL_ENABLED=1` and a one-use durable trial arm. The arm must
contain exactly `RXC1-TRIAL-0x53460004` followed by a newline in the selected
owner's `StateDirectory` as `armed`. The Bridge uses
`/var/lib/saturn-rxc1-bridge`; P2 uses `/var/lib/saturn-rxc1-p2`. The file
must be owned by that service's runtime user (`pi` or `saturn-radio`). The
installer creates the directory but never creates the arm, even if it detects
the candidate FPGA. The arm must be no more than ten minutes old; a malformed
or stale arm is spent and rejected. Before the first RXC1 BAR access, the owner
renames the arm to `spent` and syncs it to disk. A restart or reboot therefore
runs with polling off, even if the environment still says `1`. Re-arm only after
reviewing the previous trial. This prevents a repeated boot loop; it cannot
make the first hardware access safe if the FPGA itself resets the host.
An unknown or baseline build reports RXC1 `unsupported`; an unset, `0`, or
unrecognized setting reports `disabled`; an enabled candidate without an arm
reports `unarmed`. In all inactive cases the RXC1 counters are null, while IQ
acquisition and other telemetry continue.
Read, reset, and stale acquisition failures on an enabled candidate are
reported as unavailable rather than zero loss.

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
2. Install the validated ARM release *inactive*. Staging/installing that
   release does **not** update the privileged activator at
   `/usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh`.
   Before activation, use the bounded helper/config installation below.
   Rehearse rollback from the
   present legacy `/opt/saturn-go` layout on a spare appliance before
   enabling `saturn-release-activate-root.sh`; there is no pre-existing
   `/opt/saturn/current` release pointer on this G2. The activation helper's
   automatic failure rollback restores prior unit drop-ins, but successful
   first activation has no old immutable release to select. The legacy
   binary/web backups are therefore required for an operator-directed return.
3. With 1.31.001 (`0x53460003`) still installed, unkeyed and receiving,
   activate the compatible new ARM host through the approved transaction
   helper. Keep an auto-reconnecting receive client attached: activation
   commits only when the selected P2 **or** direct-XDMA FPGA owner (never both)
   reports fresh, PID-matched, active receive telemetry in addition to the
   Saturn Go commit. The supported service arrangements are direct XDMA
   (Bridge active, P2 inactive), P2 alone, and P2 with Bridge active as a
   **P2 client**. In the last arrangement P2 remains the sole FPGA owner and
   the Bridge must explicitly report runtime backend `p2`; activation
   preserves both services in P2-then-Bridge order. Confirm the old image ID and normal receive after this
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
   run P2 and direct XDMA together. P2 plus Bridge configured as a P2 client
   is a separate supported service arrangement, not simultaneous FPGA
   ownership. In each mode verify all DDC identities,
   nonzero session/configuration/serial/token, freshness age, and the
   diagnostics page's unavailable-versus-zero distinction. Restart the
   owner with an old snapshot pending, exercise bounded request-error
   recovery, and check reset/configuration changes invalidate reads.
6. Compare receive timing with RXC1 polling enabled and disabled at the same
   sample rate and settings: DMA gaps, FIFO state, host drops, audio
   underruns, CPU/scheduler wait and counter freshness. Do not infer FPGA
   loss solely from a browser pause. Keep TX disabled during this receive
   gate, and do not claim RF TX qualification from the identity allowlist.

For that comparison on the approved candidate only, put
`Environment=SATURN_RXC1_POLL_ENABLED=1` in a
separate root-owned systemd drop-in for **only the selected FPGA owner**
(`saturn-bridge.service` for `xdma`, `p2app.service` for `p2`), reload systemd
and create one valid `armed` file in that owner's StateDirectory before
restarting it through its controlled owner path. Sync the file and directory
before restarting. Do not arm both owners or have the installer create an arm.
If the first trial crashes, do not create another arm as part of recovery:
the next start must report `unarmed` and keep serving radio traffic with RXC1
off. In P2-client mode,
the Bridge can remain active but must report runtime backend `p2`; it must not
poll the FPGA RXC1 register bank directly. Verify IQ reception and non-RXC1
telemetry continue, then compare against polling disabled (`0` or no
override), where RXC1 is explicitly `disabled` / `unavailable` with null
counters. Remove the opt-in drop-in, reload and restart the same owner to
return to the safe disabled default. Allow
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

### Bounded privileged-helper installation (after approval, before activation)

Use the **installed, validated** immutable ARM release, not an unverified
checkout or archive. The reviewed activator in this source revision has
SHA-256 `72ac994576e17d40b88347688e590b4a1b0c3194ea0898011a0bc1280f78c160`;
the earlier reviewed helper was
`bee25b458ec0c2f75f7f0b78a01b8cbed016af4be86532fc5802f4fbe4113800`.
The new hash must match both the release copy and installed copy. The config
is host-specific: preserve its reviewed values and keep activation disabled
during this helper installation. Set `commit` to the exact
40-character commit in the validated ARM manifest; do not use a moving
`current` pointer as the source.

The installer checks every prerequisite and the source hash before writing,
verifies both backups before replacing the helper, and checks the installed
hash and unchanged config afterward. A failed check exits nonzero, so the
chained validation does not run. Do not retry over an existing backup without
reviewing the failed attempt and preserving its evidence.

```bash
commit=REPLACE_WITH_REVIEWED_40_HEX_COMMIT
sudo "/opt/saturn/releases/$commit/scripts/saturn-release-install-activator-root.sh" \
  "$commit" 72ac994576e17d40b88347688e590b4a1b0c3194ea0898011a0bc1280f78c160 \
  && sudo /usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh --validate "$commit"
```

Record the two pre-install hashes, the config hash after installation, the
validated release manifest hash and the commands' output off-device. Stop
if the config is already enabled or its paths/backend helper/URL values are
not the reviewed host values. Enabling activation is a **separate approved
configuration change**; record its new config hash. For a helper-only
reversal, restore `activator.before` and `config.before` with their recorded
modes/ownership, recheck hashes and leave activation disabled. This does not
replace the legacy-layout host rollback or the firmware-before-legacy-host
rollback order above.
