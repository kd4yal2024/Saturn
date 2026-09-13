#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fpga_dir=$(cd "$script_dir/../.." && pwd)
build_dir=$(mktemp -d)
trap 'rm -rf "$build_dir"' EXIT

iverilog -g2012 -Wall -o "$build_dir/fifo_monitor_tb" \
  "$fpga_dir/sources/verilogmodules/FIFO_Monitor.v" \
  "$fpga_dir/lab/rtl-tests/fifo-monitor/fifo_monitor_tb.sv"
vvp "$build_dir/fifo_monitor_tb"
iverilog -g2012 -Wall -o "$build_dir/adc_fifo_reader_tb" \
  "$fpga_dir/sources/verilogmodules/AXI_FIFO_overflow_latch_reader.v" \
  "$fpga_dir/lab/rtl-tests/fifo-monitor/adc_fifo_reader_tb.sv"
vvp "$build_dir/adc_fifo_reader_tb"
iverilog -g2012 -Wall -o "$build_dir/i2s_rcv_backpressure_tb" \
  "$fpga_dir/sources/verilogmodules/I2S_rcv.v" \
  "$fpga_dir/lab/rtl-tests/fifo-monitor/i2s_rcv_backpressure_tb.sv"
vvp "$build_dir/i2s_rcv_backpressure_tb"
echo SATURN_LAB_TELEMETRY_OK
