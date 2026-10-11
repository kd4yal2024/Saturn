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
out="$(run check bogus)"; [[ "$out" == *"must look like 500M"* ]] || fail "the real entry point did not hand the size to the check stage: $out"

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
            "wire.only_allowed_commands", "page.no_fatal_errors", "host.memory_headroom", "browser.sandbox_intact", "browser.memory_limit_applied",
            "cleanup.browser_gone", "cleanup.client_released"]
good = {
    "state": "done", "driver": "native", "role": "viewer", "role_policy": "required", "legacy_fallback_used": False, "memory_enforcement": "cgroup",
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
# the Raspberry Pi image has no memory controller: the validator's watchdog is the cap, and must say so and show the browser stayed within it
watchdog = copy.deepcopy(good)
watchdog["memory_enforcement"] = "watchdog"
watchdog["conditions"]["browser.memory_within_cap"] = {"ok": True, "detail": "the browser's peak was 160 MiB"}
watchdog["isolation"] = {"memory": {"max_bytes": "infinity", "cap_bytes": 367001600, "peak_bytes": "167772160", "peak_source": "validator sampling (proportional set size)",
                                    "enforcement": "watchdog"}}
cases["watchdog"] = watchdog
watchdog_exceeded = copy.deepcopy(watchdog)
watchdog_exceeded["conditions"]["browser.memory_within_cap"]["ok"] = False
cases["watchdog-exceeded"] = watchdog_exceeded
watchdog_unproven = copy.deepcopy(watchdog)
del watchdog_unproven["conditions"]["browser.memory_within_cap"]
cases["watchdog-within-cap-missing"] = watchdog_unproven
unstated = copy.deepcopy(good)
del unstated["memory_enforcement"]
cases["enforcement-unstated"] = unstated
odd = copy.deepcopy(good)
odd["memory_enforcement"] = "off"
cases["enforcement-off"] = odd
diagnosed = copy.deepcopy(good)
diagnosed["diagnostics"] = {"summary": {"samples": 40, "browser_pss_mb": {"max": 210.5, "mean": 180.0}, "browser_cpu_pct": {"max": 310.0, "mean": 250.0},
                                        "host_iowait_pct": {"max": 40.0, "mean": 27.0}, "swap_in_pages_total": 1200, "swap_out_pages_total": 90,
                                        "page_main_thread_busy": 0.93, "pss_by_process_type_mb_max": {"renderer": {"processes": 3, "pss_mb": 140.5}, "gpu-process": {"processes": 1, "pss_mb": 60.0}},
                                        "owner_counters": {"iq_tci_drop_queue_overflow_total": {"first": 1266, "last": 2900, "delta": 1634}}}}
diagnosed["last_snapshot"] = {"opusFrames": 457, "decodedFrames": 12, "audioFramesPlayed": 1, "lateDrops": 160, "audioSeqGaps": 19}
cases["diagnosed"] = diagnosed
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
    good|fallback|no-peak-reported|watchdog|diagnosed) verdict "$name" || fail "the evidence '$name' was refused: $(cat "$TMP/verdict.out")" ;;
    *) if verdict "$name"; then fail "the evidence '$name' was accepted as proof"; fi ;;
  esac
done <"$TMP/cases.txt"
verdict fallback
grep -Fq 'LEGACY BRIDGE FALLBACK USED' "$TMP/verdict.out" || fail "a fallback session was not announced"
verdict watchdog
grep -Fq "MEMORY CAP ENFORCED BY THE VALIDATOR'S WATCHDOG" "$TMP/verdict.out" || fail "a watchdog-enforced cap was not announced as such: $(cat "$TMP/verdict.out")"
grep -Fq 'memory: peak 160 MiB of a 350 MiB cap' "$TMP/verdict.out" || fail "the watchdog's measured peak was not reported: $(cat "$TMP/verdict.out")"
grep -Fq 'peak from validator sampling' "$TMP/verdict.out" || fail "the source of the peak was not named: $(cat "$TMP/verdict.out")"
verdict good
if grep -Fq 'WATCHDOG' "$TMP/verdict.out"; then fail "a kernel-enforced cap was announced as a watchdog"; fi
grep -Fq 'memory cap enforced by the kernel' "$TMP/verdict.out" || fail "a kernel-enforced cap was not announced as such"
verdict diagnosed
for want in 'browser memory (Pss) MiB max 210.5 mean 180.0' 'host I/O wait % max 40.0 mean 27.0' 'swap pages in/out 1200/90' 'page main thread busy 0.93' \
            'owner iq_tci_drop_queue_overflow_total: 1266 -> 2900 (+1634)' 'renderer x3 140.5, gpu-process x1 60.0' 'decodedFrames 12' 'lateDrops 160' 'audioSeqGaps 19'; do
  grep -Fq "$want" "$TMP/verdict.out" || fail "the diagnostics did not show '$want': $(cat "$TMP/verdict.out")"
