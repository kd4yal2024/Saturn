# RXC1 1.31.002 TX qualification decision — 2026-10-09

The operator explicitly qualified the installed primary Saturn PCB2 FPGA image
`1.31.002`, build ID `0x53460004`, for full TX use. This is an operator
acceptance decision for that exact identity, not a blanket approval for other
1.31 images or fallback configurations.

The preceding guarded RF probes used ANT1 into an inline LP-700 and 50-ohm
dummy load at 7.200 MHz. The 250 ms retry carrier passed at FPGA-reported
peak 1.002 W and SWR 1.00. The 400 ms carrier step passed at FPGA-reported
peak 4.237 W and SWR 1.00; the operator's LP-700 read 6 W and 1.3:1 SWR.
The 400 ms direct-DUC 700/1900 Hz two-tone step passed at FPGA-reported peak
0.291 W and SWR 1.00; the LP-700 read 1.2 W and 1.0:1 SWR. Each probe
verified RF cleanup and restored the selected XDMA Bridge.

These were short direct-DUC probes. They did not independently measure the
normal browser/WDSP TX chain at RF, establish a full-power external
calibration, or measure RF spectral purity. The LP-700 and FPGA power readings
disagreed, and neither has been used here to derive a correction factor. The
operator's qualification accepts these remaining evidence limits. The host
allowlist change for `0x53460004` is a policy implementation of that decision,
not additional hardware test evidence.

The production RF gate remains closed on G2 until a new validated Bridge
release containing the allowlist change is deliberately activated. The
existing `91d9766` installed release must not be described as TX-unlocked.
