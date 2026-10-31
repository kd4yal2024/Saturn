use super::{adapters::*, controller::*, *};
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "offline-puresignal-policy-proposal")]
mod proposal_tests {
    use super::*;
    use crate::puresignal::admission_proposal::*;
    fn limits() -> DraftLimits {
        DraftLimits {
            active_reference_fraction: 0.1,
            feedback_rms_floor: 1e-6,
            component_limit: 8388607.0 / 8388608.0,
        }
    }
    fn draft(backend: Backend) -> (Controller<FakeNative>, Rc<RefCell<Trace>>) {
        let t = Rc::new(RefCell::new(Trace::default()));
        let mut c = Controller::new_offline_admission_proposal(
            FakeNative(t.clone()),
            Owner::Bridge,
            backend,
            77,
            Qualification {
                native_abi_verified: true,
                feedback_topology_verified: true,
            },
            4,
            limits(),
        )
        .unwrap();
        c.configure(c.fence(), config()).unwrap();
        c.on_tx_state(1, true, 0).unwrap();
        (c, t)
    }
    #[test]
    fn default_gate_is_still_strict_with_feature_compiled() {
        let (mut c, _) = setup(Backend::P2, 4);
        assert!(!c.uses_offline_admission_proposal());
        let mut b = block(&c, 0, 1024, 0, 1);
        b.meta.overload = [None; 3];
        assert_eq!(c.submit(&b, 1), Err(Fault::ClipStatusUnknown));
    }
    #[test]
    fn draft_missing_and_uncertain_evidence_never_certifies_capture() {
        for backend in [Backend::P2, Backend::DirectXdma] {
            let (mut c, t) = draft(backend);
            let mut b = block(&c, 0, 1024, 0, 1);
            b.meta.overload = [None; 3];
            c.submit(&b, 1).unwrap();
            c.pump(1).unwrap();
            assert_eq!(
                c.raw_unclipped_qualification(),
                Some(Err(Fault::ClipStatusUnknown))
            );
            b = block(&c, 1024, 1024, 1, 2);
            b.meta.overload[0].as_mut().unwrap().saturated = Some(true);
            c.submit(&b, 2).unwrap();
            c.pump(2).unwrap();
            assert_eq!(
                c.raw_unclipped_qualification(),
                Some(Err(Fault::CounterInexact))
            );
            b = block(&c, 2048, 1024, 2, 3);
            c.submit(&b, 3).unwrap();
            c.pump(3).unwrap();
            assert_eq!(c.raw_unclipped_qualification(), Some(Ok(())));
            assert_eq!(t.borrow().processed, 3);
        }
    }
    #[test]
    fn any_unqualified_fragment_invalidates_entire_measurement_window() {
        let (mut c, _) = draft(Backend::P2);
        let mut b = block(&c, 0, 512, 0, 1);
        b.meta.overload = [None; 3];
        c.submit(&b, 1).unwrap();
        c.pump(1).unwrap();
        c.submit(&block(&c, 512, 512, 1, 2), 2).unwrap();
        c.pump(2).unwrap();
        assert_eq!(
            c.raw_unclipped_qualification(),
            Some(Err(Fault::ClipStatusUnknown))
        );
    }
    #[test]
    fn wide_known_hit_stops_even_without_capture_coverage() {
        let (mut c, t) = draft(Backend::DirectXdma);
        let mut b = block(&c, 0, 4, 0, 1);
        let hit = b.meta.overload[1].as_mut().unwrap();
        hit.interval.first = 200;
        hit.interval.end = 300;
        hit.complete = false;
        hit.end_count += 1;
        assert_eq!(c.submit(&b, 1), Err(Fault::DdcClamp));
        assert_eq!(t.borrow().processed, 0);
        assert_eq!(c.status().state, State::Fault);
    }
    #[test]
    fn delayed_hazard_resets_but_old_generation_and_invalid_events_do_not() {
        let (mut c, t) = draft(Backend::P2);
        let mut e = HazardObservation {
            fence: c.fence(),
            source: OverloadSource::Adc1,
            observed_ns: 1,
            valid: false,
        };
        assert_eq!(
            c.observe_hazard(e, 300_000_000),
            Err(Fault::ClipStatusUnknown)
        );
        e.valid = true;
        e.fence.generation += 1;
        assert_eq!(
            c.observe_hazard(e, 300_000_000),
            Err(Fault::GenerationMismatch)
        );
        assert!(c.status().fault.is_none());
        e.fence = c.fence();
        assert_eq!(c.observe_hazard(e, 300_000_000), Err(Fault::AdcOverrange));
        assert_eq!(t.borrow().events.last(), Some(&"reset"));
        c.on_tx_state(2, false, 300_000_001).unwrap();
        c.on_tx_state(3, true, 300_000_002).unwrap();
        assert_eq!(c.status().fault, Some(Fault::AdcOverrange));
    }
    #[test]
    fn absent_rf_with_active_reference_and_rail_hit_reset_before_native_submission() {
        for fault in [Fault::FeedbackLevel, Fault::SevereOverdrive] {
            let (mut c, t) = draft(Backend::DirectXdma);
            let mut b = block(&c, 0, 1024, 0, 1);
            b.meta.overload = [None; 3];
            if fault == Fault::FeedbackLevel {
                b.rf.fill(0.0);
            } else {
                b.rf[0] = limits().component_limit;
            }
            c.submit(&b, 1).unwrap();
            assert_eq!(c.pump(1), Err(fault));
            assert_eq!(t.borrow().processed, 0);
            assert!(c.raw_capture().is_none());
        }
    }
    #[test]
    fn no_excitation_does_not_mean_transport_stale_and_bad_limits_are_rejected() {
        let (mut c, t) = draft(Backend::P2);
        let mut b = block(&c, 0, 1024, 0, 1);
        b.meta.overload = [None; 3];
        b.tx.fill(0.0);
        b.rf.fill(0.0);
        c.submit(&b, 1).unwrap();
        c.pump(1).unwrap();
        assert_eq!(t.borrow().processed, 1);
        assert!(c.status().fault.is_none());
        assert_eq!(c.pump(250_000_002), Err(Fault::Stale));
        let mut invalid = limits();
        invalid.feedback_rms_floor = f64::NAN;
        assert_eq!(invalid.validate(), Err(Fault::InvalidConfig));
    }
    #[test]
    fn draft_still_enforces_continuity_and_sample_observation_time() {
        let (mut c, _) = draft(Backend::P2);
        let mut b = block(&c, 0, 4, 0, 1);
        b.meta.overload = [None; 3];
        c.submit(&b, 1).unwrap();
        c.pump(1).unwrap();
        b = block(&c, 4, 4, 2, 2);
        b.meta.overload = [None; 3];
        assert_eq!(c.submit(&b, 2), Err(Fault::SequenceGap));
        let (mut c, _) = draft(Backend::DirectXdma);
        let mut b = block(&c, 0, 4, 0, 1);
        b.meta.overload = [None; 3];
        assert_eq!(c.submit(&b, 250_000_002), Err(Fault::Stale));
    }
}

