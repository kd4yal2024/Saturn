#!/bin/bash
# The headless-validation update, one stage per run.
#
# Run ON the G2 as user pi, at a terminal (sudo asks for pi's password). After each stage, send the output; it is read from elsewhere
# without sudo. Every stage stops on the first surprise.
#
#     PIN=<full commit> bash ~/g2_headless_steps.sh build   resolves the branch, checks it is exactly PIN, builds it natively in the background (no sudo)
#     bash ~/g2_headless_steps.sh status      read-only summary                                              (no sudo)
#     bash ~/g2_headless_steps.sh validate    the finished bundle: manifest check (no sudo) and the installer's --validate (sudo)
#     bash ~/g2_headless_steps.sh install     installs the release INACTIVE and validates the installed copy. Changes /opt/saturn/releases only
#     bash ~/g2_headless_steps.sh helper      installs the NEW activation helper + receive validator + guard script, each pinned by SHA-256
#     bash ~/g2_headless_steps.sh check       proves reception on the release that is RUNNING NOW with the production launcher, verifies the
#                                             browser's sandbox and memory cap, and verifies the cleanup from the host. Changes nothing
#     bash ~/g2_headless_steps.sh reconcile   records the verified recovery on the stuck 'rollback_failed' record, only if the previous
#                                             release is verifiably running AND receiving. The failed activation stays a failed activation
#     bash ~/g2_headless_steps.sh failure     OPTIONAL exercise of failure + rollback: an activation that is meant to fail validation, then roll back
#     bash ~/g2_headless_steps.sh activate    the normal update with every browser closed: RXC1 arm, then the helper (restarts the Bridge and Go)
#
# Order: build, validate, install, helper, check, reconcile, [failure], activate.
# CLOSE EVERY BROWSER TAB OF THE G2 (remote page, telemetry, anything) before check, reconcile, failure and activate: that is the condition under test.
#
# This script is part of the pushed source (update_manager/docs/headless-validation/g2_headless_steps.sh). PIN is the full commit the branch
# must resolve to, as published in the hand-off message; the script cannot name the commit it is stored in. The three hashes below are those of
# the committed blobs of the helper, the validator and the guard.
set -u
BRANCH=goldmarsh/headless-validation
ACT_SHA=2e3b8a98733eb82cc1ce4190a9550feb6cdc09be9e5419980da71341150bea9b
VAL_SHA=03c7228a2c4ce4f9561dad092261a65337309504d8065861ad9b552580a6c21f
GUARD_SHA=ff9200109db6dc01c524719fde03aa637eb536114322beea91475e6eedd1f2b7
PREV=329f3109b17428ee04e3f14031da6cf25cb04810
REPO_URL=https://github.com/kd4yal2024/Saturn.git
PIN_FILE=$HOME/.g2_headless_pin
H=/usr/local/lib/saturn-go/scripts
RELEASES=/opt/saturn/releases
ARM_DIR=/var/lib/saturn-rxc1-bridge
PERF=/run/saturn-bridge/perf.json
ACT_CFG=/etc/default/saturn-release-activate
TXN=/var/lib/saturn-state/deployments/current.json
EVIDENCE_ROOT=/var/lib/saturn-state/deployments/validation
LEFTOVER_DIRS=(/tmp /var/tmp)       # where a browser profile or private temp directory would be left behind
SETTLE_SECONDS=3                    # how long the owner is given to drop the session before the host is asked
say() { printf '\n--- %s\n' "$*"; }
die() { echo "STOP: $*" >&2; exit 1; }

# the commit under test: given once to the build stage, remembered, and every later stage refuses a different one
if [ "${1:-}" = build ]; then
  [[ "${PIN:-}" =~ ^[0-9a-f]{40}$ ]] || die "give the full 40-character commit: PIN=<commit> bash $0 build"
  if [ -s "$PIN_FILE" ] && [ "$(cat "$PIN_FILE")" != "$PIN" ]; then die "$PIN_FILE says $(cat "$PIN_FILE"); remove it deliberately to test another commit"; fi
  printf '%s\n' "$PIN" > "$PIN_FILE"
