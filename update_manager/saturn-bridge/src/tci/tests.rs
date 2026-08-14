use super::*;
use crate::radio_model::{DemodMode, Nr2GainMethod, Nr2NpeMethod, WbfmDeemphasis};
use crate::tx_codec::{TxCodecDecoder, TxDecodeError, TxMicCodec};
use crate::tx_codec::{
    TX_MIC_CODEC_OPUS_WB_ID, TX_MIC_CODEC_PCM_ID, TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
    TX_OPUS_WB_TEST_PACKET, TX_SAMPLE_TYPE_FLOAT32, TX_SAMPLE_TYPE_S16,
};
use tungstenite::Message;

fn test_client_registry(client_id: u64) -> ClientRegistry {
    let mut clients = BTreeMap::new();
    clients.insert(
        client_id,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    Arc::new(Mutex::new(clients))
}

fn opus_wb_runtime_available() -> bool {
    let mut decoder = TxCodecDecoder::new_with_flags(
        TxMicCodec::OpusWb,
        TxCodecRuntimeFlags {
            opus_decode_enabled: true,
        },
    );
    matches!(
        decoder.decode(
            TX_SAMPLE_TYPE_S16,
            TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
            &TX_OPUS_WB_TEST_PACKET,
            TX_OPUS_WB_TEST_PACKET.len(),
        ),
        Ok(_)
    )
}

#[test]
fn builds_iq_frame_with_expected_header() {
    let frame = build_tci_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
    assert_eq!(frame.len(), 64 + 16);
    assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
    assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 0);
}

#[test]
fn builds_tx_iq_frame_with_distinct_stream_type() {
    let frame = build_tci_tx_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
    assert_eq!(frame.len(), 64 + 16);
    assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
    assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 2);
}

#[test]
fn builds_audio_frame_with_expected_header() {
    let frame = build_tci_audio_frame(0, 48_000, 1, &[0.25, -0.25, 0.5, -0.5], 7);
    assert_eq!(frame.len(), 64 + 16);
    assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 48_000);
    assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(frame[32..36].try_into().unwrap()), 7);
}

#[test]
fn shapes_rx_audio_for_wan_transport() {
    let input = [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0];
    let (rate, channels, output) = shape_rx_audio_for_transport(&input, 48_000, 2, 12_000, 1);
    assert_eq!(rate, 12_000);
    assert_eq!(channels, 1);
    assert_eq!(output.len(), 1);
    assert!((output[0] - 0.5).abs() < 0.001);
}

#[test]
fn per_client_audio_profile_honors_request_and_service_caps() {
    assert_eq!(
        effective_rx_audio_transport_profile(48_000, 2, 48_000, 48_000, 2),
        (48_000, 2)
    );
    assert_eq!(
        effective_rx_audio_transport_profile(12_000, 1, 48_000, 48_000, 2),
        (12_000, 1)
    );
    assert_eq!(
        effective_rx_audio_transport_profile(48_000, 2, 48_000, 24_000, 1),
        (24_000, 1)
    );
}

#[test]
fn mixed_clients_receive_independent_lan_and_wan_audio_shapes() {
    let clients = test_client_registry(1);
    {
        let mut clients = clients.lock_unpoisoned();
        let lan = clients.get_mut(&1).unwrap();
        lan.state.audio_stream_enabled = true;
        lan.state.audio_sample_rate_hz = 48_000;
        lan.state.audio_channels = 2;
        clients.insert(
            2,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState {
                    audio_stream_enabled: true,
                    audio_sample_rate_hz: 12_000,
                    audio_channels: 1,
                    ..ClientState::default()
                },
            },
        );
    }

    let source = vec![0.25; 2_048];
    assert_eq!(
        enqueue_rx_audio_for_clients(&clients, 48_000, &source, 48_000, 2, false),
        0
    );

    let clients = clients.lock_unpoisoned();
    let lan = clients
        .get(&1)
        .unwrap()
        .outbound
        .queues
        .lock_unpoisoned()
        .audio
        .front()
        .unwrap()
        .message
        .clone();
    let wan = clients
        .get(&2)
        .unwrap()
        .outbound
        .queues
        .lock_unpoisoned()
        .audio
        .front()
        .unwrap()
        .message
        .clone();

    match lan {
        OutboundMessage::AudioFrame {
            sample_rate,
            channels,
            audio_samples,
            ..
        } => {
            assert_eq!(sample_rate, 48_000);
            assert_eq!(channels, 2);
            assert_eq!(audio_samples.len(), 2_048);
        }
        _ => panic!("expected LAN audio frame"),
    }
    match wan {
        OutboundMessage::AudioFrame {
            sample_rate,
            channels,
            audio_samples,
            ..
        } => {
            assert_eq!(sample_rate, 12_000);
            assert_eq!(channels, 1);
            assert_eq!(audio_samples.len(), 256);
        }
        _ => panic!("expected WAN audio frame"),
    }
}

#[test]
fn outbound_scheduler_prioritizes_safety_and_control_over_display() {
    let outbound = ClientOutbound::new();
    outbound.enqueue(OutboundMessage::IqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    });
    outbound.enqueue(OutboundMessage::Text("rx_smeter:0,0,-110.0;".to_string()));
    outbound.enqueue(OutboundMessage::SafetyText(
        "tx_fault:0,power_trip,126.3,110.0;".to_string(),
    ));

    let safety = outbound.next_message(true).unwrap();
    assert_eq!(safety.class, OutboundClass::Safety);
    let control = outbound.next_message(true).unwrap();
    assert_eq!(control.class, OutboundClass::Control);
    let display = outbound.next_message(true).unwrap();
    assert_eq!(display.class, OutboundClass::Display);
}

#[test]
fn outbound_scheduler_treats_snapshot_rf_state_as_control() {
    assert_eq!(
        OutboundMessage::Text("remote_tx_rf_enabled:0,false;".to_string()).class(),
        OutboundClass::Control
    );
    assert_eq!(
        OutboundMessage::SafetyText("remote_tx_rf_enabled:0,false;".to_string()).class(),
        OutboundClass::Safety
    );
}

#[test]
fn outbound_scheduler_replaces_display_depth_one() {
    let outbound = ClientOutbound::new();
    assert_eq!(
        outbound.enqueue(OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 48_000,
            iq_samples: vec![1.0, 2.0],
        }),
        0
    );
    assert_eq!(
        outbound.enqueue(OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: 96_000,
            iq_samples: vec![3.0, 4.0],
        }),
        1
    );
    let item = outbound.next_message(true).unwrap();
    match item.message {
        OutboundMessage::IqFrame { sample_rate, .. } => assert_eq!(sample_rate, 96_000),
        _ => panic!("expected display frame"),
    }
    let delta = outbound.drain_stats();
    assert_eq!(delta.display_replaced, 1);
}

#[test]
fn outbound_scheduler_coalesces_control_state_and_keeps_latest() {
    let outbound = ClientOutbound::new();
    assert_eq!(
        outbound.enqueue(OutboundMessage::Text("vfo:0,0,7100000;".to_string())),
        0
    );
    assert_eq!(
        outbound.enqueue(OutboundMessage::Text("vfo:0,0,7200000;".to_string())),
        1
    );
    let item = outbound.next_message(true).unwrap();
    assert!(matches!(
        item.message,
        OutboundMessage::Text(text) if text == "vfo:0,0,7200000;"
    ));
    let delta = outbound.drain_stats();
    assert_eq!(delta.control_replaced, 1);
}

