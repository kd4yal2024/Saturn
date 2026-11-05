//! Explicit shared startup/runtime; the legacy TX scheduler is never spawned.
//! RF/topology permission and timing limits remain explicit, unqualified inputs.
use crate::{
    config::{BridgeConfig, RadioBackend},
    p2::session::{P2Event, P2Session},
    radio_model::{RadioModel, TxPhase},
    sync_ext::MutexExt,
    tci::{TciCommand, TciFrontend},
    tx_audio::TxAudioIngress,
    tx_thread::production_owner::{self, Options, Radio},
    wdsp::WdspRxEngine,
    xdma_rx::OperationalRxSession,
    xdma_tx_radio::DirectXdmaTxRadio,
};
use saturn_bridge::puresignal::Fault;
use std::{
    error::Error,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

fn fault_error(fault: Fault) -> std::io::Error {
    std::io::Error::other(format!("shared owner: {fault:?}"))
}
enum Acquisition {
    P2(Arc<P2Session>),
    Direct(OperationalRxSession),
}
#[cfg(test)]
fn trace_step(label: &str, started: Instant) {
    let elapsed = started.elapsed();
    if elapsed > Duration::from_millis(5) {
        eprintln!(
            "runtime fixture slow step={label} us={}",
            elapsed.as_micros()
        );
    }
}
// Only the external I/O boundary is replaceable. Tests use this SAME startup,
// receive, stop/restoration and join orchestration, not a fixture owner loop.
trait RuntimeIo {
    fn open(
        &mut self,
        config: &mut BridgeConfig,
        backend: RadioBackend,
        model: &Arc<Mutex<RadioModel>>,
    ) -> Result<(Radio, Acquisition), Box<dyn Error>>;
    fn stop_requested(&self) -> bool;
    fn started(&mut self, _runtime: &production_owner::Runtime, _front: &Arc<TciFrontend>) {}
    fn before_receive(&mut self, _acquisition: &mut Acquisition) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn joined(&mut self, _runtime: &production_owner::Runtime) {}
    fn wait_for_input(&mut self, wait: Option<Duration>) {
        if let Some(wait) = wait {
            std::thread::sleep(wait);
        }
    }
}

pub(crate) fn acquisition_wait(requires_drain: bool) -> Option<Duration> {
    (!requires_drain).then_some(Duration::from_millis(1))
}
struct HardwareIo;
#[cfg(all(test, not(saturn_bridge_stub_native)))]
#[path = "shared_backend_tests.rs"]
mod runtime_tests;
impl RuntimeIo for HardwareIo {
    fn open(
        &mut self,
        config: &mut BridgeConfig,
        backend: RadioBackend,
        model: &Arc<Mutex<RadioModel>>,
    ) -> Result<(Radio, Acquisition), Box<dyn Error>> {
        match backend {
            RadioBackend::P2 => {
                let p2 = Arc::new(P2Session::bind_shared(config.clone(), model)?);
                p2.shared_receive_keepalive(&model.lock_unpoisoned())?;
                Ok((Radio::P2(p2.clone()), Acquisition::P2(p2)))
            }
            RadioBackend::Xdma => {
                let radio = Arc::new(DirectXdmaTxRadio::open_shared(
                    config.tx_power_meter_scale,
                    model.clone(),
                )?);
                config.remote_tx_rf_enabled &= radio.rf_tx_qualified();
                let desired = model.lock_unpoisoned().desired.clone();
                let rx = OperationalRxSession::open(
                    desired.iq_center_hz,
                    desired.rx_antenna,
                    desired.rx_attenuation_db,
                )?;
                Ok((Radio::Direct(radio), Acquisition::Direct(rx)))
            }
        }
    }
    fn stop_requested(&self) -> bool {
        crate::P2_STOP_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
    }
}
pub(crate) fn run(
    config: BridgeConfig,
    backend: RadioBackend,
    options: Options,
) -> Result<(), Box<dyn Error>> {
    let ready = std::env::var_os(if backend == RadioBackend::Xdma {
        "SATURN_BRIDGE_XDMA_READY_PATH"
    } else {
        "SATURN_BRIDGE_SHARED_READY_PATH"
    })
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|| {
        std::path::PathBuf::from(if backend == RadioBackend::Xdma {
            "/run/saturn-bridge/xdma-ready.json"
        } else {
            "/run/saturn-bridge/shared-ready.json"
        })
    });
    let perf = std::env::var_os("SATURN_BRIDGE_PERF_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/run/saturn-bridge/perf.json"));
    let label = if backend == RadioBackend::Xdma {
        "xdma"
    } else {
        "p2"
    };
    crate::xdma_telemetry::record_shared_review_status(&ready, label, "starting", None)?;
    crate::xdma_telemetry::record_shared_review_status(&perf, label, "starting", None)?;
    let _signals = crate::P2SignalGuard::install()?;
    let result = run_inner(config, backend, options, &ready, &perf, &mut HardwareIo);
    let phase = if result.is_ok() { "stopped" } else { "failed" };
    for path in [&ready, &perf] {
        let _ = crate::xdma_telemetry::record_shared_review_status(path, label, phase, None);
    }
    result
}
fn run_inner<I: RuntimeIo>(
    config: BridgeConfig,
    backend: RadioBackend,
    options: Options,
    ready: &std::path::Path,
    perf: &std::path::Path,
    io: &mut I,
) -> Result<(), Box<dyn Error>> {
    run_inner_with_state_store(
        config,
        backend,
        options,
        ready,
        perf,
        io,
        crate::rx_state::RxStateStore::from_env(),
    )
}

