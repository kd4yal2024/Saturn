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
    let mut last_duc_specific = Instant::now();
    let tx_silence_gap = Duration::from_millis(75);
    let tx_packet_period = Duration::from_secs_f64(DUC_IQ_SAMPLES_PER_PACKET as f64 / 48_000.0);
    let tx_silence_packet = vec![0.0_f32; DUC_IQ_SAMPLES_PER_PACKET * 2];
    let mut last_tx_audio_at = Instant::now();
    let mut next_tx_keepalive_at = Instant::now();
    let mut tx_silence_keepalive_active = false;
    loop {
        let mut did_work = false;
        let mut needs_bootstrap = false;
        let mut needs_stop = false;
        let mut needs_duc_specific = false;
        let mut needs_high_priority = false;

        while let Some(command) = tci.try_recv_command() {
            did_work = true;
            let mut model = radio_model.lock().unwrap();
            let mut reconfigure_ddc = false;

            match command {
                TciCommand::SetVfoA(freq_hz) => {
                    model.desired.vfo_a_hz = freq_hz;
                    model.desired.iq_center_hz = freq_hz;
                    model.desired.tx_frequency_hz = freq_hz;
                    needs_high_priority = true;
                }
                TciCommand::SetVfoB(freq_hz) => {
                    model.desired.vfo_b_hz = freq_hz;
                }
                TciCommand::SetIqCenter(freq_hz) => {
                    model.desired.iq_center_hz = freq_hz;
                    needs_high_priority = true;
                }
                TciCommand::SetMode(mode) => {
                    model.desired.mode = mode;
                    let (rx_low_hz, rx_high_hz) = mode.default_filter_band();
                    model.desired.filter_low_hz = rx_low_hz;
                    model.desired.filter_high_hz = rx_high_hz;
                    let (tx_low_hz, tx_high_hz) = mode.default_tx_filter_band();
                    model.desired.tx_filter_low_hz = tx_low_hz;
                    model.desired.tx_filter_high_hz = tx_high_hz;
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
                    needs_high_priority = true;
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
                TciCommand::SetRxAnrVals {
                    taps,
                    delay,
                    gain,
                    leakage,
                } => {
                    if let Some(taps) = taps {
                        model.desired.rx_anr_taps = taps.clamp(1, 128);
                    }
                    if let Some(delay) = delay {
                        model.desired.rx_anr_delay = delay.clamp(0, 127);
                    }
                    if let Some(gain) = gain {
                        model.desired.rx_anr_gain = gain.clamp(0.0, 1.0);
                    }
                    if let Some(leakage) = leakage {
                        model.desired.rx_anr_leakage = leakage.clamp(0.0, 1.0);
                    }
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
                    let normalized = wdsp.set_audio_frame_float_count(sample_count as usize) as u32;
                    if sample_count != normalized {
                        eprintln!(
                            "saturn-bridge: requested audio frame size {} float32 samples, using {}",
                            sample_count,
                            normalized
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
                    needs_duc_specific = true;
                    println!("saturn-bridge: TCI client active — taking P2 controller role");
                }
                TciCommand::ClientDisconnected => {
                    if model.desired.tx_enabled {
                        model.desired.tx_enabled = false;
                        wdsp.reset_stream_buffers();
                        wdsp_tx.set_active(false);
                        needs_duc_specific = true;
                        needs_high_priority = true;
                        tx_silence_keepalive_active = false;
                    }
                    model.desired.two_tone_enabled = false;
                    model.desired.running = false;
                    needs_stop = true;
                    println!("saturn-bridge: no TCI clients — releasing P2 controller role");
                }
                TciCommand::SetTxEnabled(enabled) => {
                    if model.desired.tx_enabled != enabled {
                        model.desired.tx_enabled = enabled;
                        wdsp.reset_stream_buffers();
                        wdsp_tx.set_active(enabled);
                        needs_duc_specific = true;
                        needs_high_priority = true;
                        if enabled {
                            let now = Instant::now();
                            last_tx_audio_at = now.checked_sub(tx_silence_gap).unwrap_or(now);
                            next_tx_keepalive_at = now;
                        } else {
                            tx_silence_keepalive_active = false;
                        }
                        println!(
                            "saturn-bridge: TX state -> {}",
                            if enabled { "ON" } else { "OFF" }
                        );
                    }
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
                TciCommand::SetRxAnfVals {
                    taps,
                    delay,
                    gain,
                    leakage,
                } => {
                    if let Some(taps) = taps {
                        model.desired.rx_anf_taps = taps.clamp(1, 128);
                    }
                    if let Some(delay) = delay {
                        model.desired.rx_anf_delay = delay.clamp(0, 127);
                    }
                    if let Some(gain) = gain {
                        model.desired.rx_anf_gain = gain.clamp(0.0, 1.0);
                    }
                    if let Some(leakage) = leakage {
                        model.desired.rx_anf_leakage = leakage.clamp(0.0, 1.0);
                    }
                }
                TciCommand::SetAgcMode(mode) => {
                    model.desired.agc_mode = mode;
                }
                TciCommand::SetAgcGain(gain) => {
                    model.desired.agc_gain = gain.clamp(0.0, 100.0);
                }
                TciCommand::SetTxDrive(drive) => {
                    model.desired.tx_drive = drive.min(100);
                    needs_high_priority = true;
                }
                TciCommand::SetTxMicGain(gain_db) => {
                    model.desired.tx_mic_gain_db = gain_db.clamp(-20.0, 20.0);
                }
                TciCommand::SetTxFilterBand { low_hz, high_hz } => {
                    model.desired.tx_filter_low_hz = low_hz;
                    model.desired.tx_filter_high_hz = high_hz;
                }
                TciCommand::SetRxEqEnabled(enabled) => {
                    model.desired.rx_eq_enabled = enabled;
                }
                TciCommand::SetRxEqBand { band, gain_db } => {
                    model.desired.rx_eq_bands[band] = gain_db.clamp(-20, 20);
                }
                TciCommand::SetTxEqEnabled(enabled) => {
                    model.desired.tx_eq_enabled = enabled;
                }
                TciCommand::SetTxEqBand { band, gain_db } => {
                    model.desired.tx_eq_bands[band] = gain_db.clamp(-20, 20);
                }
                TciCommand::SetTxCfcEnabled(enabled) => {
                    model.desired.cfc_enabled = enabled;
                }
                TciCommand::SetTxCfcPrecomp(db) => {
                    model.desired.cfc_precomp_db = db.clamp(0.0, 20.0);
                }
                TciCommand::SetTxCfcBand { band, gain_db } => {
                    model.desired.cfc_bands[band - 1] = gain_db.clamp(0.0, 20.0);
                }
                TciCommand::SetTxTwoToneTest(enabled) => {
                    model.desired.two_tone_enabled = enabled;
                    println!(
                        "saturn-bridge: two-tone test -> {}",
                        if enabled { "ON (700/1900 Hz)" } else { "OFF" }
                    );
                }
                TciCommand::SetTxTwoToneFreq1(freq_hz) => {
                    model.desired.tx_two_tone_freq1_hz = freq_hz.clamp(10.0, 10_000.0);
                }
                TciCommand::SetTxTwoToneFreq2(freq_hz) => {
                    model.desired.tx_two_tone_freq2_hz = freq_hz.clamp(10.0, 10_000.0);
                }
                TciCommand::SetTxTwoToneLevelDb(level_db) => {
                    model.desired.tx_two_tone_level_db = level_db.clamp(-40.0, 0.0);
                }
                TciCommand::SetTxTwoToneInvertLsb(enabled) => {
                    model.desired.tx_two_tone_invert_lsb = enabled;
                }
                TciCommand::SetTxTwoToneDelayMs(delay_ms) => {
                    model.desired.tx_two_tone_delay_ms = delay_ms.min(2_000);
                }
                TciCommand::MicAudioFrame(samples) => {
                    last_tx_audio_at = Instant::now();
                    next_tx_keepalive_at = last_tx_audio_at + tx_packet_period;
                    if tx_silence_keepalive_active {
                        tx_silence_keepalive_active = false;
                        println!("saturn-bridge: TX live audio active");
                    }
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

        if needs_duc_specific {
            session.send_duc_specific()?;
            last_duc_specific = Instant::now();
            did_work = true;
        }

        // bootstrap() acquires the radio_model lock internally, so it must be called
        // after the command loop has released it, never while holding it.
        if needs_bootstrap {
            session.bootstrap(&radio_model)?;
            last_duc_specific = Instant::now();
            did_work = true;
        }
        if needs_stop {
            session.send_stop()?;
            did_work = true;
        }
        if needs_high_priority {
            let model = radio_model.lock().unwrap();
            session.send_high_priority(&model)?;
            did_work = true;
        }

        let running = { radio_model.lock().unwrap().desired.running };
        if running && last_duc_specific.elapsed() >= config.high_priority_period {
            session.send_duc_specific()?;
            last_duc_specific = Instant::now();
            did_work = true;
        }

        let tx_enabled = { radio_model.lock().unwrap().desired.tx_enabled };
        if tx_enabled && last_tx_audio_at.elapsed() >= tx_silence_gap {
            let now = Instant::now();
            let mut sent_packets = 0usize;
            while now >= next_tx_keepalive_at && sent_packets < 8 {
                session.send_duc_iq(&tx_silence_packet)?;
                next_tx_keepalive_at += tx_packet_period;
                sent_packets += 1;
            }
            if sent_packets > 0 && !tx_silence_keepalive_active {
                tx_silence_keepalive_active = true;
                println!("saturn-bridge: TX silence keepalive active (no mic audio)");
            }
            if sent_packets == 8 && now >= next_tx_keepalive_at {
                next_tx_keepalive_at = now + tx_packet_period;
            }
            if sent_packets > 0 {
                did_work = true;
            }
        }

        if let Some(event) = session.recv_event()? {
            did_work = true;
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
                    // Mute RX audio during TX to prevent TX carrier bleed-through.
                    if !model.desired.tx_enabled {
                        for audio_frame in wdsp.push_iq(&frame.iq_samples) {
                            tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio_frame);
                        }
                    } else {
                        // Still drain WDSP to keep its internal state consistent.
                        let _ = wdsp.push_iq(&frame.iq_samples);
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
            did_work = true;
        }

        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        if !did_work {
            thread::sleep(Duration::from_millis(1));
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    let hp_result = hp_thread
        .join()
        .map_err(|_| "high-priority thread panicked")?;
    hp_result?;
    thread::sleep(Duration::from_millis(10));
    Ok(())
}