#[test]
fn outbound_scheduler_bounds_unique_control_messages() {
    let outbound = ClientOutbound::new();
    for index in 0..(MAX_CONTROL_QUEUE_MESSAGES + 10) {
        outbound.enqueue(OutboundMessage::Text(format!(
            "test_metric_{index}:0,{index};"
        )));
    }
    let mut retained = 0;
    while outbound.next_message(true).is_some() {
        retained += 1;
    }
    assert_eq!(retained, MAX_CONTROL_QUEUE_MESSAGES);
    let delta = outbound.drain_stats();
    assert_eq!(delta.control_dropped, 10);
    assert_eq!(
        delta.control_queue_high_watermark,
        MAX_CONTROL_QUEUE_MESSAGES as u64
    );
}

#[test]
fn bridge_connection_slots_are_globally_bounded() {
    let active = AtomicU64::new(0);
    let high_watermark = AtomicU64::new(0);
    for _ in 0..MAX_TCI_CONNECTIONS {
        assert!(try_reserve_connection_slot(&active, &high_watermark));
    }
    assert!(!try_reserve_connection_slot(&active, &high_watermark));
    assert_eq!(active.load(Ordering::Relaxed), MAX_TCI_CONNECTIONS);
    assert_eq!(high_watermark.load(Ordering::Relaxed), MAX_TCI_CONNECTIONS);
}

#[test]
fn outbound_scheduler_panic_drains_stale_audio() {
    let outbound = ClientOutbound::new();
    let audio = vec![0.0; max_audio_queued_frames(8_000) * 2];
    assert_eq!(
        outbound.enqueue(OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 8_000,
            channels: 2,
            audio_samples: audio.clone(),
            sequence: 0,
        }),
        0
    );
    assert_eq!(
        outbound.enqueue(OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 8_000,
            channels: 2,
            audio_samples: audio,
            sequence: 0,
        }),
        1
    );
    let item = outbound.next_message(true).unwrap();
    match item.message {
        OutboundMessage::AudioFrame { sequence, .. } => assert_eq!(sequence, 2),
        _ => panic!("expected audio frame"),
    }
    let delta = outbound.drain_stats();
    assert_eq!(delta.audio_panic_drain, 1);
    assert_eq!(delta.audio_dropped, 1);
}

#[test]
fn bulk_tcp_outq_guard_blocks_bulk_at_limit() {
    assert!(bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES - 1));
    assert!(!bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES));
    assert!(!bulk_allowed_for_tcp_outq(BULK_TCP_OUTQ_LIMIT_BYTES * 2));
}

#[test]
fn outbound_scheduler_tracks_tcp_outq_high_watermark() {
    let outbound = ClientOutbound::new();
    outbound.record_tcp_outq_high_watermark(32 * 1024);
    outbound.record_tcp_outq_high_watermark(96 * 1024);
    outbound.record_tcp_outq_high_watermark(64 * 1024);
    let delta = outbound.drain_stats();
    assert_eq!(delta.tcp_outq_high_watermark_bytes, 96 * 1024);
}

#[test]
fn display_frame_interval_supports_limit_and_disable() {
    assert_eq!(display_frame_interval_for_limit(0), Duration::ZERO);
    assert_eq!(
        display_frame_interval_for_limit(25),
        Duration::from_millis(40)
    );
    assert_eq!(
        display_frame_interval_for_limit(50),
        Duration::from_millis(20)
    );
}

#[test]
fn rejects_oversized_tci_mic_frames() {
    let sample_count = (MAX_TCI_MIC_SAMPLES + 1) as u32;
    let mut frame = vec![0u8; 64];
    write_u32_le(&mut frame, 20, sample_count);
    write_u32_le(&mut frame, 24, 2);
    frame.resize(64 + sample_count as usize * 4, 0);
    assert!(parse_tci_mic_frame(&frame).is_none());
}

#[test]
fn parses_mono_tci_mic_frame_with_channel_metadata() {
    let mut frame = vec![0u8; 64 + 8];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 20, 2);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 32, 77);
    frame[64..68].copy_from_slice(&0.25f32.to_le_bytes());
    frame[68..72].copy_from_slice(&(-0.5f32).to_le_bytes());

    let parsed = parse_tci_mic_frame(&frame).unwrap();
    assert_eq!(parsed.sample_rate_hz, 48_000);
    assert_eq!(parsed.channels, 1);
    assert_eq!(parsed.sequence, 77);
    assert_eq!(parsed.samples, vec![0.25, -0.5]);
}

#[test]
fn parses_stereo_tci_mic_frame_with_channel_metadata() {
    let mut frame = vec![0u8; 64 + 16];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_FLOAT32);
    write_u32_le(&mut frame, 20, 4);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 2);
    for (index, sample) in [0.25f32, -0.25, 0.5, -0.5].iter().enumerate() {
        let offset = 64 + index * 4;
        frame[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
    }

    let parsed = parse_tci_mic_frame(&frame).unwrap();
    assert_eq!(parsed.sample_rate_hz, 48_000);
    assert_eq!(parsed.channels, 2);
    assert_eq!(parsed.sequence, 0);
    assert_eq!(parsed.samples, vec![0.25, -0.25, 0.5, -0.5]);
}

#[test]
fn parses_s16_tci_mic_frame_with_channel_metadata() {
    let mut frame = vec![0u8; 64 + 6];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 3);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 32, 78);
    for (index, sample) in [8192i16, -16384, 32767].iter().enumerate() {
        let offset = 64 + index * 2;
        frame[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
    }

    let parsed = parse_tci_mic_frame(&frame).unwrap();
    assert_eq!(parsed.sample_rate_hz, 48_000);
    assert_eq!(parsed.channels, 1);
    assert_eq!(parsed.sequence, 78);
    assert_eq!(parsed.samples.len(), 3);
    assert!((parsed.samples[0] - 0.25).abs() < 0.0001);
    assert!((parsed.samples[1] + 0.5).abs() < 0.0001);
    assert!((parsed.samples[2] - 0.9999).abs() < 0.0001);
}

#[test]
fn parses_pcm_tci_mic_frame_with_codec_header() {
    let mut frame = vec![0u8; 64 + 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 2);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 32, 79);
    write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
    write_u32_le(&mut frame, 40, 4);
    for (index, sample) in [8192i16, -16384].iter().enumerate() {
        let offset = 64 + index * 2;
        frame[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
    }

    let parsed = parse_tci_mic_frame(&frame).unwrap();
    assert_eq!(parsed.sample_rate_hz, 48_000);
    assert_eq!(parsed.channels, 1);
    assert_eq!(parsed.sequence, 79);
    assert_eq!(parsed.samples.len(), 2);
    assert!((parsed.samples[0] - 0.25).abs() < 0.0001);
    assert!((parsed.samples[1] + 0.5).abs() < 0.0001);
}

#[test]
fn rejects_mic_frame_with_unsupported_codec() {
    let mut frame = vec![0u8; 64 + 2];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 1);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 36, 2);
    write_u32_le(&mut frame, 40, 2);

    assert!(parse_tci_mic_frame(&frame).is_none());
}

#[test]
fn rejects_pcm_mic_frame_with_payload_size_mismatch() {
    let mut frame = vec![0u8; 64 + 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 2);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
    write_u32_le(&mut frame, 40, 2);

    assert!(parse_tci_mic_frame(&frame).is_none());
}

#[test]
fn rejects_unknown_tci_mic_sample_type() {
    let mut frame = vec![0u8; 64 + 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, 99);
    write_u32_le(&mut frame, 20, 1);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);

    assert!(parse_tci_mic_frame(&frame).is_none());
}

