# Changelog

All notable changes to `P3_app` from this hardening pass are documented here.

## [2026-03-23] TX DUC Queue Decoupling And Telemetry

### Changed

- Reworked `InDUCIQ.c` so UDP ingress and TX XDMA writes are now decoupled by a
  bounded software queue instead of sharing one combined receive/write loop.
- Added an explicit oldest-frame drop policy for queued TX frames so stale live
  TX does not continue to accumulate latency when backlog exceeds the software
  age budget.
- Added TX DUC sequence-gap accounting and queue/drop telemetry so `/p23_perf`
  can show when the client stream skipped ahead or the app had to shed queued
  TX frames to stay current.

### Added

- Added `/p23_perf` DUC queue gauges for:
  - last queued frame count
  - last observed TX FIFO frame depth
  - last queue age in microseconds
  - last write mode (`normal`, `prefill`, `emergency`)

### Notes

- This is the first flow-control increment toward a steadier client -> P3 ->
  XDMA path.
- It deliberately stays within the current UDP protocol semantics; there is
  still no true end-to-end backpressure to the client yet.

## [2026-03-21] Speaker Startup Refill Timing and PureSignal Telemetry

### Changed

- Reset the cached speaker FIFO estimate on SDR activation/deactivation and
  added a short activation grace window that bypasses the queued-audio receive
  wait while the speaker reserve settles, so startup no longer relies on a
  stale healthy FIFO estimate before the first real occupancy read.
- Added a live `pure_signal_enabled` flag to the shared `/p23_perf` app
  telemetry export so lab snapshots can distinguish plain TX from PureSignal
  TX without relying on client-side UI state.

## [2026-03-20] Audio Baseline, Realtime Tuning, and Queue-Ready Underrun Reduction

### Changed

- Tightened inbound speaker/DUC batching decisions so they refresh real FIFO
  occupancy before deferring a DMA write, instead of relying on the previous
  write's cached fill estimate.
- Added a low-water fast-flush path for speaker and DUC so prefill mode stops
  waiting for a deep batch when the hardware FIFO is already near empty.
- Increased the speaker-only prefill reserve band and made low-water speaker
  flushes more aggressive after dual-RX testing showed the speaker path still
  needed more cushion than the TX DUC path.
- Replaced the speaker path's short-lived batch with a bounded software reserve
  queue so queued speaker audio can survive scheduler jitter across loop
  iterations and DMA writes can be sized from real FIFO need instead of only
  from the latest socket wakeup.
- Added sequence-aware speaker discontinuity handling so short network jitter
  no longer looks like a stream gap. The speaker thread now bridges real packet
  sequence jumps with bounded silence in its software reserve, and only falls
  back to silence refill after a much longer true empty-queue stall.
- After testing a deeper steady-state speaker reserve band, returned to the
  earlier sequence-gap baseline when the larger reserve did not reduce the
  normalized dual-RX underrun rate.
- Added speaker underrun-context telemetry so snapshots now distinguish
  sequence gaps from true empty-queue stalls and record whether each new
  speaker underrun happened with queued audio already available, along with the
  last underrun's queue depth, FIFO depth, queue age, and write mode.
- Added an emergency-only speaker fast path that skips the receive wait when
  the last observed hardware speaker FIFO was already near empty and queued
  audio is ready, so the remaining queue-ready underruns can be attacked
  without reintroducing the earlier high-CPU loop ordering regression.

### Added

- Added opt-in realtime tuning for the two most timing-sensitive inbound worker
  threads:
  - speaker audio (`InSpkrAudio.c`)
  - TX DUC I/Q (`InDUCIQ.c`)
- New environment knobs:
  - `SATURN_P3_RT_AUDIO_ENABLE=1`
  - `SATURN_P3_RT_AUDIO_POLICY=rr|fifo|other`
  - `SATURN_P3_RT_AUDIO_PRIORITY=<n>`
  - `SATURN_P3_RT_AUDIO_CPUS=<cpu-list>`
- The app now parses that profile once during startup and applies it from
  inside the speaker / DUC thread contexts so CPU affinity can target the
  running thread directly and scheduler failures degrade safely with a log
  message instead of aborting startup.

