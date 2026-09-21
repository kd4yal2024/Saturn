# Saturn Remote native TX audio and G2 MON

SATP's native audio path is separate from browser microphone capture:

    Native ASIO (48 kHz mono) -> SATP v2 UDP -> jitter buffer -> TX rate matcher
       -> WDSP TX processing -> DUC IQ -> RF output
                                    -> existing G2 headphone MON worker
    Saturn Remote -- authenticated WSS/TCI --> source selection / pairing / PTT

MON is the existing processed-voice transmit monitor at the **G2 physical
headphone jack**, not browser sidetone or Windows playback. The MON switch and
level remain independent of microphone gain and RF drive. RX, CW, and two-tone
remain silent. Source changes invalidate pending monitor playback; MON cannot key RF.

## Compatibility and use after deployment

### Staged preflight (no installation)

The staging helpers are `update_manager/saturn-bridge/scripts/build-satp-staged.sh`
and `preflight-satp-staged.sh`. They are copied to a dedicated G2 bundle as
`build.sh` and `preflight.sh`; do not run them directly in the source checkout.
The bundle contains a checksummed source snapshot, matching generated web assets,
the base commit ID (marked dirty), and checksums of the currently installed bridge
and four web files. The ARM build uses real WDSP, a separate target directory,
and an isolated test-status path. It never runs the production bridge executable.
After the SATP/MON tests pass, it seals the payload with `SHA256SUMS`.

Run `bash /home/pi/<staging-directory>/preflight.sh --check` without sudo.
It checks payload/source integrity, the installed baseline, ARM64/native linkage,
running-versus-installed bridge identity, free disk space, and fresh XDMA RX
readiness with TX unkeyed and inactive. It has no install mode. A passing check
does not verify the Windows executable, physical audio/RF, or authorize a later
restart. An eventual installer must repeat the checks, back up the matched
bridge/UI set, and restore that set if startup fails. Do not reuse the older
MON-only deployment script for this update.

Helper checks: `bash -n` and `shellcheck` on both scripts, plus
`python3 update_manager/saturn-bridge/scripts/test-satp-preflight.py`.

### Explicit matched-set installer

`scripts/deploy-satp-staged.sh` and `scripts/deploy-satp-staged.py` (under
`update_manager/saturn-bridge/`) are staged as `deploy.sh` and `installer.py`.
A separate `INSTALLER-SHA256SUMS` pins those helpers and the existing payload
manifest; the sealed source/build payload is not rebuilt or modified.
`bash deploy.sh --check` remains read-only. Only `sudo bash deploy.sh --install`
performs installation. Release PTT/MOX, lock TX, close Remote/Thetis clients, and
stop the native sender first; installation interrupts RX.

The installer locks against simultaneous invocations, freezes verified old/new
payloads in a private root-owned `/opt/saturn-go/satp-backup.*` directory, rechecks
RX/baseline, stops the bridge, and replaces the bridge plus Remote HTML, JS, JS
checksum, and Settings HTML. It verifies three successive fresh RX-ready samples
and running-binary identity after restart. Detected installation failure attempts
a matched-set rollback. An unrecoverable rollback reports failure and retains
the backup; power loss/SIGKILL cannot be recovered automatically.

The printed `--rollback /opt/saturn-go/satp-backup.ACTUAL_SUFFIX` command restores
that backup after validating checksums and ensuring no unrelated update would be
overwritten. It accepts an inactive/failed bridge, but an active bridge must be in
fresh RX-ready state. No SATP configuration is enabled by installation itself.
Regression tests use isolated temporary files and a fake service controller:
`python3 update_manager/saturn-bridge/scripts/test-satp-installer.py`.

### Paired operation

Update the G2 bridge, Remote template **and generated JS bundle**, and the native
Windows audio sender together. The secure receiver rejects legacy SATP v1 packets.
The sender's legacy v1 diagnostic CLI is not a paired radio source.

1. In Settings, enable the SATP listener on G2's LAN IPv4/UDP port (default 50100).
   Prefer restricting the source IPv4 to the Windows audio PC. Saving listener
   settings restarts an active bridge; do this in RX. Leave the startup source as
   browser/TCI unless native-only startup is intentional.
2. Open Saturn Remote through its authenticated HTTPS service as the operator.
   In RX, click **Pair native sender**. Copy the temporary key.
3. In the updated native sender's Network tab, set G2's IP/port and paste the key.
   Apply settings, select a 48 kHz ASIO device/input, then Start audio.
