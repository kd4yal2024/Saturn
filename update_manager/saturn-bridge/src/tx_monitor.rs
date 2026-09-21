//! G2 PCB2 local TX monitor. A bounded, best-effort branch of processed TX IQ;
//! never part of RF pacing, key qualification, microphone ingress or RX audio.
//! Codec DMA and SPI run only on this worker. No RF-enable writes exist here.
use crate::radio_model::{DemodMode, RadioModel};
use crate::sync_ext::MutexExt;
use crate::xdma::{ensure_p2app_inactive, XdmaError, XdmaRegisterDevice};
use crate::xdma_rx::AlignedBuffer;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RATE: f32 = 192_000.0;
const TAPS: usize = 127;
const DMA_BYTES: usize = 1024; // 256 stereo i16 frames, 5.33 ms
const MAX_AGE: Duration = Duration::from_millis(30);
const RF_GPIO_REGISTER: u64 = 0x2014;
const AUDIO_MUTE_BIT: u32 = 1 << 4;

pub(crate) fn clamp_level(db: f64) -> f64 {
    if db.is_finite() {
        db.clamp(-60.0, -6.0)
    } else {
        -30.0
    }
}

fn voice_mode(mode: DemodMode) -> bool {
    matches!(
        mode,
        DemodMode::Usb
            | DemodMode::Lsb
            | DemodMode::DigU
            | DemodMode::DigL
            | DemodMode::Am
            | DemodMode::Sam
            | DemodMode::Fm
    )
}

fn monitor_keyed_voice(model: &RadioModel) -> bool {
    // Source-independent: both browser and SATP feed the same processed TX IQ.
    model.desired.tx_enabled && !model.desired.two_tone_enabled && voice_mode(model.desired.mode)
}

/// Demodulate the *processed* 192 kHz complex TX signal, then low-pass and
/// decimate to the codec's 48 kHz. No raw microphone loopback. SSB takes I;
/// AM uses envelope detection; FM uses a conjugate phase discriminator.
struct MonitorDsp {
    taps: [f32; TAPS],
    history: [f32; TAPS],
    position: usize,
    phase: usize,
    previous_iq: [f32; 2],
    previous_audio: f32,
    dc_output: f32,
}

impl MonitorDsp {
    fn new() -> Self {
        let mut taps = [0.0; TAPS];
        let cutoff = 7000.0 / RATE;
        for (n, tap) in taps.iter_mut().enumerate() {
            let x = n as f32 - (TAPS - 1) as f32 / 2.0;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
            };
            *tap = sinc
                * (0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / (TAPS - 1) as f32).cos());
        }
        let sum: f32 = taps.iter().sum();
        for tap in &mut taps {
            *tap /= sum;
        }
        Self {
            taps,
            history: [0.0; TAPS],
            position: 0,
            phase: 0,
            previous_iq: [0.0; 2],
            previous_audio: 0.0,
            dc_output: 0.0,
        }
    }

    fn process(&mut self, iq: &[f32], mode: DemodMode, level: f64, output: &mut VecDeque<f32>) {
        let gain = 10.0_f32.powf(clamp_level(level) as f32 / 20.0);
        for pair in iq.as_chunks::<2>().0 {
            let i = if pair[0].is_finite() { pair[0] } else { 0.0 };
            let q = if pair[1].is_finite() { pair[1] } else { 0.0 };
            let audio = match mode {
                DemodMode::Am | DemodMode::Sam => i.hypot(q),
                DemodMode::Fm => {
                    let [pi, pq] = self.previous_iq;
                    if i.hypot(q) < 1e-6 || pi.hypot(pq) < 1e-6 {
                        0.0
                    } else {
                        (q * pi - i * pq).atan2(i * pi + q * pq) * RATE
                            / (2.0 * std::f32::consts::PI * 5000.0)
                    }
                }
                DemodMode::Usb | DemodMode::Lsb | DemodMode::DigU | DemodMode::DigL => i,
                _ => 0.0,
            };
            self.previous_iq = [i, q];
            self.history[self.position] = audio;
            self.position = (self.position + 1) % TAPS;
            self.phase = (self.phase + 1) % 4;
            if self.phase == 0 {
                let filtered: f32 = self
                    .taps
                    .iter()
                    .enumerate()
                    .map(|(n, tap)| tap * self.history[(self.position + TAPS - 1 - n) % TAPS])
                    .sum();
                // Remove AM carrier/DC. Also avoids a held nonzero DAC sample.
                let dc = filtered - self.previous_audio + 0.995 * self.dc_output;
                self.previous_audio = filtered;
                self.dc_output = dc;
                output.push_back(if dc.is_finite() {
                    (dc * gain).clamp(-0.5, 0.5)
                } else {
                    0.0
                });
            }
        }
    }
}

