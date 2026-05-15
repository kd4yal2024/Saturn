use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::raw::{c_int, c_ulong};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::error::Error as WsError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_with_config, Message};

use crate::config::BridgeConfig;
use crate::radio_model::{AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel};

#[derive(Clone, Debug)]
pub struct TciMicFrame {
    pub sample_rate_hz: u32,
    pub channels: u32,
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
            Self::Text(text) if is_safety_text_message(text) => OutboundClass::Safety,
            Self::Text(_) => OutboundClass::Control,
            Self::AudioFrame { .. } => OutboundClass::Audio,
            Self::IqFrame { .. } | Self::TxIqFrame { .. } => OutboundClass::Display,
        }
    }

    fn estimated_bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
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

fn is_safety_text_message(text: &str) -> bool {
    text.starts_with("tx_fault:")
        || text.starts_with("remote_tx_rf_enabled:")
        || text.starts_with("remote_client_role:")
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
struct ClientState {
    iq_stream_enabled: bool,
    audio_stream_enabled: bool,
    audio_sample_rate_hz: u32,
    audio_frame_float_count: u32,
    audio_channels: u32,
    audio_seq_gap_count: u64,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            iq_stream_enabled: false,
            audio_stream_enabled: false,
            audio_sample_rate_hz: 48_000,
            audio_frame_float_count: 2048,
            audio_channels: 2,
            audio_seq_gap_count: 0,
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
const MAX_TCI_MIC_FLOAT_SAMPLES: usize = 32_768;
const BULK_TCP_OUTQ_LIMIT_BYTES: usize = 64 * 1024;
const BULK_BACKPRESSURE_PAUSE_MS: u64 = 10;

#[cfg(target_os = "linux")]
const TIOCOUTQ: c_ulong = 0x5411;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

fn tx_power_trip_fault_message(forward_watts: f32, limit_watts: f32) -> String {
    format!("tx_fault:0,power_trip,{forward_watts:.1},{limit_watts:.1};")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TciClientRole {
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
}

fn remote_client_role_message(client_id: u64, role: TciClientRole) -> String {
    format!("remote_client_role:0,{},{client_id};", role.as_tci())
}

pub struct TciFrontend {
    command_rx: Receiver<TciCommand>,
    clients: ClientRegistry,
    drop_count: Arc<AtomicU64>,
    display_rate_limited_count: AtomicU64,
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
        let drop_count = Arc::new(AtomicU64::new(0));
        let remote_tx_rf_enabled = config.remote_tx_rf_enabled;

        let client_registry = clients.clone();
        let next_client = next_client_id.clone();
        let operator_client = operator_client_id.clone();
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
                    let drop_count = drop_counter.clone();
                    let radio_model = radio_model.clone();

                    thread::spawn(move || {
                        handle_client(
                            stream,
                            addr,
                            client_id,
                            &command_tx,
                            &clients,
                            &operator_client_id,
                            &radio_model,
                            &drop_count,
                            remote_tx_rf_enabled,
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
            drop_count,
            display_rate_limited_count: AtomicU64::new(0),
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

    pub fn client_snapshot(&self) -> TciClientSnapshot {
        let clients = self.clients.lock().unwrap();
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

    pub fn publish_saturn_pong(&self, client_id: u64, nonce: &str, sent_at: &str) {
        self.send_text_to(client_id, format!("saturn_pong:{nonce},{sent_at};"));
    }

    pub fn publish_tx_power_trip(&self, forward_watts: f32, limit_watts: f32) {
        self.send_text(tx_power_trip_fault_message(forward_watts, limit_watts));
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
        let clients = self.clients.lock().unwrap();
        for client in clients.values() {
            if !client_wants_outbound_message(client, &message) {
                continue;
            }
            let drops = client.outbound.enqueue(message.clone());
            self.drop_count.fetch_add(drops, Ordering::Relaxed);
        }
    }
}

fn client_wants_outbound_message(client: &ClientConnection, message: &OutboundMessage) -> bool {
    match message {
        OutboundMessage::Text(_) => true,
        OutboundMessage::IqFrame { .. } | OutboundMessage::TxIqFrame { .. } => {
            client.state.iq_stream_enabled
        }
        OutboundMessage::AudioFrame { .. } => client.state.audio_stream_enabled,
    }
}

fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    client_id: u64,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    radio_model: &Arc<Mutex<RadioModel>>,
    drop_count: &Arc<AtomicU64>,
    remote_tx_rf_enabled: bool,
) {
    let _ = stream.set_nonblocking(true);
    match accept_with_config(stream, Some(tci_websocket_config())) {
        Ok(mut websocket) => {
            let outbound = ClientOutbound::new();
            let (role, first_client, client_count) =
                register_client(clients, operator_client_id, client_id, outbound.clone());
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
                    match send_outbound(&mut websocket, &item.message) {
                        Ok(()) => {
                            outbound.record_write(item.class, item.enqueued_at.elapsed());
                            pending_flush = true;
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
                let _ = command_tx.send(TciCommand::SetTxEnabled(false));
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
    promoted_operator: Option<u64>,
    remaining_clients: usize,
}

fn register_client(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    client_id: u64,
    outbound: Arc<ClientOutbound>,
) -> (TciClientRole, bool, usize) {
    let mut clients = clients.lock().unwrap();
    let first_client = clients.is_empty();
    clients.insert(
        client_id,
        ClientConnection {
            outbound,
            state: ClientState::default(),
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
    clients.remove(&client_id);

    let was_operator = operator_client_id.load(Ordering::SeqCst) == client_id;
    let mut promoted_operator = None;
    if was_operator {
        if let Some((&next_operator, _)) = clients.iter().next() {
            operator_client_id.store(next_operator, Ordering::SeqCst);
            promoted_operator = Some(next_operator);
        } else {
            operator_client_id.store(0, Ordering::SeqCst);
        }
    }

    ClientDisconnect {
        was_operator,
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
        let _ = outbound.enqueue(OutboundMessage::Text(remote_client_role_message(
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
    client_id: u64,
) -> bool {
    let is_operator = operator_client_id.load(Ordering::SeqCst) == client_id;
    match message {
        Message::Text(text) => {
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
            if is_operator {
                if let Some(frame) = parse_tci_mic_frame(&data) {
                    let _ = command_tx.send(TciCommand::MicAudioFrame(frame));
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
        "audio_stream_sample_type" => {}
        "rx_smeter" | "s_meter" | "smeter" => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
        }
        "trx" => {
            // trx:0,true or trx:0,true,tci — PTT on/off
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    println!("saturn-bridge: TCI trx requested -> {}", enabled);
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

fn set_client_iq_stream_enabled(clients: &ClientRegistry, client_id: u64, enabled: bool) -> bool {
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.iq_stream_enabled = enabled;
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
    clients
        .values()
        .any(|client| client.state.audio_stream_enabled)
}

fn set_client_audio_sample_rate(clients: &ClientRegistry, client_id: u64, sample_rate_hz: u32) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.audio_sample_rate_hz = sample_rate_hz;
    }
}

fn set_client_audio_frame_float_count(clients: &ClientRegistry, client_id: u64, sample_count: u32) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.audio_frame_float_count = sample_count;
    }
}

fn set_client_audio_channels(clients: &ClientRegistry, client_id: u64, channels: u32) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.audio_channels = channels;
    }
}

fn set_client_audio_seq_gap_count(clients: &ClientRegistry, client_id: u64, gaps: u64) {
    if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
        client.state.audio_seq_gap_count = gaps;
    }
}

/// Parse a TCI binary frame that contains TX mic audio from the client.
/// Frame layout: 64-byte header + f32 LE samples.
///   header[20..24] = sample_count (u32 LE)
///   header[24..28] = stream_type  (u32 LE); must be 2 (TX mic)
///   header[28..32] = channels     (u32 LE); 1=mono, 2=stereo
///
/// stream_type == 1 is intentionally excluded: it is the RX audio type used by
/// the server→client direction and must not be fed into the TX DSP path.
fn parse_tci_mic_frame(data: &[u8]) -> Option<TciMicFrame> {
    if data.len() < 68 {
        return None; // need at least header + 1 sample
    }
    let sample_rate_hz = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let stream_type = u32::from_le_bytes(data[24..28].try_into().ok()?);
    if stream_type != 2 {
        return None;
    }
    let raw_channels = u32::from_le_bytes(data[28..32].try_into().ok()?);
    let channels = match raw_channels {
        0 | 1 => 1,
        _ => 2,
    };
    let sample_count = u32::from_le_bytes(data[20..24].try_into().ok()?) as usize;
    if sample_count == 0 || sample_count > MAX_TCI_MIC_FLOAT_SAMPLES {
        return None;
    }
    let payload = &data[64..];
    if payload.len() < sample_count * 4 {
        return None;
    }
    let mut samples = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let bytes: [u8; 4] = payload[i * 4..i * 4 + 4].try_into().ok()?;
        samples.push(f32::from_le_bytes(bytes));
    }
    Some(TciMicFrame {
        sample_rate_hz,
        channels,
        samples,
    })
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
        OutboundMessage::Text(text) => websocket.send(Message::Text(text.clone())),
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
        outbound.enqueue(OutboundMessage::Text(
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
        let sample_count = (MAX_TCI_MIC_FLOAT_SAMPLES + 1) as u32;
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
        frame[64..68].copy_from_slice(&0.25f32.to_le_bytes());
        frame[68..72].copy_from_slice(&(-0.5f32).to_le_bytes());

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn parses_stereo_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 16];
        write_u32_le(&mut frame, 4, 48_000);
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
        assert_eq!(parsed.samples, vec![0.25, -0.25, 0.5, -0.5]);
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
    fn formats_tx_power_trip_fault_message() {
        assert_eq!(
            tx_power_trip_fault_message(126.34, 110.0),
            "tx_fault:0,power_trip,126.3,110.0;"
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
        assert_eq!(disconnect.remaining_clients, 1);
        assert_eq!(operator_client_id.load(Ordering::SeqCst), 2);
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
