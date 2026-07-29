//! Phase 4 direct Saturn/XDMA DUC performance probe.
//!
//! This one-shot path writes zero-valued or deterministic changing IQ to the
//! TX DUC while every RF control remains forced off and TX amplitude remains
//! zero. The DUC mux is enabled solely so the FPGA can consume the test stream
//! and its sustained 192 kHz pacing can be measured.

use crate::xdma::{ensure_p2app_inactive, SaturnIdentity, XdmaError, XdmaRegisterDevice};
use crate::xdma_rx::AlignedBuffer;
use crate::xdma_telemetry::{record_probe_outcome, TelemetryValue};
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_DUC_DEVICE: &str = "/dev/xdma0_h2c_0";
const DEFAULT_PROBE_DURATION_MS: u64 = 3_000;
const MIN_PROBE_DURATION_MS: u64 = 500;
const MAX_PROBE_DURATION_MS: u64 = 86_400_000;
const DEFAULT_MAX_P9999_REFILL_SERVICE_US: u64 = 5_000;
const MIN_MAX_P9999_REFILL_SERVICE_US: u64 = 100;
const MAX_MAX_P9999_REFILL_SERVICE_US: u64 = 100_000;
const DEFAULT_MAX_P9999_MARGIN_PERCENT: u64 = 60;
const MIN_MAX_P9999_MARGIN_PERCENT: u64 = 10;
const MAX_MAX_P9999_MARGIN_PERCENT: u64 = 95;
const MAX_RT_PRIORITY: u64 = 80;

const DUC_DMA_AXI_OFFSET: u64 = 0;
const FIFO_RESET_REGISTER: u64 = 0x7000;
const DUC_FIFO_MONITOR_REGISTER: u64 = 0x9004;
const DUC_FIFO_MONITOR_CONFIG_REGISTER: u64 = 0x9014;
const KEYER_CONFIG_REGISTER: u64 = 0x2000;
const TX_CONFIG_REGISTER: u64 = 0x2008;
const RF_GPIO_REGISTER: u64 = 0x2014;

const DUC_FIFO_RESET_BIT: u32 = 1 << 3;
const MOX_BIT: u32 = 1 << 24;
const TX_ENABLE_BIT: u32 = 1 << 25;
const TX_RELAY_DISABLE_BIT: u32 = 1 << 27;
const CW_KEYER_ENABLE_BIT: u32 = 1 << 31;
const TX_MODULATION_SOURCE_MASK: u32 = 0b11;
const TX_OUTPUT_GATE_BIT: u32 = 1 << 2;
const TX_PROTOCOL_P2_BIT: u32 = 1 << 3;
const TX_AMPLITUDE_MASK: u32 = 0x3ffff << 4;
const TX_WATCHDOG_OVERRIDE_BIT: u32 = 1 << 28;
const DUC_MUX_RESET_BIT: u32 = 1 << 29;
const TX_IQ_DEINTERLEAVE_BIT: u32 = 1 << 30;
const DUC_STREAM_ENABLE_BIT: u32 = 1 << 31;

const DUC_SAMPLE_RATE_HZ: u64 = 192_000;
const DUC_IQ_PAIRS_PER_FRAME: usize = 240;
const DUC_FRAME_BYTES: usize = 1_440;
const DUC_FIFO_WORDS_PER_FRAME: usize = 180;
const DUC_FRAMES_PER_SECOND: u64 = DUC_SAMPLE_RATE_HZ / DUC_IQ_PAIRS_PER_FRAME as u64;
const DUC_PREFILL_FRAMES: usize = 20;
const DUC_REFILL_LOW_FRAMES: usize = 12;
const DUC_REFILL_TARGET_FRAMES: usize = 20;
const DUC_MAX_DMA_BATCH_FRAMES: usize = 11;
const DMA_BUFFER_BYTES: usize = DUC_MAX_DMA_BATCH_FRAMES * DUC_FRAME_BYTES;
const FIFO_POLL_INTERVAL: Duration = Duration::from_micros(250);
const SOAK_PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
const HISTOGRAM_BUCKET_WIDTH_NS: u128 = 10_000;
const HISTOGRAM_MAX_NS: u128 = 100_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DucIqPattern {
    Zero,
    Changing,
    Carrier,
}

impl DucIqPattern {
    fn from_env() -> Result<Self, XdmaError> {
        match env::var("SATURN_BRIDGE_XDMA_DUC_PATTERN") {
            Ok(value) if value.eq_ignore_ascii_case("zero") => Ok(Self::Zero),
            Ok(value) if value.eq_ignore_ascii_case("changing") => Ok(Self::Changing),
            Ok(value) => Err(XdmaError::Incompatible(format!(
                "SATURN_BRIDGE_XDMA_DUC_PATTERN must be zero or changing, not {value:?}"
            ))),
            Err(env::VarError::NotPresent) => Ok(Self::Zero),
            Err(error) => Err(XdmaError::Incompatible(format!(
                "could not read SATURN_BRIDGE_XDMA_DUC_PATTERN: {error}"
            ))),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::Changing => "changing",
            Self::Carrier => "carrier",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DucProbeConfig {
    duration: Duration,
    iq_pattern: DucIqPattern,
    max_p9999_refill_service: Duration,
    max_p9999_margin_percent: u64,
    cpu: Option<usize>,
    rt_priority: i32,
}

impl DucProbeConfig {
    fn from_env() -> Result<Self, XdmaError> {
        let duration_ms = parse_env_u64(
            "SATURN_BRIDGE_XDMA_DUC_DURATION_MS",
            DEFAULT_PROBE_DURATION_MS,
        )?;
        if !(MIN_PROBE_DURATION_MS..=MAX_PROBE_DURATION_MS).contains(&duration_ms) {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA DUC duration {duration_ms} ms is outside the supported {MIN_PROBE_DURATION_MS}..={MAX_PROBE_DURATION_MS} ms range"
            )));
        }
        let max_p9999_refill_service_us = parse_env_u64(
            "SATURN_BRIDGE_XDMA_DUC_MAX_P9999_REFILL_SERVICE_US",
            DEFAULT_MAX_P9999_REFILL_SERVICE_US,
        )?;
        if !(MIN_MAX_P9999_REFILL_SERVICE_US..=MAX_MAX_P9999_REFILL_SERVICE_US)
            .contains(&max_p9999_refill_service_us)
        {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA DUC p99.99 service gate {max_p9999_refill_service_us} us is outside the supported {MIN_MAX_P9999_REFILL_SERVICE_US}..={MAX_MAX_P9999_REFILL_SERVICE_US} us range"
            )));
        }
        let max_p9999_margin_percent = parse_env_u64(
            "SATURN_BRIDGE_XDMA_DUC_MAX_P9999_MARGIN_PERCENT",
            DEFAULT_MAX_P9999_MARGIN_PERCENT,
        )?;
        if !(MIN_MAX_P9999_MARGIN_PERCENT..=MAX_MAX_P9999_MARGIN_PERCENT)
            .contains(&max_p9999_margin_percent)
        {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA DUC p99.99 margin gate {max_p9999_margin_percent}% is outside the supported {MIN_MAX_P9999_MARGIN_PERCENT}..={MAX_MAX_P9999_MARGIN_PERCENT}% range"
            )));
        }
        let allowed_cpus = allowed_cpu_ids()?;
        let cpu = match env::var("SATURN_BRIDGE_XDMA_DUC_CPU") {
            Ok(value) if value.eq_ignore_ascii_case("none") => None,
            Ok(value) if value.eq_ignore_ascii_case("auto") => allowed_cpus.last().copied(),
            Ok(value) => {
                let cpu = value.parse::<usize>().map_err(|_| {
                    XdmaError::Incompatible(
                        "SATURN_BRIDGE_XDMA_DUC_CPU must be a CPU number, auto, or none".into(),
                    )
                })?;
                if !allowed_cpus.contains(&cpu) {
                    return Err(XdmaError::Incompatible(format!(
                        "direct XDMA DUC CPU {cpu} is unavailable; process can use {allowed_cpus:?}"
                    )));
                }
                Some(cpu)
            }
            Err(env::VarError::NotPresent) => None,
            Err(error) => {
                return Err(XdmaError::Incompatible(format!(
                    "could not read SATURN_BRIDGE_XDMA_DUC_CPU: {error}"
                )));
            }
        };
        let rt_priority = parse_env_u64("SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY", 0)?;
        if rt_priority > MAX_RT_PRIORITY {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA DUC real-time priority {rt_priority} exceeds the supported maximum {MAX_RT_PRIORITY}"
            )));
        }
        Ok(Self {
            duration: Duration::from_millis(duration_ms),
            iq_pattern: DucIqPattern::from_env()?,
            max_p9999_refill_service: Duration::from_micros(max_p9999_refill_service_us),
            max_p9999_margin_percent,
            cpu,
            rt_priority: rt_priority as i32,
        })
    }
}

