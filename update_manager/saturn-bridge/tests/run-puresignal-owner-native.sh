#!/usr/bin/env bash
# Combined offline replay. Actual UDP / ordinary-file output owners + real WDSP.
# NEVER open hardware, discover G2, start services or transmit RF.
set -euo pipefail
OWNER_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OWNER_WDSP="${1:?usage: bash tests/run-puresignal-owner-native.sh VERIFIED_WDSP_BUILD_DIR}"
OWNER_WDSP="$(cd "$OWNER_WDSP" && pwd)"
OWNER_FILTER="${2:-combined_}"
printf '%s  %s\n' \
  447c032a79b44d5bc5ea92576a9dc3cf11fd7aaaca493f30468a348e074384e0 "$OWNER_WDSP/libwdsp.a" \
  3c4e9f9094a49bc7651686b28a15e8cf3733baac19af66e1cf6d56a9b6f6c20f "$OWNER_WDSP/wdsp.h" \
  584c2250c5492e3dec8cbb752c0196d25bcd9c86360d8f8daeefe78e5ce66311 "$OWNER_WDSP/calcc.c" \
  be84f36bebfa5bdc6da45ce25e1f5401429b8ecff8e2840464d49274a2caf47e "$OWNER_WDSP/iqc.c" \
  3274590454c2da3d00ac3925f1ffb0de3b97a191681293d136e74ec93d8d5aa2 "$OWNER_WDSP/TXA.c" \
  c16f1d78b74772e724699aa73e0d456624f73ac898df785025d7d8e3ebf79763 "$OWNER_WDSP/comm.h" \
  ca3c0ab07587884dfe849687fc5685c2b9539385b41358ffd1ba24879d2398df "$OWNER_WDSP/iqc.h" \
  707dc16f4f6eeca80f0578fccc156fffa7af7090da48013e8e188dd95dcb7c32 "$OWNER_WDSP/TXA.h" \
  d9d7c3fce85dc022d7eed7918a22099689746a46f85b042554d13013e05f66c4 "$OWNER_WDSP/channel.h" \
  ca7baa8db63ca9d644c663aca1ba6944341d2cc218c533c984e9bf1b39908dcb "$OWNER_WDSP/linux_port.h" \
  c23cd7fe7c5e0c8fa9060b5b8f4b77e57f66484e36b9b0e00ad674237184e66d "$OWNER_WDSP/rmatch.c" \
  f67a2c5f95911bb43dce830546b6daa307f89424e43b2e54a7bec123cb925abd "$OWNER_WDSP/rmatch.h" \
  10b4bde3e71bc6b36047f6bf318cfc8bcbdcfbba9d983679a42ff6118ee94213 "$OWNER_WDSP/varsamp.c" \
  a252eec7bd8ed4dddb81a028d9d0813d1f178fa03a94d4aa7b220797d5ec5f6e "$OWNER_WDSP/varsamp.h" \
  16bd1b04a93b9e05232c5ed8148ed2048358c65e08dbf1d019bb07d876ef36fb "$OWNER_WDSP/eq.h" \
  73e5a18014826bace9c57eedb5b6418aca05f1663f2bd74e696359cbb67ee4d4 "$OWNER_WDSP/cfcomp.h" \
  e92363d86a2ce971052f962de04f241e5b569512b0e784d9014cbfdd77ad149e "$OWNER_WDSP/patchpanel.h" \
  a8c339858e51ea1e12a7841b07eb7dd6a5de45f9d9c1f8acd72e3bb76e7a067d "$OWNER_WDSP/phrot.h" \
  865b9a046d4bafb477f0232b2a05e2444e3851e3ba839dbbe1cb2f43145bda8c "$OWNER_WDSP/compress.h" \
  b0ae285a06280f184b1b9c2d163336325a232200455dac8fd49716df7ddf14d9 "$OWNER_WDSP/osctrl.h" \
  a1c77d3731549647091e052a3e98f6f87db026db5f9af8d41fa364ed4c0aabcb "$OWNER_WDSP/amsq.h" \
  6dcfca66ede935dae28e70bb5c82b0df08d3b1e3a4ec03f81f64267b62ec772b "$OWNER_WDSP/dexp.h" \
  | sha256sum --check --strict
OWNER_OBSERVER="$(mktemp -d /tmp/saturn-ps-owner-native.XXXXXX)"
cc -std=gnu11 -O2 -Wall -Wextra -I"$OWNER_WDSP" -c "$OWNER_TEST_DIR/support/puresignal-native-observer.c" -o "$OWNER_OBSERVER/observer.o"
ar rcs "$OWNER_OBSERVER/libps_observer.a" "$OWNER_OBSERVER/observer.o"
cd "$OWNER_TEST_DIR/.."
unset SATURN_BRIDGE_STUB_NATIVE
SATURN_WDSP_DIR="$OWNER_WDSP" RUSTFLAGS="-L native=$OWNER_OBSERVER -l static=ps_observer" \
  timeout 120s cargo test --offline --features offline-puresignal-owner-native \
  "$OWNER_FILTER" -- --nocapture --test-threads=1 "${@:3}"
printf 'Observer artifact retained: %s\n' "$OWNER_OBSERVER"
