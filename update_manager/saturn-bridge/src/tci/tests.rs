use super::*;
use crate::radio_model::{
    AgcMode, DemodMode, NoiseBlankerMode, Nr2GainMethod, Nr2NpeMethod, WbfmDeemphasis,
};
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

#[test]
fn satp_control_requires_operator_role_and_strict_actions() {
    let clients = test_client_registry(7);
    let (tx, rx) = mpsc::channel();
    for action in ["pair", "renew", "tci", "satp"] {
        parse_tci_command(
            &format!("saturn_satp_control:{action}"),
            &tx,
            &clients,
            7,
            false,
        );
        assert!(rx.try_recv().is_err());
        parse_tci_command(
            &format!("saturn_satp_control:{action}"),
            &tx,
            &clients,
            7,
            true,
        );
        assert!(
            matches!(rx.try_recv().unwrap(),TciCommand::SatpControl{client_id:7,action:a} if a==action)
        );
    }
    for invalid in ["pair,extra", "unknown", ""] {
        parse_tci_command(
            &format!("saturn_satp_control:{invalid}"),
            &tx,
            &clients,
            7,
            true,
        );
        assert!(rx.try_recv().is_err());
    }
}

#[test]
fn ptt_release_cancels_a_queued_arm_through_the_parser() {
    let clients = test_client_registry(7);
    let (tx, rx) = tci_command_mailbox();
    parse_tci_command("trx:0,true", &tx, &clients, 7, true);
    parse_tci_command("trx:0,false", &tx, &clients, 7, true);
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
    assert!(rx.try_recv().is_err(), "a stale arm survived the release");
}

#[test]
fn ptt_press_after_a_release_still_arms_through_the_parser() {
    let clients = test_client_registry(7);
    let (tx, rx) = tci_command_mailbox();
    parse_tci_command("trx:0,true", &tx, &clients, 7, true);
    parse_tci_command("trx:0,false", &tx, &clients, 7, true);
    parse_tci_command("trx:0,true", &tx, &clients, 7, true);
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));
    assert!(rx.try_recv().is_err());
}

#[test]
fn client_disconnect_cancels_a_queued_arm() {
    let (tx, rx) = tci_command_mailbox();
    tx.send(TciCommand::SetTxEnabled(true)).unwrap();
    tx.send(TciCommand::ClientDisconnected).unwrap();
    assert!(matches!(rx.try_recv(), Ok(TciCommand::ClientDisconnected)));
    assert!(rx.try_recv().is_err(), "a stale arm survived the disconnect");
}

const HANDSHAKE_REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

struct HandshakeProbe {
    upgraded: bool,
    handler_ran_for: Duration,
}

/// Connects a loopback peer to the production `handle_client`, writes
/// `pieces` (bytes, milliseconds to wait before writing them), optionally
/// closes the peer, and reports whether the server upgraded the connection
/// and how long the handler ran.
fn handshake_probe(pieces: &[(&[u8], u64)], close_after_writing: bool) -> HandshakeProbe {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, addr) = listener.accept().unwrap();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        let (commands, _rx) = mpsc::channel();
        handle_client(
            socket,
            addr,
            1,
            &commands,
            &clients,
            &Arc::new(AtomicU64::new(0)),
            &Arc::new(Mutex::new(None)),
            &Arc::new(Mutex::new(RadioModel::new(
                6, 7_215_000, 0, 384, 24, 2048, true, 4096, true,
            ))),
            &Arc::new(AtomicU64::new(0)),
            &Arc::new(FullRateIqTransportStats::default()),
            false,
            TxCodecRuntimeFlags::default(),
            (false, 50100),
            &DisplayTransport::default(),
        );
        started.elapsed()
    });
    for (bytes, delay_ms) in pieces {
        thread::sleep(Duration::from_millis(*delay_ms));
        // A server that has already given up may reset the connection.
        let _ = peer.write_all(bytes);
    }
    if close_after_writing {
        let _ = peer.shutdown(Shutdown::Write);
    }
    peer.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match peer.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                response.extend_from_slice(&buffer[..n]);
                if response.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let upgraded = response.starts_with(b"HTTP/1.1 101");
    let _ = peer.shutdown(Shutdown::Both);
    drop(peer);
    HandshakeProbe {
        upgraded,
        handler_ran_for: worker.join().unwrap(),
    }
}

#[test]
fn complete_upgrade_request_is_accepted() {
    assert!(handshake_probe(&[(HANDSHAKE_REQUEST, 0)], false).upgraded);
}

#[test]
fn upgrade_request_split_at_any_position_is_accepted() {
    let len = HANDSHAKE_REQUEST.len();
    for split in [1, 32, len / 2, len - 4, len - 2, len - 1] {
        let (first, rest) = HANDSHAKE_REQUEST.split_at(split);
        let probe = handshake_probe(&[(first, 0), (rest, 40)], false);
        assert!(probe.upgraded, "request split after {split} of {len} bytes was dropped");
    }
}

#[test]
fn upgrade_request_in_several_slow_pieces_is_accepted() {
    let len = HANDSHAKE_REQUEST.len();
    let probe = handshake_probe(
        &[
            (&HANDSHAKE_REQUEST[..10], 0),
            (&HANDSHAKE_REQUEST[10..len / 2], 60),
            (&HANDSHAKE_REQUEST[len / 2..], 60),
        ],
        false,
    );
    assert!(probe.upgraded);
}

#[test]
fn upgrade_request_that_starts_late_is_accepted() {
    // The connection is accepted before the first byte is on the socket.
    let probe = handshake_probe(&[(HANDSHAKE_REQUEST, 80)], false);
    assert!(probe.upgraded);
}

#[test]
fn incomplete_upgrade_requests_release_their_slot_at_the_deadline() {
    let deadline = super::client::TCI_HANDSHAKE_TIMEOUT;
    let partial = thread::spawn(|| handshake_probe(&[(&HANDSHAKE_REQUEST[..32], 0)], false));
    let silent = thread::spawn(|| handshake_probe(&[], false));
    for probe in [partial.join().unwrap(), silent.join().unwrap()] {
        assert!(!probe.upgraded);
        assert!(
            probe.handler_ran_for >= deadline - Duration::from_millis(100),
            "gave up after {:?}, before the {:?} deadline",
            probe.handler_ran_for,
            deadline
        );
        assert!(
            probe.handler_ran_for < deadline + Duration::from_millis(1500),
            "still held the connection after {:?}",
            probe.handler_ran_for
        );
    }
}

#[test]
fn peer_that_closes_mid_handshake_releases_its_slot_promptly() {
    let probe = handshake_probe(&[(&HANDSHAKE_REQUEST[..32], 0)], true);
    assert!(!probe.upgraded);
    assert!(
        probe.handler_ran_for < Duration::from_millis(1000),
        "waited {:?} for a peer that had already closed",
        probe.handler_ran_for
    );
}

#[test]
fn malformed_upgrade_request_is_rejected_without_waiting_for_the_deadline() {
    let probe = handshake_probe(&[(b"NOT HTTP AT ALL\r\n\r\n", 0)], false);
    assert!(!probe.upgraded);
    assert!(
        probe.handler_ran_for < Duration::from_millis(1000),
        "took {:?} to reject malformed input",
        probe.handler_ran_for
    );
}

#[test]
fn handshake_resumption_stops_at_the_timeout_it_is_given() {
    use std::io::Write;
    use std::net::TcpStream;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, _) = listener.accept().unwrap();
    socket.set_nonblocking(true).unwrap();
    peer.write_all(&HANDSHAKE_REQUEST[..32]).unwrap();

    let started = Instant::now();
    let result = super::client::accept_with_deadline(
        socket,
        |_: &tungstenite::handshake::server::Request,
         response: tungstenite::handshake::server::Response| Ok(response),
        tci_websocket_config(),
        Duration::from_millis(150),
    );
    let elapsed = started.elapsed();
    assert!(matches!(result, Err(super::client::AcceptError::TimedOut)));
    assert!(elapsed >= Duration::from_millis(150), "returned after {elapsed:?}");
    assert!(elapsed < Duration::from_millis(1000), "returned after {elapsed:?}");
}

/// Bytes held in every queue, to compare with the running total.
fn outbound_byte_total(queues: &OutboundQueues) -> usize {
    let held = |queue: &std::collections::VecDeque<QueuedOutbound>| -> usize {
        queue.iter().map(|item| item.estimated_bytes).sum()
    };
    held(&queues.safety)
        + held(&queues.control)
        + held(&queues.audio)
        + held(&queues.full_rate_iq)
        + queues
            .display
            .as_ref()
            .map_or(0, |item| item.estimated_bytes)
}

fn raw_iq_frame() -> OutboundMessage {
    // The direct-XDMA frame: 12,800 complex pairs, 102,464 bytes with header.
    OutboundMessage::FullRateIqFrame {
        receiver: 0,
        sample_rate: 384_000,
        iq_samples: vec![0.0; 25_600],
    }
}

/// Fills one media class well past the 256 KiB control budget.
fn fill_media_class(outbound: &ClientOutbound, class: OutboundClass) {
    match class {
        OutboundClass::FullRateIq => {
            for _ in 0..MAX_FULL_RATE_IQ_QUEUE_MESSAGES {
                outbound.enqueue(raw_iq_frame());
            }
        }
        OutboundClass::Display => {
            outbound.enqueue(OutboundMessage::IqFrame {
                receiver: 0,
                sample_rate: 384_000,
                iq_samples: vec![0.0; 100_000],
            });
        }
        OutboundClass::Audio => {
            outbound.enqueue(OutboundMessage::AudioFrame {
                receiver: 0,
                sample_rate: 48_000,
                channels: 1,
                audio_samples: vec![0.0; 100_000],
                sequence: 0,
            });
        }
        _ => unreachable!("not a media class"),
    }
    assert!(
        outbound.queues.lock_unpoisoned().queued_bytes > MAX_CONTROL_QUEUE_BYTES,
        "{class:?} backlog must exceed the control budget for this test to mean anything"
    );
}

#[test]
fn media_backlog_does_not_delete_fresh_control_state() {
    for classes in [
        vec![OutboundClass::FullRateIq],
        vec![OutboundClass::Display],
        vec![OutboundClass::Audio],
        vec![
            OutboundClass::FullRateIq,
            OutboundClass::Display,
            OutboundClass::Audio,
        ],
    ] {
        let outbound = ClientOutbound::new();
        for class in &classes {
            fill_media_class(&outbound, *class);
        }
        let media_before = {
            let queues = outbound.queues.lock_unpoisoned();
            (
                queues.full_rate_iq.len(),
                queues.display.is_some(),
                queues.audio.len(),
            )
        };

        let dropped = outbound.enqueue(OutboundMessage::Text("vfo:0,0,7215000;".into()));
        assert_eq!(dropped, 0, "{classes:?}: the new control message was dropped");
        {
            let queues = outbound.queues.lock_unpoisoned();
            assert_eq!(queues.control.len(), 1, "{classes:?}");
            assert_eq!(
                (
                    queues.full_rate_iq.len(),
                    queues.display.is_some(),
                    queues.audio.len()
                ),
                media_before,
                "{classes:?}: media must be left to its own policy"
            );
            assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
        }
        let next = outbound.next_message(true).unwrap();
        assert_eq!(next.class, OutboundClass::Control);
        assert!(matches!(&next.message, OutboundMessage::Text(text) if text == "vfo:0,0,7215000;"));
        assert_eq!(outbound.drain_stats().control_dropped, 0, "{classes:?}");
    }
}

