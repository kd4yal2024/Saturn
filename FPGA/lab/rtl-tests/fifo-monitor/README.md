# FIFO monitor RTL gate

`FIFO_Monitor.v` is part of the Phase 0 Verilator lint gate. The V29
self-checking AXI-Lite regression is `fifo_monitor_tb.sv` and covers coherent
snapshot capture, extrema, saturating state-transition counters, build-ID
reporting, stalled read-response stability, and read-clear boundary events.

Register map additions (relative to the existing 0x9000 base):

* `0x20`: snapshot control/status (`bit 0` capture, `bit 1` clear; read
  `bit 31` valid and `bits 15:0` sequence)
* `0x24..0x30`: captured FIFO counts
* `0x34..0x40`: minimum occupancy
* `0x44..0x50`: maximum occupancy
* `0x54..0x60`: saturating almost-full/full/empty transition counters (these
  are diagnostics, not physical sample-loss counters)
* `0x64`: build ID

`i2s_rcv_backpressure_tb.sv` reproduces the codec receive defect in which a
new physical I2S frame overwrote an AXI-Stream word while `TVALID` was high
and `TREADY` was low. The regression requires the pending word and `TVALID`
to remain stable until acceptance. A future hardware loss counter is still
needed to observe a new physical frame that arrives during a prolonged stall.

`adc_fifo_reader_tb.sv` also covers the V30 ADC episode bank at the existing
`0x5000` peripheral base. A low-to-high overrange transition starts exactly
one episode; boot-lifetime episode counts, high-clock totals, longest runs,
and the latest/current run length and peak are captured coherently by the
legacy status read. Repeated reads during one sustained high interval must not
increase the episode count. Durations use the exported 122.88 MHz clock rate.
