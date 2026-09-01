# Saturn Bridge Architecture Audit

Audit date: 2026-08-31

Checkout: `phase0b-bridge-correctness` at `977e242` plus the uncommitted files called out in section 2.3.

## 1. Status vocabulary and scope

- **VERIFIED CURRENT BEHAVIOR** — traced to the current checkout. A nearby source reference identifies the implementation evidence.
- **INTENDED DESIGN** — a requirement supplied for the Saturn TCI/native-audio architecture; it is not evidence that the checkout already behaves this way.
- **TECHNICAL DEBT** — a verified gap, ambiguity, unsafe coupling, or incomplete implementation in the current checkout.
- **PROPOSED CHANGE** — a recommended implementation direction that remains outstanding. Remediations completed in the audited dirty worktree are labeled verified current behavior.

- **VERIFIED CURRENT BEHAVIOR:** This is a source audit only. It did not deploy binaries, start or stop `p2app.service` or `saturn-bridge.service`, switch the radio backend, access RF hardware, or perform a transmit test.
- **VERIFIED CURRENT BEHAVIOR:** The principal implementation is `update_manager/saturn-bridge/src`; Saturn Go's WebSocket proxy is in `update_manager/rust-server/src/remote_tls.rs`; P2_app ownership and hardware-MOX behavior are in `sw_projects/P2_app`.
- **VERIFIED CURRENT BEHAVIOR:** TCI compatibility statements were compared against the official Expert Electronics TCI Protocol 2.0 specification (12 January 2024): <https://raw.githubusercontent.com/ExpertSDR3/TCI/main/TCI%20Protocol.pdf>.
- **TECHNICAL DEBT:** Runtime configuration, installed binaries, and service state can differ from this checkout. This document describes the inspected source, not proof of what is currently installed.

## 2. System boundary and current topologies

### 2.1 P2 backend

- **VERIFIED CURRENT BEHAVIOR:** `SATURN_RADIO_BACKEND=p2` selects the bridge's UDP Protocol 2 implementation (`main.rs:277-300`).

```text
TCI client / Saturn Remote
            |
            | TCI over WebSocket
            v
      Saturn Bridge (Rust)
        |            |
        |            +---- native WDSP RX/TX
        |
        | Protocol 2 UDP
        v
         P2_app
        |
        | XDMA / PCIe
        v
     Saturn FPGA
```

- **VERIFIED CURRENT BEHAVIOR:** In this mode the bridge is a Protocol 2 controller and P2_app owns XDMA/FPGA transport (`main.rs:288-365`, `p2/session.rs:35-272`).
- **VERIFIED CURRENT BEHAVIOR:** The installation policy is P2-only at boot: P2_app is enabled while the bridge is stopped/disabled and may be started on demand in P2 mode (`scripts/install-saturn-bridge.sh:509-538`).
- **INTENDED DESIGN:** A clean boot starts P2_app first. An operator explicitly starts the bridge in P2 mode when remote control is wanted; direct XDMA is a separate manual ownership switch.

### 2.2 Direct-XDMA backend

- **VERIFIED CURRENT BEHAVIOR:** `SATURN_RADIO_BACKEND=xdma` calls `xdma_backend::run`; the bridge then opens XDMA register/DDC/DUC devices itself and bypasses P2_app (`main.rs:277-287`, `xdma_backend.rs:179-230`).

```text
TCI client / Saturn Remote
            |
            | TCI over WebSocket
            v
      Saturn Bridge (Rust)
        |            |
        |            +---- native WDSP RX/TX
        |
        | XDMA / PCIe
        v
     Saturn FPGA
```

- **VERIFIED CURRENT BEHAVIOR:** The backend switch broker treats direct XDMA as exclusive ownership: it stops P2_app before starting the bridge; P2 selection stops the bridge before starting P2_app (`scripts/saturn-radio-backend-switch-root.sh:509-570`).
- **VERIFIED CURRENT BEHAVIOR:** The supported P2-mode on-demand path starts the bridge without stopping an already-active P2_app (`scripts/saturn-radio-backend-switch-root.sh:584-613`).
- **TECHNICAL DEBT:** Documentation and operations must always name the backend. “The bridge owns the radio” means UDP controller ownership in P2 mode but direct FPGA/XDMA ownership in direct-XDMA mode.
- **PROPOSED CHANGE:** Preserve P2_app as the clean-boot owner and make every direct-XDMA transition use the backend broker. A helper that merely restarts `saturn-bridge.service` must not become an ownership switch.

### 2.3 Saturn Go and TCI settings

- **VERIFIED CURRENT BEHAVIOR:** Saturn Go exposes `/tci`, `/saturn/control`, and `/saturn/media`; the remote TLS router applies authentication, Origin validation, and a client-capacity gate before proxying (`rust-server/src/remote_tls.rs:423-429`, `505-585`, `682-737`).
- **VERIFIED CURRENT BEHAVIOR:** Saturn Go dials the bridge's configured loopback URL and appends `/control` or `/media`; lane-incompatible text/binary frames are rejected by the proxy (`remote_tls.rs:1150-1346`).
- **VERIFIED CURRENT BEHAVIOR:** The installed bridge default is `127.0.0.1:50001` (`saturn-bridge/src/config.rs:75,121-146`, `scripts/install-saturn-bridge.sh:498-499`).
- **VERIFIED CURRENT BEHAVIOR:** When TCI is explicitly bound to a specific non-loopback address for a LAN accessory, the bridge also opens the same port on the matching-family loopback address. Saturn Go therefore retains its trusted loopback operator path while raw LAN sessions remain viewer-only (`tci/mod.rs`, `tci/client.rs`). Wildcard binds do not create a redundant listener.
- **VERIFIED CURRENT BEHAVIOR:** The dirty worktree adds a Saturn Go Settings page and `/tci_status` plus `/tci_settings` handlers. POST uses the existing CSRF boundary, the packaged root helper validates the bind, atomically writes its drop-in, and rolls back on a failed restart (`rust-server/src/main.rs`, `templates/settings.html`, `scripts/saturn-tci-bind.sh`).
- **VERIFIED CURRENT BEHAVIOR:** Saving a bind while the bridge is inactive leaves it inactive, preserving P2-first boot. If already active, the helper invokes `restart bridge` through the backend ownership broker; the broker keeps stop/start under one radio-ownership lock (`scripts/saturn-tci-bind.sh`, `scripts/saturn-radio-backend-switch-root.sh`).
- **VERIFIED CURRENT BEHAVIOR:** Raw non-loopback TCI connections are accessory/viewer-only. Only loopback sessions are eligible for operator assignment or promotion, and a non-loopback split-session operator claim is downgraded to viewer (`tci/client.rs`, `tci/session_pair.rs`, `tci/protocol.rs`).
- **TECHNICAL DEBT:** The raw LAN endpoint still has no authentication or confidentiality. Viewer-only authorization removes control/TX authority but does not make the endpoint suitable for an untrusted network.

