#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
ACTIVATOR="$REPO_ROOT/update_manager/scripts/saturn-release-activate-root.sh"
MANIFEST_TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
STATE_TOOL="$REPO_ROOT/update_manager/scripts/saturn-state-compatibility.py"
COMPONENTS="$REPO_ROOT/update_manager/release/components-v1.json"
REAL_PYTHON="$(command -v python3)"
TMP_ROOT="$(mktemp -d)"
SATURN_ROOT="$TMP_ROOT/saturn"
RELEASES_ROOT="$SATURN_ROOT/releases"
CURRENT_LINK="$SATURN_ROOT/current"
TRANSACTION_FILE="$TMP_ROOT/state/deployments/current.json"
LOCK_FILE="$TMP_ROOT/run/saturn-release-activate.lock"
SYSTEMD_ROOT="$TMP_ROOT/systemd"
CONFIG_FILE="$TMP_ROOT/activate.conf"
FAKE_BIN="$TMP_ROOT/bin"
SYSTEMCTL_LOG="$TMP_ROOT/systemctl.log"
SYSTEMCTL_STATE="$TMP_ROOT/service-state"
OLD_COMMIT="1111111111111111111111111111111111111111"
NEW_COMMIT="2222222222222222222222222222222222222222"
BAD_COMMIT="3333333333333333333333333333333333333333"
STARTUP_COMMIT="4444444444444444444444444444444444444444"
CONFIG_COMMIT="5555555555555555555555555555555555555555"
WRONG_COMMIT="6666666666666666666666666666666666666666"
ROLLBACK_FAIL_COMMIT="7777777777777777777777777777777777777777"
MIGRATION_FAIL_COMMIT="8888888888888888888888888888888888888888"
OWNER_FAIL_COMMIT="9999999999999999999999999999999999999999"
P2_COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
P2_CLIENT_COMMIT="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
P2_CLIENT_FAIL_COMMIT="cccccccccccccccccccccccccccccccccccccccc"
PAGE_FAIL_COMMIT="dddddddddddddddddddddddddddddddddddddddd"

cleanup(){ rm -rf -- "$TMP_ROOT"; }
trap cleanup EXIT

create_release(){
  local commit="$1" release
  release="$RELEASES_ROOT/$commit"
  mkdir -p "$release/share/release"
  install -m 0644 "$COMPONENTS" "$release/share/release/components-v1.json"
  while IFS=$'\t' read -r relative executable; do
    mkdir -p "$release/$(dirname "$relative")"
    printf 'fixture %s for %s\n' "$relative" "$commit" >"$release/$relative"
    if [[ "$executable" == "true" ]]; then
      chmod 0755 "$release/$relative"
    else
      chmod 0644 "$release/$relative"
    fi
  done < <(python3 - "$COMPONENTS" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    descriptor = json.load(handle)
for component in descriptor["components"]:
    print(f'{component["path"]}\t{str(bool(component.get("executable"))).lower()}')
PY
  )
  find "$release" -type d -exec chmod 0755 {} +
  local -a args=(
    create
    --release-root "$release"
    --repo-root "$REPO_ROOT"
    --components "$COMPONENTS"
    --commit "$commit"
    --repository fixture://saturn
    --requested-ref fixture
    --resolved-ref refs/heads/fixture
    --created-at 2026-07-20T12:00:00Z
  )
  while IFS= read -r result; do
    args+=(--build-result "$result")
  done < <(python3 - "$COMPONENTS" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    descriptor = json.load(handle)
for result in descriptor["required_build_results"]:
    print(result)
PY
  )
  python3 "$MANIFEST_TOOL" "${args[@]}" >/dev/null
}

write_config(){
  local enabled="$1"
  cat >"$CONFIG_FILE" <<EOF
ACTIVATION_ENABLED="$enabled"
SATURN_ROOT="$SATURN_ROOT"
RELEASES_ROOT="$RELEASES_ROOT"
CURRENT_LINK="$CURRENT_LINK"
TRANSACTION_FILE="$TRANSACTION_FILE"
LOCK_FILE="$LOCK_FILE"
MANIFEST_TOOL="$MANIFEST_TOOL"
COMPONENTS_FILE="$COMPONENTS"
STATE_TOOL="$STATE_TOOL"
STATE_ROOT="$TMP_ROOT/state"
STATE_BACKUP_ROOT="$TMP_ROOT/state/deployments/state-backups"
SYSTEMD_ROOT="$SYSTEMD_ROOT"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
SATURN_GO_READY_URL="http://127.0.0.1:18080/readyz"
P23_PERF_URL="http://127.0.0.1:18080/p23_perf"
BACKEND_STATUS_HELPER="$FAKE_BIN/backend-status"
READY_TIMEOUT_SECONDS="2"
P2APP_PANEL_ENABLED="0"
TRANSACTION_GROUP="$(id -gn)"
VALIDATION_ENABLED="${SATURN_TEST_VALIDATION_ENABLED:-0}"
VALIDATOR="$FAKE_BIN/fake-validator"
VALIDATOR_GUARD="$TMP_ROOT/receive-guard.js"
VALIDATION_LAUNCHER="direct"
VALIDATION_BROWSER="$FAKE_BIN/fake-browser"
VALIDATION_TLS_CERT="$TMP_ROOT/remote.crt"
VALIDATION_PERF_JSON="$TMP_ROOT/perf.json"
VALIDATION_LOG_ROOT="$TMP_ROOT/state/deployments/validation"
VALIDATION_TIMEOUT_SECONDS="20"
VALIDATION_EXPECT_RXC1="${SATURN_TEST_EXPECT_RXC1:-any}"
EOF
  chmod 0644 "$CONFIG_FILE"
}

mkdir -p "$RELEASES_ROOT" "$FAKE_BIN" "$(dirname "$TRANSACTION_FILE")"
chmod 0755 "$SATURN_ROOT" "$RELEASES_ROOT" "$FAKE_BIN"
create_release "$OLD_COMMIT"
create_release "$NEW_COMMIT"
create_release "$BAD_COMMIT"
create_release "$STARTUP_COMMIT"
create_release "$CONFIG_COMMIT"
create_release "$WRONG_COMMIT"
create_release "$ROLLBACK_FAIL_COMMIT"
create_release "$MIGRATION_FAIL_COMMIT"
create_release "$OWNER_FAIL_COMMIT"
create_release "$P2_COMMIT"
create_release "$P2_CLIENT_COMMIT"
create_release "$P2_CLIENT_FAIL_COMMIT"
create_release "$PAGE_FAIL_COMMIT"

mkdir -p "$SYSTEMCTL_STATE"
: >"$SYSTEMCTL_STATE/saturn-go.service"
: >"$SYSTEMCTL_STATE/saturn-bridge.service"

cat >"$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >>"$SATURN_TEST_SYSTEMCTL_LOG"
if [[ -n "${SATURN_TEST_SYSTEMCTL_FAIL_ONCE:-}" \
      && "$*" == "$SATURN_TEST_SYSTEMCTL_FAIL_ONCE" \
      && ! -e "$SATURN_TEST_SYSTEMCTL_FAIL_MARKER" ]]; then
  : >"$SATURN_TEST_SYSTEMCTL_FAIL_MARKER"
  exit 1
fi
case "$1" in
  is-active) [[ -f "$SATURN_TEST_SYSTEMCTL_STATE/$3" ]] ;;
  start)
    : >"$SATURN_TEST_SYSTEMCTL_STATE/$2"
    if [[ "$2" == "${SATURN_TEST_COACTIVATE_ONCE:-}" \
          && ! -e "$SATURN_TEST_COACTIVATE_MARKER" ]]; then
      : >"$SATURN_TEST_COACTIVATE_MARKER"
      : >"$SATURN_TEST_SYSTEMCTL_STATE/${SATURN_TEST_COACTIVATE_SERVICE:?}"
    fi
    ;;
  stop) rm -f -- "$SATURN_TEST_SYSTEMCTL_STATE/$2" ;;