fi
C=$(cat "$PIN_FILE" 2>/dev/null || true)
if [ "${1:-}" != "" ] && [ "${1:-}" != build ] && [ "${1:-}" != status ]; then
  [[ "$C" =~ ^[0-9a-f]{40}$ ]] || die "no commit is pinned yet: run the build stage first (PIN=<commit> bash $0 build)"
  if [ -n "${PIN:-}" ] && [ "$PIN" != "$C" ]; then die "PIN=$PIN but the build stage pinned $C"; fi
fi
B=/var/lib/saturn-state/release-staging/$C

clients() { ss -tn state established '( sport = :50001 or sport = :8443 or sport = :1024 )' 2>/dev/null | tail -n +2 | wc -l; }
confirm() { printf '%s\n   type  yes  to continue: ' "$1"; read -r a; [ "$a" = yes ] || die "not confirmed"; }
txn_status() { python3 -c "import json; print(json.load(open('$TXN')).get('status',''))" 2>/dev/null; }
perf_field() { python3 -c "import json,sys; d=json.load(open('$PERF')); m=d.get('metrics',{}); print(eval(sys.argv[1]))" "$1" 2>/dev/null; }
need_no_browsers() {
  local n; n=$(clients)
  if [ "$n" != 0 ]; then
    echo "  $n connection(s) are open on the Bridge / TLS / P2 ports. Close every browser tab of the G2 and run this again."
    [ "${ALLOW_CLIENTS:-0}" = 1 ] || die "a human browser is connected; the point of this stage is that none is (ALLOW_CLIENTS=1 overrides, but then it proves nothing)"
  fi
}
perf_summary() {
  python3 - "$PERF" <<'PYEOF'
import json, sys, time
try:
    d = json.load(open(sys.argv[1]))
except Exception as e:
    print(f"  no telemetry at {sys.argv[1]}: {e}"); sys.exit(0)
m = d.get("metrics", {}); rx = m.get("rx_counter_v31") or {}
age = int(time.time() * 1000) - d.get("updated_at_ms", 0)
print(f"  backend {d.get('backend')}  status {d.get('status')}  age {age} ms  pid {m.get('pid')}  build {str(m.get('build_git_sha'))[:12]}  dirty {m.get('build_git_dirty')}")
print(f"  FPGA {m.get('date_code_hex')} fw {m.get('firmware_major')}.{m.get('firmware_minor')}  rf_safe {d.get('rf_safe')}  client {m.get('client')} connections {m.get('connections')}")
print(f"  RXC1 status {rx.get('status')}  host_acquisition_failures {rx.get('host_acquisition_failures')}")
PYEOF
}
record_summary() {
  python3 - "$TXN" <<'PYEOF'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception as e:
    print(f"  no transaction record: {e}"); sys.exit(0)
rb = d.get("rollback") or {}
print(f"  record: {d.get('status')} / {d.get('phase')}  target {str(d.get('target_commit'))[:12]}  previous {str(d.get('previous_commit'))[:12]}")
if d.get("activation_failure"): print(f"  activation failure: phase {d['activation_failure'].get('phase')} exit {d['activation_failure'].get('exit_status')}")
if rb: print(f"  rollback: {rb.get('status')}  release_restored {rb.get('release_restored')}  receive_verified {rb.get('receive_verified')}")
for label, v in (d.get("validation") or {}).items():
    print(f"  validation[{label}]: {v.get('final_state')}  driver {v.get('driver')}  role {v.get('role')}  policy {v.get('viewer_role_policy')}  legacy fallback used {v.get('legacy_fallback_used')}  failed {v.get('failed')}")
    print(f"      evidence: {v.get('control_directory')}")
rec = d.get("reconciliation")
if rec:
    print(f"  reconciliation: release_restored {rec.get('release_restored')}  receive_verified {rec.get('receive_verified')}  verified {str(rec.get('verified_release'))[:12]}  original {rec.get('original_status')}")
    print(f"      original record kept at {rec.get('original_record')}  sha256 {str(rec.get('original_record_sha256'))[:16]}")
PYEOF
}
latest_evidence() { find "$EVIDENCE_ROOT" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p/\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-; }

