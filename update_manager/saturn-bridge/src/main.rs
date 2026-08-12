mod config;
mod p2;
mod radio_model;
mod rx_thread;
mod sync_ext;
mod tci;
mod tx_codec;
mod tx_thread;
mod wdsp;
mod xdma;
mod xdma_audio;
mod xdma_backend;
mod xdma_duc;
mod xdma_rx;
mod xdma_telemetry;
mod xdma_tx;
mod xdma_tx_radio;

use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use config::{BridgeConfig, RadioBackend};
use p2::session::P2Session;
use radio_model::{DemodMode, PureSignalState, RadioModel, TxPhase};
use rx_thread::{RxCommand, RxEvent, RxStats};
use sync_ext::MutexExt;
use tci::{TciCommand, TciFrontend};
use tx_thread::{TxCommand, TxDiagnostics, TxEvent};
use wdsp::{normalize_audio_frame_float_count, WdspRxEngine, WDSP_AUDIO_RATE_HZ};

const MAX_TCI_COMMANDS_PER_LOOP: usize = 128;
const DEFAULT_REMOTE_TX_MAX_WATTS: u8 = 100;
const DEFAULT_REMOTE_TX_POWER_TRIP_WATTS: f32 = 110.0;
const DEFAULT_TCI_CLIENT_RELEASE_GRACE_MS: u64 = 3_000;
const MAX_TCI_CLIENT_RELEASE_GRACE_MS: u64 = 30_000;
// Match the TX mic-recency gate: short phone/VPN scheduler stalls can briefly
// exceed 250 ms even while the transmission remains intelligible, but a mic path
// that stays stale past this window is no longer safe to keep keyed.
const TX_UPLINK_LATE_LIMIT: Duration = Duration::from_millis(500);
const TX_UPLINK_LATE_SUSTAIN: Duration = Duration::from_millis(100);
// Bridge force-RX if no operator control message arrives
// within this window while keyed. Independent safety net for the case where
// browser PTT-release bytes are stuck behind queued media on the single
// WebSocket. Paired split sessions use a wider limit because release
// and disconnect are already isolated on the control lane; cannot be disabled.
const TX_CONTROL_WATCHDOG_LIMIT: Duration = Duration::from_millis(500);
// Paired control/media sessions have an independent control socket,
// so a delayed mobile-browser heartbeat is less dangerous than it was on the
// old single media/control socket. Keep the watchdog, but avoid false RX faults
// from phone/Tailscale timer jitter while mic frames still flow on media.
const TX_CONTROL_WATCHDOG_SPLIT_LIMIT: Duration = Duration::from_millis(1500);

fn parse_remote_tx_max_watts(value: Option<&str>) -> u8 {
    value
        .and_then(|value| value.trim().parse::<u8>().ok())
        .map(|value| value.clamp(1, 100))
        .unwrap_or(DEFAULT_REMOTE_TX_MAX_WATTS)
}

fn remote_tx_max_watts() -> u8 {
    let explicit = env::var("SATURN_REMOTE_TX_MAX_WATTS").ok();
    let legacy = env::var("SATURN_REMOTE_TX_MAX_DRIVE").ok();
    parse_remote_tx_max_watts(explicit.as_deref().or(legacy.as_deref()))
}

fn parse_remote_tx_power_trip_watts(value: Option<&str>) -> f32 {
    value
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(5.0, 200.0))
        .unwrap_or(DEFAULT_REMOTE_TX_POWER_TRIP_WATTS)
}

fn remote_tx_power_trip_watts() -> f32 {
    parse_remote_tx_power_trip_watts(
        env::var("SATURN_REMOTE_TX_POWER_TRIP_WATTS")
            .ok()
            .as_deref(),
    )
}

fn parse_tci_client_release_grace(value: Option<&str>) -> Duration {
    let ms = value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.min(MAX_TCI_CLIENT_RELEASE_GRACE_MS))
        .unwrap_or(DEFAULT_TCI_CLIENT_RELEASE_GRACE_MS);
    Duration::from_millis(ms)
}

fn tci_client_release_grace() -> Duration {
    parse_tci_client_release_grace(
        env::var("SATURN_TCI_CLIENT_RELEASE_GRACE_MS")
            .ok()
            .as_deref(),
    )
}

/// Convert a Saturn G2 raw power ADC reading to watts.
/// Same calibration as the TCI telemetry path.
fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (raw as i32 - offset).max(0) as f32;
    let v = (corrected / 4095.0) * 5.0;
    ((v * v) / 0.12) * scale
}

