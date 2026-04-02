#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  protocol2_regression_summary.sh <capture.pcap>

Reads a Protocol 2 regression capture with tcpdump and prints a simple flow
summary:
  - UDP source port
  - UDP destination port
  - UDP payload length
  - packet count
  - duration between first/last packet in the flow
  - average/min/max inter-packet gap in milliseconds
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'ERROR: required command not found: %s\n' "$1" >&2
    exit 1
  }
}

[[ $# -eq 1 ]] || { usage >&2; exit 2; }
[[ "$1" == "-h" || "$1" == "--help" ]] && { usage; exit 0; }
PCAP_PATH="$1"
[[ -f "$PCAP_PATH" ]] || { printf 'ERROR: pcap not found: %s\n' "$PCAP_PATH" >&2; exit 2; }

need_cmd tcpdump
need_cmd awk
need_cmd sort

printf 'Protocol 2 regression summary\n'
printf 'Capture: %s\n\n' "$PCAP_PATH"

tcpdump -tt -nn -r "$PCAP_PATH" 2>/dev/null | awk '
function port_of(endpoint, count, parts) {
  count = split(endpoint, parts, ".");
  return parts[count];
}

/^[0-9]+(\.[0-9]+)?[[:space:]]+IP / && / UDP, length / {
  ts = $1 + 0.0;
  src = $3;
  dst = $5;
  sub(/:$/, "", dst);
  src_port = port_of(src);
  dst_port = port_of(dst);
  len = $NF + 0;
  key = src_port "->" dst_port " len=" len;

  count[key]++;
  if (!(key in first_ts))
    first_ts[key] = ts;
  last_ts[key] = ts;

  if (key in prev_ts) {
    gap_ms = (ts - prev_ts[key]) * 1000.0;
    gap_count[key]++;
    gap_sum[key] += gap_ms;
    if (!(key in gap_min) || gap_ms < gap_min[key])
      gap_min[key] = gap_ms;
    if (!(key in gap_max) || gap_ms > gap_max[key])
      gap_max[key] = gap_ms;
  }
  prev_ts[key] = ts;
}

END {
  if (length(count) == 0) {
    print "No UDP flow records found.";
    exit 1;
  }

  print "Packets      Flow                    Duration    Gap avg    Gap min    Gap max";
  print "-----------  ----------------------  ----------  ---------  ---------  ---------";
  for (key in count) {
    duration = last_ts[key] - first_ts[key];
    avg_gap = (gap_count[key] > 0) ? (gap_sum[key] / gap_count[key]) : 0.0;
    min_gap = (gap_count[key] > 0) ? gap_min[key] : 0.0;
    max_gap = (gap_count[key] > 0) ? gap_max[key] : 0.0;
    printf "%011d  %-22s  %10.3fs  %8.3fms  %8.3fms  %8.3fms\n",
           count[key], key, duration, avg_gap, min_gap, max_gap;
  }
}
' | sort -k2,2 -k3,3n
