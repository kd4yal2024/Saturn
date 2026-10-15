//! Standalone real-WDSP/shared-controller replay. No Cargo production wiring.
#[cfg(not(feature = "offline-puresignal-owner-native"))]
mod native_port;
#[cfg(feature = "offline-puresignal-owner-native")]
use crate::ps_native_port as native_port;
#[allow(dead_code)]
#[path = "../../src/puresignal/mod.rs"]
#[cfg(not(feature = "offline-puresignal-owner-native"))]
mod puresignal;
#[cfg(feature = "offline-puresignal-owner-native")]
use crate::puresignal;
use native_port::{OfflineNative, Stats};
#[cfg(feature = "offline-puresignal-policy-proposal")]
use puresignal::admission_proposal::*;
use puresignal::{adapters::*, controller::*, *};
use std::{cell::RefCell, rc::Rc, time::Duration};
#[cfg(feature = "offline-puresignal-tx-lifecycle")]
mod tx_authority_fixture;
#[cfg(feature = "offline-puresignal-tx-lifecycle")]
use puresignal::tx_lifecycle::*;

pub(crate) trait FeedbackTarget {
    fn fence(&self) -> Fence;
    fn status(&self) -> Status;
    fn submit(&mut self, block: &PairBlock, now: u64) -> Result<(), Fault>;
    fn pump(&mut self, now: u64) -> Result<usize, Fault>;
    fn fault(&mut self, fault: Fault, now: u64);
}
impl FeedbackTarget for Controller<OfflineNative> {
    fn fence(&self) -> Fence {
        self.fence()
    }
    fn status(&self) -> Status {
        self.status()
    }
    fn submit(&mut self, block: &PairBlock, now: u64) -> Result<(), Fault> {
        self.submit(block, now)
    }
    fn pump(&mut self, now: u64) -> Result<usize, Fault> {
        self.pump(now)
    }
    fn fault(&mut self, fault: Fault, _: u64) {
        self.latch_fault(fault);
    }
}
#[cfg(feature = "offline-puresignal-tx-lifecycle")]
impl FeedbackTarget for TxLifecycle<OfflineNative, tx_authority_fixture::SimAuthority> {
    fn fence(&self) -> Fence {
        self.fence()
    }
    fn status(&self) -> Status {
        self.feedback_status()
    }
    fn submit(&mut self, block: &PairBlock, now: u64) -> Result<(), Fault> {
        self.submit(block, now)
    }
    fn pump(&mut self, now: u64) -> Result<usize, Fault> {
        self.pump(now)
    }
    fn fault(&mut self, fault: Fault, now: u64) {
        self.notify_fault(fault, now).unwrap();
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Evidence {
    Clear,
    Missing,
    Inexact,
    Hit,
}
#[derive(Clone, Copy)]
enum Waveform {
    Normal,
    Silence,
    MissingRf,
    Rail,
}

pub(crate) struct Replay {
    backend: Backend,
    p2: P2Adapter,
    dma: DirectXdmaAdapter,
    output: PairBlock,
    sequence: u32,
    first: u64,
    pub(crate) clock: u64,
    random: u32,
    pub(crate) evidence: Evidence,
    waveform: Waveform,
}
fn encode24(value: f64, bytes: &mut [u8]) {
    let value = (value * 8388608.).round() as i32;
    assert!((-8388608..8388608).contains(&value));
    bytes.copy_from_slice(&value.to_be_bytes()[1..]);
}
impl Replay {
    pub(crate) fn new(backend: Backend, fence: Fence, clock: u64) -> Self {
        Self {
            backend,
            p2: P2Adapter::new(fence, 20),
            dma: DirectXdmaAdapter::new(fence, 20, true),
            output: PairBlock::default(),
            sequence: 0,
            first: 0,
            clock,
            random: 0x53460004,
            evidence: Evidence::Clear,
            waveform: Waveform::Normal,
        }
    }
    fn uniform(&mut self) -> f64 {
        self.random = self.random.wrapping_mul(1664525).wrapping_add(1013904223);
        f64::from(self.random) / 4294967296.
    }
    fn pair(&mut self) -> [u8; 12] {
        let r = 0.001 + 0.999 * self.uniform();
        let angle = std::f64::consts::TAU * self.uniform();
        let feedback = 0.5 * r * (1. - 0.25 * r * r);
        let phase = angle + 0.18 * r * r;
        let mut samples = [
            feedback * phase.cos(),
            feedback * phase.sin(),
            0.6121 * r * angle.cos(),
            0.6121 * r * angle.sin(),
        ];
        match self.waveform {
            Waveform::Normal => (),
            Waveform::Silence => samples.fill(0.0),
            Waveform::MissingRf => samples[..2].fill(0.0),
            Waveform::Rail => samples[0] = 8388607.0 / 8388608.0,
        }
        let mut bytes = [0; 12];
        for (value, dest) in samples.into_iter().zip(bytes.chunks_exact_mut(3)) {
            encode24(value, dest);
        }
        bytes
    }
    pub(crate) fn feed(&mut self, c: &mut impl FeedbackTarget) -> usize {
        self.feed_checked(c).unwrap()
    }
    fn feed_checked(&mut self, c: &mut impl FeedbackTarget) -> Result<usize, Fault> {
        self.clock += 10_000; // deterministic host-observation clock, not RF time
        let pairs = if self.backend == Backend::P2 { 119 } else { 4 };
        let interval = Interval {
            fence: c.fence(),
            first: self.first,
            end: self.first + pairs,
        };
        // Fixture generator knows every modeled sample was unclipped. These
        // brackets are SYNTHETIC, not derived from periodic hardware snapshots.
        let evidence = [
            OverloadSource::Adc1,
            OverloadSource::Ddc0Clamp,
            OverloadSource::Ddc1Clamp,
        ]
        .map(|source| {
            if matches!(self.evidence, Evidence::Missing) {
                return None;
            }
            Some(CounterBracket {
                source,
                interval,
                observed_ns: self.clock,
                counter_generation: 1,
                end_counter_generation: 1,
                start_count: 0,
                end_count: u64::from(matches!(self.evidence, Evidence::Hit)),
                width_bits: Some(64),
                saturated: Some(matches!(self.evidence, Evidence::Inexact)),
                overflow: Some(false),
                complete: true,
                valid: true,
            })
        });
        if self.backend == Backend::P2 {
            let mut packet = [0; 1444];
            packet[..4].copy_from_slice(&self.sequence.to_be_bytes());
            packet[12..14].copy_from_slice(&24_u16.to_be_bytes());
            packet[14..16].copy_from_slice(&238_u16.to_be_bytes());
            for dest in packet[16..].chunks_exact_mut(12) {
                dest.copy_from_slice(&self.pair());
            }
            if let Err(fault) = self
                .p2
                .decode(0, &packet, self.clock, evidence, &mut self.output)
            {
                c.fault(fault, self.clock); // explicit offline model of owner fault handoff
                return Err(fault);
            }
            self.sequence = self.sequence.wrapping_add(1);
        } else {
            let mut words = [[0; 8]; 17];
            words[0][..4].copy_from_slice(&PS_WITH_DDC6_RATE_WORD.to_le_bytes());
            words[0][7] = 0x80;
            for i in 0..4 {
                let pair = self.pair();
                words[1 + 2 * i][..6].copy_from_slice(&pair[..6]);
                words[2 + 2 * i][..6].copy_from_slice(&pair[6..]);
            }
            let mut ddc6 = [0.; 16];
            for (i, w) in words.iter().enumerate() {
                assert_eq!(
                    self.dma
                        .push_word(w, self.clock, evidence, &mut self.output, &mut ddc6)
                        .unwrap(),
                    i == 16
                );
            }
        }
        self.first += pairs;
        c.submit(&self.output, self.clock)?;
        let processed = c.pump(self.clock)?;
        if processed != 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(processed)
    }
    pub(crate) fn until(
        &mut self,
        c: &mut impl FeedbackTarget,
        condition: impl Fn(Status) -> bool,
    ) {
        let mut blocks = 0;
        loop {
            blocks += self.feed(c);
            if condition(c.status()) {
                break;
            }
            assert!(
                blocks < 3000,
                "native replay did not reach condition: {:?}",
                c.status()
            );
        }
    }
}
#[cfg(feature = "offline-puresignal-policy-proposal")]
fn proposal_offline(backend: Backend) {
    let stats = Rc::new(RefCell::new(Stats::default()));
    let proposal_config = |id| Config {
        automatic: true,
        ..config(id, 0.)
    };
    {
        let mut c = Controller::new_offline_admission_proposal(
            OfflineNative::open(stats.clone()),
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
        assert!(c.uses_offline_admission_proposal());
        c.configure(c.fence(), proposal_config(10)).unwrap();
        c.on_tx_state(1, true, 1).unwrap();
        let mut replay = Replay::new(backend, c.fence(), 1);
        replay.evidence = Evidence::Missing;
        replay.waveform = Waveform::Silence;
        let mut blocks = 0;
        while blocks < 12 {
            blocks += replay.feed(&mut c);
        }
        assert_eq!(c.status().info_raw[5], 0);
        assert!(!c.status().correction_applied_reported);
        assert_eq!(
            c.raw_unclipped_qualification(),
            Some(Err(Fault::ClipStatusUnknown))
        );
        replay.waveform = Waveform::Normal;
        replay.until(&mut c, |s| s.correction_applied_reported);
        // The REAL native fit is allowed without inventing clear brackets.
        assert_eq!(
            c.raw_unclipped_qualification(),
            Some(Err(Fault::ClipStatusUnknown))
        );
        let applied = c.status().info_raw[5];
        replay.waveform = Waveform::Silence;
        blocks = 0;
        while blocks < 12 {
            blocks += replay.feed(&mut c);
        }
        assert!(c.status().correction_applied_reported);
        assert_eq!(c.status().info_raw[5], applied); // no new fit attributed to silence
        replay.waveform = Waveform::Normal;
        replay.evidence = Evidence::Inexact;
        blocks = 0;
        while blocks < 2 {
            blocks += replay.feed(&mut c);
        }
        assert_eq!(
            c.raw_unclipped_qualification(),
            Some(Err(Fault::CounterInexact))
        );
        assert!(c.status().correction_applied_reported);
        replay.evidence = Evidence::Clear;
        blocks = 0;
        while blocks < 2 {
            blocks += replay.feed(&mut c);
        }
        assert_eq!(c.raw_unclipped_qualification(), Some(Ok(()))); // SYNTHETIC evidence only
        let before = stats.borrow().dsp_exchanges;
        // Existing-interface ADC overflow event, delayed by > stale deadline.
        let event = HazardObservation {
            fence: c.fence(),
            source: OverloadSource::Adc1,
            observed_ns: replay.clock,
            valid: true,
        };
        assert_eq!(
            c.observe_hazard(event, replay.clock + 300_000_000),
            Err(Fault::AdcOverrange)
        );
        replay.clock += 300_000_000;
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        assert!(stats.borrow().dsp_exchanges > before);
        assert!(!c.status().correction_applied_reported);
        assert!(c.raw_capture().is_none());
        c.on_tx_state(2, false, replay.clock + 1).unwrap();
        c.on_tx_state(3, true, replay.clock + 2).unwrap();
        assert_eq!(c.status().fault, Some(Fault::AdcOverrange));
        // Explicit reconfiguration and later TX epoch required for recovery.
        c.configure(c.fence(), proposal_config(11)).unwrap();
        c.on_tx_state(4, true, replay.clock + 3).unwrap();
        replay = Replay::new(backend, c.fence(), replay.clock + 3);
        replay.evidence = Evidence::Missing;
        replay.until(&mut c, |s| s.correction_applied_reported);
        replay.evidence = Evidence::Hit;
        assert_eq!(replay.feed_checked(&mut c), Err(Fault::AdcOverrange));
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        // Test known missing RF and component-rail guards on an applied curve.
        for (id, epoch, waveform, expected) in [
            (12, 5, Waveform::MissingRf, Fault::FeedbackLevel),
            (13, 6, Waveform::Rail, Fault::SevereOverdrive),
        ] {
            c.configure(c.fence(), proposal_config(id)).unwrap();
            c.on_tx_state(epoch, true, replay.clock + 1).unwrap();
            replay = Replay::new(backend, c.fence(), replay.clock + 1);
            replay.evidence = Evidence::Missing;
            replay.until(&mut c, |s| s.correction_applied_reported);
            replay.waveform = waveform;
            let before = stats.borrow().dsp_exchanges;
            loop {
                match replay.feed_checked(&mut c) {
                    Ok(_) => continue,
                    Err(fault) => {
                        assert_eq!(fault, expected);
                        break;
                    }
                }
            }
            assert_eq!(stats.borrow().final_iqc_flags, 0);
            assert!(stats.borrow().dsp_exchanges > before);
            assert_eq!(c.status().fault, Some(expected));
        }
        for (id, epoch, stale) in [(14, 7, false), (15, 8, true)] {
            c.configure(c.fence(), proposal_config(id)).unwrap();
            c.on_tx_state(epoch, true, replay.clock + 1).unwrap();
            replay = Replay::new(backend, c.fence(), replay.clock + 1);
            replay.evidence = Evidence::Missing;
            replay.until(&mut c, |s| s.correction_applied_reported);
            if stale {
                assert_eq!(c.pump(replay.clock + 250_000_001), Err(Fault::Stale));
                replay.clock += 250_000_001;
            } else if backend == Backend::P2 {
                replay.sequence = replay.sequence.wrapping_add(1);
                assert_eq!(replay.feed_checked(&mut c), Err(Fault::SequenceGap));
            } else {
                replay.dma.mark_loss();
                c.latch_fault(Fault::FifoLoss); // explicit owner callback, no hardware I/O
            }
            assert_eq!(stats.borrow().final_iqc_flags, 0);
            assert!(!c.status().correction_applied_reported);
        }
    }
    let stats = stats.borrow();
    assert!(stats.closed);
    assert_eq!(stats.final_iqc_flags, 0);
    println!("UNREVIEWED automatic admission proposal {:?} passed: real_native_blocks={} dsp_exchanges={} resets={} final_iqc_flags={} closed={} missing_telemetry_fit=true missing_telemetry_unclipped=false silence_no_new_fit=true delayed_hazard_reset=true missing_rf_reset=true rail_reset=true continuity_reset=true stale_reset=true",
        backend, stats.feedback_blocks, stats.dsp_exchanges, stats.resets, stats.final_iqc_flags, stats.closed);
}
fn config(id: u128, delay: f64) -> Config {
    Config {
        id,
        route: 20,
        physical_adc: OverloadSource::Adc1,
        enabled: true,
        automatic: false,
        hardware_peak: 0.6121,
        requested_delay_seconds: delay,
        settling_ns: 0,
        loop_delay_seconds: 0.,
        stale_after_ns: 250_000_000,
    }
}
fn qualify_offline(backend: Backend) {
    let stats = Rc::new(RefCell::new(Stats::default()));
    let actual_delay;
    {
        let mut c = Controller::new(
            OfflineNative::open(stats.clone()),
            Owner::Bridge,
            backend,
            77,
            Qualification {
                native_abi_verified: true,
                feedback_topology_verified: true, /* synthetic wire fixture ONLY */
            },
            4,
        );
        c.configure(c.fence(), config(10, 150e-9)).unwrap();
        let actual = c.status().actual_delay_seconds.unwrap();
        actual_delay = actual;
        assert!((actual - 150e-9).abs() < 21e-9);
        c.configure(c.fence(), config(11, 0.)).unwrap();
        c.on_tx_state(1, true, 1).unwrap();
        let mut replay = Replay::new(backend, c.fence(), 1);
        replay.until(&mut c, |s| s.info_raw[15] == 6 && s.info_raw[14] == 0);
        // Interrupt the real async fit, before successful application is reported.
        let old = c.fence();
        let before = stats.borrow().dsp_exchanges;
        c.on_tx_state(2, false, replay.clock).unwrap();
        assert!(stats.borrow().resets_during_fit > 0);
        assert!(stats.borrow().dsp_exchanges > before);
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        assert_ne!(c.fence(), old);
        assert!(!c.status().correction_applied_reported);
        assert!(c.raw_capture().is_none());
        assert_eq!(
            c.submit(&replay.output, replay.clock),
            Err(Fault::GenerationMismatch)
        );
        c.on_tx_state(3, true, replay.clock + 1).unwrap();
        replay = Replay::new(backend, c.fence(), replay.clock + 1);
        replay.until(&mut c, |s| s.correction_applied_reported);
        let applied = c.status().info_raw[5];
        assert!(applied >= 2); // drained fit cannot be attributed to this epoch
        let raw = c.raw_capture().unwrap().2;
        assert!(raw.iter().all(|v| v.is_finite()));
        // Configuration while keyed drains actual IQC, clears raw/queues, and
        // cannot arm new calibration without a later supervised TX epoch.
        let before = stats.borrow().dsp_exchanges;
        c.configure(c.fence(), config(12, 0.)).unwrap();
        assert!(stats.borrow().dsp_exchanges > before);
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        assert_eq!(c.status().state, State::Waiting);
        assert!(!c.status().correction_applied_reported);
        c.on_tx_state(4, true, replay.clock + 1).unwrap();
        replay = Replay::new(backend, c.fence(), replay.clock + 1);
        replay.until(&mut c, |s| s.correction_applied_reported);
        assert!(c.status().info_raw[5] > applied);
        // Freshness fault must reset real native correction too, not just flags.
        assert_eq!(c.pump(replay.clock + 250_000_001), Err(Fault::Stale));
        assert_eq!(c.status().state, State::Fault);
        assert!(!c.status().correction_applied_reported);
        assert_eq!(stats.borrow().final_iqc_flags, 0);
        c.on_tx_state(5, false, replay.clock + 250_000_002).unwrap();
        c.on_tx_state(6, true, replay.clock + 250_000_003).unwrap();
        assert_eq!(c.status().state, State::Fault);
        assert!(c.raw_capture().is_none());
    }
    let stats = stats.borrow();
    assert!(stats.closed);
    assert!(stats.reset_dsp_exchanges > 0);
    assert_eq!(stats.final_iqc_flags, 0);
    println!("combined offline {:?} passed: actual_delay_ns={:.6} real_native_blocks={} dsp_exchanges={} resets={} reset_dsp_exchanges={} unkey_during_fit={} final_iqc_flags={} closed={}",
        backend, actual_delay * 1e9, stats.feedback_blocks, stats.dsp_exchanges, stats.resets,
        stats.reset_dsp_exchanges, stats.resets_during_fit, stats.final_iqc_flags, stats.closed);
}
fn main() {
    qualify_offline(Backend::P2);
    qualify_offline(Backend::DirectXdma);
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    {
        proposal_offline(Backend::P2);
        proposal_offline(Backend::DirectXdma);
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    {
        lifecycle_offline(Backend::P2);
        lifecycle_offline(Backend::DirectXdma);
    }
}

#[cfg(feature = "offline-puresignal-tx-lifecycle")]
fn lifecycle_offline(backend: Backend) {
    use tx_authority_fixture::*;
    type Life = TxLifecycle<OfflineNative, SimAuthority>;
    fn clean(l: &mut Life, clock: &mut u64, release_during_reset: Option<&Rc<RefCell<Stats>>>) {
        let mut released = false;
        for _ in 0..3000 {
            *clock += 1_000_000;
            if release_during_reset.is_some_and(|s| s.borrow().cooperative_reset_pending)
                && !released
            {
                l.release_handle().release();
                released = true;
            }
            l.tick(*clock).unwrap();
            if l.status().phase == Phase::ReadyOff {
                assert!(
                    release_during_reset.is_none() || released,
                    "release must occur in native RESET, not just fit drainage"
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(1)); // caller yields, NOT inside cleanup
        }
        panic!(
            "cooperative native cleanup did not finish: {:?}",
            l.status()
        );
    }
    fn key(l: &mut Life, clock: &mut u64, seq: u64) {
        *clock += 1;
        l.fresh_tx_action(
            FreshTxAction {
                sequence: seq,
                release_ticket: l.status().release_handled,
                expected_fence: l.fence(),
            },
            *clock,
        )
        .unwrap();
        *clock += 1;
        l.tick(*clock).unwrap();
        assert_eq!(l.status().phase, Phase::Active);
    }
    let stats = Rc::new(RefCell::new(Stats::default()));
    let authority = Rc::new(RefCell::new(AuthorityTrace::default()));
    let controller = Controller::new_offline_admission_proposal(
        OfflineNative::open(stats.clone()),
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
    let mut l = Life::new(
        controller,
        SimAuthority::new(authority.clone()),
        backend,
        Limits {
            authority_timeout_ns: 100_000_000,
            cleanup_timeout_ns: 3_000_000_000,
        },
    )
    .unwrap();
    let mut clock = 1;
    l.reconfigure(
        l.fence(),
        Config {
            automatic: true,
            ..config(40, 0.)
        },
        clock,
    )
    .unwrap();
    clean(&mut l, &mut clock, None);
    key(&mut l, &mut clock, 1);
    let mut replay = Replay::new(backend, l.fence(), clock);
    replay.evidence = Evidence::Missing;
    replay.until(&mut l, |s| s.info_raw[15] == 6);
    clock = replay.clock;
    authority.borrow_mut().receipt_after_polls = 2;
    l.release_handle().release();
    let before = stats.borrow().cooperative_steps;
    clock += 1;
    l.tick(clock).unwrap();
    assert_eq!(stats.borrow().cooperative_steps, before); // authority not done: NO reset
    assert!(authority.borrow().output_inhibited);
    clean(&mut l, &mut clock, Some(&stats));
    assert!(stats.borrow().resets_during_fit > 0);
    assert_eq!(stats.borrow().blocking_resets, 0);
    assert_eq!(stats.borrow().final_iqc_flags, 0);
    assert_eq!(authority.borrow().release_tickets, [1, 2]);
    key(&mut l, &mut clock, 2);
    replay = Replay::new(backend, l.fence(), clock);
    replay.evidence = Evidence::Missing;
    replay.until(&mut l, |s| s.correction_applied_reported);
    clock = replay.clock;
    assert_eq!(l.status().correction_applied, Some(true));
    assert_eq!(
        l.raw_unclipped_qualification(),
        Some(Err(Fault::ClipStatusUnknown))
    );
    let hazard_fence = l.fence();
    let observed = clock;
    clock += 300_000_000;
    assert_eq!(
        l.observe_hazard(
            HazardObservation {
                fence: hazard_fence,
                source: OverloadSource::Adc1,
                observed_ns: observed,
                valid: true
            },
            clock
        ),
        Err(Fault::AdcOverrange)
    );
    assert_eq!(l.status().phase, Phase::AwaitingAuthority);
    assert!(l.status().last_known_applied_before_stop);
    assert_eq!(stats.borrow().blocking_resets, 0);
    l.release_handle().release();
    clean(&mut l, &mut clock, None);
    let snapshot = l.status();
    assert!(snapshot.authority_completed_ns <= snapshot.cleanup_started_ns);
    assert!(snapshot.cleanup_started_ns < snapshot.cleanup_completed_ns);
    assert_eq!(snapshot.primary_fault, Some(Fault::AdcOverrange));
    assert!(l
        .fresh_tx_action(
            FreshTxAction {
                sequence: 3,
                release_ticket: snapshot.release_handled,
                expected_fence: l.fence()
            },
            clock
        )
        .is_err());
    l.acknowledge_fault(snapshot.stop_id, clock).unwrap();
    assert_eq!(l.status().phase, Phase::ReadyOff);
    key(&mut l, &mut clock, 3);
    replay = Replay::new(backend, l.fence(), clock);
    replay.evidence = Evidence::Missing;
    replay.until(&mut l, |s| s.correction_applied_reported);
    replay.waveform = Waveform::MissingRf;
    loop {
        match replay.feed_checked(&mut l) {
            Ok(_) => (),
            Err(Fault::FeedbackLevel) => break,
            other => panic!("unexpected: {:?}", other),
        }
    }
    clock = replay.clock;
    assert_eq!(l.status().primary_fault, Some(Fault::FeedbackLevel));
    l.release_handle().release();
    clean(&mut l, &mut clock, None);
    l.acknowledge_fault(l.status().stop_id, clock).unwrap();
    key(&mut l, &mut clock, 4);
    replay = Replay::new(backend, l.fence(), clock);
    replay.evidence = Evidence::Missing;
    replay.until(&mut l, |s| s.info_raw[15] == 6);
    clock = replay.clock;
    l.reconfigure(
        l.fence(),
        Config {
            automatic: true,
            ..config(41, 0.)
        },
        clock,
    )
    .unwrap();
    assert_eq!(l.fence().config, 40);
    l.release_handle().release();
    clean(&mut l, &mut clock, None);
    assert_eq!(l.fence().config, 40); // release canceled the staged transaction
                                      // A genuinely fresh configuration after cancellation can still commit.
    clock += 1;
    l.reconfigure(
        l.fence(),
        Config {
            automatic: true,
            ..config(41, 0.)
        },
        clock,
    )
    .unwrap();
    clean(&mut l, &mut clock, None);
    assert_eq!(l.fence().config, 41);
    assert_eq!(l.status().phase, Phase::ReadyOff);
    assert_eq!(stats.borrow().blocking_resets, 0);
    // No arrival callback and no pump: the periodic service alone must stop.
    key(&mut l, &mut clock, 5);
    clock += 250_000_001;
    let before = stats.borrow().cooperative_steps;
    assert_eq!(l.tick(clock), Err(Fault::Stale));
    assert_eq!(l.status().phase, Phase::AwaitingAuthority);
    assert_eq!(stats.borrow().cooperative_steps, before);
    l.release_handle().release();
    clean(&mut l, &mut clock, None);
    assert_eq!(l.status().primary_fault, Some(Fault::Stale));
    l.acknowledge_fault(l.status().stop_id, clock).unwrap();
    key(&mut l, &mut clock, 6);
    replay = Replay::new(backend, l.fence(), clock);
    replay.evidence = Evidence::Missing;
    replay.until(&mut l, |s| s.correction_applied_reported);
    clock = replay.clock + 250_000_001;
    assert_eq!(l.tick(clock), Err(Fault::Stale));
    assert_eq!(l.status().phase, Phase::AwaitingAuthority);
    assert!(l.status().last_known_applied_before_stop);
    l.release_handle().release();
    clean(&mut l, &mut clock, None);
    assert_eq!(l.status().primary_fault, Some(Fault::Stale));
    assert_eq!(l.status().correction_applied, Some(false));
    println!("periodic watchdog {:?} passed: no-first-feedback and stopped-feedback-after-real-correction; fault_latched=true pump_required=false", backend);
    l.shutdown(clock).unwrap();
    clean(&mut l, &mut clock, None);
    let cleanup_steps = stats.borrow().cooperative_steps;
    assert!(authority.borrow().output_inhibited);
    assert_eq!(authority.borrow().drive, 0);
    drop(l);
    assert!(stats.borrow().closed);
    println!("lifecycle offline {:?} passed: real_feedback_blocks={} cooperative_steps={} reset_dsp_exchanges={} interrupted_fit={} runtime_blocking_resets=0 final_iqc_flags={} closed={} simulated_authority_only=true notification_to_stop_receipt_ns={} notification_to_cleanup_ns={}",
        backend, stats.borrow().feedback_blocks, cleanup_steps, stats.borrow().reset_dsp_exchanges,
        stats.borrow().resets_during_fit, stats.borrow().final_iqc_flags, stats.borrow().closed,
        snapshot.authority_completed_ns.unwrap() - snapshot.notified_ns.unwrap(),
        snapshot.cleanup_completed_ns.unwrap() - snapshot.notified_ns.unwrap());
}
