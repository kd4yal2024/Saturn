# Shared TX owner idle notifications

The shared owner parks only after an empty, inactive (`ReadyOff`) turn. A common
wake hint connects command, PCM/SATP audio and feedback mailboxes. Cancellation
and newly latched faults also notify it. The idle timer is capped at 5 ms and
at the caller's keepalive period. Active, arming and cleanup turns retain the
existing active wait; work drains without an extra idle delay. Legacy timing
and standard-channel fixtures retain their 1 ms idle fallback.

## Ordering and ownership

One atomic word tracks pending notification and a parked consumer. A producer
publishes its original envelope, releases the queue lock and then notifies.
Notifications coalesce and never dequeue, replace, restamp or authorize data.
Rust's retained unpark token handles notification between arming and parking.
The active receiver still uses try_lock and cannot wait on a paused producer.
The final sender drops its live reference before waking the disconnected reader.

Release closes the output fence before waking. Fault publication notifies again
after the fault mutex is released, covering a consumer that runs between those
two steps. Runtime shutdown stores its external stop flag and explicitly unparks
the worker after release, so a release notification cannot race ahead of stop.
The worker still completes authority/native cleanup and joins before success.

The mailbox wake hint registers one owner thread. Shared construction connects
all three mailboxes to that same hint; older non-notifying test channels do not
silently receive the longer wait. No new thread, sample queue, dependency, RF
permission or output owner is introduced.

## Validation

Tests cover pre-registration notification without fabricated messages, each
shared mailbox at the armed-before-park boundary, visible last-sender disconnect,
FIFO/capacity/original-age handling, paused-producer contention and payload drop
outside the queue lock. Scheduler checks prohibit notification waits on active
or worked turns and verify external stop after an idle wakeup.

Real-WDSP production-runtime fixtures cover P2 and Direct XDMA receive/release/
re-arm/active shutdown, rear PTT, PS-off microphone input, band recall, SATP
pair/renew/source changes, SATP startup failure and acquisition-error restoration.
These use files/loopback sockets, not physical RF.

The ignored `matched_production_idle_owner_profile_both_backends` uses the real
production constructor and native engine. It samples the live owner thread's CPU
clock over three idle seconds, requests shutdown and requires completed cleanup,
joined worker, restored acquisition and rejection of a second Legacy native owner.
Fixture owners use CFS; they do not claim installed FIFO scheduling or physical
key/release latency. Run it serially with the reviewed native archive:

```sh
cargo test --locked --features managed-puresignal --bin saturn-bridge \
  matched_production_idle_owner_profile_both_backends -- --ignored --nocapture --test-threads=1
```

Four interleaved runs per version on the local x86 native fixture gave:

| Backend | Idle CPU median, baseline / candidate (one core) | Complete shutdown median, baseline / candidate |
| --- | --- | --- |
| P2 | 1.834% / 0.840% | 164.636 / 163.224 ms |
| Direct XDMA | 1.761% / 0.838% | 93.881 / 94.531 ms |

Direct XDMA's sample shutdown maxima were 95.047 / 95.782 ms. All runs completed
within the existing fixture deadline. Four samples are not p99, and these values
do not establish a cleanup latency improvement. CPU frequency was not fixed.
Installed G2 CPU and receive-validation evidence is recorded separately in
`Downloads/Saturn_Shared_Owner_Wake_20261115`.

## XDMA reader investigation

The current `linuxdriver/xdma/cdev_sgdma.c` file operations expose synchronous
read/read_iter and ioctl, with no poll handler. A C2H read submits to
`xdma_xfer_submit` with the driver's C2H timeout; `libxdma.c` waits for transfer
completion or timeout. The application's FIFO precheck and reader pause protocol
therefore matter for stop and feedback-layout transitions. Socket-style readiness
is not established for this interface.

This slice retains the 250 us hardware FIFO poll, DMA sizing, driver, read-to-clear
register ownership and feedback pause/restoration barriers. A reader change needs
measured FIFO refill/headroom at the actual RX/feedback rates plus cancellability
and stop tests before a new wait policy can be qualified. The separate main
acquisition 1 ms control wait is also retained. Earlier display underruns,
worker-3D tail observations and hosted CI availability remain separate open work.
