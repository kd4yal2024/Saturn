//! Phase 2 RX-only Saturn/XDMA DDC capture.
//!
//! This module configures one local Saturn DDC, reads the raw C2H stream, and
//! validates its hardware rate headers and packed 24-bit I/Q framing.  It does
//! not expose an operational client backend and contains no TX DMA path.

use crate::xdma::{
    alex_receive_state_word, alex_rx_filter_word, ensure_p2app_inactive, SaturnIdentity, XdmaError,
    XdmaRegisterDevice, ALEX_RX_FILTER_REGISTER, ALEX_TX_FILTER_RX_ANTENNA_REGISTER,
};
use crate::xdma_telemetry::{record_probe_outcome, TelemetryValue};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_DDC_DEVICE: &str = "/dev/xdma0_c2h_0";
const DEFAULT_FREQUENCY_HZ: u32 = 14_200_000;
const DEFAULT_CAPTURE_DURATION_MS: u64 = 2_000;
const MIN_CAPTURE_DURATION_MS: u64 = 250;
const MAX_CAPTURE_DURATION_MS: u64 = 10_000;
const ADC_SAMPLE_CLOCK_HZ: u128 = 122_880_000;

pub(crate) const DIRECT_DDC_INDEX: usize = 6;
pub(crate) const DIRECT_DDC_SAMPLE_RATE_KHZ: u32 = 384;
const DIRECT_DDC_RATE_CODE: u32 = 4;
const DDC_RATE_REGISTER: u64 = 0x100C;
const DDC_INPUT_SELECT_REGISTER: u64 = 0x1010;
const DDC6_FREQUENCY_REGISTER: u64 = 0x0018;
const FIFO_RESET_REGISTER: u64 = 0x7000;
const DDC_FIFO_MONITOR_REGISTER: u64 = 0x9000;
const ADC_ATTENUATION_REGISTER: u64 = 0x2018;
const ADC1_RX_ATTENUATION_MASK: u32 = 0x1f;
const ADC_OVERFLOW_REGISTER: u64 = 0x5000;
const ADC1_PEAK_REGISTER: u64 = 0x5004;
const ADC2_PEAK_REGISTER: u64 = 0x5008;
const DDC_FIFO_RESET_BIT: u32 = 1 << 2;
const DDC_STREAM_ENABLE_BIT: u32 = 1 << 30;
const DDC6_ADC_MASK: u32 = 0x3 << (DIRECT_DDC_INDEX * 2);

const DMA_ALIGNMENT: usize = 4096;
const DMA_MIN_READ_BYTES: usize = 4096;
const DMA_MAX_READ_BYTES: usize = 32768;
const FIFO_WORD_BYTES: usize = 8;
const FIFO_POLL_INTERVAL: Duration = Duration::from_micros(250);
const STARTUP_FIFO_DRAIN_MAX_READS: usize = 16;
pub(crate) const RUNTIME_FIFO_DRAIN_MAX_READS: usize = 16;

const RATE_CODES_TO_SAMPLE_WORDS: [usize; 8] = [0, 1, 2, 4, 8, 16, 32, 0];

#[derive(Clone, Copy, Debug)]
struct RxProbeConfig {
    frequency_hz: u32,
    duration: Duration,
}

