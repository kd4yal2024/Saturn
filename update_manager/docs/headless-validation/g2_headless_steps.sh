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
#     bash ~/g2_headless_steps.sh check [SIZE] proves reception on the release that is RUNNING NOW with the production launcher, verifies the
#                                             browser's sandbox and memory cap, and verifies the cleanup from the host. Changes nothing.
#                                             SIZE (for example 500M) measures with another browser memory cap for this one check only
#     bash ~/g2_headless_steps.sh reconcile   records the verified recovery on the stuck 'rollback_failed' record, only if the previous
#                                             release is verifiably running AND receiving. The failed activation stays a failed activation
#     bash ~/g2_headless_steps.sh failure     OPTIONAL exercise of failure + rollback: an activation that is meant to fail validation, then roll back
#     bash ~/g2_headless_steps.sh activate    the normal update with every browser closed: RXC1 arm, then the helper (restarts the Bridge and Go)
#
# reconcile, failure and activate need the root-owned ACTIVATION_ENABLED="1" for the helper to act. Each sets it for that one helper run and puts the
# configuration back byte-identical, or says loudly that it could not. A stage exits 0 only when the outcome is the expected one; activate exits 3
# when the release is active and receiving but the RXC1 status never read 'valid'.
# Order: build, validate, install, helper, check, reconcile, [failure], activate.
# CLOSE EVERY BROWSER TAB OF THE G2 (remote page, telemetry, anything) before check, reconcile, failure and activate: that is the condition under test.
#
# This script is part of the pushed source (update_manager/docs/headless-validation/g2_headless_steps.sh). PIN is the full commit the branch
# must resolve to, as published in the hand-off message; the script cannot name the commit it is stored in. The four hashes below are those of
# the committed blobs of the helper's installer, the helper, the validator and the guard.
set -u
# A non-interactive shell (ssh pi@g2 'command') has no Rust toolchain on its PATH; the builder needs cargo.
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) PATH="$HOME/.cargo/bin:$PATH" ;; esac
BRANCH=goldmarsh/headless-validation
ACT_SHA=f4c307d7e6f5a1172837d1b12ff83589386f239cf01d541ae9530456178b829d
VAL_SHA=5d95ec94e6c68fdb925aa4e0ee10b1b3b00f88fdea53c6728cbd6cc31c0fa020
GUARD_SHA=b5ad14b439bf48bcc49703368fac9000a6dfd02b481419c3123ce35476a721fb
INSTALLER_SHA=6a094523d810f2058bcc22fa7c5d80a401d78e5760b145bef98451a7459f2b67
PREV=329f3109b17428ee04e3f14031da6cf25cb04810
REPO_URL=https://github.com/kd4yal2024/Saturn.git
PIN_FILE=$HOME/.g2_headless_pin
H=/usr/local/lib/saturn-go/scripts
RELEASES=/opt/saturn/releases
CURRENT_LINK=/opt/saturn/current
BROWSER=/usr/bin/chromium
SYSTEMD_RUN=systemd-run
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
  echo "current -> $(readlink -f "$CURRENT_LINK")"; ls -1 /opt/saturn/releases
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
  out=$(./saturn-release-build.sh --source-remote "$REPO_URL" --source-ref "$BRANCH" --resolve-only 2>&1); echo "$out" | tail -4
  echo "$out" | grep -Fxq "resolved_commit=$C" || die "the builder did not report resolved_commit=$C (its output is above: either the branch is not at the pinned commit or has not been pushed, or the builder could not run)"
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
  echo "OK: installed inactive; /opt/saturn/current still -> $(readlink -f "$CURRENT_LINK"). Next:  bash ~/g2_headless_steps.sh helper"
}

