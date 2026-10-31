//! TX-authority contract and cooperative state machine. Offline feature only;
//! no production caller exists. Offline stop-only owner adapters are separate;
//! this state machine never owns RF registers.
use super::{admission_proposal::HazardObservation, controller::*, *};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupProgress {
    Pending,
    Complete,
}

/// One native owner, no second WDSP thread. Each operation performs at most ONE
/// DSP exchange and returns; no reset loop, sleep, join or channel destruction.
/// Complete means no async fit/old IQC work remains and IQC run/busy are both 0.
pub trait CooperativeNative: NativePs {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault>;
    fn begin_cleanup(&mut self) -> Result<(), Fault>;
    fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCause {
    Release,
    Fault(Fault),
    Reconfiguration,
    Shutdown,
}
#[derive(Clone, Copy, Debug)]
pub struct StopRequest {
    pub id: u64,
    pub backend: Backend,
    pub fence: Fence,
    pub cause: StopCause,
    /// Controller notification, NOT hardware overload onset or browser receipt.
    pub notified_ns: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopEvidence {
    /// Local producer barrier plus completed tx=false/drive-zero command burst.
    /// UDP transmission completion is NOT RF-off readback/acknowledgment.
    P2CommandBurstOnly,
    /// Local producer barrier plus source-qualified direct control readback.
    /// Control readback is NOT an external RF power measurement.
    DirectControlReadback,
}
#[derive(Clone, Copy, Debug)]
pub struct StopReceipt {
    pub id: u64,
    pub output_barrier_closed: bool,
    pub in_flight_writes_drained: bool,
    pub queued_output_discarded: bool,
    pub unkey_operation_completed: bool,
    pub evidence: StopEvidence,
}
#[derive(Clone, Copy, Debug)]
pub struct FreshTxAction {
    /// Existing TX authority must authenticate this as a NEW operator action.
    pub sequence: u64,
    pub release_ticket: u64,
    pub expected_fence: Fence,
}
#[derive(Clone, Copy, Debug)]
pub struct KeyReceipt {
    pub action_sequence: u64,
    pub output_generation: u64,
    pub keyed_ns: u64,
}

/// Implement on the EXISTING TX authority, not the DSP/fitter or poller. All
/// methods must be nonblocking progress operations. begin_stop fences pending
/// arms/new writes immediately; host-requested native cleanup waits for receipt.
/// Completion requires queued IQ discarded, in-flight writes drained, and the
/// backend unkey/drive-disable operation complete. Keep output inhibited until
/// a new authenticated action, approved drive policy and normal mic/IQ gates.
pub trait TxAuthority {
    /// Managed owners may share their atomic output-inhibition/cancellation
    /// state with this mailbox. Default offline authorities retain a counter.
    fn release_mailbox(&self) -> Option<ReleaseHandle> {
        None
    }
    fn release_received(&mut self, ticket: u64, notified_ns: u64) -> Result<(), Fault>;
    fn begin_stop(&mut self, request: StopRequest) -> Result<(), Fault>;
    fn poll_stop(&mut self, id: u64) -> Result<Option<StopReceipt>, Fault>;
    fn begin_arm(&mut self, action: FreshTxAction, output_generation: u64) -> Result<(), Fault>;
    fn poll_key(&mut self, now_ns: u64) -> Result<Option<KeyReceipt>, Fault>;
    /// Sole lifecycle notification after native quiescence/config retirement.
    /// It does not authorize TX. Owner adapters invalidate all pre-cleanup arms.
    fn cleanup_complete(&mut self, _stop_id: u64, _fence: Fence) -> Result<(), Fault> {
        Ok(())
    }
    /// Nonblocking owner revocation check, including disconnected/changed
    /// operator identity. Periodic service must not require another packet.
    fn poll_health(&mut self) -> Result<(), Fault> {
        Ok(())
    }
}

/// Out-of-band, bounded release mailbox; never behind feedback/audio commands.
/// Posting does not wait for DSP, fitting/reset, queue space or the native lock.
pub trait ReleaseMailbox: Send + Sync {
    fn current(&self) -> u64;
    fn release(&self) -> u64;
}
struct CounterRelease(AtomicU64);
impl ReleaseMailbox for CounterRelease {
    fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn release(&self) -> u64 {
        let old = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.saturating_add(1))
            })
            .unwrap();
        old.saturating_add(1)
    }
}
#[derive(Clone)]
pub struct ReleaseHandle(Arc<dyn ReleaseMailbox>);
impl Default for ReleaseHandle {
    fn default() -> Self {
        Self(Arc::new(CounterRelease(AtomicU64::new(0))))
    }
}
impl ReleaseHandle {
    pub fn from_mailbox(mailbox: Arc<dyn ReleaseMailbox>) -> Self {
        Self(mailbox)
    }
    pub fn current(&self) -> u64 {
        self.0.current()
    }
    pub fn release(&self) -> u64 {
        self.0.release()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    AwaitingAuthority,
    CleaningNative,
    ReadyOff,
    AwaitingKey,
    Active,
    Blocked,
}
/// MeasurementOnly is a distinct operator action, never a disabled correction
/// switch or an ordinary arm converted at dequeue. Duration starts at action
/// admission (including key preparation), never refreshed by feedback arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Correction,
    MeasurementOnly,
}
pub const MAX_MEASUREMENT_DURATION_NS: u64 = 60_000_000_000;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Explicit integration limits; deliberately no production defaults.
    pub authority_timeout_ns: u64,
    pub cleanup_timeout_ns: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct LifecycleStatus {
    pub mode: RunMode,
    pub measurement_deadline_ns: Option<u64>,
    pub phase: Phase,
    pub primary_fault: Option<Fault>,
    pub cleanup_fault: Option<Fault>,
    pub authority_fault: Option<Fault>,
    pub stop_id: u64,
    pub release_handled: u64,
    pub required_release: u64,
    pub notified_ns: Option<u64>,
    pub authority_completed_ns: Option<u64>,
    pub cleanup_started_ns: Option<u64>,
    pub cleanup_completed_ns: Option<u64>,
    pub cleanup_steps: u64,
    pub stop_evidence: Option<StopEvidence>,
    /// Unknown during stop/cleanup; never infer native curve removal merely
    /// because the logical controller has stopped admitting feedback.
    pub correction_applied: Option<bool>,
    pub last_known_applied_before_stop: bool,
}

