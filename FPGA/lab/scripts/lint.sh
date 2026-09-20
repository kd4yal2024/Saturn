#!/usr/bin/env bash
set -u

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fpga_dir=$(cd "$script_dir/../.." && pwd)

if ! command -v verilator >/dev/null 2>&1; then
    echo 'ERROR: verilator is not on PATH. Source the OSS CAD Suite environment.' >&2
    exit 1
fi

warning_mode=(--Wno-fatal)
if [[ "${SATURN_LINT_STRICT:-0}" == "1" ]]; then
    warning_mode=()
fi

sources=(
    "$fpga_dir/sources/verilogmodules/activitywatchdog.v"
    "$fpga_dir/sources/verilogmodules/FIFO_Monitor.v"
    "$fpga_dir/sources/verilogmodules/AXI_FIFO_overflow_latch_reader.v"
    "$fpga_dir/sources/verilogmodules/I2S_rcv.v"
    "$fpga_dir/sources/verilogmodules/DDCMux.v"
)

status=0
for source in "${sources[@]}"; do
    printf '\n==> Verilator lint: %s\n' "${source#$fpga_dir/}"
    verilator --lint-only --Wall "${warning_mode[@]}" "$source" || status=1
done

if (( status )); then
    echo 'SATURN_LAB_LINT_FAIL' >&2
    exit "$status"
fi

echo 'SATURN_LAB_LINT_OK'