#[derive(Default)]
struct Trace {
    events: Vec<&'static str>,
    processed: usize,
    info: [i32; 16],
    fail_reset: bool,
}
struct FakeNative(Rc<RefCell<Trace>>);
impl NativePs for FakeNative {
    fn configure(&mut self, _: Config) -> Result<f64, Fault> {
        self.0.borrow_mut().events.push("configure");
        Ok(140.224e-9)
    }
    fn mox(&mut self, active: bool) {
        self.0
            .borrow_mut()
            .events
            .push(if active { "mox-on" } else { "mox-off" });
    }
    fn reset(&mut self) -> Result<(), Fault> {
        self.0.borrow_mut().events.push("reset");
        self.0.borrow_mut().info[14] = 0;
        if self.0.borrow().fail_reset {
            Err(Fault::NativeFailure)
        } else {
            Ok(())
        }
    }
    fn process(&mut self, tx: &mut [f64; 2048], rf: &mut [f64; 2048]) -> Result<(), Fault> {
        let mut trace = self.0.borrow_mut();
        trace.processed += 1;
        trace.events.push("process");
        assert!(tx.iter().chain(rf.iter()).all(|x| x.is_finite()));
        rf.fill(99.0);
        tx.fill(88.0);
        Ok(()) // emulate native delay mutation
    }
    fn information(&mut self) -> Result<[i32; 16], Fault> {
        Ok(self.0.borrow().info)
    }
}
fn config() -> Config {
    Config {
        id: 10,
        route: 20,
        physical_adc: OverloadSource::Adc1,
        enabled: true,
        automatic: true,
        hardware_peak: 0.6121,
        requested_delay_seconds: 150e-9,
        settling_ns: 0,
        loop_delay_seconds: 0.0,
        stale_after_ns: 250_000_000,
    }
}
fn setup(backend: Backend, capacity: usize) -> (Controller<FakeNative>, Rc<RefCell<Trace>>) {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut c = Controller::new(
        FakeNative(trace.clone()),
        Owner::Bridge,
        backend,
        77,
        Qualification {
            native_abi_verified: true,
            feedback_topology_verified: true,
        },
        capacity,
    );
    c.configure(c.fence(), config()).unwrap();
    c.on_tx_state(1, true, 0).unwrap();
    (c, trace)
}
fn brackets(interval: Interval, now: u64) -> [Option<CounterBracket>; 3] {
    [
        OverloadSource::Adc1,
        OverloadSource::Ddc0Clamp,
        OverloadSource::Ddc1Clamp,
    ]
    .map(|source| {
        Some(CounterBracket {
            source,
            interval,
            observed_ns: now,
            counter_generation: 3,
            end_counter_generation: 3,
            start_count: 7,
            end_count: 7,
            width_bits: Some(64),
            saturated: Some(false),
            overflow: Some(false),
            complete: true,
            valid: true,
        })
    })
}
fn block(c: &Controller<FakeNative>, first: u64, pairs: usize, seq: u32, now: u64) -> PairBlock {
    let mut b = PairBlock::default();
    b.pairs = pairs;
    let interval = Interval {
        fence: c.fence(),
        first,
        end: first + pairs as u64,
    };
    b.meta = BlockMeta {
        fence: c.fence(),
        backend: c.test_backend(),
        rate_hz: RATE_HZ,
        route: config().route,
        interval,
        observed_ns: now,
        overload: brackets(interval, now),
        sequence_packets: if c.test_backend() == Backend::P2 { 1 } else { 0 },
        sequence: if c.test_backend() == Backend::P2 {
            Some(seq)
        } else {
            None
        },
    };
    b.tx[..pairs * 2].fill(0.25);
    b.rf[..pairs * 2].fill(0.125);
    b
}
#[test]
fn p2_batched_wire_sequence_span_preserves_gap_detection_and_wrap() {
    for first_sequence in [0, u32::MAX - 3] {
        let (mut c, _) = setup(Backend::P2, 4);
        let mut first = block(&c, 0, 952, first_sequence, 1);
        first.meta.sequence_packets = 8;
        c.submit(&first, 1).unwrap();
        let mut second = block(&c, 952, 952, first_sequence.wrapping_add(8), 2);
        second.meta.sequence_packets = 8;
        c.submit(&second, 2).unwrap();
        let mut gap = block(&c, 1904, 952, first_sequence.wrapping_add(17), 3);
        gap.meta.sequence_packets = 8;
        assert_eq!(c.submit(&gap, 3), Err(Fault::SequenceGap));
    }
}
#[test]
fn p2_batch_geometry_and_oldest_observation_are_mandatory() {
    for (packets, pairs, fault) in [(0, 952, Fault::WrongFormat), (9, 952, Fault::WrongFormat), (8, 951, Fault::WrongFormat)] {
        let (mut c, _) = setup(Backend::P2, 4);
        let mut b = block(&c, 0, pairs, 0, 1); b.meta.sequence_packets = packets;
        assert_eq!(c.submit(&b, 1), Err(fault));
    }
    let (mut c, _) = setup(Backend::P2, 4);
    let mut b = block(&c, 0, 952, 0, 1); b.meta.sequence_packets = 8;
    assert_eq!(c.submit(&b, 300_000_001), Err(Fault::Stale));
}
#[test]
fn unverified_or_external_owner_never_calls_native() {
    for owner in [Owner::Bridge, Owner::ExternalP2Client, Owner::None] {
        let t = Rc::new(RefCell::new(Trace::default()));
        {
            let mut c = Controller::new(
                FakeNative(t.clone()),
                owner,
                Backend::DirectXdma,
                1,
                Qualification::UNVERIFIED,
                2,
            );
            assert!(c.configure(c.fence(), config()).is_err());
            c.on_tx_state(1, true, 0).unwrap();
        }
        assert!(t.borrow().events.is_empty());
    }
}
#[test]
fn configuration_never_keys_and_actual_delay_comes_from_engine() {
    let (mut c, t) = setup(Backend::P2, 4);
    t.borrow_mut().events.clear();
    let mut cfg = config();
    cfg.id = 11;
    cfg.route = 21;
    c.configure(c.fence(), cfg).unwrap();
    assert!(!t.borrow().events.contains(&"mox-on"));
    assert_eq!(c.status().state, State::Waiting);
    assert_eq!(c.status().actual_delay_seconds, Some(140.224e-9));
    let mut new_block = block(&c, 0, 100, 1, 1);
    new_block.meta.route = cfg.route;
    assert_eq!(c.submit(&new_block, 1), Err(Fault::NotKeyed));
}
#[test]
fn failed_native_quiescence_retires_generation_and_refuses_new_configuration() {
    let (mut c, t) = setup(Backend::P2, 2);
    let old = c.fence();
    t.borrow_mut().fail_reset = true;
    assert_eq!(c.configure(old, config()), Err(Fault::NativeFailure));
    assert_ne!(c.fence(), old);
    assert_eq!(c.status().state, State::Fault);
    assert_eq!(c.status().queue_depth, 0);
}
#[test]
fn old_reported_applied_curve_is_not_a_new_generation_calibration() {
    let (mut c, t) = setup(Backend::P2, 2);
    t.borrow_mut().info[14] = 1;
    c.submit(&block(&c, 0, 1024, 1, 1), 1).unwrap();
    c.pump(1).unwrap();
    assert!(!c.status().correction_applied_reported);
    t.borrow_mut().info[5] = 1;
    c.submit(&block(&c, 1024, 1024, 2, 2), 2).unwrap();
    c.pump(2).unwrap();
    assert!(c.status().correction_applied_reported);
}
#[test]
fn verified_library_does_not_allow_external_owner_to_touch_native() {
    let t = Rc::new(RefCell::new(Trace::default()));
    {
        let mut c = Controller::new(
            FakeNative(t.clone()),
            Owner::ExternalP2Client,
            Backend::P2,
            1,
            Qualification {
                native_abi_verified: true,
                feedback_topology_verified: true,
            },
            2,
        );
        c.on_tx_state(1, true, 0).unwrap();
        c.latch_fault(Fault::FifoLoss);
    }
    assert!(t.borrow().events.is_empty());
}
#[test]
fn both_backends_feed_exact_native_blocks_and_preserve_raw_before_mutation() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut c, t) = setup(backend, 4);
        c.submit(&block(&c, 0, 600, 1, 1), 1).unwrap();
        c.submit(&block(&c, 600, 424, 2, 2), 2).unwrap();
        assert_eq!(c.pump(2), Ok(1));
        assert_eq!(t.borrow().processed, 1);
        let (interval, observed, raw) = c.raw_capture().unwrap();
        assert_eq!((interval.first, interval.end, observed), (0, 1024, 1));
        assert!(raw.iter().all(|x| *x == 0.125));
    }
}
#[test]
fn queued_and_partial_samples_cannot_look_fresh_on_consumption() {
    let (mut c, t) = setup(Backend::P2, 2);
    c.submit(&block(&c, 0, 1024, 1, 1), 1).unwrap();
    assert_eq!(c.pump(300_000_001), Err(Fault::Stale));
    assert_eq!(t.borrow().processed, 0);
    let (mut c, t) = setup(Backend::P2, 2);
    c.submit(&block(&c, 0, 600, 1, 1), 1).unwrap();
    c.pump(1).unwrap();
    c.submit(&block(&c, 600, 424, 2, 260_000_000), 260_000_000)
        .unwrap();
    assert_eq!(c.pump(260_000_000), Err(Fault::Stale));
    assert_eq!(t.borrow().processed, 0);
}
#[test]
fn unkey_rekey_and_config_fence_old_callbacks_and_clear_queues() {
    let (mut c, t) = setup(Backend::P2, 2);
    let old = block(&c, 0, 1024, 1, 1);
    c.submit(&old, 1).unwrap();
    c.on_tx_state(2, false, 2).unwrap();
    c.on_tx_state(3, true, 3).unwrap();
    assert_eq!(c.submit(&old, 3), Err(Fault::GenerationMismatch));
    assert_eq!(c.status().fault, None);
    c.pump(3).unwrap();
    assert_eq!(t.borrow().processed, 0);
    let expected = c.fence();
    let mut cfg = config();
    cfg.id = 11;
    c.configure(expected, cfg).unwrap();
    assert_eq!(
        c.configure(expected, config()),
        Err(Fault::GenerationMismatch)
    );
}
#[test]
fn queue_overrun_latches_and_fresh_packet_does_not_restart() {
    let (mut c, t) = setup(Backend::P2, 1);
    c.submit(&block(&c, 0, 100, 1, 1), 1).unwrap();
    assert_eq!(
        c.submit(&block(&c, 100, 100, 2, 2), 2),
        Err(Fault::QueueOverrun)
    );
    assert_eq!(
        c.submit(&block(&c, 200, 100, 3, 3), 3),
        Err(Fault::QueueOverrun)
    );
    assert_eq!(t.borrow().processed, 0);
    assert!(!c.status().correction_applied_reported);
}
#[test]
fn malformed_length_nonfinite_rate_route_future_data_are_never_trimmed_or_fed() {
    for fault in [
        Fault::UnequalLength,
        Fault::NonFinite,
        Fault::WrongRate,
        Fault::WrongRoute,
        Fault::FutureObservation,
    ] {
        let (mut c, t) = setup(Backend::P2, 2);
        let mut b = block(&c, 0, 100, 1, 1);
        match fault {
            Fault::UnequalLength => b.meta.interval.end -= 1,
            Fault::NonFinite => b.tx[0] = f64::NAN,
            Fault::WrongRate => b.meta.rate_hz = 384000,
            Fault::WrongRoute => b.meta.route = 90,
            Fault::FutureObservation => b.meta.observed_ns = 10,
            _ => unreachable!(),
        }
        assert_eq!(c.submit(&b, 1), Err(fault));
        assert_eq!(t.borrow().processed, 0);
    }
}
#[test]
fn old_clear_snapshot_cannot_certify_a_newer_capture() {
    let (mut c, _) = setup(Backend::P2, 2);
    let mut b = block(&c, 100, 100, 1, 1);
    b.meta.overload = brackets(
        Interval {
            fence: c.fence(),
            first: 0,
            end: 100,
        },
        1,
    );
    assert_eq!(c.submit(&b, 1), Err(Fault::ObservationIncomplete));
}
#[test]
fn overload_source_generation_coverage_reset_unknown_and_hit_are_distinct() {
    for fault in [
        Fault::ObservationIncomplete,
        Fault::CounterReset,
        Fault::AdcOverrange,
        Fault::DdcClamp,
        Fault::ClipStatusUnknown,
        Fault::WrongRoute,
    ] {
        let (mut c, _) = setup(Backend::P2, 2);
        let mut b = block(&c, 0, 100, 1, 1);
        match fault {
            Fault::ObservationIncomplete => {
                b.meta.overload[0]
                    .as_mut()
                    .unwrap()
                    .interval
                    .fence
                    .generation += 1
            }
            Fault::CounterReset => b.meta.overload[0].as_mut().unwrap().end_counter_generation += 1,
            Fault::AdcOverrange => b.meta.overload[0].as_mut().unwrap().end_count += 1,
            Fault::DdcClamp => b.meta.overload[1].as_mut().unwrap().end_count += 1,
            Fault::ClipStatusUnknown => b.meta.overload[0] = None,
            Fault::WrongRoute => b.meta.overload[0].as_mut().unwrap().source = OverloadSource::Adc2,
            _ => unreachable!(),
        }
        assert_eq!(c.submit(&b, 1), Err(fault));
    }
}
#[test]
fn wider_clear_counter_bracket_is_conservative_and_hits_do_not_locate_overload() {
    let capture = Interval {
        fence: Fence::default(),
        first: 100,
        end: 200,
    };
    let mut b = brackets(
        Interval {
            first: 0,
            end: 500,
            ..capture
        },
        1,
    )[0]
    .unwrap();
    assert_eq!(b.certify(capture, 1), Ok(()));
    b.end_count += 1;
    assert_eq!(b.certify(capture, 1), Err(Fault::AdcOverrange));
    b.end_count = b.start_count - 1;
    assert_eq!(b.certify(capture, 1), Err(Fault::CounterReset));
    b.end_count = b.start_count;
    b.complete = false;
    assert_eq!(b.certify(capture, 1), Err(Fault::ObservationIncomplete));
}
#[test]
fn severe_overdrive_bitmask_fault_is_not_cleared_by_unkey_rekey() {
    for code in [2, 3, 10] {
        let (mut c, t) = setup(Backend::P2, 2);
        t.borrow_mut().info[6] = code;
        t.borrow_mut().info[14] = 1;
        c.submit(&block(&c, 0, 1024, 1, 1), 1).unwrap();
        assert_eq!(c.pump(1), Err(Fault::SevereOverdrive));
        assert!(c.raw_capture().is_none());
        assert!(!c.status().correction_applied_reported);
        c.on_tx_state(2, false, 2).unwrap();
        c.on_tx_state(3, true, 3).unwrap();
        assert_eq!(c.status().state, State::Fault);
    }
}
#[test]
fn settling_and_no_feedback_timeout_do_not_feed() {
    let (mut c, t) = setup(Backend::P2, 2);
    let mut cfg = config();
    cfg.settling_ns = 10;
    c.configure(c.fence(), cfg).unwrap();
    c.on_tx_state(2, true, 10).unwrap();
    assert_eq!(
        c.submit(&block(&c, 0, 100, 1, 11), 11),
        Err(Fault::Settling)
    );
    assert_eq!(c.status().fault, None);
    assert_eq!(c.pump(300_000_000), Err(Fault::Stale));
    assert_eq!(t.borrow().processed, 0);
}
#[test]
fn drop_clears_mox_before_reset_and_native_destruction() {
    let (c, t) = setup(Backend::P2, 2);
    t.borrow_mut().events.clear();
    drop(c);
    assert_eq!(t.borrow().events, ["mox-off", "reset"]);
}
fn packet(seq: u32) -> [u8; 1444] {
    let mut p = [0; 1444];
    p[..4].copy_from_slice(&seq.to_be_bytes());
    p[12..14].copy_from_slice(&24u16.to_be_bytes());
    p[14..16].copy_from_slice(&238u16.to_be_bytes());
    for pair in p[16..].chunks_exact_mut(12) {
        pair.copy_from_slice(&[0x80, 0, 0, 0x7f, 0xff, 0xff, 0x40, 0, 0, 0xc0, 0, 0]);
    }
    p
}
#[test]
fn p2_signed_scaling_pair_order_and_wrap() {
    let mut a = P2Adapter::new(Fence::default(), 20);
    let mut out = PairBlock::default();
    for seq in [u32::MAX, 0, 1] {
        a.decode(0, &packet(seq), 1, [None; 3], &mut out).unwrap();
        assert_eq!(out.pairs, 119);
        assert_eq!((out.rf[0], out.rf[1]), (-1.0, 1.0 - 1.0 / 8388608.0));
        assert_eq!((out.tx[0], out.tx[1]), (0.5, -0.5));
    }
    assert_eq!(out.meta.interval.first, 238);
}
#[test]
fn p2_gap_duplicate_reorder_poison_until_new_generation() {
    for (seq, fault) in [
        (10, Fault::Duplicate),
        (9, Fault::Reordered),
        (12, Fault::SequenceGap),
    ] {
        let mut a = P2Adapter::new(Fence::default(), 20);
        let mut out = PairBlock::default();
        a.decode(0, &packet(10), 1, [None; 3], &mut out).unwrap();
        assert_eq!(
            a.decode(0, &packet(seq), 2, [None; 3], &mut out),
            Err(fault)
        );
        assert_eq!(out.pairs, 0);
        assert_eq!(
            a.decode(0, &packet(11), 3, [None; 3], &mut out),
            Err(Fault::GenerationMismatch)
        );
    }
}
#[test]
fn p2_wrong_format_and_one_sided_packets_not_trimmed() {
    for variant in 0..4 {
        let mut p = packet(1);
        match variant {
            0 => p[13] = 16,
            1 => p[15] = 237,
            2 => p[15] = 0,
            _ => {}
        }
        let bytes: &[u8] = if variant == 3 { &p[..1443] } else { &p };
        assert_eq!(
            P2Adapter::new(Fence::default(), 20).decode(
                0,
                bytes,
                1,
                [None; 3],
                &mut PairBlock::default()
            ),
            Err(Fault::WrongFormat)
        );
    }
}
fn dma_frame(retain: bool) -> Vec<[u8; 8]> {
    let p = packet(1);
    let mut header = [0; 8];
    header[..4].copy_from_slice(&FeedbackPlan::adc1(retain).rate_word.to_le_bytes());
    header[7] = 0x80;
    let mut words = vec![header];
    for iq in p[16..64].chunks_exact(6) {
        let mut w = [0; 8];
        w[..6].copy_from_slice(iq);
        words.push(w);
    }
    if retain {
        words.extend([[0x20, 0, 0, 0, 0, 0, 0, 0]; 8]);
    }
    words
}
#[test]
fn direct_demux_retains_ddc6_with_bounded_split_read_state() {
    for retain in [false, true] {
        let mut a = DirectXdmaAdapter::new(Fence::default(), 20, retain);
        let mut out = PairBlock::default();
        let mut rx = [0.0; 16];
        let words = dma_frame(retain);
        for (i, w) in words.iter().enumerate() {
            assert_eq!(
                a.push_word(w, 1, [None; 3], &mut out, &mut rx),
                Ok(i + 1 == words.len())
            );
        }
        assert_eq!(out.pairs, 4);
        assert_eq!((out.rf[0], out.tx[0]), (-1.0, 0.5));
        assert_eq!(out.meta.sequence, None);
        assert_eq!(rx[0], if retain { 0.25 } else { 0.0 });
        assert_eq!(FeedbackPlan::adc1(retain).reference_register_source, 3);
        assert!(!FeedbackPlan::adc1(retain).independent_lo_debug);
    }
}
#[test]
fn direct_framing_loss_or_configuration_switch_requires_new_adapter() {
    let mut out = PairBlock::default();
    let mut rx = [0.0; 16];
    let words = dma_frame(false);
    assert_eq!(
        DirectXdmaAdapter::new(Fence::default(), 20, true)
            .push_word(&words[0], 1, [None; 3], &mut out, &mut rx),
        Err(Fault::DmaFraming)
    );
    let mut a = DirectXdmaAdapter::new(Fence::default(), 20, false);
    a.push_word(&words[0], 1, [None; 3], &mut out, &mut rx)
        .unwrap();
    a.mark_loss();
    assert_eq!(
        a.push_word(&words[1], 1, [None; 3], &mut out, &mut rx),
        Err(Fault::GenerationMismatch)
    );
    assert_eq!(out.pairs, 0);
}

