mod config;
mod p2;
mod radio_model;
mod tci;
mod tx_thread;
mod wdsp;

use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use config::BridgeConfig;
use p2::session::{P2Event, P2Session};
use radio_model::{RadioModel, TxPhase};
use tci::{TciCommand, TciFrontend};
use tx_thread::{TxCommand, TxDiagnostics, TxEvent};
use wdsp::WdspRxEngine;

const MAX_TCI_COMMANDS_PER_LOOP: usize = 128;
const DEFAULT_REMOTE_TX_MAX_WATTS: u8 = 100;
const DEFAULT_REMOTE_TX_POWER_TRIP_WATTS: f32 = 110.0;
const DEFAULT_TCI_CLIENT_RELEASE_GRACE_MS: u64 = 3_000;
const MAX_TCI_CLIENT_RELEASE_GRACE_MS: u64 = 30_000;
const TX_UPLINK_LATE_LIMIT: Duration = Duration::from_millis(250);
const TX_UPLINK_LATE_SUSTAIN: Duration = Duration::from_millis(100);
// Phase 41 stopgap: bridge force-RX if no operator control message arrives
// within this window while keyed. Independent safety net for the case where
// browser PTT-release bytes are stuck behind queued media on the single
// WebSocket. Tightens to ~200ms in Phase 42 once control runs on its own
// socket; cannot be disabled.
const TX_CONTROL_WATCHDOG_LIMIT: Duration = Duration::from_millis(500);

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

