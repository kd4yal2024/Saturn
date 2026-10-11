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
  INSTALLER_SHA:update_manager/scripts/saturn-release-install-activator-root.sh \
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

# --- the release manifest must name the pinned commit, as a clean tree, for the release and every component
python3 - "$TMP" "$A" "$B" <<'PY'
import json
import os
import sys

directory, pin, other = sys.argv[1:4]
good = {"source": {"commit": pin, "dirty": False},
        "components": [{"name": "saturn-bridge", "source_commit": pin}, {"name": "saturn-go", "source_commit": pin}]}
variants = {"good": good}
for name, mutate in {
    "other-commit": lambda m: m["source"].update(commit=other),
    "dirty": lambda m: m["source"].update(dirty=True),
    "dirty-missing": lambda m: m["source"].pop("dirty"),
    "component-other-commit": lambda m: m["components"][1].update(source_commit=other),
    "component-no-commit": lambda m: m["components"][0].pop("source_commit"),
    "no-components": lambda m: m.update(components=[]),
    "no-source": lambda m: m.pop("source"),
}.items():
    broken = json.loads(json.dumps(good))
    mutate(broken)
    variants[name] = broken
for name, value in variants.items():
    os.makedirs(os.path.join(directory, "manifest-" + name))
    with open(os.path.join(directory, "manifest-" + name, "release-manifest.json"), "w", encoding="utf-8") as handle:
        json.dump(value, handle)
os.makedirs(os.path.join(directory, "manifest-garbled"))
with open(os.path.join(directory, "manifest-garbled", "release-manifest.json"), "w", encoding="utf-8") as handle:
    handle.write("{not json")
os.makedirs(os.path.join(directory, "manifest-absent"))
PY
export SCRIPT
manifest_verdict() { bash -c 'd="$1"; shift; source "$SCRIPT"; C="$PIN_UNDER_TEST"; manifest_names_pin "$d"' _ "$TMP/manifest-$1" >"$TMP/manifest.out" 2>&1; }
export PIN_UNDER_TEST="$A"
manifest_verdict good || fail "a manifest that names the pinned commit was refused: $(cat "$TMP/manifest.out")"
grep -Fq "source commit $A, clean tree, 2 components" "$TMP/manifest.out" || fail "the manifest check did not say what it proved: $(cat "$TMP/manifest.out")"
for name in other-commit dirty dirty-missing component-other-commit component-no-commit no-components no-source garbled absent; do
  if manifest_verdict "$name"; then fail "the manifest '$name' was accepted as the pinned commit's"; fi
done
PIN_UNDER_TEST="$B" manifest_verdict good && fail "a manifest of commit $A was accepted for the pin $B"
grep -Fq "does not describe the pinned commit $B" "$TMP/manifest.out" || fail "a wrong-commit manifest was refused without saying why: $(cat "$TMP/manifest.out")"

# ... and the validate and install stages refuse a bundle, or an installed copy, that names another commit
mkdir -p "$TMP/tools" && printf 'import sys\nsys.exit(0)\n' >"$TMP/tools/saturn-release-manifest.py"
stage_manifest() {  # $1 = stage, $2 = the staged bundle's manifest, $3 = the manifest the (stubbed) installer leaves in the installed release
  rm -rf "$TMP/releases" && mkdir -p "$TMP/releases"
  bash -c '
    source "$SCRIPT"
    C="$PIN_UNDER_TEST"; B="$TMP/manifest-$STAGED"; H="$TMP/tools"; RELEASES="$TMP/releases"
    sudo() { case "$1 $2" in *install-root.sh\ --validate) ;; *install-root.sh\ *) mkdir -p "$RELEASES/$C/webroot"; cp "$TMP/manifest-$INSTALLED/release-manifest.json" "$RELEASES/$C/" ;; esac; return 0; }
    "stage_$STAGE"
  ' _ >"$TMP/stage-manifest.out" 2>&1
}
export TMP
STAGE=validate STAGED=good stage_manifest || fail "the validate stage refused a bundle of the pinned commit: $(cat "$TMP/stage-manifest.out")"
STAGE=validate STAGED=other-commit stage_manifest && fail "the validate stage accepted a bundle of another commit"
grep -Fq 'the bundle' "$TMP/stage-manifest.out" || fail "validate refused without saying why: $(cat "$TMP/stage-manifest.out")"
STAGE=install STAGED=good INSTALLED=good stage_manifest || fail "the install stage refused an installed release of the pinned commit: $(cat "$TMP/stage-manifest.out")"
STAGE=install STAGED=good INSTALLED=other-commit stage_manifest && fail "the install stage accepted an installed release of another commit"
grep -Fq "the installed release's manifest" "$TMP/stage-manifest.out" || fail "install refused without saying why: $(cat "$TMP/stage-manifest.out")"