### Notes

- This is intentionally off by default.
- `SCHED_RR` is the intended first test mode; `SCHED_FIFO` is available for
  controlled experiments but should be used more carefully.
- CPU lists accept comma-separated cores and simple ranges such as `2` or
  `2-3`.

## [2026-03-19] Speaker/DUC Socket and Prefill Tuning

### Changed

- Increased UDP socket buffer sizing in `MakeSocket(...)` so active P3 ports can
  absorb short bursts of scheduler or network jitter with less packet loss
  pressure.
- Reworked `InSpkrAudio.c` and `InDUCIQ.c` to receive inbound frames with
  `recvmmsg(...)` and stage them in a short-lived software queue before DMA, so
  batching can span multiple wakeups instead of being limited to one syscall.
- Added speaker and DUC prefill hysteresis plus short queue-age flush limits so
  startup and underrun recovery aim for a healthier FIFO occupancy band without
  allowing unbounded extra latency.

### Verified

- Clean rebuild completed successfully with:
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app -j2`
- Live `/p23_perf` validation on `/saturn/p23test` showed:
  - CPU returned to the normal `~16%` per-core range after replacing the first
    spin-heavy queue attempt with blocking age-budget waits
  - XDMA efficiency improved to roughly `593 IRQ/MiB` current and `548 IRQ/MiB`
    average over a multi-hour baseline
  - DUC underruns remained at `0`
  - speaker underruns fell to rare events (`11` over about `3.7` hours)

## [2026-03-18] P23 Workload Telemetry and Underrun Episode Tracking

### Added

- Added shared `/p23_perf` app telemetry export for P3 worker activity,
  including per-path packet/byte/error counters, DMA counters, runtime flags,
  active port assignments, feature flags, and wideband configuration snapshots.
- Wired speaker, DUC, DDC, mic, wideband, and high-priority paths into the P23
  telemetry counters so lab runs can correlate underruns, socket errors, FIFO
  state, and DMA behavior from the app itself.

### Fixed

- Fixed ADC peak telemetry export ordering so `/p23_perf` snapshots capture the
  live peak values before the per-frame peak-hold counters are reset.
- Added dependency-file generation to the `Makefile`, included
  `p23_perf_telemetry.c` explicitly in the build, and ignored generated `.d`
  files so telemetry-source changes trigger correct incremental rebuilds.

### Changed

- Speaker and DUC FIFO underruns are now counted as distinct starvation
  episodes instead of incrementing continuously while the same underflow
  condition persists.

## [2026-03-18] DMA and Wideband Stability Hardening

### Fixed

- Fixed wideband buffer lifetime and cleanup so the wideband worker now frees
  only the two ADC packet buffers it actually owns, avoiding out-of-bounds
  frees during shutdown.
- Fixed DUC I/Q and speaker-audio worker startup so failed buffer allocation or
  failed XDMA device opens now abort thread startup cleanly instead of
  continuing with invalid pointers or file descriptors.
- Fixed DDC outgoing batching so partial `sendmmsg(...)` completions no longer
  silently discard unsent frames.
- Fixed wideband and mic outgoing paths so short or failed `sendmsg(...)`
  results now raise thread errors instead of being ignored.

### Changed

- Hardened low-level XDMA access wrappers to reject invalid file descriptors or
  buffers and to treat short `pread(...)` / `pwrite(...)` transfers as errors.
- Added fail-fast startup behavior in `p2app.c` when `/dev/xdma0_user` cannot be
  opened for register access.
- Propagated DMA transfer failures into the active mic, DDC, DUC, speaker, and
  wideband worker threads so datapath corruption does not continue silently.

### Verified

- Clean rebuild completed successfully with:
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app clean`
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app -j2`

## [2026-03-16] ADC Peak Telemetry in High-Priority Status

### Changed

- Extended the 60-byte outgoing high-priority status packet to include:
  - ADC1 peak amplitude at bytes `39..40`
  - ADC2 peak amplitude at bytes `41..42`
- `P3_app` version was bumped from `44` to `45`.
- Shared register access now returns optional ADC peak amplitudes together with
  the existing ADC overflow bits.

### Compatibility

- Existing ADC overflow reporting in status byte `5` is unchanged.
- Peak amplitude fields are populated only when FPGA firmware version `27` or
  newer exposes the ADC peak registers.
- Older FPGA builds continue to report `0` for the new peak fields.

## [2026-03-12] Datapath Batching and Build Hygiene

### Changed

- Enabled optimized `P3_app` builds with `-O2 -g` as the default build mode for
  deployed test binaries.
- Batched queued TX DUC I/Q writes so `InDUCIQ.c` can drain multiple ready
  frames into a single larger XDMA write when FIFO space allows.
- Batched queued speaker-audio writes so `InSpkrAudio.c` reduces tiny XDMA
  transactions under steady packet load.
- Batched outgoing DDC UDP sends with `sendmmsg(...)` so `OutDDCIQ.c` can emit
  multiple ready frames per socket with lower syscall overhead.
- Reworked the DDC packed-sample decode hot loop so 48-bit I/Q samples are
  copied as contiguous 6-byte payloads instead of repeated 16-bit scalar moves
  plus a skipped pad word.

### Fixed

- Cleared remaining `P3_app` compiler warnings seen during `p23-app-manager.sh`
  deploy builds:
  - interface-name copy in `p2app.c`
  - uninitialized `DataBit` path in `common/saturndrivers.c`
  - uninitialized `FrameLength` / startup counters in active DMA worker paths

### Verified

- Clean rebuild completed successfully with:
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app clean`
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app -j1`

## [2026-03-11] CAT GUID Safety and Runtime Robustness

### Fixed

- Fixed CAT GUID/string response generation for long payload commands such as
  `ZZGA` / `ZZGR` by sizing the CAT output path for the longest supported
  payload instead of truncating or overrunning it.
- Fixed serial-port setup error cleanup so a failed `tcsetattr(...)` no longer
  leaks an open file descriptor.

### Changed

- CAT message builders now use bounded formatting for no-param, bool, numeric,
  and string commands, and string formatting no longer mutates caller-owned
  buffers.
- SIGINT handling now uses a signal-safe request flag synchronized in the main
  loop, instead of calling stdio from signal context.
- The `P3_app` `Makefile` now selects the libgpiod-specific panel source as a
  distinct object file and sets an explicit default goal, preventing stale
  panel objects from being silently reused across libgpiod major-version
  changes.

### Verified

- Clean rebuild completed successfully with:
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app clean`
  - `make -C /home/pi/github/Saturn/sw_projects/P3_app -j$(nproc)`

