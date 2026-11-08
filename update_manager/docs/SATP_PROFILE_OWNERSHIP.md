# SATP settings and the persistent PureSignal profile

SATP transport and the default microphone source belong to
`saturn-bridge.service.d/satp.conf`, managed by `saturn-satp-config.sh`.
PureSignal configuration belongs to `/etc/default/saturn-puresignal`.

Older PS profile snapshots copied SATP settings, including the loopback bind
used during controlled diagnostics. Systemd gives `EnvironmentFile` values
precedence over `Environment=` assignments. Consequently Settings could save
a LAN bind or native source to `satp.conf`, restart successfully, and still
run with the old loopback bind and browser source from the PS environment.

The PS profile composer now excludes SATP transport and source fields. On an
explicit SATP settings update, the SATP helper migrates those fields out of
older PS profiles. It preserves all PS parameters, RF permissions, unrelated
lines, file ownership and permissions. The requested SATP configuration is
written to its existing drop-in. Reload or restart failure restores both
files, with backups retained if rollback itself fails. An inactive Bridge
stays inactive; active restarts continue through the radio ownership broker.

Regression checks cover profile precedence migration, unchanged PS/RF values,
mode preservation, rejection before changing a symlinked profile, and exact
restoration of both files after a failed restart.

On the G2 investigated on 8 November, the listener was `127.0.0.1:50100`
although the allowed sender was `192.168.0.108`; packet and session counts
were zero. A LAN sender needs the G2's LAN bind address (or an explicitly
configured wildcard bind), with the existing sender restriction retained.

A Bridge restart creates a new authenticated pairing session. Pair the native
sender again, verify packet arrival and native readiness, then select Native
SATP. Binding a socket and restarting successfully do not establish microphone
audio delivery or RF output. No firmware, driver, PA gain, attenuation or TX
authority change is part of this repair.

## Shared runtime pairing dispatch

The shared acquisition loop also needs to service the operator's `pair` and
`renew` commands. Previously they reached the receive-control mailbox but fell
through its unsupported-control branch; fixing the listener alone therefore
could not produce a key. The shared and Legacy runtimes now call the same
operator-checked pairing/renewal handler. Source selection stays on the shared
owner's ordered configuration path. Pairing neither keys nor arms TX, and no
Legacy TX worker is introduced.

The combined native runtime replay uses actual control sockets and authenticated
UDP on both backend fixtures. It checks a key response, lease extension, native
readiness and source selection, browser-source return and disconnect revocation.
A second viewer socket sends pair/renew requests followed by a ping; its pong
confirms processing without receiving a key or changing the operator's pairing
generation. The replay sends no TX request and keeps native output idle.

The accompanying toolbar repair makes Tune Peak's pressed state a real toggle.
Enabling it keeps the existing tune-to-peak action. A second click disables the
assist, clears its marker/readout, updates the Settings checkbox, and persists
the off choice without another VFO movement. Tests exercise the actual shipped
click handler with and without a usable peak.
