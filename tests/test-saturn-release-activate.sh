#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
ACTIVATOR="$REPO_ROOT/update_manager/scripts/saturn-release-activate-root.sh"
MANIFEST_TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
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
OLD_COMMIT="1111111111111111111111111111111111111111"
NEW_COMMIT="2222222222222222222222222222222222222222"
BAD_COMMIT="3333333333333333333333333333333333333333"
STARTUP_COMMIT="4444444444444444444444444444444444444444"
CONFIG_COMMIT="5555555555555555555555555555555555555555"
WRONG_COMMIT="6666666666666666666666666666666666666666"
ROLLBACK_FAIL_COMMIT="7777777777777777777777777777777777777777"

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
SYSTEMD_ROOT="$SYSTEMD_ROOT"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
SATURN_GO_READY_URL="http://127.0.0.1:18080/readyz"
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
exit 0
EOF
cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
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
chmod 0755 "$FAKE_BIN/systemctl" "$FAKE_BIN/curl"

export PATH="$FAKE_BIN:$PATH"
export SATURN_RELEASE_ACTIVATE_CONFIG="$CONFIG_FILE"
export SATURN_RELEASE_ACTIVATE_TEST_MODE=1
export SATURN_TEST_SYSTEMCTL_LOG="$SYSTEMCTL_LOG"
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
if grep -Eq 'NOPASSWD:.*saturn-release-activate-root' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"; then
  printf 'activation broker unexpectedly exposed through sudoers\n' >&2
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
ln -s /etc/passwd "$TRANSACTION_FILE"
if "$ACTIVATOR" "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'symlinked transaction state unexpectedly accepted\n' >&2
  exit 1
fi
rm -f "$TRANSACTION_FILE"
[[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]

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
PY

"$ACTIVATOR" "$NEW_COMMIT" >/dev/null
export SATURN_TEST_RUNNING_COMMIT="$NEW_COMMIT"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$NEW_COMMIT" ]]
[[ -z "$(find "$SATURN_ROOT" -maxdepth 1 -name '.current.*' -print -quit)" ]]
[[ "$(stat -c '%a' "$TRANSACTION_FILE")" == "640" ]]

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
    "p2app.service", "saturn-bridge.service", "saturn-go.service"
]
assert all(
    not item["previously_existed"]
    for item in value["service_dropins"].values()
)
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

cat >"$TMP_ROOT/expected-systemctl.log" <<'EOF'
is-active --quiet saturn-go.service
is-active --quiet saturn-bridge.service
is-active --quiet p2app.service
daemon-reload
stop saturn-go.service
stop saturn-bridge.service
stop p2app.service
start p2app.service
is-active --quiet p2app.service
start saturn-bridge.service
is-active --quiet saturn-bridge.service
start saturn-go.service
is-active --quiet saturn-go.service
stop saturn-go.service
stop saturn-bridge.service
stop p2app.service
daemon-reload
start p2app.service
is-active --quiet p2app.service
start saturn-bridge.service
is-active --quiet saturn-bridge.service
start saturn-go.service
is-active --quiet saturn-go.service
is-active --quiet saturn-go.service
is-active --quiet saturn-bridge.service
is-active --quiet p2app.service
daemon-reload
stop saturn-go.service
stop saturn-bridge.service
stop p2app.service
start p2app.service
is-active --quiet p2app.service
start saturn-bridge.service
is-active --quiet saturn-bridge.service
start saturn-go.service
is-active --quiet saturn-go.service
EOF
diff -u "$TMP_ROOT/expected-systemctl.log" "$SYSTEMCTL_LOG"

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
[[ "$(find "$RELEASES_ROOT" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 7 ]]

printf 'Saturn release activation and automatic rollback tests passed\n'
