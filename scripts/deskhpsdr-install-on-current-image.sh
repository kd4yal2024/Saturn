#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
DEFAULT_REPO_URL="$(git -C "${SCRIPT_DIR}" config --get remote.origin.url 2>/dev/null || true)"
REPO_URL="${DEFAULT_REPO_URL:-https://github.com/dl1bz/deskhpsdr.git}"
TARGET_ROOT="${HOME}/github"
TARGET_NAME="deskhpsdr"
TARGET_DIR="${TARGET_ROOT}/${TARGET_NAME}"
JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"
RUN_CLEAN=1
INSTALL_DEPS=1
CREATE_DESKTOP_SHORTCUT=1
UPDATE_EXISTING=0

usage() {
  cat <<'EOF'
Usage: deskhpsdr-install-on-current-image.sh [options]

End-to-end deskHPSDR installer for Debian/Pi OS images.

What it does:
  1. Ensures the repo exists under ~/github (or a custom target directory)
  2. Clones deskHPSDR if it is not already there
  3. Runs the local build script to install prerequisites, prepare WDSP libs,
     build deskHPSDR with SATURN=ON, and create a Desktop shortcut

Options:
  --repo-url URL            clone source (default: repo origin or GitHub HTTPS URL)
  --target-root PATH        parent directory for the clone (default: ~/github)
  --target-dir PATH         full checkout path (overrides --target-root)
  --jobs N                  parallel make jobs for the build (default: detected CPU count)
  --update                  run git pull --ff-only in an existing checkout before building
  --no-install-deps         skip apt-based dependency installation in the build step
  --no-clean                skip "make clean" in the build step
  --no-desktop-shortcut     skip creating the Desktop launcher after a successful build
  -h, --help                show this help
EOF
}

ensure_git() {
  if command -v git >/dev/null 2>&1; then
    return 0
  fi

  if [[ ${INSTALL_DEPS} -eq 0 ]]; then
    echo "git is required to clone deskHPSDR." >&2
    exit 1
  fi

  echo "Installing git so deskHPSDR can be cloned..."
  sudo apt-get update
  sudo apt-get --yes install git
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-url)
      REPO_URL="$2"
      shift 2
      ;;
    --target-root)
      TARGET_ROOT="$2"
      TARGET_DIR="${TARGET_ROOT}/${TARGET_NAME}"
      shift 2
      ;;
    --target-dir)
      TARGET_DIR="$2"
      shift 2
      ;;
    --jobs)
      JOBS="$2"
      shift 2
      ;;
    --update)
      UPDATE_EXISTING=1
      shift
      ;;
    --no-install-deps)
      INSTALL_DEPS=0
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

ensure_git

mkdir -p "$(dirname "${TARGET_DIR}")"

if [[ -e "${TARGET_DIR}" && ! -d "${TARGET_DIR}/.git" ]]; then
  echo "Target exists but is not a git checkout: ${TARGET_DIR}" >&2
  exit 1
fi

if [[ ! -d "${TARGET_DIR}/.git" ]]; then
  echo "Cloning deskHPSDR into ${TARGET_DIR}..."
  git -C "$(dirname "${TARGET_DIR}")" clone "${REPO_URL}" "$(basename "${TARGET_DIR}")"
elif [[ ${UPDATE_EXISTING} -eq 1 ]]; then
  echo "Updating existing deskHPSDR checkout at ${TARGET_DIR}..."
  git -C "${TARGET_DIR}" pull --ff-only
else
  echo "Using existing deskHPSDR checkout at ${TARGET_DIR}..."
fi

BUILD_SCRIPT="${TARGET_DIR}/test-build-on-current-image.sh"

if [[ ! -f "${BUILD_SCRIPT}" ]]; then
  echo "Build script not found in ${TARGET_DIR}: ${BUILD_SCRIPT}" >&2
  echo "The target checkout may be older than this installer entry point." >&2
  exit 1
fi

chmod +x "${BUILD_SCRIPT}"

BUILD_ARGS=(
  --repo "${TARGET_DIR}"
  --jobs "${JOBS}"
)

if [[ ${INSTALL_DEPS} -eq 1 ]]; then
  BUILD_ARGS+=(--install-deps)
fi

if [[ ${RUN_CLEAN} -eq 0 ]]; then
  BUILD_ARGS+=(--no-clean)
fi

if [[ ${CREATE_DESKTOP_SHORTCUT} -eq 0 ]]; then
  BUILD_ARGS+=(--no-desktop-shortcut)
fi

echo "Running deskHPSDR build and install flow from ${TARGET_DIR}..."
"${BUILD_SCRIPT}" "${BUILD_ARGS[@]}"
