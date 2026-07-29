//! Phase 3 direct Saturn/XDMA codec-audio probe.
//!
//! This remains an isolated, one-shot validation path. It captures microphone
//! samples from C2H1 and writes only silence to the speaker H2C1 stream while
//! the physical speaker mute bit is asserted. It contains no DUC/TX path.

use crate::xdma::{ensure_p2app_inactive, SaturnIdentity, XdmaError, XdmaRegisterDevice};
use crate::xdma_rx::AlignedBuffer;
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_MIC_DEVICE: &str = "/dev/xdma0_c2h_1";
const DEFAULT_SPEAKER_DEVICE: &str = "/dev/xdma0_h2c_1";
const DEFAULT_CAPTURE_DURATION_MS: u64 = 2_000;
const MIN_CAPTURE_DURATION_MS: u64 = 250;
const MAX_CAPTURE_DURATION_MS: u64 = 10_000;

const AUDIO_DMA_AXI_OFFSET: u64 = 0x40000;
const FIFO_RESET_REGISTER: u64 = 0x7000;
const FIFO_MONITOR_MIC_REGISTER: u64 = 0x9008;
const FIFO_MONITOR_SPEAKER_REGISTER: u64 = 0x900c;
const RF_GPIO_REGISTER: u64 = 0x2014;
const MIC_FIFO_RESET_BIT: u32 = 1 << 0;
const SPEAKER_FIFO_RESET_BIT: u32 = 1 << 1;
const SPEAKER_MUTE_BIT: u32 = 1 << 4;
const NETWORK_BYTE_ORDER_BIT: u32 = 1 << 26;

const MIC_SAMPLE_RATE_HZ: u64 = 48_000;
const MIC_DMA_BYTES: usize = 128;
const MIC_SAMPLES_PER_DMA: usize = 64;
const MIC_FIFO_WORDS_PER_DMA: usize = 16;
const MIC_FIFO_GUARD_WORDS: usize = 16;
const SPEAKER_SAMPLE_RATE_HZ: u64 = 48_000;
const SPEAKER_DMA_BYTES: usize = 4096;
const SPEAKER_FRAME_BYTES: usize = 256;
const SPEAKER_SAMPLE_PAIRS_PER_FRAME: usize = 64;
const DMA_ALIGNMENT: usize = 4096;
const FIFO_POLL_INTERVAL: Duration = Duration::from_micros(250);

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

#[derive(Clone, Debug, Default)]
struct MicStats {
    dma_reads: u64,
    dma_bytes: u64,
    samples: u64,
    fifo_hwm: usize,
    fifo_overflows: u64,
    fifo_over_threshold: u64,
    fifo_underflows: u64,
    fifo_startup_underflow: bool,
    power_sum: f64,
    peak: f32,
    elapsed: Duration,
}

impl MicStats {
    fn observe_fifo(&mut self, snapshot: FifoSnapshot) {
        self.fifo_hwm = self.fifo_hwm.max(snapshot.depth_words);
        self.fifo_overflows += u64::from(snapshot.overflow);
        self.fifo_over_threshold += u64::from(snapshot.over_threshold);
        self.fifo_underflows += u64::from(snapshot.underflow);
    }

    fn rms_dbfs(&self) -> f32 {
        if self.samples == 0 {
            return -200.0;
        }
        let mean_power = (self.power_sum / self.samples as f64).max(1.0e-20);
        (10.0 * mean_power.log10()) as f32
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SpeakerStats {
    dma_writes: u64,
    dma_bytes: u64,
    frames: u64,
    sample_pairs: u64,
    fifo_depth_after_write: usize,
    fifo_depth_after_settle: usize,
    fifo_overflows: u64,
    fifo_over_threshold: u64,
    fifo_underflows: u64,
    fifo_startup_underflow: bool,
    fifo_prefill_underflow: bool,
}

struct AudioDmaSession<'a> {
    registers: &'a mut XdmaRegisterDevice,
    mic_dma: File,
    speaker_dma: File,
    network_byte_order: bool,
    stopped: bool,
}

impl<'a> AudioDmaSession<'a> {
    fn start(
        registers: &'a mut XdmaRegisterDevice,
        mic_path: &Path,
        speaker_path: &Path,
    ) -> Result<Self, XdmaError> {
        registers.update_register(
            RF_GPIO_REGISTER,
            |value| value | SPEAKER_MUTE_BIT,
            "could not assert hardware speaker mute before opening codec DMA",
        )?;
        let mic_dma = OpenOptions::new()
            .read(true)
            .open(mic_path)
            .map_err(|source| XdmaError::Io {
                action: "could not open XDMA microphone receive device",
                source,
            })?;
        let speaker_dma = OpenOptions::new()
            .write(true)
            .open(speaker_path)
            .map_err(|source| XdmaError::Io {
                action: "could not open XDMA speaker transmit device",
                source,
            })?;
        let network_byte_order =
            registers.read_register(RF_GPIO_REGISTER)? & NETWORK_BYTE_ORDER_BIT != 0;
        let mut session = Self {
            registers,
            mic_dma,
            speaker_dma,
            network_byte_order,
            stopped: false,
        };
        if let Err(error) = session.configure() {
            let _ = session.stop();
            return Err(error);
        }
        Ok(session)
    }