impl RxProbeConfig {
    fn from_env() -> Result<Self, XdmaError> {
        let frequency_hz =
            parse_env_u32("SATURN_BRIDGE_XDMA_RX_FREQUENCY_HZ", DEFAULT_FREQUENCY_HZ)?;
        if frequency_hz > (ADC_SAMPLE_CLOCK_HZ as u32 / 2) {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA RX frequency {frequency_hz} Hz exceeds the 61.44 MHz Nyquist limit"
            )));
        }

        let duration_ms = parse_env_u64(
            "SATURN_BRIDGE_XDMA_RX_DURATION_MS",
            DEFAULT_CAPTURE_DURATION_MS,
        )?;
        if !(MIN_CAPTURE_DURATION_MS..=MAX_CAPTURE_DURATION_MS).contains(&duration_ms) {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA RX duration {duration_ms} ms is outside the supported {MIN_CAPTURE_DURATION_MS}..={MAX_CAPTURE_DURATION_MS} ms range"
            )));
        }

        Ok(Self {
            frequency_hz,
            duration: Duration::from_millis(duration_ms),
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct FifoSnapshot {
    depth_words: usize,
    overflow: bool,
    over_threshold: bool,
    underflow: bool,
}

impl FifoSnapshot {
    fn decode(value: u32) -> Self {
        Self {
            depth_words: (value & 0xffff) as usize,
            overflow: (value & (1 << 31)) != 0,
            over_threshold: (value & (1 << 30)) != 0,
            underflow: (value & (1 << 29)) != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FifoStatusPolicy {
    Legacy,
    V29Plus,
}

impl FifoStatusPolicy {
    fn for_identity(identity: &SaturnIdentity) -> Self {
        if identity.firmware_major == 1 && identity.firmware_minor >= 29 {
            Self::V29Plus
        } else {
            Self::Legacy
        }
    }
}

fn fifo_has_recoverable_high_water(snapshot: FifoSnapshot, policy: FifoStatusPolicy) -> bool {
    snapshot.over_threshold || (policy == FifoStatusPolicy::V29Plus && snapshot.overflow)
}

fn fifo_requires_drain(snapshot: FifoSnapshot, policy: FifoStatusPolicy) -> bool {
    snapshot.depth_words != 0 && fifo_has_recoverable_high_water(snapshot, policy)
}

fn startup_high_water_separately_accounted(
    snapshot: FifoSnapshot,
    allowed: bool,
    policy: FifoStatusPolicy,
) -> bool {
    allowed
        && fifo_has_recoverable_high_water(snapshot, policy)
        && !fifo_has_hard_fault(snapshot, policy)
}

fn fifo_has_hard_fault(snapshot: FifoSnapshot, policy: FifoStatusPolicy) -> bool {
    match policy {
        FifoStatusPolicy::Legacy => snapshot.overflow || snapshot.underflow,
        FifoStatusPolicy::V29Plus => snapshot.underflow && snapshot.depth_words != 0,
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RxCaptureStats {
    pub(crate) dma_reads: u64,
    pub(crate) dma_bytes: u64,
    pub(crate) frames: u64,
    pub(crate) samples: u64,
    pub(crate) header_resyncs: u64,
    pub(crate) header_errors: u64,
    pub(crate) fifo_depth_hwm: usize,
    /// V29+ bit-31/almost-full observations recovered by draining C2H.
    pub(crate) fifo_almost_full: u64,
    /// V29+ bit-29 observations whose coherent current depth was zero.
    pub(crate) fifo_empty_observations: u64,
    pub(crate) fifo_overflows: u64,
    pub(crate) fifo_over_threshold: u64,
    pub(crate) fifo_underflows: u64,
    pub(crate) fifo_startup_underflow: bool,
    pub(crate) fifo_startup_over_threshold: u64,
    power_sum: f64,
    peak: f32,
}

impl RxCaptureStats {
    fn observe_fifo(&mut self, snapshot: FifoSnapshot, policy: FifoStatusPolicy) {
        self.fifo_depth_hwm = self.fifo_depth_hwm.max(snapshot.depth_words);
        if policy == FifoStatusPolicy::V29Plus {
            self.fifo_almost_full += u64::from(snapshot.overflow);
            self.fifo_empty_observations +=
                u64::from(snapshot.underflow && snapshot.depth_words == 0);
        } else {
            self.fifo_overflows += u64::from(snapshot.overflow);
        }
        self.fifo_over_threshold += u64::from(snapshot.over_threshold);
        self.fifo_underflows += u64::from(
            snapshot.underflow
                && !(policy == FifoStatusPolicy::V29Plus && snapshot.depth_words == 0),
        );
    }

    fn observe_startup_fifo(&mut self, snapshot: FifoSnapshot, policy: FifoStatusPolicy) {
        self.fifo_depth_hwm = self.fifo_depth_hwm.max(snapshot.depth_words);
        self.fifo_startup_over_threshold += u64::from(snapshot.over_threshold);
        if policy == FifoStatusPolicy::V29Plus {
            self.fifo_almost_full += u64::from(snapshot.overflow);
            self.fifo_empty_observations +=
                u64::from(snapshot.underflow && snapshot.depth_words == 0);
        } else {
            self.fifo_overflows += u64::from(snapshot.overflow);
        }
        self.fifo_startup_underflow |= snapshot.underflow
            && !(policy == FifoStatusPolicy::V29Plus && snapshot.depth_words == 0);
    }

    fn rms_dbfs(&self) -> f32 {
        if self.samples == 0 {
            return -200.0;
        }
        let mean_power = (self.power_sum / self.samples as f64).max(1.0e-20);
        (10.0 * mean_power.log10()) as f32
    }
}

struct DdcStreamParser {
    pending: Vec<u8>,
    expected_rate_word: u32,
    synchronized: bool,
    stats: RxCaptureStats,
}

impl DdcStreamParser {
    fn new(expected_rate_word: u32) -> Self {
        Self {
            pending: Vec::with_capacity(DMA_MAX_READ_BYTES * 2),
            expected_rate_word,
            synchronized: false,
            stats: RxCaptureStats::default(),
        }
    }

    fn feed(&mut self, bytes: &[u8], iq_samples: &mut Vec<f32>) -> Result<(), XdmaError> {
        if !bytes.len().is_multiple_of(FIFO_WORD_BYTES) {
            return Err(XdmaError::Incompatible(format!(
                "DDC DMA read length {} is not a multiple of {FIFO_WORD_BYTES}",
                bytes.len()
            )));
        }
        self.pending.extend_from_slice(bytes);

        loop {
            if !self.synchronized {
                let Some(offset) = find_rate_header(&self.pending, self.expected_rate_word) else {
                    let retain = self.pending.len().min(FIFO_WORD_BYTES);
                    if self.pending.len() > retain {
                        let discard = self.pending.len() - retain;
                        self.pending.drain(..discard);
                        self.stats.header_resyncs += 1;
                    }
                    return Ok(());
                };
                if offset != 0 {
                    self.pending.drain(..offset);
                    self.stats.header_resyncs += 1;
                }
                self.synchronized = true;
            }

            if self.pending.len() < FIFO_WORD_BYTES {
                return Ok(());
            }
            let rate_word = u32::from_le_bytes(self.pending[0..4].try_into().unwrap());
            if self.pending[7] != 0x80 || rate_word != self.expected_rate_word {
                self.stats.header_errors += 1;
                return Err(XdmaError::Incompatible(format!(
                    "DDC stream framing error after synchronization: header=0x{:02x} rate=0x{rate_word:08x} expected_rate=0x{:08x}",
                    self.pending[7], self.expected_rate_word
                )));
            }

            let counts = analyse_rate_word(rate_word)?;
            let frame_words: usize = counts.iter().sum();
            if counts[DIRECT_DDC_INDEX] != direct_ddc_sample_words()
                || counts
                    .iter()
                    .enumerate()
                    .any(|(index, count)| index != DIRECT_DDC_INDEX && *count != 0)
            {
                return Err(XdmaError::Incompatible(format!(
                    "unexpected direct DDC rate layout 0x{rate_word:08x}: {counts:?}"
                )));
            }
            let frame_bytes = (frame_words + 1) * FIFO_WORD_BYTES;
            if self.pending.len() < frame_bytes {
                return Ok(());
            }

            for sample_word in
                self.pending[FIFO_WORD_BYTES..frame_bytes].chunks_exact(FIFO_WORD_BYTES)
            {
                let i = signed_24_be(&sample_word[0..3]) as f32 / 8_388_608.0;
                let q = signed_24_be(&sample_word[3..6]) as f32 / 8_388_608.0;
                iq_samples.push(i);
                iq_samples.push(q);
                self.stats.power_sum += ((i * i + q * q) * 0.5) as f64;
                self.stats.peak = self.stats.peak.max(i.abs()).max(q.abs());
                self.stats.samples += 1;
            }
            self.stats.frames += 1;
            self.pending.drain(..frame_bytes);
        }
    }
}

pub(crate) struct AlignedBuffer {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
    locked: bool,
}

// The allocation is exclusively owned, contains no borrowed data, and all
// access requires a Rust borrow of the AlignedBuffer. Moving ownership to the
// dedicated TX worker is therefore equivalent to moving a Vec<u8>.
unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    pub(crate) fn new(len: usize) -> Result<Self, XdmaError> {
        let layout = Layout::from_size_align(len, DMA_ALIGNMENT).map_err(|error| {
            XdmaError::Incompatible(format!("invalid aligned DMA buffer layout: {error}"))
        })?;
        // SAFETY: layout has a non-zero size and a power-of-two alignment.
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).ok_or_else(|| XdmaError::Io {
            action: "could not allocate aligned XDMA receive buffer",
            source: io::Error::from(io::ErrorKind::OutOfMemory),
        })?;
        Ok(Self {
            ptr,
            len,
            layout,
            locked: false,
        })
    }

    pub(crate) fn lock_memory(&mut self) -> Result<(), XdmaError> {
        // SAFETY: ptr owns `self.len` readable and writable bytes. mlock does
        // not change Rust aliasing and remains in effect until munlock/drop.
        let result = unsafe { libc::mlock(self.ptr.as_ptr().cast(), self.len) };
        if result != 0 {
            return Err(XdmaError::Io {
                action: "could not lock aligned XDMA buffer in memory",
                source: io::Error::last_os_error(),
            });
        }
        self.locked = true;
        Ok(())
    }

    pub(crate) fn as_mut_slice(&mut self, len: usize) -> &mut [u8] {
        assert!(len <= self.len);
        // SAFETY: ptr owns `self.len` initialized bytes for this object's
        // lifetime, and the mutable borrow prevents aliasing.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), len) }
    }

    pub(crate) fn as_slice(&self, len: usize) -> &[u8] {
        assert!(len <= self.len);
        // SAFETY: ptr owns `self.len` initialized bytes for this object's
        // lifetime, and this immutable borrow does not permit mutation.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        if self.locked {
            // SAFETY: the same live allocation was successfully locked and is
            // not freed until after this call.
            unsafe {
                libc::munlock(self.ptr.as_ptr().cast(), self.len);
            }
        }
        // SAFETY: ptr was allocated with this exact layout and is freed once.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

struct RxDdcSession<'a> {
    registers: &'a mut XdmaRegisterDevice,
    dma: File,
    fifo_policy: FifoStatusPolicy,
    stopped: bool,
}

impl<'a> RxDdcSession<'a> {
    fn start(
        registers: &'a mut XdmaRegisterDevice,
        dma_path: &Path,
        frequency_hz: u32,
        fifo_policy: FifoStatusPolicy,
    ) -> Result<Self, XdmaError> {
        let dma = OpenOptions::new()
            .read(true)
            .open(dma_path)
            .map_err(|source| XdmaError::Io {
                action: "could not open XDMA DDC receive device",
                source,
            })?;
        let mut session = Self {
            registers,
            dma,
            fifo_policy,
            stopped: false,
        };
        if let Err(error) = session.configure(frequency_hz) {
            let _ = session.stop();
            return Err(error);
        }
        Ok(session)
    }

    fn configure(&mut self, frequency_hz: u32) -> Result<(), XdmaError> {
        self.disable_stream()?;
        thread::sleep(Duration::from_millis(1));
        self.reset_fifo()?;
        // Reading the monitor clears its sticky condition flags.  Do this
        // while the stream is disabled so telemetry covers this capture only,
        // rather than an underflow left behind by the previous owner.
        self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?;
        self.registers
            .write_register(DDC_RATE_REGISTER, direct_ddc_rate_word())?;
        self.registers.write_register(
            DDC6_FREQUENCY_REGISTER,
            frequency_to_phase_word(frequency_hz),
        )?;
        self.registers.update_register(
            DDC_INPUT_SELECT_REGISTER,
            |value| (value & !DDC6_ADC_MASK) | DDC_STREAM_ENABLE_BIT,
            "could not route ADC1 and enable direct DDC stream",
        )?;
        Ok(())
    }

    fn capture(&mut self, duration: Duration) -> Result<RxCaptureStats, XdmaError> {
        let mut aligned = AlignedBuffer::new(DMA_MAX_READ_BYTES)?;
        let mut parser = DdcStreamParser::new(direct_ddc_rate_word());
        let mut discarded_iq = Vec::with_capacity(DMA_MAX_READ_BYTES / FIFO_WORD_BYTES * 2);
        // The FPGA can latch one benign underflow while a newly enabled,
        // empty read FIFO starts filling.  P2 likewise excludes startup FIFO
        // conditions from runtime telemetry.  Clear that boundary here so
        // the counters below describe DMA activity during this capture.
        thread::sleep(Duration::from_millis(1));
        let startup_fifo =
            FifoSnapshot::decode(self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?);
        parser
            .stats
            .observe_startup_fifo(startup_fifo, self.fifo_policy);
        let started = Instant::now();
        let deadline = started + duration;

        while Instant::now() < deadline {
            let fifo =
                FifoSnapshot::decode(self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?);
            parser.stats.observe_fifo(fifo, self.fifo_policy);
            let read_bytes = dma_read_size(fifo.depth_words);
            if read_bytes == 0 {
                thread::sleep(FIFO_POLL_INTERVAL);
                continue;
            }

            let target = aligned.as_mut_slice(read_bytes);
            let read = self
                .dma
                .read_at(target, 0)
                .map_err(|source| XdmaError::Io {
                    action: "could not read XDMA DDC receive stream",
                    source,
                })?;
            if read != read_bytes {
                return Err(XdmaError::Io {
                    action: "XDMA DDC receive stream returned a short read",
                    source: io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("read {read} of {read_bytes} bytes"),
                    ),
                });
            }
            parser.stats.dma_reads += 1;
            parser.stats.dma_bytes += read as u64;
            discarded_iq.clear();
            parser.feed(&target[..read], &mut discarded_iq)?;
        }

        if parser.stats.frames == 0 || parser.stats.samples == 0 {
            return Err(XdmaError::Incompatible(
                "direct XDMA RX capture completed without any valid DDC frames".into(),
            ));
        }
        Ok(parser.stats)
    }

    fn disable_stream(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            DDC_INPUT_SELECT_REGISTER,
            |value| value & !DDC_STREAM_ENABLE_BIT,
            "could not disable direct DDC stream",
        )
    }

    fn reset_fifo(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value & !DDC_FIFO_RESET_BIT,
            "could not assert direct DDC FIFO reset",
        )?;
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value | DDC_FIFO_RESET_BIT,
            "could not release direct DDC FIFO reset",
        )
    }

    fn stop(&mut self) -> Result<(), XdmaError> {
        if self.stopped {
            return Ok(());
        }
        let disable = self.disable_stream();
        thread::sleep(Duration::from_millis(1));
        let clear_rates = self.registers.write_register(DDC_RATE_REGISTER, 0);
        let reset = self.reset_fifo();
        let result = disable.and(clear_rates).and(reset);
        self.stopped = result.is_ok();
        result
    }
}

impl Drop for RxDdcSession<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("saturn-bridge: XDMA RX emergency cleanup failed: {error}");
        }
    }
}