#[test]
fn the_control_budget_still_applies_to_control_bytes() {
    let outbound = ClientOutbound::new();
    fill_media_class(&outbound, OutboundClass::FullRateIq);
    // Three 100,000-byte messages are 300,000 bytes: over the 262,144 budget.
    let messages: Vec<String> = (0..3).map(|n| format!("{n}{}", "x".repeat(99_999))).collect();
    let mut dropped = 0;
    for message in &messages {
        dropped += outbound.enqueue(OutboundMessage::Text(message.clone()));
    }
    let queues = outbound.queues.lock_unpoisoned();
    let control_bytes: usize = queues.control.iter().map(|item| item.estimated_bytes).sum();
    assert!(control_bytes <= MAX_CONTROL_QUEUE_BYTES, "control holds {control_bytes}");
    assert_eq!(dropped, 1, "exactly the oldest control message is dropped");
    assert!(matches!(
        &queues.control.back().unwrap().message,
        OutboundMessage::Text(text) if text == &messages[2]
    ));
    assert_eq!(queues.full_rate_iq.len(), MAX_FULL_RATE_IQ_QUEUE_MESSAGES);
    assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
    drop(queues);
    assert_eq!(outbound.drain_stats().control_dropped, 1);
}

#[test]
fn requeued_control_state_survives_a_media_backlog() {
    let outbound = ClientOutbound::new();
    outbound.enqueue(OutboundMessage::Text("vfo:0,0,7215000;".into()));
    fill_media_class(&outbound, OutboundClass::FullRateIq);

    // The writer takes the control message, then puts it back unsent.
    let taken = outbound.next_message(false).unwrap();
    assert_eq!(taken.class, OutboundClass::Control);
    outbound.requeue_front(taken);

    {
        let queues = outbound.queues.lock_unpoisoned();
        assert_eq!(queues.control.len(), 1, "the requeued message was dropped");
        assert_eq!(queues.full_rate_iq.len(), MAX_FULL_RATE_IQ_QUEUE_MESSAGES);
        assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
    }
    assert_eq!(outbound.drain_stats().control_dropped, 0);
    assert_eq!(outbound.next_message(true).unwrap().class, OutboundClass::Control);
}

#[test]
fn requeue_keeps_control_within_its_own_budget() {
    let outbound = ClientOutbound::new();
    fill_media_class(&outbound, OutboundClass::FullRateIq);
    let big = |tag: char| OutboundMessage::Text(format!("{tag}{}", "x".repeat(99_999)));
    outbound.enqueue(big('a'));
    outbound.enqueue(big('b'));
    let taken = outbound.next_message(false).unwrap();
    outbound.enqueue(big('c'));
    outbound.enqueue(big('d'));
    // Enqueueing 'd' pushed control past its budget and dropped 'b', leaving
    // 'c','d'. Putting 'a' back is over the budget again and must trim it.
    outbound.requeue_front(taken);

    let queues = outbound.queues.lock_unpoisoned();
    let control_bytes: usize = queues.control.iter().map(|item| item.estimated_bytes).sum();
    assert!(control_bytes <= MAX_CONTROL_QUEUE_BYTES, "control holds {control_bytes}");
    assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
}

#[test]
fn safety_messages_are_not_affected_by_a_media_backlog() {
    let outbound = ClientOutbound::new();
    fill_media_class(&outbound, OutboundClass::FullRateIq);
    outbound.enqueue(OutboundMessage::SafetyText("trx:0,false;".into()));
    let queues = outbound.queues.lock_unpoisoned();
    assert_eq!(queues.safety.len(), 1);
    assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
}

/// Every TCI control that carries a floating-point value, with a command
/// prefix that supplies the value as its last argument.
const FLOAT_CONTROLS: &[&str] = &[
    "rx_volume:0,",
    "rx_ssql_threshold:0,",
    "rx_nr_level:0,",
    "rx_anr_gain:0,",
    "rx_anr_leakage:0,",
    "rx_nb_threshold:0,",
    "rx_anf_gain:0,",
    "rx_anf_leakage:0,",
    "rx_agc_gain:0,",
    "tx_monitor_level:0,",
    "tx_mic_gain:0,",
    "tx_cfc_precomp:0,",
    "tx_cfc_band:0,3,",
    "tx_phase_rotator_corner:0,",
    "tx_two_tone_freq1:0,",
    "tx_two_tone_freq2:0,",
    "tx_two_tone_level_db:0,",
    "tx_noise_gate_threshold:0,",
    "tx_dexp_threshold:0,",
    "tx_dexp_expansion:0,",
    "tx_speech_processor_gain:0,",
];

fn non_finite_control_count(clients: &ClientRegistry, client_id: u64) -> u64 {
    clients
        .lock_unpoisoned()
        .get(&client_id)
        .unwrap()
        .state
        .non_finite_control_count
}

#[test]
fn non_finite_numeric_controls_are_rejected_and_counted() {
    for prefix in FLOAT_CONTROLS {
        for bad in ["NaN", "nan", "-nan", "inf", "-inf", "Infinity", "-infinity", "1e999", "-1e999"] {
            let clients = test_client_registry(7);
            let (tx, rx) = tci_command_mailbox();
            parse_tci_command(&format!("{prefix}{bad}"), &tx, &clients, 7, true);
            assert!(
                rx.try_recv().is_err(),
                "{prefix}{bad} produced a command"
            );
            assert_eq!(
                non_finite_control_count(&clients, 7),
                1,
                "{prefix}{bad} was not counted"
            );
        }
    }
}

#[test]
fn finite_numeric_controls_are_still_accepted() {
    for prefix in FLOAT_CONTROLS {
        for good in ["0", "-12.5", "3", "1e3", " 7.25 "] {
            let clients = test_client_registry(7);
            let (tx, rx) = tci_command_mailbox();
            parse_tci_command(&format!("{prefix}{good}"), &tx, &clients, 7, true);
            assert!(
                rx.try_recv().is_ok(),
                "{prefix}{good} was not accepted"
            );
            assert!(rx.try_recv().is_err(), "{prefix}{good} produced two commands");
            assert_eq!(non_finite_control_count(&clients, 7), 0, "{prefix}{good}");
        }
    }
}

#[test]
fn extreme_finite_controls_are_not_mistaken_for_non_finite() {
    // The largest finite f64 is a legal number; range limits belong to the
    // control that owns it. Only NaN and infinities are refused here.
    let clients = test_client_registry(7);
    let (tx, rx) = tci_command_mailbox();
    parse_tci_command("rx_ssql_threshold:0,1.7976931348623157e308", &tx, &clients, 7, true);
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SetRxSsqlThreshold(value)) if value == 100.0
    ));
    parse_tci_command("tx_mic_gain:0,1e300", &tx, &clients, 7, true);
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SetTxMicGain(value)) if value.is_finite()
    ));
    assert_eq!(non_finite_control_count(&clients, 7), 0);
}

#[test]
fn a_rejected_control_does_not_disturb_the_following_commands() {
    let clients = test_client_registry(7);
    let (tx, rx) = tci_command_mailbox();
    parse_tci_command("rx_ssql_threshold:0,NaN", &tx, &clients, 7, true);
    parse_tci_command("rx_ssql_threshold:0,40", &tx, &clients, 7, true);
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SetRxSsqlThreshold(value)) if value == 40.0
    ));
    assert!(rx.try_recv().is_err());
    assert_eq!(non_finite_control_count(&clients, 7), 1);
}

#[test]
fn unparsable_numeric_controls_are_ignored_but_not_counted_as_non_finite() {
    let clients = test_client_registry(7);
    let (tx, rx) = tci_command_mailbox();
    parse_tci_command("rx_ssql_threshold:0,loud", &tx, &clients, 7, true);
    assert!(rx.try_recv().is_err());
    assert_eq!(non_finite_control_count(&clients, 7), 0);
}

fn float_mic_frame(samples: &[f32]) -> Vec<u8> {
    let mut frame = vec![0u8; 64 + samples.len() * 4];
    write_u32_le(&mut frame, 4, 48_000);
    write_u32_le(&mut frame, 8, TX_SAMPLE_TYPE_FLOAT32);
    write_u32_le(&mut frame, 20, samples.len() as u32);
    write_u32_le(&mut frame, 24, 2);
    write_u32_le(&mut frame, 28, 1);
    for (index, sample) in samples.iter().enumerate() {
        let offset = 64 + index * 4;
        frame[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
    }
    frame
}

#[test]
fn float_mic_frames_with_non_finite_samples_are_rejected_as_decode_errors() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let frame = float_mic_frame(&[0.25, -0.5, bad, 0.125]);
        assert_eq!(
            parse_tci_mic_frame_result(&frame).unwrap_err(),
            TciMicFrameParseError::Decode(TxDecodeError::NonFiniteSample),
            "{bad}"
        );
    }
}

#[test]
fn rejected_float_mic_frames_count_toward_the_existing_decode_fault_escalation() {
    let clients = test_client_registry(9);
    let now = Instant::now();
    let mut forced_rx_at = None;
    for attempt in 1..=64u64 {
        let frame = float_mic_frame(&[f32::NAN]);
        assert!(parse_tci_mic_frame_result(&frame).is_err());
        let action = record_client_tx_codec_decode_error_at(&clients, 9, now);
        if action.force_rx && forced_rx_at.is_none() {
            forced_rx_at = Some(attempt);
        }
    }
    assert_eq!(
        forced_rx_at,
        Some(TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT),
        "persistently invalid float audio must release PTT"
    );
}

#[test]
fn finite_float_mic_frames_are_unchanged_including_extremes() {
    let samples = [0.0, -0.0, 0.25, -1.0, 1.0, 4.0, f32::MIN_POSITIVE, f32::MAX, f32::MIN];
    let parsed = parse_tci_mic_frame(&float_mic_frame(&samples)).unwrap();
    assert_eq!(parsed.samples.len(), samples.len());
    for (decoded, sent) in parsed.samples.iter().zip(samples) {
        assert_eq!(decoded.to_bits(), sent.to_bits());
    }
}

#[test]
fn a_bad_float_mic_frame_does_not_poison_the_next_good_one() {
    let bad = float_mic_frame(&[f32::NAN, 0.5]);
    let good = float_mic_frame(&[0.25, -0.5]);
    assert!(parse_tci_mic_frame_result(&bad).is_err());
    assert_eq!(parse_tci_mic_frame(&good).unwrap().samples, vec![0.25, -0.5]);
}