# --- the helper stage runs the installer that is IN the installed release (it is not among the installed helpers), only if all four pinned files match
mkdir -p "$TMP/cfg" && printf 'ACTIVATION_ENABLED="0"\n' >"$TMP/cfg/activate-disabled" && printf 'ACTIVATION_ENABLED="1"\n' >"$TMP/cfg/activate-enabled"
helper_stage() {  # $1 = config file; $2 = a file name in the release to corrupt (or none); $3 = "absent" to leave the installer out
  rm -rf "$TMP/releases" "$TMP/sudo.args"; mkdir -p "$TMP/releases/$A/scripts"
  for f in saturn-release-install-activator-root.sh saturn-release-activate-root.sh saturn-remote-validate.py saturn-remote-receive-guard.js; do
    cp "$ROOT/update_manager/scripts/$f" "$TMP/releases/$A/scripts/$f"
  done
  [[ "$2" == none ]] || printf '# tampered\n' >>"$TMP/releases/$A/scripts/$2"
  [[ "${3:-}" != absent ]] || rm "$TMP/releases/$A/scripts/saturn-release-install-activator-root.sh"
  CFG="$1" PIN_UNDER_TEST="$A" bash -c '
    source "$SCRIPT"
    C="$PIN_UNDER_TEST"; RELEASES="$TMP/releases"; ACT_CFG="$CFG"; BROWSER=true; SYSTEMD_RUN=true
    confirm() { :; }
    # like the real sudo: a command that is not there is not found
    sudo() { [ -x "$1" ] || { echo "sudo: $1: command not found" >&2; return 1; }; printf "%s\n" "$*" >"$TMP/sudo.args"; return "${SUDO_RC:-0}"; }
    stage_helper
  ' _ >"$TMP/stage-helper.out" 2>&1
}
helper_stage "$TMP/cfg/activate-disabled" none || fail "the helper stage failed on a release whose files are the pinned ones: $(cat "$TMP/stage-helper.out")"
want="$TMP/releases/$A/scripts/saturn-release-install-activator-root.sh $A $(sha256sum "$ROOT/update_manager/scripts/saturn-release-activate-root.sh" | cut -d' ' -f1) $(sha256sum "$ROOT/update_manager/scripts/saturn-remote-validate.py" | cut -d' ' -f1) $(sha256sum "$ROOT/update_manager/scripts/saturn-remote-receive-guard.js" | cut -d' ' -f1)"
[[ "$(cat "$TMP/sudo.args")" == "$want" ]] || fail "the installer was not run from the release with the pinned hashes: $(cat "$TMP/sudo.args" 2>&1)"
for corrupt in saturn-release-install-activator-root.sh saturn-release-activate-root.sh saturn-remote-validate.py saturn-remote-receive-guard.js; do
  if helper_stage "$TMP/cfg/activate-disabled" "$corrupt"; then fail "the helper stage accepted a release whose $corrupt differs from the pinned hash"; fi
  [[ ! -e "$TMP/sudo.args" ]] || fail "root was invoked although $corrupt differs from the pinned hash"
  grep -Fq "$corrupt differs from the pinned hash" "$TMP/stage-helper.out" || fail "a corrupted $corrupt was refused without saying so: $(cat "$TMP/stage-helper.out")"
done
if SUDO_RC=1 helper_stage "$TMP/cfg/activate-disabled" none; then fail "the helper stage reported success although the installer failed"; fi
grep -Fq 'helper installation failed' "$TMP/stage-helper.out" || fail "a failed installer was not reported: $(cat "$TMP/stage-helper.out")"
if grep -Fq 'OK. Next' "$TMP/stage-helper.out"; then fail "a failed installer was followed by the success message"; fi
if helper_stage "$TMP/cfg/activate-enabled" none; then fail "the helper stage ran with activation enabled"; fi
[[ ! -e "$TMP/sudo.args" ]] || fail "root was invoked with activation enabled"
if helper_stage "$TMP/cfg/activate-disabled" none absent; then fail "the helper stage went ahead without the installer"; fi

