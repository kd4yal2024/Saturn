//! Transport-layer screen of the display transports, on the replayed stream.
//!
//! Same synthetic input in every arm, one prompt scripted client, the real
//! Bridge: A1 raw IQ + audio, B spectrum rows + audio, A2 raw IQ + audio again
//! (the run-to-run spread), C audio only. For each arm it prints what reached
//! the client (bytes and frames per class, audio inter-arrival times and
//! sequence gaps), the Bridge's own queue and drop counters, and the Bridge's
//! CPU time.
//!
//! This is the cheap screen of docs Stage B layer 1. It says nothing about
//! listening smoothness: a scripted client has no decoder or AudioWorklet, and
//! this is one x86 machine, not a G2. It is ignored by default; run it like
//! replay_e2e (real WDSP, `--ignored --nocapture`). SATURN_SCREEN_SECONDS sets
//! the measured window per arm (default 20).

mod common;

use std::fmt::Write as _;
use std::fs;
use std::time::{Duration, Instant};

use common::*;

const AUDIO_START: &str = "audio_stream_samples:2048;audio_stream_channels:2;\
    audio_stream_sample_type:float32;audio_samplerate:48000;audio_start:0;rx_volume:0,0,-10.0;";
/// 2,048 stereo floats at 48 kHz.
const NOMINAL_AUDIO_PERIOD_MS: f64 = 1024.0 / 48.0;

#[derive(Clone, Copy, PartialEq)]
enum Display {
    RawIq,
    Rows,
    None,
}

#[derive(Default)]
struct Arm {
    name: &'static str,
    seconds: f64,
    iq_bytes: u64,
    audio_bytes: u64,
    row_bytes: u64,
    iq_frames: u64,
    audio_frames: u64,
    row_frames: u64,
    audio_gaps: u64,
    audio_intervals_ms: Vec<f64>,
    bridge_cpu_percent: f64,
    perf: Vec<(&'static str, Option<f64>)>,
}

fn cpu_seconds(pid: u32) -> f64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read /proc stat");
    let rest = &stat[stat.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the command name: state is field 3, utime 14, stime 15.
    let ticks: f64 = fields[11].parse::<f64>().unwrap() + fields[12].parse::<f64>().unwrap();
    // SAFETY: sysconf with a valid name has no preconditions.
    ticks / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len()) - 1]
}

fn run_arm(replay: &Replay, name: &'static str, display: Display, with_audio: bool, window: Duration) -> Arm {
    let mut socket = connect(replay.port);
    let mut sink = |_: &mut Socket, _: Item| {};
    collect(&mut socket, Duration::from_secs(1), &mut sink);
    match display {
        Display::RawIq => send(&mut socket, "iq_samplerate:384000;iq_start:0;"),
        Display::Rows => {
            send(&mut socket, "saturn_display:spectrum,4096,33;");
            collect(&mut socket, Duration::from_millis(300), &mut sink);
            send(&mut socket, "iq_start:0;");
        }
        Display::None => {}
    }
    if with_audio {
        send(&mut socket, AUDIO_START);
    }
    // Warm up, acking rows so credit never stalls, then measure.
    let ack = |socket: &mut Socket, item: Item| {
        if let Item::Binary(bytes) = item {
            if bytes.len() >= 64 && u32_at(&bytes, 24) == STREAM_SPECTRUM_ROW {
                send(socket, &format!("saturn_display_ack:{};", u32_at(&bytes, 32)));
            }
        }
    };
    collect(&mut socket, Duration::from_secs(5), ack);

    let mut arm = Arm { name, ..Arm::default() };
    let pid = replay.child.id();
    let (cpu_before, started) = (cpu_seconds(pid), Instant::now());
    let mut last_audio_at: Option<Instant> = None;
    let mut last_sequence: Option<u32> = None;
    collect(&mut socket, window, |socket, item| {
        let Item::Binary(bytes) = item else { return };
        if bytes.len() < 64 {
            return;
        }
        match u32_at(&bytes, 24) {
            STREAM_IQ => {
                arm.iq_bytes += bytes.len() as u64;
                arm.iq_frames += 1;
            }
            STREAM_AUDIO => {
                let now = Instant::now();
                arm.audio_bytes += bytes.len() as u64;
                arm.audio_frames += 1;
                if let Some(previous) = last_audio_at {
                    arm.audio_intervals_ms.push(now.duration_since(previous).as_secs_f64() * 1000.0);
                }
                last_audio_at = Some(now);
                let sequence = u32_at(&bytes, 32);
                if let Some(previous) = last_sequence {
                    arm.audio_gaps += u64::from(sequence.wrapping_sub(previous).wrapping_sub(1));
                }
                last_sequence = Some(sequence);
            }
            STREAM_SPECTRUM_ROW => {
                arm.row_bytes += bytes.len() as u64;
                arm.row_frames += 1;
                send(socket, &format!("saturn_display_ack:{};", u32_at(&bytes, 32)));
            }
            _ => {}
        }
    });
    arm.seconds = started.elapsed().as_secs_f64();
    arm.bridge_cpu_percent = (cpu_seconds(pid) - cpu_before) / arm.seconds * 100.0;
    let perf = replay.perf().expect("perf.json");
    arm.perf = ["outbound_drops", "out_hwm_bytes", "tcp_outq_hwm_bytes", "audio_dropped_s", "send_blocked_ms"]
        .into_iter()
        .map(|key| (key, perf.number(key)))
        .collect();
    arm
}

