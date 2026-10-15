//! One test serially owns native channel 1 for both actual output adapters.
//! Only loopback sockets and ordinary fixture files can be constructed here.
use super::tests::{model, p2_separate_lanes, queues};
use super::*;
use crate::{
    ps_native_port::{OfflineNative, Stats},
    tci::ps_authorization::OperatorIngress,
    tx_thread::{
        ps_owner_queue::{service_next, AuthorizedArm},
        TxCommand,
    },
    xdma_tx_radio::offline_fixture::DirectFixture,
};
use saturn_bridge::puresignal::{
    admission_proposal::DraftLimits,
    controller::{Config, Controller, Qualification, Status},
    tx_lifecycle::{Limits, Phase, TxLifecycle},
    *,
};
use std::{cell::RefCell, rc::Rc};
#[allow(dead_code)]
#[path = "../../tests/support/puresignal-controller-native-replay.rs"]
mod replay;
use replay::{Evidence, FeedbackTarget, Replay};
impl FeedbackTarget for TxLifecycle<OfflineNative, OwnerStopAdapter<'_>> {
    fn fence(&self) -> Fence {
        self.fence()
    }
    fn status(&self) -> Status {
        self.feedback_status()
    }
    fn submit(&mut self, b: &PairBlock, now: u64) -> Result<(), Fault> {
        self.submit(b, now)
    }
    fn pump(&mut self, now: u64) -> Result<usize, Fault> {
        let processed = self.pump(now)?;
        if processed != 0 {
            // Deliver the output of that SAME native DSP exchange. Reading it
            // performs no extra exchange; the native/controller one-step bound
            // remains intact. The ordinary owner retains partial packet tails.
            let output = self.offline_tx_exchange(now, |native| Ok(native.latest_output()))?;
            let stamp = self.offline_owner().output_stamp()?;
            self.offline_owner().enqueue_iq(stamp, now, &output)?;
            while self.offline_owner().has_iq_packet() {
                assert!(self.offline_owner().flush_iq(stamp, now)?);
            }
        }
        Ok(processed)
    }
    fn fault(&mut self, fault: Fault, now: u64) {
        self.notify_fault(fault, now).unwrap();
    }
}
type Life<'a> = TxLifecycle<OfflineNative, OwnerStopAdapter<'a>>;
fn clean(life: &mut Life<'_>, now: &mut u64) {
    let end = Instant::now() + Duration::from_secs(5);
    while life.status().phase != Phase::ReadyOff {
        *now += 1_000_000;
        life.tick(*now).unwrap();
        assert!(Instant::now() < end, "cleanup failed {:?}", life.status());
        std::thread::sleep(Duration::from_millis(1)); // harness only
    }
}
fn action(life: &mut Life<'_>, ingress: &OperatorIngress, ticket: u64, now: &mut u64) {
    let fence = life.fence();
    let revision = life.offline_owner().ready_revision().unwrap();
    assert!(ingress
        .receive(8, "trx:0,true", ticket, fence, revision)
        .is_err());
    let credential = ingress
        .receive(7, "trx:0,true", ticket, fence, revision)
        .unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    send.send(TxCommand::PureSignalAuthorizedArm(AuthorizedArm::new(
        credential,
    )))
    .unwrap();
    *now += 1;
    assert!(service_next(life, &recv, *now).unwrap());
    assert_eq!(life.status().phase, Phase::AwaitingKey);
}
fn render(life: &mut Life<'_>, now: &mut u64) {
    *now += 1_000_000;
    let stamp = life.offline_owner().output_stamp().unwrap();
    let mic: Vec<f32> = (0..512)
        .map(|i| (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.2)
        .collect();
    life.offline_owner().enqueue_mic(stamp, *now, &mic).unwrap();
    let mic = life.offline_owner().take_mic(stamp).unwrap();
    let iq = life
        .offline_tx_exchange(*now, |native| native.render_mic(&mic))
        .unwrap();
    assert!(iq.iter().all(|v| v.is_finite()));
    // Preserve each native output's observed time; bounded to complete packets.
    life.offline_owner().enqueue_iq(stamp, *now, &iq).unwrap();
}
fn key(life: &mut Life<'_>, now: &mut u64) {
    let end = Instant::now() + Duration::from_secs(5);
    while life.status().phase == Phase::AwaitingKey {
        if !life.offline_owner().has_iq_packet() {
            render(life, now);
        }
        *now += 1_000_000;
        life.tick(*now).unwrap();
        assert!(Instant::now() < end, "key failed {:?}", life.status());
    }
    assert_eq!(life.status().phase, Phase::Active);
    let stamp = life.offline_owner().output_stamp().unwrap();
    while life.offline_owner().has_iq_packet() {
        assert!(life.offline_owner().flush_iq(stamp, *now).unwrap());
    }
}
#[test]
fn combined_real_native_actual_owners_authenticated_generation_handoff() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let stats = Rc::new(RefCell::new(Stats::default()));
        let mut m = model();
        m.desired.tx_drive = 5;
        let (mut iq, mut mic) = queues();
        let p2_fixture = (backend == Backend::P2).then(p2_separate_lanes);
        let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
        if let Some(d) = &direct {
            d.fifo_ready();
        }
        let mut owner = if let Some((s, _, _)) = &p2_fixture {
            OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
        } else {
            OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                .unwrap()
        };
        owner.enable_offline_handoff(&m, 5).unwrap();
        let hp_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hp_worker = p2_fixture.as_ref().map(|(s, _, _)| {
            s.spawn_high_priority_loop(Arc::new(Mutex::new(m.clone())), hp_stop.clone())
                .unwrap()
        });
        let native = OfflineNative::open(stats.clone());
        let controller = Controller::new_offline_admission_proposal(
            native,
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
            controller,
            owner,
            backend,
            Limits {
                authority_timeout_ns: 2_000_000_000,
                cleanup_timeout_ns: 3_000_000_000,
            },
        )
        .unwrap();
        let mut now = 1;
        life.reconfigure(
            life.fence(),
            Config {
                id: 10,
                route: 20,
                physical_adc: OverloadSource::Adc1,
                enabled: true,
                automatic: true,
                hardware_peak: 0.6121,
                requested_delay_seconds: 0.,
                settling_ns: 0,
                loop_delay_seconds: 0.,
                stale_after_ns: 250_000_000,
            },
            now,
        )
        .unwrap();
        clean(&mut life, &mut now);
        let ingress = OperatorIngress::fixture();
        let revision = life.offline_owner().ready_revision().unwrap();
        let queued_old_command = ingress
            .receive(7, "trx:0,true", 0, life.fence(), revision)
            .unwrap();
        let delayed_old_command = ingress
            .receive(7, "trx:0,true", 0, life.fence(), revision)
            .unwrap();
        let (queued_send, queued_recv) = std::sync::mpsc::channel();
        queued_send
            .send(TxCommand::PureSignalAuthorizedArm(AuthorizedArm::new(
                queued_old_command,
            )))
            .unwrap();
        action(&mut life, &ingress, 0, &mut now);
        let old_stamp = life.offline_owner().output_stamp().unwrap();
        render(&mut life, &mut now); // old native/mic/IQ output queued before key
        let old_action = FreshTxAction {
            sequence: 1,
            release_ticket: 0,
            expected_fence: life.fence(),
        };
        let release = life.release_handle();
        let ticket = release.release();
        now += 1;
        // A queued arm cannot outrun the out-of-band release: one real command
        // consumer turn services release first, then refuses the canceled arm.
        assert_eq!(
            service_next(&mut life, &queued_recv, now).unwrap_err(),
            Fault::CleanupPending
        );
        assert!(life.offline_owner().queues_empty());
        assert!(life
            .offline_owner()
            .enqueue_mic(old_stamp, now, &[0.2; 512])
            .is_err());
        clean(&mut life, &mut now);
        assert!(life.fresh_tx_action(old_action, now).is_err());
        // The actual legacy command variant is deliberately not convertible.
        queued_send
            .send(TxCommand::Arm { rf_enabled: true })
            .unwrap();
        assert_eq!(
            service_next(&mut life, &queued_recv, now).unwrap_err(),
            Fault::TxAuthority
        );
        // A credential queued before release and delivered AFTER cleanup is
        // still old. Preserve its original revision across this actual queue.
        queued_send
            .send(TxCommand::PureSignalAuthorizedArm(AuthorizedArm::new(
                delayed_old_command,
            )))
            .unwrap();
        assert!(service_next(&mut life, &queued_recv, now).is_err());
        action(&mut life, &ingress, ticket, &mut now);
        let new_stamp = life.offline_owner().output_stamp().unwrap();
        assert_ne!(old_stamp, new_stamp);
        assert!(life
            .offline_owner()
            .enqueue_iq(old_stamp, now, &[0.2; 480])
            .is_err());
        assert!(life
            .offline_owner()
            .poll_stop(new_stamp.generation - 1)
            .unwrap()
            .is_none());
        key(&mut life, &mut now);
        if let Some((s, _, _)) = &p2_fixture {
            assert!(s.send_high_priority(&m).is_err());
            assert!(s.send_duc_iq(&[0.2; 480]).is_err());
        }
        let mut feedback = Replay::new(backend, life.fence(), now);
        feedback.evidence = Evidence::Missing;
        feedback.until(&mut life, |s| s.correction_applied_reported);
        now = feedback.clock;
        assert_eq!(
            life.raw_unclipped_qualification(),
            Some(Err(Fault::ClipStatusUnknown))
        );
        render(&mut life, &mut now);
        let stamp = life.offline_owner().output_stamp().unwrap();
        assert!(life.offline_owner().flush_iq(stamp, now).unwrap());
        // Hazard with real correction applied: old IQ must be erased before
        // native cleanup, which continues advancing through the actual owner receipt.
        now += 1;
        life.notify_fault(Fault::Stale, now).unwrap();
        assert!(life.offline_owner().queues_empty());
        assert!(life.status().primary_fault.is_some());
        let ticket = release.release();
        now += 1;
        life.tick(now).unwrap();
        clean(&mut life, &mut now);
        assert_eq!(life.status().correction_applied, Some(false));
        if let Some(d) = &direct {
            d.assert_off();
            assert!(d.dma_len() > 0);
        }
        now += 1;
        life.acknowledge_fault(life.status().stop_id, now).unwrap();
        action(&mut life, &ingress, ticket, &mut now);
        let before_reconfigure = life.offline_owner().output_stamp().unwrap();
        render(&mut life, &mut now);
        now += 1;
        life.reconfigure(
            life.fence(),
            Config {
                id: 11,
                route: 20,
                physical_adc: OverloadSource::Adc1,
                enabled: true,
                automatic: true,
                hardware_peak: 0.6121,
                requested_delay_seconds: 0.,
                settling_ns: 0,
                loop_delay_seconds: 0.,
                stale_after_ns: 250_000_000,
            },
            now,
        )
        .unwrap();
        let ticket = release.release();
        now += 1;
        life.tick(now).unwrap();
        clean(&mut life, &mut now);
        assert!(life
            .offline_owner()
            .enqueue_iq(before_reconfigure, now, &[0.2; 480])
            .is_err());
        action(&mut life, &ingress, ticket, &mut now);
        key(&mut life, &mut now);
        render(&mut life, &mut now);
        ingress.disconnect();
        now += 1;
        assert_eq!(life.tick(now).unwrap_err(), Fault::TxAuthority); // periodic role revocation, no packet/release needed
        assert!(life.offline_owner().queues_empty());
        release.release();
        now += 1;
        life.tick(now).unwrap();
        // Exercise a closed EXISTING command channel while native cleanup is
        // advancing. Notification may stop/fault, but must not perform a second
        // DSP exchange in the same consumer turn.
        let deadline = Instant::now() + Duration::from_secs(5);
        while life.status().phase == Phase::AwaitingAuthority {
            now += 1_000_000;
            life.tick(now).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(life.status().phase, Phase::CleaningNative);
        drop(queued_send);
        let exchanges_before = stats.borrow().dsp_exchanges;
        now += 1;
        assert_eq!(
            service_next(&mut life, &queued_recv, now).unwrap_err(),
            Fault::TxAuthority
        );
        assert!(stats.borrow().dsp_exchanges - exchanges_before <= 1);
        assert!(life.offline_owner().queues_empty());
        clean(&mut life, &mut now);
        assert!(life.offline_owner().queues_empty());
        assert_eq!(life.status().phase, Phase::ReadyOff);
        let revision = life.offline_owner().ready_revision().unwrap();
        assert!(ingress
            .receive(
                7,
                "trx:0,true",
                life.status().release_handled,
                life.fence(),
                revision
            )
            .is_err());
        now += 1;
        life.shutdown(now).unwrap();
        clean(&mut life, &mut now);
        assert_eq!(stats.borrow().blocking_resets, 0); // every runtime cleanup was cooperative
        drop(life);
        assert_eq!(stats.borrow().blocking_resets, 1); // isolated, already-quiescent final destructor
        hp_stop.store(true, Ordering::SeqCst);
        if let Some(worker) = hp_worker {
            worker.join().unwrap().unwrap();
        }
        if let Some((_, socket, iq_socket)) = &p2_fixture {
            socket.set_nonblocking(true).unwrap();
            let (mut key_commands, mut zero_drive_commands, mut iq_packets) = (0, 0, 0);
            let mut packet = [0; 2048];
            while let Ok(n) = socket.recv(&mut packet) {
                assert_eq!(n, 1444);
                if packet[4] & 2 != 0 {
                    key_commands += 1;
                } else if packet[4] & 1 != 0 {
                    assert_eq!(packet[345], 0);
                    zero_drive_commands += 1;
                }
            }
            iq_socket.set_nonblocking(true).unwrap();
            while let Ok(n) = iq_socket.recv(&mut packet) {
                assert_eq!(n, 1444);
                if packet[4..n].iter().any(|v| *v != 0) {
                    iq_packets += 1;
                }
            }
            assert_eq!(key_commands, 2); // independent stale HP model never re-keyed
            assert!(zero_drive_commands >= 12);
            assert!(iq_packets > 0);
        }
        assert!(stats.borrow().closed);
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        println!("combined actual owner + real WDSP {:?}: correction=true release/fault/disconnect canceled output, existing TxCommand sealed-arm queue, fresh role-authorized rearm, old IQ/commands/receipts rejected; DSP exchanges={} cooperative steps={}; NO RF-off claim", backend, stats.borrow().dsp_exchanges, stats.borrow().cooperative_steps);
    }
}