    fn configure(&mut self) -> Result<(), XdmaError> {
        self.mute_speaker()?;
        self.reset_fifo(MIC_FIFO_RESET_BIT)?;
        self.reset_fifo(SPEAKER_FIFO_RESET_BIT)?;
        self.registers.read_register(FIFO_MONITOR_MIC_REGISTER)?;
        self.registers
            .read_register(FIFO_MONITOR_SPEAKER_REGISTER)?;
        Ok(())
    }

    fn capture_microphone(&mut self, duration: Duration) -> Result<MicStats, XdmaError> {
        let mut buffer = AlignedBuffer::new(DMA_ALIGNMENT)?;
        let mut stats = MicStats::default();
        thread::sleep(Duration::from_millis(1));
        let startup =
            FifoSnapshot::decode(self.registers.read_register(FIFO_MONITOR_MIC_REGISTER)?);
        stats.fifo_hwm = startup.depth_words;
        stats.fifo_overflows += u64::from(startup.overflow);
        stats.fifo_over_threshold += u64::from(startup.over_threshold);
        stats.fifo_startup_underflow = startup.underflow;

        let started = Instant::now();
        while started.elapsed() < duration {
            let fifo =
                FifoSnapshot::decode(self.registers.read_register(FIFO_MONITOR_MIC_REGISTER)?);
            stats.observe_fifo(fifo);
            if fifo.depth_words < MIC_FIFO_WORDS_PER_DMA + MIC_FIFO_GUARD_WORDS {
                thread::sleep(FIFO_POLL_INTERVAL);
                continue;
            }

            let target = buffer.as_mut_slice(MIC_DMA_BYTES);
            let read = self
                .mic_dma
                .read_at(target, AUDIO_DMA_AXI_OFFSET)
                .map_err(|source| XdmaError::Io {
                    action: "could not read XDMA microphone stream",
                    source,
                })?;
            if read != MIC_DMA_BYTES {
                return Err(short_io_error(
                    "XDMA microphone stream returned a short read",
                    read,
                    MIC_DMA_BYTES,
                ));
            }
            stats.dma_reads += 1;
            stats.dma_bytes += read as u64;
            debug_assert_eq!(read / 2, MIC_SAMPLES_PER_DMA);
            observe_mic_samples(&target[..read], self.network_byte_order, &mut stats);
        }

        if stats.samples == 0 {
            return Err(XdmaError::Incompatible(
                "direct XDMA microphone capture completed without samples".into(),
            ));
        }
        stats.elapsed = started.elapsed();
        Ok(stats)
    }

    fn probe_speaker_silence(&mut self) -> Result<SpeakerStats, XdmaError> {
        self.mute_speaker()?;
        self.reset_fifo(SPEAKER_FIFO_RESET_BIT)?;
        self.registers
            .read_register(FIFO_MONITOR_SPEAKER_REGISTER)?;
        thread::sleep(Duration::from_millis(1));
        let startup = FifoSnapshot::decode(
            self.registers
                .read_register(FIFO_MONITOR_SPEAKER_REGISTER)?,
        );

        let silence = AlignedBuffer::new(SPEAKER_DMA_BYTES)?;
        if self.registers.read_register(RF_GPIO_REGISTER)? & SPEAKER_MUTE_BIT == 0 {
            return Err(XdmaError::Incompatible(
                "hardware speaker mute did not remain asserted".into(),
            ));
        }
        let written = self
            .speaker_dma
            .write_at(silence.as_slice(SPEAKER_DMA_BYTES), AUDIO_DMA_AXI_OFFSET)
            .map_err(|source| XdmaError::Io {
                action: "could not write XDMA speaker silence",
                source,
            })?;
        if written != SPEAKER_DMA_BYTES {
            return Err(short_io_error(
                "XDMA speaker stream returned a short write",
                written,
                SPEAKER_DMA_BYTES,
            ));
        }

        let after_write = FifoSnapshot::decode(
            self.registers
                .read_register(FIFO_MONITOR_SPEAKER_REGISTER)?,
        );
        thread::sleep(Duration::from_millis(2));
        let after_settle = FifoSnapshot::decode(
            self.registers
                .read_register(FIFO_MONITOR_SPEAKER_REGISTER)?,
        );
        if after_write.depth_words == 0 {
            return Err(XdmaError::Incompatible(
                "speaker DMA write completed but FIFO occupancy did not increase".into(),
            ));
        }
        if after_settle.depth_words > after_write.depth_words {
            return Err(XdmaError::Incompatible(format!(
                "speaker FIFO occupancy increased unexpectedly after silence write: {} -> {}",
                after_write.depth_words, after_settle.depth_words
            )));
        }

        Ok(SpeakerStats {
            dma_writes: 1,
            dma_bytes: SPEAKER_DMA_BYTES as u64,
            frames: (SPEAKER_DMA_BYTES / SPEAKER_FRAME_BYTES) as u64,
            sample_pairs: ((SPEAKER_DMA_BYTES / SPEAKER_FRAME_BYTES)
                * SPEAKER_SAMPLE_PAIRS_PER_FRAME) as u64,
            fifo_depth_after_write: after_write.depth_words,
            fifo_depth_after_settle: after_settle.depth_words,
            fifo_overflows: u64::from(startup.overflow)
                + u64::from(after_write.overflow)
                + u64::from(after_settle.overflow),
            fifo_over_threshold: u64::from(startup.over_threshold)
                + u64::from(after_write.over_threshold)
                + u64::from(after_settle.over_threshold),
            fifo_underflows: u64::from(after_settle.underflow),
            fifo_startup_underflow: startup.underflow,
            fifo_prefill_underflow: after_write.underflow,
        })
    }

