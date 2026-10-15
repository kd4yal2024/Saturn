# Controlled no RF ARM review of the accepted PureSignal candidate

This preparation kit preserves the accepted source and ARM binary unchanged.
The first hardware window is Direct XDMA, native identity, RX operation,
single-owner startup and orderly shutdown only. It does not install a release,
change production selection, collect keyed feedback, or authorize transmission.
Firmware stays 1.31.002 / 0x53460004; driver and RXC1 polling stay unchanged.

## Immutable candidate and reproducible build

The candidate source index is
`84387da737de7dca6e332739e0dc42b30f4ef5baf1a3db68505326faae14d800`;
ARM executable SHA256 is
`c462e688fc81842c152a48d8bcebe566f517a461eb48e898d40addb549b19292`.
The accepted review ZIP SHA256 is
`63dbc41286164b882ea47cee73367b55728711a8a6226f8977d56a68268d7616`.
`candidate-lock.json` pins native identity. The full reviewed ZIP is included
without alteration; extract it to a new directory before rebuilding.

`rebuild-reviewed.py` verifies both accepted checksum indexes, freezes external
build inputs, and builds from a fresh source copy and empty target directory.
It uses Rust 1.98.1, the pinned WDSP archive, Clang and the separately supplied
Trixie ARM sysroot. `build-inputs.json` checks the sysroot, symlinks, compiler,
Rust libraries, Cargo, archiver and linker inputs. Cargo uses --locked and
--offline: the matching dependency cache must already exist. It does not fetch
anything. Input drift or a different output binary is failure, not a newly
accepted candidate. Preserve the command, build log and rebuild-result.json.

```sh
python3 rebuild-reviewed.py \
  --review-root /absolute/path/to/extracted/accepted-review \
  --sysroot /absolute/path/to/frozen/trixie-arm-sysroot \
  --input-lock ./build-inputs.json \
  --output /absolute/path/to/NEW-rebuild-directory
```

The ARM binary is not an installer. A successful local cross-build does not
prove G2 dynamic linking, realtime scheduling or receive operation.

## Authorization and preservation before touching the G2

Obtain authorization for a no-RF exclusive hardware window. This kit has not
connected to the G2. Before stopping anything, preserve current release link,
selected backend, service unit/drop-ins and environment, executable hash,
process/device ownership, native build-info where supported, firmware identity,
and recent logs. Save those with the test evidence. Do not print credentials or
entire unrelated environments. Preserve the existing rollback release.

The previous bench report was an LP700 inline on ANT1 and a 1500 W 50-ohm dummy
load. Current bench confirmation is pending. No-RF software checks do not prove
that path or physical absence of RF; confirm independent inhibit and operator
stop arrangements in the authorized window. Do not use this document as RF
permission even if that bench is unchanged.

## Prepare an inhibited Legacy return path

Under that authorization, create a separate runtime-only systemd drop-in for
saturn-bridge.service that sets these three values:

```ini
[Service]
Environment=SATURN_REMOTE_TX_RF_ENABLED=0
Environment=SATURN_PS_MANAGED_RF_PERMITTED=0
Environment=SATURN_BRIDGE_TX_OWNER=legacy
```

Use a dedicated reviewed path such as
`/run/systemd/system/saturn-bridge.service.d/90-no-rf-review.conf`; first confirm
it does not already contain somebody else's work. Preserve an existing file
instead of overwriting it. Reload systemd and verify the effective environment.
This prepares return to the unchanged installed Legacy executable with RF still
disabled. Leave that inhibit in place after the review until separately directed.
Do not alter /opt/saturn/current or activate this candidate.

Stop the Bridge through the existing backend manager's `stop bridge` operation
so its startup selection remains recorded and the competing service is disabled
for the window. Direct XDMA additionally requires p2app.service inactive. Do not
switch backend or stop an active operator session without approval. Verify
systemctl state, competing processes, XDMA users and the listener port are clear.

## Static check and the first ARM execution

Transfer the verified kit to a new root-owned directory in the approved window.
Verify SHA256SUMS after transfer. A default invocation only hashes and inspects
ELF bytes; it does not execute ARM or open the radio:

```sh
python3 exec-no-rf.py
```

ZIP extraction may not retain executable permissions. After verification and
within the approved window, set 0755 on arm-candidate/saturn-bridge; keep the kit
root-owned and non-writable by other users. Recheck its hash before execution.

The first ARM execution is the read-only native identity query, which returns
before wisdom, sockets or devices in the accepted source:

```sh
python3 exec-no-rf.py --identity --confirm-hardware-window
```

