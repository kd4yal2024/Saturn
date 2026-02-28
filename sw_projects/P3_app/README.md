# P3_app Hardening Notes

This document records the code-safety and concurrency hardening changes applied to `P3_app` in this session, and why each change was made.

Date: 2026-02-11

## Goals

- Remove memory-safety issues in CAT input/output paths.
- Remove high-impact data races across worker threads.
- Fix shutdown/resource cleanup correctness.
- Keep runtime behavior compatible with existing protocol flow.
- Add optional true independent alias socket ports while preserving late-P2 compatibility for Thetis and piHPSDR (shared-port behavior remains default unless distinct alias ports are explicitly requested).

## Front Panel Mode Override (2026-02-25)

For field testing and service-managed P2/P3 switching, `frontpanelhandler.c` now supports
an environment variable override:

- `SATURN_FRONT_PANEL_MODE=auto` (default behavior)
- `SATURN_FRONT_PANEL_MODE=g2` (force G2 front panel path)
- `SATURN_FRONT_PANEL_MODE=g2v2` (force G2V2/serial panel path)
- `SATURN_FRONT_PANEL_MODE=prefer-g2` or `prefer_g2`
- `SATURN_FRONT_PANEL_MODE=prefer-g2v2` or `prefer_g2v2`
- `SATURN_FRONT_PANEL_MODE=off` / `none` (disable front panel initialization)

This is intended to help when auto-detection selects the wrong panel type while the SDR path
itself is otherwise working (for example, Thetis connectivity is fine but local panel controls
do not behave as expected).

## Startup Interface/MAC Enumeration Fix (2026-02-26)

`p2app.c` startup MAC discovery now copies the selected interface name before closing
`/sys/class/net`, validates that a matching interface was found, skips `lo`, and checks
`ioctl(SIOCGIFHWADDR)` for failure.

This fixes a startup bug where `readdir()` data could be referenced after `closedir()`
and improves error handling for systems with unexpected interface names.

## Thetis Startup Stability and Trace Logging (2026-02-27)

Additional startup-path hardening was applied after repeated field reports where Thetis
would intermittently fail to start the radio session.

What changed:

- General-packet handling now applies queued startup configuration immediately after packet
  receipt (duplicate packet suppression is retained).
- Added one-time startup trace logs for:
  - Discovery packet received
  - General packet received/applied
  - High-priority run-bit observed
  - Startup handshake completed (`reply + run -> active`)
- High-priority packet logging was reduced from per-packet spam to first-stream detection.
- Startup trace flags are reset on explicit client stop and no-activity timeout transitions.

Important regression note:

- A CAT keepalive respawn-throttle experiment was tested to suppress rapid CAT thread
  restart loops, but this introduced a Thetis compatibility regression in startup.
- That CAT respawn-guard logic was rolled back; Thetis startup stability returned after
  rollback.
- Current baseline keeps startup trace instrumentation and compatibility behavior, without
  CAT respawn throttling.

## What Changed And Why

### 1. Shutdown semaphore cleanup fix

Changed file:
- `p2app.c`

What changed:
- Replaced duplicate `sem_destroy(&DDCResetFIFOMutex)` with `sem_destroy(&MicWBDMAMutex)`.

Why:
- `MicWBDMAMutex` was initialized but never destroyed.
- `DDCResetFIFOMutex` was destroyed twice.
- This is a correctness/resource-lifecycle bug.

### 2. Serial CAT input hardening

Changed file:
- `serialport.c`

What changed:
- Added bounds protection when appending bytes to `CATMessageBuffer`.
- Reset parser state on unexpected control characters.
- Replaced `strstr(...) - buffer` logic with safe prefix checks using `strncmp`.

Why:
- Prevent potential buffer overwrite from long/malformed serial input.
- Avoid undefined behavior when `strstr` returns `NULL`.

### 3. CAT parser bounds checks

Changed file:
- `cathandler.c`

What changed:
- Added `VCATPARSESTRINGMAX`.
- Validated CAT parameter length before copy into local parse buffer.

Why:
- Original parser used a fixed small stack buffer with no length guard.
- Long CAT payloads could overflow stack memory.

### 4. CAT output queue synchronization and safety

Changed file:
- `cathandler.c`

What changed:
- Added `CATOPBufferMutex`.
- Added locked/unlocked queue-used helpers.
- Protected enqueue/dequeue pointer and buffer access with mutex.
- Added message length guard before queue copy (`MsgLength < VOPSTRSIZE`).

Why:
- Queue was shared across multiple producer/consumer threads without synchronization.
- Unsynchronized ring-buffer pointer updates are data races.
- Overlong CAT messages could overrun queue slot size.

### 5. Shared run-state flags converted to atomics