## 3. TCI entry points, sessions, and roles

### 3.1 Entry and threading

- **VERIFIED CURRENT BEHAVIOR:** `TciFrontend::bind` creates a nonblocking TCP listener, a bounded command mailbox, shared client registry, operator ID, watchdog timestamp, and an accept thread (`tci/mod.rs:31-53,112-219`).
- **VERIFIED CURRENT BEHAVIOR:** Each accepted WebSocket runs in a separate OS thread. The configured connection limit is eight (`tci/mod.rs:112-219`).
- **VERIFIED CURRENT BEHAVIOR:** URL path hints distinguish `/control` and `/media`; direct clients can also declare and pair a session lane in-band (`tci/client.rs`, `tci/session_pair.rs`).
- **TECHNICAL DEBT:** The accept loop and detached client threads have no shared shutdown token or join handles. Process exit, rather than cooperative session shutdown, terminates them.

### 3.2 Client/session authority

- **VERIFIED CURRENT BEHAVIOR:** The first eligible loopback client becomes `Operator`; non-loopback clients are always `Viewer` (`tci/client.rs`).
- **VERIFIED CURRENT BEHAVIOR:** When the operator disconnects, only the first remaining eligible loopback non-media client may be promoted. A LAN accessory is never promoted (`tci/client.rs`).
- **VERIFIED CURRENT BEHAVIOR:** Viewers are restricted to a small command whitelist; an operator or its paired media lane may submit binary microphone frames (`tci/protocol.rs:1141-1157`, `tci/client.rs:632-731`).
- **TECHNICAL DEBT:** Loopback operator assignment is still connection-order arbitration rather than an authenticated per-session capability. Saturn Go authenticates access to its proxy, but the bridge trusts the loopback boundary.
- **PROPOSED CHANGE:** Make roles explicit session claims supplied through a trusted Saturn Go-to-bridge mechanism. Never promote an accessory/viewer to operator merely because another connection ended.

### 3.3 Control/status surface and amplifier requirement

- **VERIFIED CURRENT BEHAVIOR:** Incoming TCI commands include VFO, DDS, mode, filters, DSP controls, stream controls, microphone frames, and `trx`/TX enable (`tci/protocol.rs:25-139,607-625`).
- **VERIFIED CURRENT BEHAVIOR:** The connection snapshot sends the standard initialization/capability messages `PROTOCOL`, `DEVICE`, `RECEIVE_ONLY`, `TRX_COUNT`, `CHANNEL_COUNT`, `VFO_LIMITS`, `IF_LIMITS`, and `MODULATIONS_LIST`, and sends `READY` only after the complete state snapshot (`tci/client.rs`).
- **VERIFIED CURRENT BEHAVIOR:** State publication sends both Saturn extensions and standard TCI names for VFO, split, DDS, modulation, RX filter, AGC, NB/NR/ANF, drive, TX permission, TX state, and sensors (`tci/client.rs`, `tci/mod.rs`).
- **VERIFIED CURRENT BEHAVIOR:** The bridge publishes the standard server-only `TX_FREQUENCY:<hz>;` notification from `RadioModel.desired.tx_frequency_hz` on connection and every tuning/split update. `sync_vfo_routes` makes this the active VFO in simplex and the opposite VFO in split (`tci/client.rs`, `tci/mod.rs`, `radio_model.rs`).
- **VERIFIED CURRENT BEHAVIOR:** Standard read forms such as `VFO:0,0;`, `DDS:0;`, `MODULATION:0;`, `TRX:0;`, and `DRIVE:0;` return a current state snapshot only to the requesting client. A LAN viewer can read these values but cannot use their set forms (`tci/protocol.rs`, `tci/mod.rs`, `tci/command_queue.rs`).
- **VERIFIED CURRENT BEHAVIOR:** No private `band:` notification exists. The official TCI accessory mechanism is the authoritative `TX_FREQUENCY` notification, from which an amplifier selects its band.
- **INTENDED DESIGN:** The amplifier must receive authoritative operating/TX frequency and band information for automatic tuning, without acquiring control or transmit authority.
- **PROPOSED CHANGE:** Confirm the Windows amplifier client's exact command spelling/case and connection behavior in a read-only integration test. Add a private `band:` extension only if that client cannot consume standard `TX_FREQUENCY`.

## 4. Authoritative radio state

- **VERIFIED CURRENT BEHAVIOR:** `RadioModel` contains `DesiredRadioState` and `ObservedRadioState` and is shared as `Arc<Mutex<RadioModel>>` (`radio_model.rs:374-491`).
- **VERIFIED CURRENT BEHAVIOR:** Desired state holds running/TX state, VFO and split routing, DDC configuration, mode/filter/DSP settings, antennas, attenuation, drive, and PureSignal settings. Observed state holds counters, meters, power/SWR, and PureSignal observations (`radio_model.rs:374-489`).
- **VERIFIED CURRENT BEHAVIOR:** `sync_vfo_routes` derives the RX IQ center and TX frequency from active VFO and split state (`radio_model.rs:491-507`).
- **VERIFIED CURRENT BEHAVIOR:** Runtime TX truth is also represented in `tx_requested`, `controller_owned`, release/watchdog locals, the TX thread's `TxState`, TCI session state, and direct-XDMA `DirectTxControl`/`DirectTxState`.
- **TECHNICAL DEBT:** `RadioModel` is the intended shared model but is not the only authoritative state machine. Multiple local copies can temporarily disagree, and `TxPhase` has `Rx`, `Armed`, and `Keyed` but no explicit disarming/fault phase (`radio_model.rs:334-340`).
- **TECHNICAL DEBT:** P2 hardware status updates counters and telemetry, but the bridge does not ingest an independently changed hardware VFO/mode/filter into the desired control model. Bidirectional state is therefore partial.
- **PROPOSED CHANGE:** Introduce one safety-owned TX state machine with explicit request, arm, keyed, disarming, fault, and RX transitions. Other threads should submit events and consume snapshots instead of owning parallel TX booleans.

