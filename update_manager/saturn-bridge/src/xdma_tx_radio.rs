//! Production direct-XDMA transmit output for the shared WDSP TX pipeline.
//!
//! This deliberately keeps the validated Phase 4/5 one-shot probes intact.
//! The runtime owns a separate register descriptor and H2C0 descriptor, but
//! shares their proven FIFO geometry, sample packing, register ordering, and
//! fail-safe receive cleanup. Initial production enablement remains limited to
//! the field-qualified primary PCB2 firmware 1.27 image and 3 W maximum.

use crate::radio_model::RadioModel;
use crate::tx_thread::{TxRadio, TxRadioResult};
use crate::xdma::{ensure_p2app_inactive, XdmaError, XdmaRegisterDevice};
use crate::xdma_rx::AlignedBuffer;
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_DUC_DEVICE: &str = "/dev/xdma0_h2c_0";
const DEFAULT_USER_DEVICE: &str = "/dev/xdma0_user";

const TX_CONFIG_REGISTER: u64 = 0x2008;
const TX_DUC_REGISTER: u64 = 0x200c;
const RF_GPIO_REGISTER: u64 = 0x2014;
const DAC_CONTROL_REGISTER: u64 = 0x201c;
const FIFO_RESET_REGISTER: u64 = 0x7000;
const DUC_FIFO_MONITOR_REGISTER: u64 = 0x9004;
const DUC_FIFO_MONITOR_CONFIG_REGISTER: u64 = 0x9014;
const ALEX_FORWARD_POWER_REGISTER: u64 = 0xa000;
const ALEX_REVERSE_POWER_REGISTER: u64 = 0xa004;
const ALEX_TX_FILTER_REGISTER: u64 = 0xb000;
const ALEX_TX_ANTENNA_REGISTER: u64 = 0xb008;

const DUC_FIFO_RESET_BIT: u32 = 1 << 3;
const MOX_BIT: u32 = 1 << 24;
const TX_ENABLE_BIT: u32 = 1 << 25;
const RF_DATA_NETWORK_ENDIAN_BIT: u32 = 1 << 26;
const TX_RELAY_DISABLE_BIT: u32 = 1 << 27;
const TX_MODULATION_SOURCE_MASK: u32 = 0b11;
const TX_OUTPUT_GATE_BIT: u32 = 1 << 2;
const TX_PROTOCOL_P2_BIT: u32 = 1 << 3;
const TX_AMPLITUDE_MASK: u32 = 0x3ffff << 4;
const TX_WATCHDOG_OVERRIDE_BIT: u32 = 1 << 28;
const DUC_MUX_RESET_BIT: u32 = 1 << 29;
const TX_IQ_DEINTERLEAVE_BIT: u32 = 1 << 30;
const DUC_STREAM_ENABLE_BIT: u32 = 1 << 31;
const PCB2_FW13_TX_AMPLITUDE: u32 = 0x2000;

const ALEX_ANT1_BIT: u16 = 0x0100;
const ALEX_TX_RELAY_BIT: u16 = 0x0800;
const DUC_FRAME_IQ_FLOATS: usize = 480;
const DUC_FRAME_BYTES: usize = 1_440;
const DUC_FIFO_WORDS_PER_FRAME: usize = 180;
const DUC_PREFILL_MINIMUM_WORDS: usize = 18 * DUC_FIFO_WORDS_PER_FRAME;
const DUC_PREFILL_TARGET_WORDS: usize = 19 * DUC_FIFO_WORDS_PER_FRAME;
const DUC_PREFILL_MAX_ATTEMPTS: usize = 8;
const DUC_MAX_DMA_BATCH_FRAMES: usize = 11;
const DIRECT_TX_STEADY_BATCH_FRAMES: usize = 8;
const DUC_PREFILL_DMA_SETTLE: Duration = Duration::from_micros(500);
const DUC_FIFO_PACING_POLL: Duration = Duration::from_micros(100);
const DUC_FIFO_PACING_POLLS: usize = 200;
const DUC_FIFO_WRITE_HEADROOM_FRAMES: usize = 3;
// The shared XDMA completion kthread runs at FIFO priority 20.  The producer
// must be one level higher so that, once an H2C completion wakes it, a busy
// completion thread servicing continuous C2H traffic cannot defer its return
// to userspace long enough to drain the DUC FIFO.
const XDMA_COMPLETION_RT_PRIORITY: i32 = 20;
const DIRECT_TX_RT_PRIORITY: i32 = XDMA_COMPLETION_RT_PRIORITY + 1;
const DMA_BUFFER_BYTES: usize = DUC_MAX_DMA_BATCH_FRAMES * DUC_FRAME_BYTES;
const INITIAL_MAX_WATTS: u8 = 3;
const REVERSE_POWER_TRIP_WATTS: f32 = 0.75;
const FORWARD_POWER_TRIP_WATTS: f32 = 4.0;
const SWR_TRIP: f32 = 3.0;
const SWR_MIN_FORWARD_WATTS: f32 = 0.25;
const DIRECT_TX_STARTUP_SETTLE_BLOCKS: usize = 4;
const DIRECT_TX_KEY_QUALIFICATION_PACKETS: usize = 8;
const DIRECT_TX_MIC_RECENCY_WINDOW: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DirectTxSnapshot {
    pub(crate) stream_active: bool,
    pub(crate) keyed: bool,
    pub(crate) dma_writes: u64,
    pub(crate) frames_written: u64,
    pub(crate) fifo_lwm: usize,
    pub(crate) fifo_hwm: usize,
    pub(crate) fifo_faults: u64,
    pub(crate) fifo_startup_underflows: u64,
    pub(crate) forward_watts: f32,
    pub(crate) reverse_watts: f32,
    pub(crate) swr: f32,
    pub(crate) sessions_started: u64,
    pub(crate) sessions_completed: u64,
    pub(crate) mux_resets: u64,
    pub(crate) last_session: Option<DirectTxSessionSnapshot>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DirectTxSessionSnapshot {
    pub(crate) id: u64,
    pub(crate) duration_ms: u64,
    pub(crate) frequency_hz: u64,
    pub(crate) filter_low_hz: i32,
    pub(crate) filter_high_hz: i32,
    pub(crate) keyed: bool,
    pub(crate) dma_writes: u64,
    pub(crate) frames_written: u64,
    pub(crate) fifo_lwm: usize,
    pub(crate) fifo_hwm: usize,
    pub(crate) fifo_faults: u64,
    pub(crate) startup_underflows: u64,
    pub(crate) mux_resets: u64,
    pub(crate) peak_forward_watts: f32,
    pub(crate) peak_reverse_watts: f32,
    pub(crate) peak_swr: f32,
}

struct ActiveTxSession {
    id: u64,
    started_at: Instant,
    frequency_hz: u64,
    filter_low_hz: i32,
    filter_high_hz: i32,
    keyed: bool,
    dma_writes_start: u64,
    frames_written_start: u64,
    fifo_faults_start: u64,
    startup_underflows_start: u64,
    mux_resets_start: u64,
    fifo_lwm: usize,
    fifo_hwm: usize,
    peak_forward_watts: f32,
    peak_reverse_watts: f32,
    peak_swr: f32,
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
            overflow: value & (1 << 31) != 0,
            over_threshold: value & (1 << 30) != 0,
            underflow: value & (1 << 29) != 0,
        }
    }
}

