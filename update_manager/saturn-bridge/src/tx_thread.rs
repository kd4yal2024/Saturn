use std::collections::VecDeque;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::p2::session::P2Session;
use crate::radio_model::{PureSignalState, RadioModel};
use crate::sync_ext::MutexExt;
use crate::wdsp::{
    WdspTxEngine, DUC_IQ_SAMPLES_PER_PACKET, TX_MIC_SAMPLES_PER_DSP_BLOCK, WDSP_TX_IQ_RATE_HZ,
};

const TX_SILENCE_GAP: Duration = Duration::from_millis(250);
const TX_KEEPALIVE_RESUME_FRAMES: u8 = 3;
const TX_UNKEY_BURST_COUNT: usize = 12;
const TX_UNKEY_BURST_SPACING: Duration = Duration::from_millis(5);
const TX_KEY_IQ_PEAK_THRESHOLD: f32 = 0.001;
/// Minimum mic input peak (linear) to consider as real operator audio.
/// Below this, mic frames are treated as silence/noise regardless of what
/// WDSP outputs (which can include residual filter state).
const TX_KEY_MIC_PEAK_THRESHOLD: f32 = 0.005;
/// Maximum age of the last keyable mic frame for the mic gate to be open.
/// If the operator stops speaking, the gate closes after this window,
/// preventing stale mic detection from allowing RF keying on WDSP residuals.
const TX_MIC_RECENCY_WINDOW: Duration = Duration::from_millis(500);
const TX_DISPLAY_DUC_PACKETS_PER_FRAME: usize = 4;
const TX_ZERO_IQ_LOG_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_TX_WATCHDOG: Duration = Duration::from_secs(180);
const MIN_TX_WATCHDOG: Duration = Duration::from_secs(3);
const MAX_TX_WATCHDOG: Duration = Duration::from_secs(180);
const MAX_DUC_PACKETS_PER_LOOP: usize = 8;
const MAX_TX_COMMANDS_PER_LOOP: usize = 128;
const TX_MIC_INPUT_QUEUE_MAX_SAMPLES: usize = 48_000;
const DEFAULT_TX_MIC_PREFILL_SAMPLES: usize = 2_048;
const MIN_TX_MIC_PREFILL_MS: u64 = 20;
const MAX_TX_MIC_PREFILL_MS: u64 = 250;
const TX_ACTIVE_IDLE_SLEEP: Duration = Duration::from_micros(250);
const PURE_SIGNAL_FEEDBACK_TIMEOUT: Duration = Duration::from_millis(250);
const PURE_SIGNAL_STATUS_PERIOD: Duration = Duration::from_millis(100);

pub type TxRadioResult = Result<(), String>;

/// Hardware-output boundary for the shared WDSP/TCI transmit state machine.
///
/// P2 remains the default implementation. The direct-XDMA implementation uses
/// the same arm, mic/IQ, watchdog, unkey, and diagnostics logic while changing
/// only how radio configuration and DUC IQ reach the FPGA.
pub trait TxRadio: Send + Sync {
    fn configure_puresignal_feedback(&self) -> TxRadioResult;
    fn send_duc_specific(&self, model: &RadioModel) -> TxRadioResult;
    fn send_high_priority(&self, model: &RadioModel) -> TxRadioResult;
    /// Stage the first keyable DUC IQ and key the selected backend when its
    /// transmit data plane is ready. Direct XDMA may return `Ok(false)` while
    /// it accumulates a safe FIFO prefill; P2 keys and sends immediately.
    fn try_key_with_iq(&self, model: &RadioModel, iq_samples: &[f32]) -> Result<bool, String>;
    fn send_duc_iq(&self, iq_samples: &[f32]) -> TxRadioResult;
    /// Exercise the DUC data path while RF controls remain forced to receive.
    /// P2 uses only the browser-side diagnostic; direct XDMA also validates
    /// packing, DMA writes, FIFO pacing, and cleanup in this mode.
    fn stage_iq_rf_disabled(&self, _model: &RadioModel, _iq_samples: &[f32]) -> TxRadioResult {
        Ok(())
    }
    /// Number of zero-input WDSP blocks to process and discard whenever TX is
    /// armed. Direct hardware uses this to flush retained native filter state;
    /// P2 keeps its established zero-latency behavior.
    fn startup_settle_blocks(&self) -> usize {
        0
    }
    /// Consecutive mic-qualified IQ packets required before RF may be keyed.
    /// A backend may raise this above one to reject startup transients.
    fn key_qualification_packets(&self) -> usize {
        1
    }
    /// How recently a keyable mic block must have entered WDSP for its output
    /// IQ to qualify. Direct hardware narrows this alignment window.
    fn keyable_mic_window(&self) -> Duration {
        TX_MIC_RECENCY_WINDOW
    }
    /// Direct hardware aligns mic qualification to the fixed block entering
    /// WDSP. P2 retains its established browser-frame qualification point.
    fn qualify_mic_at_dsp_input(&self) -> bool {
        false
    }
    /// Recreate the native WDSP channel for each arm. Direct XDMA uses this to
    /// prevent filter/ALC state from one transmission entering the next one.
    fn recreate_wdsp_on_arm(&self) -> bool {
        false
    }
    /// Direct XDMA cannot tolerate a synchronous WDSP reconfiguration while
    /// its hardware FIFO is draining. Such changes take effect on next arm.
    fn defer_model_changes_while_keyed(&self) -> bool {
        false
    }
    fn configure_rx_ddc(
        &self,
        ddc_index: u8,
        sample_rate_khz: u16,
        sample_size_bits: u8,
        adc: u8,
    ) -> TxRadioResult;
}

impl TxRadio for P2Session {
    fn configure_puresignal_feedback(&self) -> TxRadioResult {
        P2Session::configure_puresignal_feedback(self).map_err(|error| error.to_string())
    }

    fn send_duc_specific(&self, model: &RadioModel) -> TxRadioResult {
        P2Session::send_duc_specific(self, model).map_err(|error| error.to_string())
    }

    fn send_high_priority(&self, model: &RadioModel) -> TxRadioResult {
        P2Session::send_high_priority(self, model).map_err(|error| error.to_string())
    }

    fn try_key_with_iq(&self, model: &RadioModel, iq_samples: &[f32]) -> Result<bool, String> {
        P2Session::send_high_priority(self, model).map_err(|error| error.to_string())?;
        P2Session::send_duc_iq(self, iq_samples).map_err(|error| error.to_string())?;
        Ok(true)
    }