done
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
    confirm() { echo "$1"; }
    systemctl() { printf "%s" "${UNITS_LEFT:-}"; }
    pgrep() { printf "%s" "${PROCS_LEFT:-}"; return 0; }
    sudo() {
      printf "%s\n" "$*" >"$TMP/check-sudo.args"
      if [ "$STATUS_FILE" != none ]; then
        mkdir -p "$EVIDENCE_ROOT/20261010T000000Z-check-xxxxxx"
        cp "$STATUS_FILE" "$EVIDENCE_ROOT/20261010T000000Z-check-xxxxxx/status.json"
      fi
      [ -z "${CONNECTIONS_AFTER:-}" ] || printf "{\"backend\":\"xdma\",\"updated_at_ms\":0,\"metrics\":{\"connections\":%s,\"client\":0}}" "$CONNECTIONS_AFTER" >"$PERF"
      [ -z "${LEFTOVER:-}" ] || mkdir -p "$TMP/leftover/$LEFTOVER"
      return "${HELPER_RC:-0}"
    }
    stage_check ${CHECK_ARG:-}
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
grep -Fq "OK: reception, sandbox, memory cap and cleanup are all verified" "$TMP/stage.out" || fail "no verdict printed: $(tail -12 "$TMP/stage.out")"
[[ "$(cat "$TMP/check-sudo.args")" == *"saturn-release-activate-root.sh --check-reception" && "$(cat "$TMP/check-sudo.args")" != *"--memory-max"* ]] || fail "a plain check passed a memory cap: $(cat "$TMP/check-sudo.args")"
CHECK_ARG=500M check_stage || fail "a measuring check was refused: $(cat "$TMP/stage.out")"
[[ "$(cat "$TMP/check-sudo.args")" == *"saturn-release-activate-root.sh --check-reception --memory-max 500M" ]] || fail "the measuring cap was not passed to the helper: $(cat "$TMP/check-sudo.args")"
grep -Fq 'browser memory cap 500M for this check' "$TMP/stage.out" || fail "the confirmation did not say which cap was in force"
for bad in bogus 500 0M -1M 500MB; do
  if CHECK_ARG=$bad check_stage; then fail "a measuring cap of '$bad' was accepted"; fi
  grep -Fq 'must look like 500M' "$TMP/stage.out" || fail "'$bad' was refused without saying why: $(cat "$TMP/stage.out")"
done
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