pub(crate) fn allowed_cpu_ids() -> Result<Vec<usize>, XdmaError> {
    // SAFETY: the set is initialized and passed with its exact size. pid 0
    // queries the calling thread's effective affinity mask.
    let set = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(
            0,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mut set as *mut libc::cpu_set_t,
        ) != 0
        {
            return Err(XdmaError::Io {
                action: "could not determine CPUs available to XDMA DUC probe",
                source: io::Error::last_os_error(),
            });
        }
        set
    };
    let cpus: Vec<usize> = (0..libc::CPU_SETSIZE as usize)
        // SAFETY: every queried index is inside cpu_set_t's supported range.
        .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &set) })
        .collect();
    if cpus.is_empty() {
        return Err(XdmaError::Incompatible(
            "XDMA DUC probe has no CPUs in its affinity mask".into(),
        ));
    }
    Ok(cpus)
}

fn parse_env_u64(name: &'static str, default: u64) -> Result<u64, XdmaError> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| XdmaError::Incompatible(format!("{name} must be an unsigned integer"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(XdmaError::Incompatible(format!(
            "could not read {name}: {error}"
        ))),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FifoSnapshot {
    pub(crate) occupied_words: usize,
    pub(crate) overflow: bool,
    pub(crate) over_threshold: bool,
    pub(crate) underflow: bool,
}

impl FifoSnapshot {
    fn decode(value: u32) -> Self {
        Self {
            occupied_words: (value & 0xffff) as usize,
            overflow: (value & (1 << 31)) != 0,
            over_threshold: (value & (1 << 30)) != 0,
            underflow: (value & (1 << 29)) != 0,
        }
    }
}

fn is_terminal_fifo_condition(snapshot: FifoSnapshot, occupied_frames: usize) -> bool {
    snapshot.overflow || snapshot.over_threshold || snapshot.underflow || occupied_frames <= 2
}

#[derive(Debug, Default)]
struct LatencyHistogram {
    buckets: Box<[u64]>,
    observations: u64,
    overflow_observations: u64,
    max_ns: u128,
}

impl LatencyHistogram {
    fn new() -> Self {
        let bucket_count = HISTOGRAM_MAX_NS.div_ceil(HISTOGRAM_BUCKET_WIDTH_NS) as usize;
        Self {
            buckets: vec![0; bucket_count].into_boxed_slice(),
            ..Self::default()
        }
    }

    fn observe(&mut self, value_ns: u128) {
        self.observations = self.observations.saturating_add(1);
        self.max_ns = self.max_ns.max(value_ns);
        let bucket = value_ns.saturating_sub(1) / HISTOGRAM_BUCKET_WIDTH_NS;
        if let Some(count) = self.buckets.get_mut(bucket as usize) {
            *count = count.saturating_add(1);
        } else {
            self.overflow_observations = self.overflow_observations.saturating_add(1);
        }
    }

    fn percentile_ns(&self, percentile: f64) -> u128 {
        if self.observations == 0 {
            return 0;
        }
        let rank = (percentile.clamp(0.0, 1.0) * self.observations as f64).ceil() as u64;
        let mut cumulative = 0_u64;
        for (index, count) in self.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*count);
            if cumulative >= rank {
                return ((index as u128 + 1) * HISTOGRAM_BUCKET_WIDTH_NS).min(self.max_ns);
            }
        }
        // Values beyond the fixed 100 ms histogram range are represented by
        // their exact maximum. This is conservative for acceptance gates and
        // keeps soak memory usage constant.
        self.max_ns
    }
}

#[derive(Debug, Default)]
struct DucStats {
    dma_writes: u64,
    dma_bytes: u64,
    frames_written: u64,
    max_batch_frames: usize,
    fifo_lwm: usize,
    fifo_hwm: usize,
    fifo_final: usize,
    fifo_overflows: u64,
    fifo_over_threshold: u64,
    fifo_underflows: u64,
    fifo_startup_underflow: bool,
    safety_checks: u64,
    write_time_ns: u128,
    write_latencies: LatencyHistogram,
    refill_gaps: LatencyHistogram,
    refill_service_latencies: LatencyHistogram,
    max_loop_gap_ns: u128,
    low_water_events: u64,
    critical_low_events: u64,
    batch_size_changes: u64,
    terminated_early: bool,
    elapsed: Duration,
}

impl DucStats {
    fn observe_fifo(&mut self, snapshot: FifoSnapshot) {
        self.fifo_lwm = self.fifo_lwm.min(snapshot.occupied_words);
        self.fifo_hwm = self.fifo_hwm.max(snapshot.occupied_words);
        self.fifo_overflows += u64::from(snapshot.overflow);
        self.fifo_over_threshold += u64::from(snapshot.over_threshold);
        self.fifo_underflows += u64::from(snapshot.underflow);
    }

    fn consumed_fifo_words(&self) -> u64 {
        self.frames_written
            .saturating_mul(DUC_FIFO_WORDS_PER_FRAME as u64)
            .saturating_sub(self.fifo_final as u64)
    }

    fn consumed_iq_pairs(&self) -> u64 {
        self.consumed_fifo_words()
            .saturating_mul(DUC_IQ_PAIRS_PER_FRAME as u64)
            / DUC_FIFO_WORDS_PER_FRAME as u64
    }
}

#[derive(Clone, Copy, Debug)]
struct ProgressSnapshot {
    elapsed_seconds: u64,
    dma_writes: u64,
    fifo_lwm: usize,
    max_refill_service_ns: u128,
    critical_low_events: u64,
    fifo_faults: u64,
}

