use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

pub const TX_AUDIO_INGRESS_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxAudioSource {
    Tci,
    Satp,
}

impl TxAudioSource {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tci" => Some(Self::Tci),
            "satp" => Some(Self::Satp),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tci => "tci",
            Self::Satp => "satp",
        }
    }
}

pub struct TxAudioFrame {
    pub source: TxAudioSource,
    pub samples: TxAudioSamples,
    pub channels: u32,
    pub sample_rate_hz: u32,
}

pub enum TxAudioSamples {
    Dynamic(Vec<f32>),
    Satp([f32; 128]),
}

impl TxAudioSamples {
    pub fn as_slice(&self) -> &[f32] {
        match self {
            Self::Dynamic(samples) => samples,
            Self::Satp(samples) => samples,
        }
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }
}

pub enum TxAudioMessage {
    Frames(TxAudioFrame),
    Flush(TxAudioSource),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TxAudioIngressSnapshot {
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub satp_frames_enqueued: u64,
    pub satp_samples_enqueued: u64,
    pub satp_frames_queue_full: u64,
    pub satp_samples_queue_full: u64,
    pub satp_frames_accepted: u64,
    pub satp_samples_accepted: u64,
    pub satp_frames_rejected_idle: u64,
    pub satp_frames_rejected_source: u64,
    pub tci_frames_enqueued: u64,
    pub tci_samples_enqueued: u64,
    pub tci_frames_queue_full: u64,
    pub tci_samples_queue_full: u64,
    pub tci_frames_accepted: u64,
    pub tci_samples_accepted: u64,
    pub tci_frames_rejected_idle: u64,
    pub tci_frames_rejected_source: u64,
    pub flushes_enqueued: u64,
    pub flushes_processed: u64,
    pub pipeline_state: &'static str,
    pub pipeline_rf_enabled: bool,
    pub pipeline_mic_frames: u64,
    pub pipeline_duc_packets: u64,
    pub pipeline_wdsp_input_samples: u64,
    pub pipeline_wdsp_output_pairs: u64,
    pub pipeline_pending_mic_floats: usize,
    pub pipeline_pending_iq_floats: usize,
    pub pipeline_rmatch_enabled: bool,
    pub pipeline_rmatch_underflows: i32,
    pub pipeline_rmatch_overflows: i32,
    pub pipeline_rmatch_ratio: f64,
}

#[derive(Default)]
pub struct TxAudioIngressStats {
    queue_depth: AtomicUsize,
    queue_high_water: AtomicUsize,
    satp_frames_enqueued: AtomicU64,
    satp_samples_enqueued: AtomicU64,
    satp_frames_queue_full: AtomicU64,
    satp_samples_queue_full: AtomicU64,
    satp_frames_accepted: AtomicU64,
    satp_samples_accepted: AtomicU64,
    satp_frames_rejected_idle: AtomicU64,
    satp_frames_rejected_source: AtomicU64,
    tci_frames_enqueued: AtomicU64,
    tci_samples_enqueued: AtomicU64,
    tci_frames_queue_full: AtomicU64,
    tci_samples_queue_full: AtomicU64,
    tci_frames_accepted: AtomicU64,
    tci_samples_accepted: AtomicU64,
    tci_frames_rejected_idle: AtomicU64,
    tci_frames_rejected_source: AtomicU64,
    flushes_enqueued: AtomicU64,
    flushes_processed: AtomicU64,
    pipeline_state: AtomicU8,
    pipeline_rf_enabled: AtomicBool,
    pipeline_mic_frames: AtomicU64,
    pipeline_duc_packets: AtomicU64,
    pipeline_wdsp_input_samples: AtomicU64,
    pipeline_wdsp_output_pairs: AtomicU64,
    pipeline_pending_mic_floats: AtomicUsize,
    pipeline_pending_iq_floats: AtomicUsize,
    pipeline_rmatch_enabled: AtomicBool,
    pipeline_rmatch_underflows: AtomicU64,
    pipeline_rmatch_overflows: AtomicU64,
    pipeline_rmatch_ratio_bits: AtomicU64,
}

impl TxAudioIngressStats {
    fn on_enqueued(&self, source: TxAudioSource, samples: usize) {
        let samples = samples as u64;
        match source {
            TxAudioSource::Tci => {
                self.tci_frames_enqueued.fetch_add(1, Ordering::Relaxed);
                self.tci_samples_enqueued
                    .fetch_add(samples, Ordering::Relaxed);
            }
            TxAudioSource::Satp => {
                self.satp_frames_enqueued.fetch_add(1, Ordering::Relaxed);
                self.satp_samples_enqueued
                    .fetch_add(samples, Ordering::Relaxed);
            }
        }
    }