esac
EOF
cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "$*" == *'/telemetry'* || "$*" == *'/settings'* ]]; then
  current="$(readlink -f "$SATURN_TEST_CURRENT_LINK" 2>/dev/null || true)"
  if [[ "$current" == *"/${SATURN_TEST_PAGE_UNAVAILABLE_COMMIT:-not-a-commit}" ]]; then exit 22; fi
  if [[ "$*" == *'/telemetry'* ]]; then
    printf '<title>Saturn G2 Radio Telemetry</title>\n'
  else
    printf '<title>Saturn Go Settings</title>\n'
  fi
  exit 0
fi
if [[ "$*" == *'/p23_perf'* ]]; then
  backend="${SATURN_TEST_SELECTED_BACKEND:-xdma}"
  app="saturn-bridge"
  [[ "$backend" == "p2" ]] && app="p2"
  active=true
  current="$(readlink -f "$SATURN_TEST_CURRENT_LINK" 2>/dev/null || true)"
  if [[ "$current" == *"/${SATURN_TEST_OWNER_UNAVAILABLE_COMMIT:-not-a-commit}" ]]; then active=false; fi
  if [[ "${SATURN_TEST_REQUIRE_CLIENT:-0}" == 1 ]]; then
    if [[ -e "${SATURN_TEST_CLIENT_MARKER:-/nonexistent}" ]]; then
      printf 'gate-with-client\n' >>"$SATURN_TEST_SYSTEMCTL_LOG"
    else
      active=false
    fi
  fi
  printf '{"perf":{"workload":{"selected_app":"%s","service_main_pid":123},"app_telemetry":{"snapshot_readable":true,"pid_matches_service":true,"age_seconds":0.1,"current":{"app":"%s","pid":123,"state":{"sdr_active":%s,"thread_error":false}}}}}\n' \
    "$backend" "$app" "$active"
  exit 0
fi
if [[ "$*" =~ expected_commit=([0-9a-f]{40}) ]]; then
  expected="${BASH_REMATCH[1]}"
  if [[ " ${SATURN_TEST_READY_FAIL_COMMITS:-} " == *" $expected "* ]]; then
    exit 22
  fi
  reported="$expected"
  if [[ "$expected" == "${SATURN_TEST_WRONG_TARGET_COMMIT:-}" ]]; then
    reported="$SATURN_TEST_WRONG_REPORTED_COMMIT"
  fi
  printf '{"status":"ready","ready":true,"build_commit":"%s","expected_commit":"%s"}\n' \
    "$reported" "$expected"
  exit 0
fi
printf '{"status":"ready","ready":true,"build_commit":"%s","expected_commit":"%s"}\n' \
  "$SATURN_TEST_RUNNING_COMMIT" "$SATURN_TEST_RUNNING_COMMIT"
EOF
cat >"$FAKE_BIN/backend-status" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ "${1:-}" == "status" ]] || exit 1
backend="${SATURN_TEST_SELECTED_BACKEND:-xdma}"
runtime="${SATURN_TEST_BRIDGE_RUNTIME:-$backend}"
p2=inactive
bridge=inactive
[[ -f "$SATURN_TEST_SYSTEMCTL_STATE/p2app.service" ]] && p2=active
[[ -f "$SATURN_TEST_SYSTEMCTL_STATE/saturn-bridge.service" ]] && bridge=active
status=stopped
if [[ "$backend" == p2 && "$runtime" == p2 && "$p2" == active ]] || \
   [[ "$backend" == xdma && "$runtime" == xdma && "$bridge" == active && "$p2" == inactive ]]; then status=ready; fi
mutual_exclusion=true
[[ "$runtime" == xdma && "$p2" == active ]] && mutual_exclusion=false
printf '{"selected":"%s","runtime":"%s","operational_status":"%s","transaction_status":"idle","mutual_exclusion_ok":%s,"services":{"p2app":"%s","saturn_bridge":"%s"}}\n' \
  "$backend" "$runtime" "$status" "$mutual_exclusion" "$p2" "$bridge"
EOF
chmod 0755 "$FAKE_BIN/systemctl" "$FAKE_BIN/curl" "$FAKE_BIN/backend-status"

export PATH="$FAKE_BIN:$PATH"
export SATURN_RELEASE_ACTIVATE_CONFIG="$CONFIG_FILE"
export SATURN_RELEASE_ACTIVATE_TEST_MODE=1
export SATURN_TEST_SYSTEMCTL_LOG="$SYSTEMCTL_LOG"
export SATURN_TEST_SYSTEMCTL_STATE="$SYSTEMCTL_STATE"
export SATURN_TEST_CURRENT_LINK="$CURRENT_LINK"
export SATURN_TEST_RUNNING_COMMIT="$OLD_COMMIT"
export SATURN_TEST_SYSTEMCTL_FAIL_MARKER="$TMP_ROOT/systemctl-failed-once"
export SATURN_TEST_COACTIVATE_MARKER="$TMP_ROOT/systemctl-coactivated-once"

# Production installation carries the root-owned helper but keeps activation
# disabled and does not grant the web-service account passwordless access.
# These are intentionally literal installer expressions.
# shellcheck disable=SC2016
grep -Fq 'SATURN_RELEASE_ACTIVATION_ENABLED="${SATURN_RELEASE_ACTIVATION_ENABLED:-0}"' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
# shellcheck disable=SC2016
grep -Fq 'ACTIVATION_ENABLED="$SATURN_RELEASE_ACTIVATION_ENABLED"' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
# shellcheck disable=SC2016
grep -Fq '"$SOURCE_DIR/scripts/$SATURN_RELEASE_ACTIVATOR_NAME"' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
# shellcheck disable=SC2016
grep -Fq '"$SOURCE_DIR/scripts/$SATURN_STATE_COMPATIBILITY_TOOL_NAME"' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
if grep -Eq 'NOPASSWD:.*saturn-release-activate-root' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"; then
  printf 'activation broker unexpectedly exposed through sudoers\n' >&2
  exit 1
fi
if grep -Eq 'NOPASSWD:.*saturn-state-compatibility' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"; then
  printf 'state migration helper unexpectedly exposed through sudoers\n' >&2
  exit 1
fi

write_config 0
"$ACTIVATOR" --validate "$NEW_COMMIT" >/dev/null

# A failed manifest check must remain a failure when validate_release runs in
# command substitution, both in --validate and before an active transaction.
printf 'tampered payload\n' >>"$RELEASES_ROOT/$BAD_COMMIT/bin/p2app"
if "$ACTIVATOR" --validate "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'tampered release unexpectedly passed validation\n' >&2
  exit 1
fi
write_config 1
: >"$SYSTEMCTL_LOG"
if "$ACTIVATOR" "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'tampered release unexpectedly activated\n' >&2
  exit 1
fi
if grep -Eq '^(start|stop) ' "$SYSTEMCTL_LOG"; then
  printf 'tampered release changed service state\n' >&2
  exit 1
fi
[[ ! -e "$TRANSACTION_FILE" && ! -e "$CURRENT_LINK" ]]
create_release "$BAD_COMMIT"

# All other mandatory guards must fail closed as well.
chmod 0666 "$RELEASES_ROOT/$BAD_COMMIT/bin/p2app"
if "$ACTIVATOR" --validate "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'unsafe release permissions unexpectedly passed validation\n' >&2
  exit 1
fi
create_release "$BAD_COMMIT"
ln -s /etc/passwd "$RELEASES_ROOT/$BAD_COMMIT/unsafe-link"
if "$ACTIVATOR" --validate "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'symlinked release entry unexpectedly passed validation\n' >&2
  exit 1
fi
rm "$RELEASES_ROOT/$BAD_COMMIT/unsafe-link"
sed -i "s/\"architecture\": \"$(uname -m)\"/\"architecture\": \"wrong-architecture\"/" \
  "$RELEASES_ROOT/$BAD_COMMIT/release-manifest.json"