#[test]
fn websocket_config_limits_inbound_message_size() {
    let config = tci_websocket_config();
    assert_eq!(config.max_message_size, Some(MAX_TCI_INBOUND_MESSAGE_BYTES));
    assert_eq!(config.max_frame_size, Some(MAX_TCI_INBOUND_FRAME_BYTES));
}

#[test]
fn swr_formula_is_reasonable() {
    assert!((calculate_swr(1000, 0) - 1.0).abs() < 0.01);
    assert!(calculate_swr(1000, 250) > 1.0);
}

#[test]
fn parses_boolish_tci_values() {
    assert_eq!(parse_tci_bool("true"), Some(true));
    assert_eq!(parse_tci_bool("0"), Some(false));
    assert_eq!(parse_tci_bool("bogus"), None);
}

#[test]
fn parses_saturn_ping_command() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    parse_tci_command("saturn_ping:probe-1,123.456;", &tx, &clients, 7, false);

    match rx.try_recv().unwrap() {
        TciCommand::SaturnPing {
            client_id,
            nonce,
            sent_at,
        } => {
            assert_eq!(client_id, 7);
            assert_eq!(nonce, "probe-1");
            assert_eq!(sent_at, "123.456");
        }
        command => panic!("unexpected command: {command:?}"),
    }
}

#[test]
fn parses_wdsp2_nr2_wbfm_and_phase_rotator_commands() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    for command in [
        "rx_nr2_gain_method:0,TRAINED",
        "rx_nr2_npe_method:0,NSTAT",
        "rx_nr2_post_filter:0,false",
        "rx_wbfm_deemphasis:0,EU_50US",
        "modulation:0,WFM",
        "tx_phase_rotator:0,true",
        "tx_phase_rotator_auto:0,true",
        "tx_phase_rotator_corner:0,425",
    ] {
        parse_tci_command(command, &tx, &clients, 7, true);
    }

    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxNr2GainMethod(Nr2GainMethod::Trained)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxNr2NpeMethod(Nr2NpeMethod::Nstat)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxNr2PostFilterEnabled(false)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxWbfmDeemphasis(WbfmDeemphasis::Europe)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetMode(DemodMode::Wfm)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxPhaseRotatorEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxPhaseRotatorAuto(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxPhaseRotatorCorner(value) if value == 425.0
    ));
}

#[test]
fn parses_puresignal_control_commands() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    for command in [
        "tx_puresignal:0,true",
        "tx_puresignal_auto_attenuate:0,false",
        "tx_puresignal_attenuation:0,12",
        "tx_puresignal_reset:0",
    ] {
        parse_tci_command(command, &tx, &clients, 7, true);
    }

    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetPureSignalEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetPureSignalAutoAttenuate(false)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetPureSignalAttenuation(12)
    ));
    assert!(matches!(rx.recv().unwrap(), TciCommand::ResetPureSignal));
}

#[test]
fn tx_codec_caps_accepts_pcm_scaffold() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    parse_tci_command("tx_codec_caps:0,pcm;", &tx, &clients, 7, true);

    assert!(rx.try_recv().is_err());
    let outbound = {
        let clients = clients.lock_unpoisoned();
        let client = clients.get(&7).unwrap();
        assert!(client.state.tx_codec_caps.contains(&TxMicCodec::Pcm));
        assert_eq!(client.state.tx_codec_active, TxMicCodec::Pcm);
        assert!(client.state.tx_codec_negotiated_at.is_some());
        client.outbound.clone()
    };
    let queued = outbound.next_message(true).unwrap();
    match queued.message {
        OutboundMessage::Text(text) => assert_eq!(text, "tx_codec_accept:0,pcm;"),
        other => panic!("unexpected outbound: {other:?}"),
    }
}

#[test]
fn tx_codec_caps_mirror_from_control_to_paired_media() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(73);
    clients.lock_unpoisoned().insert(
        74,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );

    parse_tci_command("session_lane:phase-44,control;", &tx, &clients, 73, true);
    parse_tci_command("session_lane:phase-44,media;", &tx, &clients, 74, false);
    while rx.try_recv().is_ok() {}

    parse_tci_command("tx_codec_caps:0,pcm;", &tx, &clients, 73, true);

    let clients = clients.lock_unpoisoned();
    let control = clients.get(&73).unwrap();
    let media = clients.get(&74).unwrap();
    assert_eq!(control.state.tx_codec_active, TxMicCodec::Pcm);
    assert_eq!(media.state.tx_codec_active, TxMicCodec::Pcm);
    assert!(media.state.tx_codec_negotiated_at.is_some());
    assert_eq!(
        media.state.tx_codec_decoder.lock_unpoisoned().codec(),
        TxMicCodec::Pcm
    );
}

#[test]
fn tx_codec_caps_rejects_non_pcm_until_decoder_exists() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    parse_tci_command("tx_codec_caps:0,opus_wb;", &tx, &clients, 7, true);

    assert!(rx.try_recv().is_err());
    let outbound = {
        let clients = clients.lock_unpoisoned();
        let client = clients.get(&7).unwrap();
        assert!(client.state.tx_codec_caps.contains(&TxMicCodec::OpusWb));
        assert_eq!(client.state.tx_codec_active, TxMicCodec::Pcm);
        assert!(client.state.tx_codec_negotiated_at.is_none());
        client.outbound.clone()
    };
    let queued = outbound.next_message(true).unwrap();
    match queued.message {
        OutboundMessage::Text(text) => {
            assert_eq!(text, "tx_codec_reject:0,opus_wb,unsupported;")
        }
        other => panic!("unexpected outbound: {other:?}"),
    }
}

#[test]
fn tx_codec_caps_accepts_opus_only_when_runtime_flag_enabled() {
    let (tx, rx) = mpsc::channel();
    let mut clients_map = BTreeMap::new();
    clients_map.insert(
        7,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            }),
        },
    );
    let clients = Arc::new(Mutex::new(clients_map));

    parse_tci_command("tx_codec_caps:0,opus_wb,pcm;", &tx, &clients, 7, true);

    assert!(rx.try_recv().is_err());
    let outbound = {
        let clients = clients.lock_unpoisoned();
        let client = clients.get(&7).unwrap();
        assert!(client.state.tx_codec_caps.contains(&TxMicCodec::OpusWb));
        assert_eq!(client.state.tx_codec_active, TxMicCodec::OpusWb);
        assert!(client.state.tx_codec_negotiated_at.is_some());
        assert_eq!(
            client.state.tx_codec_decoder.lock_unpoisoned().codec(),
            TxMicCodec::OpusWb
        );
        client.outbound.clone()
    };
    let queued = outbound.next_message(true).unwrap();
    match queued.message {
        OutboundMessage::Text(text) => assert_eq!(text, "tx_codec_accept:0,opus_wb;"),
        other => panic!("unexpected outbound: {other:?}"),
    }
}

