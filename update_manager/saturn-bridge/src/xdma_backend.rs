//! Operational direct-XDMA radio backend.
//!
//! P2app remains the default service/backend. When the transactional backend
//! switch grants this process exclusive hardware ownership, this runtime owns
//! the proven DDC receive path plus the shared WDSP TX pipeline and production
//! H2C/DUC output.

use crate::config::BridgeConfig;
use crate::radio_model::{DemodMode, NoiseReductionMode, PureSignalState, RadioModel, TxPhase};
use crate::rx_thread::correct_smeter_dbm;
use crate::sync_ext::MutexExt;
use crate::tci::{TciClientSnapshot, TciCommand, TciFrontend, TciMediaDemand};
use crate::tx_audio::{TxAudioIngress, TxAudioSource};
use crate::tx_thread::{self, TxCommand, TxEvent};
use crate::wdsp::{normalize_audio_frame_float_count, WdspRxEngine, WDSP_AUDIO_RATE_HZ};
use crate::xdma::{SaturnIdentity, XdmaError};
use crate::xdma_rx::{
    FpgaAdcV30Telemetry, FpgaFifoV29Telemetry, OperationalRxSession, RxCaptureStats,
    DIRECT_DDC_INDEX, DIRECT_DDC_SAMPLE_RATE_KHZ, OPERATIONAL_RX_BUFFER_BYTES,
    OPERATIONAL_RX_BUFFER_COUNT, RUNTIME_HOST_DRAIN_MAX_READS,
};
use crate::xdma_telemetry::{record_runtime_performance, record_runtime_readiness, TelemetryValue};
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
const DEFAULT_PERF_PATH: &str = "/run/saturn-bridge/perf.json";
const READY_DMA_READS: u64 = 4;
const READY_IQ_PAIRS: u64 = 1_024;
const MAX_COMMANDS_PER_LOOP: usize = 8;
const IDLE_POLL: Duration = Duration::from_micros(250);
const READINESS_PERIOD: Duration = Duration::from_secs(1);
const STATUS_PERIOD: Duration = Duration::from_secs(5);
const PERF_PERIOD: Duration = Duration::from_secs(1);
const DIAG_PERIOD: Duration = Duration::from_secs(5);
const MEDIA_DEMAND_REFRESH: Duration = Duration::from_millis(10);
const IDLE_METER_PERIOD: Duration = Duration::from_millis(100);
const DIRECT_TX_MAX_WATTS: u8 = 100;
const TX_UPLINK_TIMEOUT: Duration = Duration::from_millis(750);
const TX_CONTROL_TIMEOUT: Duration = Duration::from_millis(1_500);

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
struct CommandEffects {
    dsp_dirty: bool,
    tuning_dirty: bool,
    tx_state_dirty: bool,
    radio_state_dirty: bool,
}

#[derive(Debug, Default)]
struct DirectTxControl {
    requested: bool,
    last_mic_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RxProcessingMode {
    Audio,
    IqOnly,
    MeterOnly,
    DrainOnly,
}

impl RxProcessingMode {
    fn label(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::IqOnly => "iq-only",
            Self::MeterOnly => "meter-only",
            Self::DrainOnly => "drain-only",
        }
    }

    fn decodes_iq(self) -> bool {
        self != Self::DrainOnly
    }
}

#[derive(Debug, Default)]
struct DirectRxPerformance {
    iq_frames_published: u64,
    audio_frames_published: u64,
    audio_samples_published: u64,
    dsp_iq_pairs: u64,
    bypassed_iq_pairs: u64,
    meter_only_iq_pairs: u64,
}

#[derive(Debug, Default)]
struct WdspResumePerformance {
    count: u64,
    flush_failures: u64,
    last_us: u64,
    max_us: u64,
}

fn rx_processing_mode(demand: TciMediaDemand, meter_due: bool) -> RxProcessingMode {
    if demand.audio_stream_enabled {
        RxProcessingMode::Audio
    } else if demand.iq_stream_enabled {
        RxProcessingMode::IqOnly
    } else if meter_due {
        RxProcessingMode::MeterOnly
    } else {
        RxProcessingMode::DrainOnly
    }
}

fn iq_rms_dbfs(iq_samples: &[f32]) -> Option<f32> {
    let mut power = 0.0f64;
    let mut count = 0u64;
    for pair in iq_samples.chunks_exact(2) {
        // Match WDSP's RXA_S_AV convention: mean complex power is I^2 + Q^2,
        // without an additional real-signal 1/2 factor.  That keeps the
        // low-rate meter continuous when the audio consumer starts or stops.
        power += f64::from(pair[0] * pair[0] + pair[1] * pair[1]);
        count += 1;
    }
    (count != 0).then(|| (10.0 * (power / count as f64).max(1.0e-20).log10()) as f32)
}

impl CommandEffects {
    fn merge(&mut self, other: Self) {
        self.dsp_dirty |= other.dsp_dirty;
        self.tuning_dirty |= other.tuning_dirty;
        self.tx_state_dirty |= other.tx_state_dirty;
        self.radio_state_dirty |= other.radio_state_dirty;
    }
}

fn effective_direct_tx_rf_enabled(requested: bool, firmware_qualified: bool) -> bool {
    requested && firmware_qualified
}