stage_helper() {
  R=$RELEASES/$C/scripts
  [ -d "$RELEASES/$C" ] || die "install the release first"
  grep -Fxq 'ACTIVATION_ENABLED="0"' "$ACT_CFG" || die "$ACT_CFG does not say ACTIVATION_ENABLED=\"0\" exactly once; the installer refuses to replace the helper otherwise. Tell me."
  say "the release's files against the hashes pinned in this script (made from the committed blobs)"
  for pair in "$INSTALLER_SHA:saturn-release-install-activator-root.sh" "$ACT_SHA:saturn-release-activate-root.sh" "$VAL_SHA:saturn-remote-validate.py" "$GUARD_SHA:saturn-remote-receive-guard.js"; do
    got=$(sha256sum "$R/${pair#*:}" | cut -d' ' -f1)
    if [ "$got" = "${pair%%:*}" ]; then echo "  match  ${pair#*:}"; else die "${pair#*:} differs from the pinned hash ($got)"; fi
  done
  say "Chromium and systemd-run (the production launcher needs both)"; "$BROWSER" --version 2>&1 | head -1; command -v "$SYSTEMD_RUN" || die "systemd-run missing"
  confirm "About to install the new activation helper, receive validator and guard (root; previous copies are backed up; activation stays DISABLED; no service is touched)."
  sudo "$R/saturn-release-install-activator-root.sh" "$C" "$ACT_SHA" "$VAL_SHA" "$GUARD_SHA" || die "helper installation failed; send me the output"
  echo "OK. Next:  bash ~/g2_headless_steps.sh check"
}

# What the check must establish, from the evidence the validator kept and from the host once it has finished.
check_evidence() {  # $1 = status.json
  python3 - "$1" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
c = d.get("conditions") or {}
mem = (d.get("isolation") or {}).get("memory") or {}
enforcement = d.get("memory_enforcement")
required = ["release.running", "owner.fresh_telemetry", "page.loaded", "guard.active", "session.started", "data.display_fresh", "data.audio_frames_arriving",
            "audio.decoded_progress", "audio.context_running", "audio.no_decode_errors", "owner.fresh_telemetry_with_session", "owner.session_registered",
            "owner.gate_equivalent", "radio.unchanged", "wire.only_allowed_commands", "page.no_fatal_errors", "host.memory_headroom",
            "browser.sandbox_intact", "browser.memory_limit_applied", "cleanup.browser_gone", "cleanup.client_released"]
if enforcement == "watchdog":
    required.append("browser.memory_within_cap")      # the validator is the cap here, so that the browser stayed within it is a condition of its own
role = d.get("role"); policy = d.get("role_policy"); fallback = d.get("legacy_fallback_used")
role_condition = "session.role_legacy_fallback" if fallback else "session.connected_as_viewer"
bad = [k for k in required + [role_condition] if not (c.get(k) or {}).get("ok")]
if enforcement not in ("cgroup", "watchdog"):
    bad.append("memory_enforcement is not stated (cgroup or watchdog)")
print(f"  state {d.get('state')}  driver {d.get('driver')}  role {role}  policy {policy}")
for k in ("host.memory_headroom", "data.display_fresh", "data.audio_frames_arriving", "audio.decoded_progress", "audio.context_running", "owner.gate_equivalent",
          "radio.unchanged", "browser.sandbox_intact", "browser.memory_limit_applied", "browser.memory_within_cap", "cleanup.browser_gone", "cleanup.client_released"):
    if k in c or k in required:
        print("  " + ("ok  " if (c.get(k) or {}).get("ok") else "FAIL") + " " + k + ": " + str((c.get(k) or {}).get("detail"))[:170])
try:
    peak, cap = int(mem.get("peak_bytes")), int(mem.get("cap_bytes") or mem.get("max_bytes"))
    print(f"  memory: peak {peak/1048576:.0f} MiB of a {cap/1048576:.0f} MiB cap ({100*peak/cap:.0f}%), peak from {mem.get('peak_source', 'the kernel')}")
except (TypeError, ValueError):
    print(f"  memory: peak not reported ({mem})")
if enforcement == "watchdog":
    print("  MEMORY CAP ENFORCED BY THE VALIDATOR'S WATCHDOG, not the kernel (no memory cgroup controller on this host): the unit is stopped above the cap")
elif enforcement == "cgroup":
    print("  memory cap enforced by the kernel (cgroup MemoryMax, no swap)")
s = (d.get("diagnostics") or {}).get("summary") or {}
if s:
    def rng(k): return f"max {s[k]['max']} mean {s[k]['mean']}" if isinstance(s.get(k), dict) else "n/a"
    print(f"  resources over {s.get('samples')} one-second samples: browser memory (Pss) MiB {rng('browser_pss_mb')}; browser CPU % of one core {rng('browser_cpu_pct')};")
    print(f"    host busy % {rng('host_busy_pct')}; host I/O wait % {rng('host_iowait_pct')}; load {rng('load1')}; available memory min {s.get('mem_available_mb_min', 'n/a')} MiB;")
    print(f"    swap pages in/out {s.get('swap_in_pages_total', 'n/a')}/{s.get('swap_out_pages_total', 'n/a')}; browser major faults {s.get('browser_major_faults_total', 'n/a')};"
          f" page main thread busy {s.get('page_main_thread_busy', 'n/a')}, script {s.get('page_script_busy', 'n/a')}")
    kinds = s.get("pss_by_process_type_mb_max") or {}
    if kinds:
        print("    memory by process type (most each reached, MiB Pss): " + ", ".join(f"{k} x{v['processes']} {v['pss_mb']}" for k, v in kinds.items()))
    for key, counter in sorted((s.get("owner_counters") or {}).items()):
        print(f"    owner {key}: {counter['first']} -> {counter['last']} (+{counter['delta']})")
last = d.get("last_snapshot") or {}
if last:
    print("  page counters: " + ", ".join(f"{k} {last[k]}" for k in ("opusFrames", "decodedFrames", "decoderFramesOutput", "audioFramesPlayed", "lateDrops", "audioSeqGaps",
                                                                   "underruns", "queuedMs", "decodeErrors") if k in last))
if fallback:
    print("  LEGACY BRIDGE FALLBACK USED (expected only for a release that predates the viewer-role fix): " + str((c.get(role_condition) or {}).get("detail"))[:200])
elif role != "viewer":
    bad.append("role is not viewer")
if d.get("failed"): print("  FIRST FAILED:", d["failed"])
if bad: print("  NOT PROVEN:", ", ".join(bad)); sys.exit(1)
PYEOF
}