#[test]
fn operator_text_updates_control_heartbeat() {
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(7);
    let operator_client_id = Arc::new(AtomicU64::new(7));
    let operator_control_at = Arc::new(Mutex::new(None));

    assert!(handle_incoming_message(
        Message::Text("saturn_ping:probe-1,123.456;".into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        7,
    ));

    assert!(operator_control_at.lock_unpoisoned().is_some());
}

#[test]
fn viewer_text_does_not_update_control_heartbeat() {
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(7);
    let operator_client_id = Arc::new(AtomicU64::new(1));
    let operator_control_at = Arc::new(Mutex::new(None));

    assert!(handle_incoming_message(
        Message::Text("saturn_ping:probe-1,123.456;".into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        7,
    ));

    assert!(operator_control_at.lock_unpoisoned().is_none());
}

#[test]
fn websocket_ping_does_not_update_control_heartbeat() {
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(7);
    let operator_client_id = Arc::new(AtomicU64::new(7));
    let operator_control_at = Arc::new(Mutex::new(None));

    assert!(handle_incoming_message(
        Message::Ping(Vec::new().into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        7,
    ));

    assert!(operator_control_at.lock_unpoisoned().is_none());
}

#[test]
fn formats_tx_power_trip_fault_message() {
    assert_eq!(
        tx_power_trip_fault_message(126.34, 110.0),
        "tx_fault:0,power_trip,126.3,110.0;"
    );
}

#[test]
fn formats_tx_uplink_late_fault_message() {
    assert_eq!(
        tx_uplink_late_fault_message(280, 250),
        "tx_fault:0,uplink_late,280,250;"
    );
}

#[test]
fn formats_tx_control_watchdog_fault_message() {
    assert_eq!(
        tx_control_watchdog_fault_message(620, 500),
        "tx_fault:0,control_watchdog,620,500;"
    );
}

#[test]
fn formats_remote_client_role_message() {
    assert_eq!(
        remote_client_role_message(42, TciClientRole::Operator),
        "remote_client_role:0,operator,42;"
    );
    assert_eq!(
        remote_client_role_message(43, TciClientRole::Viewer),
        "remote_client_role:0,viewer,43;"
    );
}

#[test]
fn split_parses_session_open_and_paired_message() {
    assert_eq!(
        parse_split_session_open("session_open:phase-42,viewer;"),
        Some(("phase-42".to_string(), TciClientRole::Viewer))
    );
    assert_eq!(
        parse_split_session_open("session_open:operator.1;"),
        Some(("operator.1".to_string(), TciClientRole::Operator))
    );
    assert_eq!(parse_split_session_open("saturn_ping:1,2;"), None);
    assert_eq!(
        split_session_paired_message("phase-42"),
        "session_paired:phase-42;"
    );
}

#[test]
fn split_parses_proxy_lane_marker() {
    assert_eq!(
        parse_split_session_lane("session_lane:phase-42,control;"),
        Some(("phase-42".to_string(), SplitSocketKind::Control))
    );
    assert_eq!(
        parse_split_session_lane("session_lane:phase%3A42,media;"),
        Some(("phase3A42".to_string(), SplitSocketKind::Media))
    );
    assert_eq!(SplitSocketKind::Control.as_tci(), "control");
    assert_eq!(
        parse_split_session_lane("session_lane:phase-42,data;"),
        None
    );
    assert_eq!(
        parse_split_session_lane("session_open:phase-42,operator;"),
        None
    );
}

#[test]
fn split_metadata_commands_cross_viewer_filter() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(51);

    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 51, false);
    match rx.try_recv().unwrap() {
        TciCommand::SplitSessionLane {
            client_id,
            session_id,
            lane,
        } => {
            assert_eq!(client_id, 51);
            assert_eq!(session_id, "phase-42");
            assert_eq!(lane, SplitSocketKind::Media);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    parse_tci_command("session_open:phase-42,viewer;", &tx, &clients, 51, false);
    match rx.try_recv().unwrap() {
        TciCommand::SplitSessionOpen {
            client_id,
            session_id,
            role,
        } => {
            assert_eq!(client_id, 51);
            assert_eq!(session_id, "phase-42");
            assert_eq!(role, TciClientRole::Viewer);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert!(rx.try_recv().is_err());
}

#[test]
fn split_metadata_updates_client_state_and_rejects_mismatch() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(52);

    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 52, false);
    parse_tci_command("session_open:phase-42,viewer;", &tx, &clients, 52, false);

    {
        let clients = clients.lock_unpoisoned();
        let split = clients.get(&52).unwrap().state.split.as_ref().unwrap();
        assert_eq!(split.session_id, "phase-42");
        assert_eq!(split.lane, Some(SplitSocketKind::Media));
        assert_eq!(split.role, Some(TciClientRole::Viewer));
    }
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SplitSessionLane { .. })
    ));
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SplitSessionOpen { .. })
    ));

    parse_tci_command(
        "session_lane:other-session,control;",
        &tx,
        &clients,
        52,
        false,
    );
    assert!(rx.try_recv().is_err());
    let clients = clients.lock_unpoisoned();
    let split = clients.get(&52).unwrap().state.split.as_ref().unwrap();
    assert_eq!(split.session_id, "phase-42");
    assert_eq!(split.lane, Some(SplitSocketKind::Media));
}

#[test]
fn split_pairing_status_derives_from_client_metadata() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(61);
    clients.lock_unpoisoned().insert(
        62,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );

    parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 61, true);
    assert_eq!(split_session_pair_for_client(&clients, 61), None);

    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 62, false);
    assert_eq!(
        split_session_pair_for_client(&clients, 62),
        Some(SplitSessionPair {
            session_id: "phase-42".to_string(),
            control_client_id: 61,
            media_client_id: 62,
        })
    );
    {
        let clients = clients.lock_unpoisoned();
        assert_eq!(
            split_lane_client_count(&clients, SplitSocketKind::Control),
            1
        );
        assert_eq!(split_lane_client_count(&clients, SplitSocketKind::Media), 1);
        assert_eq!(split_paired_session_count(&clients), 1);
    }
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SplitSessionLane { client_id: 61, .. })
    ));
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SplitSessionLane { client_id: 62, .. })
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn split_control_lane_reclaims_operator_from_media_lane() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(17);
    clients.lock_unpoisoned().insert(
        18,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(18));

    parse_tci_command_with_roles(
        "session_lane:phase-42,media;",
        &tx,
        &clients,
        18,
        true,
        Some(&operator_client_id),
    );
    parse_tci_command_with_roles(
        "session_lane:phase-42,control;",
        &tx,
        &clients,
        17,
        false,
        Some(&operator_client_id),
    );
    parse_tci_command_with_roles(
        "session_open:phase-42,operator;",
        &tx,
        &clients,
        17,
        false,
        Some(&operator_client_id),
    );
    while rx.try_recv().is_ok() {}

    assert_eq!(operator_client_id.load(Ordering::SeqCst), 17);
    assert!(split_media_client_can_supply_mic(
        &clients,
        17,
        18,
        Instant::now()
    ));

    let (control_outbound, media_outbound) = {
        let clients = clients.lock_unpoisoned();
        (
            clients.get(&17).unwrap().outbound.clone(),
            clients.get(&18).unwrap().outbound.clone(),
        )
    };
    assert!(media_outbound.next_message(true).is_none());
    match control_outbound.next_message(true).unwrap().message {
        OutboundMessage::SafetyText(text) => {
            assert_eq!(text, "remote_client_role:0,operator,17;")
        }
        other => panic!("unexpected control outbound: {other:?}"),
    }
}

