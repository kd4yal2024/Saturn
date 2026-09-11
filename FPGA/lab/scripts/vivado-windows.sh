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

git_dirty=false
if ! git -C "$repo_dir" diff --quiet --ignore-submodules -- || \
   ! git -C "$repo_dir" diff --cached --quiet --ignore-submodules -- || \
   [[ -n "$(git -C "$repo_dir" status --porcelain)" ]]; then
    git_dirty=true
fi
export SATURN_GIT_DIRTY=$git_dirty

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

# WSL environment variables are not reliably inherited by the Windows
# process launched through cmd.exe. Forward the lab's SATURN_* controls
# explicitly so simulation/build overrides work from WSL as documented.
windows_env_prefix=
for variable in \
    SATURN_ALLOW_UNSUPPORTED_VIVADO SATURN_VIVADO_JOBS SATURN_SKIP_RESET \
    SATURN_SIM_WAVES SATURN_DDC_SIM_SAMPLES SATURN_DDC_SIM_DISCARD \
    SATURN_DDC_SIM_RUNTIME SATURN_DUC_SIM_SAMPLES SATURN_DUC_SIM_DISCARD \
    SATURN_DUC_SIM_RUNTIME SATURN_IQMOD_KEY_HOLD_NS SATURN_IQMOD_SIM_RUNTIME \
    SATURN_KEY_HOLD_NS SATURN_REQUIRED_SAMPLES SATURN_DISCARD_SAMPLES \
    SATURN_GOLDEN_BIT SATURN_PRIMARY_BIT SATURN_TIMER1 SATURN_TIMER2 \
    SATURN_PROM_OUTPUT SATURN_GIT_DIRTY; do
    if [[ -n ${!variable+x} ]]; then
        value=${!variable}
        windows_env_prefix+="set ${variable}=${value}&&"
    fi
done

if [[ -n "$windows_env_prefix" ]]; then
    cmd.exe /d /c "${windows_env_prefix}call $vivado_bat_windows ${converted[*]}"
else
    cmd.exe /d /c "$vivado_bat_windows" "${converted[@]}"
fi
