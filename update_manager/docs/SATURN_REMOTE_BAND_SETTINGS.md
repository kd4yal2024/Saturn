# Automatic Saturn Remote band settings

Saturn Remote remembers the operating setup for each configured band. Adjust
controls normally; there is no separate Save requirement. Leaving a band takes
a final snapshot. Band buttons, keypad band recall and tuning into a remembered
band restore its setup. Tuning keeps the requested frequency; a band button
recalls the last frequency used on that band.
Ham HF and SWL have separate memories, including their overlapping frequency
ranges. Clicking the current band keeps its latest frequency and settings.
The band panel has only HF and SWL tabs. HF lists the eleven amateur bands.
SWL lists the broadcast bands plus 11m (27 MHz) and FM. The 27 MHz operating
memory keeps its existing `11m` key; 11m BC recalls the separate broadcast
memory near 25.9 MHz (`SWL 11m`). Removing General changes no saved memories.
An old General tab preference opens SWL after reload.

The RX filter shows its current shift. Center RX filter clears that offset,
keeping mode, width and other settings, and saves the correction for the current
band. A saved LSB filter shifted +12 kHz excludes the expected on-frequency
LSB signal; centering restores the normal signed passband. Selecting LSB again
also resets filters to the mode defaults.

On reconnect, the host's current readback does not replace an existing band
memory. The first band recall still uses the saved setup. Local tuning records
the new frequency immediately; explicit radio/display edits become the current
setup for that band. No extra Save action is needed.

Connected recall sends the destination frequency and operating preferences in
one ordered settings transaction. The requested memory is retained while the
host applies it. Autosave waits for pending settings to complete, and another
band recall does not replace a memory with intermediate host readback. This
keeps rapid band changes from saving the previous band's mode or power under
the new band's frequency. Applied/rejected results remain host reports.

A rejected recall retains its requested memory. A fresh band, Operations
memory or TX audio-source selection can submit a replacement after a matched
pre-queue `InvalidConfig` or `CleanupPending` rejection and a clean, fault-free
RX owner report. It still waits for a real applied receipt. Native/transport faults, queued failures,
unknown completions and unfinished cleanup retain the existing fault behavior.
An unsupported receive mode is still reported as rejected; this recovery does
not add mode support or retry/key a transmission. Source rejections clear the
selector waiting indicator and report the rejected receipt. Browser/Native
source changes require their own queued/applied receipts and do not key TX.

Memories include frequency, RX antenna and attenuation, mode, RX/TX filters and
RX filter shift, receiver volume, squelch, AGC, NR/NB/notch processing, RX/TX EQ,
TX power target and microphone gain, CFC/gate/DEXP/processor/CESSB, phase rotator,
PureSignal Off/Single/Auto preference and feedback attenuation, and
spectrum/waterfall/3D/zoom settings. Saved TX settings are
requests through the existing managed transaction path; the host still reports
whether each configuration was applied or rejected. Calibration coefficients
and RF measurement results are not settings and are not recalled.

Changes save locally immediately for radio controls, with debounced host saves
for the complete settings object. The last state is saved on page exit too.
The host settings and named profiles contain the same band-memory map, allowing
restoration after browser reload or appliance restart. A receive-validation
session or connected viewer does not rewrite memories. Existing memories remain
readable; newly added fields use normal defaults until saved for that band.

Headphone and MON preferences stay global rather than changing with the band.
They are separate from actual host output state. Host startup and disconnect
clear output; an authenticated operator connection restores headphones only
after the host advertises support. MON must be enabled again after reconnect.
These audio commands do not key RF.

Band recall never restores an active PTT/MOX, two-tone run, or measurement run.
Two-tone frequencies and levels remain saved preferences; shared-owner recall
does not send unsupported legacy generator setters. Auto ATT selection and manual
feedback attenuation are replayed only when the shared host advertises their
respective capabilities. With startup Auto ATT enabled, each fresh PS TX begins
at 31 dB; the applied result is not saved over the manual preset. The native
generator remains the separately authorized test action. A regular reconnect keeps the existing policy of using the
committed host TX setup; an explicit band recall submits that band's TX setup.

Connection URL, browser audio device selection, SATP pairing/source routing,
transport selection, UI theme/layout, and frequency/wake locks remain global.
The ADC uses the application's existing fixed route. Settings are per band,
not separate stacks for each modulation mode within a band.
