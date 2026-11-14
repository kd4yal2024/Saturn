Saturn Remote Ham / SWL band panel

The HF tab shows eleven amateur-band buttons, including 6m. The SWL tab shows
fifteen broadcast presets plus 11m (27 MHz) and FM. The broadcast presets are
LW/MW, 120m, 90m, 75m, 60m, 49m, 41m, 31m, 25m, 22m, 19m, 16m, 15m,
13m and 11m BC. Presets are receive-navigation ranges, not a TX authorization map.
There is no General tab; a saved General preference now opens SWL.

Selecting a group only changes the buttons and displayed band-edge guides.
It does not tune, change mode, change operating settings, or key the radio.
Clicking a SWL band recalls its saved frequency/settings; on first use its
preset defaults to AM. Saved SAM or other modes are restored on later visits.
Both banks save the active VFO's last tuned frequency and operating settings
before leaving a band. Returning recalls that exact frequency rather than the
button's first-use preset. Clicking the current band also keeps the latest tune
and settings, even if the autosave timer has not run yet.

The SWL-panel 11m (27 MHz) button starts at 27.385 MHz and remembers the last frequency
used from 26.495 MHz to just below 28 MHz, matching the existing 11m PA-profile
region. Its memory key is `11m`. The distinct 11m BC button remains the broadcast preset
at 25.9 MHz (25.67–26.1 MHz), with the separate key `SWL 11m`. Both buttons and
the keypad recall keep these operating memories distinct. These ranges are
navigation/settings boundaries; they do not change host transmit permissions.

SWL memory keys are prefixed (for example SWL 60m) so they cannot collide with
amateur 60m/15m settings. Overlapping frequencies such as 75m broadcast and
80m amateur also keep separate memories. The actual recalled memory group is
separate from the displayed bank, so browsing the other bank cannot silently
reclassify the current memory. The bank/context choice is saved locally;
SWL band memories use the existing saved/synced settings and profile path.
Operations memory recall restores the memory's own Ham/SWL context, including
when amateur and broadcast frequency ranges overlap.

Band buttons retain keyboard selection, active highlighting, responsive recent
band ordering and the existing More bands control. Band group is also
available in the Settings pop-out, which stays in sync with the tabs.

Validation: TypeScript, template/bundle seam, strict UI parity audit, full web
suite, and real Chrome through the shipped runtime. Chrome exercises both
banks, AM defaults, last-frequency recall, same-band reselect, independent
overlapping band memories, Operations recall, reload, keyboard,
phone layout and absence of any socket/microphone/TX session. Existing Settings
pop-out Chrome coverage also passes. These are offline UI checks, not RF tests.

Reproduce from update_manager/remote-web after npm ci:

    npm run typecheck
    npm run check:seam
    npm test
    npm run build
    npm run validate:bands
    npm run validate:settings
