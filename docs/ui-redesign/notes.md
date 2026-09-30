# Saturn Remote redesign: Phase 0 working notes

These notes capture findings and review questions before implementation. The owner's build sheet is the authority. The separate discovery document will carry the full control inventory.

## Current architecture

- `/remote-next` is a large server-rendered HTML template with inline CSS and JavaScript, plus a locally served Vite/TypeScript IIFE bundle. The application already has a build step; no framework migration is needed.
- State is shared between the inline script and `remote-web/src/state/`. Text TCI messages update it through `applyTciText`; binary messages feed IQ, audio, and negotiated spectrum rows.
- Traditional spectrum and waterfall renderers try WebGL2, with Canvas 2D fallbacks. The Hi-Res 3D terrain renderer uses WebGL2. LAN IQ and WAN spectrum rows converge before rendering. Canvas sizing currently references named display wrappers; preserve those relationships while relocating the display.
- Initial source inspection found locally served assets and no CDN URL in the Remote entry template. Phase 1 acceptance still needs a browser network check.
- The Rust asset route explicitly allows only the current bundle and Inter font. Because the build sheet forbids backend changes, Phase 1's IBM Plex fonts/icons/tokens CSS must be included in an already served HTML or bundle response; the deploy script also checks existing 3D markers.
- Existing TX readiness, arming, locking, PTT, MOX, and release handlers live in the inline UI script. This triggers the build sheet's explicit stop after Phase 0; moving TX controls will require a reviewed binding and safety plan before Phase 3.

## Feature parity risks to inventory

The section 9 checklist is a useful starting point but omits controls currently in subpanels and dialogs: setup profiles, detailed display effects, NR2/ANR/ANF tuning, RX/TX EQ bands, CFC, TX timeout and processing options, audio devices, WBFM deemphasis, SATP key display/copy/hide, mobile quick actions, frequency keypad, and terrain parameters. These need individual rows and Settings homes. Existing query parameters and storage keys also need explicit coverage.

The current source has 12 visible band choices including FM, while section 9 says 13. “Reference” is a VFO B state label, not a separate button. Both differences belong in the owner review, not an invented control.

## Questions for the group

1. What is the safest way to preserve and test the current TX binding/release path while moving its DOM controls in Phase 3?
2. Which current controls or query-parameter behaviors are easiest to miss in a parity audit? Please point to exact IDs or handlers.
3. Is there a small, shippable Phase 1 boundary for tokens and primitives that avoids disturbing the inline render and control paths?
4. Which browser or device cases have already caused layout, waterfall, audio, or touch regressions?

No implementation decision is made by these notes. Owner questions in section 19 remain for the relevant phases.
