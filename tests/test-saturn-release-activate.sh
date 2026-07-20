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
ln -s "$RELEASES_ROOT/$OLD_COMMIT" "$CURRENT_LINK"

cat >"$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >>"$SATURN_TEST_SYSTEMCTL_LOG"
exit 0
EOF
cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ "$*" == *"expected_commit=$SATURN_TEST_EXPECTED_COMMIT"* ]]
[[ "${SATURN_TEST_READY_FAIL:-0}" != "1" ]]
EOF
chmod 0755 "$FAKE_BIN/systemctl" "$FAKE_BIN/curl"

export PATH="$FAKE_BIN:$PATH"
export SATURN_RELEASE_ACTIVATE_CONFIG="$CONFIG_FILE"
export SATURN_RELEASE_ACTIVATE_TEST_MODE=1
export SATURN_TEST_SYSTEMCTL_LOG="$SYSTEMCTL_LOG"
export SATURN_TEST_EXPECTED_COMMIT="$NEW_COMMIT"

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
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$OLD_COMMIT" ]]
[[ ! -e "$TRANSACTION_FILE" ]]

write_config 1
ln -s /etc/passwd "$TRANSACTION_FILE"
if "$ACTIVATOR" "$NEW_COMMIT" >/dev/null 2>&1; then
  printf 'symlinked transaction state unexpectedly accepted\n' >&2
  exit 1
fi
rm -f "$TRANSACTION_FILE"
[[ "$(readlink -f "$CURRENT_LINK")" == "$RELEASES_ROOT/$OLD_COMMIT" ]]
"$ACTIVATOR" "$NEW_COMMIT" >/dev/null
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
assert value["previous_commit"] == sys.argv[2]
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

cat >"$TMP_ROOT/expected-systemctl.log" <<'EOF'
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
cmp "$TMP_ROOT/expected-systemctl.log" "$SYSTEMCTL_LOG"

# REM-0203 records an uncommitted failure but intentionally does not claim an
# automatic rollback. Production activation remains disabled until REM-0204.
export SATURN_TEST_EXPECTED_COMMIT="$BAD_COMMIT"
export SATURN_TEST_READY_FAIL=1
if "$ACTIVATOR" "$BAD_COMMIT" >/dev/null 2>&1; then
  printf 'readiness failure unexpectedly committed activation\n' >&2
  exit 1
fi
python3 - "$TRANSACTION_FILE" "$BAD_COMMIT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["status"] == "failed"
assert value["phase"] == "readiness"
assert value["target_commit"] == sys.argv[2]
PY

printf 'Saturn release activation transaction tests passed\n'