/// Owned, receive-only XDMA DDC session used by the operational backend.
///
/// The register device remains armed for emergency receive-safe cleanup for
/// the session's entire lifetime. This type exposes no H2C device and cannot
/// key RF.
pub(crate) struct OperationalRxSession {
    registers: XdmaRegisterDevice,
    dma: File,
    parser: DdcStreamParser,
    aligned: AlignedBuffer,
    identity: SaturnIdentity,
    fifo_policy: FifoStatusPolicy,
    frequency_hz: u32,
    rx_antenna: u8,
    rx_attenuation_db: u8,
    stopped: bool,
}

pub(crate) struct OperationalRxRead {
    pub(crate) samples_ready: bool,
    pub(crate) requires_drain: bool,
}

impl OperationalRxSession {
    pub(crate) fn open(
        frequency_hz: u32,
        rx_antenna: u8,
        rx_attenuation_db: u8,
    ) -> Result<Self, XdmaError> {
        ensure_p2app_inactive()?;
        validate_frequency(frequency_hz)?;
        let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
        let ddc_path = env::var_os("SATURN_BRIDGE_XDMA_RX_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DDC_DEVICE));
        let registers = XdmaRegisterDevice::open(&register_path)?;
        let identity = registers.identity().clone();
        let fifo_policy = FifoStatusPolicy::for_identity(&identity);
        let dma = OpenOptions::new()
            .read(true)
            .open(&ddc_path)
            .map_err(|source| XdmaError::Io {
                action: "could not open operational XDMA DDC receive device",
                source,
            })?;
        let mut aligned = AlignedBuffer::new(DMA_MAX_READ_BYTES)?;
        aligned.lock_memory()?;
        let mut session = Self {
            registers,
            dma,
            parser: DdcStreamParser::new(direct_ddc_rate_word()),
            aligned,
            identity,
            fifo_policy,
            frequency_hz,
            rx_antenna: rx_antenna.clamp(1, 3),
            rx_attenuation_db: rx_attenuation_db.min(31),
            stopped: false,
        };
        if let Err(error) = session.configure(frequency_hz) {
            let _ = session.stop();
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn identity(&self) -> &SaturnIdentity {
        &self.identity
    }

    pub(crate) fn frequency_hz(&self) -> u32 {
        self.frequency_hz
    }

    pub(crate) fn stats(&self) -> &RxCaptureStats {
        &self.parser.stats
    }

    pub(crate) fn verify_receive_safe(&self) -> Result<(), XdmaError> {
        self.registers.verify_safe_receive_state()
    }

    pub(crate) fn tune(&mut self, frequency_hz: u32) -> Result<(), XdmaError> {
        validate_frequency(frequency_hz)?;
        self.registers.write_register(
            ALEX_TX_FILTER_RX_ANTENNA_REGISTER,
            alex_receive_state_word(frequency_hz, self.rx_antenna),
        )?;
        self.registers
            .write_register(ALEX_RX_FILTER_REGISTER, alex_rx_filter_word(frequency_hz))?;
        self.registers.write_register(
            DDC6_FREQUENCY_REGISTER,
            frequency_to_phase_word(frequency_hz),
        )?;
        self.frequency_hz = frequency_hz;
        Ok(())
    }

    pub(crate) fn set_rx_antenna(&mut self, antenna: u8) -> Result<(), XdmaError> {
        let antenna = antenna.clamp(1, 3);
        self.registers.write_register(
            ALEX_TX_FILTER_RX_ANTENNA_REGISTER,
            alex_receive_state_word(self.frequency_hz, antenna),
        )?;
        self.rx_antenna = antenna;
        Ok(())
    }

    pub(crate) fn set_rx_attenuation(&mut self, attenuation_db: u8) -> Result<(), XdmaError> {
        let attenuation_db = attenuation_db.min(31);
        self.registers.update_register(
            ADC_ATTENUATION_REGISTER,
            |value| adc1_rx_attenuation_word(value, attenuation_db),
            "could not set ADC1 receive attenuation",
        )?;
        self.rx_attenuation_db = attenuation_db;
        Ok(())
    }

    pub(crate) fn read_adc_telemetry(&self) -> Result<(u8, u16, u16), XdmaError> {
        let overflow = (self.registers.read_register(ADC_OVERFLOW_REGISTER)? & 0x03) as u8;
        let adc1_peak = self
            .registers
            .read_register(ADC1_PEAK_REGISTER)?
            .min(u32::from(u16::MAX)) as u16;
        let adc2_peak = self
            .registers
            .read_register(ADC2_PEAK_REGISTER)?
            .min(u32::from(u16::MAX)) as u16;
        Ok((overflow, adc1_peak, adc2_peak))
    }

    pub(crate) fn read_iq(
        &mut self,
        iq_samples: &mut Vec<f32>,
    ) -> Result<OperationalRxRead, XdmaError> {
        iq_samples.clear();
        let (samples_ready, requires_drain) = self.read_iq_once(iq_samples, false)?;
        Ok(OperationalRxRead {
            samples_ready,
            requires_drain,
        })
    }

    /// Drain DDC data accumulated during bounded startup work such as atomic
    /// readiness-file updates. Threshold, plus bit 31 on V29 and later, is a
    /// recoverable high-water condition rather than proof of data loss. A V29+
    /// bit-29 observation at a coherent zero depth is likewise an empty FIFO,
    /// not a hard underflow. Legacy overflow/underflow and non-empty V29+
    /// underflow observations remain fatal at startup and runtime.
    pub(crate) fn drain_startup_fifo(
        &mut self,
        iq_samples: &mut Vec<f32>,
    ) -> Result<(), XdmaError> {
        for _ in 0..STARTUP_FIFO_DRAIN_MAX_READS {
            iq_samples.clear();
            let (_, requires_drain) = self.read_iq_once(iq_samples, true)?;
            if !requires_drain {
                return Ok(());
            }
        }
        Err(XdmaError::Incompatible(format!(
            "operational XDMA RX FIFO remained at high water after {STARTUP_FIFO_DRAIN_MAX_READS} bounded startup drains"
        )))
    }

    fn read_iq_once(
        &mut self,
        iq_samples: &mut Vec<f32>,
        allow_startup_high_water: bool,
    ) -> Result<(bool, bool), XdmaError> {
        let fifo = FifoSnapshot::decode(self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?);
        let recover_startup_high_water = startup_high_water_separately_accounted(
            fifo,
            allow_startup_high_water,
            self.fifo_policy,
        );
        if recover_startup_high_water {
            self.parser
                .stats
                .observe_startup_fifo(fifo, self.fifo_policy);
        } else {
            self.parser.stats.observe_fifo(fifo, self.fifo_policy);
        }
        if fifo_has_hard_fault(fifo, self.fifo_policy) {
            return Err(XdmaError::Incompatible(format!(
                "operational XDMA RX FIFO fault: depth={} overflow={} threshold={} underflow={}",
                fifo.depth_words,
                u8::from(fifo.overflow),
                u8::from(fifo.over_threshold),
                u8::from(fifo.underflow)
            )));
        }
        let read_bytes = dma_read_size(fifo.depth_words);
        if read_bytes == 0 {
            return Ok((false, fifo_requires_drain(fifo, self.fifo_policy)));
        }
        let target = self.aligned.as_mut_slice(read_bytes);
        let read = self
            .dma
            .read_at(target, 0)
            .map_err(|source| XdmaError::Io {
                action: "could not read operational XDMA DDC receive stream",
                source,
            })?;
        if read != read_bytes {
            return Err(XdmaError::Io {
                action: "operational XDMA DDC receive stream returned a short read",
                source: io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("read {read} of {read_bytes} bytes"),
                ),
            });
        }
        self.parser.stats.dma_reads += 1;
        self.parser.stats.dma_bytes += read as u64;
        let samples_before = iq_samples.len();
        self.parser.feed(&target[..read], iq_samples)?;
        Ok((
            iq_samples.len() != samples_before,
            fifo_requires_drain(fifo, self.fifo_policy),
        ))
    }

    fn configure(&mut self, frequency_hz: u32) -> Result<(), XdmaError> {
        self.disable_stream()?;
        thread::sleep(Duration::from_millis(1));
        self.reset_fifo()?;
        self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?;
        self.registers
            .write_register(DDC_RATE_REGISTER, direct_ddc_rate_word())?;
        self.tune(frequency_hz)?;
        // piHPSDR writes the ADC step attenuator on every Protocol 2
        // high-priority update. Direct XDMA must likewise establish the
        // model's receive attenuation while claiming hardware ownership;
        // otherwise a value left by P2 can survive while the model reports
        // ATT Off and the S-meter compensation assumes zero attenuation.
        self.set_rx_attenuation(self.rx_attenuation_db)?;
        self.registers.update_register(
            DDC_INPUT_SELECT_REGISTER,
            |value| (value & !DDC6_ADC_MASK) | DDC_STREAM_ENABLE_BIT,
            "could not route ADC1 and enable operational direct DDC stream",
        )?;
        thread::sleep(Duration::from_millis(1));
        let startup =
            FifoSnapshot::decode(self.registers.read_register(DDC_FIFO_MONITOR_REGISTER)?);
        self.parser
            .stats
            .observe_startup_fifo(startup, self.fifo_policy);
        Ok(())
    }

    fn disable_stream(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            DDC_INPUT_SELECT_REGISTER,
            |value| value & !DDC_STREAM_ENABLE_BIT,
            "could not disable operational direct DDC stream",
        )
    }

    fn reset_fifo(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value & !DDC_FIFO_RESET_BIT,
            "could not assert operational direct DDC FIFO reset",
        )?;
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value | DDC_FIFO_RESET_BIT,
            "could not release operational direct DDC FIFO reset",
        )
    }

    pub(crate) fn stop(&mut self) -> Result<(), XdmaError> {
        if self.stopped {
            return Ok(());
        }
        let disable = self.disable_stream();
        thread::sleep(Duration::from_millis(1));
        let clear_rates = self.registers.write_register(DDC_RATE_REGISTER, 0);
        let reset = self.reset_fifo();
        let safe = self.registers.force_safe_receive_state();
        let result = disable.and(clear_rates).and(reset).and(safe);
        self.stopped = result.is_ok();
        result
    }
}