#[test]
fn partial_dma_completion_preserves_oldest_arrival_and_fails_freshness() {
    for retain in [false, true] {
        let (mut c, t) = setup(Backend::DirectXdma, 2);
        let mut a = DirectXdmaAdapter::new(c.fence(), 20, retain);
        let mut out = PairBlock::default();
        let mut rx = [0.0; 16];
        let words = dma_frame(retain);
        for w in &words[..words.len() - 1] {
            assert_eq!(a.push_word(w, 1, [None; 3], &mut out, &mut rx), Ok(false));
        }
        let completion = 300_000_002;
        let interval = Interval {
            fence: c.fence(),
            first: 0,
            end: 4,
        };
        assert_eq!(
            a.push_word(
                words.last().unwrap(),
                completion,
                brackets(interval, completion),
                &mut out,
                &mut rx
            ),
            Ok(true)
        );
        assert_eq!(out.meta.observed_ns, 1);
        assert_eq!(c.submit(&out, completion), Err(Fault::Stale));
        assert_eq!(t.borrow().processed, 0);
        // A new complete frame does not inherit the retired partial timestamp.
        for w in &words {
            a.push_word(w, completion + 1, [None; 3], &mut out, &mut rx)
                .unwrap();
        }
        assert_eq!(out.meta.observed_ns, completion + 1);
    }
}

