#!/usr/bin/env bash
# The staged appliance procedure (update_manager/docs/headless-validation/g2_headless_steps.sh) is the document the hardware check is run
# from, so what it accepts as proof is tested here without touching a host: the hashes it pins, the commit it refuses to stray from, and
# the verdict it reaches on the evidence the validator keeps.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/update_manager/docs/headless-validation/g2_headless_steps.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
mkdir -p "$HOME"
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

bash -n "$SCRIPT"
shellcheck "$SCRIPT" 2>/dev/null || [[ -z "$(command -v shellcheck)" ]] || fail "shellcheck reports problems in the procedure"

# --- it pins the exact helper, validator and guard that are in this tree
for pair in \
  ACT_SHA:update_manager/scripts/saturn-release-activate-root.sh \
  VAL_SHA:update_manager/scripts/saturn-remote-validate.py \
  GUARD_SHA:update_manager/scripts/saturn-remote-receive-guard.js
do
  want="$(sha256sum "$ROOT/${pair#*:}" | cut -d' ' -f1)"
  grep -Fxq "${pair%%:*}=$want" "$SCRIPT" || fail "${pair%%:*} in the procedure is not the hash of ${pair#*:}; regenerate the procedure"
done

# --- it works on one commit and refuses another
run() { bash "$SCRIPT" "$@" 2>&1 || true; }
[[ "$(run | head -1)" == *"headless-validation update"* ]] || fail "no usage text"
out="$(run build)"; [[ "$out" == *"give the full 40-character commit"* ]] || fail "build without PIN: $out"
out="$(PIN=abc123 run build)"; [[ "$out" == *"give the full 40-character commit"* ]] || fail "build with a short PIN: $out"
[[ ! -e "$HOME/.g2_headless_pin" ]] || fail "a refused build left a pin behind"
out="$(run check)"; [[ "$out" == *"no commit is pinned yet"* ]] || fail "check without a pin: $out"
A=1111111111111111111111111111111111111111
B=2222222222222222222222222222222222222222
printf '%s\n' "$A" >"$HOME/.g2_headless_pin"
out="$(PIN=$B run check)"; [[ "$out" == *"PIN=$B but the build stage pinned $A"* ]] || fail "a different PIN was accepted: $out"
out="$(PIN=$B run build)"; [[ "$out" == *"says $A"* ]] || fail "build re-pinned silently: $out"
[[ "$(cat "$HOME/.g2_headless_pin")" == "$A" ]] || fail "the pin changed"

# --- the verdict on the evidence
python3 - "$TMP" <<'PY'
import copy
import json
import os
import sys

REQUIRED = ["release.running", "owner.fresh_telemetry", "page.loaded", "guard.active", "session.started", "data.display_fresh",
            "data.audio_frames_arriving", "audio.decoded_progress", "audio.context_running", "audio.no_decode_errors",
            "owner.fresh_telemetry_with_session", "owner.session_registered", "owner.gate_equivalent", "radio.unchanged",
            "wire.only_allowed_commands", "page.no_fatal_errors", "browser.sandbox_intact", "browser.memory_limit_applied",
            "cleanup.browser_gone", "cleanup.client_released"]
good = {
    "state": "done", "driver": "native", "role": "viewer", "role_policy": "required", "legacy_fallback_used": False,
    "conditions": {k: {"ok": True, "detail": k} for k in REQUIRED + ["session.connected_as_viewer"]},
    "isolation": {"memory": {"max_bytes": "367001600", "peak_bytes": "150000000"}},
}
cases = {}
cases["good"] = good
for key in REQUIRED + ["session.connected_as_viewer"]:
    broken = copy.deepcopy(good)
    broken["conditions"][key]["ok"] = False
    cases["false-" + key] = broken
    missing = copy.deepcopy(good)
    del missing["conditions"][key]
    cases["missing-" + key] = missing
operator = copy.deepcopy(good)
operator["role"] = "operator"
operator["conditions"].pop("session.connected_as_viewer")
cases["operator-without-fallback"] = operator
claimed = copy.deepcopy(good)
claimed["role"] = "operator"                  # a viewer condition that holds while the role is not a viewer's: inconsistent evidence
cases["operator-claimed-as-viewer"] = claimed
fallback = copy.deepcopy(operator)
fallback.update(role_policy="legacy-fallback", legacy_fallback_used=True)
fallback["conditions"]["session.role_legacy_fallback"] = {"ok": True, "detail": "LEGACY BRIDGE FALLBACK USED: x"}
cases["fallback"] = fallback
fallback_unproven = copy.deepcopy(fallback)
fallback_unproven["conditions"]["session.role_legacy_fallback"]["ok"] = False
cases["fallback-unproven"] = fallback_unproven
fallback_missing = copy.deepcopy(fallback)
del fallback_missing["conditions"]["session.role_legacy_fallback"]
cases["fallback-missing"] = fallback_missing
no_peak = copy.deepcopy(good)
no_peak["isolation"] = {}
cases["no-peak-reported"] = no_peak          # informational: still proven
directory = sys.argv[1]
for name, value in cases.items():
    with open(os.path.join(directory, f"status-{name}.json"), "w", encoding="utf-8") as handle:
        json.dump(value, handle)
with open(os.path.join(directory, "cases.txt"), "w", encoding="utf-8") as handle:
    handle.write("\n".join(cases) + "\n")
PY
verdict() { bash -c 'f="$1"; shift; source "$SCRIPT"; check_evidence "$f"' _ "$TMP/status-$1.json" >"$TMP/verdict.out" 2>&1; }
export SCRIPT
while IFS= read -r name; do
  case "$name" in
    good|fallback|no-peak-reported) verdict "$name" || fail "the evidence '$name' was refused: $(cat "$TMP/verdict.out")" ;;
    *) if verdict "$name"; then fail "the evidence '$name' was accepted as proof"; fi ;;
  esac