# The release manifest in directory $1 must name the pinned commit, as a clean tree, for the release and for every component in it:
# the script, the build and the manifest are then provably the one commit.
manifest_names_pin() {
  python3 - "$1/release-manifest.json" "$C" <<'PYEOF'
import json, sys
path, pin = sys.argv[1], sys.argv[2]
try:
    with open(path, encoding="utf-8") as handle:
        manifest = json.load(handle)
except (OSError, ValueError) as error:
    print(f"  cannot read {path}: {error}"); sys.exit(1)
source = manifest.get("source") if isinstance(manifest.get("source"), dict) else {}
components = manifest.get("components") if isinstance(manifest.get("components"), list) else []
problems = []
if source.get("commit") != pin:
    problems.append(f"source.commit is {source.get('commit')!r}")
if source.get("dirty") is not False:
    problems.append(f"source.dirty is {source.get('dirty')!r}")
if not components:
    problems.append("no components are listed")
for component in components:
    if not isinstance(component, dict) or component.get("source_commit") != pin:
        problems.append(f"component {component.get('name') if isinstance(component, dict) else component!r} names another commit")
if problems:
    print(f"  {path} does not describe the pinned commit {pin}: " + "; ".join(problems)); sys.exit(1)
print(f"  {path}: source commit {pin}, clean tree, {len(components)} components all from it")
PYEOF
}

stage_status() {
  say "commit under test: ${C:-<none pinned yet>}"
  say "release and binaries"
  echo "current -> $(readlink -f /opt/saturn/current)"; ls -1 /opt/saturn/releases
  for n in saturn-bridge saturn-go p2app; do for p in $(pgrep -x "$n"); do echo "$n pid $p  $(readlink "/proc/$p/exe" 2>/dev/null)  $(sha256sum "/proc/$p/exe" 2>/dev/null | cut -c1-16)"; done; done
  say "services (active / enabled)"
  for u in p2app saturn-bridge saturn-go; do echo "$u: $(systemctl is-active $u) / $(systemctl is-enabled $u)"; done
  say "telemetry"; perf_summary
  say "activation policy / transaction record"; grep -E '^(ACTIVATION_ENABLED|VALIDATION_)' "$ACT_CFG" 2>/dev/null; record_summary
  say "installed helpers"; for f in saturn-release-activate-root.sh saturn-remote-validate.py saturn-remote-receive-guard.js; do echo "$(sha256sum $H/$f 2>/dev/null | cut -c1-16)  $f"; done
  say "RXC1 state directory"; ls -la "$ARM_DIR"
  say "connected clients on the Bridge / TLS / P2 ports: $(clients)"
  say "readiness"; curl -fsS -m 5 http://127.0.0.1:8080/readyz 2>&1 | python3 -c "import json,sys; d=json.load(sys.stdin); print(' ', d.get('status'), 'build', d.get('build_commit'))" 2>&1
  say "build"; if [ -n "$C" ] && [ -d "$B" ]; then echo "bundle present: $B"; manifest_names_pin "$B"; else echo "bundle NOT present yet; tail ~/release-build-headless.log"; fi
}

stage_build() {
  [ ! -d "$B" ] || die "the bundle already exists: $B"
  cd /home/pi/github/Saturn/update_manager/scripts || die "the Saturn checkout is not where I expect it"
  say "resolving $BRANCH (no build yet)"
  out=$(./saturn-release-build.sh --source-remote "$REPO_URL" --source-ref "$BRANCH" --resolve-only 2>&1); echo "$out" | tail -3
  echo "$out" | grep -q "@ $C" || die "the branch does not resolve to the pinned commit $C. Has it been pushed? Tell me."
  SATURN_RELEASE_OUTPUT_ROOT=/var/lib/saturn-state/release-staging nohup ./saturn-release-build.sh --source-remote "$REPO_URL" --source-ref "$BRANCH" > ~/release-build-headless.log 2>&1 &
  echo "build started, pid $!. It runs the whole test gate and compiles natively; it takes a while. Progress: tail -f ~/release-build-headless.log"
}