#[test]
fn retained_pre_key_observation_is_rejected_without_false_clock_fault() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        let (mut c, t) = setup(backend, 2);
        c.on_tx_state(2, true, 100).unwrap();
        let old = block(&c, 0, 100, 1, 99);
        assert_eq!(c.submit(&old, 110), Err(Fault::NotKeyed));
        assert_eq!(old.meta.observed_ns, 99);
        assert_eq!(c.status().fault, None);
        assert_eq!(c.status().queue_depth, 0);
        assert_eq!(t.borrow().processed, 0);
        c.submit(&block(&c, 100, 1024, 2, 101), 110).unwrap();
        assert_eq!(c.pump(110), Ok(1));
        assert_eq!(t.borrow().processed, 1);
        assert_eq!(
            c.submit(&block(&c, 1124, 100, 3, 120), 110),
            Err(Fault::FutureObservation)
        );
        assert_eq!(c.status().fault, Some(Fault::FutureObservation));
    }
}

#[test]
fn delayed_settling_observation_is_rejected_after_boundary() {
    let (mut c, t) = setup(Backend::P2, 2);
    let mut cfg = config();
    cfg.settling_ns = 100;
    c.configure(c.fence(), cfg).unwrap();
    c.on_tx_state(2, true, 10).unwrap();
    assert_eq!(
        c.submit(&block(&c, 0, 1024, 1, 109), 200),
        Err(Fault::Settling)
    );
    assert_eq!(c.status().fault, None);
    assert_eq!(c.status().queue_depth, 0);
    assert_eq!(t.borrow().processed, 0);
    c.submit(&block(&c, 1024, 1024, 2, 110), 200).unwrap();
    assert_eq!(c.pump(200), Ok(1));
}