    fn send_duc_iq(&self, iq_samples: &[f32]) -> TxRadioResult {
        P2Session::send_duc_iq(self, iq_samples).map_err(|error| error.to_string())
    }

    fn configure_rx_ddc(
        &self,
        ddc_index: u8,
        sample_rate_khz: u16,
        sample_size_bits: u8,
        adc: u8,
    ) -> TxRadioResult {
        P2Session::configure_rx_ddc(self, ddc_index, sample_rate_khz, sample_size_bits, adc)
            .map_err(|error| error.to_string())
    }
}

fn duc_iq_packet_period() -> Duration {
    Duration::from_secs_f64(DUC_IQ_SAMPLES_PER_PACKET as f64 / WDSP_TX_IQ_RATE_HZ as f64)
}

fn duc_iq_packet_can_key_rf(rf_enabled: bool, peak: f32) -> bool {
    rf_enabled && peak >= TX_KEY_IQ_PEAK_THRESHOLD
}

/// Compound RF keying predicate. Requires:
///  1. RF is enabled and IQ peak exceeds threshold
///  2. Recent mic audio above noise floor (or two-tone mode)
fn can_key_rf(rf_enabled: bool, iq_peak: f32, mic_recent: bool, two_tone: bool) -> bool {
    duc_iq_packet_can_key_rf(rf_enabled, iq_peak) && (mic_recent || two_tone)
}

#[derive(Clone, Copy, Debug)]
struct KeyQualification {
    required: usize,
    consecutive: usize,
}

impl KeyQualification {
    fn new(required: usize) -> Self {
        Self {
            required: required.max(1),
            consecutive: 0,
        }
    }

    fn reset(&mut self) {
        self.consecutive = 0;
    }

    fn observe(&mut self, eligible: bool) -> bool {
        if eligible {
            self.consecutive = self.consecutive.saturating_add(1).min(self.required);
        } else {
            self.reset();
        }
        self.consecutive >= self.required
    }
}

fn settle_wdsp_tx(wdsp_tx: &mut WdspTxEngine, blocks: usize) {
    if blocks == 0 {
        return;
    }
    let silence = vec![0.0; TX_MIC_SAMPLES_PER_DSP_BLOCK];
    for _ in 0..blocks {
        wdsp_tx.push_mic(&silence);
    }
    wdsp_tx.pending_iq.clear();
}

pub enum TxCommand {
    /// PTT pressed — arm WDSP TX channel, wait for DUC IQ before keying.
    Arm {
        rf_enabled: bool,
    },
    /// PTT released — unkey, send stop burst, deactivate WDSP.
    Disarm,
    /// Mic audio samples from TCI client.
    MicAudio {
        samples: Vec<f32>,
        channels: u32,
        sample_rate_hz: u32,
    },
    PureSignalFeedback {
        sequence: u32,
        tx_reference: Vec<f64>,
        rx_feedback: Vec<f64>,
        received_at: Instant,
    },
    PureSignalReset,
    /// TX-relevant model parameters changed — re-sync WDSP TX DSP chain.
    ModelChanged,
    /// Shut down the TX thread.
    Shutdown,
}

pub enum TxEvent {
    /// Radio keyed (MOX asserted, DUC IQ flowing).
    Keyed,
    /// Radio unkeyed (MOX de-asserted).
    Unkeyed,
    /// TX DUC IQ display frame for the browser panadapter/waterfall.
    TxIqFrame {
        sample_rate_hz: u32,
        iq_samples: Vec<f32>,
    },
    /// Compact TX timing/level diagnostics for bridge status logging.
    Diagnostics(TxDiagnostics),
    PureSignalStatus(PureSignalStatus),
}

#[derive(Clone, Copy, Debug)]
pub struct PureSignalStatus {
    pub state: PureSignalState,
    pub feedback_level: i32,
    pub calibration_count: i32,
    pub correcting: bool,
    pub max_tx: f64,
    pub feedback_packets: u64,
    pub feedback_gaps: u64,
    pub attenuation_db: u8,
}

#[derive(Clone, Debug)]
pub struct TxDiagnostics {
    pub state: &'static str,
    pub rf_enabled: bool,
    pub mic_frames: u64,
    pub duc_packets: u64,
    pub armed_ms: u64,
    pub first_mic_ms: Option<u64>,
    pub first_iq_ms: Option<u64>,
    pub first_keyable_iq_ms: Option<u64>,
    pub mic_recent: bool,
    pub keyed_ms: Option<u64>,
    pub input_peak: f32,
    pub output_peak: f32,
    pub mic_peak_db: f64,
    pub comp_peak_db: f64,
    pub comp_avg_db: f64,
    pub alc_peak_db: f64,
    pub alc_avg_db: f64,
    pub alc_gain_db: f64,
    pub out_peak_db: f64,
    pub total_input_samples: u64,
    pub total_output_pairs: u64,
    pub pending_mic_floats: usize,
    pub pending_iq_floats: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxState {
    Idle,
    Armed,
    Keyed,
}

impl TxState {
    fn as_str(self) -> &'static str {
        match self {
            TxState::Idle => "idle",
            TxState::Armed => "armed",
            TxState::Keyed => "keyed",
        }
    }
}

fn should_defer_model_change(state: TxState, backend_requires_deferral: bool) -> bool {
    backend_requires_deferral && state == TxState::Keyed
}

pub fn spawn(
    session: Arc<dyn TxRadio>,
    radio_model: Arc<Mutex<RadioModel>>,
    cmd_rx: Receiver<TxCommand>,
    event_tx: Sender<TxEvent>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("saturn-tx".into())
        .spawn(move || {
            run(session, radio_model, cmd_rx, event_tx, stop_flag);
        })
        .expect("failed to spawn TX thread")
}

