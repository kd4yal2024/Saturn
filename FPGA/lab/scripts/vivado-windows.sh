#!/usr/bin/env bash
set -euo pipefail

vivado_home_windows=${VIVADO_HOME_WINDOWS:-C:\\Xilinx\\Vivado\\2023.1}
vivado_bat_windows="${vivado_home_windows}\\bin\\vivado.bat"
vivado_bat_wsl=$(wslpath -u "$vivado_bat_windows")
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(cd "$script_dir/../../.." && pwd)
project_rel=FPGA/saturn_project
project_file="$repo_dir/$project_rel/saturn_project.xpr"
guard_paths=(
    FPGA/saturn_project
    FPGA/IP/DDCIP
    FPGA/IP/DUCIP
    FPGA/IP/CODEC_IQMOD_IP
)
backup_dir=
guarded_files=()

if [[ ! -f "$vivado_bat_wsl" ]]; then
    printf 'Vivado launcher not found: %s\n' "$vivado_bat_windows" >&2
    exit 127
fi

# WSL normally registers this handler at distro startup.  Without it, PE
# binaries are visible under /mnt/c but fail with "Exec format error" before
# Vivado can print anything.  Give the operator a repair path instead of a
# misleading Vivado failure.
if [[ ! -e /proc/sys/fs/binfmt_misc/WSLInterop ]]; then
    cat >&2 <<'EOF'
Windows interop is unavailable in this WSL session (WSLInterop handler missing).
From an elevated Windows PowerShell, run:
  wsl --shutdown
Then reopen the Ubuntu distro explicitly with:
  wsl -d Ubuntu
If the handler is still absent, check /etc/wsl.conf for [interop] enabled=true.
EOF
    exit 126
fi

restore_project() {
    local status=$?
    if [[ -n "$backup_dir" ]]; then
        local relative
        for relative in "${guarded_files[@]}"; do
            cp -p "$backup_dir/$relative" "$repo_dir/$relative"
            rm -f "$backup_dir/$relative"
        done
        find "$backup_dir" -depth -type d -empty -delete
    fi
    return "$status"
}

if [[ -f "$project_file" ]]; then
    backup_dir=$(mktemp -d "${TMPDIR:-/tmp}/saturn-vivado-xpr.XXXXXXXX")
    mapfile -d '' -t guarded_files < <(git -C "$repo_dir" ls-files -z -- "${guard_paths[@]}")
    for relative in "${guarded_files[@]}"; do
        mkdir -p "$backup_dir/$(dirname "$relative")"
        cp -p "$repo_dir/$relative" "$backup_dir/$relative"
    done
    trap restore_project EXIT
fi

converted=()
for argument in "$@"; do
    if [[ "$argument" == /* && -e "$argument" ]]; then
        converted+=("$(wslpath -w "$argument")")
    else
        converted+=("$argument")
    fi
done

cmd.exe /d /c "$vivado_bat_windows" "${converted[@]}"
