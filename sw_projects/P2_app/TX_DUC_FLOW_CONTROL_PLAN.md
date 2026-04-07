# TX DUC Flow-Control Plan

Date: 2026-03-23

## Problem

The hardened `p2app` TX DUC traffic is sensitive to short bursts of scheduler jitter,
socket burstiness, and XDMA write timing because the client-to-app-to-XDMA path
has very little deliberately managed elasticity:

- the client sends live TX I/Q over UDP
- `InDUCIQ.c` receives packets
- `InDUCIQ.c` writes queued payload into the TX DUC FIFO through XDMA

Before this pass, ingress and DMA writing were still coupled inside one worker
thread. That meant:

- while waiting for FIFO space or doing `pwrite(...)`, the app was not draining
  the UDP socket
- the kernel socket buffer carried most of the jitter-absorption burden
- the app had only a small short-lived batch, not a real bounded reserve
- there was no explicit stale-frame drop policy when backlog aged out

## Target Architecture

The intended steady-flow shape is:

1. UDP ingress drains packets as soon as Linux wakes the app.
2. Payloads are placed into a bounded software queue.
3. A dedicated XDMA writer keeps the hardware FIFO inside a stable occupancy
   band and drains the software queue in bounded bursts.
4. If backlog exceeds the allowed live-latency budget, oldest TX frames are
   dropped explicitly rather than transmitted late.

This is not true end-to-end backpressure because HPSDR/Thetis UDP TX does not
currently expose queue-credit pacing from the app back to the client. Within
the existing protocol, the best practical strategy is:

- absorb short bursts in Linux socket buffers plus a bounded app queue
- preserve FIFO reserve with a dedicated writer
- drop stale frames when software backlog exceeds the live-TX age budget

## First Increment Implemented

The first code slice keeps the external `IncomingDUCIQ(...)` entry point but
changes its internals to a two-stage model:

- ingress thread:
  - `recvmmsg(...)`
  - sequence/gap accounting
  - 24-bit I/Q byte-swap into app-owned queue frames
  - bounded queue insert with oldest-frame drop on overflow
- writer thread:
  - owns the XDMA TX handle
  - monitors TX DUC FIFO occupancy
  - keeps the FIFO in a target reserve band
  - drains the queue in bounded DMA bursts
  - drops oldest frames once queued age exceeds the stale-TX limit

## Validation Focus

For this increment, the main things to watch are:

- `/p23_perf` counters:
  - `duc_gap_events`
  - `duc_gap_dropped_frames`
  - `duc_queue_drop_events`
  - `duc_queue_dropped_frames`
  - existing DUC DMA/underflow counters
- `/p23_perf` gauges:
  - `duc_queue.last_queue_frames`
  - `duc_queue.last_fifo_frames`
  - `duc_queue.last_queue_age_us`
  - `duc_queue.last_mode`
- field behavior:
  - reduced TX underruns under bursty client or scheduler jitter
  - no unwanted TX latency growth under steady load

## Likely Next Steps

- tune queue depth / age limits with live `p23test` telemetry
- consider per-client or per-mode TX age thresholds
- if protocol changes are acceptable later, add explicit queue-credit feedback
  so the client can pace TX toward the app instead of relying only on app-side
  smoothing and stale-drop control