fn prefill_has_hard_fault(snapshot: FifoSnapshot) -> bool {
    snapshot.overflow || snapshot.over_threshold
}

fn zero_frames_needed_for_prefill(occupied_words: usize) -> usize {
    let deficit = DUC_PREFILL_TARGET_WORDS.saturating_sub(occupied_words);
    deficit.saturating_add(DUC_FIFO_WORDS_PER_FRAME - 1) / DUC_FIFO_WORDS_PER_FRAME
}

struct DirectTxState {
    registers: XdmaRegisterDevice,
    dma: File,
    buffer: AlignedBuffer,
    fifo_depth_words: usize,
    stream_active: bool,
    keyed: bool,
    dma_writes: u64,
    frames_written: u64,
    fifo_lwm: usize,
    fifo_hwm: usize,
    fifo_faults: u64,
    fifo_startup_underflows: u64,
    forward_watts: f32,
    reverse_watts: f32,
    swr: f32,
    power_meter_scale: f32,
    sessions_started: u64,
    sessions_completed: u64,
    mux_resets: u64,
    active_session: Option<ActiveTxSession>,
    last_session: Option<DirectTxSessionSnapshot>,
}

impl DirectTxState {
    fn open(power_meter_scale: f32) -> Result<Self, XdmaError> {
        ensure_p2app_inactive()?;
        let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_USER_DEVICE));
        let duc_path = env::var_os("SATURN_BRIDGE_XDMA_DUC_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DUC_DEVICE));
        let registers = XdmaRegisterDevice::open(&register_path)?;
        let identity = registers.identity();
        if identity.is_fallback()
            || identity.pcb_version != 2
            || identity.firmware_major != 1
            || identity.firmware_minor != 27
        {
            return Err(XdmaError::Incompatible(format!(
                "production direct TX is qualified only for primary Saturn PCB2 firmware 1.27; found pcb={} firmware={}.{} image={}",
                identity.pcb_version,
                identity.firmware_major,
                identity.firmware_minor,
                if identity.is_fallback() { "fallback" } else { "primary" }
            )));
        }
        let dma = OpenOptions::new()
            .write(true)
            .open(&duc_path)
            .map_err(|source| XdmaError::Io {
                action: "could not open production XDMA DUC device",
                source,
            })?;
        let mut buffer = AlignedBuffer::new(DMA_BUFFER_BYTES)?;
        buffer.lock_memory()?;
        let fifo_depth_words = 4_096;
        let mut state = Self {
            registers,
            dma,
            buffer,
            fifo_depth_words,
            stream_active: false,
            keyed: false,
            dma_writes: 0,
            frames_written: 0,
            fifo_lwm: usize::MAX,
            fifo_hwm: 0,
            fifo_faults: 0,
            fifo_startup_underflows: 0,
            forward_watts: 0.0,
            reverse_watts: 0.0,
            swr: 1.0,
            power_meter_scale: power_meter_scale.clamp(0.5, 1.5),
            sessions_started: 0,
            sessions_completed: 0,
            mux_resets: 0,
            active_session: None,
            last_session: None,
        };
        state.shutdown()?;
        Ok(state)
    }

    fn configure_stream(&mut self, model: &RadioModel) -> Result<(), XdmaError> {
        self.shutdown()?;
        self.registers
            .write_register(DAC_CONTROL_REGISTER, dac_control_word(0))?;
        self.registers.write_register(
            TX_DUC_REGISTER,
            frequency_to_phase_word(model.desired.tx_frequency_hz),
        )?;
        let filter = alex_tx_filter_bits(model.desired.tx_frequency_hz);
        self.registers
            .write_register(ALEX_TX_FILTER_REGISTER, u32::from(filter))?;
        self.registers
            .write_register(ALEX_TX_ANTENNA_REGISTER, u32::from(filter | ALEX_ANT1_BIT))?;
        self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| {
                (value | RF_DATA_NETWORK_ENDIAN_BIT | TX_RELAY_DISABLE_BIT)
                    & !(MOX_BIT | TX_ENABLE_BIT)
            },
            "could not configure direct TX RF byte order",
        )?;
        // Match P2_app's InDUCIQ startup boundary exactly. Resetting only the
        // FIFO is insufficient: an unkey can leave the FPGA's 64-to-48-bit
        // DUC multiplexer holding a partial word. If that state survives into
        // the next stream, otherwise valid Q/I bytes are decoded on the wrong
        // 48-bit boundary and sound like wideband static.
        self.begin_session(model);
        self.reset_duc_input_path()?;
        self.registers.write_register(
            DUC_FIFO_MONITOR_CONFIG_REGISTER,
            self.fifo_depth_words as u32,
        )?;
        self.registers.update_register(
            TX_CONFIG_REGISTER,
            rf_disabled_stream_config,
            "could not enable RF-inhibited production DUC stream",
        )?;
        // Clear the expected empty-FIFO startup condition.
        let _ = self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?;
        self.stream_active = true;
        self.keyed = false;
        Ok(())
    }

    fn stage_frame_batch(&mut self, model: &RadioModel, iq: &[f32]) -> Result<(), XdmaError> {
        if iq.is_empty() || !iq.len().is_multiple_of(DUC_FRAME_IQ_FLOATS) {
            return Err(XdmaError::Incompatible(format!(
                "invalid RF-disabled production DUC batch: floats={} frame_floats={DUC_FRAME_IQ_FLOATS}",
                iq.len()
            )));
        }
        let frames = iq.len() / DUC_FRAME_IQ_FLOATS;
        if frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "RF-disabled production DUC batch exceeds DMA buffer: frames={frames} maximum={DUC_MAX_DMA_BATCH_FRAMES}"
            )));
        }
        if !self.stream_active {
            self.configure_stream(model)?;
            let (first, remaining) = iq.split_at(DUC_FRAME_IQ_FLOATS);
            self.prefill_frame(first)?;
            if !remaining.is_empty() {
                self.write_frame_batch_paced(remaining, true)?;
            }
        } else {
            self.write_frame_batch_paced(iq, true)?;
        }
        Ok(())
    }

    fn key_with_frame(&mut self, model: &RadioModel, iq: &[f32]) -> Result<(), XdmaError> {
        let fifo = if !self.stream_active {
            self.configure_stream(model)?;
            self.prefill_frame(iq)?
        } else {
            self.observe_fifo(false)?
        };
        if fifo.occupied_words < DUC_PREFILL_MINIMUM_WORDS {
            return Err(XdmaError::Incompatible(format!(
                "production DUC prefill has {} words; at least {} required before key",
                fifo.occupied_words, DUC_PREFILL_MINIMUM_WORDS
            )));
        }
        self.apply_frequency_and_filter(model)?;
        self.registers.update_register(
            TX_CONFIG_REGISTER,
            keyed_stream_config,
            "could not arm production direct-XDMA TX configuration",
        )?;
        let filter = alex_tx_filter_bits(model.desired.tx_frequency_hz);
        self.registers.write_register(
            ALEX_TX_ANTENNA_REGISTER,
            u32::from(filter | ALEX_ANT1_BIT | ALEX_TX_RELAY_BIT),
        )?;
        self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| (value | TX_ENABLE_BIT) & !(MOX_BIT | TX_RELAY_DISABLE_BIT),
            "could not enable production direct-XDMA TX hardware",
        )?;
        self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| value | MOX_BIT,
            "could not assert production direct-XDMA MOX",
        )?;
        let drive = tx_drive_watts_to_byte(model.desired.tx_drive.min(INITIAL_MAX_WATTS));
        self.registers
            .write_register(DAC_CONTROL_REGISTER, dac_control_word(drive))?;
        self.verify_keyed(model)?;
        println!(
            "saturn-bridge: direct XDMA TX keyed carrier={}Hz mode={} filter={}..{}Hz phase_word=0x{:08x} iq_pack=Q,I fifo_words={}",
            model.desired.tx_frequency_hz,
            model.desired.mode,
            model.desired.tx_filter_low_hz,
            model.desired.tx_filter_high_hz,
            frequency_to_phase_word(model.desired.tx_frequency_hz),
            fifo.occupied_words
        );
        self.keyed = true;
        if let Some(session) = self.active_session.as_mut() {
            session.keyed = true;
        }
        Ok(())
    }

    fn begin_session(&mut self, model: &RadioModel) {
        self.sessions_started = self.sessions_started.saturating_add(1);
        self.active_session = Some(ActiveTxSession {
            id: self.sessions_started,
            started_at: Instant::now(),
            frequency_hz: u64::from(model.desired.tx_frequency_hz),
            filter_low_hz: model.desired.tx_filter_low_hz,
            filter_high_hz: model.desired.tx_filter_high_hz,
            keyed: false,
            dma_writes_start: self.dma_writes,
            frames_written_start: self.frames_written,
            fifo_faults_start: self.fifo_faults,
            startup_underflows_start: self.fifo_startup_underflows,
            mux_resets_start: self.mux_resets,
            fifo_lwm: usize::MAX,
            fifo_hwm: 0,
            peak_forward_watts: 0.0,
            peak_reverse_watts: 0.0,
            peak_swr: 1.0,
        });
    }

    fn observe_session_fifo(&mut self, occupied_words: usize) {
        if let Some(session) = self.active_session.as_mut() {
            session.fifo_lwm = session.fifo_lwm.min(occupied_words);
            session.fifo_hwm = session.fifo_hwm.max(occupied_words);
        }
    }

    fn finish_session(&mut self) {
        let Some(session) = self.active_session.take() else {
            return;
        };
        self.sessions_completed = self.sessions_completed.saturating_add(1);
        self.last_session = Some(DirectTxSessionSnapshot {
            id: session.id,
            duration_ms: session.started_at.elapsed().as_millis() as u64,
            frequency_hz: session.frequency_hz,
            filter_low_hz: session.filter_low_hz,
            filter_high_hz: session.filter_high_hz,
            keyed: session.keyed,
            dma_writes: self.dma_writes.saturating_sub(session.dma_writes_start),
            frames_written: self
                .frames_written
                .saturating_sub(session.frames_written_start),
            fifo_lwm: if session.fifo_lwm == usize::MAX {
                0
            } else {
                session.fifo_lwm
            },
            fifo_hwm: session.fifo_hwm,
            fifo_faults: self.fifo_faults.saturating_sub(session.fifo_faults_start),
            startup_underflows: self
                .fifo_startup_underflows
                .saturating_sub(session.startup_underflows_start),
            mux_resets: self.mux_resets.saturating_sub(session.mux_resets_start),
            peak_forward_watts: session.peak_forward_watts,
            peak_reverse_watts: session.peak_reverse_watts,
            peak_swr: session.peak_swr,
        });
    }

    fn apply_frequency_and_filter(&self, model: &RadioModel) -> Result<(), XdmaError> {
        let filter = alex_tx_filter_bits(model.desired.tx_frequency_hz);
        self.registers.write_register(
            TX_DUC_REGISTER,
            frequency_to_phase_word(model.desired.tx_frequency_hz),
        )?;
        self.registers
            .write_register(ALEX_TX_FILTER_REGISTER, u32::from(filter))
    }

    fn write_repeated_frame(&mut self, iq: &[f32], frames: usize) -> Result<(), XdmaError> {
        if iq.len() < DUC_FRAME_IQ_FLOATS || frames == 0 || frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "invalid production DUC frame: floats={} frames={frames}",
                iq.len()
            )));
        }
        let bytes = frames * DUC_FRAME_BYTES;
        let buffer = self.buffer.as_mut_slice(bytes);
        for frame in buffer.chunks_exact_mut(DUC_FRAME_BYTES) {
            encode_iq_frame(frame, iq);
        }
        self.write_buffer(bytes, frames)
    }

    fn write_frame_batch(&mut self, iq: &[f32]) -> Result<(), XdmaError> {
        if iq.is_empty() || !iq.len().is_multiple_of(DUC_FRAME_IQ_FLOATS) {
            return Err(XdmaError::Incompatible(format!(
                "invalid production DUC batch: floats={} frame_floats={DUC_FRAME_IQ_FLOATS}",
                iq.len()
            )));
        }
        let frames = iq.len() / DUC_FRAME_IQ_FLOATS;
        if frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "production DUC batch exceeds DMA buffer: frames={frames} maximum={DUC_MAX_DMA_BATCH_FRAMES}"
            )));
        }
        let bytes = frames * DUC_FRAME_BYTES;
        let buffer = self.buffer.as_mut_slice(bytes);
        encode_iq_batch(buffer, iq);
        self.write_buffer(bytes, frames)
    }

    fn write_frame_batch_paced(
        &mut self,
        iq: &[f32],
        underflow_is_fault: bool,
    ) -> Result<(), XdmaError> {
        if iq.is_empty() || !iq.len().is_multiple_of(DUC_FRAME_IQ_FLOATS) {
            return Err(XdmaError::Incompatible(format!(
                "invalid paced production DUC batch: floats={} frame_floats={DUC_FRAME_IQ_FLOATS}",
                iq.len()
            )));
        }
        let requested_frames = iq.len() / DUC_FRAME_IQ_FLOATS;
        if requested_frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "paced production DUC batch exceeds DMA buffer: frames={requested_frames} maximum={DUC_MAX_DMA_BATCH_FRAMES}"
            )));
        }

        let mut sent_frames = 0usize;
        let mut polls_without_progress = 0usize;
        while sent_frames < requested_frames {
            let fifo = self.observe_fifo(underflow_is_fault)?;
            let remaining_frames = requested_frames - sent_frames;
            let frames = fifo_batch_frames_available(
                self.fifo_depth_words,
                fifo.occupied_words,
                remaining_frames,
            );
            if frames == 0 {
                if polls_without_progress >= DUC_FIFO_PACING_POLLS {
                    self.fifo_faults = self.fifo_faults.saturating_add(1);
                    let _ = self.shutdown();
                    return Err(XdmaError::Incompatible(format!(
                        "production DUC FIFO pacing timeout: words={} remaining_frames={remaining_frames}",
                        fifo.occupied_words
                    )));
                }
                thread::sleep(DUC_FIFO_PACING_POLL);
                polls_without_progress += 1;
                continue;
            }

            let start = sent_frames * DUC_FRAME_IQ_FLOATS;
            let end = start + frames * DUC_FRAME_IQ_FLOATS;
            self.write_frame_batch(&iq[start..end])?;
            sent_frames += frames;
            polls_without_progress = 0;
        }
        Ok(())
    }

    fn write_zero_frames(&mut self, frames: usize) -> Result<(), XdmaError> {
        if frames == 0 || frames > DUC_MAX_DMA_BATCH_FRAMES {
            return Err(XdmaError::Incompatible(format!(
                "invalid production zero-IQ DUC batch: frames={frames}"
            )));
        }
        let bytes = frames * DUC_FRAME_BYTES;
        self.buffer.as_mut_slice(bytes).fill(0);
        self.write_buffer(bytes, frames)
    }

    fn write_buffer(&mut self, bytes: usize, frames: usize) -> Result<(), XdmaError> {
        let written = self
            .dma
            .write_at(self.buffer.as_slice(bytes), 0)
            .map_err(|source| XdmaError::Io {
                action: "could not write production IQ to XDMA DUC stream",
                source,
            })?;
        if written != bytes {
            return Err(XdmaError::Io {
                action: "production XDMA DUC stream returned a short write",
                source: io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("transferred {written} of {bytes} bytes"),
                ),
            });
        }
        self.dma_writes = self.dma_writes.saturating_add(1);
        self.frames_written = self.frames_written.saturating_add(frames as u64);
        Ok(())
    }

    fn prefill_frame(&mut self, iq: &[f32]) -> Result<FifoSnapshot, XdmaError> {
        // The stream drains while H2C writes and register reads execute, so a
        // fixed frame count is not a reliable occupancy guarantee. Seed one
        // bounded batch, then close the measured deficit until the target is
        // reached. Keep one live-frame slot below the 4096-word FIFO ceiling.
        self.write_zero_frames(DUC_MAX_DMA_BATCH_FRAMES)?;
        thread::sleep(DUC_PREFILL_DMA_SETTLE);
        let mut startup_underflow_seen = false;
        let mut attempts = 1usize;

        loop {
            let prefill =
                FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
            self.fifo_lwm = self.fifo_lwm.min(prefill.occupied_words);
            self.fifo_hwm = self.fifo_hwm.max(prefill.occupied_words);
            self.observe_session_fifo(prefill.occupied_words);
            if prefill.underflow {
                startup_underflow_seen = true;
                self.fifo_startup_underflows = self.fifo_startup_underflows.saturating_add(1);
            }
            if prefill_has_hard_fault(prefill) {
                return self.prefill_fault(prefill, startup_underflow_seen);
            }
            if prefill.occupied_words >= DUC_PREFILL_TARGET_WORDS {
                break;
            }
            if attempts >= DUC_PREFILL_MAX_ATTEMPTS {
                return self.prefill_fault(prefill, startup_underflow_seen);
            }

            let room_before_live = self
                .fifo_depth_words
                .saturating_sub(DUC_FIFO_WORDS_PER_FRAME)
                .saturating_sub(prefill.occupied_words)
                / DUC_FIFO_WORDS_PER_FRAME;
            let frames = zero_frames_needed_for_prefill(prefill.occupied_words)
                .min(DUC_MAX_DMA_BATCH_FRAMES)
                .min(room_before_live);
            if frames == 0 {
                return self.prefill_fault(prefill, startup_underflow_seen);
            }
            self.write_zero_frames(frames)?;
            thread::sleep(DUC_PREFILL_DMA_SETTLE);
            attempts += 1;
        }

        // The final pre-key frame is the first live IQ; silence is never
        // inserted behind it. The following read is a hard boundary: the
        // startup underflow was already observed and cleared above.
        self.write_repeated_frame(iq, 1)?;
        thread::sleep(DUC_PREFILL_DMA_SETTLE);
        let ready = FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        self.fifo_lwm = self.fifo_lwm.min(ready.occupied_words);
        self.fifo_hwm = self.fifo_hwm.max(ready.occupied_words);
        self.observe_session_fifo(ready.occupied_words);
        if prefill_has_hard_fault(ready)
            || ready.underflow
            || ready.occupied_words < DUC_PREFILL_MINIMUM_WORDS
        {
            return self.prefill_fault(ready, startup_underflow_seen);
        }
        Ok(ready)
    }

    fn prefill_fault<T>(
        &mut self,
        snapshot: FifoSnapshot,
        startup_underflow_seen: bool,
    ) -> Result<T, XdmaError> {
        self.fifo_faults = self.fifo_faults.saturating_add(1);
        let _ = self.shutdown();
        Err(XdmaError::Incompatible(format!(
            "production DUC occupancy prefill fault: words={} minimum={} target={} overflow={} threshold={} underflow={} startup_underflow_seen={}",
            snapshot.occupied_words,
            DUC_PREFILL_MINIMUM_WORDS,
            DUC_PREFILL_TARGET_WORDS,
            snapshot.overflow,
            snapshot.over_threshold,
            snapshot.underflow,
            startup_underflow_seen
        )))
    }

    fn observe_fifo(&mut self, underflow_is_fault: bool) -> Result<FifoSnapshot, XdmaError> {
        let fifo = FifoSnapshot::decode(self.registers.read_register(DUC_FIFO_MONITOR_REGISTER)?);
        self.fifo_lwm = self.fifo_lwm.min(fifo.occupied_words);
        self.fifo_hwm = self.fifo_hwm.max(fifo.occupied_words);
        self.observe_session_fifo(fifo.occupied_words);
        // A low but non-empty FIFO is recoverable: the cadence loop may be
        // catching up after a bounded scheduler/driver delay and this call is
        // immediately followed by a write.  Treat only the FPGA's latched
        // fault bits as fatal.  The former two-frame occupancy guard could
        // force RX at 230 words even though the underflow bit was clear,
        // turning a recoverable refill into an audible TX interruption.
        let faulted = steady_state_fifo_fault(fifo, underflow_is_fault);
        if faulted {
            self.fifo_faults = self.fifo_faults.saturating_add(1);
            let _ = self.shutdown();
            return Err(XdmaError::Incompatible(format!(
                "production DUC FIFO fault: words={} overflow={} threshold={} underflow={}",
                fifo.occupied_words, fifo.overflow, fifo.over_threshold, fifo.underflow
            )));
        }
        Ok(fifo)
    }

    fn sample_power(&mut self) -> Result<(), XdmaError> {
        let forward_raw = self
            .registers
            .read_register(ALEX_FORWARD_POWER_REGISTER)?
            .min(u32::from(u16::MAX)) as u16;
        let reverse_raw = self
            .registers
            .read_register(ALEX_REVERSE_POWER_REGISTER)?
            .min(u32::from(u16::MAX)) as u16;
        self.forward_watts = saturn_adc_to_watts(forward_raw, 32, self.power_meter_scale);
        self.reverse_watts = saturn_adc_to_watts(reverse_raw, 28, self.power_meter_scale);
        self.swr = calculate_swr(self.forward_watts, self.reverse_watts);
        if let Some(session) = self.active_session.as_mut() {
            session.peak_forward_watts = session.peak_forward_watts.max(self.forward_watts);
            session.peak_reverse_watts = session.peak_reverse_watts.max(self.reverse_watts);
            session.peak_swr = session.peak_swr.max(self.swr);
        }
        if self.forward_watts > FORWARD_POWER_TRIP_WATTS
            || self.reverse_watts > REVERSE_POWER_TRIP_WATTS
            || (self.forward_watts >= SWR_MIN_FORWARD_WATTS && self.swr > SWR_TRIP)
        {
            let _ = self.shutdown();
            return Err(XdmaError::Incompatible(format!(
                "production direct TX power trip: forward={:.3}W reverse={:.3}W swr={:.2}",
                self.forward_watts, self.reverse_watts, self.swr
            )));
        }
        Ok(())
    }

    fn verify_keyed(&self, model: &RadioModel) -> Result<(), XdmaError> {
        let gpio = self.registers.read_register(RF_GPIO_REGISTER)?;
        let tx = self.registers.read_register(TX_CONFIG_REGISTER)?;
        let tx_duc = self.registers.read_register(TX_DUC_REGISTER)?;
        let expected_tx_duc = frequency_to_phase_word(model.desired.tx_frequency_hz);
        let filter = alex_tx_filter_bits(model.desired.tx_frequency_hz);
        let alex = self.registers.read_register(ALEX_TX_ANTENNA_REGISTER)?;
        if gpio & (MOX_BIT | TX_ENABLE_BIT) != MOX_BIT | TX_ENABLE_BIT
            || gpio & RF_DATA_NETWORK_ENDIAN_BIT == 0
            || gpio & TX_RELAY_DISABLE_BIT != 0
            || tx & DUC_STREAM_ENABLE_BIT == 0
            || tx & (DUC_MUX_RESET_BIT | TX_IQ_DEINTERLEAVE_BIT) != 0
            || tx & TX_AMPLITUDE_MASK == 0
            || tx_duc != expected_tx_duc
            || alex != u32::from(filter | ALEX_ANT1_BIT | ALEX_TX_RELAY_BIT)
        {
            return Err(XdmaError::Incompatible(format!(
                "production direct TX readback failed: gpio=0x{gpio:08x} tx=0x{tx:08x} tx_duc=0x{tx_duc:08x} expected_tx_duc=0x{expected_tx_duc:08x} alex=0x{alex:08x}"
            )));
        }
        Ok(())
    }

    fn pulse_fifo_reset(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value & !DUC_FIFO_RESET_BIT,
            "could not assert production DUC FIFO reset",
        )?;
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value | DUC_FIFO_RESET_BIT,
            "could not release production DUC FIFO reset",
        )
    }

    fn reset_duc_input_path(&mut self) -> Result<(), XdmaError> {
        self.registers.update_register(
            TX_CONFIG_REGISTER,
            duc_mux_disabled_for_reset,
            "could not disable production DUC mux before reset",
        )?;
        self.registers.update_register(
            TX_CONFIG_REGISTER,
            duc_mux_reset_asserted,
            "could not assert production DUC mux reset",
        )?;
        self.registers.update_register(
            TX_CONFIG_REGISTER,
            duc_mux_reset_released,
            "could not release production DUC mux reset",
        )?;
        self.pulse_fifo_reset()?;
        self.mux_resets = self.mux_resets.saturating_add(1);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), XdmaError> {
        let drive = self
            .registers
            .write_register(DAC_CONTROL_REGISTER, dac_control_word(0));
        let amplitude = self.registers.update_register(
            TX_CONFIG_REGISTER,
            |value| {
                value
                    & !(TX_AMPLITUDE_MASK
                        | TX_OUTPUT_GATE_BIT
                        | DUC_STREAM_ENABLE_BIT
                        | DUC_MUX_RESET_BIT)
            },
            "could not disable production direct-XDMA DUC stream",
        );
        let gpio = self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| (value & !(MOX_BIT | TX_ENABLE_BIT)) | TX_RELAY_DISABLE_BIT,
            "could not force production direct-XDMA RF receive state",
        );
        let alex = self.registers.update_register(
            ALEX_TX_ANTENNA_REGISTER,
            |value| value & !u32::from(ALEX_TX_RELAY_BIT),
            "could not release production direct-XDMA TX relay",
        );
        let reset = self.pulse_fifo_reset();
        self.stream_active = false;
        self.keyed = false;
        let result = drive.and(amplitude).and(gpio).and(alex).and(reset);
        self.finish_session();
        result
    }

    fn snapshot(&self) -> DirectTxSnapshot {
        DirectTxSnapshot {
            stream_active: self.stream_active,
            keyed: self.keyed,
            dma_writes: self.dma_writes,
            frames_written: self.frames_written,
            fifo_lwm: if self.fifo_lwm == usize::MAX {
                0
            } else {
                self.fifo_lwm
            },
            fifo_hwm: self.fifo_hwm,
            fifo_faults: self.fifo_faults,
            fifo_startup_underflows: self.fifo_startup_underflows,
            forward_watts: self.forward_watts,
            reverse_watts: self.reverse_watts,
            swr: self.swr,
            sessions_started: self.sessions_started,
            sessions_completed: self.sessions_completed,
            mux_resets: self.mux_resets,
            last_session: self.last_session,
        }
    }
}

