use super::*;
use crate::puresignal::admission_proposal::DraftLimits;
use std::{cell::RefCell, rc::Rc};
#[path = "../../../tests/support/tx_authority_fixture.rs"]
mod authority_fixture;
use authority_fixture::*;

#[derive(Default)]
struct NativeTrace {
    release_on_configure: Option<ReleaseHandle>,
    measurement_disabled: bool,
    measurement_guard_fault: bool,
    events: Vec<&'static str>,
    info: [i32; 16],
    remaining: u32,
    discard_exchanges: u32,
    processed: u32,
    dropped: bool,
    stuck: bool,
}
struct Native(Rc<RefCell<NativeTrace>>, Rc<RefCell<AuthorityTrace>>);
impl NativePs for Native {
    fn configure(&mut self, c: Config) -> Result<f64, Fault> {
        assert!(self.1.borrow().output_inhibited);
        self.0.borrow_mut().events.push("configure");
        self.0.borrow_mut().measurement_disabled = !c.enabled && !c.automatic;
        if let Some(release) = self.0.borrow_mut().release_on_configure.take() {
            release.release();
        }
        Ok(c.requested_delay_seconds)
    }
    fn measurement_only_guard(&mut self) -> Result<(), Fault> {
        let t = self.0.borrow();
        if t.measurement_disabled && !t.measurement_guard_fault && t.info[14] == 0 {
            Ok(())
        } else {
            Err(Fault::NativeFailure)
        }
    }
    fn mox(&mut self, on: bool) {
        self.0
            .borrow_mut()
            .events
            .push(if on { "mox-on" } else { "mox-off" });
    }
    fn reset(&mut self) -> Result<(), Fault> {
        panic!("blocking reset reached from lifecycle");
    }
    fn process(&mut self, _: &mut [f64; 2048], _: &mut [f64; 2048]) -> Result<(), Fault> {
        let mut t = self.0.borrow_mut();
        t.processed += 1;
        t.info[5] += 1;
        t.info[14] = 1;
        t.info[15] = 6;
        Ok(())
    }
    fn information(&mut self) -> Result<[i32; 16], Fault> {
        Ok(self.0.borrow().info)
    }
}
impl CooperativeNative for Native {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
        self.0.borrow_mut().discard_exchanges += 1;
        Ok(())
    }
    fn begin_cleanup(&mut self) -> Result<(), Fault> {
        assert!(self.1.borrow().output_inhibited);
        assert_eq!(
            self.1.borrow().drive,
            0,
            "old corrected drive must be disabled first"
        );
        assert_eq!(
            self.1.borrow().events.last(),
            Some(&"unkey-complete-simulated")
        );
        self.mox(false);
        let mut t = self.0.borrow_mut();
        t.events.push("begin-cleanup");
        t.remaining = 4;
        Ok(())
    }
    fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault> {
        assert!(
            self.1.borrow().output_inhibited,
            "no uncorrected output can pass during reset"
        );
        let mut t = self.0.borrow_mut();
        t.events.push("one-cleanup-exchange");
        if !t.stuck {
            t.remaining -= 1;
        }
        if t.remaining == 0 {
            t.info[14] = 0;
            t.info[15] = 0;
            Ok(CleanupProgress::Complete)
        } else {
            Ok(CleanupProgress::Pending)
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.0.borrow_mut().dropped = true;
    }
}
type Life = TxLifecycle<Native, SimAuthority>;
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
fn make(backend: Backend) -> (Life, Rc<RefCell<NativeTrace>>, Rc<RefCell<AuthorityTrace>>) {
    let n = Rc::new(RefCell::new(NativeTrace::default()));
    let a = Rc::new(RefCell::new(AuthorityTrace::default()));
    let c = Controller::new_offline_admission_proposal(
        Native(n.clone(), a.clone()),
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
            component_limit: 8388607.0 / 8388608.0,
        },
    )
    .unwrap();
    let l = Life::new(
        c,
        SimAuthority::new(a.clone()),
        backend,
        Limits {
            authority_timeout_ns: 100,
            cleanup_timeout_ns: 100,
        },
    )
    .unwrap();
    (l, n, a)
}
fn off(l: &mut Life, mut now: u64) -> u64 {
    for _ in 0..30 {
        now += 1;
        l.tick(now).unwrap();
        if l.status().phase == Phase::ReadyOff {
            return now;
        }
    }
    panic!("did not quiesce");
}
fn ready(l: &mut Life) -> u64 {
    l.reconfigure(l.fence(), cfg(10), 1).unwrap();
    off(l, 1)
}
#[test]
fn idle_bypass_updates_reuse_completed_cleanup_but_retire_old_authority() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let mut c = cfg(10);
        c.enabled = false;
        c.automatic = false;
        l.reconfigure(l.fence(), c, 1).unwrap();
        let mut now = off(&mut l, 1);
        let stop = l.status().stop_id;
        let cleaned = n
            .borrow()
            .events
            .iter()
            .filter(|e| **e == "begin-cleanup")
            .count();
        let old = l.fence();
        for id in 11..1011 {
            c.id = id;
            now += 1;
            l.reconfigure(l.fence(), c, now).unwrap();
            assert_eq!(l.status().phase, Phase::ReadyOff);
            assert_eq!(l.status().stop_id, stop);
            assert_eq!(l.fence().config, id);
            assert!(a.borrow().output_inhibited);
        }
        assert_ne!(l.fence(), old);
        assert_eq!(
            n.borrow()
                .events
                .iter()
                .filter(|e| **e == "begin-cleanup")
                .count(),
            cleaned
        );
        assert_eq!(l.reconfigure(old, c, now), Err(Fault::GenerationMismatch));
        // A concurrent release is serviced before another idle configuration.
        l.release_handle().release();
        now += 1;
        l.reconfigure(l.fence(), c, now).unwrap();
        assert_ne!(l.status().phase, Phase::ReadyOff);
        off(&mut l, now);
        assert!(l.status().stop_id > stop);
        assert_eq!(l.status().primary_fault, None);
    }
}
#[test]
fn idle_bypass_update_services_release_during_configure_and_preserves_native_faults() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for cancel in [true, false] {
            let (mut l, n, a) = make(backend);
            let mut c = cfg(10);
            c.enabled = false;
            c.automatic = false;
            l.reconfigure(l.fence(), c, 1).unwrap();
            let now = off(&mut l, 1);
            let stop = l.status().stop_id;
            c.id = 11;
            if cancel {
                n.borrow_mut().release_on_configure = Some(l.release_handle());
                l.reconfigure(l.fence(), c, now + 1).unwrap();
                assert_eq!(l.status().primary_fault, None);
            } else {
                n.borrow_mut().info[14] = 1; // quiescence is no longer verified
                assert_eq!(
                    l.reconfigure(l.fence(), c, now + 1),
                    Err(Fault::NativeFailure)
                );
                assert_eq!(l.status().primary_fault, Some(Fault::NativeFailure));
            }
            assert_ne!(l.status().phase, Phase::ReadyOff);
            assert!(l.status().stop_id > stop);
            assert!(a.borrow().output_inhibited);
            off(&mut l, now + 1);
        }
    }
}
fn key(l: &mut Life, sequence: u64, now: u64) -> u64 {
    l.fresh_tx_action(
        FreshTxAction {
            sequence,
            release_ticket: l.status().release_handled,
            expected_fence: l.fence(),
        },
        now + 1,
    )
    .unwrap();
    l.tick(now + 2).unwrap();
    now + 2
}
fn block(l: &Life, now: u64, missing: bool) -> PairBlock {
    let mut b = PairBlock {
        pairs: 1024,
        ..PairBlock::default()
    };
    b.tx.fill(0.3);
    b.rf.fill(if missing { 0. } else { 0.2 });
    b.meta = BlockMeta {
        fence: l.fence(),
        backend: Backend::P2,
        rate_hz: 192000,
        route: 20,
        interval: Interval {
            fence: l.fence(),
            first: 0,
            end: 1024,
        },
        observed_ns: now,
        sequence: Some(0),
        sequence_packets: 1,
        overload: [None; 3],
    };
    b
}

