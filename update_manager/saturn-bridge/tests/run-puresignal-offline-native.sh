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
  447c032a79b44d5bc5ea92576a9dc3cf11fd7aaaca493f30468a348e074384e0 "$NATIVE_BUILD/libwdsp.a" \
  3c4e9f9094a49bc7651686b28a15e8cf3733baac19af66e1cf6d56a9b6f6c20f "$NATIVE_BUILD/wdsp.h" \
  584c2250c5492e3dec8cbb752c0196d25bcd9c86360d8f8daeefe78e5ce66311 "$NATIVE_BUILD/calcc.c" \
  3274590454c2da3d00ac3925f1ffb0de3b97a191681293d136e74ec93d8d5aa2 "$NATIVE_BUILD/TXA.c" \
  c16f1d78b74772e724699aa73e0d456624f73ac898df785025d7d8e3ebf79763 "$NATIVE_BUILD/comm.h" \
  ca3c0ab07587884dfe849687fc5685c2b9539385b41358ffd1ba24879d2398df "$NATIVE_BUILD/iqc.h" \
  be84f36bebfa5bdc6da45ce25e1f5401429b8ecff8e2840464d49274a2caf47e "$NATIVE_BUILD/iqc.c" \
  707dc16f4f6eeca80f0578fccc156fffa7af7090da48013e8e188dd95dcb7c32 "$NATIVE_BUILD/TXA.h" \
  d9d7c3fce85dc022d7eed7918a22099689746a46f85b042554d13013e05f66c4 "$NATIVE_BUILD/channel.h" \
  ca7baa8db63ca9d644c663aca1ba6944341d2cc218c533c984e9bf1b39908dcb "$NATIVE_BUILD/linux_port.h" \
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
