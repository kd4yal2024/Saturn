#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
REPO_DIR="${SCRIPT_DIR}"
PATCH_DIR="${SCRIPT_DIR}/patches"
DESKHPSDR_GPIO_PATCH="${PATCH_DIR}/deskhpsdr-libgpiod-v2.patch"
DEPS_HELPER_NAME="deskhpsdr-install-deps-on-current-image.sh"
PRIVILEGED_DEPS_HELPER="/usr/local/lib/saturn-go/scripts/${DEPS_HELPER_NAME}"
JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"
INSTALL_DEPS=0
RUN_CLEAN=1
CREATE_DESKTOP_SHORTCUT=1
LEGACY_GPIO_AVAILABLE=0

usage() {
  cat <<'EOF'
Usage: deskhpsdr-test-build-on-current-image.sh [options]

Build-probe deskHPSDR against the current image using the upstream Linux build.

Options:
  --repo PATH         deskHPSDR repo path (default: script directory)
  --jobs N            parallel make jobs (default: detected CPU count)
  --install-deps      install Debian prerequisites before building
  --no-clean          skip "make clean" before the build probe
  --no-desktop-shortcut
                      skip creating a Desktop launcher for the built binary
  -h, --help          show this help

Notes:
  - This script probes build compatibility for the current image.
  - It applies the local Saturn libgpiod v2 compatibility patch before building.
  - It forces SATURN=ON and GPIO=ON for the build probe.
  - It does not run "make install".
  - On success it creates a user-level launcher and Desktop shortcut by default.
EOF
}

apply_saturn_patch() {
  local repo_dir patch_file patch_name

  repo_dir="$1"
  patch_file="$2"
  patch_name="$(basename "${patch_file}")"

  if [[ ! -f "${patch_file}" ]]; then
    echo "Required Saturn patch not found: ${patch_file}" >&2
    exit 1
  fi

  if ! command -v git >/dev/null 2>&1; then
    echo "git is required to apply Saturn deskHPSDR patches." >&2
    exit 1
  fi

  if git -C "${repo_dir}" apply --check "${patch_file}" >/dev/null 2>&1; then
    echo "Applying local Saturn patch: ${patch_name}"
    git -C "${repo_dir}" apply "${patch_file}"
    return 0
  fi

  if git -C "${repo_dir}" apply --reverse --check "${patch_file}" >/dev/null 2>&1; then
    echo "Local Saturn patch already present: ${patch_name}"
    return 0
  fi

  echo "Local Saturn patch could not be applied cleanly: ${patch_name}" >&2
  git -C "${repo_dir}" apply --check "${patch_file}" || true
  exit 1
}

package_installed() {
  dpkg-query -W -f='${Status}\n' "$1" 2>/dev/null | grep -q "install ok installed"
}

detect_legacy_gpio_support() {
  if [[ -f "${REPO_DIR}/src/gpio.c" ]] && grep -q "GPIOD_VERSION" "${REPO_DIR}/Makefile"; then
    LEGACY_GPIO_AVAILABLE=1
  else
    LEGACY_GPIO_AVAILABLE=0
  fi
}

install_debian_prerequisites() {
  local helper

  if [[ -x "${PRIVILEGED_DEPS_HELPER}" ]]; then
    helper="${PRIVILEGED_DEPS_HELPER}"
  elif [[ -x "${SCRIPT_DIR}/${DEPS_HELPER_NAME}" ]]; then
    helper="${SCRIPT_DIR}/${DEPS_HELPER_NAME}"
  else
    echo "deskHPSDR dependency helper not found: ${PRIVILEGED_DEPS_HELPER}" >&2
    echo "Reinstall Saturn Go so the privileged helper and sudoers policy are provisioned." >&2
    exit 1
  fi

  echo "Installing deskHPSDR prerequisites for ${PRETTY_NAME:-Debian-based image}..."
  if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    "${helper}" --repo "${REPO_DIR}"
  else
    sudo -n "${helper}" --repo "${REPO_DIR}"
  fi
}

