mod config;
mod p2;
mod radio_model;
mod tci;
mod wdsp;

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use config::BridgeConfig;
use p2::session::{P2Event, P2Session};
use radio_model::RadioModel;
use tci::{TciCommand, TciFrontend};
use wdsp::{WdspRxEngine, WdspTxEngine, DUC_IQ_SAMPLES_PER_PACKET};

fn main() -> Result<(), Box<dyn Error>> {
    let config = BridgeConfig::from_env();
    let radio_model = Arc::new(Mutex::new(RadioModel::new(
        config.rx_ddc_index,
        config.ddc0_frequency_hz,
        config.ddc0_adc,
        config.ddc0_sample_rate_khz,
        config.ddc0_sample_size_bits,
    )));
    let session = P2Session::bind(config.clone())?;
    let tci = TciFrontend::bind(&config, radio_model.clone())?;
    let mut wdsp = {
        let model = radio_model.lock().unwrap();
        WdspRxEngine::new(&model)?
    };
    let mut wdsp_tx = {
        let model = radio_model.lock().unwrap();
        WdspTxEngine::new(&model)
    };
    let stop_flag = Arc::new(AtomicBool::new(false));

    println!(
        "saturn-bridge: binding {} -> radio {} | TCI {}",
        session.client_bind_addr(),
        config.radio_command_addr,
        config.tci_bind_addr
    );

    let hp_thread = session.spawn_high_priority_loop(radio_model.clone(), stop_flag.clone())?;

    let mut last_status = Instant::now();
    loop {
        let mut needs_bootstrap = false;
        let mut needs_stop = false;

        while let Some(command) = tci.try_recv_command() {
            let mut model = radio_model.lock().unwrap();
            let mut reconfigure_ddc = false;

            match command {
                TciCommand::SetVfoA(freq_hz) => {
                    model.desired.vfo_a_hz = freq_hz;
                    model.desired.iq_center_hz = freq_hz;
                    model.desired.tx_frequency_hz = freq_hz;
                }
                TciCommand::SetVfoB(freq_hz) => {
                    model.desired.vfo_b_hz = freq_hz;
                }
                TciCommand::SetIqCenter(freq_hz) => {
                    model.desired.iq_center_hz = freq_hz;
                }
                TciCommand::SetMode(mode) => {
                    model.desired.mode = mode;
                    let (low_hz, high_hz) = mode.default_filter_band();
                    model.desired.filter_low_hz = low_hz;
                    model.desired.filter_high_hz = high_hz;
                }
                TciCommand::SetFilterBand { low_hz, high_hz } => {
                    model.desired.filter_low_hz = low_hz;
                    model.desired.filter_high_hz = high_hz;
                }
                TciCommand::SetRxAdc(adc) => {
                    model.desired.ddc0_adc = adc.min(2);
                    reconfigure_ddc = true;
                }
                TciCommand::SetRxAntenna(antenna) => {
                    model.desired.rx_antenna = antenna.clamp(1, 3);
                }
                TciCommand::SetRxVolume(volume_db) => {
                    model.desired.rx_volume_db = volume_db.clamp(-40.0, 12.0);
                }
                TciCommand::SetRxNoiseReductionMode(mode) => {
                    model.desired.rx_noise_reduction_mode = mode;
                }
                TciCommand::SetRxNoiseReductionEnabled(enabled) => {
                    model.desired.rx_noise_reduction_mode = if enabled {
                        radio_model::NoiseReductionMode::Nr1
                    } else {
                        radio_model::NoiseReductionMode::Off
                    };
                }
                TciCommand::SetRxNoiseReductionLevel(level) => {
                    model.desired.rx_noise_reduction_level = level.clamp(0.0, 100.0);
                }
                TciCommand::SetIqSampleRate(rate_hz) => {
                    let rate_khz = (rate_hz / 1000).clamp(48, u16::MAX as u32) as u16;
                    model.desired.ddc0_sample_rate_khz = rate_khz;
                    reconfigure_ddc = true;
                }
                TciCommand::SetIqStreaming => {}
                TciCommand::RequestSmeter => {}
                TciCommand::SetAudioStreaming(enabled) => {
                    if enabled {
                        tci.publish_audio_started(wdsp.audio_sample_rate_hz());
                    } else {
                        tci.publish_audio_stopped();
                    }
                }
                TciCommand::SetAudioSampleRate(rate_hz) => {
                    if rate_hz != wdsp.audio_sample_rate_hz() {
                        eprintln!(
                            "saturn-bridge: requested audio sample rate {} Hz, using {} Hz",
                            rate_hz,
                            wdsp.audio_sample_rate_hz()
                        );
                    }
                    tci.publish_audio_started(wdsp.audio_sample_rate_hz());
                }
                TciCommand::SetAudioFrameSamples(sample_count) => {
                    if sample_count != 2048 {
                        eprintln!(
                            "saturn-bridge: requested audio frame size {} float32 samples, using 2048",
                            sample_count
                        );
                    }
                }
                TciCommand::SetAudioChannels(channels) => {
                    if channels != 2 {
                        eprintln!(
                            "saturn-bridge: requested {} audio channels, using stereo",
                            channels
                        );
                    }
                }
                TciCommand::ClientConnected => {
                    model.desired.running = true;
                    needs_bootstrap = true;
                    println!("saturn-bridge: TCI client active — taking P2 controller role");
                }
                TciCommand::ClientDisconnected => {
                    model.desired.running = false;
                    needs_stop = true;
                    println!("saturn-bridge: no TCI clients — releasing P2 controller role");
                }
                TciCommand::SetTxEnabled(enabled) => {
                    model.desired.tx_enabled = enabled;
                    wdsp_tx.set_active(enabled);
                }
                TciCommand::SetNoiseBlankerMode(mode) => {
                    model.desired.nb_mode = mode;
                }
                TciCommand::SetNoiseBlankerThreshold(thresh) => {
                    model.desired.nb_threshold = thresh;
                }
                TciCommand::SetAnfEnabled(enabled) => {
                    model.desired.anf_enabled = enabled;
                }
                TciCommand::SetAgcMode(mode) => {
                    model.desired.agc_mode = mode;
                }
                TciCommand::SetTxDrive(drive) => {
                    model.desired.tx_drive = drive;
                }
                TciCommand::SetTxMicGain(gain_db) => {
                    model.desired.tx_mic_gain_db = gain_db.clamp(-20.0, 20.0);
                }
                TciCommand::MicAudioFrame(samples) => {
                    // Extract mono from interleaved stereo (or treat as mono).
                    let mono: Vec<f32> = if samples.len() % 2 == 0 {
                        // Stereo interleaved: take every other sample (left channel)
                        samples.iter().step_by(2).copied().collect()
                    } else {
                        samples
                    };
                    wdsp_tx.push_mic(&mono);
                    // Drain completed 240-sample DUC IQ packets
                    let floats_per_packet = DUC_IQ_SAMPLES_PER_PACKET * 2;
                    while wdsp_tx.pending_iq.len() >= floats_per_packet {
                        let chunk: Vec<f32> =
                            wdsp_tx.pending_iq.drain(..floats_per_packet).collect();
                        session.send_duc_iq(&chunk)?;
                    }
                }
            }

            if reconfigure_ddc {
                session.configure_rx_ddc(
                    model.desired.rx_ddc_index,
                    model.desired.ddc0_sample_rate_khz,
                    model.desired.ddc0_sample_size_bits,
                    model.desired.ddc0_adc,
                )?;
            }
            wdsp.sync_model(&model)?;
            wdsp_tx.sync_model(&model);
            tci.publish_radio_state(&model);
        }

        // bootstrap() acquires the radio_model lock internally, so it must be called
        // after the command loop has released it, never while holding it.
        if needs_bootstrap {
            session.bootstrap(&radio_model)?;
        }
        if needs_stop {
            session.send_stop()?;
        }

        if let Some(event) = session.recv_event()? {
            let mut model = radio_model.lock().unwrap();
            match event {
                P2Event::HighPriorityFromSdr(packet) => {
                    model.apply_high_priority(packet);
                    tci.publish_telemetry(&model);
                }
                P2Event::DdcIq(frame) => {
                    if frame.ddc_index != model.desired.rx_ddc_index {
                        continue;
                    }
                    let sample_rate_hz = model.desired.ddc0_sample_rate_khz as u32 * 1000;
                    tci.publish_iq_frame(sample_rate_hz, &frame.iq_samples);
                    for audio_frame in wdsp.push_iq(&frame.iq_samples) {
                        tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio_frame);
                    }
                    if let Some(dbm) = wdsp.smeter_dbm() {
                        model.observed.ddc0_meter_dbm = Some(dbm);
                    }
                    model.apply_ddc_frame(frame);
                }
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            let model = radio_model.lock().unwrap();
            println!("saturn-bridge: {}", model.status_line());
            last_status = Instant::now();
        }

        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    let hp_result = hp_thread.join().map_err(|_| "high-priority thread panicked")?;
    hp_result?;
    thread::sleep(Duration::from_millis(10));
    Ok(())
}
