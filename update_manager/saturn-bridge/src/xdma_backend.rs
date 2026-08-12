//! Operational direct-XDMA radio backend.
//!
//! P2app remains the default service/backend. When the transactional backend
//! switch grants this process exclusive hardware ownership, this runtime owns
//! the proven DDC receive path plus the shared WDSP TX pipeline and production
//! H2C/DUC output.

use crate::config::BridgeConfig;
use crate::radio_model::{DemodMode, NoiseReductionMode, PureSignalState, RadioModel, TxPhase};
use crate::sync_ext::MutexExt;
use crate::tci::{TciCommand, TciFrontend};
use crate::tx_thread::{self, TxCommand, TxEvent};
use crate::wdsp::{normalize_audio_frame_float_count, WdspRxEngine, WDSP_AUDIO_RATE_HZ};
use crate::xdma::{SaturnIdentity, XdmaError};
use crate::xdma_rx::{OperationalRxSession, DIRECT_DDC_INDEX, DIRECT_DDC_SAMPLE_RATE_KHZ};
use crate::xdma_telemetry::{record_runtime_readiness, TelemetryValue};
use crate::xdma_tx_radio::{DirectTxSnapshot, DirectXdmaTxRadio};
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_READY_PATH: &str = "/run/saturn-bridge/xdma-ready.json";
const READY_DMA_READS: u64 = 4;
const READY_IQ_PAIRS: u64 = 1_024;
const MAX_COMMANDS_PER_LOOP: usize = 8;
const IDLE_POLL: Duration = Duration::from_micros(250);
const READINESS_PERIOD: Duration = Duration::from_secs(1);
const STATUS_PERIOD: Duration = Duration::from_secs(5);
const DIRECT_TX_MAX_WATTS: u8 = 3;
const TX_UPLINK_TIMEOUT: Duration = Duration::from_millis(750);
const TX_CONTROL_TIMEOUT: Duration = Duration::from_millis(1_500);

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
struct CommandEffects {
    dsp_dirty: bool,
    tuning_dirty: bool,
    tx_state_dirty: bool,
}

#[derive(Debug, Default)]
struct DirectTxControl {
    requested: bool,
    last_mic_at: Option<Instant>,
}

impl CommandEffects {
    fn merge(&mut self, other: Self) {
        self.dsp_dirty |= other.dsp_dirty;
        self.tuning_dirty |= other.tuning_dirty;
        self.tx_state_dirty |= other.tx_state_dirty;
    }
}

struct SignalGuard {
    previous_int: libc::sigaction,
    previous_term: libc::sigaction,
}

impl SignalGuard {
    fn install() -> Result<Self, XdmaError> {
        STOP_REQUESTED.store(false, Ordering::SeqCst);
        // SAFETY: the handler performs only an async-signal-safe atomic store,
        // and the exact previous dispositions are restored on normal exit.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = stop_signal as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = 0;
            let mut previous_int: libc::sigaction = std::mem::zeroed();
            let mut previous_term: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut previous_int) != 0 {
                return Err(XdmaError::Io {
                    action: "could not install operational XDMA SIGINT handler",
                    source: std::io::Error::last_os_error(),
                });
            }
            if libc::sigaction(libc::SIGTERM, &action, &mut previous_term) != 0 {
                libc::sigaction(libc::SIGINT, &previous_int, std::ptr::null_mut());
                return Err(XdmaError::Io {
                    action: "could not install operational XDMA SIGTERM handler",
                    source: std::io::Error::last_os_error(),
                });
            }
            Ok(Self {
                previous_int,
                previous_term,
            })
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        // SAFETY: these values were returned by sigaction for these signals.
        unsafe {
            libc::sigaction(libc::SIGINT, &self.previous_int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.previous_term, std::ptr::null_mut());
        }
    }
}

