# Automatic Saturn Remote band settings

Saturn Remote remembers the operating setup for each configured band. Adjust
controls normally; there is no separate Save requirement. Leaving a band takes
a final snapshot. Band buttons, keypad band recall and tuning into a remembered
band restore its setup. Tuning keeps the requested frequency; a band button
recalls the last frequency used on that band.

Memories include frequency, RX antenna and attenuation, mode, RX/TX filters and
RX filter shift, receiver volume, squelch, AGC, NR/NB/notch processing, RX/TX EQ,
TX power target and microphone gain, CFC/gate/DEXP/processor/CESSB, phase rotator,
PureSignal Off/Single/Auto preference and feedback attenuation, rear headphones,
MON and its level, and spectrum/waterfall/3D/zoom settings. Saved TX settings are
requests through the existing managed transaction path; the host still reports
whether each configuration was applied or rejected. Calibration coefficients
and RF measurement results are not settings and are not recalled.

Changes save locally immediately for radio controls, with debounced host saves
for the complete settings object. The last state is saved on page exit too.
The host settings and named profiles contain the same band-memory map, allowing
restoration after browser reload or appliance restart. A receive-validation
session or connected viewer does not rewrite memories. Existing memories remain
readable; newly added fields use normal defaults until saved for that band.

Headphone/MON preferences are separate from actual host output state. Host
startup and disconnect still clear output; an authenticated operator connection
restores the saved choice only after the host advertises support. An unsupported
backend does not erase that choice. These audio commands do not key RF.

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
