//! Phase 4 direct Saturn/XDMA DUC performance probe.
//!
//! This one-shot path writes only zero-valued IQ to the TX DUC while every RF
//! control remains forced off. The DUC mux is enabled solely so the FPGA can
//! consume the test stream and its sustained 192 kHz pacing can be measured.

use crate::xdma::{ensure_p2app_inactive, SaturnIdentity, XdmaError, XdmaRegisterDevice};
use crate::xdma_rx::AlignedBuffer;
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_DUC_DEVICE: &str = "/dev/xdma0_h2c_0";
const DEFAULT_PROBE_DURATION_MS: u64 = 3_000;
const MIN_PROBE_DURATION_MS: u64 = 500;
const MAX_PROBE_DURATION_MS: u64 = 10_000;

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
const DUC_PREFILL_FRAMES: usize = 9;
const DUC_REFILL_LOW_FRAMES: usize = 5;
const DUC_MAX_DMA_BATCH_FRAMES: usize = 11;
const DMA_BUFFER_BYTES: usize = DUC_MAX_DMA_BATCH_FRAMES * DUC_FRAME_BYTES;
const FIFO_POLL_INTERVAL: Duration = Duration::from_micros(250);
const ADAPTIVE_HOLD_TIME: Duration = Duration::from_millis(500);
const EXPAND_TO_TEN_STALL: Duration = Duration::from_millis(2);
const EXPAND_TO_ELEVEN_STALL: Duration = Duration::from_millis(3);
const MAX_P9999_REFILL_SERVICE: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug)]
struct DucProbeConfig {
    duration: Duration,
}

