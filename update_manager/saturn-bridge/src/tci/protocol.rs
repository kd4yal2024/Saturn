use std::collections::BTreeSet;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;

use crate::radio_model::{
    AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, Nr2GainMethod, Nr2NpeMethod,
    WbfmDeemphasis,
};
use crate::sync_ext::MutexExt;
use crate::tx_codec::{TxCodecDecoder, TxCodecRuntimeFlags, TxDecodeError, TxMicCodec};

use super::*;

#[derive(Clone, Debug)]
pub struct TciMicFrame {
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub sequence: u32,
    pub received_at: Instant,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum TciCommand {
    SetVfoA(u32),
    SetVfoB(u32),
    SetIqCenter(u32),
    SetMode(DemodMode),
    SetFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxAdc(u8),
    SetRxAntenna(u8),
    SetRxVolume(f64),
    SetRxNoiseReductionMode(NoiseReductionMode),
    SetRxNoiseReductionEnabled(bool),
    SetRxNoiseReductionLevel(f64),
    SetRxNr2GainMethod(Nr2GainMethod),
    SetRxNr2NpeMethod(Nr2NpeMethod),
    SetRxNr2PostFilterEnabled(bool),
    SetRxWbfmDeemphasis(WbfmDeemphasis),
    SetRxAnrVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetIqSampleRate(u32),
    SetIqStreaming,
    RequestSmeter,
    SaturnPing {
        client_id: u64,
        nonce: String,
        sent_at: String,
    },
    SplitSessionOpen {
        client_id: u64,
        session_id: String,
        role: TciClientRole,
    },
    SplitSessionLane {
        client_id: u64,
        session_id: String,
        lane: SplitSocketKind,
    },
    SetAudioStreaming(bool),
    SetAudioSampleRate(u32),
    SetAudioFrameSamples(u32),
    SetAudioChannels(u32),
    SetTxEnabled(bool),
    SetNoiseBlankerMode(NoiseBlankerMode),
    SetNoiseBlankerThreshold(f64),
    SetAnfEnabled(bool),
    SetRxAnfVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetAgcMode(AgcMode),
    SetAgcGain(f64),
    SetTxDrive(u8),
    SetTxMicGain(f64),
    SetTxFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxEqEnabled(bool),
    SetRxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxEqEnabled(bool),
    SetTxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxCfcEnabled(bool),
    SetTxCfcPrecomp(f64),
    SetTxCfcBand {
        band: usize,
        gain_db: f64,
    },
    SetTxPhaseRotatorEnabled(bool),
    SetTxPhaseRotatorAuto(bool),
    SetTxPhaseRotatorCorner(f64),
    SetPureSignalEnabled(bool),
    SetPureSignalAutoAttenuate(bool),
    SetPureSignalAttenuation(u8),
    ResetPureSignal,
    SetTxTwoToneTest(bool),
    SetTxTwoToneFreq1(f64),
    SetTxTwoToneFreq2(f64),
    SetTxTwoToneLevelDb(f64),
    SetTxTwoToneInvertLsb(bool),
    SetTxTwoToneDelayMs(u16),
    SetTxNoiseGateEnabled(bool),
    SetTxNoiseGateThreshold(f64),
    SetRxFftSize(u32),
    SetRxLowLatency(bool),
    SetTxFftSize(u32),
    SetTxLowLatency(bool),
    MicAudioFrame(TciMicFrame),
    ClientConnected,
    ClientDisconnected,
}

pub(crate) const MAX_TCI_MIC_SAMPLES: usize = 32_768;

pub(crate) fn tx_power_trip_fault_message(forward_watts: f32, limit_watts: f32) -> String {
    format!("tx_fault:0,power_trip,{forward_watts:.1},{limit_watts:.1};")
}

pub(crate) fn tx_uplink_late_fault_message(age_ms: u64, limit_ms: u64) -> String {
    format!("tx_fault:0,uplink_late,{age_ms},{limit_ms};")
}

pub(crate) fn tx_control_watchdog_fault_message(silence_ms: u64, limit_ms: u64) -> String {
    format!("tx_fault:0,control_watchdog,{silence_ms},{limit_ms};")
}

pub(crate) fn tx_codec_decode_fault_message(count: u64, limit: u64) -> String {
    format!("tx_fault:0,codec_decode,count={count},limit={limit};")
}

#[cfg(test)]
pub(crate) fn parse_tci_command(
    command: &str,
    command_tx: &impl TciCommandSink,
    clients: &ClientRegistry,
    client_id: u64,
    allow_control: bool,
) {
    parse_tci_command_with_roles(command, command_tx, clients, client_id, allow_control, None);
}

pub(crate) fn parse_tci_command_with_roles(
    command: &str,
    command_tx: &impl TciCommandSink,
    clients: &ClientRegistry,
    client_id: u64,
    allow_control: bool,
    operator_client_id: Option<&Arc<AtomicU64>>,
) {
    let Some((name, rest)) = command.split_once(':') else {
        return;
    };

    let args: Vec<&str> = rest.split(',').collect();
    let name = name.to_ascii_lowercase();
    if let Some((session_id, role)) = parse_split_session_open(command) {
        if set_client_split_session_open(clients, client_id, &session_id, role) {
            if let Some(operator_client_id) = operator_client_id {
                reconcile_split_operator_role(clients, operator_client_id, client_id);
            }
            let _ = command_tx.send(TciCommand::SplitSessionOpen {
                client_id,
                session_id,
                role,
            });
        }
        return;
    }
    if let Some((session_id, lane)) = parse_split_session_lane(command) {
        if set_client_split_session_lane(clients, client_id, &session_id, lane) {
            if let Some(operator_client_id) = operator_client_id {
                reconcile_split_operator_role(clients, operator_client_id, client_id);
            }
            let _ = command_tx.send(TciCommand::SplitSessionLane {
                client_id,
                session_id,
                lane,
            });
        }
        return;
    }
    if !allow_control && !viewer_tci_command_allowed(&name) {
        return;
    }
    match name.as_str() {
        "vfo" => {
            if args.len() >= 3 {
                if let Ok(freq_hz) = args[2].trim().parse::<u32>() {
                    let which = args[1].trim();
                    let _ = match which {
                        "0" => command_tx.send(TciCommand::SetVfoA(freq_hz)),
                        "1" => command_tx.send(TciCommand::SetVfoB(freq_hz)),
                        _ => Ok(()),
                    };
                }
            }
        }
        "dds" => {
            if args.len() >= 2 {
                if let Ok(freq_hz) = args[1].trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqCenter(freq_hz));
                }
            }
        }
        "saturn_ping" => {
            if args.len() >= 2 {
                let nonce = sanitize_token(args[0], 32);
                let sent_at = sanitize_token(args[1], 32);
                if !nonce.is_empty() && !sent_at.is_empty() {
                    let _ = command_tx.send(TciCommand::SaturnPing {
                        client_id,
                        nonce,
                        sent_at,
                    });
                }
            }
        }
        "modulation" => {
            if args.len() >= 2 {
                let _ = command_tx.send(TciCommand::SetMode(DemodMode::from_tci(args[1])));
            }
        }
        "rx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_volume" => {
            let volume_arg = if args.len() >= 3 {
                args.get(2)
            } else if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(volume_text) = volume_arg {
                if let Ok(volume_db) = volume_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxVolume(volume_db));
                }
            }
        }
        "rx_nr_mode" | "nr_mode" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetRxNoiseReductionMode(
                    NoiseReductionMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nr" | "nr" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionEnabled(enabled));
                }
            }
        }
        "rx_nr_level" | "nr_level" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionLevel(level));
                }
            }
        }
        "rx_nr2_gain_method" => {
            let method_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(method_text) = method_arg {
                let _ = command_tx.send(TciCommand::SetRxNr2GainMethod(Nr2GainMethod::from_tci(
                    method_text,
                )));
            }
        }
        "rx_nr2_npe_method" => {
            let method_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(method_text) = method_arg {
                let _ = command_tx.send(TciCommand::SetRxNr2NpeMethod(Nr2NpeMethod::from_tci(
                    method_text,
                )));
            }
        }
        "rx_nr2_post_filter" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxNr2PostFilterEnabled(enabled));
                }
            }
        }
        "rx_wbfm_deemphasis" => {
            let deemphasis_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(deemphasis_text) = deemphasis_arg {
                let _ = command_tx.send(TciCommand::SetRxWbfmDeemphasis(WbfmDeemphasis::from_tci(
                    deemphasis_text,
                )));
            }
        }
        "rx_anr_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_adc" => {
            let adc_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(adc_text) = adc_arg {
                if let Ok(adc) = adc_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAdc(adc.min(2)));
                }
            }
        }
        "rx_antenna" => {
            let antenna_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(antenna_text) = antenna_arg {
                if let Ok(antenna) = antenna_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAntenna(antenna.clamp(1, 3)));
                }
            }
        }
        "iq_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqSampleRate(rate_hz));
                }
            }
        }
        "iq_start" => {
            set_client_iq_stream_enabled(clients, client_id, true);
            println!("saturn-bridge: TCI iq_start requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "iq_stop" => {
            set_client_iq_stream_enabled(clients, client_id, false);
            println!("saturn-bridge: TCI iq_stop requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "audio_start" => {
            let audio_enabled = set_client_audio_stream_enabled(clients, client_id, true);
            println!("saturn-bridge: TCI audio_start requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(audio_enabled));
        }
        "audio_stop" => {
            let audio_enabled = set_client_audio_stream_enabled(clients, client_id, false);
            println!("saturn-bridge: TCI audio_stop requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(audio_enabled));
        }
        "audio_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    set_client_audio_sample_rate(clients, client_id, rate_hz);
                    let _ = command_tx.send(TciCommand::SetAudioSampleRate(rate_hz));
                }
            }
        }
        "audio_stream_samples" => {
            if let Some(sample_text) = args.first() {
                if let Ok(sample_count) = sample_text.trim().parse::<u32>() {
                    set_client_audio_frame_float_count(clients, client_id, sample_count);
                    let _ = command_tx.send(TciCommand::SetAudioFrameSamples(sample_count));
                }
            }
        }
        "audio_stream_channels" => {
            if let Some(channel_text) = args.first() {
                if let Ok(channels) = channel_text.trim().parse::<u32>() {
                    set_client_audio_channels(clients, client_id, channels);
                    let _ = command_tx.send(TciCommand::SetAudioChannels(channels));
                }
            }
        }
        "audio_seq_gap_count" => {
            let gap_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gap_text) = gap_arg {
                if let Ok(gaps) = gap_text.trim().parse::<u64>() {
                    set_client_audio_seq_gap_count(clients, client_id, gaps);
                }
            }
        }
        "tx_uplink_stats" => {
            let offset = if args.len() >= 6 { 1 } else { 0 };
            let degraded = args
                .get(offset)
                .and_then(|value| parse_tci_bool(value))
                .unwrap_or(false);
            let last_seq = args
                .get(offset + 1)
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let dropped_count = args
                .get(offset + 2)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let buffered_bytes = args
                .get(offset + 3)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let high_watermark_bytes = args
                .get(offset + 4)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(buffered_bytes);
            set_client_tx_uplink_stats(
                clients,
                client_id,
                degraded,
                last_seq,
                dropped_count,
                buffered_bytes,
                high_watermark_bytes,
            );
        }
        "tx_codec_caps" => {
            let caps = parse_tx_codec_caps_args(&args);
            let selected = set_client_tx_codec_caps(clients, client_id, caps.clone());
            println!(
                "saturn-bridge: TCI client {client_id} tx_codec_caps={:?} selected={:?}",
                caps, selected
            );
            if let Some(codec) = selected {
                send_text_to_client(clients, client_id, tx_codec_accept_message(codec));
            } else {
                let requested = caps.iter().next().copied().unwrap_or(TxMicCodec::Pcm);
                send_text_to_client(
                    clients,
                    client_id,
                    tx_codec_reject_message(requested, "unsupported"),
                );
            }
        }
        "remote_tx_media_priority" => {
            // TX media priority is derived from the bridge's
            // authoritative on-air state, not from this browser command.
            // Accept and ignore for backward compatibility with older
            // clients; no side effect.
            let _ = args;
        }
        "audio_stream_sample_type" => {}
        "rx_smeter" | "s_meter" | "smeter" => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
        }
        "trx" => {
            // trx:0,true or trx:0,true,tci — PTT on/off
            // This no longer sets a per-client tx_media_priority flag.
            // The bridge derives media priority from its own model and main.rs
            // calls tci.set_tx_media_priority_active(...) accordingly. This
            // eliminates the stuck-flag bug class.
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    println!("saturn-bridge: TCI trx requested -> {}", enabled);
                    if enabled {
                        reset_client_tx_uplink_attempt(clients, client_id);
                    }
                    let _ = command_tx.send(TciCommand::SetTxEnabled(enabled));
                }
            }
        }
        "rx_nb" => {
            // rx_nb:0,mode — NB mode: 0=OFF, 1=NB1, 2=NB2
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetNoiseBlankerMode(
                    NoiseBlankerMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nb_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh) = thresh_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetNoiseBlankerThreshold(thresh));
                }
            }
        }
        "rx_anf" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetAnfEnabled(enabled));
                }
            }
        }
        "rx_anf_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_agc" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetAgcMode(AgcMode::from_tci(mode_text)));
            }
        }
        "rx_agc_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetAgcGain(gain));
                }
            }
        }
        "tx_drive" => {
            let drive_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(drive_text) = drive_arg {
                if let Ok(drive) = drive_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetTxDrive(drive));
                }
            }
        }
        "tx_mic_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain_db) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxMicGain(gain_db));
                }
            }
        }
        "tx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetTxFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxEqEnabled(enabled));
                }
            }
        }
        "tx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxEqEnabled(enabled));
                }
            }
        }
        "rx_eq_band" => {
            // rx_eq_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB integers
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetRxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_eq_band" => {
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_cfc_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxCfcEnabled(enabled));
                }
            }
        }
        "tx_cfc_precomp" => {
            let precomp_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(precomp_text) = precomp_arg {
                if let Ok(db) = precomp_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxCfcPrecomp(db));
                }
            }
        }
        "tx_cfc_band" => {
            // tx_cfc_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<f64>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxCfcBand { band, gain_db });
                    }
                }
            }
        }
        "tx_phase_rotator" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxPhaseRotatorEnabled(enabled));
                }
            }
        }
        "tx_phase_rotator_auto" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxPhaseRotatorAuto(enabled));
                }
            }
        }
        "tx_phase_rotator_corner" => {
            let corner_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(corner_text) = corner_arg {
                if let Ok(corner_hz) = corner_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxPhaseRotatorCorner(corner_hz));
                }
            }
        }
        "tx_puresignal" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetPureSignalEnabled(enabled));
                }
            }
        }
        "tx_puresignal_auto_attenuate" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetPureSignalAutoAttenuate(enabled));
                }
            }
        }
        "tx_puresignal_attenuation" => {
            let attenuation_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(attenuation_text) = attenuation_arg {
                if let Ok(attenuation) = attenuation_text.trim().parse::<u8>() {
                    let _ =
                        command_tx.send(TciCommand::SetPureSignalAttenuation(attenuation.min(31)));
                }
            }
        }
        "tx_puresignal_reset" => {
            let _ = command_tx.send(TciCommand::ResetPureSignal);
        }
        "tx_two_tone" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneTest(enabled));
                }
            }
        }
        "tx_two_tone_freq1" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq1(freq_hz));
                }
            }
        }
        "tx_two_tone_freq2" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq2(freq_hz));
                }
            }
        }
        "tx_two_tone_level_db" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level_db) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneLevelDb(level_db));
                }
            }
        }
        "tx_two_tone_invert_lsb" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneInvertLsb(enabled));
                }
            }
        }
        "tx_two_tone_delay_ms" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay_ms) = delay_text.trim().parse::<u16>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneDelayMs(delay_ms));
                }
            }
        }
        "tx_noise_gate" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateEnabled(enabled));
                }
            }
        }
        "tx_noise_gate_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh_db) = thresh_text.trim().parse::<f64>() {
                    let clamped = thresh_db.clamp(-80.0, 0.0);
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateThreshold(clamped));
                }
            }
        }
        "rx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetRxFftSize(size));
                }
            }
        }
        "rx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetRxLowLatency(val));
                }
            }
        }
        "tx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetTxFftSize(size));
                }
            }
        }
        "tx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetTxLowLatency(val));
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn viewer_tci_command_allowed(name: &str) -> bool {
    matches!(
        name,
        "saturn_ping"
            | "iq_start"
            | "iq_stop"
            | "audio_start"
            | "audio_stop"
            | "audio_samplerate"
            | "audio_stream_samples"
            | "audio_stream_channels"
            | "audio_seq_gap_count"
            | "audio_stream_sample_type"
            | "rx_smeter"
            | "s_meter"
            | "smeter"
    )
}