## 5. Protocol 2 ownership and sockets

### 5.1 Socket ownership and ports

- **VERIFIED CURRENT BEHAVIOR:** `P2Session` owns one bound UDP socket, a receive lock, discovery exclusivity, destination addresses, and sequence counters (`p2/session.rs:35-48`).
- **VERIFIED CURRENT BEHAVIOR:** It sends General to 1024, DDC-specific to 1025, DUC-specific to 1026, high-priority control to 1027, and DUC IQ to 1029; it receives high-priority status from 1025 and DDC IQ from 1035 plus DDC index (`p2/ports.rs`, `p2/session.rs`).
- **VERIFIED CURRENT BEHAVIOR:** `decode_event` selects packet type from the UDP source port (`p2/session.rs:239-243,356-389`).
- **TECHNICAL DEBT:** P2 receive dispatch does not validate the source IP against the configured radio address. The loopback default reduces exposure, but a widened bind could admit source-port-matching packets from another host.

### 5.2 Controller lease

- **VERIFIED CURRENT BEHAVIOR:** The first TCI client causes the bridge to bootstrap and acquire a P2 controller role; the last client disarms TX and releases that role immediately or after a configured grace period (`main.rs:603-643,945-1020`).
- **VERIFIED CURRENT BEHAVIOR:** Bootstrap requires idle discovery, then sends General, DDC-specific, and DUC-specific configuration (`p2/session.rs:141-181`).
- **VERIFIED CURRENT BEHAVIOR:** `send_stop` sends a final high-priority packet with `run=false` and `tx=false` (`p2/session.rs:250-272`).
- **VERIFIED CURRENT BEHAVIOR:** P2_app claims its controller lease on a General packet and gates high-priority, DDC-specific, DUC-specific, DUC IQ, and speaker traffic through the lease (`P2_app/p2app.c:1805-1823`, `controller_lease.c`, the corresponding `In*.c` handlers).
- **VERIFIED CURRENT BEHAVIOR:** P2_app identifies the lease only by source IPv4 address, not UDP source port, process, or session token (`P2_app/controller_lease.c:7-61`).
- **TECHNICAL DEBT:** Two controllers on the same Pi both appear as `127.0.0.1` and satisfy the same lease. The current lease cannot enforce “one authoritative local controller.”
- **PROPOSED CHANGE:** Extend the lease identity to at least source IP plus the controller's negotiated/source port, or use an explicit session token established by General/discovery. Preserve compatibility through a versioned transition.

### 5.3 High-priority generation

- **VERIFIED CURRENT BEHAVIOR:** A dedicated bridge thread emits high-priority control at a faster cadence during TX and at a configured RX period otherwise (`p2/session.rs:184-219`).
- **VERIFIED CURRENT BEHAVIOR:** Each high-priority packet carries run/TX flags, RX and TX phase words, drive, ALEX routing/filter data, DDC state, and attenuation derived from `RadioModel` (`p2/session.rs:417-455`).
- **VERIFIED CURRENT BEHAVIOR:** P2_app applies the accepted transmit bit to hardware MOX and TX enable, and releases the lease on `run=false` (`P2_app/InHighPriority.c:267-337`, `protocol2_command.c:553-597`).

## 6. RX/DDC and WDSP receive path

- **VERIFIED CURRENT BEHAVIOR:** P2 mode owns a named `saturn-rx` thread. It drains RX control commands, reads the shared P2 socket, parses high-priority/DDC events, and reports fatal socket or WDSP errors to main (`rx_thread.rs:55-165`).
- **VERIFIED CURRENT BEHAVIOR:** Matching DDC IQ is published to TCI display clients, sent through RX WDSP when not transmitting, packetized as audio, and published to TCI audio clients (`rx_thread.rs:169-242`).
- **VERIFIED CURRENT BEHAVIOR:** During TX request/keying, RX WDSP is suspended while display IQ remains available. DDC0 may instead feed PureSignal TX reference/RX feedback (`rx_thread.rs:194-230`, `wdsp.rs:820-860`).
- **VERIFIED CURRENT BEHAVIOR:** `WdspRxEngine` owns WDSP channel 0, input/output buffers, pending IQ/audio deques, DSP configuration, and meter state; `Drop` destroys blankers and closes the channel (`wdsp.rs:578-673,862-960,1356-1367`).
- **VERIFIED CURRENT BEHAVIOR:** Direct-XDMA mode performs operational DDC reads and RX WDSP processing in the backend's main loop instead of the P2 RX thread (`xdma_backend.rs`).
- **TECHNICAL DEBT:** TCI receiver index, DDC index, and WDSP channel are implicit single-receiver conventions rather than one explicit mapping object. This is unsafe groundwork for multi-receiver expansion.
- **PROPOSED CHANGE:** Add an explicit `ReceiverContext` mapping TCI receiver, DDC, WDSP channel, sample rate, frequency, antenna, and enabled state; make every RX route resolve through it.

## 7. TX/DUC and WDSP transmit path

### 7.1 Shared TX owner

- **VERIFIED CURRENT BEHAVIOR:** Both backends use one TX worker and the `TxRadio` trait. The worker owns `WdspTxEngine`, microphone pacing, DUC packetization, key qualification, TX diagnostics, and arm/key/unkey transitions (`tx_thread.rs:58-159,351-428`).
- **VERIFIED CURRENT BEHAVIOR:** TCI `trx=true` does not immediately key RF. Main verifies controller/mode, records a request, sets phase `Armed`, resets RX buffers, and sends `TxCommand::Arm` (`main.rs:645-685`).
- **VERIFIED CURRENT BEHAVIOR:** The TX worker requires RF enable, keyable WDSP IQ, recent above-threshold mic audio (or two-tone), and backend key qualification before keying (`tx_thread.rs:1035-1066`).
- **VERIFIED CURRENT BEHAVIOR:** For P2, the transition packet is handled by `try_key_with_iq`: it sends a high-priority `tx=true` packet first, then the DUC IQ packet (`tx_thread.rs:169-178,1068-1145`).
- **VERIFIED CURRENT BEHAVIOR:** For direct XDMA, IQ is staged/prefilled with RF inhibited; TX configuration, relay, TX enable, MOX, drive, and register readback are applied in a deterministic sequence (`xdma_tx_radio.rs:244-374,700-744`).

