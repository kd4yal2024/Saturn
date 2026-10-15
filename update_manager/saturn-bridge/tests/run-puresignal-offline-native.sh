#!/usr/bin/env bash
# Offline-only replay runner: never contacts G2 or opens a radio device.
set -euo pipefail
REPLAY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NATIVE_BUILD="${1:?usage: bash tests/run-puresignal-offline-native.sh VERIFIED_WDSP_BUILD_DIR}"
for tool in cc ar rustc sha256sum timeout; do command -v "$tool" >/dev/null; done
# Fail closed on library/header/source mismatch. This archive identity is the
# reviewed x86_64 Linux offline build, not the installed G2 archive. For another
# toolchain, independently review/rebuild and record a separate identity.
printf '%s  %s\n' \
  8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248 "$NATIVE_BUILD/libwdsp.a" \
  3dde5a9bfafe7e87f76684eeaab7da22fd5d421c08a8a725b0e2ec3cbd5416af "$NATIVE_BUILD/wdsp.h" \
  1aa8470294640e7109824db7cc41786a02e21a5f0b9cf54bbbcfaf2df190ab66 "$NATIVE_BUILD/calcc.c" \
  3274590454c2da3d00ac3925f1ffb0de3b97a191681293d136e74ec93d8d5aa2 "$NATIVE_BUILD/TXA.c" \
  3cfe70965c9da3b7ff99098c00ddd8f42624d2081569e4fd1fd22c14f945d8dc "$NATIVE_BUILD/comm.h" \
  ca3c0ab07587884dfe849687fc5685c2b9539385b41358ffd1ba24879d2398df "$NATIVE_BUILD/iqc.h" \
  be84f36bebfa5bdc6da45ce25e1f5401429b8ecff8e2840464d49274a2caf47e "$NATIVE_BUILD/iqc.c" \
  707dc16f4f6eeca80f0578fccc156fffa7af7090da48013e8e188dd95dcb7c32 "$NATIVE_BUILD/TXA.h" \
  d9d7c3fce85dc022d7eed7918a22099689746a46f85b042554d13013e05f66c4 "$NATIVE_BUILD/channel.h" \
  0b41dcd45db2feca301d9c94c60d75839f83e9e110dd09cdc390c6421a965f73 "$NATIVE_BUILD/linux_port.h" \
  | sha256sum --check --strict
REPLAY_BUILD="$(mktemp -d /tmp/saturn-puresignal-combined.XXXXXX)"
rustc --edition=2021 --test -D warnings "$REPLAY_DIR/../src/lib.rs" \
  -o "$REPLAY_BUILD/controller-unit"
"$REPLAY_BUILD/controller-unit"
rustc --edition=2021 --test -D warnings \
  --cfg 'feature="offline-puresignal-policy-proposal"' "$REPLAY_DIR/../src/lib.rs" \
  -o "$REPLAY_BUILD/proposal-unit"
"$REPLAY_BUILD/proposal-unit"
rustc --edition=2021 --test -D warnings \
  --cfg 'feature="offline-puresignal-policy-proposal"' \
  --cfg 'feature="offline-puresignal-tx-lifecycle"' "$REPLAY_DIR/../src/lib.rs" \
  -o "$REPLAY_BUILD/lifecycle-unit"
"$REPLAY_BUILD/lifecycle-unit"
cc -std=gnu11 -O2 -Wall -Wextra -I"$NATIVE_BUILD" \
  -c "$REPLAY_DIR/support/puresignal-native-observer.c" -o "$REPLAY_BUILD/observer.o"
ar rcs "$REPLAY_BUILD/libps_observer.a" "$REPLAY_BUILD/observer.o"
rustc --edition=2021 -D warnings "$REPLAY_DIR/support/puresignal-controller-native-replay.rs" \
  --cfg 'feature="offline-puresignal-policy-proposal"' \
  --cfg 'feature="offline-puresignal-tx-lifecycle"' \
  -L "native=$REPLAY_BUILD" -L "native=$NATIVE_BUILD" \
  -l static=ps_observer -l static=wdsp -l fftw3 -l m -l pthread \
  -o "$REPLAY_BUILD/controller-replay"
cc -std=c11 -O2 -Wall -Wextra -I"$NATIVE_BUILD" \
  "$REPLAY_DIR/puresignal-native-replay.c" "$NATIVE_BUILD/libwdsp.a" \
  -lfftw3 -lm -lpthread -o "$REPLAY_BUILD/fitting-replay"
timeout 60s "$REPLAY_BUILD/controller-replay"
timeout 30s "$REPLAY_BUILD/fitting-replay"
sha256sum "$REPLAY_BUILD/controller-replay" "$REPLAY_BUILD/fitting-replay"
printf 'Offline replay executables retained at: %s\n' "$REPLAY_BUILD"