#[test]
fn saturated_overflowed_or_uncertain_counters_never_certify_clear() {
    let interval = Interval {
        fence: Fence::default(),
        first: 0,
        end: 4,
    };
    let clear = brackets(interval, 1)[0].unwrap();
    for width in [1, 8, 32, 64] {
        let mut b = clear;
        b.width_bits = Some(width);
        b.start_count = if width == 64 {
            u64::MAX
        } else {
            (1_u64 << width) - 1
        };
        b.end_count = b.start_count;
        assert_eq!(b.certify(interval, 1), Err(Fault::CounterInexact));
    }
    for variant in 0..7 {
        let mut b = clear;
        match variant {
            0 => b.width_bits = None,
            1 => b.width_bits = Some(0),
            2 => b.width_bits = Some(65),
            3 => b.saturated = Some(true),
            4 => b.saturated = None,
            5 => b.overflow = Some(true),
            _ => b.overflow = None,
        }
        assert_eq!(b.certify(interval, 1), Err(Fault::CounterInexact));
    }
    let (mut c, t) = setup(Backend::P2, 2);
    let mut b = block(&c, 0, 1024, 1, 1);
    b.meta.overload[1].as_mut().unwrap().saturated = Some(true);
    assert_eq!(c.submit(&b, 1), Err(Fault::CounterInexact));
    assert_eq!(c.status().fault, Some(Fault::CounterInexact));
    assert_eq!(t.borrow().processed, 0);
}