stage_validate() {
  [ -d "$B" ] || die "the build has not produced $B yet (tail ~/release-build-headless.log)"
  say "manifest check (no root)"; python3 "$H/saturn-release-manifest.py" validate --release-root "$B" --components /usr/local/lib/saturn-go/release/components-v1.json || die "manifest validation failed"
  manifest_names_pin "$B" || die "the bundle's manifest is not the pinned commit's"
  say "installer --validate (sudo)"; sudo "$H/saturn-release-install-root.sh" --validate "$B" || die "installer validation failed"
  echo "OK: the bundle is valid. Next:  bash ~/g2_headless_steps.sh install"
}

stage_install() {
  [ -d "$B" ] || die "no bundle at $B"
  if [ -d "$RELEASES/$C" ]; then say "already installed: $RELEASES/$C"
  else say "installing INACTIVE (sudo)"; sudo "$H/saturn-release-install-root.sh" "$B" || die "install failed"; fi
  manifest_names_pin "$RELEASES/$C" || die "the installed release's manifest is not the pinned commit's"
  say "validating the installed release (sudo; read-only)"; sudo "$H/saturn-release-activate-root.sh" --validate "$C" || die "installed-release validation failed"
  bad=$(find "$RELEASES/$C/webroot" -type f ! -perm -o=r | wc -l); echo "unreadable web files: $bad"; [ "$bad" = 0 ] || die "web assets are not world-readable"
  echo "OK: installed inactive; /opt/saturn/current still -> $(readlink -f /opt/saturn/current). Next:  bash ~/g2_headless_steps.sh helper"
}

stage_helper() {
  R=/opt/saturn/releases/$C/scripts
  [ -d "/opt/saturn/releases/$C" ] || die "install the release first"
  grep -Fxq 'ACTIVATION_ENABLED="0"' "$ACT_CFG" || die "$ACT_CFG does not say ACTIVATION_ENABLED=\"0\" exactly once; the installer refuses to replace the helper otherwise. Tell me."
  say "the release's files against the hashes pinned in this script (made from the committed blobs)"
  for pair in "$ACT_SHA:saturn-release-activate-root.sh" "$VAL_SHA:saturn-remote-validate.py" "$GUARD_SHA:saturn-remote-receive-guard.js"; do
    got=$(sha256sum "$R/${pair#*:}" | cut -d' ' -f1)
    if [ "$got" = "${pair%%:*}" ]; then echo "  match  ${pair#*:}"; else die "${pair#*:} differs from the pinned hash ($got)"; fi
  done
  say "Chromium and systemd-run (the production launcher needs both)"; /usr/bin/chromium --version 2>&1 | head -1; command -v systemd-run || die "systemd-run missing"
  confirm "About to install the new activation helper, receive validator and guard (root; previous copies are backed up; activation stays DISABLED; no service is touched)."
  sudo "$H/saturn-release-install-activator-root.sh" "$C" "$ACT_SHA" "$VAL_SHA" "$GUARD_SHA" || die "helper installation failed; send me the output"
  echo "OK. Next:  bash ~/g2_headless_steps.sh check"
}

