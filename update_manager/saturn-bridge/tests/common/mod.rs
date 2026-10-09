//! Shared by the replay integration tests: start the unmodified Bridge through
//! tools/replay/run_replay_bridge.sh and talk to it as a TCI client.
#![allow(dead_code)]

use std::f64::consts::PI;
use std::fs;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tungstenite::{Error as WsError, Message, WebSocket};

pub const IQ_FRAME_BYTES: usize = 102_464; // 64-byte header + 12,800 complex float pairs
pub const IQ_PAIRS_PER_FRAME: usize = 12_800;
pub const SAMPLE_RATE: f64 = 384_000.0;
pub const TONE_OFFSET_HZ: f64 = 1_500.0;
pub const STREAM_IQ: u32 = 0;
pub const STREAM_AUDIO: u32 = 1;
pub const STREAM_SPECTRUM_ROW: u32 = 16;

pub struct Replay {
    pub child: Child,
    pub work: PathBuf,
    pub port: u16,
}

impl Replay {
    pub fn start() -> Self {
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

    pub fn perf(&self) -> Option<serde_like::Perf> {
        let text = fs::read_to_string(self.work.join("perf.json")).ok()?;
        serde_like::Perf::parse(&text)
    }

    /// Waits until the Bridge reports a running receive stream.
    pub fn wait_ready(&mut self, limit: Duration) {
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

    pub fn stop(&mut self) -> std::process::ExitStatus {
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
pub mod serde_like {
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

pub type Socket = WebSocket<TcpStream>;

pub fn connect(port: u16) -> Socket {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the Bridge");
    let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .expect("websocket handshake");
    socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    socket
}

pub fn send(socket: &mut Socket, text: &str) {
    socket.send(Message::text(text)).expect("send a TCI command");
}

pub enum Item {
    Text(String),
    Binary(Vec<u8>),
}

pub fn collect(socket: &mut Socket, duration: Duration, mut on_item: impl FnMut(&mut Socket, Item)) {
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

pub fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Level in dB of a complex tone at `frequency_hz` in interleaved IQ.
pub fn iq_tone_db(iq: &[f32], frequency_hz: f64) -> f64 {
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
pub fn real_tone_db(samples: &[f32], sample_rate: f64, frequency_hz: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, value) in samples.iter().enumerate() {
        let (sin, cos) = (-2.0 * PI * frequency_hz * n as f64 / sample_rate).sin_cos();
        re += f64::from(*value) * cos;
        im += f64::from(*value) * sin;
    }
    20.0 * ((re * re + im * im).sqrt() * 2.0 / samples.len() as f64).max(1e-12).log10()
}

