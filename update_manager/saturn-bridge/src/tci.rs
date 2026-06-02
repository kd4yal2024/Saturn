use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::raw::{c_int, c_ulong};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::error::Error as WsError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_with_config, Message};

use crate::config::BridgeConfig;
use crate::radio_model::{AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel};
use crate::tx_codec::{
    tx_codec_frame_is_stale, TxCodecDecoder, TxCodecRuntimeFlags, TxDecodeError, TxMicCodec,
};

#[derive(Clone, Debug)]
pub struct TciMicFrame {
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub sequence: u32,
    pub received_at: Instant,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum TciCommand {
    SetVfoA(u32),
    SetVfoB(u32),
    SetIqCenter(u32),
    SetMode(DemodMode),
    SetFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxAdc(u8),
    SetRxAntenna(u8),
    SetRxVolume(f64),
    SetRxNoiseReductionMode(NoiseReductionMode),
    SetRxNoiseReductionEnabled(bool),
    SetRxNoiseReductionLevel(f64),
    SetRxAnrVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetIqSampleRate(u32),
    SetIqStreaming,
    RequestSmeter,
    SaturnPing {
        client_id: u64,
        nonce: String,
        sent_at: String,
    },
    Phase42SessionOpen {
        client_id: u64,
        session_id: String,
        role: TciClientRole,
    },
    Phase42SessionLane {
        client_id: u64,
        session_id: String,
        lane: Phase42SocketKind,
    },
    SetAudioStreaming(bool),
    SetAudioSampleRate(u32),
    SetAudioFrameSamples(u32),
    SetAudioChannels(u32),
    SetTxEnabled(bool),
    SetNoiseBlankerMode(NoiseBlankerMode),
    SetNoiseBlankerThreshold(f64),
    SetAnfEnabled(bool),
    SetRxAnfVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetAgcMode(AgcMode),
    SetAgcGain(f64),
    SetTxDrive(u8),
    SetTxMicGain(f64),
    SetTxFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxEqEnabled(bool),
    SetRxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxEqEnabled(bool),
    SetTxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxCfcEnabled(bool),
    SetTxCfcPrecomp(f64),
    SetTxCfcBand {
        band: usize,
        gain_db: f64,
    },
    SetTxTwoToneTest(bool),
    SetTxTwoToneFreq1(f64),
    SetTxTwoToneFreq2(f64),
    SetTxTwoToneLevelDb(f64),
    SetTxTwoToneInvertLsb(bool),
    SetTxTwoToneDelayMs(u16),
    SetTxNoiseGateEnabled(bool),
    SetTxNoiseGateThreshold(f64),
    SetRxFftSize(u32),
    SetRxLowLatency(bool),
    SetTxFftSize(u32),
    SetTxLowLatency(bool),
    MicAudioFrame(TciMicFrame),
    ClientConnected,
    ClientDisconnected,
}

#[derive(Clone, Debug)]
enum OutboundMessage {
    Text(String),
    SafetyText(String),
    Close,
    IqFrame {
        receiver: u32,
        sample_rate: u32,
        iq_samples: Vec<f32>,
    },
    TxIqFrame {
        receiver: u32,
        sample_rate: u32,
        iq_samples: Vec<f32>,
    },
    AudioFrame {
        receiver: u32,
        sample_rate: u32,
        channels: u32,
        audio_samples: Vec<f32>,
        sequence: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutboundClass {
    Safety,
    Control,
    Audio,
    Display,
}

impl OutboundClass {
    fn records_enqueue_to_write_latency(self) -> bool {
        matches!(self, Self::Safety | Self::Control)
    }

    fn is_never_drop(self) -> bool {
        matches!(self, Self::Safety | Self::Control)
    }
}

impl OutboundMessage {
    fn class(&self) -> OutboundClass {
        match self {
            Self::Close => OutboundClass::Safety,
            Self::SafetyText(_) => OutboundClass::Safety,
            Self::Text(_) => OutboundClass::Control,
            Self::AudioFrame { .. } => OutboundClass::Audio,
            Self::IqFrame { .. } | Self::TxIqFrame { .. } => OutboundClass::Display,
        }
    }

    fn estimated_bytes(&self) -> usize {
        match self {
            Self::Close => 0,
            Self::Text(text) | Self::SafetyText(text) => text.len(),
            Self::IqFrame { iq_samples, .. } | Self::TxIqFrame { iq_samples, .. } => {
                64 + iq_samples.len() * std::mem::size_of::<f32>()
            }
            Self::AudioFrame { audio_samples, .. } => {
                64 + audio_samples.len() * std::mem::size_of::<f32>()
            }
        }
    }

    fn audio_frame_count(&self) -> usize {
        match self {
            Self::AudioFrame {
                audio_samples,
                channels,
                ..
            } => audio_samples.len() / usize::try_from((*channels).max(1)).unwrap_or(2),
            _ => 0,
        }
    }

    fn audio_sample_rate(&self) -> u32 {
        match self {
            Self::AudioFrame { sample_rate, .. } => *sample_rate,
            _ => 0,
        }
    }

    fn with_audio_sequence(mut self, sequence: u32) -> Self {
        if let Self::AudioFrame {
            sequence: frame_sequence,
            ..
        } = &mut self
        {
            *frame_sequence = sequence;
        }
        self
    }
}

#[derive(Clone, Debug)]
struct QueuedOutbound {
    message: OutboundMessage,
    class: OutboundClass,
    enqueued_at: Instant,
    estimated_bytes: usize,
    audio_frames: usize,
}

impl QueuedOutbound {
    fn new(message: OutboundMessage) -> Self {
        let class = message.class();
        let estimated_bytes = message.estimated_bytes();
        let audio_frames = message.audio_frame_count();
        Self {
            message,
            class,
            enqueued_at: Instant::now(),
            estimated_bytes,
            audio_frames,
        }
    }
}

#[derive(Default, Clone, Debug)]
struct ClientSchedulerStatsDelta {
    safety_latencies_us: Vec<u64>,
    control_latencies_us: Vec<u64>,
    display_replaced: u64,
    display_dropped: u64,
    audio_dropped: u64,
    audio_panic_drain: u64,
    send_blocked_ms: u64,
    outbound_high_watermark_bytes: u64,
    tcp_outq_high_watermark_bytes: u64,
    safety_queue_depth_overflow: u64,
}

#[derive(Default, Debug)]
struct ClientSchedulerStatsInner {
    safety_latencies_us: Vec<u64>,
    control_latencies_us: Vec<u64>,
    display_replaced: u64,
    display_dropped: u64,
    audio_dropped: u64,
    audio_panic_drain: u64,
    send_blocked_ms: u64,
    outbound_high_watermark_bytes: u64,
    tcp_outq_high_watermark_bytes: u64,
    safety_queue_depth_overflow: u64,
}

#[derive(Default, Debug)]
struct ClientSchedulerStats {
    inner: Mutex<ClientSchedulerStatsInner>,
}

impl ClientSchedulerStats {
    fn record_write(&self, class: OutboundClass, latency: Duration) {
        if !class.records_enqueue_to_write_latency() {
            return;
        }
        let latency_us = latency.as_micros().min(u128::from(u64::MAX)) as u64;
        let mut inner = self.inner.lock().unwrap();
        match class {
            OutboundClass::Safety => inner.safety_latencies_us.push(latency_us),
            OutboundClass::Control => inner.control_latencies_us.push(latency_us),
            OutboundClass::Audio | OutboundClass::Display => {}
        }
    }

    fn record_display_replaced(&self) {
        self.inner.lock().unwrap().display_replaced += 1;
    }

    fn record_display_dropped(&self) {
        self.inner.lock().unwrap().display_dropped += 1;
    }

    fn record_audio_dropped(&self, count: u64) {
        self.inner.lock().unwrap().audio_dropped += count;
    }

    fn record_audio_panic_drain(&self) {
        self.inner.lock().unwrap().audio_panic_drain += 1;
    }

    fn record_send_blocked(&self, duration: Duration) {
        self.inner.lock().unwrap().send_blocked_ms += duration.as_millis().max(1) as u64;
    }

    fn record_high_watermark(&self, bytes: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.outbound_high_watermark_bytes = inner.outbound_high_watermark_bytes.max(bytes as u64);
    }

    fn record_tcp_outq_high_watermark(&self, bytes: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.tcp_outq_high_watermark_bytes = inner.tcp_outq_high_watermark_bytes.max(bytes as u64);
    }

    fn record_safety_queue_depth_overflow(&self) {
        self.inner.lock().unwrap().safety_queue_depth_overflow += 1;
    }

    fn drain(&self) -> ClientSchedulerStatsDelta {
        let mut inner = self.inner.lock().unwrap();
        ClientSchedulerStatsDelta {
            safety_latencies_us: std::mem::take(&mut inner.safety_latencies_us),
            control_latencies_us: std::mem::take(&mut inner.control_latencies_us),
            display_replaced: std::mem::take(&mut inner.display_replaced),
            display_dropped: std::mem::take(&mut inner.display_dropped),
            audio_dropped: std::mem::take(&mut inner.audio_dropped),
            audio_panic_drain: std::mem::take(&mut inner.audio_panic_drain),
            send_blocked_ms: std::mem::take(&mut inner.send_blocked_ms),
            outbound_high_watermark_bytes: std::mem::take(&mut inner.outbound_high_watermark_bytes),
            tcp_outq_high_watermark_bytes: std::mem::take(&mut inner.tcp_outq_high_watermark_bytes),
            safety_queue_depth_overflow: std::mem::take(&mut inner.safety_queue_depth_overflow),
        }
    }
}

#[derive(Debug)]
struct OutboundQueues {
    safety: VecDeque<QueuedOutbound>,
    control: VecDeque<QueuedOutbound>,
    audio: VecDeque<QueuedOutbound>,
    display: Option<QueuedOutbound>,
    queued_bytes: usize,
    audio_queued_frames: usize,
    audio_sequence: u32,
    writer_started: bool,
}

impl Default for OutboundQueues {
    fn default() -> Self {
        Self {
            safety: VecDeque::new(),
            control: VecDeque::new(),
            audio: VecDeque::new(),
            display: None,
            queued_bytes: 0,
            audio_queued_frames: 0,
            audio_sequence: 0,
            writer_started: false,
        }
    }
}

#[derive(Debug)]
struct ClientOutbound {
    queues: Mutex<OutboundQueues>,
    stats: ClientSchedulerStats,
}

impl ClientOutbound {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            queues: Mutex::new(OutboundQueues::default()),
            stats: ClientSchedulerStats::default(),
        })
    }

    fn mark_writer_started(&self) {
        self.queues.lock().unwrap().writer_started = true;
    }

    fn enqueue(&self, message: OutboundMessage) -> u64 {
        let mut message = message;
        let class = message.class();
        let mut dropped = 0;
        let mut queues = self.queues.lock().unwrap();

        if class == OutboundClass::Audio {
            queues.audio_sequence = queues.audio_sequence.wrapping_add(1).max(1);
            message = message.with_audio_sequence(queues.audio_sequence);
        }

        let item = QueuedOutbound::new(message);
        match item.class {
            OutboundClass::Safety => {
                queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
                queues.safety.push_back(item);
                if queues.writer_started && queues.safety.len() > 1 {
                    self.stats.record_safety_queue_depth_overflow();
                    eprintln!(
                        "saturn-bridge: safety outbound queue depth is {}",
                        queues.safety.len()
                    );
                }
            }
            OutboundClass::Control => {
                queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
                queues.control.push_back(item);
            }
            OutboundClass::Audio => {
                dropped += self.enqueue_audio_locked(&mut queues, item);
            }
            OutboundClass::Display => {
                if let Some(old) = queues.display.replace(item) {
                    queues.queued_bytes = queues.queued_bytes.saturating_sub(old.estimated_bytes);
                    dropped += 1;
                    self.stats.record_display_replaced();
                }
                if let Some(display) = queues.display.as_ref() {
                    queues.queued_bytes =
                        queues.queued_bytes.saturating_add(display.estimated_bytes);
                }
            }
        }
        self.stats.record_high_watermark(queues.queued_bytes);
        dropped
    }

    fn enqueue_audio_locked(&self, queues: &mut OutboundQueues, item: QueuedOutbound) -> u64 {
        let max_frames = max_audio_queued_frames(item.message.audio_sample_rate());
        let mut dropped = 0;

        if queues.audio_queued_frames >= max_frames && !queues.audio.is_empty() {
            dropped += queues.audio.len() as u64;
            self.stats.record_audio_panic_drain();
            self.stats.record_audio_dropped(queues.audio.len() as u64);
            queues.audio.clear();
            queues.audio_queued_frames = 0;
            queues.queued_bytes = queued_bytes_without_audio(queues);
        }

        while queues.audio_queued_frames.saturating_add(item.audio_frames) > max_frames {
            if let Some(old) = queues.audio.pop_front() {
                queues.audio_queued_frames =
                    queues.audio_queued_frames.saturating_sub(old.audio_frames);
                queues.queued_bytes = queues.queued_bytes.saturating_sub(old.estimated_bytes);
                dropped += 1;
                self.stats.record_audio_dropped(1);
            } else {
                break;
            }
        }

        queues.audio_queued_frames = queues.audio_queued_frames.saturating_add(item.audio_frames);
        queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
        queues.audio.push_back(item);
        dropped
    }

    fn next_message(&self, allow_bulk: bool) -> Option<QueuedOutbound> {
        let mut queues = self.queues.lock().unwrap();
        let item = if let Some(item) = queues.safety.pop_front() {
            Some(item)
        } else if let Some(item) = queues.control.pop_front() {
            Some(item)
        } else if allow_bulk {
            if let Some(item) = queues.audio.pop_front() {
                queues.audio_queued_frames =
                    queues.audio_queued_frames.saturating_sub(item.audio_frames);
                Some(item)
            } else {
                queues.display.take()
            }
        } else {
            None
        };
        if let Some(item) = item.as_ref() {
            queues.queued_bytes = queues.queued_bytes.saturating_sub(item.estimated_bytes);
        }
        item
    }

    fn requeue_front(&self, item: QueuedOutbound) {
        let mut queues = self.queues.lock().unwrap();
        queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
        match item.class {
            OutboundClass::Safety => queues.safety.push_front(item),
            OutboundClass::Control => queues.control.push_front(item),
            OutboundClass::Audio => {
                queues.audio_queued_frames =
                    queues.audio_queued_frames.saturating_add(item.audio_frames);
                queues.audio.push_front(item);
            }
            OutboundClass::Display => {
                if let Some(old) = queues.display.replace(item) {
                    queues.queued_bytes = queues.queued_bytes.saturating_sub(old.estimated_bytes);
                    self.stats.record_display_dropped();
                }
            }
        }
        self.stats.record_high_watermark(queues.queued_bytes);
    }

    fn record_bulk_send_drop(&self, class: OutboundClass) {
        match class {
            OutboundClass::Audio => self.stats.record_audio_dropped(1),
            OutboundClass::Display => self.stats.record_display_dropped(),
            OutboundClass::Safety | OutboundClass::Control => {}
        }
    }

    fn record_write(&self, class: OutboundClass, latency: Duration) {
        self.stats.record_write(class, latency);
    }

    fn record_send_blocked(&self, duration: Duration) {
        self.stats.record_send_blocked(duration);
    }

    fn record_tcp_outq_high_watermark(&self, bytes: usize) {
        self.stats.record_tcp_outq_high_watermark(bytes);
    }

    fn drain_stats(&self) -> ClientSchedulerStatsDelta {
        self.stats.drain()
    }
}

fn max_audio_queued_frames(sample_rate_hz: u32) -> usize {
    let sample_rate = usize::try_from(sample_rate_hz.max(8_000)).unwrap_or(48_000);
    (sample_rate / 4).max(1)
}

fn shape_rx_audio_for_transport(
    samples: &[f32],
    source_rate_hz: u32,
    source_channels: u32,
    target_rate_hz: u32,
    target_channels: u32,
) -> (u32, u32, Vec<f32>) {
    let source_channels = usize::try_from(source_channels.clamp(1, 2)).unwrap_or(2);
    let target_channels = usize::try_from(target_channels.clamp(1, 2)).unwrap_or(2);
    let source_rate_hz = source_rate_hz.clamp(8_000, 48_000);
    let target_rate_hz = target_rate_hz.clamp(8_000, source_rate_hz);
    let source_frames = samples.len() / source_channels;
    if source_frames == 0 {
        return (target_rate_hz, target_channels as u32, Vec::new());
    }

    if source_rate_hz == target_rate_hz && source_channels == target_channels {
        return (target_rate_hz, target_channels as u32, samples.to_vec());
    }

    let target_frames =
        ((source_frames as u64 * target_rate_hz as u64) / source_rate_hz as u64).max(1) as usize;
    let mut output = Vec::with_capacity(target_frames * target_channels);
    for frame in 0..target_frames {
        let src = (frame as f64 * source_rate_hz as f64) / target_rate_hz as f64;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(source_frames - 1);
        let frac = (src - lo as f64) as f32;
        let left_lo = samples[lo * source_channels];
        let left_hi = samples[hi * source_channels];
        let left = left_lo + (left_hi - left_lo) * frac;
        let right = if source_channels > 1 {
            let right_lo = samples[lo * source_channels + 1];
            let right_hi = samples[hi * source_channels + 1];
            right_lo + (right_hi - right_lo) * frac
        } else {
            left
        };

        if target_channels == 1 {
            output.push((left + right) * 0.5);
        } else {
            output.push(left);
            output.push(right);
        }
    }

    (target_rate_hz, target_channels as u32, output)
}

fn display_frame_interval_for_limit(limit_hz: u16) -> Duration {
    if limit_hz == 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(1_000_000_000u64 / u64::from(limit_hz))
    }
}

fn queued_bytes_without_audio(queues: &OutboundQueues) -> usize {
    let safety = queues
        .safety
        .iter()
        .map(|item| item.estimated_bytes)
        .sum::<usize>();
    let control = queues
        .control
        .iter()
        .map(|item| item.estimated_bytes)
        .sum::<usize>();
    let display = queues
        .display
        .as_ref()
        .map(|item| item.estimated_bytes)
        .unwrap_or(0);
    safety.saturating_add(control).saturating_add(display)
}

fn percentile_us(samples: &mut [u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile.min(100)).div_ceil(100);
    samples[index]
}

