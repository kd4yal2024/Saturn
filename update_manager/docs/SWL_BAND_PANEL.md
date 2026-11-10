Saturn Remote Ham / SWL band panel

The Band group selector switches the existing Band panel between Ham HF
(the existing twelve buttons, including 6m/FM) and fifteen broadcast presets:
LW/MW, 120m, 90m, 75m, 60m, 49m, 41m, 31m, 25m, 22m, 19m, 16m, 15m,
13m and 11m. Presets are receive-navigation ranges, not a TX authorization map.

Selecting a group only changes the buttons and displayed band-edge guides.
It does not tune, change mode, change operating settings, or key the radio.
Clicking a SWL band recalls its saved frequency/settings; on first use its
preset defaults to AM. Saved SAM or other modes are restored on later visits.

SWL memory keys are prefixed (for example SWL 60m) so they cannot collide with
amateur 60m/15m settings. Overlapping frequencies such as 75m broadcast and
80m amateur also keep separate memories. The actual recalled memory group is
separate from the displayed bank, so browsing the other bank cannot silently
reclassify the current memory. The bank/context choice is saved locally;
SWL band memories use the existing saved/synced settings and profile path.

Band buttons retain keyboard selection, active highlighting, responsive recent
band ordering and the existing More bands control. The selector is also indexed
in the Settings pop-out. The new bundle URL prevents a cached older bundle
from being paired with the updated template.

Validation: TypeScript, template/bundle seam, strict UI parity audit, full web
suite, and real Chrome through the shipped runtime. Chrome exercises both
banks, AM defaults, independent overlapping band memories, reload, keyboard,
phone layout and absence of any socket/microphone/TX session. Existing Settings
pop-out Chrome coverage also passes. These are offline UI checks, not RF tests.

Reproduce from update_manager/remote-web after npm ci:

    npm run typecheck
    npm run check:seam
    npm test
    npm run build
    npm run validate:bands
    npm run validate:settings
