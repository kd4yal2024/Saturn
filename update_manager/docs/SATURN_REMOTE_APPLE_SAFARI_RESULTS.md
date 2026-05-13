# Saturn Remote Apple Safari Results

Created: 2026-05-13

Scope: evidence ledger for `/remote-next` Apple browser validation. The test
procedure lives in `SATURN_REMOTE_APPLE_SAFARI_VALIDATION.md`; this file records
the actual Mac Safari, iPhone Safari, and iPad Safari results.

## Current Status

Apple support is not certified yet. The required real-device rows are still
pending.

| Device class | Browser | Network | Status | Evidence |
| --- | --- | --- | --- | --- |
| Mac | Safari | LAN or Tailnet | Pending | None recorded |
| iPhone | Safari | LAN or Tailnet | Pending | None recorded |
| iPad | Safari | LAN or Tailnet | Pending | None recorded |

Recommended comparison rows, if available:

| Device class | Browser | Purpose | Status |
| --- | --- | --- | --- |
| Mac | Chrome or Edge | Separate Safari-specific failures from Mac hardware/network failures | Optional |
| iPhone/iPad | Home-screen PWA mode | Check standalone-mode audio and page-hide behavior | Optional |

## Evidence Rules

- Record one result block per physical device and browser combination.
- A result is `PASS`, `FAIL`, or `BLOCKED`; avoid partial pass labels.
- A `PASS` requires every pass criterion in the validation runbook.
- A `FAIL` must name the failing step and the user-visible behavior.
- A `BLOCKED` must name the blocker, such as missing device access, auth issue,
  network issue, unavailable microphone permission, or inability to capture a
  screenshot.
- Store screenshots under a stable local path, then list the path in the result
  block. Do not rely on browser tab state as evidence.
- If RF is enabled, state the RF-safe setup used. If RF is disabled, state the
  observed RF State pill value.
- Treat Safari failures as highest priority until a comparison browser proves
  the problem is hardware, network, or device-specific rather than Safari-only.

Suggested screenshot directory:

```text
/home/pi/Documents/saturn-remote-apple-results/YYYY-MM-DD/
```

Suggested file names:

```text
mac-safari-before-live.png
mac-safari-live.png
mac-safari-operator-state.png
iphone-safari-before-live.png
iphone-safari-live.png
iphone-safari-background-return.png
ipad-safari-before-live.png
ipad-safari-live.png
ipad-safari-operator-state.png
```

## Required Result Blocks

Copy these blocks as each physical device is tested.

### Mac Safari

```text
Apple Safari validation result:
- Device: Mac
- Hardware:
- OS / browser:
- Network path:
- URL:
- Date/time:
- RF-safe setup:
- Result: PASS / FAIL / BLOCKED
- Screenshots:
- Go Live audio:
- Spectrum/waterfall:
- State strip layout:
- Operator State drawer:
- Touch/keyboard PTT release:
- Page hide / screen lock fail-closed:
- Mic permission behavior:
- Settings persistence:
- Console/log notes:
- Follow-up fixes:
```

### iPhone Safari

```text
Apple Safari validation result:
- Device: iPhone
- Hardware:
- OS / browser:
- Network path:
- URL:
- Date/time:
- RF-safe setup:
- Result: PASS / FAIL / BLOCKED
- Screenshots:
- Go Live audio:
- Spectrum/waterfall:
- State strip layout:
- Operator State drawer:
- Touch/keyboard PTT release:
- Page hide / screen lock fail-closed:
- Mic permission behavior:
- Settings persistence:
- Stay Awake result:
- Console/log notes:
- Follow-up fixes:
```

### iPad Safari

```text
Apple Safari validation result:
- Device: iPad
- Hardware:
- OS / browser:
- Network path:
- URL:
- Date/time:
- RF-safe setup:
- Result: PASS / FAIL / BLOCKED
- Screenshots:
- Go Live audio:
- Spectrum/waterfall:
- State strip layout:
- Operator State drawer:
- Touch/keyboard PTT release:
- Page hide / screen lock fail-closed:
- Mic permission behavior:
- Settings persistence:
- Stay Awake result:
- Console/log notes:
- Follow-up fixes:
```

## Result Ledger

Append completed result blocks below this line.

### 2026-05-13

- No real Apple Safari device results recorded yet.