Require AArch64, version 210, the exact source/native archive/header pins, real
native rather than stub, and default Legacy / opt-in-review wiring. Record
ldd/readelf dependency resolution for libfftw3.so.3, libm, libgcc_s and libc.
Mismatch, missing libraries or an unknown installed executable stops the test.

## Start the shared owner with both RF gates disabled

The launcher does not stop services or fix permissions. It requires root,
adequate real-time/memory-lock limits, inactive Bridge and p2app services, clear
XDMA device ownership, free localhost:50001 and the existing appliance radio
maintenance lock. It holds that lock through exec until the candidate exits.
This coordinates cooperating backend operations; it cannot stop unrelated root
processes that ignore the ownership policy.

The accepted constructor requires a topology-review attestation even for no-RF
startup. `--ack-reviewed-source-layout` acknowledges the reviewed source mapping
only, not proven hardware pair integrity. If that attestation is not acceptable
for this window, do not launch or silently weaken the gate. No layout is prepared
for keyed feedback here: the initial correction configuration remains disabled.

```sh
sudo prlimit --memlock=16777216:16777216 --rtprio=22:22 \
  python3 exec-no-rf.py --run-no-rf --confirm-hardware-window \
  --ack-reviewed-source-layout \
  --run-dir /var/tmp/NEW-saturn-no-rf-review
```

The clean environment hard-sets both RF permissions to 0, strict admission,
no two-tone exception, no Opus TX and no SATP. It cannot inherit permissive shell
settings or device-path overrides. Native required numeric settings are explicitly
inactive startup placeholders, not measured feedback limits. There is no mic,
IQ TX, key, PureSignal-enable or correction request in this test. The launcher
execs the candidate instead of spawning a timeout supervisor; the visible PID
is the actual Bridge process, and the lock survives until it exits.

## Observations and shutdown acceptance

Record launch.json, bridge.log, ready.json and perf.json in the NEW run directory.
Verify /proc for that PID: expected executable hash, both RF gates 0, shared
selection, and one TX/native owner thread with required scheduling. Verify the
existing sole C2H owner, not a second acquisition process. Thread counts alone
cannot establish an RF shutdown guarantee.

Allow startup/wisdom planning to finish; its 300 s budget is a no-RF startup
observation limit, not an RF protection bound. Require review-only status,
shared-ready-off, correct source pin and continued IQ/audio through one
authenticated read-only viewer. Keep the existing proxy unchanged; its existing
upstream localhost:50001 is used only while the original Bridge is stopped.
Do not open a second operator or submit settings/PTT. Observe RX for a bounded
agreed window, recording frame continuity and resource use without altering
polling or TCP/display settings.

Request normal SIGINT or SIGTERM to the actual PID. Capture exit status and the
final stopped review status. Require zero exit, no faults, no failed/Blocked
cleanup, and no remaining candidate or reader/native owner. If it hangs, retain
inhibit and investigate; do not automatically SIGKILL it, declare cleanup
successful or restart Legacy over a remaining owner. If a manual kill becomes
necessary, record shutdown as failed and obtain new recovery direction.

After verified owner exit, use the existing manager's `start bridge` operation
to return to the preserved installed Legacy executable with the runtime no-RF
drop-in still present. Verify executable identity, selected backend, RF=0,
normal RX and no parallel shared owner. The launcher never restarts Legacy
automatically. Native identity, RX, owner startup, shutdown and Legacy return
are separate results in the evidence record; none is a physical RF timing or
IMD result.

### Bounded fresh process telemetry wait

The 20261014 hardware window exposed an immediate post-start missing-file race
in the external return harness. Preserve the old MainPID and a wall-clock start
boundary before starting Legacy, then use `wait-legacy-return.py` with the new
actual MainPID, preserved executable SHA256 and that boundary. Its default wait
is 30 seconds, hard-capped at 60; it retries absent, torn and old telemetry
without restarting the service. Stable process-start ticks, both RF gates off
and fresh matching-PID ready telemetry are required. Process replacement,
failure or TX-not-off fails verification. Follow it with a bounded IQ/audio
delivery observation before removing maintenance guards. Telemetry readiness
alone is not receive delivery or physical RF-off proof.

## Before any later keying

The concrete collection design is in FEEDBACK_MEASUREMENT_METHOD.md. It remains
a proposal, not an implemented diagnostic in this accepted executable. It must
be reviewed and implemented without adaptation or a second acquisition reader
before RF measurements can be collected. Current bench confirmation, actual
measured limits and separate RF authorization remain mandatory.
