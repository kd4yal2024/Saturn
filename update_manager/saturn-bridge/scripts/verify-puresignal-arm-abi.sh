#!/usr/bin/env bash
# Syntax/ABI comparison only: never executes ARM code or accesses the radio.
set -uo pipefail
ABI_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${SATURN_ARM_SYSROOT:?explicit ARM sysroot required}"
: "${SATURN_WDSP_DIR:?explicit pinned WDSP directory required}"
ABI_COMMAND=(/usr/bin/clang --target=aarch64-linux-gnu
  "--sysroot=$SATURN_ARM_SYSROOT" -isystem "$SATURN_ARM_SYSROOT/usr/include/aarch64-linux-gnu"
  -I "$SATURN_WDSP_DIR" -std=c11 -Werror -include wdsp.h -fsyntax-only
  "$ABI_SCRIPT_DIR/../../docs/puresignal/native-abi.h")
printf 'check=public-WDSP-header-versus-reviewed-native-declarations\n'
printf 'execution=syntax-only-no-ARM-execution\n'
/usr/bin/clang --version
sha256sum "$SATURN_WDSP_DIR/wdsp.h" "$ABI_SCRIPT_DIR/../../docs/puresignal/native-abi.h"
printf 'command:'
printf ' %q' "${ABI_COMMAND[@]}"
printf '\n'
"${ABI_COMMAND[@]}"
ABI_EXIT=$?
printf 'exit_status=%s\n' "$ABI_EXIT"
exit "$ABI_EXIT"
