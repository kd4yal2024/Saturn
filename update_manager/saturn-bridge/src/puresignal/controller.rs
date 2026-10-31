use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Qualification {
    pub native_abi_verified: bool,
    pub feedback_topology_verified: bool,
}
impl Qualification {
    pub const UNVERIFIED: Self = Self {
        native_abi_verified: false,
        feedback_topology_verified: false,
    };
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub id: u128,
    pub route: u128,
    pub physical_adc: OverloadSource,
    pub enabled: bool,
    pub automatic: bool,
    pub hardware_peak: f64,
    pub requested_delay_seconds: f64,
    pub settling_ns: u64,
    pub loop_delay_seconds: f64,
    pub stale_after_ns: u64,
}
impl Config {
    pub(super) fn validate(self) -> Result<(), Fault> {
        if self.id == 0
            || self.route == 0
            || !matches!(
                self.physical_adc,
                OverloadSource::Adc1 | OverloadSource::Adc2
            )
            || !self.hardware_peak.is_finite()
            || self.hardware_peak <= 0.0
            || self.hardware_peak > 1.0
            || !self.requested_delay_seconds.is_finite()
            || self.requested_delay_seconds.abs() > 0.1
            || !self.loop_delay_seconds.is_finite()
            || self.loop_delay_seconds < 0.0
            || self.settling_ns > 10_000_000_000
            || self.stale_after_ns == 0
            || self.stale_after_ns > 1_000_000_000
        {
            return Err(Fault::InvalidConfig);
        }
        Ok(())
    }
}

/// Implement on the existing TX thread, with verified native identity and the
/// channel lifetime fenced there. This trait intentionally has no RF key method.
/// configure must SetPSFeedbackRate before setting delay and return its actual
/// quantized value. reset must quiesce native asynchronous work/old curves before
/// admitting a new epoch; a mock implementation does NOT prove that contract.
pub trait NativePs {
    /// Explicit readback from the matched native owner. The default refuses:
    /// a disabled configuration alone cannot certify fitting/correction off.
    fn measurement_only_guard(&mut self) -> Result<(), Fault> {
        Err(Fault::NativeUnverified)
    }
    fn configure(&mut self, config: Config) -> Result<f64, Fault>;
    fn mox(&mut self, active: bool);
    fn reset(&mut self) -> Result<(), Fault>;
    fn process(
        &mut self,
        tx: &mut [f64; NATIVE_PAIRS * 2],
        rf: &mut [f64; NATIVE_PAIRS * 2],
    ) -> Result<(), Fault>;
    fn information(&mut self) -> Result<[i32; 16], Fault>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Waiting,
    Settling,
    Collecting,
    Correcting,
    Fault,
}
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub fence: Fence,
    pub state: State,
    pub fault: Option<Fault>,
    pub keyed: bool,
    pub info_raw: [i32; 16],
    pub correction_applied_reported: bool,
    pub actual_delay_seconds: Option<f64>,
    pub accepted_pairs: u64,
    pub rejected_blocks: u64,
    pub queue_depth: usize,
    pub queue_capacity: usize,
}