#[test]
fn split_paired_media_socket_can_supply_mic_binary() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(71);
    clients.lock_unpoisoned().insert(
        72,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(71));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 71, true);
    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 72, false);
    while rx.try_recv().is_ok() {}

    let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 91);
    assert!(handle_incoming_message(
        Message::Binary(frame.into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        72,
    ));

    match rx.try_recv().unwrap() {
        TciCommand::MicAudioFrame(frame) => {
            assert_eq!(frame.sequence, 91);
            assert_eq!(frame.samples, vec![0.25, -0.25]);
        }
        other => panic!("unexpected command: {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn split_release_window_blocks_paired_media_mic_binary() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(73);
    clients.lock_unpoisoned().insert(
        74,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(73));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 73, true);
    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 74, false);
    while rx.try_recv().is_ok() {}

    let now = Instant::now();
    assert_eq!(
        set_split_media_ignore_until(&clients, 73, Some(now + SPLIT_RELEASE_IGNORE_WINDOW)),
        1
    );
    assert!(!split_media_client_can_supply_mic(&clients, 73, 74, now));
    assert!(split_media_client_can_supply_mic(
        &clients,
        73,
        74,
        now + SPLIT_RELEASE_IGNORE_WINDOW + Duration::from_millis(1)
    ));

    let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 93);
    assert!(handle_incoming_message(
        Message::Binary(frame.into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        74,
    ));
    assert!(rx.try_recv().is_err());

    assert_eq!(set_split_media_ignore_until(&clients, 73, None), 1);
    assert!(split_media_client_can_supply_mic(
        &clients,
        73,
        74,
        Instant::now()
    ));
}

#[test]
fn media_decode_errors_force_rx_and_report_on_control_lane() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(73);
    clients.lock_unpoisoned().insert(
        74,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(73));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command("session_lane:phase-44,control;", &tx, &clients, 73, true);
    parse_tci_command("session_lane:phase-44,media;", &tx, &clients, 74, false);
    while rx.try_recv().is_ok() {}

    let mut frame = vec![0u8; 64 + 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 2);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
    write_u32_le(&mut frame, 40, 2);

    for _ in 0..TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT {
        assert!(handle_incoming_message(
            Message::Binary(frame.clone().into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            74,
        ));
    }

    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
    assert!(rx.try_recv().is_err());

    let (control_outbound, media_outbound) = {
        let clients = clients.lock_unpoisoned();
        let media = clients.get(&74).unwrap();
        assert_eq!(
            media.state.tx_codec_decode_error_count,
            TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT
        );
        assert!(media.state.tx_codec_degraded);
        (
            clients.get(&73).unwrap().outbound.clone(),
            media.outbound.clone(),
        )
    };

    let fault = control_outbound.next_message(true).unwrap();
    match fault.message {
        OutboundMessage::SafetyText(text) => {
            assert_eq!(text, "tx_fault:0,codec_decode,count=10,limit=10;")
        }
        other => panic!("unexpected outbound: {other:?}"),
    }
    assert!(media_outbound.next_message(true).is_none());
}

#[test]
fn media_lane_decodes_opus_mic_frame_when_runtime_flag_enabled() {
    if !opus_wb_runtime_available() {
        return;
    }

    let (tx, rx) = mpsc::channel();
    let mut clients_map = BTreeMap::new();
    clients_map.insert(
        73,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            }),
        },
    );
    clients_map.insert(
        74,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            }),
        },
    );
    let clients = Arc::new(Mutex::new(clients_map));
    let operator_client_id = Arc::new(AtomicU64::new(73));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command("session_lane:phase-44,control;", &tx, &clients, 73, true);
    parse_tci_command("session_lane:phase-44,media;", &tx, &clients, 74, false);
    while rx.try_recv().is_ok() {}
    parse_tci_command("tx_codec_caps:0,opus_wb,pcm;", &tx, &clients, 73, true);

    let mut frame = vec![0u8; 64 + TX_OPUS_WB_TEST_PACKET.len()];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES as u32);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 32, 120);
    write_u32_le(&mut frame, 36, TX_MIC_CODEC_OPUS_WB_ID);
    write_u32_le(&mut frame, 40, TX_OPUS_WB_TEST_PACKET.len() as u32);
    frame[64..].copy_from_slice(&TX_OPUS_WB_TEST_PACKET);

    assert!(handle_incoming_message(
        Message::Binary(frame.into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        74,
    ));

    match rx.try_recv().unwrap() {
        TciCommand::MicAudioFrame(frame) => {
            assert_eq!(frame.sample_rate_hz, 48_000);
            assert_eq!(frame.channels, 1);
            assert_eq!(frame.sequence, 120);
            assert_eq!(frame.samples.len(), TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES);
            assert!(frame.samples.iter().all(|sample| sample.is_finite()));
            let peak = frame
                .samples
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0f32, f32::max);
            assert!(peak > 0.001);
        }
        other => panic!("unexpected command: {other:?}"),
    }
    assert!(rx.try_recv().is_err());

    let clients = clients.lock_unpoisoned();
    let media = clients.get(&74).unwrap();
    assert_eq!(media.state.tx_codec_decode_error_count, 0);
    assert!(!media.state.tx_codec_degraded);
}