## [2026-03-01] Startup Handshake and High-Priority Socket Stabilization

### Fixed

- Fixed startup handshake activation race by re-evaluating activation from the
  main control loop after queued port rebind work, instead of relying on a
  single timing-sensitive activation point.
- Fixed duplicate-general startup behavior so repeated identical general packets
  still refresh startup handshake state (`ReplyAddressSet`) and can complete
  activation with an already-seen run-bit.
- Fixed high-priority incoming thread socket handling for shared-alias sockets:
  - avoid closing owner sockets from alias threads
  - use alias-aware socket resolution for `recvmsg(...)`
  - avoid alias-owner double-close during shutdown

### Changed

- Startup sequencing now consistently performs:
  - queue/apply general control updates
  - queued outgoing rebind processing (while inactive)
  - handshake activation check (`reply + run -> active`)
- This ordering reduces startup timing sensitivity observed during repeated
  Thetis connect/disconnect cycles.

## [2026-02-27] Thetis Startup Compatibility and Diagnostics

### Fixed

- Restored Thetis startup reliability by removing the startup holdoff behavior that delayed
  General-packet application in active startup traffic conditions.
- Startup configuration now applies immediately after queueing (with duplicate-suppression retained).

### Added

- Added one-time startup trace markers to make startup sequencing visible in logs:
  - Discovery received
  - General received
  - General applied
  - High-priority run-bit seen
  - Handshake complete (`reply + run -> active`)

### Changed

- Reduced high-priority log spam by replacing per-packet receive logging with first-stream detection.
- Startup trace state now resets when client stop/inactivity transitions return the app to inactive state.