impl DucProbeConfig {
    fn from_env() -> Result<Self, XdmaError> {
        let duration_ms = match env::var("SATURN_BRIDGE_XDMA_DUC_DURATION_MS") {
            Ok(value) => value.parse::<u64>().map_err(|_| {
                XdmaError::Incompatible(
                    "SATURN_BRIDGE_XDMA_DUC_DURATION_MS must be an unsigned integer".into(),
                )
            })?,
            Err(env::VarError::NotPresent) => DEFAULT_PROBE_DURATION_MS,
            Err(error) => {
                return Err(XdmaError::Incompatible(format!(
                    "could not read SATURN_BRIDGE_XDMA_DUC_DURATION_MS: {error}"
                )));
            }
        };
        if !(MIN_PROBE_DURATION_MS..=MAX_PROBE_DURATION_MS).contains(&duration_ms) {
            return Err(XdmaError::Incompatible(format!(
                "direct XDMA DUC duration {duration_ms} ms is outside the supported {MIN_PROBE_DURATION_MS}..={MAX_PROBE_DURATION_MS} ms range"
            )));
        }
        Ok(Self {
            duration: Duration::from_millis(duration_ms),
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct FifoSnapshot {
    occupied_words: usize,
    overflow: bool,
    over_threshold: bool,
    underflow: bool,
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

#[derive(Clone, Debug, Default)]
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
    max_write_time_ns: u128,
    write_latencies_ns: Vec<u128>,
    refill_gaps_ns: Vec<u128>,
    refill_service_latencies_ns: Vec<u128>,
    max_loop_gap_ns: u128,
    low_water_events: u64,
    critical_low_events: u64,
    batch_size_changes: u64,
    expansions_to_ten: u64,
    expansions_to_eleven: u64,
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

struct DucDmaSession<'a> {
    registers: &'a mut XdmaRegisterDevice,
    dma: File,
    fifo_depth_words: usize,
    buffer: AlignedBuffer,
    stopped: bool,
}

impl<'a> DucDmaSession<'a> {
    fn start(registers: &'a mut XdmaRegisterDevice, dma_path: &Path) -> Result<Self, XdmaError> {
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
        let buffer = match AlignedBuffer::new(DMA_BUFFER_BYTES) {
            Ok(buffer) => buffer,
            Err(error) => {
                let _ = apply_rf_disabled_duc_shutdown(registers);
                return Err(error);
            }
        };
        let fifo_depth_words = duc_fifo_depth_words(registers.identity().firmware_minor);
        let mut session = Self {
            registers,
            dma,
            fifo_depth_words,
            buffer,
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

    fn run(&mut self, duration: Duration) -> Result<DucStats, XdmaError> {
        let mut stats = DucStats {
            fifo_lwm: usize::MAX,
            write_latencies_ns: Vec::with_capacity(
                (duration.as_secs_f64() * DUC_FRAMES_PER_SECOND as f64 / 2.0) as usize + 32,
            ),
            refill_gaps_ns: Vec::with_capacity(
                (duration.as_secs_f64() * DUC_FRAMES_PER_SECOND as f64 / 2.0) as usize + 32,
            ),
            refill_service_latencies_ns: Vec::with_capacity(
                (duration.as_secs_f64() * DUC_FRAMES_PER_SECOND as f64 / 2.0) as usize + 32,
            ),
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
        self.write_frames(DUC_PREFILL_FRAMES, &mut stats)?;
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
        let mut previous_loop_at = started;
        let mut previous_refill_at = started;
        let mut elevated_until: Option<Instant> = None;
        let mut target_frames = DUC_PREFILL_FRAMES;
        let mut low_water_active = false;
        let mut critical_low_active = false;
        let mut previous_batch_frames = DUC_PREFILL_FRAMES;
        while started.elapsed() < duration {
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

            let requested_target = adaptive_target_frames(occupied_frames, loop_gap);
            if requested_target > target_frames {
                target_frames = requested_target;
                elevated_until = Some(loop_at + ADAPTIVE_HOLD_TIME);
                if target_frames == DUC_MAX_DMA_BATCH_FRAMES {
                    stats.expansions_to_eleven += 1;
                } else {
                    stats.expansions_to_ten += 1;
                }
            } else if elevated_until.is_some_and(|deadline| loop_at >= deadline) {
                target_frames = DUC_PREFILL_FRAMES;
                elevated_until = None;
            }

            if low_now {
                let frames =
                    refill_batch_frames(fifo.occupied_words, self.fifo_depth_words, target_frames);
                if frames != 0 {
                    let refill_started = Instant::now();
                    apply_rf_disabled_duc_state(self.registers, true)?;
                    stats.safety_checks += 1;
                    stats.refill_gaps_ns.push(
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
                        .refill_service_latencies_ns
                        .push(refill_started.elapsed().as_nanos());
                    continue;
                }
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

        let pair_rate = stats.consumed_iq_pairs() as f64 / stats.elapsed.as_secs_f64().max(0.001);
        let minimum_rate = DUC_SAMPLE_RATE_HZ as f64 * 0.95;
        let maximum_rate = DUC_SAMPLE_RATE_HZ as f64 * 1.05;
        if !(minimum_rate..=maximum_rate).contains(&pair_rate) {
            return Err(XdmaError::Incompatible(format!(
                "DUC consumed IQ at {pair_rate:.1} pairs/s; expected within 5% of {DUC_SAMPLE_RATE_HZ}"
            )));
        }
        if stats.fifo_overflows != 0 || stats.fifo_over_threshold != 0 || stats.fifo_underflows != 0
        {
            return Err(XdmaError::Incompatible(format!(
                "DUC runtime FIFO fault: overflow={} threshold={} underflow={}",
                stats.fifo_overflows, stats.fifo_over_threshold, stats.fifo_underflows
            )));
        }
        let p9999_refill_service = percentile_ns(&stats.refill_service_latencies_ns, 0.9999);
        if p9999_refill_service > MAX_P9999_REFILL_SERVICE.as_nanos() {
            return Err(XdmaError::Incompatible(format!(
                "DUC p99.99 refill service {:.3} ms exceeds the {:.3} ms Phase 4 performance gate",
                p9999_refill_service as f64 / 1_000_000.0,
                MAX_P9999_REFILL_SERVICE.as_secs_f64() * 1_000.0
            )));
        }
        Ok(stats)
    }

    fn write_frames(&mut self, frames: usize, stats: &mut DucStats) -> Result<(), XdmaError> {
        if frames == 0 || frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "invalid DUC DMA batch of {frames} frames"
            )));
        }
        let bytes = frames * DUC_FRAME_BYTES;
        let write_started = Instant::now();
        let written = self
            .dma
            .write_at(self.buffer.as_slice(bytes), DUC_DMA_AXI_OFFSET)
            .map_err(|source| XdmaError::Io {
                action: "could not write zero IQ to XDMA DUC stream",
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
        stats.max_write_time_ns = stats.max_write_time_ns.max(write_time);
        stats.write_latencies_ns.push(write_time);
        Ok(())
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

    fn stop(&mut self) -> Result<(), XdmaError> {
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
    let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
    let duc_path = env::var_os("SATURN_BRIDGE_XDMA_DUC_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DUC_DEVICE));

    let mut registers = XdmaRegisterDevice::open(&register_path)?;
    let identity: SaturnIdentity = registers.identity().clone();
    let mut session = DucDmaSession::start(&mut registers, &duc_path)?;
    let probe = session.run(config.duration);
    let stop = session.stop();
    drop(session);
    let stats = probe?;
    stop?;
    registers.close_safely()?;

    let elapsed = stats.elapsed.as_secs_f64().max(0.001);
    let pair_rate = stats.consumed_iq_pairs() as f64 / elapsed;
    let payload_mbps = stats.dma_bytes as f64 * 8.0 / elapsed / 1_000_000.0;
    let average_batch = stats.frames_written as f64 / stats.dma_writes.max(1) as f64;
    let average_write_us = stats.write_time_ns as f64 / stats.dma_writes.max(1) as f64 / 1_000.0;
    let max_write_us = stats.max_write_time_ns as f64 / 1_000.0;
    let p99_write_us = percentile_ns(&stats.write_latencies_ns, 0.99) as f64 / 1_000.0;
    let p9999_write_us = percentile_ns(&stats.write_latencies_ns, 0.9999) as f64 / 1_000.0;
    let p99_refill_gap_ms = percentile_ns(&stats.refill_gaps_ns, 0.99) as f64 / 1_000_000.0;
    let p9999_refill_gap_ms = percentile_ns(&stats.refill_gaps_ns, 0.9999) as f64 / 1_000_000.0;
    let p99_refill_service_ms =
        percentile_ns(&stats.refill_service_latencies_ns, 0.99) as f64 / 1_000_000.0;
    let p9999_refill_service_ms =
        percentile_ns(&stats.refill_service_latencies_ns, 0.9999) as f64 / 1_000_000.0;
    let max_loop_gap_ms = stats.max_loop_gap_ns as f64 / 1_000_000.0;
    let minimum_fifo_margin_ms = stats.fifo_lwm as f64
        / (DUC_FIFO_WORDS_PER_FRAME * DUC_FRAMES_PER_SECOND as usize) as f64
        * 1_000.0;

    println!(
        "saturn-bridge: XDMA Phase 4 DUC probe passed product={} pcb={} firmware={}.{} device={} duration_ms={} target_rate={}Hz target_frames_s={} frames_written={} iq_pairs_consumed={} iq_rate={:.1}/s dma_writes={} dma_bytes={} payload={:.3}Mbit/s average_batch={:.2} max_batch={} batch_changes={} average_write={:.1}us p99_write={:.1}us p99.99_write={:.1}us max_write={:.1}us p99_refill_gap={:.3}ms p99.99_refill_gap={:.3}ms p99_refill_service={:.3}ms p99.99_refill_service={:.3}ms max_loop_gap={:.3}ms fifo_depth={} fifo_lwm={} fifo_hwm={} fifo_final={} minimum_fifo_margin={:.3}ms low_water_events={} critical_low_events={} expand_10={} expand_11={} fifo_startup_underflow={} fifo_overflow={} fifo_threshold={} fifo_underflow={} safety_checks={} zero_iq=1 amplitude_zero=1 mox=0 tx_enable=0 pa_relay=0 cw=0",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        duc_path.display(),
        stats.elapsed.as_millis(),
        DUC_SAMPLE_RATE_HZ,
        DUC_FRAMES_PER_SECOND,
        stats.frames_written,
        stats.consumed_iq_pairs(),
        pair_rate,
        stats.dma_writes,
        stats.dma_bytes,
        payload_mbps,
        average_batch,
        stats.max_batch_frames,
        stats.batch_size_changes,
        average_write_us,
        p99_write_us,
        p9999_write_us,
        max_write_us,
        p99_refill_gap_ms,
        p9999_refill_gap_ms,
        p99_refill_service_ms,
        p9999_refill_service_ms,
        max_loop_gap_ms,
        duc_fifo_depth_words(identity.firmware_minor),
        stats.fifo_lwm,
        stats.fifo_hwm,
        stats.fifo_final,
        minimum_fifo_margin_ms,
        stats.low_water_events,
        stats.critical_low_events,
        stats.expansions_to_ten,
        stats.expansions_to_eleven,
        u8::from(stats.fifo_startup_underflow),
        stats.fifo_overflows,
        stats.fifo_over_threshold,
        stats.fifo_underflows,
        stats.safety_checks,
    );
    println!(
        "saturn-bridge: XDMA Phase 4 cleanup completed; DUC stream and output gate disabled, DUC FIFO reset, zero amplitude retained, and RF remains receive-safe"
    );
    Ok(())
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

fn adaptive_target_frames(occupied_frames: usize, loop_gap: Duration) -> usize {
    if occupied_frames <= 2 || loop_gap >= EXPAND_TO_ELEVEN_STALL {
        11
    } else if occupied_frames <= 3 || loop_gap >= EXPAND_TO_TEN_STALL {
        10
    } else {
        DUC_PREFILL_FRAMES
    }
}

fn refill_batch_frames(
    occupied_words: usize,
    fifo_depth_words: usize,
    target_frames: usize,
) -> usize {
    let target_words = target_frames.min(DUC_MAX_DMA_BATCH_FRAMES) * DUC_FIFO_WORDS_PER_FRAME;
    if occupied_words >= target_words {
        return 0;
    }
    let needed_words = target_words - occupied_words;
    let needed_frames = needed_words.div_ceil(DUC_FIFO_WORDS_PER_FRAME);
    let free_frames = fifo_depth_words.saturating_sub(occupied_words) / DUC_FIFO_WORDS_PER_FRAME;
    needed_frames.min(free_frames).min(DUC_MAX_DMA_BATCH_FRAMES)
}

fn percentile_ns(samples: &[u128], percentile: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (percentile.clamp(0.0, 1.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
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
    fn refill_window_batches_to_nine_frames() {
        assert_eq!(refill_batch_frames(9 * 180, 4_096, 9), 0);
        assert_eq!(refill_batch_frames(5 * 180, 4_096, 9), 4);
        assert_eq!(refill_batch_frames(4 * 180 + 1, 4_096, 9), 5);
        assert_eq!(refill_batch_frames(0, 1_024, 9), 5);
        assert_eq!(refill_batch_frames(4 * 180, 4_096, 10), 6);
        assert_eq!(refill_batch_frames(2 * 180, 4_096, 11), 9);
    }

    #[test]
    fn adaptive_target_expands_on_low_water_or_scheduler_stall() {
        assert_eq!(adaptive_target_frames(5, Duration::from_millis(1)), 9);
        assert_eq!(adaptive_target_frames(4, Duration::from_millis(1)), 9);
        assert_eq!(adaptive_target_frames(3, Duration::from_millis(1)), 10);
        assert_eq!(adaptive_target_frames(2, Duration::from_millis(1)), 11);
        assert_eq!(adaptive_target_frames(8, Duration::from_millis(2)), 10);
        assert_eq!(adaptive_target_frames(8, Duration::from_millis(3)), 11);
    }

    #[test]
    fn percentile_uses_nearest_rank_tail() {
        let samples = [10, 20, 30, 40, 50];
        assert_eq!(percentile_ns(&samples, 0.5), 30);
        assert_eq!(percentile_ns(&samples, 0.99), 50);
        assert_eq!(percentile_ns(&samples, 0.9999), 50);
        assert_eq!(percentile_ns(&[], 0.99), 0);
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
}
