#!/usr/bin/env bash
set -euo pipefail
: "${SATURN_ARM_LLD:?explicit rust-lld path required}"
exec "$SATURN_ARM_LLD" -flavor gnu "$@"
