#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
remote_web="$repo_root/update_manager/remote-web"
bridge_manifest="$repo_root/update_manager/saturn-bridge/Cargo.toml"
template="$repo_root/update_manager/templates/saturn-remote-next.html"

echo "[rx-opus] browser tests"
(
  cd "$remote_web"
  npm test
)

echo "[rx-opus] browser typecheck, bundle build, and template seam"
(
  cd "$remote_web"
  npm run typecheck
  npm run build
  npm run check:seam
)

echo "[rx-opus] template script syntax"
node -e "const fs=require('fs'); const html=fs.readFileSync(process.argv[1], 'utf8'); const scripts=[...html.matchAll(/<script[^>]*>([\\s\\S]*?)<\\/script>/g)].map((m)=>m[1]); scripts.forEach((s)=>new Function(s)); console.log('checked scripts', scripts.length);" "$template"

echo "[rx-opus] bridge tests"
if [[ -z "${SATURN_BRIDGE_STUB_NATIVE:-}" && -z "${SATURN_WDSP_DIR:-}" ]] &&
   { [[ ! -f "$repo_root/../pihpsdr/wdsp/libwdsp.a" ]] ||
     [[ ! -f "$repo_root/../pihpsdr/rnnoise/librnnoise.a" ]] ||
     [[ ! -f "$repo_root/../pihpsdr/libspecbleach/libspecbleach.a" ]]; }; then
  echo "[rx-opus] native DSP libraries unavailable; using bridge native stub"
  export SATURN_BRIDGE_STUB_NATIVE=1
fi
cargo test -j1 --manifest-path "$bridge_manifest"

echo "[rx-opus] integration checks complete; live WAN A/B remains a separate gate"
