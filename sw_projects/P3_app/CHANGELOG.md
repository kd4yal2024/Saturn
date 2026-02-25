# Changelog

All notable changes to `P3_app` from this hardening pass are documented here.

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
