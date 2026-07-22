use std::collections::VecDeque;
use std::io;
use std::net::TcpStream;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::raw::{c_int, c_ulong};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::error::Error as WsError;
use tungstenite::Message;

use crate::sync_ext::MutexExt;

use super::*;

#[derive(Clone, Debug)]
pub(crate) enum OutboundMessage {
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
pub(crate) enum OutboundClass {
    Safety,
    Control,
    Audio,
    Display,
}

impl OutboundClass {
    pub(crate) fn records_enqueue_to_write_latency(self) -> bool {
        matches!(self, Self::Safety | Self::Control)
    }

    pub(crate) fn is_never_drop(self) -> bool {
        matches!(self, Self::Safety)
    }
}

impl OutboundMessage {
    pub(crate) fn class(&self) -> OutboundClass {
        match self {
            Self::Close => OutboundClass::Safety,
            Self::SafetyText(_) => OutboundClass::Safety,
            Self::Text(_) => OutboundClass::Control,
            Self::AudioFrame { .. } => OutboundClass::Audio,
            Self::IqFrame { .. } | Self::TxIqFrame { .. } => OutboundClass::Display,
        }
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
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

    pub(crate) fn audio_frame_count(&self) -> usize {
        match self {
            Self::AudioFrame {
                audio_samples,
                channels,
                ..
            } => audio_samples.len() / usize::try_from((*channels).max(1)).unwrap_or(2),
            _ => 0,
        }
    }

    pub(crate) fn audio_sample_rate(&self) -> u32 {
        match self {
            Self::AudioFrame { sample_rate, .. } => *sample_rate,
            _ => 0,
        }
    }

    pub(crate) fn with_audio_sequence(mut self, sequence: u32) -> Self {
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
pub(crate) struct QueuedOutbound {
    pub(crate) message: OutboundMessage,
    pub(crate) class: OutboundClass,
    pub(crate) enqueued_at: Instant,
    pub(crate) estimated_bytes: usize,
    pub(crate) audio_frames: usize,
}

impl QueuedOutbound {
    pub(crate) fn new(message: OutboundMessage) -> Self {
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
pub(crate) struct ClientSchedulerStatsDelta {
    pub(crate) safety_latencies_us: Vec<u64>,
    pub(crate) control_latencies_us: Vec<u64>,
    pub(crate) display_replaced: u64,
    pub(crate) display_dropped: u64,
    pub(crate) control_replaced: u64,
    pub(crate) control_dropped: u64,
    pub(crate) audio_dropped: u64,
    pub(crate) audio_panic_drain: u64,
    pub(crate) send_blocked_ms: u64,
    pub(crate) outbound_high_watermark_bytes: u64,
    pub(crate) tcp_outq_high_watermark_bytes: u64,
    pub(crate) safety_queue_depth_overflow: u64,
    pub(crate) control_queue_high_watermark: u64,
}

#[derive(Default, Debug)]
pub(crate) struct ClientSchedulerStatsInner {
    pub(crate) safety_latencies_us: Vec<u64>,
    pub(crate) control_latencies_us: Vec<u64>,
    pub(crate) display_replaced: u64,
    pub(crate) display_dropped: u64,
    pub(crate) control_replaced: u64,
    pub(crate) control_dropped: u64,
    pub(crate) audio_dropped: u64,
    pub(crate) audio_panic_drain: u64,
    pub(crate) send_blocked_ms: u64,
    pub(crate) outbound_high_watermark_bytes: u64,
    pub(crate) tcp_outq_high_watermark_bytes: u64,
    pub(crate) safety_queue_depth_overflow: u64,
    pub(crate) control_queue_high_watermark: u64,
}

#[derive(Default, Debug)]
pub(crate) struct ClientSchedulerStats {
    pub(crate) inner: Mutex<ClientSchedulerStatsInner>,
}

impl ClientSchedulerStats {
    pub(crate) fn record_write(&self, class: OutboundClass, latency: Duration) {
        if !class.records_enqueue_to_write_latency() {
            return;
        }
        let latency_us = latency.as_micros().min(u128::from(u64::MAX)) as u64;
        let mut inner = self.inner.lock_unpoisoned();
        match class {
            OutboundClass::Safety => inner.safety_latencies_us.push(latency_us),
            OutboundClass::Control => inner.control_latencies_us.push(latency_us),
            OutboundClass::Audio | OutboundClass::Display => {}
        }
    }

    pub(crate) fn record_display_replaced(&self) {
        self.inner.lock_unpoisoned().display_replaced += 1;
    }

    pub(crate) fn record_display_dropped(&self) {
        self.inner.lock_unpoisoned().display_dropped += 1;
    }

    pub(crate) fn record_control_replaced(&self) {
        self.inner.lock_unpoisoned().control_replaced += 1;
    }

    pub(crate) fn record_control_dropped(&self) {
        self.inner.lock_unpoisoned().control_dropped += 1;
    }

    pub(crate) fn record_audio_dropped(&self, count: u64) {
        self.inner.lock_unpoisoned().audio_dropped += count;
    }

    pub(crate) fn record_audio_panic_drain(&self) {
        self.inner.lock_unpoisoned().audio_panic_drain += 1;
    }

    pub(crate) fn record_send_blocked(&self, duration: Duration) {
        self.inner.lock_unpoisoned().send_blocked_ms += duration.as_millis().max(1) as u64;
    }

    pub(crate) fn record_high_watermark(&self, bytes: usize) {
        let mut inner = self.inner.lock_unpoisoned();
        inner.outbound_high_watermark_bytes = inner.outbound_high_watermark_bytes.max(bytes as u64);
    }

    pub(crate) fn record_tcp_outq_high_watermark(&self, bytes: usize) {
        let mut inner = self.inner.lock_unpoisoned();
        inner.tcp_outq_high_watermark_bytes = inner.tcp_outq_high_watermark_bytes.max(bytes as u64);
    }

    pub(crate) fn record_safety_queue_depth_overflow(&self) {
        self.inner.lock_unpoisoned().safety_queue_depth_overflow += 1;
    }

    pub(crate) fn record_control_queue_high_watermark(&self, depth: usize) {
        let mut inner = self.inner.lock_unpoisoned();
        inner.control_queue_high_watermark = inner.control_queue_high_watermark.max(depth as u64);
    }

    pub(crate) fn drain(&self) -> ClientSchedulerStatsDelta {
        let mut inner = self.inner.lock_unpoisoned();
        ClientSchedulerStatsDelta {
            safety_latencies_us: std::mem::take(&mut inner.safety_latencies_us),
            control_latencies_us: std::mem::take(&mut inner.control_latencies_us),
            display_replaced: std::mem::take(&mut inner.display_replaced),
            display_dropped: std::mem::take(&mut inner.display_dropped),
            control_replaced: std::mem::take(&mut inner.control_replaced),
            control_dropped: std::mem::take(&mut inner.control_dropped),
            audio_dropped: std::mem::take(&mut inner.audio_dropped),
            audio_panic_drain: std::mem::take(&mut inner.audio_panic_drain),
            send_blocked_ms: std::mem::take(&mut inner.send_blocked_ms),
            outbound_high_watermark_bytes: std::mem::take(&mut inner.outbound_high_watermark_bytes),
            tcp_outq_high_watermark_bytes: std::mem::take(&mut inner.tcp_outq_high_watermark_bytes),
            safety_queue_depth_overflow: std::mem::take(&mut inner.safety_queue_depth_overflow),
            control_queue_high_watermark: std::mem::take(&mut inner.control_queue_high_watermark),
        }
    }
}

#[derive(Debug)]
pub(crate) struct OutboundQueues {
    pub(crate) safety: VecDeque<QueuedOutbound>,
    pub(crate) control: VecDeque<QueuedOutbound>,
    pub(crate) audio: VecDeque<QueuedOutbound>,
    pub(crate) display: Option<QueuedOutbound>,
    pub(crate) queued_bytes: usize,
    pub(crate) audio_queued_frames: usize,
    pub(crate) audio_sequence: u32,
    pub(crate) writer_started: bool,
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
pub(crate) struct ClientOutbound {
    pub(crate) queues: Mutex<OutboundQueues>,
    pub(crate) stats: ClientSchedulerStats,
}

impl ClientOutbound {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            queues: Mutex::new(OutboundQueues::default()),
            stats: ClientSchedulerStats::default(),
        })
    }

    pub(crate) fn mark_writer_started(&self) {
        self.queues.lock_unpoisoned().writer_started = true;
    }

    pub(crate) fn enqueue(&self, message: OutboundMessage) -> u64 {
        let mut message = message;
        let class = message.class();
        let mut dropped = 0;
        let mut queues = self.queues.lock_unpoisoned();

        if class == OutboundClass::Audio {
            queues.audio_sequence = queues.audio_sequence.wrapping_add(1).max(1);
            message = message.with_audio_sequence(queues.audio_sequence);
        }

        let item = QueuedOutbound::new(message);
        match item.class {
            OutboundClass::Safety => {
                if let Some(key) = safety_coalesce_key(&item.message) {
                    if let Some(position) = queues.safety.iter().position(|queued| {
                        safety_coalesce_key(&queued.message) == Some(key.clone())
                    }) {
                        let old = std::mem::replace(&mut queues.safety[position], item);
                        queues.queued_bytes = queues
                            .queued_bytes
                            .saturating_sub(old.estimated_bytes)
                            .saturating_add(queues.safety[position].estimated_bytes);
                        self.stats.record_high_watermark(queues.queued_bytes);
                        return 0;
                    }
                }
                if queues.safety.len() >= MAX_SAFETY_QUEUE_MESSAGES {
                    if let Some(old) = queues.safety.pop_front() {
                        queues.queued_bytes =
                            queues.queued_bytes.saturating_sub(old.estimated_bytes);
                        dropped += 1;
                        self.stats.record_safety_queue_depth_overflow();
                    }
                }
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
                if let Some(key) = control_coalesce_key(&item.message) {
                    if let Some(position) = queues.control.iter().position(|queued| {
                        control_coalesce_key(&queued.message) == Some(key.clone())
                    }) {
                        let old = std::mem::replace(&mut queues.control[position], item);
                        queues.queued_bytes = queues
                            .queued_bytes
                            .saturating_sub(old.estimated_bytes)
                            .saturating_add(queues.control[position].estimated_bytes);
                        self.stats.record_control_replaced();
                        self.stats.record_high_watermark(queues.queued_bytes);
                        return 1;
                    }
                }
                queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
                queues.control.push_back(item);
                while queues.control.len() > MAX_CONTROL_QUEUE_MESSAGES
                    || queues.queued_bytes > MAX_CONTROL_QUEUE_BYTES
                {
                    let Some(old) = queues.control.pop_front() else {
                        break;
                    };
                    queues.queued_bytes = queues.queued_bytes.saturating_sub(old.estimated_bytes);
                    dropped += 1;
                    self.stats.record_control_dropped();
                }
                self.stats
                    .record_control_queue_high_watermark(queues.control.len());
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

    pub(crate) fn enqueue_audio_locked(
        &self,
        queues: &mut OutboundQueues,
        item: QueuedOutbound,
    ) -> u64 {
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

    pub(crate) fn next_message(&self, allow_bulk: bool) -> Option<QueuedOutbound> {
        let mut queues = self.queues.lock_unpoisoned();
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

    pub(crate) fn requeue_front(&self, item: QueuedOutbound) {
        let mut queues = self.queues.lock_unpoisoned();
        queues.queued_bytes = queues.queued_bytes.saturating_add(item.estimated_bytes);
        match item.class {
            OutboundClass::Safety => queues.safety.push_front(item),
            OutboundClass::Control => {
                queues.control.push_front(item);
                while queues.control.len() > MAX_CONTROL_QUEUE_MESSAGES
                    || queues.queued_bytes > MAX_CONTROL_QUEUE_BYTES
                {
                    let Some(old) = queues.control.pop_back() else {
                        break;
                    };
                    queues.queued_bytes = queues.queued_bytes.saturating_sub(old.estimated_bytes);
                    self.stats.record_control_dropped();
                }
                self.stats
                    .record_control_queue_high_watermark(queues.control.len());
            }
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

    pub(crate) fn record_bulk_send_drop(&self, class: OutboundClass) {
        match class {
            OutboundClass::Audio => self.stats.record_audio_dropped(1),
            OutboundClass::Display => self.stats.record_display_dropped(),
            OutboundClass::Control => self.stats.record_control_dropped(),
            OutboundClass::Safety => {}
        }
    }

    pub(crate) fn record_write(&self, class: OutboundClass, latency: Duration) {
        self.stats.record_write(class, latency);
    }

    pub(crate) fn record_send_blocked(&self, duration: Duration) {
        self.stats.record_send_blocked(duration);
    }

    pub(crate) fn record_tcp_outq_high_watermark(&self, bytes: usize) {
        self.stats.record_tcp_outq_high_watermark(bytes);
    }

    pub(crate) fn drain_stats(&self) -> ClientSchedulerStatsDelta {
        self.stats.drain()
    }

    pub(crate) fn queued_bytes(&self) -> u64 {
        self.queues.lock_unpoisoned().queued_bytes as u64
    }
}

pub(crate) const MAX_SAFETY_QUEUE_MESSAGES: usize = 16;
pub(crate) const MAX_CONTROL_QUEUE_MESSAGES: usize = 256;
pub(crate) const MAX_CONTROL_QUEUE_BYTES: usize = 256 * 1024;

fn safety_coalesce_key(message: &OutboundMessage) -> Option<String> {
    match message {
        OutboundMessage::Close => Some("close".to_string()),
        OutboundMessage::SafetyText(text) => tci_text_coalesce_key(text),
        _ => None,
    }
}

fn control_coalesce_key(message: &OutboundMessage) -> Option<String> {
    match message {
        OutboundMessage::Text(text) => tci_text_coalesce_key(text),
        _ => None,
    }
}

fn tci_text_coalesce_key(text: &str) -> Option<String> {
    let text = text.trim();
    if text.matches(';').count() > 1 {
        return None;
    }
    let text = text.strip_suffix(';').unwrap_or(text);
    let (name, rest) = text.split_once(':')?;
    if name.is_empty() || rest.is_empty() {
        return None;
    }
    let args: Vec<&str> = rest.split(',').collect();
    if name == "tx_fault" && args.len() >= 2 {
        return Some(format!("{name}:{},{}", args[0], args[1]));
    }
    if matches!(name, "remote_backpressure" | "remote_tx_uplink") {
        return Some(name.to_string());
    }
    if args.len() == 1 {
        Some(name.to_string())
    } else {
        Some(format!("{name}:{}", args[..args.len() - 1].join(",")))
    }
}

pub(crate) fn max_audio_queued_frames(sample_rate_hz: u32) -> usize {
    let sample_rate = usize::try_from(sample_rate_hz.max(8_000)).unwrap_or(48_000);
    (sample_rate / 4).max(1)
}

pub(crate) fn shape_rx_audio_for_transport(
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

pub(crate) fn display_frame_interval_for_limit(limit_hz: u16) -> Duration {
    if limit_hz == 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(1_000_000_000u64 / u64::from(limit_hz))
    }
}

pub(crate) fn queued_bytes_without_audio(queues: &OutboundQueues) -> usize {
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

pub(crate) fn percentile_us(samples: &mut [u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile.min(100)).div_ceil(100);
    samples[index]
}

pub(crate) const BULK_TCP_OUTQ_LIMIT_BYTES: usize = 64 * 1024;

pub(crate) const BULK_BACKPRESSURE_PAUSE_MS: u64 = 10;

#[cfg(target_os = "linux")]
pub(crate) const TIOCOUTQ: c_ulong = 0x5411;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

pub(crate) fn client_wants_outbound_message(
    client: &ClientConnection,
    message: &OutboundMessage,
    tx_media_priority_active: bool,
) -> bool {
    // Lane awareness: text goes only to control-lane clients (or
    // legacy non-Phase-42 clients); binary RX frames go only to media-lane
    // clients (or legacy non-Phase-42 clients). Sending text on a media
    // socket or binary on a control socket would be rejected by the
    // browser-side adapter as a protocol violation.
    //
    // While TX media priority is active, binary RX (IQ + audio) is additionally
    // suppressed on the media lane to give uplink mic frames sole ownership of
    // the media TCP send buffer. The bridge derives this from TX intent/armed/
    // keyed state; when it returns to false the suppression lifts automatically.
    //
    // Before the in-band session_lane command arrives, the lane declared by
    // the websocket request path (connect_lane_hint) applies, so broadcasts
    // never race the declaration onto the wrong lane.
    let lane = client
        .state
        .split
        .as_ref()
        .and_then(|m| m.lane)
        .or(client.state.connect_lane_hint);
    match message {
        OutboundMessage::Close => true,
        OutboundMessage::Text(_) | OutboundMessage::SafetyText(_) => {
            lane != Some(SplitSocketKind::Media)
        }
        OutboundMessage::IqFrame { .. } | OutboundMessage::TxIqFrame { .. } => {
            lane != Some(SplitSocketKind::Control)
                && client.state.iq_stream_enabled
                && !tx_media_priority_active
        }
        OutboundMessage::AudioFrame { .. } => {
            lane != Some(SplitSocketKind::Control)
                && client.state.audio_stream_enabled
                && !tx_media_priority_active
        }
    }
}

pub(crate) fn bulk_allowed_for_tcp_outq(tcp_outq_bytes: usize) -> bool {
    tcp_outq_bytes < BULK_TCP_OUTQ_LIMIT_BYTES
}

#[cfg(target_os = "linux")]
pub(crate) fn tcp_outq_bytes(stream: &TcpStream) -> io::Result<usize> {
    let mut bytes: c_int = 0;
    let result = unsafe { ioctl(stream.as_raw_fd(), TIOCOUTQ, &mut bytes) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(bytes.max(0) as usize)
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn tcp_outq_bytes(_stream: &TcpStream) -> io::Result<usize> {
    Ok(0)
}

pub(crate) fn send_outbound(
    websocket: &mut tungstenite::WebSocket<std::net::TcpStream>,
    message: &OutboundMessage,
) -> Result<(), WsError> {
    match message {
        OutboundMessage::Close => websocket.send(Message::Close(None)),
        OutboundMessage::Text(text) | OutboundMessage::SafetyText(text) => {
            websocket.send(Message::Text(text.clone().into()))
        }
        OutboundMessage::IqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(
            build_tci_iq_frame(*receiver, *sample_rate, iq_samples).into(),
        )),
        OutboundMessage::TxIqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(
            build_tci_tx_iq_frame(*receiver, *sample_rate, iq_samples).into(),
        )),
        OutboundMessage::AudioFrame {
            receiver,
            sample_rate,
            channels,
            audio_samples,
            sequence,
        } => websocket.send(Message::Binary(
            build_tci_audio_frame(*receiver, *sample_rate, *channels, audio_samples, *sequence)
                .into(),
        )),
    }
}