### 7.2 WDSP lifecycle

- **VERIFIED CURRENT BEHAVIOR:** `WdspTxEngine` owns WDSP channel 1 and configures it stopped with `SetPSMox(0)` during creation (`wdsp.rs:1380-1495,1540-1620`).
- **VERIFIED CURRENT BEHAVIOR:** Activating TX clears buffers then runs the channel. Deactivation calls `SetPSMox(0)` before channel state 0, feeds a bounded down-slew, discards flush output, and marks the channel for recreation if the flush fails (`wdsp.rs:1912-1979`).
- **VERIFIED CURRENT BEHAVIOR:** Normal `do_unkey` disables PureSignal MOX and WDSP before sending hardware TX-off packets (`tx_thread.rs:1410-1462`).
- **VERIFIED CURRENT BEHAVIOR:** `WdspTxEngine::drop` defensively calls `SetPSMox(0)` and stops the channel before close. Explicit network/hardware disarm remains outside `Drop` (`wdsp.rs`).

### 7.3 Direct-XDMA safety

- **VERIFIED CURRENT BEHAVIOR:** Direct TX refuses to open while P2_app is active and is qualified for a specific primary Saturn PCB/FPGA version (`xdma_tx_radio.rs:193-231`).
- **VERIFIED CURRENT BEHAVIOR:** Direct TX enforces forward-power, reverse-power, and SWR trips and calls shutdown on a trip (`xdma_tx_radio.rs:680-713`).
- **VERIFIED CURRENT BEHAVIOR:** `DirectTxState::shutdown` zeros drive, disables DUC amplitude/output, clears MOX/TX enable, inhibits the TX relay, releases ALEX TX relay, resets the FIFO, and marks the stream unkeyed (`xdma_tx_radio.rs:760-810`).
- **VERIFIED CURRENT BEHAVIOR:** `DirectTxState::drop` retries that shutdown as emergency cleanup (`xdma_tx_radio.rs:844-850`).

## 8. MOX/PTT state transitions and all key/dekey sites

| Status | Site | Current effect |
|---|---|---|
| VERIFIED CURRENT BEHAVIOR | `tci/protocol.rs:607-625` | Parses `trx` into `TciCommand::SetTxEnabled`; it does not directly touch hardware. |
| VERIFIED CURRENT BEHAVIOR | `main.rs:645-702` | Validates/arms/disarms the P2 request and updates shared desired state. |
| VERIFIED CURRENT BEHAVIOR | `xdma_backend.rs` command handler | Performs the analogous direct-backend request validation and forwards Arm/Disarm. |
| VERIFIED CURRENT BEHAVIOR | `tx_thread.rs:1068-1145` | Sole normal transition from armed media/DSP state to backend key. |
| VERIFIED CURRENT BEHAVIOR | `tx_thread.rs:169-178` | P2 key site: sends HP TX true, then first DUC IQ. |
| VERIFIED CURRENT BEHAVIOR | `xdma_tx_radio.rs:318-374` | Production direct-XDMA key site: prefill, TX configuration/relay, TX enable, MOX, drive, readback. |
| VERIFIED CURRENT BEHAVIOR | `tx_thread.rs:1410-1462` | Normal shared unkey: WDSP/PS off, 12 P2 TX-false packets or direct shutdown, RX DDC/DUC restore. |
| VERIFIED CURRENT BEHAVIOR | `xdma_tx_radio.rs:760-850` | Direct-XDMA explicit and drop-time emergency hardware dekey. |
| VERIFIED CURRENT BEHAVIOR | `xdma_tx.rs:520-585` | Separate guarded diagnostic TX probe can assert direct hardware TX/MOX; it is CLI-only, not the service path. |
| VERIFIED CURRENT BEHAVIOR | `xdma.rs` and `xdma_duc.rs` | Probe/cleanup guards clear MOX and TX enable; these are diagnostic paths. |
| VERIFIED CURRENT BEHAVIOR | `P2_app/InHighPriority.c:57-60,267-337` | Final P2 hardware key/dekey sink applies accepted high-priority transmit state. |
| VERIFIED CURRENT BEHAVIOR | `P2_app/p2app.c:1055-1120` | Inactivity watchdog forces MOX/TX enable/CW false and clears controller state. |
| VERIFIED CURRENT BEHAVIOR | `P2_app/p2app.c:1127-1200` | P2_app shutdown forces RF safe before and after worker joins. |

- **TECHNICAL DEBT:** TX state transitions are spread across TCI parsing, main/backend control, TX worker, backend output, WDSP, and P2_app. There is no single transition table whose completion is acknowledged at every layer.
- **PROPOSED CHANGE:** Implement one event-driven safety engine and require an acknowledged `RX_SAFE` terminal state from DSP and the selected hardware backend for every release, error, and shutdown.

## 9. Watchdogs and safety enforcement

| Status | Watchdog | Current behavior |
|---|---|---|
| VERIFIED CURRENT BEHAVIOR | Maximum TX duration | TX worker defaults to 180 seconds and clamps configuration to 3–180 seconds (`tx_thread.rs:35-39,834-859`). |
| VERIFIED CURRENT BEHAVIOR | TX source stall | Real mic-frame arrival must remain fresh; default 2 seconds, configurable/clamped 0.5–10 seconds; filler does not refresh it (`tx_thread.rs:41-45,861-889`). |
| VERIFIED CURRENT BEHAVIOR | P2 uplink freshness | Main trips after mic age exceeds 500 ms for a sustained 100 ms while on-air (`main.rs:1120-1163`). |
| VERIFIED CURRENT BEHAVIOR | Control freshness | Main independently trips stale operator control; limit depends on legacy versus paired split session (`main.rs:1166-1194`). |
| VERIFIED CURRENT BEHAVIOR | Direct control/media | Direct backend independently checks mic and control timestamps and sends Disarm (`xdma_backend.rs:374-401`). |
| VERIFIED CURRENT BEHAVIOR | P2_app inactivity | Once per second, missing activity with the hardware watchdog enabled forces RF off and clears the lease (`P2_app/p2app.c:1055-1120`). |
| VERIFIED CURRENT BEHAVIOR | P2 bridge power | P2 main trips forward power above the configured threshold (`main.rs:1090-1111`). |
| VERIFIED CURRENT BEHAVIOR | Direct power/SWR | Direct backend trips forward power, reverse power, or high SWR (`xdma_tx_radio.rs:680-713`). |

