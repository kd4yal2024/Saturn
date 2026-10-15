//! Offline generation handoff for existing TX output owners. No production
//! constructor attaches these fences. Default adapters remain stop-only;
//! explicit offline re-arm uses sealed TCI operator credentials and normal gates.
//! A receipt is local output completion, never an external RF-off measurement.
#![allow(dead_code)] // review-only interfaces have no live callers
use crate::{p2::session::P2Session, radio_model::RadioModel, xdma_tx_radio::DirectXdmaTxRadio};
use saturn_bridge::puresignal::{
    tx_lifecycle::{
        FreshTxAction, KeyReceipt, ReleaseHandle, ReleaseMailbox, StopEvidence, StopReceipt,
        StopRequest, TxAuthority,
    },
    Backend, Fault,
};
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, TryLockError,
    },
    time::{Duration, Instant},
};

/// Shared by *every* final output producer, including P2's independent HP
/// thread. close() never waits on an output/DMA mutex. Existing writes finish;
/// a complete stop receipt waits for their permits to drain. Once managed,
/// untagged output is permanently refused, even after authenticated reopening.
pub(crate) struct OutputFence {
    // Closed bit, invalidation revision AND cancellation ticket have ONE
    // linearization point. Generic faults close without manufacturing release.
    // Generation changes remain serialized by writer; close never takes it.
    state: AtomicU64,
    writer: Mutex<()>,
    generation: AtomicU64,
    managed: AtomicBool,
}
impl OutputFence {
    const CLOSED: u64 = 1 << 63;
    // Host-only state; wire/envelope fields remain u64. Exhaustion permanently
    // inhibits output, never wraps. 32 revision + 31 release bits + closed.
    const MAX_REVISION: u64 = (1 << 32) - 1;
    const MAX_RELEASE: u64 = (1 << 31) - 1;
    const RELEASE_SHIFT: u32 = 32;
    const RELEASE_MASK: u64 = Self::MAX_RELEASE << Self::RELEASE_SHIFT;