// The store is injectable so runtime tests never change process-wide settings
// or read/write an operator's configured receive-state directory.
fn run_inner_with_state_store<I: RuntimeIo>(
    mut config: BridgeConfig,
    backend: RadioBackend,
    options: Options,
    ready: &std::path::Path,
    perf: &std::path::Path,
    io: &mut I,
    rx_state_store: Option<crate::rx_state::RxStateStore>,
) -> Result<(), Box<dyn Error>> {
    if backend == RadioBackend::Xdma {
        config.rx_ddc_index = 6;
        config.ddc0_adc = 0;
        config.ddc0_sample_rate_khz = 384;
        config.max_client_ddc0_sample_rate_khz = 384;
        config.ddc0_sample_size_bits = 24;
    }
    if config.rx_ddc_index < 2 {
        return Err(std::io::Error::other("shared startup reserves DDC0/1 for feedback").into());
    }
    let model = Arc::new(Mutex::new(RadioModel::new(
        config.rx_ddc_index,
        config.ddc0_frequency_hz,
        config.ddc0_adc,
        config.ddc0_sample_rate_khz,
        config.ddc0_sample_size_bits,
        config.rx_fft_size,
        config.rx_low_latency,
        config.tx_fft_size,
        config.tx_low_latency,
    )));
    {
        let mut m = model.lock_unpoisoned();
        m.desired.running = true;
        m.desired.tx_enabled = false;
        m.desired.tx_phase = TxPhase::Rx;
        m.desired.tx_drive = m.desired.tx_drive.min(options.max_watts);
        m.desired.pure_signal_attenuation_db = options.feedback_attenuation_db;
    }
    // Restore only GoldMarsh's allow-listed receive fields, before acquisition,
    // DSP or client admission. Never restore TX authority or PureSignal state.
    let rx_state_baseline = rx_state_store.as_ref().and_then(|store| {
        store.restore(&mut model.lock_unpoisoned(), &|hz| match backend {
            RadioBackend::Xdma => crate::xdma_rx::validate_frequency(hz).is_ok(),
            RadioBackend::P2 => crate::rx_state::generic_frequency_ok(hz),
        })
    });
    let initial_receive = model.lock_unpoisoned().desired.clone();
    config.ddc0_frequency_hz = initial_receive.iq_center_hz;
    let (radio, mut acquisition) = io.open(&mut config, backend, &model)?;
    let acquisition_identity = match &acquisition {
        Acquisition::Direct(rx) => Some(rx.identity().clone()),
        Acquisition::P2(_) => None,
    };
    config.remote_tx_rf_enabled &= options.managed_rf_permitted;
    let (frontend, commands) = TciFrontend::reserve_shared_startup(&config, model.clone(), true)?;
    let frontend = Arc::new(frontend);
    // Reuse the Legacy observer: acquisition only copies into its lock-free
    // latest-IQ ring; FFT work stays outside the acquisition/TX owner loops.
    let spectrum_ring = Arc::new(crate::display_spectrum::LatestIqRing::new(u64::from(
        initial_receive.iq_center_hz,
    )));
    let _spectrum_worker = crate::display_spectrum::SpectrumWorker::spawn(
        frontend.clone(),
        spectrum_ring.clone(),
        u32::from(config.ddc0_sample_rate_khz) * 1000,
    )?;
    let _correction_worker = crate::correction_monitor::Worker::optional(
        frontend.clone(),
        match backend {
            RadioBackend::P2 => "P2",
            RadioBackend::Xdma => "DirectXdma",
        },
    )?;
    if _correction_worker.is_some() {
        frontend.set_correction_monitor_available();
    }
    let monitor_input = (std::env::var("SATURN_PS_LIVE_MONITOR").as_deref() == Ok("1"))
        .then(crate::rf_monitor::Input::new);
    let _rf_monitor = monitor_input
        .as_ref()
        .map(|input| crate::rf_monitor::Worker::spawn(frontend.clone(), input.clone()))
        .transpose()?;
    let mut rx_dsp = WdspRxEngine::new(&model.lock_unpoisoned())?;
    let mut runtime = production_owner::spawn(
        radio.clone(),
        frontend.clone(),
        model.clone(),
        config.tx_audio_source,
        options,
        config.remote_tx_rf_enabled,
        config.high_priority_period,
    )
    .map_err(fault_error)?;
    if let Some(input) = monitor_input {
        let _ = runtime.ingress.rf_monitor.set(input);
        frontend.set_rf_monitor_available();
    }
    // RAII flush also covers startup failures and acquisition-error shutdown.
    let _rx_state_saver = rx_state_store
        .map(|store| crate::rx_state::RxStateSaver::spawn(store, model.clone(), rx_state_baseline));
    // After spawn, every failure must reach the cooperative stop loop. Joining
    // before acquisition restoration would deadlock successful cleanup.
    let attached = match &mut acquisition {
        Acquisition::P2(p2) => p2.attach_managed_feedback(
            runtime.link.clone(),
            runtime.ingress.clone(),
            model.clone(),
            runtime.origin,
        ),
        Acquisition::Direct(rx) => rx
            .attach_managed_feedback(
                runtime.link.clone(),
                runtime.ingress.clone(),
                runtime.origin,
            )
            .map_err(std::io::Error::other),
    };
    // No parallel legacy audio or command consumer. SATP gets the same ingress.
    let (unused_legacy_commands, _unused_receiver) = mpsc::channel();
    let satp = attached.and_then(|()| {
        crate::satp::SatpRuntime::start(
            &config,
            model.clone(),
            TxAudioIngress::shared(runtime.ingress.clone()),
            unused_legacy_commands,
        )
        .map_err(std::io::Error::other)
    });
    let (_satp, mut terminal_error): (_, Option<Box<dyn Error>>) = match satp {
        Ok(satp) => (Some(satp), None),
        Err(error) => (None, Some(error.into())),
    };
    let mut admitted = false;
    let mut stopping = terminal_error.is_some();
    if stopping {
        runtime.request_stop();
    }
    io.started(&runtime, &frontend);
    let mut last_state = Instant::now();
    let mut published_desired = None;
    let mut last_keepalive = Instant::now();
    let mut last_health = Instant::now();
    let mut last_status = Instant::now();
    let mut status_rx_frames = 0u64;
    let mut iq = Vec::with_capacity(8192);
    let mut display_iq_packetizer = Some(crate::xdma_backend::DirectIqPacketizer::for_rate(
        config.ddc0_sample_rate_khz as u32 * 1000,
    ));
    let mut received_frames = 0u64;
    let mut received_pairs = 0u64;
    let mut spectrum_scope = None;
    let mut direct_applied = (
        initial_receive.iq_center_hz,
        initial_receive.rx_antenna,
        initial_receive.rx_attenuation_db,
    );
    loop {
        frontend.service_hardware_ptt(Instant::now());
        if io.stop_requested() && !stopping {
            runtime.request_stop();
            stopping = true;
        }
        if !admitted && runtime.ingress.startup_ready() && !stopping {
            match frontend.admit_shared_clients() {
                Ok(()) => admitted = true,
                Err(error) => {
                    terminal_error = Some(error.into());
                    runtime.request_stop();
                    stopping = true;
                }
            }
        }
        for _ in 0..16 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            receive_control(command, &frontend, &model, &runtime.ingress, &mut rx_dsp);
        }
        let snapshot = model.lock_unpoisoned().clone();
        if let Some(input) = runtime.ingress.rf_monitor.get() {
            input.configure(crate::rf_monitor::Settings::model(&snapshot));
        }
        let transmitting = snapshot.desired.tx_phase != TxPhase::Rx;
        let spectrum_feed_active = !transmitting && frontend.media_demand().spectrum_stream_enabled;
        let scope = (spectrum_feed_active, snapshot.desired.iq_center_hz);
        if spectrum_scope != Some(scope) {
            spectrum_ring.restart(u64::from(scope.1));
            spectrum_scope = Some(scope);
        }
        if transmitting || !frontend.media_demand().raw_iq_stream_enabled {
            if let Some(packetizer) = &mut display_iq_packetizer {
                packetizer.reset();
            }
        }
        frontend.set_tx_media_priority_active(transmitting);
        #[cfg(test)]
        let rx_transition_started = Instant::now();
        if transmitting {
            rx_dsp.suspend_for_tx();
        } else {
            rx_dsp.resume_from_tx();
        }
        #[cfg(test)]
        trace_step("rx-suspend-resume", rx_transition_started);
        let mut requires_drain = false;
        let frames_before_receive = received_frames;
        #[cfg(test)]
        let receive_started = Instant::now();
        let receive = (|| -> Result<(), Box<dyn Error>> {
            if !io.before_receive(&mut acquisition)? {
                return Ok(());
            }
            rx_dsp.sync_model(&snapshot)?;
            match &mut acquisition {
                Acquisition::P2(p2) => {
                    if last_keepalive.elapsed() >= config.high_priority_period && !transmitting {
                        p2.shared_receive_keepalive(&snapshot)?;
                        last_keepalive = Instant::now();
                    }
                    let received = p2.recv_outcome()?;
                    // At 192 kHz, 119-pair feedback packets arrive faster
                    // than 1 kHz too. A consumed event is work, not an idle
                    // turn. The next receive retains the existing bounded
                    // socket wait and services restoration before reading.
                    requires_drain = received.consumed;
                    match received.event {
                        Some(P2Event::DdcIq(frame))
                            if frame.ddc_index == snapshot.desired.rx_ddc_index =>
                        {
                            received_frames = received_frames.saturating_add(1);
                            received_pairs =
                                received_pairs.saturating_add((frame.iq_samples.len() / 2) as u64);
                            publish_receive(
                                &frontend,
                                &mut rx_dsp,
                                &frame.iq_samples,
                                config.ddc0_sample_rate_khz as u32 * 1000,
                                &snapshot,
                                transmitting,
                                display_iq_packetizer.as_mut(),
                                spectrum_feed_active.then_some(spectrum_ring.as_ref()),
                            );
                            model.lock_unpoisoned().apply_ddc_frame(frame);
                        }
                        Some(P2Event::HighPriorityFromSdr(packet)) => {
                            frontend.observe_hardware_ptt(packet.ptt_bits & 1 != 0, Instant::now());
                            if transmitting
                                && (packet.adc_overflows & 1 != 0
                                    || crate::saturn_adc_to_watts(
                                        packet.forward_power,
                                        32,
                                        config.tx_power_meter_scale,
                                    ) > crate::remote_tx_power_trip_watts())
                            {
                                runtime.ingress.stop(Fault::SevereOverdrive);
                            }
                            model.lock_unpoisoned().apply_high_priority(packet);
                        }
                        _ => {}
                    }
                }
                Acquisition::Direct(rx) => {
                    let now = Instant::now();
                    if frontend.hardware_ptt_poll_due(now) {
                        frontend.observe_hardware_ptt(rx.read_ptt_input()?, now);
                    }
                    // Tuning/relay/attenuation application waits until the sole
                    // lifecycle is clean/off, never overwriting a keyed route.
                    if runtime.ingress.startup_ready() || runtime.link.preparing() {
                        let requested = (
                            snapshot.desired.iq_center_hz,
                            snapshot.desired.rx_antenna,
                            snapshot.desired.rx_attenuation_db,
                        );
                        if direct_applied != requested {
                            if let Some(packetizer) = &mut display_iq_packetizer {
                                packetizer.reset();
                            }
                        }
                        if direct_applied.0 != requested.0 {
                            rx.tune(requested.0)?;
                        }
                        if direct_applied.1 != requested.1 {
                            rx.set_rx_antenna(requested.1)?;
                        }
                        if direct_applied.2 != requested.2 {
                            rx.set_rx_attenuation(requested.2)?;
                        }
                        direct_applied = requested;
                    }
                    let outcome = rx.read_iq(&mut iq)?;
                    requires_drain = outcome.requires_drain;
                    if outcome.samples_ready {
                        received_frames = received_frames.saturating_add(1);
                        received_pairs = received_pairs.saturating_add((iq.len() / 2) as u64);
                        publish_receive(
                            &frontend,
                            &mut rx_dsp,
                            &iq,
                            384000,
                            &snapshot,
                            transmitting,
                            display_iq_packetizer.as_mut(),
                            spectrum_feed_active.then_some(spectrum_ring.as_ref()),
                        );
                    }
                    if last_health.elapsed() >= Duration::from_secs(1) {
                        // Existing owner's configured poll/arm policy is intact.
                        let _ = rx.sample_per_ddc_almost_full_telemetry();
                        rx.sample_rx_counter_v31();
                        let _ = rx.sample_extended_telemetry();
                        last_health = Instant::now();
                    }
                    if let Radio::Direct(radio) = &radio {
                        let power = radio.snapshot();
                        if power.keyed && power.forward_watts > crate::remote_tx_power_trip_watts()
                        {
                            runtime.ingress.stop(Fault::SevereOverdrive);
                        }
                    }
                }
            }
            Ok(())
        })();
        #[cfg(test)]
        trace_step("receive", receive_started);
        if let Err(error) = receive {
            if terminal_error.is_none() {
                terminal_error = Some(error);
            }
            runtime.request_stop();
            stopping = true;
        }
        if received_frames != frames_before_receive && !transmitting {
            // The shared path owns RX DSP too. Copy its measurements back to
            // the observed model, just as the legacy RX owners do, so periodic
            // TCI telemetry includes the S-meter rather than only IQ/audio.
            let mut current = model.lock_unpoisoned();
            current.observed.ddc0_meter_dbm = rx_dsp.smeter_dbm().map(|raw| {
                crate::rx_thread::correct_smeter_dbm(
                    raw,
                    snapshot.desired.rx_attenuation_db,
                    config.smeter_calibration_db,
                )
            });
            current.observed.rx_wbfm_stereo_detected = rx_dsp.wbfm_stereo_detected();
        }
        if last_state.elapsed() >= Duration::from_millis(200) {
            #[cfg(test)]
            let started = Instant::now();
            // Full settings are change notifications, not a meter heartbeat.
            // Repeating the entire configuration while idle starves small
            // browsers with hundreds of settings messages per second.
            if published_desired.as_ref() != Some(&snapshot.desired) {
                frontend.publish_radio_state(&snapshot);
                published_desired = Some(snapshot.desired.clone());
            } else {
                // Late clients still need the small observer capability
                // reports, without replaying every radio setting.
                frontend.publish_observer_capabilities();
                frontend.publish_telemetry(&snapshot);
            }
            #[cfg(test)]
            trace_step("publish-state", started);
            last_state = Instant::now();
        }
        if last_status.elapsed() >= Duration::from_secs(1) {
            #[cfg(test)]
            let started = Instant::now();
            let counter = match &acquisition {
                Acquisition::Direct(rx) => Some(rx.rx_counter_v31_json()),
                Acquisition::P2(_) => None,
            };
            let label = if backend == RadioBackend::Xdma {
                "xdma"
            } else {
                "p2"
            };
            let clients = frontend.client_snapshot();
            let ddc_per_second =
                (received_frames - status_rx_frames) as f64 / last_status.elapsed().as_secs_f64();
            for path in [ready, perf] {
                let (phase, clean_idle) = runtime.ingress.host_readiness();
                let observation = crate::xdma_telemetry::SharedHostObservation {
                    model: &snapshot,
                    clients: &clients,
                    ddc_per_second,
                    identity: acquisition_identity.as_ref(),
                };
                if let Err(error) = crate::xdma_telemetry::record_shared_observed_host_status(
                    path,
                    label,
                    phase,
                    counter.clone(),
                    clean_idle && !stopping,
                    received_frames,
                    received_pairs,
                    Some(&observation),
                ) {
                    if terminal_error.is_none() {
                        terminal_error = Some(error.into());
                    }
                    runtime.request_stop();
                    stopping = true;
                }
            }
            status_rx_frames = received_frames;
            last_status = Instant::now();
            #[cfg(test)]
            trace_step("write-status", started);
        }
        match runtime.done.try_recv() {
            Ok(result) => {
                if let Err(fault) = result {
                    if terminal_error.is_none() {
                        terminal_error = Some(fault_error(fault).into());
                    }
                }
                break;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                terminal_error =
                    Some(std::io::Error::other("shared owner exited without completion").into());
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        // One DMA buffer per turn, then all control/stop/restoration services
        // above run again. Do not add an idle delay while completed buffers
        // remain: small DMA reads can arrive faster than 1 kHz. Sleeping after
        // every buffer accumulated old feedback despite an available reader.
        // This neither retimes samples nor changes freshness/polling limits.
        io.wait_for_input(acquisition_wait(requires_drain));
    }
    if let Some(worker) = runtime.worker.take() {
        worker
            .join()
            .map_err(|_| std::io::Error::other("shared owner panicked"))?;
    }
    io.joined(&runtime);
    if let Acquisition::P2(p2) = &acquisition {
        p2.shared_stop_run(&model.lock_unpoisoned())?;
    }
    if let Some(error) = terminal_error {
        return Err(error);
    }
    Ok(())
}
fn publish_receive(
    front: &TciFrontend,
    dsp: &mut WdspRxEngine,
    iq: &[f32],
    rate: u32,
    model: &RadioModel,
    transmitting: bool,
    display_packetizer: Option<&mut crate::xdma_backend::DirectIqPacketizer>,
    spectrum_ring: Option<&crate::display_spectrum::LatestIqRing>,
) {
    if let Some(ring) = spectrum_ring {
        ring.push(iq);
    }
    if let Some(packetizer) = display_packetizer {
        // Same full-stream batching as Legacy Direct XDMA. Sending each tiny
        // DMA completion through the snapshot limiter discards most samples
        // and starves the browser FFT/3D display despite a nominal 384 kHz tag.
        if !transmitting && front.media_demand().raw_iq_stream_enabled {
            packetizer.push(iq, |frame| {
                front.publish_full_rate_iq_frame(rate, frame);
            });
        } else {
            packetizer.reset();
        }
    } else {
        front.publish_iq_frame(rate, iq);
    }
    if !transmitting {
        let audio_rate = dsp.audio_sample_rate_hz();
        dsp.process_iq(iq, |samples| {
            front.publish_audio_frame(audio_rate, samples, model.desired.rx_volume_db)
        });
    }
}
fn receive_control(
    command: TciCommand,
    front: &TciFrontend,
    model: &Arc<Mutex<RadioModel>>,
    ingress: &crate::tx_thread::ps_ingress::SharedIngress,
    dsp: &mut WdspRxEngine,
) {
    use TciCommand::*;
    let mut m = model.lock_unpoisoned();
    match command {
        RequestRadioState { client_id } => front.publish_standard_radio_state_to(client_id, &m),
        SaturnPing {
            client_id,
            nonce,
            sent_at,
        } => front.publish_saturn_pong(client_id, &nonce, &sent_at),
        SetIqStreaming
        | RequestSmeter
        | ClientConnected
        | SplitSessionOpen { .. }
        | SplitSessionLane { .. } => {}
        SetAudioStreaming(enabled) => {
            dsp.reset_audio_packetizer();
            if enabled {
                front.publish_audio_started(48000);
            } else {
                front.publish_audio_stopped();
            }
        }
        SetAudioSampleRate(_) | SetAudioChannels(_) => {
            dsp.reset_audio_packetizer();
            front.publish_audio_started(48000);
        }
        SetAudioFrameSamples(samples) => {
            dsp.set_audio_frame_float_count(samples as usize);
        }
        SetRxVolume(value) => m.desired.rx_volume_db = value.clamp(-40., 12.),
        SetTxMonitor(enabled) => {
            m.desired.tx_monitor_enabled = enabled && m.desired.tx_monitor_available;
            front.publish_radio_state(&m);
        }
        SetRxHeadphones(enabled) => {
            m.desired.rx_headphones_enabled = enabled && m.desired.rx_headphones_available;
            front.publish_radio_state(&m);
        }
        SetTxMonitorLevel(level) => {
            m.desired.tx_monitor_level_db = crate::tx_monitor::clamp_level(level);
            front.publish_radio_state(&m);
        }
        SetFilterBand { low_hz, high_hz } => {
            m.desired.filter_low_hz = low_hz;
            m.desired.filter_high_hz = high_hz;
        }
        SetRxSsqlEnabled(value) => m.desired.rx_ssql_enabled = value,
        SetRxSsqlThreshold(value) => m.desired.rx_ssql_threshold = value.clamp(0., 100.),
        SetRxNoiseReductionMode(value) => m.desired.rx_noise_reduction_mode = value,
        SetRxNoiseReductionEnabled(enabled) => {
            m.desired.rx_noise_reduction_mode = if enabled {
                crate::radio_model::NoiseReductionMode::Nr1
            } else {
                crate::radio_model::NoiseReductionMode::Off
            }
        }
        SetRxNoiseReductionLevel(value) => {
            m.desired.rx_noise_reduction_level = value.clamp(0., 100.)
        }
        SetRxNr2GainMethod(value) => m.desired.rx_nr2_gain_method = value,
        SetRxNr2NpeMethod(value) => m.desired.rx_nr2_npe_method = value,
        SetRxNr2PostFilterEnabled(value) => m.desired.rx_nr2_post_filter_enabled = value,
        SetRxWbfmDeemphasis(value) => m.desired.rx_wbfm_deemphasis = value,
        SetAgcMode(value) => m.desired.agc_mode = value,
        SetAgcGain(value) => m.desired.agc_gain = value.clamp(0., 100.),
        SetNoiseBlankerMode(value) => m.desired.nb_mode = value,
        SetNoiseBlankerThreshold(value) => m.desired.nb_threshold = value.clamp(0., 100.),
        SetAnfEnabled(value) => m.desired.anf_enabled = value,
        SetRxEqEnabled(value) => m.desired.rx_eq_enabled = value,
        SetRxEqBand { band, gain_db } if band < m.desired.rx_eq_bands.len() => {
            m.desired.rx_eq_bands[band] = gain_db.clamp(-20, 20)
        }
        SetRxFftSize(size) => {
            let value = size.clamp(1024, 262144);
            m.desired.rx_fft_size = 1 << (31 - value.leading_zeros());
        }
        SetRxLowLatency(value) => m.desired.rx_low_latency = value,
        SetIqSampleRate(value) if value == m.desired.ddc0_sample_rate_khz as u32 * 1000 => {}
        SetIqCenter(value) if value == m.desired.iq_center_hz => {}
        SetRxAdc(value) if value == m.desired.ddc0_adc => {}
        SetRxAntenna(value) if value == m.desired.rx_antenna => {}
        SetRxAttenuation(value) if value == m.desired.rx_attenuation_db => {}
        SetRxAntenna(_) | SetRxAttenuation(_) | SetRxAdc(_) | SetIqCenter(_)
        | SetIqSampleRate(_) => {
            // No untagged route mutation while a correction generation exists.
            ingress.stop(Fault::WrongRoute);
            front.publish_shared_control_unavailable("fixed-feedback-geometry");
        }
        _ => {
            front.publish_shared_control_unavailable("unsupported-control");
            front.publish_radio_state(&m);
        }
    }
}