#[derive(Clone, Debug)]
struct Phase42ClientMetadata {
    session_id: String,
    lane: Option<Phase42SocketKind>,
    role: Option<TciClientRole>,
    ignore_media_until: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Phase42SessionPair {
    session_id: String,
    control_client_id: u64,
    media_client_id: u64,
}

#[derive(Clone, Debug)]
struct ClientState {
    iq_stream_enabled: bool,
    audio_stream_enabled: bool,
    audio_sample_rate_hz: u32,
    audio_frame_float_count: u32,
    audio_channels: u32,
    audio_seq_gap_count: u64,
    tx_uplink_degraded: bool,
    tx_mic_browser_last_seq: u32,
    tx_mic_browser_dropped_count: u64,
    tx_uplink_buffered_bytes: u64,
    tx_uplink_buffered_high_watermark_bytes: u64,
    tx_mic_last_arrived_seq: u32,
    tx_mic_seq_gap_count: u64,
    tx_mic_last_arrived_at: Option<Instant>,
    tx_codec_caps: BTreeSet<TxMicCodec>,
    tx_codec_active: TxMicCodec,
    tx_codec_negotiated_at: Option<Instant>,
    tx_codec_runtime_flags: TxCodecRuntimeFlags,
    tx_codec_decoder: Arc<Mutex<TxCodecDecoder>>,
    tx_codec_degraded: bool,
    tx_codec_decode_error_count: u64,
    tx_codec_decode_error_window_started_at: Option<Instant>,
    tx_codec_decode_error_window_count: u64,
    tx_codec_stale_drop_count: u64,
    tx_codec_release_flush_count: u64,
    phase42: Option<Phase42ClientMetadata>,
}

impl Default for ClientState {
    fn default() -> Self {
        Self::with_tx_codec_runtime_flags(TxCodecRuntimeFlags::default())
    }
}

impl ClientState {
    fn with_tx_codec_runtime_flags(tx_codec_runtime_flags: TxCodecRuntimeFlags) -> Self {
        Self {
            iq_stream_enabled: false,
            audio_stream_enabled: false,
            audio_sample_rate_hz: 48_000,
            audio_frame_float_count: 2048,
            audio_channels: 2,
            audio_seq_gap_count: 0,
            tx_uplink_degraded: false,
            tx_mic_browser_last_seq: 0,
            tx_mic_browser_dropped_count: 0,
            tx_uplink_buffered_bytes: 0,
            tx_uplink_buffered_high_watermark_bytes: 0,
            tx_mic_last_arrived_seq: 0,
            tx_mic_seq_gap_count: 0,
            tx_mic_last_arrived_at: None,
            tx_codec_caps: BTreeSet::from([TxMicCodec::Pcm]),
            tx_codec_active: TxMicCodec::Pcm,
            tx_codec_negotiated_at: None,
            tx_codec_runtime_flags,
            tx_codec_decoder: Arc::new(Mutex::new(TxCodecDecoder::new_with_flags(
                TxMicCodec::Pcm,
                tx_codec_runtime_flags,
            ))),
            tx_codec_degraded: false,
            tx_codec_decode_error_count: 0,
            tx_codec_decode_error_window_started_at: None,
            tx_codec_decode_error_window_count: 0,
            tx_codec_stale_drop_count: 0,
            tx_codec_release_flush_count: 0,
            phase42: None,
        }
    }
}

#[derive(Clone)]
struct ClientConnection {
    outbound: Arc<ClientOutbound>,
    state: ClientState,
}

type ClientRegistry = Arc<Mutex<BTreeMap<u64, ClientConnection>>>;

const MAX_TCI_INBOUND_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_TCI_INBOUND_FRAME_BYTES: usize = 256 * 1024;
const MAX_TCI_MIC_SAMPLES: usize = 32_768;
const BULK_TCP_OUTQ_LIMIT_BYTES: usize = 64 * 1024;
const BULK_BACKPRESSURE_PAUSE_MS: u64 = 10;
const TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT: u64 = 10;
const TX_CODEC_DECODE_ERROR_WINDOW: Duration = Duration::from_secs(1);

#[cfg(target_os = "linux")]
const TIOCOUTQ: c_ulong = 0x5411;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

fn tx_power_trip_fault_message(forward_watts: f32, limit_watts: f32) -> String {
    format!("tx_fault:0,power_trip,{forward_watts:.1},{limit_watts:.1};")
}

fn tx_uplink_late_fault_message(age_ms: u64, limit_ms: u64) -> String {
    format!("tx_fault:0,uplink_late,{age_ms},{limit_ms};")
}

fn tx_control_watchdog_fault_message(silence_ms: u64, limit_ms: u64) -> String {
    format!("tx_fault:0,control_watchdog,{silence_ms},{limit_ms};")
}

fn tx_codec_decode_fault_message(count: u64, limit: u64) -> String {
    format!("tx_fault:0,codec_decode,count={count},limit={limit};")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TciClientRole {
    Operator,
    Viewer,
}

impl TciClientRole {
    fn as_tci(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    fn from_tci(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "viewer" | "view" => Self::Viewer,
            _ => Self::Operator,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase42SocketKind {
    Control,
    Media,
}

impl Phase42SocketKind {
    fn as_tci(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Media => "media",
        }
    }

    fn from_tci(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "control" => Some(Self::Control),
            "media" => Some(Self::Media),
            _ => None,
        }
    }
}

fn remote_client_role_message(client_id: u64, role: TciClientRole) -> String {
    format!("remote_client_role:0,{},{client_id};", role.as_tci())
}

const PHASE42_SESSION_PAIRING_TIMEOUT: Duration = Duration::from_secs(30);
const PHASE42_RELEASE_IGNORE_WINDOW: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase42SessionState {
    WaitingMedia,
    Paired,
    Keyed,
    Terminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase42MediaFrameAction {
    Accept,
    DropNotKeyed,
    DropReleaseWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Phase42DisconnectAction {
    force_rx: bool,
    close_peer_socket: bool,
    state: Phase42SessionState,
}

#[derive(Clone, Debug)]
struct Phase42SplitSession {
    session_id: String,
    state: Phase42SessionState,
    control_connected: bool,
    media_connected: bool,
    created_at: Instant,
    ignore_media_until: Option<Instant>,
    release_window_drops: u64,
}

impl Phase42SplitSession {
    fn new_control(session_id: &str, now: Instant) -> Option<Self> {
        let session_id = normalize_phase42_session_id(session_id)?;
        Some(Self {
            session_id,
            state: Phase42SessionState::WaitingMedia,
            control_connected: true,
            media_connected: false,
            created_at: now,
            ignore_media_until: None,
            release_window_drops: 0,
        })
    }

    fn connect_media(&mut self) -> Option<String> {
        if self.state == Phase42SessionState::Terminated {
            return None;
        }
        self.media_connected = true;
        if self.control_connected {
            self.state = Phase42SessionState::Paired;
            return Some(phase42_session_paired_message(&self.session_id));
        }
        None
    }

    fn pairing_timed_out(&self, now: Instant) -> bool {
        self.state == Phase42SessionState::WaitingMedia
            && now.saturating_duration_since(self.created_at) >= PHASE42_SESSION_PAIRING_TIMEOUT
    }

    fn key(&mut self) -> bool {
        if self.state != Phase42SessionState::Paired {
            return false;
        }
        self.state = Phase42SessionState::Keyed;
        true
    }

    fn release(&mut self, now: Instant) -> bool {
        if self.state != Phase42SessionState::Keyed {
            return false;
        }
        self.state = Phase42SessionState::Paired;
        self.ignore_media_until = Some(now + PHASE42_RELEASE_IGNORE_WINDOW);
        true
    }

    fn media_frame_action(&mut self, now: Instant) -> Phase42MediaFrameAction {
        if self
            .ignore_media_until
            .map(|until| now < until)
            .unwrap_or(false)
        {
            self.release_window_drops = self.release_window_drops.saturating_add(1);
            return Phase42MediaFrameAction::DropReleaseWindow;
        }
        if self.state == Phase42SessionState::Keyed && self.media_connected {
            Phase42MediaFrameAction::Accept
        } else {
            Phase42MediaFrameAction::DropNotKeyed
        }
    }

    fn disconnect_control(&mut self) -> Phase42DisconnectAction {
        self.control_connected = false;
        self.media_connected = false;
        self.state = Phase42SessionState::Terminated;
        Phase42DisconnectAction {
            force_rx: true,
            close_peer_socket: true,
            state: self.state,
        }
    }

    fn disconnect_media(&mut self) -> Phase42DisconnectAction {
        self.media_connected = false;
        let was_keyed = self.state == Phase42SessionState::Keyed;
        if self.state != Phase42SessionState::Terminated {
            self.state = Phase42SessionState::WaitingMedia;
        }
        Phase42DisconnectAction {
            force_rx: was_keyed,
            close_peer_socket: false,
            state: self.state,
        }
    }
}

fn normalize_phase42_session_id(value: &str) -> Option<String> {
    let session_id = sanitize_token(value, 64);
    if session_id.is_empty() {
        None
    } else {
        Some(session_id)
    }
}

fn phase42_session_paired_message(session_id: &str) -> String {
    format!(
        "session_paired:{};",
        normalize_phase42_session_id(session_id).unwrap_or_default()
    )
}

fn parse_phase42_session_open(command: &str) -> Option<(String, TciClientRole)> {
    let command = command.trim().trim_end_matches(';');
    let (name, rest) = command.split_once(':')?;
    if !name.eq_ignore_ascii_case("session_open") {
        return None;
    }
    let mut args = rest.split(',');
    let session_id = normalize_phase42_session_id(args.next().unwrap_or_default())?;
    let role = args
        .next()
        .map(TciClientRole::from_tci)
        .unwrap_or(TciClientRole::Operator);
    Some((session_id, role))
}

fn parse_phase42_session_lane(command: &str) -> Option<(String, Phase42SocketKind)> {
    let command = command.trim().trim_end_matches(';');
    let (name, rest) = command.split_once(':')?;
    if !name.eq_ignore_ascii_case("session_lane") {
        return None;
    }
    let mut args = rest.split(',');
    let session_id = normalize_phase42_session_id(args.next().unwrap_or_default())?;
    let lane = Phase42SocketKind::from_tci(args.next()?)?;
    Some((session_id, lane))
}

pub struct TciFrontend {
    command_rx: Receiver<TciCommand>,
    clients: ClientRegistry,
    operator_client_id: Arc<AtomicU64>,
    operator_control_at: Arc<Mutex<Option<Instant>>>,
    drop_count: Arc<AtomicU64>,
    display_rate_limited_count: AtomicU64,
    // Phase 42 TX media priority: derived from the bridge's authoritative
    // TX intent/armed/keyed state, not from a browser command. While active,
    // RX binary frames are suppressed on the media lane so uplink mic owns the
    // media TCP send buffer. Cleared automatically when TX returns to RX,
    // eliminating the stuck-flag class of bug. Single source of truth.
    tx_media_priority_active: AtomicBool,
    tx_power_meter_scale: f32,
    remote_tx_rf_enabled: bool,
    display_frame_interval: Duration,
    last_display_frame_at: Mutex<Option<Instant>>,
    rx_audio_transport_rate_hz: u32,
    rx_audio_transport_channels: u32,
    _accept_thread: JoinGuard,
}

#[derive(Clone, Copy, Debug)]
pub struct TciClientSnapshot {
    pub active: bool,
    pub iq_stream_enabled: bool,
    pub audio_stream_enabled: bool,
    pub outbound_drops: u64,
    pub safety_enqueue_to_write_p50_us: u64,
    pub safety_enqueue_to_write_p95_us: u64,
    pub safety_enqueue_to_write_p99_us: u64,
    pub control_enqueue_to_write_p50_us: u64,
    pub control_enqueue_to_write_p95_us: u64,
    pub control_enqueue_to_write_p99_us: u64,
    pub display_replaced_per_sec: u64,
    pub display_dropped_per_sec: u64,
    pub audio_dropped_per_sec: u64,
    pub audio_seq_gap_count: u64,
    pub audio_panic_drain_count: u64,
    pub send_blocked_ms: u64,
    pub outbound_high_watermark_bytes: u64,
    pub tcp_outq_high_watermark_bytes: u64,
    pub display_rate_limited_per_sec: u64,
    pub safety_queue_depth_overflow_count: u64,
    pub phase42_control_clients: u64,
    pub phase42_media_clients: u64,
    pub phase42_paired_sessions: u64,
    pub tx_uplink_degraded: bool,
    pub tx_mic_browser_dropped_count: u64,
    pub tx_uplink_buffered_bytes: u64,
    pub tx_uplink_buffered_high_watermark_bytes: u64,
    pub tx_mic_last_arrived_seq: u32,
    pub tx_mic_seq_gap_count: u64,
    pub tx_mic_age_ms: u64,
    pub tx_media_priority_active: bool,
    pub tx_codec_decode_error_count: u64,
    pub tx_codec_stale_drop_count: u64,
    pub tx_codec_release_flush_count: u64,
}

struct JoinGuard {
    #[allow(dead_code)]
    handle: thread::JoinHandle<()>,
}

impl TciFrontend {
    pub fn bind(config: &BridgeConfig, radio_model: Arc<Mutex<RadioModel>>) -> io::Result<Self> {
        let listener = TcpListener::bind(config.tci_bind_addr)?;
        listener.set_nonblocking(true)?;

        let (command_tx, command_rx) = mpsc::channel();
        let clients = Arc::new(Mutex::new(BTreeMap::new()));
        let next_client_id = Arc::new(AtomicU64::new(0));
        let operator_client_id = Arc::new(AtomicU64::new(0));
        let operator_control_at = Arc::new(Mutex::new(None));
        let drop_count = Arc::new(AtomicU64::new(0));
        let remote_tx_rf_enabled = config.remote_tx_rf_enabled;
        let tx_codec_runtime_flags = TxCodecRuntimeFlags {
            opus_decode_enabled: config.tx_opus_decode_enabled,
        };

        let client_registry = clients.clone();
        let next_client = next_client_id.clone();
        let operator_client = operator_client_id.clone();
        let operator_control = operator_control_at.clone();
        let drop_counter = drop_count.clone();
        let radio_model = radio_model.clone();
        let handle = thread::spawn(move || loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let client_id = next_client.fetch_add(1, Ordering::SeqCst) + 1;
                    println!(
                        "saturn-bridge: TCI websocket client {client_id} connected from {addr}"
                    );

                    let command_tx = command_tx.clone();
                    let clients = client_registry.clone();
                    let operator_client_id = operator_client.clone();
                    let operator_control_at = operator_control.clone();
                    let drop_count = drop_counter.clone();
                    let radio_model = radio_model.clone();
                    let tx_codec_runtime_flags = tx_codec_runtime_flags;

                    thread::spawn(move || {
                        handle_client(
                            stream,
                            addr,
                            client_id,
                            &command_tx,
                            &clients,
                            &operator_client_id,
                            &operator_control_at,
                            &radio_model,
                            &drop_count,
                            remote_tx_rf_enabled,
                            tx_codec_runtime_flags,
                        );
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    eprintln!("saturn-bridge: TCI listener error: {error}");
                    thread::sleep(Duration::from_millis(250));
                }
            }
        });

        Ok(Self {
            command_rx,
            clients,
            operator_client_id,
            operator_control_at,
            drop_count,
            display_rate_limited_count: AtomicU64::new(0),
            tx_media_priority_active: AtomicBool::new(false),
            tx_power_meter_scale: config.tx_power_meter_scale,
            remote_tx_rf_enabled,
            display_frame_interval: display_frame_interval_for_limit(config.display_frame_limit_hz),
            last_display_frame_at: Mutex::new(None),
            rx_audio_transport_rate_hz: config.rx_audio_transport_rate_hz,
            rx_audio_transport_channels: config.rx_audio_transport_channels.clamp(1, 2),
            _accept_thread: JoinGuard { handle },
        })
    }

    pub fn try_recv_command(&self) -> Option<TciCommand> {
        self.command_rx.try_recv().ok()
    }

    /// Phase 42: set the bridge's authoritative TX media-priority state.
    /// Called from the main loop from TX intent/armed/keyed state. While true,
    /// RX binary frames (IqFrame, TxIqFrame, AudioFrame) are suppressed on
    /// the media lane so uplink mic owns the media TCP send buffer. The flag
    /// cannot drift out of sync because there is no browser-owned state to
    /// forget to clear.
    pub fn set_tx_media_priority_active(&self, active: bool) {
        self.tx_media_priority_active
            .store(active, Ordering::Relaxed);
    }

    pub fn tx_media_priority_active(&self) -> bool {
        self.tx_media_priority_active.load(Ordering::Relaxed)
    }

    pub fn has_phase42_paired_session(&self) -> bool {
        let clients = self.clients.lock().unwrap();
        phase42_paired_session_count(&clients) > 0
    }

    pub fn last_operator_control_at(&self) -> Option<Instant> {
        *self.operator_control_at.lock().unwrap()
    }

    pub fn clear_phase42_release_window(&self) {
        let operator_client_id = self.operator_client_id.load(Ordering::SeqCst);
        set_phase42_media_ignore_until(&self.clients, operator_client_id, None);
    }

    pub fn mark_phase42_released(&self, now: Instant) {
        let operator_client_id = self.operator_client_id.load(Ordering::SeqCst);
        flush_client_tx_codec_decode_queue(&self.clients, operator_client_id);
        set_phase42_media_ignore_until(
            &self.clients,
            operator_client_id,
            Some(now + PHASE42_RELEASE_IGNORE_WINDOW),
        );
        // Source-of-truth release: bridge clears media priority here so the
        // next send_message call lifts RX media suppression on the media lane.
        // Replaces the previous per-client clear_tx_media_priority_active; the
        // stuck-flag class of bug cannot recur because there is no per-client
        // flag to drift out of sync.
        self.set_tx_media_priority_active(false);
    }

    pub fn client_snapshot(&self) -> TciClientSnapshot {
        let clients = self.clients.lock().unwrap();
        let now = Instant::now();
        let mut safety_latencies_us = Vec::new();
        let mut control_latencies_us = Vec::new();
        let mut display_replaced_per_sec = 0u64;
        let mut display_dropped_per_sec = 0u64;
        let mut audio_dropped_per_sec = 0u64;
        let mut audio_panic_drain_count = 0u64;
        let mut send_blocked_ms = 0u64;
        let mut outbound_high_watermark_bytes = 0u64;
        let mut tcp_outq_high_watermark_bytes = 0u64;
        let mut safety_queue_depth_overflow_count = 0u64;

        for client in clients.values() {
            let delta = client.outbound.drain_stats();
            safety_latencies_us.extend(delta.safety_latencies_us);
            control_latencies_us.extend(delta.control_latencies_us);
            display_replaced_per_sec =
                display_replaced_per_sec.saturating_add(delta.display_replaced);
            display_dropped_per_sec = display_dropped_per_sec.saturating_add(delta.display_dropped);
            audio_dropped_per_sec = audio_dropped_per_sec.saturating_add(delta.audio_dropped);
            audio_panic_drain_count =
                audio_panic_drain_count.saturating_add(delta.audio_panic_drain);
            send_blocked_ms = send_blocked_ms.saturating_add(delta.send_blocked_ms);
            outbound_high_watermark_bytes =
                outbound_high_watermark_bytes.max(delta.outbound_high_watermark_bytes);
            tcp_outq_high_watermark_bytes =
                tcp_outq_high_watermark_bytes.max(delta.tcp_outq_high_watermark_bytes);
            safety_queue_depth_overflow_count =
                safety_queue_depth_overflow_count.saturating_add(delta.safety_queue_depth_overflow);
        }

        TciClientSnapshot {
            active: !clients.is_empty(),
            iq_stream_enabled: clients
                .values()
                .any(|client| client.state.iq_stream_enabled),
            audio_stream_enabled: clients
                .values()
                .any(|client| client.state.audio_stream_enabled),
            outbound_drops: self.drop_count.load(Ordering::Relaxed),
            safety_enqueue_to_write_p50_us: percentile_us(&mut safety_latencies_us, 50),
            safety_enqueue_to_write_p95_us: percentile_us(&mut safety_latencies_us, 95),
            safety_enqueue_to_write_p99_us: percentile_us(&mut safety_latencies_us, 99),
            control_enqueue_to_write_p50_us: percentile_us(&mut control_latencies_us, 50),
            control_enqueue_to_write_p95_us: percentile_us(&mut control_latencies_us, 95),
            control_enqueue_to_write_p99_us: percentile_us(&mut control_latencies_us, 99),
            display_replaced_per_sec,
            display_dropped_per_sec,
            audio_dropped_per_sec,
            audio_seq_gap_count: clients
                .values()
                .map(|client| client.state.audio_seq_gap_count)
                .sum(),
            audio_panic_drain_count,
            send_blocked_ms,
            outbound_high_watermark_bytes,
            tcp_outq_high_watermark_bytes,
            display_rate_limited_per_sec: self
                .display_rate_limited_count
                .swap(0, Ordering::Relaxed),
            safety_queue_depth_overflow_count,
            phase42_control_clients: phase42_lane_client_count(
                &clients,
                Phase42SocketKind::Control,
            ),
            phase42_media_clients: phase42_lane_client_count(&clients, Phase42SocketKind::Media),
            phase42_paired_sessions: phase42_paired_session_count(&clients),
            tx_uplink_degraded: clients
                .values()
                .any(|client| client.state.tx_uplink_degraded),
            tx_mic_browser_dropped_count: clients
                .values()
                .map(|client| client.state.tx_mic_browser_dropped_count)
                .sum(),
            tx_uplink_buffered_bytes: clients
                .values()
                .map(|client| client.state.tx_uplink_buffered_bytes)
                .max()
                .unwrap_or(0),
            tx_uplink_buffered_high_watermark_bytes: clients
                .values()
                .map(|client| client.state.tx_uplink_buffered_high_watermark_bytes)
                .max()
                .unwrap_or(0),
            tx_mic_last_arrived_seq: clients
                .values()
                .map(|client| client.state.tx_mic_last_arrived_seq)
                .max()
                .unwrap_or(0),
            tx_mic_seq_gap_count: clients
                .values()
                .map(|client| client.state.tx_mic_seq_gap_count)
                .sum(),
            tx_mic_age_ms: clients
                .values()
                .filter_map(|client| client.state.tx_mic_last_arrived_at)
                .map(|arrived_at| now.saturating_duration_since(arrived_at).as_millis() as u64)
                .max()
                .unwrap_or(0),
            tx_media_priority_active: self.tx_media_priority_active(),
            tx_codec_decode_error_count: clients
                .values()
                .map(|client| client.state.tx_codec_decode_error_count)
                .sum(),
            tx_codec_stale_drop_count: clients
                .values()
                .map(|client| client.state.tx_codec_stale_drop_count)
                .sum(),
            tx_codec_release_flush_count: clients
                .values()
                .map(|client| client.state.tx_codec_release_flush_count)
                .sum(),
        }
    }

    pub fn publish_radio_state(&self, model: &RadioModel) {
        self.send_text(format!("vfo:0,0,{};", model.desired.vfo_a_hz));
        self.send_text(format!("vfo:0,1,{};", model.desired.vfo_b_hz));
        self.send_text(format!("dds:0,{};", model.desired.iq_center_hz));
        self.send_text(format!("rx_adc:0,{};", model.desired.ddc0_adc));
        self.send_text(format!(
            "rx_antenna:0,{};",
            model.desired.rx_antenna.max(1).min(3)
        ));
        self.send_text(format!(
            "iq_samplerate:{};",
            model.desired.ddc0_sample_rate_khz as u32 * 1000
        ));
        self.send_text(format!("modulation:0,{};", model.desired.mode));
        self.send_text(format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db));
        self.send_text(format!(
            "rx_nr:0,{};",
            model.desired.rx_noise_reduction_mode != NoiseReductionMode::Off
        ));
        self.send_text(format!(
            "rx_nr_mode:0,{};",
            model.desired.rx_noise_reduction_mode
        ));
        self.send_text(format!(
            "rx_nr_level:0,{:.0};",
            model.desired.rx_noise_reduction_level
        ));
        self.send_text(format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ));
        self.send_text(format!("rx_nb:0,{};", model.desired.nb_mode));
        self.send_text(format!(
            "rx_nb_threshold:0,{:.2};",
            model.desired.nb_threshold
        ));
        self.send_text(format!("rx_anr_taps:0,{};", model.desired.rx_anr_taps));
        self.send_text(format!("rx_anr_delay:0,{};", model.desired.rx_anr_delay));
        self.send_text(format!("rx_anr_gain:0,{:.6};", model.desired.rx_anr_gain));
        self.send_text(format!(
            "rx_anr_leakage:0,{:.6};",
            model.desired.rx_anr_leakage
        ));
        self.send_text(format!("rx_anf:0,{};", model.desired.anf_enabled));
        self.send_text(format!("rx_anf_taps:0,{};", model.desired.rx_anf_taps));
        self.send_text(format!("rx_anf_delay:0,{};", model.desired.rx_anf_delay));
        self.send_text(format!("rx_anf_gain:0,{:.6};", model.desired.rx_anf_gain));
        self.send_text(format!(
            "rx_anf_leakage:0,{:.6};",
            model.desired.rx_anf_leakage
        ));
        self.send_text(format!("rx_agc:0,{};", model.desired.agc_mode));
        self.send_text(format!("rx_agc_gain:0,{:.0};", model.desired.agc_gain));
        self.send_text(format!("tx_drive:0,{};", model.desired.tx_drive));
        self.send_text(format!(
            "remote_tx_rf_enabled:0,{};",
            self.remote_tx_rf_enabled
        ));
        self.send_text(format!(
            "tx_mic_gain:0,{:.1};",
            model.desired.tx_mic_gain_db
        ));
        self.send_text(format!("trx:0,{};", model.desired.tx_enabled));
        self.send_text(format!("tx_state:0,{};", model.desired.tx_phase));
        self.send_text(format!(
            "tx_filter_band:0,{},{};",
            model.desired.tx_filter_low_hz, model.desired.tx_filter_high_hz
        ));
        self.send_text(format!("rx_eq_enable:0,{};", model.desired.rx_eq_enabled));
        self.send_text(format!("tx_eq_enable:0,{};", model.desired.tx_eq_enabled));
        for i in 1..=10 {
            self.send_text(format!(
                "rx_eq_band:0,{},{};",
                i, model.desired.rx_eq_bands[i]
            ));
            self.send_text(format!(
                "tx_eq_band:0,{},{};",
                i, model.desired.tx_eq_bands[i]
            ));
        }
        self.send_text(format!("tx_cfc_enable:0,{};", model.desired.cfc_enabled));
        self.send_text(format!(
            "tx_cfc_precomp:0,{:.1};",
            model.desired.cfc_precomp_db
        ));
        for i in 0..10 {
            self.send_text(format!(
                "tx_cfc_band:0,{},{:.1};",
                i + 1,
                model.desired.cfc_bands[i]
            ));
        }
        self.send_text(format!("tx_two_tone:0,{};", model.desired.two_tone_enabled));
        self.send_text(format!(
            "tx_two_tone_freq1:0,{:.0};",
            model.desired.tx_two_tone_freq1_hz
        ));
        self.send_text(format!(
            "tx_two_tone_freq2:0,{:.0};",
            model.desired.tx_two_tone_freq2_hz
        ));
        self.send_text(format!(
            "tx_two_tone_level_db:0,{:.1};",
            model.desired.tx_two_tone_level_db
        ));
        self.send_text(format!(
            "tx_two_tone_invert_lsb:0,{};",
            model.desired.tx_two_tone_invert_lsb
        ));
        self.send_text(format!(
            "tx_two_tone_delay_ms:0,{};",
            model.desired.tx_two_tone_delay_ms
        ));
        self.send_text(format!(
            "tx_noise_gate:0,{};",
            model.desired.tx_noise_gate_enabled
        ));
        self.send_text(format!(
            "tx_noise_gate_threshold:0,{:.1};",
            model.desired.tx_noise_gate_threshold_db
        ));
        self.send_text(format!("rx_fft_size:0,{};", model.desired.rx_fft_size));
        self.send_text(format!(
            "rx_low_latency:0,{};",
            model.desired.rx_low_latency
        ));
        self.send_text(format!("tx_fft_size:0,{};", model.desired.tx_fft_size));
        self.send_text(format!(
            "tx_low_latency:0,{};",
            model.desired.tx_low_latency
        ));
        self.send_text("tune:0,false;".to_string());
        self.publish_telemetry(model);
    }

    pub fn publish_telemetry(&self, model: &RadioModel) {
        if let Some(meter_dbm) = model.observed.ddc0_meter_dbm {
            self.send_text(format!("rx_smeter:0,0,{meter_dbm:.1};"));
        }
        if let Some(packet) = model.observed.high_priority.as_ref() {
            let fwd_watts =
                saturn_adc_to_watts(packet.forward_power, 32, self.tx_power_meter_scale);
            let rev_watts =
                saturn_adc_to_watts(packet.reverse_power, 28, self.tx_power_meter_scale);
            self.send_text(format!("tx_power:0,{:.1};", fwd_watts));
            self.send_text(format!(
                "swr:0,{:.2};",
                calculate_swr_watts(fwd_watts, rev_watts)
            ));
        }
        let drops = self.drop_count.swap(0, Ordering::Relaxed);
        if drops > 0 {
            self.send_text(format!("rx_drops:{drops};"));
        }
    }

    pub fn publish_scheduler_telemetry(&self, snapshot: &TciClientSnapshot) {
        self.send_text(format!(
            "remote_backpressure:0,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{};",
            snapshot.safety_enqueue_to_write_p50_us,
            snapshot.safety_enqueue_to_write_p95_us,
            snapshot.safety_enqueue_to_write_p99_us,
            snapshot.control_enqueue_to_write_p50_us,
            snapshot.control_enqueue_to_write_p95_us,
            snapshot.control_enqueue_to_write_p99_us,
            snapshot.display_replaced_per_sec,
            snapshot.display_dropped_per_sec,
            snapshot.audio_dropped_per_sec,
            snapshot.audio_seq_gap_count,
            snapshot.audio_panic_drain_count,
            snapshot.send_blocked_ms,
            snapshot.outbound_high_watermark_bytes,
            snapshot.safety_queue_depth_overflow_count,
            snapshot.tcp_outq_high_watermark_bytes,
            snapshot.display_rate_limited_per_sec
        ));
    }

    pub fn publish_tx_uplink_telemetry(&self, snapshot: &TciClientSnapshot) {
        self.send_text(format!(
            "remote_tx_uplink:0,{},{},{},{},{},{},{},{},{},{};",
            if snapshot.tx_uplink_degraded { 1 } else { 0 },
            snapshot.tx_mic_browser_dropped_count,
            snapshot.tx_uplink_buffered_bytes,
            snapshot.tx_uplink_buffered_high_watermark_bytes,
            snapshot.tx_mic_last_arrived_seq,
            snapshot.tx_mic_seq_gap_count,
            snapshot.tx_mic_age_ms,
            snapshot.tx_codec_decode_error_count,
            snapshot.tx_codec_stale_drop_count,
            snapshot.tx_codec_release_flush_count
        ));
    }

    pub fn publish_saturn_pong(&self, client_id: u64, nonce: &str, sent_at: &str) {
        self.send_text_to(client_id, format!("saturn_pong:{nonce},{sent_at};"));
    }

    pub fn publish_tx_power_trip(&self, forward_watts: f32, limit_watts: f32) {
        self.send_safety_text(tx_power_trip_fault_message(forward_watts, limit_watts));
    }

    pub fn publish_tx_uplink_late(&self, age_ms: u64, limit_ms: u64) {
        self.send_safety_text(tx_uplink_late_fault_message(age_ms, limit_ms));
    }

    pub fn publish_tx_control_watchdog(&self, silence_ms: u64, limit_ms: u64) {
        self.send_safety_text(tx_control_watchdog_fault_message(silence_ms, limit_ms));
    }

    pub fn publish_iq_frame(&self, sample_rate_hz: u32, iq_samples: &[f32]) {
        if !self.is_iq_stream_enabled() {
            return;
        }
        if !self.should_publish_display_frame() {
            self.display_rate_limited_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.send_message(OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: sample_rate_hz,
            iq_samples: iq_samples.to_vec(),
        });
    }

    pub fn publish_tx_iq_frame(&self, sample_rate_hz: u32, iq_samples: &[f32]) {
        if !self.is_iq_stream_enabled() {
            return;
        }
        if !self.should_publish_display_frame() {
            self.display_rate_limited_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.send_message(OutboundMessage::TxIqFrame {
            receiver: 0,
            sample_rate: sample_rate_hz,
            iq_samples: iq_samples.to_vec(),
        });
    }

    pub fn publish_audio_started(&self, sample_rate_hz: u32) {
        self.send_text("audio_start:0;".to_string());
        self.send_text(format!(
            "audio_samplerate:{};",
            self.audio_transport_sample_rate(sample_rate_hz)
        ));
    }

    pub fn publish_audio_stopped(&self) {
        self.send_text("audio_stop:0;".to_string());
    }

    pub fn publish_audio_frame(&self, sample_rate_hz: u32, audio_samples: &[f32]) {
        if !self.is_audio_stream_enabled() {
            return;
        }

        let (transport_rate_hz, transport_channels, transport_samples) =
            shape_rx_audio_for_transport(
                audio_samples,
                sample_rate_hz,
                2,
                self.audio_transport_sample_rate(sample_rate_hz),
                self.rx_audio_transport_channels,
            );

        self.send_message(OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: transport_rate_hz,
            channels: transport_channels,
            audio_samples: transport_samples,
            sequence: 0,
        });
    }

    fn audio_transport_sample_rate(&self, source_rate_hz: u32) -> u32 {
        self.rx_audio_transport_rate_hz
            .clamp(8_000, source_rate_hz.max(8_000).min(48_000))
    }

    fn is_iq_stream_enabled(&self) -> bool {
        self.clients
            .lock()
            .unwrap()
            .values()
            .any(|client| client.state.iq_stream_enabled)
    }

    fn should_publish_display_frame(&self) -> bool {
        if self.display_frame_interval.is_zero() {
            return true;
        }
        let now = Instant::now();
        let mut last = self.last_display_frame_at.lock().unwrap();
        if last
            .map(|sent_at| now.duration_since(sent_at) >= self.display_frame_interval)
            .unwrap_or(true)
        {
            *last = Some(now);
            true
        } else {
            false
        }
    }

    fn is_audio_stream_enabled(&self) -> bool {
        self.clients
            .lock()
            .unwrap()
            .values()
            .any(|client| client.state.audio_stream_enabled)
    }

    fn send_text(&self, text: String) {
        self.send_message(OutboundMessage::Text(text));
    }

    fn send_safety_text(&self, text: String) {
        self.send_message(OutboundMessage::SafetyText(text));
    }

    fn send_text_to(&self, client_id: u64, text: String) {
        self.send_message_to(client_id, OutboundMessage::Text(text));
    }

    fn send_message_to(&self, client_id: u64, message: OutboundMessage) {
        if let Some(outbound) = self
            .clients
            .lock()
            .unwrap()
            .get(&client_id)
            .map(|client| client.outbound.clone())
        {
            let drops = outbound.enqueue(message);
            self.drop_count.fetch_add(drops, Ordering::Relaxed);
        }
    }

    fn send_message(&self, message: OutboundMessage) {
        let tx_media_priority_active = self.tx_media_priority_active();
        let clients = self.clients.lock().unwrap();
        for client in clients.values() {
            if !client_wants_outbound_message(client, &message, tx_media_priority_active) {
                continue;
            }
            let drops = client.outbound.enqueue(message.clone());
            self.drop_count.fetch_add(drops, Ordering::Relaxed);
        }
    }
}

fn client_wants_outbound_message(
    client: &ClientConnection,
    message: &OutboundMessage,
    tx_media_priority_active: bool,
) -> bool {
    // Phase 42 lane awareness: text goes only to control-lane clients (or
    // legacy non-Phase-42 clients); binary RX frames go only to media-lane
    // clients (or legacy non-Phase-42 clients). Sending text on a media
    // socket or binary on a control socket would be rejected by the
    // browser-side adapter as a protocol violation.
    //
    // While TX media priority is active, binary RX (IQ + audio) is additionally
    // suppressed on the media lane to give uplink mic frames sole ownership of
    // the media TCP send buffer. The bridge derives this from TX intent/armed/
    // keyed state; when it returns to false the suppression lifts automatically.
    let lane = client.state.phase42.as_ref().and_then(|m| m.lane);
    match message {
        OutboundMessage::Close => true,
        OutboundMessage::Text(_) | OutboundMessage::SafetyText(_) => {
            lane != Some(Phase42SocketKind::Media)
        }
        OutboundMessage::IqFrame { .. } | OutboundMessage::TxIqFrame { .. } => {
            lane != Some(Phase42SocketKind::Control)
                && client.state.iq_stream_enabled
                && !tx_media_priority_active
        }
        OutboundMessage::AudioFrame { .. } => {
            lane != Some(Phase42SocketKind::Control)
                && client.state.audio_stream_enabled
                && !tx_media_priority_active
        }
    }
}

fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    client_id: u64,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    operator_control_at: &Arc<Mutex<Option<Instant>>>,
    radio_model: &Arc<Mutex<RadioModel>>,
    drop_count: &Arc<AtomicU64>,
    remote_tx_rf_enabled: bool,
    tx_codec_runtime_flags: TxCodecRuntimeFlags,
) {
    let _ = stream.set_nonblocking(true);
    match accept_with_config(stream, Some(tci_websocket_config())) {
        Ok(mut websocket) => {
            let outbound = ClientOutbound::new();
            let (role, first_client, client_count) = register_client(
                clients,
                operator_client_id,
                client_id,
                outbound.clone(),
                tx_codec_runtime_flags,
            );
            println!(
                "saturn-bridge: TCI client {client_id} assigned {} role ({client_count} connected)",
                role.as_tci()
            );

            for message in initial_snapshot_messages(
                &radio_model.lock().unwrap(),
                remote_tx_rf_enabled,
                client_id,
                role,
            ) {
                let drops = outbound.enqueue(OutboundMessage::Text(message));
                drop_count.fetch_add(drops, Ordering::Relaxed);
            }
            outbound.mark_writer_started();

            if first_client {
                let _ = command_tx.send(TciCommand::ClientConnected);
            }

            let mut bulk_pause_until: Option<Instant> = None;
            loop {
                let mut pending_flush = false;
                let mut client_closed = false;
                for _ in 0..64 {
                    match websocket.read() {
                        Ok(message) => {
                            if !handle_incoming_message(
                                message,
                                command_tx,
                                clients,
                                operator_client_id,
                                operator_control_at,
                                client_id,
                            ) {
                                client_closed = true;
                                break;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                            client_closed = true;
                            break;
                        }
                        Err(error) => {
                            eprintln!(
                                "saturn-bridge: TCI websocket read error from {addr}: {error}"
                            );
                            client_closed = true;
                            break;
                        }
                    }
                }
                if client_closed {
                    break;
                }

                loop {
                    let now = Instant::now();
                    if bulk_pause_until.map(|until| now >= until).unwrap_or(false) {
                        bulk_pause_until = None;
                    }

                    let tcp_outq_bytes = tcp_outq_bytes(websocket.get_ref()).unwrap_or(0);
                    outbound.record_tcp_outq_high_watermark(tcp_outq_bytes);
                    let tcp_outq_allows_bulk = bulk_allowed_for_tcp_outq(tcp_outq_bytes);
                    if !tcp_outq_allows_bulk {
                        outbound.record_send_blocked(Duration::from_millis(2));
                        bulk_pause_until =
                            Some(now + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS));
                    }

                    let allow_bulk = bulk_pause_until.is_none() && tcp_outq_allows_bulk;
                    let Some(item) = outbound.next_message(allow_bulk) else {
                        break;
                    };
                    let closes_client = matches!(&item.message, OutboundMessage::Close);
                    match send_outbound(&mut websocket, &item.message) {
                        Ok(()) => {
                            outbound.record_write(item.class, item.enqueued_at.elapsed());
                            pending_flush = true;
                            if closes_client {
                                client_closed = true;
                                break;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            pending_flush = true;
                            outbound.record_send_blocked(Duration::from_millis(2));
                            bulk_pause_until = Some(
                                Instant::now() + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS),
                            );
                            if item.class.is_never_drop() {
                                outbound.requeue_front(item);
                            } else {
                                outbound.record_bulk_send_drop(item.class);
                                drop_count.fetch_add(1, Ordering::Relaxed);
                            }
                            break;
                        }
                        Err(error) => {
                            eprintln!("saturn-bridge: TCI websocket send error to {addr}: {error}");
                            pending_flush = true;
                            client_closed = true;
                            break;
                        }
                    }
                }
                if client_closed {
                    break;
                }

                if pending_flush {
                    match websocket.flush() {
                        Ok(()) => {
                            if bulk_pause_until
                                .map(|until| Instant::now() >= until)
                                .unwrap_or(false)
                            {
                                bulk_pause_until = None;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            outbound.record_send_blocked(Duration::from_millis(2));
                            bulk_pause_until = Some(
                                Instant::now() + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS),
                            );
                        }
                        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => break,
                        Err(error) => {
                            eprintln!(
                                "saturn-bridge: TCI websocket flush error for {addr}: {error}"
                            );
                            break;
                        }
                    }
                }

                thread::sleep(Duration::from_millis(2));
            }

            let disconnect = unregister_client(clients, operator_client_id, client_id);
            if disconnect.was_operator {
                *operator_control_at.lock().unwrap() = None;
                let _ = command_tx.send(TciCommand::SetTxEnabled(false));
            }
            if disconnect.phase42_media_loss_forces_rx {
                let _ = command_tx.send(TciCommand::SetTxEnabled(false));
            }
            if let Some(peer_id) = disconnect.phase42_closed_peer {
                println!(
                    "saturn-bridge: Phase 42 closed peer media client {peer_id} after control disconnect"
                );
            }
            if let Some(promoted_id) = disconnect.promoted_operator {
                send_role_to_client(
                    clients,
                    promoted_id,
                    TciClientRole::Operator,
                    &format!("saturn-bridge: TCI promoted client {promoted_id} to operator"),
                );
            }
            if disconnect.remaining_clients == 0 {
                let _ = command_tx.send(TciCommand::ClientDisconnected);
            }
            println!(
                "saturn-bridge: TCI client {client_id} disconnected from {addr} ({} connected)",
                disconnect.remaining_clients
            );
        }
        Err(error) => {
            eprintln!("saturn-bridge: TCI websocket accept failed from {addr}: {error}");
        }
    }
}

struct ClientDisconnect {
    was_operator: bool,
    phase42_media_loss_forces_rx: bool,
    phase42_closed_peer: Option<u64>,
    promoted_operator: Option<u64>,
    remaining_clients: usize,
}

fn register_client(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    client_id: u64,
    outbound: Arc<ClientOutbound>,
    tx_codec_runtime_flags: TxCodecRuntimeFlags,
) -> (TciClientRole, bool, usize) {
    let mut clients = clients.lock().unwrap();
    let first_client = clients.is_empty();
    clients.insert(
        client_id,
        ClientConnection {
            outbound,
            state: ClientState::with_tx_codec_runtime_flags(tx_codec_runtime_flags),
        },
    );

    let current_operator = operator_client_id.load(Ordering::SeqCst);
    let role = if current_operator == 0 || !clients.contains_key(&current_operator) {
        operator_client_id.store(client_id, Ordering::SeqCst);
        TciClientRole::Operator
    } else {
        TciClientRole::Viewer
    };
    (role, first_client, clients.len())
}

fn unregister_client(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    client_id: u64,
) -> ClientDisconnect {
    let mut clients = clients.lock().unwrap();
    let current_operator = operator_client_id.load(Ordering::SeqCst);
    let phase42_media_loss_forces_rx =
        phase42_media_client_paired_with_operator_in_clients(&clients, current_operator, client_id);
    let phase42_closed_peer =
        queue_phase42_media_peer_close_for_control_in_clients(&clients, client_id);
    clients.remove(&client_id);

    let was_operator = current_operator == client_id;
    let mut promoted_operator = None;
    if was_operator {
        if let Some((&next_operator, _)) = clients
            .iter()
            .find(|(_, client)| !client_is_phase42_media(client))
        {
            operator_client_id.store(next_operator, Ordering::SeqCst);
            promoted_operator = Some(next_operator);
        } else {
            operator_client_id.store(0, Ordering::SeqCst);
        }
    }

    ClientDisconnect {
        was_operator,
        phase42_media_loss_forces_rx,
        phase42_closed_peer,
        promoted_operator,
        remaining_clients: clients.len(),
    }
}

fn send_role_to_client(
    clients: &ClientRegistry,
    client_id: u64,
    role: TciClientRole,
    log_message: &str,
) {
    if let Some(outbound) = clients
        .lock()
        .unwrap()
        .get(&client_id)
        .map(|client| client.outbound.clone())
    {
        let _ = outbound.enqueue(OutboundMessage::SafetyText(remote_client_role_message(
            client_id, role,
        )));
        println!("{log_message}");
    }
}

fn initial_snapshot_messages(
    model: &RadioModel,
    remote_tx_rf_enabled: bool,
    client_id: u64,
    role: TciClientRole,
) -> Vec<String> {
    vec![
        "ready;".to_string(),
        remote_client_role_message(client_id, role),
        format!("vfo:0,0,{};", model.desired.vfo_a_hz),
        format!("vfo:0,1,{};", model.desired.vfo_b_hz),
        format!("dds:0,{};", model.desired.iq_center_hz),
        format!("rx_adc:0,{};", model.desired.ddc0_adc),
        format!("rx_antenna:0,{};", model.desired.rx_antenna.max(1).min(3)),
        format!(
            "iq_samplerate:{};",
            model.desired.ddc0_sample_rate_khz as u32 * 1000
        ),
        format!("modulation:0,{};", model.desired.mode),
        format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db),
        format!(
            "rx_nr:0,{};",
            model.desired.rx_noise_reduction_mode != NoiseReductionMode::Off
        ),
        format!("rx_nr_mode:0,{};", model.desired.rx_noise_reduction_mode),
        format!(
            "rx_nr_level:0,{:.0};",
            model.desired.rx_noise_reduction_level
        ),
        format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ),
        format!("rx_nb:0,{};", model.desired.nb_mode),
        format!("rx_nb_threshold:0,{:.2};", model.desired.nb_threshold),
        format!("rx_anr_taps:0,{};", model.desired.rx_anr_taps),
        format!("rx_anr_delay:0,{};", model.desired.rx_anr_delay),
        format!("rx_anr_gain:0,{:.6};", model.desired.rx_anr_gain),
        format!("rx_anr_leakage:0,{:.6};", model.desired.rx_anr_leakage),
        format!("rx_anf:0,{};", model.desired.anf_enabled),
        format!("rx_anf_taps:0,{};", model.desired.rx_anf_taps),
        format!("rx_anf_delay:0,{};", model.desired.rx_anf_delay),
        format!("rx_anf_gain:0,{:.6};", model.desired.rx_anf_gain),
        format!("rx_anf_leakage:0,{:.6};", model.desired.rx_anf_leakage),
        format!("rx_agc:0,{};", model.desired.agc_mode),
        format!("rx_agc_gain:0,{:.0};", model.desired.agc_gain),
        format!("tx_drive:0,{};", model.desired.tx_drive),
        format!("remote_tx_rf_enabled:0,{remote_tx_rf_enabled};"),
        format!("tx_mic_gain:0,{:.1};", model.desired.tx_mic_gain_db),
        format!("trx:0,{};", model.desired.tx_enabled),
        format!("tx_state:0,{};", model.desired.tx_phase),
        format!(
            "tx_filter_band:0,{},{};",
            model.desired.tx_filter_low_hz, model.desired.tx_filter_high_hz
        ),
        format!("rx_eq_enable:0,{};", model.desired.rx_eq_enabled),
        format!("tx_eq_enable:0,{};", model.desired.tx_eq_enabled),
    ]
    .into_iter()
    .chain((1..=10).flat_map(|i| {
        vec![
            format!("rx_eq_band:0,{},{};", i, model.desired.rx_eq_bands[i]),
            format!("tx_eq_band:0,{},{};", i, model.desired.tx_eq_bands[i]),
        ]
    }))
    .chain([
        format!("tx_cfc_enable:0,{};", model.desired.cfc_enabled),
        format!("tx_cfc_precomp:0,{:.1};", model.desired.cfc_precomp_db),
    ])
    .chain(
        (0..10usize).map(|i| format!("tx_cfc_band:0,{},{:.1};", i + 1, model.desired.cfc_bands[i])),
    )
    .chain([
        format!("tx_two_tone:0,{};", model.desired.two_tone_enabled),
        format!(
            "tx_two_tone_freq1:0,{:.0};",
            model.desired.tx_two_tone_freq1_hz
        ),
        format!(
            "tx_two_tone_freq2:0,{:.0};",
            model.desired.tx_two_tone_freq2_hz
        ),
        format!(
            "tx_two_tone_level_db:0,{:.1};",
            model.desired.tx_two_tone_level_db
        ),
        format!(
            "tx_two_tone_invert_lsb:0,{};",
            model.desired.tx_two_tone_invert_lsb
        ),
        format!(
            "tx_two_tone_delay_ms:0,{};",
            model.desired.tx_two_tone_delay_ms
        ),
        format!("tx_noise_gate:0,{};", model.desired.tx_noise_gate_enabled),
        format!(
            "tx_noise_gate_threshold:0,{:.1};",
            model.desired.tx_noise_gate_threshold_db
        ),
        format!("rx_fft_size:0,{};", model.desired.rx_fft_size),
        format!("rx_low_latency:0,{};", model.desired.rx_low_latency),
        format!("tx_fft_size:0,{};", model.desired.tx_fft_size),
        format!("tx_low_latency:0,{};", model.desired.tx_low_latency),
        "tune:0,false;".to_string(),
        "audio_samplerate:48000;".to_string(),
    ])
    .collect()
}