fn free_loopback_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn connect_when_listening(port: u16) -> std::net::TcpStream {
    for _ in 0..100 {
        if let Ok(stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            return stream;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("TCI frontend never listened on {port}");
}

fn wait_until(limit: Duration, mut condition: impl FnMut() -> bool) -> Option<Duration> {
    let started = Instant::now();
    while started.elapsed() < limit {
        if condition() {
            return Some(started.elapsed());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn incomplete_handshakes_release_their_connection_slots_in_the_real_accept_loop() {
    use std::io::Write;

    let port = free_loopback_port();
    let mut config = crate::config::BridgeConfig::default();
    config.tci_bind_addr = SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), port);
    let model = Arc::new(Mutex::new(RadioModel::new(
        6, 7_215_000, 0, 384, 24, 2048, true, 4096, true,
    )));
    let (tci, _commands) = TciFrontend::bind(&config, model).unwrap();
    let active = || tci.client_snapshot().active_connections;
    let rejected = || tci.client_snapshot().rejected_connections;

    // A peer that sends part of a request and disconnects frees its slot at once.
    let mut partial = connect_when_listening(port);
    partial.write_all(&HANDSHAKE_REQUEST[..32]).unwrap();
    assert!(wait_until(Duration::from_secs(1), || active() == 1).is_some());
    drop(partial);
    let freed = wait_until(Duration::from_secs(1), || active() == 0);
    assert!(freed.is_some(), "a closed partial handshake kept its slot");

    // Silent peers fill every slot, and the next connection is refused.
    let silent: Vec<_> = (0..MAX_TCI_CONNECTIONS)
        .map(|_| connect_when_listening(port))
        .collect();
    assert!(wait_until(Duration::from_secs(1), || active() == MAX_TCI_CONNECTIONS).is_some());
    let _refused = connect_when_listening(port);
    assert!(wait_until(Duration::from_secs(1), || rejected() == 1).is_some());

    // The deadline, not a disconnect, frees them: the peers are still open.
    let deadline = super::client::TCI_HANDSHAKE_TIMEOUT;
    let waited = wait_until(deadline + Duration::from_secs(2), || active() == 0)
        .expect("silent connections never released their slots");
    assert!(
        waited + Duration::from_millis(200) >= deadline,
        "slots freed after {waited:?}, before the {deadline:?} deadline"
    );
    drop(silent);

    // And a real client can connect afterwards.
    let stream = connect_when_listening(port);
    let (mut client, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .expect("a valid client connects once the slots are free");
    assert!(wait_until(Duration::from_secs(1), || active() == 1).is_some());
    let _ = client.close(None);
}

#[test]
fn outbound_byte_total_stays_exact_through_every_queue_operation() {
    let outbound = ClientOutbound::new();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut in_hand: Option<QueuedOutbound> = None;
    for step in 0..30_000u32 {
        match random() % 9 {
            0 => {
                outbound.enqueue(OutboundMessage::Text(format!("vfo:0,0,{};", random() % 50_000_000)));
            }
            1 => {
                let size = 1 + (random() % 140_000) as usize;
                outbound.enqueue(OutboundMessage::Text("x".repeat(size)));
            }
            2 => {
                outbound.enqueue(OutboundMessage::SafetyText(format!("trx:0,{};", random() % 2 == 0)));
            }
            3 => {
                outbound.enqueue(OutboundMessage::OpusAudioFrame {
                    receiver: 0,
                    sample_rate: 48_000,
                    channels: 1,
                    packet: vec![0; 20 + (random() % 200) as usize],
                    sequence: 0,
                });
            }
            4 => {
                outbound.enqueue(raw_iq_frame());
            }
            5 => {
                outbound.enqueue(OutboundMessage::IqFrame {
                    receiver: 0,
                    sample_rate: 384_000,
                    iq_samples: vec![0.0; 1 + (random() % 60_000) as usize],
                });
            }
            6 => {
                if let Some(item) = outbound.next_message(random() % 3 != 0) {
                    in_hand = Some(item);
                }
            }
            7 => {
                if let Some(item) = in_hand.take() {
                    outbound.requeue_front(item);
                }
            }
            _ => outbound.clear_audio(),
        }
        let queues = outbound.queues.lock_unpoisoned();
        assert_eq!(
            queues.queued_bytes,
            outbound_byte_total(&queues),
            "byte total drifted at step {step}"
        );
        let control: usize = queues.control.iter().map(|item| item.estimated_bytes).sum();
        assert!(control <= MAX_CONTROL_QUEUE_BYTES, "control holds {control} at step {step}");
        assert!(queues.control.len() <= MAX_CONTROL_QUEUE_MESSAGES);
        assert!(queues.full_rate_iq.len() <= MAX_FULL_RATE_IQ_QUEUE_MESSAGES);
    }

    // Draining everything returns the total to exactly zero.
    drop(in_hand);
    while outbound.next_message(true).is_some() {
        let queues = outbound.queues.lock_unpoisoned();
        assert_eq!(queues.queued_bytes, outbound_byte_total(&queues));
    }
    let queues = outbound.queues.lock_unpoisoned();
    assert_eq!(queues.queued_bytes, 0);
    assert_eq!(outbound_byte_total(&queues), 0);
}

#[test]
fn bad_float_mic_frames_through_the_message_handler_are_counted_and_recovered_from() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(71);
    let operator_client_id = Arc::new(AtomicU64::new(71));
    let operator_control_at = Arc::new(Mutex::new(None));
    let decode_errors = |clients: &ClientRegistry| {
        clients
            .lock_unpoisoned()
            .get(&71)
            .unwrap()
            .state
            .tx_codec_decode_error_count
    };
    let send = |samples: &[f32], sequence: u32| {
        let frame = build_tci_float_frame(0, 48_000, samples, 2, 1, sequence);
        assert!(handle_incoming_message(
            Message::Binary(frame.into()),
            &tx,
            &clients,
            &operator_client_id,
            &operator_control_at,
            71,
        ));
    };

    // A frame with a non-finite sample is counted once and delivers nothing.
    send(&[0.25, f32::NAN], 5);
    assert!(rx.try_recv().is_err(), "a bad frame produced a command");
    assert_eq!(decode_errors(&clients), 1);

    // The next valid frame is delivered unchanged and does not add to the count.
    send(&[0.25, -0.25], 6);
    match rx.try_recv().unwrap() {
        TciCommand::MicAudioFrame(frame) => {
            assert_eq!(frame.sequence, 6);
            assert_eq!(frame.samples, vec![0.25, -0.25]);
        }
        other => panic!("unexpected command: {other:?}"),
    }
    assert_eq!(decode_errors(&clients), 1);

    // Persistent bad frames reach the existing limit and force the radio to RX.
    for sequence in 7..(7 + TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT as u32) {
        send(&[f32::INFINITY, 0.5], sequence);
    }
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|command| matches!(command, TciCommand::SetTxEnabled(false))),
        "persistent invalid audio did not force RX"
    );
}

#[test]
fn command_guard_totals_reach_the_client_snapshot_for_telemetry() {
    let port = free_loopback_port();
    let mut config = crate::config::BridgeConfig::default();
    config.tci_bind_addr = SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), port);
    let model = Arc::new(Mutex::new(RadioModel::new(
        6, 7_215_000, 0, 384, 24, 2048, true, 4096, true,
    )));
    // The receiver is held and never read, so queued commands stay queued.
    let (tci, _commands) = TciFrontend::bind(&config, model).unwrap();
    let stream = connect_when_listening(port);
    let (mut client, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .expect("websocket handshake");

    let before = tci.client_snapshot();
    // A microphone frame and an arm are queued, then released: both are cancelled.
    let mic = build_tci_float_frame(0, 48_000, &[0.25, -0.25], 2, 1, 1);
    client.send(Message::Binary(mic.into())).unwrap();
    client.send(Message::text("trx:0,true;")).unwrap();
    client.send(Message::text("trx:0,false;")).unwrap();
    // A non-finite control is refused.
    client.send(Message::text("rx_ssql_threshold:0,NaN;")).unwrap();

    let settled = wait_until(Duration::from_secs(3), || {
        let now = tci.client_snapshot();
        now.command_arm_cancelled > before.command_arm_cancelled
            && now.command_mic_cancelled > before.command_mic_cancelled
            && now.non_finite_controls_rejected > before.non_finite_controls_rejected
    });
    let after = tci.client_snapshot();
    assert!(
        settled.is_some(),
        "totals did not move: arm {}→{}, mic {}→{}, non-finite {}→{}",
        before.command_arm_cancelled,
        after.command_arm_cancelled,
        before.command_mic_cancelled,
        after.command_mic_cancelled,
        before.non_finite_controls_rejected,
        after.non_finite_controls_rejected
    );
    // This frontend's own queue is exact; the refusal total is process-wide, so
    // other tests may add to it.
    assert_eq!(after.command_arm_cancelled, before.command_arm_cancelled + 1);
    assert_eq!(after.command_mic_cancelled, before.command_mic_cancelled + 1);
    let _ = client.close(None);
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
        enqueue_rx_audio_for_clients(&clients, 48_000, &source, 48_000, 2, false, 0.0),
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
fn outbound_scheduler_buffers_full_rate_iq_in_order_and_bounds_latency() {
    let stats = Arc::new(FullRateIqTransportStats::default());
    let outbound = ClientOutbound::new_with_full_rate_iq_stats(Arc::clone(&stats));

    for frame in 1..=MAX_FULL_RATE_IQ_QUEUE_MESSAGES + 1 {
        let dropped = outbound.enqueue(OutboundMessage::FullRateIqFrame {
            receiver: 0,
            sample_rate: frame as u32,
            iq_samples: vec![frame as f32, -(frame as f32)],
        });
        assert_eq!(dropped, u64::from(frame > MAX_FULL_RATE_IQ_QUEUE_MESSAGES));
    }

    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(
        snapshot.enqueued_deliveries_total,
        (MAX_FULL_RATE_IQ_QUEUE_MESSAGES + 1) as u64
    );
    assert_eq!(snapshot.dropped_deliveries_total, 1);
    assert_eq!(snapshot.dropped_deliveries_interval, 1);
    assert_eq!(
        snapshot.queue_high_watermark,
        MAX_FULL_RATE_IQ_QUEUE_MESSAGES as u64
    );
    assert_eq!(
        outbound.full_rate_iq_queue_depth(),
        MAX_FULL_RATE_IQ_QUEUE_MESSAGES as u64
    );

    let mut retained = Vec::new();
    while let Some(item) = outbound.next_message(true) {
        assert_eq!(item.class, OutboundClass::FullRateIq);
        match item.message {
            OutboundMessage::FullRateIqFrame { sample_rate, .. } => retained.push(sample_rate),
            _ => panic!("expected full-rate IQ frame"),
        }
        outbound.record_write(item.class, Duration::ZERO);
    }
    assert_eq!(retained, vec![2, 3, 4, 5]);
    assert_eq!(
        stats.snapshot_and_drain_interval().written_deliveries_total,
        MAX_FULL_RATE_IQ_QUEUE_MESSAGES as u64
    );
}

#[test]
fn outbound_scheduler_requeues_blocked_full_rate_iq_without_reordering() {
    let stats = Arc::new(FullRateIqTransportStats::default());
    let outbound = ClientOutbound::new_with_full_rate_iq_stats(Arc::clone(&stats));
    for frame in 1..=MAX_FULL_RATE_IQ_QUEUE_MESSAGES {
        outbound.enqueue(OutboundMessage::FullRateIqFrame {
            receiver: 0,
            sample_rate: frame as u32,
            iq_samples: vec![frame as f32, -(frame as f32)],
        });
    }

    let blocked = outbound.next_message(true).unwrap();
    outbound.enqueue(OutboundMessage::FullRateIqFrame {
        receiver: 0,
        sample_rate: 5,
        iq_samples: vec![5.0, -5.0],
    });
    outbound.requeue_front(blocked);

    let mut retained = Vec::new();
    while let Some(item) = outbound.next_message(true) {
        match item.message {
            OutboundMessage::FullRateIqFrame { sample_rate, .. } => retained.push(sample_rate),
            _ => panic!("expected full-rate IQ frame"),
        }
    }
    assert_eq!(retained, vec![1, 2, 3, 4]);
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(snapshot.dropped_deliveries_total, 1);
    assert_eq!(snapshot.dropped_deliveries_interval, 1);
    assert_eq!(
        snapshot.drops_by_reason[IqDropReason::RequeueOverflow as usize],
        1
    );
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

// Explicit local microbenchmark: not a live-radio performance qualification.
#[test]
#[ignore]
fn benchmark_control_state_publication_queue() {
    let outbound = ClientOutbound::new();
    let started = Instant::now();
    for round in 0..1000 {
        for index in 0..128 {
            std::hint::black_box(
                outbound.enqueue(OutboundMessage::Text(format!("state_{index}:0,{round};"))),
            );
        }
        while let Some(item) = outbound.next_message(true) {
            std::hint::black_box(item);
        }
    }
    println!(
        "control_queue_benchmark rounds=1000 fields=128 elapsed_us={}",
        started.elapsed().as_micros()
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
fn outbound_scheduler_keeps_only_eighty_milliseconds_of_opus() {
    let outbound = ClientOutbound::new();
    for sequence in 0..5 {
        outbound.enqueue(OutboundMessage::OpusAudioFrame {
            receiver: 0,
            sample_rate: 48_000,
            channels: 2,
            packet: vec![1, 2, 3],
            sequence,
        });
    }
    let mut sequences = Vec::new();
    while let Some(item) = outbound.next_message(true) {
        match item.message {
            OutboundMessage::OpusAudioFrame { sequence, .. } => sequences.push(sequence),
            _ => panic!("expected Opus audio frame"),
        }
    }
    assert_eq!(sequences, vec![2, 3, 4, 5]);
    assert_eq!(outbound.drain_stats().audio_dropped, 1);
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
    assert_eq!(config.read_buffer_size, 8 * 1024);
    assert_eq!(config.max_message_size, Some(MAX_TCI_INBOUND_MESSAGE_BYTES));
    assert_eq!(config.max_frame_size, Some(MAX_TCI_INBOUND_FRAME_BYTES));
}

#[derive(Default)]
struct PartialWsWriter {
    allowance: usize,
    fail: bool,
    bytes: Vec<u8>,
    incoming: std::io::Cursor<Vec<u8>>,
}
impl std::io::Read for PartialWsWriter {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.incoming.position() as usize == self.incoming.get_ref().len() {
            Err(std::io::ErrorKind::WouldBlock.into())
        } else {
            std::io::Read::read(&mut self.incoming, bytes)
        }
    }
}
impl std::io::Write for PartialWsWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        if self.allowance == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let n = bytes.len().min(self.allowance);
        self.bytes.extend_from_slice(&bytes[..n]);
        self.allowance -= n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn queued_test_iq(outbound: &ClientOutbound, number: u32) {
    outbound.enqueue(OutboundMessage::FullRateIqFrame {
        receiver: 0,
        sample_rate: number,
        iq_samples: vec![0.25; 25_600],
    });
}

fn decode_server_wire(bytes: Vec<u8>, count: usize) -> Vec<Message> {
    let mut receiver = tungstenite::WebSocket::from_raw_socket(
        std::io::Cursor::new(bytes),
        tungstenite::protocol::Role::Client,
        None,
    );
    let messages = (0..count).map(|_| receiver.read().unwrap()).collect();
    assert!(receiver.read().is_err(), "unexpected duplicate on wire");
    messages
}

#[test]
fn buffered_sender_partial_stalls_keep_exact_once_iq_and_safety_priority() {
    let stats = Arc::new(FullRateIqTransportStats::default());
    let outbound = ClientOutbound::new_with_full_rate_iq_stats(stats.clone());
    let mut sender = BufferedSender::new(outbound.clone());
    let mut ws = tungstenite::WebSocket::from_raw_socket(
        PartialWsWriter {
            allowance: 17,
            ..Default::default()
        },
        tungstenite::protocol::Role::Server,
        Some(tci_websocket_config()),
    );
    queued_test_iq(&outbound, 1);
    let mut first = outbound.next_message(true).unwrap();
    first.enqueued_at = Instant::now() - Duration::from_millis(60);
    assert!(!sender.send(&mut ws, first).unwrap());
    assert!(sender.is_pending());
    assert!(stats.snapshot_and_drain_interval().stall_max_us[0] >= 60_000);
    // An outgoing stall must not prevent receiving a dekey/control message.
    ws.get_mut().incoming = std::io::Cursor::new(encoded_ws_messages(vec![Message::Text(
        "trx:0,false;".into(),
    )]));
    assert_eq!(ws.read().unwrap(), Message::Text("trx:0,false;".into()));
    for number in 2..=5 {
        queued_test_iq(&outbound, number);
    }
    outbound.enqueue(OutboundMessage::SafetyText("trx:0,false;".into()));
    for _ in 0..3 {
        assert!(!sender.flush(&mut ws).unwrap());
        assert!(sender.is_pending());
    }
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(snapshot.in_flight, 1);
    assert_eq!(snapshot.written_deliveries_total, 0);
    assert_eq!(snapshot.dropped_deliveries_total, 0);
    assert_eq!(outbound.full_rate_iq_queue_depth(), 4);
    ws.get_mut().allowance = usize::MAX;
    assert!(!sender.flush(&mut ws).unwrap());
    assert!(!sender.is_pending());
    while let Some(item) = outbound.next_message(true) {
        sender.send(&mut ws, item).unwrap();
    }
    let wire = decode_server_wire(ws.into_inner().bytes, 6);
    assert_eq!(wire[1], Message::Text("trx:0,false;".into()));
    for (index, number) in [(0, 1), (2, 2), (3, 3), (4, 4), (5, 5)] {
        assert_eq!(
            wire[index],
            Message::Binary(build_tci_iq_frame(0, number, &vec![0.25; 25_600]).into())
        );
    }
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(snapshot.written_deliveries_total, 5);
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.dropped_deliveries_total, 0);
}

#[test]
fn buffered_sender_counts_send_flush_and_teardown_failures_once() {
    for reason in [
        IqDropReason::SendError,
        IqDropReason::FlushError,
        IqDropReason::ConnectionClosed,
    ] {
        let stats = Arc::new(FullRateIqTransportStats::default());
        let outbound = ClientOutbound::new_with_full_rate_iq_stats(stats.clone());
        let mut sender = BufferedSender::new(outbound.clone());
        let mut ws = tungstenite::WebSocket::from_raw_socket(
            PartialWsWriter {
                fail: matches!(reason, IqDropReason::SendError),
                allowance: 17,
                ..Default::default()
            },
            tungstenite::protocol::Role::Server,
            Some(tci_websocket_config()),
        );
        queued_test_iq(&outbound, 1);
        let result = sender.send(&mut ws, outbound.next_message(true).unwrap());
        if matches!(reason, IqDropReason::SendError) {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
            assert!(sender.is_pending());
        }
        if matches!(reason, IqDropReason::FlushError) {
            ws.get_mut().fail = true;
            assert!(sender.flush(&mut ws).is_err());
        }
        drop(sender);
        drop(outbound);
        let snapshot = stats.snapshot_and_drain_interval();
        assert_eq!(snapshot.dropped_deliveries_total, 1);
        assert_eq!(snapshot.drops_by_reason[reason as usize], 1);
        assert_eq!(snapshot.drops_by_reason.iter().sum::<u64>(), 1);
        assert_eq!(snapshot.in_flight, 0);
        assert!(snapshot.last_drop_epoch_ms > 0);
        assert_eq!(
            stats
                .snapshot_and_drain_interval()
                .dropped_deliveries_interval,
            0
        );
    }
}

#[test]
fn buffered_sender_only_requeues_explicitly_unaccepted_writes() {
    let stats = Arc::new(FullRateIqTransportStats::default());
    let outbound = ClientOutbound::new_with_full_rate_iq_stats(stats.clone());
    let mut sender = BufferedSender::new(outbound.clone());
    let mut ws = tungstenite::WebSocket::from_raw_socket(
        PartialWsWriter {
            allowance: usize::MAX,
            ..Default::default()
        },
        tungstenite::protocol::Role::Server,
        Some(
            tci_websocket_config()
                .write_buffer_size(0)
                .max_write_buffer_size(1024),
        ),
    );
    queued_test_iq(&outbound, 1);
    assert!(
        matches!(sender.send(&mut ws, outbound.next_message(true).unwrap()),
        Err(tungstenite::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(!sender.is_pending());
    assert_eq!(outbound.full_rate_iq_queue_depth(), 1);
    assert!(ws.get_ref().bytes.is_empty());
    ws.set_config(|config| config.max_write_buffer_size = usize::MAX);
    sender
        .send(&mut ws, outbound.next_message(true).unwrap())
        .unwrap();
    decode_server_wire(ws.into_inner().bytes, 1);
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(snapshot.written_deliveries_total, 1);
    assert_eq!(snapshot.dropped_deliveries_total, 0);
}

#[test]
fn buffered_sender_flushes_close_before_reporting_closed() {
    let outbound = ClientOutbound::new();
    let mut sender = BufferedSender::new(outbound.clone());
    let mut ws = tungstenite::WebSocket::from_raw_socket(
        PartialWsWriter {
            allowance: 1,
            ..Default::default()
        },
        tungstenite::protocol::Role::Server,
        Some(tci_websocket_config()),
    );
    outbound.enqueue(OutboundMessage::Close);
    assert!(!sender
        .send(&mut ws, outbound.next_message(true).unwrap())
        .unwrap());
    assert!(sender.is_pending());
    ws.get_mut().allowance = usize::MAX;
    assert!(sender.flush(&mut ws).unwrap());
    assert!(!sender.is_pending());
    assert_eq!(
        decode_server_wire(ws.into_inner().bytes, 1),
        vec![Message::Close(None)]
    );
}

#[test]
fn iq_drop_reasons_account_for_queue_overflow_and_shutdown() {
    let stats = Arc::new(FullRateIqTransportStats::default());
    let outbound = ClientOutbound::new_with_full_rate_iq_stats(stats.clone());
    for number in 1..=5 {
        queued_test_iq(&outbound, number);
    }
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(snapshot.dropped_deliveries_total, 1);
    assert_eq!(
        snapshot.drops_by_reason[IqDropReason::QueueOverflow as usize],
        1
    );
    assert_eq!(snapshot.dropped_deliveries_interval, 1);
    drop(outbound);
    let snapshot = stats.snapshot_and_drain_interval();
    assert_eq!(
        snapshot.drops_by_reason[IqDropReason::ConnectionClosed as usize],
        4
    );
    assert_eq!(snapshot.dropped_deliveries_interval, 4);
    assert_eq!(snapshot.dropped_deliveries_total, 5);
    assert_eq!(snapshot.drops_by_reason.iter().sum::<u64>(), 5);
}

#[test]
fn buffered_sender_stalled_audio_and_control_are_not_dropped_or_duplicated() {
    for message in [
        OutboundMessage::Text("vfo:0,0,7200000;".into()),
        OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: 48_000,
            channels: 2,
            audio_samples: vec![0.25; 2048],
            sequence: 7,
        },
    ] {
        let outbound = ClientOutbound::new();
        let drops = Arc::new(AtomicU64::new(0));
        let mut sender = BufferedSender::with_drop_counter(outbound.clone(), drops.clone());
        let mut ws = tungstenite::WebSocket::from_raw_socket(
            PartialWsWriter::default(),
            tungstenite::protocol::Role::Server,
            Some(tci_websocket_config()),
        );
        outbound.enqueue(message);
        assert!(!sender
            .send(&mut ws, outbound.next_message(true).unwrap())
            .unwrap());
        assert!(sender.is_pending());
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        ws.get_mut().allowance = usize::MAX;
        sender.flush(&mut ws).unwrap();
        assert!(!sender.is_pending());
        decode_server_wire(ws.into_inner().bytes, 1);
        drop(sender);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn tungstenite_would_block_retains_iq_and_resending_duplicates_it() {
    #[derive(Default)]
    struct BlockedWriter {
        blocked: bool,
        bytes: Vec<u8>,
    }
    impl std::io::Read for BlockedWriter {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::WouldBlock.into())
        }
    }
    impl std::io::Write for BlockedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.blocked {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    for resend in [false, true] {
        let mut sender = tungstenite::WebSocket::from_raw_socket(
            BlockedWriter {
                blocked: true,
                ..Default::default()
            },
            tungstenite::protocol::Role::Server,
            Some(tci_websocket_config()),
        );
        let iq = Message::Binary(build_tci_iq_frame(0, 384_000, &vec![0.25; 25_600]).into());
        assert!(
            matches!(sender.send(iq.clone()), Err(tungstenite::Error::Io(error))
            if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        assert!(sender.get_ref().bytes.is_empty());
        sender.get_mut().blocked = false;
        // Flush alone delivers the original. Sending the message again, as
        // the current application requeue path does, delivers it twice.
        if resend {
            sender.send(iq.clone()).unwrap();
        } else {
            sender.flush().unwrap();
        }
        let mut receiver = tungstenite::WebSocket::from_raw_socket(
            std::io::Cursor::new(sender.into_inner().bytes),
            tungstenite::protocol::Role::Client,
            None,
        );
        assert_eq!(receiver.read().unwrap(), iq);
        if resend {
            assert_eq!(receiver.read().unwrap(), iq);
        }
        assert!(receiver.read().is_err(), "unexpected extra wire message");
    }
}

// Exercise the real WebSocket codec, including partial reads and WouldBlock.
// Counting attempted read bytes also guards against reintroducing large
// zero-filled scratch buffers in the idle polling path.
#[derive(Default)]
struct TestWsInput {
    bytes: std::io::Cursor<Vec<u8>>,
    attempted_bytes: usize,
    chunk_limit: usize,
    block_next: bool,
}

impl std::io::Read for TestWsInput {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.attempted_bytes += out.len();
        if self.block_next || self.bytes.position() as usize == self.bytes.get_ref().len() {
            self.block_next = false;
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        self.block_next = true;
        let len = out.len().min(self.chunk_limit);
        std::io::Read::read(&mut self.bytes, &mut out[..len])
    }
}

impl std::io::Write for TestWsInput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_ws_messages(messages: Vec<Message>) -> Vec<u8> {
    let mut client = tungstenite::WebSocket::from_raw_socket(
        std::io::Cursor::new(Vec::new()),
        tungstenite::protocol::Role::Client,
        None,
    );
    for message in messages {
        client.send(message).unwrap();
    }
    client.into_inner().into_inner()
}

fn decode_test_ws(bytes: Vec<u8>) -> Result<Message, tungstenite::Error> {
    let mut server = tungstenite::WebSocket::from_raw_socket(
        TestWsInput {
            bytes: std::io::Cursor::new(bytes),
            chunk_limit: 997,
            ..Default::default()
        },
        tungstenite::protocol::Role::Server,
        Some(tci_websocket_config()),
    );
    for _ in 0..2048 {
        match server.read() {
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock => {}
            result => return result,
        }
    }
    panic!("WebSocket did not finish within bounded partial reads");
}

#[test]
fn websocket_idle_reads_use_small_scratch_buffer() {
    let mut server = tungstenite::WebSocket::from_raw_socket(
        TestWsInput::default(),
        tungstenite::protocol::Role::Server,
        Some(tci_websocket_config()),
    );
    for _ in 0..1000 {
        assert!(matches!(server.read(), Err(tungstenite::Error::Io(error))
            if error.kind() == std::io::ErrorKind::WouldBlock));
    }
    assert_eq!(server.get_ref().attempted_bytes, 1000 * 8 * 1024);
}

#[test]
fn websocket_small_read_buffer_preserves_maximum_binary_message() {
    let payload: Vec<u8> = (0..MAX_TCI_INBOUND_MESSAGE_BYTES)
        .map(|i| i as u8)
        .collect();
    let expected = Message::Binary(payload.into());
    assert_eq!(
        decode_test_ws(encoded_ws_messages(vec![expected.clone()])).unwrap(),
        expected
    );
}

#[test]
fn websocket_small_read_buffer_preserves_fragmented_message() {
    use tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    let first = vec![0x55; 100_000];
    let second = vec![0xaa; 100_000];
    let expected = [first.as_slice(), second.as_slice()].concat();
    let wire = encoded_ws_messages(vec![
        Message::Frame(Frame::message(first, OpCode::Data(Data::Binary), false)),
        Message::Frame(Frame::message(second, OpCode::Data(Data::Continue), true)),
    ]);
    assert_eq!(
        decode_test_ws(wire).unwrap(),
        Message::Binary(expected.into())
    );
}

#[test]
fn websocket_small_read_buffer_still_rejects_oversized_messages() {
    let wire = encoded_ws_messages(vec![Message::Binary(
        vec![0; MAX_TCI_INBOUND_MESSAGE_BYTES + 1].into(),
    )]);
    assert!(matches!(
        decode_test_ws(wire),
        Err(tungstenite::Error::Capacity(_))
    ));
}

#[test]
fn websocket_small_read_buffer_still_limits_fragmented_total() {
    use tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    // Each frame fits independently, but their combined message must fail.
    let wire = encoded_ws_messages(vec![
        Message::Frame(Frame::message(
            vec![0; 150_000],
            OpCode::Data(Data::Binary),
            false,
        )),
        Message::Frame(Frame::message(
            vec![0; 150_000],
            OpCode::Data(Data::Continue),
            true,
        )),
    ]);
    assert!(matches!(
        decode_test_ws(wire),
        Err(tungstenite::Error::Capacity(_))
    ));
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
fn parses_vfo_split_attenuation_and_squelch_commands() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    for command in [
        "vfo_active:0,B",
        "split:0,true",
        "rx_attenuation:0,20",
        "rx_ssql:0,true",
        "rx_ssql_threshold:0,27",
    ] {
        parse_tci_command(command, &tx, &clients, 7, true);
    }

    assert!(matches!(rx.recv().unwrap(), TciCommand::SetActiveVfo(1)));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetSplitEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxAttenuation(20)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxSsqlEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxSsqlThreshold(value) if value == 27.0
    ));
}

#[test]
fn parses_nb3_dexp_speech_processor_and_cessb_commands() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(7);

    for command in [
        "rx_nb:0,NB3",
        "tx_dexp:0,true",
        "tx_dexp_threshold:0,-42.5",
        "tx_dexp_expansion:0,12",
        "tx_speech_processor:0,true",
        "tx_speech_processor_gain:0,8.5",
        "tx_cessb:0,true",
    ] {
        parse_tci_command(command, &tx, &clients, 7, true);
    }

    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetNoiseBlankerMode(NoiseBlankerMode::Nb3)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxDexpEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxDexpThreshold(value) if value == -42.5
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxDexpExpansion(value) if value == 12.0
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxSpeechProcessorEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxSpeechProcessorGain(value) if value == 8.5
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetTxCessbEnabled(true)
    ));
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
    parse_tci_command("tx_monitor:0,true", &tx, &clients, 9, false);
    parse_tci_command("rx_headphones:0,true", &tx, &clients, 9, false);
    parse_tci_command("tx_monitor_level:0,-6", &tx, &clients, 9, false);
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
fn monitor_commands_are_operator_only_bounded_and_not_tx_commands() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(9);
    parse_tci_command("tx_monitor:0,true", &tx, &clients, 9, true);
    assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxMonitor(true))));
    parse_tci_command("rx_headphones:0,true", &tx, &clients, 9, true);
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SetRxHeadphones(true))
    ));
    parse_tci_command("tx_monitor_level:0,40", &tx, &clients, 9, true);
    assert!(matches!(
        rx.try_recv(),
        Ok(TciCommand::SetTxMonitorLevel(-6.0))
    ));
    for command in [
        "tx_monitor:1,true",
        "tx_monitor:0,bogus",
        "rx_headphones:1,true",
        "rx_headphones:0,bogus",
        "tx_monitor_level:0,NaN",
        "tx_monitor_level:0,inf",
        "tx_monitor_level:0",
    ] {
        parse_tci_command(command, &tx, &clients, 9, true);
        assert!(rx.try_recv().is_err(), "accepted {command}");
    }
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
fn lan_accessory_cannot_become_or_inherit_operator_role() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    let operator_client_id = Arc::new(AtomicU64::new(0));

    let (lan_role, first_client, _) = register_client(
        &clients,
        &operator_client_id,
        41,
        ClientOutbound::new(),
        TxCodecRuntimeFlags::default(),
        None,
        false,
    );
    assert!(first_client);
    assert_eq!(lan_role, TciClientRole::Viewer);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 0);

    let (loopback_role, _, _) = register_client(
        &clients,
        &operator_client_id,
        42,
        ClientOutbound::new(),
        TxCodecRuntimeFlags::default(),
        None,
        true,
    );
    assert_eq!(loopback_role, TciClientRole::Operator);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 42);

    let disconnect = unregister_client(&clients, &operator_client_id, 42);
    assert!(disconnect.was_operator);
    assert_eq!(disconnect.promoted_operator, None);
    assert_eq!(operator_client_id.load(Ordering::SeqCst), 0);
}

