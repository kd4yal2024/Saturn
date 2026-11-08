//! Exercise the scheduling driver and migrated consumer, not direct lifecycle
//! stepping. Both adapters are actual loopback/file owners. No hardware exists.
#[cfg(all(
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
use super::tests::p2_separate_lanes;
use super::{
    tests::{model, queues},
    *,
};
use crate::{
    tci::ps_authorization::OperatorIngress,
    tx_audio::{TxAudioFrame, TxAudioMessage, TxAudioSamples, TxAudioSource},
    tx_thread::{
        ps_owner_loop::{Audio, Configuration, OwnerLoop, ScheduledNative, Timing},
        ps_owner_queue::AuthorizedArm,
        TxCommand,
    },
    xdma_tx_radio::offline_fixture::DirectFixture,
};
use saturn_bridge::puresignal::{
    admission_proposal::DraftLimits,
    controller::{Config, Controller, NativePs, Qualification},
    tx_lifecycle::{CleanupProgress, CooperativeNative, Limits, Phase, TxLifecycle},
    *,
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{self, SyncSender},
};

#[derive(Default)]
struct Trace {
    exchanges: u64,
    renders: u64,
    nonzero_renders: u64,
    fits: u64,
    ps_enabled: bool,
    models: Vec<i32>,
    cleanup: u64,
    keyed: bool,
    cancel_in_render: Option<crate::tx_thread::ps_owner_loop::Cancel>,
    process_fault: Option<Fault>,
    info_fault_bits: i32,
    model_fault: Option<Fault>,
    cancel_in_configure: Option<crate::tx_thread::ps_owner_loop::Cancel>,
    cancel_in_information: Option<crate::tx_thread::ps_owner_loop::Cancel>,
}
struct Fake {
    trace: Rc<RefCell<Trace>>,
}

fn remote_media_scenario(case: &str) {
    let d = DirectFixture::with_key_clock();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let issuer = OperatorIngress::fixture();
    let (clients, operator) = issuer.fixture_registry();
    for c in clients.lock().unwrap().values_mut() {
        c.state.tx_codec_runtime_flags.opus_decode_enabled = true;
    }
    let origin = Instant::now();
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        issuer,
        Arc::new(Mutex::new(m.clone())),
        origin,
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    ingress.enable_remote_media_review().unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    ingress.bootstrap(m, cfg(11), TxAudioSource::Tci).unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    let (legacy, rx) = mpsc::channel();
    let control = Arc::new(Mutex::new(None));
    let dispatch = |id, text: &str| {
        crate::tci::handle_incoming_message_shared(
            tungstenite::Message::Text(text.into()),
            &legacy,
            &clients,
            &operator,
            &control,
            id,
            Some(&ingress),
        );
    };
    dispatch(
        7,
        "session_open:media-fixture,operator;session_lane:media-fixture,control;",
    );
    dispatch(
        8,
        "session_open:media-fixture,viewer;session_lane:media-fixture,media;",
    );
    let opus = matches!(case, "opus" | "stale");
    dispatch(
        7,
        if opus {
            "tx_codec_caps:pcm,opus_wb;"
        } else {
            "tx_codec_caps:pcm;"
        },
    );
    while let Ok(c) = rx.try_recv() {
        assert!(!matches!(c, crate::tci::TciCommand::MicAudioFrame(_)));
    }
    dispatch(7, "trx:0,true;");
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.life.status().phase, Phase::AwaitingKey);
    if case == "pcm-delay" {
        std::thread::sleep(Duration::from_millis(70));
    }
    let old = ingress.capture(Instant::now()).unwrap();
    let token = ingress.remote_media_token(&old).unwrap();
    let first = crate::tx_thread::ps_media::tests::frame(token, 0, opus);
    let send = |id, bytes: Vec<u8>| {
        crate::tci::handle_incoming_message_shared(
            tungstenite::Message::Binary(bytes.into()),
            &legacy,
            &clients,
            &operator,
            &control,
            id,
            Some(&ingress),
        );
    };
    send(8, first.clone());
    // The actual handler timestamps arrivals with Instant::now(). Its consumer
    // clock must not remain behind that timestamp when the test was descheduled.
    now = now.max(origin.elapsed().as_nanos() as u64);
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(
        r.life.status().phase,
        Phase::Active,
        "paired tagged media must actually key"
    );
    assert!(trace.borrow().nonzero_renders > 0);
    assert!(r.counters.iq_packets > 0);
    assert!(rx.try_recv().is_err(), "no parallel legacy mic path");
    let before = trace.borrow().nonzero_renders;
    match case {
        "old" => {
            ingress.release();
            for _ in 0..30 {
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.life.status().phase, Phase::ReadyOff);
            dispatch(7, "trx:0,true;");
            now += 1_000_000;
            r.turn(now);
            let fresh = ingress.capture(Instant::now()).unwrap();
            let new = ingress.remote_media_token(&fresh).unwrap();
            assert_ne!(new, token);
            send(8, first); // old packet arrives on the other TCP lane AFTER re-arm
            ingress.stop_remote(&old, Fault::WrongFormat); // delayed old decoder fault
            assert!(ingress.selected_active(TxAudioSource::Tci));
            now += 1_000_000;
            r.turn(now);
            assert_eq!(r.life.status().primary_fault, None);
            assert_eq!(trace.borrow().nonzero_renders, before);
            send(8, crate::tx_thread::ps_media::tests::frame(new, 0, false));
            now = now.max(origin.elapsed().as_nanos() as u64);
            for _ in 0..20 {
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.life.status().phase, Phase::Active);
            assert!(trace.borrow().nonzero_renders > before);
        }
        "gap" | "lane" | "switch" | "stale" => {
            if case == "stale" {
                std::thread::sleep(Duration::from_millis(170));
            }
            let seq = if case == "gap" { 3 } else { 1 };
            let codec = opus || case == "switch";
            send(
                if case == "lane" { 7 } else { 8 },
                crate::tx_thread::ps_media::tests::frame(token, seq, codec),
            );
            assert!(
                !ingress.selected_active(TxAudioSource::Tci),
                "hazard fences output before owner queue work"
            );
            for _ in 0..30 {
                now += 1_000_000;
                r.turn(now);
            }
            let fault = match case {
                "gap" => Fault::SequenceGap,
                "lane" => Fault::WrongRoute,
                "stale" => Fault::Stale,
                _ => Fault::WrongFormat,
            };
            assert_eq!(r.life.status().primary_fault, Some(fault));
            assert_eq!(trace.borrow().nonzero_renders, before);
        }
        _ => {}
    }
    ingress.release();
    for _ in 0..30 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert!(r.life.offline_owner().queues_empty());
    d.assert_off();
}

#[test]
fn hardware_ptt_cancel_after_enqueue_is_not_a_fault_or_a_new_arm() {
    let d = DirectFixture::with_key_clock();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(Fake { trace }, owner, Backend::DirectXdma);
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        OperatorIngress::fixture(),
        Arc::new(Mutex::new(m.clone())),
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    ingress.bootstrap(m, cfg(11), TxAudioSource::Tci).unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    let c = ingress.capture(Instant::now()).unwrap();
    let permit = crate::tci::HardwarePttPermit::new(1, Some(c.hardware_ptt_context()));
    assert!(permit.claim());
    ingress
        .tci(
            7,
            Some(&c),
            &crate::tci::TciCommand::HardwarePttArm(permit.clone()),
        )
        .unwrap();
    // Pause the input caller before it publishes the lifecycle release. The
    // queued arm must already be invalid, without inventing TxAuthority.
    assert!(permit.cancel());
    now += 1_000_000;
    r.turn(now);
    let status = r.life.status();
    assert_eq!(status.phase, Phase::ReadyOff);
    assert!(
        status.primary_fault.is_none()
            && status.authority_fault.is_none()
            && status.cleanup_fault.is_none()
    );
    let c = ingress.capture(Instant::now()).unwrap();
    let fresh = crate::tci::HardwarePttPermit::new(2, Some(c.hardware_ptt_context()));
    assert!(fresh.claim());
    ingress
        .tci(7, Some(&c), &crate::tci::TciCommand::HardwarePttArm(fresh))
        .unwrap();
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.life.status().phase, Phase::AwaitingKey);
    ingress.release();
    for _ in 0..40 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
}
#[test]
fn media_paired_pcm_reaches_shared_owner_without_legacy_tee() {
    remote_media_scenario("pcm");
}

#[test]
fn media_paired_pcm_delayed_handler_uses_current_consumer_time() {
    remote_media_scenario("pcm-delay");
}
#[test]
fn media_paired_opus_reaches_shared_owner_with_real_decode() {
    remote_media_scenario("opus");
}
#[test]
fn media_old_lane_packet_and_delayed_fault_cannot_cross_rearm() {
    remote_media_scenario("old");
}
#[test]
fn media_sequence_gap_closes_output_before_stop_cleanup() {
    remote_media_scenario("gap");
}
#[test]
fn media_wrong_lane_closes_output_before_stop_cleanup() {
    remote_media_scenario("lane");
}
#[test]
fn media_codec_switch_requires_new_generation() {
    remote_media_scenario("switch");
}
#[test]
fn media_stale_opus_history_closes_output_without_another_native_write() {
    remote_media_scenario("stale");
}

fn settings_scenario(case: &str) {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut initial = model();
    initial.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&initial, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let issuer = OperatorIngress::fixture();
    let (clients, operator) = issuer.fixture_registry();
    let model = Arc::new(Mutex::new(initial.clone()));
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        issuer,
        model.clone(),
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    ingress
        .bootstrap(initial, cfg(11), TxAudioSource::Tci)
        .unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.counters.configurations, 1);
    let old = ingress.capture(Instant::now()).unwrap();
    let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
    let (legacy, legacy_rx) = mpsc::channel();
    let control = Arc::new(Mutex::new(None));
    let dispatch = |text: &str| {
        crate::tci::handle_incoming_message_shared(
            tungstenite::Message::Text(text.into()),
            &legacy,
            &clients,
            &operator,
            &control,
            7,
            Some(&ingress),
        );
    };
    // Separate WebSocket messages, with NO owner turn/publication between them.
    dispatch("tx_mic_gain:0,-3;");
    dispatch("tx_eq_enable:0,true;");
    assert_eq!(r.counters.configurations, 1);
    if case == "paused-release" {
        let cancel = r.cancel_handle();
        cancel.offline_release_boundary(|| {
            // The cancellation caller is suspended after its atomic commit.
            // Cleanup and fresh client settings can progress independently.
            for _ in 0..30 {
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.life.status().phase, Phase::ReadyOff);
            assert_eq!(r.life.status().primary_fault, None);
            assert_eq!(r.counters.configurations, 1);
            dispatch("tx_mic_gain:0,-6;");
            for _ in 0..30 {
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.counters.configurations, 2);
            dispatch("tx_eq_enable:0,true;"); // fresh transaction still queued
        }); // resuming cancellation must not invalidate this newer work
        for _ in 0..30 {
            now += 1_000_000;
            r.turn(now);
        }
        assert_eq!(r.counters.configurations, 3);
        assert_eq!(model.lock().unwrap().desired.tx_mic_gain_db, -6.);
        assert!(model.lock().unwrap().desired.tx_eq_enabled);
        let mut replies = Vec::new();
        while let Some(item) = outbound.next_message(false) {
            if let crate::tci::OutboundMessage::TransactionText(text) = item.message {
                replies.push(text);
            }
        }
        for id in [1, 2] {
            assert!(replies
                .iter()
                .any(|s| s.starts_with(&format!("tx_config:0,{id},rejected,"))));
        }
        for id in [3, 4] {
            assert!(replies
                .iter()
                .any(|s| s.starts_with(&format!("tx_config:0,{id},applied,"))));
        }
        let fresh = ingress.capture(Instant::now()).unwrap();
        ingress
            .tci(7, Some(&fresh), &crate::tci::TciCommand::SetTxEnabled(true))
            .unwrap();
        now += 1_000_000;
        r.turn(now);
        assert_eq!(r.life.status().phase, Phase::AwaitingKey);
        assert_eq!(r.life.status().primary_fault, None);
        ingress.release();
        for _ in 0..30 {
            now += 1_000_000;
            r.turn(now);
        }
        assert_eq!(r.life.status().phase, Phase::ReadyOff);
        assert_eq!(r.life.status().primary_fault, None);
        assert!(r.life.offline_owner().queues_empty());
        d.assert_off();
        assert!(legacy_rx.try_recv().is_err());
        return;
    }
    let count = if case == "queue-full" {
        for _ in 2..17 {
            dispatch("tx_cfc_enable:0,true;");
        }
        17
    } else {
        2
    };
    if matches!(
        case,
        "release"
            | "disconnect"
            | "native-failure"
            | "install-release"
            | "authority-timeout"
            | "cleanup-timeout"
    ) {
        now += 1_000_000;
        r.turn(now); // first configuration is IN FLIGHT, second still queued
        assert_ne!(r.life.status().phase, Phase::ReadyOff);
        assert_eq!(r.counters.configurations, 1);
        match case {
            "release" => {
                dispatch("trx:0,false;");
            }
            "disconnect" => {
                crate::tci::handle_incoming_message_shared(
                    tungstenite::Message::Close(None),
                    &legacy,
                    &clients,
                    &operator,
                    &control,
                    7,
                    Some(&ingress),
                );
                clients
                    .lock()
                    .unwrap()
                    .get(&7)
                    .unwrap()
                    .state
                    .authorization_live
                    .store(false, Ordering::SeqCst);
            }
            "install-release" => {
                trace.borrow_mut().cancel_in_configure = Some(r.cancel_handle());
            }
            "authority-timeout" => {
                now += 3_000_000_000;
            }
            "cleanup-timeout" => {
                now += 1_000_000;
                r.turn(now);
                assert_eq!(r.life.status().phase, Phase::CleaningNative);
                now += 4_000_000_000;
            }
            _ => {
                trace.borrow_mut().model_fault = Some(Fault::NativeUnverified);
            }
        }
    }
    for _ in 0..100 {
        now += 1_000_000;
        r.turn(now);
    }
    let mut replies = Vec::new();
    while let Some(item) = outbound.next_message(false) {
        if let crate::tci::OutboundMessage::TransactionText(text) = item.message {
            if text.starts_with("tx_config:") {
                replies.push(text);
            }
        }
    }
    for id in 1..=count {
        assert_eq!(
            replies
                .iter()
                .filter(|s| s.starts_with(&format!("tx_config:0,{id},queued,")))
                .count(),
            1,
            "{replies:?}"
        );
        let terminal: Vec<_> = replies
            .iter()
            .filter(|s| {
                s.starts_with(&format!("tx_config:0,{id},applied,"))
                    || s.starts_with(&format!("tx_config:0,{id},rejected,"))
            })
            .collect();
        assert_eq!(
            terminal.len(),
            1,
            "one terminal result for each separate message: {replies:?}"
        );
        assert!(
            terminal[0].contains(if case == "ordered" {
                ",applied,"
            } else {
                ",rejected,"
            }),
            "{replies:?}"
        );
    }
    assert_eq!(trace.borrow().nonzero_renders, 0);
    assert!(r.life.offline_owner().queues_empty());
    d.assert_off();
    assert!(legacy_rx.try_recv().is_err());
    if case == "ordered" {
        let m = model.lock().unwrap();
        assert_eq!(m.desired.tx_mic_gain_db, -3.);
        assert!(m.desired.tx_eq_enabled);
        drop(m);
        assert_eq!(r.counters.configurations, 3);
        assert_eq!(r.life.fence().config, 13);
        assert_eq!(
            replies,
            vec![
                "tx_config:0,1,queued,12;",
                "tx_config:0,2,queued,13;",
                "tx_config:0,1,applied,12;",
                "tx_config:0,2,applied,13;"
            ]
        );
        let rejected = r.counters.rejected;
        ingress
            .tci(7, Some(&old), &crate::tci::TciCommand::SetTxEnabled(true))
            .unwrap();
        now += 1_000_000;
        r.turn(now);
        assert_eq!(r.life.status().phase, Phase::ReadyOff);
        assert_eq!(
            r.counters.rejected,
            rejected + 1,
            "settings continuation must NOT refresh old arm"
        );
    } else {
        assert_eq!(r.counters.configurations, 1);
        assert_ne!(model.lock().unwrap().desired.tx_mic_gain_db, -3.);
        if matches!(case, "release" | "install-release") {
            assert_eq!(r.life.status().primary_fault, None);
            assert_eq!(
                ingress.configure_batch(
                    7,
                    Some(&old),
                    &[crate::tci::TciCommand::SetTxMicGain(-9.)]
                ),
                Err(Fault::GenerationMismatch)
            );
            dispatch("tx_mic_gain:0,-6;"); // genuinely fresh config after cancellation
            for _ in 0..30 {
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.counters.configurations, 2);
            assert_eq!(model.lock().unwrap().desired.tx_mic_gain_db, -6.);
        }
    }
    if matches!(case, "ordered" | "release" | "install-release") {
        let fresh = ingress.capture(Instant::now()).unwrap();
        ingress
            .tci(7, Some(&fresh), &crate::tci::TciCommand::SetTxEnabled(true))
            .unwrap();
        now += 1_000_000;
        r.turn(now);
        assert_eq!(
            r.life.status().phase,
            Phase::AwaitingKey,
            "only a fresh authorized action arms"
        );
        ingress.release();
        for _ in 0..30 {
            now += 1_000_000;
            r.turn(now);
        }
        assert_eq!(r.life.status().phase, Phase::ReadyOff);
    }
}
#[test]
fn settings_after_completed_cleanup_survive_resuming_cancel_caller() {
    settings_scenario("paused-release");
}
#[test]
fn settings_separate_messages_ordered_completion_and_fresh_arm() {
    settings_scenario("ordered");
}
#[test]
fn settings_release_during_pending_update_cancels_chain_and_allows_fresh_config_arm() {
    settings_scenario("release");
}
#[test]
fn settings_disconnect_during_pending_update_rejects_entire_chain() {
    settings_scenario("disconnect");
}
#[test]
fn settings_release_inside_native_installation_remains_normal_cancellation() {
    settings_scenario("install-release");
}
#[test]
fn settings_stop_receipt_timeout_reports_rejection_instead_of_hanging_pending() {
    settings_scenario("authority-timeout");
}
#[test]
fn settings_native_cleanup_timeout_reports_rejection_instead_of_hanging_pending() {
    settings_scenario("cleanup-timeout");
}
#[test]
fn settings_native_failure_rejects_transaction_and_dependent_successor() {
    settings_scenario("native-failure");
}
#[test]
fn settings_full_command_queue_reports_every_rejection() {
    settings_scenario("queue-full");
}
#[test]
fn settings_reply_queue_stall_revokes_session_without_evicting_receipts() {
    let issuer = OperatorIngress::fixture();
    let permission = issuer.settings_permission(7).unwrap();
    let (clients, _) = issuer.fixture_registry();
    let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
    for id in 1..=64 {
        assert!(
            issuer.settings_reply(7, format!("tx_config:0,{id},applied,{id};"), || panic!(
                "unexpected backpressure"
            ))
        );
    }
    // Unrelated control/safety churn may not coalesce or evict receipts.
    for _ in 0..300 {
        outbound.enqueue(crate::tci::OutboundMessage::SafetyText(
            "tx_fault:0,other,Stale;".into(),
        ));
        outbound.enqueue(crate::tci::OutboundMessage::Text("vfo:0,0,7200000;".into()));
    }
    assert!(permission.valid());
    let canceled = std::cell::Cell::new(false);
    assert!(
        !issuer.settings_reply(7, "tx_config:0,65,applied,65;".into(), || {
            assert!(permission.valid(), "cancel precedes authority revocation");
            canceled.set(true);
        })
    );
    assert!(canceled.get());
    assert!(
        !permission.valid(),
        "undeliverable reply revokes further TX/settings authority"
    );
    let mut replies = Vec::new();
    let mut close = false;
    while let Some(item) = outbound.next_message(false) {
        match item.message {
            crate::tci::OutboundMessage::TransactionText(s) => replies.push(s),
            crate::tci::OutboundMessage::Close => close = true,
            _ => {}
        }
    }
    assert!(close);
    assert_eq!(replies.len(), 64);
    for (i, s) in replies.iter().enumerate() {
        let id = i + 1;
        assert_eq!(s, &format!("tx_config:0,{id},applied,{id};"));
    }
}