- **TECHNICAL DEBT:** The P2 bridge path lacks the direct backend's explicit reverse-power and SWR trip in its own safety layer. It currently relies on forward-power limiting plus whatever protection exists below the bridge.
- **TECHNICAL DEBT:** P2_app's inactivity check is one-second-granularity and can be enabled/disabled by protocol configuration; it is an independent last line of defense, not a substitute for explicit bridge shutdown.
- **INTENDED DESIGN:** Control health, maximum TX duration, media freshness, backend readiness, and FPGA FIFO safety remain independent watchdogs. Media silence/filler must not reset control or maximum-duration watchdogs.
- **PROPOSED CHANGE:** Normalize safety events into named counters and state transitions, and add P2-path reverse-power/SWR policy based on validated telemetry/calibration.

## 10. Audio/media paths and clock domains

### 10.1 Current TCI/WebSocket audio

- **VERIFIED CURRENT BEHAVIOR:** Browser microphone audio currently arrives as TCI binary WebSocket frames. An operator or its paired media client may enqueue `MicAudioFrame`; codec faults can force RX (`tci/client.rs:632-731`).
- **VERIFIED CURRENT BEHAVIOR:** RX WDSP audio is packetized and placed on each client's outbound audio queue; control and media can use distinct WebSockets, but both still use TCI/WebSocket framing (`rx_thread.rs:215-230`, `tci/outbound.rs`).
- **VERIFIED CURRENT BEHAVIOR:** Standard float32 RX audio uses TCI stream type 1 and accepted microphone audio uses stream type 2. The bridge also supports a negotiated Opus extension on the same WebSocket media lane (`tci/protocol.rs`, `tci/outbound.rs`, `tx_codec.rs`).
- **TECHNICAL DEBT:** The custom TX-IQ display frame uses TCI stream type 3, which the official protocol reserves for `TX_CHRONO`. Saturn Remote currently relies on that private interpretation, so changing it without a versioned capability would break existing clients (`tci/outbound.rs`, `tci/tests.rs`).
- **TECHNICAL DEBT:** The standard `TX_CHRONO` request/pacing exchange for third-party TCI TX-audio clients is not implemented. Current Saturn Remote sends type-2 microphone frames using its own negotiated pacing and safety telemetry.
- **PROPOSED CHANGE:** Allocate a private, explicitly negotiated stream type for TX-IQ display and then implement standard `TX_CHRONO` pacing independently. Preserve the existing stream type until both bridge and Saturn Remote negotiate the replacement.
- **TECHNICAL DEBT:** The `/media` lane is a WebSocket scheduling separation, not the proposed native SATP/UDP transport.

### 10.2 Rate matching

- **VERIFIED CURRENT BEHAVIOR:** TX consumes microphone samples on a steady 48 kHz schedule, performs bounded catch-up, and fills only short gaps (`tx_thread.rs:891-1034`).
- **VERIFIED CURRENT BEHAVIOR:** The legacy mic deque is capped at 48,000 samples and drops oldest samples on overflow (`tx_thread.rs:48,1400-1407`).
- **VERIFIED CURRENT BEHAVIOR:** A WDSP `rmatch`-based microphone rate matcher and occupancy diagnostics exist, but are enabled only when `SATURN_BRIDGE_TX_MIC_RMATCH` opts in (`wdsp.rs:449-568`, `tx_thread.rs:1329`).
- **TECHNICAL DEBT:** Clock-domain correction is not the default path. Long-running independent Windows/network/FPGA clocks therefore depend on queue/drop/fill behavior unless the experimental matcher is enabled and validated.
- **INTENDED DESIGN:** Occupancy-driven rate matching targets a bounded FIFO midpoint and reports ratio/correction ppm; it never turns audio arrival into PTT authority.

### 10.3 Native SATP/UDP boundary

- **VERIFIED CURRENT BEHAVIOR:** No `satp` module, SATP packet parser, native UDP audio socket, jitter/reorder buffer, or SATP loss/reorder telemetry exists in `saturn-bridge` in this checkout.
- **INTENDED DESIGN:** TCI remains the control/status/negotiation plane. SATP/UDP is a separate low-latency media plane from the Windows native audio client to Saturn-side TX DSP.
- **INTENDED DESIGN:** TX may occur only when an authorized control-plane state and valid media/DSP/backend conditions are all true. SATP packets alone never arm, key, extend, or re-authorize TX.
- **PROPOSED CHANGE:** Add a separate `satp/` subsystem with a bounded sequence-aware jitter buffer, explicit session identity negotiated over TCI, replay/stale rejection, rate matching, and loss/reorder/jitter counters.
- **PROPOSED CHANGE:** Feed both current TCI mic frames and future SATP frames into a common bounded `TxAudioIngress` abstraction after transport-specific validation. Keep PTT/release on the prioritized control path.

## 11. Queue inventory and overload policy