#[test]
fn split_opus_decode_failure_falls_back_both_lanes_to_pcm_without_forcing_rx() {
    let (tx, rx) = mpsc::channel();
    let mut clients_map = BTreeMap::new();
    for client_id in [73, 74] {
        clients_map.insert(
            client_id,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                    opus_decode_enabled: true,
                }),
            },
        );
    }
    let clients = Arc::new(Mutex::new(clients_map));
    let operator_client_id = Arc::new(AtomicU64::new(73));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command(
        "session_lane:codec-fallback,control;",
        &tx,
        &clients,
        73,
        true,
    );
    parse_tci_command(
        "session_lane:codec-fallback,media;",
        &tx,
        &clients,
        74,
        false,
    );
    while rx.try_recv().is_ok() {}
    parse_tci_command("tx_codec_caps:0,opus_wb,pcm;", &tx, &clients, 73, true);

    let mut bad_opus = vec![0u8; 66];
    write_u32_le(&mut bad_opus, 4, 48_000);
    write_u32_le(&mut bad_opus, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(
        &mut bad_opus,
        20,
        TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES as u32,
    );
    write_u32_le(&mut bad_opus, 24, 2);
    write_u32_le(&mut bad_opus, 28, 1);
    write_u32_le(&mut bad_opus, 32, 1);
    write_u32_le(&mut bad_opus, 36, TX_MIC_CODEC_OPUS_WB_ID);
    write_u32_le(&mut bad_opus, 40, 2);
    bad_opus[64..].copy_from_slice(&[0xff, 0xff]);

    assert!(handle_incoming_message(
        Message::Binary(bad_opus.clone().into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        74,
    ));
    assert!(rx.try_recv().is_err());

    let control_outbound = {
        let clients = clients.lock_unpoisoned();
        for client_id in [73, 74] {
            let state = &clients.get(&client_id).unwrap().state;
            assert_eq!(state.tx_codec_active, TxMicCodec::Pcm);
            assert!(state.tx_codec_degraded);
        }
        clients.get(&73).unwrap().outbound.clone()
    };
    let mut saw_pcm_accept = false;
    while let Some(message) = control_outbound.next_message(true) {
        if matches!(message.message, OutboundMessage::SafetyText(ref text) if text == "tx_codec_accept:0,pcm;")
        {
            saw_pcm_accept = true;
        }
    }
    assert!(saw_pcm_accept);

    // An Opus chunk already queued by WebCodecs is ignored during the handoff.
    assert!(handle_incoming_message(
        Message::Binary(bad_opus.into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        74,
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn split_media_ignores_opus_that_arrives_before_control_lane_negotiation() {
    let (tx, rx) = mpsc::channel();
    let mut clients_map = BTreeMap::new();
    for client_id in [73, 74] {
        clients_map.insert(
            client_id,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::with_tx_codec_runtime_flags(TxCodecRuntimeFlags {
                    opus_decode_enabled: true,
                }),
            },
        );
    }
    let clients = Arc::new(Mutex::new(clients_map));
    let operator_client_id = Arc::new(AtomicU64::new(73));
    let operator_control_at = Arc::new(Mutex::new(None));
    parse_tci_command("session_lane:codec-race,control;", &tx, &clients, 73, true);
    parse_tci_command("session_lane:codec-race,media;", &tx, &clients, 74, false);
    while rx.try_recv().is_ok() {}

    let mut early_opus = vec![0u8; 66];
    write_u32_le(&mut early_opus, 4, 48_000);
    write_u32_le(&mut early_opus, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(
        &mut early_opus,
        20,
        TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES as u32,
    );
    write_u32_le(&mut early_opus, 24, 2);
    write_u32_le(&mut early_opus, 28, 1);
    write_u32_le(&mut early_opus, 36, TX_MIC_CODEC_OPUS_WB_ID);
    write_u32_le(&mut early_opus, 40, 2);
    early_opus[64..].copy_from_slice(&[0xff, 0xff]);

    for _ in 0..TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT {
        assert!(handle_incoming_message(
            Message::Binary(early_opus.clone().into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            74,
        ));
    }
    assert!(rx.try_recv().is_err());
    let clients = clients.lock_unpoisoned();
    let media = &clients.get(&74).unwrap().state;
    assert_eq!(media.tx_codec_active, TxMicCodec::Pcm);
    assert_eq!(media.tx_codec_decode_error_count, 0);
    assert!(!media.tx_codec_degraded);
}

#[test]
fn split_unpaired_media_socket_cannot_supply_mic_binary() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(81);
    let operator_client_id = Arc::new(AtomicU64::new(80));
    let operator_control_at = Arc::new(Mutex::new(None));

    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 81, false);
    while rx.try_recv().is_ok() {}

    let frame = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 92);
    assert!(handle_incoming_message(
        Message::Binary(frame.into()),
        &tx,
        &clients,
        &operator_client_id,
        &operator_control_at,
        81,
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn split_session_pairs_after_control_and_media_connect() {
    let now = Instant::now();
    let mut session = SplitSession::new_control("phase-42", now).unwrap();

    assert_eq!(session.state, SplitSessionState::WaitingMedia);
    assert!(!session.pairing_timed_out(now + Duration::from_secs(29)));
    assert!(session.pairing_timed_out(now + Duration::from_secs(30)));

    assert_eq!(
        session.connect_media(),
        Some("session_paired:phase-42;".to_string())
    );
    assert_eq!(session.state, SplitSessionState::Paired);
}

#[test]
fn split_release_opens_media_ignore_window() {
    let now = Instant::now();
    let mut session = SplitSession::new_control("phase-42", now).unwrap();
    session.connect_media();
    assert!(session.key());
    assert_eq!(
        session.media_frame_action(now + Duration::from_millis(10)),
        SplitMediaFrameAction::Accept
    );

    assert!(session.release(now + Duration::from_millis(20)));
    assert_eq!(session.state, SplitSessionState::Paired);
    assert_eq!(
        session.media_frame_action(now + Duration::from_millis(30)),
        SplitMediaFrameAction::DropReleaseWindow
    );
    assert_eq!(session.release_window_drops, 1);
    assert_eq!(
        session.media_frame_action(now + Duration::from_millis(300)),
        SplitMediaFrameAction::DropNotKeyed
    );
}

#[test]
fn split_disconnects_force_rx_at_safety_boundaries() {
    let now = Instant::now();
    let mut media_loss = SplitSession::new_control("phase-42", now).unwrap();
    media_loss.connect_media();
    media_loss.key();
    assert_eq!(
        media_loss.disconnect_media(),
        SplitDisconnectAction {
            force_rx: true,
            close_peer_socket: false,
            state: SplitSessionState::WaitingMedia,
        }
    );

    let mut control_loss = SplitSession::new_control("phase-43", now).unwrap();
    control_loss.connect_media();
    control_loss.key();
    assert_eq!(
        control_loss.disconnect_control(),
        SplitDisconnectAction {
            force_rx: true,
            close_peer_socket: true,
            state: SplitSessionState::Terminated,
        }
    );
}

fn insert_split_paired_client(
    clients: &ClientRegistry,
    client_id: u64,
    session_id: &str,
    lane: SplitSocketKind,
    role: Option<TciClientRole>,
) {
    let mut clients = clients.lock_unpoisoned();
    clients.insert(
        client_id,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState {
                split: Some(SplitClientMetadata {
                    session_id: session_id.to_string(),
                    lane: Some(lane),
                    role,
                    ignore_media_until: None,
                }),
                ..ClientState::default()
            },
        },
    );
}

#[test]
fn split_iq_stream_enable_propagates_from_control_to_media() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        80,
        "phase-42",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 81, "phase-42", SplitSocketKind::Media, None);

    let any_enabled = set_client_iq_stream_enabled(&clients, 80, true);
    assert!(any_enabled);

    let snapshot = clients.lock_unpoisoned();
    assert!(snapshot.get(&80).unwrap().state.iq_stream_enabled);
    assert!(snapshot.get(&81).unwrap().state.iq_stream_enabled);
}

#[test]
fn split_audio_stream_enable_propagates_from_control_to_media() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        82,
        "phase-42",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 83, "phase-42", SplitSocketKind::Media, None);

    let any_enabled = set_client_audio_stream_enabled(&clients, 82, true);
    assert!(any_enabled);

    let snapshot = clients.lock_unpoisoned();
    assert!(snapshot.get(&82).unwrap().state.audio_stream_enabled);
    assert!(snapshot.get(&83).unwrap().state.audio_stream_enabled);
}

#[test]
fn split_audio_format_state_propagates_from_control_to_media() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        84,
        "phase-42",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 85, "phase-42", SplitSocketKind::Media, None);

    set_client_audio_sample_rate(&clients, 84, 24_000);
    set_client_audio_frame_float_count(&clients, 84, 4096);
    set_client_audio_channels(&clients, 84, 1);

    let snapshot = clients.lock_unpoisoned();
    assert_eq!(
        snapshot.get(&85).unwrap().state.audio_sample_rate_hz,
        24_000
    );
    assert_eq!(
        snapshot.get(&85).unwrap().state.audio_frame_float_count,
        4096
    );
    assert_eq!(snapshot.get(&85).unwrap().state.audio_channels, 1);
}

#[test]
fn split_tx_media_priority_suppresses_media_downlink() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        86,
        "phase-42",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 87, "phase-42", SplitSocketKind::Media, None);
    set_client_iq_stream_enabled(&clients, 86, true);
    set_client_audio_stream_enabled(&clients, 86, true);

    let snapshot = clients.lock_unpoisoned();
    let media = snapshot.get(&87).unwrap();

    let rx_iq = OutboundMessage::IqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };
    let tx_iq = OutboundMessage::TxIqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };
    let audio = OutboundMessage::AudioFrame {
        receiver: 0,
        sample_rate: 48_000,
        channels: 1,
        audio_samples: vec![0.0, 0.0],
        sequence: 7,
    };

    // While on-air: all three binary variants are suppressed on the media lane.
    assert!(!client_wants_outbound_message(media, &rx_iq, true));
    assert!(!client_wants_outbound_message(media, &tx_iq, true));
    assert!(!client_wants_outbound_message(media, &audio, true));

    // Off-air: media lane receives binary as normal.
    assert!(client_wants_outbound_message(media, &rx_iq, false));
    assert!(client_wants_outbound_message(media, &tx_iq, false));
    assert!(client_wants_outbound_message(media, &audio, false));
}

