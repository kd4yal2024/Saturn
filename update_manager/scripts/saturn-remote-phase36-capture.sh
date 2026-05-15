#!/usr/bin/env bash
set -euo pipefail

duration_sec="${1:-300}"
interval_sec="${2:-1}"
session_label="${3:-phase36}"
out_dir="${4:-$HOME/Documents/perf-captures/phase36}"
remote_path="${SATURN_REMOTE_PATH:-/remote-next}"
asset_path="${SATURN_REMOTE_ASSET:-/remote-assets/remote-next.js}"
stamp="$(date +%Y%m%d-%H%M%S)"
safe_label="$(printf '%s' "$session_label" | tr -c 'A-Za-z0-9_.=-' '_')"
out_file="$out_dir/saturn-remote-${safe_label}-${stamp}.log"

mkdir -p "$out_dir"

curl_metric() {
  local label="$1"
  local path="$2"
  curl -k -sS -o /dev/null \
    -w "${label}_http=%{http_code} connect=%{time_connect} starttransfer=%{time_starttransfer} total=%{time_total}\n" \
    "https://127.0.0.1:8443${path}" || true
}

{
  echo "# Saturn Remote Phase 36 host capture"
  echo "timestamp=$(date --iso-8601=seconds)"
  echo "duration_sec=$duration_sec"
  echo "interval_sec=$interval_sec"
  echo "session_label=$session_label"
  echo "remote_path=$remote_path"
  echo "asset_path=$asset_path"
  echo
  echo "## service status"
  systemctl status saturn-go.service --no-pager || true
  echo
  systemctl status saturn-bridge.service --no-pager || true
  echo
  echo "## listeners"
  ss -ltnp || true
  echo
  echo "## live samples"
} > "$out_file"

samples=$((duration_sec / interval_sec))
if (( samples < 1 )); then
  samples=1
fi

for ((i = 1; i <= samples; i += 1)); do
  {
    echo
    echo "### sample=$i wall_time=$(date --iso-8601=seconds)"
    echo "## processes"
    ps -C saturn-go,saturn-bridge -o pid,ppid,%cpu,%mem,rss,etime,cmd --no-headers || true
    echo
    echo "## tcp queues"
    ss -tinp | grep -E '(:8443|:50001)' || true
    echo
    echo "## endpoint timing"
    curl_metric remote "$remote_path"
    curl_metric remote_asset "$asset_path"
    echo
    echo "## saturn-bridge recent diag"
    journalctl -u saturn-bridge.service -n 12 --no-pager | grep -E 'diag|TCI websocket|safety outbound|remote_backpressure' || true
    echo
    echo "## saturn-go recent logs"
    journalctl -u saturn-go.service -n 5 --no-pager || true
  } >> "$out_file"

  if (( i < samples )); then
    sleep "$interval_sec"
  fi
done

echo "capture_file=$out_file"