# --- the stages that change the host (reconcile, failure, activate), run against a stand-in for the root helper.
# The stand-in applies the same root-owned policy gates as the real helper and writes the same record fields; the first part of this section
# runs the REAL helper (in its own non-root test mode, on temporary paths) to show that the gate is where the stand-in says it is.
PREV_COMMIT="$(sed -n 's/^PREV=\([0-9a-f]\{40\}\)$/\1/p' "$SCRIPT")"
[[ "$PREV_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail "cannot read PREV from the procedure"
REAL_HELPER="$ROOT/update_manager/scripts/saturn-release-activate-root.sh"
grep -Fq 'activation is disabled by root-owned policy' "$REAL_HELPER" || fail "the real helper no longer has the activation policy gate the stand-in copies"
real_gate() {  # $1 = 0 or 1: what the real helper says to --reconcile-rollback with that policy
  local d="$TMP/real-gate-$1"
  rm -rf "$d"; mkdir -p "$d/deployments" "$d/saturn/releases" "$d/systemd"
  printf 'ACTIVATION_ENABLED="%s"\nTRANSACTION_GROUP="%s"\nSATURN_ROOT="%s/saturn"\nRELEASES_ROOT="%s/saturn/releases"\nCURRENT_LINK="%s/saturn/current"\nSYSTEMD_ROOT="%s/systemd"\nLOCK_FILE="%s/lock"\nMANIFEST_TOOL="%s/update_manager/scripts/saturn-release-manifest.py"\nSTATE_TOOL="%s/update_manager/scripts/saturn-state-compatibility.py"\nCOMPONENTS_FILE="%s/update_manager/release/components-v1.json"\nTRANSACTION_FILE="%s/deployments/current.json"\nSTATE_BACKUP_ROOT="%s/deployments/state-backups"\nSTATE_ROOT="%s/state"\nBACKEND_STATUS_HELPER="/bin/true"\nVALIDATION_ENABLED="0"\n' \
    "$1" "$(id -gn)" "$d" "$d" "$d" "$d" "$d" "$ROOT" "$ROOT" "$ROOT" "$d" "$d" "$d" >"$d/config"
  SATURN_RELEASE_ACTIVATE_TEST_MODE=1 SATURN_RELEASE_ACTIVATE_CONFIG="$d/config" bash "$REAL_HELPER" --reconcile-rollback 2>&1 || true
}
[[ "$(real_gate 0)" == *"reconciliation is disabled by root-owned policy (ACTIVATION_ENABLED)"* ]] || fail "the real helper did not refuse to reconcile with the policy off: $(real_gate 0)"
[[ "$(real_gate 1)" != *"disabled by root-owned policy"* && "$(real_gate 1)" == *"nothing to reconcile"* ]] || fail "the real helper did not get past its policy gate with the policy on: $(real_gate 1)"

mkdir -p "$TMP/standin"
cat >"$TMP/standin/saturn-release-activate-root.sh" <<'STANDIN'
#!/usr/bin/env bash
# Stands in for the root helper: the same policy gates (with the real messages), the same exit statuses (70 = previous release not restored,
# 71 = restored but reception unproven) and the same record fields. FAKE_OUTCOME says what the "activation" or "reconcile" turns out to be.
printf '%s\n' "$*" >>"$FAKE_LOG"
{ grep -E '^(ACTIVATION_ENABLED|VALIDATION_EXPECT_RXC1)=' "$FAKE_CFG" || true; } | tr '\n' ' ' >"$FAKE_SEEN"
if [ -e "$FAKE_ARM" ]; then { printf 'present:'; cat "$FAKE_ARM"; } >"$FAKE_SEEN_ARM"; else echo absent >"$FAKE_SEEN_ARM"; fi
[ -z "${FAKE_HANG:-}" ] || sleep "$FAKE_HANG"
[ -z "${FAKE_TAMPER:-}" ] || echo 'STRAY="x"' >>"$FAKE_CFG"     # something else changes the configuration while the helper runs
enabled() { grep -Fxq 'ACTIVATION_ENABLED="1"' "$FAKE_CFG"; }
write_record() {  # $1 status, $2 phase, $3 failure phase or -, $4 restored, $5 verified, $6 role, $7 legacy fallback used
  python3 - "$FAKE_TXN" "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$FAKE_TARGET" <<'PY'
import json
import os
import sys
path, status, phase, fphase, restored, verified, role, legacy, target = sys.argv[1:10]
d = json.load(open(path))
d.update(status=status, phase=phase, target_commit=target)
if fphase != "-":
    d["activation_failure"] = {"phase": fphase, "command": "x", "exit_status": 1}
    d["rollback"] = {"status": "complete", "release_restored": restored == "true", "receive_verified": verified == "true"}
d["validation"] = {"target": {"final_state": os.environ.get("FAKE_FINAL_STATE", "done"), "role": role, "legacy_fallback_used": legacy == "true", "failed": None}}
json.dump(d, open(path, "w"))
PY
}
case "$1" in
  --reconcile-rollback)
    enabled || { echo "[saturn-release-activate] ERROR: reconciliation is disabled by root-owned policy (ACTIVATION_ENABLED)" >&2; exit 1; }
    case "$FAKE_OUTCOME" in
      reconcile-fail) exit 1 ;;
      reconcile-ok|reconcile-altered)
        python3 - "$FAKE_TXN" "$FAKE_OUTCOME" <<'PY'
import json
import sys
path, outcome = sys.argv[1:3]
d = json.load(open(path))
original = {"status": d["status"], "activation_failure": d["activation_failure"], "rollback": d["rollback"]}
d["status"] = "rolled_back"
d["phase"] = "reconciled"
if outcome == "reconcile-altered":
    d["rollback"] = dict(d["rollback"], status="complete")     # the original outcome rewritten: not allowed
