#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="${REPO_ROOT}/scripts/deskhpsdr-test-build-on-current-image.sh"
INSTALLER="${REPO_ROOT}/scripts/deskhpsdr-install-on-current-image.sh"
UPDATER="${REPO_ROOT}/update_manager/scripts/update-deskhpsdr.py"
CONFIG="${REPO_ROOT}/update_manager/scripts/config.json"
UI_TEMPLATE="${REPO_ROOT}/update_manager/templates/deskhpsdr.html"
GPIO_PATCH="${REPO_ROOT}/scripts/patches/deskhpsdr-libgpiod-v2.patch"
GPIO_CLEANUP_PATCH="${REPO_ROOT}/scripts/patches/deskhpsdr-libgpiod-v2-cleanup.patch"
STARTUP_PATCH="${REPO_ROOT}/scripts/patches/deskhpsdr-active-receiver-init.patch"
LEGACY_STARTUP_PATCH="${REPO_ROOT}/scripts/patches/deskhpsdr-2.6.84-active-receiver-init.patch"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

mkdir -p "${TMP_DIR}/src"
cat >"${TMP_DIR}/src/new_protocol.c" <<'EOF'
static void new_protocol_high_priority(void) {
  int rxant, txant;
  long long DDCfrequency[2];  // DDC frequencies of the radio
  long long DUCfrequency;     // DUC frequency of the radio
  long long txfreq;           // frequency used for out-of-band detection
  long long duc_txfreq;       // frequency used for the TX carrier/DUC
  long long HPFfreq;          // frequency determining the HPF filters
  long long LPFfreq;          // frequency determining the LPF filters
  long long BPFfreq;          // frequency determining the BPF filters
  unsigned long phase;
  if (data_socket == -1 && !have_saturn_xdma) {
    return;
  }
  pthread_mutex_lock(&hi_prio_mutex);
  memset(high_priority_buffer_to_radio, 0, sizeof(high_priority_buffer_to_radio));
  //
EOF

git -C "${TMP_DIR}" apply --check "${STARTUP_PATCH}"
git -C "${TMP_DIR}" apply "${STARTUP_PATCH}"
grep -Fq 'if (active_receiver == NULL)' "${TMP_DIR}/src/new_protocol.c"

# These are intentionally literal shell fragments from the helper.
# shellcheck disable=SC2016
grep -Fq 'apply_saturn_patch "${REPO_DIR}" "${DESKHPSDR_STARTUP_PATCH}"' "${HELPER}"
# shellcheck disable=SC2016
grep -Fq 'REQUIRE_LEGACY_GPIO=1' "${HELPER}"
# shellcheck disable=SC2016
grep -Fq 'apply_saturn_patch "${REPO_DIR}" "${DESKHPSDR_LEGACY_STARTUP_PATCH}"' "${HELPER}"
# shellcheck disable=SC2016
grep -Fq 'BUILD_ARGS+=(--require-legacy-gpio)' "${INSTALLER}"
grep -Fq 'LEGACY_GPIO_REF = "2.6.84"' "${UPDATER}"
grep -Fq 'cmd.append("--require-legacy-gpio")' "${UPDATER}"
jq -e '
  (map(select(.filename == "update-deskhpsdr.py"))[0].flags | index("--legacy-gpio")) != null and
  (map(select(.filename == "update-pihpsdr.py"))[0].flags | index("--legacy-gpio")) == null and
  (map(select(.filename == "update-pihpsdr.py"))[0].flags | index("--no-gpio")) != null
' "${CONFIG}" >/dev/null
grep -Fq 'Legacy GPIO V1 (deskHPSDR 2.6.84 / Trixie)' "${UI_TEMPLATE}"
grep -Fq 'gpiod_chip_request_lines' "${GPIO_PATCH}"
grep -Fq 'gpiod_line_request_read_edge_events' "${GPIO_PATCH}"
grep -Fq 'release_request(&monitor_request);' "${GPIO_CLEANUP_PATCH}"
grep -Fq 'DESKHPSDR_GPIO_CLEANUP_PATCH' "${HELPER}"
reverse_check_line="$(grep -n 'apply --reverse --check' "${HELPER}" | head -n1 | cut -d: -f1)"
forward_check_line="$(grep -n "apply --check \"\${patch_file}\"" "${HELPER}" | head -n1 | cut -d: -f1)"
if [[ -z "${reverse_check_line}" || -z "${forward_check_line}" || ${reverse_check_line} -ge ${forward_check_line} ]]; then
  echo "deskHPSDR patch helper must detect already-applied insertion patches before testing forward application" >&2
  exit 1
fi
grep -Fq 'if (active_receiver == NULL)' "${LEGACY_STARTUP_PATCH}"
grep -Fq 'gpiod_line_request_read_edge_events' "${HELPER}"
# shellcheck disable=SC2016
grep -Fq 'install -m 0755 "${app_file}" "${link_file}"' "${HELPER}"
if grep -Eq '^[[:space:]]*Type=Link' "${HELPER}" || grep -Eq '^[[:space:]]*URL=' "${HELPER}"; then
  echo "deskHPSDR Desktop shortcut must be a direct application launcher" >&2
  exit 1
fi

printf 'deskHPSDR integration tests passed\n'
