# Saturn Remote Settings window

Open the header gear or press **comma** to open Settings. Desktop browsers use a
separate, resizable window. Launching it again focuses that same window. Phones,
and browsers that block pop-ups, use a full-screen/in-page dialog instead.

The left navigation provides Radio & bands, Receiver, Transmitter, PureSignal,
Display, Audio devices, Meters, Connection, Appearance & layout, and System & logs.
Each category has subcategory buttons and an overview. Search works across all
categories and recognizes the existing names, including MON, ATT, NR2 and SATP.

Sliders have exact numeric entry alongside the current value/readout. Detailed
RX/TX equalizer and CFC bands, NR/ANR/ANF parameters, processing controls and
two-tone settings are editable here. Controls that need their original screen
say **Open on main screen**. The main-screen layout/pin editor remains available
from the footer. The ordinary TX/PTT controls stay on the main TX panel.

Changes apply immediately through the existing radio controls. Closing Settings
keeps those changes. There is no new Apply/Cancel transaction or second set of
settings. Per-band badges describe the existing band-memory contract; device,
connection and browser preferences remain global. The header shows the current
band, frequency, mode, profile, connection and TX status. Changing band, mode or
profile refreshes the page and discards an unfinished edit from the old context.

The window follows the radio's dark/light theme. Its geometry and last selected
category are remembered in this browser. **Ctrl/Cmd+K** focuses search; **Escape**
closes Settings. Keep the main Saturn Remote page open: this window shares that
session and closes when the page leaves. Closing Settings does not disconnect the
radio. Host availability and settings-transaction results remain authoritative.

## Implementation boundaries

`settings/workbench.ts` presents the existing control elements without moving,
cloning event handlers, opening a transport, capturing audio, or allocating a
second radio/native owner. User events are forwarded in the original element's
realm; values, limits, options, readouts and disabled states are mirrored back.
Out-of-range edits, detached controls and currently disabled targets cannot relay
changes. Merely rendering or refreshing a page sends no commands.

The embedded fallback contains keyboard navigation events so radio shortcuts do
not fire behind the dialog. Key-up still propagates to allow PTT release. Window
close/navigation removes timers, listeners and the secondary view. Old detached
controls cannot deliver an edit into a newly selected band.

The catalog groups the existing Settings registry and adds presentation entries
for existing two-tone, PS report, and dynamically mounted EQ/CFC controls. It adds
no FPGA, driver, WDSP, TX-authority or persistence policy changes.

## Validation

- `npm run typecheck`, `npm run check:seam`, `npm test`, `npm run build`.
- `npm run validate:settings`: real Chrome with the actual Remote runtime. Covers
  pop-out/reuse/close, canonical event delivery and per-band saving, reverse
  synchronization, numeric validation, disabled/detached controls, an unfinished
  edit across band recall, all ten categories, EQ/CFC controls, theme, search,
  blocked pop-ups, phone layout, focus return and shortcut isolation. It confirms
  no WebSocket or microphone capture starts. No Bridge or RF is used.
- `npm run validate:ui-tiers`: retains the drawer/layout-editor regression audit.
  That fixture selects the retained drawer; the dedicated Settings audit exercises
  the production default pop-out and fallback through the actual header handler.

The Chrome audit is included in the Remote web CI job and produces JSON plus
screenshots when run with `-- --output=PATH`.
