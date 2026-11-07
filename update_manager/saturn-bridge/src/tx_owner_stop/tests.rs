use super::*;
use crate::{
    config::BridgeConfig, tx_thread::TxRadio, xdma_tx_radio::offline_fixture::DirectFixture,
};
use saturn_bridge::puresignal::{tx_lifecycle::StopCause, Fence};
use std::{net::UdpSocket, sync::atomic::AtomicBool, thread};

#[test]
fn manual_ptt_keys_in_silence_but_requires_fresh_source_and_diagnostic_gates_remain() {
    use crate::radio_model::DemodMode;
    use crate::tci::ps_authorization::OperatorIngress;
    for (mode, manual, iq_level) in [
        (DemodMode::Am, false, 0.353553),
        (DemodMode::Usb, true, 0.),
        (DemodMode::Lsb, true, 0.),
        (DemodMode::Usb, false, 0.), // diagnostic/unstamped zero IQ never keys
    ] {
        for direct in [false, true] {
            for source in [None, Some(0), Some(1_000_000_000)] {
                let d = DirectFixture::with_key_clock();
                d.fifo_ready();
                let (session, _hp, _duc) = p2_separate_lanes();
                let (mut iq, mut mic) = queues();
                let mut m = model();
                m.desired.mode = mode;
                m.desired.tx_drive = 5;
                let backend = if direct {
                    Backend::DirectXdma
                } else {
                    Backend::P2
                };
                let mut owner = if direct {
                    OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap()
                } else {
                    OwnerStopAdapter::p2(session, &m, &mut iq, &mut mic).unwrap()
                };
                owner.enable_offline_handoff(&m, 5).unwrap();
                let r = request(backend, 1);
                owner.begin_stop(r).unwrap();
                let mut stopped = false;
                for _ in 1..200 {
                    if owner.poll_stop(1).unwrap().is_some() {
                        stopped = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                assert!(stopped);
                owner.cleanup_complete(1, r.fence).unwrap();
                let ingress = OperatorIngress::fixture();
                let credential = ingress
                    .receive(7, "trx:0,true", 0, r.fence, owner.ready_revision().unwrap())
                    .unwrap();
                let action = owner.authorize(credential).unwrap();
                owner.begin_arm(action, 1).unwrap();
                let stamp = owner.output_stamp().unwrap();
                if manual {
                    owner.manual_ptt(stamp).unwrap();
                }
                if let Some(observed) = source {
                    owner.note_source(stamp, observed).unwrap();
                }
                let mut keyed = false;
                for i in 0..35 {
                    let now = 1_001_000_000 + i * 1_000_000;
                    owner.set_test_key_time(now);
                    if !owner.has_iq_packet() {
                        owner.enqueue_mic(stamp, now, &[0.; 512]).unwrap();
                        assert!(owner.take_mic(stamp).unwrap().iter().all(|v| *v == 0.));
                        owner.enqueue_iq(stamp, now, &[iq_level; 480]).unwrap();
                    }
                    if owner.poll_key(now).unwrap().is_some() {
                        keyed = true;
                        break;
                    }
                }
                assert_eq!(
                    keyed,
                    source == Some(1_000_000_000) && (manual || mode == DemodMode::Am),
                    "direct={direct} source={source:?}"
                );
                owner.begin_stop(request(backend, 2)).unwrap();
                let mut stopped = false;
                for _ in 0..200 {
                    if owner.poll_stop(2).unwrap().is_some() {
                        stopped = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                assert!(stopped);
                assert!(
                    owner.note_source(stamp, 240_000_000).is_err(),
                    "old source cannot cross cancellation"
                );
                assert!(
                    owner.manual_ptt(stamp).is_err(),
                    "cancelled arm cannot grant manual PTT"
                );
                assert!(owner.queues_empty());
                if direct {
                    d.assert_off();
                }
            }
        }
    }
}

#[test]
fn direct_uses_fifo_backpressure_while_p2_retains_udp_packet_clock() {
    let fixture = DirectFixture::new();
    let (mut iq, mut mic) = queues();
    let owner = OwnerStopAdapter::direct(fixture.radio.clone(), &mut iq, &mut mic).unwrap();
    assert_eq!(owner.iq_period_ns(), 0);
    drop(owner);
    let (session, _) = p2();
    let model = model();
    let owner = OwnerStopAdapter::p2(session, &model, &mut iq, &mut mic).unwrap();
    assert_eq!(owner.iq_period_ns(), 1_250_000);
}

#[test]
fn close_publishes_closed_and_revision_together_before_paused_reopen() {
    use std::sync::mpsc;
    let fence = Arc::new(OutputFence::new_open_for_offline_trial());
    fence.close(); // cleaned, closed baseline eligible for generation 1
    let old_revision = fence.revision();
    let (paused_tx, paused_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let worker_fence = fence.clone();
    let cancellation = thread::spawn(move || {
        worker_fence.close_transition(|| {
            // Former closed=true / revision increment split point. Both now
            // publish together, before cancellation is allowed to return.
            paused_tx.send(()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
    });
    paused_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(fence.is_closed());
    assert_eq!(fence.revision(), old_revision + 1);
    assert_eq!(
        fence.reopen(old_revision, 1),
        Err(Fault::GenerationMismatch)
    );
    assert!(fence.enter_generation(1).is_err());
    resume_tx.send(()).unwrap();
    cancellation.join().unwrap();
    assert!(fence.is_closed());
    assert!(fence.enter_generation(1).is_err());

    // Positive fence control. Owner tests below additionally require a new
    // sealed operator action and completed native cleanup before calling this.
    fence.reopen(fence.revision(), 2).unwrap();
    assert!(fence.enter_generation(1).is_err());
    assert!(fence.enter_generation(2).unwrap().is_some());
}

#[test]
fn cancellation_does_not_wait_on_permitted_writer_and_prevents_new_permits() {
    use std::sync::mpsc;
    let fence = Arc::new(OutputFence::new_open_for_offline_trial());
    fence.close();
    fence.reopen(fence.revision(), 1).unwrap();
    let already_permitted = fence.enter_generation(1).unwrap().unwrap();
    let (completed_tx, completed_rx) = mpsc::channel();
    let worker_fence = fence.clone();
    let cancellation = thread::spawn(move || {
        worker_fence.close();
        completed_tx.send(()).unwrap();
    });
    // Cancellation must finish while the writer permit is still held. This
    // write is in flight, NOT retroactively revoked or reported as drained.
    completed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    cancellation.join().unwrap();
    assert!(fence.is_closed());
    assert!(fence.enter_generation(1).is_err());
    assert!(fence.drained().unwrap().is_none());
    assert_eq!(
        fence.reopen(fence.revision(), 2),
        Err(Fault::CleanupPending)
    );
    drop(already_permitted);
    assert!(fence.drained().unwrap().is_some());
    fence.reopen(fence.revision(), 2).unwrap();
    assert!(fence.enter_generation(2).unwrap().is_some());
}

#[test]
fn exhausted_fence_revision_is_permanently_closed_without_wrapping() {
    let fence = OutputFence::new_open_for_offline_trial();
    fence
        .state
        .store(OutputFence::MAX_REVISION - 1, Ordering::SeqCst);
    fence.close();
    fence.close();
    assert_eq!(fence.revision(), OutputFence::MAX_REVISION);
    assert!(fence.is_closed());
    assert_eq!(
        fence.reopen(fence.revision(), 1),
        Err(Fault::GenerationMismatch)
    );
    assert!(fence.enter_generation(1).is_err());
}
#[test]
fn cancellation_ticket_and_closed_revision_publish_together_without_waiting_on_writer() {
    let fence = OutputFence::new_open_for_offline_trial();
    let held = fence.enter().unwrap();
    let old_revision = fence.revision();
    assert_eq!(
        fence.cancel_transition(|| {
            assert!(fence.is_closed());
            assert_eq!(fence.cancellation_ticket(), 1);
            assert_eq!(fence.revision(), old_revision + 1);
            assert!(fence.enter_generation(0).is_err());
        }),
        1
    );
    assert!(fence.drained().unwrap().is_none());
    drop(held);
    fence.close();
    assert_eq!(
        fence.cancellation_ticket(),
        1,
        "stop/fault closure is not a second release"
    );
    fence.reopen(fence.revision(), 1).unwrap();
    let stamp = OutputStamp {
        generation: 1,
        revision: fence.revision(),
    };
    assert!(fence.cancel_generation_transition(stamp, || {
        assert!(fence.is_closed());
        assert_eq!(fence.cancellation_ticket(), 2);
    }));
    assert!(!fence.cancel_generation(stamp));
    assert_eq!(
        fence.cancellation_ticket(),
        2,
        "duplicate stale cancel cannot publish a ticket"
    );
    fence.reopen(fence.revision(), 2).unwrap();
    assert!(!fence.cancel_generation(stamp));
    assert_eq!(fence.cancellation_ticket(), 2);
    assert!(fence.enter_generation(2).unwrap().is_some());
}
#[test]
fn cancellation_counter_exhaustion_is_terminal_and_never_reopens() {
    let fence = OutputFence::new_open_for_offline_trial();
    fence.state.store(
        (OutputFence::MAX_RELEASE - 1) << OutputFence::RELEASE_SHIFT,
        Ordering::SeqCst,
    );
    assert_eq!(fence.cancel(), u64::MAX);
    assert_eq!(fence.cancellation_ticket(), u64::MAX);
    assert_eq!(fence.revision(), OutputFence::MAX_REVISION);
    assert!(fence.is_closed());
    fence.close();
    assert_eq!(fence.cancel(), u64::MAX);
    assert!(fence.reopen(fence.revision(), 1).is_err());
    assert!(fence.enter_generation(1).is_err());
}
#[test]
fn owner_health_distinguishes_coherent_cancel_from_real_revocation_and_raw_closure() {
    use crate::tci::ps_authorization::OperatorIngress;
    for case in ["cancel", "generation-cancel", "identity", "raw-close"] {
        let d = DirectFixture::new();
        d.fifo_ready();
        let mut m = model();
        m.desired.tx_drive = 5;
        let (mut iq, mut mic) = queues();
        let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
        owner.enable_offline_handoff(&m, 5).unwrap();
        let r = request(Backend::DirectXdma, 1);
        owner.begin_stop(r).unwrap();
        owner.poll_stop(1).unwrap();
        owner.cleanup_complete(1, r.fence).unwrap();
        let issuer = OperatorIngress::fixture();
        let action = owner
            .authorize(
                issuer
                    .receive(7, "trx:0,true", 0, r.fence, owner.ready_revision().unwrap())
                    .unwrap(),
            )
            .unwrap();
        owner.begin_arm(action, 1).unwrap();
        let stamp = owner.output_stamp().unwrap();
        assert!(owner.poll_health().is_ok()); // first mailbox/health snapshot
        match case {
            "cancel" => {
                owner.fence.cancel();
            }
            "generation-cancel" => {
                assert!(owner.fence.cancel_generation(stamp));
            }
            "identity" => issuer.disconnect(),
            "raw-close" => owner.fence.close(),
            _ => unreachable!(),
        }
        if case.ends_with("cancel") {
            assert_eq!(owner.poll_health(), Ok(()));
            assert!(owner.poll_key(1).unwrap().is_none());
            assert_eq!(owner.fence.cancellation_ticket(), 1);
        } else {
            assert_eq!(owner.poll_health(), Err(Fault::TxAuthority));
            assert_eq!(owner.fence.cancellation_ticket(), 0);
        }
        owner.begin_stop(request(Backend::DirectXdma, 2)).unwrap();
        assert!(owner.poll_stop(2).unwrap().is_some());
        d.assert_off();
    }
}
#[test]
fn cancellation_just_before_key_permit_never_writes_or_becomes_authority_fault() {
    use crate::tci::ps_authorization::OperatorIngress;
    for backend in [Backend::P2, Backend::DirectXdma] {
        let d = (backend == Backend::DirectXdma).then(DirectFixture::new);
        if let Some(d) = &d {
            d.fifo_ready();
        }
        let p = (backend == Backend::P2).then(p2);
        let mut m = model();
        m.desired.tx_drive = 5;
        let (mut iq, mut mic) = queues();
        let mut owner = if let Some((session, _)) = &p {
            OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap()
        } else {
            OwnerStopAdapter::direct(d.as_ref().unwrap().radio.clone(), &mut iq, &mut mic).unwrap()
        };
        owner.enable_offline_handoff(&m, 5).unwrap();
        let r = request(backend, 1);
        owner.begin_stop(r).unwrap();
        let start = Instant::now();
        for i in 0..12 {
            owner
                .progress(1, start + Duration::from_millis(i * 5))
                .unwrap();
            if let Some((_, socket)) = &p {
                packet(socket);
            }
        }
        owner.cleanup_complete(1, r.fence).unwrap();
        let issuer = OperatorIngress::fixture();
        let action = owner
            .authorize(
                issuer
                    .receive(7, "trx:0,true", 0, r.fence, owner.ready_revision().unwrap())
                    .unwrap(),
            )
            .unwrap();
        owner.begin_arm(action, 1).unwrap();
        let stamp = owner.output_stamp().unwrap();
        let cancellation = owner.fence.clone();
        owner.before_key_permit = Some(Box::new(move || {
            cancellation.cancel();
        }));
        for now in 1..100 {
            owner.enqueue_mic(stamp, now, &[0.2; 512]).unwrap();
            owner.take_mic(stamp).unwrap();
            owner.enqueue_iq(stamp, now, &[0.2; 480]).unwrap();
            assert!(owner.poll_key(now).unwrap().is_none());
            if owner.fence.is_closed() {
                break;
            }
        }
        assert!(
            owner.before_key_permit.is_none(),
            "key eligibility reached actual permit boundary"
        );
        assert_eq!(owner.fence.cancellation_ticket(), 1);
        assert_eq!(owner.poll_health(), Ok(()));
        if let Some(d) = &d {
            assert_eq!(d.dma_len(), 0);
            d.assert_off();
        }
        if let Some((_, socket)) = &p {
            socket.set_nonblocking(true).unwrap();
            assert!(
                socket.recv(&mut [0; 2048]).is_err(),
                "no key or IQ packet written after cancellation"
            );
            socket.set_nonblocking(false).unwrap();
        }
        owner.begin_stop(request(backend, 2)).unwrap();
        let start = Instant::now();
        for i in 0..12 {
            owner
                .progress(2, start + Duration::from_millis(i * 5))
                .unwrap();
            if let Some((_, socket)) = &p {
                packet(socket);
            }
        }
        assert!(owner.queues_empty());
    }
}

#[test]
fn close_between_generation_store_and_reopen_cas_cannot_be_overwritten() {
    let fence = OutputFence::new_open_for_offline_trial();
    fence.close();
    let revision = fence.revision();
    assert_eq!(
        fence.reopen_transition(revision, 1, || fence.close()),
        Err(Fault::GenerationMismatch)
    );
    assert!(fence.is_closed());
    assert!(fence.enter_generation(1).is_err());
    fence.reopen(fence.revision(), 2).unwrap();
    assert!(fence.enter_generation(2).unwrap().is_some());
}

#[test]
fn overlapping_close_rejects_old_credential_then_clean_fresh_action_reopens() {
    use crate::tci::ps_authorization::OperatorIngress;
    use std::sync::mpsc;
    let d = DirectFixture::new();
    let mut m = model();
    m.desired.tx_drive = 5;
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let r = request(Backend::DirectXdma, 1);
    owner.begin_stop(r).unwrap();
    owner.poll_stop(1).unwrap();
    owner.cleanup_complete(1, r.fence).unwrap();
    let ingress = OperatorIngress::fixture();
    let old = ingress
        .receive(7, "trx:0,true", 0, r.fence, owner.ready_revision().unwrap())
        .unwrap();
    let action = owner.authorize(old).unwrap();
    let worker_fence = owner.fence.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let cancellation = thread::spawn(move || {
        worker_fence.close_transition(|| {
            paused_tx.send(()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
    });
    paused_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(owner.begin_arm(action, 1).is_err());
    assert!(owner.fence.enter_generation(1).is_err());
    resume_tx.send(()).unwrap();
    cancellation.join().unwrap();
    assert!(owner.fence.is_closed());

    let next = request(Backend::DirectXdma, 2);
    owner.begin_stop(next).unwrap();
    owner.poll_stop(2).unwrap();
    assert!(owner.ready_revision().is_err());
    owner.cleanup_complete(2, next.fence).unwrap();
    let fresh = ingress
        .receive(
            7,
            "trx:0,true",
            1,
            next.fence,
            owner.ready_revision().unwrap(),
        )
        .unwrap();
    let fresh_action = owner.authorize(fresh).unwrap();
    owner.begin_arm(fresh_action, 2).unwrap();
    assert!(owner.fence.enter_generation(1).is_err());
    assert!(owner.fence.enter_generation(2).unwrap().is_some());
    d.assert_off(); // opening the output generation alone does NOT key RF
}
fn request(backend: Backend, id: u64) -> StopRequest {
    StopRequest {
        id,
        backend,
        fence: Fence {
            boot: 1,
            config: 2,
            generation: 3,
            tx_epoch: 4,
        },
        cause: StopCause::Fault(Fault::Stale),
        notified_ns: 100,
    }
}
pub(crate) fn model() -> RadioModel {
    let mut m = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
    m.desired.tx_enabled = true;
    m.desired.running = true;
    m.desired.tx_drive = 23;
    m.desired.pure_signal_auto_attenuate = false; // fixture's manual-gain contract
    m
}
pub(super) fn queues() -> (VecDeque<f32>, VecDeque<f32>) {
    (
        VecDeque::from(vec![0.3; 1920]),
        VecDeque::from(vec![0.1; 512]),
    )
}
pub(super) fn p2() -> (Arc<P2Session>, UdpSocket) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(80)))
        .unwrap();
    let mut cfg = BridgeConfig {
        client_bind_addr: "127.0.0.1:0".parse().unwrap(),
        radio_command_addr: socket.local_addr().unwrap(),
        high_priority_period: Duration::from_millis(1),
        ..BridgeConfig::default()
    };
    cfg.port_map.high_priority_to_sdr = socket.local_addr().unwrap().port();
    cfg.port_map.duc_iq = socket.local_addr().unwrap().port();
    (
        Arc::new(P2Session::bind_offline_fenced(cfg).unwrap()),
        socket,
    )
}
// This fixture only binds loopback UDP lanes; both native and stub owner tests use it.
pub(crate) fn p2_separate_lanes() -> (Arc<P2Session>, UdpSocket, UdpSocket) {
    let hp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let iq = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut cfg = BridgeConfig {
        client_bind_addr: "127.0.0.1:0".parse().unwrap(),
        radio_command_addr: hp.local_addr().unwrap(),
        high_priority_period: Duration::from_millis(1),
        ..BridgeConfig::default()
    };
    cfg.port_map.high_priority_to_sdr = hp.local_addr().unwrap().port();
    cfg.port_map.duc_iq = iq.local_addr().unwrap().port();
    (
        Arc::new(P2Session::bind_offline_fenced(cfg).unwrap()),
        hp,
        iq,
    )
}
#[test]
fn native_output_tail_retains_samples_and_oldest_observation_across_append() {
    use crate::tci::ps_authorization::OperatorIngress;
    let d = DirectFixture::new();
    let mut m = model();
    m.desired.tx_drive = 5;
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let r = request(Backend::DirectXdma, 1);
    owner.begin_stop(r).unwrap();
    owner.poll_stop(1).unwrap();
    owner.cleanup_complete(1, r.fence).unwrap();
    let ingress = OperatorIngress::fixture();
    let revision = owner.ready_revision().unwrap();
    let action = owner
        .authorize(
            ingress
                .receive(7, "trx:0,true", 0, r.fence, revision)
                .unwrap(),
        )
        .unwrap();
    owner.begin_arm(action, 1).unwrap();
    let stamp = owner.output_stamp().unwrap();
    owner.enqueue_iq(stamp, 0, &[0.1; 256]).unwrap();
    owner.enqueue_iq(stamp, 100_000_000, &[0.2; 4096]).unwrap();
    assert_eq!(owner.pending_iq.len(), 4352);
    assert!(owner.pending_iq.iter().take(256).all(|v| *v == 0.1));
    assert!(owner.pending_iq.iter().skip(256).all(|v| *v == 0.2));
    owner.enqueue_mic(stamp, 100_000_000, &[0.2; 512]).unwrap();
    owner.take_mic(stamp).unwrap();
    assert_eq!(owner.poll_key(150_000_001).unwrap_err(), Fault::Stale);
    owner.begin_stop(request(Backend::DirectXdma, 2)).unwrap();
    owner.poll_stop(2).unwrap();
    assert!(owner.queues_empty());
    d.assert_off();
}
fn packet(socket: &UdpSocket) -> Vec<u8> {
    let mut p = vec![0; 2048];
    let n = socket.recv(&mut p).unwrap();
    p.truncate(n);
    p
}
#[test]
fn handoff_requires_native_cleanup_and_sealed_current_operator_action() {
    use crate::tci::ps_authorization::OperatorIngress;
    let d = DirectFixture::new();
    d.fifo_ready();
    let mut m = model();
    m.desired.tx_drive = 5;
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let r = request(Backend::DirectXdma, 1);
    owner.begin_stop(r).unwrap();
    assert!(owner.poll_stop(1).unwrap().is_some());
    assert!(owner.ready_revision().is_err()); // stop receipt alone is not cleanup
    let forged = FreshTxAction {
        sequence: 1,
        release_ticket: 0,
        expected_fence: r.fence,
    };
    assert!(owner.begin_arm(forged, 1).is_err());
    owner.cleanup_complete(1, r.fence).unwrap();
    assert!(owner.begin_arm(forged, 1).is_err()); // no sealed ingress credential
    owner.cleanup_complete(1, r.fence).unwrap();
    let ingress = OperatorIngress::fixture();
    let revision = owner.ready_revision().unwrap();
    assert!(ingress
        .receive(8, "trx:0,true", 0, r.fence, revision)
        .is_err());
    assert!(ingress
        .receive(7, "trx:0,false", 0, r.fence, revision)
        .is_err());
    let credential = ingress
        .receive(7, "trx:0,true", 0, r.fence, revision)
        .unwrap();
    let action = owner.authorize(credential).unwrap();
    ingress.disconnect();
    assert!(owner.begin_arm(action, 1).is_err());
    assert!(owner.fence.is_closed());
    d.assert_off();
}
#[test]
fn release_cancels_grant_and_stale_receipts_cannot_reopen_output() {
    use crate::tci::ps_authorization::OperatorIngress;
    let d = DirectFixture::new();
    let mut m = model();
    m.desired.tx_drive = 5;
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let r = request(Backend::DirectXdma, 1);
    owner.begin_stop(r).unwrap();
    owner.poll_stop(1).unwrap();
    owner.cleanup_complete(1, r.fence).unwrap();
    let ingress = OperatorIngress::fixture();
    let revision = owner.ready_revision().unwrap();
    let credential = ingress
        .receive(7, "trx:0,true", 0, r.fence, revision)
        .unwrap();
    let action = owner.authorize(credential).unwrap();
    owner.release_received(1, 200).unwrap();
    assert!(owner.begin_arm(action, 1).is_err());
    owner.begin_stop(request(Backend::DirectXdma, 2)).unwrap();
    assert!(owner.poll_stop(1).unwrap().is_none());
    assert!(owner.cleanup_complete(1, r.fence).is_err());
    owner.poll_stop(2).unwrap();
    owner.cleanup_complete(2, r.fence).unwrap();
    let stale = ingress
        .receive(7, "trx:0,true", 1, r.fence, revision)
        .unwrap();
    assert!(owner.authorize(stale).is_err());
    assert!(owner.fence.is_closed());
}
#[test]
fn direct_authenticated_generation_preserves_normal_mic_and_iq_gates() {
    use crate::tci::ps_authorization::OperatorIngress;
    let d = DirectFixture::with_key_clock();
    d.fifo_ready();
    let mut m = model();
    m.desired.tx_drive = 5;
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let r = request(Backend::DirectXdma, 1);
    owner.begin_stop(r).unwrap();
    owner.poll_stop(1).unwrap();
    owner.cleanup_complete(1, r.fence).unwrap();
    let ingress = OperatorIngress::fixture();
    let revision = owner.ready_revision().unwrap();
    let action = owner
        .authorize(
            ingress
                .receive(7, "trx:0,true", 0, r.fence, revision)
                .unwrap(),
        )
        .unwrap();
    owner.begin_arm(action, 1).unwrap();
    let stamp = owner.output_stamp().unwrap();
    assert!(owner.poll_stop(1).unwrap().is_none()); // cached receipt revoked
    for n in 0..10 {
        // zero mic must never qualify, despite nonzero IQ
        owner.enqueue_mic(stamp, n, &[0.; 512]).unwrap();
        owner.take_mic(stamp).unwrap();
        owner.enqueue_iq(stamp, n, &[0.2; 480]).unwrap();
        assert!(owner.poll_key(n).unwrap().is_none());
        d.assert_off();
    }
    let mut keyed = false;
    for n in 10..24 {
        d.radio.set_test_key_time(n * 1_000_000);
        owner.enqueue_mic(stamp, n, &[0.2; 512]).unwrap();
        owner.take_mic(stamp).unwrap();
        if owner.pending_iq.is_empty() {
            owner.enqueue_iq(stamp, n, &[0.2; 480]).unwrap();
        }
        if let Some(receipt) = owner.poll_key(n).unwrap() {
            assert_eq!(receipt.output_generation, 1);
            keyed = true;
            break;
        }
    }
    assert!(keyed);
    assert!(d.dma_len() > 0);
    assert!(d.radio.send_duc_iq(&[0.2; 480]).is_err()); // untagged writers stay fenced
    owner.enqueue_iq(stamp, 30, &[0.2; 480]).unwrap();
    owner.begin_stop(request(Backend::DirectXdma, 2)).unwrap();
    assert!(owner.queues_empty());
    owner.poll_stop(2).unwrap();
    d.assert_off();
    assert!(owner.enqueue_iq(stamp, 31, &[0.2; 480]).is_err());
}
#[test]
fn p2_actual_packets_fenced_and_receipt_waits_for_entire_zero_drive_burst() {
    let (session, socket) = p2();
    let m = model();
    let (mut iq, mut mic) = queues();
    session.send_high_priority(&m).unwrap();
    let keyed = packet(&socket);
    assert_eq!(keyed[4] & 2, 2);
    assert_ne!(keyed[345], 0);
    session.send_duc_iq(&[0.2; 480]).unwrap();
    assert_eq!(packet(&socket).len(), 1444);
    let fence = session.offline_output_fence().unwrap();
    let in_flight = fence.enter().unwrap();
    let mut owner = OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap();
    owner.begin_stop(request(Backend::P2, 7)).unwrap();
    assert!(owner.queues_empty());
    assert!(session.send_high_priority(&m).is_err());
    assert!(session.send_duc_iq(&[0.3; 480]).is_err());
    assert!(owner.poll_stop(7).unwrap().is_none());
    drop(in_flight);
    assert!(owner.poll_stop(6).unwrap().is_none());
    let start = Instant::now();
    for index in 0..12 {
        let now = start + Duration::from_millis(index * 5);
        let receipt = owner.progress(7, now).unwrap();
        let p = packet(&socket);
        assert_eq!(p[4] & 2, 0);
        assert_eq!(p[345], 0);
        assert_eq!(u16::from_be_bytes([p[1428], p[1429]]) & 0x0800, 0);
        if index < 11 {
            assert!(receipt.is_none());
            assert!(owner.progress(7, now).unwrap().is_none()); // no sleeps or burst in one step
        } else {
            assert_eq!(receipt.unwrap().evidence, StopEvidence::P2CommandBurstOnly);
        }
    }
    assert!(owner.poll_stop(7).unwrap().is_some());
    assert!(owner
        .begin_arm(
            FreshTxAction {
                sequence: 9,
                release_ticket: 1,
                expected_fence: request(Backend::P2, 7).fence
            },
            7
        )
        .is_err());
    assert!(fence.is_closed());
}
#[test]
fn independent_p2_hp_sender_cannot_rekey_after_stop_receipt() {
    let (session, socket) = p2();
    let m = Arc::new(Mutex::new(model()));
    let stop = Arc::new(AtomicBool::new(false));
    let hp = session
        .spawn_high_priority_loop(m.clone(), stop.clone())
        .unwrap();
    assert_eq!(packet(&socket)[4] & 2, 2);
    let (mut iq, mut mic) = queues();
    let frozen = model();
    let mut owner = OwnerStopAdapter::p2(session.clone(), &frozen, &mut iq, &mut mic).unwrap();
    owner.begin_stop(request(Backend::P2, 1)).unwrap();
    let fence = session.offline_output_fence().unwrap();
    let wait_until = Instant::now() + Duration::from_secs(1);
    let drained = loop {
        if let Some(guard) = fence.drained().unwrap() {
            break guard;
        }
        assert!(Instant::now() < wait_until);
        thread::yield_now();
    };
    // Drain packets already sent before the barrier; not falsely called canceled UDP.
    socket.set_nonblocking(true).unwrap();
    let mut b = [0; 2048];
    while socket.recv(&mut b).is_ok() {}
    socket.set_nonblocking(false).unwrap();
    drop(drained);
    let start = Instant::now();
    for index in 0..12 {
        owner
            .progress(1, start + Duration::from_millis(index * 5))
            .unwrap();
        assert_eq!(packet(&socket)[4] & 2, 0);
    }
    thread::sleep(Duration::from_millis(15));
    assert!(socket.recv(&mut b).is_err()); // model remains TX=true; separate sender shares fence
    stop.store(true, Ordering::Relaxed);
    hp.join().unwrap().unwrap();
}
#[test]
fn direct_actual_write_drains_then_shutdown_readback_certifies_local_completion() {
    let fixture = DirectFixture::new();
    let (mut iq, mut mic) = queues();
    let (ready, release, worker) = fixture.paused_write();
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut owner = OwnerStopAdapter::direct(fixture.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.begin_stop(request(Backend::DirectXdma, 5)).unwrap();
    assert!(owner.queues_empty());
    assert!(owner.poll_stop(5).unwrap().is_none());
    assert_eq!(fixture.dma_len(), 0); // pending output was not flushed as uncorrected samples
    assert!(fixture.radio.send_duc_iq_batch(&[0.2; 480]).is_err());
    assert!(fixture
        .radio
        .try_key_with_iq(&model(), &[0.2; 480])
        .is_err());
    release.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(fixture.dma_len(), 1440); // permitted pre-stop write completed, then barrier drained
    let receipt = owner.poll_stop(5).unwrap().unwrap();
    assert_eq!(receipt.id, 5);
    assert_eq!(receipt.evidence, StopEvidence::DirectControlReadback);
    fixture.assert_off();
    assert_eq!(fixture.radio.snapshot().dma_writes, 1);
    assert!(fixture
        .radio
        .stage_iq_batch_rf_disabled(&model(), &[0.2; 480])
        .is_err());
    assert_eq!(fixture.radio.snapshot().dma_writes, 1);
}
#[test]
fn direct_register_io_failure_never_creates_receipt() {
    let fixture = DirectFixture::new();
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::direct(fixture.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.begin_stop(request(Backend::DirectXdma, 1)).unwrap();
    fixture.damage_register_file();
    assert!(matches!(owner.poll_stop(1), Err(Fault::TxAuthority)));
    assert!(matches!(owner.poll_stop(1), Err(Fault::TxAuthority)));
    assert!(fixture.radio.offline_output_fence().unwrap().is_closed());
}
#[test]
fn malformed_or_replayed_stop_is_fenced_without_valid_completion() {
    let (session, _) = p2();
    let m = model();
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap();
    assert_eq!(
        owner.begin_stop(request(Backend::DirectXdma, 1)),
        Err(Fault::TxAuthority)
    );
    assert!(owner.queues_empty());
    assert!(session.send_high_priority(&m).is_err());
    assert!(matches!(owner.poll_stop(1), Err(Fault::TxAuthority)));
}
#[test]
fn p2_udp_send_error_cannot_complete_stop_burst() {
    let mut cfg = BridgeConfig {
        client_bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..BridgeConfig::default()
    };
    cfg.port_map.high_priority_to_sdr = 0; // actual send_to() EINVAL, not a simulated fault
    let session = Arc::new(P2Session::bind_offline_fenced(cfg).unwrap());
    let m = model();
    let (mut iq, mut mic) = queues();
    let mut owner = OwnerStopAdapter::p2(session, &m, &mut iq, &mut mic).unwrap();
    owner.begin_stop(request(Backend::P2, 1)).unwrap();
    assert!(matches!(owner.poll_stop(1), Err(Fault::TxAuthority)));
    assert!(matches!(owner.poll_stop(1), Err(Fault::TxAuthority)));
    assert_eq!(owner.commands, 0);
}