### Reverted

- Reverted CAT keepalive respawn-throttle guard in `SetupCATPort(...)` after it introduced
  a Thetis startup regression in field testing.

## [2026-02-26] Startup MAC Discovery Hardening

### Fixed

- Fixed startup interface/MAC enumeration in `p2app.c` so the selected interface name is copied before `closedir()` instead of using a `readdir()` entry pointer after directory close.
- Added a clean failure path when no matching Ethernet-style interface is found.
- Added `SIOCGIFHWADDR` error checking during discovery-reply MAC initialization.

### Changed

- Startup interface scan now ignores loopback (`lo`) when selecting the network interface for discovery MAC reporting.

## [2026-02-25] Front Panel Mode Override

### Added

- Added front-panel detection override via environment variable `SATURN_FRONT_PANEL_MODE` in `frontpanelhandler.c`.
- Supported values:
  - `auto` (default)
  - `g2`
  - `g2v2`
  - `prefer-g2`
  - `prefer-g2v2`
  - `off` / `none`

### Changed

- `InitialiseFrontPanelHandler()` now resets detected-panel flags before each initialization and can force/disable panel initialization based on `SATURN_FRONT_PANEL_MODE`.
- Added explicit startup log messages for forced/override panel modes to make field troubleshooting easier when `auto` detection picks the wrong panel path.

## [2026-02-16] Goal Update

### Added

- Documented a forward goal to support optional true independent alias socket ports while preserving late-P2 compatibility for Thetis and piHPSDR.
- Clarified intended behavior model:
  - Shared alias sockets remain the default compatibility path.
  - Independent alias sockets are a future opt-in path when distinct alias ports are explicitly provided.

## [2026-02-11] Safety and Concurrency Hardening

### Fixed

- Corrected shutdown semaphore cleanup:
  - `MicWBDMAMutex` is now destroyed correctly.
  - Removed duplicate destroy of `DDCResetFIFOMutex`.
- Hardened serial CAT parser against malformed input:
  - Added bounds checks while assembling CAT frames.
  - Removed undefined behavior from unchecked `strstr` pointer arithmetic.
- Hardened CAT command parsing:
  - Added maximum parameter length guard before stack-buffer copy.
- Fixed CAT queue safety and thread behavior:
  - Added queue mutex protection for producer/consumer access.
  - Prevented overrun by dropping oversized queue messages.
- Fixed `g2v2panel_i2c` pthread entrypoint signatures:
  - Updated thread handlers to `void *(*)(void *)` and return `NULL`.

### Changed

- Converted shared cross-thread run-state flags to atomics (`atomic_bool`):
  - `IsTXMode`, `SDRActive`, `ReplyAddressSet`, `StartBitReceived`,
    `NewMessageReceived`, `ThreadError`, `ExitRequested`.
- Converted CAT lifecycle/shared state to atomics:
  - `CATPortAssigned`, `ThreadActive`, `CATKeepaliveActive`, `SignalThreadEnd`.
- Converted `CATPort` to `atomic_int` and updated setup/connect lifecycle:
  - Uses compare-exchange to ensure single initializer in `SetupCATPort`.
- Converted `SocketData[*].Cmdid` to `atomic_uint_fast32_t`:
  - Replaced plain bit operations with `atomic_fetch_or`, `atomic_fetch_and`, and `atomic_load`.
- Synchronized additional shared `SocketData` fields:
  - `Socketid` converted to `atomic_int`.
  - `Portid` converted to `atomic_ushort`.
  - Updated runtime paths to use atomic loads/stores for socket/port access in control and worker threads.
- Hardened shared-socket alias ownership behavior:
  - Added helper APIs to resolve owner-vs-alias socket FD access.
  - Shared alias threads now avoid closing/rebinding owner sockets.
  - `MakeSocket` now re-synchronizes alias socket IDs from owner sockets.
- Synchronized thread activity-state tracking:
  - `ThreadSocketData.Active` converted to `atomic_bool`.
  - Startup/shutdown activity transitions now use atomic stores.
  - Thread activity scans now use atomic loads.