done <"$TMP/cases.txt"
verdict fallback
grep -Fq 'LEGACY BRIDGE FALLBACK USED' "$TMP/verdict.out" || fail "a fallback session was not announced"
verdict good
grep -Fq 'memory: peak 143 MiB of a 350 MiB cap' "$TMP/verdict.out" || fail "the memory peak was not reported: $(cat "$TMP/verdict.out")"
if grep -Fq 'LEGACY' "$TMP/verdict.out"; then fail "an ordinary viewer session was announced as a fallback"; fi

# --- the check stage, driven end to end with the host's commands replaced: what it counts as proof, and as an incomplete cleanup
cp "$TMP/status-good.json" "$TMP/status-for-check.json"
check_stage() {
  # $1 is the scenario; the environment carries what the stubbed host does
  bash -c '
    source "$SCRIPT"
    H="$TMP/helpers"; mkdir -p "$H"; : >"$H/saturn-remote-validate.py"; chmod +x "$H/saturn-remote-validate.py"
    PERF="$TMP/perf.json"; EVIDENCE_ROOT="$TMP/evidence"; LEFTOVER_DIRS=("$TMP/leftover"); SETTLE_SECONDS=0
    ss() { echo "State Recv-Q Send-Q Local Peer"; }
    confirm() { :; }
    systemctl() { printf "%s" "${UNITS_LEFT:-}"; }
    pgrep() { printf "%s" "${PROCS_LEFT:-}"; return 0; }
    sudo() {
      if [ "$STATUS_FILE" != none ]; then
        mkdir -p "$EVIDENCE_ROOT/20261010T000000Z-check-xxxxxx"
        cp "$STATUS_FILE" "$EVIDENCE_ROOT/20261010T000000Z-check-xxxxxx/status.json"
      fi
      [ -z "${CONNECTIONS_AFTER:-}" ] || printf "{\"backend\":\"xdma\",\"updated_at_ms\":0,\"metrics\":{\"connections\":%s,\"client\":0}}" "$CONNECTIONS_AFTER" >"$PERF"
      [ -z "${LEFTOVER:-}" ] || mkdir -p "$TMP/leftover/$LEFTOVER"
      return "${HELPER_RC:-0}"
    }
    stage_check
  ' _ >"$TMP/stage.out" 2>&1
}
export TMP SCRIPT STATUS_FILE="$TMP/status-for-check.json"
fresh_host() {
  rm -rf "$TMP/evidence" "$TMP/leftover"; mkdir -p "$TMP/leftover"
  printf '{"backend":"xdma","updated_at_ms":0,"metrics":{"connections":0,"client":0}}' >"$TMP/perf.json"
  unset UNITS_LEFT PROCS_LEFT CONNECTIONS_AFTER LEFTOVER HELPER_RC
  STATUS_FILE="$TMP/status-good.json"
}
fresh_host
check_stage || fail "a complete check was refused: $(cat "$TMP/stage.out")"
grep -Fq 'OK: reception, sandbox, memory cap and cleanup are all verified' "$TMP/stage.out" || fail "no verdict printed"
refused() {  # $1 = what was wrong
  if check_stage; then fail "$1 was accepted as a complete check"; fi
  grep -Fq 'NOT PROVEN' "$TMP/stage.out" || fail "$1: no NOT PROVEN verdict: $(tail -5 "$TMP/stage.out")"
  fresh_host
}
export UNITS_LEFT="saturn-remote-validator-1.service loaded active running x
"; refused "a transient unit left behind"
export PROCS_LEFT="4242
"; refused "a browser process left behind"
export CONNECTIONS_AFTER=2; refused "a connection left at the Bridge"
export LEFTOVER=saturn-validate-abc; refused "a leftover profile directory"
export HELPER_RC=1; refused "a helper that exited non-zero"
STATUS_FILE=none; refused "no evidence at all"
STATUS_FILE="$TMP/status-false-browser.sandbox_intact.json"; refused "an unverified sandbox"
STATUS_FILE="$TMP/status-false-browser.memory_limit_applied.json"; refused "an unverified memory cap"
STATUS_FILE="$TMP/status-missing-audio.decoded_progress.json"; refused "audio that was not shown to progress"
STATUS_FILE="$TMP/status-false-data.display_fresh.json"; refused "display data that was not shown to be fresh"
fresh_host
STATUS_FILE="$TMP/status-fallback.json"
check_stage || fail "a named legacy fallback was refused: $(cat "$TMP/stage.out")"
grep -Fq 'LEGACY BRIDGE FALLBACK USED' "$TMP/stage.out" || fail "the fallback was not announced by the check stage"

printf 'ok: the staged procedure pins this tree, refuses another commit and accepts only complete evidence\n'