    fn on_queue_full(&self, source: TxAudioSource, samples: usize) {
        let samples = samples as u64;
        match source {
            TxAudioSource::Tci => {
                self.tci_frames_queue_full.fetch_add(1, Ordering::Relaxed);
                self.tci_samples_queue_full
                    .fetch_add(samples, Ordering::Relaxed);
            }
            TxAudioSource::Satp => {
                self.satp_frames_queue_full.fetch_add(1, Ordering::Relaxed);
                self.satp_samples_queue_full
                    .fetch_add(samples, Ordering::Relaxed);
            }
        }
    }

    pub fn on_dequeued(&self) {
        let _ = self
            .queue_depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(1))
            });
    }

    pub fn on_accepted(&self, source: TxAudioSource, samples: usize) {
        let samples = samples as u64;
        match source {
            TxAudioSource::Tci => {
                self.tci_frames_accepted.fetch_add(1, Ordering::Relaxed);
                self.tci_samples_accepted
                    .fetch_add(samples, Ordering::Relaxed);
            }
            TxAudioSource::Satp => {
                self.satp_frames_accepted.fetch_add(1, Ordering::Relaxed);
                self.satp_samples_accepted
                    .fetch_add(samples, Ordering::Relaxed);
            }
        }
    }

    pub fn on_rejected_idle(&self, source: TxAudioSource) {
        match source {
            TxAudioSource::Tci => self
                .tci_frames_rejected_idle
                .fetch_add(1, Ordering::Relaxed),
            TxAudioSource::Satp => self
                .satp_frames_rejected_idle
                .fetch_add(1, Ordering::Relaxed),
        };
    }

    pub fn on_rejected_source(&self, source: TxAudioSource) {
        match source {
            TxAudioSource::Tci => self
                .tci_frames_rejected_source
                .fetch_add(1, Ordering::Relaxed),
            TxAudioSource::Satp => self
                .satp_frames_rejected_source
                .fetch_add(1, Ordering::Relaxed),
        };
    }

    pub fn on_flush_processed(&self) {
        self.flushes_processed.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_pipeline(
        &self,
        state: &str,
        rf_enabled: bool,
        mic_frames: u64,
        duc_packets: u64,
        wdsp_input_samples: u64,
        wdsp_output_pairs: u64,
        pending_mic_floats: usize,
        pending_iq_floats: usize,
        rmatch_enabled: bool,
        rmatch_underflows: i32,
        rmatch_overflows: i32,
        rmatch_ratio: f64,
    ) {
        let state = match state {
            "armed" => 1,
            "keyed" => 2,
            _ => 0,
        };
        self.pipeline_state.store(state, Ordering::Relaxed);
        self.pipeline_rf_enabled
            .store(rf_enabled, Ordering::Relaxed);
        self.pipeline_mic_frames
            .store(mic_frames, Ordering::Relaxed);
        self.pipeline_duc_packets
            .store(duc_packets, Ordering::Relaxed);
        self.pipeline_wdsp_input_samples
            .store(wdsp_input_samples, Ordering::Relaxed);
        self.pipeline_wdsp_output_pairs
            .store(wdsp_output_pairs, Ordering::Relaxed);
        self.pipeline_pending_mic_floats
            .store(pending_mic_floats, Ordering::Relaxed);
        self.pipeline_pending_iq_floats
            .store(pending_iq_floats, Ordering::Relaxed);
        self.pipeline_rmatch_enabled
            .store(rmatch_enabled, Ordering::Relaxed);
        self.pipeline_rmatch_underflows
            .store(rmatch_underflows as i64 as u64, Ordering::Relaxed);
        self.pipeline_rmatch_overflows
            .store(rmatch_overflows as i64 as u64, Ordering::Relaxed);
        self.pipeline_rmatch_ratio_bits
            .store(rmatch_ratio.to_bits(), Ordering::Relaxed);
    }

    pub fn mark_pipeline_idle(&self) {
        self.pipeline_state.store(0, Ordering::Relaxed);
        self.pipeline_rf_enabled.store(false, Ordering::Relaxed);
    }

    pub fn mark_pipeline_armed(&self, rf_enabled: bool) {
        self.pipeline_rf_enabled
            .store(rf_enabled, Ordering::Relaxed);
        self.pipeline_state.store(1, Ordering::Release);
    }

    pub fn snapshot(&self) -> TxAudioIngressSnapshot {
        macro_rules! load {
            ($field:ident) => {
                self.$field.load(Ordering::Relaxed)
            };
        }
        let pipeline_state = match self.pipeline_state.load(Ordering::Relaxed) {
            1 => "armed",
            2 => "keyed",
            _ => "idle",
        };
        TxAudioIngressSnapshot {
            queue_depth: load!(queue_depth),
            queue_high_water: load!(queue_high_water),
            satp_frames_enqueued: load!(satp_frames_enqueued),
            satp_samples_enqueued: load!(satp_samples_enqueued),
            satp_frames_queue_full: load!(satp_frames_queue_full),
            satp_samples_queue_full: load!(satp_samples_queue_full),
            satp_frames_accepted: load!(satp_frames_accepted),
            satp_samples_accepted: load!(satp_samples_accepted),
            satp_frames_rejected_idle: load!(satp_frames_rejected_idle),
            satp_frames_rejected_source: load!(satp_frames_rejected_source),
            tci_frames_enqueued: load!(tci_frames_enqueued),
            tci_samples_enqueued: load!(tci_samples_enqueued),
            tci_frames_queue_full: load!(tci_frames_queue_full),
            tci_samples_queue_full: load!(tci_samples_queue_full),
            tci_frames_accepted: load!(tci_frames_accepted),
            tci_samples_accepted: load!(tci_samples_accepted),
            tci_frames_rejected_idle: load!(tci_frames_rejected_idle),
            tci_frames_rejected_source: load!(tci_frames_rejected_source),
            flushes_enqueued: load!(flushes_enqueued),
            flushes_processed: load!(flushes_processed),
            pipeline_state,
            pipeline_rf_enabled: self.pipeline_rf_enabled.load(Ordering::Relaxed),
            pipeline_mic_frames: load!(pipeline_mic_frames),
            pipeline_duc_packets: load!(pipeline_duc_packets),
            pipeline_wdsp_input_samples: load!(pipeline_wdsp_input_samples),
            pipeline_wdsp_output_pairs: load!(pipeline_wdsp_output_pairs),
            pipeline_pending_mic_floats: load!(pipeline_pending_mic_floats),
            pipeline_pending_iq_floats: load!(pipeline_pending_iq_floats),
            pipeline_rmatch_enabled: self.pipeline_rmatch_enabled.load(Ordering::Relaxed),
            pipeline_rmatch_underflows: load!(pipeline_rmatch_underflows) as i64 as i32,
            pipeline_rmatch_overflows: load!(pipeline_rmatch_overflows) as i64 as i32,
            pipeline_rmatch_ratio: match load!(pipeline_rmatch_ratio_bits) {
                0 => 1.0,
                bits => f64::from_bits(bits),
            },
        }
    }
}