// Phase 41 stopgap: detect prolonged absence of operator control messages
// while keyed. Returns Some(silence) when the bridge has been keyed and
// silent of control input for longer than TX_CONTROL_WATCHDOG_LIMIT.
fn update_tx_control_watchdog(
    on_air: bool,
    last_control_at: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    if !on_air {
        return None;
    }
    let last_control_at = last_control_at?;
    let silence = now.saturating_duration_since(last_control_at);
    if silence > TX_CONTROL_WATCHDOG_LIMIT {
        Some(silence)
    } else {
        None
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

fn main() -> Result<(), Box<dyn Error>> {
    let config = BridgeConfig::from_env();
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
    let tci = TciFrontend::bind(&config, radio_model.clone())?;
    let mut wdsp = {
        let model = radio_model.lock().unwrap();
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
        let mut model = radio_model.lock().unwrap();
        model.desired.tx_drive = model.desired.tx_drive.clamp(1, remote_tx_max_watts);
    }
    let (tx_cmd_tx, tx_cmd_rx) = mpsc::channel();
    let (tx_event_tx, tx_event_rx) = mpsc::channel();

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

    let mut last_status = Instant::now();
    let mut last_duc_specific = Instant::now();
    let mut tx_requested = false;
    let mut status_hp_packets = 0u64;
    let mut status_ddc_packets = 0u64;
    let mut status_rx_audio_frames = 0u64;
    let mut status_rx_audio_samples = 0u64;
    let mut status_tci_mic_frames = 0u64;
    let mut status_tci_mic_samples = 0u64;
    let mut latest_tx_diag: Option<TxDiagnostics> = None;
    let mut controller_release_deadline: Option<Instant> = None;
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
            let Some(command) = tci.try_recv_command() else {
                break;
            };
            did_work = true;
            let mut model = radio_model.lock().unwrap();
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
                    model.desired.mode = mode;
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
                    let rate_khz = clamp_client_ddc0_sample_rate_khz(
                        rate_hz,
                        config.max_client_ddc0_sample_rate_khz,
                    );
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
                TciCommand::Phase42SessionOpen {
                    client_id,
                    session_id,
                    role,
                } => {
                    println!(
                        "saturn-bridge: Phase 42 client {client_id} opened session {session_id} as {:?}",
                        role
                    );
                }
                TciCommand::Phase42SessionLane {
                    client_id,
                    session_id,
                    lane,
                } => {
                    println!(
                        "saturn-bridge: Phase 42 client {client_id} marked {:?} lane for session {session_id}",
                        lane
                    );
                }
                TciCommand::SetAudioStreaming(enabled) => {
                    if enabled {
                        wdsp.reset_audio_packetizer();
                        tci.publish_audio_started(wdsp.audio_sample_rate_hz());
                    } else {
                        tci.publish_audio_stopped();
                    }
                }
                TciCommand::SetAudioSampleRate(rate_hz) => {
                    if rate_hz != wdsp.audio_sample_rate_hz() {
                        eprintln!(
                            "saturn-bridge: requested audio sample rate {} Hz, using {} Hz",
                            rate_hz,
                            wdsp.audio_sample_rate_hz()
                        );
                    }
                    wdsp.reset_audio_packetizer();
                    tci.publish_audio_started(wdsp.audio_sample_rate_hz());
                }
                TciCommand::SetAudioFrameSamples(sample_count) => {
                    let normalized = wdsp.set_audio_frame_float_count(sample_count as usize) as u32;
                    if sample_count != normalized {
                        eprintln!(
                            "saturn-bridge: requested audio frame size {} float32 samples, using {}",
                            sample_count,
                            normalized
                        );
                    }
                }
                TciCommand::SetAudioChannels(channels) => {
                    if channels != 2 {
                        eprintln!(
                            "saturn-bridge: requested {} audio channels, using stereo",
                            channels
                        );
                    }
                }
                TciCommand::ClientConnected => {
                    controller_release_deadline = None;
                    model.desired.running = true;
                    needs_bootstrap = true;
                    needs_duc_specific = true;
                    println!("saturn-bridge: TCI client active — taking P2 controller role");
                }
                TciCommand::ClientDisconnected => {
                    tx_requested = false;
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
                    if tci_client_release_grace.is_zero() {
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
                    if enabled {
                        if !tx_requested {
                            if model.desired.tx_drive > remote_tx_max_watts {
                                eprintln!(
                                    "saturn-bridge: clamping remote TX target {}W -> {}W before arm",
                                    model.desired.tx_drive, remote_tx_max_watts
                                );
                                model.desired.tx_drive = remote_tx_max_watts;
                                needs_high_priority = true;
                            }
                            tx_requested = true;
                            last_operator_mic_at = None;
                            tx_uplink_late_since = None;
                            tx_uplink_fault_active = false;
                            tx_control_watchdog_fault_active = false;
                            model.desired.tx_enabled = false;
                            model.desired.tx_phase = TxPhase::Armed;
                            wdsp.reset_stream_buffers();
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
                    } else if tx_requested || model.desired.tx_enabled {
                        tx_requested = false;
                        last_operator_mic_at = None;
                        tx_uplink_late_since = None;
                        tx_uplink_fault_active = false;
                        tx_control_watchdog_fault_active = false;
                        wdsp.reset_stream_buffers();
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
                    if !tx_requested {
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

            if reconfigure_ddc {
                session.configure_rx_ddc(
                    model.desired.rx_ddc_index,
                    model.desired.ddc0_sample_rate_khz,
                    model.desired.ddc0_sample_size_bits,
                    model.desired.ddc0_adc,
                )?;
            }
            wdsp.sync_model(&model)?;
            tci.publish_radio_state(&model);
        }

        if needs_duc_specific {
            session.send_duc_specific()?;
            last_duc_specific = Instant::now();
            did_work = true;
        }

        // bootstrap() acquires the radio_model lock internally, so it must be called
        // after the command loop has released it, never while holding it.
        if needs_bootstrap {
            session.bootstrap(&radio_model)?;
            last_duc_specific = Instant::now();
            did_work = true;
        }
        if needs_stop {
            session.send_stop()?;
            did_work = true;
        }
        if needs_high_priority {
            let model = radio_model.lock().unwrap();
            session.send_high_priority(&model)?;
            did_work = true;
        }

        let running = { radio_model.lock().unwrap().desired.running };
        if running && last_duc_specific.elapsed() >= config.high_priority_period {
            session.send_duc_specific()?;
            last_duc_specific = Instant::now();
            did_work = true;
        }

        if let Some(deadline) = controller_release_deadline {
            if Instant::now() >= deadline {
                controller_release_deadline = None;
                let mut model = radio_model.lock().unwrap();
                if model.desired.running {
                    model.desired.running = false;
                    session.send_stop()?;
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
                    let mut model = radio_model.lock().unwrap();
                    model.desired.tx_phase = TxPhase::Keyed;
                    tci.publish_radio_state(&model);
                }
                TxEvent::Unkeyed => {
                    tx_requested = false;
                    last_operator_mic_at = None;
                    tx_uplink_late_since = None;
                    tx_uplink_fault_active = false;
                    tx_control_watchdog_fault_active = false;
                    latest_tx_diag = None;
                    let mut model = radio_model.lock().unwrap();
                    model.desired.tx_phase = TxPhase::Rx;
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
            }
        }

        if let Some(event) = session.recv_event()? {
            did_work = true;
            let mut model = radio_model.lock().unwrap();
            match event {
                P2Event::HighPriorityFromSdr(packet) => {
                    status_hp_packets = status_hp_packets.saturating_add(1);
                    let forward_watts =
                        saturn_adc_to_watts(packet.forward_power, 32, tx_power_meter_scale);
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
                        tx_requested = false;
                        last_operator_mic_at = None;
                        tx_uplink_late_since = None;
                        tx_uplink_fault_active = false;
                        tx_control_watchdog_fault_active = false;
                        model.desired.tx_enabled = false;
                        model.desired.tx_phase = TxPhase::Rx;
                        let _ = tx_cmd_tx.send(TxCommand::Disarm);
                        session.send_high_priority(&model)?;
                        session.send_duc_specific()?;
                    }
                    tci.publish_telemetry(&model);
                }
                P2Event::DdcIq(frame) => {
                    if frame.ddc_index != model.desired.rx_ddc_index {
                        continue;
                    }
                    status_ddc_packets = status_ddc_packets.saturating_add(1);
                    let sample_rate_hz = model.desired.ddc0_sample_rate_khz as u32 * 1000;
                    tci.publish_iq_frame(sample_rate_hz, &frame.iq_samples);
                    // Keep display IQ live during MOX, but do not drive RX WDSP
                    // audio/AGC with local TX energy; that makes unkey recovery
                    // feel slow, especially with SLOW/LONG AGC.
                    if !tx_requested && !model.desired.tx_enabled {
                        for audio_frame in wdsp.push_iq(&frame.iq_samples) {
                            status_rx_audio_frames = status_rx_audio_frames.saturating_add(1);
                            status_rx_audio_samples =
                                status_rx_audio_samples.saturating_add(audio_frame.len() as u64);
                            tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio_frame);
                        }
                    }
                    if let Some(dbm) = wdsp.smeter_dbm() {
                        model.observed.ddc0_meter_dbm = Some(dbm);
                    }
                    model.apply_ddc_frame(frame);
                }
            }
        }

        {
            let now = Instant::now();
            let mut model = radio_model.lock().unwrap();
            let on_air = model.desired.tx_phase == TxPhase::Keyed || model.desired.tx_enabled;
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
                    tx_requested = false;
                    tx_uplink_fault_active = true;
                    last_operator_mic_at = None;
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    session.send_high_priority(&model)?;
                    session.send_duc_specific()?;
                    last_duc_specific = now;
                    did_work = true;
                }
            }

            // Phase 41 stopgap: independent of mic freshness. This catches
            // single-WebSocket media backlog that delays operator release bytes.
            if on_air && !tx_control_watchdog_fault_active {
                if let Some(silence) =
                    update_tx_control_watchdog(on_air, tci.last_operator_control_at(), now)
                {
                    let silence_ms = silence.as_millis() as u64;
                    let limit_ms = TX_CONTROL_WATCHDOG_LIMIT.as_millis() as u64;
                    eprintln!(
                        "saturn-bridge: TX control watchdog {}ms > {}ms; forcing RX",
                        silence_ms, limit_ms
                    );
                    tci.publish_tx_control_watchdog(silence_ms, limit_ms);
                    tx_requested = false;
                    tx_control_watchdog_fault_active = true;
                    model.desired.tx_enabled = false;
                    model.desired.tx_phase = TxPhase::Rx;
                    let _ = tx_cmd_tx.send(TxCommand::Disarm);
                    session.send_high_priority(&model)?;
                    session.send_duc_specific()?;
                    last_duc_specific = now;
                    did_work = true;
                }
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            let elapsed = last_status.elapsed().as_secs_f64().max(0.001);
            let client = tci.client_snapshot();
            println!(
                "saturn-bridge: diag hp_s={:.1} ddc_s={:.1} rx_audio_frames_s={:.1} rx_audio_samples_s={:.0} tci_mic_frames_s={:.1} tci_mic_samples_s={:.0} client={} iq={} audio={} phase42_control={} phase42_media={} phase42_paired={} outbound_drops={} safety_p99_us={} control_p99_us={} display_replaced_s={} display_dropped_s={} display_rate_limited_s={} audio_dropped_s={} audio_gaps={} audio_panic={} send_blocked_ms={} out_hwm_bytes={} tcp_outq_hwm_bytes={} safety_depth_overflow={} tx_uplink_degraded={} tx_mic_age_ms={} tx_mic_seq={} tx_mic_seq_gaps={} tx_mic_drops={} tx_uplink_buf={} tx_uplink_hwm={} {}",
                status_hp_packets as f64 / elapsed,
                status_ddc_packets as f64 / elapsed,
                status_rx_audio_frames as f64 / elapsed,
                status_rx_audio_samples as f64 / elapsed,
                status_tci_mic_frames as f64 / elapsed,
                status_tci_mic_samples as f64 / elapsed,
                bool01(client.active),
                bool01(client.iq_stream_enabled),
                bool01(client.audio_stream_enabled),
                client.phase42_control_clients,
                client.phase42_media_clients,
                client.phase42_paired_sessions,
                client.outbound_drops,
                client.safety_enqueue_to_write_p99_us,
                client.control_enqueue_to_write_p99_us,
                client.display_replaced_per_sec,
                client.display_dropped_per_sec,
                client.display_rate_limited_per_sec,
                client.audio_dropped_per_sec,
                client.audio_seq_gap_count,
                client.audio_panic_drain_count,
                client.send_blocked_ms,
                client.outbound_high_watermark_bytes,
                client.tcp_outq_high_watermark_bytes,
                client.safety_queue_depth_overflow_count,
                bool01(client.tx_uplink_degraded),
                client.tx_mic_age_ms,
                client.tx_mic_last_arrived_seq,
                client.tx_mic_seq_gap_count,
                client.tx_mic_browser_dropped_count,
                client.tx_uplink_buffered_bytes,
                client.tx_uplink_buffered_high_watermark_bytes,
                format_tx_diag(latest_tx_diag.as_ref())
            );
            tci.publish_scheduler_telemetry(&client);
            tci.publish_tx_uplink_telemetry(&client);
            let model = radio_model.lock().unwrap();
            println!("saturn-bridge: {}", model.status_line());
            last_status = Instant::now();
            status_hp_packets = 0;
            status_ddc_packets = 0;
            status_rx_audio_frames = 0;
            status_rx_audio_samples = 0;
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

        assert!(update_tx_control_watchdog(false, stale_control, now).is_none());
    }

    #[test]
    fn tx_control_watchdog_fires_after_limit() {
        let now = Instant::now();
        let fresh_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT);
        let stale_control = Some(now - TX_CONTROL_WATCHDOG_LIMIT - Duration::from_millis(1));

        assert!(update_tx_control_watchdog(true, fresh_control, now).is_none());
        assert!(update_tx_control_watchdog(true, stale_control, now).is_some());
    }

    #[test]
    fn tx_control_watchdog_waits_for_first_control_message() {
        assert!(update_tx_control_watchdog(true, None, Instant::now()).is_none());
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