create_desktop_shortcut() {
  local desktop_dir app_dir app_file link_file icon_path binary_path

  binary_path="${REPO_DIR}/deskhpsdr"
  if [[ ! -x "${binary_path}" ]]; then
    echo "Built binary not found, skipping Desktop shortcut: ${binary_path}" >&2
    return 1
  fi

  if command -v xdg-user-dir >/dev/null 2>&1; then
    desktop_dir="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
  fi
  desktop_dir="${desktop_dir:-${HOME}/Desktop}"
  app_dir="${HOME}/.local/share/applications"
  app_file="${app_dir}/deskHPSDR-local.desktop"
  link_file="${desktop_dir}/deskHPSDR-local.desktop"
  icon_path="${REPO_DIR}/release/deskhpsdr/trx_icon.png"

  mkdir -p "${app_dir}" "${desktop_dir}"

  if [[ ! -f "${icon_path}" ]]; then
    icon_path="applications-other"
  fi

  cat > "${app_file}" <<EOF
[Desktop Entry]
Version=1.0
Categories=X-Hamradio
Comment=deskHPSDR local build from ${REPO_DIR}
Type=Application
Terminal=false
Exec=${binary_path}
Path=${REPO_DIR}
Icon=${icon_path}
Name=deskHPSDR Local
GenericName=SDR-Application
StartupNotify=true
EOF
  chmod 0644 "${app_file}"

  cat > "${link_file}" <<EOF
[Desktop Entry]
Type=Link
Name=deskHPSDR Local
Icon=${icon_path}
URL=${app_file}
EOF
  chmod 0755 "${link_file}"

  update-desktop-database "${app_dir}" >/dev/null 2>&1 || :

  echo "Desktop launcher created:"
  echo "  app:     ${app_file}"
  echo "  shortcut:${link_file}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      REPO_DIR="$2"
      shift 2
      ;;
    --jobs)
      JOBS="$2"
      shift 2
      ;;
    --install-deps)
      INSTALL_DEPS=1
      shift
      ;;
    --no-clean)
      RUN_CLEAN=0
      shift
      ;;
    --no-desktop-shortcut)
      CREATE_DESKTOP_SHORTCUT=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! -d "${REPO_DIR}" ]]; then
  echo "deskHPSDR repo not found: ${REPO_DIR}" >&2
  exit 1
fi

if [[ ! -f "${REPO_DIR}/Makefile" ]]; then
  echo "No Makefile found under ${REPO_DIR}" >&2
  exit 1
fi

detect_legacy_gpio_support

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  . /etc/os-release
else
  echo "/etc/os-release missing; cannot determine base image" >&2
  exit 1
fi

if [[ "${ID:-}" != "debian" && "${ID_LIKE:-}" != *debian* ]]; then
  echo "This script currently expects a Debian-based image. Detected ID=${ID:-unknown}." >&2
  exit 1
fi

case "${VERSION_CODENAME:-}" in
  trixie)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.1-dev"
    ;;
  bookworm)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.0-dev"
    ;;
  *)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.1-dev"
    ;;
esac

DEBIAN_PACKAGES=(
  build-essential
  pkg-config
  make
  gcc
  g++
  git
  cmake
  autoconf
  autopoint
  gettext
  automake
  libtool
  dos2unix
  libzstd-dev
  python3-dev
  wget
  meson
  ninja-build
  clang
  llvm
  libfftw3-dev
  libgtk-3-dev
  "${WEBKIT_DEV_PKG}"
  libasound2-dev
  libssl-dev
  libcurl4-openssl-dev
  libusb-1.0-0-dev
  libi2c-dev
  libpulse-dev
  libpcap-dev
  libjson-c-dev
  gnome-themes-extra
  libaio-dev
  libavahi-client-dev
  libad9361-dev
  libiio-dev
  bison
  flex
  libxml2-dev
)

if [[ ${LEGACY_GPIO_AVAILABLE} -eq 1 ]]; then
  DEBIAN_PACKAGES+=(libgpiod-dev)
fi

