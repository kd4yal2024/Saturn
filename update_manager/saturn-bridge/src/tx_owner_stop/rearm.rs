//! Bounded re-arm handoff. All envelopes originate BEFORE queuing; cancellation
//! erases capabilities and queues. Untagged legacy writers never reopen.
use super::*;
use crate::{
    tci::ps_authorization::Credential,
    tx_thread::{can_key_rf, KeyQualification, TxRadio},
};
use saturn_bridge::puresignal::{tx_lifecycle::StopCause, Fence};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputStamp {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
}
#[derive(Default)]
pub(super) struct Rearm {
    pub(super) capture_scope: Option<crate::tx_thread::feedback_owner::Scope>,
    enabled: bool,
    ready: Option<(u64, Fence, u64)>,
    grant: Option<Credential>,
    active: Option<(FreshTxAction, OutputStamp, Credential)>,
    qualification: Option<KeyQualification>,
    last_mic: Option<u64>,
    last_source: Option<u64>,
    queued_observed: Option<u64>,
    iq_spans: VecDeque<(usize, u64)>, // at most one partial tail plus one native block
    pending_mic_observed: Option<u64>,
    keyed: bool,
    manual_ptt: Option<OutputStamp>,
    direct_stage: crate::xdma_tx_radio::DirectKeyStage,
    model: Option<RadioModel>,
    max_watts: u8,
    production_rf_permission: Option<bool>,
    stimulus: crate::tx_thread::feedback_owner::Stimulus,
    auto_limits: Option<(f64, f64, u64)>,
    auto_selected: bool,
    retained_attenuation: Option<u8>,
    resume_correction: bool,
    retained_revision: u64,
    pub(super) auto_att: Option<super::auto_attenuation::Servo>,
    pub(super) held_key: Option<KeyReceipt>,
}
impl Rearm {
    pub(super) fn installed_model(&self) -> Option<&RadioModel> {
        self.model.as_ref()
    }
    pub(super) fn cancel(&mut self) {
        self.ready = None;
        self.grant = None;
        self.active = None;
        self.capture_scope = None;
        self.qualification = None;
        self.last_mic = None;
        self.last_source = None;
        self.queued_observed = None;
        self.iq_spans.clear();
        self.pending_mic_observed = None;
        self.keyed = false;
        self.manual_ptt = None;
        self.stimulus = Default::default();
        self.direct_stage = Default::default();
        self.auto_att = None;
        self.auto_selected = false;
        self.resume_correction = false;
        self.retained_revision = 0;
        self.held_key = None;
    }
    fn consume_iq(&mut self, mut count: usize) {
        while count > 0 {
            let (length, _) = self
                .iq_spans
                .front_mut()
                .expect("IQ metadata accompanies every sample");
            let consumed = count.min(*length);
            *length -= consumed;
            count -= consumed;
            if *length == 0 {
                self.iq_spans.pop_front();
            }
        }
        self.queued_observed = self.iq_spans.front().map(|span| span.1);
    }
}
impl OwnerStopAdapter<'_> {
    pub(crate) fn enable_auto_attenuation(
        &mut self,
        peak: f64,
        rail: f64,
        stale: u64,
    ) -> Result<(), Fault> {
        if self.feedback.is_none() || self.rearm.active.is_some() {
            return Err(Fault::InvalidConfig);
        }
        super::auto_attenuation::Servo::new(1, peak, rail, stale)?;
        self.rearm.auto_limits = Some((peak, rail, stale));
        Ok(())
    }
    pub(crate) fn auto_attenuation_for_arm(&mut self, selected: bool) -> Result<(), Fault> {
        if selected && self.rearm.auto_limits.is_none() {
            return Err(Fault::InvalidConfig);
        }
        self.rearm.auto_selected = selected;
        Ok(())
    }
    pub(crate) fn resume_correction_for_arm(&mut self, retained: bool) {
        self.rearm.resume_correction = retained;
    }
    pub(super) fn remember_release_attenuation(&mut self, cause: StopCause) {
        if cause != StopCause::Release {
            self.rearm.retained_attenuation = None;
        } else if let Some(scope) = self.rearm.capture_scope {
            self.rearm.retained_attenuation = Some(scope.attenuation_db);
        }
    }
    pub(crate) fn auto_attenuation_status(&self, faulted: bool) -> super::AutoAttStatus {
        if faulted {
            if let Some((_, snapshot, _, _)) = self.last_auto_att_fault {
                return snapshot;
            }
        }
        if self.rearm.auto_selected && self.rearm.resume_correction {
            if let Some(scope) = self.rearm.capture_scope {
                return super::AutoAttStatus {
                    phase: if self.rearm.keyed {
                        "retained"
                    } else {
                        "retained-pending"
                    },
                    db: scope.attenuation_db,
                    applied_db: self.rearm.keyed.then_some(scope.attenuation_db),
                    info: None,
                    revision: self.rearm.retained_revision,
                    keyed: self.rearm.keyed,
                };
            }
        }
        if let Some(a) = &self.rearm.auto_att {
            super::AutoAttStatus {
                phase: a.phase.label(),
                db: a.db,
                applied_db: a.applied_db,
                info: a.info,
                revision: a.revision,
                keyed: self.rearm.keyed,
            }
        } else {
            super::AutoAttStatus {
                phase: "off",
                db: 31,
                applied_db: None,
                info: None,
                revision: 0,
                keyed: false,
            }
        }
    }
    pub(crate) fn startup_leveling_active(&self) -> bool {
        self.rearm.held_key.is_some()
    }
    pub(crate) fn auto_attenuation_fault(&self) -> Option<(Fault, super::AutoAttStatus, f64, f64)> {
        self.last_auto_att_fault
    }
    /// Sole-owner startup feedback consumer. Returning true means intentionally
    /// consumed for leveling with native fitting/correction still disabled.
    pub(crate) fn level_feedback(
        &mut self,
        b: &saturn_bridge::puresignal::PairBlock,
        now: u64,
    ) -> Result<bool, Fault> {
        if self.rearm.held_key.is_none() {
            return Ok(false);
        }
        let scope = self.rearm.capture_scope.ok_or(Fault::InvalidConfig)?;
        self.validate_stamp(scope.output)?;
        if b.meta.fence != scope.fence {
            return Ok(true);
        }
        if b.meta.backend != self.backend() {
            return Err(Fault::WrongBackend);
        }
        if b.meta.route != scope.route {
            return Err(Fault::WrongRoute);
        }
        if b.meta.rate_hz != saturn_bridge::puresignal::RATE_HZ {
            return Err(Fault::WrongRate);
        }
        let level = self
            .rearm
            .auto_att
            .as_mut()
            .ok_or(Fault::InvalidConfig)?
            .observe(b, now)?;
        if let Some(db) = level {
            let scope = crate::tx_thread::feedback_owner::Scope {
                attenuation_db: db,
                ..scope
            };
            let serial = self
                .feedback
                .as_ref()
                .ok_or(Fault::InvalidConfig)?
                .0
                .request(Some(scope))?
                .serial;
            self.rearm
                .auto_att
                .as_mut()
                .unwrap()
                .requested(db, serial, now);
            self.rearm.capture_scope = Some(scope);
            // Private TX-owner snapshot drives P2 keepalive too. The saved
            // manual preference in installed/global configuration is untouched.
            self.rearm
                .model
                .as_mut()
                .ok_or(Fault::InvalidConfig)?
                .desired
                .pure_signal_attenuation_db = db;
        }
        Ok(true)
    }
    /// Only the existing owner sets this before arm_generation creates the
    /// acquisition scope. Cancellation clears it along with queued output.
    pub(crate) fn measurement_stimulus(
        &mut self,
        stimulus: crate::tx_thread::feedback_owner::Stimulus,
    ) {
        self.rearm.stimulus = stimulus;
    }
    #[cfg(test)]
    pub(crate) fn set_test_key_time(&self, now_ns: u64) {
        if let OutputOwner::Direct(radio) = &self.owner {
            radio.set_test_key_time(now_ns);
        }
    }
    #[cfg(feature = "managed-puresignal")]
    pub(crate) fn enable_production_handoff(
        &mut self,
        model: &RadioModel,
        max_watts: u8,
        rf_permitted: bool,
    ) -> Result<(), Fault> {
        // tx_enabled describes present MOX, not permission to arm. Do not
        // publish a fictitious key merely to satisfy the offline constructor.
        let mut target = model.clone();
        target.desired.tx_enabled = true;
        self.enable_offline_handoff(&target, max_watts)?;
        self.rearm.production_rf_permission = Some(rf_permitted);
        Ok(())
    }
    pub(super) fn cancellation_pending(&self) -> bool {
        self.rearm
            .active
            .as_ref()
            .is_some_and(|(action, stamp, _)| {
                self.fence.generation.load(Ordering::SeqCst) == stamp.generation
                    && self.fence.cancellation_ticket() != action.release_ticket
            })
    }
    pub(super) fn rearm_authorization_expired(&self) -> bool {
        self.rearm.enabled
            && self
                .rearm
                .active
                .as_ref()
                .is_none_or(|(_, stamp, credential)| {
                    !credential.valid()
                        || self.fence.revision() != stamp.revision
                        || self.fence.is_closed()
                })
    }
    // Explicit opt-in. Default stop-only adapters remain unable to re-arm.
    pub(crate) fn enable_offline_handoff(
        &mut self,
        model: &RadioModel,
        max_watts: u8,
    ) -> Result<(), Fault> {
        if self.request.is_some()
            || !model.desired.running
            || (!model.desired.tx_enabled && self.rearm.production_rf_permission.is_none())
            || model.desired.tx_drive == 0
            || model.desired.tx_drive > max_watts
            || model.desired.mode == crate::radio_model::DemodMode::Wfm
        {
            return Err(Fault::TxAuthority);
        }
        self.fence.managed.store(true, Ordering::SeqCst);
        self.fence.close();
        self.rearm.enabled = true;
        self.pending_mic.clear();
        self.pending_iq.clear();
        let mut target = model.clone();
        if self.rearm.production_rf_permission.is_some() {
            target.desired.tx_enabled = true;
        }
        self.rearm.model = Some(target);
        self.rearm.max_watts = max_watts;
        Ok(())
    }
    /// Model changes are snapshots, never mutable live-model reads at keying.
    /// The runtime invokes this only after the matching stop/native cleanup.
    pub(crate) fn install_clean_model(&mut self, model: &RadioModel) -> Result<(), Fault> {
        if self.rearm.ready.is_none()
            || !self.fence.is_closed()
            || !self.queues_empty()
            || !model.desired.running
            || (!model.desired.tx_enabled && self.rearm.production_rf_permission.is_none())
            || model.desired.tx_drive == 0
            || model.desired.tx_drive > self.rearm.max_watts
            || model.desired.two_tone_enabled
            || model.desired.mode == crate::radio_model::DemodMode::Wfm
        {
            return Err(Fault::InvalidConfig);
        }
        let mut target = model.clone();
        if self.rearm.production_rf_permission.is_some() {
            target.desired.tx_enabled = true;
        }
        self.rearm.model = Some(target);
        Ok(())
    }
    pub(crate) fn cancellation_fence(&self) -> Arc<OutputFence> {
        self.fence.clone()
    }
    pub(crate) fn current_revision(&self) -> u64 {
        self.fence.revision()
    }
    pub(super) fn mark_clean(&mut self, id: u64, fence: Fence) -> Result<(), Fault> {
        if !self.rearm.enabled {
            return Ok(());
        }
        if self.complete.is_none_or(|r| r.id != id)
            || !self.fence.is_closed()
            || !self.queues_empty()
        {
            return Err(Fault::TxAuthority);
        }
        self.rearm.ready = Some((id, fence, self.fence.revision()));
        self.rearm.grant = None; // an acknowledgment/configuration is never an arm replay
        Ok(())
    }
    pub(crate) fn ready_revision(&self) -> Result<u64, Fault> {
        self.rearm.ready.map(|r| r.2).ok_or(Fault::CleanupPending)
    }
    pub(crate) fn authorize(&mut self, credential: Credential) -> Result<FreshTxAction, Fault> {
        let (_, fence, revision) = self.rearm.ready.ok_or(Fault::CleanupPending)?;
        if !credential.valid()
            || credential.revision() != revision
            || self.fence.revision() != revision
            || credential.action().expected_fence != fence
            || self.rearm.grant.is_some()
        {
            return Err(Fault::GenerationMismatch);
        }
        let action = credential.action();
        self.rearm.grant = Some(credential);
        Ok(action)
    }
    pub(super) fn arm_generation(
        &mut self,
        action: FreshTxAction,
        generation: u64,
    ) -> Result<(), Fault> {
        if self.rearm.production_rf_permission == Some(false) {
            return Err(Fault::TxAuthority);
        }
        let (id, fence, revision) = self.rearm.ready.take().ok_or(Fault::TxAuthority)?;
        let grant = self.rearm.grant.take().ok_or(Fault::TxAuthority)?;
        if !grant.valid()
            || id != generation
            || action.expected_fence != fence
            || action.sequence != grant.action().sequence
            || action.release_ticket != grant.action().release_ticket
            || grant.revision() != revision
            || self.failed
            || !self.queues_empty()
        {
            return Err(Fault::TxAuthority);
        }
        self.fence.reopen(revision, generation)?;
        self.last_auto_att_fault = None;
        let required = match &self.owner {
            OutputOwner::P2 { session, .. } => session.key_qualification_packets(),
            OutputOwner::Direct(r) => r.key_qualification_packets(),
        };
        self.rearm.qualification = Some(KeyQualification::new(required));
        self.rearm.active = Some((
            action,
            OutputStamp {
                generation,
                revision,
            },
            grant,
        ));
        {
            let mut keyed_fence = action.expected_fence;
            // Controller::on_tx_state retires the pre-key acquisition
            // generation as well as advancing the TX epoch at key receipt.
            keyed_fence.generation = keyed_fence
                .generation
                .checked_add(1)
                .ok_or(Fault::GenerationMismatch)?;
            keyed_fence.tx_epoch = keyed_fence
                .tx_epoch
                .checked_add(1)
                .ok_or(Fault::GenerationMismatch)?;
            let mut scope = crate::tx_thread::feedback_owner::Scope {
                stimulus: self.rearm.stimulus,
                fence: keyed_fence,
                route: self.feedback.as_ref().map_or(1, |(_, route)| *route),
                frequency_hz: self
                    .rearm
                    .model
                    .as_ref()
                    .ok_or(Fault::InvalidConfig)?
                    .desired
                    .tx_frequency_hz,
                attenuation_db: self
                    .rearm
                    .model
                    .as_ref()
                    .ok_or(Fault::InvalidConfig)?
                    .desired
                    .pure_signal_attenuation_db
                    .min(31),
                output: OutputStamp {
                    generation,
                    revision,
                },
            };
            if self.rearm.auto_selected {
                scope.attenuation_db = if self.rearm.resume_correction {
                    self.rearm
                        .retained_attenuation
                        .ok_or(Fault::InvalidConfig)?
                } else {
                    super::auto_attenuation::START_DB
                };
                self.rearm
                    .model
                    .as_mut()
                    .ok_or(Fault::InvalidConfig)?
                    .desired
                    .pure_signal_attenuation_db = scope.attenuation_db;
            }
            self.rearm.capture_scope = Some(scope);
            if let Some((link, _)) = &self.feedback {
                let request = link.request(Some(scope))?;
                self.rearm.retained_revision = request.serial;
                if self.rearm.auto_selected && !self.rearm.resume_correction {
                    let (peak, rail, stale) = self.rearm.auto_limits.ok_or(Fault::InvalidConfig)?;
                    self.rearm.auto_att = Some(super::auto_attenuation::Servo::new(
                        request.serial,
                        peak,
                        rail,
                        stale,
                    )?);
                }
            }
        }
        self.complete = None; // stale stop receipts cannot appear after reopening
        self.request = None;
        Ok(())
    }
    pub(crate) fn output_stamp(&self) -> Result<OutputStamp, Fault> {
        self.rearm
            .active
            .as_ref()
            .map(|a| a.1)
            .ok_or(Fault::CleanupPending)
    }
    /// Publication must not advertise a retired permit between atomic
    /// cancellation and the lifecycle's next release service.
    pub(crate) fn publication_output_stamp(&self) -> Option<OutputStamp> {
        let stamp = self.output_stamp().ok()?;
        self.validate_stamp(stamp).ok().map(|_| stamp)
    }
    fn validate_stamp(&self, stamp: OutputStamp) -> Result<(), Fault> {
        let (_, current, grant) = self
            .rearm
            .active
            .as_ref()
            .ok_or(Fault::GenerationMismatch)?;
        if *current != stamp
            || self.fence.revision() != stamp.revision
            || !grant.valid()
            || self.fence.is_closed()
        {
            return Err(Fault::GenerationMismatch);
        }
        Ok(())
    }
    /// Set only after the scheduling owner admits an ordinary operator arm.
    /// The validated current output stamp cannot authorize another generation.
    pub(crate) fn manual_ptt(&mut self, stamp: OutputStamp) -> Result<(), Fault> {
        self.validate_stamp(stamp)?;
        self.rearm.manual_ptt = Some(stamp);
        Ok(())
    }
    pub(crate) fn enqueue_mic(
        &mut self,
        stamp: OutputStamp,
        observed: u64,
        samples: &[f32],
    ) -> Result<(), Fault> {
        self.validate_stamp(stamp)?;
        if samples.len() != 512 || samples.iter().any(|v| !v.is_finite()) {
            return Err(Fault::InvalidConfig);
        }
        // Same owner's pending_mic type, bounded to one native input block.
        self.pending_mic.clear();
        self.pending_mic.extend(samples.iter().copied());
        self.rearm.pending_mic_observed = Some(observed);
        Ok(())
    }
    /// Original ingress observation only, after format/age/generation checks.
    /// Rendering silence and draining queues cannot refresh this evidence.
    pub(crate) fn note_source(&mut self, stamp: OutputStamp, observed: u64) -> Result<(), Fault> {
        self.validate_stamp(stamp)?;
        self.rearm.last_source = Some(self.rearm.last_source.unwrap_or(0).max(observed));
        Ok(())
    }
    pub(crate) fn take_mic(&mut self, stamp: OutputStamp) -> Result<[f32; 512], Fault> {
        self.validate_stamp(stamp)?;
        if self.pending_mic.len() != 512 {
            return Err(Fault::UnequalLength);
        }
        let samples = std::array::from_fn(|_| self.pending_mic.pop_front().unwrap());
        if samples
            .iter()
            .any(|v| v.abs() >= crate::tx_thread::TX_KEY_MIC_PEAK_THRESHOLD)
        {
            self.rearm.last_mic = self.rearm.pending_mic_observed;
        }
        self.rearm.pending_mic_observed = None;
        Ok(samples)
    }
    pub(crate) fn enqueue_iq(
        &mut self,
        stamp: OutputStamp,
        observed: u64,
        iq: &[f32],
    ) -> Result<(), Fault> {
        self.validate_stamp(stamp)?;
        if iq.is_empty()
            || !iq.len().is_multiple_of(2)
            || iq.len() > 4096
            || iq.iter().any(|v| !v.is_finite())
            || self.pending_iq.len() >= 480
            || self.rearm.iq_spans.len() >= 2
        {
            return Err(Fault::InvalidConfig);
        }
        self.pending_iq.extend(iq.iter().copied());
        self.rearm.iq_spans.push_back((iq.len(), observed));
        self.rearm.queued_observed = self.rearm.iq_spans.front().map(|span| span.1);
        Ok(())
    }
    pub(crate) fn has_iq_packet(&self) -> bool {
        self.pending_iq.len() >= 480
    }
    #[cfg(feature = "managed-puresignal")]
    pub(crate) fn keyed_keepalive(&self) -> Result<(), Fault> {
        if !self.rearm.keyed {
            return Ok(());
        }
        let stamp = self.output_stamp()?;
        self.validate_stamp(stamp)?;
        if let OutputOwner::P2 { session, .. } = &self.owner {
            match session.shared_keyed_keepalive(
                stamp.generation,
                self.rearm.model.as_ref().ok_or(Fault::InvalidConfig)?,
            ) {
                Err(Fault::GenerationMismatch) if self.cancellation_pending() => return Ok(()),
                Err(fault) => return Err(fault),
                Ok(()) => {}
            }
        }
        Ok(())
    }
    /// Disconnect notification uses the same out-of-band release plus stop
    /// lifecycle as release/fault. Revoke identity before dispatching it.
    pub(crate) fn disconnect_output(&mut self) {
        self.fence.close();
        self.rearm.cancel();
        self.pending_iq.clear();
        self.pending_mic.clear();
    }
    pub(super) fn key_generation(&mut self, now: u64) -> Result<Option<KeyReceipt>, Fault> {
        let stamp = self.output_stamp().map_err(|_| Fault::TxAuthority)?;
        if self.validate_stamp(stamp).is_err() {
            if self.cancellation_pending() {
                return Ok(None);
            }
            self.disconnect_output();
            return Err(Fault::TxAuthority);
        }
        if self.rearm.keyed {
            if let Some(receipt) = self.rearm.held_key {
                let prepared = self
                    .feedback
                    .as_ref()
                    .ok_or(Fault::InvalidConfig)?
                    .0
                    .prepared(stamp)?;
                let servo = self.rearm.auto_att.as_mut().ok_or(Fault::InvalidConfig)?;
                servo.tick(now)?;
                if prepared {
                    servo.applied(now)
                }
                if prepared
                    && servo.ready_ns.is_some()
                    && matches!(
                        servo.phase,
                        auto_attenuation::Phase::Stable | auto_attenuation::Phase::Limited
                    )
                    && servo.applied_db == Some(servo.db)
                {
                    self.rearm.held_key = None;
                    return Ok(Some(receipt));
                }
            }
            return Ok(None);
        }
        if let Some((link, _)) = &self.feedback {
            if !link.prepared(stamp)? {
                return Ok(None);
            }
        }
        if self.pending_iq.len() < 480 {
            return Ok(None);
        }
        let (window, two_tone) = match &self.owner {
            OutputOwner::P2 { session, .. } => (
                session.keyable_mic_window(),
                self.rearm.model.as_ref().unwrap().desired.two_tone_enabled,
            ),
            OutputOwner::Direct(r) => (r.keyable_mic_window(), false), // never silently bypass mic gate
        };
        let recent = self
            .rearm
            .last_mic
            .is_some_and(|t| t <= now && now - t <= window.as_nanos() as u64);
        let source_recent = self
            .rearm
            .last_source
            .is_some_and(|t| t <= now && now - t <= window.as_nanos() as u64);
        // Only the sealed, versioned diagnostic action can replace SSB mic
        // level with its genuine zero-input ingress liveness. Nonzero native
        // IQ and the existing consecutive-packet gate remain required below.
        let qualified =
            if self.rearm.stimulus == crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone {
                source_recent
            } else {
                crate::tx_thread::key_source_qualified(
                    self.rearm
                        .model
                        .as_ref()
                        .ok_or(Fault::InvalidConfig)?
                        .desired
                        .mode,
                    recent,
                    source_recent,
                )
            };
        let observed = self.rearm.queued_observed.ok_or(Fault::TxAuthority)?;
        if observed > now || now - observed > window.as_nanos() as u64 {
            return Err(Fault::Stale);
        }
        let mut iq = [0.; 480];
        for (destination, source) in iq.iter_mut().zip(self.pending_iq.iter()) {
            *destination = *source;
        }
        let peak = iq.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
        // Manual PTT is not VOX. A fresh, authorized source may key in
        // silence, before the first syllable. Native recreation, generation,
        // source-age and FIFO checks still apply; zero fill cannot establish
        // source liveness. Diagnostic actions keep their signal qualification.
        let manual = self.rearm.manual_ptt == Some(stamp)
            && matches!(
                self.rearm
                    .model
                    .as_ref()
                    .ok_or(Fault::InvalidConfig)?
                    .desired
                    .mode,
                crate::radio_model::DemodMode::Usb
                    | crate::radio_model::DemodMode::Lsb
                    | crate::radio_model::DemodMode::Dsb
                    | crate::radio_model::DemodMode::DigU
                    | crate::radio_model::DemodMode::DigL
                    | crate::radio_model::DemodMode::Am
                    | crate::radio_model::DemodMode::Sam
                    | crate::radio_model::DemodMode::Fm
            );
        let ready = if manual {
            source_recent
        } else {
            self.rearm
                .qualification
                .as_mut()
                .unwrap()
                .observe(can_key_rf(true, peak, qualified, two_tone))
        };
        if !ready {
            self.pending_iq.drain(..480);
            self.rearm.consume_iq(480);
            return Ok(None);
        }
        #[cfg(test)]
        if let Some(boundary) = self.before_key_permit.take() {
            boundary();
        }
        let diagnostic = self.diagnostic_submission(Some(observed));
        let keyed = match &self.owner {
            OutputOwner::P2 { session, .. } => session.offline_key_generation(
                stamp.generation,
                self.rearm.model.as_ref().unwrap(),
                &iq,
            ),
            OutputOwner::Direct(r) => r.offline_key_generation_captured(
                stamp.generation,
                self.rearm.model.as_ref().unwrap(),
                &mut self.rearm.direct_stage,
                &iq,
                diagnostic.as_ref(),
            ),
        };
        let keyed = match keyed {
            Ok(keyed) => keyed,
            Err(Fault::GenerationMismatch) if self.cancellation_pending() => return Ok(None),
            Err(Fault::GenerationMismatch) => return Err(Fault::TxAuthority),
            Err(fault) => return Err(fault), // genuine backend/authorization error stays a fault
        };
        if !keyed {
            return Ok(None);
        }
        self.pending_iq.drain(..480);
        self.rearm.consume_iq(480);
        self.rearm.keyed = true;
        let action = self.rearm.active.as_ref().unwrap().0;
        let receipt = KeyReceipt {
            action_sequence: action.sequence,
            output_generation: stamp.generation,
            keyed_ns: now,
        };
        if let Some(servo) = self.rearm.auto_att.as_mut() {
            servo.keyed(now);
            self.rearm.held_key = Some(receipt);
            Ok(None)
        } else {
            Ok(Some(receipt))
        }
    }
    pub(crate) fn flush_iq(&mut self, stamp: OutputStamp, now: u64) -> Result<bool, Fault> {
        self.validate_stamp(stamp)?;
        if !self.rearm.keyed {
            return Err(Fault::CleanupPending);
        }
        if self.pending_iq.len() < 480 {
            return Ok(false);
        }
        let t = self.rearm.queued_observed.ok_or(Fault::TxAuthority)?;
        if t > now || now - t > 150_000_000 {
            return Err(Fault::Stale);
        }
        let mut iq = [0.; 480];
        for (destination, source) in iq.iter_mut().zip(self.pending_iq.iter()) {
            *destination = *source;
        }
        let diagnostic = self.diagnostic_submission(Some(t));
        let done = match &self.owner {
            OutputOwner::P2 { session, .. } => session
                .offline_iq_generation(stamp.generation, &iq)
                .map_err(|_| Fault::TxAuthority)?,
            OutputOwner::Direct(r) => {
                r.offline_iq_generation_captured(stamp.generation, &iq, diagnostic.as_ref())?
            }
        };
        if done {
            if let OutputOwner::Direct(radio) = &self.owner {
                radio.monitor_iq(
                    &iq,
                    self.rearm
                        .model
                        .as_ref()
                        .ok_or(Fault::InvalidConfig)?
                        .desired
                        .mode,
                );
            }
            self.pending_iq.drain(..480);
            self.rearm.consume_iq(480);
        }
        Ok(done)
    }
    pub(crate) fn iq_period_ns(&self) -> u64 {
        match self.owner {
            // UDP has no hardware FIFO-space readback; retain packet pacing.
            OutputOwner::P2 { .. } => 240_000_000_000 / 192_000,
            // The direct writer already checks real FIFO room before EVERY
            // write. A second nominal-rate timer accumulates scheduling jitter
            // into old mic/IQ output. Service it cooperatively each turn;
            // full FIFO remains backpressure, never permission to overwrite.
            OutputOwner::Direct(_) => 0,
        }
    }
}