# --- the build stage: a non-interactive shell gets the Rust toolchain on its PATH, and only the builder's own resolve-only report counts
path_of() { bash -c 'source "$SCRIPT"; printf "%s" "$PATH"' _ ; }
# shellcheck disable=SC2016  # the program text for the child shell is meant to be expanded there, not here
[[ "$(env -i HOME="$HOME" PATH=/usr/bin:/bin SCRIPT="$SCRIPT" "$(command -v bash)" -c 'source "$SCRIPT"; printf "%s" "$PATH"')" == "$HOME/.cargo/bin:/usr/bin:/bin" ]] || fail "the toolchain directory was not put on the PATH of a bare shell"
[[ "$(PATH="$HOME/.cargo/bin:$PATH" path_of)" == "$HOME/.cargo/bin:"* && "$(PATH="$HOME/.cargo/bin:$PATH" path_of)" != *"$HOME/.cargo/bin:$HOME/.cargo/bin"* ]] || fail "the toolchain directory was added twice"
# the stage reads the format the real builder prints; if that changes, this must fail rather than the hardware stage
grep -Fq "printf 'resolved_commit=%s\\n' \"\$RESOLVED_SOURCE_COMMIT\"" "$ROOT/update_manager/scripts/saturn-release-build.sh" || fail "the builder no longer prints resolved_commit=<sha> in --resolve-only mode; update stage_build"
mkdir -p "$TMP/fakebuild"
cat >"$TMP/fakebuild/saturn-release-build.sh" <<'FAKE'
#!/bin/bash
# Same report as the real builder's --resolve-only mode (build_resolved_remote_source), or a failure before it
case "$FAKE_MODE" in nocargo) echo "[saturn-release-build] ERROR: required command not found: cargo"; exit 1 ;; esac
if [[ " $* " == *" --resolve-only "* ]]; then
  printf 'source_remote=%s\nrequested_ref=%s\nresolved_ref=refs/heads/%s\n' "$2" "$4" "$4"
  case "$FAKE_MODE" in
    ok) printf 'resolved_commit=%s\n' "$FAKE_PIN" ;;
    moved) printf 'resolved_commit=%s\n' "$FAKE_OTHER"; printf '[saturn-release-build] Resolved x -> y @ %s\n' "$FAKE_PIN" ;;
    longer) printf 'resolved_commit=%s0\n' "$FAKE_PIN" ;;
    quoted) printf 'note: resolved_commit=%s\n' "$FAKE_PIN" ;;
  esac
  exit 0
fi
printf 'built %s\n' "$FAKE_PIN" >"$FAKE_BUILT"
FAKE
chmod +x "$TMP/fakebuild/saturn-release-build.sh"
build_stage() {  # $1 = what the fake builder does
  rm -f "$TMP/built"; rm -rf "$TMP/bundles"
  FAKE_MODE="$1" FAKE_PIN="$A" FAKE_OTHER="$B" FAKE_BUILT="$TMP/built" PIN_UNDER_TEST="$A" bash -c '
    source "$SCRIPT"
    C="$PIN_UNDER_TEST"; B="$TMP/bundles/$C"
    cd() { builtin cd "$TMP/fakebuild"; }
    stage_build
    wait
  ' _ >"$TMP/stage-build.out" 2>&1
}
build_stage ok || fail "the build stage refused a branch that resolves to the pinned commit: $(cat "$TMP/stage-build.out")"
grep -Fq 'build started' "$TMP/stage-build.out" || fail "no build was started: $(cat "$TMP/stage-build.out")"
[[ "$(cat "$TMP/built" 2>/dev/null)" == "built $A" ]] || fail "the builder was not run for the pinned commit"
for mode in moved longer quoted nocargo; do
  if build_stage "$mode"; then fail "the build stage went ahead on a '$mode' report: $(cat "$TMP/stage-build.out")"; fi
  [[ ! -e "$TMP/built" ]] || fail "a build was started on a '$mode' report"
  grep -Fq 'did not report resolved_commit' "$TMP/stage-build.out" || fail "'$mode' was refused without saying why: $(cat "$TMP/stage-build.out")"
done
grep -Fq 'required command not found: cargo' "$TMP/stage-build.out" || fail "the builder's own error was hidden from the operator: $(cat "$TMP/stage-build.out")"

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
