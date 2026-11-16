# XDMA reader wait study

This slice publishes existing Direct XDMA acquisition statistics through the
shared owner's one-second report. It does not change FIFO polling (250 us),
DMA sizes, queue policy, driver, scheduling, RX/TX ownership or RF permissions.
The reader still performs no JSON formatting, disk I/O or new sample copying.
The publication owner clones fixed-size statistics under the existing mutex,
then formats outside that lock, once for both status destinations. P2 and an
unavailable acquisition source publish `shared_acquisition: null`.

## Collecting evidence

Use the running owner's reports; never open another C2H reader or externally
read the clear-on-read FIFO register. This collector reads only JSON files and
writes its explicitly selected output file:

```sh
python3 update_manager/scripts/saturn-xdma-wait-study.py \
  --seconds 60 --output /tmp/xdma-wait-study.json
```

It accepts 2–600 seconds and 0.25–5 second sampling intervals. It retains distinct
publications (at most 2401 observations), rejects stale or regressing clocks,
changed PID/build/source, other backends/owners and active/unknown TX state, and
preserves partial evidence on failure. It never stops, keys or reconfigures the
radio. Radio/client changes remain visible and mark the window nonstationary.
Collection completion is not a hardware acceptance or longer-wait approval.

Reports include DMA read/byte rates, framing and host-loss counter deltas,
latency histogram bucket deltas, session FIFO high-water, and the last owner
FPGA snapshot. Histogram durations round up to microseconds; bucket i covers up
to 2^i us except the last, open-ended bucket. Window p95/p99 are bucket upper
bounds, null when empty or in the open bucket. A session maximum is never
subtracted or relabeled as a window maximum. Dequeued-buffer queue-age samples
exclude reclaimed buffers, so drop counters remain necessary.

FPGA min/max/event counters span FPGA boot lifetime. Owner session counters
can include startup and multiple feedback layouts. The reported FPGA snapshot
has unknown acquisition age, even when its generation/valid bit is retained.
One-second snapshots do not establish sub-second FIFO headroom. Zero sampled
host events do not prove that unavailable ADC/FPGA overload coverage was clean.
Fresh SATP endpoint counters are collected in the same service window, avoiding
the previous slice's uncontrolled background-traffic comparison.

Run alongside the existing read-only `saturn-thread-profile.py` when CPU,
scheduler, priority, affinity and governor observations are needed. Keep builds
and receive-validator browser sessions out of measurement windows.

## Nominal geometry, not measured headroom

The checked-in DDC FIFO IP has depth 16,384 64-bit words. The DDC6-only 384 kS/s
layout emits eight IQ words plus one header per frame: 3,456,000 bytes/s, or
432,000 words/s. Its nominal entire-FIFO fill time is 37.926 ms; the 4,096-byte
minimum block fills in 1.185 ms. These are source calculations for that layout,
not a scheduling budget, a measured worst case, or a guarantee for PS feedback.
The configured monitor threshold must not be mistaken for physical capacity.

## Cancellation boundary

`cdev_sgdma.c` supplies synchronous read/read_iter and ioctl but no `.poll`.
A read calls `xdma_xfer_submit`; `libxdma.c` waits for completion or timeout
(default C2H module timeout 10 seconds). The driver's wait is interruptible, but
the application has no implemented reader-thread interrupt/cancel transaction.
Closing a different descriptor or setting the Rust stop flag is not such a
transaction. `OperationalRxSession::stop` sets its flag, joins the reader, and
only then disables/resets the stream. An outstanding read must therefore
complete before this normal stop can progress.

`ReaderGate` acknowledges layout pause only at the reader's next loop boundary,
after any read returns. Tests retain this invariant: a pause request cannot
acknowledge/cancel an in-flight read; an old serial cannot authorize a new layout;
failed layout writes stay paused; release and restoration remain generation
fenced. File/loopback tests do not establish kernel cancellation latency.

Replacing the FIFO precheck with an unconditional read would add a new blocking
point to those boundaries. This slice deliberately does not do that. A future
wait candidate needs measured refill/headroom under the actual layouts, read
completion tails, control wake/stop deadlines, and explicit stalled-read recovery
before live promotion. Preserve the existing independent TX release authority.