# What the check must establish, from the evidence the validator kept and from the host once it has finished.
check_evidence() {  # $1 = status.json
  python3 - "$1" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
c = d.get("conditions") or {}
required = ["release.running", "owner.fresh_telemetry", "page.loaded", "guard.active", "session.started", "data.display_fresh", "data.audio_frames_arriving",
            "audio.decoded_progress", "audio.context_running", "audio.no_decode_errors", "owner.fresh_telemetry_with_session", "owner.session_registered",
            "owner.gate_equivalent", "radio.unchanged", "wire.only_allowed_commands", "page.no_fatal_errors", "browser.sandbox_intact",
            "browser.memory_limit_applied", "cleanup.browser_gone", "cleanup.client_released"]
role = d.get("role"); policy = d.get("role_policy"); fallback = d.get("legacy_fallback_used")
role_condition = "session.role_legacy_fallback" if fallback else "session.connected_as_viewer"
bad = [k for k in required + [role_condition] if not (c.get(k) or {}).get("ok")]
print(f"  state {d.get('state')}  driver {d.get('driver')}  role {role}  policy {policy}")
for k in ("data.display_fresh", "data.audio_frames_arriving", "audio.decoded_progress", "audio.context_running", "owner.gate_equivalent", "radio.unchanged",
          "browser.sandbox_intact", "browser.memory_limit_applied", "cleanup.browser_gone", "cleanup.client_released"):
    print("  " + ("ok  " if (c.get(k) or {}).get("ok") else "FAIL") + " " + k + ": " + str((c.get(k) or {}).get("detail"))[:170])
mem = (d.get("isolation") or {}).get("memory") or {}
try:
    peak, cap = int(mem.get("peak_bytes")), int(mem.get("max_bytes"))
    print(f"  memory: peak {peak/1048576:.0f} MiB of a {cap/1048576:.0f} MiB cap ({100*peak/cap:.0f}%)")
except (TypeError, ValueError):
    print(f"  memory: peak not reported ({mem})")
if fallback:
    print("  LEGACY BRIDGE FALLBACK USED (expected only for a release that predates the viewer-role fix): " + str((c.get(role_condition) or {}).get("detail"))[:200])
elif role != "viewer":
    bad.append("role is not viewer")
if d.get("failed"): print("  FIRST FAILED:", d["failed"])
if bad: print("  NOT PROVEN:", ", ".join(bad)); sys.exit(1)
PYEOF
}

stage_check() {
  [ -x "$H/saturn-remote-validate.py" ] || die "run the helper stage first"
  [ "$(perf_field "d.get('backend')")" = xdma ] || die "the Bridge must be on the xdma backend (it reports '$(perf_field "d.get('backend')")')"
  need_no_browsers
  confirm "About to attach a temporary RECEIVE-ONLY headless browser session to the running release for about 15 seconds. It changes nothing: no tuning, no setting, no service restart, no record."
  before_conn=$(perf_field "m.get('connections')"); say "before: connections $before_conn, validator units: $(systemctl list-units --all --plain --no-legend 'saturn-remote-validator-*' | wc -l)"
  sudo "$H/saturn-release-activate-root.sh" --check-reception; rc=$?
  say "result of the helper (exit $rc)"; ev=$(latest_evidence); echo "evidence: $ev"
  proven=1
  if [ -n "$ev" ] && [ -r "$ev/status.json" ]; then check_evidence "$ev/status.json" || proven=0; else echo "  cannot read status.json"; proven=0; fi
  say "cleanup, checked from the host"
  units=$(systemctl list-units --all --plain --no-legend 'saturn-remote-validator-*' | wc -l); echo "  validator units left: $units"
  procs=$(pgrep -f -- '--remote-debugging-pipe' | wc -l); echo "  browser processes left (--remote-debugging-pipe): $procs"
  sleep "$SETTLE_SECONDS"; after_conn=$(perf_field "m.get('connections')"); echo "  Bridge connections: $before_conn before, $after_conn after"
  echo "  client flag at the Bridge: $(perf_field "m.get('client')")"
  leftovers=$(find "${LEFTOVER_DIRS[@]}" -maxdepth 1 \( -name 'saturn-validate-*' -o -name 'systemd-private-*saturn-remote-validator*' \) 2>/dev/null | wc -l); echo "  leftover profile/temp directories: $leftovers"
  if [ "$units" != 0 ] || [ "$procs" != 0 ] || [ "$after_conn" != "$before_conn" ] || [ "$leftovers" != 0 ]; then echo "  CLEANUP NOT COMPLETE"; proven=0; fi
  perf_summary
  if [ "$rc" = 0 ] && [ "$proven" = 1 ]; then echo "OK: reception, sandbox, memory cap and cleanup are all verified. Next:  bash ~/g2_headless_steps.sh reconcile"; return 0; fi
  echo "NOT PROVEN. Send me this output; the evidence directory above has the full log (readable by pi)."; return 1
}