    pub(crate) fn new_open_for_offline_trial() -> Self {
        Self {
            state: AtomicU64::new(0),
            writer: Mutex::new(()),
            generation: AtomicU64::new(0),
            managed: AtomicBool::new(false),
        }
    }
    pub(crate) fn close(&self) {
        self.close_transition(|| {});
    }
    /// A media hazard belongs to the generation that observed it. Atomic CAS
    /// cannot let a delayed old-lane fault cancel a newly reopened generation.
    pub(crate) fn cancel_generation(&self, stamp: OutputStamp) -> bool {
        self.cancel_generation_transition(stamp, || {})
    }
    pub(crate) fn cancel_generation_transition(
        &self,
        stamp: OutputStamp,
        after: impl FnOnce(),
    ) -> bool {
        if stamp.revision >= Self::MAX_REVISION
            || self.generation.load(Ordering::SeqCst) != stamp.generation
        {
            return false;
        }
        let expected = self.state.load(Ordering::SeqCst);
        if expected & Self::CLOSED != 0 || expected & Self::MAX_REVISION != stamp.revision {
            return false;
        }
        let committed = self
            .state
            .compare_exchange(
                expected,
                Self::cancelled_state(expected),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok();
        if committed {
            after();
        }
        committed
    }
    fn cancelled_state(state: u64) -> u64 {
        let ticket = ((state & Self::RELEASE_MASK) >> Self::RELEASE_SHIFT)
            .saturating_add(1)
            .min(Self::MAX_RELEASE);
        let revision = if ticket == Self::MAX_RELEASE {
            Self::MAX_REVISION
        } else {
            (state & Self::MAX_REVISION)
                .saturating_add(1)
                .min(Self::MAX_REVISION)
        };
        Self::CLOSED | (ticket << Self::RELEASE_SHIFT) | revision
    }
    pub(crate) fn cancellation_ticket(&self) -> u64 {
        let ticket =
            (self.state.load(Ordering::SeqCst) & Self::RELEASE_MASK) >> Self::RELEASE_SHIFT;
        if ticket == Self::MAX_RELEASE {
            u64::MAX
        } else {
            ticket
        }
    }
    pub(crate) fn cancel(&self) -> u64 {
        self.cancel_transition(|| {})
    }
    pub(crate) fn cancel_transition(&self, after: impl FnOnce()) -> u64 {
        let previous = self
            .state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |state| {
                Some(Self::cancelled_state(state))
            })
            .unwrap();
        after();
        let ticket = (Self::cancelled_state(previous) & Self::RELEASE_MASK) >> Self::RELEASE_SHIFT;
        if ticket == Self::MAX_RELEASE {
            u64::MAX
        } else {
            ticket
        }
    }
    // The callback is a deterministic test seam AFTER invalidation publishes;
    // in production it is an empty closure. There is no flag/revision gap in
    // which to pause any longer. fetch_update retries a racing reopen CAS.
    fn close_transition(&self, after_transition: impl FnOnce()) {
        let _ = self
            .state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |state| {
                let revision = (state & Self::MAX_REVISION)
                    .saturating_add(1)
                    .min(Self::MAX_REVISION);
                Some((state & Self::RELEASE_MASK) | Self::CLOSED | revision)
            });
        after_transition();
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.state.load(Ordering::SeqCst) & Self::CLOSED != 0
    }
    pub(crate) fn enter(&self) -> io::Result<MutexGuard<'_, ()>> {
        // Existing producers have no generation envelope. Never stamp old work
        // with the new generation at delivery time.
        if self.managed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if self.is_closed() || self.managed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let guard = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("output fence poisoned"))?;
        if self.is_closed() || self.managed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(guard)
    }
    pub(crate) fn revision(&self) -> u64 {
        self.state.load(Ordering::SeqCst) & Self::MAX_REVISION
    }
    pub(crate) fn enter_generation(
        &self,
        generation: u64,
    ) -> io::Result<Option<MutexGuard<'_, ()>>> {
        let expected = self.state.load(Ordering::SeqCst);
        let valid = || {
            expected & Self::CLOSED == 0
                && self.generation.load(Ordering::SeqCst) == generation
                && self.state.load(Ordering::SeqCst) == expected
        };
        if !valid() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let guard = match self.writer.try_lock() {
            Ok(g) => g,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err(io::Error::other("output fence poisoned"))
            }
        };
        if !valid() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(Some(guard))
    }
    /// Distinguish refusal BEFORE backend I/O from a genuine writer/backend
    /// failure. Only the matching cancellation ticket may normalize refusal.
    pub(crate) fn enter_key_generation(
        &self,
        generation: u64,
    ) -> Result<Option<MutexGuard<'_, ()>>, Fault> {
        self.enter_generation(generation).map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                Fault::GenerationMismatch
            } else {
                Fault::TxAuthority
            }
        })
    }
    fn reopen(&self, revision: u64, generation: u64) -> Result<(), Fault> {
        self.reopen_transition(revision, generation, || {})
    }
    fn reopen_transition(
        &self,
        revision: u64,
        generation: u64,
        before_publish: impl FnOnce(),
    ) -> Result<(), Fault> {
        let Some(_guard) = self.drained()? else {
            return Err(Fault::CleanupPending);
        };
        if revision >= Self::MAX_REVISION || generation <= self.generation.load(Ordering::SeqCst) {
            return Err(Fault::GenerationMismatch);
        }
        let expected = self.state.load(Ordering::SeqCst);
        if expected & Self::CLOSED == 0 || expected & Self::MAX_REVISION != revision {
            return Err(Fault::GenerationMismatch);
        }
        self.generation.store(generation, Ordering::SeqCst);
        before_publish();
        // Conditional reopen and close/invalidation compete on the SAME word.
        // A close ordered before this CAS makes it fail; a close ordered after
        // it leaves the fence closed. The writer guard prevents output during
        // the generation store/CAS and failure never publishes an open state.
        self.state
            .compare_exchange(
                expected,
                expected & !Self::CLOSED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(|_| Fault::GenerationMismatch)?;
        Ok(())
    }
    pub(crate) fn drained(&self) -> Result<Option<MutexGuard<'_, ()>>, Fault> {
        if !self.is_closed() {
            return Err(Fault::TxAuthority);
        }
        match self.writer.try_lock() {
            Ok(g) if self.is_closed() => Ok(Some(g)),
            Ok(_) => Err(Fault::TxAuthority),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(Fault::TxAuthority),
        }
    }
}
impl ReleaseMailbox for OutputFence {
    fn current(&self) -> u64 {
        self.cancellation_ticket()
    }
    fn release(&self) -> u64 {
        self.cancel()
    }
}