4. Remote must show native audio healthy and a changing level when speaking.
   Select **Native SATP** and wait for the bridge acknowledgement. This changes
   the running source without restarting RX or requiring browser microphone permission.
5. Enable MON at a low level. With the operator's usual TX safety checks and a
   suitable dummy load, use PTT and verify G2 headphone audio and RF audio independently.

Keep Remote connected. After a sender restart, operator disconnect, bridge
restart, or expired lease, stop native audio and repeat pairing. No automatic
key-on or automatic re-pairing occurs. Native source selection is runtime-only;
the startup configuration still governs a bridge restart.

## Control and security contract

Only the active TCI operator can issue `saturn_satp_control:pair|renew|tci|satp;`.
Backend processing rechecks ownership. Pairing and source changes require RX;
TX is blocked while a source change awaits the idle TX worker's acknowledgement.
The existing authenticated TLS proxy remains the trust boundary for TCI.

`pair` returns `saturn_satp_key:<64 hex characters>;` only to the requester.
The key is 32 bytes from the OS RNG. It is not part of broadcast state, status
files, diagnostics, saved preferences, or native sender TOML. Remote keeps it
only in a temporary password field, clears it on disconnect, and hides it after
60 seconds. Copying intentionally places it in the OS clipboard; clear clipboard
history when appropriate. The native app retains its applied key only in memory.

The lease lasts five seconds and only its operator can renew it. Expired leases
cannot be revived by renewal. Operator disconnect revokes the lease immediately.
The first valid packet binds the lease to the sender's full UDP endpoint,
session ID, and stream ID. Changing those requires a new pairing. The source-IP
allowlist remains an additional restriction, not authentication.

SATP v2 preserves the v1 32-byte header and 512-byte float32 payload, sets the
header version byte (offset 4) to 2, and appends a 32-byte HMAC-SHA256 over the
entire 544-byte header/payload. Total datagram size is **576 bytes**. The magic
remains `SAT1`. The receiver validates the fixed mono/48 kHz/128-frame contract,
finite samples, and the MAC before accepting session/timeline changes.

Only advancing authenticated sample counters refresh audio liveness; duplicates
and reordering do not. A bounded 64-packet replay window permits each reordered
packet only once; PTT establishes a fresh sample-counter floor so pre-arm audio
cannot be replayed into the next transmission. Invalid packets never refresh
liveness. Lease/audio loss clears
buffered media and requests normal TX disarm. Existing source-stall, RF, control,
and power protections remain in force. Audio authenticity does **not** provide
confidentiality: use a trusted LAN or VPN, not an exposed Internet UDP listener.

`saturn_satp_state` broadcasts source, receiver-enabled, paired-to-this-client,
ready, health, native peak dBFS, buffer frames, missing packets, generation, and
pending-source-acknowledgement. Remote blocks voice TX if this state is stale,
pending, unpaired, or not ready. Ordinary browser mic operation remains available.

## Verification and latency tuning

Operator acceptance on 2026-09-20 (2026-09-21 UTC): the Windows release build
with real ASIO completed, native 48 kHz Voicemeeter audio reached G2's TX/WDSP
chain, and the operator heard smooth microphone MON at G2's physical headphone
jack. The observed ASIO callback size was 512 frames; defaults were retained.
This confirms the tested headphone path, not off-air RF audio quality or a
measured microphone-to-RF/MON latency. The full failure/mode matrix below still
requires separate hardware acceptance.

Automated coverage includes authentication/tampering, wrong sender/session,
re-pairing, lease ownership/expiry, replay liveness, RX-only source selection,
source readiness, the real localhost UDP-to-bounded-ingress path with RF
inhibited, browser PTT source choice, and source-independent G2 MON gating.

Hardware/ASIO acceptance is still required before deployment is called proven:
test browser mic permission denied with native selected; lost UDP, stopped sender,
Remote disconnect/reconnect, expired lease, MON off/on and level changes; verify
RX/CW/two-tone silence and no stale audio after release/source changes. Confirm
off-air TX audio separately from headphone playback.

Buffer defaults are deliberately not reduced without hardware measurements.
128 samples at 48 kHz represent 2.667 ms, but a 512-frame ASIO callback still
delivers bursts every 10.667 ms. Record native effective buffer/callback timing,
sender queue depth/drops, SATP buffer/missing packets, and TX rate-matcher
occupancy/underflows together. Measure microphone-to-RF and microphone-to-MON
delay, then change one buffer at a time; keep destination-clock rate matching.
Do not infer end-to-end latency from packet size or add overlapping counters.