#[test]
fn shared_ingress_full_command_queue_release_cancels_old_authority() {
    use crate::tx_thread::ps_ingress::SharedIngress;
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let (mut r, ingress): (OwnerLoop<'_, Fake>, SharedIngress) = OwnerLoop::with_shared_ingress(
        life,
        OperatorIngress::fixture(),
        Arc::new(Mutex::new(m.clone())),
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    ingress.bootstrap(m, cfg(11), TxAudioSource::Tci).unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.counters.configurations == 1 {
            break;
        }
    }
    let capture = ingress
        .capture(r_origin(&r) + Duration::from_nanos(now))
        .unwrap();
    for _ in 0..16 {
        ingress
            .tci(
                7,
                Some(&capture),
                &crate::tci::TciCommand::SetTxEnabled(true),
            )
            .unwrap();
    }
    ingress.release(); // no command slot needed, all 16 arm envelopes are old
    for _ in 0..40 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert_eq!(r.life.status().primary_fault, None);
    assert_eq!(trace.borrow().renders, 0);
    assert!(r.life.offline_owner().queues_empty());
    d.assert_off();
    let fresh = ingress
        .capture(r_origin(&r) + Duration::from_nanos(now))
        .unwrap();
    ingress
        .tci(7, Some(&fresh), &crate::tci::TciCommand::SetTxEnabled(true))
        .unwrap();
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.life.status().phase, Phase::AwaitingKey);
    ingress.shutdown().unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    d.assert_off();
}

#[test]
fn shared_ingress_does_not_refresh_delayed_pcm_at_dequeue() {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        OperatorIngress::fixture(),
        Arc::new(Mutex::new(m.clone())),
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    ingress.bootstrap(m, cfg(11), TxAudioSource::Tci).unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.counters.configurations == 1 {
            break;
        }
    }
    let c = ingress
        .capture(r_origin(&r) + Duration::from_nanos(now))
        .unwrap();
    ingress
        .tci(7, Some(&c), &crate::tci::TciCommand::SetTxEnabled(true))
        .unwrap();
    now += 1_000_000;
    r.turn(now);
    let arrived = r_origin(&r) + Duration::from_nanos(now);
    let c = ingress.capture(arrived).unwrap();
    ingress
        .enqueue_audio(
            &c,
            TxAudioFrame {
                enqueued_at: arrived,
                source: TxAudioSource::Tci,
                samples: TxAudioSamples::Dynamic(vec![0.3; 512]),
                channels: 1,
                sample_rate_hz: 48000,
            },
        )
        .unwrap();
    now += 160_000_000;
    r.turn(now);
    assert_eq!(trace.borrow().nonzero_renders, 0); // zero-input DSP may advance
    assert!(r.counters.rejected > 0);
    now += 500_000_000;
    r.turn(now); // no new source arrival can hide the deadline
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    assert_eq!(r.life.status().primary_fault, Some(Fault::Stale));
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    d.assert_off();
}

#[test]
fn shared_ingress_actual_role_parser_batches_config_and_release_cancels_batch() {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let issuer = OperatorIngress::fixture();
    let (clients, operator) = issuer.fixture_registry();
    let model = Arc::new(Mutex::new(m.clone()));
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        issuer,
        model.clone(),
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    ingress.bootstrap(m, cfg(11), TxAudioSource::Tci).unwrap();
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.counters.configurations == 1 {
            break;
        }
    }
    let (legacy, rx) = mpsc::channel();
    let control = Arc::new(Mutex::new(None));
    let dispatch = |client, text: &str| {
        crate::tci::handle_incoming_message_shared(
            tungstenite::Message::Text(text.into()),
            &legacy,
            &clients,
            &operator,
            &control,
            client,
            Some(&ingress),
        )
    };
    dispatch(8, "tx_mic_gain:0,-9;trx:0,true;");
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.counters.configurations, 1);
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    model.lock().unwrap().desired.rx_volume_db = -7.;
    dispatch(
        7,
        "tx_mic_gain:0,-3;tx_eq_enable:0,true;tx_cfc_enable:0,true;",
    );
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
        if r.counters.configurations == 2 {
            break;
        }
    }
    {
        let m = model.lock().unwrap();
        assert_eq!(m.desired.tx_mic_gain_db, -3.);
        assert!(m.desired.tx_eq_enabled && m.desired.cfc_enabled);
        assert_eq!(
            m.desired.rx_volume_db, -7.,
            "TX commit must preserve unrelated RX state"
        );
    }
    assert_eq!(r.counters.configurations, 2);
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    dispatch(7, "tx_mic_gain:0,-12;trx:0,false;trx:0,true;");
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.counters.configurations, 2);
    assert_eq!(model.lock().unwrap().desired.tx_mic_gain_db, -3.);
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert_eq!(r.life.status().primary_fault, None);
    assert_eq!(trace.borrow().nonzero_renders, 0);
    assert!(rx.try_recv().is_err());
    let mut encoded = crate::tci::build_tci_audio_frame(0, 48000, 1, &[0.3; 512], 1);
    encoded[24..28].copy_from_slice(&2u32.to_le_bytes());
    encoded[36..40].copy_from_slice(&2u32.to_le_bytes()); // Opus WB: reject BEFORE decoder
    crate::tci::handle_incoming_message_shared(
        tungstenite::Message::Binary(encoded.clone().into()),
        &legacy,
        &clients,
        &operator,
        &control,
        7,
        Some(&ingress),
    );
    assert_eq!(
        clients
            .lock()
            .unwrap()
            .get(&7)
            .unwrap()
            .state
            .tx_codec_decode_error_count,
        0
    );
    encoded[36..40].copy_from_slice(&0u32.to_le_bytes());
    crate::tci::set_client_split_session_open(
        &clients,
        7,
        "shared-review",
        crate::tci::TciClientRole::Operator,
    );
    crate::tci::set_client_split_session_lane(
        &clients,
        7,
        "shared-review",
        crate::tci::SplitSocketKind::Control,
    );
    crate::tci::set_client_split_session_open(
        &clients,
        8,
        "shared-review",
        crate::tci::TciClientRole::Viewer,
    );
    crate::tci::set_client_split_session_lane(
        &clients,
        8,
        "shared-review",
        crate::tci::SplitSocketKind::Media,
    );
    assert!(crate::tci::split_media_client_can_supply_mic(
        &clients,
        7,
        8,
        Instant::now()
    ));
    crate::tci::handle_incoming_message_shared(
        tungstenite::Message::Binary(encoded.into()),
        &legacy,
        &clients,
        &operator,
        &control,
        8,
        Some(&ingress),
    );
    now += 1_000_000;
    r.turn(now);
    assert!(rx.try_recv().is_err());
    assert_eq!(trace.borrow().nonzero_renders, 0);
    d.assert_off();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PumpBoundary {
    NativeFailure,
    SevereOverdrive,
    Release,
    MissingAuthority,
    Healthy,
    RetainedRelease,
    RetainedMissingAuthority,
    RetainedHealthy,
    RetainedStale,
}

#[cfg(feature = "managed-puresignal")]
#[test]
fn production_turn_clock_is_sampled_after_dequeue_without_rejuvenating_observations() {
    for future in [false, true] {
        let d = DirectFixture::new();
        d.fifo_ready();
        let (mut iq, mut mic) = queues();
        let mut m = model();
        m.desired.tx_drive = 5;
        let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
        owner.enable_offline_handoff(&m, 5).unwrap();
        let trace = Rc::new(RefCell::new(Trace::default()));
        let life = controller(
            Fake {
                trace: trace.clone(),
            },
            owner,
            Backend::DirectXdma,
        );
        let (commands, rx) = mpsc::sync_channel(16);
        let (audio, arx) = mpsc::sync_channel(16);
        let (fb, frx) = mpsc::sync_channel(2);
        let origin = Instant::now() - Duration::from_secs(1);
        let mut r = OwnerLoop::new(
            life,
            rx,
            arx,
            frx,
            origin,
            Timing {
                mic_age_ns: 150_000_000,
                source_stall_ns: 1_000_000_000,
                tx_duration_ns: 2_000_000_000,
            },
        )
        .unwrap();
        let issuer = OperatorIngress::fixture();
        let mut stage = 0;
        let end = Instant::now() + Duration::from_secs(2);
        while r.life.status().phase != Phase::Active {
            let now = origin.elapsed().as_nanos() as u64;
            if stage == 0 && r.life.status().phase == Phase::ReadyOff {
                send_config(&mut r, &commands, 300, TxAudioSource::Tci, 11);
                stage = 1;
            }
            if stage == 1 && r.counters.configurations == 1 {
                send_arm(&mut r, &commands, &issuer);
                stage = 2;
            }
            if r.life.status().phase == Phase::AwaitingKey && r.counters.mic_blocks == 0 {
                send_audio(&mut r, &audio, now);
            }
            r.turn(now);
            assert!(Instant::now() < end, "{:?}", r.life.status());
            std::thread::sleep(Duration::from_millis(1));
        }
        let tick_now = origin.elapsed().as_nanos() as u64;
        let mut observed = 0;
        r.turn_realtime_after_tick(tick_now, |r| {
            while origin.elapsed().as_nanos() as u64 <= tick_now {
                std::hint::spin_loop();
            }
            observed = origin.elapsed().as_nanos() as u64 + if future { 1_000_000_000 } else { 0 };
            let mut block = PairBlock::default();
            block.pairs = 1024;
            block.tx.fill(0.3);
            block.rf.fill(0.2);
            block.meta = BlockMeta {
                fence: r.life.fence(),
                backend: Backend::DirectXdma,
                route: 20,
                rate_hz: 192000,
                observed_ns: observed,
                sequence: None,
                sequence_packets: 0,
                overload: [None; 3],
                interval: Interval {
                    fence: r.life.fence(),
                    first: 0,
                    end: 1024,
                },
            };
            fb.try_send(Box::new(block)).unwrap();
        });
        assert!(observed > tick_now);
        if future {
            assert_eq!(
                r.life.status().primary_fault,
                Some(Fault::FutureObservation)
            );
        } else {
            assert_eq!(r.life.status().primary_fault, None);
            assert_eq!(r.life.feedback_status().accepted_pairs, 1024);
            assert!(trace.borrow().fits > 0);
        }
        commands.try_send(TxCommand::Shutdown).unwrap();
        while r.life.status().phase != Phase::ReadyOff {
            r.turn(origin.elapsed().as_nanos() as u64);
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        d.assert_off();
    }
}

// Injection is at the actual initial-tick -> pump boundary, with rendering and
// packet sending due. No sleeps or races determine which operation happens.
fn exercise_pump_boundary(case: PumpBoundary) {
    exercise_pump_boundary_backend(case, Backend::DirectXdma);
}
fn exercise_pump_boundary_backend(case: PumpBoundary, backend: Backend) {
    let d = DirectFixture::with_key_clock();
    d.fifo_ready();
    let p2 = (backend == Backend::P2).then(super::tests::p2_separate_lanes);
    if let Some((_, hp, iq)) = &p2 {
        hp.set_nonblocking(true).unwrap();
        iq.set_nonblocking(true).unwrap();
    }
    let drain = || {
        if let Some((_, hp, iq)) = &p2 {
            let mut bytes = [0; 2048];
            while hp.recv(&mut bytes).is_ok() {}
            while iq.recv(&mut bytes).is_ok() {}
        }
    };
    let wait = || {
        if backend == Backend::P2 {
            std::thread::sleep(Duration::from_millis(2));
        }
        drain();
    };
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = if let Some((session, _, _)) = &p2 {
        OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap()
    } else {
        OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap()
    };
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        backend,
    );
    let (commands, rx) = mpsc::sync_channel(16);
    let (audio, arx) = mpsc::sync_channel(16);
    let (fb, frx) = mpsc::sync_channel(2);
    let mut r = OwnerLoop::new(
        life,
        rx,
        arx,
        frx,
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 1_000_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let ingress = OperatorIngress::fixture();
    let mut now = 0;
    let mut stage = 0;
    let mut next_audio = 0;
    // Scheduled setup progresses the actual owner/native lifecycle to Active.
    for _ in 0..100 {
        wait();
        now += 1_000_000;
        match stage {
            0 if r.life.status().phase == Phase::ReadyOff => {
                send_config(&mut r, &commands, 300, TxAudioSource::Tci, 11);
                stage = 1;
            }
            1 if r.counters.configurations == 1 => {
                send_arm(&mut r, &commands, &ingress);
                stage = 2;
            }
            _ => {}
        }
        if r.life.status().phase == Phase::AwaitingKey && now >= next_audio {
            send_audio(&mut r, &audio, now);
            next_audio = now + 11_000_000;
        }
        r.turn(now);
        if r.life.status().phase == Phase::Active {
            break;
        }
    }
    assert_eq!(
        r.life.status().phase,
        Phase::Active,
        "setup failed {:?}",
        r.life.status()
    );
    // Drain setup output before the observed turn, so the rendering guard
    // cannot hide the defect behind a nonempty IQ queue.
    let stamp = r.life.offline_owner().output_stamp().unwrap();
    while r.life.offline_owner().has_iq_packet() {
        assert!(r.life.offline_owner().flush_iq(stamp, now).unwrap());
    }
    let mut first_pair = 0;
    if backend == Backend::P2 {
        // A real eight-packet P2 batch is 952 pairs. Prime the native 1024
        // pair block without rendering, then supply the next sequential batch
        // on the observed turn so both adapters actually call native fitting.
        now += 1_000_000;
        feedback(&mut r, &fb, &mut first_pair, now, backend);
        let renders = trace.borrow().renders;
        r.turn(now);
        assert_eq!(trace.borrow().renders, renders);
        assert_eq!(trace.borrow().fits, 0);
    }
    if case == PumpBoundary::RetainedStale {
        // Stage real source audio while the next render is not due, then let
        // it age beyond 150 ms without changing its ingress timestamp.
        now += 1_000_000;
        let renders = trace.borrow().renders;
        send_audio(&mut r, &audio, now);
        r.turn(now);
        assert_eq!(trace.borrow().renders, renders);
        now += 151_000_000;
    }
    now += 1_000_000;
    r.render_due(now);
    if !matches!(
        case,
        PumpBoundary::MissingAuthority | PumpBoundary::RetainedStale
    ) {
        send_audio(&mut r, &audio, now);
    }
    feedback(&mut r, &fb, &mut first_pair, now, backend);
    let before_render = trace.borrow().renders;
    let before_cleanup = trace.borrow().cleanup;
    let before_fit = trace.borrow().fits;
    let before_exchanges = trace.borrow().exchanges;
    let before_packets = r.counters.iq_packets;
    let before_dma = d.dma_writes();
    let mut hook_ran = false;
    // In the ordinary-release case, hold the actual production queue's
    // producer lock throughout the scheduled stop and native cleanup. A
    // consumer that waits/spins cannot reach the release hook promptly.
    let paused_producer = if case == PumpBoundary::Release {
        let (tx, rx) = crate::tx_thread::owner_mailbox::sync_channel(2);
        r.replace_feedback_receiver(rx);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            tx.send_with_pause(Box::new(PairBlock::default()), || {
                entered_tx.send(()).unwrap();
                // Finite failure bound; an assertion failure cannot strand a
                // helper thread or leave its test mutex held indefinitely.
                let _ = resume_rx.recv_timeout(Duration::from_secs(2));
            })
            .ok();
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        Some((resume_tx, worker, Instant::now()))
    } else {
        None
    };
    r.counters.active_max_turn_ns = 0;
    if matches!(
        case,
        PumpBoundary::RetainedRelease
            | PumpBoundary::RetainedMissingAuthority
            | PumpBoundary::RetainedHealthy
    ) {
        r.turn_with_before_audio_check(
            now,
            match case {
                PumpBoundary::RetainedRelease => |r| r.cancel_handle().release(),
                PumpBoundary::RetainedMissingAuthority => {
                    |r| r.life.offline_owner().disconnect_output()
                }
                PumpBoundary::RetainedHealthy => |_| {},
                _ => unreachable!(),
            },
        );
        hook_ran = true;
    } else {
        r.turn_with_before_pump(now, |r| {
            hook_ran = true;
            assert_eq!(
                r.life.status().phase,
                Phase::Active,
                "initial tick must precede injection"
            );
            assert!(!r.life.offline_owner().has_iq_packet());
            match case {
                PumpBoundary::NativeFailure => {
                    trace.borrow_mut().process_fault = Some(Fault::NativeFailure)
                }
                PumpBoundary::SevereOverdrive => trace.borrow_mut().info_fault_bits = 2,
                PumpBoundary::Release => r.cancel_handle().release(),
                PumpBoundary::MissingAuthority => r.life.offline_owner().disconnect_output(),
                PumpBoundary::Healthy | PumpBoundary::RetainedStale => {}
                _ => unreachable!(),
            }
        });
    }
    assert!(hook_ran);
    let active_turn_ns = r.counters.active_max_turn_ns;
    assert!(
        active_turn_ns > 0,
        "retain the final active turn even when pump stops it"
    );
    if matches!(case, PumpBoundary::Healthy | PumpBoundary::RetainedHealthy) {
        assert_eq!(r.life.status().phase, Phase::Active);
        assert_eq!(r.life.status().primary_fault, None);
        assert_eq!(trace.borrow().renders, before_render + 1);
        assert_eq!(trace.borrow().exchanges, before_exchanges + 1);
        assert_eq!(r.counters.iq_packets, before_packets + 1);
        if backend == Backend::DirectXdma {
            assert_eq!(d.dma_writes(), before_dma + 1);
        }
        commands.try_send(TxCommand::Shutdown).unwrap();
        now += 1_000_000;
        r.turn(now);
    } else {
        let fault = match case {
            PumpBoundary::NativeFailure => Some(Fault::NativeFailure),
            PumpBoundary::SevereOverdrive => Some(Fault::SevereOverdrive),
            PumpBoundary::MissingAuthority | PumpBoundary::RetainedMissingAuthority => {
                Some(Fault::TxAuthority)
            }
            PumpBoundary::Release | PumpBoundary::RetainedRelease => None,
            PumpBoundary::RetainedStale => Some(Fault::Stale),
            PumpBoundary::Healthy | PumpBoundary::RetainedHealthy => unreachable!(),
        };
        assert_eq!(
            r.life.status().primary_fault,
            fault,
            "preserve original fault or normal cancellation"
        );
        assert_eq!(r.life.status().phase, Phase::AwaitingAuthority);
        assert_eq!(
            r.life.status().authority_completed_ns,
            None,
            "closed fence is not a stop receipt"
        );
        assert_eq!(trace.borrow().cleanup, before_cleanup);
        assert_eq!(trace.borrow().renders, before_render);
        assert_eq!(trace.borrow().exchanges, before_exchanges);
        assert_eq!(r.counters.iq_packets, before_packets);
        assert_eq!(d.dma_writes(), before_dma);
        assert!(r.life.offline_owner().queues_empty());
        assert_eq!(
            trace.borrow().fits,
            before_fit + u64::from(case != PumpBoundary::Release)
        );
    }
    // Subsequent scheduled turns, not a second tick in the faulting turn,
    // must obtain the actual fixture receipt and finish native cleanup.
    for _ in 0..100 {
        wait();
        now += 1_000_000;
        let exchanges = trace.borrow().exchanges;
        r.turn(now);
        assert!(trace.borrow().exchanges - exchanges <= 1);
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert!(r.life.status().authority_completed_ns.is_some());
    assert!(r.life.status().cleanup_completed_ns.is_some());
    assert_eq!(
        r.life.status().stop_evidence,
        Some(if backend == Backend::DirectXdma {
            saturn_bridge::puresignal::tx_lifecycle::StopEvidence::DirectControlReadback
        } else {
            saturn_bridge::puresignal::tx_lifecycle::StopEvidence::P2CommandBurstOnly
        })
    );
    assert!(trace.borrow().cleanup > before_cleanup);
    if let Some((resume, worker, started)) = paused_producer {
        let elapsed = started.elapsed();
        resume.send(()).ok();
        worker.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(1),
            "release/receipt/native cleanup waited behind paused queue producer: {elapsed:?}"
        );
    }
    if !matches!(case, PumpBoundary::Healthy | PumpBoundary::RetainedHealthy) {
        assert_eq!(
            r.counters.active_max_turn_ns, active_turn_ns,
            "later stop/cleanup turns cannot replace active timing evidence"
        );
    }
    assert!(r.life.offline_owner().queues_empty());
    if backend == Backend::DirectXdma {
        d.assert_off(); // ordinary-file control readback, NOT physical RF-off
    }
    if !matches!(case, PumpBoundary::Healthy | PumpBoundary::RetainedHealthy) {
        assert_eq!(trace.borrow().renders, before_render);
        assert_eq!(r.counters.iq_packets, before_packets);
        assert_eq!(d.dma_writes(), before_dma);
    }

    if case == PumpBoundary::RetainedStale {
        assert_eq!(r.counters.last_stale.unwrap().fields[0], 3);
    } else if case == PumpBoundary::RetainedRelease {
        assert!(r.counters.last_stale.is_none());
        // Cleanup itself never re-arms. Only this fresh authorized action
        // can grant a new generation; no fault acknowledgment is necessary.
        let stopped_generation = r.snapshot();
        send_arm(&mut r, &commands, &ingress);
        for _ in 0..100 {
            wait();
            now += 1_000_000;
            if r.life.status().phase == Phase::AwaitingKey {
                send_audio(&mut r, &audio, now);
            }
            r.turn(now);
            if r.life.status().phase == Phase::Active {
                break;
            }
        }
        assert_eq!(r.life.status().phase, Phase::Active);
        assert_ne!(r.snapshot(), stopped_generation);
        assert_eq!(r.life.status().primary_fault, None);
        r.cancel_handle().release();
        for _ in 0..100 {
            wait();
            now += 1_000_000;
            r.turn(now);
            if r.life.status().phase == Phase::ReadyOff {
                break;
            }
        }
        assert_eq!(r.life.status().phase, Phase::ReadyOff);
        assert!(r.life.status().cleanup_completed_ns.is_some());
        assert_eq!(r.life.status().primary_fault, None);
        if backend == Backend::DirectXdma {
            d.assert_off();
        }
    }
}

#[test]
fn scheduled_pump_native_failure_skips_due_render_and_completes_cleanup() {
    exercise_pump_boundary(PumpBoundary::NativeFailure);
}
#[test]
fn scheduled_pump_severe_overdrive_skips_due_render_and_completes_cleanup() {
    exercise_pump_boundary(PumpBoundary::SevereOverdrive);
}
#[test]
fn scheduled_pump_release_after_tick_skips_due_render_and_completes_cleanup() {
    exercise_pump_boundary(PumpBoundary::Release);
}
#[test]
fn scheduled_pump_missing_authority_does_not_unwrap_and_completes_cleanup() {
    exercise_pump_boundary(PumpBoundary::MissingAuthority);
}
#[test]
fn scheduled_pump_healthy_control_renders_and_sends_one_exchange() {
    exercise_pump_boundary(PumpBoundary::Healthy);
}
#[test]
fn scheduled_retained_audio_release_is_not_stale_and_completes_cleanup() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        exercise_pump_boundary_backend(PumpBoundary::RetainedRelease, backend);
    }
}
#[test]
fn scheduled_retained_audio_revoked_authority_remains_a_fault() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        exercise_pump_boundary_backend(PumpBoundary::RetainedMissingAuthority, backend);
    }
}
#[test]
fn scheduled_retained_audio_healthy_control_renders_and_sends_one_exchange() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        exercise_pump_boundary_backend(PumpBoundary::RetainedHealthy, backend);
    }
}