d["reconciliation"] = {"original_transaction": original, "release_restored": True, "receive_verified": True}
json.dump(d, open(path, "w"))
PY
        exit 0 ;;
    esac
    exit 99 ;;
  *)
    enabled || { echo "[saturn-release-activate] ERROR: activation is disabled by root-owned policy; enable only for an approved appliance rollback test" >&2; exit 1; }
    case "$FAKE_OUTCOME" in
      success)
        write_record committed commit - - - "${FAKE_ROLE:-viewer}" "${FAKE_LEGACY:-false}"
        [ -n "${FAKE_NO_REPOINT:-}" ] || ln -sfn "$FAKE_RELEASES/$FAKE_TARGET" "$FAKE_CURRENT"
        [ -z "${FAKE_CLIENTS_AFTER:-}" ] || echo "$FAKE_CLIENTS_AFTER" >"$FAKE_CLIENTS_FILE"
        python3 - "$FAKE_PERF" "${FAKE_RXC1:-valid}" <<'PY'
import json
import sys
d = json.load(open(sys.argv[1]))
d["metrics"]["rx_counter_v31"] = {"status": sys.argv[2]}
json.dump(d, open(sys.argv[1], "w"))
PY
        exit 0 ;;
      success-uncommitted) write_record rolled_back rollback-complete receive-validation true true viewer false; ln -sfn "$FAKE_RELEASES/$FAKE_TARGET" "$FAKE_CURRENT"; exit 0 ;;
      rolled-back) write_record "${FAKE_RECORD_STATUS:-rolled_back}" rollback-complete "${FAKE_FAIL_PHASE:-receive-validation}" true "${FAKE_RB_VERIFIED:-true}" viewer false; exit 1 ;;
      rolled-back-wrong-current) write_record rolled_back rollback-complete receive-validation true true viewer false; ln -sfn "$FAKE_RELEASES/$FAKE_TARGET" "$FAKE_CURRENT"; exit 1 ;;
      rollback-failed) write_record rollback_failed rollback-failed receive-validation false false viewer false; exit 70 ;;
      unproven) write_record rollback_failed rollback-failed receive-validation true false viewer false; exit 71 ;;
      early-fail) exit 1 ;;
    esac
    exit 99 ;;
esac
STANDIN
chmod +x "$TMP/standin/saturn-release-activate-root.sh"

hs_setup() {  # the host as the G2 is now: policy off, the record rollback_failed, the previous release active
  local hs="$TMP/hs"
  rm -rf "$hs" "$TMP/hs.out" "$TMP/hs.stage"; mkdir -p "$hs/releases/$PREV_COMMIT" "$hs/releases/$A" "$hs/arm" "$hs/H"
  ln -s "$hs/releases/$PREV_COMMIT" "$hs/current"
  cp "$TMP/standin/saturn-release-activate-root.sh" "$hs/H/"; printf '# validator\n' >"$hs/H/saturn-remote-validate.py"
  printf '# Managed by install_saturn_go_nginx.sh.\nACTIVATION_ENABLED="0"\nSATURN_ROOT="/opt/saturn"\nVALIDATION_MEMORY_MAX="500M"\nTRANSACTION_GROUP="pi"\n' >"$hs/activate.conf"
  cp "$hs/activate.conf" "$hs/activate.conf.orig"
  printf '{"backend":"xdma","status":"ready","updated_at_ms":0,"metrics":{"date_code_hex":"53460004","connections":0,"client":0,"rx_counter_v31":{"status":"unarmed"}}}' >"$hs/perf.json"
  printf '{"status":"rollback_failed","phase":"rollback-failed","target_commit":"%s","activation_failure":{"phase":"readiness","command":"w","exit_status":1},"rollback":{"status":"failed","release_restored":false}}' "$B" >"$hs/current.json"
}
hs_reconciled() { printf '{"status":"rolled_back","phase":"reconciled","target_commit":"%s","activation_failure":{"phase":"readiness","command":"w","exit_status":1},"rollback":{"status":"failed"}}' "$B" >"$TMP/hs/current.json"; }
hs_env() {  # the environment the stand-in helper and the stage see
  export FAKE_CFG="$TMP/hs/activate.conf" FAKE_TXN="$TMP/hs/current.json" FAKE_PERF="$TMP/hs/perf.json" FAKE_ARM="$TMP/hs/arm/armed" FAKE_LOG="$TMP/hs.helper.log"
  export FAKE_CLIENTS_FILE="$TMP/hs/clients" FAKE_SEEN="$TMP/hs.seen" FAKE_SEEN_ARM="$TMP/hs.seen.arm" FAKE_TARGET="$A" FAKE_RELEASES="$TMP/hs/releases" FAKE_CURRENT="$TMP/hs/current"
  rm -f "$FAKE_LOG" "$FAKE_SEEN" "$FAKE_SEEN_ARM"
}
cat >"$TMP/stage-run.sh" <<'STAGERUN'
  source "$SCRIPT"
  HS="$TMP/hs"; C="$A"; ACT_CFG="$HS/activate.conf"; TXN="$HS/current.json"; PERF="$HS/perf.json"; ARM_DIR="$HS/arm"; H="$HS/H"
  RELEASES="$HS/releases"; CURRENT_LINK="$HS/current"
  ACT_SHA=$(sha256sum "$H/saturn-release-activate-root.sh" | cut -d" " -f1); VAL_SHA=$(sha256sum "$H/saturn-remote-validate.py" | cut -d" " -f1)
  confirm() { :; }; sleep() { :; }
  clients() { if [ -e "$HS/clients" ]; then cat "$HS/clients"; else echo "${CLIENTS:-0}"; fi; }
  pgrep() { [ -z "${PROCS_LEFT:-}" ] || printf "%s\n" "$PROCS_LEFT"; return 0; }
  curl() { printf "{\"status\":\"ready\",\"build_commit\":\"%s\"}" "${READY_COMMIT:-$C}"; }
  stage_status() { echo "--- release and binaries"; echo "(stubbed)"; echo "--- telemetry"; }
  # like sudo, but the Nth call to sed -i can be made to fail, to see what a restoration that does not happen looks like
  SEDS=0
  sudo() {
    if [ "$1" = sed ] && [ "${2:-}" = -i ]; then
      SEDS=$((SEDS + 1))
      if [ "${FAIL_SED:-0}" = "$SEDS" ]; then return 1; fi
      if [ "${FAIL_SED_AFTER:-0}" = "$SEDS" ]; then "$@"; return 1; fi      # does the work, then reports failure
    fi
    "$@"
  }
  # a stage that only switches the policy on and ends, to see what the exit does about a restoration it still owes
  stage_policy_only() { with_activation_enabled; return 0; }