fn run(
    session: Arc<dyn TxRadio>,
    radio_model: Arc<Mutex<RadioModel>>,
    cmd_rx: Receiver<TxCommand>,
    event_tx: Sender<TxEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut wdsp_tx = {
        let model = radio_model.lock_unpoisoned();
        WdspTxEngine::new(&model)
    };

    let mic_block_period = Duration::from_secs_f64(TX_MIC_SAMPLES_PER_DSP_BLOCK as f64 / 48_000.0);
    let duc_packet_period = duc_iq_packet_period();
    let floats_per_packet = DUC_IQ_SAMPLES_PER_PACKET * 2;
    let tx_display_frame_floats = floats_per_packet * TX_DISPLAY_DUC_PACKETS_PER_FRAME;

    let mut state = TxState::Idle;
    let mut rf_enabled = false;
    let mut two_tone = false;
    let mut last_mic_audio_at = Instant::now();
    let mut next_mic_dsp_at = Instant::now();
    let mut next_duc_iq_at = Instant::now();
    let mut pending_mic_samples: VecDeque<f32> = VecDeque::new();
    let mut mic_queue_underruns = 0u64;
    let mut mic_output_started = false;
    let mut last_mic_output_sample = 0.0f32;
    let mut keepalive_active = false;
    let mut keepalive_resume_frames = 0u8;
    let mut tx_armed_at = Instant::now();
    let mut last_zero_iq_log_at = Instant::now();
    let mut mic_frame_count = 0u64;
    let mut duc_packet_count = 0u64;
    let mut tx_display_buffer = Vec::with_capacity(tx_display_frame_floats);
    let mut tx_display_peak = 0.0f32;
    let mut last_diag_at = Instant::now();
    let mut last_diag_event_at = Instant::now();
    let mut first_mic_audio_at: Option<Instant> = None;
    let mut first_iq_at: Option<Instant> = None;
    let mut first_keyable_iq_at: Option<Instant> = None;
    let mut keyed_at: Option<Instant> = None;
    let mut last_keyable_mic_at: Option<Instant> = None;
    let mut pure_signal_last_sequence: Option<u32> = None;
    let mut pure_signal_last_feedback_at: Option<Instant> = None;
    let mut pure_signal_feedback_packets = 0u64;
    let mut pure_signal_feedback_gaps = 0u64;
    let mut pure_signal_last_calibration_count = 0i32;
    let mut pure_signal_fault_active = false;
    let mut pure_signal_last_status_at = Instant::now();
    let tx_watchdog = tx_watchdog_duration();
    let tx_mic_prefill_samples = tx_mic_prefill_samples();
    let tx_mic_prefill_ms = tx_mic_prefill_samples as f64 / 48.0;
    let startup_settle_blocks = session.startup_settle_blocks();
    let keyable_mic_window = session.keyable_mic_window();
    let qualify_mic_at_dsp_input = session.qualify_mic_at_dsp_input();
    let recreate_wdsp_on_arm = session.recreate_wdsp_on_arm();
    let defer_model_changes_while_keyed = session.defer_model_changes_while_keyed();
    let mut key_qualification = KeyQualification::new(session.key_qualification_packets());
    let mut logged_keyable_mic_input = false;

    println!(
        "saturn-bridge: TX thread started; watchdog={}s mic_prefill={} samples ({:.1}ms) settle_blocks={} key_qualify_packets={} mic_window={}ms",
        tx_watchdog.as_secs(),
        tx_mic_prefill_samples,
        tx_mic_prefill_ms,
        startup_settle_blocks,
        key_qualification.required,
        keyable_mic_window.as_millis()
    );

    while !stop_flag.load(Ordering::Relaxed) {
        let mut did_work = false;

        // Bound command draining so synchronized feedback traffic cannot starve
        // the fixed-cadence DUC IQ producer.
        for _ in 0..MAX_TX_COMMANDS_PER_LOOP {
            match cmd_rx.try_recv() {
                Ok(TxCommand::Arm {
                    rf_enabled: arm_rf_enabled,
                }) => {
                    if state == TxState::Idle {
                        state = TxState::Armed;
                        rf_enabled = arm_rf_enabled;
                        let now = Instant::now();
                        tx_armed_at = now;
                        last_zero_iq_log_at = now;
                        if recreate_wdsp_on_arm {
                            let model = radio_model.lock_unpoisoned();
                            wdsp_tx.recreate_channel(&model);
                            println!(
                                "saturn-bridge: TX native WDSP channel recreated for clean arm"
                            );
                        }
                        wdsp_tx.set_active(true);
                        {
                            let model = radio_model.lock_unpoisoned();
                            wdsp_tx.sync_model(&model);
                            two_tone = model.desired.two_tone_enabled;
                            if let Err(e) = session.send_duc_specific(&model) {
                                eprintln!("saturn-bridge: TX thread: duc_specific error: {e}");
                            }
                        }
                        settle_wdsp_tx(&mut wdsp_tx, startup_settle_blocks);
                        if startup_settle_blocks != 0 {
                            println!(
                                "saturn-bridge: TX startup filters settled with {} zero-input WDSP blocks",
                                startup_settle_blocks
                            );
                        }
                        last_mic_audio_at = now;
                        next_mic_dsp_at = now;
                        next_duc_iq_at = now;
                        pending_mic_samples.clear();
                        mic_queue_underruns = 0;
                        mic_output_started = false;
                        last_mic_output_sample = 0.0;
                        keepalive_active = false;
                        keepalive_resume_frames = 0;
                        mic_frame_count = 0;
                        duc_packet_count = 0;
                        tx_display_buffer.clear();
                        tx_display_peak = 0.0;
                        last_diag_at = now;
                        last_diag_event_at = now;
                        first_mic_audio_at = None;
                        first_iq_at = None;
                        first_keyable_iq_at = None;
                        keyed_at = None;
                        last_keyable_mic_at = None;
                        logged_keyable_mic_input = false;
                        key_qualification.reset();
                        pure_signal_last_sequence = None;
                        pure_signal_last_feedback_at = None;
                        pure_signal_feedback_packets = 0;
                        pure_signal_feedback_gaps = 0;
                        pure_signal_last_calibration_count = 0;
                        pure_signal_fault_active = false;
                        println!(
                            "saturn-bridge: TX armed; waiting for mic audio + nonzero DUC IQ{}",
                            if rf_enabled { "" } else { " (RF disabled)" }
                        );
                        did_work = true;
                    }
                }
                Ok(TxCommand::Disarm) => {
                    if state != TxState::Idle {
                        do_unkey(
                            session.as_ref(),
                            &radio_model,
                            &mut wdsp_tx,
                            &event_tx,
                            state,
                        );
                        state = TxState::Idle;
                        rf_enabled = false;
                        two_tone = false;
                        key_qualification.reset();
                        keepalive_active = false;
                        keepalive_resume_frames = 0;
                        tx_display_buffer.clear();
                        tx_display_peak = 0.0;
                        did_work = true;
                    }
                }
                Ok(TxCommand::MicAudio {
                    samples,
                    channels,
                    sample_rate_hz,
                }) => {
                    if state != TxState::Idle {
                        let mono = mic_samples_to_mono(samples, channels);
                        let mic_peak = mono.iter().fold(0.0f32, |p, s| p.max(s.abs()));
                        if mic_peak >= TX_KEY_MIC_PEAK_THRESHOLD && !logged_keyable_mic_input {
                            logged_keyable_mic_input = true;
                            println!(
                                "saturn-bridge: TX mic audio detected (peak={:.4}, threshold={:.4})",
                                mic_peak, TX_KEY_MIC_PEAK_THRESHOLD
                            );
                        }
                        if mic_peak >= TX_KEY_MIC_PEAK_THRESHOLD && !qualify_mic_at_dsp_input {
                            last_keyable_mic_at = Some(Instant::now());
                        }
                        for sample in mono.iter().copied() {
                            if pending_mic_samples.len() >= TX_MIC_INPUT_QUEUE_MAX_SAMPLES {
                                let _ = pending_mic_samples.pop_front();
                            }
                            pending_mic_samples.push_back(sample);
                        }
                        mic_frame_count = mic_frame_count.saturating_add(1);
                        last_mic_audio_at = Instant::now();
                        if first_mic_audio_at.is_none() {
                            first_mic_audio_at = Some(last_mic_audio_at);
                        }
                        if mic_frame_count == 1
                            || last_diag_at.elapsed() >= Duration::from_millis(500)
                        {
                            let diag = wdsp_tx.diagnostics();
                            println!(
                                "saturn-bridge: TX diag mic frame={} channels={} sample_rate={}Hz mono_samples={} queue_samples={} underruns={} total_samples={} input_peak={:.4} output_peak={:.4} wdsp_mic_pk={:.1}dB wdsp_out_pk={:.1}dB iq_pairs={}",
                                mic_frame_count,
                                channels,
                                sample_rate_hz,
                                mono.len(),
                                pending_mic_samples.len(),
                                mic_queue_underruns,
                                diag.total_input_samples,
                                diag.input_peak,
                                diag.output_peak,
                                diag.mic_peak_db,
                                diag.out_peak_db,
                                diag.total_output_pairs
                            );
                            last_diag_at = Instant::now();
                        }
                        if keepalive_active {
                            keepalive_resume_frames = keepalive_resume_frames.saturating_add(1);
                            if keepalive_resume_frames >= TX_KEEPALIVE_RESUME_FRAMES {
                                keepalive_active = false;
                                keepalive_resume_frames = 0;
                                println!("saturn-bridge: TX live audio resumed");
                            }
                        } else {
                            keepalive_resume_frames = 0;
                        }
                        did_work = true;
                    }
                }
                Ok(TxCommand::PureSignalFeedback {
                    sequence,
                    tx_reference,
                    rx_feedback,
                    received_at,
                }) => {
                    if state == TxState::Keyed {
                        if let Some(expected) =
                            pure_signal_last_sequence.map(|value| value.wrapping_add(1))
                        {
                            pure_signal_feedback_gaps = pure_signal_feedback_gaps
                                .saturating_add(puresignal_sequence_gap(expected, sequence));
                        }
                        pure_signal_last_sequence = Some(sequence);
                        pure_signal_last_feedback_at = Some(received_at);
                        pure_signal_feedback_packets =
                            pure_signal_feedback_packets.saturating_add(1);

                        if pure_signal_fault_active {
                            pure_signal_fault_active = false;
                            wdsp_tx.reset_puresignal();
                            wdsp_tx.set_puresignal_mox(true);
                        }

                        if let Some(diag) =
                            wdsp_tx.push_puresignal_feedback(&tx_reference, &rx_feedback)
                        {
                            let mut model = radio_model.lock_unpoisoned();
                            if diag.calibration_count != pure_signal_last_calibration_count {
                                pure_signal_last_calibration_count = diag.calibration_count;
                                if model.desired.pure_signal_auto_attenuate {
                                    let adjusted = puresignal_auto_attenuation(
                                        diag.feedback_level,
                                        model.desired.pure_signal_attenuation_db,
                                    );
                                    if adjusted != model.desired.pure_signal_attenuation_db {
                                        model.desired.pure_signal_attenuation_db = adjusted;
                                        wdsp_tx.reset_puresignal();
                                        if let Err(error) = session.send_duc_specific(&model) {
                                            eprintln!(
                                                "saturn-bridge: PureSignal attenuation update failed: {error}"
                                            );
                                        }
                                    }
                                }
                            }
                            if pure_signal_last_status_at.elapsed() >= PURE_SIGNAL_STATUS_PERIOD {
                                let _ =
                                    event_tx.send(TxEvent::PureSignalStatus(PureSignalStatus {
                                        state: diag.state,
                                        feedback_level: diag.feedback_level,
                                        calibration_count: diag.calibration_count,
                                        correcting: diag.correcting,
                                        max_tx: diag.max_tx,
                                        feedback_packets: pure_signal_feedback_packets,
                                        feedback_gaps: pure_signal_feedback_gaps,
                                        attenuation_db: model.desired.pure_signal_attenuation_db,
                                    }));
                                pure_signal_last_status_at = Instant::now();
                            }
                        }
                        did_work = true;
                    }
                }
                Ok(TxCommand::PureSignalReset) => {
                    wdsp_tx.reset_puresignal();
                    pure_signal_last_calibration_count = 0;
                    pure_signal_fault_active = false;
                    did_work = true;
                }
                Ok(TxCommand::ModelChanged) => {
                    if should_defer_model_change(state, defer_model_changes_while_keyed) {
                        println!("saturn-bridge: TX DSP model change deferred until next arm");
                    } else {
                        let model = radio_model.lock_unpoisoned();
                        wdsp_tx.sync_model(&model);
                        two_tone = model.desired.two_tone_enabled;
                    }
                    did_work = true;
                }
                Ok(TxCommand::Shutdown) => {
                    if state != TxState::Idle {
                        do_unkey(
                            session.as_ref(),
                            &radio_model,
                            &mut wdsp_tx,
                            &event_tx,
                            state,
                        );
                    }
                    println!("saturn-bridge: TX thread shutting down");
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if state != TxState::Idle {
                        do_unkey(
                            session.as_ref(),
                            &radio_model,
                            &mut wdsp_tx,
                            &event_tx,
                            state,
                        );
                    }
                    println!("saturn-bridge: TX thread: command channel closed");
                    return;
                }
            }
        }

        if state == TxState::Keyed && !pure_signal_fault_active {
            let model = radio_model.lock_unpoisoned();
            if model.desired.pure_signal_enabled {
                let feedback_stale = pure_signal_last_feedback_at
                    .map(|received_at| received_at.elapsed() > PURE_SIGNAL_FEEDBACK_TIMEOUT)
                    .unwrap_or_else(|| {
                        keyed_at
                            .map(|started_at| started_at.elapsed() > PURE_SIGNAL_FEEDBACK_TIMEOUT)
                            .unwrap_or(false)
                    });
                if feedback_stale {
                    pure_signal_fault_active = true;
                    wdsp_tx.set_puresignal_mox(false);
                    wdsp_tx.reset_puresignal();
                    let _ = event_tx.send(TxEvent::PureSignalStatus(PureSignalStatus {
                        state: PureSignalState::Fault,
                        feedback_level: 0,
                        calibration_count: pure_signal_last_calibration_count,
                        correcting: false,
                        max_tx: 0.0,
                        feedback_packets: pure_signal_feedback_packets,
                        feedback_gaps: pure_signal_feedback_gaps,
                        attenuation_db: model.desired.pure_signal_attenuation_db,
                    }));
                    eprintln!("saturn-bridge: PureSignal feedback timeout; correction bypassed");
                }
            }
        }

        // TX watchdog — auto-unkey after the configured maximum transmit time.
        if state != TxState::Idle && tx_armed_at.elapsed() >= tx_watchdog {
            eprintln!(
                "saturn-bridge: TX watchdog timeout ({}s), auto-unkeying",
                tx_watchdog.as_secs()
            );
            do_unkey(
                session.as_ref(),
                &radio_model,
                &mut wdsp_tx,
                &event_tx,
                state,
            );
            state = TxState::Idle;
            rf_enabled = false;
            two_tone = false;
            key_qualification.reset();
            keepalive_active = false;
            keepalive_resume_frames = 0;
            tx_display_buffer.clear();
            tx_display_peak = 0.0;
            did_work = true;
        }

        // Feed WDSP TX from a steady 48 kHz clock. Browser mic callbacks can
        // arrive with scheduler jitter; queue them here and clock fixed-size
        // blocks into WDSP so DUC IQ leaves at a stable cadence.
        if state == TxState::Armed || state == TxState::Keyed {
            let now = Instant::now();
            if !two_tone && !mic_output_started {
                if pending_mic_samples.len() >= tx_mic_prefill_samples {
                    mic_output_started = true;
                    next_mic_dsp_at = now;
                }
            }
            let mut sent = 0usize;
            while (two_tone || mic_output_started) && now >= next_mic_dsp_at && sent < 8 {
                let mut block = Vec::with_capacity(TX_MIC_SAMPLES_PER_DSP_BLOCK);
                let mut block_underrun = false;
                for _ in 0..TX_MIC_SAMPLES_PER_DSP_BLOCK {
                    block.push(if two_tone {
                        0.0
                    } else {
                        match pending_mic_samples.pop_front() {
                            Some(sample) => {
                                last_mic_output_sample = sample;
                                sample
                            }
                            None => {
                                block_underrun = true;
                                if last_mic_audio_at.elapsed() < TX_SILENCE_GAP {
                                    last_mic_output_sample
                                } else {
                                    0.0
                                }
                            }
                        }
                    });
                }
                if !two_tone && block_underrun {
                    mic_queue_underruns = mic_queue_underruns.saturating_add(1);
                }
                let block_peak = block
                    .iter()
                    .fold(0.0f32, |current, sample| current.max(sample.abs()));
                wdsp_tx.push_mic(&block);
                // Qualify the mic at the point its block enters WDSP, not when
                // a browser callback happens to arrive. This keeps the gate
                // aligned with the IQ that WDSP subsequently produces.
                if qualify_mic_at_dsp_input && block_peak >= TX_KEY_MIC_PEAK_THRESHOLD {
                    last_keyable_mic_at = Some(Instant::now());
                }
                next_mic_dsp_at += mic_block_period;
                sent += 1;
            }
            if sent > 0 {
                if two_tone {
                    last_mic_audio_at = now;
                    keepalive_active = false;
                } else if last_mic_audio_at.elapsed() >= TX_SILENCE_GAP && !keepalive_active {
                    keepalive_active = true;
                    keepalive_resume_frames = 0;
                    println!("saturn-bridge: TX silence fill active");
                }
                did_work = true;
            }
            if sent == 8 && now >= next_mic_dsp_at {
                next_mic_dsp_at = now + mic_block_period;
            }
        }

        if state == TxState::Idle {
            if !wdsp_tx.pending_iq.is_empty() {
                wdsp_tx.pending_iq.clear();
                did_work = true;
            }
        }

        // Drain WDSP TX IQ output to radio as DUC IQ packets. Idle is excluded
        // explicitly so stale WDSP output can never keep P2_app in TX after an
        // unkey or client disconnect.
        if state != TxState::Idle {
            let mut sent_this_loop = 0usize;
            let mut output_fault = false;
            while wdsp_tx.pending_iq.len() >= floats_per_packet
                && Instant::now() >= next_duc_iq_at
                && sent_this_loop < MAX_DUC_PACKETS_PER_LOOP
            {
                let chunk: Vec<f32> = wdsp_tx.pending_iq.drain(..floats_per_packet).collect();
                let mut chunk_consumed_on_key = false;
                next_duc_iq_at += duc_packet_period;
                sent_this_loop += 1;
                let peak = chunk
                    .iter()
                    .fold(0.0f32, |current, sample| current.max(sample.abs()));
                if first_iq_at.is_none() {
                    first_iq_at = Some(Instant::now());
                }
                let iq_is_keyable = duc_iq_packet_can_key_rf(rf_enabled, peak);
                if iq_is_keyable && first_keyable_iq_at.is_none() {
                    first_keyable_iq_at = Some(Instant::now());
                }

                if state == TxState::Armed {
                    // Always publish TX IQ to the browser display, even
                    // before keying — the operator should see the TX
                    // spectrum as soon as MOX is armed.
                    maybe_publish_tx_iq_display(
                        &event_tx,
                        &mut tx_display_buffer,
                        &mut tx_display_peak,
                        tx_display_frame_floats,
                        peak,
                        &chunk,
                    );

                    let mic_recent = last_keyable_mic_at
                        .map(|t| t.elapsed() < keyable_mic_window)
                        .unwrap_or(false);

                    if !rf_enabled {
                        let staged = {
                            let model = radio_model.lock_unpoisoned();
                            session.stage_iq_rf_disabled(&model, &chunk)
                        };
                        if let Err(error) = staged {
                            eprintln!("saturn-bridge: TX RF-disabled DUC staging failed: {error}");
                            output_fault = true;
                            break;
                        }
                        if last_zero_iq_log_at.elapsed() >= TX_ZERO_IQ_LOG_INTERVAL {
                            let diag = wdsp_tx.diagnostics();
                            println!(
                            "saturn-bridge: TX armed with RF disabled; holding off key packet_peak={:.4} input_peak={:.4} output_peak={:.4} wdsp_out_pk={:.1}dB mic_recent={}",
                            peak, diag.input_peak, diag.output_peak, diag.out_peak_db, mic_recent
                        );
                            last_zero_iq_log_at = Instant::now();
                        }
                        did_work = true;
                        continue;
                    }

                    // Gate RF keying on BOTH conditions:
                    //  1. WDSP IQ output exceeds keying threshold (non-zero signal)
                    //  2. A recently processed mic block above the noise floor
                    //  3. The backend's required number of consecutive
                    //     eligible packets (direct XDMA rejects transients)
                    // This prevents keying from WDSP residual filter state or
                    // AMSQ gate leakage when the operator is not speaking.
                    // Two-tone bypasses the mic check since it generates signal
                    // internally via PostGen.
                    let can_key = can_key_rf(rf_enabled, peak, mic_recent, two_tone);
                    let key_is_qualified = key_qualification.observe(can_key);
                    if !key_is_qualified {
                        if last_zero_iq_log_at.elapsed() >= TX_ZERO_IQ_LOG_INTERVAL {
                            let diag = wdsp_tx.diagnostics();
                            println!(
                            "saturn-bridge: TX armed; waiting for qualified mic+IQ packet_peak={:.4} input_peak={:.4} output_peak={:.4} wdsp_out_pk={:.1}dB mic_recent={} iq_keyable={} consecutive={}/{}",
                            peak, diag.input_peak, diag.output_peak, diag.out_peak_db, mic_recent, iq_is_keyable,
                            key_qualification.consecutive, key_qualification.required
                        );
                            last_zero_iq_log_at = Instant::now();
                        }
                        did_work = true;
                        continue;
                    }

                    // First keyable mic+IQ packet — key the radio.
                    let diag = wdsp_tx.diagnostics();
                    let keyed = {
                        let mut model = radio_model.lock_unpoisoned();
                        if model.desired.pure_signal_enabled {
                            if let Err(e) = session.configure_puresignal_feedback() {
                                eprintln!(
                                    "saturn-bridge: PureSignal feedback configuration failed: {e}"
                                );
                                continue;
                            }
                            if let Err(e) = session.send_duc_specific(&model) {
                                eprintln!(
                                    "saturn-bridge: PureSignal TX-specific configuration failed: {e}"
                                );
                                continue;
                            }
                            wdsp_tx.set_puresignal_mox(true);
                        }
                        model.desired.tx_enabled = true;
                        match session.try_key_with_iq(&model, &chunk) {
                            Ok(keyed) => keyed,
                            Err(error) => {
                                model.desired.tx_enabled = false;
                                eprintln!(
                                    "saturn-bridge: TX output failed while staging/keying: {error}"
                                );
                                output_fault = true;
                                false
                            }
                        }
                    };
                    if output_fault {
                        break;
                    }
                    if !keyed {
                        did_work = true;
                        continue;
                    }
                    chunk_consumed_on_key = true;
                    state = TxState::Keyed;
                    keyed_at = Some(Instant::now());
                    // Arming can wait for microphone prefill and backend FIFO
                    // prefill. Those intervals are not missed on-air packet
                    // deadlines: resetting here prevents a direct-XDMA backend
                    // from immediately bursting several artificial catch-up
                    // frames into an already prefilled DUC FIFO. Ordinary
                    // steady-state lateness still uses the bounded catch-up
                    // loop above.
                    next_duc_iq_at = Instant::now() + duc_packet_period;
                    let _ = event_tx.send(TxEvent::Keyed);
                    println!(
                    "saturn-bridge: TX state -> ON (packet_peak={:.4}, input_peak={:.4}, output_peak={:.4}, wdsp_mic_avg={:.1}dB, wdsp_alc_pk={:.1}dB, wdsp_out_avg={:.1}dB)",
                    peak,
                    diag.input_peak,
                    diag.output_peak,
                    diag.mic_avg_db,
                    diag.alc_peak_db,
                    diag.out_avg_db
                );
                }

                maybe_publish_tx_iq_display(
                    &event_tx,
                    &mut tx_display_buffer,
                    &mut tx_display_peak,
                    tx_display_frame_floats,
                    peak,
                    &chunk,
                );
                if !chunk_consumed_on_key {
                    // The transition packet is consumed by try_key_with_iq().
                    // Every later keyed packet uses the steady-state path.
                    if let Err(e) = session.send_duc_iq(&chunk) {
                        eprintln!("saturn-bridge: TX thread: DUC IQ send error: {e}");
                        output_fault = true;
                        break;
                    }
                }
                duc_packet_count = duc_packet_count.saturating_add(1);
                if duc_packet_count == 1 || last_diag_at.elapsed() >= Duration::from_millis(500) {
                    println!(
                        "saturn-bridge: TX diag duc_packet={} packet_peak={:.4}",
                        duc_packet_count, peak
                    );
                    last_diag_at = Instant::now();
                }
                did_work = true;
            }
            if output_fault {
                do_unkey(
                    session.as_ref(),
                    &radio_model,
                    &mut wdsp_tx,
                    &event_tx,
                    state,
                );
                state = TxState::Idle;
                rf_enabled = false;
                two_tone = false;
                key_qualification.reset();
                pending_mic_samples.clear();
                wdsp_tx.pending_iq.clear();
                println!("saturn-bridge: TX output fault forced receive state");
            }
        }

        if state != TxState::Idle && last_diag_event_at.elapsed() >= Duration::from_secs(1) {
            let mic_recent = last_keyable_mic_at
                .map(|t| t.elapsed() < keyable_mic_window)
                .unwrap_or(false);
            publish_tx_diagnostics(
                &event_tx,
                &wdsp_tx,
                state,
                rf_enabled,
                tx_armed_at,
                first_mic_audio_at,
                first_iq_at,
                first_keyable_iq_at,
                mic_recent,
                keyed_at,
                mic_frame_count,
                duc_packet_count,
            );
            last_diag_event_at = Instant::now();
        }

        if !did_work {
            thread::sleep(if state == TxState::Idle {
                Duration::from_millis(1)
            } else {
                TX_ACTIVE_IDLE_SLEEP
            });
        }
    }

    if state != TxState::Idle {
        do_unkey(
            session.as_ref(),
            &radio_model,
            &mut wdsp_tx,
            &event_tx,
            state,
        );
    }
    println!("saturn-bridge: TX thread stopped");
}