#[test]
fn scheduled_retained_audio_expired_samples_still_fault_and_complete_cleanup() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        exercise_pump_boundary_backend(PumpBoundary::RetainedStale, backend);
    }
}

// A control/media stall can deliver valid PCM in bursts while acquisition
// preparation is pending. The bounded ingress FIFO must not be drained into
// an already-full, smaller local staging buffer before rendering can resume.
fn exercise_pcm_burst_while_preparing(case: &str) {
    use crate::tx_thread::feedback_owner::Link;
    let d = DirectFixture::with_key_clock();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut output = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    output.enable_offline_handoff(&m, 5).unwrap();
    let link = Link::new();
    output.attach_feedback_owner(link.clone(), 20).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        output,
        Backend::DirectXdma,
    );
    let (commands, rx) = mpsc::sync_channel(16);
    let (audio, arx) = mpsc::sync_channel(16);
    let (fb, frx) = mpsc::sync_channel(2);
    let mut r = OwnerLoop::new(
        life,
        rx,
        arx,
        frx,
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let issuer = OperatorIngress::fixture();
    let mut now = 0;
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    send_config(&mut r, &commands, 300, TxAudioSource::Tci, 11);
    for _ in 0..20 {
        now += 1_000_000;
        r.turn(now);
    }
    send_arm(&mut r, &commands, &issuer);
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.life.status().phase, Phase::AwaitingKey);
    assert!(link.preparing());
    for _ in 0..2 {
        now += 1_000_000;
        for _ in 0..4 {
            send_audio(&mut r, &audio, now);
        }
        r.turn(now);
    }
    assert_eq!(
        r.life.status().primary_fault,
        None,
        "valid burst must remain bounded, not overflow the smaller staging ring"
    );
    assert_eq!(r.life.status().phase, Phase::AwaitingKey);
    d.assert_off(); // no key before acquisition completion
    let before = d.dma_writes();
    if case == "release" {
        r.cancel_handle().release();
    } else {
        if case == "stale" {
            now += 160_000_000;
        }
        assert!(link.complete(link.desired()));
    }
    let mut first = 0;
    for _ in 0..140 {
        now += 1_000_000;
        if r.life.status().phase == Phase::Active && now % 5_000_000 == 0 {
            feedback(&mut r, &fb, &mut first, now, Backend::DirectXdma);
        }
        r.turn(now);
        let request = link.desired();
        if request.scope.is_none() && link.may_apply(request) {
            link.complete(request);
        }
        if case == "healthy" && trace.borrow().nonzero_renders >= 8 {
            break;
        }
        if case != "healthy" && r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    if case == "healthy" {
        assert_eq!(r.life.status().phase, Phase::Active);
        assert_eq!(r.life.status().primary_fault, None);
        assert_eq!(
            trace.borrow().nonzero_renders,
            8,
            "every queued source block must render, without dropping or retiming it"
        );
        assert!(r.counters.iq_packets > 0);
        assert!(d.dma_writes() > before);
        r.cancel_handle().release();
    } else {
        assert_eq!(
            r.life.status().primary_fault,
            if case == "stale" {
                Some(Fault::Stale)
            } else {
                None
            }
        );
        assert_eq!(
            d.dma_writes(),
            before,
            "deferred stale/released audio cannot produce RF output"
        );
    }
    for _ in 0..40 {
        now += 1_000_000;
        r.turn(now);
        let request = link.desired();
        if request.scope.is_none() && link.may_apply(request) {
            link.complete(request);
        }
        if r.life.status().phase == Phase::ReadyOff {
            break;
        }
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert!(r.life.status().cleanup_completed_ns.is_some());
    assert!(r.life.offline_owner().queues_empty());
    d.assert_off();
}
#[test]
fn scheduled_pcm_burst_keeps_bounded_ingress_until_rendering_can_resume() {
    exercise_pcm_burst_while_preparing("healthy");
}
#[test]
fn scheduled_pcm_burst_release_precedes_deferred_audio() {
    exercise_pcm_burst_while_preparing("release");
}
#[test]
fn scheduled_pcm_burst_stale_samples_cannot_be_refreshed_or_key() {
    exercise_pcm_burst_while_preparing("stale");
}
impl NativePs for Fake {
    fn measurement_only_guard(&mut self) -> Result<(), Fault> {
        if self.trace.borrow().ps_enabled {
            Err(Fault::NativeFailure)
        } else {
            Ok(())
        }
    }
    fn configure(&mut self, config: Config) -> Result<f64, Fault> {
        self.trace.borrow_mut().ps_enabled = config.enabled || config.automatic;
        Ok(0.)
    }
    fn mox(&mut self, keyed: bool) {
        self.trace.borrow_mut().keyed = keyed;
    }
    fn reset(&mut self) -> Result<(), Fault> {
        panic!("blocking reset not allowed")
    }
    fn process(&mut self, _: &mut [f64; 2048], _: &mut [f64; 2048]) -> Result<(), Fault> {
        let mut trace = self.trace.borrow_mut();
        trace.fits += 1;
        trace.process_fault.map_or(Ok(()), Err)
    }
    fn information(&mut self) -> Result<[i32; 16], Fault> {
        if let Some(cancel) = self.trace.borrow_mut().cancel_in_information.take() {
            cancel.release();
        }
        let mut i = [0; 16];
        let t = self.trace.borrow();
        i[5] = t.fits as i32;
        i[14] = i32::from(t.keyed && t.fits != 0);
        i[6] = if t.keyed { t.info_fault_bits } else { 0 };
        Ok(i)
    }
}
impl CooperativeNative for Fake {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
        self.trace.borrow_mut().exchanges += 1;
        Ok(())
    }
    fn begin_cleanup(&mut self) -> Result<(), Fault> {
        self.mox(false);
        Ok(())
    }
    fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault> {
        let mut t = self.trace.borrow_mut();
        t.cleanup += 1;
        t.exchanges += 1;
        Ok(CleanupProgress::Complete)
    }
}
impl ScheduledNative for Fake {
    fn measurement_mic_gain(&mut self, _: bool) -> Result<(), Fault> {
        Ok(())
    }
    fn configure_model(&mut self, m: &RadioModel) -> Result<(), Fault> {
        if let Some(fault) = self.trace.borrow_mut().model_fault.take() {
            return Err(fault);
        }
        if let Some(cancel) = self.trace.borrow_mut().cancel_in_configure.take() {
            cancel.release();
        }
        self.trace
            .borrow_mut()
            .models
            .push(m.desired.tx_filter_low_hz);
        Ok(())
    }
    fn render(&mut self, mic: &[f32]) -> Result<Vec<f32>, Fault> {
        assert_eq!(mic.len(), 512);
        let mut t = self.trace.borrow_mut();
        t.exchanges += 1;
        t.renders += 1;
        if mic.iter().any(|v| *v != 0.) {
            t.nonzero_renders += 1;
        }
        let cancel = t.cancel_in_render.take();
        drop(t);
        if let Some(cancel) = cancel {
            cancel.release();
        }
        Ok((0..4096).map(|i| mic[(i / 8) % 512] * 0.2).collect())
    }
}