Changed files:
- `threaddata.h`
- `p2app.c`
- `cathandler.h`
- `cathandler.c`

What changed:
- Converted global cross-thread flags to `atomic_bool`, including:
  - `IsTXMode`
  - `SDRActive`
  - `ReplyAddressSet`
  - `StartBitReceived`
  - `NewMessageReceived`
  - `ThreadError`
  - `ExitRequested`
  - CAT lifecycle flags (`CATPortAssigned`, `ThreadActive`, `CATKeepaliveActive`, `SignalThreadEnd`)

Why:
- These flags are written/read from multiple detached threads.
- Plain `bool` access in this model is undefined behavior in C due to data races.
- Atomics remove undefined behavior while preserving simple usage.

### 6. `SocketData[*].Cmdid` race removal

Changed files:
- `threaddata.h`
- `p2app.c`
- `OutMicAudio.c`
- `OutHighPriority.c`
- `OutDDCIQ.c`
- `Outwideband.c`

What changed:
- Converted `Cmdid` to `atomic_uint_fast32_t`.
- Replaced direct bit operations with atomic ops:
  - set bit: `atomic_fetch_or(...)`
  - clear bit: `atomic_fetch_and(...)`
  - read bits: `atomic_load(...)`

Why:
- `Cmdid` is written in control paths and read/modified in worker loops concurrently.
- Non-atomic read/modify/write on shared bitfields can lose updates and race.

### 7. `CATPort` lifecycle race fixes

Changed file:
- `cathandler.c`

What changed:
- Converted `CATPort` from `int` to `atomic_int`.
- Used `atomic_compare_exchange_strong` in `SetupCATPort` so only one initializer wins.
- Used atomic loads/stores in CAT thread connect/reconnect loop.
- Removed stale `return` path ordering issue where `CATPort = 0` was unreachable after `return`.

Why:
- `CATPort` is shared between setup, running handler loop, and shutdown/reconnect paths.
- Non-atomic access can race and cause inconsistent connect behavior.

### 8. `reply_addr` synchronization

Changed files:
- `threaddata.h`
- `p2app.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `OutDDCIQ.c`
- `Outwideband.c`
- `cathandler.c`

What changed:
- Added global mutex `g_reply_addr_mutex`.
- Locked around `reply_addr` writes in main command path.
- Locked around snapshots of `reply_addr` in TX/CAT threads.

Why:
- `reply_addr` is a multi-field struct updated in one thread and copied in several others.
- Without locking, readers can observe torn/inconsistent values.

### 9. Thread entrypoint signature correctness (i2c panel)

Changed file:
- `g2v2panel_i2c.c`

What changed:
- Updated `G2V2PanelInterrupt` and `G2V2PanelTick` to `void*` return type.
- Added `return NULL;` in both functions.
- Removed now-unused local variables in interrupt function.

Why:
- `pthread_create` requires `void *(*)(void *)`.
- Previous signature mismatch generated warnings and is unsafe/incorrect.

### 10. Minor header cleanup

Changed file:
- `AriesATU.c`

What changed:
- Removed redundant local `extern bool IsTXMode;`.

Why:
- `IsTXMode` now comes from shared header as `atomic_bool`.
- Avoid type mismatch/confusion.

### 11. `SocketData` Port/Socket synchronization (`Portid` and `Socketid`)

Changed files:
- `threaddata.h`
- `p2app.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `OutDDCIQ.c`
- `Outwideband.c`
- `InHighPriority.c`
- `InDUCIQ.c`
- `InSpkrAudio.c`
- `IncomingDDCSpecific.c`
- `IncomingDUCSpecific.c`

What changed:
- Converted `ThreadSocketData` fields:
  - `Socketid` -> `atomic_int`
  - `Portid` -> `atomic_ushort`
- Updated `SetPort` to use atomic load/store for `Portid`.
- Updated `MakeSocket` to:
  - use explicit local `Socketfd`/`Portid` snapshots
  - atomically store `Socketid`
  - clear `Socketid` on bind failure
- Updated thread IO and rebind paths to use atomic loads/stores when calling:
  - `recvmsg`, `sendmsg`, `sendto`, `ioctl`, `close`
  - shared-socket alias assignment in startup paths.

Why:
- `Portid` and `Socketid` are shared between control and worker threads.
- Plain concurrent reads/writes to these fields were data races under the C memory model.
- Atomic access makes these fields race-safe while preserving existing behavior.

### 12. Shared-socket alias ownership hardening

Changed files:
- `threaddata.h`
- `p2app.c`
- `OutMicAudio.c`
- `OutHighPriority.c`
- `Outwideband.c`

