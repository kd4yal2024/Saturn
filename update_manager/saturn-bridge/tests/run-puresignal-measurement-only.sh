#!/usr/bin/env bash
# Local loopback/ordinary-file replay only. No G2, RF, services or installer.
set -euo pipefail
MEASUREMENT_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEASUREMENT_WDSP="${1:?usage: bash tests/run-puresignal-measurement-only.sh PINNED_WDSP_DIR NEW_ABSOLUTE_EVIDENCE_DIR}"
MEASUREMENT_OUTPUT="${2:?new absolute evidence directory required}"
MEASUREMENT_WDSP="$(cd "$MEASUREMENT_WDSP" && pwd)"
if [[ "$MEASUREMENT_OUTPUT" != /* || -e "$MEASUREMENT_OUTPUT" ]]; then
  printf 'Evidence output must be a new absolute directory\n' >&2
  exit 1
fi
printf '%s  %s\n' \
  8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248 "$MEASUREMENT_WDSP/libwdsp.a" \
  3dde5a9bfafe7e87f76684eeaab7da22fd5d421c08a8a725b0e2ec3cbd5416af "$MEASUREMENT_WDSP/wdsp.h" \
  1aa8470294640e7109824db7cc41786a02e21a5f0b9cf54bbbcfaf2df190ab66 "$MEASUREMENT_WDSP/calcc.c" \
  | sha256sum --check --strict
mkdir -p "$MEASUREMENT_OUTPUT/native-captures" "$MEASUREMENT_OUTPUT/observer"
cc -std=gnu11 -O2 -Wall -Wextra -I"$MEASUREMENT_WDSP" -c \
  "$MEASUREMENT_SCRIPT_DIR/support/puresignal-native-observer.c" \
  -o "$MEASUREMENT_OUTPUT/observer/observer.o"
ar rcs "$MEASUREMENT_OUTPUT/observer/libps_observer.a" "$MEASUREMENT_OUTPUT/observer/observer.o"
cd "$MEASUREMENT_SCRIPT_DIR/.."
unset SATURN_BRIDGE_STUB_NATIVE
SATURN_WDSP_DIR="$MEASUREMENT_WDSP" \
  SATURN_MEASUREMENT_EVIDENCE_DIR="$MEASUREMENT_OUTPUT/native-captures" \
  RUSTFLAGS="-L native=$MEASUREMENT_OUTPUT/observer -l static=ps_observer" \
  timeout 90s cargo test --offline --features managed-puresignal \
  measurement_only_actual -- --ignored --nocapture --test-threads=1 \
  > "$MEASUREMENT_OUTPUT/measurement-only-real-native.txt" 2>&1
printf 'Passed offline MeasurementOnly replay; evidence: %s\n' "$MEASUREMENT_OUTPUT"
