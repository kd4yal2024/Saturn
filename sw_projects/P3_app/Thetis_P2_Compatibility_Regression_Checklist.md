# Thetis P2 Compatibility Regression Checklist (P3_app)

Date: 2026-02-25

Purpose:
- Keep `P3_app` compatible with Thetis (HPSDR Protocol 2 over Ethernet) while `P3_app` is hardened/refactored.
- Provide a repeatable before/after regression method for any `P3_app` change.

Scope:
- `P3_app` running as a P2-compatible shim for Thetis.
- Ethernet/UDP Protocol 2 discovery, control, streaming, CAT-related behavior.
- On-wire compatibility first; internal refactors are allowed if behavior is unchanged.

## Rules (Do Not Break By Accident)

Treat these as regressions unless intentionally changed and documented:
- UDP port numbers used by Thetis/P2 traffic
- UDP payload lengths on each port
- Packet cadence/timing patterns (especially high-priority and DDC IQ streams)
- Startup order (`discovery -> general/control -> specific/high-priority -> streaming`)
- Key field semantics in high-priority/general packets (run bit, ports, CAT port, drive/Alex fields)

## Important Length Note (Wireshark)

Wireshark `udp.length` includes the 8-byte UDP header.

When comparing to `P3_app` packet constants:
- UDP payload length = `udp.length - 8`
- Do not compare `udp.length` directly to payload constants without subtracting 8

## Capture Setup (Required)

Capture-side consistency:
- Use the same capture side for before/after comparisons (preferably the same host/interface).
- Do not compare timing/cadence across captures taken on different hosts unless necessary.

Version stamping (record every run):
- `P3_app` git commit
- Thetis version/build
- FPGA firmware version/bitstream
- Active app (`P2_app` or `P3_app`)
- `P3_app` startup args/profile (for example `-s -p`)
- Panel mode override (if used): `SATURN_FRONT_PANEL_MODE=...`
- Capture side (`Thetis host` or `Saturn`)
- Interface used (`eth0`, etc.)
- Test state (`idle`, `RX`, `TX`)

## Packet/Port Expectations (P2-Compatible)

Common Thetis/P2 UDP ports to monitor:
- `1024` general/discovery
- `1025` high-priority from SDR
- `1026` mic from SDR
- `1027` high-priority to SDR
- `1028` speaker audio to SDR
- `1029` mic/DUC IQ to SDR
- `1035-1044` DDC IQ streams (as used)

Common `P3_app` payload lengths to verify:
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
sudo tcpdump -i <iface> -nn -s 0 -w p3app-regression.pcap \
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
- Restart `p3app`.
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
P3_app commit:
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