pub struct TxLifecycle<N: CooperativeNative, A: TxAuthority> {
    measurement: Option<super::measurement::MeasurementRecorder>,
    controller: Option<Controller<N>>,
    authority: A,
    backend: Backend,
    limits: Limits,
    release: ReleaseHandle,
    status: LifecycleStatus,
    now: u64,
    ptt_requested: bool,
    current_config: Option<Config>,
    pending_config: Option<Config>,
    last_action: u64,
    pending_action: Option<FreshTxAction>,
    shutdown_requested: bool,
}
impl<N: CooperativeNative, A: TxAuthority> TxLifecycle<N, A> {
    pub fn new(
        mut controller: Controller<N>,
        authority: A,
        backend: Backend,
        limits: Limits,
    ) -> Result<Self, Fault> {
        if limits.authority_timeout_ns == 0
            || limits.cleanup_timeout_ns == 0
            || controller.lifecycle_backend() != backend
        {
            return Err(Fault::InvalidConfig);
        }
        controller.attach_lifecycle()?;
        let release = authority.release_mailbox().unwrap_or_default();
        Ok(Self {
            measurement: None,
            controller: Some(controller),
            authority,
            backend,
            limits,
            release,
            now: 0,
            ptt_requested: false,
            current_config: None,
            pending_config: None,
            last_action: 0,
            pending_action: None,
            shutdown_requested: false,
            status: LifecycleStatus {
                mode: RunMode::Correction,
                measurement_deadline_ns: None,
                phase: Phase::ReadyOff,
                primary_fault: None,
                cleanup_fault: None,
                authority_fault: None,
                stop_id: 0,
                release_handled: 0,
                required_release: 0,
                notified_ns: None,
                authority_completed_ns: None,
                cleanup_started_ns: None,
                cleanup_completed_ns: None,
                cleanup_steps: 0,
                stop_evidence: None,
                correction_applied: None,
                last_known_applied_before_stop: false,
            },
        })
    }
    fn c(&self) -> &Controller<N> {
        self.controller.as_ref().unwrap()
    }
    fn c_mut(&mut self) -> &mut Controller<N> {
        self.controller.as_mut().unwrap()
    }
    pub fn release_handle(&self) -> ReleaseHandle {
        self.release.clone()
    }
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    pub fn offline_owner(&mut self) -> &mut A {
        &mut self.authority
    }
    /// Read-only native audit/telemetry. No mutable port and no DSP budget is
    /// granted, including while stop receipt/native cleanup is progressing.
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    pub fn offline_native_snapshot<R>(&self, read: impl FnOnce(&N) -> R) -> R {
        read(self.c().lifecycle_native_view())
    }
    /// Offline sole-owner TX exchange. Release checked before AND after it;
    /// output still needs its generation permit at the final writer.
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    pub fn offline_tx_exchange<R>(
        &mut self,
        now: u64,
        exchange: impl FnOnce(&mut N) -> Result<R, Fault>,
    ) -> Result<R, Fault> {
        self.clock(now)?;
        self.service_release()?;
        if !matches!(self.status.phase, Phase::AwaitingKey | Phase::Active) {
            return Err(Fault::CleanupPending);
        }
        if self.status.mode == RunMode::MeasurementOnly {
            if let Err(fault) = self.c_mut().measurement_guard() {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        }
        let output = match exchange(self.c_mut().lifecycle_native()) {
            Ok(output) => output,
            Err(fault) => {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        };
        self.service_release()?;
        if self.status.mode == RunMode::MeasurementOnly
            && matches!(self.status.phase, Phase::AwaitingKey | Phase::Active)
        {
            if let Err(fault) = self.c_mut().measurement_guard() {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        }
        if !matches!(self.status.phase, Phase::AwaitingKey | Phase::Active) {
            return Err(Fault::CleanupPending);
        }
        Ok(output)
    }
    /// Configuration-only operation after quiescence. Must perform ZERO DSP
    /// exchanges and never key. Release is serviced on either side of the call.
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    pub fn offline_clean_configure(
        &mut self,
        now: u64,
        configure: impl FnOnce(&mut N) -> Result<(), Fault>,
    ) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if self.status.phase != Phase::ReadyOff || self.status.primary_fault.is_some() {
            return Err(Fault::CleanupPending);
        }
        if let Err(fault) = configure(self.c_mut().lifecycle_native()) {
            self.stop(StopCause::Fault(fault))?;
            return Err(fault);
        }
        self.service_release()?;
        if self.status.phase != Phase::ReadyOff {
            return Err(Fault::CleanupPending);
        }
        Ok(())
    }
    pub fn fence(&self) -> Fence {
        self.c().fence()
    }
    pub fn feedback_status(&self) -> Status {
        self.c().status()
    }
    pub fn status(&self) -> LifecycleStatus {
        let mut s = self.status;
        s.correction_applied = if s.phase == Phase::Active {
            Some(self.c().status().correction_applied_reported)
        } else if s.phase == Phase::ReadyOff && s.cleanup_completed_ns.is_some() {
            Some(false)
        } else {
            None
        };
        s
    }
    pub fn raw_unclipped_qualification(&self) -> Option<Result<(), Fault>> {
        if self.status.phase == Phase::Active {
            self.c().raw_unclipped_qualification()
        } else {
            None
        }
    }
    fn clock(&mut self, now: u64) -> Result<(), Fault> {
        if now < self.now {
            self.service_release()?;
            self.stop(StopCause::Fault(Fault::FutureObservation))?;
            return Err(Fault::FutureObservation);
        }
        self.now = now;
        Ok(())
    }
    fn service_release(&mut self) -> Result<(), Fault> {
        let ticket = self.release.current();
        if ticket != self.status.release_handled {
            self.status.release_handled = ticket;
            self.ptt_requested = false;
            self.pending_action = None;
            self.pending_config = None; // queued configuration is canceled too
            if let Err(fault) = self.authority.release_received(ticket, self.now) {
                self.status.authority_fault = Some(fault);
            }
            self.stop(StopCause::Release)?;
            if ticket == u64::MAX {
                self.status.phase = Phase::Blocked;
                return Err(Fault::InvalidConfig);
            }
        }
        Ok(())
    }
    fn stop(&mut self, cause: StopCause) -> Result<(), Fault> {
        self.status.measurement_deadline_ns = None;
        let applied = self.c().status().correction_applied_reported
            || self.status.last_known_applied_before_stop;
        if let StopCause::Fault(fault) = cause {
            if let Some(recorder) = &mut self.measurement {
                recorder.note_fault(fault);
            }
            self.pending_config = None;
            self.status.primary_fault.get_or_insert(fault);
            self.c_mut().latch_fault(fault); // logical latch ONLY in this attached controller
        }
        self.status.required_release = self.status.required_release.max(
            self.status
                .release_handled
                .saturating_add(u64::from(self.ptt_requested)),
        );
        self.pending_action = None;
        if matches!(
            self.status.phase,
            Phase::AwaitingAuthority | Phase::CleaningNative | Phase::Blocked
        ) {
            return Ok(()); // do not restart cleanup or erase the primary fault
        }
        self.status.last_known_applied_before_stop = applied;
        self.c_mut().lifecycle_pause();
        let exhausted = self.status.stop_id == u64::MAX;
        self.status.stop_id = self.status.stop_id.saturating_add(1);
        self.status.notified_ns = Some(self.now);
        self.status.authority_completed_ns = None;
        self.status.cleanup_started_ns = None;
        self.status.cleanup_completed_ns = None;
        self.status.cleanup_steps = 0;
        self.status.stop_evidence = None;
        self.status.phase = Phase::AwaitingAuthority;
        let request = StopRequest {
            id: self.status.stop_id,
            backend: self.backend,
            fence: self.fence(),
            cause,
            notified_ns: self.now,
        };
        if let Err(fault) = self.authority.begin_stop(request) {
            self.status.authority_fault = Some(fault);
            self.status.phase = Phase::Blocked;
            return Err(fault);
        }
        if exhausted {
            self.status.authority_fault = Some(Fault::InvalidConfig);
            self.status.phase = Phase::Blocked;
            return Err(Fault::InvalidConfig); // barrier requested, never reuse exhausted receipt ID
        }
        Ok(())
    }
    pub fn notify_fault(&mut self, fault: Fault, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        self.stop(StopCause::Fault(fault))
    }
    pub fn reconfigure(&mut self, expected: Fence, config: Config, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if expected != self.fence() {
            return Err(Fault::GenerationMismatch);
        }
        if self.status.mode == RunMode::MeasurementOnly
            && matches!(self.status.phase, Phase::Active | Phase::AwaitingKey)
        {
            // Even invalid configuration is cancellation, not an opportunity
            // to leave diagnostic output active while rejecting a setting.
            self.stop(StopCause::Reconfiguration)?;
            return Err(Fault::ConfigChanged);
        }
        if self.status.phase == Phase::ReadyOff {
            self.status.mode = RunMode::Correction;
            self.c_mut().lifecycle_measurement_only(false);
        }
        config.validate()?;
        if self.status.phase == Phase::Blocked || self.shutdown_requested {
            return Err(Fault::CleanupPending);
        }
        // RX-only updates after a COMPLETED stop do not need another native
        // teardown. In particular, absolute VFO/DDC changes must not recreate
        // the full TX engine for every knob step. This is deliberately limited
        // to bypass -> bypass with no retained measurement or fault. Active,
        // fitting, initial-startup and uncertain states keep stop-before-cleanup.
        if self.status.phase == Phase::ReadyOff
            && self.status.cleanup_completed_ns.is_some()
            && self.status.primary_fault.is_none()
            && self.status.authority_fault.is_none()
            && self.status.cleanup_fault.is_none()
            && self.measurement.is_none()
            && self.pending_action.is_none()
            && self
                .current_config
                .is_some_and(|c| !c.enabled && !c.automatic)
            && !config.enabled
            && !config.automatic
        {
            let fence = self.fence();
            if let Err(fault) = self.c_mut().configure(fence, config) {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
            self.current_config = Some(config);
            // Retire old arm credentials even though the existing stop proof
            // remains valid. This never opens the output fence or keys RF.
            let fence = self.fence();
            if let Err(fault) = self.authority.cleanup_complete(self.status.stop_id, fence) {
                self.status.authority_fault = Some(fault);
                self.status.phase = Phase::Blocked;
                return Err(fault);
            }
            self.service_release()?;
            return Ok(());
        }
        self.pending_config = Some(config); // one bounded slot, never a command backlog
        self.stop(StopCause::Reconfiguration)
    }
    pub fn fresh_tx_action(&mut self, action: FreshTxAction, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if self.status.mode == RunMode::MeasurementOnly {
            return Err(Fault::InvalidConfig);
        }
        self.begin_fresh_action(action, now, false)
    }
    /// Explicit uncorrected TX: same fresh authority and stop barriers, with
    /// native fitting/correction verified disabled before opening output.
    pub fn fresh_bypass_tx_action(&mut self, action: FreshTxAction, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if self.status.mode == RunMode::MeasurementOnly
            || !self
                .current_config
                .is_some_and(|c| !c.enabled && !c.automatic)
        {
            return Err(Fault::InvalidConfig);
        }
        self.c_mut().measurement_guard()?;
        self.begin_fresh_action(action, now, true)
    }
    pub fn fresh_measurement_action(
        &mut self,
        action: FreshTxAction,
        duration_ns: u64,
        recorder: super::measurement::MeasurementRecorder,
        now: u64,
    ) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if duration_ns == 0
            || duration_ns > MAX_MEASUREMENT_DURATION_NS
            || self.measurement.is_some()
            || !self
                .current_config
                .is_some_and(|c| !c.enabled && !c.automatic)
            || self.status.phase != Phase::ReadyOff
        {
            return Err(Fault::InvalidConfig);
        }
        let deadline = now.checked_add(duration_ns).ok_or(Fault::InvalidConfig)?;
        self.c_mut().measurement_guard()?;
        // Do not install the mode until the supplied fresh authority passes.
        self.begin_fresh_action(action, now, true)?;
        self.measurement = Some(recorder);
        self.status.mode = RunMode::MeasurementOnly;
        self.status.measurement_deadline_ns = Some(deadline);
        self.c_mut().lifecycle_measurement_only(true);
        Ok(())
    }
    fn begin_fresh_action(
        &mut self,
        action: FreshTxAction,
        now: u64,
        disabled_native_verified: bool,
    ) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if self.shutdown_requested
            || self.status.phase != Phase::ReadyOff
            || self.current_config.is_none()
            || self.status.primary_fault.is_some()
            || self.status.cleanup_fault.is_some()
            || self.status.authority_fault.is_some()
            || self.status.cleanup_completed_ns.is_none()
        {
            return Err(Fault::CleanupPending);
        }
        if !disabled_native_verified && !self.current_config.unwrap().enabled {
            return Err(Fault::InvalidConfig);
        }
        if action.expected_fence != self.fence()
            || action.sequence <= self.last_action
            || action.release_ticket != self.status.release_handled
            || action.release_ticket != self.release.current()
            || action.release_ticket < self.status.required_release
        {
            return Err(Fault::GenerationMismatch);
        }
        // Authority still enforces its authenticated action, safe drive/mic/IQ,
        // backend readiness and owner-generation checks. This is NOT RF keying.
        self.last_action = action.sequence;
        self.ptt_requested = true;
        if let Err(fault) = self.authority.begin_arm(action, self.status.stop_id) {
            // Conditional reopen can refuse a cancellation that raced the
            // preceding check. Normalize only that pre-I/O refusal; a genuine
            // authority/backend failure remains latched even with a release.
            if fault == Fault::GenerationMismatch && self.release.current() != action.release_ticket
            {
                self.service_release()?;
                return Err(Fault::GenerationMismatch);
            }
            self.stop(StopCause::Fault(fault))?;
            return Err(fault);
        }
        self.pending_action = Some(action);
        self.status.phase = Phase::AwaitingKey;
        self.status.notified_ns = Some(now);
        Ok(())
    }
    pub fn acknowledge_fault(&mut self, stop_id: u64, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if stop_id != self.status.stop_id
            || self.status.phase != Phase::ReadyOff
            || self.status.cleanup_fault.is_some()
            || self.status.authority_fault.is_some()
            || self.status.release_handled < self.status.required_release
        {
            return Err(Fault::CleanupPending);
        }
        let config = self.current_config.ok_or(Fault::InvalidConfig)?;
        let release = self.release.current();
        let fence = self.fence();
        self.c_mut().configure(fence, config)?;
        if self.release.current() != release {
            self.service_release()?;
            return Err(Fault::GenerationMismatch);
        }
        let fence = self.fence();
        if let Err(fault) = self.authority.cleanup_complete(self.status.stop_id, fence) {
            self.status.authority_fault = Some(fault);
            self.status.phase = Phase::Blocked;
            return Err(fault);
        }
        self.status.primary_fault = None; // acknowledges ONLY, never arms or keys
        Ok(())
    }
    pub fn observe_hazard(&mut self, event: HazardObservation, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        self.remember_applied();
        let result = self.c_mut().observe_hazard(event, now);
        if let Some(fault) = self.c().status().fault {
            self.stop(StopCause::Fault(fault))?;
        }
        result
    }
    pub fn submit(&mut self, block: &PairBlock, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        let guard =
            if self.status.mode == RunMode::MeasurementOnly && self.status.phase == Phase::Active {
                self.c_mut().measurement_guard()
            } else {
                Err(Fault::CleanupPending)
            };
        self.remember_applied();
        let result = if self.status.phase != Phase::Active {
            Err(Fault::CleanupPending)
        } else if self.status.mode == RunMode::MeasurementOnly && guard.is_err() {
            guard
        } else {
            self.c_mut().submit(block, now)
        };
        // Retention occurs before acting on rejection/stop and includes old
        // generation, settling, clipped and malformed observations unchanged.
        if let Some(recorder) = self
            .measurement
            .as_mut()
            .filter(|_| self.status.mode == RunMode::MeasurementOnly)
        {
            let state = super::measurement::NativeMeasurementState {
                fitting_enabled: false,
                automatic_enabled: false,
                correction_applied: false,
                evidence_verified: guard.is_ok(),
            };
            if let Err(fault) = recorder.record(block, now, state, result) {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        }
        if self.status.phase == Phase::Active && self.status.mode == RunMode::MeasurementOnly {
            if let Err(fault) = guard {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        }
        if let Some(fault) = self.c().status().fault {
            self.stop(StopCause::Fault(fault))?;
        }
        result
    }
    pub fn measurement_records(&self) -> Option<&super::measurement::MeasurementRecorder> {
        self.measurement.as_ref()
    }
    /// No evidence extraction can race native/output ownership. The record
    /// store is not silently overwritten by an automatic subsequent run.
    pub fn take_measurement_records(
        &mut self,
    ) -> Result<Option<super::measurement::MeasurementRecorder>, Fault> {
        if self.status.phase != Phase::ReadyOff || self.status.cleanup_completed_ns.is_none() {
            return Err(Fault::CleanupPending);
        }
        if let Some(recorder) = &mut self.measurement {
            recorder.finish(
                self.status
                    .primary_fault
                    .or(self.status.cleanup_fault)
                    .or(self.status.authority_fault),
                self.status.cleanup_completed_ns,
            );
        }
        Ok(self.measurement.take())
    }
    pub fn pump(&mut self, now: u64) -> Result<usize, Fault> {
        self.clock(now)?;
        self.service_release()?;
        if self.status.phase != Phase::Active {
            return Err(Fault::CleanupPending);
        }
        self.remember_applied();
        let result = self.c_mut().pump(now);
        if let Some(fault) = self.c().status().fault {
            self.stop(StopCause::Fault(fault))?;
        }
        result
    }
    fn remember_applied(&mut self) {
        if self.status.phase == Phase::Active {
            self.status.last_known_applied_before_stop =
                self.c().status().correction_applied_reported;
        }
    }
    fn native_failed(&mut self, fault: Fault) -> Result<(), Fault> {
        self.status.cleanup_fault = Some(fault);
        self.status.phase = Phase::Blocked;
        Err(fault)
    }
    /// One control quantum, release FIRST, then at most one DSP exchange.
    /// Caller continues invoking this through unkey/cleanup; never stop native
    /// DSP or CloseChannel while a fit/IQC transition may still be waiting.
    pub fn tick(&mut self, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        if matches!(self.status.phase, Phase::AwaitingKey | Phase::Active)
            && self.status.mode == RunMode::MeasurementOnly
        {
            if self
                .status
                .measurement_deadline_ns
                .is_some_and(|deadline| now >= deadline)
            {
                self.ptt_requested = false;
                self.stop(StopCause::Release)?;
                return Ok(());
            }
            if let Err(fault) = self.c_mut().measurement_guard() {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
        }
        if matches!(self.status.phase, Phase::AwaitingKey | Phase::Active) {
            if let Err(fault) = self.authority.poll_health() {
                self.stop(StopCause::Fault(fault))?;
                return Err(fault);
            }
            // A coherent cancellation can arrive after the initial mailbox
            // read. Owner health recognizes that exact cancellation, not a
            // generic authorization failure. Service it before key/native work.
            self.service_release()?;
        }
        match self.status.phase {
            Phase::AwaitingAuthority => {
                if now.saturating_sub(self.status.notified_ns.unwrap())
                    > self.limits.authority_timeout_ns
                {
                    self.status.authority_fault = Some(Fault::TxAuthority);
                    self.status.phase = Phase::Blocked;
                    return Err(Fault::TxAuthority);
                }
                let receipt = match self.authority.poll_stop(self.status.stop_id) {
                    Ok(value) => value,
                    Err(fault) => {
                        self.status.authority_fault = Some(fault);
                        self.status.phase = Phase::Blocked;
                        return Err(fault);
                    }
                };
                if let Some(receipt) = receipt {
                    if receipt.id != self.status.stop_id {
                        return Ok(());
                    } // obsolete ack cannot clean
                    let source_matches = matches!(
                        (self.backend, receipt.evidence),
                        (Backend::P2, StopEvidence::P2CommandBurstOnly)
                            | (Backend::DirectXdma, StopEvidence::DirectControlReadback)
                    );
                    if !receipt.output_barrier_closed
                        || !receipt.in_flight_writes_drained
                        || !receipt.queued_output_discarded
                        || !receipt.unkey_operation_completed
                        || !source_matches
                    {
                        self.status.authority_fault = Some(Fault::TxAuthority);
                        self.status.phase = Phase::Blocked;
                        return Err(Fault::TxAuthority);
                    }
                    self.status.authority_completed_ns = Some(now);
                    self.status.stop_evidence = Some(receipt.evidence);
                    self.status.cleanup_started_ns = Some(now);
                    self.status.phase = Phase::CleaningNative;
                    if let Err(fault) = self.c_mut().lifecycle_native().begin_cleanup() {
                        return self.native_failed(fault);
                    }
                } else {
                    if let Err(fault) = self.c_mut().lifecycle_native().advance_dsp_discard() {
                        return self.native_failed(fault);
                    }
                }
            }
            Phase::CleaningNative => {
                if now.saturating_sub(self.status.cleanup_started_ns.unwrap())
                    > self.limits.cleanup_timeout_ns
                {
                    return self.native_failed(Fault::CleanupTimeout);
                }
                self.status.cleanup_steps += 1;
                let progress = match self.c_mut().lifecycle_native().cleanup_step() {
                    Ok(p) => p,
                    Err(fault) => return self.native_failed(fault),
                };
                // A release during that bounded native step cancels the staged
                // configuration before cleanup completion can install it.
                self.service_release()?;
                if progress == CleanupProgress::Complete {
                    if let Err(fault) = self.c_mut().lifecycle_native_clean() {
                        return self.native_failed(fault);
                    }
                    let Some(epoch) = self.fence().tx_epoch.checked_add(1) else {
                        return self.native_failed(Fault::InvalidConfig);
                    };
                    if let Err(fault) = self.c_mut().on_tx_state(epoch, false, now) {
                        return self.native_failed(fault);
                    }
                    if let Some(config) = self.pending_config.take() {
                        let fence = self.fence();
                        if let Err(fault) = self.c_mut().configure(fence, config) {
                            return self.native_failed(fault);
                        }
                        self.current_config = Some(config);
                    }
                    self.status.cleanup_completed_ns = Some(now);
                    let fence = self.fence();
                    if let Err(fault) = self.authority.cleanup_complete(self.status.stop_id, fence)
                    {
                        self.status.authority_fault = Some(fault);
                        self.status.phase = Phase::Blocked;
                        return Err(fault);
                    }
                    self.status.phase = Phase::ReadyOff;
                    self.ptt_requested = false;
                }
            }
            Phase::AwaitingKey => {
                if now.saturating_sub(self.status.notified_ns.unwrap())
                    > self.limits.authority_timeout_ns
                {
                    self.stop(StopCause::Fault(Fault::TxAuthority))?;
                    return Err(Fault::TxAuthority);
                }
                let receipt = match self.authority.poll_key(now) {
                    Ok(value) => value,
                    Err(fault) => {
                        self.stop(StopCause::Fault(fault))?;
                        return Err(fault);
                    }
                };
                if let Some(receipt) = receipt {
                    let action = self.pending_action.unwrap();
                    if receipt.action_sequence != action.sequence
                        || receipt.output_generation != self.status.stop_id
                    {
                        return Ok(());
                    }
                    if receipt.keyed_ns > now {
                        self.stop(StopCause::Fault(Fault::FutureObservation))?;
                        return Err(Fault::FutureObservation);
                    }
                    self.service_release()?;
                    if self.status.phase != Phase::AwaitingKey {
                        return Ok(());
                    }
                    let Some(epoch) = self.fence().tx_epoch.checked_add(1) else {
                        self.stop(StopCause::Fault(Fault::InvalidConfig))?;
                        return Err(Fault::InvalidConfig);
                    };
                    if let Err(fault) = self.c_mut().on_tx_state(epoch, true, receipt.keyed_ns) {
                        self.stop(StopCause::Fault(fault))?;
                        return Err(fault);
                    }
                    self.status.phase = Phase::Active;
                    self.status.last_known_applied_before_stop = false;
                }
            }
            Phase::Blocked => {
                // No host-requested correction removal or native destruction.
                // Inhibition was requested, not necessarily physically verified
                // if authority failed. Fit ACK DSP can still advance/discard.
                if let Err(fault) = self.c_mut().lifecycle_native().advance_dsp_discard() {
                    return self.native_failed(fault);
                }
            }
            Phase::Active => {
                self.remember_applied();
                if let Err(fault) = self.c().check_deadline(now) {
                    self.stop(StopCause::Fault(fault))?;
                    return Err(fault);
                }
            }
            Phase::ReadyOff => (),
        }
        Ok(())
    }
    pub fn shutdown(&mut self, now: u64) -> Result<(), Fault> {
        self.clock(now)?;
        self.service_release()?;
        self.shutdown_requested = true;
        self.pending_config = None;
        self.stop(StopCause::Shutdown)
    }
}

#[cfg(test)]
mod tests;
impl<N: CooperativeNative, A: TxAuthority> Drop for TxLifecycle<N, A> {
    fn drop(&mut self) {
        if self.status.phase != Phase::ReadyOff || self.status.cleanup_completed_ns.is_none() {
            // Fail closed: request output inhibition, do not invoke blocking
            // cleanup/free a native worker before proving the output barrier.
            let _ = self.authority.begin_stop(StopRequest {
                id: self.status.stop_id.saturating_add(1),
                backend: self.backend,
                fence: self.fence(),
                cause: StopCause::Shutdown,
                notified_ns: self.now,
            });
            if let Some(controller) = self.controller.take() {
                std::mem::forget(controller);
            }
            eprintln!("PureSignal lifecycle dropped before quiescence; native owner retained, recovery blocked");
        }
    }
}