What changed:
- Added shared-socket helper APIs:
  - `GetThreadSocketFD(...)`
  - `ThreadSocketIsSharedAlias(...)`
  - `SyncSocketAliasesForOwner(...)`
- Added explicit owner mapping for shared socket aliases:
  - `VPORTMICAUDIO` -> `VPORTDUCSPECIFIC`
  - `VPORTHIGHPRIORITYFROMSDR` -> `VPORTDDCSPECIFIC`
  - `VPORTWIDEBAND0` -> `VPORTHIGHPRIORITYTOSDR`
  - `VPORTWIDEBAND1` -> `VPORTSPKRAUDIO`
- `MakeSocket(...)` now synchronizes alias `Socketid` mirrors after owner socket creation/rebind.
- Shared-alias worker threads now:
  - resolve send socket via `GetThreadSocketFD(...)`
  - skip close/rebind on `VBITCHANGEPORT` when they are aliases
  - skip socket `close(...)` on thread shutdown when they are aliases

Why:
- Alias threads were previously able to close/rebind a socket FD that is also used by an owning thread.
- That creates cross-thread FD lifecycle races (owner can keep using a closed/reused FD).
- Owner-resolved FD access plus owner-only lifecycle control removes this class of threading bugs.

### 13. Thread activity-state synchronization (`Active`)

