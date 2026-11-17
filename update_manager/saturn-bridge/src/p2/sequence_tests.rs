//! Actual loopback datagrams, not a mock sequence counter or radio.
use super::*;

fn fixture() -> (P2Session, UdpSocket, RadioModel) {
    let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    sink.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let port = sink.local_addr().unwrap().port();
    let mut config = BridgeConfig {
        client_bind_addr: "127.0.0.1:0".parse().unwrap(),
        radio_command_addr: sink.local_addr().unwrap(),
        enable_discovery: false,
        ..BridgeConfig::default()
    };
    config.port_map.high_priority_to_sdr = port;
    config.port_map.duc_iq = port;
    config.port_map.duc_specific = port;
    let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    model.desired.running = true;
    model.desired.tx_enabled = true;
    model.desired.tx_drive = 5;
    (P2Session::bind(config).unwrap(), sink, model)
}

fn packet(sink: &UdpSocket) -> Vec<u8> {
    let mut bytes = [0; 2048];
    let n = sink.recv(&mut bytes).unwrap();
    bytes[..n].to_vec()
}

fn sequence(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap())
}

#[test]
fn duc_specific_control_is_zero_on_every_send() {
    let (session, sink, model) = fixture();
    for _ in 0..3 {
        session.send_duc_specific(&model).unwrap();
        let bytes = packet(&sink);
        assert_eq!(bytes.len(), 60);
        assert_eq!(sequence(&bytes), 0);
    }
}

#[test]
fn run_stop_restarts_iq_but_ptt_release_does_not() {
    let (session, sink, mut model) = fixture();
    for expected in 0..2 {
        session.send_duc_iq(&[0.; 480]).unwrap();
        assert_eq!(sequence(&packet(&sink)), expected);
    }
    model.desired.tx_enabled = false;
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 1); // RUN remains on across PTT release
    model.desired.tx_enabled = true;
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 3);
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 2);
    session.send_stop().unwrap();
    assert_eq!(packet(&sink)[4] & 3, 0);
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 3);
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 0);
}

#[test]
fn run_false_high_priority_uses_the_same_reset_boundary() {
    let (session, sink, mut model) = fixture();
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 0);
    model.desired.running = false;
    model.desired.tx_enabled = false;
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 0);
    model.desired.running = true;
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 1);
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 0);
}

#[test]
fn unsuccessful_run_stop_does_not_reset_the_data_counter() {
    let (mut session, sink, _) = fixture();
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 0);
    // Linux rejects UDP sends to destination port zero: no stop was submitted.
    session.config.port_map.high_priority_to_sdr = 0;
    assert!(session.send_stop().is_err());
    session.send_duc_iq(&[0.; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 1);
}

#[test]
fn data_sequence_wraps_at_the_wire_u32_boundary() {
    let (session, sink, _) = fixture();
    *session.duc_iq_sequence.lock_unpoisoned() = u32::MAX - 1;
    for expected in [u32::MAX - 1, u32::MAX, 0] {
        session.send_duc_iq(&[0.; 480]).unwrap();
        assert_eq!(sequence(&packet(&sink)), expected);
    }
}

#[test]
fn concurrent_iq_writers_and_run_stop_keep_wire_order_and_reset() {
    use std::sync::Barrier;
    let (session, sink, model) = fixture();
    let session = Arc::new(session);
    // A known nonzero sequence before the contested boundary.
    session.send_duc_iq(&[0.; 960]).unwrap();
    assert_eq!(sequence(&packet(&sink)), 0);
    assert_eq!(sequence(&packet(&sink)), 1);
    let barrier = Arc::new(Barrier::new(7));
    thread::scope(|scope| {
        for _ in 0..6 {
            let session = session.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                session.send_duc_iq(&[0.25; 1440]).unwrap();
            });
        }
        let session = session.clone();
        scope.spawn(move || {
            barrier.wait();
            session.send_stop().unwrap();
        });
    });
    let mut next = 2;
    let mut saw_stop = false;
    for _ in 0..19 {
        let bytes = packet(&sink);
        if bytes[4] & 1 == 0 && bytes[4..].iter().all(|b| *b == 0) {
            assert!(!saw_stop);
            saw_stop = true;
            next = 0;
        } else {
            // Nonzero IQ payload distinguishes data sequence zero from HP stop.
            assert_eq!(sequence(&bytes), next);
            next += 1;
        }
    }
    assert!(saw_stop);
    session.send_high_priority(&model).unwrap();
    assert_eq!(packet(&sink)[4] & 3, 3);
    session.send_duc_iq(&[0.25; 480]).unwrap();
    assert_eq!(sequence(&packet(&sink)), next);
}