fn handle_incoming_message(
    message: Message,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    operator_control_at: &Arc<Mutex<Option<Instant>>>,
    client_id: u64,
) -> bool {
    let current_operator_client_id = operator_client_id.load(Ordering::SeqCst);
    let is_operator = current_operator_client_id == client_id;
    match message {
        Message::Text(text) => {
            if is_operator {
                *operator_control_at.lock().unwrap() = Some(Instant::now());
            }
            for command in text
                .split(';')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                parse_tci_command(command, command_tx, clients, client_id, is_operator);
            }
            true
        }
        Message::Binary(data) => {
            if is_operator
                || phase42_media_client_can_supply_mic(
                    clients,
                    current_operator_client_id,
                    client_id,
                    Instant::now(),
                )
            {
                match parse_tci_mic_frame_result_for_client(clients, client_id, &data) {
                    Ok(frame) => {
                        if tx_codec_frame_is_stale(frame.received_at, Instant::now()) {
                            record_client_tx_codec_stale_drop(clients, client_id);
                            return true;
                        }
                        record_client_tx_mic_frame(
                            clients,
                            client_id,
                            frame.sequence,
                            frame.received_at,
                        );
                        let _ = command_tx.send(TciCommand::MicAudioFrame(frame));
                    }
                    Err(TciMicFrameParseError::NotMicFrame) => {}
                    Err(_) => {
                        let action = record_client_tx_codec_decode_error(clients, client_id);
                        if action.force_rx {
                            send_safety_text_to_client_or_control(
                                clients,
                                client_id,
                                tx_codec_decode_fault_message(action.count, action.limit),
                            );
                            let _ = command_tx.send(TciCommand::SetTxEnabled(false));
                        }
                    }
                }
            }
            true
        }
        Message::Ping(payload) => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
            let _ = payload;
            true
        }
        Message::Close(_) => false,
        _ => true,
    }
}

