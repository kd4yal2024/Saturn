#!/usr/bin/env bash
set -u

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lab_dir=$(cd "$script_dir/.." && pwd)
fpga_dir=$(cd "$lab_dir/.." && pwd)

missing=0

check_tool() {
    local tool=$1
    if command -v "$tool" >/dev/null 2>&1; then
        printf '%-12s %s\n' "$tool" "$(command -v "$tool")"
    else
        printf '%-12s MISSING\n' "$tool"
        missing=1
    fi
}

printf 'Saturn FPGA Lab doctor\n'
printf 'lab          %s\n' "$lab_dir"
printf 'platform     %s\n\n' "$(uname -srmo)"

for tool in git make python3 verilator iverilog yosys sby; do
    check_tool "$tool"
done

printf '\nPython modules\n'
venv_dir=${SATURN_VENV:-$HOME/.venvs/saturn-fpga}
python_bin="$venv_dir/bin/python"
if [[ -x "$python_bin" ]]; then
    env -u PYTHONHOME -u PYTHONEXECUTABLE -u PYTHONNOUSERSITE "$python_bin" - <<'PY'
for name in ("numpy", "scipy", "matplotlib"):
    module = __import__(name)
    print(f"{name:<12} {module.__version__}")
PY
else
    printf '%-12s MISSING (%s)\n' 'Python env' "$python_bin"
    missing=1
fi

printf '\nVivado\n'
if command -v vivado >/dev/null 2>&1; then
    vivado -version | sed -n '1,2p'
elif [[ -f /mnt/c/Xilinx/Vivado/2023.1/bin/vivado.bat ]]; then
    printf '%-12s %s\n' 'vivado' 'C:\Xilinx\Vivado\2023.1 (Windows; use make vivado-validate)'
else
    printf 'vivado       MISSING (checked WSL PATH and C:\Xilinx\Vivado\2023.1)\n'
    missing=1
fi

printf '\nRepository assets\n'
for asset in \
    "$fpga_dir/saturn_project/saturn_project.xpr" \
    "$fpga_dir/IP/DDCIP/DDCIP.xpr" \
    "$fpga_dir/IP/DUCIP/DUCIP.xpr" \
    "$fpga_dir/IP/CODEC_IQMOD_IP/CODEC_IQMOD_IP.xpr" \
    "$fpga_dir/saturnfallback.bin"; do
    if [[ -f "$asset" ]]; then
        printf 'present      %s\n' "$asset"
    else
        printf 'MISSING      %s\n' "$asset"
        missing=1
    fi
done

if command -v sha256sum >/dev/null 2>&1 && [[ -f "$fpga_dir/saturnfallback.bin" ]]; then
    printf '\nGolden fallback SHA256\n'
    sha256sum "$fpga_dir/saturnfallback.bin"
fi

if (( missing )); then
    printf '\nLAB_NOT_READY: install the missing dependencies shown above.\n' >&2
    exit 1
fi

printf '\nLAB_READY\n'