fn bool01(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

fn tx_media_priority_active(tx_requested: bool, model: &RadioModel) -> bool {
    tx_requested || model.desired.tx_phase != TxPhase::Rx || model.desired.tx_enabled
}

fn update_tx_uplink_late_detector(
    on_air: bool,
    last_mic_at: Option<Instant>,
    now: Instant,
    late_since: &mut Option<Instant>,
) -> Option<Duration> {
    if !on_air {
        *late_since = None;
        return None;
    }

    let Some(last_mic_at) = last_mic_at else {
        *late_since = None;
        return None;
    };

    let age = now.saturating_duration_since(last_mic_at);
    if age <= TX_UPLINK_LATE_LIMIT {
        *late_since = None;
        return None;
    }

    let since = late_since.get_or_insert(now);
    if now.saturating_duration_since(*since) >= TX_UPLINK_LATE_SUSTAIN {
        Some(age)
    } else {
        None
    }
}

// Detect prolonged absence of operator control messages
// while keyed. Returns Some(silence) when the bridge has been keyed and
// silent of control input for longer than the supplied watchdog limit.
fn update_tx_control_watchdog(
    on_air: bool,
    last_control_at: Option<Instant>,
    now: Instant,
    limit: Duration,
) -> Option<Duration> {
    if !on_air {
        return None;
    }
    let last_control_at = last_control_at?;
    let silence = now.saturating_duration_since(last_control_at);
    if silence > limit {
        Some(silence)
    } else {
        None
    }
}

fn tx_control_watchdog_limit(split_paired: bool) -> Duration {
    if split_paired {
        TX_CONTROL_WATCHDOG_SPLIT_LIMIT
    } else {
        TX_CONTROL_WATCHDOG_LIMIT
    }
}

fn ms_field(value: Option<u64>) -> String {
    value
        .map(|ms| ms.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_tx_diag(diag: Option<&TxDiagnostics>) -> String {
    match diag {
        Some(diag) => format!(
            "tx_diag state={} rf={} armed_ms={} first_mic_ms={} first_iq_ms={} first_keyable_iq_ms={} mic_recent={} keyed_ms={} mic_frames={} duc_packets={} in_pk={:.4} out_pk={:.4} mic_pk_db={:.1} comp_pk_db={:.1} comp_avg_db={:.1} alc_pk_db={:.1} alc_avg_db={:.1} alc_gain_db={:.1} out_pk_db={:.1} in_samples={} out_pairs={} pending_mic={} pending_iq={}",
            diag.state,
            bool01(diag.rf_enabled),
            diag.armed_ms,
            ms_field(diag.first_mic_ms),
            ms_field(diag.first_iq_ms),
            ms_field(diag.first_keyable_iq_ms),
            bool01(diag.mic_recent),
            ms_field(diag.keyed_ms),
            diag.mic_frames,
            diag.duc_packets,
            diag.input_peak,
            diag.output_peak,
            diag.mic_peak_db,
            diag.comp_peak_db,
            diag.comp_avg_db,
            diag.alc_peak_db,
            diag.alc_avg_db,
            diag.alc_gain_db,
            diag.out_peak_db,
            diag.total_input_samples,
            diag.total_output_pairs,
            diag.pending_mic_floats,
            diag.pending_iq_floats
        ),
        None => "tx_diag state=idle".to_string(),
    }
}

fn clamp_client_ddc0_sample_rate_khz(rate_hz: u32, max_rate_khz: u16) -> u16 {
    let requested_rate_khz = (rate_hz / 1000).clamp(48, u16::MAX as u32) as u16;
    requested_rate_khz.min(max_rate_khz.max(48))
}

fn finish_xdma_probe_command(
    phase: u8,
    probe: &str,
    result: Result<(), xdma::XdmaError>,
) -> Result<(), Box<dyn Error>> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            xdma_telemetry::record_probe_failure_if_unrecorded(phase, probe, &error.to_string());
            Err(Box::new(error))
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    if env::args().skip(1).any(|arg| arg == "--xdma-probe") {
        return finish_xdma_probe_command(1, "identity", xdma::run_phase1_probe());
    }
    if env::args().skip(1).any(|arg| arg == "--xdma-rx-probe") {
        return finish_xdma_probe_command(2, "rx-ddc", xdma_rx::run_phase2_rx_probe());
    }
    if env::args().skip(1).any(|arg| arg == "--xdma-audio-probe") {
        return finish_xdma_probe_command(3, "codec-audio", xdma_audio::run_phase3_audio_probe());
    }
    if env::args().skip(1).any(|arg| arg == "--xdma-duc-probe") {
        return finish_xdma_probe_command(4, "duc-performance", xdma_duc::run_phase4_duc_probe());
    }
    if env::args().skip(1).any(|arg| arg == "--xdma-tx-preflight") {
        return finish_xdma_probe_command(5, "tx-preflight", xdma_tx::run_phase5_tx_preflight());
    }
    if env::args().skip(1).any(|arg| arg == "--xdma-tx-probe") {
        return finish_xdma_probe_command(5, "guarded-tx", xdma_tx::run_phase5_tx_probe());
    }

    let backend = RadioBackend::from_env()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let mut config = BridgeConfig::from_env();
    if backend == RadioBackend::Xdma {
        config.rx_ddc_index = xdma_rx::DIRECT_DDC_INDEX as u8;
        config.ddc0_adc = 0;
        config.ddc0_sample_rate_khz = xdma_rx::DIRECT_DDC_SAMPLE_RATE_KHZ as u16;
        config.max_client_ddc0_sample_rate_khz = xdma_rx::DIRECT_DDC_SAMPLE_RATE_KHZ as u16;
        config.ddc0_sample_size_bits = 24;
        return xdma_backend::run(config);
    }
    let radio_model = Arc::new(Mutex::new(RadioModel::new(
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
    let session = Arc::new(P2Session::bind(config.clone())?);
    let (tci, tci_command_rx) = TciFrontend::bind(&config, radio_model.clone())?;
    let tci = Arc::new(tci);
    let wdsp = {
        let model = radio_model.lock_unpoisoned();
        WdspRxEngine::new(&model)?
    };
    let stop_flag = Arc::new(AtomicBool::new(false));
    let remote_tx_rf_enabled = config.remote_tx_rf_enabled;
    let allow_rf_disabled_two_tone = config.allow_rf_disabled_two_tone;
    let remote_tx_max_watts = remote_tx_max_watts();
    let remote_tx_power_trip_watts = remote_tx_power_trip_watts();
    let tci_client_release_grace = tci_client_release_grace();
    let tx_power_meter_scale = config.tx_power_meter_scale;
    {
        let mut model = radio_model.lock_unpoisoned();
        model.desired.tx_drive = model.desired.tx_drive.clamp(1, remote_tx_max_watts);
    }
    let (tx_cmd_tx, tx_cmd_rx) = mpsc::channel();
    let (tx_event_tx, tx_event_rx) = mpsc::channel();
    let (rx_cmd_tx, rx_cmd_rx) = mpsc::channel();
    let (rx_event_tx, rx_event_rx) = mpsc::channel();
    let rx_stats = Arc::new(RxStats::default());
    let tx_requested = Arc::new(AtomicBool::new(false));

    println!(
        "saturn-bridge: binding {} -> radio {} | TCI {} | remote TX RF {} | remote TX max target={}W 100W-drive-byte={} power meter scale={:.4} power trip={:.1}W | TCI release grace={}ms | display fps cap={} | max client DDC0 rate={}k | RX audio transport={}Hz/{}ch",
        session.client_bind_addr(),
        config.radio_command_addr,
        config.tci_bind_addr,
        if remote_tx_rf_enabled {
            "enabled"
        } else {
            "disabled"
        },
        remote_tx_max_watts,
        config.remote_tx_100w_drive_byte,
        tx_power_meter_scale,
        remote_tx_power_trip_watts,
        tci_client_release_grace.as_millis(),
        config.display_frame_limit_hz,
        config.max_client_ddc0_sample_rate_khz,
        config.rx_audio_transport_rate_hz,
        config.rx_audio_transport_channels
    );

    let hp_thread = session.spawn_high_priority_loop(radio_model.clone(), stop_flag.clone())?;
    let tx_thread = tx_thread::spawn(
        session.clone(),
        radio_model.clone(),
        tx_cmd_rx,
        tx_event_tx,
        stop_flag.clone(),
    );
    let rx_thread = rx_thread::spawn(
        session.clone(),
        radio_model.clone(),
        tci.clone(),
        wdsp,
        rx_cmd_rx,
        rx_event_tx,
        tx_cmd_tx.clone(),
        tx_requested.clone(),
        rx_stats.clone(),
        stop_flag.clone(),
    );

    let mut last_status = Instant::now();
    let mut last_duc_specific = Instant::now();
    let mut status_hp_packets = 0u64;
    let mut status_tci_mic_frames = 0u64;
    let mut status_tci_mic_samples = 0u64;
    let mut latest_tx_diag: Option<TxDiagnostics> = None;
    let mut controller_release_deadline: Option<Instant> = None;
    let mut controller_owned = false;
    let mut last_operator_mic_at: Option<Instant> = None;
    let mut tx_uplink_late_since: Option<Instant> = None;
    let mut tx_uplink_fault_active = false;
    let mut tx_control_watchdog_fault_active = false;
    loop {
        let mut did_work = false;
        let mut needs_bootstrap = false;
        let mut needs_stop = false;
        let mut needs_duc_specific = false;
        let mut needs_high_priority = false;

        for _ in 0..MAX_TCI_COMMANDS_PER_LOOP {
            let Ok(command) = tci_command_rx.try_recv() else {
                break;
            };
            did_work = true;
            let mut model = radio_model.lock_unpoisoned();
            let mut reconfigure_ddc = false;

            match command {
                TciCommand::SetVfoA(freq_hz) => {
                    model.desired.vfo_a_hz = freq_hz;
                    model.desired.iq_center_hz = freq_hz;
                    model.desired.tx_frequency_hz = freq_hz;
                    needs_high_priority = true;
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetVfoB(freq_hz) => {
                    model.desired.vfo_b_hz = freq_hz;
                }
                TciCommand::SetIqCenter(freq_hz) => {
                    model.desired.iq_center_hz = freq_hz;
                    needs_high_priority = true;
                }
                TciCommand::SetMode(mode) => {
                    let mode = if mode == DemodMode::Wfm
                        && (!wdsp::wbfm_supported() || config.max_client_ddc0_sample_rate_khz < 192)
                    {
                        eprintln!(
                            "saturn-bridge: WFM requested but unavailable (wdsp={} max_ddc={}k); using FM",
                            wdsp::wbfm_supported(),
                            config.max_client_ddc0_sample_rate_khz
                        );
                        DemodMode::Fm
                    } else {
                        mode
                    };
                    model.desired.mode = mode;
                    if mode == DemodMode::Wfm && model.desired.ddc0_sample_rate_khz != 192 {
                        model.desired.ddc0_sample_rate_khz = 192;
                        reconfigure_ddc = true;
                    }
                    let (rx_low_hz, rx_high_hz) = mode.default_filter_band();
                    model.desired.filter_low_hz = rx_low_hz;
                    model.desired.filter_high_hz = rx_high_hz;
                    let (tx_low_hz, tx_high_hz) = mode.default_tx_filter_band();
                    model.desired.tx_filter_low_hz = tx_low_hz;
                    model.desired.tx_filter_high_hz = tx_high_hz;
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetFilterBand { low_hz, high_hz } => {
                    model.desired.filter_low_hz = low_hz;
                    model.desired.filter_high_hz = high_hz;
                }
                TciCommand::SetRxAdc(adc) => {
                    let effective_adc = adc.min(2);
                    if model.desired.ddc0_adc != effective_adc {
                        println!(
                            "saturn-bridge: RX ADC change requested ADC{} -> DDC{} rate={}k bits={}",
                            effective_adc + 1,
                            model.desired.rx_ddc_index,
                            model.desired.ddc0_sample_rate_khz,
                            model.desired.ddc0_sample_size_bits
                        );
                    }
                    model.desired.ddc0_adc = effective_adc;
                    reconfigure_ddc = true;
                }
                TciCommand::SetRxAntenna(antenna) => {
                    model.desired.rx_antenna = antenna.clamp(1, 3);
                    needs_high_priority = true;
                }
                TciCommand::SetRxVolume(volume_db) => {
                    model.desired.rx_volume_db = volume_db.clamp(-40.0, 12.0);
                }
                TciCommand::SetRxNoiseReductionMode(mode) => {
                    model.desired.rx_noise_reduction_mode = mode;
                }
                TciCommand::SetRxNoiseReductionEnabled(enabled) => {
                    model.desired.rx_noise_reduction_mode = if enabled {
                        radio_model::NoiseReductionMode::Nr1
                    } else {
                        radio_model::NoiseReductionMode::Off
                    };
                }
                TciCommand::SetRxNoiseReductionLevel(level) => {
                    model.desired.rx_noise_reduction_level = level.clamp(0.0, 100.0);
                }
                TciCommand::SetRxNr2GainMethod(method) => {
                    model.desired.rx_nr2_gain_method = method;
                }
                TciCommand::SetRxNr2NpeMethod(method) => {
                    model.desired.rx_nr2_npe_method = method;
                }
                TciCommand::SetRxNr2PostFilterEnabled(enabled) => {
                    model.desired.rx_nr2_post_filter_enabled = enabled;
                }
                TciCommand::SetRxWbfmDeemphasis(deemphasis) => {
                    model.desired.rx_wbfm_deemphasis = deemphasis;
                }
                TciCommand::SetRxAnrVals {
                    taps,
                    delay,
                    gain,
                    leakage,
                } => {
                    if let Some(taps) = taps {
                        model.desired.rx_anr_taps = taps.clamp(1, 128);
                    }
                    if let Some(delay) = delay {
                        model.desired.rx_anr_delay = delay.clamp(0, 127);
                    }
                    if let Some(gain) = gain {
                        model.desired.rx_anr_gain = gain.clamp(0.0, 1.0);
                    }
                    if let Some(leakage) = leakage {
                        model.desired.rx_anr_leakage = leakage.clamp(0.0, 1.0);
                    }
                }
                TciCommand::SetIqSampleRate(rate_hz) => {
                    let requested_rate_khz = (rate_hz / 1000).clamp(48, u16::MAX as u32) as u16;
                    let rate_khz = if model.desired.mode == DemodMode::Wfm {
                        192
                    } else {
                        clamp_client_ddc0_sample_rate_khz(
                            rate_hz,
                            config.max_client_ddc0_sample_rate_khz,
                        )
                    };
                    if rate_khz != requested_rate_khz {
                        eprintln!(
                            "saturn-bridge: clamped client DDC0 sample rate request {}k -> {}k",
                            requested_rate_khz, rate_khz
                        );
                    }
                    model.desired.ddc0_sample_rate_khz = rate_khz;
                    reconfigure_ddc = true;
                }
                TciCommand::SetIqStreaming => {}
                TciCommand::RequestSmeter => {}
                TciCommand::SaturnPing {
                    client_id,
                    nonce,
                    sent_at,
                } => {
                    tci.publish_saturn_pong(client_id, &nonce, &sent_at);
                }
                TciCommand::SplitSessionOpen {
                    client_id,
                    session_id,
                    role,
                } => {
                    println!(
                        "saturn-bridge: split client {client_id} opened session {session_id} as {:?}",
                        role
                    );
                }
                TciCommand::SplitSessionLane {
                    client_id,
                    session_id,
                    lane,
                } => {
                    println!(
                        "saturn-bridge: split client {client_id} marked {:?} lane for session {session_id}",
                        lane
                    );
                }
                TciCommand::SetAudioStreaming(enabled) => {
                    if enabled {
                        let _ = rx_cmd_tx.send(RxCommand::ResetAudioPacketizer);
                        tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
                    } else {
                        tci.publish_audio_stopped();
                    }
                }
                TciCommand::SetAudioSampleRate(_rate_hz) => {
                    let _ = rx_cmd_tx.send(RxCommand::ResetAudioPacketizer);
                    tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
                }
                TciCommand::SetAudioFrameSamples(sample_count) => {
                    let normalized =
                        normalize_audio_frame_float_count(sample_count as usize) as u32;
                    let _ = rx_cmd_tx.send(RxCommand::SetAudioFrameFloatCount(normalized as usize));
                    if sample_count != normalized {
                        eprintln!(
                            "saturn-bridge: requested audio frame size {} float32 samples, using {}",
                            sample_count,
                            normalized
                        );
                    }
                }
                TciCommand::SetAudioChannels(_channels) => {
                    tci.publish_audio_started(WDSP_AUDIO_RATE_HZ);
                }
                TciCommand::ClientConnected => {
                    controller_release_deadline = None;
                    if controller_owned {
                        println!("saturn-bridge: TCI client active — retaining P2 controller role");
                    } else {
                        model.desired.running = false;
                        needs_bootstrap = true;
                        println!(
                            "saturn-bridge: TCI client active — requesting P2 controller role"
                        );
                    }
                }
                TciCommand::ClientDisconnected => {
                    tx_requested.store(false, Ordering::Relaxed);
                    last_operator_mic_at = None;
                    tx_uplink_late_since = None;
                    tx_uplink_fault_active = false;
                    tx_control_watchdog_fault_active = false;
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    needs_duc_specific = true;
                    needs_high_priority = true;
                    model.desired.two_tone_enabled = false;
                    if !controller_owned {
                        model.desired.running = false;
                        println!(
                            "saturn-bridge: no TCI clients — no P2 controller role to release"
                        );
                    } else if tci_client_release_grace.is_zero() {
                        model.desired.running = false;
                        needs_stop = true;
                        println!("saturn-bridge: no TCI clients — releasing P2 controller role");
                    } else {
                        controller_release_deadline =
                            Some(Instant::now() + tci_client_release_grace);
                        println!(
                            "saturn-bridge: no TCI clients — holding P2 controller role for {}ms",
                            tci_client_release_grace.as_millis()
                        );
                    }
                }
                TciCommand::SetTxEnabled(enabled) => {
                    if enabled && !controller_owned {
                        eprintln!("saturn-bridge: refusing TX without P2 controller ownership");
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                    } else if enabled && model.desired.mode == DemodMode::Wfm {
                        eprintln!("saturn-bridge: refusing TX while WFM receive mode is active");
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                    } else if enabled {
                        if !tx_requested.load(Ordering::Relaxed) {
                            if model.desired.tx_drive > remote_tx_max_watts {
                                eprintln!(
                                    "saturn-bridge: clamping remote TX target {}W -> {}W before arm",
                                    model.desired.tx_drive, remote_tx_max_watts
                                );
                                model.desired.tx_drive = remote_tx_max_watts;
                                needs_high_priority = true;
                            }
                            tx_requested.store(true, Ordering::Relaxed);
                            last_operator_mic_at = None;
                            tx_uplink_late_since = None;
                            tx_uplink_fault_active = false;
                            tx_control_watchdog_fault_active = false;
                            tci.clear_split_release_window();
                            model.desired.tx_enabled = false;
                            model.desired.tx_phase = TxPhase::Armed;
                            tci.set_tx_media_priority_active(true);
                            let _ = rx_cmd_tx.send(RxCommand::ResetStreamBuffers);
                            let _ = tx_cmd_tx.send(TxCommand::Arm {
                                rf_enabled: remote_tx_rf_enabled,
                            });
                            needs_duc_specific = true;
                            println!(
                                "saturn-bridge: TX armed; waiting for DUC IQ audio{}",
                                if remote_tx_rf_enabled {
                                    ""
                                } else {
                                    " (RF disabled)"
                                }
                            );
                        }
                    } else if tx_requested.load(Ordering::Relaxed) || model.desired.tx_enabled {
                        tci.mark_split_released(Instant::now());
                        tx_requested.store(false, Ordering::Relaxed);
                        last_operator_mic_at = None;
                        tx_uplink_late_since = None;
                        tx_uplink_fault_active = false;
                        tx_control_watchdog_fault_active = false;
                        let _ = rx_cmd_tx.send(RxCommand::ResetStreamBuffers);
                        let _ = tx_cmd_tx.send(TxCommand::Disarm);
                        needs_duc_specific = true;
                        reconfigure_ddc = true;
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                        needs_high_priority = true;
                        println!("saturn-bridge: TX state -> OFF");
                    }
                }
                TciCommand::SetNoiseBlankerMode(mode) => {
                    model.desired.nb_mode = mode;
                }
                TciCommand::SetNoiseBlankerThreshold(thresh) => {
                    model.desired.nb_threshold = thresh;
                }
                TciCommand::SetAnfEnabled(enabled) => {
                    model.desired.anf_enabled = enabled;
                }
                TciCommand::SetRxAnfVals {
                    taps,
                    delay,
                    gain,
                    leakage,
                } => {
                    if let Some(taps) = taps {
                        model.desired.rx_anf_taps = taps.clamp(1, 128);
                    }
                    if let Some(delay) = delay {
                        model.desired.rx_anf_delay = delay.clamp(0, 127);
                    }
                    if let Some(gain) = gain {
                        model.desired.rx_anf_gain = gain.clamp(0.0, 1.0);
                    }
                    if let Some(leakage) = leakage {
                        model.desired.rx_anf_leakage = leakage.clamp(0.0, 1.0);
                    }
                }
                TciCommand::SetAgcMode(mode) => {
                    model.desired.agc_mode = mode;
                }
                TciCommand::SetAgcGain(gain) => {
                    model.desired.agc_gain = gain.clamp(0.0, 100.0);
                }
                TciCommand::SetTxDrive(drive) => {
                    let requested = drive.clamp(1, 100);
                    let clamped = requested.min(remote_tx_max_watts);
                    if clamped != requested {
                        eprintln!(
                            "saturn-bridge: clamping remote TX target request {}W -> {}W",
                            requested, clamped
                        );
                    }
                    model.desired.tx_drive = clamped;
                    needs_high_priority = true;
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
                TciCommand::SetRxEqEnabled(enabled) => {
                    model.desired.rx_eq_enabled = enabled;
                }
                TciCommand::SetRxEqBand { band, gain_db } => {
                    model.desired.rx_eq_bands[band] = gain_db.clamp(-20, 20);
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
                    model.desired.cfc_bands[band - 1] = gain_db.clamp(0.0, 20.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
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
                    model.desired.tx_phase_rotator_corner_hz = corner_hz.clamp(50.0, 2000.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetPureSignalEnabled(enabled) => {
                    if tx_requested.load(Ordering::Relaxed) || model.desired.tx_phase != TxPhase::Rx
                    {
                        eprintln!(
                            "saturn-bridge: refusing PureSignal enable change while TX is armed"
                        );
                    } else {
                        model.desired.pure_signal_enabled = enabled;
                        model.observed.pure_signal_state = if enabled {
                            PureSignalState::Waiting
                        } else {
                            PureSignalState::Off
                        };
                        model.observed.pure_signal_correcting = false;
                        needs_duc_specific = true;
                        needs_high_priority = true;
                        let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                    }
                }
                TciCommand::SetPureSignalAutoAttenuate(enabled) => {
                    if tx_requested.load(Ordering::Relaxed) || model.desired.tx_phase != TxPhase::Rx
                    {
                        eprintln!(
                            "saturn-bridge: refusing PureSignal auto-attenuation change while TX is armed"
                        );
                    } else {
                        model.desired.pure_signal_auto_attenuate = enabled;
                    }
                }
                TciCommand::SetPureSignalAttenuation(attenuation_db) => {
                    if tx_requested.load(Ordering::Relaxed) || model.desired.tx_phase != TxPhase::Rx
                    {
                        eprintln!(
                            "saturn-bridge: refusing PureSignal attenuation change while TX is armed"
                        );
                    } else {
                        model.desired.pure_signal_attenuation_db = attenuation_db.min(31);
                        needs_duc_specific = true;
                    }
                }
                TciCommand::ResetPureSignal => {
                    if model.desired.pure_signal_enabled {
                        let _ = tx_cmd_tx.send(TxCommand::PureSignalReset);
                    }
                }
                TciCommand::SetTxTwoToneTest(enabled) => {
                    if enabled && !remote_tx_rf_enabled && !allow_rf_disabled_two_tone {
                        model.desired.two_tone_enabled = false;
                        eprintln!(
                            "saturn-bridge: two-tone request refused; RF TX is disabled for safety"
                        );
                        continue;
                    }
                    model.desired.two_tone_enabled = enabled;
                    println!(
                        "saturn-bridge: two-tone test -> {}",
                        if enabled && !remote_tx_rf_enabled {
                            "ON (700/1900 Hz, RF disabled diagnostic)"
                        } else if enabled {
                            "ON (700/1900 Hz)"
                        } else {
                            "OFF"
                        }
                    );
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxTwoToneFreq1(freq_hz) => {
                    model.desired.tx_two_tone_freq1_hz = freq_hz.clamp(10.0, 10_000.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxTwoToneFreq2(freq_hz) => {
                    model.desired.tx_two_tone_freq2_hz = freq_hz.clamp(10.0, 10_000.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxTwoToneLevelDb(level_db) => {
                    model.desired.tx_two_tone_level_db = level_db.clamp(-40.0, 0.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxTwoToneInvertLsb(enabled) => {
                    model.desired.tx_two_tone_invert_lsb = enabled;
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxTwoToneDelayMs(delay_ms) => {
                    model.desired.tx_two_tone_delay_ms = delay_ms.min(2_000);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxNoiseGateEnabled(enabled) => {
                    model.desired.tx_noise_gate_enabled = enabled;
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxNoiseGateThreshold(thresh_db) => {
                    model.desired.tx_noise_gate_threshold_db = thresh_db.clamp(-80.0, 0.0);
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetRxFftSize(size) => {
                    let clamped = size.clamp(1024, 262144);
                    model.desired.rx_fft_size = 1 << (31 - clamped.leading_zeros());
                }
                TciCommand::SetRxLowLatency(enabled) => {
                    model.desired.rx_low_latency = enabled;
                }
                TciCommand::SetTxFftSize(size) => {
                    let clamped = size.clamp(1024, 262144);
                    model.desired.tx_fft_size = 1 << (31 - clamped.leading_zeros());
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::SetTxLowLatency(enabled) => {
                    model.desired.tx_low_latency = enabled;
                    let _ = tx_cmd_tx.send(TxCommand::ModelChanged);
                }
                TciCommand::MicAudioFrame(frame) => {
                    if !tx_requested.load(Ordering::Relaxed) {
                        continue;
                    }
                    last_operator_mic_at = Some(frame.received_at);
                    tx_uplink_late_since = None;
                    status_tci_mic_frames = status_tci_mic_frames.saturating_add(1);
                    status_tci_mic_samples =
                        status_tci_mic_samples.saturating_add(frame.samples.len() as u64);
                    let _ = tx_cmd_tx.send(TxCommand::MicAudio {
                        samples: frame.samples,
                        channels: frame.channels,
                        sample_rate_hz: frame.sample_rate_hz,
                    });
                }
            }

            if reconfigure_ddc && controller_owned {
                session.configure_rx_ddc(
                    model.desired.rx_ddc_index,
                    model.desired.ddc0_sample_rate_khz,
                    model.desired.ddc0_sample_size_bits,
                    model.desired.ddc0_adc,
                )?;
            }
            let _ = rx_cmd_tx.send(RxCommand::ModelChanged);
            tci.publish_radio_state(&model);
        }

        // bootstrap() acquires the radio_model lock internally, so it must be called
        // after the command loop has released it, never while holding it.
        if needs_bootstrap && !controller_owned {
            if session.bootstrap(&radio_model)? {
                controller_owned = true;
                let mut model = radio_model.lock_unpoisoned();
                model.desired.running = true;
                session.send_high_priority(&model)?;
                last_duc_specific = Instant::now();
                println!("saturn-bridge: P2 controller role acquired");
            } else {
                let mut model = radio_model.lock_unpoisoned();
                model.desired.running = false;
                model.desired.tx_enabled = false;
                model.desired.tx_phase = TxPhase::Rx;
                let _ = tx_cmd_tx.send(TxCommand::Disarm);
            }
            did_work = true;
        }
        if needs_duc_specific && controller_owned {
            let model = radio_model.lock_unpoisoned();
            session.send_duc_specific(&model)?;
            last_duc_specific = Instant::now();
            did_work = true;
        }
        if needs_stop && controller_owned {
            session.send_stop()?;
            controller_owned = false;
            did_work = true;
        }
        if needs_high_priority && controller_owned {
            let model = radio_model.lock_unpoisoned();
            session.send_high_priority(&model)?;
            did_work = true;
        }

        let running = { radio_model.lock_unpoisoned().desired.running };
        if running && last_duc_specific.elapsed() >= config.high_priority_period {
            let model = radio_model.lock_unpoisoned();
            session.send_duc_specific(&model)?;
            last_duc_specific = Instant::now();
            did_work = true;
        }

        if let Some(deadline) = controller_release_deadline {
            if Instant::now() >= deadline {
                controller_release_deadline = None;
                let mut model = radio_model.lock_unpoisoned();
                if model.desired.running {
                    model.desired.running = false;
                    session.send_stop()?;
                    controller_owned = false;
                    println!(
                        "saturn-bridge: TCI release grace expired — releasing P2 controller role"
                    );
                    did_work = true;
                }
            }
        }

        while let Ok(event) = tx_event_rx.try_recv() {
            did_work = true;
            match event {
                TxEvent::Keyed => {
                    let mut model = radio_model.lock_unpoisoned();
                    model.desired.tx_phase = TxPhase::Keyed;
                    if model.desired.pure_signal_enabled {
                        model.observed.pure_signal_state = PureSignalState::Waiting;
                        model.observed.pure_signal_correcting = false;
                    }
                    tci.publish_radio_state(&model);
                }
                TxEvent::Unkeyed => {
                    tci.mark_split_released(Instant::now());
                    tx_requested.store(false, Ordering::Relaxed);
                    last_operator_mic_at = None;
                    tx_uplink_late_since = None;
                    tx_uplink_fault_active = false;
                    tx_control_watchdog_fault_active = false;
                    latest_tx_diag = None;
                    let mut model = radio_model.lock_unpoisoned();
                    model.desired.tx_phase = TxPhase::Rx;
                    model.observed.pure_signal_state = if model.desired.pure_signal_enabled {
                        PureSignalState::Waiting
                    } else {
                        PureSignalState::Off
                    };
                    model.observed.pure_signal_correcting = false;
                    tci.publish_radio_state(&model);
                }
                TxEvent::TxIqFrame {
                    sample_rate_hz,
                    iq_samples,
                } => {
                    tci.publish_tx_iq_frame(sample_rate_hz, &iq_samples);
                }
                TxEvent::Diagnostics(diag) => {
                    latest_tx_diag = Some(diag);
                }
                TxEvent::PureSignalStatus(status) => {
                    let mut model = radio_model.lock_unpoisoned();
                    model.observed.pure_signal_state = status.state;
                    model.observed.pure_signal_feedback_level = status.feedback_level;
                    model.observed.pure_signal_calibration_count = status.calibration_count;
                    model.observed.pure_signal_correcting = status.correcting;
                    model.observed.pure_signal_max_tx = status.max_tx;
                    model.observed.pure_signal_feedback_packets = status.feedback_packets;
                    model.observed.pure_signal_feedback_gaps = status.feedback_gaps;
                    model.desired.pure_signal_attenuation_db = status.attenuation_db;
                    tci.publish_puresignal_status(&model);
                }
            }
        }

        while let Ok(event) = rx_event_rx.try_recv() {
            did_work = true;
            match event {
                RxEvent::HighPriority(packet) => {
                    status_hp_packets = status_hp_packets.saturating_add(1);
                    let forward_watts =
                        saturn_adc_to_watts(packet.forward_power, 32, tx_power_meter_scale);
                    let mut model = radio_model.lock_unpoisoned();
                    model.apply_high_priority(packet);
                    if remote_tx_rf_enabled
                        && model.desired.tx_enabled
                        && forward_watts > remote_tx_power_trip_watts
                    {
                        eprintln!(
                            "saturn-bridge: remote TX power trip {:.1}W > {:.1}W; forcing RX",
                            forward_watts, remote_tx_power_trip_watts
                        );
                        tci.publish_tx_power_trip(forward_watts, remote_tx_power_trip_watts);
                        tci.mark_split_released(Instant::now());
                        tx_requested.store(false, Ordering::Relaxed);
                        last_operator_mic_at = None;
                        tx_uplink_late_since = None;
                        tx_uplink_fault_active = false;
                        tx_control_watchdog_fault_active = false;
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                        let _ = tx_cmd_tx.send(TxCommand::Disarm);
                        session.send_high_priority(&model)?;
                        session.send_duc_specific(&model)?;
                    }
                    tci.publish_telemetry(&model);
                }
                RxEvent::Fatal(message) => {
                    return Err(message.into());
                }
            }
        }

        {
            let now = Instant::now();
            let mut model = radio_model.lock_unpoisoned();
            let on_air = model.desired.tx_phase == TxPhase::Keyed || model.desired.tx_enabled;
            let tx_media_priority =
                tx_media_priority_active(tx_requested.load(Ordering::Relaxed), &model);
            // The bridge owns the authoritative TX media-priority state.
            // Suppress RX media as soon as TX is requested/armed, before RF is
            // keyed, so mic prefill is not starved on constrained VPN links.
            // This keeps the no-sticky-browser-flag architecture from the
            // tx-on-air refactor while covering the armed/prefill window.
            tci.set_tx_media_priority_active(tx_media_priority);
            if !on_air {
                tx_uplink_late_since = None;
                tx_uplink_fault_active = false;
                tx_control_watchdog_fault_active = false;
            } else if !tx_uplink_fault_active {
                if let Some(age) = update_tx_uplink_late_detector(
                    on_air,
                    last_operator_mic_at,
                    now,
                    &mut tx_uplink_late_since,
                ) {
                    let age_ms = age.as_millis() as u64;
                    let limit_ms = TX_UPLINK_LATE_LIMIT.as_millis() as u64;
                    eprintln!(
                        "saturn-bridge: TX uplink late {}ms > {}ms for {}ms; forcing RX",
                        age_ms,
                        limit_ms,
                        TX_UPLINK_LATE_SUSTAIN.as_millis()
                    );
                    tci.publish_tx_uplink_late(age_ms, limit_ms);
                    tci.mark_split_released(now);
                    tx_requested.store(false, Ordering::Relaxed);
                    tx_uplink_fault_active = true;
                    last_operator_mic_at = None;
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    session.send_high_priority(&model)?;
                    session.send_duc_specific(&model)?;
                    last_duc_specific = now;
                    did_work = true;
                }
            }

            // Independent of mic freshness. This catches
            // single-WebSocket media backlog that delays operator release bytes.
            if on_air && !tx_control_watchdog_fault_active {
                let control_watchdog_limit =
                    tx_control_watchdog_limit(tci.has_split_paired_session());
                if let Some(silence) = update_tx_control_watchdog(
                    on_air,
                    tci.last_operator_control_at(),
                    now,
                    control_watchdog_limit,
                ) {
                    let silence_ms = silence.as_millis() as u64;
                    let limit_ms = control_watchdog_limit.as_millis() as u64;
                    eprintln!(
                        "saturn-bridge: TX control watchdog {}ms > {}ms; forcing RX",
                        silence_ms, limit_ms
                    );
                    tci.publish_tx_control_watchdog(silence_ms, limit_ms);
                    tci.mark_split_released(now);
                    tx_requested.store(false, Ordering::Relaxed);
                    tx_control_watchdog_fault_active = true;
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    session.send_high_priority(&model)?;
                    session.send_duc_specific(&model)?;
                    last_duc_specific = now;
                    did_work = true;
                }
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            let elapsed = last_status.elapsed().as_secs_f64().max(0.001);
            let client = tci.client_snapshot();
            println!(
                "saturn-bridge: diag hp_s={:.1} ddc_s={:.1} rx_audio_frames_s={:.1} rx_audio_samples_s={:.0} tci_mic_frames_s={:.1} tci_mic_samples_s={:.0} client={} connections={} connection_limit={} connection_rejected={} connection_hwm={} iq={} audio={} split_control={} split_media={} split_paired={} outbound_drops={} safety_p99_us={} control_p99_us={} control_replaced_s={} control_dropped_s={} control_q_hwm={} display_replaced_s={} display_dropped_s={} display_rate_limited_s={} audio_dropped_s={} audio_gaps={} audio_panic={} command_q={} command_q_hwm={} command_coalesced={} command_dropped={} command_mic_dropped={} send_blocked_ms={} out_hwm_bytes={} tcp_outq_hwm_bytes={} safety_depth_overflow={} tx_media_priority={} tx_uplink_degraded={} tx_mic_age_ms={} tx_mic_seq={} tx_mic_seq_gaps={} tx_mic_drops={} tx_uplink_buf={} tx_uplink_hwm={} tx_codec_decode_errors={} tx_codec_stale_drops={} tx_codec_release_flushes={} {}",
                status_hp_packets as f64 / elapsed,
                rx_stats.ddc_packets.swap(0, Ordering::Relaxed) as f64 / elapsed,
                rx_stats.audio_frames.swap(0, Ordering::Relaxed) as f64 / elapsed,
                rx_stats.audio_samples.swap(0, Ordering::Relaxed) as f64 / elapsed,
                status_tci_mic_frames as f64 / elapsed,
                status_tci_mic_samples as f64 / elapsed,
                bool01(client.active),
                client.active_connections,
                client.connection_limit,
                client.rejected_connections,
                client.connection_high_watermark,
                bool01(client.iq_stream_enabled),
                bool01(client.audio_stream_enabled),
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
                bool01(client.tx_media_priority_active),
                bool01(client.tx_uplink_degraded),
                client.tx_mic_age_ms,
                client.tx_mic_last_arrived_seq,
                client.tx_mic_seq_gap_count,
                client.tx_mic_browser_dropped_count,
                client.tx_uplink_buffered_bytes,
                client.tx_uplink_buffered_high_watermark_bytes,
                client.tx_codec_decode_error_count,
                client.tx_codec_stale_drop_count,
                client.tx_codec_release_flush_count,
                format_tx_diag(latest_tx_diag.as_ref())
            );
            tci.publish_scheduler_telemetry(&client);
            tci.publish_tx_uplink_telemetry(&client);
            let model = radio_model.lock_unpoisoned();
            println!("saturn-bridge: {}", model.status_line());
            last_status = Instant::now();
            status_hp_packets = 0;
            status_tci_mic_frames = 0;
            status_tci_mic_samples = 0;
            did_work = true;
        }

        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        if !did_work {
            thread::sleep(Duration::from_millis(1));
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    let _ = tx_cmd_tx.send(TxCommand::Shutdown);
    let hp_result = hp_thread
        .join()
        .map_err(|_| "high-priority thread panicked")?;
    hp_result?;
    tx_thread.join().map_err(|_| "TX thread panicked")?;
    rx_thread.join().map_err(|_| "RX thread panicked")?;
    thread::sleep(Duration::from_millis(10));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_tx_watt_cap_defaults_and_clamps() {
        assert_eq!(parse_remote_tx_max_watts(None), DEFAULT_REMOTE_TX_MAX_WATTS);
        assert_eq!(parse_remote_tx_max_watts(Some("7")), 7);
        assert_eq!(parse_remote_tx_max_watts(Some("0")), 1);
        assert_eq!(parse_remote_tx_max_watts(Some("150")), 100);
        assert_eq!(
            parse_remote_tx_max_watts(Some("bad")),
            DEFAULT_REMOTE_TX_MAX_WATTS
        );
    }

    #[test]
    fn remote_tx_power_trip_defaults_and_clamps() {
        assert_eq!(
            parse_remote_tx_power_trip_watts(None),
            DEFAULT_REMOTE_TX_POWER_TRIP_WATTS
        );
        assert_eq!(parse_remote_tx_power_trip_watts(Some("60")), 60.0);
        assert_eq!(parse_remote_tx_power_trip_watts(Some("1")), 5.0);
        assert_eq!(parse_remote_tx_power_trip_watts(Some("300")), 200.0);
        assert_eq!(
            parse_remote_tx_power_trip_watts(Some("nan")),
            DEFAULT_REMOTE_TX_POWER_TRIP_WATTS
        );
    }

    #[test]
    fn tx_uplink_late_detector_is_scoped_to_on_air_tx() {
        let now = Instant::now();
        let stale_mic = Some(now - TX_UPLINK_LATE_LIMIT - TX_UPLINK_LATE_SUSTAIN);
        let mut late_since = Some(now - TX_UPLINK_LATE_SUSTAIN);

        assert!(update_tx_uplink_late_detector(false, stale_mic, now, &mut late_since).is_none());
        assert!(late_since.is_none());
    }

    #[test]
    fn tx_media_priority_covers_requested_and_armed_before_on_air() {
        let mut model = RadioModel::new(0, 14_200_000, 0, 192, 24, 2048, false, 2048, false);

        assert!(!tx_media_priority_active(false, &model));

        assert!(tx_media_priority_active(true, &model));

        model.desired.tx_phase = TxPhase::Armed;
        assert!(tx_media_priority_active(false, &model));

        model.desired.tx_phase = TxPhase::Keyed;
        assert!(tx_media_priority_active(false, &model));

        model.desired.tx_phase = TxPhase::Rx;
        model.desired.tx_enabled = true;
        assert!(tx_media_priority_active(false, &model));
    }

    #[test]
    fn tx_uplink_late_detector_requires_sustained_breach() {
        let now = Instant::now();
        let stale_mic = Some(now - TX_UPLINK_LATE_LIMIT - Duration::from_millis(1));
        let mut late_since = None;

        assert!(update_tx_uplink_late_detector(true, stale_mic, now, &mut late_since).is_none());
        let later = now + TX_UPLINK_LATE_SUSTAIN;
        assert!(update_tx_uplink_late_detector(true, stale_mic, later, &mut late_since).is_some());
    }

    #[test]
    fn tx_uplink_late_detector_ignores_missing_first_mic() {
        let now = Instant::now();
        let mut late_since = None;

        assert!(update_tx_uplink_late_detector(true, None, now, &mut late_since).is_none());
        assert!(late_since.is_none());
    }

    #[test]
    fn tx_control_watchdog_is_scoped_to_on_air_tx() {
        let now = Instant::now();
        let stale_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT - Duration::from_millis(1));

        assert!(
            update_tx_control_watchdog(false, stale_control, now, TX_CONTROL_WATCHDOG_LIMIT)
                .is_none()
        );
    }

    #[test]
    fn tx_control_watchdog_fires_after_limit() {
        let now = Instant::now();
        let fresh_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT);
        let stale_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT - Duration::from_millis(1));

        assert!(
            update_tx_control_watchdog(true, fresh_control, now, TX_CONTROL_WATCHDOG_LIMIT)
                .is_none()
        );
        assert!(
            update_tx_control_watchdog(true, stale_control, now, TX_CONTROL_WATCHDOG_LIMIT)
                .is_some()
        );
    }

    #[test]
    fn tx_control_watchdog_waits_for_first_control_message() {
        assert!(
            update_tx_control_watchdog(true, None, Instant::now(), TX_CONTROL_WATCHDOG_LIMIT)
                .is_none()
        );
    }

    #[test]
    fn tx_control_watchdog_uses_wider_split_limit() {
        assert_eq!(tx_control_watchdog_limit(false), TX_CONTROL_WATCHDOG_LIMIT);
        assert_eq!(
            tx_control_watchdog_limit(true),
            TX_CONTROL_WATCHDOG_SPLIT_LIMIT
        );

        let now = Instant::now();
        let legacy_stale_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT - Duration::from_millis(1));
        let split_stale_control =
            Some(now - TX_CONTROL_WATCHDOG_SPLIT_LIMIT - Duration::from_millis(1));

        assert!(update_tx_control_watchdog(
            true,
            legacy_stale_control,
            now,
            TX_CONTROL_WATCHDOG_LIMIT
        )
        .is_some());
        assert!(update_tx_control_watchdog(
            true,
            legacy_stale_control,
            now,
            TX_CONTROL_WATCHDOG_SPLIT_LIMIT
        )
        .is_none());
        assert!(update_tx_control_watchdog(
            true,
            split_stale_control,
            now,
            TX_CONTROL_WATCHDOG_SPLIT_LIMIT
        )
        .is_some());
    }

    #[test]
    fn saturn_power_adc_conversion_applies_meter_scale() {
        let unscaled = saturn_adc_to_watts(1500, 32, 1.0);
        let scaled = saturn_adc_to_watts(1500, 32, 0.8365);
        assert!((scaled - (unscaled * 0.8365)).abs() < 0.001);
    }

    #[test]
    fn tci_client_release_grace_defaults_and_clamps() {
        assert_eq!(
            parse_tci_client_release_grace(None),
            Duration::from_millis(DEFAULT_TCI_CLIENT_RELEASE_GRACE_MS)
        );
        assert_eq!(
            parse_tci_client_release_grace(Some("0")),
            Duration::from_millis(0)
        );
        assert_eq!(
            parse_tci_client_release_grace(Some("750")),
            Duration::from_millis(750)
        );
        assert_eq!(
            parse_tci_client_release_grace(Some("999999")),
            Duration::from_millis(MAX_TCI_CLIENT_RELEASE_GRACE_MS)
        );
        assert_eq!(
            parse_tci_client_release_grace(Some("bad")),
            Duration::from_millis(DEFAULT_TCI_CLIENT_RELEASE_GRACE_MS)
        );
    }

    #[test]
    fn client_ddc0_sample_rate_respects_low_bandwidth_cap() {
        assert_eq!(clamp_client_ddc0_sample_rate_khz(192_000, 48), 48);
        assert_eq!(clamp_client_ddc0_sample_rate_khz(96_000, 48), 48);
        assert_eq!(clamp_client_ddc0_sample_rate_khz(48_000, 48), 48);
        assert_eq!(clamp_client_ddc0_sample_rate_khz(96_000, 192), 96);
        assert_eq!(clamp_client_ddc0_sample_rate_khz(12_000, 48), 48);
    }
}