struct ProgressReporter {
    sender: Option<SyncSender<ProgressSnapshot>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProgressReporter {
    fn start() -> Result<Self, XdmaError> {
        let (sender, receiver) = sync_channel::<ProgressSnapshot>(1);
        let thread = thread::Builder::new()
            .name("xdma-duc-progress".into())
            .spawn(move || {
                while let Ok(snapshot) = receiver.recv() {
                    eprintln!(
                        "saturn-bridge: XDMA Phase 4 soak progress elapsed_s={} dma_writes={} fifo_lwm={} max_refill_service_ms={:.3} critical_low_events={} fifo_faults={}",
                        snapshot.elapsed_seconds,
                        snapshot.dma_writes,
                        snapshot.fifo_lwm,
                        snapshot.max_refill_service_ns as f64 / 1_000_000.0,
                        snapshot.critical_low_events,
                        snapshot.fifo_faults,
                    );
                }
            })
            .map_err(|source| XdmaError::Io {
                action: "could not start non-real-time XDMA DUC progress reporter",
                source,
            })?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
        })
    }

    fn report(&self, snapshot: ProgressSnapshot) {
        let Some(sender) = &self.sender else {
            return;
        };
        match sender.try_send(snapshot) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn stop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct DucDmaSession<'a> {
    registers: &'a mut XdmaRegisterDevice,
    dma: File,
    fifo_depth_words: usize,
    buffer: AlignedBuffer,
    iq_pattern: DucIqPattern,
    pattern_pair_index: u64,
    stopped: bool,
}

impl<'a> DucDmaSession<'a> {
    pub(crate) fn start(
        registers: &'a mut XdmaRegisterDevice,
        dma_path: &Path,
    ) -> Result<Self, XdmaError> {
        apply_rf_disabled_duc_state(registers, false)?;
        let dma = match OpenOptions::new().write(true).open(dma_path) {
            Ok(dma) => dma,
            Err(source) => {
                let _ = apply_rf_disabled_duc_shutdown(registers);
                return Err(XdmaError::Io {
                    action: "could not open XDMA DUC transmit device",
                    source,
                });
            }
        };
        let mut buffer = match AlignedBuffer::new(DMA_BUFFER_BYTES) {
            Ok(buffer) => buffer,
            Err(error) => {
                let _ = apply_rf_disabled_duc_shutdown(registers);
                return Err(error);
            }
        };
        if let Err(error) = buffer.lock_memory() {
            let _ = apply_rf_disabled_duc_shutdown(registers);
            return Err(error);
        }
        let fifo_depth_words = duc_fifo_depth_words(registers.identity().firmware_minor);
        let mut session = Self {
            registers,
            dma,
            fifo_depth_words,
            buffer,
            iq_pattern: DucIqPattern::Zero,
            pattern_pair_index: 0,
            stopped: false,
        };
        if let Err(error) = session.configure() {
            let _ = session.stop();
            return Err(error);
        }
        Ok(session)
    }

    fn configure(&mut self) -> Result<(), XdmaError> {
        apply_rf_disabled_duc_state(self.registers, false)?;
        self.pulse_duc_mux_reset()?;
        self.reset_fifo()?;
        self.registers.write_register(
            DUC_FIFO_MONITOR_CONFIG_REGISTER,
            self.fifo_depth_words as u32,
        )?;
        // Clear sticky conditions from the previous owner while the mux is off.
        self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?;
        Ok(())
    }

    fn run(
        &mut self,
        config: DucProbeConfig,
        progress: &ProgressReporter,
    ) -> Result<DucStats, XdmaError> {
        self.iq_pattern = config.iq_pattern;
        let mut stats = DucStats {
            fifo_lwm: usize::MAX,
            write_latencies: LatencyHistogram::new(),
            refill_gaps: LatencyHistogram::new(),
            refill_service_latencies: LatencyHistogram::new(),
            ..DucStats::default()
        };

        // H2C0 writes are discarded while the DUC mux is disabled, so the
        // RF-disabled, zero-amplitude mux must be enabled immediately before
        // seeding the FIFO. Clear the expected empty-FIFO startup condition;
        // every later underflow remains a hard probe failure.
        apply_rf_disabled_duc_state(self.registers, true)?;
        stats.safety_checks += 1;
        let startup =
            FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        stats.fifo_startup_underflow = startup.underflow;
        let mut prefill_frames = DUC_PREFILL_FRAMES;
        while prefill_frames != 0 {
            let frames = prefill_frames.min(DUC_MAX_DMA_BATCH_FRAMES);
            self.write_frames(frames, &mut stats)?;
            prefill_frames -= frames;
        }
        let prefill =
            FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        stats.fifo_startup_underflow |= prefill.underflow;
        let minimum_prefill_words = (DUC_PREFILL_FRAMES - 2) * DUC_FIFO_WORDS_PER_FRAME;
        if prefill.occupied_words < minimum_prefill_words {
            return Err(XdmaError::Incompatible(format!(
                "DUC prefill accepted only {} of at least {} expected FIFO words",
                prefill.occupied_words, minimum_prefill_words
            )));
        }
        stats.fifo_lwm = prefill.occupied_words;
        stats.fifo_hwm = prefill.occupied_words;

        apply_rf_disabled_duc_state(self.registers, true)?;
        stats.safety_checks += 1;

        let started = Instant::now();
        let mut next_progress_at = started + SOAK_PROGRESS_INTERVAL;
        let mut previous_loop_at = started;
        let mut previous_refill_at = started;
        let mut low_water_active = false;
        let mut critical_low_active = false;
        let mut previous_batch_frames = DUC_PREFILL_FRAMES % DUC_MAX_DMA_BATCH_FRAMES;
        while started.elapsed() < config.duration {
            let loop_at = Instant::now();
            let loop_gap = loop_at.saturating_duration_since(previous_loop_at);
            previous_loop_at = loop_at;
            stats.max_loop_gap_ns = stats.max_loop_gap_ns.max(loop_gap.as_nanos());

            let fifo =
                FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
            stats.observe_fifo(fifo);

            let occupied_frames = fifo.occupied_words / DUC_FIFO_WORDS_PER_FRAME;
            let low_now = occupied_frames <= DUC_REFILL_LOW_FRAMES;
            if low_now && !low_water_active {
                stats.low_water_events += 1;
            }
            low_water_active = low_now;
            let critical_now = occupied_frames <= 2;
            if critical_now && !critical_low_active {
                stats.critical_low_events += 1;
            }
            critical_low_active = critical_now;
            if is_terminal_fifo_condition(fifo, occupied_frames) {
                stats.terminated_early = true;
                break;
            }

            if low_now {
                let frames = refill_batch_frames(
                    fifo.occupied_words,
                    self.fifo_depth_words,
                    DUC_REFILL_TARGET_FRAMES,
                );
                if frames != 0 {
                    let refill_started = Instant::now();
                    // Configuration is owned exclusively for the probe.
                    // Re-read and verify the three safety registers before
                    // every refill, but avoid rewriting unchanged values on
                    // the 100-200 Hz hot path.
                    verify_rf_disabled_duc_state(self.registers, true)?;
                    stats.safety_checks += 1;
                    stats.refill_gaps.observe(
                        loop_at
                            .saturating_duration_since(previous_refill_at)
                            .as_nanos(),
                    );
                    previous_refill_at = loop_at;
                    if frames != previous_batch_frames {
                        stats.batch_size_changes += 1;
                        previous_batch_frames = frames;
                    }
                    self.write_frames(frames, &mut stats)?;
                    stats
                        .refill_service_latencies
                        .observe(refill_started.elapsed().as_nanos());
                    continue;
                }
            }
            if loop_at >= next_progress_at {
                progress.report(ProgressSnapshot {
                    elapsed_seconds: started.elapsed().as_secs(),
                    dma_writes: stats.dma_writes,
                    fifo_lwm: stats.fifo_lwm,
                    max_refill_service_ns: stats.refill_service_latencies.max_ns,
                    critical_low_events: stats.critical_low_events,
                    fifo_faults: stats.fifo_overflows
                        + stats.fifo_over_threshold
                        + stats.fifo_underflows,
                });
                next_progress_at += SOAK_PROGRESS_INTERVAL;
            }
            thread::sleep(FIFO_POLL_INTERVAL);
        }
        stats.elapsed = started.elapsed();

        let final_fifo =
            FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        stats.observe_fifo(final_fifo);
        stats.fifo_final = final_fifo.occupied_words;
        apply_rf_disabled_duc_state(self.registers, true)?;
        stats.safety_checks += 1;

        Ok(stats)
    }

