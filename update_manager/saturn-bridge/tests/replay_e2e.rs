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

mod common;

use std::time::{Duration, Instant};

use common::*;

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
    // TCP_NODELAY evidence from the serving process itself: the setting, and the sockets the kernel read back as on.
    let nodelay_off = std::env::var("SATURN_BRIDGE_TCI_NODELAY")
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"));
    assert_eq!(perf.number("tci_nodelay_enabled"), Some(if nodelay_off { 0.0 } else { 1.0 }));
    assert_eq!(perf.number("tci_nodelay_failed_total"), Some(0.0), "a TCP_NODELAY call failed");
    let confirmed = perf.number("tci_nodelay_confirmed_total").expect("perf.json lacks tci_nodelay_confirmed_total");
    assert_eq!(confirmed >= 1.0, !nodelay_off, "confirmed sockets {confirmed} with the setting {}", if nodelay_off { "off" } else { "on" });

    // A SIGTERM sent to the running Bridge is a clean, receive-safe shutdown:
    // it must exit with status 0. A crash signal, a nonzero exit, or an exit
    // before the request all fail the test and keep the logs.
    drop(socket);
    replay.stop_clean();
}

/// The wire sequence the Remote page's RX Measure uses under the display
/// override: the stream is on server rows, the page asks for raw IQ explicitly
/// (`saturn_display:iq;`), captures, then asks for the rows again. On the real
/// Bridge: rows carry no IQ, the explicit request brings raw IQ back on the
/// running stream without a new `iq_start`, and the rows return afterwards.
#[test]
#[ignore = "needs the real native WDSP and about a minute; see the module docs"]
fn the_display_switches_from_rows_to_raw_iq_and_back_on_a_live_stream() {
    let mut replay = Replay::start();
    replay.wait_ready(Duration::from_secs(300));
    let mut socket = connect(replay.port);
    collect(&mut socket, Duration::from_secs(2), |_, _| {});

    // What arrived in a window, ignoring frames already in flight at the switch.
    fn window(socket: &mut Socket, seconds: u64) -> (usize, usize, Vec<String>) {
        let (mut iq, mut rows, mut texts) = (0, 0, Vec::new());
        let settled = Instant::now() + Duration::from_millis(800);
        collect(socket, Duration::from_secs(seconds), |socket, item| match item {
            Item::Text(text) if text.contains("saturn_display:") => texts.push(text),
            Item::Binary(bytes) if bytes.len() >= 64 => match u32_at(&bytes, 24) {
                STREAM_SPECTRUM_ROW => {
                    send(socket, &format!("saturn_display_ack:{};", u32_at(&bytes, 32)));
                    if Instant::now() >= settled {
                        rows += 1;
                    }
                }
                STREAM_IQ if Instant::now() >= settled => iq += 1,
                _ => {}
            },
            _ => {}
        });
        (iq, rows, texts)
    }

    // Rows first, as the override holds the page, with iq_start after the echo.
    send(&mut socket, "saturn_display:spectrum,2048,50;");
    collect(&mut socket, Duration::from_millis(300), |_, _| {});
    send(&mut socket, "iq_samplerate:384000;iq_start:0;");
    let (iq, rows, _) = window(&mut socket, 4);
    println!("rows mode: {rows} rows, {iq} raw IQ frames");
    assert!(rows >= 60 && iq == 0, "rows mode should carry rows and no raw IQ: {rows} rows, {iq} IQ");

    // RX Measure asks for raw IQ: the echo comes back, raw IQ flows, the rows stop.
    send(&mut socket, "saturn_display:iq;");
    let (iq, rows, texts) = window(&mut socket, 4);
    println!("after saturn_display:iq; {iq} raw IQ frames, {rows} rows, echo {texts:?}");
    assert!(texts.iter().any(|text| text.contains("iq")), "no echo for the raw IQ request: {texts:?}");
    assert!(iq >= 60 && rows == 0, "raw IQ should resume on the running stream: {iq} IQ, {rows} rows");

    // The capture is over: the page asks for the rows again.
    send(&mut socket, "saturn_display:spectrum,2048,50;");
    let (iq, rows, texts) = window(&mut socket, 4);
    println!("after the rows request: {rows} rows, {iq} raw IQ frames, echo {texts:?}");
    assert!(texts.iter().any(|text| text.contains("spectrum")), "no echo for the rows request: {texts:?}");
    assert!(rows >= 60 && iq == 0, "the rows should return: {rows} rows, {iq} IQ");

    drop(socket);
    replay.stop_clean();
}