fn steady_state_fifo_fault(snapshot: FifoSnapshot, underflow_is_fault: bool) -> bool {
    snapshot.overflow || snapshot.over_threshold || (underflow_is_fault && snapshot.underflow)
}

fn fifo_batch_frames_available(
    fifo_depth_words: usize,
    occupied_words: usize,
    requested_frames: usize,
) -> usize {
    let safe_ceiling_words =
        fifo_depth_words.saturating_sub(DUC_FIFO_WRITE_HEADROOM_FRAMES * DUC_FIFO_WORDS_PER_FRAME);
    safe_ceiling_words
        .saturating_sub(occupied_words)
        .div_euclid(DUC_FIFO_WORDS_PER_FRAME)
        .min(requested_frames)
        .min(DUC_MAX_DMA_BATCH_FRAMES)
}

impl Drop for DirectTxState {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            eprintln!("saturn-bridge: production XDMA TX emergency cleanup failed: {error}");
        }
    }
}

pub(crate) struct DirectXdmaTxRadio {
    state: Mutex<DirectTxState>,
}

impl DirectXdmaTxRadio {
    pub(crate) fn open(power_meter_scale: f32) -> Result<Self, XdmaError> {
        Ok(Self {
            state: Mutex::new(DirectTxState::open(power_meter_scale)?),
        })
    }