stage_"$STAGE"
STAGERUN
export SCRIPT TMP A B PREV_COMMIT
run_stage() {  # $1 = reconcile | failure | activate; the rest are VAR=value settings for the stand-in and the stage. Output: $TMP/hs.out, status: $TMP/hs.stage
  local stage=$1; shift
  hs_env
  set +e
  env STAGE="$stage" "$@" bash "$TMP/stage-run.sh" >"$TMP/hs.out" 2>&1 </dev/null
  echo $? >"$TMP/hs.stage"
  set -e
}
stage_rc() { cat "$TMP/hs.stage"; }
out_has() { grep -Fq -- "$1" "$TMP/hs.out"; }
cfg_unchanged() { cmp -s "$TMP/hs/activate.conf" "$TMP/hs/activate.conf.orig"; }
helper_was_run() { [ -s "$TMP/hs.helper.log" ]; }
expect() {  # $1 = description; the remaining arguments are a command that must succeed
  local what=$1; shift
  "$@" || fail "$what
$(sed 's/^/    | /' "$TMP/hs.out" 2>/dev/null | tail -25)"
}

# reconcile --------------------------------------------------------------------------------------------------------------------------
hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-ok
expect "reconcile with the policy off did not succeed" test "$(stage_rc)" = 0
expect "the helper did not see the policy on" test "$(cat "$TMP/hs.seen")" = 'ACTIVATION_ENABLED="1" '
expect "the configuration is not byte-identical after reconcile" cfg_unchanged
expect "reconcile did not say the policy was restored" out_has 'activation policy restored to: ACTIVATION_ENABLED="0" (the configuration is byte-identical to before)'
expect "reconcile did not confirm the original outcome is kept" out_has 'recorded beside them: yes'
expect "reconcile did not say what comes next" out_has 'OK. Next: bash ~/g2_headless_steps.sh activate'
expect "reconcile did not run the helper in reconcile mode" grep -Fxq -- '--reconcile-rollback' "$TMP/hs.helper.log"
record_copy_kept() { compgen -G "$TMP/hs/current.json.before-reconcile.*" >/dev/null; }
expect "reconcile left no copy of the record it was about to change" record_copy_kept

hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-fail
expect "a failed reconcile reported success" test "$(stage_rc)" != 0
expect "the configuration is not restored after a failed reconcile" cfg_unchanged
if out_has 'OK. Next'; then fail "a failed reconcile was followed by the success message"; fi

hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-altered
expect "a reconcile that rewrote the original outcome reported success" test "$(stage_rc)" != 0
expect "the altered original was not called out" out_has 'recorded beside them: NO'
expect "the configuration is not restored after an altered record" cfg_unchanged

hs_setup; sed -i 's/^ACTIVATION_ENABLED=.*/ACTIVATION_ENABLED="1"/' "$TMP/hs/activate.conf"; cp "$TMP/hs/activate.conf" "$TMP/hs/activate.conf.orig"
run_stage reconcile FAKE_OUTCOME=reconcile-ok
expect "reconcile with the policy already on failed" test "$(stage_rc)" = 0
expect "a policy that was already on was changed" cfg_unchanged
expect "the policy was not left on" grep -Fxq 'ACTIVATION_ENABLED="1"' "$TMP/hs/activate.conf"

hs_setup; printf '{"status":"rolled_back","target_commit":"x"}' >"$TMP/hs/current.json"; run_stage reconcile FAKE_OUTCOME=reconcile-ok
expect "reconcile ran on a record that is not rollback_failed" test "$(stage_rc)" != 0
expect "reconcile did not say there is nothing to reconcile" out_has 'there is nothing to reconcile'
if helper_was_run; then fail "the helper was run although there is nothing to reconcile"; fi
expect "the configuration changed although reconcile refused" cfg_unchanged

hs_setup; ln -sfn "$TMP/hs/releases/$A" "$TMP/hs/current"; run_stage reconcile FAKE_OUTCOME=reconcile-ok
expect "reconcile ran although the previous release is not the active one" test "$(stage_rc)" != 0
if helper_was_run; then fail "the helper was run although the previous release is not active"; fi
expect "the configuration changed although reconcile refused" cfg_unchanged

hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-ok CLIENTS=2
expect "reconcile ran with a browser connected" test "$(stage_rc)" != 0
if helper_was_run; then fail "the helper was run with a browser connected"; fi

# a restoration that does not happen is reported, and fails the stage even though the helper succeeded
hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-ok FAIL_SED=2
expect "a failed restoration did not fail reconcile" test "$(stage_rc)" != 0
expect "a failed restoration was not reported loudly" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
expect "the report did not say how to put it right" out_has "sudo sed -i 's/^ACTIVATION_ENABLED=.*/ACTIVATION_ENABLED=\"0\"/'"
if out_has 'OK. Next'; then fail "reconcile said OK although the policy was not restored"; fi

# something else changed the configuration while the helper ran: the stage cannot claim it is as it was
hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-ok FAKE_TAMPER=1
expect "a configuration changed under the stage was reported as restored" test "$(stage_rc)" != 0
expect "the changed configuration was not reported" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
# a command that reports failure after doing its work is still reported
hs_setup; run_stage reconcile FAKE_OUTCOME=reconcile-ok FAIL_SED_AFTER=2
expect "a restoration command that failed was not reported" test "$(stage_rc)" != 0
expect "the failed restoration command was not reported loudly" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
# an exit that still owes the restoration makes the stage fail when it cannot do it, and restores when it can
hs_setup; run_stage policy_only
expect "a stage that owed only the restoration did not succeed" test "$(stage_rc)" = 0
expect "the exit did not restore the configuration" cfg_unchanged
hs_setup; run_stage policy_only FAIL_SED=2
expect "an exit that could not restore reported success" test "$(stage_rc)" != 0
expect "the exit did not report the failed restoration" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
# a policy that is already on is neither written nor restored (so a failing sed is never reached)
hs_setup; sed -i 's/^ACTIVATION_ENABLED=.*/ACTIVATION_ENABLED="1"/' "$TMP/hs/activate.conf"; cp "$TMP/hs/activate.conf" "$TMP/hs/activate.conf.orig"
run_stage reconcile FAKE_OUTCOME=reconcile-ok FAIL_SED=1
expect "a policy that was already on was rewritten" test "$(stage_rc)" = 0