if "$ACTIVATOR" --validate "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'wrong release architecture unexpectedly passed validation\n' >&2
  exit 1
fi
create_release "$BAD_COMMIT"
sed -i "s/$BAD_COMMIT/$OLD_COMMIT/" "$RELEASES_ROOT/$BAD_COMMIT/release-manifest.json"
if "$ACTIVATOR" --validate "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'wrong release commit unexpectedly passed validation\n' >&2
  exit 1
fi
create_release "$BAD_COMMIT"

# The identity producer must finish before the helper drops the lock. The
# delayed wrapper emits the identity line, then holds inherited FD 9 briefly.
cat >"$FAKE_BIN/python3" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "${1:-}" == "-" && "${2:-}" == */release-manifest.json ]]; then
  if [[ "${SATURN_TEST_IDENTITY_MODE:-}" == fail ]]; then exit 42; fi
  "$SATURN_TEST_REAL_PYTHON" "$@"
  if [[ "${SATURN_TEST_IDENTITY_MODE:-}" == delay ]]; then
    : >"$SATURN_TEST_IDENTITY_MARKER"
    sleep 2
  fi
  exit 0
fi
exec "$SATURN_TEST_REAL_PYTHON" "$@"
EOF
chmod 0755 "$FAKE_BIN/python3"
export SATURN_TEST_REAL_PYTHON="$REAL_PYTHON"
export SATURN_TEST_IDENTITY_MARKER="$TMP_ROOT/identity-delayed"
export SATURN_TEST_IDENTITY_MODE=fail
if "$ACTIVATOR" --validate "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'failed identity producer unexpectedly passed validation\n' >&2
  exit 1
fi
export SATURN_TEST_IDENTITY_MODE=delay
"$ACTIVATOR" --validate "$NEW_COMMIT" >/dev/null
[[ -f "$SATURN_TEST_IDENTITY_MARKER" ]]
unset SATURN_TEST_IDENTITY_MODE
rm "$FAKE_BIN/python3"
write_config 0

install -d -m 0755 "$(dirname "$LOCK_FILE")"
exec 8>"$LOCK_FILE"
flock -n 8
if "$ACTIVATOR" --validate "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'concurrent activation lock unexpectedly allowed a second transaction\n' >&2
  exit 1
fi
exec 8>&-
if "$ACTIVATOR" "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'disabled production activation unexpectedly succeeded\n' >&2
  exit 1