fn parse_tci_command(
    command: &str,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    client_id: u64,
    allow_control: bool,
) {
    let Some((name, rest)) = command.split_once(':') else {
        return;
    };

    let args: Vec<&str> = rest.split(',').collect();
    let name = name.to_ascii_lowercase();
    if let Some((session_id, role)) = parse_phase42_session_open(command) {
        if set_client_phase42_session_open(clients, client_id, &session_id, role) {
            let _ = command_tx.send(TciCommand::Phase42SessionOpen {
                client_id,
                session_id,
                role,
            });
        }
        return;
    }
    if let Some((session_id, lane)) = parse_phase42_session_lane(command) {
        if set_client_phase42_session_lane(clients, client_id, &session_id, lane) {
            let _ = command_tx.send(TciCommand::Phase42SessionLane {
                client_id,
                session_id,
                lane,
            });
        }
        return;
    }
    if !allow_control && !viewer_tci_command_allowed(&name) {
        return;
    }
    match name.as_str() {
        "vfo" => {
            if args.len() >= 3 {
                if let Ok(freq_hz) = args[2].trim().parse::<u32>() {
                    let which = args[1].trim();
                    let _ = match which {
                        "0" => command_tx.send(TciCommand::SetVfoA(freq_hz)),
                        "1" => command_tx.send(TciCommand::SetVfoB(freq_hz)),
                        _ => Ok(()),
                    };
                }
            }
        }
        "dds" => {
            if args.len() >= 2 {
                if let Ok(freq_hz) = args[1].trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqCenter(freq_hz));
                }
            }
        }
        "saturn_ping" => {
            if args.len() >= 2 {
                let nonce = sanitize_token(args[0], 32);
                let sent_at = sanitize_token(args[1], 32);
                if !nonce.is_empty() && !sent_at.is_empty() {
                    let _ = command_tx.send(TciCommand::SaturnPing {
                        client_id,
                        nonce,
                        sent_at,
                    });
                }
            }
        }
        "modulation" => {
            if args.len() >= 2 {
                let _ = command_tx.send(TciCommand::SetMode(DemodMode::from_tci(args[1])));
            }
        }
        "rx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_volume" => {
            let volume_arg = if args.len() >= 3 {
                args.get(2)
            } else if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(volume_text) = volume_arg {
                if let Ok(volume_db) = volume_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxVolume(volume_db));
                }
            }
        }
        "rx_nr_mode" | "nr_mode" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetRxNoiseReductionMode(
                    NoiseReductionMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nr" | "nr" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionEnabled(enabled));
                }
            }
        }
        "rx_nr_level" | "nr_level" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionLevel(level));
                }
            }
        }
        "rx_anr_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_adc" => {
            let adc_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(adc_text) = adc_arg {
                if let Ok(adc) = adc_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAdc(adc.min(2)));
                }
            }
        }
        "rx_antenna" => {
            let antenna_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(antenna_text) = antenna_arg {
                if let Ok(antenna) = antenna_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAntenna(antenna.clamp(1, 3)));
                }
            }
        }
        "iq_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqSampleRate(rate_hz));
                }
            }
        }
        "iq_start" => {
            set_client_iq_stream_enabled(clients, client_id, true);
            println!("saturn-bridge: TCI iq_start requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "iq_stop" => {
            set_client_iq_stream_enabled(clients, client_id, false);
            println!("saturn-bridge: TCI iq_stop requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "audio_start" => {
            let audio_enabled = set_client_audio_stream_enabled(clients, client_id, true);
            println!("saturn-bridge: TCI audio_start requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(audio_enabled));
        }
        "audio_stop" => {
            let audio_enabled = set_client_audio_stream_enabled(clients, client_id, false);
            println!("saturn-bridge: TCI audio_stop requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(audio_enabled));
        }
        "audio_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    set_client_audio_sample_rate(clients, client_id, rate_hz);
                    let _ = command_tx.send(TciCommand::SetAudioSampleRate(rate_hz));
                }
            }
        }
        "audio_stream_samples" => {
            if let Some(sample_text) = args.first() {
                if let Ok(sample_count) = sample_text.trim().parse::<u32>() {
                    set_client_audio_frame_float_count(clients, client_id, sample_count);
                    let _ = command_tx.send(TciCommand::SetAudioFrameSamples(sample_count));
                }
            }
        }
        "audio_stream_channels" => {
            if let Some(channel_text) = args.first() {
                if let Ok(channels) = channel_text.trim().parse::<u32>() {
                    set_client_audio_channels(clients, client_id, channels);
                    let _ = command_tx.send(TciCommand::SetAudioChannels(channels));
                }
            }
        }
        "audio_seq_gap_count" => {
            let gap_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gap_text) = gap_arg {
                if let Ok(gaps) = gap_text.trim().parse::<u64>() {
                    set_client_audio_seq_gap_count(clients, client_id, gaps);
                }
            }
        }
        "tx_uplink_stats" => {
            let offset = if args.len() >= 6 { 1 } else { 0 };
            let degraded = args
                .get(offset)
                .and_then(|value| parse_tci_bool(value))
                .unwrap_or(false);
            let last_seq = args
                .get(offset + 1)
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let dropped_count = args
                .get(offset + 2)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let buffered_bytes = args
                .get(offset + 3)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let high_watermark_bytes = args
                .get(offset + 4)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(buffered_bytes);
            set_client_tx_uplink_stats(
                clients,
                client_id,
                degraded,
                last_seq,
                dropped_count,
                buffered_bytes,
                high_watermark_bytes,
            );
        }
        "tx_codec_caps" => {
            let caps = parse_tx_codec_caps_args(&args);
            let selected = set_client_tx_codec_caps(clients, client_id, caps.clone());
            if let Some(codec) = selected {
                send_text_to_client(clients, client_id, tx_codec_accept_message(codec));
            } else {
                let requested = caps.iter().next().copied().unwrap_or(TxMicCodec::Pcm);
                send_text_to_client(
                    clients,
                    client_id,
                    tx_codec_reject_message(requested, "unsupported"),
                );
            }
        }
        "remote_tx_media_priority" => {
            // Phase 42: TX media priority is now derived from the bridge's
            // authoritative on-air state, not from this browser command.
            // Accept and ignore for backward compatibility with older
            // clients; no side effect.
            let _ = args;
        }
        "audio_stream_sample_type" => {}
        "rx_smeter" | "s_meter" | "smeter" => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
        }
        "trx" => {
            // trx:0,true or trx:0,true,tci — PTT on/off
            // Phase 42: no longer sets a per-client tx_media_priority flag.
            // The bridge derives media priority from its own model and main.rs
            // calls tci.set_tx_media_priority_active(...) accordingly. This
            // eliminates the stuck-flag bug class.
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    println!("saturn-bridge: TCI trx requested -> {}", enabled);
                    if enabled {
                        reset_client_tx_uplink_attempt(clients, client_id);
                    }
                    let _ = command_tx.send(TciCommand::SetTxEnabled(enabled));
                }
            }
        }
        "rx_nb" => {
            // rx_nb:0,mode — NB mode: 0=OFF, 1=NB1, 2=NB2
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetNoiseBlankerMode(
                    NoiseBlankerMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nb_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh) = thresh_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetNoiseBlankerThreshold(thresh));
                }
            }
        }
        "rx_anf" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetAnfEnabled(enabled));
                }
            }
        }
        "rx_anf_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_agc" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetAgcMode(AgcMode::from_tci(mode_text)));
            }
        }
        "rx_agc_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetAgcGain(gain));
                }
            }
        }
        "tx_drive" => {
            let drive_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(drive_text) = drive_arg {
                if let Ok(drive) = drive_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetTxDrive(drive));
                }
            }
        }
        "tx_mic_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain_db) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxMicGain(gain_db));
                }
            }
        }
        "tx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetTxFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxEqEnabled(enabled));
                }
            }
        }
        "tx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxEqEnabled(enabled));
                }
            }
        }
        "rx_eq_band" => {
            // rx_eq_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB integers
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetRxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_eq_band" => {
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_cfc_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxCfcEnabled(enabled));
                }
            }
        }
        "tx_cfc_precomp" => {
            let precomp_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(precomp_text) = precomp_arg {
                if let Ok(db) = precomp_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxCfcPrecomp(db));
                }
            }
        }
        "tx_cfc_band" => {
            // tx_cfc_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<f64>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxCfcBand { band, gain_db });
                    }
                }
            }
        }
        "tx_two_tone" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneTest(enabled));
                }
            }
        }
        "tx_two_tone_freq1" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq1(freq_hz));
                }
            }
        }
        "tx_two_tone_freq2" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq2(freq_hz));
                }
            }
        }
        "tx_two_tone_level_db" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level_db) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneLevelDb(level_db));
                }
            }
        }
        "tx_two_tone_invert_lsb" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneInvertLsb(enabled));
                }
            }
        }
        "tx_two_tone_delay_ms" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay_ms) = delay_text.trim().parse::<u16>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneDelayMs(delay_ms));
                }
            }
        }
        "tx_noise_gate" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateEnabled(enabled));
                }
            }
        }
        "tx_noise_gate_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh_db) = thresh_text.trim().parse::<f64>() {
                    let clamped = thresh_db.clamp(-80.0, 0.0);
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateThreshold(clamped));
                }
            }
        }
        "rx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetRxFftSize(size));
                }
            }
        }
        "rx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetRxLowLatency(val));
                }
            }
        }
        "tx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetTxFftSize(size));
                }
            }
        }
        "tx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetTxLowLatency(val));
                }
            }
        }
        _ => {}
    }
}