stage_reconcile() {
  [ "$(txn_status)" = rollback_failed ] || die "the record says '$(txn_status)'; there is nothing to reconcile"
  [ "$(readlink -f /opt/saturn/current)" = "/opt/saturn/releases/$PREV" ] || die "the previous release ($PREV) is not the active one"
  need_no_browsers
  sudo cp -a "$TXN" "$TXN.before-reconcile.$(date +%Y%m%dT%H%M%S)" || die "could not keep a copy of the record"
  confirm "Record the verified recovery on the 'rollback_failed' record? The helper verifies that $PREV is running (pointer, drop-ins, services, commit, owner) and receiving; if anything differs it changes nothing. The failed activation stays a failed activation and the original record is preserved."
  sudo "$H/saturn-release-activate-root.sh" --reconcile-rollback; rc=$?
  say "result (exit $rc)"; record_summary
  if [ "$rc" = 0 ]; then
    python3 - "$TXN" <<'PYEOF' || return 1
import json, sys
d = json.load(open(sys.argv[1]))
rec = d.get("reconciliation") or {}
orig = rec.get("original_transaction") or {}
ok = (d.get("status") == "rolled_back" and d.get("status") != "committed"
      and d.get("activation_failure") == orig.get("activation_failure") and d.get("rollback") == orig.get("rollback")
      and orig.get("status") == "rollback_failed" and rec.get("release_restored") is True and rec.get("receive_verified") is True)
print("  the original failure and rollback outcome are unchanged, and the recovery is recorded beside them:", "yes" if ok else "NO")
sys.exit(0 if ok else 1)
PYEOF
    echo "OK. Next: bash ~/g2_headless_steps.sh activate   (or 'failure' first, if you want the rollback exercised on the real appliance)"
  fi
  return "$rc"
}

# Sets the activation policy for the duration of one helper run, then restores it.
with_activation_enabled() {
  OLD=$(grep -E '^ACTIVATION_ENABLED=' "$ACT_CFG" | head -1); echo "activation policy now: $OLD"
  restore_policy() { if [ "$OLD" != 'ACTIVATION_ENABLED="1"' ]; then sudo sed -i "s/^ACTIVATION_ENABLED=.*/$OLD/" "$ACT_CFG" && echo "activation policy restored to: $OLD"; fi; }
  if [ "$OLD" != 'ACTIVATION_ENABLED="1"' ]; then sudo sed -i 's/^ACTIVATION_ENABLED=.*/ACTIVATION_ENABLED="1"/' "$ACT_CFG" || die "could not enable activation"; fi
  trap restore_policy EXIT
}

common_activation_preconditions() {
  [ -d "/opt/saturn/releases/$C" ] || die "install the release first"
  [ "$(perf_field "d.get('backend')")" = xdma ] || die "the Bridge must be on the xdma backend"
  [ "$(perf_field "m.get('date_code_hex')")" = 53460004 ] || die "the FPGA is not 0x53460004"
  [ "$(readlink -f /opt/saturn/current)" != "/opt/saturn/releases/$C" ] || die "the release is already active"
  case "$(txn_status)" in ""|committed|rolled_back) ;; *) die "the transaction record says '$(txn_status)'; run the reconcile stage first" ;; esac
  grep -Fq "$VAL_SHA" <(sha256sum "$H/saturn-remote-validate.py") || die "the pinned validator is not the installed one; run the helper stage"
  grep -Fq "$ACT_SHA" <(sha256sum "$H/saturn-release-activate-root.sh") || die "the pinned activation helper is not the installed one; run the helper stage"
  need_no_browsers
}