#[test]
fn lan_tci_bind_also_opens_same_family_loopback_listener() {
    let configured: SocketAddr = "192.168.0.139:50001".parse().unwrap();
    assert_eq!(
        listener_addrs(configured),
        vec![configured, "127.0.0.1:50001".parse().unwrap()]
    );
}

#[test]
fn loopback_and_wildcard_tci_binds_do_not_add_duplicate_listener() {
    let loopback: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    assert_eq!(listener_addrs(loopback), vec![loopback]);

    let wildcard: SocketAddr = "0.0.0.0:50001".parse().unwrap();
    assert_eq!(listener_addrs(wildcard), vec![wildcard]);
}

#[test]
fn lan_split_session_operator_request_is_downgraded_to_viewer() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    let mut state = ClientState::default();
    state.operator_eligible = false;
    clients.lock_unpoisoned().insert(
        51,
        ClientConnection {
            outbound: ClientOutbound::new(),
            state,
        },
    );

    let role =
        set_client_split_session_open(&clients, 51, "lan-accessory", TciClientRole::Operator);
    assert_eq!(role, Some(TciClientRole::Viewer));
    assert_eq!(
        clients
            .lock_unpoisoned()
            .get(&51)
            .unwrap()
            .state
            .split
            .as_ref()
            .unwrap()
            .role,
        Some(TciClientRole::Viewer)
    );
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
    let disabled =
        initial_snapshot_messages(&model, false, 7, TciClientRole::Viewer, (false, 50100));
    let enabled =
        initial_snapshot_messages(&model, true, 8, TciClientRole::Operator, (true, 50100));

    assert!(disabled.contains(&"remote_tx_rf_enabled:0,false;".to_string()));
    assert!(disabled.contains(
        &"tx_monitor_supported:0,false;tx_monitor:0,false;tx_monitor_level:0,-30.0;".to_string()
    ));
    assert!(
        disabled.contains(&"rx_headphones_supported:0,false;rx_headphones:0,false;".to_string())
    );
    assert!(enabled.contains(&"remote_tx_rf_enabled:0,true;".to_string()));
    assert!(disabled.contains(&"remote_client_role:0,viewer,7;".to_string()));
    assert!(enabled.contains(&"remote_client_role:0,operator,8;".to_string()));
    assert!(enabled.contains(&"tx_dexp:0,false;".to_string()));
    assert!(enabled.contains(&"tx_dexp_threshold:0,-40.0;".to_string()));
    assert!(enabled.contains(&"tx_dexp_expansion:0,10.0;".to_string()));
    assert!(enabled.contains(&"tx_speech_processor:0,false;".to_string()));
    assert!(enabled.contains(&"tx_speech_processor_gain:0,10.0;".to_string()));
    assert!(enabled.contains(&"tx_cessb:0,false;".to_string()));
    assert!(disabled.contains(&"saturn_satp_enabled:false;".to_string()));
    assert!(enabled.contains(&"saturn_satp_enabled:true;".to_string()));
    assert!(enabled.contains(&"saturn_satp_version:2;".to_string()));
    assert!(enabled.contains(&"saturn_satp_tx_port:50100;".to_string()));
    assert!(enabled.contains(&"saturn_satp_tx_format:48000,float32_le,1,128;".to_string()));
}