- Synchronized wideband runtime parameter sharing:
  - Added mutex protection for `WBParamsChanged` and `Stored*` wideband settings.
  - Wideband worker now consumes a locked parameter snapshot per loop iteration.
- Synchronized activity-timeout enable state:
  - `HW_Timer_Enable` converted to `atomic_bool`.
  - Watchdog reads and general-packet writes now use atomic access.
- Normalized shared-alias port updates:
  - `SetPort` now applies owner-driven updates for alias port entries.
  - Alias entries no longer request independent rebinds.
  - General packet handling parses alias-port fields again for late-P2 compatibility; non-zero alias values are applied to shared owner ports.
- Synchronized front-panel shared flags (`g2panel*` / `g2v2panel*`):
  - Converted panel run-state flags to atomics.
  - Converted shared optical-encoder delta counters to atomics.
  - Converted CAT-updated G2V2 indicator/detection state to atomics and used atomic snapshots in panel tick logic.
- Synchronized serial-reader thread state (`TSerialThreadData`):
  - Converted shared serial handle/open/active/request flags to atomics.
  - Updated CAT serial thread and G2V2/Aries control paths to use atomic lifecycle checks and handle snapshots.
- Synchronized Aries ATU shared run-state flags:
  - Converted `AriesATUActive` to `atomic_bool` and updated Aries tick/init/shutdown and high-priority Alex override checks to atomic loads/stores.
  - Converted internal Aries detection state (`AriesDetected`) to atomic access during probe/response flow.
- Synchronized Aries ATU shared tune/antenna state:
  - Added a dedicated Aries mutex to protect `CurrentTXAntenna`, `CurrentRXAntenna`, `CurrentFrequency`, `TuneSolutionFound`, and `EnabledForAntenna[]`.
  - Updated Aries tick/CAT/incoming-control paths to use locked snapshots for state decisions without holding locks during CAT I/O.
- Synchronized CAT lifecycle state and fixed failed-connect recovery:
  - Updated CAT setup/worker/keepalive/shutdown paths to use explicit atomic lifecycle flag access.
  - Fixed CAT `connect(...)` failure path to clear shared CAT port/active state so future `SetupCATPort(...)` calls can retry.
  - Closed CAT socket FD on failed `connect(...)` and made keepalive exit when CAT port state is cleared.
  - Added CAT thread-running tracking and shutdown wait hardening so CAT teardown is deterministic even when threads are still in startup wait.
  - Replaced detached CAT worker/keepalive threads with tracked joinable lifecycle and explicit join-pending cleanup in setup/shutdown/failure paths.
- Normalized shared run-state flag access across worker/control paths:
  - Replaced remaining mixed direct accesses with explicit `atomic_load`/`atomic_store` for shared flags (`SDRActive`, `IsTXMode`, `ReplyAddressSet`, `StartBitReceived`, `NewMessageReceived`, `ThreadError`, `ExitRequested`, `CATPortAssigned`).
  - Updated RX/TX worker loops and activity/control paths to use consistent atomic gating.
  - Converted LDG ATU local control/tune-request state to atomic access aligned with CAT lifecycle state.
- Serialized shared I2C bus access in the common I2C driver:
  - Added mutex protection around SMBus read/write helpers so concurrent panel threads cannot interleave bus transactions on `i2c_fd`.
  - Added shared I2C open/close helpers and moved panel probe/shutdown paths to those wrappers for mutex-aligned FD lifecycle handling.
- Localized tick-owned G2V2 panel state:
  - Moved `CATPollCntr`, `GLEDState`, and I2C `VKeepAliveCnt` from file-scope globals into `G2V2PanelTick(...)` locals.
  - Reduced global shared-state surface and enforced tick-thread ownership in code structure.
- Localized tick-owned G2 panel state:
  - Moved `CATDetected`, `EncodersInitialised`, and `TickCounter` from file-scope globals into `G2PanelTick(...)` locals.
  - Updated `EncoderTick(...)` call flow to pass encoder-init state explicitly instead of using shared globals.
