#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lab_dir=$(cd "$script_dir/.." && pwd)
venv_dir=${SATURN_VENV:-$HOME/.venvs/saturn-fpga}
venv_python="$venv_dir/bin/python"
clean_python_env=(env -u PYTHONHOME -u PYTHONEXECUTABLE -u PYTHONNOUSERSITE)

if [[ ! -x "$venv_python" ]]; then
    mkdir -p "$(dirname "$venv_dir")"
    "${clean_python_env[@]}" /usr/bin/python3 -m venv --without-pip "$venv_dir"
fi

if "${clean_python_env[@]}" "$venv_python" -m pip --version >/dev/null 2>&1; then
    "${clean_python_env[@]}" "$venv_python" -m pip install --upgrade pip
else
    # OSS CAD Suite ships a pure-Python pip wheel, while this Ubuntu image may
    # omit python3-venv/ensurepip. Seed pip from that wheel without sudo.
    pip_wheel=${SATURN_PIP_WHEEL:-}
    if [[ -z "$pip_wheel" ]]; then
        for candidate in "$HOME"/opt/oss-cad-suite/lib/python3.*/ensurepip/_bundled/pip-*.whl; do
            if [[ -f "$candidate" ]]; then
                pip_wheel=$candidate
                break
            fi
        done
    fi
    if [[ -z "$pip_wheel" || ! -f "$pip_wheel" ]]; then
        echo 'ERROR: no pip bootstrap wheel found; install python3-venv or activate OSS CAD Suite.' >&2
        exit 1
    fi
    site_packages=$("${clean_python_env[@]}" "$venv_python" -c 'import site; print(site.getsitepackages()[0])')
    "${clean_python_env[@]}" /usr/bin/python3 -m zipfile -e "$pip_wheel" "$site_packages"
    "${clean_python_env[@]}" "$venv_python" -m pip install --upgrade pip
fi

"${clean_python_env[@]}" "$venv_python" -m pip install -r "$lab_dir/requirements.txt"
"${clean_python_env[@]}" "$venv_python" -m pip check

echo "SATURN_LAB_PYTHON_OK: $venv_dir"