Changed files:
- `threaddata.h`
- `p2app.c`
- `IncomingDDCSpecific.c`
- `IncomingDUCSpecific.c`
- `InHighPriority.c`
- `InDUCIQ.c`
- `InSpkrAudio.c`
- `OutDDCIQ.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `Outwideband.c`

What changed:
- Converted `ThreadSocketData.Active` from `bool` to `atomic_bool`.
- Updated thread lifecycle writes to use `atomic_store(...)` on startup/shutdown paths.
- Updated `CheckActiveThreads(...)` to use `atomic_load(...)` when scanning thread states.

Why:
- `Active` is written by worker threads and read by control/shutdown logic in other threads.
- Plain `bool` access across those threads is a C data race.
- Atomic access removes undefined behavior while keeping existing lifecycle semantics.

### 14. Wideband parameter synchronization

Changed file:
- `Outwideband.c`

What changed:
- Added `g_wideband_params_mutex` to protect shared wideband control parameters.
- `SetWidebandParams(...)` now locks around compare/update of:
  - `WBParamsChanged`
  - `StoredEnables`
  - `StoredSamplePerPktCount`
  - `StoredSampleSize`
  - `StoredRate`
  - `StoredPacketCount`
- `OutgoingWidebandSamples(...)` now takes a locked snapshot per loop iteration and uses local copies for reconfiguration and packetization decisions.
- Transition back to inactive now updates `StoredEnables`/`WBParamsChanged` under the same mutex.

Why:
- General-packet handling writes these fields while the wideband worker thread reads them concurrently.
- Unsynchronized access can produce torn/mixed parameter sets and undefined behavior.
- Snapshot+mutex access makes reconfiguration deterministic and race-safe.

### 15. Activity-timeout enable flag synchronization

Changed files:
- `generalpacket.h`
- `generalpacket.c`
- `p2app.c`

What changed:
- Converted `HW_Timer_Enable` from `bool` to `atomic_bool`.
- Updated general-packet update path to use `atomic_store(...)`.
- Updated activity-watchdog read path to use `atomic_load(...)`.

Why:
- `HW_Timer_Enable` is written when protocol settings are updated and read asynchronously by the watchdog thread.
- Plain `bool` access in those concurrent paths is a data race under C.
- Atomic access makes timeout-enable behavior race-safe and deterministic.

### 16. Shared-alias port compatibility normalization

Changed files:
- `p2app.c`
- `generalpacket.c`

What changed:
- Updated `SetPort(...)` to resolve owner-vs-alias thread indices before applying updates.
- Alias threads now:
  - mirror owner `Portid` by default
  - clear local `VBITCHANGEPORT`
  - do not independently request socket rebinds
- Non-zero alias-port values are accepted as compatibility overrides and applied to the shared owner socket port.
- Owner updates now also mirror alias `Portid` entries and clear alias rebind bits.
- General-packet handling once again parses alias-port fields (high-priority-from-SDR, mic, wideband) to preserve late-P2 behavior.

Why:
- Shared alias ports (`Mic`, `HighPriorityFromSDR`, `Wideband0/1`) represent the same underlying sockets as their owners.
- Independent alias rebinds can cause table drift and inconsistent FD lifecycle behavior.
- Owner-driven normalization with compatibility overrides keeps socket/port lifecycle coherent while still honoring non-zero alias field inputs.

### 17. Front-panel shared-flag synchronization (`g2panel*` / `g2v2panel*`)

Changed files:
- `g2panel.c`
- `g2panel_libgpiodv2.c`
- `g2v2panel.c`
- `g2v2panel_i2c.c`

What changed:
- Converted panel thread run-state flags to atomics and updated loop/control sites:
  - `G2PanelActive`
  - `G2V2PanelActive`
- Converted shared optical-encoder delta counter to atomic and removed unsynchronized updates:
  - `GDeltaCount` now uses atomic load/CAS in `ReadOpticalEncoder(...)`
  - event threads now use atomic fetch add/sub
- Converted CAT-updated G2V2 panel state to atomics:
  - `G2V2Detected`, `G2V1AdapterDetected`, `GZZZIReceived`
  - `G2ToneState`, `GVFOBSelected`, `GCombinedVFOState`
  - `ATURedLED`, `ATUGreenLED`
  - product/version fields used by panel identity reporting (`G2V2PanelProductID`, `G2V2PanelHWVersion`, `G2V2PanelSWID`)
- Updated panel tick paths to consume atomic snapshots before LED-state calculations.

Why:
- Front-panel threads poll and update shared state concurrently with CAT handler and shutdown paths.
- Plain shared `bool`/integer accesses in those paths are data races under C.
- Atomic access makes panel lifecycle, detection, and indicator-state behavior thread-safe without protocol redesign.

### 18. Serial thread state synchronization (`TSerialThreadData`)

Changed files:
- `serialport.h`
- `serialport.c`
- `g2v2panel.c`
- `AriesATU.c`

What changed:
- Converted shared serial-thread state fields in `TSerialThreadData` to atomics:
  - `DeviceHandle` -> `atomic_int`
  - `DeviceActive` -> `atomic_bool`
  - `RequestID` -> `atomic_bool`
  - `IsOpen` -> `atomic_bool`
- Updated `CATSerial(...)` to use atomic load/store for lifecycle state and to operate on a stable local `DeviceHandle` snapshot.
- Updated G2V2 panel and Aries ATU control paths to:
  - initialize serial thread state via atomic stores
  - stop serial threads via atomic `DeviceActive` writes
  - gate outgoing serial/CAT writes on atomic `IsOpen` + valid handle checks
  - use atomic checks in `IsFrontPanelSerial(...)` and `IsAriesSerial(...)`

Why:
- Serial thread state is shared between detached serial reader threads and control/tick/CAT paths.
- Plain concurrent reads/writes to open/active/handle fields can race and produce stale or invalid handle use.
- Atomic synchronization removes this undefined behavior and hardens serial device lifecycle transitions.

### 19. Aries ATU active/detected flag synchronization

Changed files:
- `AriesATU.c`
- `AriesATU.h`
- `InHighPriority.c`

What changed:
- Converted `AriesATUActive` to `atomic_bool` and updated all cross-thread checks/stores to `atomic_load(...)` / `atomic_store(...)`.
- Converted internal Aries probe state `AriesDetected` to `static atomic_bool` and updated probe/result paths to atomic access.
- Updated Aries init flow to reset `AriesDetected` and `AriesATUActive` before probing the serial device.
- Updated incoming high-priority Alex-ant override checks to read `AriesATUActive` atomically.

Why:
- Aries ATU state flags are written from Aries detection/shutdown paths and read concurrently from tick and incoming packet threads.
- Plain shared `bool` access in these paths is a data race under C and can cause stale/undefined state decisions.
- Atomic access removes that undefined behavior and keeps Aries enable/override behavior deterministic.

### 20. Aries ATU shared tune/antenna state synchronization

Changed file:
- `AriesATU.c`

What changed:
- Added `g_aries_state_mutex` to protect shared Aries state variables accessed from multiple threads:
  - `CurrentTXAntenna`
  - `CurrentRXAntenna`
  - `CurrentFrequency`
  - `TuneSolutionFound`
  - `EnabledForAntenna[]`
- Updated Aries tick, CAT callback handlers, and incoming control handlers to lock around shared-state reads/writes and use local snapshots for logging/CAT sends.
- Kept lock scopes short and avoided holding the mutex across CAT I/O calls.

Why:
- These fields are updated from different execution contexts (Aries tick thread, incoming high-priority path, and CAT/serial-driven handlers).
- Unsynchronized access can produce stale/torn decisions for ATU LED state, antenna state, and frequency change handling.
- Mutex-protected snapshots make Aries behavior deterministic under concurrent updates.

### 21. CAT lifecycle synchronization and failed-connect recovery

Changed file:
- `cathandler.c`

What changed:
- Tightened CAT lifecycle flag access to explicit atomic operations across CAT setup, keepalive, worker, and shutdown paths:
  - `CATPortAssigned`
  - `ThreadActive`
  - `CATKeepaliveActive`
  - `SignalThreadEnd`
- Updated CAT send/loop gating checks to use atomic loads for shared run-state flags.
- Fixed failed `connect(...)` cleanup path to clear shared CAT port/lifecycle state (`CATPort`, `CATPortAssigned`, `ThreadActive`) before exiting.
- Closed CAT socket FD on `connect(...)` failure and constrained keepalive loop to exit when `CATPort` is cleared.
- Added explicit CAT thread-running flags so shutdown waits for CAT worker/keepalive threads even during startup wait windows.
- Updated startup wait loops and shutdown to honor `SignalThreadEnd` and forced `CATPort` clear for deterministic teardown.

Why:
- CAT lifecycle flags are shared across the high-priority input path, CAT worker thread, keepalive thread, and shutdown logic.
- Explicit atomic access keeps lifecycle transitions deterministic and avoids stale state checks.
- Without clearing shared state on `connect` failure, `CATPort` could remain non-zero and block subsequent `SetupCATPort(...)` retries.

### 22. Shared run-state atomic access normalization

Changed files:
- `p2app.c`
- `InHighPriority.c`
- `InDUCIQ.c`
- `InSpkrAudio.c`
- `IncomingDDCSpecific.c`
- `IncomingDUCSpecific.c`
- `OutDDCIQ.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `Outwideband.c`
- `g2panel.c`
- `g2panel_libgpiodv2.c`
- `LDGATU.c`
- `AriesATU.c`

