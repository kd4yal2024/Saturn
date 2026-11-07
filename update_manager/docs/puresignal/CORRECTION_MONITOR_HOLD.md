# Correction / Phase Monitor: inspect after release

The monitor retains the last valid keyed WDSP curve for display after TX stops.
A fresh empty native-cleanup snapshot continues updating current PS status, but
cannot erase those held magnitude and phase plots. The docked panel and pop-out
label retained data as held and not live. Copy measurements and Export curve JSON
use the displayed snapshot, including its original frequency, attenuation,
calibration count, native flags and timestamps.

Retention requires a valid bounded-age native snapshot after the current TX
clock barrier. RX-only snapshots, delayed packets and empty readbacks cannot
replace it. A new valid keyed curve replaces the held result. Starting another
TX with no fitted curve shows that current empty readback; it does not label an
old curve as live. A new connection generation clears the retained history.

Held plots do not certify current correction application, recent fit age or RF
IMD improvement. Current PS status remains sourced from the latest readback.
There are no TX commands, native DSP changes or acquisition changes in this fix.

Validation: `npm test`, `npm run typecheck`, `npm run check:seam`, `npm run build`,
and `node scripts/validate-correction-hold.mjs`. The Chrome check uses the shipped
bundle and actual panel markup with synthetic native curves, and exercises real
pop-out canvases, cleanup, copy, export and reconnection without a radio socket.
