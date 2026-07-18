#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="${REPO_ROOT}/scripts/deskhpsdr-test-build-on-current-image.sh"
STARTUP_PATCH="${REPO_ROOT}/scripts/patches/deskhpsdr-active-receiver-init.patch"
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
grep -Fq 'install -m 0755 "${app_file}" "${link_file}"' "${HELPER}"
if grep -Eq '^[[:space:]]*Type=Link' "${HELPER}" || grep -Eq '^[[:space:]]*URL=' "${HELPER}"; then
  echo "deskHPSDR Desktop shortcut must be a direct application launcher" >&2
  exit 1
fi

printf 'deskHPSDR integration tests passed\n'