pub(crate) fn parse_tx_codec_caps_args(args: &[&str]) -> BTreeSet<TxMicCodec> {
    let offset = if args
        .first()
        .map(|value| value.trim() == "0")
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    args.iter()
        .skip(offset)
        .filter_map(|value| TxMicCodec::from_tci(value))
        .collect()
}

pub(crate) fn select_tx_codec(
    caps: &BTreeSet<TxMicCodec>,
    flags: TxCodecRuntimeFlags,
) -> Option<TxMicCodec> {
    if flags.opus_decode_enabled {
        if caps.contains(&TxMicCodec::OpusWb) {
            return Some(TxMicCodec::OpusWb);
        }
        if caps.contains(&TxMicCodec::OpusNb) {
            return Some(TxMicCodec::OpusNb);
        }
    }
    caps.contains(&TxMicCodec::Pcm).then_some(TxMicCodec::Pcm)
}

pub(crate) fn tx_codec_accept_message(codec: TxMicCodec) -> String {
    format!("tx_codec_accept:0,{};", codec.as_tci())
}

pub(crate) fn tx_codec_reject_message(codec: TxMicCodec, reason: &str) -> String {
    format!(
        "tx_codec_reject:0,{},{};",
        codec.as_tci(),
        sanitize_token(reason, 48)
    )
}