    fn write_frames(&mut self, frames: usize, stats: &mut DucStats) -> Result<(), XdmaError> {
        if frames == 0 || frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "invalid DUC DMA batch of {frames} frames"
            )));
        }
        let bytes = frames * DUC_FRAME_BYTES;
        fill_iq_pattern(
            self.buffer.as_mut_slice(bytes),
            self.iq_pattern,
            &mut self.pattern_pair_index,
        );
        let write_started = Instant::now();
        let written = self
            .dma
            .write_at(self.buffer.as_slice(bytes), DUC_DMA_AXI_OFFSET)
            .map_err(|source| XdmaError::Io {
                action: "could not write IQ to XDMA DUC stream",
                source,
            })?;
        let write_time = write_started.elapsed().as_nanos();
        if written != bytes {
            return Err(XdmaError::Io {
                action: "XDMA DUC stream returned a short write",
                source: io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("transferred {written} of {bytes} bytes"),
                ),
            });
        }
        stats.dma_writes += 1;
        stats.dma_bytes += written as u64;
        stats.frames_written += frames as u64;
        stats.max_batch_frames = stats.max_batch_frames.max(frames);
        stats.write_time_ns += write_time;
        stats.write_latencies.observe(write_time);
        Ok(())
    }

    /// Seed the proven Phase 4 FIFO geometry with a full-scale constant
    /// complex carrier while RF remains inhibited and TX amplitude is zero.
    /// The caller must explicitly arm and key the RF path afterward.
    pub(crate) fn prefill_guarded_carrier(&mut self) -> Result<FifoSnapshot, XdmaError> {
        self.iq_pattern = DucIqPattern::Carrier;
        apply_rf_disabled_duc_state(self.registers, true)?;
        // Clear the expected empty-FIFO condition before the guarded prefill.
        self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?;
        let mut remaining = DUC_PREFILL_FRAMES;
        let mut stats = DucStats::default();
        while remaining != 0 {
            let frames = remaining.min(DUC_MAX_DMA_BATCH_FRAMES);
            self.write_frames(frames, &mut stats)?;
            remaining -= frames;
        }
        let snapshot =
            FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        let minimum_words = (DUC_PREFILL_FRAMES - 2) * DUC_FIFO_WORDS_PER_FRAME;
        if snapshot.occupied_words < minimum_words {
            return Err(XdmaError::Incompatible(format!(
                "guarded TX prefill accepted only {} of at least {} expected FIFO words",
                snapshot.occupied_words, minimum_words
            )));
        }
        Ok(snapshot)
    }

    /// Service one guarded-TX refill cycle and return the post-service FIFO
    /// snapshot. Runtime FIFO faults are never recoverable during keyed RF.
    pub(crate) fn service_guarded_carrier(&mut self) -> Result<FifoSnapshot, XdmaError> {
        let before = FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        if before.overflow || before.over_threshold || before.underflow {
            return Err(XdmaError::Incompatible(format!(
                "guarded TX DUC FIFO fault: occupied={} overflow={} threshold={} underflow={}",
                before.occupied_words, before.overflow, before.over_threshold, before.underflow
            )));
        }
        let occupied_frames = before.occupied_words / DUC_FIFO_WORDS_PER_FRAME;
        if occupied_frames <= 2 {
            return Err(XdmaError::Incompatible(format!(
                "guarded TX DUC reached critical FIFO level: {} words",
                before.occupied_words
            )));
        }
        if occupied_frames <= DUC_REFILL_LOW_FRAMES {
            let frames = refill_batch_frames(
                before.occupied_words,
                self.fifo_depth_words,
                DUC_REFILL_TARGET_FRAMES,
            );
            if frames != 0 {
                let mut stats = DucStats::default();
                self.write_frames(frames, &mut stats)?;
            }
        }
        let after = FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        if after.overflow || after.over_threshold || after.underflow {
            return Err(XdmaError::Incompatible(format!(
                "guarded TX DUC FIFO fault after refill: occupied={} overflow={} threshold={} underflow={}",
                after.occupied_words, after.overflow, after.over_threshold, after.underflow
            )));
        }
        Ok(after)
    }

    pub(crate) fn registers(&self) -> &XdmaRegisterDevice {
        self.registers
    }

    fn pulse_duc_mux_reset(&self) -> Result<(), XdmaError> {
        let base = self.registers.read_register(TX_CONFIG_REGISTER)?
            & !(DUC_STREAM_ENABLE_BIT | DUC_MUX_RESET_BIT);
        self.registers
            .write_register(TX_CONFIG_REGISTER, base | DUC_MUX_RESET_BIT)?;
        self.registers.write_register(TX_CONFIG_REGISTER, base)
    }

    fn reset_fifo(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value & !DUC_FIFO_RESET_BIT,
            "could not assert direct DUC FIFO reset",
        )?;
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value | DUC_FIFO_RESET_BIT,
            "could not release direct DUC FIFO reset",
        )
    }

    pub(crate) fn stop(&mut self) -> Result<(), XdmaError> {
        if self.stopped {
            return Ok(());
        }
        let disable = apply_rf_disabled_duc_shutdown(self.registers);
        let reset = self.reset_fifo();
        let result = disable.and(reset);
        self.stopped = result.is_ok();
        result
    }
}

impl Drop for DucDmaSession<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("saturn-bridge: XDMA DUC emergency cleanup failed: {error}");
        }
    }
}

