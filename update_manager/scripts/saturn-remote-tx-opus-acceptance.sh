#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
remote_web="$repo_root/update_manager/remote-web"
bridge_manifest="$repo_root/update_manager/saturn-bridge/Cargo.toml"
template="$repo_root/update_manager/templates/saturn-remote-next.html"

echo "[tx-opus] browser Opus acceptance"
(
  cd "$remote_web"
  npm test -- --run \
    tests/tx-opus-acceptance.test.ts \
    tests/tx-opus-encoder.test.ts \
    tests/tx-uplink.test.ts
)

echo "[tx-opus] browser typecheck"
(
  cd "$remote_web"
  npm run typecheck
)

echo "[tx-opus] template script syntax"
node -e "const fs=require('fs'); const html=fs.readFileSync(process.argv[1], 'utf8'); const scripts=[...html.matchAll(/<script[^>]*>([\\s\\S]*?)<\\/script>/g)].map((m)=>m[1]); scripts.forEach((s)=>new Function(s)); console.log('checked scripts', scripts.length);" "$template"

echo "[tx-opus] bridge Opus negotiation gate"
cargo test -j1 --manifest-path "$bridge_manifest" \
  tx_codec_caps_accepts_opus_only_when_runtime_flag_enabled

echo "[tx-opus] bridge Opus decoder fixture"
cargo test -j1 --manifest-path "$bridge_manifest" \
  opus_decoder_decodes_real_wideband_packet_when_enabled

echo "[tx-opus] bridge split-session media-lane Opus fixture"
cargo test -j1 --manifest-path "$bridge_manifest" \
  media_lane_decodes_opus_mic_frame_when_runtime_flag_enabled

echo "[tx-opus] acceptance harness complete"
