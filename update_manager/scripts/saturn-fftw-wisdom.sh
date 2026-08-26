#!/usr/bin/env bash
set -Eeuo pipefail

ACTION="${1:---check}"
BRIDGE_BIN="${SATURN_FFTW_WISDOM_BRIDGE_BIN:-/opt/saturn-go/bin/saturn-bridge}"
CACHE_DIR="${SATURN_FFTW_WISDOM_CACHE_DIR:-/var/cache/saturn-bridge}"
WISDOM_FILE="${SATURN_FFTW_WISDOM_FILE:-${CACHE_DIR}/wdspWisdom01}"
FINGERPRINT_FILE="${SATURN_FFTW_WISDOM_FINGERPRINT_FILE:-${CACHE_DIR}/fingerprint.sha256}"
INPUTS_FILE="${SATURN_FFTW_WISDOM_INPUTS_FILE:-${CACHE_DIR}/fingerprint.inputs}"
LOCK_FILE="${SATURN_FFTW_WISDOM_LOCK_FILE:-${CACHE_DIR}/.wisdom.lock}"
CPUINFO_FILE="${SATURN_FFTW_WISDOM_CPUINFO_FILE:-/proc/cpuinfo}"
MAX_SIZE="${SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE:-262144}"
GENERATOR_SCHEMA="saturn-rust-fftw-wisdom-v1"

log() {
  printf '[saturn-fftw-wisdom] %s\n' "$*"
}

die() {
  printf '[saturn-fftw-wisdom] ERROR: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: saturn-fftw-wisdom.sh [--check|--rebuild|--status]

  --check    Rebuild only when wisdom is missing, invalid, or stale (default).
  --rebuild  Force a new machine-local FFTW wisdom cache.
  --status   Report the current fingerprint state without changing files.
EOF
}

case "$ACTION" in
  --check|--rebuild|--status) ;;
  --help|-h) usage; exit 0 ;;
  *) usage >&2; die "unknown action: $ACTION" ;;
esac

[[ -x "$BRIDGE_BIN" ]] || die "Saturn Bridge binary is not executable: $BRIDGE_BIN"
[[ "$MAX_SIZE" =~ ^[0-9]+$ ]] || die "invalid FFT wisdom maximum: $MAX_SIZE"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

cpu_fingerprint() {
  if [[ -r "$CPUINFO_FILE" ]]; then
    awk -F: '
      BEGIN { OFS="=" }
      /^(model name|Model|CPU implementer|CPU architecture|CPU variant|CPU part|CPU revision|Features)[[:space:]]*:/ {
        key=$1; value=$2
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
        if (!seen[key "=" value]++) print key, value
      }
    ' "$CPUINFO_FILE"
  else
    printf 'cpuinfo=unavailable\n'
  fi
}

fftw_fingerprint() {
  local version="unknown" library="" library_hash="unknown"
  if command -v pkg-config >/dev/null 2>&1; then
    version="$(pkg-config --modversion fftw3 2>/dev/null || printf 'unknown')"
  fi
  if command -v ldconfig >/dev/null 2>&1; then
    library="$(ldconfig -p 2>/dev/null | awk '/libfftw3\.so\.3 .*=>/ && !found { print $NF; found=1 }')"
  fi
  if [[ -n "$library" && -r "$library" ]]; then
    library_hash="$(sha256sum "$library" | awk '{print $1}')"
  fi
  printf 'fftw_version=%s\n' "$version"
  printf 'fftw_library=%s\n' "${library:-unknown}"
  printf 'fftw_library_sha256=%s\n' "$library_hash"
}

write_fingerprint_inputs() {
  local destination="$1"
  {
    printf 'schema=%s\n' "$GENERATOR_SCHEMA"
    printf 'architecture=%s\n' "$(uname -m)"
    printf 'max_size=%s\n' "$MAX_SIZE"
    printf 'planner=FFTW_PATIENT\n'
    printf 'plans=complex-forward,complex-backward,complex-backward-plus-one,real-forward,real-inverse\n'
    printf 'bridge_sha256=%s\n' "$(sha256sum "$BRIDGE_BIN" | awk '{print $1}')"
    fftw_fingerprint
    cpu_fingerprint
  } >"$destination"
}