#[test]
fn initial_snapshot_has_standard_tci_initialization_before_ready() {
    let model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
    let messages =
        initial_snapshot_messages(&model, false, 7, TciClientRole::Viewer, (true, 50100));

    assert_eq!(
        messages.first().map(String::as_str),
        Some("protocol:SaturnBridge,2.0;")
    );
    assert_eq!(messages.last().map(String::as_str), Some("ready;"));
    for required in [
        "device:ANAN-G2;",
        "receive_only:false;",
        "trx_count:1;",
        "channel_count:2;",
        "vfo_limits:10000,61440000;",
        "if_limits:-96000,96000;",
        "modulations_list:LSB,USB,CWL,CWU,AM,SAM,DSB,FM,NFM,DIGL,DIGU,WFM;",
    ] {
        assert!(
            messages.iter().any(|message| message == required),
            "missing {required}"
        );
    }
}

#[test]
fn initial_snapshot_publishes_authoritative_split_tx_frequency() {
    let mut model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
    model.desired.vfo_a_hz = 7_100_000;
    model.desired.vfo_b_hz = 14_250_000;
    model.desired.active_vfo = 0;
    model.desired.split_enabled = true;
    model.sync_vfo_routes();

    let messages =
        initial_snapshot_messages(&model, false, 7, TciClientRole::Viewer, (false, 50100));
    assert!(messages.contains(&"vfo:0,0,7100000;".to_string()));
    assert!(messages.contains(&"split_enable:0,true;".to_string()));
    assert!(messages.contains(&"tx_frequency:14250000;".to_string()));
}