pub fn run_phase4_duc_probe() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let config = DucProbeConfig::from_env()?;
    // Start the reporter before changing this thread's affinity or scheduler.
    // It therefore remains ordinary SCHED_OTHER work and can never block the
    // real-time refill loop on terminal or journal output.
    let mut progress = ProgressReporter::start()?;
    if let Some(cpu) = config.cpu {
        pin_current_thread(cpu)?;
    }
    if config.rt_priority != 0 {
        enable_realtime_fifo(config.rt_priority)?;
    }
    let (scheduler_policy, scheduler_priority) = current_scheduler()?;
    let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
    let duc_path = env::var_os("SATURN_BRIDGE_XDMA_DUC_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DUC_DEVICE));

    let mut registers = XdmaRegisterDevice::open(&register_path)?;
    let identity: SaturnIdentity = registers.identity().clone();
    let mut session = DucDmaSession::start(&mut registers, &duc_path)?;
    let probe = session.run(config, &progress);
    let stop = session.stop();
    drop(session);
    progress.stop();
    let stats = probe?;
    stop?;
    registers.close_safely()?;
    let validation = validate_duc_stats(&stats, config);

    let elapsed = stats.elapsed.as_secs_f64().max(0.001);
    let pair_rate = stats.consumed_iq_pairs() as f64 / elapsed;
    let payload_mbps = stats.dma_bytes as f64 * 8.0 / elapsed / 1_000_000.0;
    let average_batch = stats.frames_written as f64 / stats.dma_writes.max(1) as f64;
    let average_write_us = stats.write_time_ns as f64 / stats.dma_writes.max(1) as f64 / 1_000.0;
    let max_write_us = stats.write_latencies.max_ns as f64 / 1_000.0;
    let p99_write_us = stats.write_latencies.percentile_ns(0.99) as f64 / 1_000.0;
    let p9999_write_us = stats.write_latencies.percentile_ns(0.9999) as f64 / 1_000.0;
    let p99_refill_gap_ms = stats.refill_gaps.percentile_ns(0.99) as f64 / 1_000_000.0;
    let p9999_refill_gap_ms = stats.refill_gaps.percentile_ns(0.9999) as f64 / 1_000_000.0;
    let p99_refill_service_ms =
        stats.refill_service_latencies.percentile_ns(0.99) as f64 / 1_000_000.0;
    let p9999_refill_service_ms =
        stats.refill_service_latencies.percentile_ns(0.9999) as f64 / 1_000_000.0;
    let max_refill_service_ms = stats.refill_service_latencies.max_ns as f64 / 1_000_000.0;
    let max_loop_gap_ms = stats.max_loop_gap_ns as f64 / 1_000_000.0;
    let minimum_fifo_margin_ms = fifo_margin_ns(stats.fifo_lwm) as f64 / 1_000_000.0;
    let p9999_margin_percent = if minimum_fifo_margin_ms > 0.0 {
        p9999_refill_service_ms / minimum_fifo_margin_ms * 100.0
    } else {
        f64::INFINITY
    };
    let histogram_overflows = stats.write_latencies.overflow_observations
        + stats.refill_gaps.overflow_observations
        + stats.refill_service_latencies.overflow_observations;
    let p9999_sample_sufficient = stats.refill_service_latencies.observations >= 10_000;
    let validation_error = validation.as_ref().err().map(ToString::to_string);
    let validation_status = if validation.is_ok() {
        "passed"
    } else {
        "failed"
    };
    let margin_used = if p9999_margin_percent.is_finite() {
        TelemetryValue::number(p9999_margin_percent)
    } else {
        TelemetryValue::text("infinite")
    };
    record_probe_outcome(
        4,
        "duc-performance",
        validation_status,
        "receive-safe-verified",
        validation_error.as_deref(),
        &[
            (
                "device",
                TelemetryValue::text(duc_path.display().to_string()),
            ),
            ("product", TelemetryValue::number(identity.product_id)),
            ("pcb", TelemetryValue::number(identity.pcb_version)),
            (
                "firmware",
                TelemetryValue::text(format!(
                    "{}.{}",
                    identity.firmware_major, identity.firmware_minor
                )),
            ),
            (
                "duration_ms",
                TelemetryValue::number(stats.elapsed.as_millis()),
            ),
            (
                "terminated_early",
                TelemetryValue::boolean(stats.terminated_early),
            ),
            (
                "iq_pattern",
                TelemetryValue::text(config.iq_pattern.label()),
            ),
            (
                "cpu",
                TelemetryValue::text(
                    config
                        .cpu
                        .map_or_else(|| "none".to_string(), |cpu| cpu.to_string()),
                ),
            ),
            (
                "scheduler",
                TelemetryValue::text(scheduler_policy.to_string()),
            ),
            (
                "scheduler_priority",
                TelemetryValue::number(scheduler_priority),
            ),
            ("target_rate_hz", TelemetryValue::number(DUC_SAMPLE_RATE_HZ)),
            (
                "frames_written",
                TelemetryValue::number(stats.frames_written),
            ),
            (
                "iq_pairs_consumed",
                TelemetryValue::number(stats.consumed_iq_pairs()),
            ),
            ("iq_rate", TelemetryValue::number(pair_rate)),
            ("dma_writes", TelemetryValue::number(stats.dma_writes)),
            ("dma_bytes", TelemetryValue::number(stats.dma_bytes)),
            ("payload_mbps", TelemetryValue::number(payload_mbps)),
            ("average_batch", TelemetryValue::number(average_batch)),
            (
                "p9999_sample_sufficient",
                TelemetryValue::boolean(p9999_sample_sufficient),
            ),
            ("p9999_write_us", TelemetryValue::number(p9999_write_us)),
            ("max_write_us", TelemetryValue::number(max_write_us)),
            (
                "p9999_refill_gap_ms",
                TelemetryValue::number(p9999_refill_gap_ms),
            ),
            (
                "p9999_refill_service_ms",
                TelemetryValue::number(p9999_refill_service_ms),
            ),
            (
                "max_refill_service_ms",
                TelemetryValue::number(max_refill_service_ms),
            ),
            (
                "minimum_fifo_margin_ms",
                TelemetryValue::number(minimum_fifo_margin_ms),
            ),
            ("p9999_margin_used_percent", margin_used),
            ("max_loop_gap_ms", TelemetryValue::number(max_loop_gap_ms)),
            (
                "histogram_overflows",
                TelemetryValue::number(histogram_overflows),
            ),
            ("fifo_lwm", TelemetryValue::number(stats.fifo_lwm)),
            ("fifo_hwm", TelemetryValue::number(stats.fifo_hwm)),
            ("fifo_final", TelemetryValue::number(stats.fifo_final)),
            (
                "critical_low_events",
                TelemetryValue::number(stats.critical_low_events),
            ),
            (
                "fifo_overflow",
                TelemetryValue::number(stats.fifo_overflows),
            ),
            (
                "fifo_threshold",
                TelemetryValue::number(stats.fifo_over_threshold),
            ),
            (
                "fifo_underflow",
                TelemetryValue::number(stats.fifo_underflows),
            ),
            ("safety_checks", TelemetryValue::number(stats.safety_checks)),
            ("amplitude_zero", TelemetryValue::boolean(true)),
            ("rf_keyed", TelemetryValue::boolean(false)),
        ],
    );

    println!(
        "saturn-bridge: XDMA Phase 4 DUC probe {} product={} pcb={} firmware={}.{} device={} duration_ms={} terminated_early={} iq_pattern={} cpu={} scheduler={} scheduler_priority={} dma_buffer_locked=1 target_rate={}Hz target_frames_s={} refill_low_frames={} refill_target_frames={} frames_written={} iq_pairs_consumed={} iq_rate={:.1}/s dma_writes={} dma_bytes={} payload={:.3}Mbit/s average_batch={:.2} max_batch={} batch_changes={} write_observations={} refill_observations={} p99.99_sample_sufficient={} average_write={:.1}us p99_write={:.1}us p99.99_write={:.1}us max_write={:.1}us p99_refill_gap={:.3}ms p99.99_refill_gap={:.3}ms p99_refill_service={:.3}ms p99.99_refill_service={:.3}ms max_refill_service={:.3}ms p99.99_margin_used={} max_loop_gap={:.3}ms histogram_overflows={} fifo_depth={} fifo_lwm={} fifo_hwm={} fifo_final={} minimum_fifo_margin={:.3}ms low_water_events={} critical_low_events={} fifo_startup_underflow={} fifo_overflow={} fifo_threshold={} fifo_underflow={} safety_checks={} amplitude_zero=1 mox=0 tx_enable=0 pa_relay=0 cw=0",
        if validation.is_ok() { "passed" } else { "FAILED" },
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        duc_path.display(),
        stats.elapsed.as_millis(),
        u8::from(stats.terminated_early),
        config.iq_pattern.label(),
        config
            .cpu
            .map_or_else(|| "none".to_string(), |cpu| cpu.to_string()),
        scheduler_policy,
        scheduler_priority,
        DUC_SAMPLE_RATE_HZ,
        DUC_FRAMES_PER_SECOND,
        DUC_REFILL_LOW_FRAMES,
        DUC_REFILL_TARGET_FRAMES,
        stats.frames_written,
        stats.consumed_iq_pairs(),
        pair_rate,
        stats.dma_writes,
        stats.dma_bytes,
        payload_mbps,
        average_batch,
        stats.max_batch_frames,
        stats.batch_size_changes,
        stats.write_latencies.observations,
        stats.refill_service_latencies.observations,
        u8::from(p9999_sample_sufficient),
        average_write_us,
        p99_write_us,
        p9999_write_us,
        max_write_us,
        p99_refill_gap_ms,
        p9999_refill_gap_ms,
        p99_refill_service_ms,
        p9999_refill_service_ms,
        max_refill_service_ms,
        if p9999_margin_percent.is_finite() {
            format!("{p9999_margin_percent:.1}%")
        } else {
            "infinite".to_string()
        },
        max_loop_gap_ms,
        histogram_overflows,
        duc_fifo_depth_words(identity.firmware_minor),
        stats.fifo_lwm,
        stats.fifo_hwm,
        stats.fifo_final,
        minimum_fifo_margin_ms,
        stats.low_water_events,
        stats.critical_low_events,
        u8::from(stats.fifo_startup_underflow),
        stats.fifo_overflows,
        stats.fifo_over_threshold,
        stats.fifo_underflows,
        stats.safety_checks,
    );
    println!(
        "saturn-bridge: XDMA Phase 4 cleanup completed; DUC stream and output gate disabled, DUC FIFO reset, zero amplitude retained, and RF remains receive-safe"
    );
    validation?;
    Ok(())
}

