# Saturn Remote Apple Safari Validation

Created: 2026-05-13

Scope: `/remote-next` on macOS Safari, iPhone Safari, and iPad Safari. This is
a manual validation runbook for real Apple browsers and devices. The automated
Chromium layout gate from phase 27 is useful for geometry regressions, but it
does not certify Safari audio, touch, wake/sleep, or WebGL behavior.

`/remote` remains the stable fallback. Do not promote `/remote-next` based on
this checklist until the device rows below have real evidence.

## Browser Contract To Verify

Safari has stricter runtime behavior than desktop Chromium in the areas that
matter most for Saturn Remote:

- RX audio must be started from the operator's `Go Live` tap/click.
- Microphone capture must start only from an intentional TX action and must fail
  closed if permission is denied or capture fails.
- iPhone and iPad may not support the Screen Wake Lock API; `Stay Awake N/A` is
  acceptable only if page hide, screen lock, and backgrounding unkey and lock TX.
- Touch PTT must unkey on pointer release, pointer cancel, lost capture, page
  hide, and browser focus loss.
- Spectrum and waterfall canvases must render nonblank, and WebGL fallback
  behavior must be visible in the event log if WebGL2 is unavailable.
- Settings changed from Safari must persist through reload without breaking
  startup normalization.

## Device Matrix

Minimum required before calling Apple support validated:

| Device | Browser | Network | Required Result | Evidence |
| --- | --- | --- | --- | --- |
| Mac | Safari | LAN or Tailnet | Pass all desktop rows | Screenshot plus notes |
| iPhone | Safari | LAN or Tailnet | Pass all phone rows | Screenshot plus notes |
| iPad | Safari | LAN or Tailnet | Pass all tablet rows | Screenshot plus notes |

Recommended comparison rows:

| Device | Browser | Purpose |
| --- | --- | --- |
| Mac | Chrome or Edge | Separate Safari-specific failures from Mac hardware/network failures |
| iPhone/iPad | Installed PWA/home-screen shortcut, if used | Check standalone mode page-hide and audio behavior |

## Safety Preflight

Run these before TX-related checks:

1. Put the radio into a controlled RF-safe state: dummy load, attenuator, or the
   bridge RF gate disabled.
2. Confirm the `/remote-next` RF State pill shows the expected gate state.
3. Confirm `Esc` unkeys from a desktop keyboard before using MOX or hold-to-PTT.
4. Keep a local way to kill TX available outside the Apple browser.

Do not use this checklist as permission to radiate. The goal is browser safety
and UI behavior, not RF output validation.

## Apple Device Steps

For each device row in the matrix:

1. Open `https://<saturn-host>:8443/remote-next`.
2. Authenticate and hard refresh once.
3. Record device, OS version, browser version, network path, and page URL.
4. Capture a screenshot before pressing `Go Live`.
5. Tap/click `Go Live`.
6. Confirm the Connection pill reaches `Connected`.
7. Confirm the Audio pill leaves `Stopped` after RX audio is requested.
8. Confirm RX audio is heard or record why it is muted.
9. Confirm the spectrum and waterfall are nonblank and updating.
10. Confirm the state strip remains readable with no horizontal page scroll.
11. Toggle `WF` on phone layout and confirm waterfall collapse/restore works.
12. Tune by tapping a VFO digit and by dragging or tapping the panadapter.
13. Adjust RX volume, NR, NB threshold, and display zoom; reload and confirm
    settings survive.
14. Open the direct frequency-entry sheet; close it with `Esc` on Mac or the
    close control on touch devices.
15. If testing TX controls, first press hold-to-PTT while TX is locked and
    confirm it only arms `TX READY`.
16. Hold PTT again, release, and confirm RX/TX returns to RX.
17. While PTT is held, move focus away or background the page; confirm TX unkeys
    and the TX zone locks.
18. If Safari asks for microphone permission, deny once and confirm TX does not
    remain keyed; allow once and confirm release still unkeys.
19. Toggle `Stay Awake`. If Safari reports `Stay Awake N/A`, continue with the
    screen-lock/background checks instead of treating that as a failure.
20. Lock the screen or background Safari while idle and while TX is armed; after
    returning, confirm the page is not keyed and requires reconfirm before TX.

## Pass Criteria

A device row passes only if:

- `Go Live` starts the browser audio path from a user gesture.
- The state strip and TX zone fit without overlap or unreadable text.
- Spectrum and waterfall render nonblank.
- Touch or keyboard PTT releases cleanly.
- Page hide, browser backgrounding, screen lock, and focus loss fail closed.
- Mic permission denial and mic errors fail closed.
- Settings persist after reload.
- No Safari console errors indicate repeated audio, WebGL, storage, or websocket
  failures.

## Result Template

Record one block per device in `SATURN_REMOTE_APPLE_SAFARI_RESULTS.md` after
testing, then summarize the pass/fail state in `~/Documents/hand-off.md`:

```text
Apple Safari validation result:
- Device:
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

## Known Acceptable Differences

- `Stay Awake N/A` is acceptable on Safari/iOS if screen lock and backgrounding
  still force TX safe and require reconfirm.
- iOS may stop audio when Safari is backgrounded. That is acceptable if the page
  reconnects cleanly and TX remains locked.
- Safari may require a fresh `Go Live` after reload or after the page has been
  backgrounded for a long time.

## Current Blocking Status

As of 2026-05-13, this runbook is ready, but Apple support is not certified
until real Mac, iPhone, and iPad Safari results are recorded.
