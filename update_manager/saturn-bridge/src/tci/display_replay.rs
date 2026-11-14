//! Explicit, bounded loopback fixture for display comparisons. Test binary only:
//! real TCI dispatch/queues/framing and spectrum worker, synthetic IQ/PCM, no DSP
//! channel, acquisition owner, device, PTT or radio backend is constructed.
use super::*;
use crate::display_spectrum::{LatestIqRing, SpectrumWorker};

#[test]
#[ignore = "launched explicitly by profile-display.mjs with a private loopback port"]
fn loopback_display_fixture() {
    let output = std::path::PathBuf::from(
        std::env::var_os("SATURN_DISPLAY_REPLAY_DIR").expect("private fixture directory"),
    );
    let port: u16 = std::env::var("SATURN_DISPLAY_REPLAY_PORT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(port > 1024);
    let mut config = BridgeConfig::default();
    config.tci_bind_addr = ([127, 0, 0, 1], port).into();
    config.remote_tx_rf_enabled = false;
    config.satp_enabled = false;
    let model = Arc::new(Mutex::new(RadioModel::new(
        6, 14_200_000, 0, 384, 24, 2048, true, 4096, true,
    )));
    let (front, commands) =
        TciFrontend::bind_with_display_spectrum(&config, model.clone(), true).unwrap();
    let front = Arc::new(front);
    let ring = Arc::new(LatestIqRing::new(14_200_000));
    let worker = SpectrumWorker::spawn(front.clone(), ring.clone(), 384_000).unwrap();
    // Integral cycles per 12800-pair packet; contiguous on repeated publication.
    let iq: Vec<f32> = (0..12800)
        .flat_map(|n| {
            let p = 2.0 * std::f64::consts::PI * n as f64 / 256.0;
            [
                (0.15 * p.cos() + 0.05 * (3.0 * p).cos()) as f32,
                (0.15 * p.sin() - 0.05 * (3.0 * p).sin()) as f32,
            ]
        })
        .collect();
    let audio: Vec<f32> = (0..960)
        .flat_map(|n| {
            let a =
                (0.02 * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 48000.0).sin()) as f32;
            [a, a]
        })
        .collect();
    std::fs::write(
        output.join("ready.json"),
        format!("{{\"port\":{port},\"synthetic\":true,\"hardwareAccess\":false}}\n"),
    )
    .unwrap();
    let started = Instant::now();
    let mut iq_due = started;
    let mut audio_due = started;
    let iq_period = Duration::from_nanos(1_000_000_000 / 30);
    let audio_period = Duration::from_millis(20);
    let mut iq_frames = 0u64;
    let mut audio_frames = 0u64;
    while started.elapsed() < Duration::from_secs(120) && !output.join("stop").exists() {
        while let Ok(command) = commands.try_recv() {
            let mut m = model.lock_unpoisoned();
            match command {
                TciCommand::SetIqCenter(hz) => {
                    m.desired.iq_center_hz = hz;
                    ring.restart(u64::from(hz));
                    front.publish_tuning_state(&m);
                }
                TciCommand::SetVfoA(hz) => {
                    m.desired.vfo_a_hz = hz;
                    front.publish_tuning_state(&m);
                }
                TciCommand::RequestRadioState { client_id } => {
                    front.publish_standard_radio_state_to(client_id, &m)
                }
                TciCommand::SaturnPing {
                    client_id,
                    nonce,
                    sent_at,
                } => front.publish_saturn_pong(client_id, &nonce, &sent_at),
                // All TX/settings requests are inert in this synthetic fixture.
                _ => {}
            }
        }
        let now = Instant::now();
        if now >= iq_due {
            ring.push(&iq);
            if front.media_demand().raw_iq_stream_enabled {
                front.publish_full_rate_iq_frame(384_000, &iq);
            }
            iq_frames += 1;
            iq_due += iq_period;
            if iq_due <= now {
                iq_due = now + iq_period;
            }
        }
        if now >= audio_due {
            front.publish_audio_frame(48_000, &audio, 0.0);
            audio_frames += 1;
            audio_due += audio_period;
            if audio_due <= now {
                audio_due = now + audio_period;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    // Stop/join the actual spectrum worker before collecting its final counters.
    let stats = worker.stats.clone();
    drop(worker);
    let fft_histogram = stats
        .fft_latency
        .lock_unpoisoned()
        .fields("fft")
        .into_iter()
        .map(|(key, value)| {
            let crate::xdma_telemetry::TelemetryValue::Number(number) = value else {
                panic!("expected numeric histogram");
            };
            format!("\"{key}\":{number}")
        })
        .collect::<Vec<_>>()
        .join(",");
    let snapshot = front.client_snapshot();
    let spectrum = snapshot.spectrum;
    std::fs::write(output.join("result.json"), format!(concat!(
        "{{\"synthetic\":true,\"hardwareAccess\":false,\"seconds\":{},",
        "\"sourceIqFrames\":{},\"sourceAudioFrames\":{},\"rowsComputed\":{},\"capturesSkipped\":{},",
        "\"rowsWritten\":{},\"rowBytesWritten\":{},\"rowReplacements\":{},\"rowAcks\":{},",
        "\"rowCreditHighWater\":{},\"rawIqWritten\":{},\"rawIqDropped\":{},\"rawIqQueueHighWater\":{},",
        "\"activeConnectionsAtStop\":{},\"fftHistogram\":{{{}}}}}\n"),
        started.elapsed().as_secs_f64(), iq_frames, audio_frames,
        stats.rows_computed.load(Ordering::Relaxed), stats.captures_skipped.load(Ordering::Relaxed),
        spectrum.written, spectrum.bytes_written, spectrum.replaced, spectrum.acks, spectrum.in_flight_max,
        snapshot.full_rate_iq_written_deliveries_total, snapshot.full_rate_iq_dropped_deliveries_total,
        snapshot.full_rate_iq_queue_high_watermark, snapshot.active_connections, fft_histogram)).unwrap();
    assert_eq!(
        snapshot.active_connections, 0,
        "profiler must close its socket before stopping the fixture"
    );
}