What changed:
- Normalized remaining shared run-state flag usage to explicit atomic operations in worker/control loops:
  - `SDRActive`
  - `IsTXMode`
  - `ReplyAddressSet`
  - `StartBitReceived`
  - `NewMessageReceived`
  - `ThreadError`
  - `ExitRequested`
  - `CATPortAssigned`
- Updated producer/consumer and activity watchdog paths to use `atomic_load(...)`/`atomic_store(...)` consistently instead of mixed direct expressions.
- Converted LDG ATU local control/tune-request state to atomic access to keep CAT-request gating synchronized with shared CAT lifecycle state.

Why:
- These flags are shared across detached RX/TX worker threads, CAT/control threads, and top-level control/shutdown logic.
- Mixed direct/implicit access makes synchronization intent harder to reason about and risks stale lifecycle checks in concurrent paths.
- Explicit atomic access clarifies thread-safety boundaries and makes behavior deterministic under concurrent updates.

### 23. Shared I2C bus access serialization

Changed file:
- `i2cdriver.c`

What changed:
- Added a global I2C bus mutex (`g_i2c_bus_mutex`) in the shared I2C driver wrapper.
- Wrapped all SMBus read/write operations in `i2cdriver.c` with mutex lock/unlock:
  - `i2c_write_byte_data(...)`
  - `i2c_write_word_data(...)`
  - `i2c_read_byte_data(...)`
  - `i2c_read_word_data(...)`
- Added shared I2C lifecycle helpers in the same driver (`i2c_open_device(...)`, `i2c_close_device(...)`) so open/close use the same bus mutex discipline.
- Switched G2 panel implementations to use the shared open/close helpers and initialize `i2c_fd` to `-1`, eliminating stale-FD/leak edges on probe/shutdown paths.

Why:
- Front-panel implementations can issue I2C operations from more than one thread (notably G2V2 interrupt + tick paths).
- Serializing low-level SMBus calls prevents interleaved transactions on the shared `i2c_fd` and makes panel event/LED I2C traffic deterministic.

### 24. G2V2 tick-owned state localization (`CATPollCntr`/`GLEDState`/`VKeepAliveCnt`)

Changed files:
- `g2v2panel.c`
- `g2v2panel_i2c.c`

What changed:
- Removed file-scope globals for tick-only state in G2V2 panel handlers:
  - `CATPollCntr`
  - `GLEDState`
  - `VKeepAliveCnt` (I2C variant)
- Added these as local variables inside each `G2V2PanelTick(...)` function with explicit tick-thread ownership comments.

Why:
- These values are only consumed/updated by the tick thread logic.
- Keeping them as globals unnecessarily widens shared-state surface and invites accidental cross-thread access in future edits.
- Constraining them to tick scope enforces ownership at compile time and improves synchronization clarity.

### 25. G2 panel tick-owned state localization (`CATDetected`/`EncodersInitialised`/`TickCounter`)

Changed files:
- `g2panel.c`
- `g2panel_libgpiodv2.c`

What changed:
- Removed file-scope globals used only by `G2PanelTick(...)`:
  - `CATDetected`
  - `EncodersInitialised`
  - `TickCounter`
