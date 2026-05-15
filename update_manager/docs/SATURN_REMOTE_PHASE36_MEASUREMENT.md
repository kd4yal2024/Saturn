# Saturn Remote Phase 36 Measurement Runbook

Phase 36 is measurement only. Do not add auto-rate logic, server-side FFT, or
new operator UI until these captures identify stable thresholds.

## Goal

Measure whether the Phase 35 scheduler keeps control and safety latency bounded
while raw IQ display load changes across network paths.

The provisional Phase 35 bars remain:

- `safety_enqueue_to_write_us` p99 <= 5 ms.
- `control_text_enqueue_to_write_us` p99 <= 20 ms.
- Bridge IQ processing rate within +/-2% of the no-slow-client baseline.

## Matrix

Run all six normal-client sessions:

| Network | IQ Rate | Audio | Duration |
| --- | ---: | --- | ---: |
| LAN | 48 kHz | 48 kHz on | 5 min |
| LAN | 96 kHz | 48 kHz on | 5 min |
| LAN | 192 kHz | 48 kHz on | 5 min |
| VPN | 48 kHz | 48 kHz on | 5 min |
| VPN | 96 kHz | 48 kHz on | 5 min |
| VPN | 192 kHz | 48 kHz on | 5 min |

Then run one slow-client overload case on LAN and one on VPN at 192 kHz.

## Browser Capture

Use the test browser on `/remote-next`. For each session:

```js
window.SaturnRemotePerf.clear()
window.SaturnRemotePerf.start()
```

After the run:

```js
window.SaturnRemotePerf.stop()
window.SaturnRemotePerf.downloadJson()
window.SaturnRemotePerf.downloadSummary()
```

The Phase 36 browser summary includes:

- frame-rate min/avg/max.
- max IQ idle time.
- max bridge RTT.
- max safety/control enqueue-to-write p99.
- total display replacements/drops.
- total bridge audio drops and panic drains.
- browser and bridge audio sequence gap counters.
- total send-blocked milliseconds.
- max outbound high-watermark bytes.
- total safety queue depth overflow count.

## Host Capture

Start the matching host capture from the G2 SSH shell:

```bash
/home/pi/github/Saturn/update_manager/scripts/saturn-remote-phase36-capture.sh 300 1 lan-96k
```

Use labels like:

- `lan-48k`
- `lan-96k`
- `lan-192k`
- `vpn-48k`
- `vpn-96k`
- `vpn-192k`
- `lan-192k-slow-client`
- `vpn-192k-slow-client`

The script writes logs under:

```text
~/Documents/perf-captures/phase36/
```

Each host capture records process CPU/RSS, TCP queues for HTTPS and bridge TCI,
endpoint timing for `/remote-next` and `remote-next.js`, and recent bridge diag
lines containing the Phase 35 counters.

## Recording Results

For each session, keep these files together:

- Browser `SaturnRemotePerf` JSON.
- Browser `SaturnRemotePerf` summary.
- Host capture log.
- Notes about device, browser, network path, and whether the tab was visible.

Do not interpret one run as a threshold. Phase 37 should use the measured LAN
and VPN distributions, not single spikes.