fn report(arms: &[Arm]) -> String {
    let mut out = String::new();
    writeln!(out, "\nArm  secs  IQ Mbit/s  rows Mbit/s  audio Mbit/s  IQ fps  row fps  audio fps  gaps  audio gap ms p50/p95/p99/max  late(>2x)  Bridge CPU%").unwrap();
    for arm in arms {
        let mbit = |bytes: u64| bytes as f64 * 8.0 / arm.seconds / 1e6;
        let mut intervals = arm.audio_intervals_ms.clone();
        intervals.sort_by(f64::total_cmp);
        let late = intervals.iter().filter(|&&v| v > 2.0 * NOMINAL_AUDIO_PERIOD_MS).count();
        writeln!(
            out,
            "{:<4} {:>4.0}  {:>9.2}  {:>11.2}  {:>12.2}  {:>6.1}  {:>7.1}  {:>9.1}  {:>4}  {:>7.1}/{:.1}/{:.1}/{:.1}  {:>9}  {:>10.1}",
            arm.name, arm.seconds, mbit(arm.iq_bytes), mbit(arm.row_bytes), mbit(arm.audio_bytes),
            arm.iq_frames as f64 / arm.seconds, arm.row_frames as f64 / arm.seconds,
            arm.audio_frames as f64 / arm.seconds, arm.audio_gaps,
            percentile(&intervals, 0.50), percentile(&intervals, 0.95), percentile(&intervals, 0.99),
            intervals.last().copied().unwrap_or(f64::NAN), late, arm.bridge_cpu_percent
        )
        .unwrap();
    }
    writeln!(out, "\nBridge counters after each arm (session values; audio_dropped_s is per second):").unwrap();
    for arm in arms {
        let values: Vec<String> = arm
            .perf
            .iter()
            .map(|(key, value)| format!("{key}={}", value.map_or("n/a".into(), |v| format!("{v:.0}"))))
            .collect();
        writeln!(out, "  {:<4} {}", arm.name, values.join("  ")).unwrap();
    }
    out
}

/// TCP_NODELAY as the shim reports it for the sockets the Bridge accepted. A run
/// that asked for it must show it set and read back on every arm's connection;
/// a run that did not ask must show none. Anything else means the arm is not
/// what its label says.
fn nodelay_summary(replay: &Replay, connections: usize) -> (String, Result<(), String>) {
    let requested = std::env::var("SATURN_REPLAY_TCP_NODELAY").is_ok_and(|value| value == "1");
    let log = replay.log_text("stderr.log");
    let enabled = log.matches("TCP_NODELAY enabled on accepted fd").count();
    let failed = log.matches("TCP_NODELAY FAILED").count();
    let verdict = if failed != 0 {
        Err("the shim could not set or read back TCP_NODELAY".to_string())
    } else if requested && enabled < connections {
        Err(format!("TCP_NODELAY was requested but confirmed on {enabled} of {connections} accepted sockets"))
    } else if !requested && enabled != 0 {
        Err("TCP_NODELAY was not requested but was set".to_string())
    } else {
        Ok(())
    };
    let text = format!(
        "TCP_NODELAY requested: {requested}; confirmed (set and read back) on {enabled} accepted sockets; failures: {failed}"
    );
    (text, verdict)
}