| Status | Queue/buffer | Capacity | Overload behavior |
|---|---|---:|---|
| VERIFIED CURRENT BEHAVIOR | TCI safety command mailbox | 16 commands | Same command kind is coalesced; at capacity the oldest safety command is removed (`tci/command_queue.rs:10-12,108-125`). |
| VERIFIED CURRENT BEHAVIOR | TCI control command mailbox | 256 commands | Latest matching setting replaces old; otherwise oldest control command drops (`command_queue.rs:132-155`). |
| VERIFIED CURRENT BEHAVIOR | TCI mic command mailbox | 8 frames | Drop oldest mic frame and increment counter (`command_queue.rs:126-131`). |
| VERIFIED CURRENT BEHAVIOR | Per-client outbound safety | 16 messages | Coalesce keyed messages; otherwise pop oldest and count depth overflow (`tci/outbound.rs:314-347,519-521`). |
| VERIFIED CURRENT BEHAVIOR | Per-client outbound control | 256 messages and 256 KiB | Coalesce latest setting; drop oldest until both limits hold (`outbound.rs:348-376,519-521`). |
| VERIFIED CURRENT BEHAVIOR | Per-client display | 1 frame | Replace latest and count replacement (`outbound.rs:381-390`). |
| VERIFIED CURRENT BEHAVIOR | Per-client audio | 250 ms at negotiated sample rate | Drop oldest; if already at the ceiling, panic-drain stale audio (`outbound.rs:397-429,562-565`). |
| VERIFIED CURRENT BEHAVIOR | TCP kernel outbound gate | 64 KiB | Bulk audio/display pauses while queued kernel bytes exceed limit; safety/control remain prioritized (`outbound.rs:668-724`). |
| VERIFIED CURRENT BEHAVIOR | TX legacy mic samples | 48,000 mono samples | Drop oldest samples and retain newest (`tx_thread.rs:48,1400-1407`). |
| VERIFIED CURRENT BEHAVIOR | TX `MicRateMatcher` | Native `rmatch` ring plus small staging vector | Native occupancy/rate matching; enabled only by environment opt-in (`wdsp.rs:449-568`). |
| VERIFIED CURRENT BEHAVIOR | Main → TX commands | Unbounded `std::sync::mpsc` | No admission limit; can carry microphone vectors (`main.rs:317`, `xdma_backend.rs:204`). |
| VERIFIED CURRENT BEHAVIOR | TX → main events | Unbounded `std::sync::mpsc` | No admission limit; can carry TX IQ display vectors and diagnostics (`main.rs:318`, `xdma_backend.rs:205`). |
| VERIFIED CURRENT BEHAVIOR | Main → RX commands | Unbounded `std::sync::mpsc` | No admission limit (`main.rs:319`). |
| VERIFIED CURRENT BEHAVIOR | RX → main events | Unbounded `std::sync::mpsc` | No admission limit (`main.rs:320`). |
| VERIFIED CURRENT BEHAVIOR | RX WDSP pending IQ/audio | Structurally unbounded `VecDeque` | Drained synchronously in `push_iq`; no explicit hard cap (`wdsp.rs:610-611,862-944`). |
| VERIFIED CURRENT BEHAVIOR | TX WDSP pending mic/IQ and PureSignal | Structurally unbounded `VecDeque` | Cleared on lifecycle transitions and normally drained synchronously; no explicit hard cap (`wdsp.rs:1393-1406,1912-1921`). |

- **TECHNICAL DEBT:** The four unbounded inter-thread channels violate the realtime bounded-queue requirement and are the clearest source-level memory-growth path under producer/consumer imbalance.
- **TECHNICAL DEBT:** Safety queues are bounded but can evict an oldest non-coalesced safety item. Hardware PTT release currently also travels through additional state/Disarm paths, but “release is never dropped” is not encoded as a queue invariant.
- **PROPOSED CHANGE:** Replace realtime inter-thread channels with bounded, typed lanes: non-droppable/coalesced safety state, latest-wins control state, bounded drop-oldest media, and priority fault/shutdown signaling.

## 12. Thread/task ownership

| Status | Execution owner | Responsibility |
|---|---|---|
| VERIFIED CURRENT BEHAVIOR | Main thread, P2 mode | TCI command application, controller lease lifecycle, state publication, P2 watchdogs, telemetry summaries. |
| VERIFIED CURRENT BEHAVIOR | TCI accept thread | Accept up to eight WebSockets. |
| VERIFIED CURRENT BEHAVIOR | Per-TCI-client thread | WebSocket read/write scheduling, parsing, session lane/role handling. |
| VERIFIED CURRENT BEHAVIOR | P2 high-priority thread | Periodic run/TX/frequency/drive/ALEX control packets. |
| VERIFIED CURRENT BEHAVIOR | `saturn-rx` thread | P2 receive, DDC routing, RX WDSP, RX audio, PureSignal feedback extraction. |
| VERIFIED CURRENT BEHAVIOR | TX thread | WDSP TX, mic pacing/rmatch, arm/key/unkey, DUC IQ output, TX watchdogs/diagnostics. |
| VERIFIED CURRENT BEHAVIOR | Direct-XDMA main loop | TCI control plus hardware DDC polling and RX WDSP; replaces P2 RX/high-priority threads. |
| VERIFIED CURRENT BEHAVIOR | P2_app worker threads | XDMA/FPGA command, DDC, DUC, speaker, telemetry, panel/CAT, and inactivity watchdog work. |
- **TECHNICAL DEBT:** Direct-XDMA and P2 modes share TX code but have different RX/control-loop ownership and cleanup strength, increasing the chance of backend-specific lifecycle drift.
- **PROPOSED CHANGE:** Keep the backend abstraction, but give both modes the same top-level runtime guard, shutdown event, safety state machine, and bounded channel contract.

## 13. Shutdown and error paths

### 13.1 P2 mode

- **VERIFIED CURRENT BEHAVIOR:** P2 mode installs SIGINT/SIGTERM handlers. A signal requests an orderly loop exit rather than relying on process termination (`main.rs`).
- **VERIFIED CURRENT BEHAVIOR:** `P2RuntimeSafetyGuard` covers early `?`/fatal returns after worker startup: it clears requested/model TX state, queues Disarm and Shutdown, sends the final `run=false`/`tx=false` packet, and stops workers (`main.rs`).
- **VERIFIED CURRENT BEHAVIOR:** Normal cleanup lets the TX worker explicitly unkey before setting the shared worker stop flag, then releases the P2 controller and joins all workers (`main.rs`, `tx_thread.rs`).
- **TECHNICAL DEBT:** TCI accept/client threads remain detached and are terminated by process exit rather than cooperatively joined after radio cleanup.

### 13.2 Direct-XDMA mode

- **VERIFIED CURRENT BEHAVIOR:** Direct-XDMA installs SIGINT/SIGTERM handlers that set an atomic stop request (`xdma_backend.rs:120-166`).
- **VERIFIED CURRENT BEHAVIOR:** After its runtime closure ends, it sends Disarm and Shutdown, joins TX, stops RX, records stopped readiness, and reports verified receive-safe cleanup before returning (`xdma_backend.rs:487-520`).
- **VERIFIED CURRENT BEHAVIOR:** Direct hardware state also has `Drop`-time emergency shutdown (`xdma_tx_radio.rs:760-850`).
- **TECHNICAL DEBT:** The safer direct-XDMA cleanup architecture has not been generalized to the default P2 path.

