//! Startup level servo; no RF writes, DSP calls, allocation or clear-overload claims.
//! Physical key precedes leveling; the key receipt is held so native fitting/IQC
//! remain disabled. Once stable or limited without a rail/validity fault,
//! normal native admission starts and gain is held for this TX action.
use saturn_bridge::puresignal::{Fault, PairBlock};

pub(super) const START_DB: u8 = 31;
const INITIAL_SETTLE_NS: u64 = 500_000_000;
const STEP_SETTLE_NS: u64 = 30_000_000;
const WINDOW_NS: u64 = 40_000_000;
const MAX_STEPS: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Applying,
    Settling,
    Waiting,
    Stable,
    /// Startup ratio above target at maximum attenuation; not a native fit
    /// result or a clipping measurement. Normal controller admission still runs.
    Limited,
}
impl Phase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Applying => "applying",
            Self::Settling => "settling",
            Self::Waiting => "leveling",
            Self::Stable => "stable",
            Self::Limited => "limited",
        }
    }
}
pub(super) struct Servo {
    pub(super) phase: Phase,
    pub(super) db: u8,
    pub(super) applied_db: Option<u8>,
    pub(super) revision: u64,
    pub(super) info: Option<u32>,
    pub(super) ready_ns: Option<u64>,
    pub(super) reference_peak: f64,
    pub(super) rf_peak: f64,
    keyed: Option<u64>,
    applied: u64,
    boundary: u64,
    stale_ns: u64,
    peak: f64,
    rail: f64,
    last_observed: Option<u64>,
    next_pair: Option<u64>,
    next_sequence: Option<u32>,
    window_start: Option<u64>,
    tx_energy: f64,
    rf_energy: f64,
    active_pairs: usize,
    stable_windows: u8,
    limited_windows: u8,
    steps: u8,
}
impl Servo {
    pub(super) fn new(revision: u64, peak: f64, rail: f64, stale_ns: u64) -> Result<Self, Fault> {
        if revision == 0
            || !peak.is_finite()
            || peak <= 0.
            || !rail.is_finite()
            || !(0.5..1.).contains(&rail)
            || stale_ns == 0
        {
            return Err(Fault::InvalidConfig);
        }
        Ok(Self {
            phase: Phase::Settling,
            db: START_DB,
            applied_db: None,
            revision,
            info: None,
            ready_ns: None,
            reference_peak: 0.,
            rf_peak: 0.,
            keyed: None,
            applied: 0,
            boundary: 0,
            stale_ns,
            peak,
            rail,
            last_observed: None,
            next_pair: None,
            next_sequence: None,
            window_start: None,
            tx_energy: 0.,
            rf_energy: 0.,
            active_pairs: 0,
            stable_windows: 0,
            limited_windows: 0,
            steps: 0,
        })
    }
    pub(super) fn keyed(&mut self, now: u64) {
        self.applied_db = Some(self.db);
        self.keyed = Some(now);
        self.boundary = now.saturating_add(INITIAL_SETTLE_NS);
    }
    pub(super) fn requested(&mut self, db: u8, revision: u64, now: u64) {
        self.db = db;
        self.revision = revision;
        self.phase = Phase::Applying;
        self.ready_ns = None;
        self.applied_db = None;
        self.info = None;
        self.applied = now;
        self.next_pair = None;
        self.next_sequence = None;
        self.stable_windows = 0;
        self.limited_windows = 0;
        self.reset_window();
    }
    pub(super) fn applied(&mut self, now: u64) {
        if self.phase == Phase::Applying {
            self.applied_db = Some(self.db);
            self.phase = Phase::Settling;
            self.boundary = now.saturating_add(STEP_SETTLE_NS);
        }
    }
    fn reset_window(&mut self) {
        self.window_start = None;
        self.tx_energy = 0.;
        self.rf_energy = 0.;
        self.active_pairs = 0;
    }
    pub(super) fn tick(&self, now: u64) -> Result<(), Fault> {
        let Some(keyed) = self.keyed else {
            return Ok(());
        };
        if now < keyed {
            return Err(Fault::FutureObservation);
        }
        let anchor = self.last_observed.unwrap_or(keyed);
        if now.saturating_sub(anchor) > self.stale_ns {
            return Err(Fault::Stale);
        }
        if self.phase == Phase::Applying && now.saturating_sub(self.applied) > self.stale_ns {
            return Err(Fault::Stale);
        }
        Ok(())
    }
    /// Original ages and acquisition revision survive the queue. Wrong/old
    /// revisions are discarded, never promoted to evidence for a new gain.
    pub(super) fn observe(&mut self, b: &PairBlock, now: u64) -> Result<Option<u8>, Fault> {
        let Some(keyed) = self.keyed else {
            return Ok(None);
        };
        if b.acquisition_revision != self.revision {
            return Ok(None);
        }
        if b.meta.observed_ns > now {
            return Err(Fault::FutureObservation);
        }
        if b.meta.observed_ns < keyed {
            return Ok(None);
        }
        if now - b.meta.observed_ns > self.stale_ns {
            return Err(Fault::Stale);
        }
        if b.pairs == 0
            || b.pairs > saturn_bridge::puresignal::MAX_PAIRS
            || b.meta.interval.end.checked_sub(b.meta.interval.first) != Some(b.pairs as u64)
        {
            return Err(Fault::UnequalLength);
        }
        if b.meta.interval.fence != b.meta.fence {
            return Err(Fault::GenerationMismatch);
        }
        if self
            .next_pair
            .is_some_and(|next| b.meta.interval.first != next)
        {
            return Err(Fault::SequenceGap);
        }
        if let Some(seq) = b.meta.sequence {
            if self.next_sequence.is_some_and(|next| seq != next) {
                return Err(Fault::SequenceGap);
            }
            self.next_sequence = Some(seq.wrapping_add(u32::from(b.meta.sequence_packets)));
        }
        self.next_pair = Some(b.meta.interval.end);
        self.last_observed = Some(b.meta.observed_ns);
        saturn_bridge::puresignal::admission_proposal::check_operational_evidence(
            b.meta,
            saturn_bridge::puresignal::OverloadSource::Adc1,
            now,
        )?;
        let tx = &b.tx[..b.pairs * 2];
        let rf = &b.rf[..b.pairs * 2];
        if tx.iter().chain(rf).any(|x| !x.is_finite()) {
            return Err(Fault::NonFinite);
        }
        self.reference_peak = tx.iter().fold(0f64, |p, v| p.max(v.abs()));
        self.rf_peak = rf.iter().fold(0f64, |p, v| p.max(v.abs()));
        if tx.iter().any(|x| x.abs() >= self.rail) {
            return Err(Fault::SevereOverdrive);
        }
        // A revision identifies the requested layout, not the analogue gain
        // at capture time. Wait for acquisition completion AND settling before
        // interpreting RF level. Known overload/nonfinite/reference faults above
        // remain authoritative throughout the transition.
        if self.phase == Phase::Applying || b.meta.observed_ns < self.boundary {
            return Ok(None);
        }
        if rf.iter().any(|x| x.abs() >= self.rail) {
            if self.db == 31 {
                return Err(Fault::SevereOverdrive);
            }
            self.stable_windows = 0;
            self.limited_windows = 0;
            self.steps = self.steps.saturating_add(1);
            if self.steps > MAX_STEPS {
                return Err(Fault::FeedbackLevel);
            }
            return Ok(Some(self.db.saturating_add(6).min(31)));
        }
        if matches!(self.phase, Phase::Stable | Phase::Limited) {
            return Ok(None);
        }
        self.phase = Phase::Waiting;
        self.window_start.get_or_insert(b.meta.observed_ns);
        for (tx, rf) in tx.chunks_exact(2).zip(rf.chunks_exact(2)) {
            let t = tx[0] * tx[0] + tx[1] * tx[1];
            // Do not use speech pauses/two-tone minima as missing feedback or
            // as a reason to run the attenuator down to zero.
            if t >= (self.peak * 0.03).powi(2) {
                self.tx_energy += t;
                self.rf_energy += rf[0] * rf[0] + rf[1] * rf[1];
                self.active_pairs += 1;
            }
        }
        if b.meta.observed_ns - self.window_start.unwrap() < WINDOW_NS {
            return Ok(None);
        }
        if self.active_pairs < 128 {
            self.reset_window();
            return Ok(None);
        }
        let ratio = (self.rf_energy / self.tx_energy).sqrt();
        if !ratio.is_finite() {
            return Err(Fault::NonFinite);
        }
        let info = (256. * ratio).round().clamp(0., 65535.) as u32;
        self.info = Some(info);
        self.reset_window();
        if (140..=165).contains(&info) {
            self.limited_windows = 0;
            self.stable_windows = self.stable_windows.saturating_add(1);
            if self.stable_windows >= 2 {
                self.phase = Phase::Stable;
                self.ready_ns = Some(now)
            }
            return Ok(None);
        }
        self.stable_windows = 0;
        // The startup RMS ratio is not WDSP's extrapolated info[4]. Running
        // out of actuator range cannot establish clipping. Keep maximum ATT,
        // require two valid windows, and let the normal controller/native fit
        // evaluate the feedback. All timestamp, continuity, overload, finite
        // and component-rail checks above remain prerequisites on every block.
        if self.db == 31 && info > 165 {
            self.limited_windows = self.limited_windows.saturating_add(1);
            if self.limited_windows >= 2 {
                self.phase = Phase::Limited;
                self.ready_ns = Some(now);
            }
            return Ok(None);
        }
        self.limited_windows = 0;
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_STEPS {
            return Err(Fault::FeedbackLevel);
        }
        let delta = if info == 0 {
            -6
        } else {
            (20. * (f64::from(info) / 152.293).log10())
                .round()
                .clamp(-6., 6.) as i16
        };
        let db = (i16::from(self.db) + delta).clamp(0, 31) as u8;
        if db == self.db {
            return Err(Fault::FeedbackLevel);
        }
        Ok(Some(db))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block(revision: u64, first: u64, now: u64, gain: f64) -> PairBlock {
        let mut b = PairBlock::default();
        b.pairs = 1024;
        b.acquisition_revision = revision;
        b.meta.observed_ns = now;
        b.meta.interval.first = first;
        b.meta.interval.end = first + 1024;
        for i in 0..b.pairs {
            b.tx[i * 2] = 0.3;
            b.rf[i * 2] = 0.3 * gain;
        }
        b
    }
    #[test]
    fn starts_at_31_and_converges_without_refreshing_old_gain_blocks() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        assert_eq!(s.db, 31);
        let mut first = 0;
        let mut revision = 1;
        for step in 0..250 {
            let now = step * 10_000_000;
            s.tick(now).unwrap();
            let b = block(
                revision,
                first,
                now,
                2. * 10f64.powf(-f64::from(s.db) / 20.),
            );
            first += 1024;
            if let Some(db) = s.observe(&b, now).unwrap() {
                let old = revision;
                revision += 1;
                s.requested(db, revision, now);
                first = 0;
                assert_eq!(s.observe(&block(old, 0, now, 100.), now).unwrap(), None);
                s.applied(now);
            }
            if s.phase == Phase::Stable {
                break;
            }
        }
        assert_eq!(s.phase, Phase::Stable);
        assert_eq!(s.db, 11);
        assert!(s.ready_ns.unwrap() >= INITIAL_SETTLE_NS);
        assert!((140..=165).contains(&s.info.unwrap()));
    }
    #[test]
    fn gain_transition_cannot_reinterpret_old_rail_samples_or_reuse_readiness() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        s.phase = Phase::Stable;
        s.ready_ns = Some(10);
        s.requested(31, 2, 20);
        assert_eq!(s.ready_ns, None);
        assert_eq!(s.applied_db, None);
        assert_eq!(s.observe(&block(2, 0, 21, 4.), 21), Ok(None));
        s.applied(22);
        assert_eq!(s.observe(&block(2, 1024, 23, 4.), 23), Ok(None));
        assert_eq!(
            s.observe(
                &block(2, 2048, 22 + STEP_SETTLE_NS, 4.),
                22 + STEP_SETTLE_NS
            ),
            Err(Fault::SevereOverdrive)
        );
    }
    #[test]
    fn completed_leveling_keeps_its_status_until_an_explicit_gain_change() {
        for phase in [Phase::Stable, Phase::Limited] {
            let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
            s.keyed(0);
            s.phase = phase;
            s.ready_ns = Some(INITIAL_SETTLE_NS);
            assert_eq!(
                s.observe(&block(1, 0, INITIAL_SETTLE_NS, 0.6), INITIAL_SETTLE_NS),
                Ok(None)
            );
            assert_eq!(s.phase, phase);
            assert_eq!(s.ready_ns, Some(INITIAL_SETTLE_NS));
        }
    }
    #[test]
    fn silence_and_two_tone_minima_do_not_run_attenuation_down() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        for step in 0..400 {
            let now = step * 10_000_000;
            let mut b = block(1, step * 1024, now, 0.);
            b.tx.fill(0.);
            s.tick(now).unwrap();
            assert_eq!(s.observe(&b, now).unwrap(), None);
        }
        assert_eq!(s.db, 31);
        assert_eq!(s.ready_ns, None);
    }
    #[test]
    fn missing_feedback_expires_from_periodic_service_without_a_packet() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(10);
        assert_eq!(s.tick(150_000_011), Err(Fault::Stale));
    }
    #[test]
    fn stalled_layout_and_old_or_future_samples_never_look_fresh() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        assert_eq!(
            s.observe(&block(1, 0, 2, 0.1), 1),
            Err(Fault::FutureObservation)
        );
        assert_eq!(
            s.observe(&block(1, 0, 0, 0.1), 150_000_001),
            Err(Fault::Stale)
        );
        s.requested(25, 2, 0);
        assert_eq!(
            s.observe(&block(1, 0, 140_000_000, 1.), 140_000_000)
                .unwrap(),
            None
        );
        assert_eq!(s.tick(150_000_001), Err(Fault::Stale));
    }
    #[test]
    fn full_scale_at_maximum_attenuation_and_bad_reference_remain_faults() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        assert_eq!(
            s.observe(&block(1, 0, INITIAL_SETTLE_NS, 4.), INITIAL_SETTLE_NS),
            Err(Fault::SevereOverdrive)
        );
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        let mut b = block(1, 0, 1, 0.1);
        b.tx[0] = 1.;
        assert_eq!(s.observe(&b, 1), Err(Fault::SevereOverdrive));
    }
    #[test]
    fn unclipped_258_estimate_at_maximum_is_a_limit_notice_not_a_tx_fault() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        let mut limited_at = None;
        for step in 0..80u64 {
            let now = step * 10_000_000;
            s.tick(now).unwrap();
            let b = block(1, step * 1024, now, 258. / 256.);
            assert_eq!(s.observe(&b, now).unwrap(), None);
            if step < 59 {
                assert!(
                    s.ready_ns.is_none(),
                    "two completed post-settling windows required"
                );
            }
            if s.ready_ns.is_some() {
                limited_at = Some(now);
                break;
            }
        }
        assert!(limited_at.is_some());
        assert_eq!(s.phase, Phase::Limited);
        assert_eq!(s.db, 31);
        assert_eq!(s.info, Some(258));
        let now = limited_at.unwrap() + 10_000_000;
        let mut clipped = block(1, (now / 10_000_000) * 1024, now, 258. / 256.);
        clipped.rf[0] = 0.95;
        assert_eq!(s.observe(&clipped, now), Err(Fault::SevereOverdrive));
    }
    #[test]
    fn discontinuity_nonfinite_and_positive_overload_cannot_enable_fitting() {
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        s.observe(&block(1, 0, 0, 0.1), 0).unwrap();
        assert_eq!(
            s.observe(&block(1, 2048, 1, 0.1), 1),
            Err(Fault::SequenceGap)
        );
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        let mut b = block(1, 0, 1, 0.1);
        b.rf[0] = f64::NAN;
        assert_eq!(s.observe(&b, 1), Err(Fault::NonFinite));
        let mut s = Servo::new(1, 0.67, 0.95, 150_000_000).unwrap();
        s.keyed(0);
        let mut b = block(1, 0, 1, 0.1);
        b.meta.overload[0] = Some(saturn_bridge::puresignal::CounterBracket {
            source: saturn_bridge::puresignal::OverloadSource::Adc1,
            interval: b.meta.interval,
            observed_ns: 1,
            valid: true,
            counter_generation: 1,
            end_counter_generation: 1,
            start_count: 0,
            end_count: 1,
            width_bits: Some(32),
            saturated: Some(false),
            overflow: Some(false),
            complete: true,
        });
        assert_eq!(s.observe(&b, 1), Err(Fault::AdcOverrange));
    }
}