/// Parse a TCI binary frame that contains TX mic audio from the client.
/// Frame layout: 64-byte header + LE samples.
///   header[8..12]  = sample_type  (u32 LE); 1=s16, 3=float32, 0=legacy float32
///   header[20..24] = sample_count (u32 LE)
///   header[24..28] = stream_type  (u32 LE); must be 2 (TX mic)
///   header[28..32] = channels     (u32 LE); 1=mono, 2=stereo
///   header[32..36] = tx_mic_seq   (u32 LE); 0 means legacy/unknown
///   header[36..40] = codec_id     (u32 LE); 0=PCM, other values select Opus
///   header[40..44] = payload_bytes (u32 LE); 0 means legacy/full payload
///
/// stream_type == 1 is intentionally excluded: it is the RX audio type used by
/// the server→client direction and must not be fed into the TX DSP path.
#[cfg(test)]
pub(crate) fn parse_tci_mic_frame(data: &[u8]) -> Option<TciMicFrame> {
    parse_tci_mic_frame_result(data).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TciMicFrameParseError {
    NotMicFrame,
    Malformed,
    UnsupportedCodec,
    Decode(TxDecodeError),
}

pub(crate) struct TciMicFrameParts<'a> {
    pub(crate) sample_rate_hz: u32,
    pub(crate) sample_type: u32,
    pub(crate) channels: u32,
    pub(crate) sequence: u32,
    pub(crate) codec: TxMicCodec,
    pub(crate) sample_count: usize,
    pub(crate) payload: &'a [u8],
    pub(crate) declared_payload_bytes: usize,
}