expected_fingerprint() {
  sha256sum "$1" | awk '{print $1}'
}

current_fingerprint() {
  if [[ -r "$FINGERPRINT_FILE" ]]; then
    tr -d '[:space:]' <"$FINGERPRINT_FILE"
  fi
}

wisdom_valid() {
  [[ -s "$WISDOM_FILE" ]] \
    && "$BRIDGE_BIN" --validate-fftw-wisdom "$WISDOM_FILE" >/dev/null 2>&1
}

mkdir -p "$(dirname "$LOCK_FILE")"
exec 9>"$LOCK_FILE"
if ! flock -n 9; then
  log "another wisdom check is already running; leaving it in control"
  exit 0
fi

if [[ "$ACTION" == "--status" ]]; then
  tmp_inputs="$(mktemp)"
  trap 'rm -f "$tmp_inputs"' EXIT
  write_fingerprint_inputs "$tmp_inputs"
  expected="$(expected_fingerprint "$tmp_inputs")"
  current="$(current_fingerprint)"
  validity="invalid"
  wisdom_valid && validity="valid"
  state="stale"
  [[ "$validity" == "valid" && "$current" == "$expected" ]] && state="fresh"
  printf 'state=%s\nwisdom=%s\nfingerprint=%s\nexpected=%s\n' \
    "$state" "$validity" "${current:-missing}" "$expected"
  exit 0
fi

install -d -m 0755 "$CACHE_DIR"
tmp_inputs="$(mktemp "${CACHE_DIR}/.fingerprint.inputs.XXXXXX")"
tmp_wisdom="$(mktemp "${CACHE_DIR}/.wdspWisdom01.XXXXXX")"
tmp_fingerprint="$(mktemp "${CACHE_DIR}/.fingerprint.sha256.XXXXXX")"
trap 'rm -f "$tmp_inputs" "$tmp_wisdom" "$tmp_fingerprint"' EXIT

write_fingerprint_inputs "$tmp_inputs"
expected="$(expected_fingerprint "$tmp_inputs")"
current="$(current_fingerprint)"

if [[ "$ACTION" == "--check" ]] && wisdom_valid && [[ "$current" == "$expected" ]]; then
  log "wisdom is valid and fingerprint is current"
  exit 0
fi

if [[ "$ACTION" == "--rebuild" ]]; then
  log "forcing FFTW wisdom rebuild through size $MAX_SIZE"
elif [[ ! -s "$WISDOM_FILE" ]]; then
  log "wisdom is missing; generating through size $MAX_SIZE"
elif ! wisdom_valid; then
  log "wisdom is invalid; regenerating through size $MAX_SIZE"
else
  log "wisdom fingerprint changed; regenerating through size $MAX_SIZE"
fi

rm -f "$tmp_wisdom"
SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE="$MAX_SIZE" \
  nice -n 19 ionice -c 3 \
  "$BRIDGE_BIN" --generate-fftw-wisdom "$tmp_wisdom"
[[ -s "$tmp_wisdom" ]] || die "Saturn Bridge did not create a wisdom file"
"$BRIDGE_BIN" --validate-fftw-wisdom "$tmp_wisdom" >/dev/null

printf '%s\n' "$expected" >"$tmp_fingerprint"
chmod 0644 "$tmp_wisdom" "$tmp_fingerprint" "$tmp_inputs"
mv -f "$tmp_wisdom" "$WISDOM_FILE"
mv -f "$tmp_fingerprint" "$FINGERPRINT_FILE"
mv -f "$tmp_inputs" "$INPUTS_FILE"
trap - EXIT
log "installed fresh FFTW wisdom at $WISDOM_FILE"