#[allow(clippy::too_many_arguments)]
fn publish_tx_diagnostics(
    event_tx: &Sender<TxEvent>,
    wdsp_tx: &WdspTxEngine,
    state: TxState,
    rf_enabled: bool,
    tx_armed_at: Instant,
    first_mic_audio_at: Option<Instant>,
    first_iq_at: Option<Instant>,
    first_keyable_iq_at: Option<Instant>,
    mic_recent: bool,
    keyed_at: Option<Instant>,
    mic_frame_count: u64,
    duc_packet_count: u64,
) {
    let diag = wdsp_tx.diagnostics();
    let _ = event_tx.send(TxEvent::Diagnostics(TxDiagnostics {
        state: state.as_str(),
        rf_enabled,
        mic_frames: mic_frame_count,
        duc_packets: duc_packet_count,
        armed_ms: tx_armed_at.elapsed().as_millis() as u64,
        first_mic_ms: first_mic_audio_at
            .map(|instant| instant.duration_since(tx_armed_at).as_millis() as u64),
        first_iq_ms: first_iq_at
            .map(|instant| instant.duration_since(tx_armed_at).as_millis() as u64),
        first_keyable_iq_ms: first_keyable_iq_at
            .map(|instant| instant.duration_since(tx_armed_at).as_millis() as u64),
        mic_recent,
        keyed_ms: keyed_at.map(|instant| instant.duration_since(tx_armed_at).as_millis() as u64),
        input_peak: diag.input_peak,
        output_peak: diag.output_peak,
        mic_peak_db: diag.mic_peak_db,
        comp_peak_db: diag.comp_peak_db,
        comp_avg_db: diag.comp_avg_db,
        alc_peak_db: diag.alc_peak_db,
        alc_avg_db: diag.alc_avg_db,
        alc_gain_db: diag.alc_gain_db,
        out_peak_db: diag.out_peak_db,
        total_input_samples: diag.total_input_samples,
        total_output_pairs: diag.total_output_pairs,
        pending_mic_floats: diag.pending_mic_floats,
        pending_iq_floats: diag.pending_iq_floats,
    }));
}

