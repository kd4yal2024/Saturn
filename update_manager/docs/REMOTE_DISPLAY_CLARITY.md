# Saturn Remote display clarity

These browser-only changes leave bridge DSP, audio, IQ rates, FFT resolution,
temporal spectrum averaging and peak-hold behavior unchanged.

## Controls

- Setup → Display → Transceiver clarity selects Enhanced spectrum colors,
  15% fill and the Enhanced waterfall, with glow, sheen and smoothing off.
  Undo appearance restores the previous appearance within the current page.
- Enhanced Trace / Fill Colors can be toggled independently. Turning it off
  restores the selected solid trace color. Existing settings default to off.
- Enhanced (Thetis-style) is an additional waterfall palette, not a replacement
  for Classic, Ice, Ember or Forest. Settings and profiles retain the selection.
- Waterfall Noise Cleanup now smooths small temporal fluctuations without
  background subtraction. It retains at most 50% history, passes changes of
  6 dB or more immediately, and preserves all steady levels. It is not RF noise
  reduction. Small brief signals can still be softened at higher settings.

## Rendering and reference

The Enhanced palette uses the signal-level breakpoints of Thetis's
`Project Files/Source/Console/display.cs`, `ColorScheme.enhanced` (local reference
revision 02db47d5). It progresses from black through blue, cyan, green, yellow,
red and magenta to pale purple. Interpolation uses a continuous endpoint rather
than reproducing Thetis's small saturated-endpoint discontinuity. Absolute
Thetis dB thresholds are not copied: Saturn's existing floor/ceiling controls
and level scaling still apply.

Spectrum gradient colors use the same stops in WebGL2 and Canvas2D. WebGL's
previous 5% top/bottom trace padding is removed to agree with Canvas dB mapping.
The waterfall retains single-row texture uploads and its existing ring buffer.

The former cleanup subtracted up to 18 dB from background levels, then fed that
suppressed output into the next average. This caused accumulating level/color
drift. Removing subtraction keeps the history in the same level domain as the
input, while bounded averaging avoids the former 92% history weight.

## Validation and deployment

Run `npm run typecheck`, `npm test`, `npm run build`,
`npm run validate:waterfall` and `npm run validate:remote-next-layout` in
`update_manager/remote-web`. Renderer checks use headless Chrome/SwiftShader:
they verify colors, dB alignment, ring order and Canvas fallback, not live GPU
performance or listening quality. Regression tests cover repeated cleanup
feedback, steady colors, onsets/departures and settings persistence.

Build with `saturn_go_build_remote_web_assets` from
`update_manager/scripts/saturn-go-web-assets.sh` to generate the bundle checksum.
Back up and deploy the matching HTML, JavaScript and checksum together. No
bridge rebuild or restart is needed. Refreshing the browser reconnects its
session. Keep the old web assets for rollback; a later appliance update must
use a source revision containing these changes to retain them.