fi
[[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]
[[ ! -e "$TRANSACTION_FILE" ]]

write_config 1
printf '{"activeProfile":"operator","theme":"dark"}\n' \
  >"$TMP_ROOT/state/remote_settings.json"
cp "$TMP_ROOT/state/remote_settings.json" "$TMP_ROOT/remote-settings.expected"
ln -s /etc/passwd "$TRANSACTION_FILE"
if "$ACTIVATOR" "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'symlinked transaction state unexpectedly accepted\n' >&2
  exit 1
fi
rm -f "$TRANSACTION_FILE"
[[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]

# A migration failure occurs after the prior services are stopped, but before
# service wiring or pointer activation. Automatic rollback restarts the legacy
# services and leaves both settings and schema marker unchanged.
ln -s /etc/passwd "$TMP_ROOT/state/remote_profiles.json"
if "$ACTIVATOR" "$MIGRATION_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'failed state migration unexpectedly activated a release\n' >&2
  exit 1
fi
rm "$TMP_ROOT/state/remote_profiles.json"
[[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]
[[ ! -e "$TMP_ROOT/state/state-schema.json" ]]
cmp "$TMP_ROOT/remote-settings.expected" "$TMP_ROOT/state/remote_settings.json"
python3 - "$TRANSACTION_FILE" "$MIGRATION_FAIL_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == sys.argv[2]
assert value["activation_failure"]["phase"] == "state-migration"
assert value["rollback"]["status"] == "succeeded"
PY

# A failed first activation restores the legacy no-pointer deployment and
# removes all newly introduced systemd drop-ins.
export SATURN_TEST_READY_FAIL_COMMITS="$BAD_COMMIT"
if "$ACTIVATOR" "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'failed first activation unexpectedly succeeded\n' >&2
  exit 1
fi
unset SATURN_TEST_READY_FAIL_COMMITS
[[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]
[[ ! -e "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf" ]]
[[ ! -e "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf" ]]
[[ ! -e "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf" ]]
[[ ! -e "$TMP_ROOT/state/state-schema.json" ]]
cmp "$TMP_ROOT/remote-settings.expected" "$TMP_ROOT/state/remote_settings.json"
python3 - "$TRANSACTION_FILE" "$BAD_COMMIT" "$OLD_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == sys.argv[2]
assert value["previous_commit"] is None
assert value["previous_ready_commit"] == sys.argv[3]
assert value["activation_failure"]["phase"] == "readiness"
assert value["activation_failure"]["exit_status"] != 0
assert value["rollback"]["status"] == "succeeded"
assert value["state_compatibility"]["migrated"] is True
assert value["state_compatibility"]["backup_directory"]
PY

"$ACTIVATOR" "$NEW_COMMIT" >/dev/null
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ -z "$(find "$SATURN_ROOT" -maxdepth 1 -name '.current.*' -print -quit)" ]]
[[ "$(stat -c '%a' "$TRANSACTION_FILE")" == "640" ]]
[[ "$(stat -c '%a' "$TRANSACTION_FILE.last-good")" == "640" ]]
cmp "$TRANSACTION_FILE" "$TRANSACTION_FILE.last-good"

# A damaged active rollback release must be rejected before touching the
# selected owner, pointer, drop-ins, or transaction record.
cp "$TRANSACTION_FILE" "$TMP_ROOT/transaction.before-corrupt-rollback"
printf 'tampered active payload\n' >>"$RELEASES_ROOT/$NEW_COMMIT/bin/p2app"
: >"$SYSTEMCTL_LOG"
if "$ACTIVATOR" "$STARTUP_COMMIT" >/dev/null 2>&1; then
  printf 'damaged rollback release unexpectedly allowed activation\n' >&2
  exit 1
fi
if grep -Eq '^(start|stop) ' "$SYSTEMCTL_LOG"; then
  printf 'damaged rollback release changed service state\n' >&2
  exit 1
fi
cmp "$TMP_ROOT/transaction.before-corrupt-rollback" "$TRANSACTION_FILE"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
create_release "$NEW_COMMIT"

python3 - "$TRANSACTION_FILE" "$OLD_COMMIT" "$NEW_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["format"] == "saturn-deployment-transaction"
assert value["schema_version"] == 1
assert value["status"] == "committed"
assert value["phase"] == "commit"
assert value["previous_commit"] is None
assert value["previous_ready_commit"] == sys.argv[2]
assert value["target_commit"] == sys.argv[3]
assert value["services"]["stop_order"] == [
    "saturn-go.service", "saturn-bridge.service", "p2app.service"
]
assert value["services"]["start_order"] == [
    "saturn-bridge.service", "saturn-go.service"
]
assert value["services"]["selected_backend"] == "xdma"
assert all(
    not item["previously_existed"]
    for item in value["service_dropins"].values()
)
state = value["state_compatibility"]
assert state["migrated"] is True
assert state["backup_directory"]
assert state["plan"]["current_state_schema_version"] == 0
assert state["plan"]["target_state_schema_version"] == 1
assert state["plan"]["rollback_safe"] is True
PY
python3 - "$TMP_ROOT/state/state-schema.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["state_schema_version"] == 1
PY

grep -Fq "ExecStart=$CURRENT_LINK/bin/saturn-go" \
  "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
grep -Fq "Environment=SATURN_WEBROOT=$CURRENT_LINK/webroot" \
  "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
grep -Fq "ExecStart=$CURRENT_LINK/bin/saturn-bridge" \
  "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
grep -Fq "ExecStart=$CURRENT_LINK/bin/p2app -s" \
  "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
cp "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf" "$TMP_ROOT/saturn-go.expected"
cp "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf" "$TMP_ROOT/saturn-bridge.expected"
cp "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf" "$TMP_ROOT/p2app.expected"

if grep -Fq 'start p2app.service' "$SYSTEMCTL_LOG"; then
  printf 'direct-XDMA activation started P2 unexpectedly\n' >&2
  exit 1
fi

# Readiness must exercise the actual pages, not only the JSON health endpoint.
export SATURN_TEST_PAGE_UNAVAILABLE_COMMIT="$PAGE_FAIL_COMMIT"
if "$ACTIVATOR" "$PAGE_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'activation with unreadable web pages unexpectedly committed\n' >&2
  exit 1
fi
unset SATURN_TEST_PAGE_UNAVAILABLE_COMMIT
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "readiness"
assert value["rollback"]["status"] == "succeeded"
PY

# If P2 appears while starting the direct-XDMA Bridge, fail before commit and
# roll back to the original exclusive service set.
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
export SATURN_TEST_COACTIVATE_ONCE=saturn-bridge.service
export SATURN_TEST_COACTIVATE_SERVICE=p2app.service
rm -f "$SATURN_TEST_COACTIVATE_MARKER"
if "$ACTIVATOR" "$STARTUP_COMMIT" >/dev/null 2>&1; then
  printf 'direct-XDMA activation with unexpected P2 unexpectedly committed\n' >&2
  exit 1
fi
unset SATURN_TEST_COACTIVATE_ONCE SATURN_TEST_COACTIVATE_SERVICE
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ -f "$SYSTEMCTL_STATE/saturn-bridge.service" && ! -f "$SYSTEMCTL_STATE/p2app.service" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "service-start"
assert value["rollback"]["status"] == "succeeded"
PY

# A running Bridge without selected-owner reception must not commit.
export SATURN_TEST_OWNER_UNAVAILABLE_COMMIT="$OWNER_FAIL_COMMIT"
if "$ACTIVATOR" "$OWNER_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'unavailable Bridge receive unexpectedly committed activation\n' >&2
  exit 1
fi
unset SATURN_TEST_OWNER_UNAVAILABLE_COMMIT
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["services"]["selected_backend"] == "xdma"
assert value["activation_failure"]["phase"] == "readiness"
PY

# A target service startup failure returns to the verified prior release.
export SATURN_TEST_SYSTEMCTL_FAIL_ONCE="start saturn-bridge.service"
rm -f "$SATURN_TEST_SYSTEMCTL_FAIL_MARKER"
if "$ACTIVATOR" "$STARTUP_COMMIT" >/dev/null 2>&1; then
  printf 'bridge startup failure unexpectedly committed activation\n' >&2
  exit 1
fi
unset SATURN_TEST_SYSTEMCTL_FAIL_ONCE
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
cmp "$TMP_ROOT/saturn-go.expected" "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/p2app.expected" "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
python3 - "$TRANSACTION_FILE" "$STARTUP_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == sys.argv[2]
assert value["activation_failure"]["phase"] == "service-start"
assert value["activation_failure"]["exit_status"] != 0
assert value["rollback"]["status"] == "succeeded"
PY

# Invalid generated service configuration (represented by daemon-reload
# failure) restores the prior drop-ins before any target pointer is committed.
export SATURN_TEST_SYSTEMCTL_FAIL_ONCE="daemon-reload"
rm -f "$SATURN_TEST_SYSTEMCTL_FAIL_MARKER"
if "$ACTIVATOR" "$CONFIG_COMMIT" >/dev/null 2>&1; then
  printf 'invalid service configuration unexpectedly committed activation\n' >&2
  exit 1
fi
unset SATURN_TEST_SYSTEMCTL_FAIL_ONCE
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
cmp "$TMP_ROOT/saturn-go.expected" "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/p2app.expected" "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
python3 - "$TRANSACTION_FILE" "$CONFIG_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == sys.argv[2]
assert value["activation_failure"]["phase"] == "service-wiring"
assert value["rollback"]["status"] == "succeeded"
PY

# A 200 response carrying the wrong commit is not accepted as readiness.
export SATURN_TEST_WRONG_TARGET_COMMIT="$WRONG_COMMIT"
export SATURN_TEST_WRONG_REPORTED_COMMIT="$OLD_COMMIT"
if "$ACTIVATOR" "$WRONG_COMMIT" >/dev/null 2>&1; then
  printf 'wrong-commit readiness unexpectedly committed activation\n' >&2
  exit 1
fi
unset SATURN_TEST_WRONG_TARGET_COMMIT SATURN_TEST_WRONG_REPORTED_COMMIT
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
cmp "$TMP_ROOT/saturn-go.expected" "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/p2app.expected" "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
python3 - "$TRANSACTION_FILE" "$WRONG_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == sys.argv[2]
assert value["activation_failure"]["phase"] == "readiness"
assert value["rollback"]["status"] == "succeeded"
PY

# =====================================================================================================================
# Automatic headless receive validation.
#
# Nobody has a browser open anywhere in this section: with SATURN_TEST_REQUIRE_CLIENT=1 the fake owner reports no
# receive client (sdr_active false) unless the validation session has attached one. The activation that failed on the
# appliance failed exactly this way. The updater must establish and verify reception itself, in activation and again
# after a rollback, and report "release restored" separately from "receive verified".
# =====================================================================================================================
cp "$REPO_ROOT/tests/fake-saturn-remote-validate.py" "$FAKE_BIN/fake-validator"
chmod 0755 "$FAKE_BIN/fake-validator"
printf '// receive-only guard stand-in\n' >"$TMP_ROOT/receive-guard.js"
: >"$TMP_ROOT/remote.crt"
printf '{}\n' >"$TMP_ROOT/perf.json"
printf '#!/bin/sh\n' >"$FAKE_BIN/fake-browser"
chmod 0755 "$FAKE_BIN/fake-browser"
export SATURN_TEST_CLIENT_MARKER="$TMP_ROOT/client-attached"
export SATURN_TEST_VALIDATOR_ARGS_LOG="$TMP_ROOT/validator-args.log"
export SATURN_TEST_REQUIRE_CLIENT=1
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
txn_snapshot(){ cp -a "$TRANSACTION_FILE" "$TMP_ROOT/transaction.snapshot"; }
expect_failure(){
  local expected="$1"; shift
  local rc=0
  "$@" >"$TMP_ROOT/last-run.log" 2>&1 || rc=$?
  if [[ "$expected" == "nonzero" ]]; then
    [[ "$rc" != 0 ]] || { printf 'expected a failure but the command succeeded: %s\n' "$*" >&2; exit 1; }
  else
    [[ "$rc" == "$expected" ]] || { printf 'expected exit %s, got %s: %s\n' "$expected" "$rc" "$*" >&2; cat "$TMP_ROOT/last-run.log" >&2; exit 1; }
  fi
}
assert_dropins_unchanged(){
  cmp "$TMP_ROOT/saturn-go.expected" "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
  cmp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
  cmp "$TMP_ROOT/p2app.expected" "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
}

# --- Control: this is the incident. Validation off, nobody connected. The release is restored, but reception cannot be proven.
# It used to be reported as "rollback_failed" (exit 70, activation blocked); it is now exactly what it is.
export SATURN_TEST_VALIDATION_ENABLED=0
write_config 1
expect_failure 71 "$ACTIVATOR" "$STARTUP_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
assert_dropins_unchanged
[[ ! -s "$SATURN_TEST_VALIDATOR_ARGS_LOG" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back", value["status"]
assert value["activation_failure"]["phase"] == "readiness"
assert value["rollback"]["status"] == "succeeded"
assert value["rollback"]["release_restored"] is True
assert value["rollback"]["receive_verified"] is False
assert "NOT verified" in value["rollback"]["message"]
assert "validation" not in value
PY

# --- Success with every human browser closed.
export SATURN_TEST_VALIDATION_ENABLED=1 SATURN_TEST_EXPECT_RXC1=unarmed
write_config 1
: >"$SYSTEMCTL_LOG"
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
"$ACTIVATOR" "$STARTUP_COMMIT" >"$TMP_ROOT/last-run.log"
export SATURN_TEST_RUNNING_COMMIT="$STARTUP_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$STARTUP_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
python3 - "$TRANSACTION_FILE" "$STARTUP_COMMIT" "$SATURN_TEST_VALIDATOR_ARGS_LOG" "$SYSTEMCTL_LOG" "$TMP_ROOT/receive-guard.js" <<'PY'
import json
import sys
record, commit, args_log, shared_log, guard = sys.argv[1:]
with open(record, encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "committed" and value["phase"] == "commit"
target = value["validation"]["target"]
assert target["final_state"] == "done" and target["exit_code"] == 0 and target["failed"] is None
assert target["driver"] == "native" and target["ready_at"] and target["finished_at"]
assert "role" in target and "role_enforced_by_bridge" in target
assert all(target["conditions"].values()) and "data.audio_frames_arriving" in target["conditions"]
assert "rollback" not in value
with open(args_log, encoding="utf-8") as handle:
    runs = [json.loads(line) for line in handle if line.strip()]
assert len(runs) == 1, runs
argv = runs[0]
def option(flag):
    return argv[argv.index(flag) + 1]
assert argv[0] == "run"
assert option("--target-commit") == commit and option("--backend") == "xdma"
assert option("--expect-rxc1") == "unarmed" and option("--launcher") == "direct"
assert option("--viewer-role") == "preferred"      # the default: a Bridge that predates viewer enforcement must still validate
assert option("--guard-script") == guard
assert option("--ready-url").endswith("/readyz") and "?" not in option("--ready-url")
assert option("--owner-unit") == "saturn-bridge.service" and option("--auth-unit") == "saturn-go.service"
assert argv.count("--unit") == 2
assert "--no-exe-check" not in argv and "--no-sandbox" not in argv
with open(shared_log, encoding="utf-8") as handle:
    lines = [line.strip() for line in handle if line.strip()]
def first(prefix, after=-1):
    return next(i for i, line in enumerate(lines) if i > after and line.startswith(prefix))
started = max(i for i, line in enumerate(lines) if line == "start saturn-go.service")
ran = first("validator-run", started)
ready = first("validator-ready", ran)
gate = first("gate-with-client", ready)
stopped = first("validator-stopped", gate)
assert started < ran < ready < gate < stopped, lines
PY
# return to the original release (also without a person)
"$ACTIVATOR" "$NEW_COMMIT" >"$TMP_ROOT/last-run.log"
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]

# --- Failed media delivery: the session attaches (so the owner's own gate would pass) but the target never delivers decoded audio.
# Only the validator's verdict can stop the commit. The failed condition is kept exactly; the previous release is restored and its
# reception is proven by the same validation.
export SATURN_TEST_VALIDATOR_STALL_COMMITS="$STARTUP_COMMIT"
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
assert_dropins_unchanged
python3 - "$TRANSACTION_FILE" "$STARTUP_COMMIT" "$NEW_COMMIT" "$SATURN_TEST_VALIDATOR_ARGS_LOG" <<'PY'
import json
import sys
record, target, previous, args_log = sys.argv[1:]
with open(record, encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["target_commit"] == target
assert value["activation_failure"]["phase"] == "receive-validation"
validation = value["validation"]
assert validation["target"]["final_state"] == "failed"
assert validation["target"]["failed"]["id"] == "audio.decoded_progress"
assert "advanced by 0" in validation["target"]["failed"]["detail"]
assert validation["rollback"]["final_state"] == "done" and validation["rollback"]["failed"] is None
assert value["rollback"]["status"] == "succeeded"
assert value["rollback"]["release_restored"] is True
assert value["rollback"]["receive_verified"] is True
assert "NOT verified" not in value["rollback"]["message"]
with open(args_log, encoding="utf-8") as handle:
    runs = [json.loads(line) for line in handle if line.strip()]
assert [run[run.index("--target-commit") + 1] for run in runs] == [target, previous], runs
assert "--no-exe-check" not in runs[1]   # the previous release is a release directory
assert runs[1][runs[1].index("--expect-rxc1") + 1] == "any"      # the intended RXC1 state belongs to the target only
PY

# --- The restored release also cannot be proven to receive: reported as such (exit 71), not as a failed restoration.
unset SATURN_TEST_VALIDATOR_STALL_COMMITS
export SATURN_TEST_VALIDATOR_FAIL_COMMITS="$STARTUP_COMMIT $NEW_COMMIT"
expect_failure 71 "$ACTIVATOR" "$STARTUP_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
assert_dropins_unchanged
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["rollback"]["status"] == "succeeded"
assert value["rollback"]["release_restored"] is True
assert value["rollback"]["receive_verified"] is False
assert value["validation"]["rollback"]["failed"]["id"] == "data.audio_frames_arriving"
assert value["validation"]["target"]["failed"]["id"] == "data.audio_frames_arriving"
PY

# --- A session that cannot be cleaned up fails the activation too: nothing may be left attached to the radio.
export SATURN_TEST_VALIDATOR_FAIL_COMMITS=""
export SATURN_TEST_VALIDATOR_STOP_FAIL_COMMITS="$STARTUP_COMMIT"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
unset SATURN_TEST_VALIDATOR_STOP_FAIL_COMMITS
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "receive-validation"
assert value["validation"]["target"]["failed"]["id"] == "cleanup.client_released"
assert value["rollback"]["receive_verified"] is True
PY

# --- A validation that never becomes ready is bounded by VALIDATION_TIMEOUT_SECONDS, ended, and rolled back.
export SATURN_TEST_VALIDATOR_HANG_COMMITS="$STARTUP_COMMIT"
sed -i 's/^VALIDATION_TIMEOUT_SECONDS=.*/VALIDATION_TIMEOUT_SECONDS="2"/' "$CONFIG_FILE"
started_at=$SECONDS
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
(( SECONDS - started_at < 60 ))
unset SATURN_TEST_VALIDATOR_HANG_COMMITS
write_config 1
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "receive-validation"
assert value["validation"]["target"]["final_state"] == "failed"
assert value["rollback"]["receive_verified"] is True
PY

# --- Missing prerequisites refuse the activation before anything is touched.
txn_snapshot
mv "$FAKE_BIN/fake-validator" "$FAKE_BIN/fake-validator.away"
: >"$SYSTEMCTL_LOG"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
grep -Fq 'receive validator is not installed' "$TMP_ROOT/last-run.log"
mv "$FAKE_BIN/fake-validator.away" "$FAKE_BIN/fake-validator"
mv "$FAKE_BIN/fake-browser" "$FAKE_BIN/fake-browser.away"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
grep -Fq 'validation browser is not executable' "$TMP_ROOT/last-run.log"
mv "$FAKE_BIN/fake-browser.away" "$FAKE_BIN/fake-browser"
mv "$TMP_ROOT/remote.crt" "$TMP_ROOT/remote.crt.away"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
grep -Fq 'remote proxy certificate is not readable' "$TMP_ROOT/last-run.log"
mv "$TMP_ROOT/remote.crt.away" "$TMP_ROOT/remote.crt"
if grep -Eq '^(start|stop) ' "$SYSTEMCTL_LOG"; then
  printf 'a missing validation prerequisite changed service state\n' >&2
  exit 1
fi
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
# bad validation policy is refused as well
sed -i 's/^VALIDATION_EXPECT_RXC1=.*/VALIDATION_EXPECT_RXC1="maybe"/' "$CONFIG_FILE"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
grep -Fq 'VALIDATION_EXPECT_RXC1' "$TMP_ROOT/last-run.log"
write_config 1
printf 'VALIDATION_VIEWER_ROLE="whenever"\n' >>"$CONFIG_FILE"
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"
grep -Fq 'VALIDATION_VIEWER_ROLE must be required or preferred' "$TMP_ROOT/last-run.log"
write_config 1

# --- An interrupted helper must not leave a validation session attached to the radio.
export SATURN_TEST_VALIDATOR_HANG_COMMITS="$STARTUP_COMMIT"
sed -i 's/^VALIDATION_TIMEOUT_SECONDS=.*/VALIDATION_TIMEOUT_SECONDS="120"/' "$CONFIG_FILE"
: >"$SYSTEMCTL_LOG"
"$ACTIVATOR" "$STARTUP_COMMIT" >"$TMP_ROOT/term-run.log" 2>&1 &
helper_pid=$!
for _ in $(seq 1 100); do
  grep -q "^validator-run $STARTUP_COMMIT" "$SYSTEMCTL_LOG" && break
  sleep 0.1
done
grep -q "^validator-run $STARTUP_COMMIT" "$SYSTEMCTL_LOG"
term_started=$SECONDS
kill -TERM "$helper_pid"
term_rc=0
wait "$helper_pid" || term_rc=$?
[[ "$term_rc" == 143 ]]
# the validation wait was 120 s: an interrupted helper must not sit through it
(( SECONDS - term_started < 30 ))
python3 - "$(ls -d "$TMP_ROOT"/state/deployments/validation/*-target-"${STARTUP_COMMIT:0:12}".* | tail -1)/status.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    status = json.load(handle)
assert status["state"] == "failed", status   # the fake validator was told to stop and finished
PY
[[ -z "$(pgrep -f "fake-validator run --control-dir $TMP_ROOT/" || true)" ]]
unset SATURN_TEST_VALIDATOR_HANG_COMMITS
# the interrupted activation is abandoned mid-way by design; put the fixture back as it was
rm -f -- "$TRANSACTION_FILE" "$TRANSACTION_FILE.last-good"
ln -sfn "$RELEASES_ROOT/$NEW_COMMIT" "$CURRENT_LINK"
cp "$TMP_ROOT/saturn-go.expected" "$SYSTEMD_ROOT/saturn-go.service.d/50-saturn-release.conf"
cp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
cp "$TMP_ROOT/p2app.expected" "$SYSTEMD_ROOT/p2app.service.d/50-saturn-release.conf"
: >"$SYSTEMCTL_STATE/saturn-go.service"
: >"$SYSTEMCTL_STATE/saturn-bridge.service"
rm -f -- "$SYSTEMCTL_STATE/p2app.service" "$SATURN_TEST_CLIENT_MARKER"
write_config 1

# --- A deployment that predates release directories has no release path to hold its binaries to: only its rollback validation says so.
(
  # shellcheck disable=SC1090
  source "$ACTIVATOR"
  VALIDATION_LOG_ROOT="$TMP_ROOT/legacy-validation"
  VALIDATOR="$FAKE_BIN/fake-validator"
  VALIDATOR_GUARD="$TMP_ROOT/receive-guard.js"
  SELECTED_BACKEND=xdma
  PREVIOUS_RELEASE=""
  : >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
  validation_start rollback "$OLD_COMMIT"
  validation_wait_ready >/dev/null
  validation_stop_record
  PREVIOUS_RELEASE="$RELEASES_ROOT/$NEW_COMMIT"
  validation_start rollback "$NEW_COMMIT"
  validation_wait_ready >/dev/null
  validation_stop_record
  validation_start target "$NEW_COMMIT"
  validation_wait_ready >/dev/null
  validation_stop_record
  # Two validations of the same commit with the same label in the same second must not share a control directory: the second would
  # read the first one's finished status instead of its own.
  date(){ printf '20990101T000000Z\n'; }
  export SATURN_TEST_VALIDATOR_FAIL_COMMITS="$NEW_COMMIT"
  validation_start reconcile "$NEW_COMMIT"
  first_dir="$VALIDATION_CONTROL_DIR"
  validation_wait_ready >/dev/null || true
  validation_stop_record || true
  unset SATURN_TEST_VALIDATOR_FAIL_COMMITS
  validation_start reconcile "$NEW_COMMIT"
  [[ "$VALIDATION_CONTROL_DIR" != "$first_dir" ]]
  validation_wait_ready >/dev/null
  validation_stop_record
  # The target is always a release directory, even when the deployment it replaces is a legacy one; a check of a legacy host is not.
  PREVIOUS_RELEASE=""
  validation_start target "$NEW_COMMIT"
  validation_wait_ready >/dev/null
  validation_stop_record
  validation_start check "$NEW_COMMIT"
  validation_wait_ready >/dev/null
  validation_stop_record
) >/dev/null
python3 - "$SATURN_TEST_VALIDATOR_ARGS_LOG" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    runs = [json.loads(line) for line in handle if line.strip()]
assert [("--no-exe-check" in run) for run in runs] == [True, False, False, False, False, False, True], runs
PY

# --- The helper and the real validator agree on the command line. The fake validator above stands in for the program; this guards
# against the two drifting apart.
python3 - "$ACTIVATOR" "$REPO_ROOT/update_manager/scripts/saturn-remote-validate.py" <<'PY'
import re
import subprocess
import sys
helper = open(sys.argv[1], encoding="utf-8").read()
block = helper[helper.index("  args=(\n    run --control-dir"):]
block = block[:block.index("\n  )\n")]
used = set(re.findall(r"--[a-z0-9-]+", block)) | {"--no-sandbox", "--no-exe-check"}
for subcommand in ("run", "wait", "stop"):
    text = subprocess.run([sys.executable, sys.argv[2], subcommand, "--help"], capture_output=True, text=True, check=True).stdout
    offered = set(re.findall(r"--[a-z0-9-]+", text))
    if subcommand == "run":
        missing = used - offered
        assert not missing, f"the helper passes options the validator does not accept: {sorted(missing)}"
for flag in ("--control-dir", "--timeout"):
    for subcommand in ("wait", "stop"):
        text = subprocess.run([sys.executable, sys.argv[2], subcommand, "--help"], capture_output=True, text=True, check=True).stdout
        assert flag in text, (subcommand, flag)
PY

# --- Reconciling a failed rollback. A restoration that failed only for want of a receive client is the same situation as the one
# found on the appliance: everything is back, but the record says rollback_failed and blocks every update. The supported path clears it,
# and only when the previous release is verifiably running and receiving.
export SATURN_TEST_READY_FAIL_COMMITS="$ROLLBACK_FAIL_COMMIT $NEW_COMMIT"
expect_failure 70 "$ACTIVATOR" "$ROLLBACK_FAIL_COMMIT"
unset SATURN_TEST_READY_FAIL_COMMITS
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rollback_failed"
assert value["rollback"]["status"] == "failed"
assert value["rollback"]["release_restored"] is False
assert value["rollback"]["receive_verified"] == "not_attempted"
PY
expect_failure nonzero "$ACTIVATOR" "$STARTUP_COMMIT"      # a failed rollback still blocks another activation
grep -Fq "unresolved deployment transaction has status 'rollback_failed'" "$TMP_ROOT/last-run.log"
txn_snapshot
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"

# refusals leave the record exactly as it was
ln -sfn "$RELEASES_ROOT/$STARTUP_COMMIT" "$CURRENT_LINK"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'is not the recorded previous release' "$TMP_ROOT/last-run.log"
ln -sfn "$RELEASES_ROOT/$NEW_COMMIT" "$CURRENT_LINK"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

rm -f -- "$SYSTEMCTL_STATE/saturn-bridge.service"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'was active before the activation and is not now' "$TMP_ROOT/last-run.log"
: >"$SYSTEMCTL_STATE/saturn-bridge.service"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

printf '# tampered\n' >>"$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'differs from its rollback backup' "$TMP_ROOT/last-run.log"
cp "$TMP_ROOT/saturn-bridge.expected" "$SYSTEMD_ROOT/saturn-bridge.service.d/50-saturn-release.conf"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

export SATURN_TEST_RUNNING_COMMIT="$STARTUP_COMMIT"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'is not the recorded previous commit' "$TMP_ROOT/last-run.log"
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

export SATURN_TEST_VALIDATOR_FAIL_COMMITS="$NEW_COMMIT"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'reception on the previous release could not be proven' "$TMP_ROOT/last-run.log"
unset SATURN_TEST_VALIDATOR_FAIL_COMMITS
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]

# a previous release that no longer validates is not "running and receiving", whatever else holds
printf 'tampered previous payload\n' >>"$RELEASES_ROOT/$NEW_COMMIT/bin/p2app"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'the previous release failed validation' "$TMP_ROOT/last-run.log"
create_release "$NEW_COMMIT"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

# a service that was inactive before the activation and is active now
: >"$SYSTEMCTL_STATE/p2app.service"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback
grep -Fq 'was inactive before the activation and is active now' "$TMP_ROOT/last-run.log"
rm -f -- "$SYSTEMCTL_STATE/p2app.service"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

# no extra arguments, no other mode
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback "$NEW_COMMIT"
expect_failure nonzero "$ACTIVATOR" --validate --reconcile-rollback

# the supported path: verified, then the record is cleared, with the original preserved
"$ACTIVATOR" --reconcile-rollback >"$TMP_ROOT/last-run.log"
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
assert_dropins_unchanged
python3 - "$TRANSACTION_FILE" "$TMP_ROOT/transaction.snapshot" "$NEW_COMMIT" "$ROLLBACK_FAIL_COMMIT" <<'PY'
import json
import os
import sys
record, snapshot, previous, target = sys.argv[1:]
with open(record, encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back" and value["phase"] == "reconciled"
assert value["target_commit"] == target
assert value["rollback"]["status"] == "succeeded"
assert value["rollback"]["release_restored"] is True and value["rollback"]["receive_verified"] is True
assert value["validation"]["reconcile"]["final_state"] == "done"
assert value["activation_failure"]["phase"] == "readiness"           # what actually failed is not rewritten
original = value["reconciliation"]["original_record"]
assert os.path.basename(original).startswith("current.json.rollback-failed.")
with open(original, encoding="utf-8") as handle:
    kept = json.load(handle)
with open(snapshot, encoding="utf-8") as handle:
    before = json.load(handle)
assert kept == before and kept["status"] == "rollback_failed"       # preserved byte for byte
assert oct(os.stat(record).st_mode & 0o777) == "0o640"
PY
cmp "$TRANSACTION_FILE" "$TRANSACTION_FILE.last-good"
expect_failure nonzero "$ACTIVATOR" --reconcile-rollback            # nothing left to reconcile
grep -Fq "not rollback_failed" "$TMP_ROOT/last-run.log"
# ...and updates are possible again
"$ACTIVATOR" "$STARTUP_COMMIT" >"$TMP_ROOT/last-run.log"
export SATURN_TEST_RUNNING_COMMIT="$STARTUP_COMMIT"
"$ACTIVATOR" "$NEW_COMMIT" >"$TMP_ROOT/last-run.log"
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]

# --- --check-reception tries the production launcher out on the release that is running, and changes nothing. A host's browser sandbox,
# memory cap and credentials can be settled with it before any update is attempted.
txn_snapshot
: >"$SYSTEMCTL_LOG"
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
"$ACTIVATOR" --check-reception >"$TMP_ROOT/last-run.log"
grep -Fq "reception verified on $NEW_COMMIT" "$TMP_ROOT/last-run.log"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"
if grep -Eq '^(start|stop|daemon-reload)' "$SYSTEMCTL_LOG"; then
  printf 'a reception check changed service state\n' >&2
  exit 1
fi
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" && ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
assert_dropins_unchanged
python3 - "$SATURN_TEST_VALIDATOR_ARGS_LOG" "$NEW_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    runs = [json.loads(line) for line in handle if line.strip()]
assert len(runs) == 1 and runs[0][runs[0].index("--target-commit") + 1] == sys.argv[2], runs
assert "--no-exe-check" not in runs[0]
PY
ls -d "$TMP_ROOT"/state/deployments/validation/*-check-"${NEW_COMMIT:0:12}".* >/dev/null

export SATURN_TEST_VALIDATOR_STALL_COMMITS="$NEW_COMMIT"
expect_failure nonzero "$ACTIVATOR" --check-reception
unset SATURN_TEST_VALIDATOR_STALL_COMMITS
grep -Fq 'reception could not be proven' "$TMP_ROOT/last-run.log"
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"
[[ ! -e "$SATURN_TEST_CLIENT_MARKER" ]]
expect_failure nonzero "$ACTIVATOR" --check-reception "$NEW_COMMIT"
expect_failure nonzero "$ACTIVATOR" --check-reception --reconcile-rollback
expect_failure nonzero "$ACTIVATOR" --check-reception --validate
export SATURN_TEST_VALIDATION_ENABLED=0
write_config 1
expect_failure nonzero "$ACTIVATOR" --check-reception
grep -Fq 'disabled by root-owned policy' "$TMP_ROOT/last-run.log"
export SATURN_TEST_VALIDATION_ENABLED=1
write_config 1

# An interrupted check must not leave its session attached either.
export SATURN_TEST_VALIDATOR_HANG_COMMITS="$NEW_COMMIT"
sed -i 's/^VALIDATION_TIMEOUT_SECONDS=.*/VALIDATION_TIMEOUT_SECONDS="120"/' "$CONFIG_FILE"
: >"$SYSTEMCTL_LOG"
"$ACTIVATOR" --check-reception >"$TMP_ROOT/term-check.log" 2>&1 &
check_pid=$!
for _ in $(seq 1 100); do
  grep -q "^validator-run $NEW_COMMIT" "$SYSTEMCTL_LOG" && break
  sleep 0.1
done
grep -q "^validator-run $NEW_COMMIT" "$SYSTEMCTL_LOG"
check_term_started=$SECONDS
kill -TERM "$check_pid"
check_rc=0
wait "$check_pid" || check_rc=$?
[[ "$check_rc" == 143 ]]
(( SECONDS - check_term_started < 30 ))
python3 - "$(ls -d "$TMP_ROOT"/state/deployments/validation/*-check-"${NEW_COMMIT:0:12}".* | tail -1)/status.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    assert json.load(handle)["state"] == "failed"      # told to stop, and it finished
PY
[[ -z "$(pgrep -f "fake-validator run --control-dir $TMP_ROOT/" || true)" ]]
unset SATURN_TEST_VALIDATOR_HANG_COMMITS
write_config 1
cmp "$TMP_ROOT/transaction.snapshot" "$TRANSACTION_FILE"

# The launcher, the memory cap and the sandbox switch reach the validator; the systemd launcher needs systemd-run.
printf '#!/bin/sh\n' >"$FAKE_BIN/systemd-run"
chmod 0755 "$FAKE_BIN/systemd-run"
sed -i 's/^VALIDATION_LAUNCHER=.*/VALIDATION_LAUNCHER="systemd"/' "$CONFIG_FILE"
printf 'VALIDATION_MEMORY_MAX="123M"\nVALIDATION_NO_SANDBOX="1"\nVALIDATION_VIEWER_ROLE="required"\n' >>"$CONFIG_FILE"
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
"$ACTIVATOR" --check-reception >"$TMP_ROOT/last-run.log"
python3 - "$SATURN_TEST_VALIDATOR_ARGS_LOG" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    argv = json.loads(handle.readline())
assert argv[argv.index("--launcher") + 1] == "systemd"
assert argv[argv.index("--memory-max") + 1] == "123M"
assert "--no-sandbox" in argv
assert argv[argv.index("--viewer-role") + 1] == "required"
PY
rm "$FAKE_BIN/systemd-run"
# The systemd launcher cannot be used without systemd-run. (This machine may have a real one, so the PATH is restricted to python3.)
mkdir -p "$TMP_ROOT/minbin"
ln -sf "$(command -v python3)" "$TMP_ROOT/minbin/python3"
if (
  # shellcheck disable=SC1090
  source "$ACTIVATOR"
  VALIDATOR="$FAKE_BIN/fake-validator"
  VALIDATOR_GUARD="$TMP_ROOT/receive-guard.js"
  VALIDATION_BROWSER="$FAKE_BIN/fake-browser"
  VALIDATION_TLS_CERT="$TMP_ROOT/remote.crt"
  VALIDATION_PERF_JSON="$TMP_ROOT/perf.json"
  VALIDATION_LAUNCHER=systemd
  PATH="$TMP_ROOT/minbin"
  validation_preflight
) >"$TMP_ROOT/last-run.log" 2>&1; then
  printf 'the systemd launcher passed preflight without systemd-run\n' >&2
  exit 1
fi
grep -Fq 'required command not found: systemd-run' "$TMP_ROOT/last-run.log"
write_config 1

# Back to the fixture's earlier conditions for the remaining cases.
unset SATURN_TEST_VALIDATOR_FAIL_COMMITS SATURN_TEST_REQUIRE_CLIENT
export SATURN_TEST_VALIDATION_ENABLED=0
write_config 1

# A rollback verification failure is persisted distinctly and blocks another
# activation until an operator resolves the transaction.
export SATURN_TEST_READY_FAIL_COMMITS="$ROLLBACK_FAIL_COMMIT $NEW_COMMIT"
if "$ACTIVATOR" "$ROLLBACK_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'rollback verification failure unexpectedly succeeded\n' >&2
  exit 1
fi
unset SATURN_TEST_READY_FAIL_COMMITS
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
python3 - "$TRANSACTION_FILE" "$ROLLBACK_FAIL_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rollback_failed"
assert value["target_commit"] == sys.argv[2]
assert value["activation_failure"]["phase"] == "readiness"
assert value["rollback"]["status"] == "failed"
assert "did not fully restore" in value["rollback"]["message"]
PY

# Activation and rollback never prune the active or prior immutable releases.
[[ "$(find "$RELEASES_ROOT" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 13 ]]

# P2-selected activation must not initialize the direct-XDMA Bridge.
rm -f -- "$TRANSACTION_FILE" "$SYSTEMCTL_STATE/saturn-bridge.service"
: >"$SYSTEMCTL_STATE/p2app.service"
export SATURN_TEST_SELECTED_BACKEND=p2
# Receive validation supports the direct-XDMA owner only. With it enabled, a P2 activation says so and keeps its previous gate.
export SATURN_TEST_VALIDATION_ENABLED=1
write_config 1
: >"$SATURN_TEST_VALIDATOR_ARGS_LOG"
"$ACTIVATOR" "$P2_COMMIT" >"$TMP_ROOT/p2-run.log"
grep -Fq 'receive validation supports the direct-XDMA owner only' "$TMP_ROOT/p2-run.log"
[[ ! -s "$SATURN_TEST_VALIDATOR_ARGS_LOG" ]]
export SATURN_TEST_VALIDATION_ENABLED=0
write_config 1
[[ -f "$SYSTEMCTL_STATE/p2app.service" && ! -f "$SYSTEMCTL_STATE/saturn-bridge.service" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "committed"
assert value["services"]["selected_backend"] == "p2"
assert value["services"]["start_order"] == ["p2app.service", "saturn-go.service"]
PY

# A Bridge that unexpectedly starts during P2-only activation is a failure,
# even when it would otherwise report runtime=p2. Automatic rollback must
# restore the original P2-only service set.
export SATURN_TEST_RUNNING_COMMIT="$P2_COMMIT"
export SATURN_TEST_COACTIVATE_ONCE=p2app.service
export SATURN_TEST_COACTIVATE_SERVICE=saturn-bridge.service
rm -f "$SATURN_TEST_COACTIVATE_MARKER"
if "$ACTIVATOR" "$P2_CLIENT_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'P2-only activation with unexpected Bridge unexpectedly committed\n' >&2
  exit 1
fi
unset SATURN_TEST_COACTIVATE_ONCE SATURN_TEST_COACTIVATE_SERVICE
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$P2_COMMIT" ]]
[[ -f "$SYSTEMCTL_STATE/p2app.service" && ! -f "$SYSTEMCTL_STATE/saturn-bridge.service" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "service-start"
assert value["rollback"]["status"] == "succeeded"
PY

# P2 remains the sole FPGA owner when the Bridge is explicitly a P2 client.
# The client must be restarted after P2, before Saturn Go, and restored after
# a failed activation instead of silently being left down.
export SATURN_TEST_RUNNING_COMMIT="$P2_COMMIT"
: >"$SYSTEMCTL_STATE/saturn-bridge.service"
: >"$SYSTEMCTL_LOG"
"$ACTIVATOR" "$P2_CLIENT_COMMIT" >/dev/null
[[ -f "$SYSTEMCTL_STATE/p2app.service" && -f "$SYSTEMCTL_STATE/saturn-bridge.service" ]]
python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "committed"
assert value["services"]["selected_backend"] == "p2"
assert value["services"]["previously_active"]["p2app.service"] is True
assert value["services"]["previously_active"]["saturn-bridge.service"] is True
assert value["services"]["start_order"] == [
    "p2app.service", "saturn-bridge.service", "saturn-go.service"
]
PY
python3 - "$SYSTEMCTL_LOG" <<'PY'
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    calls = [line.strip() for line in handle]
starts = [line for line in calls if line.startswith("start ")]
assert starts == [
    "start p2app.service", "start saturn-bridge.service", "start saturn-go.service"
], starts
PY

export SATURN_TEST_RUNNING_COMMIT="$P2_CLIENT_COMMIT"
export SATURN_TEST_SYSTEMCTL_FAIL_ONCE="start saturn-bridge.service"
rm -f "$SATURN_TEST_SYSTEMCTL_FAIL_MARKER"
: >"$SYSTEMCTL_LOG"
if "$ACTIVATOR" "$P2_CLIENT_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'P2-client Bridge startup failure unexpectedly committed activation\n' >&2
  exit 1
fi
unset SATURN_TEST_SYSTEMCTL_FAIL_ONCE
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$P2_CLIENT_COMMIT" ]]
[[ -f "$SYSTEMCTL_STATE/p2app.service" && -f "$SYSTEMCTL_STATE/saturn-bridge.service" ]]
python3 - "$TRANSACTION_FILE" "$SYSTEMCTL_LOG" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "rolled_back"
assert value["activation_failure"]["phase"] == "service-start"
assert value["rollback"]["status"] == "succeeded"
with open(sys.argv[2], encoding="utf-8") as handle:
    starts = [line.strip() for line in handle if line.startswith("start ")]
assert starts == [
    "start p2app.service", "start saturn-bridge.service",
    "start p2app.service", "start saturn-bridge.service", "start saturn-go.service"
], starts
PY

# A concurrent P2/direct-XDMA combination is not a P2-client configuration.
# It must be rejected before service interruption, even if both are active.
rm -f -- "$TRANSACTION_FILE"
export SATURN_TEST_BRIDGE_RUNTIME=xdma
: >"$SYSTEMCTL_LOG"
if "$ACTIVATOR" "$P2_CLIENT_FAIL_COMMIT" >/dev/null 2>&1; then
  printf 'P2 with direct-XDMA Bridge unexpectedly passed preflight\n' >&2
  exit 1
fi
unset SATURN_TEST_BRIDGE_RUNTIME
if grep -Eq '^(start|stop) ' "$SYSTEMCTL_LOG"; then
  printf 'invalid P2/direct-XDMA baseline changed service state\n' >&2
  exit 1
fi
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$P2_CLIENT_COMMIT" ]]

printf 'Saturn release activation and automatic rollback tests passed\n'
