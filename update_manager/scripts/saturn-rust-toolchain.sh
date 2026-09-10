#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

RUSTUP_VERSION="${SATURN_RUSTUP_VERSION:-1.29.1}"
TOOLCHAIN_FILE="${SATURN_RUST_TOOLCHAIN_FILE:-${REPO_ROOT}/rust-toolchain.toml}"
TOOLCHAIN="${SATURN_RUST_TOOLCHAIN:-}"
TARGET="${SATURN_RUSTUP_TARGET:-}"
BUILD_USER="${SATURN_RUST_BUILD_USER:-}"
BUILD_HOME="${SATURN_RUST_USER_HOME:-}"

log() { printf '[saturn-rust-toolchain] %s\n' "$*"; }
die() { printf '[saturn-rust-toolchain] ERROR: %s\n' "$*" >&2; exit 1; }

resolve_toolchain() {
  if [[ -n "$TOOLCHAIN" ]]; then
    return 0
  fi
  [[ -r "$TOOLCHAIN_FILE" ]] || die "Rust toolchain file not found: $TOOLCHAIN_FILE"
  TOOLCHAIN="$(awk -F '"' '/^[[:space:]]*channel[[:space:]]*=/ { print $2; exit }' "$TOOLCHAIN_FILE")"
  [[ -n "$TOOLCHAIN" ]] || die "Rust channel is missing from: $TOOLCHAIN_FILE"
}

resolve_target() {
  if [[ -n "$TARGET" ]]; then
    return 0
  fi
  [[ "$(uname -s)" == "Linux" ]] || die "Only Linux rustup bootstrap targets are supported"
  case "$(uname -m)" in
    aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
    x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    armv7l|armv7) TARGET="armv7-unknown-linux-gnueabihf" ;;
    *) die "Unsupported rustup bootstrap architecture: $(uname -m)" ;;
  esac
}

default_rustup_sha256() {
  [[ "$RUSTUP_VERSION" == "1.29.1" ]] \
    || die "No built-in rustup checksum is available for version: $RUSTUP_VERSION"
  case "$TARGET" in
    aarch64-unknown-linux-gnu)
      printf '%s\n' '15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433'
      ;;
    x86_64-unknown-linux-gnu)
      printf '%s\n' 'dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71'
      ;;
    armv7-unknown-linux-gnueabihf)
      printf '%s\n' '6f34abb0d553273ce08306ea3adb758d0171f21090cc5ad5426f474ded5504d1'
      ;;
    *) die "Unsupported rustup bootstrap target: $TARGET" ;;
  esac
}

