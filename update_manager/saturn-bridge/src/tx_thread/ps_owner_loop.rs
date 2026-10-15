//! Gated owner migration. Uses the installed TX scheduling driver, actual
//! TxCommand/TxAudioMessage types and actual output adapters.
//! Socket/UDP ingress can attach only through the explicit review route;
//! default startup still selects Legacy; explicit managed startup selects this owner.
#![allow(dead_code)]
use super::{mic_samples_to_mono, satp_provenance::Provenance, scheduler, TxCommand};
use crate::{
    radio_model::RadioModel,
    tx_audio::{TxAudioMessage, TxAudioSource},
    tx_owner_stop::{OutputFence, OutputStamp, OwnerStopAdapter},
};
use saturn_bridge::puresignal::{
    controller::Config,
    tx_lifecycle::{CooperativeNative, Phase, TxLifecycle},
    Fault, Fence, PairBlock,
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, TryRecvError},
        Arc,
    },
    time::Instant,
};

/// process() admits pscc only (ZERO exchanges); render() performs one exchange.
/// Feedback fitting, mic rendering, stop progress and cleanup own the SAME N.
pub(crate) trait ScheduledNative: CooperativeNative {
    fn configure_model(&mut self, model: &RadioModel) -> Result<(), Fault>;
    fn render(&mut self, mono: &[f32]) -> Result<Vec<f32>, Fault>;
    /// None preserves the existing inhibited SATP default. Only a verified
    /// equal-rate port may report its native FIR retention capacity.
    fn satp_history_samples(&self) -> Option<usize> {
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IngressStamp {
    pub(super) fence: Fence,
    pub(super) revision: u64,
    pub(super) release: u64,
    pub(super) output: Option<OutputStamp>,
}
impl IngressStamp {
    #[cfg(test)]
    pub(crate) fn output(self) -> Option<OutputStamp> {
        self.output
    }
}
/// Must be captured at ingress BEFORE enqueue. A source/model selection is a
/// single snapshot transaction, not ModelChanged's later shared-model read.
pub struct Configuration {
    stamp: IngressStamp,
    model: RadioModel,
    config: Config,
    source: TxAudioSource,
    pub(super) transaction: Option<super::ps_ingress::SettingsTransaction>,
}
impl Drop for Configuration {
    fn drop(&mut self) {
        // Every dequeued rejection, cancellation, shutdown and queue drop has
        // a terminal result, even when its caller only counts rejected work.
        if let Some(t) = &self.transaction {
            t.reject(Fault::GenerationMismatch);
        }
    }
}
impl Configuration {
    pub(super) fn view(&self) -> (RadioModel, Config, TxAudioSource) {
        (self.model.clone(), self.config, self.source)
    }
}
impl Configuration {
    pub(crate) fn at_ingress(
        stamp: IngressStamp,
        model: RadioModel,
        config: Config,
        source: TxAudioSource,
    ) -> Self {
        Self {
            stamp,
            model,
            config,
            source,
            transaction: None,
        }
    }
}
pub(crate) struct Audio {
    stamp: IngressStamp,
    message: TxAudioMessage,
}
impl Audio {
    pub(crate) fn at_ingress(stamp: IngressStamp, message: TxAudioMessage) -> Self {
        Self { stamp, message }
    }
}
#[derive(Clone)]
pub(crate) struct Cancel {
    fence: Arc<OutputFence>,
    pub(super) settings_epoch: Arc<SettingsEpoch>,
}
/// Cancellation is read from the fence's atomic transition, never published
/// afterward. Owner-local mutations are serialized by the scheduling loop.
pub(super) struct SettingsEpoch {
    fence: Arc<OutputFence>,
    mutation: AtomicU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SettingsEpochValue {
    cancellation: u64,
    mutation: u64,
}
impl SettingsEpochValue {
    pub(super) fn exhausted(self) -> bool {
        self.cancellation == u64::MAX || self.mutation == u64::MAX
    }
}
impl SettingsEpoch {
    fn new(fence: Arc<OutputFence>) -> Self {
        Self {
            fence,
            mutation: AtomicU64::new(0),
        }
    }
    pub(super) fn current(&self) -> SettingsEpochValue {
        SettingsEpochValue {
            cancellation: self.fence.cancellation_ticket(),
            mutation: self.mutation.load(Ordering::SeqCst),
        }
    }
}
impl Cancel {
    #[cfg(test)]
    pub(crate) fn fixture(fence: Arc<OutputFence>) -> Self {
        Self {
            settings_epoch: Arc::new(SettingsEpoch::new(fence.clone())),
            fence,
        }
    }
    pub(super) fn release_generation(&self, stamp: OutputStamp) -> bool {
        self.fence.cancel_generation(stamp)
    }
    pub(super) fn output_open(&self) -> bool {
        !self.fence.is_closed()
    }
    /// Works with full/disconnected channels and an in-flight DMA permit.
    pub(crate) fn release(&self) {
        self.fence.cancel();
    }
    // Same atomic transition as release; pausing after it cannot reveal a
    // closed fence without its cancellation ticket.
    #[cfg(test)]
    pub(crate) fn offline_release_boundary(&self, boundary: impl FnOnce()) {
        self.fence.cancel_transition(boundary);
    }
    #[cfg(test)]
    pub(crate) fn offline_generation_release_boundary(
        &self,
        stamp: OutputStamp,
        boundary: impl FnOnce(),
    ) -> bool {
        self.fence.cancel_generation_transition(stamp, boundary)
    }
    pub(super) fn invalidate_settings(&self) {
        let _ =
            self.settings_epoch
                .mutation
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                    Some(v.saturating_add(1))
                });
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) mic_age_ns: u64,
    pub(crate) source_stall_ns: u64,
    pub(crate) tx_duration_ns: u64,
}
#[derive(Default, Debug)]
pub(crate) struct Counters {
    pub(crate) turns: u64,
    pub(crate) rejected: u64,
    pub(crate) mic_blocks: u64,
    pub(crate) iq_packets: u64,
    pub(crate) configurations: u64,
    pub(crate) deadline_stops: u64,
}
pub(crate) struct OwnerLoop<'a, N: ScheduledNative> {
    pub(crate) life: TxLifecycle<N, OwnerStopAdapter<'a>>,
    commands: Receiver<TxCommand>,
    audio: Receiver<Audio>,
    feedback: Receiver<Box<PairBlock>>,
    origin: Instant,
    timing: Timing,
    selected: TxAudioSource,
    configured: bool,
    pending_config: Option<(Configuration, u64, u64, u64)>, // snapshot, stop id, revision, release
    mic: VecDeque<(f32, u64)>, // exact source age; <= four 512-sample blocks
    mic_stamp: Option<IngressStamp>,
    satp: Option<Provenance<crate::wdsp::MicRateMatcher>>,
    satp_history: Option<usize>,
    satp_prepared: Option<u64>,
    satp_started: bool,
    next_mic: u64,
    next_iq: u64,
    armed: Option<u64>,
    last_arrival: Option<u64>,
    seen_stop: u64,
    command_closed: bool,
    audio_closed: bool,
    feedback_closed: bool,
    shutdown: bool,
    pub(crate) counters: Counters,
    ingress: Option<super::ps_ingress::SharedIngress>,
    installed: Option<(RadioModel, Config)>,
    settings_epoch: Arc<SettingsEpoch>,
    measurement_ticket: Option<super::ps_measurement::Ticket>,
}
impl<'a, N: ScheduledNative> OwnerLoop<'a, N> {
    /// Exclusive channel construction for the selected shared owner. No
    /// legacy audio/command consumer is constructed by this branch.
    pub(crate) fn with_shared_ingress(
        mut life: TxLifecycle<N, OwnerStopAdapter<'a>>,
        operator: crate::tci::ps_authorization::OperatorIngress,
        model: Arc<std::sync::Mutex<RadioModel>>,
        origin: Instant,
        timing: Timing,
    ) -> Result<(Self, super::ps_ingress::SharedIngress), Fault> {
        let fence = life.offline_owner().cancellation_fence();
        let cancel = Cancel {
            settings_epoch: Arc::new(SettingsEpoch::new(fence.clone())),
            fence,
        };
        let (ingress, rx) = super::ps_ingress::SharedIngress::channels(operator, cancel, model);
        let mut owner = Self::new(life, rx.commands, rx.audio, rx.feedback, origin, timing)?;
        owner.attach_ingress(ingress.clone());
        Ok((owner, ingress))
    }
    pub(crate) fn new(
        mut life: TxLifecycle<N, OwnerStopAdapter<'a>>,
        commands: Receiver<TxCommand>,
        audio: Receiver<Audio>,
        feedback: Receiver<Box<PairBlock>>,
        origin: Instant,
        timing: Timing,
    ) -> Result<Self, Fault> {
        if timing.mic_age_ns == 0 || timing.source_stall_ns == 0 || timing.tx_duration_ns == 0 {
            return Err(Fault::InvalidConfig);
        }
        let seen_stop = life.status().stop_id;
        let settings_epoch = Arc::new(SettingsEpoch::new(
            life.offline_owner().cancellation_fence(),
        ));
        Ok(Self {
            life,
            commands,
            audio,
            feedback,
            origin,
            timing,
            selected: TxAudioSource::Tci,
            configured: false,
            pending_config: None,
            mic: VecDeque::new(),
            mic_stamp: None,
            satp: None,
            satp_history: None,
            satp_prepared: None,
            satp_started: false,
            next_mic: 0,
            next_iq: 0,
            armed: None,
            last_arrival: None,
            seen_stop,
            command_closed: false,
            audio_closed: false,
            feedback_closed: false,
            shutdown: false,
            counters: Counters::default(),
            ingress: None,
            installed: None,
            settings_epoch,
            measurement_ticket: None,
        })
    }
    pub(crate) fn cancel_handle(&mut self) -> Cancel {
        Cancel {
            fence: self.life.offline_owner().cancellation_fence(),
            settings_epoch: self.settings_epoch.clone(),
        }
    }
    pub(crate) fn attach_ingress(&mut self, ingress: super::ps_ingress::SharedIngress) {
        assert!(self.ingress.is_none(), "one ingress route per TX owner");
        self.settings_epoch = ingress.settings_epoch();
        self.ingress = Some(ingress);
        self.publish_ingress();
    }
    fn publish_ingress(&mut self) {
        if let Some(ingress) = self.ingress.clone() {
            let snapshot = self.snapshot();
            ingress.publish(
                snapshot,
                self.life.status(),
                self.installed.clone(),
                self.selected,
            );
        }
        if self
            .measurement_ticket
            .as_ref()
            .is_some_and(|t| t.service(&mut self.life))
        {
            self.measurement_ticket = None;
        }
    }
    #[cfg(test)]
    pub(crate) fn ingress_origin(&self) -> Instant {
        self.origin
    }
    pub(crate) fn snapshot(&mut self) -> IngressStamp {
        IngressStamp {
            fence: self.life.fence(),
            revision: self.life.offline_owner().current_revision(),
            release: self.life.status().release_handled,
            output: self.life.offline_owner().publication_output_stamp(),
        }
    }
    fn matches(&mut self, stamp: IngressStamp) -> bool {
        stamp == self.snapshot()
    }
    fn audio_matches(&mut self, stamp: IngressStamp) -> bool {
        let current = self.snapshot();
        // Feedback tx_epoch changes at key receipt, but TX output generation
        // does not. Preserve valid source history across that boundary only.
        stamp.output.is_some()
            && stamp.output == current.output
            && stamp.revision == current.revision
            && stamp.release == current.release
            && stamp.fence.boot == current.fence.boot
            && stamp.fence.generation == current.fence.generation
            && stamp.fence.config == current.fence.config
    }
    fn discard_local(&mut self) {
        self.satp_started = false;
        self.mic.clear();
        self.mic_stamp = None;
        self.armed = None;
        self.last_arrival = None;
        if self.satp.as_ref().is_some_and(|s| s.is_bound()) {
            self.satp.as_mut().unwrap().invalidate();
            self.satp_prepared = None;
        }
    }
    fn fault(&mut self, fault: Fault, now: u64) {
        #[cfg(test)]
        if fault == Fault::QueueOverrun {
            eprintln!(
                "queue diagnostic phase={:?} mic_buffered={} counters={:?}",
                self.life.status().phase,
                self.mic.len(),
                self.counters
            );
        }
        #[cfg(test)]
        if fault == Fault::Stale {
            eprintln!("offline stale diagnostic: phase={:?} buffered={} oldest={:?} last_arrival={:?} counters={:?}",
                self.life.status().phase, self.mic.len(), self.mic.front().map(|v| now.saturating_sub(v.1)),
                self.last_arrival.map(|at| now.saturating_sub(at)), self.counters);
        }
        eprintln!(
            "shared TX owner fault={fault:?} source={:?} observed_ns={now}",
            self.selected
        );
        self.cancel_handle().invalidate_settings();
        if let Some((config, _, _, _)) = &self.pending_config {
            if let Some(t) = &config.transaction {
                t.reject(fault);
            }
        }
        let _ = self.life.notify_fault(fault, now);
        self.discard_local();
        self.pending_config = None;
    }
    fn output_error(&mut self, fault: Fault, now: u64) {
        // A release can race the exchange's final return or queue write. Give
        // its zero-exchange service first refusal: ordinary cancellation is
        // not a new CleanupPending/GenerationMismatch fault requiring ack.
        let _ = self.life.offline_tx_exchange(now, |_| Ok(()));
        if matches!(self.life.status().phase, Phase::AwaitingKey | Phase::Active) {
            self.fault(fault, now);
        } else {
            self.discard_local();
        }
    }
    fn command(&mut self, command: TxCommand, now: u64) -> Result<(), Fault> {
        match command {
            TxCommand::PureSignalAuthorizedArm(arm) => {
                if self
                    .ingress
                    .as_ref()
                    .is_some_and(|i| i.measurement_only_mode())
                {
                    return Err(Fault::InvalidConfig);
                }
                if !self.configured
                    || self.pending_config.is_some()
                    || (self.selected == TxAudioSource::Satp
                        && self.satp_prepared != Some(self.life.status().stop_id))
                    || self.life.status().phase != Phase::ReadyOff
                {
                    return Err(Fault::InvalidConfig);
                }
                let action = self.life.offline_owner().authorize(arm.into_credential())?;
                self.life.fresh_tx_action(action, now)?;
                self.cancel_handle().invalidate_settings();
                self.discard_local();
                if self.selected == TxAudioSource::Satp {
                    let stamp = self.life.offline_owner().output_stamp()?;
                    let result = self
                        .satp
                        .as_mut()
                        .ok_or(Fault::NativeUnverified)?
                        .bind(stamp);
                    if let Err(fault) = result {
                        self.fault(fault, now);
                        return Err(fault);
                    }
                }
                self.armed = Some(now);
                self.last_arrival = Some(now);
                self.next_mic = now;
                self.next_iq = now;
            }
            TxCommand::MeasurementOnly(mut request) => {
                let ticket = request.diagnostic.take();
                let result = (|| {
                    if !self.configured
                        || self.pending_config.is_some()
                        || self.life.status().phase != Phase::ReadyOff
                        || (self.selected == TxAudioSource::Satp
                            && self.satp_prepared != Some(self.life.status().stop_id))
                    {
                        return Err(Fault::CleanupPending);
                    }
                    let requested_ns = request
                        .requested_at
                        .checked_duration_since(self.origin)
                        .and_then(|d| u64::try_from(d.as_nanos()).ok())
                        .ok_or(Fault::FutureObservation)?;
                    if requested_ns > now || now - requested_ns > self.timing.mic_age_ns {
                        return Err(Fault::Stale);
                    }
                    let deadline = requested_ns
                        .checked_add(request.duration_ns)
                        .ok_or(Fault::InvalidConfig)?;
                    let remaining = deadline
                        .checked_sub(now)
                        .filter(|d| *d != 0)
                        .ok_or(Fault::Stale)?;
                    let action = self.life.offline_owner().authorize(request.credential)?;
                    self.life
                        .fresh_measurement_action(action, remaining, request.recorder, now)?;
                    self.cancel_handle().invalidate_settings();
                    self.discard_local();
                    if self.selected == TxAudioSource::Satp {
                        let stamp = self.life.offline_owner().output_stamp()?;
                        if let Err(fault) = self
                            .satp
                            .as_mut()
                            .ok_or(Fault::NativeUnverified)?
                            .bind(stamp)
                        {
                            self.fault(fault, now);
                            return Err(fault);
                        }
                    }
                    self.armed = Some(now);
                    self.last_arrival = Some(now);
                    self.next_mic = now;
                    self.next_iq = now;
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        if let Some(ticket) = ticket {
                            ticket.running();
                            self.measurement_ticket = Some(ticket);
                        }
                    }
                    Err(fault) => {
                        if let Some(ticket) = ticket {
                            if self.life.measurement_records().is_some() {
                                // A source bind can fail AFTER native admission.
                                // Retain that run's ticket through stop/cleanup
                                // so its incomplete evidence is still exportable.
                                ticket.running();
                                self.measurement_ticket = Some(ticket);
                            } else {
                                ticket.reject(fault);
                            }
                        }
                        return Err(fault);
                    }
                }
            }
            TxCommand::PureSignalOwnerConfig(config) => {
                if self
                    .ingress
                    .as_ref()
                    .is_some_and(|i| i.measurement_only_mode())
                    && (config.config.enabled || config.config.automatic)
                {
                    if let Some(t) = &config.transaction {
                        t.reject(Fault::InvalidConfig);
                    }
                    return Err(Fault::InvalidConfig);
                }
                let current = self.snapshot();
                let eligible = if let Some(t) = &config.transaction {
                    t.admit(config.stamp, current)
                } else if self.matches(config.stamp) {
                    Ok(())
                } else {
                    Err(Fault::GenerationMismatch)
                };
                if let Err(fault) = eligible {
                    if let Some(t) = &config.transaction {
                        t.reject(fault);
                    }
                    return Err(fault);
                }
                if self.shutdown
                    || self.pending_config.is_some()
                    || !matches!(
                        self.life.status().phase,
                        Phase::ReadyOff | Phase::Active | Phase::AwaitingKey
                    )
                    || self.life.status().primary_fault.is_some()
                {
                    if let Some(t) = &config.transaction {
                        t.reject(Fault::GenerationMismatch);
                    }
                    return Err(Fault::GenerationMismatch);
                }
                // Only an exact owner-issued predecessor completion permits a
                // settings successor. The original ingress stamp is retained;
                // this never refreshes an arm/audio envelope.
                if let Err(fault) = self.life.reconfigure(current.fence, config.config, now) {
                    if let Some(t) = &config.transaction {
                        t.reject(fault);
                    }
                    return Err(fault);
                }
                if config.transaction.is_none() {
                    self.cancel_handle().invalidate_settings();
                }
                self.configured = false;
                self.discard_local();
                self.pending_config = Some((
                    *config,
                    self.life.status().stop_id,
                    self.life.offline_owner().current_revision(),
                    self.life.status().release_handled,
                ));
            }
            TxCommand::Disarm => {
                self.cancel_handle().release();
                self.discard_local();
                self.pending_config = None;
            }
            TxCommand::Shutdown => {
                self.shutdown = true;
                self.pending_config = None;
                self.discard_local();
                self.life.shutdown(now)?;
            }
            // Never convert booleans, legacy feedback, untagged model/source
            // changes or blocking reset commands into current authority.
            _ => return Err(Fault::TxAuthority),
        }
        Ok(())
    }
    fn accept_audio(&mut self, audio: Audio, now: u64) -> Result<(), Fault> {
        if !self.audio_matches(audio.stamp)
            || audio.stamp.output.is_none()
            || !matches!(self.life.status().phase, Phase::AwaitingKey | Phase::Active)
        {
            return Err(Fault::GenerationMismatch);
        }
        match audio.message {
            TxAudioMessage::Flush(source) if source == self.selected => {
                self.cancel_handle().release();
                self.discard_local();
            }
            TxAudioMessage::Frames(frame) => {
                let observed = frame
                    .enqueued_at
                    .checked_duration_since(self.origin)
                    .ok_or(Fault::Stale)?
                    .as_nanos() as u64;
                if frame.source != self.selected
                    || frame.sample_rate_hz != 48000
                    || frame.channels == 0
                    || frame.channels > 2
                {
                    return Err(Fault::InvalidConfig);
                }
                if observed > now || now - observed > self.timing.mic_age_ns {
                    return Err(Fault::Stale);
                }
                let raw = frame.samples.as_slice();
                if !raw.len().is_multiple_of(frame.channels as usize)
                    || raw.len() > 1024
                    || raw.iter().any(|v| !v.is_finite())
                {
                    return Err(Fault::InvalidConfig);
                }
                let mono = mic_samples_to_mono(raw, frame.channels);
                if self.selected == TxAudioSource::Satp {
                    let stamp = audio.stamp.output.ok_or(Fault::GenerationMismatch)?;
                    self.satp.as_mut().ok_or(Fault::NativeUnverified)?.feed(
                        stamp,
                        observed,
                        &mono,
                        now,
                        self.timing.mic_age_ns,
                    )?;
                } else {
                    if self.mic.len() + mono.len() > 2048 {
                        return Err(Fault::QueueOverrun);
                    }
                    self.mic.extend(mono.iter().map(|v| (*v, observed)));
                    self.mic_stamp = Some(audio.stamp);
                }
                // Silence counts as a live source, but never mic qualification.
                self.last_arrival = Some(self.last_arrival.unwrap_or(0).max(observed));
            }
            _ => return Err(Fault::InvalidConfig),
        }
        Ok(())
    }
    /// One bounded turn: lifecycle/release FIRST; one command, four audio
    /// messages, one feedback block, at most one render OR cleanup exchange.
    pub(crate) fn turn(&mut self, now: u64) -> scheduler::Turn {
        let result = self.turn_inner(now, |_| {}, false, |_| {});
        self.publish_ingress();
        result
    }
    // Deterministic seam at the actual tick/pump boundary. Normal turns pass
    // an empty callback; tests can post a release without scheduling sleeps.
    #[cfg(test)]
    pub(crate) fn turn_with_before_pump(
        &mut self,
        now: u64,
        before_pump: impl FnOnce(&mut Self),
    ) -> scheduler::Turn {
        self.turn_inner(now, before_pump, false, |_| {})
    }
    #[cfg(test)]
    pub(crate) fn turn_realtime_after_tick(
        &mut self,
        now: u64,
        after_tick: impl FnOnce(&mut Self),
    ) -> scheduler::Turn {
        self.turn_inner(now, |_| {}, true, after_tick)
    }
    #[cfg(test)]
    pub(crate) fn render_due(&mut self, now: u64) {
        self.next_mic = now;
        self.next_iq = now;
    }
    fn finish_turn(&self, worked: bool) -> scheduler::Turn {
        if self.shutdown && self.life.status().phase == Phase::ReadyOff {
            scheduler::Turn::Exit
        } else {
            scheduler::Turn::Continue {
                worked,
                active: self.life.status().phase != Phase::ReadyOff,
            }
        }
    }
    fn turn_inner(
        &mut self,
        mut now: u64,
        before_pump: impl FnOnce(&mut Self),
        realtime: bool,
        after_tick: impl FnOnce(&mut Self),
    ) -> scheduler::Turn {
        self.counters.turns += 1;
        let phase_before_tick = self.life.status().phase;
        let _ = self.life.tick(now); // faults are latched; continue stop progress
        after_tick(self);
        if let Some(fault) = self.ingress.as_ref().and_then(|i| i.take_fault()) {
            self.fault(fault, now);
        }
        if self.life.status().stop_id != self.seen_stop
            || !matches!(self.life.status().phase, Phase::Active | Phase::AwaitingKey)
        {
            self.discard_local();
            self.seen_stop = self.life.status().stop_id;
        }
        let current_revision = self.life.offline_owner().current_revision();
        if self
            .pending_config
            .as_ref()
            .is_some_and(|(c, _, _, _)| c.transaction.as_ref().is_some_and(|t| !t.live()))
        {
            // Covers revoked operator permission even without a socket close.
            // Also services an out-of-band release arriving after tick().
            self.cancel_handle().release();
            let _ = self.life.offline_tx_exchange(now, |_| Ok(()));
        }
        if self
            .pending_config
            .as_ref()
            .is_some_and(|(_, stop, revision, release)| {
                *stop != self.life.status().stop_id
                    || *release != self.life.status().release_handled
                    || *revision != current_revision
                    || self.life.status().primary_fault.is_some()
                    || self.life.status().authority_fault.is_some()
                    || self.life.status().cleanup_fault.is_some()
                    || self.life.status().phase == Phase::Blocked
            })
            || self
                .pending_config
                .as_ref()
                .is_some_and(|(c, _, _, _)| c.transaction.as_ref().is_some_and(|t| !t.live()))
        {
            if let Some((config, _, _, _)) = &self.pending_config {
                if let Some(t) = &config.transaction {
                    let status = self.life.status();
                    t.reject(
                        status
                            .primary_fault
                            .or(status.authority_fault)
                            .or(status.cleanup_fault)
                            .unwrap_or(Fault::GenerationMismatch),
                    );
                }
            }
            self.pending_config = None;
        }
        if phase_before_tick == Phase::AwaitingKey && self.life.status().phase == Phase::Active {
            // Key receipt starts a new feedback epoch. Never restamp a mic
            // backlog captured before it; retained output already has permits.
            self.mic.clear();
            self.mic_stamp = None;
        }
        if self.life.status().phase == Phase::ReadyOff {
            if let Some((config, _, _, _)) = self.pending_config.take() {
                // Applying a configuration does NOT authorize/reopen/key.
                let mut history = None;
                let route_result = self
                    .life
                    .offline_owner()
                    .set_feedback_route(config.config.route);
                let result = self
                    .life
                    .offline_clean_configure(now, |native| {
                        native.configure_model(&config.model)?;
                        history = native.satp_history_samples();
                        Ok(())
                    })
                    .and_then(|_| route_result)
                    .and_then(|_| self.life.offline_owner().install_clean_model(&config.model));
                if (result.is_ok()
                    || matches!(
                        result,
                        Err(Fault::CleanupPending | Fault::GenerationMismatch | Fault::TxAuthority)
                    ))
                    && config.transaction.as_ref().is_some_and(|t| !t.live())
                {
                    // Cancellation during native/model installation is not a
                    // completed settings transaction or a new ordinary-release
                    // fault. Continue the existing stop/cleanup path.
                    self.cancel_handle().release();
                    let _ = self.life.offline_tx_exchange(now, |_| Ok(()));
                    self.discard_local();
                    return scheduler::Turn::Continue {
                        worked: true,
                        active: false,
                    };
                }
                match result {
                    Ok(()) => {
                        let (model, ps, _) = config.view();
                        self.installed = Some((model, ps));
                        self.selected = config.source;
                        self.satp_history = history;
                        self.satp_prepared = None;
                        self.configured = true;
                        self.counters.configurations += 1;
                        // Publish the committed snapshot before the completion
                        // reply can cause a fresh client configuration/arm.
                        self.publish_ingress();
                        if let Some(t) = &config.transaction {
                            t.applied(self.snapshot());
                        }
                    }
                    Err(fault) => {
                        if let Some(t) = &config.transaction {
                            t.reject(fault);
                        }
                        self.fault(fault, now);
                    }
                }
            }
            // Recreate source history only after stop receipt/native cleanup,
            // never rebind retained native samples to a new generation.
            if self.selected == TxAudioSource::Satp
                && self.satp_prepared != Some(self.life.status().stop_id)
            {
                if let Some(history) = self.satp_history {
                    match Provenance::pinned(history) {
                        Ok(port) => {
                            self.satp = Some(port);
                            self.satp_prepared = Some(self.life.status().stop_id);
                        }
                        Err(fault) => self.fault(fault, now),
                    }
                }
            } else if self.selected == TxAudioSource::Tci {
                self.satp = None;
                self.satp_prepared = None;
            }
        }
        // Owner watchdogs precede queue draining. Busy queues cannot renew
        // source age at dequeue or mask a timed-out TX action.
        if self
            .armed
            .is_some_and(|t| now.saturating_sub(t) > self.timing.tx_duration_ns)
            || self
                .last_arrival
                .is_some_and(|t| now.saturating_sub(t) > self.timing.source_stall_ns)
        {
            self.counters.deadline_stops += 1;
            self.fault(Fault::Stale, now);
        }
        let mut worked = false;
        if !self.command_closed && self.pending_config.is_none() {
            match self.commands.try_recv() {
                Ok(command) => {
                    if realtime {
                        now = now.max(self.origin.elapsed().as_nanos() as u64);
                    }
                    worked = true;
                    if self.command(command, now).is_err() {
                        self.counters.rejected += 1;
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.command_closed = true;
                    self.fault(Fault::TxAuthority, now);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        for _ in 0..4 {
            if self.audio_closed {
                break;
            }
            match self.audio.try_recv() {
                Ok(audio) => {
                    if realtime {
                        now = now.max(self.origin.elapsed().as_nanos() as u64);
                    }
                    worked = true;
                    if let Err(fault) = self.accept_audio(audio, now) {
                        self.counters.rejected += 1;
                        if matches!(fault, Fault::QueueOverrun | Fault::NonFinite)
                            || (self.selected == TxAudioSource::Satp
                                && matches!(
                                    fault,
                                    Fault::Stale
                                        | Fault::SequenceGap
                                        | Fault::Reordered
                                        | Fault::FutureObservation
                                        | Fault::NativeUnverified
                                ))
                        {
                            self.output_error(fault, now);
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.audio_closed = true;
                    self.fault(Fault::TxAuthority, now);
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if !self.feedback_closed {
            match self.feedback.try_recv() {
                Ok(block) => {
                    // Producer observation is before enqueue, but may be
                    // after this turn's initial tick. Read the consumer clock
                    // AFTER dequeue; never clamp or rewrite the sample age.
                    if realtime {
                        now = now.max(self.origin.elapsed().as_nanos() as u64);
                    }
                    worked = true;
                    if self.life.submit(&block, now).is_err() {
                        self.counters.rejected += 1;
                    }
                    if let Some(ingress) = &self.ingress {
                        ingress.recycle_feedback_buffer(block);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.feedback_closed = true;
                    self.fault(Fault::TxAuthority, now);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        // A tick that advanced cleanup consumes the native budget even if a
        // fresh arm arrived this turn. No second owner or exchange can follow.
        if matches!(phase_before_tick, Phase::AwaitingKey | Phase::Active)
            && matches!(self.life.status().phase, Phase::AwaitingKey | Phase::Active)
        {
            if self.life.status().phase == Phase::Active {
                before_pump(self);
                if realtime {
                    now = now.max(self.origin.elapsed().as_nanos() as u64);
                }
                // ScheduledNative process() does pscc only. Rendering advances
                // IQC/fitter and returns output from that single DSP exchange.
                if self.life.pump(now).is_err() {
                    self.discard_local();
                }
            }
            // pump() is a lifecycle boundary: feedback/native faults and a
            // pending release can retire output authority. Do not render or
            // send anything in that turn; subsequent turns must progress the
            // stop receipt and native cleanup, not mistake a fence for unkey.
            if !matches!(self.life.status().phase, Phase::AwaitingKey | Phase::Active) {
                self.discard_local();
                return self.finish_turn(worked);
            }
            if now >= self.next_mic && !self.life.offline_owner().has_iq_packet() {
                if self.selected == TxAudioSource::Satp && !self.satp_started {
                    if !self.satp.as_ref().is_some_and(|p| p.source_primed()) {
                        // Jitter startup must not consume the destination bank
                        // before any real source block exists. RF is not keyed;
                        // one zero-input native exchange is discarded. Existing
                        // source/key deadlines still run and release stays first.
                        let result = self
                            .life
                            .offline_tx_exchange(now, |n| n.advance_dsp_discard());
                        if let Err(fault) = result {
                            self.output_error(fault, now);
                        }
                        self.next_mic = now;
                        return self.finish_turn(true);
                    }
                    self.satp_started = true;
                    self.next_mic = now;
                }
                let stamp = if self.selected == TxAudioSource::Satp {
                    self.snapshot()
                } else {
                    self.mic_stamp.unwrap_or_else(|| self.snapshot())
                };
                let Some(output_stamp) = stamp.output else {
                    self.output_error(Fault::TxAuthority, now);
                    return self.finish_turn(worked);
                };
                let input = if self.selected == TxAudioSource::Satp {
                    self.satp
                        .as_mut()
                        .ok_or(Fault::NativeUnverified)
                        .and_then(|s| s.take(output_stamp, now, self.timing.mic_age_ns))
                        .map(|b| (b.oldest.unwrap_or(now), b.samples))
                } else {
                    let observed = self.mic.front().map(|s| s.1).unwrap_or(now);
                    let take = self.mic.len().min(512);
                    let mut samples: Vec<f32> = self.mic.drain(..take).map(|v| v.0).collect();
                    samples.resize(512, 0.);
                    Ok((observed, samples))
                };
                let (observed, mic) = match input {
                    Ok(input) => input,
                    Err(fault) => {
                        self.output_error(fault, now);
                        return self.finish_turn(worked);
                    }
                };
                if !self.audio_matches(stamp)
                    || observed > now
                    || now - observed > self.timing.mic_age_ns
                {
                    self.counters.rejected += 1;
                    self.discard_local();
                    self.fault(Fault::Stale, now);
                } else {
                    // Fixed-clock zero fill continues native DSP during source
                    // pauses. It NEVER updates last_arrival or grants mic-level
                    // qualification; both watchdogs remain based on ingress.
                    let result = self
                        .life
                        .offline_owner()
                        .enqueue_mic(output_stamp, observed, &mic)
                        .and_then(|_| self.life.offline_owner().take_mic(output_stamp))
                        .and_then(|mic| {
                            self.life
                                .offline_tx_exchange(now, |native| native.render(&mic))
                        })
                        .and_then(|iq| {
                            self.life
                                .offline_owner()
                                .enqueue_iq(output_stamp, observed, &iq)
                        });
                    match result {
                        Ok(()) => {
                            worked = true;
                            self.counters.mic_blocks += 1;
                        }
                        Err(fault) => self.output_error(fault, now),
                    }
                    let period = 512_000_000_000 / 48_000;
                    self.next_mic = self
                        .next_mic
                        .saturating_add(period)
                        .max(now.saturating_sub(period));
                }
            }
            if self.life.status().phase == Phase::Active && now >= self.next_iq {
                if let Ok(stamp) = self.life.offline_owner().output_stamp() {
                    match self.life.offline_owner().flush_iq(stamp, now) {
                        Ok(true) => {
                            self.counters.iq_packets += 1;
                            worked = true;
                        }
                        Ok(false) => {}
                        Err(fault) => self.output_error(fault, now),
                    }
                }
                let period = 240_000_000_000 / 192_000;
                self.next_iq = self
                    .next_iq
                    .saturating_add(period)
                    .max(now.saturating_sub(period));
            }
        }
        self.finish_turn(worked)
    }
    /// Offline harness hooks enqueue BEFORE the consumer turn. This runs the
    /// same driver as run(), not a hand-written sequence of lifecycle calls.
    pub(crate) fn run(&mut self, stop: &AtomicBool, mut ingress: impl FnMut(&mut Self, u64)) {
        // The external flag requests shutdown, not immediate channel destruction
        // while RF/native stop is pending. Continue the driver until ReadyOff.
        let draining = AtomicBool::new(false);
        scheduler::drive(&draining, || {
            let now = self.origin.elapsed().as_nanos() as u64;
            if stop.load(std::sync::atomic::Ordering::Relaxed) && !self.shutdown {
                self.shutdown = true;
                self.pending_config = None;
                self.discard_local();
                let _ = self.life.shutdown(now);
            }
            if self.shutdown && self.life.status().phase == Phase::Blocked {
                // Stop/cleanup failure is NOT success. Production returns an
                // error; the native wrapper retains uncertain native storage.
                return scheduler::Turn::Exit;
            }
            ingress(self, now);
            let result = self.turn_inner(now, |_| {}, true, |_| {});
            self.publish_ingress();
            result
        });
    }
}