#[derive(Clone)]
pub struct TxAudioIngress {
    sender: mpsc::SyncSender<TxAudioMessage>,
    stats: Arc<TxAudioIngressStats>,
}

impl TxAudioIngress {
    pub fn bounded() -> (
        Self,
        mpsc::Receiver<TxAudioMessage>,
        Arc<TxAudioIngressStats>,
    ) {
        let (sender, receiver) = mpsc::sync_channel(TX_AUDIO_INGRESS_CAPACITY);
        let stats = Arc::new(TxAudioIngressStats::default());
        (
            Self {
                sender,
                stats: stats.clone(),
            },
            receiver,
            stats,
        )
    }

    pub fn write_frame(
        &self,
        source: TxAudioSource,
        samples: Vec<f32>,
        channels: u32,
        sample_rate_hz: u32,
    ) -> io::Result<()> {
        let samples = TxAudioSamples::Dynamic(samples);
        let sample_count = samples.len();
        let depth = self.stats.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        match self.sender.try_send(TxAudioMessage::Frames(TxAudioFrame {
            source,
            samples,
            channels,
            sample_rate_hz,
        })) {
            Ok(()) => {
                self.stats
                    .queue_high_water
                    .fetch_max(depth, Ordering::Relaxed);
                self.stats.on_enqueued(source, sample_count);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.stats.on_dequeued();
                self.stats.on_queue_full(source, sample_count);
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "TX audio ingress queue is full",
                ))
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.stats.on_dequeued();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "TX audio ingress is disconnected",
                ))
            }
        }
    }

    pub fn write_satp_packet(&self, samples: [f32; 128]) -> io::Result<()> {
        let source = TxAudioSource::Satp;
        let sample_count = samples.len();
        let depth = self.stats.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        match self.sender.try_send(TxAudioMessage::Frames(TxAudioFrame {
            source,
            samples: TxAudioSamples::Satp(samples),
            channels: 1,
            sample_rate_hz: 48_000,
        })) {
            Ok(()) => {
                self.stats
                    .queue_high_water
                    .fetch_max(depth, Ordering::Relaxed);
                self.stats.on_enqueued(source, sample_count);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.stats.on_dequeued();
                self.stats.on_queue_full(source, sample_count);
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "TX audio ingress queue is full",
                ))
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.stats.on_dequeued();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "TX audio ingress is disconnected",
                ))
            }
        }
    }

    pub fn flush(&self, source: TxAudioSource) -> io::Result<()> {
        let depth = self.stats.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        match self.sender.try_send(TxAudioMessage::Flush(source)) {
            Ok(()) => {
                self.stats
                    .queue_high_water
                    .fetch_max(depth, Ordering::Relaxed);
                self.stats.flushes_enqueued.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.stats.on_dequeued();
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "TX audio ingress queue is full",
                ))
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.stats.on_dequeued();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "TX audio ingress is disconnected",
                ))
            }
        }
    }

    pub fn stats(&self) -> TxAudioIngressSnapshot {
        self.stats.snapshot()
    }

    pub fn pipeline_accepting_audio(&self) -> bool {
        self.stats.pipeline_state.load(Ordering::Acquire) != 0
    }
}