    pub(crate) fn snapshot(&self) -> DirectTxSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot()
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut DirectTxState) -> Result<T, XdmaError>,
    ) -> Result<T, String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation(&mut state).map_err(|error| error.to_string())
    }
}

impl TxRadio for DirectXdmaTxRadio {
    fn configure_puresignal_feedback(&self) -> TxRadioResult {
        Err("PureSignal is not supported by the production direct-XDMA backend".into())
    }

    fn send_duc_specific(&self, _model: &RadioModel) -> TxRadioResult {
        Ok(())
    }

    fn send_high_priority(&self, model: &RadioModel) -> TxRadioResult {
        self.with_state(|state| {
            if !model.desired.tx_enabled {
                state.shutdown()
            } else if state.keyed {
                state.apply_frequency_and_filter(model)?;
                let drive = tx_drive_watts_to_byte(model.desired.tx_drive.min(INITIAL_MAX_WATTS));
                state
                    .registers
                    .write_register(DAC_CONTROL_REGISTER, dac_control_word(drive))
            } else {
                Ok(())
            }
        })
    }

    fn try_key_with_iq(&self, model: &RadioModel, iq_samples: &[f32]) -> Result<bool, String> {
        self.with_state(|state| {
            let result = state.key_with_frame(model, iq_samples);
            if result.is_err() {
                let _ = state.shutdown();
            }
            result
        })?;
        Ok(true)
    }