enum OutputOwner<'a> {
    P2 {
        session: Arc<P2Session>,
        model: &'a RadioModel,
    },
    Direct(Arc<DirectXdmaTxRadio>),
}
pub(crate) struct OwnerStopAdapter<'a> {
    owner: OutputOwner<'a>,
    fence: Arc<OutputFence>,
    // Exclusive borrow of the existing owner's WDSP pending_iq and mic queues.
    // They cannot be refilled during this offline ownership handoff.
    pending_iq: &'a mut VecDeque<f32>,
    pending_mic: &'a mut VecDeque<f32>,
    request: Option<StopRequest>,
    commands: u8,
    next_command: Option<Instant>,
    last_id: u64,
    failed: bool,
    complete: Option<StopReceipt>,
    backend_stopped: bool,
    rearm: rearm::Rearm,
    feedback: Option<(crate::tx_thread::feedback_owner::Link, u128)>,
    #[cfg(test)]
    before_key_permit: Option<Box<dyn FnOnce() + Send>>,
}
impl<'a> OwnerStopAdapter<'a> {
    pub(crate) fn p2(
        session: Arc<P2Session>,
        model: &'a RadioModel,
        pending_iq: &'a mut VecDeque<f32>,
        pending_mic: &'a mut VecDeque<f32>,
    ) -> Result<Self, Fault> {
        let fence = session.offline_output_fence().ok_or(Fault::TxAuthority)?;
        Ok(Self::new(
            OutputOwner::P2 { session, model },
            fence,
            pending_iq,
            pending_mic,
        ))
    }
    pub(crate) fn direct(
        radio: Arc<DirectXdmaTxRadio>,
        pending_iq: &'a mut VecDeque<f32>,
        pending_mic: &'a mut VecDeque<f32>,
    ) -> Result<Self, Fault> {
        let fence = radio.offline_output_fence().ok_or(Fault::TxAuthority)?;
        Ok(Self::new(
            OutputOwner::Direct(radio),
            fence,
            pending_iq,
            pending_mic,
        ))
    }
    fn new(
        owner: OutputOwner<'a>,
        fence: Arc<OutputFence>,
        pending_iq: &'a mut VecDeque<f32>,
        pending_mic: &'a mut VecDeque<f32>,
    ) -> Self {
        Self {
            owner,
            fence,
            pending_iq,
            pending_mic,
            request: None,
            commands: 0,
            next_command: None,
            last_id: 0,
            failed: false,
            complete: None,
            backend_stopped: false,
            rearm: rearm::Rearm::default(),
            feedback: None,
            #[cfg(test)]
            before_key_permit: None,
        }
    }
    fn backend(&self) -> Backend {
        match self.owner {
            OutputOwner::P2 { .. } => Backend::P2,
            OutputOwner::Direct(_) => Backend::DirectXdma,
        }
    }
    pub(crate) fn attach_feedback_owner(
        &mut self,
        link: crate::tx_thread::feedback_owner::Link,
        route: u128,
    ) -> Result<(), Fault> {
        if route == 0 || self.feedback.is_some() || self.request.is_some() {
            return Err(Fault::InvalidConfig);
        }
        self.feedback = Some((link, route));
        Ok(())
    }
    pub(crate) fn set_feedback_route(&mut self, route: u128) -> Result<(), Fault> {
        if route == 0 || !self.fence.is_closed() {
            return Err(Fault::InvalidConfig);
        }
        if let Some((_, installed)) = &mut self.feedback {
            *installed = route;
        }
        Ok(())
    }
    pub(crate) fn queues_empty(&self) -> bool {
        self.pending_iq.is_empty() && self.pending_mic.is_empty()
    }
    // Actual wall-clock spacing, never fabricated controller timestamps.
    fn progress(&mut self, id: u64, now: Instant) -> Result<Option<StopReceipt>, Fault> {
        if self.failed {
            return Err(Fault::TxAuthority);
        }
        if self.request.is_none_or(|r| r.id != id) {
            return Ok(None);
        }
        if self.complete.is_some() {
            return Ok(self.complete);
        }
        let Some(_drained) = self.fence.drained()? else {
            return Ok(None);
        };
        let result = if self.backend_stopped {
            Ok(true)
        } else {
            match &self.owner {
                OutputOwner::P2 { session, model } => {
                    if self.commands >= 12 {
                        Ok(true)
                    } else {
                        if self.next_command.is_some_and(|due| now < due) {
                            return Ok(None);
                        }
                        let result = session
                            .offline_unkey_command(self.rearm.installed_model().unwrap_or(model))
                            .map_err(|_| Fault::TxAuthority);
                        if result.is_ok() {
                            self.commands += 1;
                            self.next_command = now.checked_add(Duration::from_millis(5));
                        }
                        result.map(|_| self.commands == 12)
                    }
                }
                OutputOwner::Direct(radio) => radio.offline_stop_and_readback(),
            }
        };
        let complete = match result {
            Ok(done) => done,
            Err(fault) => {
                self.failed = true;
                return Err(fault);
            }
        };
        if !complete {
            return Ok(None);
        }
        self.backend_stopped = true;
        // RF shutdown always progresses first. Acquisition restoration is a
        // second completion barrier, never a reason to postpone unkeying.
        if let Some((link, _)) = &self.feedback {
            link.permit_restoration(link.desired());
            if !link.restored()? {
                return Ok(None);
            }
        }
        if !self.queues_empty() {
            self.failed = true;
            return Err(Fault::TxAuthority);
        }
        let receipt = StopReceipt {
            id,
            output_barrier_closed: true,
            in_flight_writes_drained: true,
            queued_output_discarded: true,
            unkey_operation_completed: true,
            evidence: match self.backend() {
                Backend::P2 => StopEvidence::P2CommandBurstOnly,
                Backend::DirectXdma => StopEvidence::DirectControlReadback,
            },
        };
        self.complete = Some(receipt);
        Ok(Some(receipt))
    }
}
impl TxAuthority for OwnerStopAdapter<'_> {
    fn release_mailbox(&self) -> Option<ReleaseHandle> {
        Some(ReleaseHandle::from_mailbox(self.fence.clone()))
    }
    fn release_received(&mut self, _: u64, _: u64) -> Result<(), Fault> {
        self.fence.close();
        self.pending_iq.clear();
        self.pending_mic.clear();
        self.rearm.cancel();
        if let Some((link, _)) = &self.feedback {
            let _ = link.request(None);
        }
        Ok(())
    }
    fn begin_stop(&mut self, request: StopRequest) -> Result<(), Fault> {
        // Even malformed requests fence immediately; they cannot mint receipts.
        self.fence.close();
        self.pending_iq.clear();
        self.pending_mic.clear();
        self.rearm.cancel();
        if let Some((link, _)) = &self.feedback {
            let _ = link.request(None);
        }
        if self.failed || request.backend != self.backend() || request.id <= self.last_id {
            self.failed = true;
            return Err(Fault::TxAuthority);
        }
        self.last_id = request.id;
        self.request = Some(request);
        self.commands = 0;
        self.next_command = None;
        self.complete = None;
        self.backend_stopped = false;
        Ok(())
    }
    fn poll_stop(&mut self, id: u64) -> Result<Option<StopReceipt>, Fault> {
        self.progress(id, Instant::now())
    }
    fn begin_arm(&mut self, action: FreshTxAction, generation: u64) -> Result<(), Fault> {
        self.arm_generation(action, generation)
    }
    fn poll_key(&mut self, now: u64) -> Result<Option<KeyReceipt>, Fault> {
        self.key_generation(now)
    }
    fn cleanup_complete(
        &mut self,
        id: u64,
        fence: saturn_bridge::puresignal::Fence,
    ) -> Result<(), Fault> {
        self.mark_clean(id, fence)
    }
    fn poll_health(&mut self) -> Result<(), Fault> {
        if self.cancellation_pending() {
            return Ok(()); // exact atomic cancellation; lifecycle services it
        }
        if self.rearm_authorization_expired() {
            if self.cancellation_pending() {
                return Ok(());
            }
            self.disconnect_output();
            return Err(Fault::TxAuthority);
        }
        Ok(())
    }
}
impl Drop for OwnerStopAdapter<'_> {
    fn drop(&mut self) {
        self.fence.close();
        self.pending_iq.clear();
        self.pending_mic.clear();
    }
}

#[cfg(all(
    test,
    feature = "offline-puresignal-owner-native",
    not(saturn_bridge_stub_native)
))]
mod combined_native;
mod rearm;
pub(crate) use rearm::OutputStamp;
#[cfg(all(test, feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
mod measurement;
#[cfg(test)]
mod scheduled;
#[cfg(test)]
pub(crate) mod tests;