fn validate_duc_stats(stats: &DucStats, config: DucProbeConfig) -> Result<(), XdmaError> {
    if stats.fifo_overflows != 0 || stats.fifo_over_threshold != 0 || stats.fifo_underflows != 0 {
        return Err(XdmaError::Incompatible(format!(
            "DUC runtime FIFO fault: overflow={} threshold={} underflow={}",
            stats.fifo_overflows, stats.fifo_over_threshold, stats.fifo_underflows
        )));
    }
    if stats.critical_low_events != 0 {
        return Err(XdmaError::Incompatible(format!(
            "DUC reached the critical FIFO low-water boundary {} time(s)",
            stats.critical_low_events
        )));
    }
    let pair_rate = stats.consumed_iq_pairs() as f64 / stats.elapsed.as_secs_f64().max(0.001);
    let minimum_rate = DUC_SAMPLE_RATE_HZ as f64 * 0.95;
    let maximum_rate = DUC_SAMPLE_RATE_HZ as f64 * 1.05;
    if !(minimum_rate..=maximum_rate).contains(&pair_rate) {
        return Err(XdmaError::Incompatible(format!(
            "DUC consumed IQ at {pair_rate:.1} pairs/s; expected within 5% of {DUC_SAMPLE_RATE_HZ}"
        )));
    }
    let p9999_refill_service = stats.refill_service_latencies.percentile_ns(0.9999);
    if p9999_refill_service > config.max_p9999_refill_service.as_nanos() {
        return Err(XdmaError::Incompatible(format!(
            "DUC p99.99 refill service {:.3} ms exceeds the {:.3} ms Phase 4 performance gate",
            p9999_refill_service as f64 / 1_000_000.0,
            config.max_p9999_refill_service.as_secs_f64() * 1_000.0
        )));
    }
    let minimum_fifo_margin_ns = fifo_margin_ns(stats.fifo_lwm);
    if stats.refill_service_latencies.max_ns >= minimum_fifo_margin_ns {
        return Err(XdmaError::Incompatible(format!(
            "DUC maximum refill service {:.3} ms exhausted the minimum {:.3} ms FIFO margin",
            stats.refill_service_latencies.max_ns as f64 / 1_000_000.0,
            minimum_fifo_margin_ns as f64 / 1_000_000.0,
        )));
    }
    let p9999_margin_percent =
        p9999_refill_service.saturating_mul(100) / minimum_fifo_margin_ns.max(1);
    if p9999_margin_percent > config.max_p9999_margin_percent as u128 {
        return Err(XdmaError::Incompatible(format!(
            "DUC p99.99 refill service uses {p9999_margin_percent}% of the minimum FIFO margin; gate is {}%",
            config.max_p9999_margin_percent
        )));
    }
    Ok(())
}

pub(crate) fn pin_current_thread(cpu: usize) -> Result<(), XdmaError> {
    // SAFETY: cpu_set_t is initialized before use, cpu was range-checked
    // against the process's available parallelism, and pid 0 means the
    // calling thread.
    let result = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(
            0,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set as *const libc::cpu_set_t,
        )
    };
    if result != 0 {
        return Err(XdmaError::Io {
            action: "could not pin XDMA DUC probe to its dedicated CPU",
            source: io::Error::last_os_error(),
        });
    }
    Ok(())
}

pub(crate) fn enable_realtime_fifo(priority: i32) -> Result<(), XdmaError> {
    let parameter = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: pid 0 targets the calling thread, SCHED_FIFO accepts the
    // initialized sched_param, and the priority was range-limited above.
    let result = unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &parameter) };
    if result != 0 {
        return Err(XdmaError::Io {
            action:
                "could not enable SCHED_FIFO for XDMA DUC probe (run as root or grant CAP_SYS_NICE)",
            source: io::Error::last_os_error(),
        });
    }
    Ok(())
}

