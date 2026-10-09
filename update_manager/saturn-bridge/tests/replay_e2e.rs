//! End-to-end replay of the unmodified Bridge, with no XDMA hardware.
//!
//! Starts the real `saturn-bridge` binary through `tools/replay/run_replay_bridge.sh`
//! (fake register file, `/dev/null` DUC, a paced synthetic receive stream with a
//! positive-frequency carrier 1,500 Hz from the center, RF TX inhibited) and checks what a
//! real client sees: raw IQ frames, WAN spectrum rows and RX audio, then the
//! Bridge's own `perf.json`.
//!
//! Ignored by default because it needs the real native WDSP (audio is silent
//! with the stub) and takes about 40 s with FFTW wisdom, about two minutes
//! without. Run it with:
//!
//!   SATURN_WDSP_DIR=<built WDSP> SATURN_BRIDGE_WDSP_FLAVOR=wdsp2-2.10 \
//!   SATURN_REPLAY_E2E_WISDOM=<wisdom file, created on first use> \
//!   cargo test --release --locked --test replay_e2e -- --ignored --nocapture
//!
//! It never touches /dev/xdma*; the runner refuses to start if they exist.

use std::f64::consts::PI;
use std::fs;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tungstenite::{Error as WsError, Message, WebSocket};

const IQ_FRAME_BYTES: usize = 102_464; // 64-byte header + 12,800 complex float pairs
const IQ_PAIRS_PER_FRAME: usize = 12_800;
const SAMPLE_RATE: f64 = 384_000.0;
const TONE_OFFSET_HZ: f64 = 1_500.0;
const STREAM_IQ: u32 = 0;
const STREAM_AUDIO: u32 = 1;
const STREAM_SPECTRUM_ROW: u32 = 16;

struct Replay {
    child: Child,
    work: PathBuf,
    port: u16,
}

impl Replay {
    fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let work = std::env::temp_dir().join(format!("saturn-replay-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/replay/run_replay_bridge.sh");
        let mut command = Command::new(runner);
        command
            .arg("--bridge")
            .arg(env!("CARGO_BIN_EXE_saturn-bridge"))
            .arg("--work")
            .arg(&work)
            .arg("--port")
            .arg(port.to_string())
            .arg("--tones")
            .arg(format!("{TONE_OFFSET_HZ}:-30"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Ok(wisdom) = std::env::var("SATURN_REPLAY_E2E_WISDOM") {
            command.arg("--wisdom").arg(wisdom);
        }
        let child = command.spawn().expect("start the replay runner");
        Self { child, work, port }
    }

    fn perf(&self) -> Option<serde_like::Perf> {
        let text = fs::read_to_string(self.work.join("perf.json")).ok()?;
        serde_like::Perf::parse(&text)
    }

    /// Waits until the Bridge reports a running receive stream.
    fn wait_ready(&mut self, limit: Duration) {
        let started = Instant::now();
        while started.elapsed() < limit {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("the Bridge exited early with {status}");
            }
            if let Some(perf) = self.perf() {
                if perf.number("ddc_s").is_some_and(|rate| rate > 800.0) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        panic!("the Bridge did not start receiving within {limit:?}");
    }

    fn stop(&mut self) -> std::process::ExitStatus {
        // SAFETY: plain signal to a child process we own.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "the Bridge ignored SIGTERM");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_dir_all(&self.work);
    }
}

/// Just enough JSON handling to read `perf.json` metrics without a new dependency.
mod serde_like {
    pub struct Perf(String);

    impl Perf {
        pub fn parse(text: &str) -> Option<Self> {
            text.contains("\"metrics\"").then(|| Self(text.to_string()))
        }

        /// A numeric top-level metric, or None if absent or null.
        pub fn number(&self, key: &str) -> Option<f64> {
            let needle = format!("\"{key}\": ");
            let at = self.0.find(&needle)? + needle.len();
            let end = self.0[at..].find(|c: char| c == ',' || c == '\n' || c == '}')?;
            self.0[at..at + end].trim().parse().ok()
        }
    }
}

type Socket = WebSocket<TcpStream>;

fn connect(port: u16) -> Socket {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the Bridge");
    let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .expect("websocket handshake");
    socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    socket
}

fn send(socket: &mut Socket, text: &str) {
    socket.send(Message::text(text)).expect("send a TCI command");
}

enum Item {
    Text(String),
    Binary(Vec<u8>),
}

fn collect(socket: &mut Socket, duration: Duration, mut on_item: impl FnMut(&mut Socket, Item)) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Text(text)) => on_item(socket, Item::Text(text.as_str().to_string())),
            Ok(Message::Binary(bytes)) => on_item(socket, Item::Binary(bytes.to_vec())),
            Ok(_) => {}
            Err(WsError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => panic!("websocket error: {error}"),
        }
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Level in dB of a complex tone at `frequency_hz` in interleaved IQ.
fn iq_tone_db(iq: &[f32], frequency_hz: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, pair) in iq.chunks_exact(2).enumerate() {
        let (sin, cos) = (-2.0 * PI * frequency_hz * n as f64 / SAMPLE_RATE).sin_cos();
        re += f64::from(pair[0]) * cos - f64::from(pair[1]) * sin;
        im += f64::from(pair[0]) * sin + f64::from(pair[1]) * cos;
    }
    let count = (iq.len() / 2) as f64;
    20.0 * ((re * re + im * im).sqrt() / count).max(1e-12).log10()
}

/// Level in dB of a real tone at `frequency_hz` (Goertzel-style DFT bin).
fn real_tone_db(samples: &[f32], sample_rate: f64, frequency_hz: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, value) in samples.iter().enumerate() {
        let (sin, cos) = (-2.0 * PI * frequency_hz * n as f64 / sample_rate).sin_cos();
        re += f64::from(*value) * cos;
        im += f64::from(*value) * sin;
    }
    20.0 * ((re * re + im * im).sqrt() * 2.0 / samples.len() as f64).max(1e-12).log10()
}

