# Thetis P2 Compatibility Regression Checklist

Date: 2026-02-25

Purpose:
- Keep the hardened `p2app` implementation compatible with Thetis (HPSDR Protocol 2 over Ethernet).
- Provide a repeatable before/after regression method for future `p2app` changes.

Scope:
- the converged hardened `P2_app` implementation running as a P2-compatible shim for Thetis.
- Ethernet/UDP Protocol 2 discovery, control, streaming, CAT-related behavior.
- On-wire compatibility first; internal refactors are allowed if behavior is unchanged.

Current status:
- The April 7, 2026 restart/reconnect gate has been treated as passed for cutover planning.
- The repo should still keep concrete capture/run records with the checklist whenever reconnect-related behavior changes again.

## Rules (Do Not Break By Accident)

Treat these as regressions unless intentionally changed and documented:
- UDP port numbers used by Thetis/P2 traffic
- UDP payload lengths on each port
- Packet cadence/timing patterns (especially high-priority and DDC IQ streams)
- Startup order (`discovery -> general/control -> specific/high-priority -> streaming`)
- Key field semantics in high-priority/general packets (run bit, ports, CAT port, drive/Alex fields)
- Thetis sends every high-priority and DDC-specific CONTROL packet with sequence
  number zero (only its data streams increment). Control packets must never be
  gated on duplicate sequence numbers; doing so freezes frequency/drive/run
  updates after the first packet (root cause of the 2026-07 "cannot change
  frequency" regression)

## Important Length Note (Wireshark)

Wireshark `udp.length` includes the 8-byte UDP header.

When comparing to current app packet constants:
- UDP payload length = `udp.length - 8`
- Do not compare `udp.length` directly to payload constants without subtracting 8

## Capture Setup (Required)

Capture-side consistency:
- Use the same capture side for before/after comparisons (preferably the same host/interface).
- Do not compare timing/cadence across captures taken on different hosts unless necessary.

Version stamping (record every run):
- app git commit
- Thetis version/build
- FPGA firmware version/bitstream
- Active runtime app identity / binary family
- `p2app` startup args/profile (for example `-s -p`)
- Panel mode override (if used): `SATURN_FRONT_PANEL_MODE=...`
- Capture side (`Thetis host` or `Saturn`)
- Interface used (`eth0`, etc.)
- Test state (`idle`, `RX`, `TX`)

## Automation Helpers

Repo helpers now exist to make the checklist more repeatable:

- `tools/protocol2_regression_capture.sh`
  - writes a run-record sidecar with the same metadata this checklist asks for
  - launches `tcpdump` with the standard Saturn/Thetis UDP filter
- `tools/protocol2_regression_summary.sh`
  - reads a saved pcap with `tcpdump -r`
  - summarizes observed UDP source/destination ports, payload lengths, counts,
    and simple gap timing per flow

Example capture:

```bash
./tools/protocol2_regression_capture.sh \
  --iface eth0 \
  --out-dir ./captures \
  --label after-clientcontrol \
  --client Thetis \
  --app p2app \
  --state RX
```

Example summary:

```bash
./tools/protocol2_regression_summary.sh ./captures/after-clientcontrol-*.pcap
```

## Packet/Port Expectations (P2-Compatible)

Common Thetis/P2 UDP ports to monitor:
- `1024` general/discovery
- `1025` high-priority from SDR
- `1026` mic from SDR
- `1027` high-priority to SDR
- `1028` speaker audio to SDR
- `1029` mic/DUC IQ to SDR
- `1035-1044` DDC IQ streams (as used)

Common app payload lengths to verify:
- discovery request/reply payload: `60`
- DUC specific payload: `60`
- high-priority from SDR payload: `60`
- DDC specific payload: `1444`
- high-priority to SDR payload: `1444`
- mic packet payload: `132`

## Wireshark / tcpdump Filters

Wireshark display filter:

```text
udp && (
  udp.port == 1024 || udp.port == 1025 || udp.port == 1026 || udp.port == 1027 ||
  udp.port == 1028 || udp.port == 1029 || (udp.port >= 1035 && udp.port <= 1044)
)
```

Recommended Wireshark columns:
- `frame.time_relative`
- `ip.src`
- `ip.dst`
- `udp.srcport`
- `udp.dstport`
- `udp.length`

Saturn-side capture (`tcpdump`):

```bash
sudo tcpdump -i <iface> -nn -s 0 -w protocol2-regression.pcap \
'udp and (port 1024 or port 1025 or port 1026 or port 1027 or port 1028 or port 1029 or portrange 1035-1044)'
```

## Test Matrix (Required)

Run and record each state separately:
- `Idle / discovery only`
- `RX stable run`
- `TX active`

Tests:

1. Discovery / Connect
- Verify UDP `1024` discovery request/reply exchange is present.
- Verify discovery request/reply payload size is `60` bytes (`udp.length == 68` in Wireshark).
- Verify connect sequence progresses to general/high-priority/specific traffic.
- Verify discovery reply active/idle state transitions as expected (for example idle vs active indication byte).

Pass:
- Discovery and connection complete without retries/stalls.
- Packet sizes and ports match expected P2 behavior.

2. RX Stable Run
- Verify sustained outbound DDC IQ traffic on `1035+` (as configured).
- Verify periodic `1025` high-priority packets continue during RX.
- Verify payload lengths stay constant (no malformed packets).
- Compare cadence against baseline capture (no new bursts/gaps/jitter patterns).

Pass:
- Continuous RX streaming with stable packet sizes/cadence.
- No port changes or unexpected stream interruptions.

3. TX / Drive / Alex Control
- Verify `1027` incoming high-priority packets are present during run.
- Verify expected hardware behavior when changing drive/Alex controls.
- Verify no malformed payload lengths on control/high-priority paths.

Pass:
- TX control changes take effect correctly.
- Packet sizes/ports remain P2-compatible.

4. CAT Port Behavior
- Verify CAT port field is conveyed/used as expected in high-priority flow.
- Verify TCP CAT connect/disconnect works across restarts/port changes.
- Verify no stale socket behavior after disconnect/reconnect.

Pass:
- CAT connects reliably and recovers cleanly.

5. Panel Activity Under Load
- Exercise front panel while RX/TX traffic is active.
- Verify panel activity does not disrupt packet cadence (especially `1025` and `1035+`).
- Compare timing against baseline capture and runtime telemetry if available.

Pass:
- No observable stream timing regression or packet disruption during panel interaction.

6. Reconnect / Restart Recovery
- Stop/start Thetis.
- Restart the app under test.
- Reconnect and verify discovery + port rebinding recover cleanly.

Pass:
- Recovery works without lingering sockets/ports or manual cleanup.

## Before/After Comparison Checklist

Compare baseline (`before`) vs changed build (`after`) for the same test state:
- Ports in use (unchanged)
- UDP payload lengths per port (unchanged unless intentional)
- Packet cadence/timing (no new stalls/bursts)
- Stream continuity on `1025` and `1035+`
- Connect/disconnect/reconnect behavior
- CAT connect/disconnect behavior
- Panel activity impact under load

## Failure Criteria

Fail the regression if any of the following occur (unless intentionally changed and documented):
- Different port numbers used
- Different payload lengths on existing ports
- Missing expected packet stream (discovery/high-priority/DDC IQ/etc.)
- New cadence instability (bursts, gaps, stalls) compared to baseline
- CAT port/connect behavior regresses
- Reconnect/restart recovery regresses
- Panel activity causes observable packet timing disruption not present in baseline

## Run Record Template (Copy/Paste)

```text
Date/Time:
Tester:
App commit:
Thetis version:
FPGA version:
Active app/profile:
SATURN_FRONT_PANEL_MODE:
Capture side/interface:
Test state (Idle/RX/TX):
Baseline pcap:
After-change pcap:
Result: PASS / FAIL
Notes:
```