pub(crate) fn current_scheduler() -> Result<(&'static str, i32), XdmaError> {
    // SAFETY: pid 0 queries the calling thread and the parameter pointer is
    // valid for the duration of the syscall.
    let (policy, parameter) = unsafe {
        let policy = libc::sched_getscheduler(0);
        let mut parameter: libc::sched_param = std::mem::zeroed();
        let result = libc::sched_getparam(0, &mut parameter);
        if policy == -1 || result != 0 {
            return Err(XdmaError::Io {
                action: "could not query XDMA DUC probe scheduler",
                source: io::Error::last_os_error(),
            });
        }
        (policy, parameter)
    };
    let label = match policy {
        libc::SCHED_FIFO => "fifo",
        libc::SCHED_RR => "rr",
        libc::SCHED_BATCH => "batch",
        libc::SCHED_IDLE => "idle",
        _ => "other",
    };
    Ok((label, parameter.sched_priority))
}

fn apply_rf_disabled_duc_state(
    registers: &XdmaRegisterDevice,
    stream_enabled: bool,
) -> Result<(), XdmaError> {
    registers.update_register(
        RF_GPIO_REGISTER,
        |value| (value & !(MOX_BIT | TX_ENABLE_BIT)) | TX_RELAY_DISABLE_BIT,
        "could not force RF GPIO off for direct DUC probe",
    )?;
    registers.update_register(
        KEYER_CONFIG_REGISTER,
        |value| value & !CW_KEYER_ENABLE_BIT,
        "could not disable CW keyer for direct DUC probe",
    )?;
    registers.update_register(
        TX_CONFIG_REGISTER,
        |value| phase4_tx_config(value, stream_enabled),
        "could not configure RF-disabled direct DUC stream",
    )?;
    verify_rf_disabled_duc_state(registers, stream_enabled)
}

fn apply_rf_disabled_duc_shutdown(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    registers.update_register(
        RF_GPIO_REGISTER,
        |value| (value & !(MOX_BIT | TX_ENABLE_BIT)) | TX_RELAY_DISABLE_BIT,
        "could not force RF GPIO off during direct DUC cleanup",
    )?;
    registers.update_register(
        KEYER_CONFIG_REGISTER,
        |value| value & !CW_KEYER_ENABLE_BIT,
        "could not disable CW keyer during direct DUC cleanup",
    )?;
    registers.update_register(
        TX_CONFIG_REGISTER,
        |value| {
            phase4_tx_config(value, false)
                & !(TX_OUTPUT_GATE_BIT | DUC_STREAM_ENABLE_BIT | DUC_MUX_RESET_BIT)
        },
        "could not shut down direct DUC stream",
    )?;
    verify_rf_disabled_duc_shutdown(registers)
}

fn phase4_tx_config(current: u32, stream_enabled: bool) -> u32 {
    let mut next = current
        & !(TX_MODULATION_SOURCE_MASK
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT
            | DUC_STREAM_ENABLE_BIT);
    next |= TX_OUTPUT_GATE_BIT | TX_PROTOCOL_P2_BIT;
    if stream_enabled {
        next |= DUC_STREAM_ENABLE_BIT;
    }
    next
}

fn verify_rf_disabled_duc_state(
    registers: &XdmaRegisterDevice,
    stream_enabled: bool,
) -> Result<(), XdmaError> {
    let gpio = registers.read_register(RF_GPIO_REGISTER)?;
    let keyer = registers.read_register(KEYER_CONFIG_REGISTER)?;
    let tx = registers.read_register(TX_CONFIG_REGISTER)?;
    let expected_stream = if stream_enabled {
        DUC_STREAM_ENABLE_BIT
    } else {
        0
    };
    let unsafe_gpio = gpio & (MOX_BIT | TX_ENABLE_BIT);
    let required_gpio = gpio & TX_RELAY_DISABLE_BIT;
    let unsafe_tx = tx
        & (TX_MODULATION_SOURCE_MASK
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT);
    let required_tx = tx & (TX_OUTPUT_GATE_BIT | TX_PROTOCOL_P2_BIT);
    if unsafe_gpio != 0
        || required_gpio == 0
        || keyer & CW_KEYER_ENABLE_BIT != 0
        || unsafe_tx != 0
        || required_tx != TX_OUTPUT_GATE_BIT | TX_PROTOCOL_P2_BIT
        || tx & DUC_STREAM_ENABLE_BIT != expected_stream
    {
        return Err(XdmaError::Incompatible(format!(
            "direct DUC RF safety verification failed: gpio=0x{gpio:08x} keyer=0x{keyer:08x} tx=0x{tx:08x}"
        )));
    }
    Ok(())
}

fn verify_rf_disabled_duc_shutdown(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    let gpio = registers.read_register(RF_GPIO_REGISTER)?;
    let keyer = registers.read_register(KEYER_CONFIG_REGISTER)?;
    let tx = registers.read_register(TX_CONFIG_REGISTER)?;
    if gpio & (MOX_BIT | TX_ENABLE_BIT) != 0
        || gpio & TX_RELAY_DISABLE_BIT == 0
        || keyer & CW_KEYER_ENABLE_BIT != 0
        || tx
            & (TX_AMPLITUDE_MASK
                | TX_OUTPUT_GATE_BIT
                | TX_WATCHDOG_OVERRIDE_BIT
                | DUC_MUX_RESET_BIT
                | TX_IQ_DEINTERLEAVE_BIT
                | DUC_STREAM_ENABLE_BIT)
            != 0
    {
        return Err(XdmaError::Incompatible(format!(
            "direct DUC shutdown verification failed: gpio=0x{gpio:08x} keyer=0x{keyer:08x} tx=0x{tx:08x}"
        )));
    }
    Ok(())
}

fn duc_fifo_depth_words(firmware_minor: u16) -> usize {
    match firmware_minor {
        13.. => 4_096,
        10..=12 => 2_048,
        _ => 1_024,
    }
}

fn refill_batch_frames(
    occupied_words: usize,
    fifo_depth_words: usize,
    target_frames: usize,
) -> usize {
    let fifo_frames = fifo_depth_words / DUC_FIFO_WORDS_PER_FRAME;
    let target_words = target_frames.min(fifo_frames) * DUC_FIFO_WORDS_PER_FRAME;
    if occupied_words >= target_words {
        return 0;
    }
    let needed_words = target_words - occupied_words;
    let needed_frames = needed_words.div_ceil(DUC_FIFO_WORDS_PER_FRAME);
    let free_frames = fifo_depth_words.saturating_sub(occupied_words) / DUC_FIFO_WORDS_PER_FRAME;
    needed_frames.min(free_frames).min(DUC_MAX_DMA_BATCH_FRAMES)
}