fn viewer_tci_command_allowed(name: &str) -> bool {
    matches!(
        name,
        "saturn_ping"
            | "iq_start"
            | "iq_stop"
            | "audio_start"
            | "audio_stop"
            | "audio_samplerate"
            | "audio_stream_samples"
            | "audio_stream_channels"
            | "audio_seq_gap_count"
            | "audio_stream_sample_type"
            | "rx_smeter"
            | "s_meter"
            | "smeter"
    )
}

// Phase 42 lane-aware routing helper. Given a control-lane client_id,
// returns the paired media-lane client_id if both halves of the split
// session are connected and registered. Used to propagate RX stream-enable
// state from the control client (which receives iq_start/audio_start text)
// to the media client (which is the destination for binary RX frames).
// Returns None for non-Phase-42 clients, unpaired sessions, or when called
// on a media-lane client.
fn phase42_paired_media_client_id(
    clients: &BTreeMap<u64, ClientConnection>,
    control_client_id: u64,
) -> Option<u64> {
    let metadata = clients.get(&control_client_id)?.state.phase42.as_ref()?;
    if metadata.lane != Some(Phase42SocketKind::Control) {
        return None;
    }
    let pair = phase42_session_pair_in_clients(clients, &metadata.session_id)?;
    if pair.control_client_id != control_client_id {
        return None;
    }
    Some(pair.media_client_id)
}

fn set_client_iq_stream_enabled(clients: &ClientRegistry, client_id: u64, enabled: bool) -> bool {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.iq_stream_enabled = enabled;
    }
    // Phase 42: mirror to the paired media client so binary IQ frames have
    // a destination on the media lane. client_wants_outbound_message then
    // routes RX IQ to the media client only.
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.iq_stream_enabled = enabled;
        }
    }
    clients
        .values()
        .any(|client| client.state.iq_stream_enabled)
}

fn set_client_audio_stream_enabled(
    clients: &ClientRegistry,
    client_id: u64,
    enabled: bool,
) -> bool {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_stream_enabled = enabled;
    }
    // Phase 42: mirror to the paired media client.
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_stream_enabled = enabled;
        }
    }
    clients
        .values()
        .any(|client| client.state.audio_stream_enabled)
}

fn set_client_audio_sample_rate(clients: &ClientRegistry, client_id: u64, sample_rate_hz: u32) {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_sample_rate_hz = sample_rate_hz;
    }
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_sample_rate_hz = sample_rate_hz;
        }
    }
}

fn set_client_audio_frame_float_count(clients: &ClientRegistry, client_id: u64, sample_count: u32) {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_frame_float_count = sample_count;
    }
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_frame_float_count = sample_count;
        }
    }
}

fn set_client_audio_channels(clients: &ClientRegistry, client_id: u64, channels: u32) {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_channels = channels;
    }
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_channels = channels;
        }
    }
}

fn set_client_audio_seq_gap_count(clients: &ClientRegistry, client_id: u64, gaps: u64) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.audio_seq_gap_count = gaps;
    }
}

fn set_client_phase42_session_open(
    clients: &ClientRegistry,
    client_id: u64,
    session_id: &str,
    role: TciClientRole,
) -> bool {
    let Some(session_id) = normalize_phase42_session_id(session_id) else {
        return false;
    };
    let mut clients = clients.lock().unwrap();
    let Some(client) = clients.get_mut(&client_id) else {
        return false;
    };
    let metadata = client
        .state
        .phase42
        .get_or_insert_with(|| Phase42ClientMetadata {
            session_id: session_id.clone(),
            lane: None,
            role: None,
            ignore_media_until: None,
        });
    if metadata.session_id != session_id {
        return false;
    }
    metadata.role = Some(role);
    true
}

fn set_client_phase42_session_lane(
    clients: &ClientRegistry,
    client_id: u64,
    session_id: &str,
    lane: Phase42SocketKind,
) -> bool {
    let Some(session_id) = normalize_phase42_session_id(session_id) else {
        return false;
    };
    let mut clients = clients.lock().unwrap();
    let Some(client) = clients.get_mut(&client_id) else {
        return false;
    };
    let metadata = client
        .state
        .phase42
        .get_or_insert_with(|| Phase42ClientMetadata {
            session_id: session_id.clone(),
            lane: None,
            role: None,
            ignore_media_until: None,
        });
    if metadata.session_id != session_id {
        return false;
    }
    metadata.lane = Some(lane);
    true
}

fn phase42_session_pair_for_client(
    clients: &ClientRegistry,
    client_id: u64,
) -> Option<Phase42SessionPair> {
    let clients = clients.lock().unwrap();
    let session_id = clients
        .get(&client_id)?
        .state
        .phase42
        .as_ref()?
        .session_id
        .clone();
    phase42_session_pair_in_clients(&clients, &session_id)
}

fn phase42_media_client_can_supply_mic(
    clients: &ClientRegistry,
    operator_client_id: u64,
    media_client_id: u64,
    now: Instant,
) -> bool {
    if operator_client_id == 0 || operator_client_id == media_client_id {
        return false;
    }
    let clients = clients.lock().unwrap();
    let Some(pair) = phase42_session_pair_for_client_in_clients(&clients, media_client_id) else {
        return false;
    };
    if pair.control_client_id != operator_client_id || pair.media_client_id != media_client_id {
        return false;
    }
    !clients
        .get(&media_client_id)
        .and_then(|client| client.state.phase42.as_ref())
        .and_then(|metadata| metadata.ignore_media_until)
        .map(|until| now < until)
        .unwrap_or(false)
}

fn phase42_media_client_paired_with_operator_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    operator_client_id: u64,
    media_client_id: u64,
) -> bool {
    if operator_client_id == 0 || operator_client_id == media_client_id {
        return false;
    }
    phase42_session_pair_for_client_in_clients(clients, media_client_id)
        .map(|pair| {
            pair.control_client_id == operator_client_id && pair.media_client_id == media_client_id
        })
        .unwrap_or(false)
}

fn queue_phase42_media_peer_close_for_control_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    control_client_id: u64,
) -> Option<u64> {
    let metadata = clients.get(&control_client_id)?.state.phase42.as_ref()?;
    if metadata.lane != Some(Phase42SocketKind::Control) {
        return None;
    }
    let pair = phase42_session_pair_in_clients(clients, &metadata.session_id)?;
    if pair.control_client_id != control_client_id {
        return None;
    }
    let media = clients.get(&pair.media_client_id)?;
    let _ = media.outbound.enqueue(OutboundMessage::Close);
    Some(pair.media_client_id)
}

fn client_is_phase42_media(client: &ClientConnection) -> bool {
    client
        .state
        .phase42
        .as_ref()
        .and_then(|metadata| metadata.lane)
        == Some(Phase42SocketKind::Media)
}

fn phase42_session_pair_for_client_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    client_id: u64,
) -> Option<Phase42SessionPair> {
    let session_id = clients
        .get(&client_id)?
        .state
        .phase42
        .as_ref()?
        .session_id
        .clone();
    phase42_session_pair_in_clients(clients, &session_id)
}

fn set_phase42_media_ignore_until(
    clients: &ClientRegistry,
    operator_client_id: u64,
    ignore_until: Option<Instant>,
) -> u64 {
    if operator_client_id == 0 {
        return 0;
    }
    let mut clients = clients.lock().unwrap();
    let Some(operator_session_id) = clients
        .get(&operator_client_id)
        .and_then(|client| client.state.phase42.as_ref())
        .filter(|metadata| metadata.lane == Some(Phase42SocketKind::Control))
        .map(|metadata| metadata.session_id.clone())
    else {
        return 0;
    };

    let mut updated = 0;
    for client in clients.values_mut() {
        let Some(metadata) = client.state.phase42.as_mut() else {
            continue;
        };
        if metadata.session_id == operator_session_id
            && metadata.lane == Some(Phase42SocketKind::Media)
        {
            metadata.ignore_media_until = ignore_until;
            updated += 1;
        }
    }
    updated
}

fn phase42_session_pair_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    session_id: &str,
) -> Option<Phase42SessionPair> {
    let mut control_client_id = None;
    let mut media_client_id = None;
    for (&client_id, client) in clients {
        let Some(metadata) = client.state.phase42.as_ref() else {
            continue;
        };
        if metadata.session_id != session_id {
            continue;
        }
        match metadata.lane {
            Some(Phase42SocketKind::Control) => control_client_id.get_or_insert(client_id),
            Some(Phase42SocketKind::Media) => media_client_id.get_or_insert(client_id),
            None => continue,
        };
    }
    Some(Phase42SessionPair {
        session_id: session_id.to_string(),
        control_client_id: control_client_id?,
        media_client_id: media_client_id?,
    })
}

fn phase42_lane_client_count(
    clients: &BTreeMap<u64, ClientConnection>,
    lane: Phase42SocketKind,
) -> u64 {
    clients
        .values()
        .filter(|client| {
            client
                .state
                .phase42
                .as_ref()
                .and_then(|metadata| metadata.lane)
                == Some(lane)
        })
        .count() as u64
}

fn phase42_paired_session_count(clients: &BTreeMap<u64, ClientConnection>) -> u64 {
    let mut control_sessions = BTreeSet::new();
    let mut media_sessions = BTreeSet::new();
    for client in clients.values() {
        let Some(metadata) = client.state.phase42.as_ref() else {
            continue;
        };
        match metadata.lane {
            Some(Phase42SocketKind::Control) => {
                control_sessions.insert(metadata.session_id.clone());
            }
            Some(Phase42SocketKind::Media) => {
                media_sessions.insert(metadata.session_id.clone());
            }
            None => {}
        }
    }
    control_sessions.intersection(&media_sessions).count() as u64
}

fn set_client_tx_uplink_stats(
    clients: &ClientRegistry,
    client_id: u64,
    degraded: bool,
    last_seq: u32,
    dropped_count: u64,
    buffered_bytes: u64,
    high_watermark_bytes: u64,
) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.tx_uplink_degraded = degraded;
        client.state.tx_mic_browser_last_seq = last_seq;
        client.state.tx_mic_browser_dropped_count = dropped_count;
        client.state.tx_uplink_buffered_bytes = buffered_bytes;
        client.state.tx_uplink_buffered_high_watermark_bytes =
            high_watermark_bytes.max(buffered_bytes);
    }
}

fn parse_tx_codec_caps_args(args: &[&str]) -> BTreeSet<TxMicCodec> {
    let offset = if args
        .first()
        .map(|value| value.trim() == "0")
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    args.iter()
        .skip(offset)
        .filter_map(|value| TxMicCodec::from_tci(value))
        .collect()
}

fn select_tx_codec(caps: &BTreeSet<TxMicCodec>, flags: TxCodecRuntimeFlags) -> Option<TxMicCodec> {
    if flags.opus_decode_enabled {
        if caps.contains(&TxMicCodec::OpusWb) {
            return Some(TxMicCodec::OpusWb);
        }
        if caps.contains(&TxMicCodec::OpusNb) {
            return Some(TxMicCodec::OpusNb);
        }
    }
    caps.contains(&TxMicCodec::Pcm).then_some(TxMicCodec::Pcm)
}

fn tx_codec_accept_message(codec: TxMicCodec) -> String {
    format!("tx_codec_accept:0,{};", codec.as_tci())
}

fn tx_codec_reject_message(codec: TxMicCodec, reason: &str) -> String {
    format!(
        "tx_codec_reject:0,{},{};",
        codec.as_tci(),
        sanitize_token(reason, 48)
    )
}

fn reset_client_tx_codec_state(
    client: &mut ClientConnection,
    caps: BTreeSet<TxMicCodec>,
    selected: Option<TxMicCodec>,
    now: Instant,
) {
    client.state.tx_codec_caps = caps;
    client.state.tx_codec_decode_error_window_started_at = None;
    client.state.tx_codec_decode_error_window_count = 0;
    if let Some(codec) = selected {
        client.state.tx_codec_active = codec;
        client.state.tx_codec_negotiated_at = Some(now);
        client.state.tx_codec_decoder = Arc::new(Mutex::new(TxCodecDecoder::new_with_flags(
            codec,
            client.state.tx_codec_runtime_flags,
        )));
        client.state.tx_codec_degraded = false;
    } else {
        client.state.tx_codec_negotiated_at = None;
    }
}

fn set_client_tx_codec_caps(
    clients: &ClientRegistry,
    client_id: u64,
    caps: BTreeSet<TxMicCodec>,
) -> Option<TxMicCodec> {
    let mut clients = clients.lock().unwrap();
    let flags = clients
        .get(&client_id)
        .map(|client| client.state.tx_codec_runtime_flags)
        .unwrap_or_default();
    let selected = select_tx_codec(&caps, flags);
    let now = Instant::now();
    if let Some(client) = clients.get_mut(&client_id) {
        reset_client_tx_codec_state(client, caps.clone(), selected, now);
    }
    // Phase 42: codec negotiation happens on the control lane, but TX mic
    // binary frames arrive on the paired media lane. Mirror the accepted state
    // so the media client owns the decoder that will actually consume frames.
    if let Some(media_id) = phase42_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            reset_client_tx_codec_state(media, caps, selected, now);
        }
    }
    selected
}

fn send_text_to_client(clients: &ClientRegistry, client_id: u64, text: String) {
    if let Some(outbound) = clients
        .lock()
        .unwrap()
        .get(&client_id)
        .map(|client| client.outbound.clone())
    {
        let _ = outbound.enqueue(OutboundMessage::Text(text));
    }
}