fn adc1_rx_attenuation_word(value: u32, attenuation_db: u8) -> u32 {
    (value & !ADC1_RX_ATTENUATION_MASK) | u32::from(attenuation_db.min(31))
}

impl Drop for OperationalRxSession {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("saturn-bridge: operational XDMA RX cleanup failed: {error}");
        }
    }
}

pub fn run_phase2_rx_probe() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let config = RxProbeConfig::from_env()?;
    let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
    let ddc_path = env::var_os("SATURN_BRIDGE_XDMA_RX_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DDC_DEVICE));

    let mut registers = XdmaRegisterDevice::open(&register_path)?;
    let identity: SaturnIdentity = registers.identity().clone();
    let fifo_policy = FifoStatusPolicy::for_identity(&identity);
    let started = Instant::now();
    let mut session =
        RxDdcSession::start(&mut registers, &ddc_path, config.frequency_hz, fifo_policy)?;
    let capture = session.capture(config.duration);
    let stop = session.stop();
    drop(session);
    let stats = capture?;
    stop?;
    registers.close_safely()?;
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let sample_rate = stats.samples as f64 / elapsed;

    record_probe_outcome(
        2,
        "rx-ddc",
        "passed",
        "receive-safe-verified",
        None,
        &[
            (
                "device",
                TelemetryValue::text(ddc_path.display().to_string()),
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
            ("ddc", TelemetryValue::number(DIRECT_DDC_INDEX)),
            ("adc", TelemetryValue::text("ADC1")),
            ("frequency_hz", TelemetryValue::number(config.frequency_hz)),
            (
                "sample_rate_khz",
                TelemetryValue::number(DIRECT_DDC_SAMPLE_RATE_KHZ),
            ),
            (
                "duration_ms",
                TelemetryValue::number(config.duration.as_millis()),
            ),
            ("frames", TelemetryValue::number(stats.frames)),
            ("samples", TelemetryValue::number(stats.samples)),
            ("observed_sample_rate", TelemetryValue::number(sample_rate)),
            ("dma_reads", TelemetryValue::number(stats.dma_reads)),
            ("dma_bytes", TelemetryValue::number(stats.dma_bytes)),
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
            (
                "header_resync",
                TelemetryValue::number(stats.header_resyncs),
            ),
            ("header_errors", TelemetryValue::number(stats.header_errors)),
            ("rms_dbfs", TelemetryValue::number(stats.rms_dbfs())),
            ("peak", TelemetryValue::number(stats.peak)),
            ("rf_keyed", TelemetryValue::boolean(false)),
        ],
    );

    println!(
        "saturn-bridge: XDMA Phase 2 RX probe passed device={} product={} pcb={} firmware={}.{} ddc={} adc=ADC1 frequency={}Hz rate={}kHz duration_ms={} frames={} frame_seq=0..{} samples={} sample_rate={:.1}/s dma_reads={} dma_bytes={} fifo_hwm={} fifo_almost_full={} fifo_empty_observations={} fifo_overflow={} fifo_threshold={} fifo_startup_underflow={} fifo_underflow={} header_resync={} header_errors={} rms={:.1}dBFS peak={:.4}",
        ddc_path.display(),
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        DIRECT_DDC_INDEX,
        config.frequency_hz,
        DIRECT_DDC_SAMPLE_RATE_KHZ,
        config.duration.as_millis(),
        stats.frames,
        stats.frames.saturating_sub(1),
        stats.samples,
        sample_rate,
        stats.dma_reads,
        stats.dma_bytes,
        stats.fifo_depth_hwm,
        stats.fifo_almost_full,
        stats.fifo_empty_observations,
        stats.fifo_overflows,
        stats.fifo_over_threshold,
        u8::from(stats.fifo_startup_underflow),
        stats.fifo_underflows,
        stats.header_resyncs,
        stats.header_errors,
        stats.rms_dbfs(),
        stats.peak,
    );
    println!(
        "saturn-bridge: XDMA Phase 2 RX cleanup completed; DDC stream disabled, rate word cleared, FIFO reset, and RF remains receive-safe"
    );
    Ok(())
}

fn direct_ddc_rate_word() -> u32 {
    DIRECT_DDC_RATE_CODE << (DIRECT_DDC_INDEX * 3)
}

fn direct_ddc_sample_words() -> usize {
    RATE_CODES_TO_SAMPLE_WORDS[DIRECT_DDC_RATE_CODE as usize]
}

fn validate_frequency(frequency_hz: u32) -> Result<(), XdmaError> {
    if frequency_hz > (ADC_SAMPLE_CLOCK_HZ as u32 / 2) {
        return Err(XdmaError::Incompatible(format!(
            "direct XDMA RX frequency {frequency_hz} Hz exceeds the 61.44 MHz Nyquist limit"
        )));
    }
    Ok(())
}

fn frequency_to_phase_word(frequency_hz: u32) -> u32 {
    let numerator = u128::from(frequency_hz) * (1u128 << 32);
    ((numerator + (ADC_SAMPLE_CLOCK_HZ / 2)) / ADC_SAMPLE_CLOCK_HZ) as u32
}

fn dma_read_size(depth_words: usize) -> usize {
    let available_bytes = depth_words.saturating_mul(FIFO_WORD_BYTES);
    if available_bytes < DMA_MIN_READ_BYTES {
        0
    } else if available_bytes >= DMA_MAX_READ_BYTES {
        DMA_MAX_READ_BYTES
    } else if available_bytes >= 16384 {
        16384
    } else if available_bytes >= 8192 {
        8192
    } else {
        DMA_MIN_READ_BYTES
    }
}

fn analyse_rate_word(mut rate_word: u32) -> Result<[usize; 10], XdmaError> {
    let mut counts = [0usize; 10];
    let mut ddc = 0usize;
    while ddc < counts.len() {
        let rate = (rate_word & 0x7) as usize;
        if rate == 7 {
            if ddc + 1 >= counts.len() {
                return Err(XdmaError::Incompatible(
                    "DDC9 cannot be an interleave marker".into(),
                ));
            }
            rate_word >>= 3;
            let paired_rate = (rate_word & 0x7) as usize;
            counts[ddc] = RATE_CODES_TO_SAMPLE_WORDS[paired_rate] * 2;
            counts[ddc + 1] = 0;
            ddc += 2;
        } else {
            counts[ddc] = RATE_CODES_TO_SAMPLE_WORDS[rate];
            ddc += 1;
        }
        rate_word >>= 3;
    }
    Ok(counts)
}

fn find_rate_header(bytes: &[u8], expected_rate_word: u32) -> Option<usize> {
    (0..bytes.len().saturating_sub(7))
        .step_by(FIFO_WORD_BYTES)
        .find(|offset| {
            bytes[*offset + 7] == 0x80
                && u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap())
                    == expected_rate_word
        })
}