#[test]
fn scheduled_owner_source_model_transaction_cancellation_and_release_inside_render() {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let (commands, rx) = mpsc::sync_channel(16);
    let (audio, arx) = mpsc::sync_channel(16);
    let (_fb, frx) = mpsc::sync_channel(2);
    let mut r = OwnerLoop::new(
        life,
        rx,
        arx,
        frx,
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 1_000_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let ingress = OperatorIngress::fixture();
    let mut stage = 0;
    let mut next_audio = 0;
    r.run(&AtomicBool::new(false), |r, now| {
        assert!(now < 3_000_000_000, "stage={stage} {:?}", r.life.status());
        match stage {
            0 if r.life.status().phase == Phase::ReadyOff => {
                send_config(r, &commands, 300, TxAudioSource::Satp, 11);
                stage = 1;
            }
            1 if r.counters.configurations == 1 => {
                send_arm(r, &commands, &ingress);
                stage = 2;
            }
            2 if r.counters.rejected > 0 => {
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
                send_config(r, &commands, 400, TxAudioSource::Tci, 12);
                stage = 3;
            }
            3 if r.counters.configurations == 2 => {
                send_arm(r, &commands, &ingress);
                trace.borrow_mut().cancel_in_render = Some(r.cancel_handle());
                stage = 4;
            }
            4 if r.life.status().phase == Phase::ReadyOff => {
                // Release during render prevents enqueue/key, without any
                // blocking native cleanup call on the release path.
                assert_eq!(r.counters.iq_packets, 0);
                assert!(r.life.offline_owner().queues_empty());
                send_arm(r, &commands, &ingress);
                stage = 5;
            }
            5 if r.life.status().phase == Phase::Active => {
                send_config(r, &commands, 900, TxAudioSource::Tci, 99);
                stage = 6;
            }
            6 if r.life.status().phase == Phase::CleaningNative => {
                r.cancel_handle().release();
                stage = 7;
            }
            7 if r.life.status().phase == Phase::ReadyOff => {
                assert_eq!(trace.borrow().models, vec![300, 400]);
                assert_eq!(r.counters.configurations, 2);
                assert_eq!(
                    r.life.fence().config,
                    12,
                    "canceled PS configuration was not installed either"
                );
                send_arm(r, &commands, &ingress);
                stage = 8; // canceled configuration cannot certify readiness
            }
            8 if r.counters.rejected >= 2 => {
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
                send_config(r, &commands, 500, TxAudioSource::Tci, 13);
                stage = 9;
            }
            9 if r.counters.configurations == 3 => {
                send_arm(r, &commands, &ingress);
                stage = 10;
            }
            10 if r.life.status().phase == Phase::Active => {
                commands.try_send(TxCommand::Shutdown).unwrap();
                stage = 11;
            }
            _ => {}
        }
        if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
            && matches!(stage, 4 | 5 | 10)
            && now >= next_audio
        {
            send_audio(r, &audio, now);
            next_audio = now + 11_000_000;
        }
    });
    assert_eq!(stage, 11);
    assert_eq!(trace.borrow().models, vec![300, 400, 500]);
    assert!(r.life.offline_owner().queues_empty());
}
#[test]
fn release_at_fence_boundary_cleans_without_false_fault_then_fresh_rearm() {
    cancellation_boundary_case("release");
}
#[test]
fn generation_cancel_at_boundary_cleans_and_old_cancel_cannot_cross_rearm() {
    cancellation_boundary_case("generation");
}
#[test]
fn genuine_identity_revocation_remains_latched_after_later_release() {
    cancellation_boundary_case("identity");
}
#[test]
fn unexplained_output_revocation_remains_latched_after_later_release() {
    cancellation_boundary_case("raw-close");
}
fn cancellation_boundary_case(case: &str) {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let life = controller(
        Fake {
            trace: Rc::new(RefCell::new(Trace::default())),
        },
        owner,
        Backend::DirectXdma,
    );
    let (commands, rx) = mpsc::sync_channel(16);
    let (audio, arx) = mpsc::sync_channel(16);
    let (_feedback, frx) = mpsc::sync_channel(2);
    let mut r = OwnerLoop::new(
        life,
        rx,
        arx,
        frx,
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 1_000_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let issuer = OperatorIngress::fixture();
    let mut now = 0;
    for _ in 0..24 {
        now += 1_000_000;
        r.turn(now);
    }
    send_config(&mut r, &commands, 300, TxAudioSource::Tci, 11);
    for _ in 0..24 {
        now += 1_000_000;
        r.turn(now);
    }
    send_arm(&mut r, &commands, &issuer);
    for _ in 0..500 {
        now += 1_000_000;
        if now % 11_000_000 == 0 {
            send_audio(&mut r, &audio, now);
        }
        r.turn(now);
        if r.life.status().phase == Phase::Active {
            break;
        }
    }
    assert_eq!(r.life.status().phase, Phase::Active);
    let before = r.counters.iq_packets;
    let cancel = r.cancel_handle();
    let old_stamp = r.snapshot().output().unwrap();
    if case == "identity" || case == "raw-close" {
        if case == "identity" {
            issuer.disconnect();
        } else {
            r.life.offline_owner().cancellation_fence().close();
        }
        now += 1_000_000;
        r.turn(now);
        assert_eq!(r.life.status().primary_fault, Some(Fault::TxAuthority));
        cancel.release();
        for _ in 0..24 {
            now += 1_000_000;
            r.turn(now);
        }
        assert_eq!(r.life.status().phase, Phase::ReadyOff);
        assert_eq!(r.life.status().primary_fault, Some(Fault::TxAuthority));
        assert_eq!(r.counters.iq_packets, before);
        assert!(r.life.offline_owner().queues_empty());
        d.assert_off();
        send_config(&mut r, &commands, 400, TxAudioSource::Tci, 12);
        for _ in 0..24 {
            now += 1_000_000;
            r.turn(now);
        }
        assert_eq!(
            r.counters.configurations, 1,
            "fault latch blocks new settings"
        );
        return;
    }
    let boundary = || {
        assert_eq!(
            r.snapshot().output(),
            None,
            "cancelled permit cannot mint a fresh media offer before release service"
        );
        now += 1_000_000;
        r.turn(now);
        assert_eq!(r.life.status().primary_fault, None);
        assert_ne!(r.life.status().phase, Phase::Active);
    };
    if case == "release" {
        cancel.offline_release_boundary(boundary);
    } else {
        assert!(cancel.offline_generation_release_boundary(old_stamp, boundary));
    }
    for _ in 0..24 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert_eq!(r.life.status().primary_fault, None);
    assert_eq!(r.counters.iq_packets, before);
    assert!(r.life.offline_owner().queues_empty());
    d.assert_off();
    // Settings are still an explicit transaction after completed cleanup.
    send_config(&mut r, &commands, 400, TxAudioSource::Tci, 12);
    for _ in 0..24 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.counters.configurations, 2);
    send_arm(&mut r, &commands, &issuer);
    for _ in 0..500 {
        now += 1_000_000;
        if now % 11_000_000 == 0 {
            send_audio(&mut r, &audio, now);
        }
        r.turn(now);
        if r.life.status().phase == Phase::Active {
            break;
        }
    }
    assert_eq!(r.life.status().phase, Phase::Active);
    assert_eq!(r.life.status().primary_fault, None);
    let revision = r.life.offline_owner().current_revision();
    let ticket = r.life.release_handle().current();
    assert!(
        !cancel.offline_generation_release_boundary(old_stamp, || panic!(
            "stale generation committed cancellation"
        ))
    );
    assert_eq!(r.life.offline_owner().current_revision(), revision);
    assert_eq!(r.life.release_handle().current(), ticket);
    now += 1_000_000;
    r.turn(now);
    assert_eq!(r.life.status().phase, Phase::Active);
    assert_eq!(r.life.status().primary_fault, None);
    r.cancel_handle().release();
    for _ in 0..24 {
        now += 1_000_000;
        r.turn(now);
    }
    assert_eq!(r.life.status().phase, Phase::ReadyOff);
    assert_eq!(r.life.status().primary_fault, None);
    d.assert_off();
}
fn cfg(id: u128) -> Config {
    Config {
        id,
        route: 20,
        physical_adc: OverloadSource::Adc1,
        enabled: true,
        automatic: true,
        hardware_peak: 0.6121,
        requested_delay_seconds: 0.,
        settling_ns: 0,
        loop_delay_seconds: 0.,
        stale_after_ns: 250_000_000,
    }
}

#[test]
fn generated_single_trial_actual_owner_deadline_release_and_stale_authority_both_backends() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in [
            "expiry",
            "release",
            "cancel-queued",
            "stale-request",
            "missing",
            "config",
        ] {
            generated_single_trial(backend, case, false, |_| Fake {
                trace: Rc::new(RefCell::new(Trace::default())),
            });
        }
    }
}

#[test]
fn operator_two_tone_actual_owner_runs_past_elapsed_cutoff_and_cleans_both_backends() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in ["operator-release", "operator-disconnect"] {
            generated_single_trial(backend, case, false, |_| Fake {
                trace: Rc::new(RefCell::new(Trace::default())),
            });
        }
    }
}

#[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
#[test]
#[ignore = "serial actual owner and matched WDSP, files/loopback only; no RF"]
fn operator_two_tone_real_native_manual_stop_and_cleanup_both_backends() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    for backend in [Backend::P2, Backend::DirectXdma] {
        generated_single_trial(backend, "operator-release", true, |m| {
            crate::wdsp::ps_native::FullNative::matched(m).unwrap()
        });
    }
}

#[test]
fn feedback_burst_actual_owner_keeps_freshness_release_and_overflow_both_backends() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in [
            "feedback-burst",
            "feedback-release",
            "feedback-overflow",
            "feedback-stale",
        ] {
            generated_single_trial(backend, case, false, |_| Fake {
                trace: Rc::new(RefCell::new(Trace::default())),
            });
        }
    }
}

#[cfg(all(
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
#[test]
#[ignore = "serial real WDSP and actual file/loopback owners; no RF"]
fn feedback_burst_real_native_owner_keeps_stop_cleanup_both_backends() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in [
            "feedback-burst",
            "feedback-release",
            "feedback-overflow",
            "feedback-stale",
        ] {
            generated_single_trial(backend, case, true, |m| {
                crate::wdsp::ps_native::FullNative::matched(m).unwrap()
            });
        }
    }
}

#[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
#[test]
#[ignore = "serial matched WDSP; ordinary files/loopback only, no RF"]
fn generated_single_trial_real_native_output_and_cleanup_both_backends() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    for backend in [Backend::P2, Backend::DirectXdma] {
        for native_source in [false, true] {
            for case in [
                "expiry",
                "release",
                "missing",
                "config",
                "cancel-queued",
                "stale-request",
                "disconnect",
                "missing-source",
            ] {
                generated_single_trial(backend, case, native_source, |m| {
                    crate::wdsp::ps_native::FullNative::matched(m).unwrap()
                });
            }
        }
    }
}

fn generated_single_trial<N: ScheduledNative>(
    backend: Backend,
    case: &str,
    native_source: bool,
    native: impl FnOnce(&RadioModel) -> N,
) {
    use crate::tx_thread::ps_ingress::TwoToneLimits;
    let operating = case.starts_with("operator-");
    let p2 = (backend == Backend::P2).then(super::tests::p2_separate_lanes);
    if let Some((_, hp, iq)) = &p2 {
        hp.set_nonblocking(true).unwrap();
        iq.set_nonblocking(true).unwrap();
    }
    let drain = || {
        if let Some((_, hp, iq)) = &p2 {
            let mut bytes = [0; 2048];
            while hp.recv(&mut bytes).is_ok() {}
            while iq.recv(&mut bytes).is_ok() {}
        }
    };
    let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
    if let Some(d) = &direct {
        d.fifo_ready();
    }
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = if operating { 60 } else { 5 };
    m.desired.tx_frequency_hz = if operating { 3_897_000 } else { 7_200_000 };
    let mut output = if let Some((session, _, _)) = &p2 {
        OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap()
    } else {
        OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic).unwrap()
    };
    output
        .enable_offline_handoff(&m, if operating { 100 } else { 5 })
        .unwrap();
    let life = controller(native(&m), output, backend);
    let origin = Instant::now();
    let issuer = OperatorIngress::fixture();
    let (clients, operator_id) = issuer.fixture_registry();
    let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        issuer,
        Arc::new(Mutex::new(m.clone())),
        origin,
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: if operating { 0 } else { 2_000_000_000 },
        },
    )
    .unwrap();
    if operating {
        ingress.set_drive_ceiling(100).unwrap();
    }
    ingress
        .enable_two_tone_trials(TwoToneLimits {
            frequency_hz: if operating { 0 } else { 7_200_000 },
            max_ms: if operating { 0 } else { 45000 },
        })
        .unwrap();
    if native_source {
        ingress.restrict_to_generated_trials().unwrap();
    }
    let initial_deadline = Instant::now() + Duration::from_secs(3);
    while r.life.status().phase != Phase::ReadyOff {
        assert!(
            Instant::now() < initial_deadline,
            "initial cleanup {:?}",
            r.life.status()
        );
        drain();
        r.turn(origin.elapsed().as_nanos() as u64);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut config = cfg(11);
    config.automatic = false;
    if operating {
        config.enabled = false;
    } // baseline runs without adaptation/feedback
      // This isolated native fixture must allow actual DSP startup before its
      // finite action expires; it is not a measured hardware key-latency bound.
    config.stale_after_ns = if case == "feedback-stale" {
        150_000_000
    } else if case == "missing" {
        250_000_000
    } else {
        1_000_000_000
    };
    ingress
        .bootstrap(m.clone(), config, TxAudioSource::Tci)
        .unwrap();
    let mut stage = 0;
    let mut requested = 0;
    let mut next_audio = 0;
    let mut active = false;
    let mut arm_seen = false;
    let mut active_at = None;
    let mut injected = false;
    let mut old_capture = None;
    let mut next_sample = 0u64;
    let mut stopped_packets = 0;
    let mut observed_peak = 0.0_f32;
    let mut packets_at_burst = 0;
    r.run(&AtomicBool::new(false), |r, now| {
        if now >= 5_000_000_000 {
            let mut replies = Vec::new();
            while let Some(item) = outbound.next_message(false) {
                if let crate::tci::OutboundMessage::TransactionText(text) = item.message {
                    replies.push(text);
                }
            }
            panic!(
                "trial timeout {backend:?}/{case} {:?}; rejected={} replies={replies:?}",
                r.life.status(),
                r.counters.rejected
            );
        }
        drain();
        if stage == 0 && ingress.startup_ready() && (case != "stale-request" || now > 160_000_000) {
            let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
            old_capture = Some(capture.clone());
            requested = now;
            let duration = if operating {
                0
            } else if matches!(case, "missing" | "missing-source") {
                1000
            } else {
                600
            };
            if case == "stale-request" {
                // Producer age is retained even when its queue is serviced later.
                let old = ingress.capture(origin).unwrap();
                ingress.two_tone_message(
                    7,
                    &format!(
                        "saturn_ps_two_tone:0,1,start,{duration}{};",
                        if native_source {
                            ",wdsp-postgen-two-tone-v1"
                        } else {
                            ""
                        }
                    ),
                    Some(&old),
                );
            } else {
                ingress.two_tone_message(
                    7,
                    &format!(
                        "saturn_ps_two_tone:0,1,start,{duration}{};",
                        if native_source {
                            ",wdsp-postgen-two-tone-v1"
                        } else {
                            ""
                        }
                    ),
                    Some(&capture),
                );
                if case == "cancel-queued" {
                    ingress.release();
                }
            }
            stage = 1;
        }
        let phase = r.life.status().phase;
        arm_seen |= phase == Phase::AwaitingKey;
        active |= phase == Phase::Active;
        observed_peak = observed_peak.max(r.counters.native_iq_component_peak);
        if phase == Phase::Active {
            active_at.get_or_insert(now);
        }
        if stage == 1
            && !matches!(case, "cancel-queued" | "stale-request" | "missing-source")
            && matches!(phase, Phase::AwaitingKey | Phase::Active)
            && now >= next_audio
        {
            let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
            old_capture = Some(capture.clone());
            let samples = (0..512)
                .map(|i| {
                    if native_source {
                        return 0.;
                    }
                    let n = (next_sample + i) as f64;
                    (0.2 * ((std::f64::consts::TAU * 700. * n / 48000.).sin()
                        + (std::f64::consts::TAU * 1900. * n / 48000.).sin()))
                        as f32
                })
                .collect();
            next_sample += 512;
            ingress
                .enqueue_audio(
                    &capture,
                    TxAudioFrame {
                        enqueued_at: origin + Duration::from_nanos(now),
                        source: TxAudioSource::Tci,
                        samples: TxAudioSamples::Dynamic(samples),
                        channels: 1,
                        sample_rate_hz: 48000,
                    },
                )
                .unwrap();
            next_audio = now + 10_666_667;
        }
        if stage == 1
            && active_at.is_some_and(|t| {
                now.saturating_sub(t) > if operating { 2_100_000_000 } else { 40_000_000 }
            })
            && !injected
            && matches!(
                case,
                "release" | "config" | "disconnect" | "operator-release" | "operator-disconnect"
            )
        {
            if matches!(case, "release" | "operator-release") {
                ingress.release();
            } else if matches!(case, "disconnect" | "operator-disconnect") {
                crate::tci::unregister_client(&clients, &operator_id, 7);
                ingress.release();
            } else {
                let c = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                ingress
                    .configure_batch(7, Some(&c), &[crate::tci::TciCommand::SetTxMicGain(2.)])
                    .unwrap();
            }
            injected = true;
        }
        if stage == 1
            && case.starts_with("feedback-")
            && !injected
            && active_at.is_some_and(|t| now.saturating_sub(t) > 40_000_000)
        {
            use crate::tx_thread::ps_ingress::FEEDBACK_QUEUE_BLOCKS;
            use crate::tx_thread::{
                feedback_owner::{Scope, Stimulus},
                feedback_stream::{
                    tests::{direct_frames, packet},
                    Stream,
                },
            };
            packets_at_burst = r.counters.iq_packets;
            let count = if case == "feedback-overflow" {
                (FEEDBACK_QUEUE_BLOCKS + 1) * if backend == Backend::P2 { 8 } else { 1 }
            } else if backend == Backend::P2 {
                16
            } else {
                6
            };
            let scope = Scope {
                fence: r.life.fence(),
                route: 20,
                frequency_hz: 7_200_000,
                attenuation_db: 0,
                output: r.life.offline_owner().output_stamp().unwrap(),
                stimulus: if native_source {
                    Stimulus::NativeTwoTone
                } else {
                    Stimulus::TwoTone
                },
            };
            let mut stream = Stream::new(scope, backend, origin, ingress.clone()).unwrap();
            let mut ddc6 = Vec::new();
            for n in 0..count {
                // Actual production demux/assembly -> pooled ingress -> actual
                // scheduled owner/native -> file/loopback output. This tests
                // one combined path rather than separate handoff components.
                let observed = origin + Duration::from_nanos(now);
                let result = if backend == Backend::DirectXdma {
                    stream.direct(Some(scope), &direct_frames(256), observed, &mut ddc6)
                } else {
                    stream.p2(Some(scope), &packet(n as u32), observed)
                };
                assert_eq!(
                    result.is_err(),
                    case == "feedback-overflow" && n + 1 == count
                );
            }
            if case == "feedback-release" {
                ingress.release();
            }
            if case == "feedback-stale" {
                // Hold the actual queued samples beyond the unchanged 150 ms
                // deadline. Their original observation must not be refreshed.
                std::thread::sleep(Duration::from_millis(200));
            }
            injected = true;
        }
        let canceled = matches!(case, "cancel-queued" | "stale-request") && r.counters.rejected > 0;
        if stage == 1
            && (active || canceled || (arm_seen && case == "missing-source"))
            && phase == Phase::ReadyOff
            && now > requested
        {
            assert!(r.life.status().cleanup_completed_ns.is_some());
            assert_eq!(
                r.life.status().primary_fault,
                if matches!(case, "missing" | "missing-source" | "feedback-stale") {
                    Some(Fault::Stale)
                } else if case == "feedback-overflow" {
                    Some(Fault::QueueOverrun)
                } else {
                    None
                }
            );
            if case == "feedback-burst" {
                assert!(injected);
                assert_eq!(
                    r.life.feedback_status().accepted_pairs,
                    if backend == Backend::P2 {
                        16 * 119
                    } else {
                        6 * 1024
                    }
                );
            }
            if matches!(
                case,
                "feedback-release" | "feedback-stale" | "feedback-overflow"
            ) {
                assert!(injected);
                assert_eq!(
                    r.counters.iq_packets, packets_at_burst,
                    "no output after release/expired burst/overflow"
                );
            }
            if case == "missing-source" {
                assert!(
                    !active,
                    "native generator cannot replace fresh source ingress"
                );
                assert_eq!(r.counters.iq_packets, 0);
                assert_eq!(r.counters.last_stale.unwrap().fields[0], 2);
            }
            if active {
                assert!(r.counters.iq_packets > 0);
                assert!(observed_peak > 0.01);
            }
            stopped_packets = r.counters.iq_packets;
            // An old generation's PCM is not rebound after cleanup.
            let old = old_capture.as_ref().unwrap();
            let retained = ingress.enqueue_audio(
                old,
                TxAudioFrame {
                    enqueued_at: origin + Duration::from_nanos(now),
                    source: TxAudioSource::Tci,
                    samples: TxAudioSamples::Dynamic(vec![0.1; 512]),
                    channels: 1,
                    sample_rate_hz: 48000,
                },
            );
            if !active {
                assert!(retained.is_err());
            }
            ingress.shutdown().unwrap();
            stage = 2;
        }
    });
    assert_eq!(stage, 2);
    if operating {
        assert!(active && injected && r.counters.iq_packets > 0);
        assert!(
            observed_peak > 0.,
            "manual-stop action must render real output"
        );
        assert_eq!(r.counters.deadline_stops, 0);
    }
    assert!(r.life.offline_owner().queues_empty());
    assert_eq!(
        r.counters.iq_packets, stopped_packets,
        "retained PCM wrote output after stop"
    );
    let mut replies = String::new();
    while let Some(item) = outbound.next_message(false) {
        if let crate::tci::OutboundMessage::TransactionText(s) = item.message {
            replies.push_str(&s);
        }
    }
    assert!(
        replies.contains("saturn_ps_two_tone:0,1,queued;"),
        "{replies}"
    );
    if active || (arm_seen && case == "missing-source") {
        assert!(replies.contains("saturn_ps_two_tone:0,1,accepted;"));
        if !matches!(case, "disconnect" | "operator-disconnect") {
            assert!(replies.contains("saturn_ps_two_tone:0,1,complete,"));
        }
    } else {
        assert!(replies.contains("saturn_ps_two_tone:0,1,rejected,"));
    }
    if case == "stale-request" {
        assert!(replies.contains("rejected,Stale;"));
    }
    if let Some(d) = &direct {
        d.assert_off();
    }
}
fn controller<N: ScheduledNative>(
    n: N,
    owner: OwnerStopAdapter<'_>,
    backend: Backend,
) -> TxLifecycle<N, OwnerStopAdapter<'_>> {
    let c = Controller::new_offline_admission_proposal(
        n,
        Owner::Bridge,
        backend,
        88,
        Qualification {
            native_abi_verified: true,
            feedback_topology_verified: true,
        },
        4,
        DraftLimits {
            active_reference_fraction: 0.1,
            feedback_rms_floor: 1e-6,
            component_limit: 8388607. / 8388608.,
        },
    )
    .unwrap();
    let mut life = TxLifecycle::new(
        c,
        owner,
        backend,
        Limits {
            authority_timeout_ns: 2_000_000_000,
            cleanup_timeout_ns: 3_000_000_000,
        },
    )
    .unwrap();
    life.reconfigure(life.fence(), cfg(10), 0).unwrap();
    life
}