fn send_safety_text_to_client_or_control(clients: &ClientRegistry, client_id: u64, text: String) {
    let outbound = {
        let clients = clients.lock().unwrap();
        let target_client_id = clients
            .get(&client_id)
            .and_then(|client| client.state.phase42.as_ref())
            .filter(|metadata| metadata.lane == Some(Phase42SocketKind::Media))
            .and_then(|metadata| {
                phase42_session_pair_in_clients(&clients, &metadata.session_id)
                    .map(|pair| pair.control_client_id)
            })
            .unwrap_or(client_id);
        clients
            .get(&target_client_id)
            .map(|client| client.outbound.clone())
    };
    if let Some(outbound) = outbound {
        let _ = outbound.enqueue(OutboundMessage::SafetyText(text));
    }
}

fn reset_client_tx_uplink_attempt(clients: &ClientRegistry, client_id: u64) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.tx_uplink_degraded = false;
        client.state.tx_mic_browser_last_seq = 0;
        client.state.tx_mic_browser_dropped_count = 0;
        client.state.tx_uplink_buffered_bytes = 0;
        client.state.tx_uplink_buffered_high_watermark_bytes = 0;
        client.state.tx_mic_last_arrived_seq = 0;
        client.state.tx_mic_seq_gap_count = 0;
        client.state.tx_mic_last_arrived_at = None;
        client.state.tx_codec_decode_error_window_started_at = None;
        client.state.tx_codec_decode_error_window_count = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TxCodecDecodeFaultAction {
    force_rx: bool,
    count: u64,
    limit: u64,
}

fn record_client_tx_codec_decode_error(
    clients: &ClientRegistry,
    client_id: u64,
) -> TxCodecDecodeFaultAction {
    record_client_tx_codec_decode_error_at(clients, client_id, Instant::now())
}

fn record_client_tx_codec_decode_error_at(
    clients: &ClientRegistry,
    client_id: u64,
    now: Instant,
) -> TxCodecDecodeFaultAction {
    let limit = TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT;
    let mut count = 0;
    let mut force_rx = false;
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.tx_codec_decode_error_count =
            client.state.tx_codec_decode_error_count.saturating_add(1);
        let reset_window = client
            .state
            .tx_codec_decode_error_window_started_at
            .map(|started_at| {
                now.saturating_duration_since(started_at) > TX_CODEC_DECODE_ERROR_WINDOW
            })
            .unwrap_or(true);
        if reset_window {
            client.state.tx_codec_decode_error_window_started_at = Some(now);
            client.state.tx_codec_decode_error_window_count = 0;
        }
        client.state.tx_codec_decode_error_window_count = client
            .state
            .tx_codec_decode_error_window_count
            .saturating_add(1);
        count = client.state.tx_codec_decode_error_window_count;
        if count >= limit && !client.state.tx_codec_degraded {
            client.state.tx_codec_degraded = true;
            force_rx = true;
        }
    }
    TxCodecDecodeFaultAction {
        force_rx,
        count,
        limit,
    }
}

fn record_client_tx_codec_stale_drop(clients: &ClientRegistry, client_id: u64) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.tx_codec_stale_drop_count =
            client.state.tx_codec_stale_drop_count.saturating_add(1);
    }
}

fn flush_client_tx_codec_decode_queue(clients: &ClientRegistry, operator_client_id: u64) -> bool {
    if operator_client_id == 0 {
        return false;
    }
    let mut clients = clients.lock().unwrap();
    let Some(operator) = clients.get_mut(&operator_client_id) else {
        return false;
    };
    operator.state.tx_codec_release_flush_count = operator
        .state
        .tx_codec_release_flush_count
        .saturating_add(1);
    true
}

fn next_wrapped_sequence(sequence: u32) -> u32 {
    let next = sequence.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

fn record_client_tx_mic_frame(
    clients: &ClientRegistry,
    client_id: u64,
    sequence: u32,
    received_at: Instant,
) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.tx_mic_last_arrived_at = Some(received_at);
        if sequence == 0 {
            return;
        }
        if client.state.tx_mic_last_arrived_seq != 0
            && sequence != next_wrapped_sequence(client.state.tx_mic_last_arrived_seq)
        {
            client.state.tx_mic_seq_gap_count = client.state.tx_mic_seq_gap_count.saturating_add(1);
        }
        client.state.tx_mic_last_arrived_seq = sequence;
    }
}

/// Parse a TCI binary frame that contains TX mic audio from the client.
/// Frame layout: 64-byte header + LE samples.
///   header[8..12]  = sample_type  (u32 LE); 1=s16, 3=float32, 0=legacy float32
///   header[20..24] = sample_count (u32 LE)
///   header[24..28] = stream_type  (u32 LE); must be 2 (TX mic)
///   header[28..32] = channels     (u32 LE); 1=mono, 2=stereo
///   header[32..36] = tx_mic_seq   (u32 LE); 0 means legacy/unknown
///   header[36..40] = codec_id     (u32 LE); 0=PCM, reserved for Phase 44 Opus
///   header[40..44] = payload_bytes (u32 LE); 0 means legacy/full payload
///
/// stream_type == 1 is intentionally excluded: it is the RX audio type used by
/// the server→client direction and must not be fed into the TX DSP path.
#[cfg(test)]
fn parse_tci_mic_frame(data: &[u8]) -> Option<TciMicFrame> {
    parse_tci_mic_frame_result(data).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TciMicFrameParseError {
    NotMicFrame,
    Malformed,
    UnsupportedCodec,
    Decode(TxDecodeError),
}

struct TciMicFrameParts<'a> {
    sample_rate_hz: u32,
    sample_type: u32,
    channels: u32,
    sequence: u32,
    codec: TxMicCodec,
    sample_count: usize,
    payload: &'a [u8],
    declared_payload_bytes: usize,
}

fn parse_tci_mic_frame_parts(data: &[u8]) -> Result<TciMicFrameParts<'_>, TciMicFrameParseError> {
    if data.len() < 64 {
        return Err(TciMicFrameParseError::Malformed);
    }
    let sample_rate_hz = u32::from_le_bytes(
        data[4..8]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let sample_type = u32::from_le_bytes(
        data[8..12]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let stream_type = u32::from_le_bytes(
        data[24..28]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    if stream_type != 2 {
        return Err(TciMicFrameParseError::NotMicFrame);
    }
    let raw_channels = u32::from_le_bytes(
        data[28..32]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let channels = match raw_channels {
        0 | 1 => 1,
        _ => 2,
    };
    let sequence = u32::from_le_bytes(
        data[32..36]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let codec_id = u32::from_le_bytes(
        data[36..40]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let codec = TxMicCodec::from_id(codec_id).ok_or(TciMicFrameParseError::UnsupportedCodec)?;
    let declared_payload_bytes = u32::from_le_bytes(
        data[40..44]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    ) as usize;
    let sample_count = u32::from_le_bytes(
        data[20..24]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    ) as usize;
    if sample_count == 0 || sample_count > MAX_TCI_MIC_SAMPLES {
        return Err(TciMicFrameParseError::Malformed);
    }
    let raw_payload = &data[64..];
    if declared_payload_bytes > raw_payload.len() {
        return Err(TciMicFrameParseError::Malformed);
    }
    let payload = if declared_payload_bytes == 0 {
        raw_payload
    } else {
        &raw_payload[..declared_payload_bytes]
    };
    Ok(TciMicFrameParts {
        sample_rate_hz,
        sample_type,
        channels,
        sequence,
        codec,
        sample_count,
        payload,
        declared_payload_bytes,
    })
}

fn decode_tci_mic_frame_parts(
    parts: TciMicFrameParts<'_>,
    decoder: &mut TxCodecDecoder,
) -> Result<TciMicFrame, TciMicFrameParseError> {
    if decoder.codec() != parts.codec {
        return Err(TciMicFrameParseError::Decode(TxDecodeError::CodecMismatch));
    }
    let samples = decoder
        .decode(
            parts.sample_type,
            parts.sample_count,
            parts.payload,
            parts.declared_payload_bytes,
        )
        .map_err(TciMicFrameParseError::Decode)?;
    Ok(TciMicFrame {
        sample_rate_hz: parts.sample_rate_hz,
        channels: parts.channels,
        sequence: parts.sequence,
        received_at: Instant::now(),
        samples,
    })
}

#[cfg(test)]
fn parse_tci_mic_frame_result(data: &[u8]) -> Result<TciMicFrame, TciMicFrameParseError> {
    let parts = parse_tci_mic_frame_parts(data)?;
    let mut decoder = TxCodecDecoder::new(parts.codec);
    decode_tci_mic_frame_parts(parts, &mut decoder)
}

fn parse_tci_mic_frame_result_for_client(
    clients: &ClientRegistry,
    client_id: u64,
    data: &[u8],
) -> Result<TciMicFrame, TciMicFrameParseError> {
    let parts = parse_tci_mic_frame_parts(data)?;
    let decoder = {
        let clients = clients.lock().unwrap();
        let Some(client) = clients.get(&client_id) else {
            return Err(TciMicFrameParseError::Malformed);
        };
        if client.state.tx_codec_active != parts.codec {
            return Err(TciMicFrameParseError::Decode(TxDecodeError::CodecMismatch));
        }
        client.state.tx_codec_decoder.clone()
    };
    let mut decoder = decoder.lock().unwrap();
    decode_tci_mic_frame_parts(parts, &mut decoder)
}

fn tci_websocket_config() -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(MAX_TCI_INBOUND_MESSAGE_BYTES),
        max_frame_size: Some(MAX_TCI_INBOUND_FRAME_BYTES),
        ..WebSocketConfig::default()
    }
}

fn bulk_allowed_for_tcp_outq(tcp_outq_bytes: usize) -> bool {
    tcp_outq_bytes < BULK_TCP_OUTQ_LIMIT_BYTES
}

#[cfg(target_os = "linux")]
fn tcp_outq_bytes(stream: &TcpStream) -> io::Result<usize> {
    let mut bytes: c_int = 0;
    let result = unsafe { ioctl(stream.as_raw_fd(), TIOCOUTQ, &mut bytes) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(bytes.max(0) as usize)
    }
}

#[cfg(not(target_os = "linux"))]
fn tcp_outq_bytes(_stream: &TcpStream) -> io::Result<usize> {
    Ok(0)
}

fn send_outbound(
    websocket: &mut tungstenite::WebSocket<std::net::TcpStream>,
    message: &OutboundMessage,
) -> Result<(), WsError> {
    match message {
        OutboundMessage::Close => websocket.send(Message::Close(None)),
        OutboundMessage::Text(text) | OutboundMessage::SafetyText(text) => {
            websocket.send(Message::Text(text.clone()))
        }
        OutboundMessage::IqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(build_tci_iq_frame(
            *receiver,
            *sample_rate,
            iq_samples,
        ))),
        OutboundMessage::TxIqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(build_tci_tx_iq_frame(
            *receiver,
            *sample_rate,
            iq_samples,
        ))),
        OutboundMessage::AudioFrame {
            receiver,
            sample_rate,
            channels,
            audio_samples,
            sequence,
        } => websocket.send(Message::Binary(build_tci_audio_frame(
            *receiver,
            *sample_rate,
            *channels,
            audio_samples,
            *sequence,
        ))),
    }
}

fn build_tci_iq_frame(receiver: u32, sample_rate: u32, iq_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 0, 2, 0)
}

fn build_tci_tx_iq_frame(receiver: u32, sample_rate: u32, iq_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 3, 2, 0)
}

fn build_tci_audio_frame(
    receiver: u32,
    sample_rate: u32,
    channels: u32,
    audio_samples: &[f32],
    sequence: u32,
) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, audio_samples, 1, channels, sequence)
}