#[test]
#[ignore = "needs the real native WDSP and about a minute; see the module docs"]
fn replayed_stream_reaches_a_client_as_iq_spectrum_rows_and_audio() {
    let mut replay = Replay::start();
    replay.wait_ready(Duration::from_secs(300));
    let mut socket = connect(replay.port);

    // Greeting: the Bridge advertises the display capability and its tuning.
    let mut greeting = Vec::new();
    collect(&mut socket, Duration::from_secs(2), |_, item| {
        if let Item::Text(text) = item {
            greeting.push(text);
        }
    });
    let greeting = greeting.join("");
    assert!(greeting.contains("saturn_display_caps:spectrum_u8;"), "no spectrum capability");
    assert!(greeting.contains("ready;"), "no ready; in the greeting");

    // 1. Raw IQ: size, rate and the carrier 1,500 Hz above the center.
    send(&mut socket, "iq_samplerate:384000;iq_start:0;");
    let mut iq_frames = Vec::new();
    collect(&mut socket, Duration::from_secs(3), |_, item| {
        if let Item::Binary(bytes) = item {
            if bytes.len() >= 64 && u32_at(&bytes, 24) == STREAM_IQ {
                iq_frames.push(bytes);
            }
        }
    });
    assert!((75..=105).contains(&iq_frames.len()), "{} IQ frames in 3 s", iq_frames.len());
    assert!(iq_frames.iter().all(|frame| frame.len() == IQ_FRAME_BYTES));
    assert_eq!(u32_at(&iq_frames[0], 4), SAMPLE_RATE as u32);
    let first: Vec<f32> = iq_frames[iq_frames.len() / 2][64..]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(first.len() / 2, IQ_PAIRS_PER_FRAME);
    let at_tone = iq_tone_db(&first, TONE_OFFSET_HZ);
    let mirror = iq_tone_db(&first, -TONE_OFFSET_HZ);
    let elsewhere = iq_tone_db(&first, 60_000.0);
    println!("IQ tone +1.5 kHz {at_tone:.1} dB, mirror {mirror:.1} dB, 60 kHz {elsewhere:.1} dB");
    assert!((at_tone - -30.0).abs() < 1.0, "carrier level {at_tone} dB, expected -30 dBFS");
    assert!(at_tone - mirror > 40.0 && at_tone - elsewhere > 40.0);

    // 2. Spectrum rows: the same carrier lands in the matching FFT bin.
    send(&mut socket, "saturn_display:spectrum,4096,33;");
    let mut echo = String::new();
    let mut rows = Vec::new();
    collect(&mut socket, Duration::from_secs(3), |socket, item| match item {
        Item::Text(text) if text.contains("saturn_display:") => echo.push_str(&text),
        Item::Binary(bytes) if bytes.len() >= 64 && u32_at(&bytes, 24) == STREAM_SPECTRUM_ROW => {
            let sequence = u32_at(&bytes, 32);
            send(socket, &format!("saturn_display_ack:{sequence};"));
            rows.push(bytes);
        }
        _ => {}
    });
    assert!(echo.contains("spectrum,4096,33"), "no display echo: {echo:?}");
    assert!((60..=105).contains(&rows.len()), "{} spectrum rows in 3 s", rows.len());
    let row = &rows[rows.len() / 2];
    let fft = u32_at(row, 12) as usize;
    assert_eq!(fft, 4096);
    let (offset, step) = (f32_at(row, 44), f32_at(row, 48));
    let bins: Vec<f32> = row[64..64 + fft].iter().map(|&b| offset + f32::from(b) * step).collect();
    let peak = bins.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    let expected = fft / 2 + (TONE_OFFSET_HZ / (SAMPLE_RATE / fft as f64)).round() as usize;
    println!("spectrum peak bin {peak}, expected {expected}");
    assert!(peak.abs_diff(expected) <= 2, "peak at bin {peak}, expected {expected}");
    let sequences: Vec<u32> = rows.iter().map(|r| u32_at(r, 32)).collect();
    assert!(sequences.windows(2).all(|w| w[1] == w[0] + 1), "spectrum row sequence gap");

    // 3. RX audio. The fixture is a positive-frequency complex carrier. In this
    //    build it is shown above the center (steps 1 and 2) and WDSP demodulates
    //    it in LSB, not USB, so the mode is set explicitly. That describes the
    //    synthetic signal's sign convention only; which sideband real hardware
    //    gives a station above the carrier is not established by this test.
    send(&mut socket, "modulation:0,LSB;");
    std::thread::sleep(Duration::from_millis(500));
    send(
        &mut socket,
        "audio_stream_samples:2048;audio_stream_channels:2;audio_stream_sample_type:float32;\
         audio_samplerate:48000;audio_start:0;rx_volume:0,0,-10.0;",
    );
    let mut audio: Vec<(u32, Vec<f32>)> = Vec::new();
    collect(&mut socket, Duration::from_secs(4), |socket, item| match item {
        Item::Binary(bytes) if bytes.len() >= 64 && u32_at(&bytes, 24) == STREAM_AUDIO => {
            let samples: Vec<f32> = bytes[64..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            audio.push((u32_at(&bytes, 32), samples));
        }
        Item::Binary(bytes) if bytes.len() >= 64 && u32_at(&bytes, 24) == STREAM_SPECTRUM_ROW => {
            send(socket, &format!("saturn_display_ack:{};", u32_at(&bytes, 32)));
        }
        _ => {}
    });
    assert!(audio.len() >= 120, "{} audio frames in 4 s", audio.len());
    let sequences: Vec<u32> = audio.iter().map(|(sequence, _)| *sequence).collect();
    assert!(sequences.windows(2).all(|w| w[1] == w[0] + 1), "audio sequence gap");
    // Left channel of a settled stretch, well after the AGC and filters start.
    let left: Vec<f32> = audio[audio.len() / 2..]
        .iter()
        .flat_map(|(_, samples)| samples.chunks_exact(2).map(|pair| pair[0]))
        .take(16_384)
        .collect();
    assert!(left.len() >= 8_192);
    let tone = real_tone_db(&left, 48_000.0, TONE_OFFSET_HZ);
    let off_a = real_tone_db(&left, 48_000.0, 700.0);
    let off_b = real_tone_db(&left, 48_000.0, 2_600.0);
    println!("audio 1.5 kHz {tone:.1} dB, 700 Hz {off_a:.1} dB, 2.6 kHz {off_b:.1} dB");
    assert!(tone > -30.0, "no audible carrier: {tone} dB");
    assert!(tone - off_a > 40.0 && tone - off_b > 40.0, "audio is not a clean 1.5 kHz tone");

    // The Bridge's own accounting: nothing lost, and the Stage A counters exist.
    std::thread::sleep(Duration::from_millis(1200));
    let perf = replay.perf().expect("perf.json");
    for loss in ["header_errors", "header_resync", "host_buffer_drops", "host_discontinuities", "rx_fifo_faults"] {
        assert_eq!(perf.number(loss), Some(0.0), "{loss} is not zero");
    }
    let rate = perf.number("ddc_s").unwrap();
    assert!((835.0..=850.0).contains(&rate), "DDC read rate {rate}/s");
    for guard in ["command_arm_cancelled", "command_mic_cancelled", "non_finite_controls_rejected"] {
        assert!(perf.number(guard).is_some(), "perf.json lacks {guard}");
    }

    // A SIGTERM is a clean, receive-safe shutdown.
    drop(socket);
    let status = replay.stop();
    assert!(status.success() || status.code().is_none(), "exit status {status}");
}