pub(crate) fn parse_tci_mic_frame_parts(
    data: &[u8],
) -> Result<TciMicFrameParts<'_>, TciMicFrameParseError> {
    if data.len() < 64 {
        return Err(TciMicFrameParseError::Malformed);
    }
    let sample_rate_hz = u32::from_le_bytes(
        data[4..8]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let sample_type = u32::from_le_bytes(
        data[8..12]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let stream_type = u32::from_le_bytes(
        data[24..28]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    if stream_type != 2 {
        return Err(TciMicFrameParseError::NotMicFrame);
    }
    let raw_channels = u32::from_le_bytes(
        data[28..32]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let channels = match raw_channels {
        0 | 1 => 1,
        _ => 2,
    };
    let sequence = u32::from_le_bytes(
        data[32..36]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let codec_id = u32::from_le_bytes(
        data[36..40]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    );
    let codec = TxMicCodec::from_id(codec_id).ok_or(TciMicFrameParseError::UnsupportedCodec)?;
    let declared_payload_bytes = u32::from_le_bytes(
        data[40..44]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    ) as usize;
    let sample_count = u32::from_le_bytes(
        data[20..24]
            .try_into()
            .map_err(|_| TciMicFrameParseError::Malformed)?,
    ) as usize;
    if sample_count == 0 || sample_count > MAX_TCI_MIC_SAMPLES {
        return Err(TciMicFrameParseError::Malformed);
    }
    let raw_payload = &data[64..];
    if declared_payload_bytes > raw_payload.len() {
        return Err(TciMicFrameParseError::Malformed);
    }
    let payload = if declared_payload_bytes == 0 {
        raw_payload
    } else {
        &raw_payload[..declared_payload_bytes]
    };
    Ok(TciMicFrameParts {
        sample_rate_hz,
        sample_type,
        channels,
        sequence,
        codec,
        sample_count,
        payload,
        declared_payload_bytes,
    })
}

pub(crate) fn decode_tci_mic_frame_parts(
    parts: TciMicFrameParts<'_>,
    decoder: &mut TxCodecDecoder,
) -> Result<TciMicFrame, TciMicFrameParseError> {
    if decoder.codec() != parts.codec {
        return Err(TciMicFrameParseError::Decode(TxDecodeError::CodecMismatch));
    }
    let samples = decoder
        .decode(
            parts.sample_type,
            parts.sample_count,
            parts.payload,
            parts.declared_payload_bytes,
        )
        .map_err(TciMicFrameParseError::Decode)?;
    Ok(TciMicFrame {
        sample_rate_hz: parts.sample_rate_hz,
        channels: parts.channels,
        sequence: parts.sequence,
        received_at: Instant::now(),
        samples,
    })
}

#[cfg(test)]
pub(crate) fn parse_tci_mic_frame_result(
    data: &[u8],
) -> Result<TciMicFrame, TciMicFrameParseError> {
    let parts = parse_tci_mic_frame_parts(data)?;
    let mut decoder = TxCodecDecoder::new(parts.codec);
    decode_tci_mic_frame_parts(parts, &mut decoder)
}

pub(crate) fn parse_tci_mic_frame_result_for_client(
    clients: &ClientRegistry,
    client_id: u64,
    data: &[u8],
) -> Result<TciMicFrame, TciMicFrameParseError> {
    let parts = parse_tci_mic_frame_parts(data)?;
    let decoder = {
        let clients = clients.lock_unpoisoned();
        let Some(client) = clients.get(&client_id) else {
            return Err(TciMicFrameParseError::Malformed);
        };
        if client.state.tx_codec_active != parts.codec {
            return Err(TciMicFrameParseError::Decode(TxDecodeError::CodecMismatch));
        }
        client.state.tx_codec_decoder.clone()
    };
    let mut decoder = decoder.lock_unpoisoned();
    decode_tci_mic_frame_parts(parts, &mut decoder)
}

pub(crate) fn build_tci_iq_frame(receiver: u32, sample_rate: u32, iq_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 0, 2, 0)
}

pub(crate) fn build_tci_tx_iq_frame(
    receiver: u32,
    sample_rate: u32,
    iq_samples: &[f32],
) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 3, 2, 0)
}

pub(crate) fn build_tci_audio_frame(
    receiver: u32,
    sample_rate: u32,
    channels: u32,
    audio_samples: &[f32],
    sequence: u32,
) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, audio_samples, 1, channels, sequence)
}

