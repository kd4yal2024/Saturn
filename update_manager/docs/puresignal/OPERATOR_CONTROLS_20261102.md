# Operator-controlled PureSignal two-tone

The persistent daily-driver profile can set `SATURN_PS_GENERATED_OPERATIONAL=1`
with `SATURN_PS_GENERATED_TRIALS=1`. This selects the versioned capability:

```
saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,<normal-drive-ceiling>;
```

Unlike the existing finite bench capability, this uses the committed radio TX
frequency and normal power ceiling (currently 100 commanded W), without a fixed
7.200 MHz or 5 W test policy. Older clients reject the unfamiliar version rather
than interpret its zero values as an established bench contract.

`start,0` explicitly requests manual Stop. Positive durations remain optional,
with checked nanosecond conversion. The native generator is prepared even when
there is no duration deadline. Operator Start is fresh authenticated authority;
it does not need the extra bench confirmation or five-minute arming window.
Disconnect, release, source/feedback loss, authorization and queue faults still
close output and require completed cleanup before re-arm. No automatic re-key
is introduced. Normal configuration barriers and hardware protections remain.

`SATURN_PS_TX_DURATION_NS=0` disables the separate elapsed-transmit cutoff while
retaining all source/feedback/cleanup deadlines. The browser's optional TX
timeout stays an operator preference; it does not override an operator-controlled
two-tone action's explicit duration or manual Stop. Daily deployment turns that
preference off at the operator's request, retaining the setting for future use.

Single/Auto select WDSP adaptation independently of PTT. Auto attenuation is a
separate unimplemented feature, not a gate to either mode. Two-tone continues to
use USB/LSB and the browser's tagged generator keepalive; voice/SATP operation
remains separate. This change does not claim new RF or IMD qualification.

Regression coverage includes manual operation beyond elapsed cutoffs through
both actual file/loopback owners, native WDSP output and cleanup, release and
disconnect, malformed durations, client authority, legacy bench compatibility,
and source-stall protection. Those offline fixtures do not transmit RF.