- Added these as local variables inside `G2PanelTick(...)` in both panel implementations.
- Updated `EncoderTick(...)` in both files to take `EncodersInitialised` as an explicit parameter instead of relying on global state.

Why:
- These values are tick-thread private and do not need cross-thread visibility.
- Keeping them global increases accidental shared-state coupling and makes future synchronization analysis harder.
- Explicit parameter flow and tick-local storage enforce ownership boundaries directly in code.

### 26. Front-panel thread lifecycle synchronization (deterministic shutdown)

Changed files:
- `g2panel.c`
- `g2panel_libgpiodv2.c`
- `g2v2panel.c`
- `g2v2panel_i2c.c`

What changed:
- Replaced detached front-panel tick/interrupt threads with joinable lifecycle handling.
- Added per-handler thread-start tracking flags so shutdown only joins threads that were actually started.
- Updated shutdown paths to:
  - clear active run flags first,
  - join worker threads,
  - then release GPIO resources/chips.
- Added defensive startup guards in front-panel handlers to avoid starting worker threads when GPIO setup is incomplete:
  - `g2panel.c` (`chip`)
  - `g2panel_libgpiodv2.c` (`chip`/`VFORequest`/`PBRequest`)
  - `g2v2panel_i2c.c` (`chip`/`intline`)
- Added error handling for `gpiod_line_request_falling_edge_events(...)` failure in `g2v2panel_i2c.c`.

Why:
- Previous shutdown relied on fixed sleeps before releasing GPIO resources while worker threads were detached.
- Sleep-based teardown can race under scheduler jitter and can release resources still in use by worker threads.
- Join-based teardown makes front-panel shutdown deterministic and closes a class of thread/resource lifecycle races.

### 27. Shared mic/wideband DMA FD synchronization (`DMAReadfile_fd`)

Changed files:
- `threaddata.h`
- `OutMicAudio.c`
- `Outwideband.c`

What changed:
- Converted shared DMA read FD declaration to `atomic_int` (`DMAReadfile_fd`).
- Updated mic thread DMA-device open path to publish the FD via `atomic_store(...)`.
- Updated mic and wideband DMA transfer paths to use atomic FD snapshots before issuing `DMAReadFromFPGA(...)`.
- Added defensive invalid-FD guard in wideband FIFO read helper to avoid DMA reads when FD is not available.

Why:
- `DMAReadfile_fd` is shared between mic and wideband threads.
- Plain concurrent access to shared FD state is a data race in C and can produce invalid/stale FD use under thread scheduling jitter.
- Atomic snapshots make FD visibility deterministic and harden shared DMA access coordination.

### 28. Serial CAT thread lifecycle synchronization (G2V2/Aries)

Changed files:
- `g2v2panel.c`
- `AriesATU.c`

What changed:
- Replaced detached serial-reader thread handling with tracked joinable lifecycle for:
  - G2V2 panel serial thread
  - Aries ATU serial thread
- Replaced detached Aries tick thread handling with tracked joinable lifecycle.
- Updated shutdown paths to:
  - clear active run flags,
  - join started threads,
  - avoid fixed sleep-based teardown ordering.
- Added deterministic cleanup join when probe-time detection fails (serial thread created but device not accepted).

Why:
- Serial reader threads were previously detached and shutdown relied on fixed sleeps after clearing `DeviceActive`.
- Sleep-based teardown is timing-sensitive and can leave short windows where shared serial state remains in transition during shutdown/detection failure paths.
- Join-based teardown makes serial lifecycle transitions deterministic and reduces thread/resource race exposure.

### 29. Serial start/stop handshake hardening (`DeviceActive` ownership)

Changed files:
- `serialport.c`
- `g2v2panel.c`
- `AriesATU.c`

What changed:
- Removed internal `CATSerial(...)` write that forced `DeviceActive=true` after open.
- Updated G2V2/Aries startup paths to set `DeviceActive=true` before `pthread_create(...)` (caller-owned run intent).
- Added create-failure rollback to clear `DeviceActive` when serial thread creation fails.
- Guarded initial ID-request path (`ZZZS`) so it only runs when `DeviceActive` is still asserted.

Why:
- Allowing the serial thread to unconditionally re-assert `DeviceActive` can overwrite a concurrent stop request and re-enable a thread during shutdown/probe-failure races.
- Making the controller own `DeviceActive` establishes one-way lifecycle control and removes this lost-stop-request edge.

### 30. CAT worker/keepalive thread lifecycle synchronization

Changed file:
- `cathandler.c`

What changed:
- Replaced detached CAT worker/keepalive threads with tracked joinable lifecycle flags.
- Added join-pending tracking for CAT worker and keepalive threads.
- Updated CAT setup path to:
  - join any prior completed CAT threads before re-create,
  - record join-pending state on successful creates,
  - deterministically clean up worker thread if keepalive create fails.
