//! Thetis-style operational leveling decisions. This module performs no RF,
//! native, acquisition or timing calls. The sole owner executes/acknowledges
//! each step; original acquisition revision and sample age remain authoritative.
use saturn_bridge::puresignal::Fault;

pub(super) const SETTLE_NS: u64 = 30_000_000;
const TRANSITION_NS: u64 = 3_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Monitoring,
    RequestReset,
    Resetting,
    RequestGain,
    Applying,
    Settling,
    Resume,
}
impl Phase {
    pub(super) fn label(self) -> &'static str {
        match self {
            // Preserve the existing auto-attenuation wire vocabulary. Older
            // clients must still display actual gain acknowledgments.
            Self::Monitoring => "stable",
            Self::RequestReset | Self::Resetting => "leveling",
            Self::RequestGain | Self::Applying => "applying",
            Self::Settling => "settling",
            Self::Resume => "leveling",
        }
    }
}

pub(super) struct Runtime {
    pub(super) phase: Phase,
    pub(super) revision: u64,
    pub(super) db: u8,
    pub(super) wanted: u8,
    pub(super) info: Option<u32>,
    last_attempt: i32,
    transition_at: u64,
    observation_floor: u64,
    max_age_ns: u64,
}
impl Runtime {
    pub(super) fn new(db: u8, revision: u64, max_age_ns: u64) -> Result<Self, Fault> {
        if db > 31 || revision == 0 || max_age_ns == 0 {
            return Err(Fault::InvalidConfig);
        }
        Ok(Self {
            phase: Phase::Monitoring,
            revision,
            db,
            wanted: db,
            info: None,
            last_attempt: 0,
            transition_at: 0,
            observation_floor: 0,
            max_age_ns,
        })
    }
    pub(super) fn check_deadline(&self, now: u64) -> Result<(), Fault> {
        if self.phase != Phase::Monitoring {
            if now < self.transition_at {
                return Err(Fault::FutureObservation);
            }
            if now - self.transition_at > TRANSITION_NS {
                return Err(Fault::NativeFailure);
            }
        }
        Ok(())
    }
    /// In the pinned WDSP 2.10, info[7] counts attempted fits and info[5]
    /// counts accepted solutions. Never reuse an old gain's level report.
    pub(super) fn observe(&mut self, info: [i32; 16], now: u64) -> Result<(), Fault> {
        self.check_deadline(now)?;
        if self.phase != Phase::Monitoring {
            return Ok(());
        }
        if info[7] < self.last_attempt || info[5] < 0 || info[4] < 0 {
            return Err(Fault::NativeFailure);
        }
        if info[7] == self.last_attempt {
            return Ok(());
        }
        self.last_attempt = info[7];
        // A failed extrapolation does not establish a feedback level. Zero
        // cannot drive attenuation down during absent feedback or speech gaps.
        if info[0] != 0 || info[4] == 0 {
            return Ok(());
        }
        self.info = Some(info[4] as u32);
        let delta = if (129..=181).contains(&info[4]) {
            0
        } else if info[4] > 256 {
            31
        } else {
            (20. * (f64::from(info[4]) / 152.293).log10()).round() as i16
        };
        let wanted = (i16::from(self.db) + delta).clamp(0, 31) as u8;
        if wanted != self.db {
            self.wanted = wanted;
            self.transition_at = now;
            self.phase = Phase::RequestReset;
        }
        Ok(())
    }
    pub(super) fn reset_issued(&mut self) -> Result<(), Fault> {
        if self.phase != Phase::RequestReset {
            return Err(Fault::GenerationMismatch);
        }
        self.phase = Phase::Resetting;
        Ok(())
    }
    pub(super) fn reset_complete(&mut self) -> Result<(), Fault> {
        if self.phase != Phase::Resetting {
            return Err(Fault::GenerationMismatch);
        }
        self.phase = Phase::RequestGain;
        Ok(())
    }
    pub(super) fn gain_requested(&mut self, serial: u64) -> Result<(), Fault> {
        if self.phase != Phase::RequestGain || serial <= self.revision {
            return Err(Fault::GenerationMismatch);
        }
        self.revision = serial;
        self.phase = Phase::Applying;
        Ok(())
    }
    pub(super) fn gain_applied(&mut self, now: u64) -> Result<(), Fault> {
        self.check_deadline(now)?;
        if self.phase != Phase::Applying {
            return Err(Fault::GenerationMismatch);
        }
        self.db = self.wanted;
        self.observation_floor = now.checked_add(SETTLE_NS).ok_or(Fault::FutureObservation)?;
        self.phase = Phase::Settling;
        Ok(())
    }
    /// True means this block may enter normal controller admission. No sample
    /// is retimed and no old-revision packet can open a later transition.
    pub(super) fn feedback(&mut self, revision: u64, oldest: u64, now: u64) -> Result<bool, Fault> {
        self.check_deadline(now)?;
        if revision != self.revision {
            return Ok(false);
        }
        if oldest > now {
            return Err(Fault::FutureObservation);
        }
        if now - oldest > self.max_age_ns {
            return Err(Fault::Stale);
        }
        match self.phase {
            Phase::Monitoring | Phase::RequestReset | Phase::Resetting => Ok(true),
            Phase::Settling if oldest >= self.observation_floor => {
                self.phase = Phase::Resume;
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn resumed(&mut self, info: [i32; 16]) -> Result<(), Fault> {
        if self.phase != Phase::Resume || info[7] < 0 {
            return Err(Fault::GenerationMismatch);
        }
        self.last_attempt = info[7];
        self.info = None; // old calibration level is not a new-gain reading
        self.phase = Phase::Monitoring;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(attempt: i32, level: i32) -> [i32; 16] {
        let mut i = [0; 16];
        i[7] = attempt;
        i[4] = level;
        i
    }
    #[test]
    fn low_and_high_power_levels_use_native_reports_and_thetis_hysteresis() {
        for (level, expected) in [
            (80, 6),
            (128, 10),
            (129, 12),
            (153, 12),
            (181, 12),
            (182, 14),
            (230, 16),
            (300, 31),
        ] {
            let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
            r.observe(report(1, level), 1).unwrap();
            assert_eq!(r.wanted, expected, "level {level}");
            assert_eq!(
                r.phase,
                if expected == 12 {
                    Phase::Monitoring
                } else {
                    Phase::RequestReset
                }
            );
        }
    }
    #[test]
    fn no_level_failed_extrapolation_repeated_reports_or_hardware_limit_cause_reset_loop() {
        for (db, level) in [(0, 80), (31, 230), (12, 0)] {
            let mut r = Runtime::new(db, 3, 150_000_000).unwrap();
            for n in 1..100 {
                r.observe(report(n, level), n as u64).unwrap();
                assert_eq!(r.phase, Phase::Monitoring);
            }
        }
        let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
        let mut i = report(1, 230);
        i[0] = 1;
        r.observe(i, 1).unwrap();
        i[0] = 0;
        r.observe(i, 2).unwrap();
        assert_eq!(r.phase, Phase::Monitoring);
        i[7] = 2;
        r.observe(i, 3).unwrap();
        assert_eq!(r.phase, Phase::RequestReset);
    }
    #[test]
    fn reset_gain_ack_original_sample_settling_and_resume_are_ordered() {
        let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
        r.observe(report(1, 230), 1).unwrap();
        assert_eq!(r.gain_requested(4), Err(Fault::GenerationMismatch));
        r.reset_issued().unwrap();
        assert!(r.feedback(3, 2, 2).unwrap());
        r.reset_complete().unwrap();
        assert_eq!(r.gain_requested(3), Err(Fault::GenerationMismatch));
        r.gain_requested(4).unwrap();
        assert!(!r.feedback(3, 3, 3).unwrap());
        assert!(!r.feedback(4, 3, 3).unwrap());
        r.gain_applied(4).unwrap();
        assert_eq!(r.db, 16);
        assert!(!r.feedback(4, 4, SETTLE_NS + 4).unwrap());
        assert_eq!(r.phase, Phase::Settling);
        assert!(!r.feedback(4, SETTLE_NS + 4, SETTLE_NS + 4).unwrap());
        assert_eq!(r.phase, Phase::Resume);
        r.resumed(report(1, 230)).unwrap();
        assert!(r.info.is_none());
        r.observe(report(1, 230), SETTLE_NS + 5).unwrap();
        assert_eq!(r.phase, Phase::Monitoring);
        r.observe(report(2, 153), SETTLE_NS + 6).unwrap();
        assert_eq!(r.phase, Phase::Monitoring);
    }
    #[test]
    fn delayed_old_gain_stale_and_future_samples_cannot_resume() {
        let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
        r.observe(report(1, 230), 1).unwrap();
        r.reset_issued().unwrap();
        r.reset_complete().unwrap();
        r.gain_requested(4).unwrap();
        r.gain_applied(4).unwrap();
        assert!(!r.feedback(3, SETTLE_NS + 4, SETTLE_NS + 4).unwrap());
        assert_eq!(
            r.feedback(4, SETTLE_NS + 5, SETTLE_NS + 4),
            Err(Fault::FutureObservation)
        );
        assert_eq!(
            r.feedback(4, SETTLE_NS + 4, SETTLE_NS + 150_000_005),
            Err(Fault::Stale)
        );
        assert_eq!(r.phase, Phase::Settling);
    }
    #[test]
    fn transition_deadline_and_counter_reset_are_not_hidden_by_new_reports() {
        let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
        r.observe(report(1, 230), 1).unwrap();
        assert_eq!(
            r.observe(report(100, 153), TRANSITION_NS + 2),
            Err(Fault::NativeFailure)
        );
        let mut r = Runtime::new(12, 3, 150_000_000).unwrap();
        r.observe(report(4, 153), 1).unwrap();
        assert_eq!(r.observe(report(3, 153), 2), Err(Fault::NativeFailure));
    }
}