/// The records a published comparison rests on, kept in the run directory:
/// the table, every audio inter-arrival time, the Bridge's final perf.json and
/// the settings of the run.
fn save_records(replay: &Replay, arms: &[Arm], table: &str, nodelay: &str, window: Duration) {
    let dir = replay.root().join("results");
    fs::create_dir_all(&dir).expect("create the results directory");
    let mut info = String::new();
    writeln!(info, "bridge: {}", env!("CARGO_BIN_EXE_saturn-bridge")).unwrap();
    if let Ok(output) = std::process::Command::new("sha256sum").arg(env!("CARGO_BIN_EXE_saturn-bridge")).output() {
        write!(info, "bridge sha256: {}", String::from_utf8_lossy(&output.stdout)).unwrap();
    }
    writeln!(info, "measured window per arm: {} s", window.as_secs()).unwrap();
    writeln!(info, "{nodelay}").unwrap();
    writeln!(info, "tones: {TONE_OFFSET_HZ} Hz at -30 dBFS, {SAMPLE_RATE} S/s").unwrap();
    fs::write(dir.join("run-info.txt"), info).unwrap();
    fs::write(dir.join("transport-screen.txt"), table).unwrap();
    for arm in arms {
        let rows: Vec<String> = arm.audio_intervals_ms.iter().map(|v| format!("{v:.3}")).collect();
        fs::write(dir.join(format!("arm-{}-audio-intervals-ms.txt", arm.name)), rows.join("\n") + "\n").unwrap();
    }
    if let Ok(perf) = fs::read(replay.work.join("perf.json")) {
        fs::write(dir.join("perf-final.json"), perf).unwrap();
    }
}

#[test]
#[ignore = "needs the real native WDSP and about two minutes; see the module docs"]
fn transport_screen_of_raw_iq_against_spectrum_rows() {
    let seconds: u64 = std::env::var("SATURN_SCREEN_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let window = Duration::from_secs(seconds);
    let mut replay = Replay::start();
    // This is a measurement: its records are the point, so they are kept.
    replay.keep();
    replay.wait_ready(Duration::from_secs(300));

    let arms = vec![
        run_arm(&replay, "A1", Display::RawIq, true, window),
        run_arm(&replay, "B", Display::Rows, true, window),
        run_arm(&replay, "A2", Display::RawIq, true, window),
        run_arm(&replay, "C", Display::None, true, window),
    ];
    let table = report(&arms);
    print!("{table}");
    let (nodelay, nodelay_verdict) = nodelay_summary(&replay, arms.len());
    println!("\n{nodelay}");
    save_records(&replay, &arms, &table, &nodelay, window);
    println!("records kept in {}", replay.root().join("results").display());
    // Judged after the records are saved, so a failure still leaves them.
    if let Err(why) = nodelay_verdict {
        panic!("{why}");
    }

    for arm in &arms {
        assert!(arm.audio_frames > 0, "{}: no audio", arm.name);
        assert_eq!(arm.audio_gaps, 0, "{}: audio sequence gaps", arm.name);
    }
    let (a1, b, c) = (&arms[0], &arms[1], &arms[3]);
    assert!(a1.iq_frames > 0 && a1.row_frames == 0, "A1 must carry only raw IQ");
    assert!(b.row_frames > 0 && b.iq_frames == 0, "B must carry only rows");
    assert!(c.iq_frames == 0 && c.row_frames == 0, "C must carry audio only");
    assert!(
        a1.iq_bytes as f64 / b.row_bytes as f64 > 10.0,
        "rows should cost far less than raw IQ"
    );
    replay.stop_clean();
}