## 14. Observability

- **VERIFIED CURRENT BEHAVIOR:** Once-per-second P2 diagnostics already include HP/DDC/audio rates, TCI client/session state, queue depths/high-water marks/drops, transport backlog, mic age/gaps/drops, codec faults, and TX diagnostics (`main.rs:1198-1262`).
- **VERIFIED CURRENT BEHAVIOR:** Direct-XDMA periodic status reports DDC DMA/sample counts, FIFO thresholds/faults, TX request/stream/key state, DUC DMA/frame/FIFO metrics, forward/reverse power, and SWR (`xdma_backend.rs:427-477`).
- **VERIFIED CURRENT BEHAVIOR:** State transitions and watchdog faults are logged rather than every realtime packet.
- **TECHNICAL DEBT:** There is no SATP loss/reorder/jitter telemetry because SATP is absent. Shutdown reason, controller identity changes, rate-correction ppm, and per-backend safety transition durations are not yet one structured metrics model.
- **PROPOSED CHANGE:** Add stable counters/gauges and a structured terminal shutdown reason that Saturn Go can display without scraping journal strings.

## 15. Intended target boundary

- **INTENDED DESIGN:** Saturn Bridge is the authoritative realtime radio/DSP/safety service.
- **INTENDED DESIGN:** Saturn Go owns TLS, authentication, UI, orchestration, configuration, diagnostics, update management, and external proxying; it does not become a second Protocol 2 or XDMA controller.
- **INTENDED DESIGN:** P2_app remains the normal FPGA/XDMA transport service in P2 mode. Direct XDMA remains an explicit, exclusive backend selection rather than an accidental second owner.
- **INTENDED DESIGN:** TCI owns control, status, capability and SATP session negotiation. SATP/UDP owns native low-latency audio. Audio arrival never implies PTT.
- **INTENDED DESIGN:** The Windows native audio client may source high-quality audio from Voicemeeter Virtual ASIO, but the Saturn-side bridge retains WDSP, DUC generation, TX authorization, watchdogs, and fail-safe disarm.

```text
                         Saturn Bridge
                               |
          +--------------------+--------------------+
          |                    |                    |
     TCI control          Radio state          Safety engine
          |                    |                    |
          +-------------+------+--------+-----------+
                        |               |
                    RX engine       TX engine
                        |               |
                       WDSP            WDSP
                        |               |
                     DDC IQ          DUC IQ
                        \               /
                         selected backend
                         /              \
                 P2 UDP / P2_app     direct XDMA
                         \              /
                          Saturn FPGA

     SATP/UDP is a separate bounded media ingress into TX engine;
     it has no edge to TX authorization.
```

## 16. Prioritized discrepancy register

### P0 — TX safety / possible unintended RF transmission

1. **VERIFIED CURRENT BEHAVIOR:** P2 SIGTERM/fatal-return cleanup now explicitly disarms TX and releases the controller through a runtime safety guard. WDSP Drop independently forces local MOX false.
2. **VERIFIED CURRENT BEHAVIOR:** Non-loopback raw TCI clients are viewer-only and cannot become or inherit operator/TX authority, including through split-session claims.
3. **TECHNICAL DEBT:** P2_app controller ownership is keyed only by source IPv4 address, so conflicting local controllers share one lease identity.
   - **PROPOSED CHANGE:** Strengthen lease identity and test two same-host controller processes with conflicting DUC/HP traffic.
4. **TECHNICAL DEBT:** The P2 bridge safety layer trips forward power only, unlike direct XDMA's forward/reverse/SWR policy.
   - **PROPOSED CHANGE:** Add calibrated P2 reverse-power/SWR trips without weakening the existing independent watchdogs.

### P1 — data corruption / realtime stream correctness

1. **TECHNICAL DEBT:** Four bridge inter-thread channels are unbounded and carry realtime vectors. Sustained imbalance can grow memory and latency, consistent with the class of failure suspected during development, although this audit does not prove that they caused the prior Pi crash.
   - **PROPOSED CHANGE:** Convert them to bounded policy-specific queues and export depth/high-water/drop counters.
2. **TECHNICAL DEBT:** P2 receive dispatch trusts source port without source-IP validation.
   - **PROPOSED CHANGE:** Validate the configured radio endpoint/session before applying status or DDC frames.
3. **TECHNICAL DEBT:** Native SATP sequence, jitter, reorder, stale/replay, and clock-domain handling does not exist yet.
   - **PROPOSED CHANGE:** Implement and fuzz the bounded SATP ingress independently of TCI control before connecting it to TX WDSP.

### P2 — lifecycle / race / resource leak

1. **TECHNICAL DEBT:** TX truth is duplicated across model, main/backend locals, atomics, TX worker state, and hardware backend state.
   - **PROPOSED CHANGE:** Use one acknowledged event-driven TX safety state machine.
2. **TECHNICAL DEBT:** TCI accept/client threads lack cooperative shutdown and joins.
   - **PROPOSED CHANGE:** Give the frontend a stop token and owned join handles; close sessions after TX is made safe.
3. **VERIFIED CURRENT BEHAVIOR:** TCI bind changes leave an inactive bridge stopped and restart an active bridge through one broker-held ownership transaction, with configuration rollback on restart failure.
4. **VERIFIED CURRENT BEHAVIOR:** Generated bridge units now limit startup to five attempts per 60 seconds and use a five-second restart delay, preventing the observed unbounded two-second restart storm (`scripts/install-saturn-bridge.sh`, `scripts/saturn-go-deploy-root.sh`).
5. **TECHNICAL DEBT:** Persistent hardware-readiness failures still need a structured terminal fault surfaced in Saturn Go rather than relying only on systemd's start-limit state and journal text.

### P3 — latency / DSP / pacing performance

1. **TECHNICAL DEBT:** Microphone `rmatch` exists but is opt-in, so long-duration clock drift is not uniformly corrected.
   - **PROPOSED CHANGE:** Validate occupancy/ppm behavior with the Windows source, then make the proven matcher the normal native-audio path.
2. **TECHNICAL DEBT:** TCI `/media` still uses WebSocket framing and scheduling; it is not the target native low-latency path.
   - **PROPOSED CHANGE:** Keep it for browser compatibility while SATP receives a dedicated UDP worker and bounded jitter buffer.

### P4 — protocol correctness