fn measurement_recorder(capacity: usize) -> super::super::measurement::MeasurementRecorder {
    super::super::measurement::MeasurementRecorder::new(
        capacity,
        super::super::measurement::CaptureContext {
            source_pin: "offline-fixture-not-hardware".into(),
            native_identity: "explicit simulated readback".into(),
            feedback_attenuation_db: Some(9),
            commanded_drive: Some(5),
            physical_adc: OverloadSource::Adc1,
        },
    )
    .unwrap()
}
fn measurement_ready(l: &mut Life) -> u64 {
    let mut config = cfg(10);
    config.enabled = false;
    config.automatic = false;
    l.reconfigure(l.fence(), config, 1).unwrap();
    off(l, 1)
}
#[test]
fn explicit_bypass_requires_disabled_native_and_fresh_authority_without_adaptation() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut enabled, _, _) = make(backend);
        let enabled_now = ready(&mut enabled);
        assert_eq!(
            enabled.fresh_bypass_tx_action(
                FreshTxAction {
                    sequence: 1,
                    release_ticket: enabled.status().release_handled,
                    expected_fence: enabled.fence()
                },
                enabled_now
            ),
            Err(Fault::InvalidConfig)
        );
        let (mut l, n, _) = make(backend);
        let now = measurement_ready(&mut l);
        let action = FreshTxAction {
            sequence: 1,
            release_ticket: l.status().release_handled,
            expected_fence: l.fence(),
        };
        assert_eq!(l.fresh_tx_action(action, now), Err(Fault::InvalidConfig));
        n.borrow_mut().measurement_guard_fault = true;
        assert_eq!(
            l.fresh_bypass_tx_action(action, now),
            Err(Fault::NativeFailure)
        );
        n.borrow_mut().measurement_guard_fault = false;
        assert_eq!(
            l.fresh_bypass_tx_action(
                FreshTxAction {
                    expected_fence: Default::default(),
                    ..action
                },
                now
            ),
            Err(Fault::GenerationMismatch)
        );
        l.fresh_bypass_tx_action(action, now).unwrap();
        l.tick(now + 1).unwrap();
        assert_eq!(l.status().phase, Phase::Active);
        l.pump(now + 2).unwrap();
        assert_eq!(n.borrow().processed, 0);
        assert!(!n.borrow().events.contains(&"mox-on"));
        l.release_handle().release();
        off(&mut l, now + 2);
        assert_eq!(l.status().phase, Phase::ReadyOff);
        assert_eq!(l.status().mode, RunMode::Correction);
    }
}
fn measurement_key(l: &mut Life, duration: u64, capacity: usize, now: u64) -> u64 {
    l.fresh_measurement_action(
        FreshTxAction {
            sequence: 1,
            release_ticket: l.status().release_handled,
            expected_fence: l.fence(),
        },
        duration,
        measurement_recorder(capacity),
        now + 1,
    )
    .unwrap();
    l.tick(now + 2).unwrap();
    assert_eq!(l.status().phase, Phase::Active);
    now + 2
}
fn measurement_block(l: &Life, backend: Backend, now: u64) -> PairBlock {
    let mut b = block(l, now, false);
    b.meta.backend = backend;
    b.meta.sequence = (backend == Backend::P2).then_some(0);
    b.meta.sequence_packets = if backend == Backend::P2 { 1 } else { 0 };
    b.arrival_times_recorded = true;
    b.arrival_ns.fill(now);
    b
}
#[test]
fn measurement_only_both_backends_records_raw_unknown_without_fitting_and_expires() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let ready = measurement_ready(&mut l);
        let now = measurement_key(&mut l, 1000, 4, ready);
        let b = measurement_block(&l, backend, now);
        l.submit(&b, now).unwrap();
        assert_eq!(l.pump(now).unwrap(), 1);
        assert_eq!(n.borrow().processed, 0);
        assert!(!n.borrow().events.contains(&"mox-on"));
        assert_eq!(l.status().correction_applied, Some(false));
        let records = l.measurement_records().unwrap().records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].reference, b.tx.to_vec());
        assert_eq!(records[0].rf, b.rf.to_vec());
        assert_eq!(records[0].original_arrival_ns.as_ref().unwrap()[0], now);
        assert_eq!(
            records[0].overload,
            super::super::measurement::OverloadQualification::Unknown
        );
        assert!(l.take_measurement_records().is_err());
        assert_eq!(
            l.fresh_tx_action(
                FreshTxAction {
                    sequence: 2,
                    release_ticket: l.status().release_handled,
                    expected_fence: l.fence()
                },
                now
            ),
            Err(Fault::InvalidConfig)
        );
        let deadline = l.status().measurement_deadline_ns.unwrap();
        l.tick(deadline).unwrap();
        assert_eq!(l.status().phase, Phase::AwaitingAuthority);
        assert!(a.borrow().output_inhibited);
        off(&mut l, deadline);
        assert_eq!(l.status().primary_fault, None);
        assert!(l.take_measurement_records().unwrap().is_some());
    }
}
#[test]
fn measurement_only_both_backends_retains_rejected_feedback_then_completes_stop() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for fault in [
            Fault::Stale,
            Fault::SequenceGap,
            Fault::NonFinite,
            Fault::AdcOverrange,
        ] {
            let (mut l, n, a) = make(backend);
            let ready = measurement_ready(&mut l);
            let now = measurement_key(&mut l, 1_000_000_000, 4, ready);
            let mut b = measurement_block(&l, backend, now);
            if fault == Fault::SequenceGap {
                l.submit(&b, now).unwrap();
                l.pump(now).unwrap();
                b.meta.interval.first = 2048;
                b.meta.interval.end = 3072;
                b.meta.sequence = (backend == Backend::P2).then_some(1);
            }
            if fault == Fault::NonFinite {
                b.rf[0] = f64::NAN;
            }
            if fault == Fault::AdcOverrange {
                b.meta.overload[0] = Some(CounterBracket {
                    source: OverloadSource::Adc1,
                    interval: b.meta.interval,
                    observed_ns: now,
                    counter_generation: 1,
                    end_counter_generation: 1,
                    start_count: 0,
                    end_count: 1,
                    width_bits: Some(32),
                    saturated: Some(false),
                    overflow: Some(false),
                    complete: true,
                    valid: true,
                });
            }
            let consume = if fault == Fault::Stale {
                now + 250_000_001
            } else {
                now
            };
            assert_eq!(l.submit(&b, consume), Err(fault));
            assert_eq!(l.status().primary_fault, Some(fault));
            assert!(a.borrow().output_inhibited);
            assert_eq!(
                l.measurement_records()
                    .unwrap()
                    .records()
                    .back()
                    .unwrap()
                    .admission,
                Err(fault)
            );
            let end = off(&mut l, consume);
            assert_eq!(n.borrow().processed, 0);
            assert!(l.status().cleanup_completed_ns.is_some());
            // Acknowledging the live latch must not erase historical capture
            // faults before evidence extraction.
            l.release_handle().release();
            let end = off(&mut l, end);
            l.acknowledge_fault(l.status().stop_id, end + 1).unwrap();
            assert_eq!(l.status().primary_fault, None);
            let evidence = l.take_measurement_records().unwrap().unwrap().export_json();
            assert!(evidence.contains(&format!("\"terminalFault\":\"{fault:?}\"")));
            assert!(evidence.contains("\"recordingComplete\":false"));
        }
    }
}
#[test]
fn measurement_only_guard_queue_release_missing_feedback_and_config_cancellation() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in ["guard", "queue", "release", "missing", "config"] {
            let (mut l, n, a) = make(backend);
            let ready = measurement_ready(&mut l);
            let mut now = measurement_key(&mut l, 1_000_000_000, 1, ready);
            match case {
                "guard" => {
                    n.borrow_mut().measurement_guard_fault = true;
                    assert_eq!(l.tick(now), Err(Fault::NativeFailure));
                }
                "queue" => {
                    let mut b = measurement_block(&l, backend, now);
                    l.submit(&b, now).unwrap();
                    l.pump(now).unwrap();
                    b.meta.interval.first = 1024;
                    b.meta.interval.end = 2048;
                    b.meta.sequence = (backend == Backend::P2).then_some(1);
                    assert_eq!(l.submit(&b, now), Err(Fault::QueueOverrun));
                    assert!(l.measurement_records().unwrap().overflow_record().is_some());
                }
                "release" => {
                    l.release_handle().release();
                    l.tick(now).unwrap();
                }
                "missing" => {
                    now += 250_000_001;
                    assert_eq!(l.tick(now), Err(Fault::Stale));
                }
                "config" => {
                    assert_eq!(
                        l.reconfigure(l.fence(), cfg(11), now),
                        Err(Fault::ConfigChanged)
                    );
                }
                _ => unreachable!(),
            }
            assert!(a.borrow().output_inhibited);
            off(&mut l, now);
            assert_eq!(n.borrow().processed, 0);
            assert!(!n.borrow().events.contains(&"mox-on"));
            assert!(l.status().cleanup_completed_ns.is_some());
        }
    }
}
#[test]
fn measurement_only_requires_explicit_disabled_config_fresh_action_and_bounded_duration() {
    let (mut l, _, _) = make(Backend::P2);
    let now = measurement_ready(&mut l);
    let action = FreshTxAction {
        sequence: 1,
        release_ticket: l.status().release_handled,
        expected_fence: l.fence(),
    };
    for duration in [0, MAX_MEASUREMENT_DURATION_NS + 1] {
        assert_eq!(
            l.fresh_measurement_action(action, duration, measurement_recorder(4), now),
            Err(Fault::InvalidConfig)
        );
    }
    let stale = FreshTxAction {
        expected_fence: Fence::default(),
        ..action
    };
    assert_eq!(
        l.fresh_measurement_action(stale, 100, measurement_recorder(4), now),
        Err(Fault::GenerationMismatch)
    );
    assert_eq!(l.status().mode, RunMode::Correction);
    assert!(l.measurement_records().is_none());
    l.reconfigure(l.fence(), cfg(11), now).unwrap();
    let now = off(&mut l, now);
    assert_eq!(
        l.fresh_measurement_action(
            FreshTxAction {
                expected_fence: l.fence(),
                ..action
            },
            100,
            measurement_recorder(4),
            now
        ),
        Err(Fault::InvalidConfig)
    );
}
#[test]
fn measurement_only_old_generation_retained_not_rebound_and_future_pair_stops() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, _) = make(backend);
        let ready = measurement_ready(&mut l);
        let now = measurement_key(&mut l, 1_000_000_000, 4, ready);
        let mut old = measurement_block(&l, backend, now);
        old.meta.fence.generation -= 1;
        old.meta.interval.fence = old.meta.fence;
        assert_eq!(l.submit(&old, now), Err(Fault::GenerationMismatch));
        assert_eq!(l.status().phase, Phase::Active);
        let stored = &l.measurement_records().unwrap().records()[0];
        assert_eq!(stored.meta.fence, old.meta.fence);
        assert_eq!(stored.admission, Err(Fault::GenerationMismatch));
        let mut future = measurement_block(&l, backend, now);
        future.arrival_ns[9] = now + 1;
        assert_eq!(l.submit(&future, now), Err(Fault::FutureObservation));
        assert_eq!(l.status().primary_fault, Some(Fault::FutureObservation));
        off(&mut l, now);
        assert_eq!(n.borrow().processed, 0);
    }
}
#[test]
fn release_during_conditional_arm_refusal_is_not_an_authority_fault() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for fault in [Fault::GenerationMismatch, Fault::TxAuthority] {
            let (mut l, _, a) = make(backend);
            let now = ready(&mut l);
            a.borrow_mut().arm_boundary = Some((l.release_handle(), fault));
            let action = FreshTxAction {
                sequence: 1,
                release_ticket: l.status().release_handled,
                expected_fence: l.fence(),
            };
            assert_eq!(l.fresh_tx_action(action, now + 1), Err(fault));
            let now = off(&mut l, now + 1);
            assert!(a.borrow().output_inhibited);
            if fault == Fault::GenerationMismatch {
                assert_eq!(l.status().primary_fault, None);
                key(&mut l, 2, now);
                assert_eq!(l.status().phase, Phase::Active);
            } else {
                assert_eq!(l.status().primary_fault, Some(Fault::TxAuthority));
            }
        }
    }
}
#[test]
fn release_arriving_during_health_check_is_serviced_before_key() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, _, a) = make(backend);
        let now = ready(&mut l);
        l.fresh_tx_action(
            FreshTxAction {
                sequence: 1,
                release_ticket: l.status().release_handled,
                expected_fence: l.fence(),
            },
            now + 1,
        )
        .unwrap();
        a.borrow_mut().health_release = Some(l.release_handle());
        l.tick(now + 2).unwrap();
        assert_eq!(l.status().primary_fault, None);
        assert!(!a.borrow().events.contains(&"key-complete-simulated"));
        off(&mut l, now + 2);
        assert_eq!(l.status().primary_fault, None);
    }
}
#[test]
fn periodic_ticks_stop_keyed_session_without_first_feedback_packet() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        l.tick(now + 250_000_000).unwrap(); // deadline is inclusive
        assert_eq!(l.status().phase, Phase::Active);
        let before = n.borrow().events.len();
        assert_eq!(l.tick(now + 250_000_001), Err(Fault::Stale));
        assert_eq!(l.status().phase, Phase::AwaitingAuthority);
        assert_eq!(l.status().primary_fault, Some(Fault::Stale));
        assert!(a.borrow().output_inhibited);
        assert_eq!(n.borrow().events.len(), before); // no premature native cleanup
        off(&mut l, now + 250_000_001);
        assert_eq!(l.status().primary_fault, Some(Fault::Stale));
    }
}
#[test]
fn ptt_release_precedes_expired_active_watchdog() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, _, a) = make(backend);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        l.release_handle().release();
        l.tick(now + 250_000_001).unwrap();
        assert_ne!(l.status().phase, Phase::Active);
        assert_eq!(l.status().primary_fault, None);
        assert_eq!(a.borrow().release_tickets, [1]);
        off(&mut l, now + 250_000_001);
    }
}
#[test]
fn periodic_ticks_stop_established_correction_when_feedback_ceases() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        let mut b = block(&l, now + 1, false);
        b.meta.backend = backend;
        b.meta.sequence = (backend == Backend::P2).then_some(0);
        b.meta.sequence_packets = if backend == Backend::P2 { 1 } else { 0 };
        l.submit(&b, now + 1).unwrap();
        assert_eq!(l.pump(now + 1), Ok(1));
        assert_eq!(l.status().correction_applied, Some(true));
        a.borrow_mut().drive = 23;
        let exchanges = n.borrow().processed;
        l.tick(now + 250_000_001).unwrap();
        assert_eq!(l.tick(now + 250_000_002), Err(Fault::Stale));
        assert_eq!(l.status().phase, Phase::AwaitingAuthority);
        assert_eq!(l.status().primary_fault, Some(Fault::Stale));
        assert!(l.status().last_known_applied_before_stop);
        assert_eq!(n.borrow().info[14], 1); // curve retained until complete receipt
        assert_eq!(n.borrow().processed, exchanges);
        assert!(a.borrow().output_inhibited);
        off(&mut l, now + 250_000_002);
        assert_eq!(l.status().correction_applied, Some(false));
        assert_eq!(l.status().primary_fault, Some(Fault::Stale));
    }
}
#[test]
fn release_during_fit_and_reset_is_priority_and_never_blocking() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        n.borrow_mut().info[14] = 1;
        n.borrow_mut().info[15] = 6;
        a.borrow_mut().drive = 23;
        a.borrow_mut().receipt_after_polls = 2;
        let before = n.borrow().events.len();
        let handle = l.release_handle();
        handle.release();
        l.tick(now + 1).unwrap();
        assert_eq!(a.borrow().release_tickets, [1]);
        assert!(a.borrow().output_inhibited);
        assert_eq!(n.borrow().events.len(), before);
        assert_eq!(n.borrow().info[14], 1);
        assert_eq!(l.status().correction_applied, None);
        l.tick(now + 2).unwrap();
        l.tick(now + 3).unwrap();
        assert_eq!(l.status().phase, Phase::CleaningNative);
        handle.release();
        l.tick(now + 4).unwrap();
        assert_eq!(a.borrow().release_tickets, [1, 2]);
        assert_eq!(n.borrow().discard_exchanges, 2);
        let end = off(&mut l, now + 4);
        assert_eq!(l.status().correction_applied, Some(false));
        assert_eq!(l.status().primary_fault, None);
        assert!(l
            .fresh_tx_action(
                FreshTxAction {
                    sequence: 1,
                    release_ticket: 2,
                    expected_fence: l.fence()
                },
                end + 1
            )
            .is_err());
        l.shutdown(end + 2).unwrap();
        off(&mut l, end + 2);
        drop(l);
        assert!(n.borrow().dropped);
    }
}
#[test]
fn overdrive_evidence_survives_completed_authority_and_native_cleanup() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut l, n, a) = make(backend);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        n.borrow_mut().info[6] = 2;
        let mut b = block(&l, now + 1, false);
        b.meta.backend = backend;
        if backend == Backend::DirectXdma {
            b.meta.sequence = None;
            b.meta.sequence_packets = 0;
        }
        let original = b.meta.interval;
        l.submit(&b, now + 1).unwrap();
        assert_eq!(l.pump(now + 1), Err(Fault::SevereOverdrive));
        assert!(a.borrow().output_inhibited);
        let end = off(&mut l, now + 1);
        assert!(l.status().cleanup_completed_ns.is_some());
        let evidence = l.feedback_status().overdrive_evidence.unwrap();
        assert_eq!(
            evidence.origin,
            super::super::controller::OverdriveOrigin::NativeStatusFlag2
        );
        assert_eq!(evidence.native_info[6], 2);
        assert_eq!(evidence.interval, original);
        assert_eq!(l.feedback_status().info_raw, [0; 16]);
        #[cfg(feature = "offline-puresignal-owner-adapters")]
        {
            let events = n.borrow().events.clone();
            let old = l.fence();
            l.offline_faulted_tuning(old, old.config + 1, end).unwrap();
            assert_eq!(
                l.feedback_status().overdrive_evidence.unwrap().interval,
                original
            );
            assert_eq!(
                l.feedback_status().overdrive_evidence.unwrap().native_info[6],
                2
            );
            assert_eq!(l.feedback_status().fault, Some(Fault::SevereOverdrive));
            assert_eq!(l.status().primary_fault, Some(Fault::SevereOverdrive));
            assert_eq!(l.status().phase, Phase::ReadyOff);
            assert_eq!(
                n.borrow().events,
                events,
                "no native configuration or processing"
            );
            assert!(a.borrow().output_inhibited);
            assert_eq!(
                l.offline_faulted_tuning(old, old.config + 2, end),
                Err(Fault::CleanupPending)
            );
            l.release_handle().release();
            assert_eq!(
                l.offline_faulted_tuning(l.fence(), old.config + 2, end),
                Err(Fault::CleanupPending)
            );
            off(&mut l, end);
        }
        let end = l.now;
        l.shutdown(end + 1).unwrap();
        off(&mut l, end + 1);
    }
}
#[test]
fn missing_feedback_latches_then_unkeys_before_native_cleanup() {
    let (mut l, n, a) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    let b = block(&l, now + 1, true);
    l.submit(&b, now + 1).unwrap();
    assert_eq!(l.pump(now + 1), Err(Fault::FeedbackLevel));
    assert_eq!(l.status().primary_fault, Some(Fault::FeedbackLevel));
    assert!(a.borrow().output_inhibited);
    assert_eq!(n.borrow().processed, 0);
    off(&mut l, now + 1);
    let end = l.now;
    assert_eq!(
        l.acknowledge_fault(l.status().stop_id, end + 1),
        Err(Fault::CleanupPending)
    );
    l.release_handle().release();
    l.tick(end + 2).unwrap();
    let end = off(&mut l, end + 2);
    l.acknowledge_fault(l.status().stop_id, end + 1).unwrap();
    assert_eq!(l.status().phase, Phase::ReadyOff);
    key(&mut l, 2, end + 1);
    l.shutdown(l.now + 1).unwrap();
    let now = l.now;
    off(&mut l, now);
}
#[test]
fn delayed_overload_during_cleanup_keeps_original_fault_and_no_auto_key() {
    let (mut l, _, _) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    let fence = l.fence();
    l.notify_fault(Fault::FifoLoss, now + 1).unwrap();
    l.tick(now + 2).unwrap();
    assert_eq!(
        l.observe_hazard(
            HazardObservation {
                fence,
                source: OverloadSource::Adc1,
                observed_ns: now,
                valid: true
            },
            now + 3
        ),
        Err(Fault::FifoLoss)
    );
    assert_eq!(l.status().primary_fault, Some(Fault::FifoLoss));
    l.release_handle().release();
    let now = off(&mut l, now + 3);
    l.acknowledge_fault(l.status().stop_id, now + 1).unwrap();
    assert_eq!(l.status().phase, Phase::ReadyOff);
    assert_eq!(l.status().correction_applied, Some(false));
}
#[test]
fn reconfiguration_during_fit_waits_for_unkey_and_fresh_action() {
    let (mut l, n, a) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    n.borrow_mut().info[15] = 6;
    a.borrow_mut().receipt_after_polls = 2;
    let before = n
        .borrow()
        .events
        .iter()
        .filter(|e| **e == "configure")
        .count();
    l.reconfigure(l.fence(), cfg(11), now + 1).unwrap();
    l.tick(now + 2).unwrap();
    assert_eq!(
        n.borrow()
            .events
            .iter()
            .filter(|e| **e == "configure")
            .count(),
        before
    );
    let end = off(&mut l, now + 2);
    assert_eq!(l.fence().config, 11);
    assert!(l
        .fresh_tx_action(
            FreshTxAction {
                sequence: 2,
                release_ticket: 0,
                expected_fence: l.fence()
            },
            end + 1
        )
        .is_err());
    l.release_handle().release();
    let end = off(&mut l, end + 1);
    key(&mut l, 2, end);
    l.shutdown(l.now + 1).unwrap();
    let now = l.now;
    off(&mut l, now);
}
#[test]
fn release_cancels_staged_configuration_before_native_install() {
    let (mut l, n, _) = make(Backend::P2);
    let now = ready(&mut l);
    let before = n
        .borrow()
        .events
        .iter()
        .filter(|e| **e == "configure")
        .count();
    l.reconfigure(l.fence(), cfg(11), now + 1).unwrap();
    l.tick(now + 2).unwrap();
    l.release_handle().release();
    off(&mut l, now + 2);
    assert_eq!(l.fence().config, 10);
    assert_eq!(
        n.borrow()
            .events
            .iter()
            .filter(|e| **e == "configure")
            .count(),
        before
    );
    l.shutdown(l.now + 1).unwrap();
    off(&mut l, now + 10);
}
#[test]
fn stale_receipt_does_not_start_cleanup_and_bad_receipt_blocks() {
    for invalid in [false, true] {
        let (mut l, n, a) = make(Backend::P2);
        a.borrow_mut().stale_receipt_once = true;
        l.reconfigure(l.fence(), cfg(10), 1).unwrap();
        l.tick(2).unwrap();
        assert!(!n.borrow().events.contains(&"begin-cleanup"));
        if invalid {
            a.borrow_mut().invalid_receipt = true;
            assert_eq!(l.tick(3), Err(Fault::TxAuthority));
            assert_eq!(l.status().phase, Phase::Blocked);
            l.release_handle().release();
            l.tick(4).unwrap();
            assert_eq!(a.borrow().release_tickets, [1]);
            drop(l);
            assert!(!n.borrow().dropped); // fail-closed retention, not unsafe destructor
        } else {
            off(&mut l, 2);
        }
    }
}
#[test]
fn timeouts_keep_rf_fenced_and_release_path_live_without_freeing_native() {
    for native_stall in [false, true] {
        let (mut l, n, a) = make(Backend::P2);
        let now = ready(&mut l);
        let now = key(&mut l, 1, now);
        if native_stall {
            n.borrow_mut().stuck = true;
        } else {
            a.borrow_mut().stall = true;
        }
        l.notify_fault(Fault::DdcClamp, now + 1).unwrap();
        l.tick(now + 2).unwrap();
        assert!(l.tick(now + 200).is_err());
        assert_eq!(l.status().primary_fault, Some(Fault::DdcClamp));
        assert_eq!(l.status().phase, Phase::Blocked);
        assert!(a.borrow().output_inhibited);
        l.release_handle().release();
        l.tick(now + 201).unwrap();
        assert_eq!(a.borrow().release_tickets, [1]);
        drop(l);
        assert!(!n.borrow().dropped);
    }
}
#[test]
fn release_cancels_pending_key_and_key_error_requests_stop() {
    for error in [false, true] {
        let (mut l, _, a) = make(Backend::P2);
        let now = ready(&mut l);
        l.fresh_tx_action(
            FreshTxAction {
                sequence: 1,
                release_ticket: 0,
                expected_fence: l.fence(),
            },
            now + 1,
        )
        .unwrap();
        if error {
            a.borrow_mut().key_error = true;
            assert_eq!(l.tick(now + 2), Err(Fault::TxAuthority));
        } else {
            l.release_handle().release();
            l.tick(now + 2).unwrap();
        }
        assert!(a.borrow().output_inhibited);
        assert_ne!(l.status().phase, Phase::Active);
        off(&mut l, now + 2);
    }
}
#[test]
fn applied_correction_and_unclipped_confidence_are_independent() {
    let (mut l, _, _) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    l.submit(&block(&l, now + 1, false), now + 1).unwrap();
    l.pump(now + 1).unwrap();
    assert_eq!(l.status().correction_applied, Some(true));
    assert_eq!(
        l.raw_unclipped_qualification(),
        Some(Err(Fault::ClipStatusUnknown))
    );
    l.shutdown(now + 2).unwrap();
    off(&mut l, now + 2);
}
#[test]
fn shutdown_is_terminal_and_clock_regression_is_fail_closed() {
    let (mut l, _, a) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    l.release_handle().release();
    assert_eq!(l.tick(now - 1), Err(Fault::FutureObservation));
    assert_eq!(a.borrow().release_tickets, [1]);
    assert!(a.borrow().output_inhibited);
    let end = off(&mut l, now);
    l.shutdown(end + 1).unwrap();
    let end = off(&mut l, end + 1);
    assert_eq!(
        l.fresh_tx_action(
            FreshTxAction {
                sequence: 2,
                release_ticket: 1,
                expected_fence: l.fence()
            },
            end + 1
        ),
        Err(Fault::CleanupPending)
    );
}
#[test]
fn feedback_backlog_yields_to_release_after_one_native_exchange() {
    let (mut l, n, _) = make(Backend::P2);
    let now = ready(&mut l);
    let now = key(&mut l, 1, now);
    let first = block(&l, now + 1, false);
    let mut second = block(&l, now + 1, false);
    second.meta.interval.first = 1024;
    second.meta.interval.end = 2048;
    second.meta.sequence = Some(1);
    l.submit(&first, now + 1).unwrap();
    l.submit(&second, now + 1).unwrap();
    assert_eq!(l.pump(now + 1), Ok(1));
    assert_eq!(n.borrow().processed, 1);
    assert_eq!(l.feedback_status().queue_depth, 1);
    l.release_handle().release();
    assert_eq!(l.pump(now + 2), Err(Fault::CleanupPending));
    assert_eq!(n.borrow().processed, 1);
    assert_eq!(l.feedback_status().queue_depth, 0);
    off(&mut l, now + 2);
}