#[test]
fn split_outbound_routing_sends_text_to_control_lane_not_media() {
    let mut control = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    control.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Control),
        role: Some(TciClientRole::Operator),
        ignore_media_until: None,
    });
    let mut media = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    media.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Media),
        role: None,
        ignore_media_until: None,
    });

    let text = OutboundMessage::Text("rx_smeter:0,0,-110.0;".into());
    assert!(client_wants_outbound_message(&control, &text, false));
    assert!(!client_wants_outbound_message(&media, &text, false));

    let safety = OutboundMessage::SafetyText("tx_fault:0,power_trip,126.3,110.0;".into());
    assert!(client_wants_outbound_message(&control, &safety, false));
    assert!(!client_wants_outbound_message(&media, &safety, false));
}

#[test]
fn lane_hint_for_request_path_maps_proxy_paths() {
    assert_eq!(
        lane_hint_for_request_path("/control"),
        Some(SplitSocketKind::Control)
    );
    assert_eq!(
        lane_hint_for_request_path("/media"),
        Some(SplitSocketKind::Media)
    );
    assert_eq!(lane_hint_for_request_path("/"), None);
    assert_eq!(lane_hint_for_request_path(""), None);
    assert_eq!(lane_hint_for_request_path("/tci"), None);
    assert_eq!(lane_hint_for_request_path("/media/"), None);
}

#[test]
fn connect_lane_hint_filters_outbound_before_session_lane_declaration() {
    let text = OutboundMessage::Text("rx_smeter:0,0,-110.0;".into());
    let safety = OutboundMessage::SafetyText("tx_fault:0,power_trip,126.3,110.0;".into());
    let rx_iq = OutboundMessage::IqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };

    // Media-path client without any in-band declaration yet: no text ever,
    // binary is available once streams are enabled.
    let mut media = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    media.state.connect_lane_hint = Some(SplitSocketKind::Media);
    media.state.iq_stream_enabled = true;
    assert!(!client_wants_outbound_message(&media, &text, false));
    assert!(!client_wants_outbound_message(&media, &safety, false));
    assert!(client_wants_outbound_message(&media, &rx_iq, false));

    // Control-path client: text flows, binary RX never does.
    let mut control = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    control.state.connect_lane_hint = Some(SplitSocketKind::Control);
    control.state.iq_stream_enabled = true;
    assert!(client_wants_outbound_message(&control, &text, false));
    assert!(!client_wants_outbound_message(&control, &rx_iq, false));

    // Legacy path ("/"): both kinds flow as before.
    let mut legacy = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    legacy.state.iq_stream_enabled = true;
    assert!(client_wants_outbound_message(&legacy, &text, false));
    assert!(client_wants_outbound_message(&legacy, &rx_iq, false));

    // Once the in-band declaration lands it agrees with the hint and the
    // filtering stays the same.
    media.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Media),
        role: None,
        ignore_media_until: None,
    });
    assert!(!client_wants_outbound_message(&media, &text, false));
    assert!(client_wants_outbound_message(&media, &rx_iq, false));
}

#[test]
fn split_outbound_routing_sends_iq_to_media_lane_not_control() {
    let mut control = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    control.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Control),
        role: Some(TciClientRole::Operator),
        ignore_media_until: None,
    });
    control.state.iq_stream_enabled = true;

    let mut media = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    media.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Media),
        role: None,
        ignore_media_until: None,
    });
    media.state.iq_stream_enabled = true;

    let iq = OutboundMessage::IqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };
    assert!(!client_wants_outbound_message(&control, &iq, false));
    assert!(client_wants_outbound_message(&media, &iq, false));

    let tx_iq = OutboundMessage::TxIqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };
    assert!(!client_wants_outbound_message(&control, &tx_iq, false));
    assert!(client_wants_outbound_message(&media, &tx_iq, false));
}

#[test]
fn split_outbound_routing_sends_audio_to_media_lane_not_control() {
    let mut control = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    control.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Control),
        role: Some(TciClientRole::Operator),
        ignore_media_until: None,
    });
    control.state.audio_stream_enabled = true;

    let mut media = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    media.state.split = Some(SplitClientMetadata {
        session_id: "phase-42".into(),
        lane: Some(SplitSocketKind::Media),
        role: None,
        ignore_media_until: None,
    });
    media.state.audio_stream_enabled = true;

    let audio = OutboundMessage::AudioFrame {
        receiver: 0,
        sample_rate: 48_000,
        channels: 1,
        audio_samples: vec![0.0, 0.0],
        sequence: 7,
    };
    assert!(!client_wants_outbound_message(&control, &audio, false));
    assert!(client_wants_outbound_message(&media, &audio, false));
}

#[test]
fn legacy_non_split_client_receives_text_and_binary() {
    let mut legacy = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    // No split metadata — represents a legacy single-socket client.
    legacy.state.iq_stream_enabled = true;
    legacy.state.audio_stream_enabled = true;

    let text = OutboundMessage::Text("rx_smeter:0,0,-110.0;".into());
    let iq = OutboundMessage::IqFrame {
        receiver: 0,
        sample_rate: 192_000,
        iq_samples: vec![0.0, 0.0],
    };
    let audio = OutboundMessage::AudioFrame {
        receiver: 0,
        sample_rate: 48_000,
        channels: 1,
        audio_samples: vec![0.0, 0.0],
        sequence: 0,
    };

    assert!(client_wants_outbound_message(&legacy, &text, false));
    assert!(client_wants_outbound_message(&legacy, &iq, false));
    assert!(client_wants_outbound_message(&legacy, &audio, false));
}

#[test]
fn viewer_commands_are_limited_to_streaming_and_ping() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(9);

    parse_tci_command("trx:0,true,tci;", &tx, &clients, 9, false);
    assert!(rx.try_recv().is_err());

    parse_tci_command("iq_start:0;", &tx, &clients, 9, false);
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetIqStreaming)));
    assert!(
        clients
            .lock()
            .unwrap()
            .get(&9)
            .unwrap()
            .state
            .iq_stream_enabled
    );

    parse_tci_command("audio_seq_gap_count:0,4", &tx, &clients, 9, false);
    assert_eq!(
        clients
            .lock()
            .unwrap()
            .get(&9)
            .unwrap()
            .state
            .audio_seq_gap_count,
        4
    );

    parse_tci_command(
        "tx_uplink_stats:0,true,5,6,7000,9000",
        &tx,
        &clients,
        9,
        false,
    );
    assert!(
        !clients
            .lock()
            .unwrap()
            .get(&9)
            .unwrap()
            .state
            .tx_uplink_degraded
    );

    // Viewer cannot toggle TX media priority — this command is a no-op
    // on the bridge after the source-of-truth refactor, but the
    // viewer filter must still drop it.
    parse_tci_command("remote_tx_media_priority:0,true", &tx, &clients, 9, false);
}

#[test]
fn parses_operator_tx_uplink_stats() {
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(9);

    parse_tci_command(
        "tx_uplink_stats:0,true,5,6,7000,9000",
        &tx,
        &clients,
        9,
        true,
    );
    let clients = clients.lock_unpoisoned();
    let state = &clients.get(&9).unwrap().state;
    assert!(state.tx_uplink_degraded);
    assert_eq!(state.tx_mic_browser_last_seq, 5);
    assert_eq!(state.tx_mic_browser_dropped_count, 6);
    assert_eq!(state.tx_uplink_buffered_bytes, 7000);
    assert_eq!(state.tx_uplink_buffered_high_watermark_bytes, 9000);
}