pub struct Controller<N: NativePs> {
    measurement_only: bool,
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    lifecycle_deferred: bool,
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    native_clean: bool,
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    draft_limits: Option<super::admission_proposal::DraftLimits>,
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    partial_qualification: Result<(), Fault>,
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    raw_qualification: Option<Result<(), Fault>>,
    native: N,
    owner: Owner,
    backend: Backend,
    qualification: Qualification,
    config: Option<Config>,
    fence: Fence,
    keyed: bool,
    keyed_at_ns: u64,
    adaptation_armed: bool,
    state: State,
    fault: Option<Fault>,
    actual_delay: Option<f64>,
    info: [i32; 16],
    calibration_baseline: i32,
    queue: Vec<PairBlock>,
    head: usize,
    queued: usize,
    tx: Box<[f64; NATIVE_PAIRS * 2]>,
    rf: Box<[f64; NATIVE_PAIRS * 2]>,
    raw_rf: Box<[f64; NATIVE_PAIRS * 2]>,
    raw_interval: Option<Interval>,
    raw_observed_ns: Option<u64>,
    partial_pairs: usize,
    partial_first: Option<u64>,
    partial_observed_ns: Option<u64>,
    next_pair: Option<u64>,
    next_p2_sequence: Option<u32>,
    accepted_pairs: u64,
    rejected_blocks: u64,
    last_observed_ns: Option<u64>,
}
impl<N: NativePs> Controller<N> {
    pub fn new(
        native: N,
        owner: Owner,
        backend: Backend,
        boot: u128,
        qualification: Qualification,
        capacity: usize,
    ) -> Self {
        assert!((1..=64).contains(&capacity));
        Self {
            measurement_only: false,
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            lifecycle_deferred: false,
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            native_clean: false,
            #[cfg(feature = "offline-puresignal-policy-proposal")]
            draft_limits: None, // new() ALWAYS retains the previous strict gate
            #[cfg(feature = "offline-puresignal-policy-proposal")]
            partial_qualification: Ok(()),
            #[cfg(feature = "offline-puresignal-policy-proposal")]
            raw_qualification: None,
            native,
            owner,
            backend,
            qualification,
            config: None,
            fence: Fence {
                boot,
                ..Fence::default()
            },
            keyed: false,
            keyed_at_ns: 0,
            adaptation_armed: false,
            state: State::Off,
            fault: None,
            actual_delay: None,
            info: [0; 16],
            calibration_baseline: 0,
            queue: (0..capacity).map(|_| PairBlock::default()).collect(),
            head: 0,
            queued: 0,
            tx: Box::new([0.0; NATIVE_PAIRS * 2]),
            rf: Box::new([0.0; NATIVE_PAIRS * 2]),
            raw_rf: Box::new([0.0; NATIVE_PAIRS * 2]),
            raw_interval: None,
            raw_observed_ns: None,
            partial_pairs: 0,
            partial_first: None,
            partial_observed_ns: None,
            next_pair: None,
            next_p2_sequence: None,
            accepted_pairs: 0,
            rejected_blocks: 0,
            last_observed_ns: None,
        }
    }
    pub fn fence(&self) -> Fence {
        self.fence
    }
    // Accessible ONLY to the fenced lifecycle owner, never a second WDSP owner.
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn attach_lifecycle(&mut self) -> Result<(), Fault> {
        assert!(self.config.is_none() && !self.keyed);
        self.authorized()?;
        if !self.qualification.native_abi_verified {
            return Err(Fault::NativeUnverified);
        }
        if !self.qualification.feedback_topology_verified {
            return Err(Fault::TopologyUnverified);
        }
        self.lifecycle_deferred = true;
        Ok(())
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn lifecycle_backend(&self) -> Backend {
        self.backend
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn lifecycle_measurement_only(&mut self, enabled: bool) {
        self.measurement_only = enabled;
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn measurement_guard(&mut self) -> Result<(), Fault> {
        self.native.measurement_only_guard()
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn lifecycle_native(&mut self) -> &mut N {
        &mut self.native
    }
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    pub(super) fn lifecycle_native_view(&self) -> &N {
        &self.native
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn lifecycle_pause(&mut self) {
        self.clear_buffers();
    }
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub(super) fn lifecycle_native_clean(&mut self) -> Result<(), Fault> {
        if self.native.information()?[14] != 0 {
            return Err(Fault::NativeFailure);
        }
        self.native_clean = true;
        Ok(())
    }
    /// Explicit offline-only alternative. NOT an approved production gate.
    /// Production has no caller, no live-enable control, and no adapter wiring.
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    pub fn new_offline_admission_proposal(
        native: N,
        owner: Owner,
        backend: Backend,
        boot: u128,
        qualification: Qualification,
        capacity: usize,
        limits: super::admission_proposal::DraftLimits,
    ) -> Result<Self, Fault> {
        limits.validate()?;
        let mut controller = Self::new(native, owner, backend, boot, qualification, capacity);
        controller.draft_limits = Some(limits);
        Ok(controller)
    }
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    pub fn uses_offline_admission_proposal(&self) -> bool {
        self.draft_limits.is_some()
    }
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    pub fn raw_unclipped_qualification(&self) -> Option<Result<(), Fault>> {
        self.raw_qualification
    }
    #[cfg(feature = "offline-puresignal-policy-proposal")]
    pub fn observe_hazard(
        &mut self,
        event: super::admission_proposal::HazardObservation,
        now: u64,
    ) -> Result<(), Fault> {
        self.authorized()?;
        if event.fence != self.fence {
            return Err(Fault::GenerationMismatch);
        }
        if event.observed_ns > now {
            return Err(Fault::FutureObservation);
        }
        if !event.valid {
            return Err(Fault::ClipStatusUnknown);
        }
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if !self.keyed || event.observed_ns < self.keyed_at_ns {
            return Err(Fault::NotKeyed);
        }
        let adc = self.config.ok_or(Fault::InvalidConfig)?.physical_adc;
        let fault = match event.source {
            source if source == adc => Fault::AdcOverrange,
            OverloadSource::Ddc0Clamp | OverloadSource::Ddc1Clamp => Fault::DdcClamp,
            _ => return Err(Fault::WrongRoute),
        };
        // Do NOT require a fresh event to ignore an old-but-current-epoch hit.
        // Delayed positive evidence still resets; it cannot undo published IMD.
        self.latch_fault(fault);
        Err(fault)
    }
    #[cfg(test)]
    pub(super) fn test_backend(&self) -> Backend {
        self.backend
    }
    pub fn status(&self) -> Status {
        Status {
            fence: self.fence,
            state: self.state,
            fault: self.fault,
            keyed: self.keyed,
            info_raw: self.info,
            correction_applied_reported: self.state == State::Correcting
                && self.fault.is_none()
                && self.info[14] != 0,
            actual_delay_seconds: self.actual_delay,
            accepted_pairs: self.accepted_pairs,
            rejected_blocks: self.rejected_blocks,
            queue_depth: self.queued,
            queue_capacity: self.queue.len(),
        }
    }
    fn authorized(&self) -> Result<(), Fault> {
        match self.owner {
            Owner::Bridge => Ok(()),
            Owner::ExternalP2Client => Err(Fault::ExternalController),
            Owner::None => Err(Fault::NoController),
        }
    }
    pub fn configure(&mut self, expected: Fence, config: Config) -> Result<Fence, Fault> {
        self.authorized()?;
        if expected != self.fence {
            return Err(Fault::GenerationMismatch);
        }
        config.validate()?;
        if self.measurement_only && (config.enabled || config.automatic) {
            return Err(Fault::InvalidConfig);
        }
        if config.enabled {
            if !self.qualification.native_abi_verified {
                return Err(Fault::NativeUnverified);
            }
            if !self.qualification.feedback_topology_verified {
                return Err(Fault::TopologyUnverified);
            }
        }
        self.retire()?;
        self.config = Some(config);
        self.fence.config = config.id;
        self.fault = None;
        self.actual_delay = None;
        // Do not call ANY native API with an unidentified ABI, even to disable.
        if self.owns_verified_native() {
            match self.native.configure(config) {
                Ok(delay) if delay.is_finite() && delay.abs() <= 0.1 => {
                    self.actual_delay = Some(delay)
                }
                _ => {
                    self.latch_fault(Fault::NativeFailure);
                    return Err(Fault::NativeFailure);
                }
            }
            self.calibration_baseline = match self.native.information() {
                Ok(info) if info[5] >= 0 => info[5],
                _ => {
                    self.latch_fault(Fault::NativeFailure);
                    return Err(Fault::NativeFailure);
                }
            };
        }
        self.state = if config.enabled {
            State::Waiting
        } else {
            State::Off
        };
        // Config/calibrate/open/reconnect never synthesize an authorized key edge.
        Ok(self.fence)
    }
    /// Only the existing TX authority calls this. Every transition advances its
    /// epoch; old acquisition/queue fences are rejected after unkey/rekey.
    pub fn on_tx_state(&mut self, epoch: u64, keyed: bool, now_ns: u64) -> Result<Fence, Fault> {
        if epoch <= self.fence.tx_epoch {
            return Err(Fault::GenerationMismatch);
        }
        self.retire()?;
        self.fence.tx_epoch = epoch;
        self.keyed = keyed;
        self.keyed_at_ns = now_ns;
        if self.fault.is_some() {
            self.state = State::Fault;
            return Ok(self.fence);
        }
        let enabled = self.config.is_some_and(|config| config.enabled)
            && self.owner == Owner::Bridge
            && !self.measurement_only;
        self.state = if !enabled {
            State::Off
        } else if keyed {
            State::Settling
        } else {
            State::Waiting
        };
        if enabled
            && keyed
            && self.owns_verified_native()
            && self.qualification.feedback_topology_verified
        {
            self.adaptation_armed = true;
            self.native.mox(true);
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            {
                self.native_clean = false;
            }
        }
        if self.measurement_only && keyed {
            self.native.measurement_only_guard()?;
            self.adaptation_armed = true; // feedback/watchdog eligibility ONLY
            self.state = State::Collecting;
            // Deliberately never SetPSMox(true), SetPSControl or pscc.
        }
        Ok(self.fence)
    }
    fn owns_verified_native(&self) -> bool {
        self.owner == Owner::Bridge && self.qualification.native_abi_verified
    }
    fn clear_buffers(&mut self) {
        #[cfg(feature = "offline-puresignal-policy-proposal")]
        {
            self.partial_qualification = Ok(());
            self.raw_qualification = None;
        }
        self.head = 0;
        self.queued = 0;
        self.partial_pairs = 0;
        self.partial_first = None;
        self.partial_observed_ns = None;
        self.adaptation_armed = false;
        self.raw_interval = None;
        self.raw_observed_ns = None;
        self.next_pair = None;
        self.next_p2_sequence = None;
        self.last_observed_ns = None;
        self.info = [0; 16];
    }
    fn retire(&mut self) -> Result<(), Fault> {
        #[cfg(feature = "offline-puresignal-tx-lifecycle")]
        if self.lifecycle_deferred && !self.native_clean {
            return Err(Fault::CleanupPending);
        }
        let generation = self
            .fence
            .generation
            .checked_add(1)
            .ok_or(Fault::InvalidConfig)?;
        self.clear_buffers();
        self.fence.generation = generation;
        if self.owns_verified_native() {
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            if self.lifecycle_deferred {
                let info = self.native.information()?;
                if info[14] != 0 || info[5] < 0 {
                    return Err(Fault::NativeFailure);
                }
                self.calibration_baseline = info[5];
                return Ok(()); // cooperative owner already proved quiescence
            }
            self.native.mox(false);
            if self.native.reset().is_err() {
                self.fault = Some(Fault::NativeFailure);
                self.state = State::Fault;
                return Err(Fault::NativeFailure);
            }
            // Do not attribute an old applied curve to a newly accepted epoch.
            // Native reset is responsible for quiescence, not just a flag write.
            match self.native.information() {
                Ok(info) if info[14] == 0 && info[5] >= 0 => self.calibration_baseline = info[5],
                _ => {
                    self.fault = Some(Fault::NativeFailure);
                    self.state = State::Fault;
                    return Err(Fault::NativeFailure);
                }
            }
        }
        Ok(())
    }
    pub fn latch_fault(&mut self, fault: Fault) {
        self.clear_buffers();
        #[cfg(feature = "offline-puresignal-tx-lifecycle")]
        if self.lifecycle_deferred {
            self.native_clean = false;
            self.fault = Some(fault);
            self.state = State::Fault;
            return; // MUST NOT remove curves before TX-authority output barrier
        }
        let mut reset_failed = false;
        if self.owns_verified_native() {
            self.native.mox(false);
            reset_failed = self.native.reset().is_err();
        }
        self.fault = Some(if reset_failed {
            Fault::NativeFailure
        } else {
            fault
        });
        self.state = State::Fault;
        // PS bypass request only. No unkey/rekey here and NO RF-safe power claim.
        // Integration must implement the approved controlled-unkey/envelope policy.
    }
    fn reject(&mut self, fault: Fault, latch: bool) -> Result<(), Fault> {
        self.rejected_blocks = self.rejected_blocks.saturating_add(1);
        if latch {
            self.latch_fault(fault);
        }
        Err(fault)
    }
    fn check_meta(&self, meta: BlockMeta, pairs: usize, now_ns: u64) -> Result<(), Fault> {
        self.authorized()?;
        if meta.fence != self.fence {
            return Err(Fault::GenerationMismatch);
        }
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        let config = self.config.ok_or(Fault::InvalidConfig)?;
        if !self.keyed || !self.adaptation_armed || (!config.enabled && !self.measurement_only) {
            return Err(Fault::NotKeyed);
        }
        if now_ns < self.keyed_at_ns || meta.observed_ns > now_ns {
            return Err(Fault::FutureObservation);
        }
        // Acquisition layout is prepared before key receipt. A retained block
        // can therefore contain genuine pre-key observations even with the
        // matching predicted epoch. Reject it without adapting or retiming it;
        // it is not a clock violation and must not latch a false future fault.
        if meta.observed_ns < self.keyed_at_ns {
            return Err(Fault::NotKeyed);
        }
        if meta.observed_ns - self.keyed_at_ns < config.settling_ns {
            return Err(Fault::Settling);
        }
        if now_ns - meta.observed_ns > config.stale_after_ns {
            return Err(Fault::Stale);
        }
        if meta.backend != self.backend {
            return Err(Fault::WrongBackend);
        }
        if meta.rate_hz != RATE_HZ {
            return Err(Fault::WrongRate);
        }
        if meta.route != config.route {
            return Err(Fault::WrongRoute);
        }
        if pairs == 0
            || pairs > MAX_PAIRS
            || meta.interval.fence != meta.fence
            || meta.interval.end.checked_sub(meta.interval.first) != Some(pairs as u64)
        {
            return Err(Fault::UnequalLength);
        }
        let sources = [
            config.physical_adc,
            OverloadSource::Ddc0Clamp,
            OverloadSource::Ddc1Clamp,
        ];
        #[cfg(feature = "offline-puresignal-policy-proposal")]
        if self.measurement_only {
            // Unknown overload coverage stays unknown in the recorder. Known
            // current-epoch hits still stop; no fit-level threshold is invented.
            return super::admission_proposal::check_operational_evidence(
                meta,
                config.physical_adc,
                now_ns,
            );
        }
        #[cfg(feature = "offline-puresignal-policy-proposal")]
        if self.draft_limits.is_some() {
            return super::admission_proposal::check_operational_evidence(
                meta,
                config.physical_adc,
                now_ns,
            );
        }
        for (bracket, source) in meta.overload.into_iter().zip(sources) {
            let bracket = bracket.ok_or(Fault::ClipStatusUnknown)?;
            if bracket.source != source {
                return Err(Fault::WrongRoute);
            }
            bracket.certify(meta.interval, now_ns)?;
        }
        Ok(())
    }
    /// Bounded, nonblocking copy into a preallocated ring. An acquisition-thread
    /// integration must use a bounded handoff, not the existing unbounded TX
    /// command channel. That wiring is deliberately not part of this offline core.
    pub fn submit(&mut self, block: &PairBlock, now_ns: u64) -> Result<(), Fault> {
        if let Err(fault) = self.check_meta(block.meta, block.pairs, now_ns) {
            let latch = !matches!(
                fault,
                Fault::GenerationMismatch
                    | Fault::NotKeyed
                    | Fault::Settling
                    | Fault::NoController
                    | Fault::ExternalController
            );
            return self.reject(fault, latch);
        }
        if block.tx[..block.pairs * 2]
            .iter()
            .chain(&block.rf[..block.pairs * 2])
            .any(|x| !x.is_finite())
        {
            return self.reject(Fault::NonFinite, true);
        }
        if self.measurement_only && block.arrival_times_recorded {
            let arrivals = &block.arrival_ns[..block.pairs];
            if arrivals.iter().any(|at| *at > now_ns) {
                return self.reject(Fault::FutureObservation, true);
            }
            if arrivals.iter().copied().min() != Some(block.meta.observed_ns) {
                return self.reject(Fault::ObservationIncomplete, true);
            }
        }
        if let Some(next) = self.next_pair {
            if block.meta.interval.first != next {
                return self.reject(Fault::SequenceGap, true);
            }
        }
        if self.backend == Backend::P2 {
            let packets = block.meta.sequence_packets;
            if packets == 0 || packets as usize > super::adapters::P2_BATCH_PACKETS
                || (packets > 1 && block.pairs != packets as usize * super::adapters::P2_PAIR_COUNT)
            {
                return self.reject(Fault::WrongFormat, true);
            }
            let sequence = match block.meta.sequence {
                Some(value) => value,
                None => return self.reject(Fault::WrongFormat, true),
            };
            if let Some(next) = self.next_p2_sequence {
                if sequence != next {
                    let fault = if sequence == next.wrapping_sub(1) {
                        Fault::Duplicate
                    } else if sequence.wrapping_sub(next) < 0x8000_0000 {
                        Fault::SequenceGap
                    } else {
                        Fault::Reordered
                    };
                    return self.reject(fault, true);
                }
            }
            self.next_p2_sequence = Some(sequence.wrapping_add(packets as u32));
        } else if block.meta.sequence.is_some() || block.meta.sequence_packets != 0 {
            return self.reject(Fault::WrongFormat, true);
        }
        if self.queued == self.queue.len() {
            return self.reject(Fault::QueueOverrun, true);
        }
        let tail = (self.head + self.queued) % self.queue.len();
        self.queue[tail].copy_from(block);
        self.queued += 1;
        self.next_pair = Some(block.meta.interval.end);
        self.last_observed_ns = Some(block.meta.observed_ns);
        self.accepted_pairs = self.accepted_pairs.saturating_add(block.pairs as u64);
        Ok(())
    }
    /// Packet-independent deadline check. No queue consumption or native exchange.
    /// Also check the oldest queued block: recent arrivals cannot hide a stalled
    /// consumer whose pending observations have already expired.
    pub(super) fn check_deadline(&self, now_ns: u64) -> Result<(), Fault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if let (Some(oldest), Some(config)) = (self.partial_observed_ns, self.config) {
            if now_ns < oldest || now_ns - oldest > config.stale_after_ns {
                return Err(Fault::Stale);
            }
        }
        if self.queued != 0 {
            let oldest = self.queue[self.head].meta.observed_ns;
            if now_ns < oldest {
                return Err(Fault::FutureObservation);
            }
            if self
                .config
                .is_some_and(|c| now_ns - oldest > c.stale_after_ns)
            {
                return Err(Fault::Stale);
            }
        }
        if self.adaptation_armed
            && self.keyed
            && self
                .config
                .is_some_and(|config| config.enabled || self.measurement_only)
        {
            let config = self.config.unwrap();
            if now_ns < self.keyed_at_ns {
                return Err(Fault::FutureObservation);
            }
            if now_ns - self.keyed_at_ns >= config.settling_ns {
                let last = self
                    .last_observed_ns
                    .unwrap_or(self.keyed_at_ns.saturating_add(config.settling_ns));
                if now_ns.saturating_sub(last) > config.stale_after_ns {
                    return Err(Fault::Stale);
                }
            }
        }
        Ok(())
    }
    /// Called only on the serialized native owner, NOT on the DMA/network thread.
    pub fn pump(&mut self, now_ns: u64) -> Result<usize, Fault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if let Err(fault) = self.check_deadline(now_ns) {
            return self.reject(fault, true).map(|_| 0);
        }
        let mut processed = 0;
        while self.queued != 0 {
            let meta = self.queue[self.head].meta;
            let pairs = self.queue[self.head].pairs;
            if let Err(fault) = self.check_meta(meta, pairs, now_ns) {
                return self.reject(fault, true).map(|_| 0);
            }
            if self.measurement_only {
                if let Err(fault) = self.native.measurement_only_guard() {
                    return self.reject(fault, true).map(|_| 0);
                }
                self.head = (self.head + 1) % self.queue.len();
                self.queued -= 1;
                // Raw pairs are retained by lifecycle before admission, not
                // transformed by delay filters or submitted to the fitter.
                return Ok(1);
            }
            let mut consumed = 0;
            while consumed < pairs {
                if self.partial_pairs == 0 {
                    self.partial_first = Some(meta.interval.first + consumed as u64);
                    self.partial_observed_ns = Some(meta.observed_ns);
                }
                let take = (NATIVE_PAIRS - self.partial_pairs).min(pairs - consumed);
                let dest = self.partial_pairs * 2;
                let source = consumed * 2;
                self.tx[dest..dest + take * 2]
                    .copy_from_slice(&self.queue[self.head].tx[source..source + take * 2]);
                self.rf[dest..dest + take * 2]
                    .copy_from_slice(&self.queue[self.head].rf[source..source + take * 2]);
                #[cfg(feature = "offline-puresignal-policy-proposal")]
                {
                    let slice_interval = Interval {
                        fence: self.fence,
                        first: meta.interval.first + consumed as u64,
                        end: meta.interval.first + (consumed + take) as u64,
                    };
                    self.partial_qualification = self.partial_qualification.and(
                        super::admission_proposal::certify_measurement(
                            meta,
                            slice_interval,
                            self.config.unwrap().physical_adc,
                            now_ns,
                        ),
                    );
                }
                self.partial_pairs += take;
                consumed += take;
                if self.partial_pairs == NATIVE_PAIRS {
                    #[cfg(feature = "offline-puresignal-policy-proposal")]
                    if let Some(limits) = self.draft_limits {
                        if let Err(fault) = limits.check_window(
                            &self.tx[..],
                            &self.rf[..],
                            self.config.unwrap().hardware_peak,
                        ) {
                            return self.reject(fault, true).map(|_| 0);
                        }
                    }
                    // Save raw RF before pscc's delay filters mutate BOTH arrays.
                    self.raw_rf.copy_from_slice(&self.rf[..]);
                    self.raw_interval = Some(Interval {
                        fence: self.fence,
                        first: self.partial_first.unwrap(),
                        end: meta.interval.first + consumed as u64,
                    });
                    self.raw_observed_ns = self.partial_observed_ns;
                    #[cfg(feature = "offline-puresignal-policy-proposal")]
                    {
                        self.raw_qualification = Some(self.partial_qualification);
                        self.partial_qualification = Ok(());
                    }
                    if self.native.process(&mut self.tx, &mut self.rf).is_err() {
                        return self.reject(Fault::NativeFailure, true).map(|_| 0);
                    }
                    self.info = match self.native.information() {
                        Ok(info) => info,
                        Err(_) => return self.reject(Fault::NativeFailure, true).map(|_| 0),
                    };
                    if self.info[6] & 2 != 0 {
                        return self.reject(Fault::SevereOverdrive, true).map(|_| 0);
                    }
                    if self.info[6] & 1 != 0 {
                        return self.reject(Fault::SolutionRejected, true).map(|_| 0);
                    }
                    self.state = if self.info[14] != 0 && self.info[5] > self.calibration_baseline {
                        State::Correcting
                    } else {
                        State::Collecting
                    };
                    self.partial_pairs = 0;
                    self.partial_observed_ns = None;
                    processed += 1;
                }
            }
            self.head = (self.head + 1) % self.queue.len();
            self.queued -= 1;
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            if self.lifecycle_deferred {
                break;
            } // release gets control between bounded blocks
        }
        Ok(processed)
    }
    /// Complete raw window only; caller copies to a bounded analyzer mailbox.
    /// No atomic native solution/capture association is implied by this window.
    pub fn raw_capture(&self) -> Option<(Interval, u64, &[f64])> {
        Some((self.raw_interval?, self.raw_observed_ns?, &self.raw_rf[..]))
    }
}
impl<N: NativePs> Drop for Controller<N> {
    fn drop(&mut self) {
        #[cfg(feature = "offline-puresignal-tx-lifecycle")]
        if self.lifecycle_deferred {
            return;
        } // lifecycle owner orders cleanup
          // MOX off before engine/channel destruction. Unknown ABI is never called.
        if self.owns_verified_native() {
            self.native.mox(false);
            let _ = self.native.reset();
        }
    }
}