stage_check() {
  local cap="${1:-}" helper_args=(--check-reception)
  [[ -z "$cap" || "$cap" =~ ^[1-9][0-9]*[MG]$ ]] || die "the browser memory cap for a measuring check must look like 500M"
  [ -z "$cap" ] || helper_args+=(--memory-max "$cap")
  [ -x "$H/saturn-remote-validate.py" ] || die "run the helper stage first"
  [ "$(perf_field "d.get('backend')")" = xdma ] || die "the Bridge must be on the xdma backend (it reports '$(perf_field "d.get('backend')")')"
  need_no_browsers
  confirm "About to attach a temporary RECEIVE-ONLY headless browser session to the running release for about 15 seconds${cap:+ (browser memory cap $cap for this check)}. It changes nothing: no tuning, no setting, no service restart, no record."
  before_conn=$(perf_field "m.get('connections')"); say "before: connections $before_conn, validator units: $(systemctl list-units --all --plain --no-legend 'saturn-remote-validator-*' | wc -l)"
  sudo "$H/saturn-release-activate-root.sh" "${helper_args[@]}"; rc=$?
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
  if [ "$rc" = 0 ] && [ "$proven" = 1 ]; then
    next=activate; [ "$(txn_status)" != rollback_failed ] || next=reconcile      # a record that is still rollback_failed has to be reconciled first
    echo "OK: reception, sandbox, memory cap and cleanup are all verified. Next:  bash ~/g2_headless_steps.sh $next"; return 0
  fi
  echo "NOT PROVEN. Send me this output; the evidence directory above has the full log (readable by pi)."; return 1
}

stage_reconcile() {
  [ "$(txn_status)" = rollback_failed ] || die "the record says '$(txn_status)'; there is nothing to reconcile"
  [ "$(readlink -f "$CURRENT_LINK")" = "$RELEASES/$PREV" ] || die "the previous release ($PREV) is not the active one"
  need_no_browsers
  sudo cp -a "$TXN" "$TXN.before-reconcile.$(date +%Y%m%dT%H%M%S)" || die "could not keep a copy of the record"
  confirm "Record the verified recovery on the 'rollback_failed' record? The helper verifies that $PREV is running (pointer, drop-ins, services, commit, owner) and receiving; if anything differs it changes nothing. The failed activation stays a failed activation and the original record is preserved."
  # the helper refuses to reconcile while the root-owned policy says ACTIVATION_ENABLED="0"; it is set for this one run and restored on exit
  with_activation_enabled
  sudo "$H/saturn-release-activate-root.sh" --reconcile-rollback; rc=$?
  restored=1; restore_policy || restored=0
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
    [ "$restored" = 1 ] || { echo "STOP: the recovery was recorded, but the activation policy is not confirmed restored (see above)"; return 1; }
    echo "OK. Next: bash ~/g2_headless_steps.sh activate   (or 'failure' first, if you want the rollback exercised on the real appliance)"
  fi
  return "$rc"
}