fn command_effects(command: &TciCommand) -> CommandEffects {
    CommandEffects {
        dsp_dirty: matches!(
            command,
            TciCommand::SetMode(_)
                | TciCommand::SetFilterBand { .. }
                | TciCommand::SetRxVolume(_)
                | TciCommand::SetRxSsqlEnabled(_)
                | TciCommand::SetRxSsqlThreshold(_)
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
            command,
            TciCommand::SetVfoA(_)
                | TciCommand::SetVfoB(_)
                | TciCommand::SetActiveVfo(_)
                | TciCommand::SetSplitEnabled(_)
                | TciCommand::SetIqCenter(_)
        ),
        tx_state_dirty: matches!(
            command,
            TciCommand::SetTxEnabled(_) | TciCommand::ClientDisconnected
        ),
        // Mic frames are media, not model mutations. Publishing the complete
        // radio state for every 20 ms browser frame consumed several
        // milliseconds in the direct-XDMA control loop and could starve DUC
        // pacing. A mixed batch still publishes when any real control command
        // is present.
        radio_state_dirty: !matches!(
            command,
            TciCommand::MicAudioFrame(_)
                | TciCommand::SaturnPing { .. }
                | TciCommand::RequestRadioState { .. }
        ),
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

fn run_inner(mut config: BridgeConfig, ready_path: &Path) -> Result<(), Box<dyn Error>> {
    let _signal_guard = SignalGuard::install()?;
    let perf_path = env::var_os("SATURN_BRIDGE_PERF_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PERF_PATH));
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
    let tx_radio = Arc::new(DirectXdmaTxRadio::open(config.tx_power_meter_scale)?);
    let requested_tx_rf_enabled = config.remote_tx_rf_enabled;
    let remote_tx_rf_enabled =
        effective_direct_tx_rf_enabled(requested_tx_rf_enabled, tx_radio.rf_tx_qualified());
    if requested_tx_rf_enabled && !remote_tx_rf_enabled {
        eprintln!(
            "saturn-bridge: direct XDMA RF TX requested but this firmware is not yet TX-qualified; continuing RX with RF TX inhibited"
        );
    }
    // TCI capability messages must advertise the effective safety gate, not
    // merely the configured request, so legacy clients cannot mistake an
    // unqualified firmware image for an RF-enabled backend.
    config.remote_tx_rf_enabled = remote_tx_rf_enabled;
    let (tci, command_rx) = TciFrontend::bind(&config, radio_model.clone())?;
    let tci = Arc::new(tci);
    let (tx_cmd_tx, tx_cmd_rx) = mpsc::channel();
    let (tx_audio_ingress, tx_audio_rx, tx_audio_stats) = TxAudioIngress::bounded();
    let (tx_event_tx, tx_event_rx) = mpsc::channel();
    let tx_stop = Arc::new(AtomicBool::new(false));
    let tx_worker = tx_thread::spawn(
        tx_radio.clone(),
        radio_model.clone(),
        tx_cmd_rx,
        tx_audio_rx,
        tx_audio_stats,
        config.tx_audio_source,
        tx_event_tx,
        tx_stop.clone(),
    )?;
    let _satp_runtime = crate::satp::SatpRuntime::start(
        &config,
        radio_model.clone(),
        tx_audio_ingress.clone(),
        tx_cmd_tx.clone(),
    )?;
    let mut tx_control = DirectTxControl::default();
    let mut wdsp = {
        let model = radio_model.lock_unpoisoned();
        WdspRxEngine::new(&model)?
    };
    let (rx_frequency_hz, rx_antenna, rx_attenuation_db) = {
        let model = radio_model.lock_unpoisoned();
        (
            model.desired.iq_center_hz,
            model.desired.rx_antenna,
            model.desired.rx_attenuation_db,
        )
    };
    let mut rx = OperationalRxSession::open(rx_frequency_hz, rx_antenna, rx_attenuation_db)?;
    let identity = rx.identity().clone();
    let mut iq_samples = Vec::with_capacity(8_192);
    rx.drain_startup_fifo(&mut iq_samples)?;
    let mut readiness_state = "starting";
    let mut last_readiness = Instant::now();
    let mut last_status = Instant::now();
    let mut last_perf = Instant::now();
    let mut last_diag = Instant::now() - DIAG_PERIOD;
    let mut last_perf_stats = rx.stats();
    let mut last_media_demand_refresh = Instant::now() - MEDIA_DEMAND_REFRESH;
    let mut media_demand = TciMediaDemand::default();
    let mut last_idle_meter = Instant::now() - IDLE_METER_PERIOD;
    let mut wdsp_input_gap = false;
    let mut rx_performance = DirectRxPerformance::default();
    let mut wdsp_resume_performance = WdspResumePerformance::default();
    let mut meter_source = "unavailable";
    let mut current_processing_mode = RxProcessingMode::DrainOnly;

    write_readiness(
        ready_path,
        readiness_state,
        &identity,
        &rx,
        tx_radio.snapshot(),
        remote_tx_rf_enabled,
        requested_tx_rf_enabled,
    )?;
    println!(
        "saturn-bridge: direct XDMA backend starting product={} pcb={} firmware={}.{} ddc={} adc=ADC1 frequency={}Hz rate={}kHz rx_buffers={} rx_locked_bytes={} TCI={} TX={} max={}W PureSignal=disabled",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        DIRECT_DDC_INDEX,
        rx.frequency_hz(),
        DIRECT_DDC_SAMPLE_RATE_KHZ,
        OPERATIONAL_RX_BUFFER_COUNT,
        OPERATIONAL_RX_BUFFER_BYTES,
        config.tci_bind_addr,
        if remote_tx_rf_enabled { "RF-enabled" } else { "RF-inhibited" },
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
            if last_media_demand_refresh.elapsed() >= MEDIA_DEMAND_REFRESH {
                media_demand = tci.media_demand();
                last_media_demand_refresh = Instant::now();
            }
            // Consume a bounded slice of the host ring before client control
            // work. The dedicated reader continues servicing the hardware FIFO
            // during every DSP, publication, command, and filesystem operation.
            for _ in 0..RUNTIME_HOST_DRAIN_MAX_READS {
                let mode = rx_processing_mode(
                    media_demand,
                    last_idle_meter.elapsed() >= IDLE_METER_PERIOD,
                );
                current_processing_mode = mode;
                let outcome = if mode.decodes_iq() {
                    rx.read_iq(&mut iq_samples)?
                } else {
                    rx.read_iq_discard()?
                };
                if outcome.samples_ready {
                    did_work = true;
                    let meter_due = last_idle_meter.elapsed() >= IDLE_METER_PERIOD;
                    let raw_meter = (mode != RxProcessingMode::Audio && meter_due)
                        .then(|| iq_rms_dbfs(&iq_samples))
                        .flatten();
                    meter_source = if mode == RxProcessingMode::Audio {
                        "wdsp"
                    } else {
                        "raw-iq-estimate"
                    };
                    if media_demand.iq_stream_enabled {
                        tci.publish_iq_frame(DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000, &iq_samples);
                        rx_performance.iq_frames_published += 1;
                    }
                    if mode == RxProcessingMode::Audio {
                        if wdsp_input_gap {
                            let restart_started = Instant::now();
                            let flushed = wdsp.restart_after_input_gap();
                            let elapsed_us = restart_started
                                .elapsed()
                                .as_micros()
                                .min(u128::from(u64::MAX))
                                as u64;
                            wdsp_resume_performance.count += 1;
                            wdsp_resume_performance.last_us = elapsed_us;
                            wdsp_resume_performance.max_us =
                                wdsp_resume_performance.max_us.max(elapsed_us);
                            if !flushed {
                                wdsp_resume_performance.flush_failures += 1;
                            }
                            wdsp_input_gap = false;
                        }
                        rx_performance.dsp_iq_pairs += outcome.sample_pairs;
                        for audio in wdsp.push_iq(&iq_samples) {
                            rx_performance.audio_frames_published += 1;
                            rx_performance.audio_samples_published += audio.len() as u64;
                            tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio);
                        }
                    } else {
                        wdsp_input_gap = true;
                        rx_performance.bypassed_iq_pairs += outcome.sample_pairs;
                        if mode == RxProcessingMode::MeterOnly {
                            rx_performance.meter_only_iq_pairs += outcome.sample_pairs;
                        }
                    }
                    if meter_due && mode != RxProcessingMode::DrainOnly {
                        let mut model = radio_model.lock_unpoisoned();
                        model.observed.ddc0_packets = rx.stats().dma_reads;
                        let meter = if mode == RxProcessingMode::Audio {
                            wdsp.smeter_dbm()
                        } else {
                            raw_meter
                        };
                        if let Some(raw_dbm) = meter {
                            model.observed.ddc0_meter_dbm = Some(correct_smeter_dbm(
                                raw_dbm,
                                model.desired.rx_attenuation_db,
                                config.smeter_calibration_db,
                            ));
                        }
                        model.observed.rx_wbfm_stereo_detected =
                            mode == RxProcessingMode::Audio && wdsp.wbfm_stereo_detected();
                        last_idle_meter = Instant::now();
                    }
                }
                if !outcome.requires_drain {
                    break;
                }
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
                    remote_tx_rf_enabled,
                    config.tx_audio_source,
                    &tx_audio_ingress,
                )?);
            }
            if command_count != 0 {
                if command_effects.dsp_dirty
                    || command_effects.tuning_dirty
                    || command_effects.tx_state_dirty
                    || command_effects.radio_state_dirty
                {
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
                    if command_effects.radio_state_dirty {
                        tci.publish_radio_state(&model);
                    }
                }
                let command_elapsed = command_started.elapsed();
                if command_elapsed >= Duration::from_millis(5) {
                    println!(
                    "saturn-bridge: xdma_rx control batch commands={} dsp_sync={} tuning_publish={} tx_publish={} radio_publish={} elapsed_us={}",
                    command_count,
                    u8::from(command_effects.dsp_dirty),
                    u8::from(command_effects.tuning_dirty),
                    u8::from(command_effects.tx_state_dirty),
                    u8::from(command_effects.radio_state_dirty),
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
                    TxEvent::Diagnostics(diag) => {
                        let mut model = radio_model.lock_unpoisoned();
                        let tx = tx_radio.snapshot();
                        model.observed.tx_forward_watts = Some(tx.forward_watts);
                        model.observed.tx_reflected_watts = Some(tx.reverse_watts);
                        model.observed.tx_swr = Some(tx.swr);
                        model.observed.tx_mic_peak_db = Some(diag.mic_peak_db);
                        model.observed.tx_comp_peak_db = Some(diag.comp_peak_db);
                        model.observed.tx_comp_avg_db = Some(diag.comp_avg_db);
                        model.observed.tx_alc_peak_db = Some(diag.alc_peak_db);
                        model.observed.tx_alc_avg_db = Some(diag.alc_avg_db);
                        model.observed.tx_alc_gain_db = Some(diag.alc_gain_db);
                        tci.publish_telemetry(&model);
                    }
                    TxEvent::PureSignalStatus(_status) => {}
                }
            }

            if tx_control.requested {
                let now = Instant::now();
                let mic_stale = config.tx_audio_source == TxAudioSource::Tci
                    && tx_control.last_mic_at.is_some_and(|last| {
                        now.saturating_duration_since(last) > TX_UPLINK_TIMEOUT
                    });
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

            if last_perf.elapsed() >= PERF_PERIOD {
                let elapsed = last_perf.elapsed().as_secs_f64().max(0.001);
                let client = tci.client_snapshot();
                media_demand = TciMediaDemand {
                    iq_stream_enabled: client.iq_stream_enabled,
                    audio_stream_enabled: client.audio_stream_enabled,
                };
                if let Err(error) = rx.sample_extended_telemetry() {
                    eprintln!(
                        "saturn-bridge: extended FPGA telemetry read failed without interrupting RX: {error}"
                    );
                }
                let stats = rx.stats();
                if let Err(error) = write_performance(
                    &perf_path,
                    readiness_state,
                    &identity,
                    &rx,
                    tx_radio.snapshot(),
                    &client,
                    current_processing_mode,
                    meter_source,
                    elapsed,
                    &stats,
                    &last_perf_stats,
                    &rx_performance,
                    &wdsp_resume_performance,
                    rx.fifo_v29_telemetry(),
                    rx.adc_v30_telemetry(),
                ) {
                    eprintln!(
                        "saturn-bridge: performance telemetry write failed without interrupting RX: {error}"
                    );
                }
                if last_diag.elapsed() >= DIAG_PERIOD {
                    println!(
                    "saturn-bridge: diag hp_s=0.0 ddc_s={:.1} rx_audio_frames_s={:.1} rx_audio_samples_s={:.0} tci_mic_frames_s=0.0 tci_mic_samples_s=0 client={} connections={} connection_limit={} connection_rejected={} connection_hwm={} iq={} audio={} split_control={} split_media={} split_paired={} outbound_drops={} safety_p99_us={} control_p99_us={} control_replaced_s={} control_dropped_s={} control_q_hwm={} display_replaced_s={} display_dropped_s={} display_rate_limited_s={} audio_dropped_s={} audio_gaps={} audio_panic={} command_q={} command_q_hwm={} command_coalesced={} command_dropped={} command_mic_dropped={} send_blocked_ms={} out_hwm_bytes={} tcp_outq_hwm_bytes={} safety_depth_overflow={} processing={} dsp_iq_pairs={} bypassed_iq_pairs={} meter_source={}",
                    stats.dma_reads.saturating_sub(last_perf_stats.dma_reads) as f64 / elapsed,
                    rx_performance.audio_frames_published as f64 / elapsed,
                    rx_performance.audio_samples_published as f64 / elapsed,
                    u8::from(client.active),
                    client.active_connections,
                    client.connection_limit,
                    client.rejected_connections,
                    client.connection_high_watermark,
                    u8::from(client.iq_stream_enabled),
                    u8::from(client.audio_stream_enabled),
                    client.split_control_clients,
                    client.split_media_clients,
                    client.split_paired_sessions,
                    client.outbound_drops,
                    client.safety_enqueue_to_write_p99_us,
                    client.control_enqueue_to_write_p99_us,
                    client.control_replaced_per_sec,
                    client.control_dropped_per_sec,
                    client.control_queue_high_watermark,
                    client.display_replaced_per_sec,
                    client.display_dropped_per_sec,
                    client.display_rate_limited_per_sec,
                    client.audio_dropped_per_sec,
                    client.audio_seq_gap_count,
                    client.audio_panic_drain_count,
                    client.command_queue_depth,
                    client.command_queue_high_watermark,
                    client.command_control_coalesced,
                    client.command_control_dropped,
                    client.command_mic_dropped,
                    client.send_blocked_ms,
                    client.outbound_high_watermark_bytes,
                    client.tcp_outq_high_watermark_bytes,
                    client.safety_queue_depth_overflow_count,
                    current_processing_mode.label(),
                    rx_performance.dsp_iq_pairs,
                    rx_performance.bypassed_iq_pairs,
                    meter_source,
                );
                    last_diag = Instant::now();
                }
                tci.publish_scheduler_telemetry(&client);
                tci.publish_tx_uplink_telemetry(&client);
                last_perf_stats = stats;
                rx_performance = DirectRxPerformance::default();
                last_perf = Instant::now();
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
                    remote_tx_rf_enabled,
                    requested_tx_rf_enabled,
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
                    remote_tx_rf_enabled,
                    requested_tx_rf_enabled,
                )?;
                last_readiness = Instant::now();
            }
            if last_status.elapsed() >= STATUS_PERIOD {
                if let Ok((overflow, adc1_peak, adc2_peak)) = rx.read_adc_telemetry() {
                    let mut model = radio_model.lock_unpoisoned();
                    model.observed.adc_overflows = overflow;
                    model.observed.adc1_peak = adc1_peak;
                    model.observed.adc2_peak = adc2_peak;
                    let tx = tx_radio.snapshot();
                    model.observed.tx_forward_watts = Some(tx.forward_watts);
                    model.observed.tx_reflected_watts = Some(tx.reverse_watts);
                    model.observed.tx_swr = Some(tx.swr);
                    tci.publish_telemetry(&model);
                }
                let stats = rx.stats();
                let tx = tx_radio.snapshot();
                println!(
                "saturn-bridge: xdma status={} frequency_hz={} dma_reads={} dma_bytes={} iq_pairs={} rx_fifo_hwm={} header_resync={} header_errors={} rx_fifo_thresholds={} rx_fifo_almost_full={} rx_fifo_empty_observations={} rx_fifo_faults={} rx_host_ring_hwm={} rx_host_buffer_drops={} rx_host_drop_bytes={} rx_host_discontinuities={} rx_host_pool_starvations={} tx_requested={} tx_stream={} tx_keyed={} tx_dma_writes={} tx_frames={} tx_fifo_lwm={} tx_fifo_hwm={} tx_fifo_faults={} forward_w={:.3} reverse_w={:.3} swr={:.2}",
                readiness_state,
                rx.frequency_hz(),
                stats.dma_reads,
                stats.dma_bytes,
                stats.samples,
                stats.fifo_depth_hwm,
                stats.header_resyncs,
                stats.header_errors,
                stats.fifo_over_threshold + stats.fifo_startup_over_threshold,
                stats.fifo_almost_full,
                stats.fifo_empty_observations,
                stats.fifo_overflows + stats.fifo_underflows,
                stats.host_ring_depth_hwm,
                stats.host_buffer_drops,
                stats.host_buffer_drop_bytes,
                stats.host_discontinuities,
                stats.host_pool_starvations,
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
            (
                "host_buffer_drops",
                TelemetryValue::number(rx.stats().host_buffer_drops),
            ),
            (
                "host_discontinuities",
                TelemetryValue::number(rx.stats().host_discontinuities),
            ),
            (
                "host_pool_starvations",
                TelemetryValue::number(rx.stats().host_pool_starvations),
            ),
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
    tx_audio_source: TxAudioSource,
    tx_audio_ingress: &TxAudioIngress,
) -> Result<CommandEffects, Box<dyn Error>> {
    let effects = command_effects(&command);
    let command = match command {
        TciCommand::MicAudioFrame(frame) => {
            if tx_control.requested {
                tx_control.last_mic_at = Some(frame.received_at);
                if tx_audio_source == TxAudioSource::Tci {
                    let _ = tx_audio_ingress.write_frame(
                        TxAudioSource::Tci,
                        frame.samples,
                        frame.channels,
                        frame.sample_rate_hz,
                    );
                }
            }
            return Ok(effects);
        }
        TciCommand::SaturnPing {
            client_id,
            nonce,
            sent_at,
        } => {
            // The dedicated pong is sufficient for RTT and TX-watchdog
            // freshness. A full state snapshot here turns the 200 ms keyed
            // heartbeat into avoidable control-loop work; the independent
            // one-second S-meter request retains periodic state convergence.
            tci.publish_saturn_pong(client_id, &nonce, &sent_at);
            return Ok(effects);
        }
        command => command,
    };
    let mut model = radio_model.lock_unpoisoned();
    match command {
        TciCommand::SetVfoA(frequency_hz) => {
            model.desired.vfo_a_hz = frequency_hz;
            model.sync_vfo_routes();
            rx.tune(model.desired.iq_center_hz)?;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetVfoB(frequency_hz) => {
            model.desired.vfo_b_hz = frequency_hz;
            model.sync_vfo_routes();
            rx.tune(model.desired.iq_center_hz)?;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetActiveVfo(active) => {
            model.desired.active_vfo = active.min(1);
            model.sync_vfo_routes();
            rx.tune(model.desired.iq_center_hz)?;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetSplitEnabled(enabled) => {
            model.desired.split_enabled = enabled;
            model.sync_vfo_routes();
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
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
            let antenna = antenna.clamp(1, 3);
            rx.set_rx_antenna(antenna)?;
            model.desired.rx_antenna = antenna;
        }
        TciCommand::SetRxAttenuation(attenuation_db) => {
            let attenuation_db = attenuation_db.min(31);
            rx.set_rx_attenuation(attenuation_db)?;
            model.desired.rx_attenuation_db = attenuation_db;
        }
        TciCommand::SetRxVolume(value) => model.desired.rx_volume_db = value.clamp(-40.0, 12.0),
        TciCommand::SetRxSsqlEnabled(enabled) => model.desired.rx_ssql_enabled = enabled,
        TciCommand::SetRxSsqlThreshold(threshold) => {
            model.desired.rx_ssql_threshold = threshold.clamp(0.0, 100.0)
        }
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
        TciCommand::RequestRadioState { client_id } => {
            tci.publish_standard_radio_state_to(client_id, &model);
        }
        TciCommand::SaturnPing { .. } => {
            unreachable!("Saturn heartbeat is handled before model locking")
        }
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
        TciCommand::SetTxDexpEnabled(enabled) => {
            model.desired.tx_dexp_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxDexpThreshold(threshold_db) => {
            model.desired.tx_dexp_threshold_db = threshold_db.clamp(-80.0, -6.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxDexpExpansion(expansion_db) => {
            model.desired.tx_dexp_expansion_db = expansion_db.clamp(0.0, 30.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxSpeechProcessorEnabled(enabled) => {
            model.desired.tx_speech_processor_enabled = enabled;
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxSpeechProcessorGain(gain_db) => {
            model.desired.tx_speech_processor_gain_db = gain_db.clamp(0.0, 20.0);
            let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
        }
        TciCommand::SetTxCessbEnabled(enabled) => {
            model.desired.tx_cessb_enabled = enabled;
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
        TciCommand::MicAudioFrame(_) => unreachable!("mic media is handled before model locking"),
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

#[allow(clippy::too_many_arguments)]
fn write_performance(
    path: &Path,
    status: &str,
    identity: &SaturnIdentity,
    rx: &OperationalRxSession,
    tx: DirectTxSnapshot,
    client: &TciClientSnapshot,
    processing_mode: RxProcessingMode,
    meter_source: &str,
    elapsed: f64,
    stats: &RxCaptureStats,
    previous: &RxCaptureStats,
    performance: &DirectRxPerformance,
    wdsp_resume: &WdspResumePerformance,
    fifo_v29: &FpgaFifoV29Telemetry,
    adc_v30: &FpgaAdcV30Telemetry,
) -> Result<(), XdmaError> {
    let dma_reads_per_sec = stats.dma_reads.saturating_sub(previous.dma_reads) as f64 / elapsed;
    let dma_bytes_per_sec = stats.dma_bytes.saturating_sub(previous.dma_bytes) as f64 / elapsed;
    let iq_pairs_per_sec = stats.samples.saturating_sub(previous.samples) as f64 / elapsed;
    let build_dirty = env!("SATURN_BRIDGE_GIT_DIRTY") == "true";
    record_runtime_performance(
        path,
        status,
        &[
            ("pid", TelemetryValue::number(std::process::id())),
            (
                "build_git_sha",
                TelemetryValue::text(env!("SATURN_BRIDGE_GIT_SHA")),
            ),
            ("build_git_dirty", TelemetryValue::boolean(build_dirty)),
            (
                "build_target_cpu",
                TelemetryValue::text(env!("SATURN_BRIDGE_TARGET_CPU")),
            ),
            (
                "wdsp_flavor",
                TelemetryValue::text(env!("SATURN_BRIDGE_WDSP_FLAVOR")),
            ),
            (
                "wdsp_git_sha",
                TelemetryValue::text(env!("SATURN_BRIDGE_WDSP_COMMIT")),
            ),
            (
                "processing_mode",
                TelemetryValue::text(processing_mode.label()),
            ),
            ("meter_source", TelemetryValue::text(meter_source)),
            ("client", TelemetryValue::number(u8::from(client.active))),
            (
                "connections",
                TelemetryValue::number(client.active_connections),
            ),
            (
                "connection_limit",
                TelemetryValue::number(client.connection_limit),
            ),
            (
                "connection_rejected",
                TelemetryValue::number(client.rejected_connections),
            ),
            (
                "connection_hwm",
                TelemetryValue::number(client.connection_high_watermark),
            ),
            (
                "iq",
                TelemetryValue::number(u8::from(client.iq_stream_enabled)),
            ),
            (
                "audio",
                TelemetryValue::number(u8::from(client.audio_stream_enabled)),
            ),
            (
                "split_control",
                TelemetryValue::number(client.split_control_clients),
            ),
            (
                "split_media",
                TelemetryValue::number(client.split_media_clients),
            ),
            (
                "split_paired",
                TelemetryValue::number(client.split_paired_sessions),
            ),
            ("ddc_s", TelemetryValue::number(dma_reads_per_sec)),
            ("ddc_bytes_s", TelemetryValue::number(dma_bytes_per_sec)),
            ("iq_pairs_s", TelemetryValue::number(iq_pairs_per_sec)),
            ("dma_reads", TelemetryValue::number(stats.dma_reads)),
            ("dma_bytes", TelemetryValue::number(stats.dma_bytes)),
            ("iq_pairs", TelemetryValue::number(stats.samples)),
            (
                "iq_frames_s",
                TelemetryValue::number(performance.iq_frames_published as f64 / elapsed),
            ),
            (
                "rx_audio_frames_s",
                TelemetryValue::number(performance.audio_frames_published as f64 / elapsed),
            ),
            (
                "rx_audio_samples_s",
                TelemetryValue::number(performance.audio_samples_published as f64 / elapsed),
            ),
            (
                "dsp_iq_pairs_s",
                TelemetryValue::number(performance.dsp_iq_pairs as f64 / elapsed),
            ),
            (
                "bypassed_iq_pairs_s",
                TelemetryValue::number(performance.bypassed_iq_pairs as f64 / elapsed),
            ),
            (
                "meter_only_iq_pairs_s",
                TelemetryValue::number(performance.meter_only_iq_pairs as f64 / elapsed),
            ),
            (
                "wdsp_resume_count",
                TelemetryValue::number(wdsp_resume.count),
            ),
            (
                "wdsp_resume_flush_failures",
                TelemetryValue::number(wdsp_resume.flush_failures),
            ),
            (
                "wdsp_resume_last_us",
                TelemetryValue::number(wdsp_resume.last_us),
            ),
            (
                "wdsp_resume_max_us",
                TelemetryValue::number(wdsp_resume.max_us),
            ),
            (
                "outbound_drops",
                TelemetryValue::number(client.outbound_drops),
            ),
            (
                "outbound_queued_bytes",
                TelemetryValue::number(client.outbound_queued_bytes),
            ),
            (
                "out_hwm_bytes",
                TelemetryValue::number(client.outbound_high_watermark_bytes),
            ),
            (
                "tcp_outq_hwm_bytes",
                TelemetryValue::number(client.tcp_outq_high_watermark_bytes),
            ),
            (
                "command_q",
                TelemetryValue::number(client.command_queue_depth),
            ),
            (
                "command_q_hwm",
                TelemetryValue::number(client.command_queue_high_watermark),
            ),
            (
                "audio_dropped_s",
                TelemetryValue::number(client.audio_dropped_per_sec),
            ),
            (
                "display_dropped_s",
                TelemetryValue::number(client.display_dropped_per_sec),
            ),
            (
                "header_resync",
                TelemetryValue::number(stats.header_resyncs),
            ),
            ("header_errors", TelemetryValue::number(stats.header_errors)),
            ("rx_fifo_hwm", TelemetryValue::number(stats.fifo_depth_hwm)),
            (
                "rx_fifo_thresholds",
                TelemetryValue::number(stats.fifo_over_threshold),
            ),
            (
                "rx_fifo_almost_full",
                TelemetryValue::number(stats.fifo_almost_full),
            ),
            (
                "rx_fifo_empty_observations",
                TelemetryValue::number(stats.fifo_empty_observations),
            ),
            (
                "rx_fifo_faults",
                TelemetryValue::number(stats.fifo_overflows.saturating_add(stats.fifo_underflows)),
            ),
            (
                "rx_host_ring_hwm",
                TelemetryValue::number(stats.host_ring_depth_hwm),
            ),
            (
                "host_buffer_drops",
                TelemetryValue::number(stats.host_buffer_drops),
            ),
            (
                "host_buffer_drop_bytes",
                TelemetryValue::number(stats.host_buffer_drop_bytes),
            ),
            (
                "host_discontinuities",
                TelemetryValue::number(stats.host_discontinuities),
            ),
            (
                "host_pool_starvations",
                TelemetryValue::number(stats.host_pool_starvations),
            ),
            (
                "fifo_v29_available",
                TelemetryValue::boolean(fifo_v29.available()),
            ),
            (
                "fifo_v29_status",
                TelemetryValue::text(fifo_v29.status.label()),
            ),
            (
                "fifo_v29_build_id",
                TelemetryValue::number(fifo_v29.build_id),
            ),
            (
                "fifo_v29_snapshot_valid",
                TelemetryValue::boolean(fifo_v29.snapshot_valid),
            ),
            (
                "fifo_v29_snapshot_generation",
                TelemetryValue::number(fifo_v29.snapshot_generation),
            ),
            (
                "fifo_v29_snapshot_timeout_count",
                TelemetryValue::number(fifo_v29.snapshot_timeout_count),
            ),
            (
                "fifo_v29_occupancy_ddc",
                TelemetryValue::number(fifo_v29.occupancy_words[0]),
            ),
            (
                "fifo_v29_occupancy_duc",
                TelemetryValue::number(fifo_v29.occupancy_words[1]),
            ),
            (
                "fifo_v29_occupancy_mic",
                TelemetryValue::number(fifo_v29.occupancy_words[2]),
            ),
            (
                "fifo_v29_occupancy_speaker",
                TelemetryValue::number(fifo_v29.occupancy_words[3]),
            ),
            (
                "fifo_v29_minimum_ddc",
                TelemetryValue::number(fifo_v29.minimum_words[0]),
            ),
            (
                "fifo_v29_minimum_duc",
                TelemetryValue::number(fifo_v29.minimum_words[1]),
            ),
            (
                "fifo_v29_minimum_mic",
                TelemetryValue::number(fifo_v29.minimum_words[2]),
            ),
            (
                "fifo_v29_minimum_speaker",
                TelemetryValue::number(fifo_v29.minimum_words[3]),
            ),
            (
                "fifo_v29_maximum_ddc",
                TelemetryValue::number(fifo_v29.maximum_words[0]),
            ),
            (
                "fifo_v29_maximum_duc",
                TelemetryValue::number(fifo_v29.maximum_words[1]),
            ),
            (
                "fifo_v29_maximum_mic",
                TelemetryValue::number(fifo_v29.maximum_words[2]),
            ),
            (
                "fifo_v29_maximum_speaker",
                TelemetryValue::number(fifo_v29.maximum_words[3]),
            ),
            (
                "fifo_v29_events_ddc",
                TelemetryValue::number(fifo_v29.event_transitions[0]),
            ),
            (
                "fifo_v29_events_duc",
                TelemetryValue::number(fifo_v29.event_transitions[1]),
            ),
            (
                "fifo_v29_events_mic",
                TelemetryValue::number(fifo_v29.event_transitions[2]),
            ),
            (
                "fifo_v29_events_speaker",
                TelemetryValue::number(fifo_v29.event_transitions[3]),
            ),
            (
                "adc_v30_available",
                TelemetryValue::boolean(adc_v30.available()),
            ),
            (
                "adc_v30_status",
                TelemetryValue::text(adc_v30.status.label()),
            ),
            ("adc_v30_build_id", TelemetryValue::number(adc_v30.build_id)),
            (
                "adc_v30_snapshot_valid",
                TelemetryValue::boolean(adc_v30.snapshot_valid),
            ),
            (
                "adc_v30_snapshot_generation",
                TelemetryValue::number(adc_v30.snapshot_generation),
            ),
            (
                "adc_v30_snapshot_retry_failure_count",
                TelemetryValue::number(adc_v30.snapshot_retry_failure_count),
            ),
            ("adc_v30_clock_hz", TelemetryValue::number(adc_v30.clock_hz)),
            (
                "adc_v30_adc1_episode_count",
                TelemetryValue::number(adc_v30.channels[0].episode_count),
            ),
            (
                "adc_v30_adc1_total_high_clocks",
                TelemetryValue::number(adc_v30.channels[0].total_high_clocks),
            ),
            (
                "adc_v30_adc1_longest_episode_clocks",
                TelemetryValue::number(adc_v30.channels[0].longest_episode_clocks),
            ),
            (
                "adc_v30_adc1_latest_episode_clocks",
                TelemetryValue::number(adc_v30.channels[0].latest_episode_clocks),
            ),
            (
                "adc_v30_adc1_latest_episode_peak",
                TelemetryValue::number(adc_v30.channels[0].latest_episode_peak),
            ),
            (
                "adc_v30_adc1_episode_active",
                TelemetryValue::boolean(adc_v30.channels[0].episode_active),
            ),
            (
                "adc_v30_adc1_episode_valid",
                TelemetryValue::boolean(adc_v30.channels[0].episode_valid),
            ),
            (
                "adc_v30_adc2_episode_count",
                TelemetryValue::number(adc_v30.channels[1].episode_count),
            ),
            (
                "adc_v30_adc2_total_high_clocks",
                TelemetryValue::number(adc_v30.channels[1].total_high_clocks),
            ),
            (
                "adc_v30_adc2_longest_episode_clocks",
                TelemetryValue::number(adc_v30.channels[1].longest_episode_clocks),
            ),
            (
                "adc_v30_adc2_latest_episode_clocks",
                TelemetryValue::number(adc_v30.channels[1].latest_episode_clocks),
            ),
            (
                "adc_v30_adc2_latest_episode_peak",
                TelemetryValue::number(adc_v30.channels[1].latest_episode_peak),
            ),
            (
                "adc_v30_adc2_episode_active",
                TelemetryValue::boolean(adc_v30.channels[1].episode_active),
            ),
            (
                "adc_v30_adc2_episode_valid",
                TelemetryValue::boolean(adc_v30.channels[1].episode_valid),
            ),
            ("product_id", TelemetryValue::number(identity.product_id)),
            ("pcb_version", TelemetryValue::number(identity.pcb_version)),
            ("software_id", TelemetryValue::number(identity.software_id)),
            (
                "firmware_major",
                TelemetryValue::number(identity.firmware_major),
            ),
            (
                "firmware_minor",
                TelemetryValue::number(identity.firmware_minor),
            ),
            ("clock_mask", TelemetryValue::number(identity.clock_mask)),
            (
                "date_code_hex",
                TelemetryValue::text(format!("{:08x}", identity.user_version)),
            ),
            (
                "fallback_config",
                TelemetryValue::boolean(identity.is_fallback()),
            ),
            ("frequency_hz", TelemetryValue::number(rx.frequency_hz())),
            (
                "sample_rate_hz",
                TelemetryValue::number(DIRECT_DDC_SAMPLE_RATE_KHZ * 1_000),
            ),
            ("ddc", TelemetryValue::number(DIRECT_DDC_INDEX)),
            ("adc", TelemetryValue::text("ADC1")),
            ("tx_stream", TelemetryValue::boolean(tx.stream_active)),
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
        ],
    )
    .map_err(|source| XdmaError::Io {
        action: "could not persist direct XDMA performance telemetry",
        source,
    })
}

fn write_readiness(
    path: &Path,
    status: &str,
    identity: &SaturnIdentity,
    rx: &OperationalRxSession,
    tx: DirectTxSnapshot,
    remote_tx_rf_enabled: bool,
    requested_tx_rf_enabled: bool,
) -> Result<(), XdmaError> {
    if !tx.stream_active {
        rx.verify_receive_safe()?;
    }
    let stats = rx.stats();
    let last_tx = tx.last_session.unwrap_or_default();
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
                "fifo_almost_full",
                TelemetryValue::number(stats.fifo_almost_full),
            ),
            (
                "fifo_empty_observations",
                TelemetryValue::number(stats.fifo_empty_observations),
            ),
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
            (
                "host_ring_buffers",
                TelemetryValue::number(OPERATIONAL_RX_BUFFER_COUNT),
            ),
            (
                "host_ring_bytes",
                TelemetryValue::number(OPERATIONAL_RX_BUFFER_BYTES),
            ),
            (
                "host_ring_hwm",
                TelemetryValue::number(stats.host_ring_depth_hwm),
            ),
            (
                "host_buffer_drops",
                TelemetryValue::number(stats.host_buffer_drops),
            ),
            (
                "host_buffer_drop_bytes",
                TelemetryValue::number(stats.host_buffer_drop_bytes),
            ),
            (
                "host_discontinuities",
                TelemetryValue::number(stats.host_discontinuities),
            ),
            (
                "host_pool_starvations",
                TelemetryValue::number(stats.host_pool_starvations),
            ),
            ("rf_safe", TelemetryValue::boolean(!tx.keyed)),
            // Preserve the legacy capability contract: the backend accepts TX
            // audio and can exercise RF-inhibited DUC staging. RF authority is
            // the separate, explicit qualification bit below.
            ("tx_capable", TelemetryValue::boolean(true)),
            (
                "tx_rf_qualified",
                TelemetryValue::boolean(tx.rf_tx_qualified),
            ),
            (
                "tx_rf_enabled",
                TelemetryValue::boolean(remote_tx_rf_enabled),
            ),
            (
                "tx_rf_requested",
                TelemetryValue::boolean(requested_tx_rf_enabled),
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
            (
                "tx_sessions_started",
                TelemetryValue::number(tx.sessions_started),
            ),
            (
                "tx_sessions_completed",
                TelemetryValue::number(tx.sessions_completed),
            ),
            ("tx_mux_resets", TelemetryValue::number(tx.mux_resets)),
            ("tx_last_session_id", TelemetryValue::number(last_tx.id)),
            (
                "tx_last_session_duration_ms",
                TelemetryValue::number(last_tx.duration_ms),
            ),
            (
                "tx_last_session_frequency_hz",
                TelemetryValue::number(last_tx.frequency_hz),
            ),
            (
                "tx_last_session_filter_low_hz",
                TelemetryValue::number(last_tx.filter_low_hz),
            ),
            (
                "tx_last_session_filter_high_hz",
                TelemetryValue::number(last_tx.filter_high_hz),
            ),
            (
                "tx_last_session_keyed",
                TelemetryValue::boolean(last_tx.keyed),
            ),
            (
                "tx_last_session_dma_writes",
                TelemetryValue::number(last_tx.dma_writes),
            ),
            (
                "tx_last_session_frames",
                TelemetryValue::number(last_tx.frames_written),
            ),
            (
                "tx_last_session_fifo_lwm",
                TelemetryValue::number(last_tx.fifo_lwm),
            ),
            (
                "tx_last_session_fifo_hwm",
                TelemetryValue::number(last_tx.fifo_hwm),
            ),
            (
                "tx_last_session_fifo_faults",
                TelemetryValue::number(last_tx.fifo_faults),
            ),
            (
                "tx_last_session_startup_underflows",
                TelemetryValue::number(last_tx.startup_underflows),
            ),
            (
                "tx_last_session_mux_resets",
                TelemetryValue::number(last_tx.mux_resets),
            ),
            (
                "tx_last_session_peak_forward_watts",
                TelemetryValue::number(last_tx.peak_forward_watts),
            ),
            (
                "tx_last_session_peak_reverse_watts",
                TelemetryValue::number(last_tx.peak_reverse_watts),
            ),
            (
                "tx_last_session_peak_swr",
                TelemetryValue::number(last_tx.peak_swr),
            ),
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

    fn demand(iq: bool, audio: bool) -> TciMediaDemand {
        TciMediaDemand {
            iq_stream_enabled: iq,
            audio_stream_enabled: audio,
        }
    }

    #[test]
    fn readiness_requires_advancing_dma_and_iq() {
        assert!(READY_DMA_READS > 0);
        assert!(READY_IQ_PAIRS >= READY_DMA_READS);
    }

    #[test]
    fn rx_processing_only_runs_wdsp_for_audio_consumers() {
        assert_eq!(
            rx_processing_mode(demand(true, true), false),
            RxProcessingMode::Audio
        );
        assert_eq!(
            rx_processing_mode(demand(true, false), false),
            RxProcessingMode::IqOnly
        );
        assert_eq!(
            rx_processing_mode(TciMediaDemand::default(), true),
            RxProcessingMode::MeterOnly
        );
        assert_eq!(
            rx_processing_mode(TciMediaDemand::default(), false),
            RxProcessingMode::DrainOnly
        );
    }

    #[test]
    fn raw_iq_meter_uses_complex_mean_power() {
        let meter = iq_rms_dbfs(&[0.5, 0.5, -0.5, -0.5]).unwrap();
        assert!((meter - -3.0103).abs() < 0.001);
        assert_eq!(iq_rms_dbfs(&[]), None);
    }

    #[test]
    fn direct_rf_requires_both_operator_configuration_and_firmware_qualification() {
        assert!(effective_direct_tx_rf_enabled(true, true));
        assert!(!effective_direct_tx_rf_enabled(true, false));
        assert!(!effective_direct_tx_rf_enabled(false, true));
        assert!(!effective_direct_tx_rf_enabled(false, false));
    }

    #[test]
    fn speech_squelch_commands_resynchronize_wdsp() {
        assert!(command_effects(&TciCommand::SetRxSsqlEnabled(true)).dsp_dirty);
        assert!(command_effects(&TciCommand::SetRxSsqlThreshold(16.0)).dsp_dirty);
    }

    #[test]
    fn mic_media_does_not_publish_full_radio_state_at_frame_cadence() {
        let mic_effects = command_effects(&TciCommand::MicAudioFrame(crate::tci::TciMicFrame {
            sample_rate_hz: 48_000,
            channels: 1,
            sequence: 1,
            received_at: Instant::now(),
            samples: vec![0.0; 960],
        }));
        assert!(!mic_effects.radio_state_dirty);

        let control_effects = command_effects(&TciCommand::SetRxAttenuation(10));
        assert!(control_effects.radio_state_dirty);

        let mut mixed_effects = mic_effects;
        mixed_effects.merge(control_effects);
        assert!(mixed_effects.radio_state_dirty);
    }

    #[test]
    fn watchdog_heartbeat_uses_dedicated_pong_without_full_state_publication() {
        let effects = command_effects(&TciCommand::SaturnPing {
            client_id: 7,
            nonce: "test-nonce".to_string(),
            sent_at: "123.456".to_string(),
        });
        assert!(!effects.radio_state_dirty);
        assert!(
            !command_effects(&TciCommand::RequestRadioState { client_id: 8 }).radio_state_dirty
        );

        // The one-second S-meter request remains the periodic full-state
        // convergence point.
        assert!(command_effects(&TciCommand::RequestSmeter).radio_state_dirty);
    }
}