1. **VERIFIED CURRENT BEHAVIOR:** The bridge now publishes standard initialization, standard control aliases, read replies, and authoritative `TX_FREQUENCY`; no private `band:` message is needed for a conforming accessory.
   - **PROPOSED CHANGE:** Validate the actual Windows amplifier client against this read-only interface before considering a non-standard band extension.
2. **TECHNICAL DEBT:** TCI stream type 3 is occupied by the legacy TX-IQ display extension even though the standard assigns it to `TX_CHRONO`; standard TX-audio pacing is therefore incomplete.
   - **PROPOSED CHANGE:** Migrate the private TX-IQ stream through capability negotiation, then add `TX_CHRONO` interoperability tests.
3. **TECHNICAL DEBT:** Hardware-to-TCI synchronization is telemetry-heavy but does not make external VFO/mode/filter changes authoritative in `RadioModel`.
   - **PROPOSED CHANGE:** Define explicit desired-versus-observed reconciliation rules and publish only reconciled state.
4. **TECHNICAL DEBT:** Receiver/DDC/WDSP/TCI indices are implicit single-receiver conventions.
   - **PROPOSED CHANGE:** Add an explicit receiver mapping before multi-receiver support.

### P5 — maintainability / cleanup

1. **TECHNICAL DEBT:** Backend-specific lifecycle code is split between a large P2 `main.rs` and `xdma_backend.rs`; safety behavior has already diverged.
   - **PROPOSED CHANGE:** Extract `runtime`, `safety`, `radio_state`, `protocol2`, `rx`, `tx`, `audio`, `satp`, `watchdog`, and `metrics` boundaries incrementally, preserving behavior with tests.
2. **TECHNICAL DEBT:** Existing architecture documentation describes the bridge at a higher level but does not inventory all queues, key/dekey sites, or backend-specific cleanup.
   - **PROPOSED CHANGE:** Keep this audit updated as implementation changes and require status labels on future architectural assertions until verified.

## 17. Recommended implementation order

1. **VERIFIED CURRENT BEHAVIOR:** P2 shutdown/error dekey guarantees and defensive WDSP cleanup are implemented in this worktree.
2. **VERIFIED CURRENT BEHAVIOR:** Raw LAN TCI accessories are viewer-only; Settings preserves P2-first boot and uses brokered restart with rollback.
3. **PROPOSED CHANGE:** Add mocked signal/fatal-exit integration tests that verify final P2 packet ordering while TX is armed/keyed.
4. **PROPOSED CHANGE:** Strengthen P2_app controller lease identity for same-host controllers.
5. **PROPOSED CHANGE:** Bound the four inter-thread queues and preserve priority/coalescing semantics.
6. **VERIFIED CURRENT BEHAVIOR:** Official TCI `TX_FREQUENCY` now supplies the authoritative split-aware amplifier frequency. A live read-only Windows-client interoperability check remains outstanding.
7. **PROPOSED CHANGE:** Obtain/version the Windows SATP wire contract, then add the SATP module as a separate UDP media plane with bounded buffering, sequence handling, observability, and no PTT authority.
8. **PROPOSED CHANGE:** Validate and enable occupancy-driven rate matching for the native Windows audio clock.
9. **PROPOSED CHANGE:** Consolidate TX state and backend lifecycle only after safety/behavior tests cover both P2 and direct-XDMA modes.

## 18. Live incident snapshot (read-only)

- **VERIFIED CURRENT BEHAVIOR:** At the 2026-08-31 inspection, the backend broker reported `selected=p2`, `runtime=p2`, `operational_status=ready`, mutual exclusion true, P2_app active, and Saturn Bridge inactive. The bridge unit was disabled and P2_app enabled, matching the intended clean-boot posture.
- **VERIFIED CURRENT BEHAVIOR:** The earlier flapping was a direct-XDMA bridge restart storm. Each process exited with `operational XDMA RX FIFO remained over threshold after 16 bounded startup drains`, and systemd relaunched it after two seconds.
- **VERIFIED CURRENT BEHAVIOR:** In the sampled 16:45–17:50 window, the journal contained 1,357 FIFO startup failures and 1,357 bridge starts. The restart counter was already 10,872 at 16:45 in one boot and reached 1,160 after the following reboot before the broker stopped the bridge at 17:49:11.
- **VERIFIED CURRENT BEHAVIOR:** No kernel OOM-killer, `Out of memory`, killed-process, or page-allocation-failure record was found in the affected boot journals. The boot preceding the current one ended without an orderly shutdown record; that establishes an abrupt reset but not its cause.
- **VERIFIED CURRENT BEHAVIOR:** At inspection time the 905 MiB Pi had about 469 MiB available RAM and 214 MiB swap in use. The active Codex process was the largest resident process at about 217 MiB; P2_app was active and the bridge was not consuming memory.
- **TECHNICAL DEBT:** The evidence does not support calling the incident an OOM crash. The confirmed failure is XDMA FIFO startup plus an unbounded restart storm; CPU, I/O, swap pressure, or the hardware watchdog may have contributed to the abrupt reboot, but the surviving logs do not prove which one reset the Pi.
- **VERIFIED CURRENT BEHAVIOR:** A second reboot occurred at 2026-08-31 23:39:59 while a one-job full Saturn Go `cargo test` build was running. The prior boot again contains no OOM-killer record or orderly shutdown; watchdog/status jobs stretched from seconds to more than a minute immediately before logs stopped, while systemd had a one-minute hardware watchdog armed.
- **TECHNICAL DEBT:** The second reset is consistent with system starvation causing the hardware watchdog to fire, but no surviving record names the reset source. Full Saturn Go test builds should run off-device or under stronger resource controls; only the already-passing `cargo check -j1` result is claimed here.
- **VERIFIED CURRENT BEHAVIOR:** After that reboot the broker again reported selected/runtime P2, P2app active, Saturn Bridge inactive, idle transaction state, and mutual exclusion true. The worktree changes survived.
- **TECHNICAL DEBT:** “Radio Owner stopped” is misleading for the healthy P2-only state. There is no separate required long-running owner service in the current broker result; P2_app is the owner and the transaction state is idle.
- **PROPOSED CHANGE:** Preserve cross-boot crash evidence, rate-limit bridge restarts/logging, and expose selected owner, runtime owner, transaction state, last bridge fault, and restart count as separate Saturn Go fields.