#[test]
fn viewer_may_read_standard_state_but_may_not_change_it() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(29);

    parse_tci_command("vfo:0,0", &tx, &clients, 29, false);
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::RequestRadioState { client_id: 29 }
    ));

    parse_tci_command("vfo:0,0,7100000", &tx, &clients, 29, false);
    assert!(rx.try_recv().is_err());
}

#[test]
fn parses_standard_tci_control_aliases() {
    let (tx, rx) = mpsc::channel();
    let clients = test_client_registry(30);

    for command in [
        "split_enable:0,true",
        "drive:0,25",
        "agc_mode:0,fast",
        "agc_gain:0,73",
        "rx_nb_enable:0,true",
        "rx_nr_enable:0,true",
        "rx_anf_enable:0,true",
    ] {
        parse_tci_command(command, &tx, &clients, 30, true);
    }

    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetSplitEnabled(true)
    ));
    assert!(matches!(rx.recv().unwrap(), TciCommand::SetTxDrive(25)));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetAgcMode(AgcMode::Fast)
    ));
    assert!(matches!(rx.recv().unwrap(), TciCommand::SetAgcGain(value) if value == 73.0));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetNoiseBlankerMode(NoiseBlankerMode::Nb1)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetRxNoiseReductionEnabled(true)
    ));
    assert!(matches!(
        rx.recv().unwrap(),
        TciCommand::SetAnfEnabled(true)
    ));
}

fn spectrum_test_row(fft_size: u32, interval_ms: u64, age: Duration) -> Arc<SpectrumRow> {
    Arc::new(SpectrumRow {
        span_hz: 384_000,
        fft_size,
        interval: Duration::from_millis(interval_ms),
        center_hz: 14_200_000,
        captured_at: Instant::now() - age,
        capture_to_enqueue_us: 12,
        server_ms: 34,
        codes: (0..fft_size).map(|bin| bin as u8).collect(),
    })
}

fn spectrum_message(row: &Arc<SpectrumRow>) -> OutboundMessage {
    OutboundMessage::SpectrumRow {
        row: Arc::clone(row),
        sequence: 0,
        credit_blocked: false,
    }
}

fn spectrum_client_registry(client_id: u64) -> ClientRegistry {
    let clients = test_client_registry(client_id);
    clients
        .lock_unpoisoned()
        .get_mut(&client_id)
        .unwrap()
        .state
        .display_spectrum_supported = true;
    clients
}

fn drain_texts(outbound: &ClientOutbound) -> Vec<String> {
    let mut texts = Vec::new();
    while let Some(item) = outbound.next_message(false) {
        if let OutboundMessage::Text(text) = item.message {
            texts.push(text);
        }
    }
    texts
}

#[test]
fn display_negotiation_clamps_echoes_and_reverts() {
    let (tx, _rx) = mpsc::channel();
    let clients = spectrum_client_registry(3);
    let outbound = clients.lock_unpoisoned()[&3].outbound.clone();
    assert_eq!(
        clients.lock_unpoisoned()[&3].state.display_mode,
        DisplayMode::RawIq
    );

    // Viewers may negotiate their own display transport.
    parse_tci_command("saturn_display:spectrum,3000,10", &tx, &clients, 3, false);
    assert_eq!(
        clients.lock_unpoisoned()[&3].state.display_mode,
        DisplayMode::Spectrum {
            fft_size: 2048,
            interval_ms: 33
        }
    );
    assert_eq!(
        drain_texts(&outbound),
        vec!["saturn_display:0,spectrum,2048,33;".to_string()]
    );

    parse_tci_command("saturn_display:spectrum,abc,50", &tx, &clients, 3, true);
    assert!(drain_texts(&outbound).is_empty());
    assert!(clients.lock_unpoisoned()[&3]
        .state
        .display_mode
        .spectrum_group()
        .is_some());

    parse_tci_command("saturn_display:iq", &tx, &clients, 3, true);
    assert_eq!(
        clients.lock_unpoisoned()[&3].state.display_mode,
        DisplayMode::RawIq
    );
    assert_eq!(
        drain_texts(&outbound),
        vec!["saturn_display:0,iq;".to_string()]
    );
}

#[test]
fn display_negotiation_without_backend_support_stays_raw_iq() {
    let (tx, _rx) = mpsc::channel();
    let clients = test_client_registry(4);
    let outbound = clients.lock_unpoisoned()[&4].outbound.clone();
    parse_tci_command("saturn_display:spectrum,2048,50", &tx, &clients, 4, true);
    assert_eq!(
        clients.lock_unpoisoned()[&4].state.display_mode,
        DisplayMode::RawIq
    );
    assert_eq!(
        drain_texts(&outbound),
        vec!["saturn_display:0,iq;".to_string()]
    );
}

#[test]
fn display_routing_keeps_raw_iq_for_default_clients_only() {
    let mut raw = ClientConnection {
        outbound: ClientOutbound::new(),
        state: ClientState::default(),
    };
    raw.state.iq_stream_enabled = true;
    let mut wan = raw.clone();
    wan.state.display_mode = DisplayMode::Spectrum {
        fft_size: 2048,
        interval_ms: 50,
    };
    let mut control = wan.clone();
    control.state.connect_lane_hint = Some(SplitSocketKind::Control);

    let full_rate = OutboundMessage::FullRateIqFrame {
        receiver: 0,
        sample_rate: 384_000,
        iq_samples: vec![0.0; 4],
    };
    let matching = spectrum_message(&spectrum_test_row(2048, 50, Duration::ZERO));
    let other_size = spectrum_message(&spectrum_test_row(1024, 50, Duration::ZERO));
    let other_interval = spectrum_message(&spectrum_test_row(2048, 100, Duration::ZERO));

    assert!(client_wants_outbound_message(&raw, &full_rate, false));
    assert!(!client_wants_outbound_message(&raw, &matching, false));
    assert!(!client_wants_outbound_message(&wan, &full_rate, false));
    assert!(client_wants_outbound_message(&wan, &matching, false));
    assert!(!client_wants_outbound_message(&wan, &other_size, false));
    assert!(!client_wants_outbound_message(&wan, &other_interval, false));
    assert!(!client_wants_outbound_message(&wan, &matching, true));
    assert!(!client_wants_outbound_message(&control, &matching, false));

    assert!(client_receives_raw_iq(&raw));
    assert!(!client_receives_display_spectrum(&raw));
    assert!(!client_receives_raw_iq(&wan));
    assert!(client_receives_display_spectrum(&wan));
    assert!(!client_receives_display_spectrum(&control));
    wan.state.iq_stream_enabled = false;
    assert!(!client_receives_display_spectrum(&wan));
}

