# Independent Protocol 2 v4.4 specimens

These are synthetic wire specimens, not radio captures. Their literal bytes
come from the tables in **openHPSDR Ethernet Protocol v4.4**, TAPR source commit
[`09998920f4f6f1aff52261667b7efbdb46b34037`](https://github.com/TAPR/OpenHPSDR-Firmware/blob/09998920f4f6f1aff52261667b7efbdb46b34037/Protocol%202/Documentation/openHPSDR%20Ethernet%20Protocol%20v4.4.pdf).
Tests read these files directly; no production packet encoder generates the
test input. Hex whitespace is insignificant. Byte counts and independent
expected values are asserted by the tests.

| File | Size | Independent values |
| --- | --- | --- |
| `discovery-saturn-v44.hex` | 60 bytes | Idle (2), synthetic MAC `02:11:22:33:44:55`, Saturn (10), protocol 44, firmware 31, ten DDCs, phase-word format (1), default big-endian/three-byte data (0), beta/app byte 46. This is **not** a claim that current p2app advertises 44. |
| `discovery-generic-v44.hex` | 60 bytes | Busy (3), synthetic MAC `aa:bb:cc:dd:ee:ff`, board 5, protocol 44, firmware 20, eight DDCs, Hz format (0), format flags `0x1f`, generic beta 7. Generic beta is not identified as a Saturn app build. |
| `ddc24-edge-values.hex` | 1,444 bytes | Sequence `0x01020304`, opaque timestamp bytes `01 23 45 67 89 ab cd ef`, width 24, 238 complex samples. First four pairs are `(-8388608,8388607)`, `(1,-1)`, `(4194304,-4194304)`, `(0,0)`. Next 233 pairs are zero. Last pair is `(6291456,-6291456)`. Integer-to-float scaling is exactly `1/8388608`. |
| `high-priority-v44.hex` | 60 bytes | Sequence 42, PTT bits `0x13`, ADC flags 2. Exciter/forward/reverse `0x0102/0x0304/0x0506`; FIFO flags `0x0a`; depths `0x1112/0x2122/0x3132/0x4142`; ADC0/1 peaks 32767/32768; supply `0x5152`; user ADC1/0 `0x6162/0x7172`; I/O bits `0x12`. |

Discovery offsets: protocol 12, firmware 13, DDC count 20, frequency format 21,
data-format flags 22, beta 23. Status offsets: FIFO flags 30, FIFO depths 31–38,
ADC peaks 39–42. The normal DDC profile is big-endian signed 24-bit IQ; the
fixture's nonzero timestamp is opaque to this decoder and cannot establish an
acquisition time.

Negative tests alter declared widths, lengths or counts. Legacy discovery and
unknown capability bytes remain metadata; this slice does not negotiate a new
mode, revise the server advertisement, or change TX ownership and sequencing.