fn build_tci_float_frame(
    receiver: u32,
    sample_rate: u32,
    samples: &[f32],
    stream_type: u32,
    channels: u32,
    sequence: u32,
) -> Vec<u8> {
    let mut frame = vec![0u8; 64 + samples.len() * 4];
    write_u32_le(&mut frame, 0, receiver);
    write_u32_le(&mut frame, 4, sample_rate);
    write_u32_le(&mut frame, 8, 3);
    write_u32_le(&mut frame, 12, 0);
    write_u32_le(&mut frame, 16, 0);
    write_u32_le(&mut frame, 20, samples.len() as u32);
    write_u32_le(&mut frame, 24, stream_type);
    write_u32_le(&mut frame, 28, channels);
    write_u32_le(&mut frame, 32, sequence);

    for (index, value) in samples.iter().enumerate() {
        let offset = 64 + index * 4;
        frame[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    frame
}

fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn parse_tci_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn sanitize_token(text: &str, max_len: usize) -> String {
    text.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .take(max_len)
        .collect()
}

/// Convert a Saturn G2 raw power ADC reading to watts.
/// Uses the ANAN-7000/Saturn 100 W PA calibration constants from pihpsdr:
///   ADC_REF = 5.0 V, coupling = 0.12 (fwd) / 0.12 (rev), fwd_offset = 32, rev_offset = 28.
/// Formula: V = ((raw - offset) / 4095) * 5.0;  watts = V² / 0.12
fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (raw as i32 - offset).max(0) as f32;
    let v = (corrected / 4095.0) * 5.0;
    ((v * v) / 0.12) * scale
}

fn calculate_swr_watts(fwd_watts: f32, rev_watts: f32) -> f32 {
    if fwd_watts <= 0.0 || rev_watts <= 0.0 || rev_watts >= fwd_watts {
        return 1.0;
    }
    let ratio = (rev_watts / fwd_watts).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}

#[allow(dead_code)]
fn calculate_swr(forward: u16, reverse: u16) -> f32 {
    if forward == 0 || reverse == 0 || reverse >= forward {
        return 1.0;
    }

    let ratio = (reverse as f32 / forward as f32).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx_codec::{TX_MIC_CODEC_PCM_ID, TX_SAMPLE_TYPE_FLOAT32, TX_SAMPLE_TYPE_S16};

    fn test_client_registry(client_id: u64) -> ClientRegistry {
        let mut clients = BTreeMap::new();
        clients.insert(
            client_id,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        Arc::new(Mutex::new(clients))
    }

    #[test]
    fn builds_iq_frame_with_expected_header() {
        let frame = build_tci_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 0);
    }

    #[test]
    fn builds_tx_iq_frame_with_distinct_stream_type() {
        let frame = build_tci_tx_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn builds_audio_frame_with_expected_header() {
        let frame = build_tci_audio_frame(0, 48_000, 1, &[0.25, -0.25, 0.5, -0.5], 7);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(frame[32..36].try_into().unwrap()), 7);
    }

    #[test]
    fn shapes_rx_audio_for_wan_transport() {
        let input = [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0];
        let (rate, channels, output) = shape_rx_audio_for_transport(&input, 48_000, 2, 12_000, 1);
        assert_eq!(rate, 12_000);
        assert_eq!(channels, 1);
        assert_eq!(output.len(), 1);
        assert!((output[0] - 0.5).abs() < 0.001);
    }

    #[test]
    fn outbound_scheduler_prioritizes_safety_and_control_over_display() {
        let outbound = ClientOutbound::new();
        outbound.enqueue(OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        });
        outbound.enqueue(OutboundMessage::Text("rx_smeter:0,0,-110.0;".to_string()));
        outbound.enqueue(OutboundMessage::SafetyText(
            "tx_fault:0,power_trip,126.3,110.0;".to_string(),
        ));

        let safety = outbound.next_message(true).unwrap();
        assert_eq!(safety.class, OutboundClass::Safety);
        let control = outbound.next_message(true).unwrap();
        assert_eq!(control.class, OutboundClass::Control);
        let display = outbound.next_message(true).unwrap();
        assert_eq!(display.class, OutboundClass::Display);
    }

    #[test]
    fn outbound_scheduler_treats_snapshot_rf_state_as_control() {
        assert_eq!(
            OutboundMessage::Text("remote_tx_rf_enabled:0,false;".to_string()).class(),
            OutboundClass::Control
        );
        assert_eq!(
            OutboundMessage::SafetyText("remote_tx_rf_enabled:0,false;".to_string()).class(),
            OutboundClass::Safety
        );
    }

    #[test]
    fn outbound_scheduler_replaces_display_depth_one() {
        let outbound = ClientOutbound::new();
        assert_eq!(
            outbound.enqueue(OutboundMessage::IqFrame {
                receiver: 0,
                sample_rate: 48_000,
                iq_samples: vec![1.0, 2.0],
            }),
            0
        );
        assert_eq!(
            outbound.enqueue(OutboundMessage::IqFrame {
                receiver: 0,
                sample_rate: 96_000,
                iq_samples: vec![3.0, 4.0],
            }),
            1
        );
        let item = outbound.next_message(true).unwrap();
        match item.message {
            OutboundMessage::IqFrame { sample_rate, .. } => assert_eq!(sample_rate, 96_000),
            _ => panic!("expected display frame"),
        }
        let delta = outbound.drain_stats();
        assert_eq!(delta.display_replaced, 1);
    }

    #[test]
    fn outbound_scheduler_panic_drains_stale_audio() {
        let outbound = ClientOutbound::new();
        let audio = vec![0.0; max_audio_queued_frames(8_000) * 2];
        assert_eq!(
            outbound.enqueue(OutboundMessage::AudioFrame {
                receiver: 0,
                sample_rate: 8_000,
                channels: 2,
                audio_samples: audio.clone(),
                sequence: 0,
            }),
            0
        );
        assert_eq!(
            outbound.enqueue(OutboundMessage::AudioFrame {
                receiver: 0,
                sample_rate: 8_000,
                channels: 2,
                audio_samples: audio,
                sequence: 0,
            }),
            1
        );
        let item = outbound.next_message(true).unwrap();
        match item.message {
            OutboundMessage::AudioFrame { sequence, .. } => assert_eq!(sequence, 2),
            _ => panic!("expected audio frame"),
        }
        let delta = outbound.drain_stats();
        assert_eq!(delta.audio_panic_drain, 1);
        assert_eq!(delta.audio_dropped, 1);
    }

    #[test]
    fn bulk_tcp_outq_guard_blocks_bulk_at_limit() {
        assert!(bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES - 1));
        assert!(!bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES));
        assert!(!bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES * 2));
    }

    #[test]
    fn outbound_scheduler_tracks_tcp_outq_high_watermark() {
        let outbound = ClientOutbound::new();
        outbound.record_tcp_outq_high_watermark(32 * 1024);
        outbound.record_tcp_outq_high_watermark(96 * 1024);
        outbound.record_tcp_outq_high_watermark(64 * 1024);
        let delta = outbound.drain_stats();
        assert_eq!(delta.tcp_outq_high_watermark_bytes, 96 * 1024);
    }

    #[test]
    fn display_frame_interval_supports_limit_and_disable() {
        assert_eq!(display_frame_interval_for_limit(0), Duration::ZERO);
        assert_eq!(
            display_frame_interval_for_limit(25),
            Duration::from_millis(40)
        );
        assert_eq!(
            display_frame_interval_for_limit(50),
            Duration::from_millis(20)
        );
    }

    #[test]
    fn rejects_oversized_tci_mic_frames() {
        let sample_count = (MAX_TCI_MIC_SAMPLES + 1) as u32;
        let mut frame = vec![0u8; 64];
        write_u32_le(&mut frame, 20, sample_count);
        write_u32_le(&mut frame, 24, 2);
        frame.resize(64 + sample_count as usize * 4, 0);
        assert!(parse_tci_mic_frame(&frame).is_none());
    }

    #[test]
    fn parses_mono_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 8];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 32, 77);
        frame[64..68].copy_from_slice(&0.25f32.to_le_bytes());
        frame[68..72].copy_from_slice(&(-0.5f32).to_le_bytes());

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.sequence, 77);
        assert_eq!(parsed.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn parses_stereo_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 16];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_FLOAT32);
        write_u32_le(&mut frame, 20, 4);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 2);
        for (index, sample) in [0.25f32, -0.25, 0.5, -0.5].iter().enumerate() {
            let offset = 64 + index * 4;
            frame[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
        }

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 2);
        assert_eq!(parsed.sequence, 0);
        assert_eq!(parsed.samples, vec![0.25, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn parses_s16_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 6];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 3);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 32, 78);
        for (index, sample) in [8192i16, -16384, 32767].iter().enumerate() {
            let offset = 64 + index * 2;
            frame[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
        }

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.sequence, 78);
        assert_eq!(parsed.samples.len(), 3);
        assert!((parsed.samples[0] - 0.25).abs() < 0.0001);
        assert!((parsed.samples[1] + 0.5).abs() < 0.0001);
        assert!((parsed.samples[2] - 0.9999).abs() < 0.0001);
    }

    #[test]
    fn parses_phase44_pcm_tci_mic_frame_with_codec_header() {
        let mut frame = vec![0u8; 64 + 4];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 32, 79);
        write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
        write_u32_le(&mut frame, 40, 4);
        for (index, sample) in [8192i16, -16384].iter().enumerate() {
            let offset = 64 + index * 2;
            frame[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
        }

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.sequence, 79);
        assert_eq!(parsed.samples.len(), 2);
        assert!((parsed.samples[0] - 0.25).abs() < 0.0001);
        assert!((parsed.samples[1] + 0.5).abs() < 0.0001);
    }

    #[test]
    fn rejects_phase44_mic_frame_with_unsupported_codec() {
        let mut frame = vec![0u8; 64 + 2];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 1);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 36, 2);
        write_u32_le(&mut frame, 40, 2);

        assert!(parse_tci_mic_frame(&frame).is_none());
    }

    #[test]
    fn rejects_phase44_pcm_mic_frame_with_payload_size_mismatch() {
        let mut frame = vec![0u8; 64 + 4];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
        write_u32_le(&mut frame, 40, 2);

        assert!(parse_tci_mic_frame(&frame).is_none());
    }

    #[test]
    fn rejects_unknown_tci_mic_sample_type() {
        let mut frame = vec![0u8; 64 + 4];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, 99);
        write_u32_le(&mut frame, 20, 1);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);

        assert!(parse_tci_mic_frame(&frame).is_none());
    }

    #[test]
    fn websocket_config_limits_inbound_message_size() {
        let config = tci_websocket_config();
        assert_eq!(config.max_message_size, Some(MAX_TCI_INBOUND_MESSAGE_BYTES));
        assert_eq!(config.max_frame_size, Some(MAX_TCI_INBOUND_FRAME_BYTES));
    }

    #[test]
    fn swr_formula_is_reasonable() {
        assert!((calculate_swr(1000, 0) - 1.0).abs() < 0.01);
        assert!(calculate_swr(1000, 250) > 1.0);
    }

    #[test]
    fn parses_boolish_tci_values() {
        assert_eq!(parse_tci_bool("true"), Some(true));
        assert_eq!(parse_tci_bool("0"), Some(false));
        assert_eq!(parse_tci_bool("bogus"), None);
    }

    #[test]
    fn parses_saturn_ping_command() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(7);

        parse_tci_command("saturn_ping:probe-1,123.456;", &tx, &clients, 7, false);

        match rx.try_recv().unwrap() {
            TciCommand::SaturnPing {
                client_id,
                nonce,
                sent_at,
            } => {
                assert_eq!(client_id, 7);
                assert_eq!(nonce, "probe-1");
                assert_eq!(sent_at, "123.456");
            }
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn phase44_tx_codec_caps_accepts_pcm_scaffold() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(7);

        parse_tci_command("tx_codec_caps:0,pcm;", &tx, &clients, 7, true);

        assert!(rx.try_recv().is_err());
        let outbound = {
            let clients = clients.lock().unwrap();
            let client = clients.get(&7).unwrap();
            assert!(client.state.tx_codec_caps.contains(&TxMicCodec::Pcm));
            assert_eq!(client.state.tx_codec_active, TxMicCodec::Pcm);
            assert!(client.state.tx_codec_negotiated_at.is_some());
            client.outbound.clone()
        };
        let queued = outbound.next_message(true).unwrap();
        match queued.message {
            OutboundMessage::Text(text) => assert_eq!(text, "tx_codec_accept:0,pcm;"),
            other => panic!("unexpected outbound: {other:?}"),
        }
    }

    #[test]
    fn phase44_tx_codec_caps_mirror_from_control_to_paired_media() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(73);
        clients.lock().unwrap().insert(
            74,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );

        parse_tci_command("session_lane:phase-44,control;", &tx, &clients, 73, true);
        parse_tci_command("session_lane:phase-44,media;", &tx, &clients, 74, false);
        while rx.try_recv().is_ok() {}

        parse_tci_command("tx_codec_caps:0,pcm;", &tx, &clients, 73, true);

        let clients = clients.lock().unwrap();
        let control = clients.get(&73).unwrap();
        let media = clients.get(&74).unwrap();
        assert_eq!(control.state.tx_codec_active, TxMicCodec::Pcm);
        assert_eq!(media.state.tx_codec_active, TxMicCodec::Pcm);
        assert!(media.state.tx_codec_negotiated_at.is_some());
        assert_eq!(
            media.state.tx_codec_decoder.lock().unwrap().codec(),
            TxMicCodec::Pcm
        );
    }

    #[test]
    fn phase44_tx_codec_caps_rejects_non_pcm_until_decoder_exists() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(7);

        parse_tci_command("tx_codec_caps:0,opus_wb;", &tx, &clients, 7, true);

        assert!(rx.try_recv().is_err());
        let outbound = {
            let clients = clients.lock().unwrap();
            let client = clients.get(&7).unwrap();
            assert!(client.state.tx_codec_caps.contains(&TxMicCodec::OpusWb));
            assert_eq!(client.state.tx_codec_active, TxMicCodec::Pcm);
            assert!(client.state.tx_codec_negotiated_at.is_none());
            client.outbound.clone()
        };
        let queued = outbound.next_message(true).unwrap();
        match queued.message {
            OutboundMessage::Text(text) => {
                assert_eq!(text, "tx_codec_reject:0,opus_wb,unsupported;")
            }
            other => panic!("unexpected outbound: {other:?}"),
        }
    }

    #[test]
    fn phase44_tx_codec_caps_accepts_opus_only_when_runtime_flag_enabled() {
        let (tx, rx) = mpsc::channel();
        let mut clients_map = BTreeMap::new();
        clients_map.insert(
            7,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                    opus_decode_enabled: true,
                }),
            },
        );
        let clients = Arc::new(Mutex::new(clients_map));

        parse_tci_command("tx_codec_caps:0,opus_wb,pcm;", &tx, &clients, 7, true);

        assert!(rx.try_recv().is_err());
        let outbound = {
            let clients = clients.lock().unwrap();
            let client = clients.get(&7).unwrap();
            assert!(client.state.tx_codec_caps.contains(&TxMicCodec::OpusWb));
            assert_eq!(client.state.tx_codec_active, TxMicCodec::OpusWb);
            assert!(client.state.tx_codec_negotiated_at.is_some());
            assert_eq!(
                client.state.tx_codec_decoder.lock().unwrap().codec(),
                TxMicCodec::OpusWb
            );
            client.outbound.clone()
        };
        let queued = outbound.next_message(true).unwrap();
        match queued.message {
            OutboundMessage::Text(text) => assert_eq!(text, "tx_codec_accept:0,opus_wb;"),
            other => panic!("unexpected outbound: {other:?}"),
        }
    }

    #[test]
    fn operator_text_updates_control_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let clients = test_client_registry(7);
        let operator_client_id = Arc::new(AtomicU64::new(7));
        let operator_control_at = Arc::new(Mutex::new(None));

        assert!(handle_incoming_message(
            Message::Text("saturn_ping:probe-1,123.456;".into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            7,
        ));

        assert!(operator_control_at.lock().unwrap().is_some());
    }

    #[test]
    fn viewer_text_does_not_update_control_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let clients = test_client_registry(7);
        let operator_client_id = Arc::new(AtomicU64::new(1));
        let operator_control_at = Arc::new(Mutex::new(None));

        assert!(handle_incoming_message(
            Message::Text("saturn_ping:probe-1,123.456;".into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            7,
        ));

        assert!(operator_control_at.lock().unwrap().is_none());
    }

    #[test]
    fn websocket_ping_does_not_update_control_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let clients = test_client_registry(7);
        let operator_client_id = Arc::new(AtomicU64::new(7));
        let operator_control_at = Arc::new(Mutex::new(None));

        assert!(handle_incoming_message(
            Message::Ping(Vec::new().into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            7,
        ));

        assert!(operator_control_at.lock().unwrap().is_none());
    }

    #[test]
    fn formats_tx_power_trip_fault_message() {
        assert_eq!(
            tx_power_trip_fault_message(126.34, 110.0),
            "tx_fault:0,power_trip,126.3,110.0;"
        );
    }

    #[test]
    fn formats_tx_uplink_late_fault_message() {
        assert_eq!(
            tx_uplink_late_fault_message(280, 250),
            "tx_fault:0,uplink_late,280,250;"
        );
    }

    #[test]
    fn formats_tx_control_watchdog_fault_message() {
        assert_eq!(
            tx_control_watchdog_fault_message(620, 500),
            "tx_fault:0,control_watchdog,620,500;"
        );
    }

    #[test]
    fn formats_remote_client_role_message() {
        assert_eq!(
            remote_client_role_message(42, TciClientRole::Operator),
            "remote_client_role:0,operator,42;"
        );
        assert_eq!(
            remote_client_role_message(43, TciClientRole::Viewer),
            "remote_client_role:0,viewer,43;"
        );
    }

    #[test]
    fn phase42_parses_session_open_and_paired_message() {
        assert_eq!(
            parse_phase42_session_open("session_open:phase-42,viewer;"),
            Some(("phase-42".to_string(), TciClientRole::Viewer))
        );
        assert_eq!(
            parse_phase42_session_open("session_open:operator.1;"),
            Some(("operator.1".to_string(), TciClientRole::Operator))
        );
        assert_eq!(parse_phase42_session_open("saturn_ping:1,2;"), None);
        assert_eq!(
            phase42_session_paired_message("phase-42"),
            "session_paired:phase-42;"
        );
    }

    #[test]
    fn phase42_parses_proxy_lane_marker() {
        assert_eq!(
            parse_phase42_session_lane("session_lane:phase-42,control;"),
            Some(("phase-42".to_string(), Phase42SocketKind::Control))
        );
        assert_eq!(
            parse_phase42_session_lane("session_lane:phase%3A42,media;"),
            Some(("phase3A42".to_string(), Phase42SocketKind::Media))
        );
        assert_eq!(Phase42SocketKind::Control.as_tci(), "control");
        assert_eq!(
            parse_phase42_session_lane("session_lane:phase-42,data;"),
            None
        );
        assert_eq!(
            parse_phase42_session_lane("session_open:phase-42,operator;"),
            None
        );
    }

    #[test]
    fn phase42_metadata_commands_cross_viewer_filter() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(51);

        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 51, false);
        match rx.try_recv().unwrap() {
            TciCommand::Phase42SessionLane {
                client_id,
                session_id,
                lane,
            } => {
                assert_eq!(client_id, 51);
                assert_eq!(session_id, "phase-42");
                assert_eq!(lane, Phase42SocketKind::Media);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        parse_tci_command("session_open:phase-42,viewer;", &tx, &clients, 51, false);
        match rx.try_recv().unwrap() {
            TciCommand::Phase42SessionOpen {
                client_id,
                session_id,
                role,
            } => {
                assert_eq!(client_id, 51);
                assert_eq!(session_id, "phase-42");
                assert_eq!(role, TciClientRole::Viewer);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn phase42_metadata_updates_client_state_and_rejects_mismatch() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(52);

        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 52, false);
        parse_tci_command("session_open:phase-42,viewer;", &tx, &clients, 52, false);

        {
            let clients = clients.lock().unwrap();
            let phase42 = clients.get(&52).unwrap().state.phase42.as_ref().unwrap();
            assert_eq!(phase42.session_id, "phase-42");
            assert_eq!(phase42.lane, Some(Phase42SocketKind::Media));
            assert_eq!(phase42.role, Some(TciClientRole::Viewer));
        }
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::Phase42SessionLane { .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::Phase42SessionOpen { .. })
        ));

        parse_tci_command(
            "session_lane:other-session,control;",
            &tx,
            &clients,
            52,
            false,
        );
        assert!(rx.try_recv().is_err());
        let clients = clients.lock().unwrap();
        let phase42 = clients.get(&52).unwrap().state.phase42.as_ref().unwrap();
        assert_eq!(phase42.session_id, "phase-42");
        assert_eq!(phase42.lane, Some(Phase42SocketKind::Media));
    }

    #[test]
    fn phase42_pairing_status_derives_from_client_metadata() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(61);
        clients.lock().unwrap().insert(
            62,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );

        parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 61, true);
        assert_eq!(phase42_session_pair_for_client(&clients, 61), None);

        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 62, false);
        assert_eq!(
            phase42_session_pair_for_client(&clients, 62),
            Some(Phase42SessionPair {
                session_id: "phase-42".to_string(),
                control_client_id: 61,
                media_client_id: 62,
            })
        );
        {
            let clients = clients.lock().unwrap();
            assert_eq!(
                phase42_lane_client_count(&clients, Phase42SocketKind::Control),
                1
            );
            assert_eq!(
                phase42_lane_client_count(&clients, Phase42SocketKind::Media),
                1
            );
            assert_eq!(phase42_paired_session_count(&clients), 1);
        }
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::Phase42SessionLane { client_id: 61, .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::Phase42SessionLane { client_id: 62, .. })
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn phase42_paired_media_socket_can_supply_mic_binary() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(71);
        clients.lock().unwrap().insert(
            72,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(71));
        let operator_control_at = Arc::new(Mutex::new(None));

        parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 71, true);
        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 72, false);
        while rx.try_recv().is_ok() {}

        let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 91);
        assert!(handle_incoming_message(
            Message::Binary(frame),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            72,
        ));

        match rx.try_recv().unwrap() {
            TciCommand::MicAudioFrame(frame) => {
                assert_eq!(frame.sequence, 91);
                assert_eq!(frame.samples, vec![0.25, -0.25]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn phase42_release_window_blocks_paired_media_mic_binary() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(73);
        clients.lock().unwrap().insert(
            74,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(73));
        let operator_control_at = Arc::new(Mutex::new(None));

        parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 73, true);
        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 74, false);
        while rx.try_recv().is_ok() {}

        let now = Instant::now();
        assert_eq!(
            set_phase42_media_ignore_until(&clients, 73, Some(now + PHASE42_RELEASE_IGNORE_WINDOW)),
            1
        );
        assert!(!phase42_media_client_can_supply_mic(&clients, 73, 74, now));
        assert!(phase42_media_client_can_supply_mic(
            &clients,
            73,
            74,
            now + PHASE42_RELEASE_IGNORE_WINDOW + Duration::from_millis(1)
        ));

        let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 93);
        assert!(handle_incoming_message(
            Message::Binary(frame),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            74,
        ));
        assert!(rx.try_recv().is_err());

        assert_eq!(set_phase42_media_ignore_until(&clients, 73, None), 1);
        assert!(phase42_media_client_can_supply_mic(
            &clients,
            73,
            74,
            Instant::now()
        ));
    }

    #[test]
    fn phase44_media_decode_errors_force_rx_and_report_on_control_lane() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(73);
        clients.lock().unwrap().insert(
            74,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(73));
        let operator_control_at = Arc::new(Mutex::new(None));

        parse_tci_command("session_lane:phase-44,control;", &tx, &clients, 73, true);
        parse_tci_command("session_lane:phase-44,media;", &tx, &clients, 74, false);
        while rx.try_recv().is_ok() {}

        let mut frame = vec![0u8; 64 + 4];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
        write_u32_le(&mut frame, 40, 2);

        for _ in 0..TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT {
            assert!(handle_incoming_message(
                Message::Binary(frame.clone()),
                &tx,
                &clients,
                &operator_client_id,
                &operator_control_at,
                74,
            ));
        }

        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(rx.try_recv().is_err());

        let (control_outbound, media_outbound) = {
            let clients = clients.lock().unwrap();
            let media = clients.get(&74).unwrap();
            assert_eq!(
                media.state.tx_codec_decode_error_count,
                TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT
            );
            assert!(media.state.tx_codec_degraded);
            (
                clients.get(&73).unwrap().outbound.clone(),
                media.outbound.clone(),
            )
        };

        let fault = control_outbound.next_message(true).unwrap();
        match fault.message {
            OutboundMessage::SafetyText(text) => {
                assert_eq!(text, "tx_fault:0,codec_decode,count=10,limit=10;")
            }
            other => panic!("unexpected outbound: {other:?}"),
        }
        assert!(media_outbound.next_message(true).is_none());
    }

    #[test]
    fn phase42_unpaired_media_socket_cannot_supply_mic_binary() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(81);
        let operator_client_id = Arc::new(AtomicU64::new(80));
        let operator_control_at = Arc::new(Mutex::new(None));

        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 81, false);
        while rx.try_recv().is_ok() {}

        let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 92);
        assert!(handle_incoming_message(
            Message::Binary(frame),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            81,
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn phase42_session_pairs_after_control_and_media_connect() {
        let now = Instant::now();
        let mut session = Phase42SplitSession::new_control("phase-42", now).unwrap();

        assert_eq!(session.state, Phase42SessionState::WaitingMedia);
        assert!(!session.pairing_timed_out(now + Duration::from_secs(29)));
        assert!(session.pairing_timed_out(now + Duration::from_secs(30)));

        assert_eq!(
            session.connect_media(),
            Some("session_paired:phase-42;".to_string())
        );
        assert_eq!(session.state, Phase42SessionState::Paired);
    }

    #[test]
    fn phase42_release_opens_media_ignore_window() {
        let now = Instant::now();
        let mut session = Phase42SplitSession::new_control("phase-42", now).unwrap();
        session.connect_media();
        assert!(session.key());
        assert_eq!(
            session.media_frame_action(now + Duration::from_millis(10)),
            Phase42MediaFrameAction::Accept
        );

        assert!(session.release(now + Duration::from_millis(20)));
        assert_eq!(session.state, Phase42SessionState::Paired);
        assert_eq!(
            session.media_frame_action(now + Duration::from_millis(30)),
            Phase42MediaFrameAction::DropReleaseWindow
        );
        assert_eq!(session.release_window_drops, 1);
        assert_eq!(
            session.media_frame_action(now + Duration::from_millis(300)),
            Phase42MediaFrameAction::DropNotKeyed
        );
    }

    #[test]
    fn phase42_disconnects_force_rx_at_safety_boundaries() {
        let now = Instant::now();
        let mut media_loss = Phase42SplitSession::new_control("phase-42", now).unwrap();
        media_loss.connect_media();
        media_loss.key();
        assert_eq!(
            media_loss.disconnect_media(),
            Phase42DisconnectAction {
                force_rx: true,
                close_peer_socket: false,
                state: Phase42SessionState::WaitingMedia,
            }
        );

        let mut control_loss = Phase42SplitSession::new_control("phase-43", now).unwrap();
        control_loss.connect_media();
        control_loss.key();
        assert_eq!(
            control_loss.disconnect_control(),
            Phase42DisconnectAction {
                force_rx: true,
                close_peer_socket: true,
                state: Phase42SessionState::Terminated,
            }
        );
    }

    fn insert_phase42_paired_client(
        clients: &ClientRegistry,
        client_id: u64,
        session_id: &str,
        lane: Phase42SocketKind,
        role: Option<TciClientRole>,
    ) {
        let mut clients = clients.lock().unwrap();
        clients.insert(
            client_id,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState {
                    phase42: Some(Phase42ClientMetadata {
                        session_id: session_id.to_string(),
                        lane: Some(lane),
                        role,
                        ignore_media_until: None,
                    }),
                    ..ClientState::default()
                },
            },
        );
    }

    #[test]
    fn phase42_iq_stream_enable_propagates_from_control_to_media() {
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        insert_phase42_paired_client(
            &clients,
            80,
            "phase-42",
            Phase42SocketKind::Control,
            Some(TciClientRole::Operator),
        );
        insert_phase42_paired_client(&clients, 81, "phase-42", Phase42SocketKind::Media, None);

        let any_enabled = set_client_iq_stream_enabled(&clients, 80, true);
        assert!(any_enabled);

        let snapshot = clients.lock().unwrap();
        assert!(snapshot.get(&80).unwrap().state.iq_stream_enabled);
        assert!(snapshot.get(&81).unwrap().state.iq_stream_enabled);
    }

    #[test]
    fn phase42_audio_stream_enable_propagates_from_control_to_media() {
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        insert_phase42_paired_client(
            &clients,
            82,
            "phase-42",
            Phase42SocketKind::Control,
            Some(TciClientRole::Operator),
        );
        insert_phase42_paired_client(&clients, 83, "phase-42", Phase42SocketKind::Media, None);

        let any_enabled = set_client_audio_stream_enabled(&clients, 82, true);
        assert!(any_enabled);

        let snapshot = clients.lock().unwrap();
        assert!(snapshot.get(&82).unwrap().state.audio_stream_enabled);
        assert!(snapshot.get(&83).unwrap().state.audio_stream_enabled);
    }

    #[test]
    fn phase42_audio_format_state_propagates_from_control_to_media() {
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        insert_phase42_paired_client(
            &clients,
            84,
            "phase-42",
            Phase42SocketKind::Control,
            Some(TciClientRole::Operator),
        );
        insert_phase42_paired_client(&clients, 85, "phase-42", Phase42SocketKind::Media, None);

        set_client_audio_sample_rate(&clients, 84, 24_000);
        set_client_audio_frame_float_count(&clients, 84, 4096);
        set_client_audio_channels(&clients, 84, 1);

        let snapshot = clients.lock().unwrap();
        assert_eq!(
            snapshot.get(&85).unwrap().state.audio_sample_rate_hz,
            24_000
        );
        assert_eq!(
            snapshot.get(&85).unwrap().state.audio_frame_float_count,
            4096
        );
        assert_eq!(snapshot.get(&85).unwrap().state.audio_channels, 1);
    }

    #[test]
    fn phase42_tx_media_priority_suppresses_media_downlink() {
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        insert_phase42_paired_client(
            &clients,
            86,
            "phase-42",
            Phase42SocketKind::Control,
            Some(TciClientRole::Operator),
        );
        insert_phase42_paired_client(&clients, 87, "phase-42", Phase42SocketKind::Media, None);
        set_client_iq_stream_enabled(&clients, 86, true);
        set_client_audio_stream_enabled(&clients, 86, true);

        let snapshot = clients.lock().unwrap();
        let media = snapshot.get(&87).unwrap();

        let rx_iq = OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        };
        let tx_iq = OutboundMessage::TxIqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        };
        let audio = OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 48_000,
            channels: 1,
            audio_samples: vec![0.0, 0.0],
            sequence: 7,
        };

        // While on-air: all three binary variants are suppressed on the media lane.
        assert!(!client_wants_outbound_message(media, &rx_iq, true));
        assert!(!client_wants_outbound_message(media, &tx_iq, true));
        assert!(!client_wants_outbound_message(media, &audio, true));

        // Off-air: media lane receives binary as normal.
        assert!(client_wants_outbound_message(media, &rx_iq, false));
        assert!(client_wants_outbound_message(media, &tx_iq, false));
        assert!(client_wants_outbound_message(media, &audio, false));
    }

    #[test]
    fn phase42_outbound_routing_sends_text_to_control_lane_not_media() {
        let mut control = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        control.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Control),
            role: Some(TciClientRole::Operator),
            ignore_media_until: None,
        });
        let mut media = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        media.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Media),
            role: None,
            ignore_media_until: None,
        });

        let text = OutboundMessage::Text("rx_smeter:0,0,-110.0;".into());
        assert!(client_wants_outbound_message(&control, &text, false));
        assert!(!client_wants_outbound_message(&media, &text, false));

        let safety = OutboundMessage::SafetyText("tx_fault:0,power_trip,126.3,110.0;".into());
        assert!(client_wants_outbound_message(&control, &safety, false));
        assert!(!client_wants_outbound_message(&media, &safety, false));
    }

    #[test]
    fn phase42_outbound_routing_sends_iq_to_media_lane_not_control() {
        let mut control = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        control.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Control),
            role: Some(TciClientRole::Operator),
            ignore_media_until: None,
        });
        control.state.iq_stream_enabled = true;

        let mut media = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        media.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Media),
            role: None,
            ignore_media_until: None,
        });
        media.state.iq_stream_enabled = true;

        let iq = OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        };
        assert!(!client_wants_outbound_message(&control, &iq, false));
        assert!(client_wants_outbound_message(&media, &iq, false));

        let tx_iq = OutboundMessage::TxIqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        };
        assert!(!client_wants_outbound_message(&control, &tx_iq, false));
        assert!(client_wants_outbound_message(&media, &tx_iq, false));
    }

    #[test]
    fn phase42_outbound_routing_sends_audio_to_media_lane_not_control() {
        let mut control = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        control.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Control),
            role: Some(TciClientRole::Operator),
            ignore_media_until: None,
        });
        control.state.audio_stream_enabled = true;

        let mut media = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        media.state.phase42 = Some(Phase42ClientMetadata {
            session_id: "phase-42".into(),
            lane: Some(Phase42SocketKind::Media),
            role: None,
            ignore_media_until: None,
        });
        media.state.audio_stream_enabled = true;

        let audio = OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 48_000,
            channels: 1,
            audio_samples: vec![0.0, 0.0],
            sequence: 7,
        };
        assert!(!client_wants_outbound_message(&control, &audio, false));
        assert!(client_wants_outbound_message(&media, &audio, false));
    }

    #[test]
    fn legacy_non_phase42_client_receives_text_and_binary() {
        let mut legacy = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        // No phase42 metadata — represents a legacy single-socket client.
        legacy.state.iq_stream_enabled = true;
        legacy.state.audio_stream_enabled = true;

        let text = OutboundMessage::Text("rx_smeter:0,0,-110.0;".into());
        let iq = OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 192_000,
            iq_samples: vec![0.0, 0.0],
        };
        let audio = OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 48_000,
            channels: 1,
            audio_samples: vec![0.0, 0.0],
            sequence: 0,
        };

        assert!(client_wants_outbound_message(&legacy, &text, false));
        assert!(client_wants_outbound_message(&legacy, &iq, false));
        assert!(client_wants_outbound_message(&legacy, &audio, false));
    }

    #[test]
    fn viewer_commands_are_limited_to_streaming_and_ping() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(9);

        parse_tci_command("trx:0,true,tci;", &tx, &clients, 9, false);
        assert!(rx.try_recv().is_err());

        parse_tci_command("iq_start:0;", &tx, &clients, 9, false);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetIqStreaming)));
        assert!(
            clients
                .lock()
                .unwrap()
                .get(&9)
                .unwrap()
                .state
                .iq_stream_enabled
        );

        parse_tci_command("audio_seq_gap_count:0,4", &tx, &clients, 9, false);
        assert_eq!(
            clients
                .lock()
                .unwrap()
                .get(&9)
                .unwrap()
                .state
                .audio_seq_gap_count,
            4
        );

        parse_tci_command(
            "tx_uplink_stats:0,true,5,6,7000,9000",
            &tx,
            &clients,
            9,
            false,
        );
        assert!(
            !clients
                .lock()
                .unwrap()
                .get(&9)
                .unwrap()
                .state
                .tx_uplink_degraded
        );

        // Viewer cannot toggle TX media priority — this command is a no-op
        // on the bridge after Phase 42 source-of-truth refactor, but the
        // viewer filter must still drop it.
        parse_tci_command("remote_tx_media_priority:0,true", &tx, &clients, 9, false);
    }

    #[test]
    fn parses_operator_tx_uplink_stats() {
        let (tx, _rx) = mpsc::channel();
        let clients = test_client_registry(9);

        parse_tci_command(
            "tx_uplink_stats:0,true,5,6,7000,9000",
            &tx,
            &clients,
            9,
            true,
        );
        let clients = clients.lock().unwrap();
        let state = &clients.get(&9).unwrap().state;
        assert!(state.tx_uplink_degraded);
        assert_eq!(state.tx_mic_browser_last_seq, 5);
        assert_eq!(state.tx_mic_browser_dropped_count, 6);
        assert_eq!(state.tx_uplink_buffered_bytes, 7000);
        assert_eq!(state.tx_uplink_buffered_high_watermark_bytes, 9000);
    }

    #[test]
    fn tracks_tx_codec_safety_counters_in_client_snapshot() {
        let clients = test_client_registry(9);

        record_client_tx_codec_decode_error(&clients, 9);
        record_client_tx_codec_decode_error(&clients, 9);
        record_client_tx_codec_stale_drop(&clients, 9);
        assert!(flush_client_tx_codec_decode_queue(&clients, 9));

        let clients = clients.lock().unwrap();
        let state = &clients.get(&9).unwrap().state;
        assert_eq!(state.tx_codec_decode_error_count, 2);
        assert_eq!(state.tx_codec_stale_drop_count, 1);
        assert_eq!(state.tx_codec_release_flush_count, 1);
    }

    #[test]
    fn classifies_phase44_parser_decode_errors_for_telemetry() {
        let mut frame = vec![0u8; 64 + 4];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
        write_u32_le(&mut frame, 40, 2);

        assert_eq!(
            parse_tci_mic_frame_result(&frame).unwrap_err(),
            TciMicFrameParseError::Decode(TxDecodeError::PayloadSizeMismatch)
        );

        write_u32_le(&mut frame, 24, 1);
        assert_eq!(
            parse_tci_mic_frame_result(&frame).unwrap_err(),
            TciMicFrameParseError::NotMicFrame
        );
    }

    #[test]
    fn remote_tx_media_priority_is_a_noop_after_phase42_refactor() {
        // After Phase 42 refactor, TX media priority is derived from the
        // bridge's on-air state, not from this browser command. The command
        // is accepted (no parse error) but has no side effect. Older
        // browsers may still send it; this test asserts forward compat.
        let (tx, _rx) = mpsc::channel();
        let clients = test_client_registry(9);

        parse_tci_command("remote_tx_media_priority:0,true", &tx, &clients, 9, true);
        parse_tci_command("remote_tx_media_priority:0,false", &tx, &clients, 9, true);
        // No assertion on per-client field — that field no longer exists.
        // Test passes if parse_tci_command does not panic on the command.
    }

    #[test]
    fn records_tx_mic_sequence_gaps() {
        let clients = test_client_registry(9);

        let arrived_at = Instant::now();
        record_client_tx_mic_frame(&clients, 9, 1, arrived_at);
        record_client_tx_mic_frame(&clients, 9, 2, arrived_at);
        record_client_tx_mic_frame(&clients, 9, 4, arrived_at);

        let clients = clients.lock().unwrap();
        let state = &clients.get(&9).unwrap().state;
        assert_eq!(state.tx_mic_last_arrived_seq, 4);
        assert_eq!(state.tx_mic_seq_gap_count, 1);
        assert_eq!(state.tx_mic_last_arrived_at, Some(arrived_at));
    }

    #[test]
    fn trx_true_resets_tx_uplink_attempt_telemetry() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(9);

        parse_tci_command(
            "tx_uplink_stats:0,true,12,3,4096,8192",
            &tx,
            &clients,
            9,
            true,
        );
        let arrived_at = Instant::now();
        record_client_tx_mic_frame(&clients, 9, 10, arrived_at);
        record_client_tx_mic_frame(&clients, 9, 12, arrived_at);

        {
            let clients = clients.lock().unwrap();
            let state = &clients.get(&9).unwrap().state;
            assert!(state.tx_uplink_degraded);
            assert_eq!(state.tx_mic_browser_dropped_count, 3);
            assert_eq!(state.tx_mic_seq_gap_count, 1);
            assert_eq!(state.tx_mic_last_arrived_seq, 12);
        }

        parse_tci_command("trx:0,true,tci;", &tx, &clients, 9, true);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));

        let first_frame_at = arrived_at + Duration::from_millis(20);
        record_client_tx_mic_frame(&clients, 9, 50, first_frame_at);

        let clients = clients.lock().unwrap();
        let state = &clients.get(&9).unwrap().state;
        assert!(!state.tx_uplink_degraded);
        assert_eq!(state.tx_mic_browser_last_seq, 0);
        assert_eq!(state.tx_mic_browser_dropped_count, 0);
        assert_eq!(state.tx_uplink_buffered_bytes, 0);
        assert_eq!(state.tx_uplink_buffered_high_watermark_bytes, 0);
        assert_eq!(state.tx_mic_last_arrived_seq, 50);
        assert_eq!(state.tx_mic_seq_gap_count, 0);
        assert_eq!(state.tx_mic_last_arrived_at, Some(first_frame_at));
    }

    #[test]
    fn operator_disconnect_promotes_oldest_viewer() {
        let clients = test_client_registry(1);
        clients.lock().unwrap().insert(
            2,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(1));

        let disconnect = unregister_client(&clients, &operator_client_id, 1);

        assert!(disconnect.was_operator);
        assert_eq!(disconnect.promoted_operator, Some(2));
        assert_eq!(disconnect.phase42_closed_peer, None);
        assert!(!disconnect.phase42_media_loss_forces_rx);
        assert_eq!(disconnect.remaining_clients, 1);
        assert_eq!(operator_client_id.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn phase42_media_disconnect_forces_rx_when_paired_with_operator() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(91);
        clients.lock().unwrap().insert(
            92,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(91));

        parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 91, true);
        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 92, false);
        while rx.try_recv().is_ok() {}

        let disconnect = unregister_client(&clients, &operator_client_id, 92);

        assert!(!disconnect.was_operator);
        assert!(disconnect.phase42_media_loss_forces_rx);
        assert_eq!(disconnect.phase42_closed_peer, None);
        assert_eq!(disconnect.promoted_operator, None);
        assert_eq!(disconnect.remaining_clients, 1);
        assert_eq!(operator_client_id.load(Ordering::SeqCst), 91);
    }

    #[test]
    fn phase42_control_disconnect_queues_media_peer_close() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(101);
        clients.lock().unwrap().insert(
            102,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        let operator_client_id = Arc::new(AtomicU64::new(101));

        parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 101, true);
        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 102, false);
        while rx.try_recv().is_ok() {}

        let media_outbound = clients.lock().unwrap().get(&102).unwrap().outbound.clone();
        let disconnect = unregister_client(&clients, &operator_client_id, 101);

        assert!(disconnect.was_operator);
        assert_eq!(disconnect.phase42_closed_peer, Some(102));
        assert_eq!(disconnect.remaining_clients, 1);
        let close = media_outbound.next_message(false).unwrap();
        assert!(matches!(close.message, OutboundMessage::Close));
    }

    #[test]
    fn operator_disconnect_does_not_promote_phase42_media_socket() {
        let (tx, rx) = mpsc::channel();
        let clients = test_client_registry(1);
        {
            let mut clients = clients.lock().unwrap();
            clients.insert(
                2,
                ClientConnection {
                    outbound: ClientOutbound::new(),
                    state: ClientState::default(),
                },
            );
            clients.insert(
                3,
                ClientConnection {
                    outbound: ClientOutbound::new(),
                    state: ClientState::default(),
                },
            );
        }
        let operator_client_id = Arc::new(AtomicU64::new(1));
        parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 2, false);
        while rx.try_recv().is_ok() {}

        let disconnect = unregister_client(&clients, &operator_client_id, 1);

        assert!(disconnect.was_operator);
        assert!(!disconnect.phase42_media_loss_forces_rx);
        assert_eq!(disconnect.phase42_closed_peer, None);
        assert_eq!(disconnect.promoted_operator, Some(3));
        assert_eq!(disconnect.remaining_clients, 2);
        assert_eq!(operator_client_id.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn initial_snapshot_includes_remote_tx_rf_state() {
        let model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
        let disabled = initial_snapshot_messages(&model, false, 7, TciClientRole::Viewer);
        let enabled = initial_snapshot_messages(&model, true, 8, TciClientRole::Operator);

        assert!(disabled.contains(&"remote_tx_rf_enabled:0,false;".to_string()));
        assert!(enabled.contains(&"remote_tx_rf_enabled:0,true;".to_string()));
        assert!(disabled.contains(&"remote_client_role:0,viewer,7;".to_string()));
        assert!(enabled.contains(&"remote_client_role:0,operator,8;".to_string()));
    }
}