# the restoration also happens when the stage is killed while the helper runs
for sig in TERM INT HUP; do
  hs_setup; hs_env
  # (a background job of a script starts with SIGINT ignored, and a script run under nohup with SIGHUP ignored; no trap can undo that, so the signal dispositions are reset before the stage starts)
  python3 -c 'import os, signal, sys; [signal.signal(s, signal.SIG_DFL) for s in (signal.SIGINT, signal.SIGQUIT, signal.SIGHUP, signal.SIGTERM)]; os.execvp(sys.argv[1], sys.argv[1:])' \
    env STAGE=reconcile FAKE_OUTCOME=reconcile-ok FAKE_HANG=2 bash "$TMP/stage-run.sh" >"$TMP/hs.out" 2>&1 </dev/null &
  stage_pid=$!
  for _ in $(seq 1 50); do [ -e "$TMP/hs.seen" ] && break; sleep 0.1; done
  [ -e "$TMP/hs.seen" ] || { kill -KILL "$stage_pid" 2>/dev/null || true; fail "the stage did not reach the helper (signal test $sig)"; }
  kill "-$sig" "$stage_pid"
  wait "$stage_pid" && status=0 || status=$?
  [[ "$status" != 0 ]] || fail "a stage killed with $sig reported success"
  cmp -s "$TMP/hs/activate.conf" "$TMP/hs/activate.conf.orig" || fail "the configuration was not restored after $sig: $(cat "$TMP/hs/activate.conf")"
done

# failure -----------------------------------------------------------------------------------------------------------------------------
hs_setup; hs_reconciled; run_stage failure FAKE_OUTCOME=rolled-back
expect "an exercise that failed validation and rolled back as intended was not accepted" test "$(stage_rc)" = 0
expect "the helper did not see the exercise settings" test "$(cat "$TMP/hs.seen")" = 'ACTIVATION_ENABLED="1" VALIDATION_EXPECT_RXC1="unsupported" '
expect "the exercise left the configuration changed" cfg_unchanged
expect "the exercise did not say it was as intended" out_has 'OK: the activation failed its receive validation as intended'
expect "the exercise wrote an arm" test ! -e "$TMP/hs/arm/armed"

expected_failure() {  # $1 = what the stand-in's activation turns out to be, $2 = text the report must contain, $3 = optional VAR=value for the stand-in
  hs_setup; hs_reconciled
  run_stage failure FAKE_OUTCOME="$1" ${3:+"$3"}
  expect "the exercise ($1 ${3:-}) was reported as a success" test "$(stage_rc)" != 0
  expect "the exercise ($1 ${3:-}) was not explained: wanted '$2'" out_has "$2"
  if out_has 'OK: the activation failed'; then fail "the exercise ($1 ${3:-}) printed the success line"; fi
  expect "the configuration was left changed after the exercise ($1 ${3:-})" cfg_unchanged
}
expected_failure success "the activation SUCCEEDED"
expected_failure success-uncommitted "the activation SUCCEEDED"
expected_failure rollback-failed "exit 70: the previous release was NOT restored"
expected_failure unproven "exit 71: the previous release was restored but reception on it is not proven"
expected_failure rolled-back "the activation did not fail in the receive validation" FAKE_FAIL_PHASE=readiness
expected_failure rolled-back "the rollback record does not say that the previous release was restored and is receiving" FAKE_RB_VERIFIED=false
expected_failure rolled-back-wrong-current "the active release is"
expected_failure rolled-back "the record says 'rollback_failed', not rolled_back" FAKE_RECORD_STATUS=rollback_failed
expected_failure early-fail "this run left no record of its own"

hs_setup; hs_reconciled; run_stage failure FAKE_OUTCOME=rolled-back FAIL_SED=3
expect "an exercise whose restoration failed was reported as a success" test "$(stage_rc)" != 0
expect "an unrestored exercise was not reported" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
expect "an unrestored exercise was not listed among the problems" out_has 'the activation policy was not confirmed restored'

hs_setup; hs_reconciled; touch "$TMP/hs/arm/armed"; run_stage failure FAKE_OUTCOME=rolled-back
expect "the exercise ran with an arm present" test "$(stage_rc)" != 0
if helper_was_run; then fail "the helper was run although an arm exists"; fi
hs_setup; hs_reconciled; run_stage failure FAKE_OUTCOME=rolled-back CLIENTS=2
expect "the exercise ran with a browser connected" test "$(stage_rc)" != 0
if helper_was_run; then fail "the helper was run with a browser connected"; fi
hs_setup; run_stage failure FAKE_OUTCOME=rolled-back
expect "the exercise ran on an unreconciled record" test "$(stage_rc)" != 0
expect "the exercise did not say to reconcile first" out_has 'run the reconcile stage first'
if helper_was_run; then fail "the helper was run on an unreconciled record"; fi