fn signed_24_be(bytes: &[u8]) -> i32 {
    let mut value = ((bytes[0] as i32) << 16) | ((bytes[1] as i32) << 8) | bytes[2] as i32;
    if value & 0x0080_0000 != 0 {
        value |= !0x00ff_ffff;
    }
    value
}

fn parse_env_u32(name: &str, default: u32) -> Result<u32, XdmaError> {
    match env::var(name) {
        Ok(value) => value.parse::<u32>().map_err(|_| {
            XdmaError::Incompatible(format!("{name} must be an unsigned 32-bit integer"))
        }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(XdmaError::Incompatible(format!(
            "could not read {name}: {error}"
        ))),
    }
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64, XdmaError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_i24_be(dest: &mut [u8], value: i32) {
        let value = value.clamp(-(1 << 23), (1 << 23) - 1);
        dest[0] = ((value >> 16) & 0xff) as u8;
        dest[1] = ((value >> 8) & 0xff) as u8;
        dest[2] = (value & 0xff) as u8;
    }

    fn test_frame() -> Vec<u8> {
        let mut frame = vec![0u8; (direct_ddc_sample_words() + 1) * FIFO_WORD_BYTES];
        frame[0..4].copy_from_slice(&direct_ddc_rate_word().to_le_bytes());
        frame[4..8].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        for (index, word) in frame[8..].chunks_exact_mut(8).enumerate() {
            write_i24_be(&mut word[0..3], 1000 + index as i32);
            write_i24_be(&mut word[3..6], -2000 - index as i32);
        }
        frame
    }

    #[test]
    fn direct_rate_word_selects_only_ddc6_at_384khz() {
        let counts = analyse_rate_word(direct_ddc_rate_word()).unwrap();
        assert_eq!(counts[DIRECT_DDC_INDEX], 8);
        assert_eq!(counts.iter().sum::<usize>(), 8);
    }

    #[test]
    fn phase_word_uses_saturn_adc_clock() {
        assert_eq!(frequency_to_phase_word(0), 0);
        assert_eq!(frequency_to_phase_word(30_720_000), 0x4000_0000);
        assert_eq!(frequency_to_phase_word(61_440_000), 0x8000_0000);
    }

    #[test]
    fn receive_attenuation_clamps_and_preserves_other_adc_fields() {
        let original = 0xa5a5_ffe0;
        assert_eq!(adc1_rx_attenuation_word(original, 12), 0xa5a5_ffec);
        assert_eq!(adc1_rx_attenuation_word(original, 99), 0xa5a5_ffff);
    }

    #[test]
    fn parser_resynchronizes_and_accepts_split_frames() {
        let frame = test_frame();
        let mut stream = vec![0x55; 16];
        stream.extend_from_slice(&frame);
        stream.extend_from_slice(&frame);
        let mut parser = DdcStreamParser::new(direct_ddc_rate_word());
        let mut iq_samples = Vec::new();
        parser.feed(&stream[..32], &mut iq_samples).unwrap();
        parser.feed(&stream[32..], &mut iq_samples).unwrap();
        assert_eq!(parser.stats.frames, 2);
        assert_eq!(parser.stats.samples, 16);
        assert_eq!(iq_samples.len(), 32);
        assert!((iq_samples[0] - 1000.0 / 8_388_608.0).abs() < f32::EPSILON);
        assert!((iq_samples[1] - (-2000.0 / 8_388_608.0)).abs() < f32::EPSILON);
        assert_eq!(parser.stats.header_errors, 0);
        assert_eq!(parser.stats.header_resyncs, 1);
        assert!(parser.stats.peak > 0.0);
    }

    #[test]
    fn parser_rejects_a_framing_error_after_synchronization() {
        let frame = test_frame();
        let mut parser = DdcStreamParser::new(direct_ddc_rate_word());
        let mut iq_samples = Vec::new();
        parser.feed(&frame, &mut iq_samples).unwrap();

        let mut malformed = frame;
        malformed[7] = 0;
        let error = parser.feed(&malformed, &mut iq_samples).unwrap_err();
        assert!(error.to_string().contains("DDC stream framing error"));
        assert_eq!(parser.stats.header_errors, 1);
    }

    #[test]
    fn malformed_interleave_at_ddc9_is_rejected() {
        assert!(analyse_rate_word(7 << (9 * 3)).is_err());
    }

    #[test]
    fn dma_read_sizes_are_aligned_and_bounded() {
        assert_eq!(dma_read_size(511), 0);
        assert_eq!(dma_read_size(512), 4096);
        assert_eq!(dma_read_size(1024), 8192);
        assert_eq!(dma_read_size(2048), 16384);
        assert_eq!(dma_read_size(4096), 32768);
    }

    #[test]
    fn legacy_fifo_threshold_is_a_watermark_while_fault_bits_are_fatal() {
        let threshold = FifoSnapshot {
            over_threshold: true,
            ..FifoSnapshot::default()
        };
        assert!(!fifo_has_hard_fault(threshold, FifoStatusPolicy::Legacy));
        assert!(startup_high_water_separately_accounted(
            threshold,
            true,
            FifoStatusPolicy::Legacy
        ));
        assert!(!startup_high_water_separately_accounted(
            threshold,
            false,
            FifoStatusPolicy::Legacy
        ));
        let overflow = FifoSnapshot {
            overflow: true,
            ..threshold
        };
        assert!(fifo_has_hard_fault(overflow, FifoStatusPolicy::Legacy));
        assert!(!startup_high_water_separately_accounted(
            overflow,
            true,
            FifoStatusPolicy::Legacy
        ));
        let underflow = FifoSnapshot {
            underflow: true,
            ..threshold
        };
        assert!(fifo_has_hard_fault(underflow, FifoStatusPolicy::Legacy));
        assert!(!startup_high_water_separately_accounted(
            underflow,
            true,
            FifoStatusPolicy::Legacy
        ));
    }

    #[test]
    fn v29_plus_treats_bit31_as_recoverable_high_water() {
        let almost_full = FifoSnapshot {
            depth_words: 16_000,
            overflow: true,
            ..FifoSnapshot::default()
        };
        assert!(!fifo_has_hard_fault(almost_full, FifoStatusPolicy::V29Plus));
        assert!(fifo_requires_drain(almost_full, FifoStatusPolicy::V29Plus));
        assert!(!fifo_requires_drain(
            FifoSnapshot {
                depth_words: 0,
                ..almost_full
            },
            FifoStatusPolicy::V29Plus
        ));
    }

    #[test]
    fn v29_and_v30_select_the_new_fifo_status_policy() {
        let identity = |firmware_minor| SaturnIdentity {
            product_id: 1,
            pcb_version: 2,
            software_id: 1,
            firmware_major: 1,
            firmware_minor,
            clock_mask: 0,
            user_version: 0,
        };
        assert_eq!(
            FifoStatusPolicy::for_identity(&identity(27)),
            FifoStatusPolicy::Legacy
        );
        assert_eq!(
            FifoStatusPolicy::for_identity(&identity(29)),
            FifoStatusPolicy::V29Plus
        );
        assert_eq!(
            FifoStatusPolicy::for_identity(&identity(30)),
            FifoStatusPolicy::V29Plus
        );
    }

    #[test]
    fn v29_plus_zero_depth_bit29_is_an_empty_observation() {
        let empty = FifoSnapshot {
            depth_words: 0,
            underflow: true,
            ..FifoSnapshot::default()
        };
        assert!(!fifo_has_hard_fault(empty, FifoStatusPolicy::V29Plus));
        let nonempty = FifoSnapshot {
            depth_words: 1,
            ..empty
        };
        assert!(fifo_has_hard_fault(nonempty, FifoStatusPolicy::V29Plus));
    }

    #[test]
    fn fifo_stats_separate_v29_conditions_from_hard_faults() {
        let mut stats = RxCaptureStats::default();
        stats.observe_fifo(
            FifoSnapshot {
                depth_words: 0,
                overflow: true,
                underflow: true,
                ..FifoSnapshot::default()
            },
            FifoStatusPolicy::V29Plus,
        );
        assert_eq!(stats.fifo_almost_full, 1);
        assert_eq!(stats.fifo_empty_observations, 1);
        assert_eq!(stats.fifo_overflows, 0);
        assert_eq!(stats.fifo_underflows, 0);
    }

    #[test]
    fn allocated_dma_buffer_has_page_alignment() {
        let buffer = AlignedBuffer::new(DMA_MAX_READ_BYTES).unwrap();
        assert_eq!(buffer.ptr.as_ptr() as usize % DMA_ALIGNMENT, 0);
    }
}