    fn mute_speaker(&self) -> Result<(), XdmaError> {
        self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| value | SPEAKER_MUTE_BIT,
            "could not assert hardware speaker mute",
        )
    }

    fn reset_fifo(&self, bit: u32) -> Result<(), XdmaError> {
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value & !bit,
            "could not assert codec FIFO reset",
        )?;
        self.registers.update_register(
            FIFO_RESET_REGISTER,
            |value| value | bit,
            "could not release codec FIFO reset",
        )
    }

    fn stop(&mut self) -> Result<(), XdmaError> {
        if self.stopped {
            return Ok(());
        }
        let mute = self.mute_speaker();
        let reset_mic = self.reset_fifo(MIC_FIFO_RESET_BIT);
        let reset_speaker = self.reset_fifo(SPEAKER_FIFO_RESET_BIT);
        let result = mute.and(reset_mic).and(reset_speaker);
        self.stopped = result.is_ok();
        result
    }
}

impl Drop for AudioDmaSession<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("saturn-bridge: XDMA audio emergency cleanup failed: {error}");
        }
    }
}

pub fn run_phase3_audio_probe() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let duration_ms = parse_duration_ms()?;
    let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
    let mic_path = env::var_os("SATURN_BRIDGE_XDMA_MIC_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MIC_DEVICE));
    let speaker_path = env::var_os("SATURN_BRIDGE_XDMA_SPEAKER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SPEAKER_DEVICE));

    let mut registers = XdmaRegisterDevice::open(&register_path)?;
    let identity: SaturnIdentity = registers.identity().clone();
    let mut session = AudioDmaSession::start(&mut registers, &mic_path, &speaker_path)?;
    let mic_result = session.capture_microphone(Duration::from_millis(duration_ms));
    let speaker_result = mic_result.and_then(|mic| {
        session
            .probe_speaker_silence()
            .map(|speaker| (mic, speaker))
    });
    let stop = session.stop();
    let network_byte_order = session.network_byte_order;
    drop(session);
    let (mic, speaker) = speaker_result?;
    stop?;
    registers.close_safely()?;
    let elapsed = mic.elapsed.as_secs_f64().max(0.001);

    println!(
        "saturn-bridge: XDMA Phase 3 audio probe passed product={} pcb={} firmware={}.{} mic_device={} mic_rate={}Hz mic_byte_order={} duration_ms={} mic_samples={} mic_sample_rate={:.1}/s mic_dma_reads={} mic_dma_bytes={} mic_fifo_hwm={} mic_fifo_overflow={} mic_fifo_threshold={} mic_fifo_startup_underflow={} mic_fifo_underflow={} mic_rms={:.1}dBFS mic_peak={:.4}",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        mic_path.display(),
        MIC_SAMPLE_RATE_HZ,
        if network_byte_order { "network" } else { "local" },
        duration_ms,
        mic.samples,
        mic.samples as f64 / elapsed,
        mic.dma_reads,
        mic.dma_bytes,
        mic.fifo_hwm,
        mic.fifo_overflows,
        mic.fifo_over_threshold,
        u8::from(mic.fifo_startup_underflow),
        mic.fifo_underflows,
        mic.rms_dbfs(),
        mic.peak,
    );
    println!(
        "saturn-bridge: XDMA Phase 3 speaker silence passed speaker_device={} rate={}Hz dma_writes={} dma_bytes={} frames={} sample_pairs={} fifo_after_write={} fifo_after_settle={} fifo_overflow={} fifo_threshold={} fifo_startup_underflow={} fifo_prefill_underflow={} fifo_underflow={} speaker_muted=1",
        speaker_path.display(),
        SPEAKER_SAMPLE_RATE_HZ,
        speaker.dma_writes,
        speaker.dma_bytes,
        speaker.frames,
        speaker.sample_pairs,
        speaker.fifo_depth_after_write,
        speaker.fifo_depth_after_settle,
        speaker.fifo_overflows,
        speaker.fifo_over_threshold,
        u8::from(speaker.fifo_startup_underflow),
        u8::from(speaker.fifo_prefill_underflow),
        speaker.fifo_underflows,
    );
    println!(
        "saturn-bridge: XDMA Phase 3 cleanup completed; microphone and speaker FIFOs reset, hardware speaker mute asserted, and RF remains receive-safe"
    );
    Ok(())
}

fn observe_mic_samples(bytes: &[u8], network_byte_order: bool, stats: &mut MicStats) {
    for sample in bytes.chunks_exact(2) {
        let value = if network_byte_order {
            i16::from_be_bytes([sample[0], sample[1]])
        } else {
            i16::from_le_bytes([sample[0], sample[1]])
        };
        let normalized = value as f32 / 32768.0;
        stats.power_sum += (normalized * normalized) as f64;
        stats.peak = stats.peak.max(normalized.abs());
        stats.samples += 1;
    }
}

fn parse_duration_ms() -> Result<u64, XdmaError> {
    let duration_ms = match env::var("SATURN_BRIDGE_XDMA_AUDIO_DURATION_MS") {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            XdmaError::Incompatible(
                "SATURN_BRIDGE_XDMA_AUDIO_DURATION_MS must be an unsigned integer".into(),
            )
        })?,
        Err(env::VarError::NotPresent) => DEFAULT_CAPTURE_DURATION_MS,
        Err(error) => {
            return Err(XdmaError::Incompatible(format!(
                "could not read SATURN_BRIDGE_XDMA_AUDIO_DURATION_MS: {error}"
            )));
        }
    };
    if !(MIN_CAPTURE_DURATION_MS..=MAX_CAPTURE_DURATION_MS).contains(&duration_ms) {
        return Err(XdmaError::Incompatible(format!(
            "direct XDMA audio duration {duration_ms} ms is outside the supported {MIN_CAPTURE_DURATION_MS}..={MAX_CAPTURE_DURATION_MS} ms range"
        )));
    }
    Ok(duration_ms)
}