#[test]
fn tracks_tx_codec_safety_counters_in_client_snapshot() {
    let clients = test_client_registry(9);

    record_client_tx_codec_decode_error(&clients, 9);
    record_client_tx_codec_decode_error(&clients, 9);
    record_client_tx_codec_stale_drop(&clients, 9);
    assert!(flush_client_tx_codec_decode_queue(&clients, 9));

    let clients = clients.lock_unpoisoned();
    let state = &clients.get(&9).unwrap().state;
    assert_eq!(state.tx_codec_decode_error_count, 2);
    assert_eq!(state.tx_codec_stale_drop_count, 1);
    assert_eq!(state.tx_codec_release_flush_count, 1);
}

#[test]
fn classifies_parser_decode_errors_for_telemetry() {
    let mut frame = vec![0u8; 64 + 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_S16);
    write_u32_le(&mut frame, 20, 2);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    write_u32_le(&mut frame, 36, TX_MIC_CODEC_PCM_ID);
    write_u32_le(&mut frame, 40, 2);

    assert_eq!(
        parse_tci_mic_frame_result(&frame).unwrap_err(),
        TciMicFrameParseError::Decode(TxDecodeError::PayloadSizeMismatch)
    );

    write_u32_le(&mut frame, 24, 1);
    assert_eq!(
        parse_tci_mic_frame_result(&frame).unwrap_err(),
        TciMicFrameParseError::NotMicFrame
    );
}

#[test]
fn remote_tx_media_priority_is_a_noop_after_split_refactor() {
    // TX media priority is derived from the
    // bridge's on-air state, not from this browser command. The command
    // is accepted (no parse error) but has no side effect. Older
    // browsers may still send it; this test asserts forward compat.
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(9);

    parse_tci_command("remote_tx_media_priority:0,true", &tx, &clients, 9, true);
    parse_tci_command("remote_tx_media_priority:0,false", &tx, &clients, 9, true);
    // No assertion on per-client field — that field no longer exists.
    // Test passes if parse_tci_command does not panic on the command.
}

#[test]
fn records_tx_mic_sequence_gaps() {
    let clients = test_client_registry(9);

    let arrived_at = Instant::now();
    record_client_tx_mic_frame(&clients, 9, 1, arrived_at);
    record_client_tx_mic_frame(&clients, 9, 2, arrived_at);
    record_client_tx_mic_frame(&clients, 9, 4, arrived_at);

    let clients = clients.lock_unpoisoned();
    let state = &clients.get(&9).unwrap().state;
    assert_eq!(state.tx_mic_last_arrived_seq, 4);
    assert_eq!(state.tx_mic_seq_gap_count, 1);
    assert_eq!(state.tx_mic_last_arrived_at, Some(arrived_at));
}

#[test]
fn trx_true_resets_tx_uplink_attempt_telemetry() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(9);

    parse_tci_command(
        "tx_uplink_stats:0,true,12,3,4096,8192",
        &tx,
        &clients,
        9,
        true,
    );
    let arrived_at = Instant::now();
    record_client_tx_mic_frame(&clients, 9, 10, arrived_at);
    record_client_tx_mic_frame(&clients, 9, 12, arrived_at);

    {
        let clients = clients.lock_unpoisoned();
        let state = &clients.get(&9).unwrap().state;
        assert!(state.tx_uplink_degraded);
        assert_eq!(state.tx_mic_browser_dropped_count, 3);
        assert_eq!(state.tx_mic_seq_gap_count, 1);
        assert_eq!(state.tx_mic_last_arrived_seq, 12);
    }

    parse_tci_command("trx:0,true,tci;", &tx, &clients, 9, true);
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));

    let first_frame_at = arrived_at + Duration::from_millis(20);
    record_client_tx_mic_frame(&clients, 9, 50, first_frame_at);

    let clients = clients.lock_unpoisoned();
    let state = &clients.get(&9).unwrap().state;
    assert!(!state.tx_uplink_degraded);
    assert_eq!(state.tx_mic_browser_last_seq, 0);
    assert_eq!(state.tx_mic_browser_dropped_count, 0);
    assert_eq!(state.tx_uplink_buffered_bytes, 0);
    assert_eq!(state.tx_uplink_buffered_high_watermark_bytes, 0);
    assert_eq!(state.tx_mic_last_arrived_seq, 50);
    assert_eq!(state.tx_mic_seq_gap_count, 0);
    assert_eq!(state.tx_mic_last_arrived_at, Some(first_frame_at));
}

#[test]
fn operator_disconnect_promotes_oldest_viewer() {
    let clients = test_client_registry(1);
    clients.lock_unpoisoned().insert(
        2,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(1));

    let disconnect = unregister_client(&clients, &operator_client_id, 1);

    assert!(disconnect.was_operator);
    assert_eq!(disconnect.promoted_operator, Some(2));
    assert_eq!(disconnect.split_closed_peer, None);
    assert!(!disconnect.split_media_loss_forces_rx);
    assert_eq!(disconnect.remaining_clients, 1);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 2);
}

#[test]
fn split_media_disconnect_forces_rx_when_paired_with_operator() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(91);
    clients.lock_unpoisoned().insert(
        92,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(91));

    parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 91, true);
    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 92, false);
    while rx.try_recv().is_ok() {}

    let disconnect = unregister_client(&clients, &operator_client_id, 92);

    assert!(!disconnect.was_operator);
    assert!(disconnect.split_media_loss_forces_rx);
    assert_eq!(disconnect.split_closed_peer, None);
    assert_eq!(disconnect.promoted_operator, None);
    assert_eq!(disconnect.remaining_clients, 1);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 91);
}

#[test]
fn split_control_disconnect_queues_media_peer_close() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(101);
    clients.lock_unpoisoned().insert(
        102,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        },
    );
    let operator_client_id = Arc::new(AtomicU64::new(101));

    parse_tci_command("session_lane:phase-42,control;", &tx, &clients, 101, true);
    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 102, false);
    while rx.try_recv().is_ok() {}

    let media_outbound = clients
        .lock_unpoisoned()
        .get(&102)
        .unwrap()
        .outbound
        .clone();
    let disconnect = unregister_client(&clients, &operator_client_id, 101);

    assert!(disconnect.was_operator);
    assert_eq!(disconnect.split_closed_peer, Some(102));
    assert_eq!(disconnect.remaining_clients, 1);
    let close = media_outbound.next_message(false).unwrap();
    assert!(matches!(close.message, OutboundMessage::Close));
}

#[test]
fn operator_disconnect_does_not_promote_split_media_socket() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(1);
    {
        let mut clients = clients.lock_unpoisoned();
        clients.insert(
            2,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
        clients.insert(
            3,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState::default(),
            },
        );
    }
    let operator_client_id = Arc::new(AtomicU64::new(1));
    parse_tci_command("session_lane:phase-42,media;", &tx, &clients, 2, false);
    while rx.try_recv().is_ok() {}

    let disconnect = unregister_client(&clients, &operator_client_id, 1);

    assert!(disconnect.was_operator);
    assert!(!disconnect.split_media_loss_forces_rx);
    assert_eq!(disconnect.split_closed_peer, None);
    assert_eq!(disconnect.promoted_operator, Some(3));
    assert_eq!(disconnect.remaining_clients, 2);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 3);
}

#[test]
fn initial_snapshot_includes_remote_tx_rf_state() {
    let model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
    let disabled = initial_snapshot_messages(&model, false, 7, TciClientRole::Viewer);
    let enabled = initial_snapshot_messages(&model, true, 8, TciClientRole::Operator);

    assert!(disabled.contains(&"remote_tx_rf_enabled:0,false;".to_string()));
    assert!(enabled.contains(&"remote_tx_rf_enabled:0,true;".to_string()));
    assert!(disabled.contains(&"remote_client_role:0,viewer,7;".to_string()));
    assert!(enabled.contains(&"remote_client_role:0,operator,8;".to_string()));
}