- Hardened front-panel thread lifecycle synchronization:
  - Replaced sleep-based detached-thread shutdown with tracked joinable teardown in `g2panel*`, `g2v2panel`, and `g2v2panel_i2c`.
  - Added defensive GPIO-setup gating before thread startup in `g2panel*` and `g2v2panel_i2c`, plus interrupt-line request error handling in `g2v2panel_i2c`.
- Hardened serial CAT thread lifecycle synchronization:
  - Replaced detached serial-thread teardown in `g2v2panel` and `AriesATU` with tracked joinable shutdown.
  - Replaced detached Aries tick-thread teardown with tracked joinable shutdown.
  - Added deterministic serial-thread cleanup join on probe-time detection failure paths.
  - Moved `DeviceActive` run-intent ownership to controllers (G2V2/Aries) so `CATSerial` no longer re-asserts active state and can’t overwrite stop requests.
- Synchronized shared mic/wideband DMA read FD access:
  - Converted `DMAReadfile_fd` to `atomic_int` and updated mic/wideband DMA paths to use atomic FD snapshots.
  - Added defensive guard to skip wideband DMA reads when the shared FD is unavailable.
- Fixed wideband dual-socket activity-state teardown:
  - Updated wideband thread shutdown to clear `Active` for both wideband socket entries instead of only the first entry.
  - Applied alias-aware close/teardown loop across both wideband outputs.
- Fixed DDC multi-socket activity-state teardown:
  - Updated DDC outgoing thread shutdown to clear `Active` for all DDC socket entries instead of only the first entry.
  - Applied alias-aware close/teardown loop across all DDC outputs.
- Hardened `p3app` control-thread lifecycle:
  - Converted `CheckForExit` and `CheckForActivity` helper threads to joinable shutdown with explicit startup tracking.
  - Added explicit exit signaling before shutdown joins and removed unconditional detach of `CheckForExit` when skip-exit mode is active.
  - Switched `CheckForExit` to nonblocking stdin reads so shutdown signaling is observed deterministically.
- Hardened G2V2/Aries probe wait behavior:
  - Replaced fixed 2s detection sleeps with bounded early-exit polling loops in panel/ATU probe paths.
  - Reduced startup timing dependence while preserving bounded detection timeout behavior.
- Hardened worker-loop shutdown gating:
  - Exported shared `ExitRequested` state in thread data headers for module-wide shutdown visibility.
  - Updated incoming/outgoing worker outer/wait loops to honor `ExitRequested` for deterministic shutdown exit paths.
  - Normalized several worker error paths to exit loops and run standard cleanup (`Active` clear / socket teardown).
- Converted main `p3app` data-plane workers to join-based teardown:
  - Added startup tracking for worker thread create success.
  - Removed detach usage for those workers and joined them in `Shutdown()`.
  - Aligned top-level shutdown ordering with explicit worker stop signaling.
- Hardened `OutDDCIQ` fatal decode error handling:
  - Replaced worker-thread `exit(1)` calls with thread-scoped error signaling and controlled loop exit.
  - Added explicit DDC DMA device FD close in thread shutdown cleanup.
- Added shutdown smoke helper:
  - Added `make smoke-shutdown` target.
  - Added `tools/shutdown_smoke.sh` to exercise bounded SIGINT shutdown of `p3app` and report timeout/failure conditions.
- Corrected thread-start error checks across startup paths:
  - Replaced `pthread_create(...) < 0` checks with `pthread_create(...) != 0` in `p3app`, CAT handler, Aries handler, and front-panel handlers.
  - Ensures failed thread creation does not get misclassified as success, preserving correct startup flags and join/shutdown behavior.
- Added fail-fast socket startup checks in `p3app`:
  - Validates `MakeSocket(...)` success for command, incoming control/data, and DDC IQ socket setup before launching dependent threads.
  - Exits early on socket creation/bind failures to prevent worker startup with invalid socket state.
- Added `reply_addr` synchronization:
  - Introduced `g_reply_addr_mutex`.
  - Protected writer updates and thread-local snapshots.

### Verification

- Module-level syntax checks run with `gcc -fsyntax-only` on affected files.
- Full project build passed:
  - `make -j4` in `sw_projects/P3_app`.

### Notes

- Detailed implementation notes and rationale are in `README.md`.
