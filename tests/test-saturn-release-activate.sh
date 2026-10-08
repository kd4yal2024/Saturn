#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
ACTIVATOR="$REPO_ROOT/update_manager/scripts/saturn-release-activate-root.sh"
MANIFEST_TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
STATE_TOOL="$REPO_ROOT/update_manager/scripts/saturn-state-compatibility.py"
COMPONENTS="$REPO_ROOT/update_manager/release/components-v1.json"
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
  start) : >"$SATURN_TEST_SYSTEMCTL_STATE/$2" ;;
  stop) rm -f -- "$SATURN_TEST_SYSTEMCTL_STATE/$2" ;;
esac
EOF
cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "$*" == *'/p23_perf'* ]]; then
  backend="${SATURN_TEST_SELECTED_BACKEND:-xdma}"
  app="saturn-bridge"
  [[ "$backend" == "p2" ]] && app="p2"
  active=true
  current="$(readlink -f "$SATURN_TEST_CURRENT_LINK" 2>/dev/null || true)"
  if [[ "$current" == *"/${SATURN_TEST_OWNER_UNAVAILABLE_COMMIT:-not-a-commit}" ]]; then active=false; fi
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
p2=inactive
bridge=inactive
[[ -f "$SATURN_TEST_SYSTEMCTL_STATE/p2app.service" ]] && p2=active
[[ -f "$SATURN_TEST_SYSTEMCTL_STATE/saturn-bridge.service" ]] && bridge=active
status=stopped
if [[ "$backend" == p2 && "$p2" == active && "$bridge" == inactive ]] || \
   [[ "$backend" == xdma && "$bridge" == active && "$p2" == inactive ]]; then status=ready; fi
printf '{"selected":"%s","runtime":"%s","operational_status":"%s","transaction_status":"idle","mutual_exclusion_ok":true,"services":{"p2app":"%s","saturn_bridge":"%s"}}\n' \
  "$backend" "$backend" "$status" "$p2" "$bridge"
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
[[ "$(find "$RELEASES_ROOT" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 10 ]]

# P2-selected activation must not initialize the direct-XDMA Bridge.
rm -f -- "$TRANSACTION_FILE" "$SYSTEMCTL_STATE/saturn-bridge.service"
: >"$SYSTEMCTL_STATE/p2app.service"
export SATURN_TEST_SELECTED_BACKEND=p2
"$ACTIVATOR" "$P2_COMMIT" >/dev/null
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

printf 'Saturn release activation and automatic rollback tests passed\n'
