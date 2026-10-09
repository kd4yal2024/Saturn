#!/usr/bin/env bash
# Source from the P2 installer. A new appliance defaults to P2, while an
# explicit, persisted direct-XDMA selection must not be replaced by P2.

saturn_p2_selected_backend() {
  local state_file="${SATURN_RADIO_BACKEND_STATE_FILE:-/var/lib/saturn-radio-backend/selection.json}"
  if [[ ! -e "$state_file" && ! -L "$state_file" ]]; then
    printf 'p2\n'
    return 0
  fi
  python3 - "$state_file" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        selected = json.load(handle)["active"]
except (OSError, ValueError, KeyError, TypeError) as error:
    raise SystemExit(f"invalid persisted radio backend; refusing to start P2: {error}")
if selected not in {"p2", "xdma"}:
    raise SystemExit(f"invalid persisted radio backend; refusing to start P2: {selected!r}")
print(selected)
PY
}
