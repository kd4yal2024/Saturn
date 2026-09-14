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

if [[ ${SATURN_SIM_FRESH:-0} == 1 ]]; then
    archive_suffix="$(date +%s)-$$"
    sim_roots=(
        "$repo_dir/FPGA/IP/DDCIP"
        "$repo_dir/FPGA/IP/DUCIP"
        "$repo_dir/FPGA/IP/CODEC_IQMOD_IP"
    )
    for sim_root in "${sim_roots[@]}"; do
        while IFS= read -r -d '' sim_cache; do
            archived_cache="${sim_cache}.stale-${archive_suffix}"
            printf 'Archiving stale XSim cache: %s\n' "$sim_cache"
            if ! mv -- "$sim_cache" "$archived_cache"; then
                cat >&2 <<EOF
Unable to archive the XSim cache. A Windows Vivado/XSim process may still hold it open.
Close the matching Vivado process (check tasklist.exe for vivado.exe, xvlog.exe,
xelab.exe, or xsim.exe), then rerun with SATURN_SIM_FRESH=1.
EOF
                exit 1
            fi
        done < <(find "$sim_root" -type d -path '*/sim_1/behav/xsim' -print0 2>/dev/null)
    done
fi

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
        if [[ $status -eq 0 && ${SATURN_VIVADO_WRITEBACK:-0} == 1 ]]; then
            rm -rf -- "$backup_dir"
            return "$status"
        fi
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

if [[ ${SATURN_VIVADO_WRITEBACK:-0} != 0 && ${SATURN_VIVADO_WRITEBACK:-0} != 1 ]]; then
    printf 'SATURN_VIVADO_WRITEBACK must be 0 or 1\n' >&2
    exit 2
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
    SATURN_REUSE_SYNTH SATURN_PLACE_DIRECTIVE SATURN_PHYSOPT_DIRECTIVE \
    SATURN_ROUTE_DIRECTIVE SATURN_POST_ROUTE_PHYSOPT_DIRECTIVE \
    SATURN_SIM_WAVES SATURN_SIM_FRESH SATURN_VALIDATE_UPDATE_COMPILE_ORDER \
    SATURN_DDC_SIM_SAMPLES SATURN_DDC_SIM_DISCARD \
    SATURN_DDC_SIM_RUNTIME SATURN_DUC_SIM_SAMPLES SATURN_DUC_SIM_DISCARD \
    SATURN_DUC_SIM_RUNTIME SATURN_IQMOD_KEY_HOLD_NS SATURN_IQMOD_SIM_SAMPLES \
    SATURN_IQMOD_SIM_RUNTIME \
    SATURN_KEY_HOLD_NS SATURN_REQUIRED_SAMPLES SATURN_DISCARD_SAMPLES \
    SATURN_GOLDEN_BIT SATURN_PRIMARY_BIT SATURN_PRIMARY_BIN \
    SATURN_BUILD_MANIFEST SATURN_TIMER1 SATURN_TIMER2 \
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
