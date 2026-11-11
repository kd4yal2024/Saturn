//! Offline SATP rate-matcher provenance for the pinned equal-rate WDSP port.
//! Shadow native FIR history and output occupancy, not a dequeue timestamp.
//! Any slew/discontinuity invalidates the binding; only a clean new matcher can
//! bind a fresh output generation. Production selection remains unchanged.
#![allow(dead_code)]
use crate::{
    tx_owner_stop::OutputStamp,
    wdsp::{MicRateMatcher, MicRmatchDiags},
};
use saturn_bridge::puresignal::Fault;
use std::collections::VecDeque;

pub(super) trait Port {
    fn diags(&self) -> MicRmatchDiags;
    fn feed64(&mut self, samples: &[f32]);
    fn take512(&mut self, samples: &mut Vec<f32>);
}
impl Port for MicRateMatcher {
    fn diags(&self) -> MicRmatchDiags {
        self.diags()
    }
    fn feed64(&mut self, samples: &[f32]) {
        self.feed(samples);
    }
    fn take512(&mut self, samples: &mut Vec<f32>) {
        self.take_block(samples);
    }
}
pub(super) struct Block {
    pub(super) samples: [f32; 512],
    pub(super) oldest: Option<u64>, // None = known constructor-primed zeros
    pub(super) stamp: OutputStamp,
}
pub(super) struct Provenance<P: Port> {
    port: P,
    output: Vec<f32>,
    stamp: Option<OutputStamp>,
    invalid: bool,
    pending: VecDeque<(f32, u64)>, // up to 63 original-age packet-tail samples
    history: VecDeque<u64>,        // retained raw inputs in varsamp's FIR
    ring: VecDeque<Option<u64>>,   // one label per native resampled output
    capacity: usize,
    history_size: usize,
    last_observed: Option<u64>,
    primed_inputs: usize,
}
impl Provenance<MicRateMatcher> {
    pub(super) fn pinned(history_size: usize) -> Result<Self, Fault> {
        // Caller must independently verify this port's pinned shape/identity.
        // rmatch/varsamp sources: 48k -> 48k, input 64, output 512, FIR 140.
        if history_size != 140 {
            return Err(Fault::NativeUnverified);
        }
        Self::new(MicRateMatcher::new(2048), history_size)
    }
}
impl<P: Port> Provenance<P> {
    pub(super) fn new(port: P, history_size: usize) -> Result<Self, Fault> {
        let d = port.diags();
        if history_size == 0
            || history_size > 2048
            || d.ringsize < 1024
            || d.ringsize > 8192
            || d.nring != d.ringsize / 2
            || d.underflows != 0
            || d.overflows != 0
        {
            return Err(Fault::NativeUnverified);
        }
        Ok(Self {
            port,
            output: Vec::with_capacity(512),
            stamp: None,
            invalid: false,
            pending: VecDeque::new(),
            history: VecDeque::new(),
            ring: VecDeque::from(vec![None; d.nring as usize]),
            capacity: d.ringsize as usize,
            history_size,
            last_observed: None,
            primed_inputs: 0,
        })
    }
    pub(super) fn is_bound(&self) -> bool {
        self.stamp.is_some()
    }
    pub(super) fn source_primed(&self) -> bool {
        self.primed_inputs >= 512
    }
    pub(super) fn invalidate(&mut self) {
        self.invalid = true;
        self.stamp = None;
        self.pending.clear();
        self.history.clear();
        self.ring.clear();
        self.primed_inputs = 0;
    }
    pub(super) fn bind(&mut self, stamp: OutputStamp) -> Result<(), Fault> {
        if self.invalid || self.stamp.is_some() {
            return Err(Fault::GenerationMismatch);
        }
        self.stamp = Some(stamp);
        Ok(())
    }
    fn check(&mut self, stamp: OutputStamp) -> Result<(), Fault> {
        if self.invalid || self.stamp != Some(stamp) {
            return Err(Fault::GenerationMismatch);
        }
        let d = self.port.diags();
        if d.ringsize != self.capacity as i32
            || d.nring < 0
            || d.nring as usize != self.ring.len()
            || d.underflows != 0
            || d.overflows != 0
            || !d.var_ratio.is_finite()
            || !(0.95..=1.05).contains(&d.var_ratio)
        {
            self.invalidate();
            return Err(Fault::SequenceGap);
        }
        Ok(())
    }
    pub(super) fn feed(
        &mut self,
        stamp: OutputStamp,
        observed: u64,
        samples: &[f32],
        now: u64,
        max_age: u64,
    ) -> Result<(), Fault> {
        self.check(stamp)?;
        if observed > now {
            return Err(Fault::FutureObservation);
        }
        if now - observed > max_age {
            self.invalidate();
            return Err(Fault::Stale);
        }
        if self.last_observed.is_some_and(|t| observed < t) {
            self.invalidate();
            return Err(Fault::Reordered);
        }
        if samples.len() > 1024 || samples.iter().any(|v| !v.is_finite()) {
            self.invalidate();
            return Err(Fault::NonFinite);
        }
        self.last_observed = Some(observed);
        self.pending.extend(samples.iter().map(|s| (*s, observed)));
        while self.pending.len() >= 64 {
            let chunk: Vec<_> = self.pending.drain(..64).collect();
            // Every output of this 64-input call conservatively covers the
            // previous FIR history plus ALL inputs in this call. New arrivals
            // cannot remove an older contribution from these output labels.
            let oldest = self
                .history
                .iter()
                .copied()
                .chain(chunk.iter().map(|s| s.1))
                .min()
                .unwrap();
            if now - oldest > max_age {
                self.invalidate();
                return Err(Fault::Stale);
            }
            let before = self.ring.len();
            self.port
                .feed64(&chunk.iter().map(|s| s.0).collect::<Vec<_>>());
            let d = self.port.diags();
            if d.underflows != 0
                || d.overflows != 0
                || d.ringsize != self.capacity as i32
                || d.nring < 0
                || (d.nring as usize) < before
                || (d.nring as usize) > self.capacity
            {
                self.invalidate();
                return Err(Fault::SequenceGap);
            }
            let added = d.nring as usize - before;
            // Bound allocation and refuse unexpected native accounting.
            if added > 128 {
                self.invalidate();
                return Err(Fault::SequenceGap);
            }
            self.ring.extend(std::iter::repeat_n(Some(oldest), added));
            self.history.extend(chunk.iter().map(|s| s.1));
            self.primed_inputs = (self.primed_inputs + chunk.len()).min(512);
            while self.history.len() > self.history_size {
                self.history.pop_front();
            }
        }
        self.check(stamp)
    }
    pub(super) fn take(
        &mut self,
        stamp: OutputStamp,
        now: u64,
        max_age: u64,
    ) -> Result<Block, Fault> {
        self.check(stamp)?;
        // Do not let WDSP's underflow path repeat dlast/slew samples with
        // invented freshness. Stop this generation before that call instead.
        if self.ring.len() < 512 {
            eprintln!("SATP native output underflow prevented ring={} pending={} history={} last_observed={:?} now={now}",
                self.ring.len(),self.pending.len(),self.history.len(),self.last_observed);
            self.invalidate();
            return Err(Fault::SequenceGap);
        }
        let oldest = self.ring.iter().take(512).filter_map(|t| *t).min();
        if oldest.is_some_and(|t| t > now || now - t > max_age) {
            self.invalidate();
            return Err(Fault::Stale);
        }
        self.output.clear();
        self.port.take512(&mut self.output);
        self.ring.drain(..512);
        self.check(stamp)?;
        if self.output.len() != 512 || self.output.iter().any(|v| !v.is_finite()) {
            self.invalidate();
            return Err(Fault::NativeFailure);
        }
        let samples = std::array::from_fn(|i| self.output[i]);
        Ok(Block {
            samples,
            oldest,
            stamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Sim {
        n: i32,
        under: i32,
        over: i32,
    }
    impl Port for Sim {
        fn diags(&self) -> MicRmatchDiags {
            MicRmatchDiags {
                nring: self.n,
                ringsize: 1024,
                underflows: self.under,
                overflows: self.over,
                var_ratio: 1.,
            }
        }
        fn feed64(&mut self, _: &[f32]) {
            self.n += 64;
            if self.n > 1024 {
                self.n = 1024;
                self.over += 1;
            }
        }
        fn take512(&mut self, out: &mut Vec<f32>) {
            self.n -= 512;
            out.resize(512, 0.2);
        }
    }
    fn matcher() -> (Provenance<Sim>, OutputStamp) {
        let mut m = Provenance::new(
            Sim {
                n: 512,
                under: 0,
                over: 0,
            },
            140,
        )
        .unwrap();
        let g = OutputStamp {
            generation: 1,
            revision: 2,
        };
        m.bind(g).unwrap();
        (m, g)
    }
    #[test]
    fn source_start_waits_for_real_inputs_not_primed_silence_or_partial_tail() {
        let (mut m, g) = matcher();
        assert!(!m.source_primed());
        m.feed(g, 1, &[0.2; 448], 1, 1000).unwrap();
        assert!(!m.source_primed());
        m.feed(g, 9, &[0.2; 63], 9, 1000).unwrap();
        assert!(!m.source_primed());
        m.feed(g, 10, &[0.2; 1], 10, 1000).unwrap();
        assert!(m.source_primed());
        assert_eq!(m.ring.back(), Some(&Some(1))); // no new priming timestamp
        m.invalidate();
        assert!(!m.source_primed());
        assert!(m.bind(g).is_err());
    }
    #[test]
    fn packet_tail_and_fir_history_keep_the_oldest_arrival() {
        let (mut m, g) = matcher();
        m.feed(g, 10, &[0.2; 32], 10, 1000).unwrap();
        m.feed(g, 20, &[0.2; 32], 20, 1000).unwrap();
        assert_eq!(m.ring.back(), Some(&Some(10)));
        m.feed(g, 30, &[0.2; 64], 30, 1000).unwrap();
        assert_eq!(m.ring.back(), Some(&Some(10)));
        m.feed(g, 40, &[0.2; 64], 40, 1000).unwrap();
        assert_eq!(m.ring.back(), Some(&Some(10)));
        m.take(g, 50, 1000).unwrap(); // primed zeros, not old operator samples
        let b = m.take(g, 50, 1000);
        assert_eq!(b.err(), Some(Fault::SequenceGap)); // no slew provenance fabricated
    }
    #[test]
    fn fresh_input_does_not_refresh_queued_output() {
        let (mut m, g) = matcher();
        m.feed(g, 1, &[0.2; 512], 1, 1000).unwrap();
        assert_eq!(m.take(g, 2, 1000).unwrap().oldest, None);
        m.feed(g, 99, &[0.2; 512], 99, 1000).unwrap();
        assert_eq!(m.take(g, 100, 1000).unwrap().oldest, Some(1));
    }
    #[test]
    fn old_floor_advances_only_after_fir_and_ring_both_retire_it() {
        let (mut m, g) = matcher();
        let mut ages = Vec::new();
        for time in [1, 10, 20, 30] {
            m.feed(g, time, &[0.2; 512], time, 1000).unwrap();
            ages.push(m.take(g, time, 1000).unwrap().oldest);
        }
        assert_eq!(ages, vec![None, Some(1), Some(1), Some(10)]);
    }
    #[test]
    fn partial_packet_stall_cannot_acquire_the_new_packets_age() {
        let (mut m, g) = matcher();
        m.feed(g, 1, &[0.2; 32], 1, 100).unwrap();
        assert_eq!(m.feed(g, 200, &[0.2; 32], 200, 100), Err(Fault::Stale));
        assert!(m.take(g, 200, 100).is_err());
    }
    #[test]
    fn retained_fir_samples_reject_fresh_packet_after_stall() {
        let (mut m, g) = matcher();
        m.feed(g, 1, &[0.2; 64], 1, 100).unwrap();
        assert_eq!(m.feed(g, 200, &[0.2; 64], 200, 100), Err(Fault::Stale));
        assert!(m.take(g, 200, 100).is_err());
    }
    #[test]
    fn discontinuity_and_old_generation_cannot_rebind_native_history() {
        let (mut m, g) = matcher();
        m.feed(g, 1, &[0.2; 512], 1, 1000).unwrap();
        m.invalidate();
        let fresh = OutputStamp {
            generation: 2,
            revision: 3,
        };
        assert_eq!(m.bind(fresh), Err(Fault::GenerationMismatch));
        let mut clean = Provenance::new(
            Sim {
                n: 512,
                under: 0,
                over: 0,
            },
            140,
        )
        .unwrap();
        clean.bind(fresh).unwrap();
        assert_eq!(
            clean.feed(g, 2, &[0.2; 64], 2, 1000),
            Err(Fault::GenerationMismatch)
        );
        assert_eq!(clean.take(fresh, 2, 1000).unwrap().oldest, None);
    }
    #[test]
    fn overflow_invalidates_instead_of_certifying_blended_samples() {
        let (mut m, g) = matcher();
        assert_eq!(m.feed(g, 1, &[0.2; 1024], 1, 1000), Err(Fault::SequenceGap));
        assert!(m.take(g, 2, 1000).is_err());
    }
}

#[cfg(all(
    test,
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
mod native_tests {
    use super::*;
    unsafe extern "C" {
        fn saturn_offline_satp_history() -> i32;
    }
    #[test]
    fn combined_satp_real_matcher_preserves_age_and_fir_stall() {
        assert_eq!(unsafe { saturn_offline_satp_history() }, 140);
        let g = OutputStamp {
            generation: 1,
            revision: 2,
        };
        let mut m = Provenance::pinned(140).unwrap();
        m.bind(g).unwrap();
        for i in 0..80u64 {
            let now = 1 + i * 10_666_667;
            m.feed(g, now, &[0.2; 512], now, 150_000_000).unwrap();
            let b = m.take(g, now, 150_000_000).unwrap();
            assert_eq!(b.stamp, g);
            if i < 4 {
                assert_eq!(b.oldest, None);
                assert!(b.samples.iter().all(|s| *s == 0.));
            } else {
                assert!(b.oldest.unwrap() < now);
                assert!(b.samples.iter().any(|s| s.abs() > 0.01));
            }
        }
        let old = m.history.front().copied().unwrap();
        let now = old + 150_000_001;
        assert_eq!(
            m.feed(g, now, &[0.2; 512], now, 150_000_000),
            Err(Fault::Stale)
        );
        assert!(m.take(g, now, 150_000_000).is_err());
        let newer = OutputStamp {
            generation: 2,
            revision: 3,
        };
        assert_eq!(m.bind(newer), Err(Fault::GenerationMismatch));
        let mut clean = Provenance::pinned(140).unwrap();
        clean.bind(newer).unwrap();
        let zero = clean.take(newer, now, 150_000_000).unwrap();
        assert_eq!(zero.oldest, None);
        assert!(zero.samples.iter().all(|s| *s == 0.));
    }
}