/// Consumer boundary for native transmit PCM. Implementations must enqueue
/// only; network workers never call WDSP, XDMA, or radio-control APIs directly.
pub trait TxAudioSink: Send {
    fn write_frames(&mut self, samples: [f32; 128]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    fn name(&self) -> &'static str;
}

#[derive(Default)]
pub struct NullTxAudioSink {
    frames: u64,
    flushes: u64,
}

impl NullTxAudioSink {
    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    #[cfg(test)]
    pub fn flushes(&self) -> u64 {
        self.flushes
    }
}

impl TxAudioSink for NullTxAudioSink {
    fn write_frames(&mut self, samples: [f32; 128]) -> io::Result<()> {
        self.frames = self.frames.saturating_add(samples.len() as u64);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes = self.flushes.saturating_add(1);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "null"
    }
}

pub struct TxThreadAudioSink {
    ingress: TxAudioIngress,
}

impl TxThreadAudioSink {
    pub fn new(ingress: TxAudioIngress) -> Self {
        Self { ingress }
    }
}

impl TxAudioSink for TxThreadAudioSink {
    fn write_frames(&mut self, samples: [f32; 128]) -> io::Result<()> {
        self.ingress.write_satp_packet(samples)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.ingress.flush(TxAudioSource::Satp)
    }

    fn name(&self) -> &'static str {
        "tx_thread"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_counts_without_forwarding_audio() {
        let mut sink = NullTxAudioSink::default();
        sink.write_frames([0.1; 128]).unwrap();
        sink.flush().unwrap();
        assert_eq!(sink.frames(), 128);
        assert_eq!(sink.flushes(), 1);
        assert_eq!(sink.name(), "null");
    }

    #[test]
    fn bounded_ingress_accounts_for_enqueued_and_consumed_satp_audio() {
        let (ingress, receiver, stats) = TxAudioIngress::bounded();
        ingress
            .write_frame(TxAudioSource::Satp, vec![0.25; 128], 1, 48_000)
            .unwrap();
        assert_eq!(stats.snapshot().queue_depth, 1);
        let TxAudioMessage::Frames(frame) = receiver.recv().unwrap() else {
            panic!("expected audio frame")
        };
        stats.on_dequeued();
        stats.on_accepted(frame.source, frame.samples.len());
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.satp_frames_enqueued, 1);
        assert_eq!(snapshot.satp_samples_enqueued, 128);
        assert_eq!(snapshot.satp_frames_accepted, 1);
        assert_eq!(snapshot.satp_samples_accepted, 128);
    }

    #[test]
    fn bounded_ingress_drops_new_media_when_full() {
        let (ingress, _receiver, stats) = TxAudioIngress::bounded();
        for _ in 0..TX_AUDIO_INGRESS_CAPACITY {
            ingress
                .write_frame(TxAudioSource::Satp, vec![0.0; 128], 1, 48_000)
                .unwrap();
        }
        let error = ingress
            .write_frame(TxAudioSource::Satp, vec![0.0; 128], 1, 48_000)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.queue_depth, TX_AUDIO_INGRESS_CAPACITY);
        assert_eq!(snapshot.queue_high_water, TX_AUDIO_INGRESS_CAPACITY);
        assert_eq!(snapshot.satp_frames_queue_full, 1);
        assert_eq!(snapshot.satp_samples_queue_full, 128);
    }

    #[test]
    fn pipeline_acceptance_follows_consumer_arm_state() {
        let (ingress, _receiver, stats) = TxAudioIngress::bounded();
        assert!(!ingress.pipeline_accepting_audio());

        stats.mark_pipeline_armed(true);
        assert!(ingress.pipeline_accepting_audio());
        assert_eq!(stats.snapshot().pipeline_state, "armed");
        assert!(stats.snapshot().pipeline_rf_enabled);

        stats.mark_pipeline_idle();
        assert!(!ingress.pipeline_accepting_audio());
    }
}