stage_failure() {
  common_activation_preconditions
  [ ! -e "$ARM_DIR/armed" ] || die "an 'armed' file exists; tell me before doing anything"
  confirm "OPTIONAL EXERCISE. About to run an activation that is MEANT TO FAIL: the new release is started, the receive validation runs against an RXC1 state it cannot meet ('unsupported'), the helper rolls back to $PREV and proves reception there. The Bridge and Go restart TWICE (about a minute of no radio). No arm is written. Nothing stays changed."
  with_activation_enabled
  printf 'VALIDATION_EXPECT_RXC1="unsupported"\n' | sudo tee -a "$ACT_CFG" > /dev/null || die "could not set the exercise policy"
  restore_expect() { sudo sed -i '/^VALIDATION_EXPECT_RXC1="unsupported"$/d' "$ACT_CFG"; restore_policy; }
  trap restore_expect EXIT
  sudo "$H/saturn-release-activate-root.sh" "$C"; rc=$?
  echo "activation helper exit status: $rc   (expected: non-zero, but NOT 70 and NOT 71)"
  say "result"; record_summary; stage_status | sed -n '/release and binaries/,/telemetry/p'
  [ "$(readlink -f /opt/saturn/current)" = "/opt/saturn/releases/$PREV" ] && echo "the previous release is active again, as intended" || echo "UNEXPECTED: current is $(readlink -f /opt/saturn/current)"
  return 0
}

stage_activate() {
  common_activation_preconditions
  [ "$(perf_field "m.get('date_code_hex')")" = 53460004 ] || die "the FPGA is not 0x53460004"
  [ ! -e "$ARM_DIR/armed" ] || die "an 'armed' file already exists; tell me before doing anything"
  confirm "About to write the one-use RXC1 arm and ACTIVATE $C with EVERY BROWSER CLOSED: saturn-bridge and saturn-go restart; rollback target is $PREV; reception is proven by the helper's own headless session."
  with_activation_enabled
  # the arm: written LAST, immediately before the start that will consume it (it is rejected as stale after ten minutes)
  if ! { printf 'RXC1-TRIAL-0x53460004\n' > "$ARM_DIR/armed.tmp" && sync "$ARM_DIR/armed.tmp" && mv "$ARM_DIR/armed.tmp" "$ARM_DIR/armed" && sync "$ARM_DIR/armed" "$ARM_DIR"; }; then die "could not write the arm"; fi
  echo "arm written: $(ls -l "$ARM_DIR/armed")"
  sudo "$H/saturn-release-activate-root.sh" "$C"; rc=$?
  echo "activation helper exit status: $rc"
  say "result"; record_summary
  if [ "$rc" != 0 ]; then echo "The helper rolls back by itself. Do NOT create another arm. Send me this output."; exit "$rc"; fi
  say "waiting for fresh RXC1 snapshots (up to 180 s)"
  for _ in $(seq 1 90); do [ "$(perf_field "(m.get('rx_counter_v31') or {}).get('status')")" = valid ] && break; sleep 2; done
  perf_summary; stage_status | sed -n '/release and binaries/,/telemetry/p;/RXC1 state/,/connected/p'
  echo "Done. The rest is verified from the evidence."
}

# sourced by tests/test-g2-headless-steps.sh, which exercises the checks without touching a host
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then return 0; fi

case "${1:-}" in
  status) stage_status ;;
  build) stage_build ;;
  validate) stage_validate ;;
  install) stage_install ;;
  helper) stage_helper ;;
  check) stage_check ;;
  reconcile) stage_reconcile ;;
  failure) stage_failure ;;
  activate) stage_activate ;;
  *) sed -n 2,23p "$0"; exit 2 ;;
esac