- Updated CAT shutdown path to join keepalive and worker threads after asserting stop signals.

Why:
- Detached CAT threads relied on active/running spin loops for shutdown coordination.
- Joinable teardown removes timing dependence and resource-leak edges during restart/failure paths.
- Explicit join-pending tracking keeps CAT thread lifecycle deterministic across setup/reconnect/shutdown transitions.

### 31. Wideband dual-socket activity-state teardown fix

Changed file:
- `Outwideband.c`

What changed:
- Updated wideband thread shutdown to process both wideband socket entries (`ADC0` and `ADC1`) when closing/marking inactive.
- Replaced single-entry teardown with a loop over `VNUMWBADC` for:
  - alias-aware socket close checks,
  - `Active` flag clear for each entry.

Why:
- The wideband thread sets `Active=true` for both socket entries on startup.
- Previous shutdown only cleared `ThreadData->Active` (first entry), leaving the second entry stale-active.
- Clearing both entries keeps thread-activity state consistent for shutdown/status scans and avoids false “active thread” reports.

### 32. DDC multi-socket activity teardown synchronization

Changed file:
- `OutDDCIQ.c`

What changed:
- Updated DDC outgoing thread shutdown to iterate all DDC socket entries (`VNUMDDC`) instead of only handling the first entry.
- Applied per-entry teardown for:
  - alias-aware socket close handling,
  - `Active` flag clear on each DDC socket record.

Why:
- The DDC outgoing thread marks every DDC socket entry active at startup.
- Previous shutdown only cleared one entry, leaving stale-active state on the remaining DDC entries.
- Full per-entry teardown keeps thread-state accounting consistent and prevents false-active conditions during shutdown/health checks.

### 33. `p3app` control-thread lifecycle hardening (`CheckForExit` / `CheckForActivity`)

Changed file:
- `p2app.c`

What changed:
- Converted control helper threads from detached to joinable lifecycle:
  - `CheckForExitThread`
  - `CheckForNoActivityThread`
- Added startup tracking flags and deterministic joins in `Shutdown()`.
- Added explicit shutdown signaling (`ExitRequested=true`) before teardown joins and at main-loop exit.
- Reworked `CheckForExitCommand(...)` to use nonblocking stdin reads so it can observe shutdown signals and exit promptly.
- Removed unconditional detach of `CheckForExitThread` when `-s` skip mode is enabled.

Why:
- Detached control threads relied on process exit timing and could not be cleanly synchronized with shutdown.
- Previous code detached `CheckForExitThread` even when it was never created in skip-exit mode.
- Blocking stdin reads made deterministic thread stop/join behavior difficult.
- Joinable control-thread teardown removes this shutdown race class and makes `p3app` shutdown behavior more predictable.

### 34. Probe wait-loop hardening (G2V2/Aries detection)

Changed files:
- `g2v2panel.c`
- `AriesATU.c`

What changed:
- Replaced fixed `sleep(2)` probe waits with bounded polling loops (`20 x 100ms`) during:
  - G2V2 serial panel detection
  - Aries ATU detection
- Polling loop exits early as soon as expected detection state is observed.

Why:
- Fixed sleep delays impose coarse timing and force full wait even when detection completes early.
- Bounded polling keeps probe behavior deterministic while reducing unnecessary startup delay and tightening probe lifecycle control flow.

### 35. Worker-loop shutdown gating via `ExitRequested`