#[test]
fn faulted_rx_tuning_actual_dispatch_both_backends_preserves_fault_and_output_fence() {
    use crate::tci::TciCommand;
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in [
            "tune",
            "release",
            "retune-release",
            "disconnect",
            "mixed-tx",
        ] {
            let p2 = (backend == Backend::P2).then(super::tests::p2_separate_lanes);
            let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &direct {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let mut output = if let Some((session, _, _)) = &p2 {
                OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            output.enable_offline_handoff(&m, 5).unwrap();
            let trace = Rc::new(RefCell::new(Trace::default()));
            let life = controller(
                Fake {
                    trace: trace.clone(),
                },
                output,
                backend,
            );
            let issuer = OperatorIngress::fixture();
            let (clients, operator) = issuer.fixture_registry();
            let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
            let shared_model = Arc::new(Mutex::new(m.clone()));
            let origin = Instant::now();
            let (mut r, ingress) = OwnerLoop::with_shared_ingress(
                life,
                issuer,
                shared_model.clone(),
                origin,
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 2_000_000_000,
                },
            )
            .unwrap();
            let mut now = 0;
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            ingress
                .bootstrap(m.clone(), cfg(11), TxAudioSource::Tci)
                .unwrap();
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.counters.configurations == 1 && r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            ingress.stop(Fault::SevereOverdrive);
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            assert_eq!(r.life.status().primary_fault, Some(Fault::SevereOverdrive));
            let stop = r.life.status().stop_id;
            let before = r.snapshot();
            let old_capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
            let native_before = (
                trace.borrow().exchanges,
                trace.borrow().models.len(),
                trace.borrow().cleanup,
            );
            let (legacy, legacy_rx) = mpsc::channel();
            let control = Arc::new(Mutex::new(None));
            let dispatch = |text: &str| {
                crate::tci::handle_incoming_message_shared(
                    tungstenite::Message::Text(text.into()),
                    &legacy,
                    &clients,
                    &operator,
                    &control,
                    7,
                    Some(&ingress),
                )
            };
            if case == "mixed-tx" {
                dispatch("vfo:0,0,7100000;dds:0,7100000;tx_drive:0,4;");
            } else {
                // Separate wire messages preserve projected state and real receipts.
                dispatch("vfo:0,0,7100000;dds:0,7100000;");
                dispatch("vfo:0,0,7150000;dds:0,7150000;");
            }
            if case == "release" {
                ingress.release();
            }
            if case == "retune-release" {
                trace.borrow_mut().cancel_in_information = Some(r.cancel_handle());
            }
            if case == "disconnect" {
                clients
                    .lock()
                    .unwrap()
                    .get(&7)
                    .unwrap()
                    .state
                    .authorization_live
                    .store(false, Ordering::SeqCst);
                ingress.release();
            }
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(1));
                now += 1_000_000;
                r.turn(now);
            }
            assert_eq!(r.life.status().phase, Phase::ReadyOff);
            assert_eq!(r.life.status().primary_fault, Some(Fault::SevereOverdrive));
            assert_eq!(r.life.feedback_status().fault, Some(Fault::SevereOverdrive));
            assert!(r.life.offline_owner().queues_empty());
            assert!(r.snapshot().output().is_none());
            assert_eq!(r.counters.iq_packets, 0);
            assert_eq!(trace.borrow().renders, 0);
            let mut replies = String::new();
            while let Some(item) = outbound.next_message(false) {
                if let crate::tci::OutboundMessage::TransactionText(text) = item.message {
                    replies.push_str(&text);
                }
            }
            if case == "tune" {
                assert_eq!(shared_model.lock().unwrap().desired.iq_center_hz, 7_150_000);
                assert_eq!(shared_model.lock().unwrap().desired.vfo_a_hz, 7_150_000);
                assert_eq!(r.life.fence().config, 13);
                assert_eq!(r.life.status().stop_id, stop);
                assert_ne!(r.snapshot(), before);
                assert_eq!((trace.borrow().exchanges,trace.borrow().models.len(),trace.borrow().cleanup),native_before,
                    "receive tuning must not invoke native configuration, render, fitting or cleanup");
                assert!(replies.contains("tx_config:0,1,applied,12;"), "{replies}");
                assert!(replies.contains("tx_config:0,2,applied,13;"), "{replies}");
                let attenuation_capture =
                    ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                ingress
                    .configure_batch(
                        7,
                        Some(&attenuation_capture),
                        &[TciCommand::SetPureSignalAttenuation(17)],
                    )
                    .unwrap();
                for _ in 0..10 {
                    now += 1_000_000;
                    r.turn(now);
                }
                assert_eq!(
                    shared_model
                        .lock()
                        .unwrap()
                        .desired
                        .pure_signal_attenuation_db,
                    17
                );
                assert_eq!(r.life.status().primary_fault, Some(Fault::SevereOverdrive));
                assert!(r.snapshot().output().is_none());
                // Even a new operator PTT action cannot clear the retained fault.
                let c = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                ingress
                    .tci(7, Some(&c), &TciCommand::SetTxEnabled(true))
                    .unwrap();
                now += 1_000_000;
                r.turn(now);
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
                assert_eq!(r.life.status().primary_fault, Some(Fault::SevereOverdrive));
                assert!(r.snapshot().output().is_none());
                // Acknowledgment is separate, and the old pre-tuning action
                // remains stale even after it. A genuinely fresh action works.
                let fresh = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                assert!(ingress.fault_ack_message(
                    7,
                    &format!("saturn_tx_fault_ack:0,1,{stop};"),
                    Some(&fresh)
                ));
                for _ in 0..20 {
                    now += 1_000_000;
                    r.turn(now);
                }
                assert_eq!(r.life.status().primary_fault, None);
                assert!(r.snapshot().output().is_none());
                ingress
                    .tci(7, Some(&old_capture), &TciCommand::SetTxEnabled(true))
                    .unwrap();
                now += 1_000_000;
                r.turn(now);
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
                let fresh = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                ingress
                    .tci(7, Some(&fresh), &TciCommand::SetTxEnabled(true))
                    .unwrap();
                now += 1_000_000;
                r.turn(now);
                assert_eq!(r.life.status().phase, Phase::AwaitingKey);
                ingress.release();
                for _ in 0..200 {
                    std::thread::sleep(Duration::from_millis(2));
                    now += 2_000_000;
                    r.turn(now);
                    if r.life.status().phase == Phase::ReadyOff {
                        break;
                    }
                }
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
                assert_eq!(r.life.status().primary_fault, None);
                assert_eq!(shared_model.lock().unwrap().desired.iq_center_hz, 7_150_000);
            } else {
                assert_eq!(
                    shared_model.lock().unwrap().desired.iq_center_hz,
                    m.desired.iq_center_hz
                );
                assert_eq!(shared_model.lock().unwrap().desired.tx_drive, 5);
                assert!(replies.contains(",rejected,"), "{case}: {replies}");
                assert!(!replies.contains(",applied,"), "{case}: {replies}");
                if case == "retune-release" {
                    // The rejected operation advanced identity before release.
                    // A later fresh tune must still work without clearing TX.
                    dispatch("vfo:0,0,7180000;dds:0,7180000;");
                    for _ in 0..20 {
                        now += 1_000_000;
                        r.turn(now);
                    }
                    assert_eq!(shared_model.lock().unwrap().desired.iq_center_hz, 7_180_000);
                    assert_eq!(r.life.status().primary_fault, Some(Fault::SevereOverdrive));
                    assert_eq!(r.life.status().phase, Phase::ReadyOff);
                    assert!(r.snapshot().output().is_none());
                }
            }
            assert!(legacy_rx.try_recv().is_err());
            if let Some(d) = &direct {
                d.assert_off();
            }
        }
    }
}

#[test]
fn explicit_fault_ack_actual_owner_both_backends_never_arms_and_rejects_cancelled_requests() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in ["fresh", "release", "disconnect", "wrong-stop", "stale"] {
            let p2 = (backend == Backend::P2).then(super::tests::p2_separate_lanes);
            let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &direct {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let mut output = if let Some((session, _, _)) = &p2 {
                OwnerStopAdapter::p2(session.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            output.enable_offline_handoff(&m, 5).unwrap();
            let trace = Rc::new(RefCell::new(Trace::default()));
            let life = controller(
                Fake {
                    trace: trace.clone(),
                },
                output,
                backend,
            );
            let issuer = OperatorIngress::fixture();
            let (clients, _) = issuer.fixture_registry();
            let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
            let origin = Instant::now();
            let (mut r, ingress) = OwnerLoop::with_shared_ingress(
                life,
                issuer,
                Arc::new(Mutex::new(m.clone())),
                origin,
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 2_000_000_000,
                },
            )
            .unwrap();
            let mut now = 0;
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            ingress
                .bootstrap(m.clone(), cfg(11), TxAudioSource::Tci)
                .unwrap();
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.counters.configurations == 1 && r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            ingress.stop(Fault::QueueOverrun);
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
                if r.life.status().phase == Phase::ReadyOff {
                    break;
                }
            }
            assert_eq!(r.life.status().phase, Phase::ReadyOff);
            assert_eq!(r.life.status().primary_fault, Some(Fault::QueueOverrun));
            let stop = r.life.status().stop_id;
            let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
            let requested = if case == "wrong-stop" { stop + 1 } else { stop };
            assert!(ingress.fault_ack_message(
                7,
                &format!("saturn_tx_fault_ack:0,1,{requested};"),
                Some(&capture)
            ));
            if case == "release" {
                ingress.release();
            }
            if case == "disconnect" {
                clients
                    .lock()
                    .unwrap()
                    .get(&7)
                    .unwrap()
                    .state
                    .authorization_live
                    .store(false, Ordering::SeqCst);
            }
            if case == "stale" {
                now += 151_000_000;
            }
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(2));
                now += 2_000_000;
                r.turn(now);
            }
            assert_eq!(r.life.status().phase, Phase::ReadyOff);
            assert_eq!(
                r.life.status().primary_fault,
                if case == "fresh" {
                    None
                } else {
                    Some(Fault::QueueOverrun)
                }
            );
            assert_eq!(trace.borrow().nonzero_renders, 0);
            assert_eq!(r.counters.iq_packets, 0);
            assert!(r.life.offline_owner().queues_empty());
            let mut replies = String::new();
            while let Some(item) = outbound.next_message(false) {
                if let crate::tci::OutboundMessage::TransactionText(text) = item.message {
                    replies.push_str(&text);
                }
            }
            assert!(
                replies.contains(if case == "fresh" {
                    "saturn_tx_fault_ack:0,1,applied,none,"
                } else {
                    "saturn_tx_fault_ack:0,1,rejected,"
                }),
                "{backend:?} {case}: {replies}"
            );
            if case == "fresh" {
                let fresh = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                ingress
                    .tci(7, Some(&fresh), &crate::tci::TciCommand::SetTxEnabled(true))
                    .unwrap();
                now += 1_000_000;
                r.turn(now);
                assert_eq!(r.life.status().phase, Phase::AwaitingKey); // a separate authorized action
                ingress.release();
                for _ in 0..200 {
                    std::thread::sleep(Duration::from_millis(2));
                    now += 2_000_000;
                    r.turn(now);
                    if r.life.status().phase == Phase::ReadyOff {
                        break;
                    }
                }
                assert_eq!(r.life.status().phase, Phase::ReadyOff);
            }
            if let Some(d) = &direct {
                d.assert_off();
            }
        }
    }
}
fn send_config<N: ScheduledNative>(
    r: &mut OwnerLoop<'_, N>,
    send: &SyncSender<TxCommand>,
    low: i32,
    source: TxAudioSource,
    id: u128,
) {
    let mut m = model();
    m.desired.tx_drive = 5;
    m.desired.tx_filter_low_hz = low;
    send.try_send(TxCommand::PureSignalOwnerConfig(Box::new(
        Configuration::at_ingress(r.snapshot(), m, cfg(id), source),
    )))
    .unwrap();
}
fn send_arm<N: ScheduledNative>(
    r: &mut OwnerLoop<'_, N>,
    send: &SyncSender<TxCommand>,
    ingress: &OperatorIngress,
) {
    let action = ingress
        .receive(
            7,
            "trx:0,true",
            r.life.status().release_handled,
            r.life.fence(),
            r.life.offline_owner().ready_revision().unwrap(),
        )
        .unwrap();
    send.try_send(TxCommand::PureSignalAuthorizedArm(AuthorizedArm::new(
        action,
    )))
    .unwrap();
}
fn send_audio<N: ScheduledNative>(r: &mut OwnerLoop<'_, N>, send: &SyncSender<Audio>, now: u64) {
    let samples = (0..512)
        .map(|i| (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.2)
        .collect();
    let message = TxAudioMessage::Frames(TxAudioFrame {
        enqueued_at: r_origin(r) + Duration::from_nanos(now),
        source: TxAudioSource::Tci,
        samples: TxAudioSamples::Dynamic(samples),
        channels: 1,
        sample_rate_hz: 48000,
    });
    let _ = send.try_send(Audio::at_ingress(r.snapshot(), message));
}
// Test seam returns the SAME ingress clock epoch, never a fresh dequeue stamp.
fn r_origin<N: ScheduledNative>(r: &OwnerLoop<'_, N>) -> Instant {
    r.ingress_origin()
}
fn feedback<N: ScheduledNative>(
    r: &mut OwnerLoop<'_, N>,
    send: &SyncSender<Box<PairBlock>>,
    first: &mut u64,
    now: u64,
    backend: Backend,
) {
    let mut b = PairBlock::default();
    b.pairs = if backend == Backend::P2 {
        8 * 119
    } else {
        1024
    };
    b.meta = BlockMeta {
        fence: r.life.fence(),
        interval: Interval {
            fence: r.life.fence(),
            first: *first,
            end: *first + b.pairs as u64,
        },
        backend,
        route: 20,
        observed_ns: now,
        sequence: (backend == Backend::P2).then_some((*first / 119) as u32),
        sequence_packets: if backend == Backend::P2 { 8 } else { 0 },
        rate_hz: 192000,
        overload: [None; 3],
    };
    let mut random = 0x53460004u32;
    for i in 0..b.pairs {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        let radius = 0.001 + 0.999 * f64::from(random) / 4294967296.;
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = std::f64::consts::TAU * f64::from(random) / 4294967296.;
        let rf = 0.5 * radius * (1. - 0.25 * radius * radius);
        let phase = angle + 0.18 * radius * radius;
        b.tx[i * 2] = 0.6121 * radius * angle.cos();
        b.tx[i * 2 + 1] = 0.6121 * radius * angle.sin();
        b.rf[i * 2] = rf * phase.cos();
        b.rf[i * 2 + 1] = rf * phase.sin();
    }
    let pairs = b.pairs as u64;
    if send.try_send(Box::new(b)).is_ok() {
        *first += pairs;
    }
}

// Real scheduling loop with fake native traces: deadline behavior is tested
// on BOTH empty and perpetually busy queues, independently of RF telemetry.
#[test]
fn scheduled_owner_watchdogs_empty_busy_and_single_exchange_budget() {
    for busy in [false, true] {
        for duration in [false, true] {
            let d = DirectFixture::new();
            d.fifo_ready();
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
            owner.enable_offline_handoff(&m, 5).unwrap();
            let trace = Rc::new(RefCell::new(Trace::default()));
            let life = controller(
                Fake {
                    trace: trace.clone(),
                },
                owner,
                Backend::DirectXdma,
            );
            let (commands, rx) = mpsc::sync_channel(16);
            let (audio, arx) = mpsc::sync_channel(16);
            let (fb, frx) = mpsc::sync_channel(2);
            let mut r = OwnerLoop::new(
                life,
                rx,
                arx,
                frx,
                Instant::now(),
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 40_000_000,
                    tx_duration_ns: if duration { 100_000_000 } else { 0 },
                },
            )
            .unwrap();
            let ingress = OperatorIngress::fixture();
            let mut stage = 0;
            let mut next_audio = 0;
            let mut first = 0;
            let mut last_exchanges = 0;
            let mut active_at = 0;
            let mut fault_seen = false;
            r.run(&AtomicBool::new(false), |r, now| {
                let exchanges = trace.borrow().exchanges;
                assert!(
                    exchanges - last_exchanges <= 1,
                    "second exchange in a scheduling turn"
                );
                last_exchanges = exchanges;
                assert!(
                    now < 3_000_000_000,
                    "scheduler failed {:?}",
                    r.life.status()
                );
                match stage {
                    0 if r.life.status().phase == Phase::ReadyOff => {
                        send_config(r, &commands, 300, TxAudioSource::Tci, 11);
                        stage = 1;
                    }
                    1 if r.counters.configurations == 1 => {
                        send_arm(r, &commands, &ingress);
                        stage = 2;
                    }
                    2 if r.life.status().phase == Phase::Active => {
                        active_at = now;
                        stage = 3;
                    }
                    3 => {
                        if busy {
                            let _ = commands.try_send(TxCommand::Arm { rf_enabled: true });
                        }
                        if r.life.status().phase == Phase::Active && (busy || duration) {
                            feedback(r, &fb, &mut first, now, Backend::DirectXdma);
                        }
                        if r.life.status().primary_fault == Some(Fault::Stale) {
                            fault_seen = true;
                            assert!(now - active_at < 180_000_000);
                            stage = 4;
                        }
                    }
                    4 if r.life.status().phase == Phase::ReadyOff => {
                        assert!(r.life.status().primary_fault.is_some());
                        commands.try_send(TxCommand::Shutdown).unwrap();
                        stage = 5;
                    }
                    _ => {}
                }
                if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
                    && (stage <= 2 || duration)
                    && now >= next_audio
                {
                    send_audio(r, &audio, now);
                    next_audio = now + 11_000_000;
                }
            });
            assert!(fault_seen);
            assert_eq!(r.counters.deadline_stops, 1);
            let snapshot = r.counters.last_stale.expect("watchdog cause retained");
            assert_eq!(snapshot.fields[0], if duration { 1 } else { 2 });
            assert!(snapshot.fields[1] >= snapshot.fields[2]);
            assert_eq!(snapshot.fields[6], 40_000_000);
            assert!(r.counters.mic_blocks > 0);
            assert!(trace.borrow().cleanup >= 2);
            assert_eq!(trace.borrow().models, vec![300]);
            eprintln!(
                "scheduled watchdog busy={busy} total_duration={duration}: {:?}",
                r.counters
            );
        }
    }
}

