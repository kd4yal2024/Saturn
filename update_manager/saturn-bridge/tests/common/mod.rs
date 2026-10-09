//! Shared by the replay integration tests: start the unmodified Bridge through
//! tools/replay/run_replay_bridge.sh and talk to it as a TCI client.
#![allow(dead_code)]

use std::f64::consts::PI;
use std::fs;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tungstenite::{Error as WsError, Message, WebSocket};

pub const IQ_FRAME_BYTES: usize = 102_464; // 64-byte header + 12,800 complex float pairs
pub const IQ_PAIRS_PER_FRAME: usize = 12_800;
pub const SAMPLE_RATE: f64 = 384_000.0;
pub const TONE_OFFSET_HZ: f64 = 1_500.0;
pub const STREAM_IQ: u32 = 0;
pub const STREAM_AUDIO: u32 = 1;
pub const STREAM_SPECTRUM_ROW: u32 = 16;

static RUN_COUNTER: AtomicU32 = AtomicU32::new(0);

/// How the Bridge's life ended, as the harness saw it.
#[derive(Debug)]
pub enum Stop {
    /// The child had already exited when the harness went to stop it.
    ExitedBeforeStop(ExitStatus),
    /// The harness sent SIGTERM to a running child, which then exited.
    Stopped(ExitStatus),
}

fn describe(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit status {code}"),
        (None, Some(signal)) if status.core_dumped() => format!("signal {signal} (core dumped)"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => format!("{status}"),
    }
}

/// `Ok` only for a requested, clean shutdown: the harness sent SIGTERM to a
/// Bridge that was still running, and it exited with status 0. The Bridge
/// handles SIGTERM and exits 0 (measured 2026-10-09 on the replay), so a death
/// by SIGTERM, SIGSEGV, SIGABRT or SIGKILL, a nonzero exit, or an exit before
/// the harness asked are all failures.
pub fn check_stop(stop: &Stop) -> Result<(), String> {
    match stop {
        Stop::ExitedBeforeStop(status) => Err(format!(
            "the Bridge had already exited ({}) before the harness stopped it",
            describe(*status)
        )),
        Stop::Stopped(status) if status.success() => Ok(()),
        Stop::Stopped(status) => Err(format!(
            "the Bridge did not shut down cleanly after SIGTERM: {}",
            describe(*status)
        )),
    }
}

/// A child process with its output going to files and a run directory that
/// survives a failure.
///
/// stdout and stderr are redirected to files, not pipes, so a verbose child can
/// never block on a full pipe while it is being measured. The run directory is
/// unique to this run, holds the logs and the Bridge's work directory (perf.json,
/// replay statistics), and is deleted only if the run passed and nobody asked
/// to keep it: it is kept when the thread is panicking, after `keep()`, and when
/// SATURN_REPLAY_KEEP=1.
pub struct Replay {
    pub child: Child,
    /// Handed to the runner; must start empty.
    pub work: PathBuf,
    pub port: u16,
    root: PathBuf,
    keep: bool,
}

impl Replay {
    pub fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let root = Self::new_root();
        let work = root.join("work");
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
            .arg(format!("{TONE_OFFSET_HZ}:-30"));
        if let Ok(wisdom) = std::env::var("SATURN_REPLAY_E2E_WISDOM") {
            command.arg("--wisdom").arg(wisdom);
        }
        Self::spawn(command, root, port)
    }

    /// Runs any command under the same logging and cleanup rules (used by the
    /// harness's own tests with dummy children).
    pub fn spawn_command(command: Command) -> Self {
        Self::spawn(command, Self::new_root(), 0)
    }

    fn new_root() -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saturn-replay-{}-{}-{nanos}",
            std::process::id(),
            RUN_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create a unique run directory");
        root
    }

    fn spawn(mut command: Command, root: PathBuf, port: u16) -> Self {
        let logs = root.join("logs");
        fs::create_dir(&logs).expect("create the log directory");
        let open = |name: &str| fs::File::create(logs.join(name)).expect("create a log file");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(open("stdout.log")))
            .stderr(Stdio::from(open("stderr.log")));
        let child = command.spawn().expect("start the child process");
        let keep = std::env::var("SATURN_REPLAY_KEEP").is_ok_and(|value| value == "1");
        Self { child, work: root.join("work"), port, root, keep }
    }

    /// Keep the run directory (logs, perf.json, measurement records) after the
    /// test, pass or fail.
    pub fn keep(&mut self) {
        self.keep = true;
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn log_path(&self, name: &str) -> PathBuf {
        self.root.join("logs").join(name)
    }

    pub fn log_text(&self, name: &str) -> String {
        fs::read_to_string(self.log_path(name)).unwrap_or_default()
    }

    fn log_tail(&self, name: &str, lines: usize) -> String {
        let text = self.log_text(name);
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
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
                self.keep = true;
                panic!(
                    "the Bridge exited early ({}); evidence kept in {}\n--- stderr tail ---\n{}",
                    describe(status),
                    self.root.display(),
                    self.log_tail("stderr.log", 20)
                );
            }
            if let Some(perf) = self.perf() {
                if perf.number("ddc_s").is_some_and(|rate| rate > 800.0) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        self.keep = true;
        panic!(
            "the Bridge did not start receiving within {limit:?}; evidence kept in {}\n--- stderr tail ---\n{}",
            self.root.display(),
            self.log_tail("stderr.log", 20)
        );
    }

    /// Waits for the child to exit by itself, up to `limit`.
    pub fn wait_for_exit(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until a log contains `needle`.
    pub fn wait_for_log(&self, name: &str, needle: &str, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.log_text(name).contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Stops the child with SIGTERM, unless it has already exited, and reports
    /// which of the two happened. Does not judge the outcome; see `check_stop`.
    pub fn stop(&mut self) -> Stop {
        if let Some(status) = self.child.try_wait().unwrap() {
            return Stop::ExitedBeforeStop(status);
        }
        // SAFETY: plain signal to a child process we own.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Stop::Stopped(status);
            }
            if Instant::now() >= deadline {
                self.keep = true;
                panic!("the Bridge ignored SIGTERM; evidence kept in {}", self.root.display());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Stops the child and fails the test, keeping the evidence, unless the
    /// shutdown was clean.
    pub fn stop_clean(&mut self) {
        let stop = self.stop();
        if let Err(why) = check_stop(&stop) {
            self.keep = true;
            panic!(
                "{why}; evidence kept in {}\n--- stderr tail ---\n{}",
                self.root.display(),
                self.log_tail("stderr.log", 20)
            );
        }
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if self.keep || std::thread::panicking() {
            eprintln!("replay evidence kept in {}", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
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
