# Notified Direct XDMA reader wait candidate

The operational C2H reader retains its FIFO precheck, read sizes, sole-reader
ownership, scheduling and framing rules. Only the timer used when insufficient
data is available changes: RX requests at most 500 us; active managed/passive
feedback retains 250 us. These are requested timer intervals, not scheduler or
hardware completion deadlines. `SATURN_BRIDGE_XDMA_RX_FIFO_WAIT_US=250` selects
the shorter RX interval; only 250 and 500 are accepted before hardware is opened.
The isolated phase-2 probe continues to use its existing 250 us sleep.

Stop, a new layout-pause serial, and completed layout resume notify that same
reader. The shared owner's existing coalesced wake primitive moves unchanged
into `thread_wake.rs`; TX mailbox callers keep the same re-export. Each owner
has a separate single-consumer instance. A notification publishes no sample,
layout receipt or output authority. Pending hints survive notify-before-register
and notify-before-park; producer notifications do not acquire a consumer lock.

A paused reader requests at most 5 ms rather than repeatedly polling the pause
flag at 250 us. The first pause request wakes it; repeated requests for the same
serial do not create a notification storm. Only the reader acknowledges pause,
after any in-flight read returns. The controller changes the layout only after
that receipt, installs the scope and feedback-rate flag before clearing pause,
then notifies resume. Failed restoration still leaves it paused. Stop stores its
flag before notifying and joins before the existing disable/reset/safe-RX work.
Startup timeout similarly stores stop and notifies before join.

Notifications interrupt these user-space timer waits; they do not interrupt an
already submitted synchronous kernel DMA read. The driver's existing C2H timeout
and interrupt mode are unchanged. No unconditional blocking read, driver/FPGA
change, new RF permission, extra acquisition owner, sample retiming, audio
buffering change, or work in the TX loop is introduced.

The feedback flag is a cached atomic installed at the existing layout barrier;
choosing a wait does not add a scope-mutex acquisition to every FIFO poll. RX
waits may be interrupted early. Completed buffers drain without an added wait.
The existing 50 us pool-starvation fallback is unchanged.

## Evidence and acceptance

`shared_acquisition` reports effective FIFO interval, RX cap, feedback interval,
paused timer cap and `reader_wait_mode`. A cumulative `fifo_polls` counter counts
actual existing monitor prechecks. The study tool reports its window rate when
available, and remains compatible with older baseline reports lacking it.
Counters and histogram buckets are checked at each distinct publication, so an
intermediate reset cannot disappear behind a later larger count.

Tests include RX-cap parsing, fixed feedback interval, the real reader loop with
file-backed registers and a deliberately 30-second test wait (stop/pause must
notify rather than expire it), normal session stop/join, retained pre-registration
notification, and actual layout pause/resume. Existing serial/generation guards,
readback failure, failed restoration and mailbox lifecycle tests remain required.
The long fixture wait is not reachable from production configuration and is
protected by a separate one-second cleanup watchdog. No test opens a device.

Use serial 60-second installed-release receive-only windows without overlapping
builds/validator browsers. Compare frequency, rate, client/audio/IQ demand, SATP
traffic, scheduler, affinity and governor. Require advancing data, no new observed
header errors/host drops/discontinuities, and review completion/queue-age tails.
The 500 us RX policy is a candidate until those results are reviewed; no gain is
claimed from different client or SATP workloads. The previous release remains
available through managed activation for rollback. Do not clear faults or disable
features to manufacture a passing measurement.

RX-only throughput does not qualify the higher-rate PS feedback layout. Native
P2/Direct file/loopback tests cover preparation, release, restoration and native
cleanup; physical RF/IMD and stalled-kernel-read cancellation remain separate
claims. See the retained `Saturn_XDMA_Notified_Wait_20261116` evidence for actual
ARM build, activation and comparison outcomes.

The longer ignored Auto ATT runtime fixture remains an open baseline finding:
both unchanged `76c6dc1f` and this candidate reproduce AutoCorrection stage-3
feedback-ingress QueueOverrun in the file/loopback harness. This harness does not
run the hardware reader loop. Do not count it as passing or call the full Auto
ATT suite clean. Its original failure and serial baseline/candidate comparison
are retained alongside the passing reader, release/rearm, WAN RX, acquisition-error
cleanup and rear-PTT tests. No RF was used to investigate it.