# activate ----------------------------------------------------------------------------------------------------------------------------
hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=success
expect "a complete activation was not accepted" test "$(stage_rc)" = 0
expect "the helper did not see the policy on" test "$(cat "$TMP/hs.seen")" = 'ACTIVATION_ENABLED="1" '
expect "the arm was not there, with exactly the trial text, when the helper started" test "$(cat "$TMP/hs.seen.arm")" = 'present:RXC1-TRIAL-0x53460004'
expect "the configuration is not byte-identical after activation" cfg_unchanged
expect "the activation did not report the release as active and verified" out_has 'RELEASE ACTIVE AND RECEPTION VERIFIED'
expect "the activation did not report RXC1" out_has "OK: the fresh RXC1 status is 'valid'"

hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=success FAKE_RXC1=unarmed
expect "an activation whose RXC1 never read valid exited with 0" test "$(stage_rc)" = 3
expect "the unconfirmed RXC1 was not reported" out_has "RXC1 NOT CONFIRMED: the status is 'unarmed', not 'valid'"
expect "an active and receiving release was not reported as such" out_has 'RELEASE ACTIVE AND RECEPTION VERIFIED'
if out_has 'Done'; then fail "an unconfirmed RXC1 was reported as Done"; fi
if out_has "OK: the fresh RXC1 status"; then fail "an unconfirmed RXC1 was reported as OK"; fi
expect "the configuration is not byte-identical after activation" cfg_unchanged

for case_ in "rolled-back:1" "rollback-failed:70" "unproven:71"; do
  hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=${case_%%:*}
  expect "a helper that exited ${case_##*:} did not make the stage exit with it" test "$(stage_rc)" = "${case_##*:}"
  expect "a failed activation did not warn against a second arm" out_has 'Do NOT create another arm'
  expect "the configuration was left changed after a failed activation" cfg_unchanged
  if out_has 'RELEASE ACTIVE'; then fail "a failed activation was reported as active"; fi
done

wrong_release() {  # $1 = description, $2 = text the report must contain; the rest are assignments
  local what=$1 text=$2; shift 2
  hs_setup; hs_reconciled
  run_stage activate FAKE_OUTCOME=success "$@"
  expect "$what exited with 0" test "$(stage_rc)" = 1
  expect "$what was not explained: wanted '$text'" out_has "$text"
  if out_has 'RELEASE ACTIVE AND RECEPTION VERIFIED'; then fail "$what was reported as verified"; fi
}
wrong_release "a service that runs another build" "the running service reports build" READY_COMMIT=3333333333333333333333333333333333333333
wrong_release "a session that did not get the viewer role" "did not get the viewer role" FAKE_ROLE=operator
wrong_release "a session that used the legacy fallback" "legacy fallback was used" FAKE_LEGACY=true
wrong_release "a validation browser left behind" "a validation browser is still running" PROCS_LEFT=4242
wrong_release "a pointer that did not move" "the active release is" FAKE_NO_REPOINT=1
wrong_release "a validation that did not finish" "did not finish in the state 'done'" FAKE_FINAL_STATE=failed
wrong_release "a connection left at the Bridge" "connection(s) are open" FAKE_CLIENTS_AFTER=1
hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=success-uncommitted
expect "a record that is not committed was accepted" test "$(stage_rc)" = 1
expect "the uncommitted record was not explained" out_has "the record is not 'committed'"

hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=success FAIL_SED=2
expect "an activation whose restoration failed was reported as a success" test "$(stage_rc)" != 0
expect "an unrestored activation was not reported" out_has 'THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED'
if out_has 'OK: the fresh RXC1'; then fail "an activation with an unrestored policy printed OK"; fi

hs_setup; run_stage activate FAKE_OUTCOME=success
expect "activate ran on an unreconciled record" test "$(stage_rc)" != 0
expect "activate did not say to reconcile first" out_has 'run the reconcile stage first'
if helper_was_run; then fail "the helper was run on an unreconciled record"; fi
test ! -e "$TMP/hs/arm/armed" || fail "an arm was written although the record is unreconciled"
hs_setup; hs_reconciled; run_stage activate FAKE_OUTCOME=success CLIENTS=2
expect "activate ran with a browser connected" test "$(stage_rc)" != 0
test ! -e "$TMP/hs/arm/armed" || fail "an arm was written although a browser is connected"
hs_setup; hs_reconciled; touch "$TMP/hs/arm/armed"; run_stage activate FAKE_OUTCOME=success
expect "activate ran with an arm already present" test "$(stage_rc)" != 0
if helper_was_run; then fail "the helper was run although an arm exists"; fi


printf 'ok: the staged procedure pins this tree, refuses another commit and accepts only complete evidence\n'