if ! package_installed pipewire-pulse && ! package_installed pulseaudio; then
  DEBIAN_PACKAGES+=(pipewire-pulse)
fi

missing_packages=()
for pkg in "${DEBIAN_PACKAGES[@]}"; do
  if ! package_installed "$pkg"; then
    missing_packages+=("$pkg")
  fi
done

if [[ ${INSTALL_DEPS} -eq 1 ]]; then
  install_debian_prerequisites

  missing_packages=()
  for pkg in "${DEBIAN_PACKAGES[@]}"; do
    if ! package_installed "$pkg"; then
      missing_packages+=("$pkg")
    fi
  done
fi

webkit_version="$(pkg-config --modversion webkit2gtk-4.1 2>/dev/null || pkg-config --modversion webkit2gtk-4.0 2>/dev/null || true)"
gtk_version="$(pkg-config --modversion gtk+-3.0 2>/dev/null || true)"
fftw3_version="$(pkg-config --modversion fftw3 2>/dev/null || true)"
fftw3f_version="$(pkg-config --modversion fftw3f 2>/dev/null || true)"

echo "deskHPSDR build probe"
echo "  repo:           ${REPO_DIR}"
echo "  image:          ${PRETTY_NAME:-unknown}"
echo "  codename:       ${VERSION_CODENAME:-unknown}"
echo "  jobs:           ${JOBS}"
echo "  SATURN:         ON"
if [[ ${LEGACY_GPIO_AVAILABLE} -eq 1 ]]; then
  echo "  Pi GPIO:        legacy source present; patch/build enabled"
else
  echo "  Pi GPIO:        upstream removed legacy source; patch/build skipped"
fi
echo "  WebKitGTK:      ${webkit_version:-missing}"
echo "  GTK3:           ${gtk_version:-missing}"
echo "  fftw3:          ${fftw3_version:-missing}"
echo "  fftw3f:         ${fftw3f_version:-missing}"

if [[ ${#missing_packages[@]} -gt 0 ]]; then
  echo
  echo "Missing Debian packages detected:"
  printf '  - %s\n' "${missing_packages[@]}"
fi

if [[ -z "${webkit_version}" ]]; then
  echo
  echo "Build blocker: WebKitGTK is missing."
  echo "Install ${WEBKIT_DEV_PKG} and retry, or rerun with --install-deps."
  exit 3
fi

LOG_DIR="/tmp/deskhpsdr-build"
mkdir -p "${LOG_DIR}"
STAMP="$(date +%Y%m%d-%H%M%S)"
BUILD_LOG="${LOG_DIR}/deskhpsdr-build-${STAMP}.log"

if [[ ${LEGACY_GPIO_AVAILABLE} -eq 1 ]]; then
  apply_saturn_patch "${REPO_DIR}" "${DESKHPSDR_GPIO_PATCH}"
else
  echo "Skipping Saturn libgpiod patch; upstream checkout has no src/gpio.c legacy GPIO path."
fi

MAKE_ARGS=(
  -C "${REPO_DIR}"
  "SATURN=ON"
  "SOAPYSDR=OFF"
  "AUDIO=PULSE"
  "ATU=OFF"
  "COPYMODE=OFF"
)

if [[ ${LEGACY_GPIO_AVAILABLE} -eq 1 ]]; then
  MAKE_ARGS+=("GPIO=ON")
fi

{
  echo "=== $(date -Is) make probe start ==="
  if [[ ${RUN_CLEAN} -eq 1 ]]; then
    make "${MAKE_ARGS[@]}" clean
  fi
  make "${MAKE_ARGS[@]}" prepare
  make "${MAKE_ARGS[@]}" -j"${JOBS}"
  echo "=== $(date -Is) make probe success ==="
} 2>&1 | tee "${BUILD_LOG}"

echo
echo "Build probe succeeded."
echo "Log: ${BUILD_LOG}"

if [[ ${CREATE_DESKTOP_SHORTCUT} -eq 1 ]]; then
  echo
  create_desktop_shortcut
fi
