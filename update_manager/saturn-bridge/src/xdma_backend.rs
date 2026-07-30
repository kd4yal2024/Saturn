//! Operational, receive-only direct-XDMA radio backend.
//!
//! This runtime deliberately exposes no H2C/DUC path. It owns the proven
//! Phase 2 DDC capture, feeds the existing TCI and WDSP receive pipelines, and
//! keeps every transmit request in the receive-safe state.

use crate::config::BridgeConfig;
use crate::radio_model::{DemodMode, NoiseReductionMode, RadioModel, TxPhase};
use crate::sync_ext::MutexExt;
use crate::tci::{TciCommand, TciFrontend};
use crate::wdsp::{normalize_audio_frame_float_count, WdspRxEngine, WDSP_AUDIO_RATE_HZ};
use crate::xdma::{SaturnIdentity, XdmaError};
use crate::xdma_rx::{OperationalRxSession, DIRECT_DDC_INDEX, DIRECT_DDC_SAMPLE_RATE_KHZ};
use crate::xdma_telemetry::{record_runtime_readiness, TelemetryValue};
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
struct CommandEffects {
    dsp_dirty: bool,
    tuning_dirty: bool,
    tx_state_dirty: bool,
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
    }
    let (tci, command_rx) = TciFrontend::bind(&config, radio_model.clone())?;
    let tci = Arc::new(tci);
    let mut wdsp = {
        let model = radio_model.lock_unpoisoned();
        WdspRxEngine::new(&model)?
    };
    let mut rx = OperationalRxSession::open(config.ddc0_frequency_hz)?;
    let identity = rx.identity().clone();
    let mut iq_samples = Vec::with_capacity(8_192);
    let mut readiness_state = "starting";
    let mut last_readiness = Instant::now() - READINESS_PERIOD;
    let mut last_status = Instant::now();

    write_readiness(ready_path, readiness_state, &identity, &rx)?;
    println!(
        "saturn-bridge: direct XDMA RX backend starting product={} pcb={} firmware={}.{} ddc={} adc=ADC1 frequency={}Hz rate={}kHz TCI={} TX=disabled",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        DIRECT_DDC_INDEX,
        rx.frequency_hz(),
        DIRECT_DDC_SAMPLE_RATE_KHZ,
        config.tci_bind_addr,
    );

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
            )?);
        }
        if command_count != 0 {
            let mut model = radio_model.lock_unpoisoned();
            model.desired.tx_enabled = false;
            model.desired.tx_phase = TxPhase::Rx;
            if command_effects.dsp_dirty {
                wdsp.sync_model(&model)?;
            }
            if command_effects.tuning_dirty {
                tci.publish_tuning_state(&model);
            }
            if command_effects.tx_state_dirty {
                tci.publish_tx_state(&model);
            }
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

        if readiness_state == "starting"
            && rx.stats().dma_reads >= READY_DMA_READS
            && rx.stats().samples >= READY_IQ_PAIRS
        {
            readiness_state = "ready";
            write_readiness(ready_path, readiness_state, &identity, &rx)?;
            println!(
                "saturn-bridge: direct XDMA RX backend ready dma_reads={} iq_pairs={} rf_safe=1",
                rx.stats().dma_reads,
                rx.stats().samples
            );
        }
        if last_readiness.elapsed() >= READINESS_PERIOD {
            write_readiness(ready_path, readiness_state, &identity, &rx)?;
            last_readiness = Instant::now();
        }
        if last_status.elapsed() >= STATUS_PERIOD {
            let stats = rx.stats();
            println!(
                "saturn-bridge: xdma_rx status={} frequency_hz={} dma_reads={} dma_bytes={} iq_pairs={} fifo_hwm={} header_resync={} header_errors={} fifo_faults={}",
                readiness_state,
                rx.frequency_hz(),
                stats.dma_reads,
                stats.dma_bytes,
                stats.samples,
                stats.fifo_depth_hwm,
                stats.header_resyncs,
                stats.header_errors,
                stats.fifo_overflows + stats.fifo_over_threshold + stats.fifo_underflows,
            );
            last_status = Instant::now();
        }
        if !did_work {
            thread::sleep(IDLE_POLL);
        }
    }

    rx.stop()?;
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
        "saturn-bridge: direct XDMA RX backend stopped; DDC disabled and receive-safe cleanup verified"
    );
    Ok(())
}

fn handle_command(
    command: TciCommand,
    radio_model: &Arc<Mutex<RadioModel>>,
    tci: &TciFrontend,
    wdsp: &mut WdspRxEngine,
    rx: &mut OperationalRxSession,
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
        TciCommand::SetAudioSampleRate(rate_hz) => {
            if rate_hz != WDSP_AUDIO_RATE_HZ {
                eprintln!(
                    "saturn-bridge: direct XDMA audio rate is fixed at {WDSP_AUDIO_RATE_HZ} Hz; refusing {rate_hz} Hz"
                );
            }
            wdsp.reset_audio_packetizer();
            tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
        }
        TciCommand::SetAudioFrameSamples(samples) => {
            let normalized = normalize_audio_frame_float_count(samples as usize);
            wdsp.set_audio_frame_float_count(normalized);
        }
        TciCommand::SetAudioChannels(channels) => {
            if channels != 2 {
                eprintln!("saturn-bridge: direct XDMA audio output remains stereo");
            }
        }
        TciCommand::ClientConnected => model.desired.running = true,
        TciCommand::ClientDisconnected => {
            model.desired.tx_enabled = false;
            model.desired.tx_phase = TxPhase::Rx;
        }
        TciCommand::SetTxEnabled(enabled) => {
            model.desired.tx_enabled = false;
            model.desired.tx_phase = TxPhase::Rx;
            if enabled {
                eprintln!(
                    "saturn-bridge: refusing TX request: operational direct XDMA backend is RX-only"
                );
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
        // TX preference, microphone, and PureSignal commands cannot reach
        // hardware on this RX-only backend.
        _ => {}
    }
    Ok(effects)
}

fn write_readiness(
    path: &Path,
    status: &str,
    identity: &SaturnIdentity,
    rx: &OperationalRxSession,
) -> Result<(), XdmaError> {
    rx.verify_receive_safe()?;
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
                "header_resync",
                TelemetryValue::number(stats.header_resyncs),
            ),
            ("header_errors", TelemetryValue::number(stats.header_errors)),
            ("rf_safe", TelemetryValue::boolean(true)),
            ("tx_capable", TelemetryValue::boolean(false)),
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