fn fill_iq_pattern(buffer: &mut [u8], pattern: DucIqPattern, pair_index: &mut u64) {
    if pattern == DucIqPattern::Zero {
        buffer.fill(0);
        *pair_index = pair_index.saturating_add((buffer.len() / 6) as u64);
        return;
    }
    if pattern == DucIqPattern::Carrier {
        for pair in buffer.chunks_exact_mut(6) {
            // Match P2_app's proven InDUCIQ conversion exactly: the FPGA DMA
            // stream receives Q followed by I, both as signed 24-bit
            // big-endian samples, while RF GPIO byte swapping is enabled.
            write_i24_be(&mut pair[0..3], 0);
            write_i24_be(&mut pair[3..6], 0x007f_ffff);
        }
        *pair_index = pair_index.saturating_add((buffer.len() / 6) as u64);
        return;
    }

    for pair in buffer.chunks_exact_mut(6) {
        // A repeatable quadrature ramp exercises every DMA data bit without
        // enabling RF: TX amplitude remains zero and all RF controls are
        // verified off immediately before each write.
        let phase = (*pair_index & 0xffff) as i32 - 0x8000;
        let i = phase << 7;
        let q = -i;
        write_i24_le(&mut pair[0..3], i);
        write_i24_le(&mut pair[3..6], q);
        *pair_index = pair_index.wrapping_add(1);
    }
}

fn write_i24_le(target: &mut [u8], value: i32) {
    let encoded = value.to_le_bytes();
    target.copy_from_slice(&encoded[..3]);
}

fn write_i24_be(target: &mut [u8], value: i32) {
    let encoded = value.to_be_bytes();
    target.copy_from_slice(&encoded[1..]);
}

fn fifo_margin_ns(occupied_words: usize) -> u128 {
    occupied_words as u128 * 1_000_000_000
        / (DUC_FIFO_WORDS_PER_FRAME as u128 * DUC_FRAMES_PER_SECOND as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duc_geometry_matches_saturn_protocol_two() {
        assert_eq!(DUC_FRAME_BYTES, DUC_IQ_PAIRS_PER_FRAME * 6);
        assert_eq!(DUC_FRAME_BYTES, DUC_FIFO_WORDS_PER_FRAME * 8);
        assert_eq!(DUC_FRAMES_PER_SECOND, 800);
        assert_eq!(DMA_BUFFER_BYTES, 15_840);
    }

    #[test]
    fn phase4_tx_config_forces_zero_iq_rf_disabled_test_mode() {
        let unsafe_value = TX_MODULATION_SOURCE_MASK
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT;
        let configured = phase4_tx_config(unsafe_value | (1 << 26), true);
        assert_eq!(
            configured
                & (TX_MODULATION_SOURCE_MASK
                    | TX_AMPLITUDE_MASK
                    | TX_WATCHDOG_OVERRIDE_BIT
                    | DUC_MUX_RESET_BIT
                    | TX_IQ_DEINTERLEAVE_BIT),
            0
        );
        assert_eq!(
            configured & (TX_OUTPUT_GATE_BIT | TX_PROTOCOL_P2_BIT | DUC_STREAM_ENABLE_BIT),
            TX_OUTPUT_GATE_BIT | TX_PROTOCOL_P2_BIT | DUC_STREAM_ENABLE_BIT
        );
        assert_ne!(configured & (1 << 26), 0);
    }

    #[test]
    fn refill_window_uses_fifo_headroom_and_bounds_each_dma_batch() {
        assert_eq!(refill_batch_frames(20 * 180, 4_096, 20), 0);
        assert_eq!(refill_batch_frames(12 * 180, 4_096, 20), 8);
        assert_eq!(refill_batch_frames(11 * 180 + 1, 4_096, 20), 9);
        assert_eq!(refill_batch_frames(0, 4_096, 20), 11);
        assert_eq!(refill_batch_frames(0, 1_024, 20), 5);
        assert_eq!(refill_batch_frames(4 * 180, 4_096, 10), 6);
        assert_eq!(refill_batch_frames(2 * 180, 4_096, 20), 11);
    }

    #[test]
    fn fixed_histogram_is_bounded_and_conservative() {
        let mut histogram = LatencyHistogram::new();
        for value in [10_000, 20_000, 30_000, 40_000, 50_000] {
            histogram.observe(value);
        }
        assert_eq!(histogram.observations, 5);
        assert_eq!(histogram.percentile_ns(0.5), 30_000);
        assert_eq!(histogram.percentile_ns(0.99), 50_000);
        assert_eq!(histogram.percentile_ns(0.9999), 50_000);
        assert_eq!(histogram.max_ns, 50_000);

        histogram.observe(HISTOGRAM_MAX_NS + 123);
        assert_eq!(histogram.overflow_observations, 1);
        assert_eq!(histogram.percentile_ns(1.0), HISTOGRAM_MAX_NS + 123);
    }

    #[test]
    fn changing_iq_pattern_is_deterministic_and_nonzero() {
        let mut buffer = [0_u8; 12];
        let mut pair_index = 0;
        fill_iq_pattern(&mut buffer, DucIqPattern::Changing, &mut pair_index);
        assert_eq!(pair_index, 2);
        assert_ne!(buffer, [0_u8; 12]);
        assert_eq!(&buffer[0..3], &[0x00, 0x00, 0xc0]);
        assert_eq!(&buffer[3..6], &[0x00, 0x00, 0x40]);

        fill_iq_pattern(&mut buffer, DucIqPattern::Zero, &mut pair_index);
        assert_eq!(buffer, [0_u8; 12]);
        assert_eq!(pair_index, 4);
    }

    #[test]
    fn guarded_carrier_matches_p2app_q_then_i_network_order() {
        let mut buffer = [0xa5; 12];
        let mut pair_index = 0;
        fill_iq_pattern(&mut buffer, DucIqPattern::Carrier, &mut pair_index);
        assert_eq!(
            buffer,
            [0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff,]
        );
        assert_eq!(pair_index, 2);
    }

    #[test]
    fn fifo_margin_converts_words_to_time() {
        assert_eq!(fifo_margin_ns(180), 1_250_000);
        assert_eq!(fifo_margin_ns(900), 6_250_000);
    }

    #[test]
    fn fifo_depth_tracks_firmware_generation() {
        assert_eq!(duc_fifo_depth_words(9), 1_024);
        assert_eq!(duc_fifo_depth_words(10), 2_048);
        assert_eq!(duc_fifo_depth_words(12), 2_048);
        assert_eq!(duc_fifo_depth_words(13), 4_096);
        assert_eq!(duc_fifo_depth_words(19), 4_096);
    }

    #[test]
    fn fifo_snapshot_decodes_duc_flags_and_occupancy() {
        let snapshot = FifoSnapshot::decode(0xe000_05a0);
        assert_eq!(snapshot.occupied_words, 1_440);
        assert!(snapshot.overflow);
        assert!(snapshot.over_threshold);
        assert!(snapshot.underflow);
    }

    #[test]
    fn phase4_stops_on_fifo_fault_or_critical_margin() {
        assert!(!is_terminal_fifo_condition(
            FifoSnapshot {
                occupied_words: 3 * DUC_FIFO_WORDS_PER_FRAME,
                ..FifoSnapshot::default()
            },
            3
        ));
        assert!(is_terminal_fifo_condition(
            FifoSnapshot {
                occupied_words: 2 * DUC_FIFO_WORDS_PER_FRAME,
                ..FifoSnapshot::default()
            },
            2
        ));
        assert!(is_terminal_fifo_condition(
            FifoSnapshot {
                occupied_words: 7 * DUC_FIFO_WORDS_PER_FRAME,
                underflow: true,
                ..FifoSnapshot::default()
            },
            7
        ));
    }
}