# The root-owned policy that lets the helper activate or reconcile (ACTIVATION_ENABLED) is switched on for ONE helper run and put back exactly as
# it was. The configuration is hashed first and compared afterwards, so a line this procedure added and did not remove, or any other change, is
# reported. The restoration also runs when the stage dies or is interrupted (INT, TERM, HUP), and a restoration that cannot be confirmed makes
# the stage fail loudly instead of being passed over.
POLICY_ACTIVE=0 POLICY_OLD="" POLICY_EXTRA="" POLICY_HASH=""
with_activation_enabled() {  # $1 (optional) = one extra setting line for this run, for example VALIDATION_EXPECT_RXC1="unsupported"
  local extra_re='^[A-Z][A-Z0-9_]*="[a-z0-9]+"$'
  POLICY_EXTRA="${1:-}"
  [ -z "$POLICY_EXTRA" ] || [[ "$POLICY_EXTRA" =~ $extra_re ]] || die "unsafe extra policy line: $POLICY_EXTRA"
  [ "$(grep -cE '^ACTIVATION_ENABLED=' "$ACT_CFG")" = 1 ] || die "$ACT_CFG must have exactly one ACTIVATION_ENABLED line"
  POLICY_OLD=$(grep -E '^ACTIVATION_ENABLED=' "$ACT_CFG")
  case "$POLICY_OLD" in 'ACTIVATION_ENABLED="0"'|'ACTIVATION_ENABLED="1"') ;; *) die "unexpected policy line '$POLICY_OLD'" ;; esac
  POLICY_HASH=$(sha256sum "$ACT_CFG" | cut -d' ' -f1)
  echo "activation policy now: $POLICY_OLD"
  trap policy_on_exit EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap 'exit 129' HUP
  POLICY_ACTIVE=1    # from here on a restoration is owed
  if [ "$POLICY_OLD" != 'ACTIVATION_ENABLED="1"' ]; then
    sudo sed -i 's/^ACTIVATION_ENABLED=.*/ACTIVATION_ENABLED="1"/' "$ACT_CFG" || die "could not enable activation"
    grep -Fxq 'ACTIVATION_ENABLED="1"' "$ACT_CFG" || die "activation did not read back as enabled"
  fi
  if [ -n "$POLICY_EXTRA" ]; then printf '%s\n' "$POLICY_EXTRA" | sudo tee -a "$ACT_CFG" > /dev/null || die "could not set $POLICY_EXTRA"; fi
}

# Puts the configuration back. Returns non-zero unless it is byte-identical to what it was before with_activation_enabled.
restore_policy() {
  [ "$POLICY_ACTIVE" = 1 ] || return 0
  POLICY_ACTIVE=0
  local bad=0 now
  if [ -n "$POLICY_EXTRA" ]; then sudo sed -i "/^${POLICY_EXTRA}\$/d" "$ACT_CFG" || bad=1; fi
  if [ "$POLICY_OLD" != 'ACTIVATION_ENABLED="1"' ]; then sudo sed -i "s/^ACTIVATION_ENABLED=.*/${POLICY_OLD}/" "$ACT_CFG" || bad=1; fi
  now=$(sha256sum "$ACT_CFG" 2>/dev/null | cut -d' ' -f1)
  if [ "$bad" = 0 ] && [ "$now" = "$POLICY_HASH" ]; then
    echo "activation policy restored to: $POLICY_OLD (the configuration is byte-identical to before)"; return 0
  fi
  {
    echo "!!! THE ACTIVATION POLICY IS NOT CONFIRMED RESTORED: $ACT_CFG hashes to ${now:-unreadable}; before this stage it hashed to $POLICY_HASH."
    echo "!!! It must say  $POLICY_OLD  exactly once, and nothing may have been added. Look:  grep -nE '^(ACTIVATION_ENABLED|VALIDATION_EXPECT_RXC1)=' $ACT_CFG"
    echo "!!! Put it right with:  sudo sed -i 's/^ACTIVATION_ENABLED=.*/${POLICY_OLD}/' $ACT_CFG"
  } >&2
  return 1
}

