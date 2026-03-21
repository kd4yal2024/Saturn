# P3 Audio Lab Handoff

Date: 2026-03-20

## Purpose

This document preserves the current `P3` audio/runtime baseline, the lab
instrumentation state, the most important test findings, and the next test to
run if work is interrupted.

## Stable Baseline

- Stable baseline commit: `85349c8`
- Commit message: `Establish P3 audio baseline and lab telemetry`

That baseline includes:

- P3 inbound socket buffer, prefill, and `recvmmsg(...)` batching work
- optional RT tuning for speaker/DUC threads
- Service Lab runtime capture for `p2app.service`
- `p23test` snapshot capture improvements
- sequence-aware speaker gap handling
- speaker underrun-context telemetry

## Current Live Runtime

As of this handoff, the deployed app is `P3` via:

- `/opt/saturn-go/p23-apps/current -> /opt/saturn-go/p23-apps/p3app`
- service: `p2app.service`

Realtime audio profile in use:

- `SATURN_P3_RT_AUDIO_ENABLE=1`
- `SATURN_P3_RT_AUDIO_POLICY=rr`
- `SATURN_P3_RT_AUDIO_PRIORITY=10`
- `SATURN_P3_RT_AUDIO_CPUS=2-3`

Important live telemetry sources:

- `/dev/shm/saturn_p23_perf_stats.json`
- `http://127.0.0.1:8080/p23_perf`
- `http://127.0.0.1:8080/p23_status`
- `/saturn/p23test.html`

## Key Findings

### 1. RX transport improvements worked

The socket-buffer, prefill, and `recvmmsg(...)` work produced real gains:

- DUC underruns dropped to `0` in the good runs
- XDMA efficiency improved materially versus the older path
- dual-RX scaled correctly
- speaker/DUC batching ratios improved

### 2. RT scheduling is safe but not the main win

The RT profile did not show a dramatic throughput or underrun breakthrough by
itself. It is worth keeping available, but it is not the primary explanation
for the improvements.

### 3. Blind speaker threshold tuning plateaued

Several attempts to improve speaker underruns by only changing queue depth or
prefill thresholds did not produce a durable normalized win.

Conclusion:

- keep threshold tuning conservative
- do not keep chasing speaker underruns only by changing constants

### 4. Timer-only speaker gap detection was wrong

A time-based speaker gap detector caused false discontinuities.

Bad result that was observed:

- `speaker_gap_events=27919`
- `speaker_gap_dropped_frames=74998`
- `speaker_silence_frames=74992`

Conclusion:

- do not reintroduce timer-only speaker gap detection

### 5. Sequence-aware speaker gap detection is correct

Using the packet sequence in bytes `0..3` of the speaker UDP packet fixed the
false-gap behavior.

What improved:

- gap events dropped to low single digits or zero in good runs
- dropped speaker frames went to `0`
- silence insertion stayed small and controlled

Conclusion:

- keep the sequence-aware speaker discontinuity logic

### 6. Remaining speaker underruns are not empty-input starvation

The most important telemetry result so far:

- `speaker_underrun_queue_empty_events=0`
- `speaker_underrun_queue_ready_events>0`

In the best stable rollback run we captured:

- `fifo_speaker_under_events=25`
- runtime about `4619s`
- about `19.5 underruns/hour`
- `speaker_gap_events=0`
- `speaker_stall_events=1`
- `speaker_underrun_queue_empty_events=0`
- `speaker_underrun_queue_ready_events=25`
- last underrun context:
  - `last_mode="emergency"`
  - `last_fifo_frames=0`
  - `last_queue_frames=6`
  - `last_queue_age_us=7539`
  - `last_gap_active=false`

Conclusion:

- the remaining underruns happen while speaker audio is already queued
- the last-mile problem is in speaker write timing / cadence, not packet loss

### 7. A full queue-first speaker loop was a regression

A broader change that prioritized draining queued speaker data before receiving
more caused a large CPU spike and much worse DMA cadence.

Observed bad behavior:

- CPU jumped to roughly `74-86%`
- `speaker_dma_writes` almost matched `speaker_packets` one-for-one
- XDMA IRQ pressure rose sharply

Conclusion:

- do not restore the broad queue-first loop ordering

## Best Stable Result So Far

Use this as the most trustworthy tested reference point:

- normal CPU, roughly in the `~18-24%` range depending on momentary load
- DUC underruns stayed `0`
- sequence gap handling stable
- best stable speaker underrun rate seen so far was about `19.5/hour`
- speaker batching stayed healthy:
  - `speaker_packets / speaker_dma_writes` about `3.8`

This is the baseline to compare against for future speaker-side changes.

## Confirmed Improvement After Baseline

After baseline `85349c8`, one narrow speaker-side improvement was tested,
validated, and committed as `a001dd1`:

- commit: `a001dd1`
- commit message: `Reduce queue-ready speaker underruns`

- file: `sw_projects/P3_app/InSpkrAudio.c`
- summary:
  - emergency-only fast path
  - if queued speaker data exists and the last observed hardware speaker FIFO
    was already in the emergency band, skip the normal receive wait and move
    more quickly toward the next speaker DMA write

This change is intentionally narrow:

- it keeps the sequence-aware gap handling
- it keeps the underrun-context telemetry
- it does not restore the broad queue-first loop

Initial validated result from the clean dual-RX run:

- `fifo_speaker_under_events=14`
- runtime about `3654s`
- about `13.8 underruns/hour`
- prior stable rollback baseline was about `19.5/hour`
- improvement is about `29%`
- CPU did not regress into the failed `74-86%` range
- `fifo_duc_under_events=0`
- `speaker_underrun_queue_empty_events=0`
- `speaker_underrun_queue_ready_events=14`
- `speaker_gap_dropped_frames=0`

Longer confirmation on the same live PID (`572826`) was stronger:

- later runtime about `9174s`
- `fifo_speaker_under_events` stayed at `14`
- `speaker_underrun_queue_ready_events` stayed at `14`
- no new speaker underruns for about `92` additional minutes
- whole-run normalized rate improved to about `5.5 underruns/hour`
- `fifo_duc_under_events=0`
- `speaker_gap_dropped_frames=0`
- `speaker_stall_events=1`
- CPU remained normal, with current sample at `16.0%`

Conclusion:

- treat `a001dd1` as the current RX candidate
- steady-state dual-RX no longer looks like the main problem
- remaining events are more likely clustered around startup or RX transitions

This is the current preferred runtime state after the baseline commit.

## Next Test To Run

Do not keep tuning steady-state RX blindly. The next useful work is a
startup/transition test matrix against committed build `a001dd1`.

Test setup:

- `P3`
- `panel=auto`
- RT audio profile enabled
- `RX1` and `RX2` both active
- both DDCs at `384 kHz`
- restart `p2app.service`
- capture one snapshot at `2-5 minutes`
- capture one snapshot at `30-60 minutes`
- if practical, toggle `RX1/RX2` or perform the transition that matters most
  in normal use

Success criteria:

- startup or transition events are bounded and identifiable
- later steady-state run stays flat again
- CPU remains near the normal range, not the old `74-86%` regression
- `fifo_duc_under_events=0`
- `speaker_underrun_queue_ready_events` does not continue climbing during the
  long steady-state portion
- `speaker_underrun_queue_empty_events` remains `0`
- `speaker_gap_events` stays low and does not correlate with dropped frames
- `speaker_stall_events` remains small

Key fields to inspect in the next snapshot:

- `fifo_speaker_under_events`
- `speaker_underrun_queue_ready_events`
- `speaker_underrun_queue_empty_events`
- `speaker_gap_events`
- `speaker_stall_events`
- `speaker_dma_writes`
- `speaker_packets`
- `gauges.speaker_underrun`
- `cpuPctCore`
- `xdmaIrqPerMiB`

If the early snapshot takes the hit and the later one stays flat, the next code
change should be a narrow startup/routing-change prefill or grace path, not
another steady-state speaker loop rewrite.

## Useful Commands

Check deployed service:

```bash
systemctl status p2app.service --no-pager
systemctl show -p Environment --value p2app.service
```

Check lab status:

```bash
curl -fsS http://127.0.0.1:8080/p23_perf
curl -fsS http://127.0.0.1:8080/p23_status
```

Restart P3 service:

```bash
sudo -n systemctl restart p2app.service
```

Deploy rebuilt P3 binary:

```bash
sudo -n install -m 0755 /home/pi/github/Saturn/sw_projects/P3_app/p3app /opt/saturn-go/p23-apps/p3app
```

## Files That Matter Most

P3 runtime:

- `sw_projects/P3_app/InSpkrAudio.c`
- `sw_projects/P3_app/InDUCIQ.c`
- `sw_projects/P3_app/p2app.c`
- `sw_projects/P3_app/threaddata.h`
- `sw_projects/P3_app/CHANGELOG.md`

Telemetry:

- `sw_projects/common/p23_perf_telemetry.h`
- `sw_projects/common/p23_perf_telemetry.c`

Service Lab:

- `update_manager/rust-server/src/main.rs`
- `update_manager/templates/p23test.html`
- `update_manager/docs/OPERATIONS_RUNBOOK.md`

## Dirty Worktree Notes

These unrelated local changes were present and intentionally excluded from the
audio baseline work:

- `linuxdriver/tools/*`
- `sw_tools/load-FPGA/load-FPGA`

Do not treat those as part of the P3 audio tuning baseline unless explicitly
requested.
