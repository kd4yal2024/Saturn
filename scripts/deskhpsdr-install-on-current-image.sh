#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
REPO_URL="${DESKHPSDR_REPO_URL:-https://github.com/dl1bz/deskhpsdr.git}"
TARGET_ROOT="${HOME}/github"
TARGET_NAME="deskhpsdr"
TARGET_DIR="${TARGET_ROOT}/${TARGET_NAME}"
JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"
RUN_CLEAN=1
INSTALL_DEPS=1
CREATE_DESKTOP_SHORTCUT=1
UPDATE_EXISTING=0
LEGACY_GPIO=0
LEGACY_GPIO_REF="${DESKHPSDR_LEGACY_GPIO_REF:-2.6.84}"

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
  --repo-url URL            clone source (default: upstream deskHPSDR GitHub URL)
  --target-root PATH        parent directory for the clone (default: ~/github)
  --target-dir PATH         full checkout path (overrides --target-root)
  --jobs N                  parallel make jobs for the build (default: detected CPU count)
  --update                  run git pull --ff-only in an existing checkout before building
  --legacy-gpio             pin deskHPSDR 2.6.84 and apply Saturn's Trixie
                            libgpiod-v2 patch for V1 GPIO controllers
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
    --legacy-gpio)
      LEGACY_GPIO=1
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
elif [[ ${UPDATE_EXISTING} -eq 1 && ${LEGACY_GPIO} -eq 0 ]]; then
  echo "Updating existing deskHPSDR checkout at ${TARGET_DIR}..."
  current_branch="$(git -C "${TARGET_DIR}" branch --show-current)"
  if [[ -z "${current_branch}" ]]; then
    echo "Existing deskHPSDR checkout is detached; use Saturn's deskHPSDR updater to switch channels safely." >&2
    exit 1
  fi
  git -C "${TARGET_DIR}" pull --ff-only origin "${current_branch}"
else
  echo "Using existing deskHPSDR checkout at ${TARGET_DIR}..."
fi

if [[ ${LEGACY_GPIO} -eq 1 ]]; then
  current_ref="$(git -C "${TARGET_DIR}" describe --tags --exact-match HEAD 2>/dev/null || true)"
  if [[ "${current_ref}" != "${LEGACY_GPIO_REF}" ]]; then
    if ! git -C "${TARGET_DIR}" diff-index --quiet HEAD --; then
      echo "Cannot select deskHPSDR ${LEGACY_GPIO_REF} with tracked local changes present: ${TARGET_DIR}" >&2
      echo "Use Saturn's deskHPSDR updater, which backs up and stashes the patched tree safely." >&2
      exit 1
    fi
    git -C "${TARGET_DIR}" fetch --force origin \
      "refs/tags/${LEGACY_GPIO_REF}:refs/tags/${LEGACY_GPIO_REF}"
    git -C "${TARGET_DIR}" checkout --detach "${LEGACY_GPIO_REF}"
  fi
  echo "Selected pinned legacy GPIO source: deskHPSDR ${LEGACY_GPIO_REF}"
fi

BUILD_SCRIPT="${SCRIPT_DIR}/deskhpsdr-test-build-on-current-image.sh"

if [[ ! -f "${BUILD_SCRIPT}" ]]; then
  echo "Saturn deskHPSDR build helper not found: ${BUILD_SCRIPT}" >&2
  echo "Run this installer from the Saturn repository scripts directory." >&2
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

if [[ ${LEGACY_GPIO} -eq 1 ]]; then
  BUILD_ARGS+=(--require-legacy-gpio)
fi

echo "Running deskHPSDR build and install flow from ${TARGET_DIR}..."
"${BUILD_SCRIPT}" "${BUILD_ARGS[@]}"
