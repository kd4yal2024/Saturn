//! Explicit, disabled-by-default route from the actual TCI/SATP producers.
//! Network producers enqueue sealed observations; they never touch native DSP.
//! Installation is an all-or-nothing owner selection, not a legacy tee.
#![allow(dead_code)]
use super::{
    ps_owner_loop::{
        Audio, Cancel, Configuration, IngressStamp, SettingsEpoch, SettingsEpochValue,
    },
    ps_owner_queue::AuthorizedArm,
    TxCommand,
};
use crate::{
    radio_model::{DemodMode, RadioModel},
    sync_ext::MutexExt,
    tci::{
        ps_authorization::{OperatorIngress, SettingsPermission},
        TciCommand,
    },
    tx_audio::{TxAudioFrame, TxAudioMessage, TxAudioSamples, TxAudioSource},
};
use saturn_bridge::puresignal::{
    controller::Config,
    tx_lifecycle::{LifecycleStatus, Phase},
    Fault, PairBlock,
};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::{
    io,
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    time::Instant,
};

#[derive(Clone, Debug)]
pub(crate) struct Capture {
    pub(crate) stamp: IngressStamp,
    model: RadioModel,
    config: Config,
    source: TxAudioSource,
    phase: Phase,
    pub(crate) received_at: Instant,
    settings_epoch: SettingsEpochValue,
}
struct View {
    status: LifecycleStatus,
    stamp: IngressStamp,
    phase: Phase,
    installed: Option<(RadioModel, Config)>,
    source: TxAudioSource,
    settings_epoch: SettingsEpochValue,
    clean: bool,
}
#[derive(Clone, Copy)]
enum SettingsOutcome {
    Pending,
    Applied(IngressStamp),
    Rejected(Fault),
}
struct Receipt {
    id: u64,
    client: u64,
    config: u128,
    operator: Arc<OperatorIngress>,
    outcome: Mutex<SettingsOutcome>,
    cancel: Cancel,
}
impl Receipt {
    fn reply(&self, status: &str) {
        self.operator.settings_reply(
            self.client,
            format!("tx_config:0,{},{status},{};", self.id, self.config),
            || self.cancel.release(),
        );
    }
    fn finish(&self, outcome: SettingsOutcome) {
        let mut state = self.outcome.lock_unpoisoned();
        if !matches!(*state, SettingsOutcome::Pending) {
            return;
        }
        *state = outcome;
        match outcome {
            SettingsOutcome::Applied(_) => self.reply("applied"),
            SettingsOutcome::Rejected(fault) => self.reply(&format!("rejected,{fault:?}")),
            SettingsOutcome::Pending => unreachable!(),
        }
    }
}
/// Settings-only continuation: a predecessor must have completed at the exact
/// current owner boundary. Release/fault/arm invalidate the sealed epoch.
/// Neither this permission nor a completion receipt can authorize TX output.
pub(super) struct SettingsTransaction {
    receipt: Arc<Receipt>,
    predecessor: Option<Arc<Receipt>>,
    permission: SettingsPermission,
    epoch: Arc<SettingsEpoch>,
    issued_epoch: SettingsEpochValue,
}
impl SettingsTransaction {
    pub(super) fn live(&self) -> bool {
        self.permission.valid()
            && !self.issued_epoch.exhausted()
            && self.epoch.current() == self.issued_epoch
    }
    pub(super) fn admit(&self, original: IngressStamp, current: IngressStamp) -> Result<(), Fault> {
        if !self.permission.valid() {
            return Err(Fault::TxAuthority);
        }
        if !self.live()
            || original.release != current.release
            || original.fence.boot != current.fence.boot
        {
            return Err(Fault::GenerationMismatch);
        }
        match &self.predecessor {
            None if original == current => Ok(()),
            Some(p) => match *p.outcome.lock_unpoisoned() {
                SettingsOutcome::Applied(boundary) if boundary == current => Ok(()),
                SettingsOutcome::Rejected(fault) => Err(fault),
                _ => Err(Fault::GenerationMismatch),
            },
            _ => Err(Fault::GenerationMismatch),
        }
    }
    pub(super) fn applied(&self, boundary: IngressStamp) {
        self.receipt.finish(SettingsOutcome::Applied(boundary));
    }
    pub(super) fn reject(&self, fault: Fault) {
        self.receipt.finish(SettingsOutcome::Rejected(fault));
    }
}
struct SettingsTail {
    epoch: SettingsEpochValue,
    original: IngressStamp,
    model: RadioModel,
    config: Config,
    source: TxAudioSource,
    receipt: Arc<Receipt>,
}
#[derive(Clone)]
pub(crate) struct SharedIngress {
    commands: SyncSender<TxCommand>,
    audio: SyncSender<Audio>,
    feedback: SyncSender<Box<PairBlock>>,
    feedback_pool: Arc<Mutex<Vec<Box<PairBlock>>>>,
    operator: Arc<OperatorIngress>,
    cancel: Cancel,
    view: Arc<Mutex<Option<View>>>,
    fault: Arc<Mutex<Option<Fault>>>,
    model: Arc<Mutex<RadioModel>>,
    selected: Arc<AtomicU8>,
    next_settings: Arc<AtomicU64>,
    settings_tail: Arc<Mutex<Option<SettingsTail>>>,
    media: Arc<crate::tci::ps_media::RemoteMedia>,
    owner_status: Arc<Mutex<(u64, String, String)>>, // serial, fields, published frame
}
pub(crate) struct Receivers {
    pub(crate) commands: Receiver<TxCommand>,
    pub(crate) audio: Receiver<Audio>,
    pub(crate) feedback: Receiver<Box<PairBlock>>,
}
impl SharedIngress {
    /// Explicit host diagnostic entry, deliberately NOT wired to ordinary trx,
    /// a browser setting, startup, reconnect, or any live endpoint. An eventual
    /// reviewed RF procedure must invoke this on a NEW operator action only.
    /// Capture is sealed before queuing; authority is never restamped later.
    pub(crate) fn measurement_only(
        &self,
        client: u64,
        capture: &Capture,
        duration_ns: u64,
        recorder: saturn_bridge::puresignal::measurement::MeasurementRecorder,
    ) -> Result<(), Fault> {
        let context = recorder.context();
        if context.physical_adc != capture.config.physical_adc
            || context.commanded_drive != Some(capture.model.desired.tx_drive)
            || context.feedback_attenuation_db
                != Some(capture.model.desired.pure_signal_attenuation_db)
        {
            return Err(Fault::InvalidConfig);
        }
        let credential = self.operator.receive(
            client,
            "trx:0,true",
            capture.stamp.release,
            capture.stamp.fence,
            capture.stamp.revision,
        )?;
        let request = super::ps_owner_queue::AuthorizedMeasurement::new(
            credential,
            duration_ns,
            capture.received_at,
            recorder,
        )?;
        self.send_command(TxCommand::MeasurementOnly(request))
    }
    pub(super) fn settings_epoch(&self) -> Arc<SettingsEpoch> {
        self.cancel.settings_epoch.clone()
    }
    pub(crate) fn channels(
        operator: OperatorIngress,
        cancel: Cancel,
        model: Arc<Mutex<RadioModel>>,
    ) -> (Self, Receivers) {
        let (commands, crx) = mpsc::sync_channel(16);
        let (audio, arx) = mpsc::sync_channel(16);
        let (feedback, frx) = mpsc::sync_channel(2);
        (
            Self {
                commands,
                audio,
                feedback,
                feedback_pool: Arc::new(Mutex::new(
                    (0..4).map(|_| Box::new(PairBlock::default())).collect(),
                )),
                operator: Arc::new(operator),
                cancel,
                view: Arc::new(Mutex::new(None)),
                fault: Arc::new(Mutex::new(None)),
                model,
                selected: Arc::new(AtomicU8::new(0)),
                next_settings: Arc::new(AtomicU64::new(0)),
                settings_tail: Arc::new(Mutex::new(None)),
                media: Arc::new(crate::tci::ps_media::RemoteMedia::default()),
                owner_status: Arc::new(Mutex::new((0, String::new(), String::new()))),
            },
            Receivers {
                commands: crx,
                audio: arx,
                feedback: frx,
            },
        )
    }
    pub(super) fn publish(
        &self,
        stamp: IngressStamp,
        status: LifecycleStatus,
        installed: Option<(RadioModel, Config)>,
        source: TxAudioSource,
    ) {
        let phase = status.phase;
        let config_id = installed.as_ref().map_or(0, |(_, c)| c.id);
        let fault = status
            .primary_fault
            .or(status.authority_fault)
            .or(status.cleanup_fault);
        let clean = phase == Phase::ReadyOff
            && installed.is_some()
            && status.cleanup_completed_ns.is_some()
            && fault.is_none();
        let fields = format!(
            "{:?},{},{},{},{},{}",
            phase,
            status.stop_id,
            config_id,
            if source == TxAudioSource::Satp {
                "satp"
            } else {
                "tci"
            },
            clean,
            fault.map_or_else(|| "none".into(), |f| format!("{f:?}"))
        );
        let owner_reply = {
            let mut published = self.owner_status.lock_unpoisoned();
            if published.1 == fields {
                None
            } else {
                if let Some(serial) = published.0.checked_add(1) {
                    *published = (
                        serial,
                        fields.clone(),
                        format!("saturn_tx_owner:shared,{serial},{fields};"),
                    );
                    Some(published.2.clone())
                } else {
                    self.release();
                    None
                }
            }
        };
        let changed = self
            .view
            .lock_unpoisoned()
            .as_ref()
            .and_then(|v| v.installed.as_ref().map(|(_, c)| c.id))
            != installed.as_ref().map(|(_, c)| c.id);
        if let Some((snapshot, config)) = &installed {
            let mut m = self.model.lock_unpoisoned();
            if changed {
                // Commit only TX-owned fields. Repeated TX status publication
                // must not restore stale RX volume/ADC/display/client state.
                macro_rules! copy {($($field:ident),* $(,)?)=>{$(m.desired.$field=snapshot.desired.$field;)*};}
                copy!(
                    mode,
                    vfo_a_hz,
                    vfo_b_hz,
                    active_vfo,
                    split_enabled,
                    tx_drive,
                    tx_mic_gain_db,
                    tx_filter_low_hz,
                    tx_filter_high_hz,
                    tx_eq_enabled,
                    tx_eq_bands,
                    cfc_enabled,
                    cfc_precomp_db,
                    cfc_bands,
                    tx_phase_rotator_enabled,
                    tx_phase_rotator_auto,
                    tx_phase_rotator_corner_hz,
                    tx_noise_gate_enabled,
                    tx_noise_gate_threshold_db,
                    tx_dexp_enabled,
                    tx_dexp_threshold_db,
                    tx_dexp_expansion_db,
                    tx_speech_processor_enabled,
                    tx_speech_processor_gain_db,
                    tx_cessb_enabled,
                    tx_fft_size,
                    tx_low_latency,
                    pure_signal_attenuation_db
                );
                m.sync_vfo_routes();
                m.desired.iq_center_hz = snapshot.desired.iq_center_hz;
                m.desired.pure_signal_enabled = config.enabled;
            }
            m.satp.source = source; // preserve authentication/lease/acquisition state
            m.satp.pending_source = false;
            m.desired.tx_enabled = matches!(phase, Phase::Active | Phase::AwaitingKey);
            m.desired.tx_phase = match phase {
                Phase::Active => crate::radio_model::TxPhase::Keyed,
                Phase::AwaitingKey => crate::radio_model::TxPhase::Armed,
                _ => crate::radio_model::TxPhase::Rx,
            };
        }
        *self.view.lock_unpoisoned() = Some(View {
            status,
            clean,
            stamp,
            phase,
            installed,
            source,
            settings_epoch: self.cancel.settings_epoch.current(),
        });
        self.selected.store(
            if matches!(phase, Phase::AwaitingKey | Phase::Active) {
                if source == TxAudioSource::Satp {
                    2
                } else {
                    1
                }
            } else {
                0
            },
            Ordering::Release,
        );
        match self.media.publish(
            stamp,
            source == TxAudioSource::Tci && matches!(phase, Phase::Active | Phase::AwaitingKey),
        ) {
            Ok(Some(text)) => self.operator.media_reply(text, || self.release()),
            Err(fault) => self.stop(fault),
            _ => {}
        }
        if let Some(text) = owner_reply {
            self.operator.media_reply(text, || self.release());
        }
    }
    pub(crate) fn owner_status_message(&self) -> Option<String> {
        let published = self.owner_status.lock_unpoisoned();
        (!published.2.is_empty()).then(|| published.2.clone())
    }
    pub(crate) fn enable_tagged_pcm_review(&self) -> Result<(), Fault> {
        if self
            .view
            .lock_unpoisoned()
            .as_ref()
            .is_none_or(|v| v.installed.is_some())
        {
            return Err(Fault::InvalidConfig);
        }
        self.media.enable_tagged_pcm_review()
    }
    pub(crate) fn managed_opus_available(&self) -> bool {
        self.remote_media_review() && !self.media.pcm_only()
    }
    /// Separate explicit offline profile, before initial configuration/arming.
    /// Integration selects it before attaching networking. No URL,
    /// environment override or installed startup calls this selection.
    pub(crate) fn enable_remote_media_review(&self) -> Result<(), Fault> {
        if self
            .view
            .lock_unpoisoned()
            .as_ref()
            .is_none_or(|v| v.installed.is_some())
        {
            return Err(Fault::InvalidConfig);
        }
        self.media.enable_offline_review()
    }
    pub(crate) fn remote_media_review(&self) -> bool {
        self.media.enabled()
    }
    pub(crate) fn remote_media_caps(&self) -> &'static str {
        self.media.caps()
    }
    pub(crate) fn remote_media_token(&self, capture: &Capture) -> Option<[u8; 16]> {
        self.media.token(capture.stamp)
    }
    pub(crate) fn remote_frame_current(&self, c: &Capture, data: &[u8]) -> bool {
        data.len() >= 64
            && u32::from_le_bytes(data[60..64].try_into().unwrap()) == crate::tci::ps_media::MAGIC
            && self
                .remote_media_token(c)
                .is_some_and(|t| data[44..60] == t)
    }
    pub(crate) fn stop_remote(&self, c: &Capture, fault: Fault) {
        if c.stamp
            .output
            .is_some_and(|s| self.cancel.release_generation(s))
        {
            self.fault.lock_unpoisoned().get_or_insert(fault);
        }
    }
    pub(crate) fn stop_feedback(
        &self,
        output: crate::tx_owner_stop::OutputStamp,
        fault: Fault,
    ) -> bool {
        if self.cancel.release_generation(output) {
            self.fault.lock_unpoisoned().get_or_insert(fault);
            true
        } else {
            false
        }
    }
    pub(crate) fn remote_mic(
        &self,
        capture: &Capture,
        client: u64,
        data: &[u8],
    ) -> Result<(), Fault> {
        if !self.selected_active(TxAudioSource::Tci) || capture.source != TxAudioSource::Tci {
            return Err(Fault::GenerationMismatch);
        }
        let (samples, oldest) = self.media.decode(
            capture.stamp,
            client,
            capture.received_at,
            data,
            Instant::now(),
        )?;
        let mut retained = capture.clone();
        retained.received_at = oldest;
        if !self.selected_active(TxAudioSource::Tci) {
            return Err(Fault::GenerationMismatch);
        }
        self.enqueue_audio(
            &retained,
            TxAudioFrame {
                enqueued_at: oldest,
                source: TxAudioSource::Tci,
                samples: TxAudioSamples::Dynamic(samples),
                channels: 1,
                sample_rate_hz: 48000,
            },
        )
        .map_err(|_| Fault::QueueOverrun)
    }
    pub(crate) fn capture(&self, received_at: Instant) -> Option<Capture> {
        // This short producer-only snapshot lock carries no native or device
        // work. Cancellation never takes it; preserve age across any wait.
        let guard = self.view.lock_unpoisoned();
        let v = guard.as_ref()?;
        let (model, config) = v.installed.as_ref()?;
        Some(Capture {
            stamp: v.stamp,
            model: model.clone(),
            config: *config,
            source: v.source,
            phase: v.phase,
            received_at,
            settings_epoch: v.settings_epoch,
        })
    }
    pub(crate) fn release(&self) {
        self.cancel.release();
    }
    pub(crate) fn stop(&self, fault: Fault) {
        self.release(); // closes the output fence BEFORE any mutex/queue work
        let mut pending = self.fault.lock_unpoisoned();
        pending.get_or_insert(fault);
    }
    pub(super) fn take_fault(&self) -> Option<Fault> {
        self.fault.try_lock().ok()?.take()
    }
    pub(crate) fn selected_active(&self, source: TxAudioSource) -> bool {
        self.cancel.output_open()
            && self.selected.load(Ordering::Acquire)
                == if source == TxAudioSource::Satp { 2 } else { 1 }
    }
    pub(crate) fn pipeline_state(&self) -> &'static str {
        match self.view.lock_unpoisoned().as_ref().map(|v| v.phase) {
            Some(Phase::Active) if self.cancel.output_open() => "shared-keyed",
            Some(Phase::AwaitingKey) if self.cancel.output_open() => "shared-armed",
            Some(Phase::ReadyOff) => "shared-ready-off",
            Some(_) => "shared-stopping",
            None => "shared-unavailable",
        }
    }
    pub(crate) fn startup_ready(&self) -> bool {
        self.view
            .lock_unpoisoned()
            .as_ref()
            .is_some_and(|v| v.clean && v.installed.is_some() && v.phase == Phase::ReadyOff)
    }
    pub(crate) fn lifecycle_status(&self) -> Option<LifecycleStatus> {
        self.view.lock_unpoisoned().as_ref().map(|view| view.status)
    }
    pub(crate) fn discontinuity(&self, source: TxAudioSource, at: Instant, reason: &'static str) {
        if self.selected_active(source) {
            eprintln!(
                "shared TX ingress discontinuity source={source:?} observed={at:?} reason={reason}"
            );
            self.stop(Fault::SequenceGap);
        }
    }
    /// One-time startup config. No arm, output permit or RF authorization.
    pub(crate) fn bootstrap(
        &self,
        model: RadioModel,
        config: Config,
        source: TxAudioSource,
    ) -> Result<(), Fault> {
        let guard = self.view.try_lock().map_err(|_| Fault::CleanupPending)?;
        let v = guard.as_ref().ok_or(Fault::CleanupPending)?;
        if v.phase != Phase::ReadyOff || v.installed.is_some() {
            return Err(Fault::CleanupPending);
        }
        self.commands
            .try_send(TxCommand::PureSignalOwnerConfig(Box::new(
                Configuration::at_ingress(v.stamp, model, config, source),
            )))
            .map_err(|_| Fault::QueueOverrun)
    }
    fn send_command(&self, command: TxCommand) -> Result<(), Fault> {
        self.commands.try_send(command).map_err(|_| {
            self.stop(Fault::QueueOverrun);
            Fault::QueueOverrun
        })
    }
    pub(crate) fn enqueue_audio(&self, capture: &Capture, frame: TxAudioFrame) -> io::Result<()> {
        if capture.stamp.output.is_none()
            || !matches!(capture.phase, Phase::AwaitingKey | Phase::Active)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no shared TX generation at arrival",
            ));
        }
        if frame.source != capture.source {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrong TX source",
            ));
        }
        self.audio
            .try_send(Audio::at_ingress(
                capture.stamp,
                TxAudioMessage::Frames(frame),
            ))
            .map_err(|_| {
                self.stop(Fault::QueueOverrun);
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "shared TX audio queue unavailable",
                )
            })
    }
    pub(crate) fn satp_packet(
        &self,
        capture: Option<&Capture>,
        samples: [f32; 128],
        received_at: Instant,
    ) -> io::Result<()> {
        let c = capture.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SATP packet arrived without output authority",
            )
        })?;
        self.enqueue_audio(
            c,
            TxAudioFrame {
                enqueued_at: received_at,
                source: TxAudioSource::Satp,
                samples: TxAudioSamples::Satp(samples),
                channels: 1,
                sample_rate_hz: 48000,
            },
        )
    }
    pub(crate) fn feedback(&self, block: Box<PairBlock>) -> Result<(), Fault> {
        match self.feedback.try_send(block) {
            Ok(()) => Ok(()),
            Err(error) => {
                let (mpsc::TrySendError::Full(block) | mpsc::TrySendError::Disconnected(block)) =
                    error;
                // Fault only the generation which produced this block. A
                // retired queue tail must not revoke a newly opened output.
                if let Some(output) = self.capture(Instant::now()).and_then(|c| {
                    (c.stamp.fence == block.meta.fence)
                        .then_some(c.stamp.output)
                        .flatten()
                }) {
                    self.stop_feedback(output, Fault::QueueOverrun);
                }
                self.recycle_feedback_buffer(block);
                Err(Fault::QueueOverrun)
            }
        }
    }
    pub(crate) fn take_feedback_buffer(&self) -> Result<Box<PairBlock>, Fault> {
        self.feedback_pool
            .try_lock()
            .ok()
            .and_then(|mut p| p.pop())
            .ok_or(Fault::QueueOverrun)
    }
    pub(crate) fn recycle_feedback_buffer(&self, mut block: Box<PairBlock>) {
        block.pairs = 0;
        let mut pool = self.feedback_pool.lock_unpoisoned();
        if pool.len() < 4 {
            pool.push(block);
        }
    }
    pub(crate) fn shutdown(&self) -> Result<(), Fault> {
        self.release();
        self.send_command(TxCommand::Shutdown)
    }
    /// Returns false ONLY for RX/client controls that retain their existing
    /// mailbox. Every TX control is consumed or rejected here, never teed.
    pub(crate) fn tci(
        &self,
        client: u64,
        capture: Option<&Capture>,
        command: &TciCommand,
    ) -> Result<bool, Fault> {
        match command {
            TciCommand::SetTxEnabled(false) | TciCommand::ClientDisconnected => {
                self.release();
                return Ok(true);
            }
            TciCommand::SetTxEnabled(true) => {
                let c = capture.ok_or(Fault::CleanupPending)?;
                let credential = self.operator.receive(
                    client,
                    "trx:0,true",
                    c.stamp.release,
                    c.stamp.fence,
                    c.stamp.revision,
                )?;
                self.send_command(TxCommand::PureSignalAuthorizedArm(AuthorizedArm::new(
                    credential,
                )))?;
                return Ok(true);
            }
            TciCommand::MicAudioFrame(frame) => {
                let c = capture.ok_or(Fault::CleanupPending)?;
                // Review route admits decoded PCM only. Opus history must have
                // its own verified age contract before enabling it here.
                if frame.channels == 0
                    || frame.channels > 2
                    || frame.sample_rate_hz != 48000
                    || frame.samples.len() > 2048
                    || !frame.samples.len().is_multiple_of(frame.channels as usize)
                    || frame.samples.iter().any(|s| !s.is_finite())
                {
                    self.stop(Fault::WrongFormat);
                    return Err(Fault::WrongFormat);
                }
                for samples in frame.samples.chunks(1024) {
                    self.enqueue_audio(
                        c,
                        TxAudioFrame {
                            enqueued_at: c.received_at,
                            source: TxAudioSource::Tci,
                            samples: TxAudioSamples::Dynamic(samples.to_vec()),
                            channels: frame.channels,
                            sample_rate_hz: frame.sample_rate_hz,
                        },
                    )
                    .map_err(|_| Fault::QueueOverrun)?;
                }
                return Ok(true);
            }
            _ => {}
        }
        if !is_tx_configuration(command) {
            return Ok(false);
        }
        self.configure_batch(client, capture, std::slice::from_ref(command))?;
        Ok(true)
    }
    pub(crate) fn configure_batch(
        &self,
        client: u64,
        capture: Option<&Capture>,
        commands: &[TciCommand],
    ) -> Result<(), Fault> {
        if commands.is_empty() {
            return Ok(());
        }
        let id = self
            .next_settings
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_add(1))
            .map_err(|_| Fault::InvalidConfig)?
            + 1;
        // Serializes ONLY short producer work: projected settings + enqueue.
        // No native/device work. Out-of-band release never takes this lock.
        let mut tail = self.settings_tail.lock_unpoisoned();
        let proposed = (|| {
            if commands.len() > 64 {
                self.stop(Fault::QueueOverrun);
                return Err(Fault::QueueOverrun);
            }
            let c = capture.ok_or(Fault::CleanupPending)?;
            let permission = self.operator.settings_permission(client)?;
            if c.settings_epoch.exhausted()
                || c.settings_epoch != self.cancel.settings_epoch.current()
            {
                return Err(Fault::GenerationMismatch);
            }
            if commands.iter().any(
                |command| matches!(command, TciCommand::SatpControl{action,..} if action=="satp"),
            ) {
                let m = self.model.try_lock().map_err(|_| Fault::CleanupPending)?;
                if !m.satp.paired(client, Instant::now()) || !m.satp.ready(Instant::now()) {
                    return Err(Fault::InvalidConfig);
                }
            }
            let prior = tail.as_ref().filter(|t| {
                t.epoch == c.settings_epoch
                    && t.original.release == c.stamp.release
                    && t.original.fence.boot == c.stamp.fence.boot
                    && !matches!(
                        *t.receipt.outcome.lock_unpoisoned(),
                        SettingsOutcome::Rejected(_)
                    )
            });
            let (mut model, mut config, mut source, predecessor) = match prior {
                Some(t) => (t.model.clone(), t.config, t.source, Some(t.receipt.clone())),
                None => (c.model.clone(), c.config, c.source, None),
            };
            for command in commands {
                apply_config(command, &mut model, &mut config, &mut source)?;
            }
            config.id = config.id.checked_add(1).ok_or(Fault::InvalidConfig)?;
            let receipt = Arc::new(Receipt {
                id,
                client,
                config: config.id,
                operator: self.operator.clone(),
                outcome: Mutex::new(SettingsOutcome::Pending),
                cancel: self.cancel.clone(),
            });
            let mut envelope = Configuration::at_ingress(c.stamp, model.clone(), config, source);
            envelope.transaction = Some(SettingsTransaction {
                receipt: receipt.clone(),
                predecessor,
                permission,
                epoch: self.cancel.settings_epoch.clone(),
                issued_epoch: c.settings_epoch,
            });
            // Queued means a proposal, never completion. Emit before the owner
            // can see it; even channel-full has an explicit terminal rejection.
            receipt.reply("queued");
            match self
                .commands
                .try_send(TxCommand::PureSignalOwnerConfig(Box::new(envelope)))
            {
                Ok(()) => {
                    *tail = Some(SettingsTail {
                        epoch: c.settings_epoch,
                        original: c.stamp,
                        model,
                        config,
                        source,
                        receipt,
                    });
                    Ok(())
                }
                Err(error) => {
                    receipt.finish(SettingsOutcome::Rejected(Fault::QueueOverrun));
                    drop(error); // Drop fallback cannot overwrite terminal reason
                    self.stop(Fault::QueueOverrun);
                    Err(Fault::QueueOverrun)
                }
            }
        })();
        // Validation failure occurs before an owner envelope/receipt exists.
        if let Err(fault) = proposed {
            if !matches!(fault, Fault::QueueOverrun) || commands.len() > 64 {
                self.operator.settings_reply(
                    client,
                    format!("tx_config:0,{id},rejected,{fault:?},0;"),
                    || self.release(),
                );
            }
        }
        proposed
    }
}
pub(crate) fn is_tx_configuration(c: &TciCommand) -> bool {
    use TciCommand::*;
    matches!(
        c,
        SetMode(_)
            | SetIqCenter(_)
            | SetRxAntenna(_)
            | SetRxAttenuation(_)
            | SetRxAdc(_)
            | SetIqSampleRate(_)
            | SetVfoA(_)
            | SetVfoB(_)
            | SetActiveVfo(_)
            | SetSplitEnabled(_)
            | SetTxDrive(_)
            | SetTxMicGain(_)
            | SetTxFilterBand { .. }
            | SetTxEqEnabled(_)
            | SetTxEqBand { .. }
            | SetTxCfcEnabled(_)
            | SetTxCfcPrecomp(_)
            | SetTxCfcBand { .. }
            | SetTxPhaseRotatorEnabled(_)
            | SetTxPhaseRotatorAuto(_)
            | SetTxPhaseRotatorCorner(_)
            | SetTxNoiseGateEnabled(_)
            | SetTxNoiseGateThreshold(_)
            | SetTxDexpEnabled(_)
            | SetTxDexpThreshold(_)
            | SetTxDexpExpansion(_)
            | SetTxSpeechProcessorEnabled(_)
            | SetTxSpeechProcessorGain(_)
            | SetTxCessbEnabled(_)
            | SetTxFftSize(_)
            | SetTxLowLatency(_)
            | SetPureSignalEnabled(_)
            | SetPureSignalAutoAttenuate(_)
            | SetPureSignalAttenuation(_)
            | ResetPureSignal
            | SetTxTwoToneTest(_)
            | SetTxTwoToneFreq1(_)
            | SetTxTwoToneFreq2(_)
            | SetTxTwoToneLevelDb(_)
            | SetTxTwoToneInvertLsb(_)
            | SetTxTwoToneDelayMs(_)
    ) || matches!(c, SatpControl{action,..} if action == "tci" || action == "satp")
}
fn apply_config(
    c: &TciCommand,
    m: &mut RadioModel,
    ps: &mut Config,
    source: &mut TxAudioSource,
) -> Result<(), Fault> {
    use TciCommand::*;
    match c {
        SetTxMicGain(v)
        | SetTxCfcPrecomp(v)
        | SetTxPhaseRotatorCorner(v)
        | SetTxNoiseGateThreshold(v)
        | SetTxDexpThreshold(v)
        | SetTxDexpExpansion(v)
        | SetTxSpeechProcessorGain(v)
            if !v.is_finite() =>
        {
            return Err(Fault::InvalidConfig)
        }
        SetTxCfcBand { gain_db, .. } if !gain_db.is_finite() => return Err(Fault::InvalidConfig),
        _ => {}
    }
    match *c {
        SetMode(mode) if !matches!(mode, DemodMode::Wfm | DemodMode::Unknown) => {
            m.desired.mode = mode
        }
        SetVfoA(f) => {
            m.desired.vfo_a_hz = f;
            m.sync_vfo_routes();
        }
        SetIqCenter(f) => m.desired.iq_center_hz = f,
        // Feedback geometry is fixed for this candidate. Changed values get
        // an explicit transaction rejection, never an untagged route update.
        SetRxAntenna(v) if v == m.desired.rx_antenna => {}
        SetRxAttenuation(v) if v == m.desired.rx_attenuation_db => {}
        SetRxAdc(v) if v == m.desired.ddc0_adc => {}
        SetIqSampleRate(v) if v == u32::from(m.desired.ddc0_sample_rate_khz) * 1000 => {}
        SetVfoB(f) => {
            m.desired.vfo_b_hz = f;
            m.sync_vfo_routes();
        }
        SetActiveVfo(v) => {
            m.desired.active_vfo = v.min(1);
            m.sync_vfo_routes();
        }
        SetSplitEnabled(v) => {
            m.desired.split_enabled = v;
            m.sync_vfo_routes();
        }
        SetTxDrive(v) if v <= m.desired.tx_drive => m.desired.tx_drive = v.max(1),
        SetTxMicGain(v) => m.desired.tx_mic_gain_db = v.clamp(-20., 20.),
        SetTxFilterBand { low_hz, high_hz } => {
            m.desired.tx_filter_low_hz = low_hz;
            m.desired.tx_filter_high_hz = high_hz;
        }
        SetTxEqEnabled(v) => m.desired.tx_eq_enabled = v,
        SetTxEqBand { band, gain_db } if band < m.desired.tx_eq_bands.len() => {
            m.desired.tx_eq_bands[band] = gain_db.clamp(-20, 20)
        }
        SetTxCfcEnabled(v) => m.desired.cfc_enabled = v,
        SetTxCfcPrecomp(v) => m.desired.cfc_precomp_db = v.clamp(0., 20.),
        SetTxCfcBand { band, gain_db } if (1..=m.desired.cfc_bands.len()).contains(&band) => {
            m.desired.cfc_bands[band - 1] = gain_db.clamp(0., 20.)
        }
        SetTxPhaseRotatorEnabled(v) => m.desired.tx_phase_rotator_enabled = v,
        SetTxPhaseRotatorAuto(v) => m.desired.tx_phase_rotator_auto = v,
        SetTxPhaseRotatorCorner(v) => m.desired.tx_phase_rotator_corner_hz = v.clamp(50., 2000.),
        SetTxNoiseGateEnabled(v) => m.desired.tx_noise_gate_enabled = v,
        SetTxNoiseGateThreshold(v) => m.desired.tx_noise_gate_threshold_db = v.clamp(-80., 0.),
        SetTxDexpEnabled(v) => m.desired.tx_dexp_enabled = v,
        SetTxDexpThreshold(v) => m.desired.tx_dexp_threshold_db = v.clamp(-80., -6.),
        SetTxDexpExpansion(v) => m.desired.tx_dexp_expansion_db = v.clamp(0., 30.),
        SetTxSpeechProcessorEnabled(v) => m.desired.tx_speech_processor_enabled = v,
        SetTxSpeechProcessorGain(v) => m.desired.tx_speech_processor_gain_db = v.clamp(0., 20.),
        SetTxCessbEnabled(v) => m.desired.tx_cessb_enabled = v,
        SetTxFftSize(v) => {
            let v = v.clamp(1024, 262144);
            m.desired.tx_fft_size = 1 << (31 - v.leading_zeros());
        }
        SetTxLowLatency(v) => m.desired.tx_low_latency = v,
        SetPureSignalEnabled(v) => ps.enabled = v,
        ResetPureSignal => {}
        SatpControl { ref action, .. } if action == "tci" || action == "satp" => {
            *source = if action == "satp" {
                TxAudioSource::Satp
            } else {
                TxAudioSource::Tci
            };
            m.satp.source = *source;
        }
        _ => return Err(Fault::InvalidConfig), // unsupported RF/attenuation/tone controls cannot reach legacy
    }
    Ok(())
}