    fn send_duc_iq(&self, iq_samples: &[f32]) -> TxRadioResult {
        self.send_duc_iq_batch(iq_samples)
    }

    fn max_duc_iq_batch_packets(&self) -> usize {
        DIRECT_TX_STEADY_BATCH_FRAMES
    }

    fn send_duc_iq_batch(&self, iq_samples: &[f32]) -> TxRadioResult {
        self.with_state(|state| {
            let result = (|| {
                if !state.stream_active {
                    return Err(XdmaError::Incompatible(
                        "production DUC write requested while stream is stopped".into(),
                    ));
                }
                if iq_samples.is_empty()
                    || !iq_samples.len().is_multiple_of(DUC_FRAME_IQ_FLOATS)
                {
                    return Err(XdmaError::Incompatible(format!(
                        "invalid production DUC batch: floats={} frame_floats={DUC_FRAME_IQ_FLOATS}",
                        iq_samples.len()
                    )));
                }
                state.write_frame_batch_paced(iq_samples, true)?;
                if state.keyed {
                    state.sample_power()?;
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = state.shutdown();
            }
            result
        })
    }

    fn stage_iq_rf_disabled(&self, model: &RadioModel, iq_samples: &[f32]) -> TxRadioResult {
        self.stage_iq_batch_rf_disabled(model, iq_samples)
    }

    fn stage_iq_batch_rf_disabled(&self, model: &RadioModel, iq_samples: &[f32]) -> TxRadioResult {
        self.with_state(|state| {
            let result = state.stage_frame_batch(model, iq_samples);
            if result.is_err() {
                let _ = state.shutdown();
            }
            result
        })
    }

    fn startup_settle_blocks(&self) -> usize {
        DIRECT_TX_STARTUP_SETTLE_BLOCKS
    }

    fn key_qualification_packets(&self) -> usize {
        DIRECT_TX_KEY_QUALIFICATION_PACKETS
    }

    fn keyable_mic_window(&self) -> Duration {
        DIRECT_TX_MIC_RECENCY_WINDOW
    }

    fn qualify_mic_at_dsp_input(&self) -> bool {
        true
    }

    fn recreate_wdsp_on_arm(&self) -> bool {
        true
    }

    fn defer_model_changes_while_keyed(&self) -> bool {
        true
    }

    fn realtime_priority(&self) -> Option<i32> {
        Some(DIRECT_TX_RT_PRIORITY)
    }

    fn configure_rx_ddc(
        &self,
        _ddc_index: u8,
        _sample_rate_khz: u16,
        _sample_size_bits: u8,
        _adc: u8,
    ) -> TxRadioResult {
        Ok(())
    }
}

fn rf_disabled_stream_config(current: u32) -> u32 {
    (current
        & !(TX_MODULATION_SOURCE_MASK
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT
            | DUC_STREAM_ENABLE_BIT))
        | TX_OUTPUT_GATE_BIT
        | TX_PROTOCOL_P2_BIT
        | DUC_STREAM_ENABLE_BIT
}

fn keyed_stream_config(current: u32) -> u32 {
    (current
        & !(TX_MODULATION_SOURCE_MASK
            | TX_OUTPUT_GATE_BIT
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT
            | DUC_STREAM_ENABLE_BIT))
        | TX_PROTOCOL_P2_BIT
        | (PCB2_FW13_TX_AMPLITUDE << 4)
        | DUC_STREAM_ENABLE_BIT
}

fn duc_mux_disabled_for_reset(current: u32) -> u32 {
    current & !(DUC_STREAM_ENABLE_BIT | DUC_MUX_RESET_BIT | TX_IQ_DEINTERLEAVE_BIT)
}

fn duc_mux_reset_asserted(current: u32) -> u32 {
    current | DUC_MUX_RESET_BIT
}

fn duc_mux_reset_released(current: u32) -> u32 {
    current & !DUC_MUX_RESET_BIT
}

fn encode_iq_frame(target: &mut [u8], iq: &[f32]) {
    for (encoded, pair) in target.chunks_exact_mut(6).zip(iq.chunks_exact(2)) {
        // Match P2_app's InDUCIQ path: Q then I, signed 24-bit big-endian,
        // with the FPGA RF byte-swap bit selected.
        write_i24_be(encoded, float_to_i24(pair[1]));
        write_i24_be(&mut encoded[3..], float_to_i24(pair[0]));
    }
}

fn encode_iq_batch(target: &mut [u8], iq: &[f32]) {
    for (encoded, samples) in target
        .chunks_exact_mut(DUC_FRAME_BYTES)
        .zip(iq.chunks_exact(DUC_FRAME_IQ_FLOATS))
    {
        encode_iq_frame(encoded, samples);
    }
}

fn float_to_i24(value: f32) -> i32 {
    (value.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32
}

fn write_i24_be(target: &mut [u8], value: i32) {
    target[..3].copy_from_slice(&value.to_be_bytes()[1..]);
}

fn frequency_to_phase_word(frequency_hz: u32) -> u32 {
    let numerator = u128::from(frequency_hz.min(122_880_000)) * (1u128 << 32);
    ((numerator + 61_440_000) / 122_880_000) as u32
}

fn alex_tx_filter_bits(frequency_hz: u32) -> u16 {
    match frequency_hz {
        35_600_001..=u32::MAX => 0x2000,
        24_000_001..=35_600_000 => 0x4000,
        16_500_001..=24_000_000 => 0x8000,
        8_000_001..=16_500_000 => 0x0010,
        5_000_001..=8_000_000 => 0x0020,
        2_500_001..=5_000_000 => 0x0040,
        _ => 0x0080,
    }
}

fn tx_drive_watts_to_byte(watts: u8) -> u8 {
    let watts = f32::from(watts.min(INITIAL_MAX_WATTS));
    if watts == 0.0 {
        return 0;
    }
    ((watts / 5.0) * 18.0).round().clamp(1.0, 18.0) as u8
}

fn dac_control_word(level: u8) -> u32 {
    if level == 0 {
        return 0x3f3f_0000;
    }
    let desired_atten = 20.0 * (255.0_f64 / f64::from(level)).log10();
    let step = (2.0 * desired_atten).floor().clamp(0.0, 63.0) as u32;
    let residual_atten = desired_atten - f64::from(step) * 0.5;
    let dac = (255.0 / 10.0_f64.powf(residual_atten / 20.0))
        .floor()
        .clamp(0.0, 255.0) as u32;
    dac | (dac << 8) | (step << 16) | (step << 24)
}

fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (i32::from(raw) - offset).max(0) as f32;
    let volts = corrected / 4095.0 * 5.0;
    (volts * volts / 0.12) * scale
}

fn calculate_swr(forward_watts: f32, reverse_watts: f32) -> f32 {
    if forward_watts <= 0.0 || reverse_watts <= 0.0 {
        return 1.0;
    }
    if reverse_watts >= forward_watts {
        return f32::INFINITY;
    }
    let ratio = (reverse_watts / forward_watts).sqrt();
    (1.0 + ratio) / (1.0 - ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_q_then_i_as_signed_24_bit_big_endian() {
        let mut frame = [0_u8; 6];
        encode_iq_frame(&mut frame, &[1.0, -1.0]);
        assert_eq!(&frame[..3], &[0x80, 0x00, 0x01]);
        assert_eq!(&frame[3..], &[0x7f, 0xff, 0xff]);
    }

    #[test]
    fn batch_packing_preserves_distinct_consecutive_frames() {
        let mut iq = vec![0.0; DUC_FRAME_IQ_FLOATS * 2];
        iq[0] = 1.0;
        iq[1] = -1.0;
        iq[DUC_FRAME_IQ_FLOATS] = -0.5;
        iq[DUC_FRAME_IQ_FLOATS + 1] = 0.5;
        let mut encoded = vec![0_u8; DUC_FRAME_BYTES * 2];
        encode_iq_batch(&mut encoded, &iq);

        let mut first_pair = [0_u8; 6];
        encode_iq_frame(&mut first_pair, &iq[..2]);
        let mut second_pair = [0_u8; 6];
        encode_iq_frame(
            &mut second_pair,
            &iq[DUC_FRAME_IQ_FLOATS..DUC_FRAME_IQ_FLOATS + 2],
        );
        assert_eq!(&encoded[..6], &first_pair);
        assert_eq!(&encoded[DUC_FRAME_BYTES..DUC_FRAME_BYTES + 6], &second_pair);
        assert_ne!(first_pair, second_pair);
    }

    #[test]
    fn production_drive_is_clamped_to_three_watts() {
        assert_eq!(tx_drive_watts_to_byte(3), tx_drive_watts_to_byte(100));
        assert!(tx_drive_watts_to_byte(3) < 18);
    }

    #[test]
    fn prefill_accepts_only_the_expected_startup_underflow() {
        let startup = FifoSnapshot {
            occupied_words: DUC_PREFILL_TARGET_WORDS,
            underflow: true,
            ..FifoSnapshot::default()
        };
        assert!(!prefill_has_hard_fault(startup));
        assert!(prefill_has_hard_fault(FifoSnapshot {
            overflow: true,
            ..startup
        }));
        assert!(prefill_has_hard_fault(FifoSnapshot {
            over_threshold: true,
            ..startup
        }));
    }

    #[test]
    fn occupancy_prefill_closes_the_observed_live_deficit() {
        assert_eq!(zero_frames_needed_for_prefill(3_039), 3);
        assert_eq!(zero_frames_needed_for_prefill(DUC_PREFILL_TARGET_WORDS), 0);
        assert_eq!(
            zero_frames_needed_for_prefill(DUC_PREFILL_TARGET_WORDS + 100),
            0
        );
    }

    #[test]
    fn low_nonempty_fifo_is_recoverable_until_hardware_reports_a_fault() {
        let low = FifoSnapshot {
            occupied_words: 230,
            ..FifoSnapshot::default()
        };
        assert!(!steady_state_fifo_fault(low, true));
        assert!(steady_state_fifo_fault(
            FifoSnapshot {
                underflow: true,
                ..low
            },
            true
        ));
        assert!(!steady_state_fifo_fault(
            FifoSnapshot {
                underflow: true,
                ..low
            },
            false
        ));
    }

    #[test]
    fn direct_tx_producer_preempts_the_xdma_completion_thread_after_wake() {
        assert_eq!(XDMA_COMPLETION_RT_PRIORITY, 20);
        assert_eq!(DIRECT_TX_RT_PRIORITY, 21);
        assert!(DIRECT_TX_RT_PRIORITY > XDMA_COMPLETION_RT_PRIORITY);
    }

    #[test]
    fn steady_state_batch_fits_dma_buffer_and_spans_ten_milliseconds() {
        assert_eq!(DIRECT_TX_STEADY_BATCH_FRAMES, 8);
        assert!(DIRECT_TX_STEADY_BATCH_FRAMES <= DUC_MAX_DMA_BATCH_FRAMES);
        assert_eq!(DIRECT_TX_STEADY_BATCH_FRAMES * DUC_FRAME_IQ_FLOATS, 3_840);
    }

    #[test]
    fn fifo_pacing_writes_safe_partial_batches_without_waiting_for_the_whole_batch() {
        assert_eq!(fifo_batch_frames_available(4_096, 3_560, 8), 0);
        assert_eq!(fifo_batch_frames_available(4_096, 3_376, 8), 1);
        assert_eq!(fifo_batch_frames_available(4_096, 3_016, 8), 3);
        assert_eq!(fifo_batch_frames_available(4_096, 2_116, 8), 8);
        assert_eq!(fifo_batch_frames_available(4_096, 1_184, 8), 8);
    }

    #[test]
    fn duc_mux_reset_sequence_preserves_unrelated_tx_configuration() {
        let unrelated = TX_PROTOCOL_P2_BIT | TX_OUTPUT_GATE_BIT | (0x1234 << 4);
        let initial =
            unrelated | DUC_STREAM_ENABLE_BIT | DUC_MUX_RESET_BIT | TX_IQ_DEINTERLEAVE_BIT;
        let disabled = duc_mux_disabled_for_reset(initial);
        assert_eq!(
            disabled & (DUC_STREAM_ENABLE_BIT | DUC_MUX_RESET_BIT | TX_IQ_DEINTERLEAVE_BIT),
            0
        );
        assert_eq!(disabled & unrelated, unrelated);

        let asserted = duc_mux_reset_asserted(disabled);
        assert_ne!(asserted & DUC_MUX_RESET_BIT, 0);
        assert_eq!(asserted & DUC_STREAM_ENABLE_BIT, 0);

        let released = duc_mux_reset_released(asserted);
        assert_eq!(released & DUC_MUX_RESET_BIT, 0);
        assert_eq!(released & unrelated, unrelated);
    }

    #[test]
    fn direct_tx_startup_policy_is_stricter_than_p2_defaults() {
        assert!(DIRECT_TX_STARTUP_SETTLE_BLOCKS > 0);
        assert!(DIRECT_TX_KEY_QUALIFICATION_PACKETS > 1);
        assert_eq!(DIRECT_TX_MIC_RECENCY_WINDOW, Duration::from_millis(150));
    }

    #[test]
    fn alex_filter_mapping_matches_p2_contract() {
        assert_eq!(alex_tx_filter_bits(7_200_000), 0x0020);
        assert_eq!(alex_tx_filter_bits(14_200_000), 0x0010);
        assert_eq!(alex_tx_filter_bits(3_900_000), 0x0040);
    }
}