policy_on_exit() {
  local rc=$?
  trap - EXIT INT TERM HUP
  if ! restore_policy; then [ "$rc" != 0 ] || rc=1; fi
  exit "$rc"
}

record_field() { python3 -c "import json,sys; d=json.load(open('$TXN')); print(eval(sys.argv[1]))" "$1" 2>/dev/null; }
ready_commit() { curl -fsS -m 5 http://127.0.0.1:8080/readyz 2>/dev/null | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('build_commit') if d.get('status') == 'ready' else '')" 2>/dev/null; }

common_activation_preconditions() {
  [ -d "$RELEASES/$C" ] || die "install the release first"
  [ "$(perf_field "d.get('backend')")" = xdma ] || die "the Bridge must be on the xdma backend"
  [ "$(perf_field "m.get('date_code_hex')")" = 53460004 ] || die "the FPGA is not 0x53460004"
  [ "$(readlink -f "$CURRENT_LINK")" != "$RELEASES/$C" ] || die "the release is already active"
  case "$(txn_status)" in ""|committed|rolled_back) ;; *) die "the transaction record says '$(txn_status)'; run the reconcile stage first" ;; esac
  grep -Fq "$VAL_SHA" <(sha256sum "$H/saturn-remote-validate.py") || die "the pinned validator is not the installed one; run the helper stage"
  grep -Fq "$ACT_SHA" <(sha256sum "$H/saturn-release-activate-root.sh") || die "the pinned activation helper is not the installed one; run the helper stage"
  need_no_browsers
}

# What the exercise has to have produced, read from the host's own records and not from the helper's exit status alone.
failure_verdict() {  # $1 = the helper's exit status, $2 = 1 if the configuration was confirmed restored
  local rc=$1 restored=$2 problems=() cur
  case "$rc" in
    0) problems+=("the activation SUCCEEDED, so $C may now be the active release; it was meant to fail the receive validation") ;;
    70) problems+=("exit 70: the previous release was NOT restored") ;;
    71) problems+=("exit 71: the previous release was restored but reception on it is not proven") ;;
  esac
  cur=$(readlink -f "$CURRENT_LINK")
  [ "$cur" = "$RELEASES/$PREV" ] || problems+=("the active release is ${cur:-unknown}, not $PREV")
  [ "$(record_field "d.get('target_commit')")" = "$C" ] || problems+=("the record is not about $C (its target is '$(record_field "d.get('target_commit')")'): this run left no record of its own")
  [ "$(record_field "d.get('status')")" = rolled_back ] || problems+=("the record says '$(record_field "d.get('status')")', not rolled_back")
  [ "$(record_field "(d.get('activation_failure') or {}).get('phase')")" = receive-validation ] \
    || problems+=("the activation did not fail in the receive validation (its phase is '$(record_field "(d.get('activation_failure') or {}).get('phase')")'), so the exercise did not test what it is for")
  { [ "$(record_field "(d.get('rollback') or {}).get('release_restored')")" = True ] && [ "$(record_field "(d.get('rollback') or {}).get('receive_verified')")" = True ]; } \
    || problems+=("the rollback record does not say that the previous release was restored and is receiving")
  [ "$restored" = 1 ] || problems+=("the activation policy was not confirmed restored")
  if [ "${#problems[@]}" = 0 ]; then
    echo "OK: the activation failed its receive validation as intended, rolled back to $PREV, proved reception there, and the policy is restored."; return 0
  fi
  echo "NOT AS EXPECTED:"; printf '  - %s\n' "${problems[@]}"; return 1
}

