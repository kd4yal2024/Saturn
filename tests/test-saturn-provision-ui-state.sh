#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

g++ -std=c++17 -Wall -Wextra -Werror -I"$REPO_ROOT/provision/cloud-init" \
  -x c++ -o "$TMP_DIR/ui-state-test" - <<'CPP'
#include "saturn-provision-ui-state.h"
#include <cassert>

int main() {
    // An old completion marker must not finish a new RUNNING installer.
    assert(!saturn_provision_run_succeeded(true, true, "RUNNING"));
    assert(!saturn_provision_run_succeeded(true, true, "FAILED"));
    assert(!saturn_provision_run_succeeded(true, true, "SKIPPED"));
    assert(!saturn_provision_run_succeeded(true, false, ""));
    assert(!saturn_provision_run_succeeded(false, true, "SUCCESS"));
    assert(saturn_provision_run_succeeded(true, true, "SUCCESS"));
}
CPP
"$TMP_DIR/ui-state-test"

printf 'Saturn provisioning UI state tests passed\n'