resolve_build_user() {
  if [[ -z "$BUILD_USER" ]]; then
    if [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
      BUILD_USER="$SUDO_USER"
    elif [[ -n "${SATURN_USER:-}" ]]; then
      BUILD_USER="$SATURN_USER"
    else
      BUILD_USER="$(id -un)"
    fi
  fi
  id -u "$BUILD_USER" >/dev/null 2>&1 || die "Rust build user does not exist: $BUILD_USER"
  if [[ -z "$BUILD_HOME" ]]; then
    BUILD_HOME="$(getent passwd "$BUILD_USER" | cut -d: -f6)"
  fi
  [[ -n "$BUILD_HOME" && -d "$BUILD_HOME" ]] \
    || die "Cannot resolve home directory for Rust build user: $BUILD_USER"
}

run_as_build_user() {
  local cargo_home="$1" rustup_home="$2"
  shift 2
  local path_value="${cargo_home}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  if [[ "$(id -u)" -eq "$(id -u "$BUILD_USER")" ]]; then
    env HOME="$BUILD_HOME" CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" \
      PATH="$path_value" "$@"
  elif [[ "$(id -u)" -eq 0 ]]; then
    runuser -u "$BUILD_USER" -- env \
      HOME="$BUILD_HOME" CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" \
      PATH="$path_value" "$@"
  else
    die "Cannot prepare Rust for $BUILD_USER while running as $(id -un)"
  fi
}

print_config() {
  local expected_sha url
  expected_sha="${SATURN_RUSTUP_INIT_SHA256:-$(default_rustup_sha256)}"
  url="${SATURN_RUSTUP_INIT_URL:-https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${TARGET}/rustup-init}"
  printf 'rustup_version=%s\n' "$RUSTUP_VERSION"
  printf 'rustup_target=%s\n' "$TARGET"
  printf 'rustup_url=%s\n' "$url"
  printf 'rustup_sha256=%s\n' "$expected_sha"
  printf 'toolchain=%s\n' "$TOOLCHAIN"
}

check_toolchain() {
  local cargo_home rustup_home bin_dir
  cargo_home="${SATURN_RUST_CARGO_HOME:-${CARGO_HOME:-${BUILD_HOME}/.cargo}}"
  rustup_home="${SATURN_RUSTUP_HOME:-${RUSTUP_HOME:-${BUILD_HOME}/.rustup}}"
  bin_dir="${cargo_home}/bin"
  [[ -x "${bin_dir}/rustup" ]] || die "rustup is missing for $BUILD_USER: ${bin_dir}/rustup"
  [[ -x "${bin_dir}/cargo" ]] || die "cargo is missing for $BUILD_USER: ${bin_dir}/cargo"
  [[ -x "${bin_dir}/rustc" ]] || die "rustc is missing for $BUILD_USER: ${bin_dir}/rustc"
  run_as_build_user "$cargo_home" "$rustup_home" "${bin_dir}/rustc" "+${TOOLCHAIN}" --version
  run_as_build_user "$cargo_home" "$rustup_home" "${bin_dir}/cargo" "+${TOOLCHAIN}" --version
}

ensure_toolchain() {
  local cargo_home rustup_home bin_dir rustup_bin expected_sha url temp_dir installer actual_sha
  cargo_home="${SATURN_RUST_CARGO_HOME:-${CARGO_HOME:-${BUILD_HOME}/.cargo}}"
  rustup_home="${SATURN_RUSTUP_HOME:-${RUSTUP_HOME:-${BUILD_HOME}/.rustup}}"
  bin_dir="${cargo_home}/bin"
  rustup_bin="${bin_dir}/rustup"

  if [[ ! -x "$rustup_bin" ]]; then
    command -v curl >/dev/null 2>&1 || die "curl is required to bootstrap rustup"
    command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required to verify rustup"
    expected_sha="${SATURN_RUSTUP_INIT_SHA256:-$(default_rustup_sha256)}"
    [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || die "Invalid rustup SHA-256: $expected_sha"
    url="${SATURN_RUSTUP_INIT_URL:-https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${TARGET}/rustup-init}"
    temp_dir="$(mktemp -d)"
    installer="${temp_dir}/rustup-init"
    trap 'rm -f "${installer:-}"; rmdir "${temp_dir:-}" 2>/dev/null || true' EXIT
    log "Downloading immutable rustup ${RUSTUP_VERSION} bootstrap for ${TARGET}"
    curl --proto '=https' --tlsv1.2 --retry 3 --retry-all-errors -fsSL "$url" -o "$installer"
    actual_sha="$(sha256sum "$installer" | awk '{print $1}')"
    [[ "$actual_sha" == "$expected_sha" ]] \
      || die "Checksum mismatch for $url (expected $expected_sha, got $actual_sha)"
    chmod 0755 "$installer"
    run_as_build_user "$cargo_home" "$rustup_home" \
      "$installer" -y --profile minimal --default-toolchain none --no-modify-path
    rm -f "$installer"
    rmdir "$temp_dir"
    trap - EXIT
  fi

  if ! run_as_build_user "$cargo_home" "$rustup_home" \
      "$rustup_bin" run "$TOOLCHAIN" rustc --version >/dev/null 2>&1; then
    log "Installing pinned Rust toolchain ${TOOLCHAIN} for ${BUILD_USER}"
    run_as_build_user "$cargo_home" "$rustup_home" \
      "$rustup_bin" toolchain install "$TOOLCHAIN" --profile minimal
  fi
  run_as_build_user "$cargo_home" "$rustup_home" \
    "$rustup_bin" default "$TOOLCHAIN"
  log "Rust build prerequisite is ready for ${BUILD_USER}"
  check_toolchain
}

main() {
  local command="${1:-ensure}"
  resolve_toolchain
  resolve_target
  case "$command" in
    print-config)
      print_config
      ;;
    check)
      resolve_build_user
      check_toolchain
      ;;
    ensure)
      resolve_build_user
      ensure_toolchain
      ;;
    *) die "Usage: $0 [ensure|check|print-config]" ;;
  esac
}

main "$@"