fn should_publish_tx_iq_display(
    _frame_peak: f32,
    frame_float_count: usize,
    target_float_count: usize,
) -> bool {
    // Always publish once we have enough samples — the browser display
    // should show the TX noise floor during silence, not go blank.
    frame_float_count >= target_float_count
}

fn maybe_publish_tx_iq_display(
    event_tx: &Sender<TxEvent>,
    display_buffer: &mut Vec<f32>,
    display_peak: &mut f32,
    target_float_count: usize,
    peak: f32,
    chunk: &[f32],
) {
    display_buffer.extend_from_slice(chunk);
    *display_peak = (*display_peak).max(peak);
    if should_publish_tx_iq_display(*display_peak, display_buffer.len(), target_float_count) {
        let _ = event_tx.send(TxEvent::TxIqFrame {
            sample_rate_hz: WDSP_TX_IQ_RATE_HZ,
            iq_samples: display_buffer.clone(),
        });
        display_buffer.clear();
        *display_peak = 0.0;
    } else if display_buffer.len() >= target_float_count {
        display_buffer.clear();
        *display_peak = 0.0;
    }
}

fn tx_watchdog_duration() -> Duration {
    env::var("SATURN_REMOTE_TX_WATCHDOG_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|duration| duration.clamp(MIN_TX_WATCHDOG, MAX_TX_WATCHDOG))
        .unwrap_or(DEFAULT_TX_WATCHDOG)
}

fn tx_mic_prefill_samples() -> usize {
    let prefill_ms = env::var("SATURN_BRIDGE_TX_MIC_PREFILL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    tx_mic_prefill_samples_for_ms(prefill_ms)
}

fn tx_mic_prefill_samples_for_ms(prefill_ms: Option<u64>) -> usize {
    prefill_ms
        .map(|ms| {
            let clamped = ms.clamp(MIN_TX_MIC_PREFILL_MS, MAX_TX_MIC_PREFILL_MS);
            ((clamped * 48_000).div_ceil(1_000)) as usize
        })
        .unwrap_or(DEFAULT_TX_MIC_PREFILL_SAMPLES)
}

fn puresignal_auto_attenuation(feedback_level: i32, current_db: u8) -> u8 {
    if (140..=165).contains(&feedback_level) {
        return current_db.min(31);
    }
    let delta = if feedback_level > 275 {
        15
    } else if feedback_level < 25 {
        -15
    } else {
        (20.0 * (f64::from(feedback_level) / 152.293).log10()).round() as i32
    };
    (i32::from(current_db) + delta).clamp(0, 31) as u8
}

fn puresignal_sequence_gap(expected: u32, received: u32) -> u64 {
    let forward = received.wrapping_sub(expected);
    if forward < (1u32 << 31) {
        forward as u64
    } else {
        // Reordered or duplicate UDP traffic is not a forward packet loss.
        0
    }
}

fn mic_samples_to_mono(samples: Vec<f32>, channels: u32) -> Vec<f32> {
    if channels <= 1 {
        return samples;
    }

    let channel_count = channels as usize;
    samples
        .chunks_exact(channel_count)
        .map(|frame| frame[0])
        .collect()
}

fn do_unkey(
    session: &dyn TxRadio,
    radio_model: &Mutex<RadioModel>,
    wdsp_tx: &mut WdspTxEngine,
    event_tx: &Sender<TxEvent>,
    prev_state: TxState,
) {
    wdsp_tx.set_puresignal_mox(false);
    wdsp_tx.set_active(false);

    if prev_state == TxState::Keyed {
        // Send burst of tx=false high-priority packets for reliability.
        for i in 0..TX_UNKEY_BURST_COUNT {
            {
                let mut model = radio_model.lock_unpoisoned();
                model.desired.tx_enabled = false;
                if let Err(e) = session.send_high_priority(&model) {
                    eprintln!("saturn-bridge: TX thread: HP error on unkey burst {i}: {e}");
                }
            }
            if i == 0 {
                let _ = event_tx.send(TxEvent::Unkeyed);
            }
            if i < TX_UNKEY_BURST_COUNT - 1 {
                thread::sleep(TX_UNKEY_BURST_SPACING);
            }
        }
        println!("saturn-bridge: TX state -> OFF");
    } else if prev_state == TxState::Armed {
        {
            let mut model = radio_model.lock_unpoisoned();
            model.desired.tx_enabled = false;
            if let Err(e) = session.send_high_priority(&model) {
                eprintln!("saturn-bridge: TX thread: HP error on disarm: {e}");
            }
        }
        println!("saturn-bridge: TX disarmed (never keyed)");
        let _ = event_tx.send(TxEvent::Unkeyed);
    }

    let model = radio_model.lock_unpoisoned();
    if let Err(e) = session.configure_rx_ddc(
        model.desired.rx_ddc_index,
        model.desired.ddc0_sample_rate_khz,
        model.desired.ddc0_sample_size_bits,
        model.desired.ddc0_adc,
    ) {
        eprintln!("saturn-bridge: TX thread: RX DDC restore error on unkey: {e}");
    }
    if let Err(e) = session.send_duc_specific(&model) {
        eprintln!("saturn-bridge: TX thread: DUC specific error on unkey: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rf_keying_requires_rf_enabled_and_nonzero_duc_iq() {
        assert!(!duc_iq_packet_can_key_rf(
            false,
            TX_KEY_IQ_PEAK_THRESHOLD * 2.0
        ));
        assert!(!duc_iq_packet_can_key_rf(
            true,
            TX_KEY_IQ_PEAK_THRESHOLD * 0.5
        ));
        assert!(duc_iq_packet_can_key_rf(true, TX_KEY_IQ_PEAK_THRESHOLD));
    }

    #[test]
    fn can_key_rf_requires_iq_and_mic_or_two_tone() {
        let iq_hi = TX_KEY_IQ_PEAK_THRESHOLD * 2.0;
        let iq_lo = TX_KEY_IQ_PEAK_THRESHOLD * 0.5;

        // RF disabled — never key
        assert!(!can_key_rf(false, iq_hi, true, false));
        assert!(!can_key_rf(false, iq_hi, true, true));

        // IQ below threshold — never key
        assert!(!can_key_rf(true, iq_lo, true, false));
        assert!(!can_key_rf(true, iq_lo, true, true));

        // IQ above threshold but no mic and no two-tone — don't key
        assert!(!can_key_rf(true, iq_hi, false, false));

        // IQ above threshold + recent mic — key
        assert!(can_key_rf(true, iq_hi, true, false));

        // IQ above threshold + two-tone (no mic) — key
        assert!(can_key_rf(true, iq_hi, false, true));

        // IQ above threshold + both — key
        assert!(can_key_rf(true, iq_hi, true, true));
    }

    #[test]
    fn key_qualification_requires_consecutive_eligible_packets() {
        let mut qualification = KeyQualification::new(3);
        assert!(!qualification.observe(true));
        assert!(!qualification.observe(true));
        assert!(!qualification.observe(false));
        assert_eq!(qualification.consecutive, 0);
        assert!(!qualification.observe(true));
        assert!(!qualification.observe(true));
        assert!(qualification.observe(true));
        assert!(qualification.observe(true));
    }

    #[test]
    fn key_qualification_clamps_zero_requirement_to_one() {
        let mut qualification = KeyQualification::new(0);
        assert_eq!(qualification.required, 1);
        assert!(qualification.observe(true));
    }

    #[test]
    fn direct_model_changes_are_deferred_only_while_keyed() {
        assert!(!should_defer_model_change(TxState::Idle, true));
        assert!(!should_defer_model_change(TxState::Armed, true));
        assert!(should_defer_model_change(TxState::Keyed, true));
        assert!(!should_defer_model_change(TxState::Keyed, false));
    }

    #[test]
    fn puresignal_auto_attenuation_tracks_feedback_target_window() {
        assert_eq!(puresignal_auto_attenuation(152, 10), 10);
        assert_eq!(puresignal_auto_attenuation(300, 10), 25);
        assert_eq!(puresignal_auto_attenuation(10, 20), 5);
        assert_eq!(puresignal_auto_attenuation(300, 25), 31);
        assert_eq!(puresignal_auto_attenuation(10, 5), 0);
    }

    #[test]
    fn puresignal_gap_counter_handles_wrap_and_reordering() {
        assert_eq!(puresignal_sequence_gap(10, 12), 2);
        assert_eq!(puresignal_sequence_gap(u32::MAX, 0), 1);
        assert_eq!(puresignal_sequence_gap(10, 9), 0);
        assert_eq!(puresignal_sequence_gap(10, 10), 0);
    }

    #[test]
    fn tx_mic_prefill_samples_are_env_tunable_and_clamped() {
        assert_eq!(
            tx_mic_prefill_samples_for_ms(None),
            DEFAULT_TX_MIC_PREFILL_SAMPLES
        );
        assert_eq!(tx_mic_prefill_samples_for_ms(Some(240)), 11_520);
        assert_eq!(tx_mic_prefill_samples_for_ms(Some(1)), 960);
        assert_eq!(tx_mic_prefill_samples_for_ms(Some(1_000)), 12_000);
    }

    #[test]
    fn mic_samples_to_mono_preserves_explicit_mono_frames() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(mic_samples_to_mono(samples.clone(), 1), samples);
    }

    #[test]
    fn mic_samples_to_mono_extracts_left_channel_from_stereo_frames() {
        let samples = vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3];
        assert_eq!(mic_samples_to_mono(samples, 2), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn tx_iq_display_publish_requires_contiguous_frame() {
        let target = DUC_IQ_SAMPLES_PER_PACKET * 2 * TX_DISPLAY_DUC_PACKETS_PER_FRAME;
        assert!(!should_publish_tx_iq_display(
            TX_KEY_IQ_PEAK_THRESHOLD,
            target - 1,
            target
        ));
        assert!(should_publish_tx_iq_display(
            TX_KEY_IQ_PEAK_THRESHOLD,
            target,
            target
        ));
        // Display always publishes regardless of peak — shows noise floor
        assert!(should_publish_tx_iq_display(0.0, target, target));
    }

    #[test]
    fn duc_iq_packet_period_matches_192khz_packet_rate() {
        assert_eq!(duc_iq_packet_period(), Duration::from_micros(1250));
    }
}