pub(crate) fn build_tci_float_frame(
    receiver: u32,
    sample_rate: u32,
    samples: &[f32],
    stream_type: u32,
    channels: u32,
    sequence: u32,
) -> Vec<u8> {
    let mut frame = vec![0u8; 64 + samples.len() * 4];
    write_u32_le(&mut frame, 0, receiver);
    write_u32_le(&mut frame, 4, sample_rate);
    write_u32_le(&mut frame, 8, 3);
    write_u32_le(&mut frame, 12, 0);
    write_u32_le(&mut frame, 16, 0);
    write_u32_le(&mut frame, 20, samples.len() as u32);
    write_u32_le(&mut frame, 24, stream_type);
    write_u32_le(&mut frame, 28, channels);
    write_u32_le(&mut frame, 32, sequence);

    for (index, value) in samples.iter().enumerate() {
        let offset = 64 + index * 4;
        frame[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    frame
}

pub(crate) fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn parse_tci_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

pub(crate) fn sanitize_token(text: &str, max_len: usize) -> String {
    text.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .take(max_len)
        .collect()
}

/// Convert a Saturn G2 raw power ADC reading to watts.
/// Uses the ANAN-7000/Saturn 100 W PA calibration constants from pihpsdr:
///   ADC_REF = 5.0 V, coupling = 0.12 (fwd) / 0.12 (rev), fwd_offset = 32, rev_offset = 28.
/// Formula: V = ((raw - offset) / 4095) * 5.0;  watts = V² / 0.12
pub(crate) fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (raw as i32 - offset).max(0) as f32;
    let v = (corrected / 4095.0) * 5.0;
    ((v * v) / 0.12) * scale
}

pub(crate) fn calculate_swr_watts(fwd_watts: f32, rev_watts: f32) -> f32 {
    if fwd_watts <= 0.0 || rev_watts <= 0.0 || rev_watts >= fwd_watts {
        return 1.0;
    }
    let ratio = (rev_watts / fwd_watts).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}

#[allow(dead_code)]
pub(crate) fn calculate_swr(forward: u16, reverse: u16) -> f32 {
    if forward == 0 || reverse == 0 || reverse >= forward {
        return 1.0;
    }

    let ratio = (reverse as f32 / forward as f32).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}
