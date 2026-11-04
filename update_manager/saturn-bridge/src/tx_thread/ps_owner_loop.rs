//! Gated owner migration. Uses the installed TX scheduling driver, actual
//! TxCommand/TxAudioMessage types and actual output adapters.
//! Socket/UDP ingress can attach only through the explicit review route;
//! default startup still selects Legacy; explicit managed startup selects this owner.
#![allow(dead_code)]
use super::owner_mailbox::Receiver;
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
        mpsc::TryRecvError,
        Arc,
    },
    time::Instant,
};

const TCI_MIC_STAGING_SAMPLES: usize = 2048;
// Ingress validates at most 1024 scalar samples; mono is the largest frame.
const MAX_TCI_MONO_FRAME_SAMPLES: usize = 1024;

/// process() admits pscc only (ZERO exchanges); render() performs one exchange.
/// Feedback fitting, mic rendering, stop progress and cleanup own the SAME N.
pub(crate) trait ScheduledNative: CooperativeNative {
    fn configure_model(&mut self, model: &RadioModel) -> Result<(), Fault>;
    /// Zero-exchange, RX-only override. Unsupported ports must not silently
    /// inherit the microphone profile for an authorized diagnostic action.
    fn measurement_mic_gain(&mut self, _enabled: bool) -> Result<(), Fault> {
        Err(Fault::NativeUnverified)
    }
    /// RX/clean preparation only. A versioned native generator stays inside
    /// this same native owner; unsupported ports must refuse, never fall back.
    fn diagnostic_stimulus(
        &mut self,
        stimulus: super::feedback_owner::Stimulus,
    ) -> Result<(), Fault> {
        if stimulus == super::feedback_owner::Stimulus::NativeTwoTone {
            Err(Fault::NativeUnverified)
        } else {
            Ok(())
        }
    }
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
fn audio_permit_matches(stamp: IngressStamp, current: IngressStamp, phase: Phase) -> bool {
    let same_feedback = stamp.fence.generation == current.fence.generation
        && stamp.fence.tx_epoch == current.fence.tx_epoch;
    let key_boundary = phase == Phase::Active
        && stamp.fence.generation.checked_add(1) == Some(current.fence.generation)
        && stamp.fence.tx_epoch.checked_add(1) == Some(current.fence.tx_epoch);
    matches!(phase, Phase::AwaitingKey | Phase::Active)
        && stamp.output.is_some()
        && stamp.output == current.output
        && stamp.revision == current.revision
        && stamp.release == current.release
        && stamp.fence.boot == current.fence.boot
        && (same_feedback || key_boundary)
        && stamp.fence.config == current.fence.config
}

#[cfg(test)]
mod audio_permit_tests {
    use super::*;
    #[test]
    fn only_exact_key_feedback_boundary_can_keep_the_same_audio_permit() {
        let before = IngressStamp {
            fence: Fence {
                boot: 1,
                generation: 10,
                config: 3,
                tx_epoch: 20,
            },
            revision: 7,
            release: 2,
            output: Some(OutputStamp {
                generation: 4,
                revision: 7,
            }),
        };
        let mut keyed = before;
        keyed.fence.generation += 1;
        keyed.fence.tx_epoch += 1;
        assert!(audio_permit_matches(before, before, Phase::AwaitingKey));
        assert!(audio_permit_matches(before, keyed, Phase::Active));
        assert!(!audio_permit_matches(before, keyed, Phase::AwaitingKey));
        assert!(!audio_permit_matches(before, keyed, Phase::ReadyOff));
        for field in 0..9 {
            let mut changed = before;
            match field {
                0 => changed.fence.boot += 1,
                1 => changed.fence.config += 1,
                2 => changed.release += 1,
                3 => changed.revision += 1,
                4 => changed.output.as_mut().unwrap().generation += 1,
                5 => changed.output.as_mut().unwrap().revision += 1,
                6 => changed.fence.generation -= 1,
                7 => changed.fence.tx_epoch -= 1,
                _ => changed.output = None,
            }
            assert!(
                !audio_permit_matches(changed, keyed, Phase::Active),
                "field={field}"
            );
        }
        let mut exhausted = before;
        exhausted.fence.generation = u64::MAX;
        assert!(!audio_permit_matches(exhausted, keyed, Phase::Active));
    }
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
    /// Zero disables the optional elapsed-transmit cutoff; source/feedback watchdogs remain active.
    pub(crate) tx_duration_ns: u64,
}
#[derive(Clone, Copy, Debug)]
#[repr(u64)]
pub(crate) enum StaleOrigin {
    Unknown,
    Duration,
    SourceStall,
    RetainedMic,
    Ingress,
    Output,
    Lifecycle,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct StaleSnapshot {
    pub(crate) fields: [u64; 8],
}
#[derive(Default, Debug)]
pub(crate) struct Counters {
    pub(crate) last_stale: Option<StaleSnapshot>,
    pub(crate) turns: u64,
    pub(crate) rejected: u64,
    pub(crate) mic_blocks: u64,
    pub(crate) iq_packets: u64,
    pub(crate) input_component_peak: f32,
    pub(crate) native_iq_component_peak: f32,
    pub(crate) configurations: u64,
    pub(crate) deadline_stops: u64,
    pub(crate) max_turn_ns: u64,
    // Only turns entered while keyed: cleanup must not hide the final active
    // turn in a process-wide maximum or erase it before the stop report.
    pub(crate) active_max_turn_ns: u64,
    pub(crate) max_pump_ns: u64,
    pub(crate) max_feedback_age_ns: u64,
    pub(crate) active_max_hook_ns: u64,
    pub(crate) active_max_publish_ns: u64,
    pub(crate) active_max_render_ns: u64,
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
    two_tone_ticket: Option<(super::ps_ingress::TwoToneTrial, u64)>,
    two_tone_deadline: Option<u64>,
    reported_lifecycle: Option<(Phase, u64)>,
    last_signal_report: Instant,
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
        commands: impl Into<Receiver<TxCommand>>,
        audio: impl Into<Receiver<Audio>>,
        feedback: impl Into<Receiver<Box<PairBlock>>>,
        origin: Instant,
        timing: Timing,
    ) -> Result<Self, Fault> {
        if timing.mic_age_ns == 0 || timing.source_stall_ns == 0 {
            return Err(Fault::InvalidConfig);
        }
        let seen_stop = life.status().stop_id;
        let settings_epoch = Arc::new(SettingsEpoch::new(
            life.offline_owner().cancellation_fence(),
        ));
        Ok(Self {
            life,
            commands: commands.into(),
            audio: audio.into(),
            feedback: feedback.into(),
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
            two_tone_ticket: None,
            two_tone_deadline: None,
            reported_lifecycle: None,
            last_signal_report: Instant::now(),
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
    pub(crate) fn attach_signal_capture(
        &mut self,
        p: crate::tx_signal_capture::shared::Producer,
    ) -> Result<(), Fault> {
        self.ingress
            .as_ref()
            .ok_or(Fault::NoController)?
            .signal_capture
            .set(p.clone())
            .map_err(|_| Fault::InvalidConfig)?;
        self.life.offline_owner().attach_signal_capture(p);
        Ok(())
    }
    fn publish_ingress(&mut self) {
        let publication_started = Instant::now();
        let status = self.life.status();
        // A stop notification is not a receipt. Keep the action outstanding
        // until this generation's backend AND native cleanup have completed.
        if self.two_tone_ticket.as_ref().is_some_and(|(_, stop)| {
            status.stop_id > *stop
                && status.phase == Phase::ReadyOff
                && status.cleanup_completed_ns.is_some()
        }) {
            if let Some((mut ticket, _)) = self.two_tone_ticket.take() {
                ticket.complete(
                    status
                        .primary_fault
                        .or(status.authority_fault)
                        .or(status.cleanup_fault),
                );
            }
        }
        // Active requires current-generation native IQ to have reached the
        // adapter. Do not label a previous generation's last peak as new
        // signal while a fresh arm is still waiting for its first samples.
        if status.phase == Phase::Active
            && self.last_signal_report.elapsed() >= std::time::Duration::from_secs(1)
        {
            eprintln!("saturn-bridge: shared TX signal phase={:?} stop_id={} input_component_peak={:.8} native_iq_component_peak={:.8} rendered_blocks={} completed_iq_packets={} active_max_hook_ns={} active_max_publish_ns={} active_max_render_ns={} physical_rf=unverified",
                status.phase, status.stop_id, self.counters.input_component_peak,
                self.counters.native_iq_component_peak, self.counters.mic_blocks,
                self.counters.iq_packets,self.counters.active_max_hook_ns,self.counters.active_max_publish_ns,self.counters.active_max_render_ns);
            self.last_signal_report = Instant::now();
        }
        let transition = (status.phase, status.stop_id);
        if self.reported_lifecycle != Some(transition) {
            // Keep the established lifecycle record compatible with deployed
            // readiness/rollback readers. Extra timing has its own record.
            eprintln!("saturn-bridge: shared TX lifecycle phase={:?} stop_id={} primary_fault={:?} authority_fault={:?} cleanup_fault={:?} mic_blocks={} iq_packets={} rejected={} max_turn_ns={} max_pump_ns={} max_feedback_age_ns={}",
                status.phase, status.stop_id, status.primary_fault, status.authority_fault,
                status.cleanup_fault, self.counters.mic_blocks, self.counters.iq_packets,
                self.counters.rejected, self.counters.max_turn_ns,
                self.counters.max_pump_ns, self.counters.max_feedback_age_ns);
            eprintln!("saturn-bridge: shared TX active timing phase={:?} stop_id={} active_max_turn_ns={} active_max_hook_ns={} active_max_publish_ns={} active_max_render_ns={}",
                status.phase, status.stop_id,
                self.counters.active_max_turn_ns, self.counters.active_max_hook_ns,
                self.counters.active_max_publish_ns, self.counters.active_max_render_ns);
            self.reported_lifecycle = Some(transition);
        }
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
        if status.phase == Phase::Active {
            self.counters.active_max_publish_ns = self.counters.active_max_publish_ns.max(
                publication_started
                    .elapsed()
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64,
            );
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
        // A matching key receipt advances BOTH feedback generation and epoch,
        // but not the authorized TX output permit. Preserve original source
        // stamps only across that exact adjacent key boundary. Cancellation,
        // re-arm, model/source/config changes still retire the output permit.
        audio_permit_matches(stamp, current, self.life.status().phase)
    }
    fn discard_local(&mut self) {
        self.satp_started = false;
        self.mic.clear();
        self.mic_stamp = None;
        self.armed = None;
        self.two_tone_deadline = None;
        self.last_arrival = None;
        if self.satp.as_ref().is_some_and(|s| s.is_bound()) {
            self.satp.as_mut().unwrap().invalidate();
            self.satp_prepared = None;
        }
    }
    fn fault(&mut self, fault: Fault, now: u64) {
        self.fault_from(fault, now, StaleOrigin::Unknown);
    }
    fn record_stale(&mut self, now: u64, origin: StaleOrigin, oldest: Option<u64>) {
        if self.counters.last_stale.is_some() {
            return;
        } // preserve the first stop reason until fresh authorized arm
        let fields = [
            origin as u64,
            now,
            self.armed.unwrap_or(u64::MAX),
            self.last_arrival.unwrap_or(u64::MAX),
            oldest
                .or_else(|| self.mic.front().map(|s| s.1))
                .unwrap_or(u64::MAX),
            self.timing.tx_duration_ns,
            self.timing.source_stall_ns,
            self.timing.mic_age_ns,
        ];
        self.counters.last_stale = Some(StaleSnapshot { fields });
        self.life
            .offline_owner()
            .diagnostic_stale(fields, oldest.or_else(|| self.mic.front().map(|s| s.1)));
    }
    fn fault_from(&mut self, fault: Fault, now: u64, origin: StaleOrigin) {
        self.fault_observed(fault, now, origin, None);
    }
    fn fault_observed(&mut self, fault: Fault, now: u64, origin: StaleOrigin, oldest: Option<u64>) {
        if fault == Fault::Stale {
            self.record_stale(now, origin, oldest);
        }
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
            self.fault_from(fault, now, StaleOrigin::Output);
        } else {
            self.discard_local();
        }
    }
    fn command(&mut self, command: TxCommand, now: u64) -> Result<(), Fault> {
        match command {
            TxCommand::PureSignalFaultAck(mut request) => {
                let result = (|| {
                    let age = request
                        .received_at
                        .checked_duration_since(self.origin)
                        .and_then(|d| u64::try_from(d.as_nanos()).ok())
                        .ok_or(Fault::FutureObservation)?;
                    if now < age || now - age > self.timing.mic_age_ns {
                        return Err(Fault::Stale);
                    }
                    if self.shutdown
                        || self.pending_config.is_some()
                        || !self.configured
                        || !request.valid()
                        || request.stamp != self.snapshot()
                    {
                        return Err(Fault::GenerationMismatch);
                    }
                    self.life.acknowledge_fault(request.stop_id, now)?;
                    self.discard_local();
                    // Revoke queued credentials/settings at the acknowledged
                    // boundary; acknowledgment itself never opens the fence.
                    self.cancel_handle().invalidate_settings();
                    self.publish_ingress(); // committed state precedes result
                    Ok(())
                })();
                request.finish(result);
                result?;
            }
            TxCommand::PureSignalAuthorizedArm(mut arm) => {
                // A cancelled physical press is ordinary cancellation, not an
                // authorization fault. Never restamp it at the owner boundary.
                if arm.hardware_ptt.as_ref().is_some_and(|p| !p.live()) {
                    return Ok(());
                }
                let result = (|| {
                    if self
                        .ingress
                        .as_ref()
                        .is_some_and(|i| i.generated_trials_only())
                        && arm.trial.as_ref().is_none_or(|t| {
                            t.stimulus != super::feedback_owner::Stimulus::NativeTwoTone
                        })
                    {
                        return Err(Fault::InvalidConfig);
                    }
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
                    let trial_deadline = if let Some(t) = &arm.trial {
                        if self.selected != TxAudioSource::Tci || self.two_tone_ticket.is_some() {
                            return Err(Fault::InvalidConfig);
                        }
                        let requested = t
                            .requested_at
                            .checked_duration_since(self.origin)
                            .and_then(|d| u64::try_from(d.as_nanos()).ok())
                            .ok_or(Fault::FutureObservation)?;
                        if requested > now || now - requested > self.timing.mic_age_ns {
                            return Err(Fault::Stale);
                        }
                        if t.duration_ns == 0 {
                            None
                        } else {
                            Some(
                                requested
                                    .checked_add(t.duration_ns)
                                    .filter(|d| *d > now)
                                    .ok_or(Fault::Stale)?,
                            )
                        }
                    } else {
                        None
                    };
                    let action = self.life.offline_owner().authorize(arm.credential)?;
                    let stimulus = arm
                        .trial
                        .as_ref()
                        .map_or(Default::default(), |t| t.stimulus);
                    if arm.trial.is_some() {
                        self.life.offline_clean_configure(now, |n| {
                            n.measurement_mic_gain(true)?;
                            n.diagnostic_stimulus(stimulus)
                        })?;
                    }
                    self.life.offline_owner().measurement_stimulus(stimulus);
                    let arm_result = if self
                        .installed
                        .as_ref()
                        .is_some_and(|(_, c)| !c.enabled && !c.automatic)
                    {
                        self.life.fresh_bypass_tx_action(action, now)
                    } else {
                        self.life.fresh_tx_action(action, now)
                    };
                    if let Err(fault) = arm_result {
                        if arm.trial.is_some() && self.life.status().phase == Phase::ReadyOff {
                            self.life.offline_clean_configure(now, |n| {
                                n.diagnostic_stimulus(Default::default())?;
                                n.measurement_mic_gain(false)
                            })?;
                        }
                        return Err(fault);
                    }
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
                    self.counters.active_max_turn_ns = 0;
                    self.counters.active_max_hook_ns = 0;
                    self.counters.active_max_publish_ns = 0;
                    self.counters.active_max_render_ns = 0;
                    self.counters.last_stale = None;
                    self.last_arrival = Some(now);
                    self.next_mic = now;
                    self.next_iq = now;
                    self.two_tone_deadline = trial_deadline;
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        if let Some(ticket) = arm.trial.take() {
                            ticket.accepted();
                            self.two_tone_ticket = Some((ticket, self.life.status().stop_id));
                        }
                    }
                    Err(fault) => {
                        if let Some(mut ticket) = arm.trial.take() {
                            ticket.reject(fault);
                        }
                        return Err(fault);
                    }
                }
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
                    self.life.offline_clean_configure(now, |native| {
                        native.measurement_mic_gain(true)?;
                        native.diagnostic_stimulus(request.stimulus)
                    })?;
                    self.life
                        .offline_owner()
                        .measurement_stimulus(request.stimulus);
                    if let Err(fault) =
                        self.life
                            .fresh_measurement_action(action, remaining, request.recorder, now)
                    {
                        // A rejected action may leave us quiescent: restore
                        // immediately. A raced cancellation is already stopping
                        // and must finish its normal native cleanup instead.
                        if self.life.status().phase == Phase::ReadyOff {
                            self.life
                                .offline_owner()
                                .measurement_stimulus(Default::default());
                            self.life.offline_clean_configure(now, |native| {
                                native.diagnostic_stimulus(Default::default())?;
                                native.measurement_mic_gain(false)
                            })?;
                        }
                        return Err(fault);
                    }
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
                    self.counters.last_stale = None;
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
                if raw.is_empty()
                    || !raw.len().is_multiple_of(frame.channels as usize)
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
                    if self.mic.len() + mono.len() > TCI_MIC_STAGING_SAMPLES {
                        return Err(Fault::QueueOverrun);
                    }
                    self.mic.extend(mono.iter().map(|v| (*v, observed)));
                    self.mic_stamp = Some(audio.stamp);
                }
                self.life.offline_owner().note_source(
                    audio.stamp.output.ok_or(Fault::GenerationMismatch)?,
                    observed,
                )?;
                // Silence qualifies an AM carrier's source, never SSB speech.
                self.last_arrival = Some(self.last_arrival.unwrap_or(0).max(observed));
            }
            _ => return Err(Fault::InvalidConfig),
        }
        Ok(())
    }
    /// One bounded turn: lifecycle/release FIRST; one command, four audio
    /// messages, one feedback block, at most one render OR cleanup exchange.
    pub(crate) fn turn(&mut self, now: u64) -> scheduler::Turn {
        #[cfg(test)]
        self.life.offline_owner().set_test_key_time(now);
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
        self.life.offline_owner().set_test_key_time(now);
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
    #[cfg(test)]
    pub(crate) fn replace_feedback_receiver(&mut self, receiver: Receiver<Box<PairBlock>>) {
        self.feedback = receiver;
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
        now: u64,
        before_pump: impl FnOnce(&mut Self),
        realtime: bool,
        after_tick: impl FnOnce(&mut Self),
    ) -> scheduler::Turn {
        let start = Instant::now();
        let entered_active = self.life.status().phase == Phase::Active;
        let result = self.service_turn(now, before_pump, realtime, after_tick);
        let elapsed_ns = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        if entered_active {
            self.counters.active_max_turn_ns = self.counters.active_max_turn_ns.max(elapsed_ns);
        }
        self.counters.max_turn_ns = self.counters.max_turn_ns.max(elapsed_ns);
        result
    }
    fn service_turn(
        &mut self,
        mut now: u64,
        before_pump: impl FnOnce(&mut Self),
        realtime: bool,
        after_tick: impl FnOnce(&mut Self),
    ) -> scheduler::Turn {
        self.counters.turns += 1;
        let phase_before_tick = self.life.status().phase;
        let _ = self.life.tick(now); // faults are latched; continue stop progress
        if matches!(phase_before_tick, Phase::Active | Phase::AwaitingKey)
            && !matches!(self.life.status().phase, Phase::Active | Phase::AwaitingKey)
            && self.life.status().primary_fault == Some(Fault::Stale)
        {
            self.record_stale(now, StaleOrigin::Lifecycle, None);
        }
        after_tick(self);
        if let Some(fault) = self.ingress.as_ref().and_then(|i| i.take_fault()) {
            self.fault_from(fault, now, StaleOrigin::Ingress);
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
        // A matching key receipt starts a new feedback epoch, not a new TX
        // output permit. Keep bounded mic samples with their ORIGINAL ages
        // and stamp; audio_matches admits only that exact adjacent boundary.
        // Real cancellation/configuration/re-arm still discards all history.
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
            .two_tone_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.counters.deadline_stops += 1;
            self.cancel_handle().release();
            self.discard_local();
            // No exchange here: release has first refusal on the next service
            // step, including the health check before rendering in this turn.
        }
        if self.armed.is_some_and(|t| {
            self.timing.tx_duration_ns != 0 && now.saturating_sub(t) > self.timing.tx_duration_ns
        }) || self
            .last_arrival
            .is_some_and(|t| now.saturating_sub(t) > self.timing.source_stall_ns)
        {
            self.counters.deadline_stops += 1;
            let origin = if self.armed.is_some_and(|t| {
                self.timing.tx_duration_ns != 0
                    && now.saturating_sub(t) > self.timing.tx_duration_ns
            }) {
                StaleOrigin::Duration
            } else {
                StaleOrigin::SourceStall
            };
            self.fault_from(Fault::Stale, now, origin);
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
            if self.selected == TxAudioSource::Tci
                && matches!(self.life.status().phase, Phase::AwaitingKey | Phase::Active)
                && self.mic.len() > TCI_MIC_STAGING_SAMPLES - MAX_TCI_MONO_FRAME_SAMPLES
            {
                // Leave the next sealed frame in the existing bounded FIFO
                // until a render frees room for its largest permitted size.
                // Never drop/restamp it or enlarge either buffer. Ingress-full
                // faults, original-age checks, watchdogs and out-of-band
                // release remain independent and continue to run first.
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
                    self.counters.max_feedback_age_ns = self
                        .counters
                        .max_feedback_age_ns
                        .max(now.saturating_sub(block.meta.observed_ns));
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
                let start = Instant::now();
                let result = self.life.pump(now);
                self.counters.max_pump_ns = self
                    .counters
                    .max_pump_ns
                    .max(start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
                if result.is_err() {
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
            if self.snapshot().output.is_none() {
                // Retained mic stamps cannot stand in for current output
                // authority. output_error services a raced ordinary release
                // first, preserving cancellation versus a genuine fault.
                self.output_error(Fault::TxAuthority, now);
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
                    self.fault_observed(
                        Fault::Stale,
                        now,
                        StaleOrigin::RetainedMic,
                        Some(observed),
                    );
                } else {
                    // Fixed-clock zero fill continues native DSP during source
                    // pauses. It NEVER updates last_arrival or grants mic-level
                    // qualification; both watchdogs remain based on ingress.
                    self.counters.input_component_peak =
                        mic.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
                    let capture = self
                        .life
                        .offline_owner()
                        .diagnostic_submission(Some(observed));
                    let result = self
                        .life
                        .offline_owner()
                        .enqueue_mic(output_stamp, observed, &mic)
                        .and_then(|_| self.life.offline_owner().take_mic(output_stamp))
                        .and_then(|mic| {
                            if let Some(c) = &capture {
                                c.pcm(&mic);
                            }
                            let render_started = Instant::now();
                            let result = self
                                .life
                                .offline_tx_exchange(now, |native| native.render(&mic));
                            if self.life.status().phase == Phase::Active {
                                self.counters.active_max_render_ns =
                                    self.counters.active_max_render_ns.max(
                                        render_started
                                            .elapsed()
                                            .as_nanos()
                                            .min(u128::from(u64::MAX))
                                            as u64,
                                    );
                            }
                            result
                        })
                        .and_then(|iq| {
                            if let Some(c) = &capture {
                                c.wdsp(&iq);
                            }
                            self.counters.native_iq_component_peak =
                                iq.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
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
                let period = self.life.offline_owner().iq_period_ns();
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
            let active = self.life.status().phase == Phase::Active;
            let hook_started = Instant::now();
            ingress(self, now);
            if active {
                self.counters.active_max_hook_ns = self
                    .counters
                    .active_max_hook_ns
                    .max(hook_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
            }
            // Keepalive/observation hooks may take time. Deadlines use the
            // consumer time after those hooks, never the earlier timestamp.
            let current = self.origin.elapsed().as_nanos() as u64;
            let result = self.turn_inner(current, |_| {}, true, |_| {});
            self.publish_ingress();
            result
        });
    }
}