Changed files:
- `threaddata.h`
- `InHighPriority.c`
- `InDUCIQ.c`
- `InSpkrAudio.c`
- `IncomingDDCSpecific.c`
- `IncomingDUCSpecific.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `OutDDCIQ.c`
- `Outwideband.c`

What changed:
- Exported shared shutdown flag declaration (`extern atomic_bool ExitRequested`) for worker modules.
- Updated core incoming/outgoing worker outer loops to run while shutdown is not requested.
- Updated SDR-idle wait loops and FIFO-depth wait loops to also honor shutdown requests.
- Replaced several early error `return`s with error-flag + loop-exit paths so normal cleanup/`Active=false` teardown still runs.

Why:
- Many worker threads previously used unconditional `while(1)` loops or wait loops that ignored app-wide shutdown intent.
- That made shutdown timing depend on socket/FD side effects instead of explicit lifecycle signaling.
- Propagating `ExitRequested` through worker loops reduces teardown latency and makes worker termination behavior deterministic.

### 36. `p3app` data-plane thread join-based teardown

Changed file:
- `p2app.c`

What changed:
- Added per-thread startup tracking flags for main data-plane worker threads.
- Removed detach behavior for those worker threads at startup.
- Updated `Shutdown()` to join started worker threads deterministically:
  - incoming command/data workers,
  - outgoing mic/high-priority/DDC worker,
  - optional wideband worker.

Why:
- Detached worker teardown relied on timing and implicit process exit semantics.
- With explicit `ExitRequested` gating in worker loops, joinable shutdown can now be used safely.
- Join-based shutdown gives deterministic thread completion ordering and reduces race windows around late resource teardown.

### 37. `OutDDCIQ` fatal-path containment (thread-local failure, not process exit)

Changed file:
- `OutDDCIQ.c`

What changed:
- Replaced `exit(1)` calls in DDC DMA decode paths with:
  - `InitError` assertion,
  - controlled loop exit,
  - standard thread cleanup path.
- Added thread-level error signaling on fatal decode path (`ThreadError = true`).
- Added explicit close of DDC DMA device FD on thread shutdown.

Why:
- A worker thread calling `exit(1)` can terminate the whole process abruptly, bypassing coordinated shutdown.
- Converting to thread-contained error handling preserves deterministic teardown and lets top-level control logic handle failure uniformly.

### 38. Shutdown smoke helper target/script

Changed files:
- `Makefile`
- `tools/shutdown_smoke.sh`

What changed:
- Added `make smoke-shutdown` target that invokes:
  - `./tools/shutdown_smoke.sh ./p3app`
- Added `tools/shutdown_smoke.sh` helper script to:
  - launch `p3app` in `-s` mode,
  - wait a configurable run window,
  - send `SIGINT`,
  - enforce bounded shutdown timeout and report pass/fail.
- Script supports environment overrides:
  - `RUN_SECONDS`
  - `SHUTDOWN_TIMEOUT_SECONDS`
  - `LOG_FILE`

Why:
- Provides a repeatable smoke check for shutdown/drain behavior after lifecycle synchronization changes.
- Reduces manual effort to validate that `ExitRequested`/join-based teardown paths exit within expected bounds.

### 39. Thread startup error-check correctness (`pthread_create`)

Changed files:
- `p2app.c`
- `cathandler.c`
- `AriesATU.c`
- `g2panel.c`
- `g2panel_libgpiodv2.c`
- `g2v2panel.c`
- `g2v2panel_i2c.c`

What changed:
- Normalized thread creation failure checks from `pthread_create(...) < 0` to `pthread_create(...) != 0` across control, CAT, panel, and Aries startup paths.
- Preserved existing startup-tracking flags so they now correctly remain false when creation fails.

Why:
- `pthread_create(...)` returns `0` on success and a positive error code on failure.
- Using `< 0` treated failed creates as success, which could mark threads as started when they were not and break deterministic shutdown/join behavior.

### 40. Fail-fast socket startup validation in `p3app`

Changed file:
- `p2app.c`

What changed:
- Added explicit `MakeSocket(...)` return checks for:
  - command socket,
  - incoming DDC/DUC/high-priority/speaker/DUCIQ sockets,
  - DDC IQ socket set (`VPORTDDCIQ0`..`VPORTDDCIQ9`).
- On socket setup failure, startup now logs a specific message and exits early before creating dependent worker threads.

Why:
- Startup previously ignored socket/bind failures and continued to create worker threads.
- That can launch threads with invalid/unbound socket state and produce undefined startup/runtime behavior.
- Failing fast keeps thread lifecycle and socket ownership state coherent from process start.

## File List Changed In This Hardening Pass

- `AriesATU.c`
- `AriesATU.h`
- `InDUCIQ.c`
- `InHighPriority.c`
- `InSpkrAudio.c`
- `IncomingDDCSpecific.c`
- `IncomingDUCSpecific.c`
- `LDGATU.c`
- `OutDDCIQ.c`
- `OutHighPriority.c`
- `OutMicAudio.c`
- `Outwideband.c`
- `cathandler.c`
- `cathandler.h`
- `g2panel.c`
- `g2panel_libgpiodv2.c`
- `g2v2panel.c`
- `g2v2panel_i2c.c`
- `i2cdriver.c`
- `i2cdriver.h`
- `Makefile`
- `p2app.c`
- `serialport.c`
- `serialport.h`
- `threaddata.h`
- `tools/shutdown_smoke.sh`

## Build/Verification Performed

- Repeated `gcc -fsyntax-only` checks on touched modules.
- Full project build completed successfully:
  - `make -j4` in `sw_projects/P3_app`

## Behavior Notes

- Intent is safety/correctness hardening, not protocol redesign.
- Two intentional protective behaviors:
  - Overlong CAT queue messages are dropped rather than copied unsafely.
  - Malformed serial control-character sequences reset current CAT assembly buffer.