#[test]
fn raw_iq_delivery_is_unchanged_by_a_concurrent_spectrum_client() {
    // Mirrors TciFrontend::send_message routing for a LAN client alone and
    // alongside a WAN spectrum client; the LAN byte stream must be identical.
    let frames: Vec<Vec<f32>> = (0..6)
        .map(|frame| (0..64).map(|v| (frame * 64 + v) as f32).collect())
        .collect();
    let deliver = |with_wan: bool| {
        let mut lan = ClientConnection {
            outbound: ClientOutbound::new(),
            state: ClientState::default(),
        };
        lan.state.iq_stream_enabled = true;
        let mut clients = vec![lan];
        if with_wan {
            let mut wan = clients[0].clone();
            wan.outbound = ClientOutbound::new();
            wan.state.display_mode = DisplayMode::Spectrum {
                fft_size: 2048,
                interval_ms: 50,
            };
            clients.push(wan);
        }
        let mut lan_bytes = Vec::new();
        for samples in &frames {
            let message = OutboundMessage::FullRateIqFrame {
                receiver: 0,
                sample_rate: 384_000,
                iq_samples: samples.clone(),
            };
            for client in &clients {
                if client_wants_outbound_message(client, &message, false) {
                    client.outbound.enqueue(message.clone());
                }
            }
            if with_wan {
                assert!(clients[1].outbound.next_message(true).is_none());
            }
            while let Some(item) = clients[0].outbound.next_message(true) {
                let OutboundMessage::FullRateIqFrame {
                    receiver,
                    sample_rate,
                    iq_samples,
                } = item.message
                else {
                    panic!("unexpected LAN message");
                };
                lan_bytes.push(build_tci_iq_frame(receiver, sample_rate, &iq_samples));
            }
        }
        lan_bytes
    };
    let alone = deliver(false);
    assert_eq!(alone.len(), frames.len());
    assert_eq!(alone, deliver(true));
}

#[test]
fn spectrum_rows_are_latest_wins_in_the_display_slot() {
    let outbound = ClientOutbound::new();
    let rows: Vec<_> = (0..50)
        .map(|_| spectrum_test_row(256, 50, Duration::ZERO))
        .collect();
    for row in &rows {
        outbound.enqueue(spectrum_message(row));
    }
    assert_eq!(outbound.queued_bytes(), 64 + 256);
    let item = outbound.next_message(true).unwrap();
    let OutboundMessage::SpectrumRow { row, sequence, .. } = item.message else {
        panic!("expected spectrum row");
    };
    assert!(Arc::ptr_eq(&row, rows.last().unwrap()));
    assert_eq!(sequence, 1);
    assert!(outbound.next_message(true).is_none());
    assert_eq!(outbound.queued_bytes(), 0);
    let stats = outbound.spectrum_stats_snapshot();
    assert_eq!((stats.enqueued, stats.replaced), (50, 49));
}

#[test]
fn spectrum_credit_blocks_until_ack_and_replaced_rows_cost_nothing() {
    let outbound = ClientOutbound::new();
    for expected in 1..=2 {
        // Rows replaced before the writer takes one never consume credit.
        for _ in 0..3 {
            outbound.enqueue(spectrum_message(&spectrum_test_row(
                256,
                50,
                Duration::ZERO,
            )));
        }
        let item = outbound.next_message(true).unwrap();
        assert!(matches!(
            item.message,
            OutboundMessage::SpectrumRow { sequence, .. } if sequence == expected
        ));
    }
    assert_eq!(outbound.spectrum_in_flight(), 2);

    outbound.enqueue(spectrum_message(&spectrum_test_row(
        256,
        50,
        Duration::ZERO,
    )));
    assert!(outbound.next_message(true).is_none());
    assert!(outbound.next_message(true).is_none());
    assert_eq!(outbound.spectrum_stats_snapshot().credit_blocked, 1);

    // Audio is never held behind display credit.
    outbound.enqueue(OutboundMessage::AudioFrame {
        receiver: 0,
        sample_rate: 48_000,
        channels: 2,
        audio_samples: vec![0.0; 256],
        sequence: 0,
    });
    assert_eq!(
        outbound.next_message(true).unwrap().class,
        OutboundClass::Audio
    );

    outbound.record_spectrum_ack(1);
    let item = outbound.next_message(true).unwrap();
    assert!(matches!(
        item.message,
        OutboundMessage::SpectrumRow { sequence: 3, .. }
    ));
    // Stale or duplicate acks never widen the window.
    outbound.record_spectrum_ack(1);
    outbound.record_spectrum_ack(9);
    assert_eq!(outbound.spectrum_in_flight(), 2);
}

#[test]
fn stale_spectrum_row_is_dropped_at_dequeue() {
    let outbound = ClientOutbound::new();
    outbound.enqueue(spectrum_message(&spectrum_test_row(
        256,
        50,
        Duration::from_millis(150),
    )));
    assert!(outbound.next_message(true).is_none());
    assert_eq!(outbound.queued_bytes(), 0);
    assert_eq!(outbound.spectrum_in_flight(), 0);
    assert_eq!(outbound.spectrum_stats_snapshot().dropped_stale, 1);
}

#[test]
fn requeued_spectrum_row_returns_credit_and_never_displaces_newer_row() {
    let outbound = ClientOutbound::new();
    let first = spectrum_test_row(256, 50, Duration::ZERO);
    outbound.enqueue(spectrum_message(&first));
    let taken = outbound.next_message(true).unwrap();
    assert_eq!(outbound.spectrum_in_flight(), 1);
    outbound.requeue_front(taken);
    assert_eq!(outbound.spectrum_in_flight(), 0);
    let retaken = outbound.next_message(true).unwrap();
    assert!(matches!(
        retaken.message,
        OutboundMessage::SpectrumRow { sequence: 1, .. }
    ));

    let newer = spectrum_test_row(256, 50, Duration::ZERO);
    outbound.enqueue(spectrum_message(&newer));
    outbound.requeue_front(retaken);
    let OutboundMessage::SpectrumRow { row, .. } = outbound.next_message(true).unwrap().message
    else {
        panic!("expected spectrum row");
    };
    assert!(Arc::ptr_eq(&row, &newer));
}

#[test]
fn spectrum_row_frame_header_matches_wire_contract() {
    let row = spectrum_test_row(256, 50, Duration::ZERO);
    let frame = build_tci_spectrum_row_frame(&row, 7);
    let u32_at = |offset: usize| u32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap());
    let f32_at = |offset: usize| f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap());
    assert_eq!(frame.len(), 64 + 256);
    assert_eq!(u32_at(0), 0);
    assert_eq!(u32_at(4), 384_000);
    assert_eq!(u32_at(8), 0x5301);
    assert_eq!(u32_at(12), 256);
    assert_eq!(u32_at(16), 1);
    assert_eq!(u32_at(20), 256);
    assert_eq!(u32_at(24), 16);
    assert_eq!(u32_at(28), 1);
    assert_eq!(u32_at(32), 7);
    assert_eq!(
        u64::from(u32_at(36)) | (u64::from(u32_at(40)) << 32),
        14_200_000
    );
    assert_eq!(f32_at(44), -160.0);
    assert_eq!(f32_at(48), 0.625);
    assert_eq!(u32_at(52), 12);
    assert_eq!(u32_at(56), 34);
    assert_eq!(u32_at(60), 0);
    assert_eq!(&frame[64..], &row.codes[..]);
}

#[test]
fn split_media_lane_inherits_display_mode_and_receives_acks() {
    let (tx, _rx) = mpsc::channel();
    let clients = spectrum_client_registry(71);
    let mut media_state = ClientState::default();
    media_state.display_spectrum_supported = true;
    let media_outbound = ClientOutbound::new();
    clients.lock_unpoisoned().insert(
        72,
        ClientConnection {
            outbound: media_outbound.clone(),
            state: media_state,
        },
    );

    parse_tci_command("session_lane:wan-1,control", &tx, &clients, 71, true);
    parse_tci_command("saturn_display:spectrum,1024,33", &tx, &clients, 71, true);
    assert_eq!(
        clients.lock_unpoisoned()[&72].state.display_mode,
        DisplayMode::RawIq
    );
    parse_tci_command("session_lane:wan-1,media", &tx, &clients, 72, false);
    let expected = DisplayMode::Spectrum {
        fft_size: 1024,
        interval_ms: 33,
    };
    assert_eq!(clients.lock_unpoisoned()[&72].state.display_mode, expected);

    // After pairing, changes on the control lane mirror to the media lane.
    parse_tci_command("saturn_display:iq", &tx, &clients, 71, true);
    assert_eq!(
        clients.lock_unpoisoned()[&72].state.display_mode,
        DisplayMode::RawIq
    );
    parse_tci_command("saturn_display:spectrum,1024,33", &tx, &clients, 71, true);
    assert_eq!(clients.lock_unpoisoned()[&72].state.display_mode, expected);

    media_outbound.enqueue(spectrum_message(&spectrum_test_row(
        1024,
        33,
        Duration::ZERO,
    )));
    assert!(media_outbound.next_message(true).is_some());
    assert_eq!(media_outbound.spectrum_in_flight(), 1);
    parse_tci_command("saturn_display_ack:1", &tx, &clients, 71, false);
    assert_eq!(media_outbound.spectrum_in_flight(), 0);
}

#[test]
fn opus_audio_wire_header_uses_payload_byte_count() {
    let packet = [0x11, 0x22, 0x33, 0x44, 0x55];
    let frame = build_tci_opus_audio_frame(0, 48_000, 1, &packet, 19);
    assert_eq!(frame.len(), 69);
    assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 48_000);
    assert_eq!(u32::from_le_bytes(frame[8..12].try_into().unwrap()), 20);
    assert_eq!(u32::from_le_bytes(frame[12..16].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(frame[16..20].try_into().unwrap()), 960);
    assert_eq!(u32::from_le_bytes(frame[20..24].try_into().unwrap()), 5);
    assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 17);
    assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(frame[32..36].try_into().unwrap()), 19);
    assert_eq!(&frame[64..], &packet);
}

#[test]
fn rx_audio_codec_requires_client_gain_and_echoes_selection() {
    let clients = test_client_registry(110);
    let (tx, _) = mpsc::channel();
    parse_tci_command("audio_codec:opus;", &tx, &clients, 110, false);
    assert_eq!(
        clients
            .lock_unpoisoned()
            .get(&110)
            .unwrap()
            .state
            .rx_audio_codec,
        RxAudioCodec::Pcm
    );
    let outbound = clients
        .lock_unpoisoned()
        .get(&110)
        .unwrap()
        .outbound
        .clone();
    assert!(
        matches!(outbound.next_message(true).unwrap().message, OutboundMessage::Text(ref text) if text == "audio_codec:pcm;")
    );
    parse_tci_command("audio_gain:client;", &tx, &clients, 110, false);
    let echo = outbound.next_message(true).unwrap().message;
    assert!(matches!(echo, OutboundMessage::Text(ref text) if text == "audio_gain:client;"));
    parse_tci_command("audio_codec:opus;", &tx, &clients, 110, false);
    let selected = clients
        .lock_unpoisoned()
        .get(&110)
        .unwrap()
        .state
        .rx_audio_codec;
    assert_eq!(
        selected == RxAudioCodec::Opus,
        RxOpusTransport::global().available(2)
    );
    let echo = outbound.next_message(true).unwrap().message;
    assert!(
        matches!(echo, OutboundMessage::Text(ref text) if text == if selected == RxAudioCodec::Opus { "audio_codec:opus;audio_samplerate:48000;" } else { "audio_codec:pcm;" })
    );
    parse_tci_command("audio_codec:pcm;", &tx, &clients, 110, false);
    assert_eq!(
        clients
            .lock_unpoisoned()
            .get(&110)
            .unwrap()
            .state
            .rx_audio_codec,
        RxAudioCodec::Pcm
    );
}