struct Frame {
    iq: Vec<f32>,
    at: Instant,
    generation: u64,
    mode: DemodMode,
}

fn fresh_frame(gate: bool, generation: u64, frame: &Frame) -> bool {
    gate && frame.generation == generation
        && frame.at.elapsed() <= MAX_AGE
        && voice_mode(frame.mode)
}

pub(crate) struct TxMonitor {
    sender: mpsc::SyncSender<Frame>,
    enabled: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TxMonitor {
    pub(crate) fn spawn(model: Arc<Mutex<RadioModel>>) -> Result<Self, XdmaError> {
        model.lock_unpoisoned().desired.tx_monitor_available = true;
        let (sender, receiver) = mpsc::sync_channel::<Frame>(8);
        let enabled = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (en, act, gen, end) = (
            enabled.clone(),
            active.clone(),
            generation.clone(),
            shutdown.clone(),
        );
        let worker = thread::Builder::new()
            .name("saturn-mon".into())
            .spawn(move || {
                let mut codec: Option<CodecOutput> = None;
                let mut dsp = MonitorDsp::new();
                let mut pending = VecDeque::new();
                let mut playing = false;
                let mut last_mode = DemodMode::Unknown;
                let mut last_frame = Instant::now();
                let mut last_generation = gen.load(Ordering::Acquire);
                while !end.load(Ordering::Acquire) {
                    let (requested, level, keyed, available) = {
                        let m = model.lock_unpoisoned();
                        (
                            m.desired.tx_monitor_enabled,
                            m.desired.tx_monitor_level_db,
                            monitor_keyed_voice(&m),
                            m.desired.tx_monitor_available,
                        )
                    };
                    // Initialize while RX when MON is selected, not in the RF thread.
                    if requested && available && codec.is_none() {
                        match CodecOutput::open() {
                            Ok(c) => codec = Some(c),
                            Err(e) => {
                                fail_monitor(&model, &en, e);
                                continue;
                            }
                        }
                    }
                    let wanted = requested && available;
                    if en.swap(wanted, Ordering::AcqRel) && !wanted {
                        gen.fetch_add(1, Ordering::AcqRel);
                    }
                    let current_generation = gen.load(Ordering::Acquire);
                    let mut gate = wanted && keyed && act.load(Ordering::Acquire);
                    if ((!gate || last_frame.elapsed() > MAX_AGE)
                        && (playing || !pending.is_empty()))
                        || current_generation != last_generation
                    {
                        if playing {
                            if let Some(c) = codec.as_mut() {
                                if let Err(e) = c.silence() {
                                    fail_monitor(&model, &en, e);
                                    gate = false;
                                }
                            }
                        }
                        playing = false;
                        pending.clear();
                        dsp = MonitorDsp::new();
                        last_generation = current_generation;
                    }
                    match receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok(frame) if fresh_frame(gate, current_generation, &frame) => {
                            if frame.mode != last_mode {
                                dsp = MonitorDsp::new();
                                pending.clear();
                                last_mode = frame.mode;
                            }
                            last_frame = Instant::now();
                            dsp.process(&frame.iq, frame.mode, level, &mut pending);
                            if pending.len() > 2048 {
                                pending.clear(); // bound latency; never replay a backlog
                            }
                            if let Some(c) = codec.as_mut() {
                                // Bounded writes, no spin/pacing wait. A large RF batch
                                // contains multiple codec blocks and must drain fully.
                                for _ in 0..8 {
                                    if pending.len() < DMA_BYTES / 4
                                        || !act.load(Ordering::Acquire)
                                        || gen.load(Ordering::Acquire) != current_generation
                                    {
                                        break;
                                    }
                                    match c.write(&mut pending, || {
                                        act.load(Ordering::Acquire)
                                            && gen.load(Ordering::Acquire) == current_generation
                                    }) {
                                        Ok(true) => playing = true,
                                        Ok(false) => break,
                                        Err(e) => {
                                            let _ = c.silence();
                                            fail_monitor(&model, &en, e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                // Drop mutes both DAC and headphones and resets only the audio FIFO.
            })
            .map_err(|source| XdmaError::Io {
                action: "could not start TX monitor worker",
                source,
            })?;
        model_available_note();
        Ok(Self {
            sender,
            enabled,
            active,
            generation,
            shutdown,
            worker: Some(worker),
        })
    }

    pub(crate) fn push(&self, iq: &[f32], mode: DemodMode) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        self.active.store(true, Ordering::Release);
        let frame = Frame {
            iq: iq.to_vec(),
            at: Instant::now(),
            generation: self.generation.load(Ordering::Acquire),
            mode,
        };
        // Best effort only: dropping monitor audio must not delay transmit.
        let _ = self.sender.try_send(frame);
    }

    pub(crate) fn stop(&self) {
        self.active.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

fn model_available_note() {
    eprintln!("saturn-bridge: G2 PCB2 TX headphone MON worker ready (default off)");
}

impl Drop for TxMonitor {
    fn drop(&mut self) {
        self.stop();
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn fail_monitor(model: &Mutex<RadioModel>, enabled: &AtomicBool, error: XdmaError) {
    enabled.store(false, Ordering::Release);
    let mut m = model.lock_unpoisoned();
    m.desired.tx_monitor_enabled = false;
    m.desired.tx_monitor_available = false;
    eprintln!("saturn-bridge: MON disabled after headphone output fault: {error}");
}

struct CodecOutput {
    registers: XdmaRegisterDevice,
    dma: File,
    buffer: AlignedBuffer,
    unmuted: bool,
    dma_writes: u64,
    pcm_peak: u16,
    last_diag: Instant,
}

impl CodecOutput {
    fn open() -> Result<Self, XdmaError> {
        ensure_p2app_inactive()?;
        let path = std::env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));
        let registers = XdmaRegisterDevice::open_peripheral(&path)?;
        if registers.identity().pcb_version != 2 || registers.identity().is_fallback() {
            return Err(XdmaError::Incompatible(
                "MON requires primary PCB2 firmware / AIC23B codec".into(),
            ));
        }
        let path = std::env::var_os("SATURN_BRIDGE_XDMA_SPEAKER_DEVICE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/dev/xdma0_h2c_1"));
        let dma = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|source| XdmaError::Io {
                action: "could not open MON codec DMA",
                source,
            })?;
        let mut c = Self {
            registers,
            dma,
            buffer: AlignedBuffer::new(DMA_BYTES)?,
            unmuted: false,
            dma_writes: 0,
            pcm_peak: 0,
            last_diag: Instant::now(),
        };
        // This is Saturn's SetSpkrMute control. Do not assume it is isolated
        // from the chassis headphone path. Keep it asserted until playback.
        c.silence()?;
        // AIC23B, matching Saturn's CodecInitialise: 16-bit I2S slave, 48 kHz.
        // Do not reset the codec or enable analog microphone sidetone/bypass.
        for (register, value) in [(9, 0), (4, 0x14), (6, 0), (7, 2), (8, 0), (9, 1)] {
            c.codec(register, value)?;
        }
        Ok(c)
    }

    fn codec(&self, register: u32, value: u32) -> Result<(), XdmaError> {
        self.registers
            .write_register(0x14000, (register << 9) | value)?;
        thread::sleep(Duration::from_micros(10));
        Ok(())
    }

    fn hardware_mute(&self, muted: bool) -> Result<(), XdmaError> {
        self.registers.update_register(
            RF_GPIO_REGISTER,
            |value| {
                if muted {
                    value | AUDIO_MUTE_BIT
                } else {
                    value & !AUDIO_MUTE_BIT
                }
            },
            "could not update MON hardware audio mute",
        )
    }

    fn silence(&mut self) -> Result<(), XdmaError> {
        if self.unmuted {
            eprintln!(
                "saturn-bridge: MON muted dma_writes={} pcm_peak={}",
                self.dma_writes, self.pcm_peak
            );
        }
        // Attempt all mutes even when one register operation fails.
        let hardware = self.hardware_mute(true);
        let dac = self.codec(5, 8);
        let left = self.codec(2, 0);
        let right = self.codec(3, 0);
        self.unmuted = false;
        let reset = self
            .registers
            .update_register(0x7000, |v| v & !2, "could not reset MON FIFO")
            .and_then(|()| {
                self.registers
                    .update_register(0x7000, |v| v | 2, "could not release MON FIFO")
            });
        hardware.and(dac).and(left).and(right).and(reset)
    }

    fn write(
        &mut self,
        samples: &mut VecDeque<f32>,
        still_active: impl Fn() -> bool,
    ) -> Result<bool, XdmaError> {
        if !still_active() {
            self.silence()?;
            return Ok(false);
        }
        let fifo = self.registers.read_register(0x900c)?;
        if fifo & (1 << 31) != 0 {
            return Err(XdmaError::Incompatible("MON FIFO overflow".into()));
        }
        // FIFO reports 64-bit words (two stereo frames). Stay far below the
        // validated 4096-byte silence-probe capacity and never block on full.
        if fifo & 0xffff > 256 {
            return Ok(false);
        }
        let network_order = self.registers.read_register(RF_GPIO_REGISTER)? & (1 << 26) != 0;
        let bytes = self.buffer.as_mut_slice(DMA_BYTES);
        for frame in bytes.as_chunks_mut::<4>().0 {
            let sample = samples.pop_front().unwrap_or(0.0);
            let pcm = pcm16(sample);
            self.pcm_peak = self.pcm_peak.max(pcm.unsigned_abs());
            let packed = if network_order {
                pcm.to_be_bytes()
            } else {
                pcm.to_le_bytes()
            };
            frame[..2].copy_from_slice(&packed);
            frame[2..].copy_from_slice(&packed);
        }
        let written = self
            .dma
            .write_at(self.buffer.as_slice(DMA_BYTES), 0x40000)
            .map_err(|source| XdmaError::Io {
                action: "MON codec DMA write failed",
                source,
            })?;
        if written != DMA_BYTES {
            return Err(XdmaError::Incompatible(format!(
                "short MON DMA write: {written}"
            )));
        }
        self.dma_writes = self.dma_writes.saturating_add(1);
        // A release can race the DMA syscall. Never unmute stale audio after it.
        if !still_active() {
            self.silence()?;
            return Ok(false);
        }
        if !self.unmuted {
            // Fixed -12 dB analog headphone attenuation, plus digital MON level.
            self.codec(2, 0x6d)?;
            self.codec(3, 0x6d)?;
            self.codec(5, 0)?;
            self.hardware_mute(false)?;
            self.unmuted = true;
            eprintln!("saturn-bridge: MON playback enabled (DAC unmute requested; hardware audio mute released)");
        }
        // SPI writes take time too. A stop during unmute must not leave output
        // enabled while the worker waits for its next frame.
        if !still_active() {
            self.silence()?;
            return Ok(false);
        }
        if self.last_diag.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "saturn-bridge: MON output dma_writes={} pcm_peak={} fifo_words={}",
                self.dma_writes,
                self.pcm_peak,
                fifo & 0xffff
            );
            self.pcm_peak = 0;
            self.last_diag = Instant::now();
        }
        Ok(true)
    }
}

fn pcm16(sample: f32) -> i16 {
    if sample.is_finite() {
        (sample.clamp(-0.5, 0.5) * 32767.0).round() as i16
    } else {
        0
    }
}

impl Drop for CodecOutput {
    fn drop(&mut self) {
        let _ = self.silence();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ordinary files only: no XDMA devices, codec writes or RF keying in tests.
    struct MockCodec {
        codec: CodecOutput,
        directory: PathBuf,
    }

    impl MockCodec {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "saturn-mon-output-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("registers");
            let registers = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            registers.set_len(0x1400c).unwrap();
            registers
                .write_at(
                    &((1u32 << 25) | (4 << 20) | (30 << 4) | 15).to_le_bytes(),
                    0xc000,
                )
                .unwrap();
            registers
                .write_at(&((1u32 << 16) | 2).to_le_bytes(), 0xc004)
                .unwrap();
            let codec = CodecOutput {
                registers: XdmaRegisterDevice::open_peripheral(&path).unwrap(),
                dma: OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(directory.join("dma"))
                    .unwrap(),
                buffer: AlignedBuffer::new(DMA_BYTES).unwrap(),
                unmuted: false,
                dma_writes: 0,
                pcm_peak: 0,
                last_diag: Instant::now(),
            };
            Self { codec, directory }
        }
    }

    impl Drop for MockCodec {
        fn drop(&mut self) {
            // The open handles remain valid through CodecOutput::drop.
            std::fs::remove_file(self.directory.join("registers")).unwrap();
            std::fs::remove_file(self.directory.join("dma")).unwrap();
            std::fs::remove_dir(&self.directory).unwrap();
        }
    }

    #[test]
    fn hardware_audio_mute_follows_playback_without_changing_rf_bits() {
        let mut fixture = MockCodec::new();
        let c = &mut fixture.codec;
        let unrelated = 0x0f00_2d28; // includes MOX, TX enable and network byte order
        c.registers
            .write_register(RF_GPIO_REGISTER, unrelated)
            .unwrap();
        c.silence().unwrap();
        assert_eq!(
            c.registers.read_register(RF_GPIO_REGISTER).unwrap(),
            unrelated | AUDIO_MUTE_BIT
        );
        let mut samples = VecDeque::from(vec![0.1; DMA_BYTES / 4]);
        assert!(c.write(&mut samples, || true).unwrap());
        assert!(c.unmuted);
        assert_eq!(
            c.registers.read_register(RF_GPIO_REGISTER).unwrap(),
            unrelated
        );
        assert_eq!(c.registers.read_register(0x14000).unwrap(), 5 << 9);
        let mut bytes = [0; 4];
        c.dma.read_at(&mut bytes, 0x40000).unwrap();
        assert_eq!(bytes, [0x0c, 0xcd, 0x0c, 0xcd]);
        c.silence().unwrap();
        assert!(!c.unmuted);
        assert_eq!(
            c.registers.read_register(RF_GPIO_REGISTER).unwrap(),
            unrelated | AUDIO_MUTE_BIT
        );
        assert_eq!(c.registers.read_register(0x7000).unwrap() & 2, 2);
    }

    #[test]
    fn native_and_browser_mic_mon_share_the_processed_voice_gate() {
        use crate::tx_audio::TxAudioSource;
        let mut model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
        for source in [TxAudioSource::Tci, TxAudioSource::Satp] {
            model.satp.source = source;
            model.desired.mode = DemodMode::Usb;
            model.desired.tx_enabled = false;
            model.desired.two_tone_enabled = false;
            assert!(!monitor_keyed_voice(&model));
            model.desired.tx_enabled = true;
            assert!(monitor_keyed_voice(&model));
            model.desired.two_tone_enabled = true;
            assert!(!monitor_keyed_voice(&model));
            model.desired.two_tone_enabled = false;
            model.desired.mode = DemodMode::Cwu;
            assert!(!monitor_keyed_voice(&model));
        }
    }

    #[test]
    fn stop_before_dma_after_dma_or_during_unmute_leaves_hardware_muted() {
        for stop_at_check in 0..3 {
            let mut fixture = MockCodec::new();
            let c = &mut fixture.codec;
            c.silence().unwrap();
            let checks = std::cell::Cell::new(0);
            let mut samples = VecDeque::from(vec![0.1; DMA_BYTES / 4]);
            assert!(!c
                .write(&mut samples, || {
                    let check = checks.get();
                    checks.set(check + 1);
                    check < stop_at_check
                })
                .unwrap());
            assert!(!c.unmuted);
            assert_ne!(
                c.registers.read_register(RF_GPIO_REGISTER).unwrap() & AUDIO_MUTE_BIT,
                0
            );
            assert_eq!(c.dma_writes, u64::from(stop_at_check > 0));
        }
    }

    #[test]
    fn level_and_pcm_are_bounded() {
        assert_eq!(clamp_level(f64::NAN), -30.0);
        assert_eq!(clamp_level(20.0), -6.0);
        assert_eq!(clamp_level(-100.0), -60.0);
        assert_eq!(pcm16(f32::NAN), 0);
        assert_eq!(pcm16(1.0), 16384);
        assert_eq!(pcm16(-1.0), -16384);
    }
    #[test]
    fn ssb_decimates_and_monitor_level_does_not_change_input() {
        let iq: Vec<f32> = (0..19200)
            .flat_map(|n| {
                let a = 2.0 * std::f32::consts::PI * 1000.0 * n as f32 / RATE;
                [0.4 * a.cos(), 0.4 * a.sin()]
            })
            .collect();
        let original = iq.clone();
        let mut low = VecDeque::new();
        let mut high = VecDeque::new();
        MonitorDsp::new().process(&iq, DemodMode::Usb, -30.0, &mut low);
        MonitorDsp::new().process(&iq, DemodMode::Usb, -10.0, &mut high);
        assert_eq!(low.len(), 4800);
        assert_eq!(iq, original);
        let rms =
            |v: &VecDeque<f32>| (v.iter().skip(1000).map(|x| x * x).sum::<f32>() / 3800.0).sqrt();
        assert!((rms(&high) / rms(&low) - 10.0).abs() < 0.01);
        assert!(rms(&high) > 0.08);
    }
    #[test]
    fn cw_and_unknown_modes_are_not_voice_monitor() {
        assert!(!voice_mode(DemodMode::Cwu));
        assert!(!voice_mode(DemodMode::Wfm));
        assert!(voice_mode(DemodMode::Lsb));
        assert!(voice_mode(DemodMode::Fm));
    }
    #[test]
    fn closed_gate_old_generation_and_stale_audio_are_rejected() {
        let mut frame = Frame {
            iq: vec![0.1, 0.0],
            at: Instant::now(),
            generation: 7,
            mode: DemodMode::Usb,
        };
        assert!(fresh_frame(true, 7, &frame));
        assert!(!fresh_frame(false, 7, &frame));
        assert!(!fresh_frame(true, 8, &frame));
        frame.at = Instant::now() - Duration::from_secs(1);
        assert!(!fresh_frame(true, 7, &frame));
    }
    #[test]
    fn saturated_monitor_queue_does_not_wait_and_stop_invalidates_frames() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let monitor = TxMonitor {
            sender,
            enabled: Arc::new(AtomicBool::new(true)),
            active: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        monitor.push(&[0.1, 0.0], DemodMode::Usb);
        monitor.push(&[0.2, 0.0], DemodMode::Usb);
        monitor.stop();
        assert!(!monitor.active.load(Ordering::Acquire));
        let queued = receiver.try_recv().unwrap();
        assert!(!fresh_frame(
            true,
            monitor.generation.load(Ordering::Acquire),
            &queued
        ));
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn am_fm_and_chunk_boundaries_produce_finite_audio_at_48k() {
        for mode in [DemodMode::Am, DemodMode::Fm] {
            let iq: Vec<f32> = (0..19200)
                .flat_map(|n| {
                    let a = 2.0 * std::f32::consts::PI * 1000.0 * n as f32 / RATE;
                    if mode == DemodMode::Am {
                        [0.5 + 0.2 * a.sin(), 0.0]
                    } else {
                        let phase = 2.0 * a.sin();
                        [0.5 * phase.cos(), 0.5 * phase.sin()]
                    }
                })
                .collect();
            let mut whole = VecDeque::new();
            let mut chunks = VecDeque::new();
            MonitorDsp::new().process(&iq, mode, -10.0, &mut whole);
            let mut dsp = MonitorDsp::new();
            for part in iq.chunks(480) {
                dsp.process(part, mode, -10.0, &mut chunks);
            }
            assert_eq!(whole, chunks);
            assert_eq!(whole.len(), 4800);
            assert!(whole.iter().all(|v| v.is_finite() && v.abs() <= 0.5));
            assert!(whole.iter().skip(2000).any(|v| v.abs() > 0.04));
        }
    }
    #[test]
    fn anti_alias_filter_rejects_out_of_band_tone() {
        let rms = |hz: f32| {
            let iq: Vec<f32> = (0..19200)
                .flat_map(|n| {
                    [
                        (2.0 * std::f32::consts::PI * hz * n as f32 / RATE).sin(),
                        0.0,
                    ]
                })
                .collect();
            let mut out = VecDeque::new();
            MonitorDsp::new().process(&iq, DemodMode::Usb, -10.0, &mut out);
            (out.iter().skip(1000).map(|v| v * v).sum::<f32>() / 3800.0).sqrt()
        };
        assert!(rms(30_000.0) < rms(1000.0) * 0.01);
    }
}