fn short_io_error(action: &'static str, actual: usize, expected: usize) -> XdmaError {
    XdmaError::Io {
        action,
        source: io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("transferred {actual} of {expected} bytes"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_snapshot_decodes_codec_flags_and_depth() {
        let snapshot = FifoSnapshot::decode(0xe000_002a);
        assert_eq!(snapshot.depth_words, 42);
        assert!(snapshot.overflow);
        assert!(snapshot.over_threshold);
        assert!(snapshot.underflow);
    }

    #[test]
    fn microphone_network_order_samples_are_normalized() {
        let bytes = [0x40, 0x00, 0xc0, 0x00, 0x00, 0x00];
        let mut stats = MicStats::default();
        observe_mic_samples(&bytes, true, &mut stats);
        assert_eq!(stats.samples, 3);
        assert!((stats.peak - 0.5).abs() < f32::EPSILON);
        assert!((stats.rms_dbfs() + 7.782).abs() < 0.01);
    }

    #[test]
    fn microphone_local_order_samples_are_normalized() {
        let bytes = [0x00, 0x40, 0x00, 0xc0];
        let mut stats = MicStats::default();
        observe_mic_samples(&bytes, false, &mut stats);
        assert_eq!(stats.samples, 2);
        assert!((stats.peak - 0.5).abs() < f32::EPSILON);
        assert!((stats.rms_dbfs() + 6.021).abs() < 0.01);
    }

    #[test]
    fn audio_dma_geometry_matches_saturn_protocol_frames() {
        assert_eq!(MIC_DMA_BYTES / 2, MIC_SAMPLES_PER_DMA);
        assert_eq!(MIC_DMA_BYTES / 8, MIC_FIFO_WORDS_PER_DMA);
        assert_eq!(
            MIC_FIFO_WORDS_PER_DMA + MIC_FIFO_GUARD_WORDS,
            2 * MIC_FIFO_WORDS_PER_DMA
        );
        assert_eq!(SPEAKER_DMA_BYTES % SPEAKER_FRAME_BYTES, 0);
        assert_eq!(SPEAKER_FRAME_BYTES / 4, SPEAKER_SAMPLE_PAIRS_PER_FRAME);
    }
}