#[test]
fn mixed_clients_apply_bridge_volume_once_to_pcm_only() {
    let clients = test_client_registry(111);
    {
        let mut clients = clients.lock_unpoisoned();
        clients.get_mut(&111).unwrap().state.audio_stream_enabled = true;
        clients.insert(
            112,
            ClientConnection {
                outbound: ClientOutbound::new(),
                state: ClientState {
                    audio_stream_enabled: true,
                    rx_audio_gain: RxAudioGain::Client,
                    ..ClientState::default()
                },
            },
        );
    }
    // Distinct L/R samples exercise the WBFM stereo transport path too.
    let source = vec![0.25, 0.5, 0.25, 0.5];
    assert_eq!(
        enqueue_rx_audio_for_clients(&clients, 48_000, &source, 48_000, 2, false, 6.0),
        0
    );
    let clients = clients.lock_unpoisoned();
    let samples = |id| {
        let client = clients.get(&id).unwrap();
        let queues = client.outbound.queues.lock_unpoisoned();
        match &queues.audio.front().unwrap().message {
            OutboundMessage::AudioFrame { audio_samples, .. } => audio_samples.clone(),
            _ => panic!("expected PCM"),
        }
    };
    let legacy = samples(111);
    let neutral = samples(112);
    assert!((legacy[0] - 0.4988).abs() < 0.001);
    assert!((legacy[1] - 0.9976).abs() < 0.001);
    assert_eq!(neutral, source);
}

#[test]
fn split_rx_audio_negotiation_mirrors_to_media_and_resets_on_pcm() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        113,
        "rx-opus",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 114, "rx-opus", SplitSocketKind::Media, None);
    assert_eq!(
        set_client_rx_audio_gain(&clients, 113, RxAudioGain::Client),
        RxAudioGain::Client
    );
    let selected = set_client_rx_audio_codec(&clients, 113, RxAudioCodec::Opus);
    {
        let clients = clients.lock_unpoisoned();
        assert_eq!(
            clients.get(&113).unwrap().state.rx_audio_gain,
            RxAudioGain::Client
        );
        assert_eq!(
            clients.get(&114).unwrap().state.rx_audio_gain,
            RxAudioGain::Client
        );
        assert_eq!(clients.get(&114).unwrap().state.rx_audio_codec, selected);
    }
    assert_eq!(
        set_client_rx_audio_codec(&clients, 113, RxAudioCodec::Pcm),
        RxAudioCodec::Pcm
    );
    let clients = clients.lock_unpoisoned();
    assert_eq!(
        clients.get(&114).unwrap().state.rx_audio_codec,
        RxAudioCodec::Pcm
    );
}

#[test]
fn failed_shared_opus_profile_falls_back_on_control_and_drains_media() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        115,
        "rx-fallback",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    insert_split_paired_client(&clients, 116, "rx-fallback", SplitSocketKind::Media, None);
    {
        let mut clients = clients.lock_unpoisoned();
        for id in [115, 116] {
            clients.get_mut(&id).unwrap().state.rx_audio_codec = RxAudioCodec::Opus;
        }
        clients
            .get(&116)
            .unwrap()
            .outbound
            .enqueue(OutboundMessage::OpusAudioFrame {
                receiver: 0,
                sample_rate: 48_000,
                channels: 2,
                packet: vec![1, 2, 3],
                sequence: 0,
            });
        fallback_failed_rx_opus_clients(&mut clients, |_| false);
        assert_eq!(
            clients.get(&115).unwrap().state.rx_audio_codec,
            RxAudioCodec::Pcm
        );
        assert_eq!(
            clients.get(&116).unwrap().state.rx_audio_codec,
            RxAudioCodec::Pcm
        );
        assert!(clients
            .get(&116)
            .unwrap()
            .outbound
            .queues
            .lock_unpoisoned()
            .audio
            .is_empty());
        let echo = clients
            .get(&115)
            .unwrap()
            .outbound
            .next_message(true)
            .unwrap()
            .message;
        assert!(matches!(echo, OutboundMessage::Text(ref text) if text == "audio_codec:pcm;"));
        assert!(clients
            .get(&116)
            .unwrap()
            .outbound
            .next_message(true)
            .is_none());
    }
}

#[test]
fn late_split_media_pair_inherits_rx_audio_negotiation() {
    let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
    insert_split_paired_client(
        &clients,
        117,
        "rx-late",
        SplitSocketKind::Control,
        Some(TciClientRole::Operator),
    );
    set_client_audio_sample_rate(&clients, 117, 12_000);
    set_client_audio_channels(&clients, 117, 1);
    set_client_audio_stream_enabled(&clients, 117, true);
    set_client_rx_audio_gain(&clients, 117, RxAudioGain::Client);
    let selected = set_client_rx_audio_codec(&clients, 117, RxAudioCodec::Opus);
    insert_split_paired_client(&clients, 118, "rx-late", SplitSocketKind::Media, None);
    assert!(set_client_split_session_lane(
        &clients,
        118,
        "rx-late",
        SplitSocketKind::Media
    ));
    let clients = clients.lock_unpoisoned();
    let media = &clients.get(&118).unwrap().state;
    assert_eq!(media.rx_audio_gain, RxAudioGain::Client);
    assert_eq!(media.rx_audio_codec, selected);
    assert_eq!(media.audio_channels, 1);
    assert_eq!(media.audio_sample_rate_hz, 12_000);
    assert!(media.audio_stream_enabled);
}

#[test]
fn switching_back_to_bridge_gain_resets_codec_and_audio_queue() {
    let clients = test_client_registry(119);
    set_client_rx_audio_gain(&clients, 119, RxAudioGain::Client);
    let _ = set_client_rx_audio_codec(&clients, 119, RxAudioCodec::Opus);
    {
        let clients = clients.lock_unpoisoned();
        clients
            .get(&119)
            .unwrap()
            .outbound
            .enqueue(OutboundMessage::AudioFrame {
                receiver: 0,
                sample_rate: 48_000,
                channels: 2,
                audio_samples: vec![0.1; 1920],
                sequence: 0,
            });
    }
    set_client_rx_audio_gain(&clients, 119, RxAudioGain::Bridge);
    let clients = clients.lock_unpoisoned();
    let client = clients.get(&119).unwrap();
    assert_eq!(client.state.rx_audio_gain, RxAudioGain::Bridge);
    assert_eq!(client.state.rx_audio_codec, RxAudioCodec::Pcm);
    assert!(client.outbound.queues.lock_unpoisoned().audio.is_empty());
}

// ---- TCP_NODELAY on accepted TCI sockets

#[test]
fn nodelay_defaults_on_and_the_switch_turns_it_off() {
    assert_eq!(parse_nodelay_setting(None), (true, None));
    assert_eq!(parse_nodelay_setting(Some("")), (true, None));
    assert_eq!(parse_nodelay_setting(Some("   ")), (true, None));
    for on in ["1", "true", "TRUE", "on", "yes", " 1 "] {
        assert_eq!(parse_nodelay_setting(Some(on)), (true, None), "{on:?}");
    }
    for off in ["0", "false", "False", "off", "OFF", "no", " 0\n"] {
        assert_eq!(parse_nodelay_setting(Some(off)), (false, None), "{off:?}");
    }
}

#[test]
fn an_unrecognized_nodelay_value_stays_on_and_warns() {
    for typo in ["of", "disable", "2", "nope"] {
        let (enabled, warning) = parse_nodelay_setting(Some(typo));
        assert!(enabled, "{typo:?} switched TCP_NODELAY off");
        let warning = warning.unwrap_or_else(|| panic!("{typo:?} gave no warning"));
        assert!(warning.contains(TCI_NODELAY_ENV) && warning.contains(typo), "{warning}");
    }
}

struct FakeSocket {
    refuse: bool,
    calls: std::cell::Cell<u32>,
}

impl NoDelaySocket for FakeSocket {
    fn set_nodelay(&self, enabled: bool) -> io::Result<()> {
        assert!(enabled, "the Bridge only ever turns TCP_NODELAY on");
        self.calls.set(self.calls.get() + 1);
        if self.refuse {
            Err(io::Error::new(io::ErrorKind::Unsupported, "refused by the test"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_socket_that_refuses_nodelay_is_reported_not_fatal() {
    let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let refusing = FakeSocket { refuse: true, calls: std::cell::Cell::new(0) };
    assert!(!apply_nodelay(&refusing, addr, true), "a refused option was reported as set");
    assert_eq!(refusing.calls.get(), 1);
    let accepting = FakeSocket { refuse: false, calls: std::cell::Cell::new(0) };
    assert!(apply_nodelay(&accepting, addr, true));
}

#[test]
fn a_disabled_nodelay_setting_never_touches_the_socket() {
    let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let socket = FakeSocket { refuse: false, calls: std::cell::Cell::new(0) };
    assert!(!apply_nodelay(&socket, addr, false));
    assert_eq!(socket.calls.get(), 0);
}

/// The production `handle_client` on a real accepted loopback socket: the same
/// socket (a duplicate descriptor refers to the same TCP socket) must have
/// Nagle's algorithm switched off once the handler has started.
#[test]
fn the_production_accept_path_sets_tcp_nodelay_on_the_accepted_socket() {
    let (expected, _) = parse_nodelay_setting(std::env::var(TCI_NODELAY_ENV).ok().as_deref());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, addr) = listener.accept().unwrap();
    let probe = socket.try_clone().unwrap();
    assert!(
        !probe.nodelay().unwrap(),
        "control: a freshly accepted socket must start with Nagle's algorithm on"
    );
    let worker = thread::spawn(move || {
        let clients: ClientRegistry = Arc::new(Mutex::new(BTreeMap::new()));
        let (commands, _rx) = mpsc::channel();
        handle_client(
            socket,
            addr,
            1,
            &commands,
            &clients,
            &Arc::new(AtomicU64::new(0)),
            &Arc::new(Mutex::new(None)),
            &Arc::new(Mutex::new(RadioModel::new(
                6, 7_215_000, 0, 384, 24, 2048, true, 4096, true,
            ))),
            &Arc::new(AtomicU64::new(0)),
            &Arc::new(FullRateIqTransportStats::default()),
            false,
            TxCodecRuntimeFlags::default(),
            (false, 50100),
            &DisplayTransport::default(),
        );
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while probe.nodelay().unwrap() != expected && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        probe.nodelay().unwrap(),
        expected,
        "the accepted socket's TCP_NODELAY does not match the setting ({TCI_NODELAY_ENV})"
    );
    drop(peer);
    worker.join().unwrap();
}