#[test]
fn scheduled_owner_feedback_watchdog_without_another_packet_empty_and_busy() {
    for busy in [false, true] {
        let d = DirectFixture::new();
        d.fifo_ready();
        let (mut iq, mut mic) = queues();
        let mut m = model();
        m.desired.tx_drive = 5;
        let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
        owner.enable_offline_handoff(&m, 5).unwrap();
        let trace = Rc::new(RefCell::new(Trace::default()));
        let life = controller(Fake { trace }, owner, Backend::DirectXdma);
        let (commands, rx) = mpsc::sync_channel(16);
        let (audio, arx) = mpsc::sync_channel(16);
        let (fb, frx) = mpsc::sync_channel(2);
        let mut r = OwnerLoop::new(
            life,
            rx,
            arx,
            frx,
            Instant::now(),
            Timing {
                mic_age_ns: 150_000_000,
                source_stall_ns: 1_000_000_000,
                tx_duration_ns: 2_000_000_000,
            },
        )
        .unwrap();
        let ingress = OperatorIngress::fixture();
        let mut stage = 0;
        let mut next_audio = 0;
        let mut first = 0;
        let mut stopped_feedback = 0;
        r.run(&AtomicBool::new(false), |r, now| {
            assert!(now < 3_000_000_000, "stage={stage} {:?}", r.life.status());
            match stage {
                0 if r.life.status().phase == Phase::ReadyOff => {
                    send_config(r, &commands, 300, TxAudioSource::Tci, 11);
                    stage = 1;
                }
                1 if r.counters.configurations == 1 => {
                    send_arm(r, &commands, &ingress);
                    stage = 2;
                }
                2 if r.life.status().phase == Phase::Active => {
                    feedback(r, &fb, &mut first, now, Backend::DirectXdma);
                    stage = 3;
                }
                3 if r.life.status().correction_applied == Some(true) => {
                    stopped_feedback = now;
                    stage = 4;
                }
                4 => {
                    if busy {
                        let _ = commands.try_send(TxCommand::Arm { rf_enabled: true });
                    }
                    if r.life.status().primary_fault == Some(Fault::Stale) {
                        assert!(now - stopped_feedback < 300_000_000);
                        stage = 5;
                    }
                }
                5 if r.life.status().phase == Phase::ReadyOff => {
                    assert_eq!(
                        r.counters.deadline_stops, 0,
                        "periodic lifecycle, not source/total timeout, detected loss"
                    );
                    commands.try_send(TxCommand::Shutdown).unwrap();
                    stage = 6;
                }
                _ => {}
            }
            if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
                && now >= next_audio
            {
                send_audio(r, &audio, now);
                next_audio = now + 11_000_000;
            }
        });
        assert_eq!(stage, 6);
        assert_eq!(r.life.status().primary_fault, Some(Fault::Stale));
        assert!(r.life.offline_owner().queues_empty());
    }
}

#[test]
fn scheduled_owner_release_cancels_queued_config_audio_and_arm_before_drain() {
    let d = DirectFixture::new();
    d.fifo_ready();
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 5;
    let mut owner = OwnerStopAdapter::direct(d.radio.clone(), &mut iq, &mut mic).unwrap();
    owner.enable_offline_handoff(&m, 5).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let life = controller(
        Fake {
            trace: trace.clone(),
        },
        owner,
        Backend::DirectXdma,
    );
    let (commands, rx) = mpsc::sync_channel(16);
    let (audio, arx) = mpsc::sync_channel(16);
    let (_fb, frx) = mpsc::sync_channel(2);
    let mut r = OwnerLoop::new(
        life,
        rx,
        arx,
        frx,
        Instant::now(),
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 1_000_000_000,
            tx_duration_ns: 2_000_000_000,
        },
    )
    .unwrap();
    let ingress = OperatorIngress::fixture();
    let mut stage = 0;
    let mut old_stamp = None;
    let mut next_audio = 0;
    r.run(&AtomicBool::new(false), |r, now| {
        assert!(now < 3_000_000_000, "stage={stage} {:?}", r.life.status());
        match stage {
            0 if r.life.status().phase == Phase::ReadyOff => {
                send_config(r, &commands, 300, TxAudioSource::Tci, 11);
                stage = 1;
            }
            1 if r.counters.configurations == 1 => {
                send_arm(r, &commands, &ingress);
                send_arm(r, &commands, &ingress); // second queued authorization cannot re-arm later
                stage = 2;
            }
            2 if r.life.status().phase == Phase::Active => {
                old_stamp = Some(r.snapshot());
                send_audio(r, &audio, now);
                send_config(r, &commands, 900, TxAudioSource::Satp, 99);
                r.cancel_handle().release();
                stage = 3;
            }
            3 if r.life.status().phase == Phase::ReadyOff => {
                assert_eq!(r.counters.configurations, 1);
                assert_eq!(trace.borrow().models, vec![300]);
                let old = old_stamp.unwrap().output().unwrap();
                assert_eq!(
                    r.life.offline_owner().enqueue_iq(old, now, &[0.2; 480]),
                    Err(Fault::GenerationMismatch)
                );
                send_config(r, &commands, 500, TxAudioSource::Tci, 12);
                stage = 4;
            }
            4 if r.counters.configurations == 2 => {
                send_arm(r, &commands, &ingress);
                stage = 5;
            }
            5 if r.life.status().phase == Phase::Active => {
                let old = old_stamp.unwrap().output().unwrap();
                assert_eq!(
                    r.life.offline_owner().enqueue_iq(old, now, &[0.2; 480]),
                    Err(Fault::GenerationMismatch)
                );
                ingress.disconnect();
                stage = 6;
            }
            6 if r.life.status().phase == Phase::ReadyOff => {
                assert_eq!(r.life.status().primary_fault, Some(Fault::TxAuthority));
                commands.try_send(TxCommand::Shutdown).unwrap();
                stage = 7;
            }
            _ => {}
        }
        if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
            && now >= next_audio
            && (stage == 2 || stage == 5)
        {
            send_audio(r, &audio, now);
            next_audio = now + 11_000_000;
        }
    });
    assert_eq!(stage, 7);
    assert!(r.counters.rejected >= 3);
    assert_eq!(trace.borrow().models, vec![300, 500]);
    assert!(r.life.offline_owner().queues_empty());
}