# After a helper run that reported success: is $C really the active, ready, committed release, validated as a viewer, with nothing left behind?
# Exit status 0 only if RXC1 also reads 'valid'; 1 if the release state is not as it should be; 3 if the release is fine and only RXC1 is unconfirmed.
activation_verdict() {  # $1 = 1 if the configuration was confirmed restored, $2 = the RXC1 status read after the wait
  local restored=$1 rxc1=$2 problems=() cur ready
  cur=$(readlink -f "$CURRENT_LINK"); ready=$(ready_commit)
  [ "$cur" = "$RELEASES/$C" ] || problems+=("the active release is ${cur:-unknown}, not $C")
  [ "$ready" = "$C" ] || problems+=("the running service reports build '${ready:-none}', not $C")
  { [ "$(record_field "d.get('target_commit')")" = "$C" ] && [ "$(record_field "d.get('status')")" = committed ]; } \
    || problems+=("the record is not 'committed' for $C (it says '$(record_field "d.get('status')")' for '$(record_field "d.get('target_commit')")')")
  [ "$(record_field "((d.get('validation') or {}).get('target') or {}).get('final_state')")" = "done" ] || problems+=("the receive validation of the new release did not finish in the state 'done'")
  [ "$(record_field "((d.get('validation') or {}).get('target') or {}).get('role')")" = viewer ] || problems+=("the validation session of the new release did not get the viewer role")
  [ "$(record_field "((d.get('validation') or {}).get('target') or {}).get('legacy_fallback_used')")" = False ] || problems+=("the legacy fallback was used, or not reported, for the new release")
  [ "$(clients)" = 0 ] || problems+=("$(clients) connection(s) are open on the Bridge / TLS / P2 ports")
  [ "$(pgrep -f -- '--remote-debugging-pipe' | wc -l)" = 0 ] || problems+=("a validation browser is still running")
  [ "$restored" = 1 ] || problems+=("the activation policy was not confirmed restored")
  if [ "${#problems[@]}" != 0 ]; then echo "NOT AS EXPECTED:"; printf '  - %s\n' "${problems[@]}"; return 1; fi
  echo "RELEASE ACTIVE AND RECEPTION VERIFIED: $C is the active, ready, committed release; its receive validation ran as the viewer; no browser is left; the policy is restored."
  if [ "$rxc1" = valid ]; then echo "OK: the fresh RXC1 status is 'valid'. The rest is verified from the evidence."; return 0; fi
  echo "RXC1 NOT CONFIRMED: the status is '${rxc1:-unreadable}', not 'valid', after waiting 180 s. The release is active and receiving; whether the RXC1 trial result appears is the open question. Send me this output."
  return 3
}

stage_failure() {
  common_activation_preconditions
  [ ! -e "$ARM_DIR/armed" ] || die "an 'armed' file exists; tell me before doing anything"
  confirm "OPTIONAL EXERCISE. About to run an activation that is MEANT TO FAIL: the new release is started, the receive validation runs against an RXC1 state it cannot meet ('unsupported'), the helper rolls back to $PREV and proves reception there. The Bridge and Go restart TWICE (about a minute of no radio). No arm is written. Nothing stays changed."
  with_activation_enabled 'VALIDATION_EXPECT_RXC1="unsupported"'
  sudo "$H/saturn-release-activate-root.sh" "$C"; rc=$?
  restored=1; restore_policy || restored=0
  echo "activation helper exit status: $rc   (expected: non-zero, but NOT 70 and NOT 71)"
  say "result"; record_summary; stage_status | sed -n '/release and binaries/,/telemetry/p'
  failure_verdict "$rc" "$restored"
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
  restored=1; restore_policy || restored=0
  echo "activation helper exit status: $rc"
  say "result"; record_summary
  if [ "$rc" != 0 ]; then echo "The helper rolls back by itself. Do NOT create another arm. Send me this output."; exit "$rc"; fi
  say "waiting for fresh RXC1 snapshots (up to 180 s)"
  rxc1=""
  for _ in $(seq 1 90); do rxc1=$(perf_field "(m.get('rx_counter_v31') or {}).get('status')"); [ "$rxc1" = valid ] && break; sleep 2; done
  perf_summary; stage_status | sed -n '/release and binaries/,/telemetry/p;/RXC1 state/,/connected/p'
  activation_verdict "$restored" "$rxc1"
}

# sourced by tests/test-g2-headless-steps.sh, which exercises the checks without touching a host
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then return 0; fi

case "${1:-}" in
  status) stage_status ;;
  build) stage_build ;;
  validate) stage_validate ;;
  install) stage_install ;;
  helper) stage_helper ;;
  check) stage_check "${2:-}" ;;
  reconcile) stage_reconcile ;;
  failure) stage_failure ;;
  activate) stage_activate ;;
  *) sed -n 2,26p "$0"; exit 2 ;;
esac