extern "C" fn stop_signal(_signal: libc::c_int) {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

pub(crate) fn run(config: BridgeConfig) -> Result<(), Box<dyn Error>> {
    let ready_path = env::var_os("SATURN_BRIDGE_XDMA_READY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_READY_PATH));
    let result = run_inner(config, &ready_path);
    if let Err(error) = &result {
        let _ = record_runtime_readiness(&ready_path, "failed", Some(&error.to_string()), &[]);
    }
    result
}

fn run_inner(config: BridgeConfig, ready_path: &Path) -> Result<(), Box<dyn Error>> {
    let _signal_guard = SignalGuard::install()?;
    let radio_model = Arc::new(Mutex::new(RadioModel::new(
        DIRECT_DDC_INDEX as u8,
        config.ddc0_frequency_hz,
        0,
        DIRECT_DDC_SAMPLE_RATE_KHZ as u16,
        24,
        config.rx_fft_size,
        config.rx_low_latency,
        config.tx_fft_size,
        config.tx_low_latency,
    )));
    {
        let mut model = radio_model.lock_unpoisoned();
        model.desired.running = true;
        model.desired.tx_enabled = false;
        model.desired.tx_phase = TxPhase::Rx;
        model.desired.tx_drive = model.desired.tx_drive.min(DIRECT_TX_MAX_WATTS);
        model.desired.pure_signal_enabled = false;
        model.observed.pure_signal_state = PureSignalState::Off;
    }
    let (tci, command_rx) = TciFrontend::bind(&config, radio_model.clone())?;
    let tci = Arc::new(tci);
    let tx_radio = Arc::new(DirectXdmaTxRadio::open(config.tx_power_meter_scale)?);
    let (tx_cmd_tx, tx_cmd_rx) = mpsc::channel();
    let (tx_event_tx, tx_event_rx) = mpsc::channel();
    let tx_stop = Arc::new(AtomicBool::new(false));
    let tx_worker = tx_thread::spawn(
        tx_radio.clone(),
        radio_model.clone(),
        tx_cmd_rx,
        tx_event_tx,
        tx_stop.clone(),
    );
    let mut tx_control = DirectTxControl::default();
    let mut wdsp = {
        let model = radio_model.lock_unpoisoned();
        WdspRxEngine::new(&model)?
    };
    let mut rx = OperationalRxSession::open(config.ddc0_frequency_hz)?;
    let identity = rx.identity().clone();
    let mut iq_samples = Vec::with_capacity(8_192);
    rx.drain_startup_fifo(&mut iq_samples)?;
    let mut readiness_state = "starting";
    let mut last_readiness = Instant::now() - READINESS_PERIOD;
    let mut last_status = Instant::now();

    write_readiness(
        ready_path,
        readiness_state,
        &identity,
        &rx,
        tx_radio.snapshot(),
        config.remote_tx_rf_enabled,
    )?;
    println!(
        "saturn-bridge: direct XDMA backend starting product={} pcb={} firmware={}.{} ddc={} adc=ADC1 frequency={}Hz rate={}kHz TCI={} TX={} max={}W PureSignal=disabled",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        DIRECT_DDC_INDEX,
        rx.frequency_hz(),
        DIRECT_DDC_SAMPLE_RATE_KHZ,
        config.tci_bind_addr,
        if config.remote_tx_rf_enabled { "RF-enabled" } else { "RF-inhibited" },
        DIRECT_TX_MAX_WATTS,
    );
    // Readiness persistence can occasionally stall on an appliance SD card
    // long enough for the continuously advancing DDC FIFO to reach its startup
    // threshold. Drain once more immediately before entering the runtime loop,
    // where actual overflow and underflow conditions remain fail-fast.
    rx.drain_startup_fifo(&mut iq_samples)?;

    let runtime_result = (|| -> Result<(), Box<dyn Error>> {
        while !STOP_REQUESTED.load(Ordering::Relaxed) {
            let mut did_work = false;
            // Service the continuously advancing hardware FIFO before bounded
            // client control work. A browser can submit dozens of preferences in
            // one burst; letting that burst run first can starve DDC long enough
            // to cross the FPGA FIFO threshold.
            if rx.read_iq(&mut iq_samples)? {
                did_work = true;
                tci.publish_iq_frame(DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000, &iq_samples);
                for audio in wdsp.push_iq(&iq_samples) {
                    tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio);
                }
                let mut model = radio_model.lock_unpoisoned();
                model.observed.ddc0_packets = rx.stats().dma_reads;
                model.observed.ddc0_meter_dbm = wdsp.smeter_dbm();
                model.observed.rx_wbfm_stereo_detected = wdsp.wbfm_stereo_detected();
            }

            let command_started = Instant::now();
            let mut command_count = 0;
            let mut command_effects = CommandEffects::default();
            for _ in 0..MAX_COMMANDS_PER_LOOP {
                let Ok(command) = command_rx.try_recv() else {
                    break;
                };
                command_count += 1;
                did_work = true;
                command_effects.merge(handle_command(
                    command,
                    &radio_model,
                    &tci,
                    &mut wdsp,
                    &mut rx,
                    &tx_cmd_tx,
                    &mut tx_control,
                    config.remote_tx_rf_enabled,
                )?);
            }
            if command_count != 0 {
                let model = radio_model.lock_unpoisoned();
                if command_effects.dsp_dirty {
                    wdsp.sync_model(&model)?;
                }
                if command_effects.tuning_dirty {
                    tci.publish_tuning_state(&model);
                }
                if command_effects.tx_state_dirty {
                    tci.publish_tx_state(&model);
                }
                tci.publish_radio_state(&model);
                let command_elapsed = command_started.elapsed();
                if command_elapsed >= Duration::from_millis(5) {
                    println!(
                    "saturn-bridge: xdma_rx control batch commands={} dsp_sync={} tuning_publish={} tx_publish={} elapsed_us={}",
                    command_count,
                    u8::from(command_effects.dsp_dirty),
                    u8::from(command_effects.tuning_dirty),
                    u8::from(command_effects.tx_state_dirty),
                    command_elapsed.as_micros(),
                );
                }
            }

            while let Ok(event) = tx_event_rx.try_recv() {
                did_work = true;
                match event {
                    TxEvent::Keyed => {
                        let mut model = radio_model.lock_unpoisoned();
                        model.desired.tx_phase = TxPhase::Keyed;
                        tci.publish_radio_state(&model);
                    }
                    TxEvent::Unkeyed => {
                        tx_control.requested = false;
                        tx_control.last_mic_at = None;
                        tci.mark_split_released(Instant::now());
                        let mut model = radio_model.lock_unpoisoned();
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                        tci.publish_radio_state(&model);
                    }
                    TxEvent::TxIqFrame {
                        sample_rate_hz,
                        iq_samples,
                    } => tci.publish_tx_iq_frame(sample_rate_hz, &iq_samples),
                    TxEvent::Diagnostics(_diagnostics) => {}
                    TxEvent::PureSignalStatus(_status) => {}
                }
            }

            if tx_control.requested {
                let now = Instant::now();
                let mic_stale = tx_control
                    .last_mic_at
                    .is_some_and(|last| now.saturating_duration_since(last) > TX_UPLINK_TIMEOUT);
                let control_stale = tci
                    .last_operator_control_at()
                    .is_some_and(|last| now.saturating_duration_since(last) > TX_CONTROL_TIMEOUT);
                if mic_stale || control_stale {
                    eprintln!(
                    "saturn-bridge: direct XDMA TX watchdog forced RX mic_stale={} control_stale={}",
                    mic_stale, control_stale
                );
                    tx_control.requested = false;
                    tx_control.last_mic_at = None;
                    tci.mark_split_released(now);
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    let mut model = radio_model.lock_unpoisoned();
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    tci.publish_radio_state(&model);
                }
            }
            {
                let model = radio_model.lock_unpoisoned();
                tci.set_tx_media_priority_active(
                    tx_control.requested || model.desired.tx_phase != TxPhase::Rx,
                );
            }

            if readiness_state == "starting"
                && rx.stats().dma_reads >= READY_DMA_READS
                && rx.stats().samples >= READY_IQ_PAIRS
            {
                readiness_state = "ready";
                write_readiness(
                    ready_path,
                    readiness_state,
                    &identity,
                    &rx,
                    tx_radio.snapshot(),
                    config.remote_tx_rf_enabled,
                )?;
                println!(
                "saturn-bridge: direct XDMA RX backend ready dma_reads={} iq_pairs={} rf_safe=1",
                rx.stats().dma_reads,
                rx.stats().samples
            );
            }
            if last_readiness.elapsed() >= READINESS_PERIOD {
                write_readiness(
                    ready_path,
                    readiness_state,
                    &identity,
                    &rx,
                    tx_radio.snapshot(),
                    config.remote_tx_rf_enabled,
                )?;
                last_readiness = Instant::now();
            }
            if last_status.elapsed() >= STATUS_PERIOD {
                let stats = rx.stats();
                let tx = tx_radio.snapshot();
                println!(
                "saturn-bridge: xdma status={} frequency_hz={} dma_reads={} dma_bytes={} iq_pairs={} rx_fifo_hwm={} header_resync={} header_errors={} rx_fifo_thresholds={} rx_fifo_faults={} tx_requested={} tx_stream={} tx_keyed={} tx_dma_writes={} tx_frames={} tx_fifo_lwm={} tx_fifo_hwm={} tx_fifo_faults={} forward_w={:.3} reverse_w={:.3} swr={:.2}",
                readiness_state,
                rx.frequency_hz(),
                stats.dma_reads,
                stats.dma_bytes,
                stats.samples,
                stats.fifo_depth_hwm,
                stats.header_resyncs,
                stats.header_errors,
                stats.fifo_over_threshold + stats.fifo_startup_over_threshold,
                stats.fifo_overflows + stats.fifo_underflows,
                u8::from(tx_control.requested),
                u8::from(tx.stream_active),
                u8::from(tx.keyed),
                tx.dma_writes,
                tx.frames_written,
                tx.fifo_lwm,
                tx.fifo_hwm,
                tx.fifo_faults,
                tx.forward_watts,
                tx.reverse_watts,
                tx.swr,
            );
                last_status = Instant::now();
            }
            if !did_work {
                thread::sleep(IDLE_POLL);
            }
        }
        Ok(())
    })();

    let _ = tx_cmd_tx.send(TxCommand::Disarm);
    let _ = tx_cmd_tx.send(TxCommand::Shutdown);
    tx_stop.store(true, Ordering::Relaxed);
    let tx_join = tx_worker.join();
    let rx_stop = rx.stop();
    runtime_result?;
    tx_join.map_err(|_| "direct XDMA TX thread panicked")?;
    rx_stop?;
    record_runtime_readiness(
        ready_path,
        "stopped",
        None,
        &[
            ("dma_reads", TelemetryValue::number(rx.stats().dma_reads)),
            ("iq_pairs", TelemetryValue::number(rx.stats().samples)),
            ("rf_safe", TelemetryValue::boolean(true)),
        ],
    )?;
    println!(
        "saturn-bridge: direct XDMA backend stopped; DDC and DUC disabled and receive-safe cleanup verified"
    );
    Ok(())
}

fn handle_command(
    command: TciCommand,
    radio_model: &Arc<Mutex<RadioModel>>,
    tci: &TciFrontend,
    wdsp: &mut WdspRxEngine,
    rx: &mut OperationalRxSession,
    tx_cmd_tx: &mpsc::Sender<TxCommand>,
    tx_control: &mut DirectTxControl,
    remote_tx_rf_enabled: bool,
) -> Result<CommandEffects, Box<dyn Error>> {
    let effects = CommandEffects {
        dsp_dirty: matches!(
            &command,
            TciCommand::SetMode(_)
                | TciCommand::SetFilterBand { .. }
                | TciCommand::SetRxVolume(_)
                | TciCommand::SetRxNoiseReductionMode(_)
                | TciCommand::SetRxNoiseReductionEnabled(_)
                | TciCommand::SetRxNoiseReductionLevel(_)
                | TciCommand::SetRxNr2GainMethod(_)
                | TciCommand::SetRxNr2NpeMethod(_)
                | TciCommand::SetRxNr2PostFilterEnabled(_)
                | TciCommand::SetRxWbfmDeemphasis(_)
                | TciCommand::SetRxAnrVals { .. }
                | TciCommand::SetNoiseBlankerMode(_)
                | TciCommand::SetNoiseBlankerThreshold(_)
                | TciCommand::SetAnfEnabled(_)
                | TciCommand::SetRxAnfVals { .. }
                | TciCommand::SetAgcMode(_)
                | TciCommand::SetAgcGain(_)
                | TciCommand::SetRxEqEnabled(_)
                | TciCommand::SetRxEqBand { .. }
                | TciCommand::SetRxFftSize(_)
                | TciCommand::SetRxLowLatency(_)
        ),
        tuning_dirty: matches!(
            &command,
            TciCommand::SetVfoA(_) | TciCommand::SetVfoB(_) | TciCommand::SetIqCenter(_)
        ),
        tx_state_dirty: matches!(
            &command,
            TciCommand::SetTxEnabled(_) | TciCommand::ClientDisconnected
        ),
    };
    let mut model = radio_model.lock_unpoisoned();
    match command {
        TciCommand::SetVfoA(frequency_hz) => {
            rx.tune(frequency_hz)?;
            model.desired.vfo_a_hz = frequency_hz;
            model.desired.iq_center_hz = frequency_hz;
            model.desired.tx_frequency_hz = frequency_hz;
        }
        TciCommand::SetVfoB(frequency_hz) => model.desired.vfo_b_hz = frequency_hz,
        TciCommand::SetIqCenter(frequency_hz) => {
            rx.tune(frequency_hz)?;
            model.desired.iq_center_hz = frequency_hz;
        }
        TciCommand::SetMode(mode) => {
            let mode = if mode == DemodMode::Wfm && !crate::wdsp::wbfm_supported() {
                eprintln!("saturn-bridge: direct XDMA WFM unavailable in this WDSP build; using FM");
                DemodMode::Fm
            } else {
                mode
            };
            model.desired.mode = mode;
            (model.desired.filter_low_hz, model.desired.filter_high_hz) =
                mode.default_filter_band();
        }
        TciCommand::SetFilterBand { low_hz, high_hz } => {
            model.desired.filter_low_hz = low_hz;
            model.desired.filter_high_hz = high_hz;
        }
        TciCommand::SetRxAdc(adc) => {
            if adc != 0 {
                eprintln!(
                    "saturn-bridge: direct XDMA RX currently supports ADC1 only; refusing ADC{}",
                    adc.saturating_add(1)
                );
            }
            model.desired.ddc0_adc = 0;
        }
        TciCommand::SetRxAntenna(antenna) => {
            model.desired.rx_antenna = antenna.clamp(1, 3);
            eprintln!(
                "saturn-bridge: direct XDMA RX antenna selection is not yet wired; retaining hardware relay state"
            );
        }
        TciCommand::SetRxVolume(value) => model.desired.rx_volume_db = value.clamp(-40.0, 12.0),
        TciCommand::SetRxNoiseReductionMode(mode) => {
            model.desired.rx_noise_reduction_mode = mode
        }
        TciCommand::SetRxNoiseReductionEnabled(enabled) => {
            model.desired.rx_noise_reduction_mode = if enabled {
                NoiseReductionMode::Nr1
            } else {
                NoiseReductionMode::Off
            }
        }
        TciCommand::SetRxNoiseReductionLevel(level) => {
            model.desired.rx_noise_reduction_level = level.clamp(0.0, 100.0)
        }
        TciCommand::SetRxNr2GainMethod(method) => model.desired.rx_nr2_gain_method = method,
        TciCommand::SetRxNr2NpeMethod(method) => model.desired.rx_nr2_npe_method = method,
        TciCommand::SetRxNr2PostFilterEnabled(enabled) => {
            model.desired.rx_nr2_post_filter_enabled = enabled
        }
        TciCommand::SetRxWbfmDeemphasis(value) => model.desired.rx_wbfm_deemphasis = value,
        TciCommand::SetRxAnrVals {
            taps,
            delay,
            gain,
            leakage,
        } => {
            if let Some(value) = taps {
                model.desired.rx_anr_taps = value.clamp(1, 128);
            }
            if let Some(value) = delay {
                model.desired.rx_anr_delay = value.clamp(0, 127);
            }
            if let Some(value) = gain {
                model.desired.rx_anr_gain = value.clamp(0.0, 1.0);
            }
            if let Some(value) = leakage {
                model.desired.rx_anr_leakage = value.clamp(0.0, 1.0);
            }
        }
        TciCommand::SetIqSampleRate(rate_hz) => {
            if rate_hz != DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000 {
                eprintln!(
                    "saturn-bridge: direct XDMA RX rate is fixed at {} Hz; refusing {} Hz",
                    DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000,
                    rate_hz
                );
            }
        }
        TciCommand::SetIqStreaming | TciCommand::RequestSmeter => {}
        TciCommand::SaturnPing {
            client_id,
            nonce,
            sent_at,
        } => tci.publish_saturn_pong(client_id, &nonce, &sent_at),
        TciCommand::SplitSessionOpen {
            client_id,
            session_id,
            role,
        } => println!(
            "saturn-bridge: direct XDMA split client {client_id} opened session {session_id} as {role:?}"
        ),
        TciCommand::SplitSessionLane {
            client_id,
            session_id,
            lane,
        } => println!(
            "saturn-bridge: direct XDMA split client {client_id} marked {lane:?} lane for session {session_id}"
        ),
        TciCommand::SetAudioStreaming(enabled) => {
            wdsp.reset_audio_packetizer();
            if enabled {
                tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
            } else {
                tci.publish_audio_stopped();
            }
        }
        TciCommand::SetAudioSampleRate(_rate_hz) => {
            wdsp.reset_audio_packetizer();
            tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
        }
        TciCommand::SetAudioFrameSamples(samples) => {
            let normalized = normalize_audio_frame_float_count(samples as usize);
            wdsp.set_audio_frame_float_count(normalized);
        }
        TciCommand::SetAudioChannels(_channels) => {
            tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
        }
        TciCommand::ClientConnected => model.desired.running = true,
        TciCommand::ClientDisconnected => {
            tx_control.requested = false;
            tx_control.last_mic_at = None;
            let _ = tx_cmd_tx.send(TxCommand::Disarm);
            model.desired.tx_enabled = false;
            model.desired.tx_phase = TxPhase::Rx;
        }
        TciCommand::SetTxEnabled(enabled) => {
            if enabled && model.desired.mode == DemodMode::Wfm {
                eprintln!("saturn-bridge: refusing direct XDMA TX while WFM receive mode is active");
                model.desired.tx_enabled = false;
                model.desired.tx_phase = TxPhase::Rx;
            } else if enabled && !tx_control.requested {
                tx_control.requested = true;
                tx_control.last_mic_at = None;
                model.desired.tx_enabled = false;
                model.desired.tx_phase = TxPhase::Armed;
                model.desired.tx_drive = model.desired.tx_drive.min(DIRECT_TX_MAX_WATTS);
                tci.clear_split_release_window();
                tci.set_tx_media_priority_active(true);
                let _ = tx_cmd_tx.send(TxCommand::Arm {
                    rf_enabled: remote_tx_rf_enabled,
                });
                println!(
                    "saturn-bridge: direct XDMA TX armed; waiting for DUC IQ audio{}",
                    if remote_tx_rf_enabled { "" } else { " (RF inhibited)" }
                );
            } else if !enabled && (tx_control.requested || model.desired.tx_enabled) {
                tx_control.requested = false;
                tx_control.last_mic_at = None;
                tci.mark_split_released(Instant::now());
                tci.set_tx_media_priority_active(false);
                let _ = tx_cmd_tx.send(TxCommand::Disarm);
                model.desired.tx_enabled = false;
                model.desired.tx_phase = TxPhase::Rx;
            }
        }
        TciCommand::SetNoiseBlankerMode(mode) => model.desired.nb_mode = mode,
        TciCommand::SetNoiseBlankerThreshold(value) => {
            model.desired.nb_threshold = value.clamp(0.0, 100.0)
        }
        TciCommand::SetAnfEnabled(enabled) => model.desired.anf_enabled = enabled,
        TciCommand::SetRxAnfVals {
            taps,
            delay,
            gain,
            leakage,
        } => {
            if let Some(value) = taps {
                model.desired.rx_anf_taps = value.clamp(1, 128);
            }
            if let Some(value) = delay {
                model.desired.rx_anf_delay = value.clamp(0, 127);
            }
            if let Some(value) = gain {
                model.desired.rx_anf_gain = value.clamp(0.0, 1.0);
            }
            if let Some(value) = leakage {
                model.desired.rx_anf_leakage = value.clamp(0.0, 1.0);
            }
        }
        TciCommand::SetAgcMode(mode) => model.desired.agc_mode = mode,
        TciCommand::SetAgcGain(value) => model.desired.agc_gain = value.clamp(0.0, 100.0),
        TciCommand::SetRxEqEnabled(enabled) => model.desired.rx_eq_enabled = enabled,
        TciCommand::SetRxEqBand { band, gain_db } => {
            model.desired.rx_eq_bands[band] = gain_db.clamp(-20, 20)
        }
        TciCommand::SetRxFftSize(size) => {
            let clamped = size.clamp(1024, 262_144);
            model.desired.rx_fft_size = 1 << (31 - clamped.leading_zeros());
        }
        TciCommand::SetRxLowLatency(enabled) => model.desired.rx_low_latency = enabled,
        TciCommand::SetTxDrive(drive) => {
            model.desired.tx_drive = drive.clamp(1, DIRECT_TX_MAX_WATTS);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxMicGain(gain_db) => {
            model.desired.tx_mic_gain_db = gain_db.clamp(-20.0, 20.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxFilterBand { low_hz, high_hz } => {
            model.desired.tx_filter_low_hz = low_hz;
            model.desired.tx_filter_high_hz = high_hz;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxEqEnabled(enabled) => {
            model.desired.tx_eq_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxEqBand { band, gain_db } => {
            model.desired.tx_eq_bands[band] = gain_db.clamp(-20, 20);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxCfcEnabled(enabled) => {
            model.desired.cfc_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxCfcPrecomp(db) => {
            model.desired.cfc_precomp_db = db.clamp(0.0, 20.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxCfcBand { band, gain_db } => {
            if (1..=model.desired.cfc_bands.len()).contains(&band) {
                model.desired.cfc_bands[band - 1] = gain_db.clamp(0.0, 20.0);
                let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
            }
        }
        TciCommand::SetTxPhaseRotatorEnabled(enabled) => {
            model.desired.tx_phase_rotator_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxPhaseRotatorAuto(enabled) => {
            model.desired.tx_phase_rotator_auto = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxPhaseRotatorCorner(corner_hz) => {
            model.desired.tx_phase_rotator_corner_hz = corner_hz.clamp(50.0, 2_000.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxNoiseGateEnabled(enabled) => {
            model.desired.tx_noise_gate_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxNoiseGateThreshold(threshold_db) => {
            model.desired.tx_noise_gate_threshold_db = threshold_db.clamp(-80.0, 0.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxTwoToneTest(enabled) => {
            if enabled && remote_tx_rf_enabled {
                eprintln!("saturn-bridge: direct XDMA production two-tone is disabled");
                model.desired.two_tone_enabled = false;
            } else {
                model.desired.two_tone_enabled = enabled;
                let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
            }
        }
        TciCommand::SetTxTwoToneFreq1(value) => {
            model.desired.tx_two_tone_freq1_hz = value.clamp(10.0, 10_000.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxTwoToneFreq2(value) => {
            model.desired.tx_two_tone_freq2_hz = value.clamp(10.0, 10_000.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxTwoToneLevelDb(value) => {
            model.desired.tx_two_tone_level_db = value.clamp(-40.0, 0.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxTwoToneInvertLsb(enabled) => {
            model.desired.tx_two_tone_invert_lsb = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxTwoToneDelayMs(value) => {
            model.desired.tx_two_tone_delay_ms = value.min(2_000);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxFftSize(size) => {
            let clamped = size.clamp(1024, 262_144);
            model.desired.tx_fft_size = 1 << (31 - clamped.leading_zeros());
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxLowLatency(enabled) => {
            model.desired.tx_low_latency = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::MicAudioFrame(frame) => {
            if tx_control.requested {
                tx_control.last_mic_at = Some(frame.received_at);
                let _ = tx_cmd_tx.send(TxCommand::MicAudio {
                    samples: frame.samples,
                    channels: frame.channels,
                    sample_rate_hz: frame.sample_rate_hz,
                });
            }
        }
        TciCommand::SetPureSignalEnabled(_)
        | TciCommand::SetPureSignalAutoAttenuate(_)
        | TciCommand::SetPureSignalAttenuation(_)
        | TciCommand::ResetPureSignal => {
            model.desired.pure_signal_enabled = false;
            model.observed.pure_signal_state = PureSignalState::Off;
            eprintln!("saturn-bridge: PureSignal is unavailable on direct XDMA production TX");
        }
    }
    Ok(effects)
}

fn write_readiness(
    path: &Path,
    status: &str,
    identity: &SaturnIdentity,
    rx: &OperationalRxSession,
    tx: DirectTxSnapshot,
    remote_tx_rf_enabled: bool,
) -> Result<(), XdmaError> {
    if !tx.stream_active {
        rx.verify_receive_safe()?;
    }
    let stats = rx.stats();
    record_runtime_readiness(
        path,
        status,
        None,
        &[
            ("product", TelemetryValue::number(identity.product_id)),
            ("pcb", TelemetryValue::number(identity.pcb_version)),
            (
                "firmware",
                TelemetryValue::text(format!(
                    "{}.{}",
                    identity.firmware_major, identity.firmware_minor
                )),
            ),
            ("ddc", TelemetryValue::number(DIRECT_DDC_INDEX)),
            ("adc", TelemetryValue::text("ADC1")),
            ("frequency_hz", TelemetryValue::number(rx.frequency_hz())),
            (
                "sample_rate_hz",
                TelemetryValue::number(DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000),
            ),
            ("dma_reads", TelemetryValue::number(stats.dma_reads)),
            ("dma_bytes", TelemetryValue::number(stats.dma_bytes)),
            ("iq_pairs", TelemetryValue::number(stats.samples)),
            ("fifo_hwm", TelemetryValue::number(stats.fifo_depth_hwm)),
            (
                "fifo_startup_threshold_recoveries",
                TelemetryValue::number(stats.fifo_startup_over_threshold),
            ),
            (
                "fifo_threshold",
                TelemetryValue::number(stats.fifo_over_threshold),
            ),
            (
                "fifo_overflow",
                TelemetryValue::number(stats.fifo_overflows),
            ),
            (
                "fifo_underflow",
                TelemetryValue::number(stats.fifo_underflows),
            ),
            (
                "header_resync",
                TelemetryValue::number(stats.header_resyncs),
            ),
            ("header_errors", TelemetryValue::number(stats.header_errors)),
            ("rf_safe", TelemetryValue::boolean(!tx.keyed)),
            ("tx_capable", TelemetryValue::boolean(true)),
            (
                "tx_rf_enabled",
                TelemetryValue::boolean(remote_tx_rf_enabled),
            ),
            ("tx_max_watts", TelemetryValue::number(DIRECT_TX_MAX_WATTS)),
            (
                "tx_stream_active",
                TelemetryValue::boolean(tx.stream_active),
            ),
            ("tx_keyed", TelemetryValue::boolean(tx.keyed)),
            ("tx_dma_writes", TelemetryValue::number(tx.dma_writes)),
            ("tx_frames", TelemetryValue::number(tx.frames_written)),
            ("tx_fifo_lwm", TelemetryValue::number(tx.fifo_lwm)),
            ("tx_fifo_hwm", TelemetryValue::number(tx.fifo_hwm)),
            ("tx_fifo_faults", TelemetryValue::number(tx.fifo_faults)),
            (
                "tx_fifo_startup_underflows",
                TelemetryValue::number(tx.fifo_startup_underflows),
            ),
            ("forward_watts", TelemetryValue::number(tx.forward_watts)),
            ("reverse_watts", TelemetryValue::number(tx.reverse_watts)),
            ("swr", TelemetryValue::number(tx.swr)),
        ],
    )
    .map_err(|source| XdmaError::Io {
        action: "could not persist operational XDMA readiness",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_advancing_dma_and_iq() {
        assert!(READY_DMA_READS > 0);
        assert!(READY_IQ_PAIRS >= READY_DMA_READS);
    }
}