#[cfg(all(
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
#[allow(dead_code)]
#[path = "../../tests/support/puresignal-controller-native-replay.rs"]
mod scheduler_replay;
#[cfg(all(
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
mod native {
    use super::scheduler_replay::{Evidence, FeedbackTarget, Replay};
    use super::*;
    use crate::ps_native_port::{OfflineNative, Stats};
    struct FeedbackQueue<'a> {
        sender: &'a SyncSender<Box<PairBlock>>,
        status: saturn_bridge::puresignal::controller::Status,
    }
    impl FeedbackTarget for FeedbackQueue<'_> {
        fn fence(&self) -> Fence {
            self.status.fence
        }
        fn status(&self) -> saturn_bridge::puresignal::controller::Status {
            self.status
        }
        fn submit(&mut self, b: &PairBlock, _: u64) -> Result<(), Fault> {
            let mut copy = PairBlock::default();
            copy.copy_from(b);
            self.sender
                .try_send(Box::new(copy))
                .map_err(|_| Fault::QueueOverrun)
        }
        fn pump(&mut self, _: u64) -> Result<usize, Fault> {
            Ok(0)
        } // only enqueue; actual scheduler calls native
        fn fault(&mut self, f: Fault, _: u64) {
            panic!("wire fixture rejected {f:?}")
        }
    }
    struct Port {
        native: OfflineNative,
        models: Rc<RefCell<Vec<i32>>>,
    }
    unsafe extern "C" {
        fn SetTXABandpassFreqs(channel: i32, low: f64, high: f64);
        fn SetTXAMode(channel: i32, mode: i32);
        fn SetTXAPanelGain1(channel: i32, gain: f64);
        fn GetWDSPVersion() -> i32;
        fn fftw_set_timelimit(seconds: f64);
        fn GetPSHWPeak(channel: i32, peak: *mut f64);
        fn saturn_offline_satp_history() -> i32;
        fn saturn_offline_iqc_flags() -> i32;
        fn saturn_offline_tx_features() -> i32;
        fn saturn_offline_tx_mic_gain() -> f64;
    }
    fn iqc_flags() -> i32 {
        unsafe { saturn_offline_iqc_flags() }
    }
    fn hardware_peak() -> f64 {
        let mut peak = 0.;
        unsafe { GetPSHWPeak(1, &mut peak) };
        peak
    }
    struct IngressFeedback<'a> {
        ingress: &'a crate::tx_thread::ps_ingress::SharedIngress,
        status: saturn_bridge::puresignal::controller::Status,
    }
    impl FeedbackTarget for IngressFeedback<'_> {
        fn fence(&self) -> Fence {
            self.status.fence
        }
        fn status(&self) -> saturn_bridge::puresignal::controller::Status {
            self.status
        }
        fn submit(&mut self, b: &PairBlock, _: u64) -> Result<(), Fault> {
            let mut copy = PairBlock::default();
            copy.copy_from(b);
            self.ingress.feedback(Box::new(copy))
        }
        fn pump(&mut self, _: u64) -> Result<usize, Fault> {
            Ok(0)
        }
        fn fault(&mut self, f: Fault, _: u64) {
            panic!("feedback decoder {f:?}")
        }
    }
    fn signed_satp(sequence: u32, counter: u64, key: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut b = vec![0; 544];
        b[..4].copy_from_slice(b"SAT1");
        b[4] = 2;
        b[5] = 1;
        b[6] = 1;
        b[7] = 1;
        b[8..12].copy_from_slice(&7u32.to_le_bytes());
        b[12..16].copy_from_slice(&1u32.to_le_bytes());
        b[16..20].copy_from_slice(&sequence.to_le_bytes());
        b[20..28].copy_from_slice(&counter.to_le_bytes());
        b[28..30].copy_from_slice(&128u16.to_le_bytes());
        for i in 0..128 {
            let s = (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.3;
            b[32 + i * 4..36 + i * 4].copy_from_slice(&s.to_le_bytes());
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(&b);
        b.extend_from_slice(&mac.finalize().into_bytes());
        b
    }
    struct UdpMicSource {
        stop: Arc<AtomicBool>,
        skip: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl UdpMicSource {
        fn start(destination: std::net::SocketAddr, key: Vec<u8>) -> Self {
            assert!(destination.ip().is_loopback());
            let stop = Arc::new(AtomicBool::new(false));
            let skip = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let skipped = skip.clone();
            let worker = std::thread::spawn(move || {
                let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                let mut sequence = 0;
                let mut counter = 0;
                let mut next = Instant::now();
                while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    let now = Instant::now();
                    if now < next {
                        std::thread::sleep((next - now).min(Duration::from_micros(250)));
                        continue;
                    }
                    if skipped.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        counter += 256;
                    }
                    for _ in 0..4 {
                        socket
                            .send_to(&signed_satp(sequence, counter, &key), destination)
                            .unwrap();
                        sequence += 1;
                        counter += 128;
                    }
                    next += Duration::from_nanos(10_666_667);
                }
            });
            Self {
                stop,
                skip,
                worker: Some(worker),
            }
        }
    }
    impl Drop for UdpMicSource {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }
    #[test]
    fn combined_actual_tci_socket_satp_udp_shared_owner_both_backends() {
        actual_ingress_replay(false, false);
    }
    #[test]
    #[ignore = "requires explicit offline real-browser runner and local speech fixture"]
    fn browser_experiment_actual_ingress_native_both_backends() {
        browser_ingress_native_replay(false, false);
    }
    #[test]
    #[ignore = "requires explicit offline Chrome and local speech fixture; cancellation correction only"]
    fn browser_cancellation_pcm_lifecycle_both_backends() {
        browser_ingress_native_replay(true, false);
    }
    #[test]
    #[ignore = "requires explicit offline actual Saturn Remote UI, real Chrome and local speech"]
    fn browser_managed_pcm_ui_both_backends() {
        browser_ingress_native_replay(true, true);
    }
    fn browser_ingress_native_replay(pcm_only: bool, actual_ui: bool) {
        use crate::{
            tci::ps_ingress_fixture::SocketFixture, tx_thread::ps_ingress::SharedIngress,
            wdsp::ps_native::FullNative,
        };
        use std::process::Command;
        // Explicit supplied tool paths. Never launch the installed Bridge or
        // discover/open /dev/xdma. DirectFixture uses ordinary files only.
        let python = std::env::var("PS_BROWSER_PYTHON").expect("explicit Python runner");
        let runner = std::env::var("PS_BROWSER_RUNNER").expect("explicit browser script");
        let speech = std::env::var("PS_BROWSER_SPEECH").expect("explicit local speech");
        let result_base = std::env::var("PS_BROWSER_RESULTS").expect("new result prefix");
        unsafe {
            fftw_set_timelimit(0.01);
        }
        for backend in [Backend::P2, Backend::DirectXdma] {
            let p2 = (backend == Backend::P2).then(p2_separate_lanes);
            if let Some((_, hp, iq)) = &p2 {
                hp.set_nonblocking(true).unwrap();
                iq.set_nonblocking(true).unwrap();
            }
            let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &direct {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let model = Arc::new(Mutex::new(m.clone()));
            let mut owner = if let Some((s, _, _)) = &p2 {
                OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            owner.enable_offline_handoff(&m, 5).unwrap();
            let socket = SocketFixture::new();
            let native =
                FullNative::offline(&m, unsafe { GetWDSPVersion() }, Some(140), iqc_flags).unwrap();
            let (mut r, ingress): (_, SharedIngress) = OwnerLoop::with_shared_ingress(
                controller(native, owner, backend),
                socket.issuer(),
                model.clone(),
                Instant::now(),
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 8_000_000_000,
                },
            )
            .unwrap();
            if actual_ui {
                ingress.enable_tagged_pcm_review().unwrap();
            } else {
                ingress.enable_remote_media_review().unwrap();
            }
            let listener = if actual_ui {
                socket.browser_ui_listener(ingress.clone(), model)
            } else {
                socket.browser_listener(ingress.clone(), model)
            };
            let result = format!("{result_base}-{backend:?}");
            let mut process = None;
            let mut configured = false;
            let mut done = false;
            let mut replay = None;
            let mut epoch = None;
            let mut last_state = None;
            let mut active_generations = 0;
            let mut exchanges = 0;
            let mut events = Vec::new();
            let mut next_diagnostic = 0;
            let mut ui_interrupted = false;
            r.run(&AtomicBool::new(false), |r, now| {
                assert!(
                    now < 40_000_000_000,
                    "browser timeout {:?}",
                    r.life.status()
                );
                let state = r.life.status();
                if actual_ui && !ui_interrupted && std::path::Path::new(&format!("{result}/request-disconnect")).exists() {
                    assert_eq!(state.phase, Phase::Active, "fixture interruption must occur while keyed");
                    listener.interrupt_connections(); ui_interrupted = true;
                }
                if pcm_only {
                    assert_eq!(state.primary_fault, None, "ordinary cancellation acquired a false fault");
                    assert_eq!(state.authority_fault, None);
                    assert_eq!(state.cleanup_fault, None);
                }
                let view = format!("{:?}:{:?}", state.phase, state.primary_fault);
                if last_state.as_ref() != Some(&view) {
                    events.push(format!("{now} {view}"));
                    last_state = Some(view);
                }
                let audit = r.life.offline_native_snapshot(|n| n.audit());
                if state.phase == Phase::AwaitingKey && now >= next_diagnostic {
                    let d = r.life.offline_native_snapshot(|n| n.diagnostics());
                    events.push(format!(
                        "{now} DSP input_peak={} output_peak={} input_samples={} output_pairs={}",
                        d.input_peak, d.output_peak, d.total_input_samples, d.total_output_pairs
                    ));
                    next_diagnostic = now + 100_000_000;
                }
                assert!(audit.exchanges - exchanges <= 1);
                exchanges = audit.exchanges;
                while let Ok(command) = socket.legacy.try_recv() {
                    assert!(
                        !matches!(
                            command,
                            crate::tci::TciCommand::SetTxEnabled(_)
                                | crate::tci::TciCommand::MicAudioFrame(_)
                        ),
                        "parallel legacy TX"
                    );
                }
                if let Some((_, hp, iq)) = &p2 {
                    let mut b = [0; 2048];
                    while hp.recv(&mut b).is_ok() {}
                    while iq.recv(&mut b).is_ok() {}
                }
                if !configured && state.phase == Phase::ReadyOff {
                    ingress
                        .bootstrap(m.clone(), cfg(11), TxAudioSource::Tci)
                        .unwrap();
                    configured = true;
                }
                if r.counters.configurations == 1 && process.is_none() && !done {
                    let mut command = Command::new(&python);
                    command.arg(&runner).args([
                                "--speech",
                                &speech,
                                "--output",
                                &result,
                                "--bridge",
                                &listener.url,
                            ]);
                    if pcm_only { command.arg("--lifecycle-pcm-only"); }
                    process = Some(command.spawn().unwrap());
                }
                if state.phase == Phase::Active {
                    if epoch != Some(r.life.fence().tx_epoch) {
                        epoch = Some(r.life.fence().tx_epoch);
                        active_generations += 1;
                        let mut src = Replay::new(backend, r.life.fence(), now);
                        src.evidence = Evidence::Missing;
                        replay = Some(src);
                    }
                    let src = replay.as_mut().unwrap();
                    src.clock = now - 10_000;
                    src.feed(&mut IngressFeedback {
                        ingress: &ingress,
                        status: r.life.feedback_status(),
                    });
                } else {
                    epoch = None;
                    replay = None;
                }
                if let Some(p) = process.as_mut() {
                    if let Some(status) = p.try_wait().unwrap() {
                        std::fs::write(format!("{result}/owner-events.txt"), events.join("\n")).unwrap();
                        std::fs::write(format!("{result}/owner-result.txt"), format!("{backend:?} {:?} active_generations={active_generations} exchanges={exchanges}\n", r.counters)).unwrap();
                        assert!(status.success(), "real browser failed {status}");
                        process = None;
                        done = true;
                        ingress.shutdown().unwrap();
                    }
                }
            });
            std::fs::write(format!("{result}/owner-events.txt"), events.join("\n")).unwrap();
            std::fs::write(format!("{result}/owner-result.txt"), format!("{backend:?} {:?} active_generations={active_generations} exchanges={exchanges}\n", r.counters)).unwrap();
            assert!(
                done && active_generations >= 2,
                "non-vacuous PCM release/rearm and actual native rendering"
            );
            if pcm_only {
                if actual_ui {
                    assert!(ui_interrupted, "actual keyed socket loss not exercised");
                }
                assert_eq!(
                    active_generations, 6,
                    "each fresh PCM action reaches actual native/output eligibility"
                );
                let expected_configurations = if actual_ui {
                    // The current UI also restores ordinary profiles. Match
                    // every actual committed transaction, rather than a stale
                    // fixed count from the earlier minimal UI.
                    let applied: u64 =
                        std::fs::read_to_string(format!("{result}/settings-completions.count"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                    assert!(
                        applied >= 6,
                        "separate gain and EQ settings on every reconnect"
                    );
                    applied + 1 // initial owner bootstrap
                } else {
                    7
                };
                assert_eq!(
                    r.counters.configurations, expected_configurations,
                    "all browser-applied settings correspond to native owner commits"
                );
                assert_eq!(r.life.status().primary_fault, None);
            }
            assert!(r.counters.mic_blocks > 0 && r.counters.iq_packets > 0);
            assert_eq!(iqc_flags(), 0);
            assert!(r.life.offline_owner().queues_empty());
            if let Some(d) = &direct {
                d.assert_off();
            }
            std::fs::write(format!("{result}/owner-events.txt"), events.join("\n")).unwrap();
            std::fs::write(format!("{result}/owner-result.txt"), format!("{backend:?} {:?} active_generations={active_generations} exchanges={exchanges} cleanup=complete legacy_output=none IQC=off\n", r.counters)).unwrap();
            eprintln!(
                "browser actual ingress/full native {backend:?}: {:?}",
                r.counters
            );
            drop(listener);
        }
    }
    #[test]
    #[ignore = "requires frozen actual WebCodecs corpus from the browser experiment"]
    fn browser_experiment_actual_bridge_decoder_corpus() {
        use crate::tx_codec::{TxCodecDecoder, TxCodecRuntimeFlags};
        use std::io::Write;
        let directory = std::path::PathBuf::from(std::env::var("PS_BROWSER_CORPUS").unwrap());
        for (mode, expected_samples, expected_packets) in [
            ("pcm", 691200, 1350),
            ("continuous", 692160, 721),
            ("banked", 864000, 900),
        ] {
            let bytes = std::fs::read(directory.join(format!("{mode}-bridge-corpus.bin"))).unwrap();
            let mut cursor = bytes.as_slice();
            let mut bank = None;
            let mut decoder = None;
            let mut samples = Vec::new();
            let mut packets = 0;
            while !cursor.is_empty() {
                assert!(cursor.len() >= 8);
                let id = u32::from_le_bytes(cursor[..4].try_into().unwrap());
                let size = u32::from_le_bytes(cursor[4..8].try_into().unwrap()) as usize;
                cursor = &cursor[8..];
                assert!((64..=4096).contains(&size) && size <= cursor.len());
                let parts = crate::tci::parse_tci_mic_frame_parts(&cursor[..size]).unwrap();
                // All actual raw flush packets preserved. Continuous reference
                // stays OUTSIDE managed PSG1. Bank reset follows actual encoder
                // instances, not sequence%4 after pretending fifth packets vanish.
                if bank != Some(id) {
                    decoder = Some(TxCodecDecoder::new_with_flags(
                        parts.codec,
                        TxCodecRuntimeFlags {
                            opus_decode_enabled: true,
                        },
                    ));
                    bank = Some(id);
                }
                let decoded = decoder
                    .as_mut()
                    .unwrap()
                    .decode(
                        parts.sample_type,
                        parts.sample_count,
                        parts.payload,
                        parts.declared_payload_bytes,
                    )
                    .unwrap();
                assert_eq!(decoded.len(), parts.sample_count as usize);
                assert!(decoded.iter().all(|s| s.is_finite()));
                samples.extend(decoded);
                packets += 1;
                cursor = &cursor[size..];
            }
            assert_eq!(packets, expected_packets);
            assert_eq!(samples.len(), expected_samples);
            let reference =
                std::fs::read(directory.join(format!("{mode}-all-decoded.f32"))).unwrap();
            assert_eq!(reference.len(), samples.len() * 4);
            for (got, expected) in samples.iter().zip(reference.chunks_exact(4)) {
                let expected = f32::from_le_bytes(expected.try_into().unwrap());
                assert!(
                    (got - expected).abs() < 1e-7,
                    "actual Bridge decoder disagrees with saved audio"
                );
            }
            let mut saved = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("{mode}-bridge-decoded.f32")))
                .unwrap();
            for sample in samples {
                saved.write_all(&sample.to_le_bytes()).unwrap();
            }
            eprintln!("actual Bridge decoder {mode}: packets={packets} samples={expected_samples}, saved audio matched");
        }
    }
    #[test]
    fn combined_remote_split_pcm_shared_owner_both_backends() {
        actual_ingress_replay(true, false);
    }
    #[test]
    fn combined_remote_split_opus_shared_owner_both_backends() {
        actual_ingress_replay(true, true);
    }
    fn actual_ingress_replay(tagged: bool, opus: bool) {
        // Same offline planner setting as OfflineNative::open; this focused
        // test must not depend on another test having initialized FFTW first.
        unsafe {
            fftw_set_timelimit(0.01);
        }
        use crate::{
            tci::ps_ingress_fixture::SocketFixture,
            tx_audio::TxAudioIngress,
            tx_thread::owner_selection::{select, Selection},
            wdsp::ps_native::FullNative,
        };
        for backend in [Backend::P2, Backend::DirectXdma] {
            let p2 = (backend == Backend::P2).then(p2_separate_lanes);
            if let Some((_, hp, iq)) = &p2 {
                hp.set_nonblocking(true).unwrap();
                iq.set_nonblocking(true).unwrap();
            }
            let d = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &d {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            m.satp.enabled = true;
            let key_hex = m.satp.pair(1, Instant::now()).unwrap();
            let key: Vec<_> = (0..32)
                .map(|i| u8::from_str_radix(&key_hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            let model = Arc::new(Mutex::new(m.clone()));
            let mut owner = if let Some((s, _, _)) = &p2 {
                OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(d.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            owner.enable_offline_handoff(&m, 5).unwrap();
            let socket = SocketFixture::new();
            let (mut r, ingress) = select(
                Selection::SharedReview,
                || panic!("parallel legacy TX/native owner selected"),
                || {
                    let native =
                        FullNative::offline(&m, unsafe { GetWDSPVersion() }, Some(140), iqc_flags)?;
                    OwnerLoop::with_shared_ingress(
                        controller(native, owner, backend),
                        socket.issuer(),
                        model.clone(),
                        Instant::now(),
                        Timing {
                            mic_age_ns: 150_000_000,
                            source_stall_ns: 500_000_000,
                            tx_duration_ns: 8_000_000_000,
                        },
                    )
                },
            )
            .unwrap();
            if tagged {
                ingress.enable_remote_media_review().unwrap();
            }
            let mut client = if tagged {
                socket.connect_lane(ingress.clone(), model.clone(), 1, "/control")
            } else {
                socket.connect(ingress.clone(), model.clone())
            };
            let mut media =
                tagged.then(|| socket.connect_lane(ingress.clone(), model.clone(), 2, "/media"));
            if let Some(mic) = media.as_mut() {
                client.text(
                    "session_open:combined-review,operator;session_lane:combined-review,control;",
                );
                mic.text("session_open:combined-review,viewer;session_lane:combined-review,media;");
                client.text(if opus {
                    "tx_codec_caps:pcm,opus_wb;"
                } else {
                    "tx_codec_caps:pcm;"
                });
            }
            assert!(
                matches!(
                    crate::wdsp::WdspTxEngine::try_new(&m),
                    Err(Fault::ExternalController)
                ),
                "a legacy engine cannot coexist with the selected shared engine"
            );
            let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let addr = udp.local_addr().unwrap();
            let source = UdpMicSource::start(addr, key);
            let satp = crate::satp::SatpRuntime::loopback_fixture(
                udp,
                model.clone(),
                TxAudioIngress::shared(ingress.clone()),
                512,
                4096,
            );
            let mut stage = 0;
            let mut replay = None;
            let mut next_mic = 0;
            let mut mic_sequence = 0;
            let mut corrected = [false; 3];
            let mut exchanges = 0;
            let mut last_active_stage = None;
            let mut last_token = None;
            let mut old_frame = None;
            let mut old_injected = false;
            r.run(&AtomicBool::new(false), |r, now| {
                client.drain();
                if let Some(m) = media.as_mut() {
                    m.drain();
                }
                if matches!(stage, 3 | 5 | 7)
                    && r.life.status().phase == Phase::ReadyOff
                    && last_active_stage == Some(stage)
                {
                    panic!(
                        "actual ingress stopped stage={stage} {:?} {:?} satp={:?}",
                        r.life.status(),
                        r.counters,
                        satp.snapshot()
                    );
                }
                if r.life.status().phase == Phase::Active {
                    last_active_stage = Some(stage);
                }
                assert!(
                    now < 20_000_000_000,
                    "actual ingress stage={stage} {:?} {:?} satp={:?}",
                    r.life.status(),
                    r.counters,
                    satp.snapshot()
                );
                let audit = r.life.offline_native_snapshot(|n| n.audit());
                assert!(audit.exchanges - exchanges <= 1);
                exchanges = audit.exchanges;
                while let Ok(command) = socket.legacy.try_recv() {
                    assert!(
                        !matches!(
                            command,
                            crate::tci::TciCommand::SetTxEnabled(_)
                                | crate::tci::TciCommand::MicAudioFrame(_)
                                | crate::tci::TciCommand::SetTxMicGain(_)
                                | crate::tci::TciCommand::SatpControl { .. }
                        ),
                        "TX leaked into legacy mailbox"
                    );
                }
                if let Some((_, hp, iq)) = &p2 {
                    let mut b = [0; 2048];
                    while hp.recv(&mut b).is_ok() {}
                    while iq.recv(&mut b).is_ok() {}
                }
                match stage {
                    0 if r.life.status().phase == Phase::ReadyOff => {
                        ingress
                            .bootstrap(m.clone(), cfg(11), TxAudioSource::Tci)
                            .unwrap();
                        stage = 1;
                    }
                    1 if r.counters.configurations == 1 => {
                        client.text("tx_mic_gain:0,-3;");
                        client.text("tx_eq_enable:0,true;"); // separate actual WS message
                        stage = 2;
                    }
                    2 if r.counters.configurations == 3
                        && client
                            .transactions
                            .iter()
                            .any(|s| s == "tx_config:0,2,applied,13;") =>
                    {
                        assert_eq!(model.lock().unwrap().desired.tx_mic_gain_db, -3.);
                        assert!(model.lock().unwrap().desired.tx_eq_enabled);
                        assert_ne!(unsafe { saturn_offline_tx_features() } & 1, 0);
                        assert!(
                            (unsafe { saturn_offline_tx_mic_gain() } - 10f64.powf(-3. / 20.)).abs()
                                < 1e-12
                        );
                        client.text("trx:0,true;");
                        stage = 3;
                    }
                    3 | 5 | 7 if r.life.status().phase == Phase::Active => {
                        if replay.is_none() {
                            let mut src = Replay::new(backend, r.life.fence(), now);
                            src.evidence = Evidence::Missing;
                            replay = Some(src);
                        }
                        let src = replay.as_mut().unwrap();
                        src.clock = now - 10_000;
                        src.feed(&mut IngressFeedback {
                            ingress: &ingress,
                            status: r.life.feedback_status(),
                        });
                        if r.life.status().correction_applied == Some(true) {
                            corrected[if stage == 3 {
                                0
                            } else if stage == 5 {
                                1
                            } else {
                                2
                            }] = true;
                            if stage == 3 {
                                client.text("trx:0,false;");
                                stage = 4;
                            } else if stage == 5 {
                                client.text("saturn_satp_control:satp;");
                                stage = 6;
                            } else {
                                source.skip.store(true, std::sync::atomic::Ordering::SeqCst);
                                stage = 8;
                            } // authenticated counter discontinuity before jitter
                        }
                    }
                    4 if r.life.status().phase == Phase::ReadyOff => {
                        assert!(
                            (unsafe { saturn_offline_tx_mic_gain() } - 10f64.powf(-3. / 20.)).abs()
                                < 1e-12
                        );
                        client.text("trx:0,true;");
                        replay = None;
                        stage = 5;
                    }
                    6 if r.counters.configurations == 4
                        && r.life.status().phase == Phase::ReadyOff =>
                    {
                        // A keyed configuration change requires actual PTT
                        // release before a fresh action; do not manufacture it.
                        client.text("trx:0,false;");
                        stage = 61;
                    }
                    61 if r.life.status().phase == Phase::ReadyOff
                        && r.life.status().release_handled >= r.life.status().required_release =>
                    {
                        client.text("trx:0,true;");
                        replay = None;
                        stage = 7;
                    }
                    8 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(r.life.status().primary_fault, Some(Fault::SequenceGap));
                        assert_eq!(iqc_flags(), 0);
                        if let Some(m) = media.as_mut() {
                            m.close();
                        }
                        client.close();
                        stage = 9;
                    }
                    9 if r.life.status().phase == Phase::ReadyOff => {
                        ingress.shutdown().unwrap();
                        stage = 10;
                    }
                    _ => {}
                }
                if matches!(stage, 3 | 5) && now >= next_mic {
                    let samples: Vec<_> = (0..512)
                        .map(|i| (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.3)
                        .collect();
                    if let Some(m) = media.as_mut() {
                        if let Some(token) = client.media_token {
                            if last_token != Some(token) {
                                last_token = Some(token);
                                mic_sequence = 0;
                            }
                            if stage == 5 && !old_injected {
                                m.binary(old_frame.clone().unwrap());
                                old_injected = true;
                            }
                            let frame = m.tagged_mic(token, &samples, mic_sequence, opus);
                            old_frame.get_or_insert(frame);
                            mic_sequence += 1;
                        }
                    } else {
                        client.mic(&samples, mic_sequence);
                        mic_sequence += 1;
                    }
                    // Intermittent PCM exercises speech pauses/zero fill.
                    // This is NOT a sustained TCI uplink throughput benchmark.
                    next_mic = now + 20_000_000;
                }
            });
            assert_eq!(stage, 10);
            assert!(corrected.into_iter().all(|b| b));
            assert!(r.counters.mic_blocks > 0 && r.counters.iq_packets > 0);
            assert!(satp.snapshot().frames_delivered > 0);
            if tagged {
                assert!(old_injected, "stale media really delivered after re-arm");
            }
            assert_eq!(iqc_flags(), 0);
            assert!(r.life.offline_owner().queues_empty());
            if let Some(d) = &d {
                d.assert_off();
            }
            eprintln!("actual TCI WebSocket + authenticated SATP UDP + full WDSP {backend:?}: three corrected generations, release/rearm/source change, pre-jitter discontinuity latched, disconnect/cleanup complete {:?}",r.counters);
            drop(satp);
            drop(source);
            drop(r);
        }
    }
    #[test]
    fn combined_full_native_satp_source_changes_rearm_and_discontinuity() {
        use crate::wdsp::ps_native::FullNative;
        // This test must also run alone, without inheriting planner settings
        // from an earlier test. Limit FFTW planning, not lifecycle deadlines.
        unsafe {
            fftw_set_timelimit(0.01);
        }
        for backend in [Backend::P2, Backend::DirectXdma] {
            let p2 = (backend == Backend::P2).then(p2_separate_lanes);
            if let Some((_, hp, iq)) = &p2 {
                hp.set_nonblocking(true).unwrap();
                iq.set_nonblocking(true).unwrap();
            }
            let d = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &d {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let mut owner = if let Some((s, _, _)) = &p2 {
                OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(d.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            owner.enable_offline_handoff(&m, 5).unwrap();
            assert_eq!(unsafe { saturn_offline_satp_history() }, 140);
            let native =
                FullNative::offline(&m, unsafe { GetWDSPVersion() }, Some(140), iqc_flags).unwrap();
            assert!((hardware_peak() - 0.4072).abs() < 1e-12);
            assert!(matches!(
                FullNative::offline(&m, 210, Some(140), iqc_flags),
                Err(Fault::ExternalController)
            ));
            let life = controller(native, owner, backend);
            let (commands, rx) = mpsc::sync_channel(16);
            let (audio, arx) = mpsc::sync_channel(16);
            let (fb, frx) = mpsc::sync_channel(2);
            let mut r = OwnerLoop::new(
                life,
                rx,
                arx,
                frx,
                Instant::now(),
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 8_000_000_000,
                },
            )
            .unwrap();
            let ingress = OperatorIngress::fixture();
            let mut stage = 0;
            let mut next_audio = 0;
            let mut replay = None;
            let mut exchanges = 0;
            let mut old = None;
            let mut correction_seen = false;
            let mut rearm_correction_seen = false;
            let mut retained_delay = None;
            let mut selected = TxAudioSource::Satp;
            r.run(&AtomicBool::new(false), |r, now| {
                if r.life.status().phase == Phase::Blocked {
                    unsafe extern "C" {fn GetPSInfo(channel:i32,info:*mut i32);}
                    let mut info=[0i32;16];
                    unsafe {GetPSInfo(1,info.as_mut_ptr());}
                    panic!("full pipeline blocked stage={stage} info={info:?} iqc_flags={} status={:?}",iqc_flags(),r.life.status());
                }
                assert!(
                    now < 12_000_000_000,
                    "stage={stage} full pipeline stalled {:?}",
                    r.life.status()
                );
                let a = r.life.offline_native_snapshot(|n| n.audit());
                assert!(a.exchanges - exchanges <= 1);
                exchanges = a.exchanges;
                if let Some((_, hp, duc)) = &p2 {
                    let mut b = [0u8; 2048];
                    while hp.recv(&mut b).is_ok() {}
                    while duc.recv(&mut b).is_ok() {}
                }
                match stage {
                    0 if r.life.status().phase == Phase::ReadyOff => {
                        let mut snapshot = m.clone();
                        snapshot.desired.tx_eq_enabled = true;
                        snapshot.desired.cfc_enabled = true;
                        snapshot.desired.tx_phase_rotator_enabled = true;
                        snapshot.desired.tx_speech_processor_enabled = true;
                        snapshot.desired.tx_cessb_enabled = true;
                        snapshot.desired.tx_mic_gain_db = -3.;
                        snapshot.desired.tx_dexp_enabled = true;
                        snapshot.desired.tx_noise_gate_enabled = true;
                        snapshot.desired.tx_filter_low_hz = 300;
                        // Match the synthetic reference waveform's amplitude
                        // and timing. Native default peak is 0.4072, not this.
                        let ps = cfg(11);
                        commands
                            .try_send(TxCommand::PureSignalOwnerConfig(Box::new(
                                Configuration::at_ingress(
                                    r.snapshot(),
                                    snapshot,
                                    ps,
                                    TxAudioSource::Satp,
                                ),
                            )))
                            .unwrap();
                        stage = 1;
                    }
                    1 if r.counters.configurations == 1 => {
                        assert_eq!(hardware_peak(), 0.6121);
                        retained_delay = r.life.feedback_status().actual_delay_seconds;
                        assert!(retained_delay.is_some());
                        assert_eq!(unsafe { saturn_offline_tx_features() }, 63);
                        assert!(
                            (unsafe { saturn_offline_tx_mic_gain() } - 10f64.powf(-3. / 20.)).abs()
                                < 1e-12
                        );
                        send_arm(r, &commands, &ingress);
                        stage = 2;
                    }
                    2 if r.life.status().phase == Phase::Active => {
                        let mut source = Replay::new(backend, r.life.fence(), now);
                        source.evidence = Evidence::Missing;
                        replay = Some(source);
                        stage = 3;
                    }
                    3 => {
                        if r.life.status().phase == Phase::Active {
                            let source = replay.as_mut().unwrap();
                            source.clock = now - 10_000;
                            source.feed(&mut FeedbackQueue {
                                sender: &fb,
                                status: r.life.feedback_status(),
                            });
                            correction_seen |= r.life.status().correction_applied == Some(true);
                        }
                        if correction_seen {
                            old = Some(r.snapshot());
                            r.cancel_handle().release();
                            stage = 4;
                        }
                    }
                    4 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(iqc_flags(), 0);
                        assert_eq!(unsafe { saturn_offline_tx_features() }, 63);
                        assert!(
                            (unsafe { saturn_offline_tx_mic_gain() } - 10f64.powf(-3. / 20.)).abs()
                                < 1e-12
                        );
                        let audit = r.life.offline_native_snapshot(|n| n.audit());
                        assert!(audit.dexp_exchanges > 0 && audit.recreations >= 3);
                        assert_eq!(hardware_peak(), 0.6121);
                        assert_eq!(
                            r.life.feedback_status().actual_delay_seconds,
                            retained_delay
                        );
                        // No configuration command: cleanup must preserve the
                        // controller's current PS contract on the new channel.
                        send_arm(r, &commands, &ingress);
                        replay = None;
                        stage = 40;
                    }
                    40 if r.life.status().phase == Phase::Active => {
                        if replay.is_none() {
                            assert_eq!(hardware_peak(), 0.6121);
                            let mut source = Replay::new(backend, r.life.fence(), now);
                            source.evidence = Evidence::Missing;
                            replay = Some(source);
                        }
                        let source = replay.as_mut().unwrap();
                        source.clock = now - 10_000;
                        source.feed(&mut FeedbackQueue {
                            sender: &fb,
                            status: r.life.feedback_status(),
                        });
                        if r.life.status().correction_applied == Some(true) {
                            rearm_correction_seen = true;
                            r.cancel_handle().release();
                            stage = 41;
                        }
                    }
                    41 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(hardware_peak(), 0.6121);
                        assert_eq!(unsafe { saturn_offline_tx_features() }, 63);
                        assert_eq!(r.counters.configurations, 1);
                        selected = TxAudioSource::Tci;
                        send_config(r, &commands, 500, selected, 12);
                        stage = 5;
                    }
                    5 if r.counters.configurations == 2 => {
                        assert_eq!(
                            unsafe { saturn_offline_tx_features() },
                            i32::from(model().desired.tx_noise_gate_enabled) << 5
                        );
                        send_arm(r, &commands, &ingress);
                        stage = 6;
                    }
                    6 if r.life.status().phase == Phase::Active => {
                        let old_output = old.unwrap().output().unwrap();
                        assert_eq!(
                            r.life
                                .offline_owner()
                                .enqueue_iq(old_output, now, &[0.2; 480]),
                            Err(Fault::GenerationMismatch)
                        );
                        r.cancel_handle().release();
                        stage = 7;
                    }
                    7 if r.life.status().phase == Phase::ReadyOff => {
                        selected = TxAudioSource::Satp;
                        send_config(r, &commands, 600, selected, 13);
                        stage = 8;
                    }
                    8 if r.counters.configurations == 3 => {
                        send_arm(r, &commands, &ingress);
                        replay = None;
                        stage = 9;
                    }
                    9 if r.life.status().phase == Phase::Active => {
                        if replay.is_none() {
                            let mut source = Replay::new(backend, r.life.fence(), now);
                            source.evidence = Evidence::Missing;
                            replay = Some(source);
                        }
                        let source = replay.as_mut().unwrap();
                        source.clock = now - 10_000;
                        source.feed(&mut FeedbackQueue {
                            sender: &fb,
                            status: r.life.feedback_status(),
                        });
                        // Discontinuity while real async fitting is in progress,
                        // not just a stop after an already completed fit.
                        if r.life.feedback_status().info_raw[15] == 6 {
                            audio
                                .try_send(Audio::at_ingress(
                                    r.snapshot(),
                                    TxAudioMessage::Flush(TxAudioSource::Satp),
                                ))
                                .unwrap();
                            stage = 10;
                        }
                    }
                    10 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(iqc_flags(), 0);
                        assert_eq!(r.life.status().primary_fault, None);
                        commands.try_send(TxCommand::Shutdown).unwrap();
                        stage = 11;
                    }
                    _ => {}
                }
                if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
                    && matches!(stage, 2 | 3 | 6 | 9 | 40)
                    && now >= next_audio
                {
                    // Four explicit SATP packets, original timestamps, not a
                    // freshly labeled fixed-size dequeue block.
                    let count = if selected == TxAudioSource::Satp {
                        4
                    } else {
                        1
                    };
                    for part in 0..count {
                        let n = if selected == TxAudioSource::Satp {
                            128
                        } else {
                            512
                        };
                        let samples = (0..n)
                            .map(|i| {
                                (std::f32::consts::TAU * 700. * (i + part * n) as f32 / 48000.)
                                    .sin()
                                    * 0.3
                            })
                            .collect();
                        let f = TxAudioFrame {
                            enqueued_at: r_origin(r) + Duration::from_nanos(now),
                            source: selected,
                            samples: TxAudioSamples::Dynamic(samples),
                            channels: 1,
                            sample_rate_hz: 48000,
                        };
                        audio
                            .try_send(Audio::at_ingress(r.snapshot(), TxAudioMessage::Frames(f)))
                            .unwrap();
                    }
                    next_audio = now + 10_666_667;
                }
            });
            assert_eq!(stage, 11);
            assert!(correction_seen);
            assert!(rearm_correction_seen);
            assert_eq!(iqc_flags(), 0);
            assert!(r.life.offline_owner().queues_empty());
            let audit = r.life.offline_native_snapshot(|n| n.audit());
            assert!(
                audit.feedback_blocks > 0 && audit.model_updates == 3 && audit.recreations >= 6
            );
            if let Some(d) = &d {
                d.assert_off();
            }
            eprintln!("full native + SATP {backend:?}: correction=true source_change/rearm/flush complete counters={:?}",r.counters);
            drop(r);
        }
    }
    impl NativePs for Port {
        fn configure(&mut self, c: Config) -> Result<f64, Fault> {
            self.native.configure(c)
        }
        fn mox(&mut self, b: bool) {
            self.native.mox(b)
        }
        fn reset(&mut self) -> Result<(), Fault> {
            panic!("runtime must use cooperative cleanup")
        }
        fn process(&mut self, t: &mut [f64; 2048], r: &mut [f64; 2048]) -> Result<(), Fault> {
            self.native.feed_only(t, r)
        }
        fn information(&mut self) -> Result<[i32; 16], Fault> {
            self.native.information()
        }
    }
    impl CooperativeNative for Port {
        fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
            self.native.advance_dsp_discard()
        }
        fn begin_cleanup(&mut self) -> Result<(), Fault> {
            self.native.begin_cleanup()
        }
        fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault> {
            self.native.cleanup_step()
        }
    }
    impl ScheduledNative for Port {
        fn configure_model(&mut self, m: &RadioModel) -> Result<(), Fault> {
            // Bounded USB fixture subset, NOT a claim that the installed TX
            // speech/EQ/CFC chain has migrated or is qualified.
            if m.desired.mode != crate::radio_model::DemodMode::Usb {
                return Err(Fault::InvalidConfig);
            }
            unsafe {
                SetTXAMode(1, 1);
                SetTXABandpassFreqs(
                    1,
                    f64::from(m.desired.tx_filter_low_hz),
                    f64::from(m.desired.tx_filter_high_hz),
                );
                SetTXAPanelGain1(
                    1,
                    10f64.powf(m.desired.tx_mic_gain_db.clamp(-40., 12.) / 20.),
                );
            }
            self.models.borrow_mut().push(m.desired.tx_filter_low_hz);
            Ok(())
        }
        fn render(&mut self, m: &[f32]) -> Result<Vec<f32>, Fault> {
            self.native.render_mic(m)
        }
    }
    #[test]
    fn combined_scheduled_real_wdsp_actual_owners_release_model_disconnect() {
        for backend in [Backend::P2, Backend::DirectXdma] {
            let p2 = (backend == Backend::P2).then(p2_separate_lanes);
            if let Some((_, hp, iq)) = &p2 {
                hp.set_nonblocking(true).unwrap();
                iq.set_nonblocking(true).unwrap();
            }
            let d = (backend == Backend::DirectXdma).then(DirectFixture::new);
            if let Some(d) = &d {
                d.fifo_ready();
            }
            let (mut iq, mut mic) = queues();
            let mut m = model();
            m.desired.tx_drive = 5;
            let mut owner = if let Some((s, _, _)) = &p2 {
                OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(d.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            owner.enable_offline_handoff(&m, 5).unwrap();
            let stats = Rc::new(RefCell::new(Stats::default()));
            let models = Rc::new(RefCell::new(Vec::new()));
            let life = controller(
                Port {
                    native: OfflineNative::open(stats.clone()),
                    models: models.clone(),
                },
                owner,
                backend,
            );
            let (commands, rx) = mpsc::sync_channel(16);
            let (audio, arx) = mpsc::sync_channel(16);
            let (fb, frx) = mpsc::sync_channel(2);
            let mut r = OwnerLoop::new(
                life,
                rx,
                arx,
                frx,
                Instant::now(),
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 8_000_000_000,
                },
            )
            .unwrap();
            let ingress = OperatorIngress::fixture();
            let mut stage = 0;
            let mut replay = None;
            let mut next_audio = 0;
            let mut exchanges = 0;
            let mut active = 0;
            let mut corrected = false;
            let mut old = None;
            r.run(&AtomicBool::new(false), |r, now| {
                assert!(
                    now < 10_000_000_000,
                    "real scheduling stalled {:?}",
                    r.life.status()
                );
                let current = stats.borrow().dsp_exchanges;
                assert!(
                    current - exchanges <= 1,
                    "multiple native exchanges per actual scheduler turn"
                );
                exchanges = current;
                if let Some((_, hp, duc)) = &p2 {
                    let mut packet = [0u8; 2048];
                    while hp.recv(&mut packet).is_ok() {}
                    while duc.recv(&mut packet).is_ok() {}
                }
                match stage {
                    0 if r.life.status().phase == Phase::ReadyOff => {
                        send_config(r, &commands, 300, TxAudioSource::Tci, 11);
                        stage = 1;
                    }
                    1 if r.counters.configurations == 1 => {
                        send_arm(r, &commands, &ingress);
                        stage = 2;
                    }
                    2 if r.life.status().phase == Phase::Active => {
                        active = now;
                        let mut source = Replay::new(backend, r.life.fence(), now);
                        source.evidence = Evidence::Missing;
                        replay = Some(source);
                        stage = 3;
                    }
                    3 => {
                        if r.life.status().phase == Phase::Active {
                            let source = replay.as_mut().unwrap();
                            source.clock = now - 10_000;
                            source.feed(&mut FeedbackQueue {
                                sender: &fb,
                                status: r.life.feedback_status(),
                            });
                        }
                        corrected |= r.life.status().correction_applied == Some(true);
                        if corrected && r.life.status().phase == Phase::Active {
                            assert_eq!(
                                r.life.raw_unclipped_qualification(),
                                Some(Err(Fault::ClipStatusUnknown)),
                                "missing counters cannot certify the measurement"
                            );
                        }
                        if corrected && now - active > 300_000_000 {
                            old = Some(r.snapshot());
                            send_config(r, &commands, 900, TxAudioSource::Satp, 99);
                            send_audio(r, &audio, now);
                            r.cancel_handle().release();
                            stage = 4;
                        }
                    }
                    4 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(models.borrow().as_slice(), [300]);
                        send_config(r, &commands, 500, TxAudioSource::Tci, 12);
                        stage = 5;
                    }
                    5 if r.counters.configurations == 2 => {
                        assert_eq!(models.borrow().as_slice(), [300, 500]);
                        send_arm(r, &commands, &ingress);
                        stage = 6;
                    }
                    6 if r.life.status().phase == Phase::Active => {
                        let old = old.unwrap().output().unwrap();
                        assert_eq!(
                            r.life.offline_owner().enqueue_iq(old, now, &[0.2; 480]),
                            Err(Fault::GenerationMismatch)
                        );
                        ingress.disconnect();
                        stage = 7;
                    }
                    7 if r.life.status().phase == Phase::ReadyOff => {
                        assert_eq!(r.life.status().primary_fault, Some(Fault::TxAuthority));
                        commands.try_send(TxCommand::Shutdown).unwrap();
                        stage = 8;
                    }
                    _ => {}
                }
                if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
                    && matches!(stage, 2 | 3 | 6)
                    && now >= next_audio
                {
                    send_audio(r, &audio, now);
                    next_audio = now + 11_000_000;
                }
            });
            assert!(corrected);
            assert_eq!(stage, 8);
            assert!(r.counters.mic_blocks > 0 && r.counters.iq_packets > 0);
            assert!(r.life.offline_owner().queues_empty());
            assert_eq!(stats.borrow().blocking_resets, 0);
            assert_eq!(stats.borrow().final_iqc_flags, 0);
            eprintln!("combined actual scheduler backend={backend:?} correction_seen={corrected} counters={:?} native={:?}",r.counters,stats.borrow());
            drop(r);
            assert!(stats.borrow().closed);
            assert_eq!(stats.borrow().blocking_resets, 1);
        }
    }
}
