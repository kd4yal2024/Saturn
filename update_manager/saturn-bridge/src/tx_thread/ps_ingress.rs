//! Explicit, disabled-by-default route from the actual TCI/SATP producers.
//! Network producers enqueue sealed observations; they never touch native DSP.
//! Installation is an all-or-nothing owner selection, not a legacy tee.
#![allow(dead_code)]
use super::{
    owner_mailbox::{Receiver, SyncSender},
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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::{
    io,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// Capture occurrence before notification/formatting. Journald's receive time
// can follow a blocked notifier and must not be mistaken for detection time.
fn queue_fault_report(boundary: &str, detected: SystemTime, notification_started: Instant) {
    let detected_epoch_ns = detected
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos());
    let notification_ns = notification_started.elapsed().as_nanos();
    eprintln!("shared TX queue fault boundary={boundary} fault=QueueOverrun detected_epoch_ns={detected_epoch_ns:?} notification_ns={notification_ns}");
}

// Finite burst reserve, not a replacement for the controller's freshness
// deadline. Direct's 1024-pair blocks give 85.33 ms of queued payload at
// 192 kpair/s; P2 batches eight wire datagrams (952 pairs, 79.33 ms). No samples are
// dropped/restamped to hide an exceeded budget.
pub(crate) const FEEDBACK_QUEUE_BLOCKS: usize = 16;
// Full queue + Direct scratch/accumulator + one consumer-held block + one
// replacement needed BEFORE enqueue. A full queue must reach its own explicit
// overflow path instead of accidentally exhausting the storage pool first.
pub(crate) const FEEDBACK_POOL_BLOCKS: usize = FEEDBACK_QUEUE_BLOCKS + 2 + 1 + 1;

#[derive(Clone, Debug)]
pub(crate) struct Capture {
    pub(crate) stamp: IngressStamp,
    model: RadioModel,
    config: Config,
    source: TxAudioSource,
    phase: Phase,
    pub(crate) received_at: Instant,
    settings_epoch: SettingsEpochValue,
    stop_id: u64,
    ack_eligible: bool,
}
/// Acknowledgment is settings authority, never a fresh TX action. All fields
/// are sealed before queuing and checked again by the sole native owner.
pub(crate) struct FaultAck {
    pub(super) stamp: IngressStamp,
    pub(super) stop_id: u64,
    pub(super) received_at: Instant,
    permission: SettingsPermission,
    epoch: Arc<SettingsEpoch>,
    issued_epoch: SettingsEpochValue,
    operator: Arc<OperatorIngress>,
    cancel: Cancel,
    client: u64,
    id: u64,
    terminal: bool,
}
impl FaultAck {
    pub(super) fn valid(&self) -> bool {
        self.permission.valid()
            && !self.issued_epoch.exhausted()
            && self.epoch.current() == self.issued_epoch
    }
    pub(super) fn finish(&mut self, result: Result<(), Fault>) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        let outcome = match result {
            Ok(()) => "applied,none".to_string(),
            Err(fault) => format!("rejected,{fault:?}"),
        };
        self.operator.settings_reply(
            self.client,
            format!(
                "saturn_tx_fault_ack:0,{},{outcome},{};",
                self.id, self.stop_id
            ),
            || self.cancel.release(),
        );
    }
}
impl Drop for FaultAck {
    fn drop(&mut self) {
        self.finish(Err(Fault::GenerationMismatch));
    }
}
fn ack_eligible(status: LifecycleStatus) -> bool {
    status.phase == Phase::ReadyOff
        && status.cleanup_completed_ns.is_some()
        && status.primary_fault.is_some()
        && status.cleanup_fault.is_none()
        && status.authority_fault.is_none()
        && status.release_handled >= status.required_release
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
    pub(crate) signal_capture: Arc<OnceLock<crate::tx_signal_capture::shared::Producer>>,
    two_tone_limits: Arc<OnceLock<TwoToneLimits>>,
    generated_trials_only: Arc<AtomicBool>,
    two_tone_pending: Arc<AtomicBool>,
    commands: SyncSender<TxCommand>,
    audio: SyncSender<Audio>,
    feedback: SyncSender<Box<PairBlock>>,
    feedback_pool: Arc<Mutex<Vec<Box<PairBlock>>>>,
    operator: Arc<OperatorIngress>,
    cancel: Cancel,
    view: Arc<Mutex<Option<View>>>,
    fault: Arc<Mutex<Option<Fault>>>,
    model: Arc<Mutex<RadioModel>>,
    model_publication: Arc<Mutex<Option<(u128, Phase, TxAudioSource)>>>,
    selected: Arc<AtomicU8>,
    next_settings: Arc<AtomicU64>,
    settings_tail: Arc<Mutex<Option<SettingsTail>>>,
    media: Arc<crate::tci::ps_media::RemoteMedia>,
    owner_status: Arc<Mutex<(u64, String, String)>>, // serial, fields, published frame
    measurement_host: Arc<OnceLock<Arc<super::ps_measurement::Host>>>,
    // Startup's reviewed drive ceiling, not the most recently requested drive.
    // Reducing drive may never ratchet the permitted restore value downward.
    drive_ceiling: Arc<OnceLock<u8>>,
    auto_mode_supported: Arc<AtomicBool>,
    #[cfg(feature = "managed-puresignal")]
    pub(crate) rf_monitor: Arc<OnceLock<crate::rf_monitor::Input>>,
}
#[derive(Clone, Copy)]
pub(crate) struct TwoToneLimits {
    // Both zero select operator-controlled operation at the committed radio
    // frequency/drive. Finite bench profiles retain their existing contract.
    pub(crate) frequency_hz: u32,
    pub(crate) max_ms: u64,
}
pub(super) struct TwoToneTrial {
    pub(super) stimulus: super::feedback_owner::Stimulus,
    pub(super) requested_at: Instant,
    pub(super) duration_ns: u64,
    client: u64,
    id: u64,
    operator: Arc<OperatorIngress>,
    cancel: super::ps_owner_loop::Cancel,
    pending: Arc<AtomicBool>,
    terminal: bool,
}
impl TwoToneTrial {
    fn reply(&self, result: &str) -> bool {
        self.operator.settings_reply(
            self.client,
            format!("saturn_ps_two_tone:0,{},{};", self.id, result),
            || self.cancel.release(),
        )
    }
    pub(super) fn accepted(&self) {
        self.reply("accepted");
    }
    pub(super) fn reject(&mut self, fault: Fault) {
        self.terminal = true;
        self.reply(&format!("rejected,{fault:?}"));
    }
    pub(super) fn complete(&mut self, fault: Option<Fault>) {
        self.terminal = true;
        self.reply(&format!(
            "complete,{}",
            fault.map_or("none".into(), |f| format!("{f:?}"))
        ));
    }
}
impl Drop for TwoToneTrial {
    fn drop(&mut self) {
        if !self.terminal {
            self.reply("rejected,CleanupUnverified");
        }
        self.pending.store(false, Ordering::Release);
    }
}
impl SharedIngress {
    pub(crate) fn enable_auto_mode(&self) -> Result<(), Fault> {
        if self.measurement_only_mode()
            || self
                .view
                .lock_unpoisoned()
                .as_ref()
                .is_some_and(|v| v.installed.is_some())
        {
            return Err(Fault::InvalidConfig);
        }
        self.auto_mode_supported
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Fault::InvalidConfig)
    }
    pub(crate) fn auto_mode_caps(&self) -> Option<&'static str> {
        self.auto_mode_supported
            .load(Ordering::Acquire)
            .then_some("saturn_ps_auto_caps:0,continuous-wdsp-v1;")
    }
    /// Bind the explicit host limit before client admission. Initial drive is
    /// the requested operating level, not the owner's authorization ceiling.
    pub(crate) fn set_drive_ceiling(&self, watts: u8) -> Result<(), Fault> {
        if !(1..=100).contains(&watts)
            || self
                .view
                .lock_unpoisoned()
                .as_ref()
                .is_some_and(|v| v.installed.is_some())
        {
            return Err(Fault::InvalidConfig);
        }
        self.drive_ceiling
            .set(watts)
            .map_err(|_| Fault::InvalidConfig)
    }
    pub(crate) fn fault_ack_message(
        &self,
        client: u64,
        text: &str,
        capture: Option<&Capture>,
    ) -> bool {
        if !text
            .split(';')
            .any(|c| c.trim().starts_with("saturn_tx_fault_ack:"))
        {
            return false;
        }
        if text.split(';').any(|c| c.trim() == "trx:0,false") {
            self.release();
        }
        let mut id = 0;
        let mut stop = 0;
        let result = (|| {
            let permission = self.operator.settings_permission(client)?;
            let parts: Vec<_> = text
                .split(';')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            if parts.len() != 1 {
                return Err(Fault::InvalidConfig);
            }
            let args: Vec<_> = parts[0]
                .strip_prefix("saturn_tx_fault_ack:")
                .unwrap_or("")
                .split(',')
                .collect();
            let ["0", request, stop_id] = args.as_slice() else {
                return Err(Fault::InvalidConfig);
            };
            id = request
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or(Fault::InvalidConfig)?;
            stop = stop_id.parse().map_err(|_| Fault::InvalidConfig)?;
            let c = capture.ok_or(Fault::CleanupPending)?;
            if !c.ack_eligible || c.stop_id != stop {
                return Err(Fault::CleanupPending);
            }
            let request = FaultAck {
                stamp: c.stamp,
                stop_id: stop,
                received_at: c.received_at,
                permission,
                epoch: self.cancel.settings_epoch.clone(),
                issued_epoch: c.settings_epoch,
                operator: self.operator.clone(),
                cancel: self.cancel.clone(),
                client,
                id,
                terminal: false,
            };
            match self
                .commands
                .try_send(TxCommand::PureSignalFaultAck(request))
            {
                Ok(()) => Ok(()),
                Err(error) => {
                    // Explicit rejection, without acknowledging or creating a
                    // second fault merely because the diagnostic mailbox is full.
                    let (std::sync::mpsc::TrySendError::Full(command)
                    | std::sync::mpsc::TrySendError::Disconnected(command)) = error;
                    if let TxCommand::PureSignalFaultAck(mut request) = command {
                        request.finish(Err(Fault::QueueOverrun));
                    }
                    Ok(())
                }
            }
        })();
        if let Err(fault) = result {
            self.operator.settings_reply(
                client,
                format!("saturn_tx_fault_ack:0,{id},rejected,{fault:?},{stop};"),
                || self.release(),
            );
        }
        true
    }
    /// Explicit host-only opt-in. This does not enable PS, RF permission or arm.
    pub(crate) fn enable_two_tone_trials(&self, limits: TwoToneLimits) -> Result<(), Fault> {
        let operating = limits.max_ms == 0 && limits.frequency_hz == 0;
        if self.measurement_only_mode()
            || (!operating
                && (limits.max_ms == 0
                    || limits.max_ms > 45_000
                    || !(1_500_000..=35_000_000).contains(&limits.frequency_hz)))
        {
            return Err(Fault::InvalidConfig);
        }
        self.two_tone_limits
            .set(limits)
            .map_err(|_| Fault::InvalidConfig)
    }
    pub(crate) fn two_tone_caps(&self) -> Option<String> {
        self.two_tone_limits
            .get()
            .filter(|_| !self.measurement_only_mode())
            .map(|l| {
                if l.frequency_hz == 0 {
                    format!(
                        "saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,{};",
                        self.drive_ceiling.get().copied().unwrap_or(1)
                    )
                } else {
                    format!(
                        "saturn_ps_two_tone_caps:0,tagged-two-tone-v1,{},{},5;",
                        l.max_ms, l.frequency_hz
                    )
                }
            })
    }
    pub(crate) fn restrict_to_generated_trials(&self) -> Result<(), Fault> {
        if self.measurement_only_mode() || self.two_tone_limits.get().is_none() {
            return Err(Fault::InvalidConfig);
        }
        self.generated_trials_only.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub(crate) fn generated_trials_only(&self) -> bool {
        self.generated_trials_only.load(Ordering::SeqCst)
    }
    pub(crate) fn arm_policy_message(&self) -> String {
        format!(
            "saturn_tx_arm_policy:0,{};",
            if self.generated_trials_only() {
                "native-two-tone-only"
            } else {
                "standard"
            }
        )
    }
    pub(crate) fn two_tone_message(
        &self,
        client: u64,
        text: &str,
        capture: Option<&Capture>,
    ) -> bool {
        if !text
            .split(';')
            .any(|p| p.trim().starts_with("saturn_ps_two_tone:"))
        {
            return false;
        }
        // Do not swallow an ordinary release hidden inside a malformed batch.
        if text.split(';').any(|p| p.trim() == "trx:0,false") {
            self.release();
        }
        let mut id = 0;
        let result = (|| {
            self.operator.settings_permission(client)?;
            let parts: Vec<_> = text
                .split(';')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();
            if parts.len() != 1 {
                return Err(Fault::InvalidConfig);
            }
            let args: Vec<_> = parts[0]
                .strip_prefix("saturn_ps_two_tone:")
                .unwrap_or("")
                .split(',')
                .collect();
            let (request, duration, stimulus) = match args.as_slice() {
                ["0", request, "start", duration] => (
                    *request,
                    *duration,
                    super::feedback_owner::Stimulus::TwoTone,
                ),
                ["0", request, "start", duration, "wdsp-postgen-two-tone-v1"] => (
                    *request,
                    *duration,
                    super::feedback_owner::Stimulus::NativeTwoTone,
                ),
                _ => return Err(Fault::InvalidConfig),
            };
            id = request.parse::<u64>().map_err(|_| Fault::InvalidConfig)?;
            if id == 0 {
                return Err(Fault::InvalidConfig);
            }
            let ms = duration.parse::<u64>().map_err(|_| Fault::InvalidConfig)?;
            if self.generated_trials_only()
                && stimulus != super::feedback_owner::Stimulus::NativeTwoTone
            {
                return Err(Fault::InvalidConfig);
            }
            let limits = self.two_tone_limits.get().ok_or(Fault::InvalidConfig)?;
            let c = capture.ok_or(Fault::CleanupPending)?;
            let operating = limits.frequency_hz == 0 && limits.max_ms == 0;
            if self.measurement_only_mode()
                || (!operating && (ms == 0 || ms > limits.max_ms))
                || (c.config.automatic && !self.auto_mode_supported.load(Ordering::Acquire))
                || c.model.desired.tx_drive == 0
                || c.model.desired.tx_drive
                    > if operating {
                        self.drive_ceiling.get().copied().unwrap_or(1)
                    } else {
                        5
                    }
                || (!operating && c.model.desired.tx_frequency_hz != limits.frequency_hz)
                || !matches!(c.model.desired.mode, DemodMode::Usb | DemodMode::Lsb)
                || c.source != TxAudioSource::Tci
            {
                return Err(Fault::InvalidConfig);
            }
            let duration_ns = ms.checked_mul(1_000_000).ok_or(Fault::InvalidConfig)?;
            let credential = self.operator.receive(
                client,
                "trx:0,true",
                c.stamp.release,
                c.stamp.fence,
                c.stamp.revision,
            )?;
            if self
                .two_tone_pending
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(Fault::CleanupPending);
            }
            let mut trial = TwoToneTrial {
                stimulus,
                requested_at: c.received_at,
                duration_ns,
                client,
                id,
                operator: self.operator.clone(),
                cancel: self.cancel.clone(),
                pending: self.two_tone_pending.clone(),
                terminal: false,
            };
            if !trial.reply("queued") {
                trial.reject(Fault::TxAuthority);
                return Ok(());
            }
            match self.commands.try_send(TxCommand::PureSignalAuthorizedArm(
                AuthorizedArm::two_tone(credential, trial),
            )) {
                Ok(()) => {}
                Err(error) => {
                    let command = match error {
                        mpsc::TrySendError::Full(c) | mpsc::TrySendError::Disconnected(c) => c,
                    };
                    if let TxCommand::PureSignalAuthorizedArm(mut arm) = command {
                        if let Some(t) = &mut arm.trial {
                            t.reject(Fault::QueueOverrun);
                        }
                    }
                    self.stop(Fault::QueueOverrun);
                }
            }
            Ok(())
        })();
        if let Err(f) = result {
            // Viewer backpressure must not cancel the authorized operator.
            self.operator.settings_reply(
                client,
                format!("saturn_ps_two_tone:0,{id},rejected,{f:?};"),
                || {},
            );
        }
        true
    }
}
pub(crate) struct Receivers {
    pub(crate) commands: Receiver<TxCommand>,
    pub(crate) audio: Receiver<Audio>,
    pub(crate) feedback: Receiver<Box<PairBlock>>,
}

#[cfg(test)]
mod measurement_tests {
    use super::*;
    #[cfg(not(saturn_bridge_stub_native))]
    use crate::{tci::ps_ingress_fixture::SocketFixture, tx_owner_stop::OutputFence};
    use saturn_bridge::puresignal::{tx_lifecycle::RunMode, OverloadSource};
    use std::time::Duration;

    fn ready(ingress: &SharedIngress) {
        let mut model = crate::tx_owner_stop::tests::model();
        model.desired.tx_enabled = false;
        model.desired.tx_drive = 5;
        model.desired.tx_frequency_hz = 7_200_000;
        model.desired.pure_signal_enabled = false;
        model.desired.pure_signal_attenuation_db = 31;
        let config = Config {
            id: 1,
            route: 1,
            physical_adc: OverloadSource::Adc1,
            enabled: false,
            automatic: false,
            hardware_peak: 1.,
            requested_delay_seconds: 0.,
            loop_delay_seconds: 0.,
            settling_ns: 0,
            stale_after_ns: 150_000_000,
        };
        let stamp = IngressStamp {
            fence: Default::default(),
            revision: 0,
            release: 0,
            output: None,
        };
        ingress.publish(
            stamp,
            LifecycleStatus {
                mode: RunMode::Correction,
                measurement_deadline_ns: None,
                phase: Phase::ReadyOff,
                primary_fault: None,
                cleanup_fault: None,
                authority_fault: None,
                stop_id: 1,
                release_handled: 0,
                required_release: 0,
                notified_ns: None,
                authority_completed_ns: Some(1),
                cleanup_started_ns: Some(1),
                cleanup_completed_ns: Some(2),
                cleanup_steps: 1,
                stop_evidence: None,
                correction_applied: Some(false),
                last_known_applied_before_stop: false,
            },
            Some((model, config)),
            TxAudioSource::Tci,
        );
    }
    fn collector() -> (SharedIngress, Receivers) {
        let (ingress, rx) = crate::tx_thread::feedback_owner::tests::collector();
        let directory = super::super::ps_measurement::tests::directory();
        ingress
            .enable_measurement_only(
                super::super::ps_measurement::Host::open(&directory, 7_200_000).unwrap(),
            )
            .unwrap();
        ready(&ingress);
        (ingress, rx)
    }
    #[test]
    fn normal_receive_antenna_and_attenuation_are_validated_configuration() {
        let (ingress, _) = collector();
        let capture = ingress.capture(Instant::now()).unwrap();
        let mut model = capture.model;
        let mut config = capture.config;
        let mut source = capture.source;
        for antenna in 1..=3 {
            apply_config(
                &TciCommand::SetRxAntenna(antenna),
                &mut model,
                &mut config,
                &mut source,
            )
            .unwrap();
            assert_eq!(model.desired.rx_antenna, antenna);
        }
        for attenuation in [0, 10, 20, 30, 31] {
            apply_config(
                &TciCommand::SetRxAttenuation(attenuation),
                &mut model,
                &mut config,
                &mut source,
            )
            .unwrap();
            assert_eq!(model.desired.rx_attenuation_db, attenuation);
        }
        for command in [
            TciCommand::SetRxAntenna(0),
            TciCommand::SetRxAntenna(4),
            TciCommand::SetRxAttenuation(32),
            TciCommand::SetRxAdc(1),
        ] {
            assert_eq!(
                apply_config(&command, &mut model, &mut config, &mut source),
                Err(Fault::InvalidConfig)
            );
        }
        assert_eq!(model.desired.rx_antenna, 3);
        assert_eq!(model.desired.rx_attenuation_db, 31);
        assert!(!model.desired.tx_enabled && !config.enabled && !config.automatic);
    }
    #[test]
    fn operator_two_tone_uses_current_frequency_power_and_manual_stop() {
        let (i, rx) = crate::tx_thread::feedback_owner::tests::collector();
        i.set_drive_ceiling(100).unwrap();
        ready(&i);
        i.enable_two_tone_trials(TwoToneLimits {
            frequency_hz: 0,
            max_ms: 0,
        })
        .unwrap();
        assert_eq!(
            i.two_tone_caps().as_deref(),
            Some("saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,100;")
        );
        for (n, hz, watts, duration) in [(1, 3_897_000, 60, 0), (2, 27_385_000, 100, 90_000)] {
            let mut capture = i.capture(Instant::now()).unwrap();
            capture.model.desired.tx_frequency_hz = hz;
            capture.model.desired.tx_drive = watts;
            i.two_tone_message(
                7,
                &format!("saturn_ps_two_tone:0,{n},start,{duration};"),
                Some(&capture),
            );
            let TxCommand::PureSignalAuthorizedArm(mut arm) = rx.commands.try_recv().unwrap()
            else {
                panic!("missing arm");
            };
            assert_eq!(
                arm.trial.as_ref().unwrap().duration_ns,
                duration * 1_000_000
            );
            arm.trial.as_mut().unwrap().reject(Fault::Stale);
        }
        let capture = i.capture(Instant::now()).unwrap();
        i.two_tone_message(
            7,
            "saturn_ps_two_tone:0,3,start,18446744073709551615;",
            Some(&capture),
        );
        assert!(rx.commands.try_recv().is_err());
        assert!(
            !i.two_tone_pending.load(Ordering::Acquire),
            "overflow cannot strand pending output"
        );
        i.two_tone_message(8, "saturn_ps_two_tone:0,4,start,0;", Some(&capture));
        assert!(
            rx.commands.try_recv().is_err(),
            "viewer cannot authorize output"
        );
    }

    #[test]
    fn generated_trial_is_opt_in_and_cannot_weaken_measurement_only() {
        let (i, rx) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&i);
        assert!(i.two_tone_caps().is_none());
        let capture = i.capture(Instant::now()).unwrap();
        assert!(i.two_tone_message(7, "saturn_ps_two_tone:0,1,start,100;", Some(&capture)));
        assert!(rx.commands.try_recv().is_err());
        i.enable_two_tone_trials(TwoToneLimits {
            frequency_hz: 7_200_000,
            max_ms: 45000,
        })
        .unwrap();
        assert_eq!(
            i.two_tone_caps().as_deref(),
            Some("saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;")
        );
        let mut valid = capture;
        valid.config.enabled = true;
        for case in [
            "viewer",
            "auto",
            "drive",
            "frequency",
            "source",
            "mode",
            "duration",
            "batch",
        ] {
            let mut c = valid.clone();
            let mut client = 7;
            let mut message = "saturn_ps_two_tone:0,2,start,100;";
            match case {
                "viewer" => client = 8,
                "auto" => c.config.automatic = true,
                "drive" => c.model.desired.tx_drive = 6,
                "frequency" => c.model.desired.tx_frequency_hz = 14_200_000,
                "source" => c.source = TxAudioSource::Satp,
                "mode" => c.model.desired.mode = DemodMode::Am,
                "duration" => message = "saturn_ps_two_tone:0,2,start,45001;",
                "batch" => message = "saturn_ps_two_tone:0,2,start,100;trx:0,false;",
                _ => unreachable!(),
            }
            i.two_tone_message(client, message, Some(&c));
            assert!(rx.commands.try_recv().is_err(), "{case}");
        }
        i.two_tone_message(7, "saturn_ps_two_tone:0,3,start,100;", Some(&valid));
        let TxCommand::PureSignalAuthorizedArm(mut arm) = rx.commands.try_recv().unwrap() else {
            panic!("missing sealed arm")
        };
        assert_eq!(arm.trial.as_ref().unwrap().duration_ns, 100_000_000);
        i.two_tone_message(7, "saturn_ps_two_tone:0,4,start,100;", Some(&valid));
        assert!(rx.commands.try_recv().is_err());
        arm.trial.as_mut().unwrap().reject(Fault::Stale);
        drop(arm);
        assert!(!i.two_tone_pending.load(Ordering::Acquire));
        valid.config.enabled = false;
        i.two_tone_message(
            7,
            "saturn_ps_two_tone:0,5,start,100,wdsp-postgen-two-tone-v1;",
            Some(&valid),
        );
        let TxCommand::PureSignalAuthorizedArm(mut baseline) = rx.commands.try_recv().unwrap()
        else {
            panic!("PS-off baseline must use the same sealed generated action")
        };
        assert!(!valid.config.enabled && !valid.config.automatic);
        assert_eq!(
            baseline.trial.as_ref().unwrap().stimulus,
            super::super::feedback_owner::Stimulus::NativeTwoTone
        );
        baseline.trial.as_mut().unwrap().reject(Fault::Stale);
        let (measurement, _) = collector();
        assert_eq!(
            measurement.enable_two_tone_trials(TwoToneLimits {
                frequency_hz: 7_200_000,
                max_ms: 45000
            }),
            Err(Fault::InvalidConfig)
        );
        assert!(measurement.two_tone_caps().is_none());
    }
    #[test]
    fn generated_only_requires_versioned_native_action_and_refuses_ordinary_ptt() {
        let (ingress, rx) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&ingress);
        assert_eq!(
            ingress.restrict_to_generated_trials(),
            Err(Fault::InvalidConfig)
        );
        ingress
            .enable_two_tone_trials(TwoToneLimits {
                frequency_hz: 7_200_000,
                max_ms: 45000,
            })
            .unwrap();
        ingress.restrict_to_generated_trials().unwrap();
        assert_eq!(
            ingress.arm_policy_message(),
            "saturn_tx_arm_policy:0,native-two-tone-only;"
        );
        let mut capture = ingress.capture(Instant::now()).unwrap();
        capture.config.enabled = true;
        assert_eq!(
            ingress.tci(7, Some(&capture), &TciCommand::SetTxEnabled(true)),
            Err(Fault::InvalidConfig)
        );
        ingress.two_tone_message(7, "saturn_ps_two_tone:0,1,start,500;", Some(&capture));
        assert!(rx.commands.try_recv().is_err());
        ingress.two_tone_message(
            7,
            "saturn_ps_two_tone:0,2,start,500,wdsp-postgen-two-tone-v1;",
            Some(&capture),
        );
        let TxCommand::PureSignalAuthorizedArm(arm) = rx.commands.try_recv().unwrap() else {
            panic!("sealed native trial expected")
        };
        assert_eq!(
            arm.trial.as_ref().unwrap().stimulus,
            super::super::feedback_owner::Stimulus::NativeTwoTone
        );
        assert!(rx.commands.try_recv().is_err());
    }
    #[test]
    fn feedback_pool_contention_is_not_exhaustion_and_never_blocks_release() {
        let (ingress, _rx) = collector();
        let held = ingress.feedback_pool.lock_unpoisoned();
        assert_eq!(held.len(), FEEDBACK_POOL_BLOCKS);
        let worker_ingress = ingress.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            entered_tx.send(()).unwrap();
            result_tx
                .send(worker_ingress.take_feedback_buffer())
                .unwrap();
        });
        entered_rx.recv().unwrap();
        let premature = result_rx.recv_timeout(Duration::from_millis(50));
        let before = ingress.cancel.settings_epoch.current();
        ingress.release(); // no pool lock, native call, DMA wait or queued command
        assert_ne!(before, ingress.cancel.settings_epoch.current());
        drop(held);
        worker.join().unwrap();
        assert!(
            matches!(premature, Err(mpsc::RecvTimeoutError::Timeout)),
            "a busy populated pool must not be reported as exhausted"
        );
        let block = result_rx.recv().unwrap().unwrap();
        ingress.recycle_feedback_buffer(block);
        assert_eq!(
            ingress.feedback_pool.lock_unpoisoned().len(),
            FEEDBACK_POOL_BLOCKS
        );
        assert_eq!(ingress.take_fault(), None);
    }
    #[test]
    fn actual_feedback_pool_exhaustion_still_rejects_without_allocating() {
        let (ingress, _rx) = collector();
        let blocks: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert!(matches!(
            ingress.take_feedback_buffer(),
            Err(Fault::QueueOverrun)
        ));
        for block in blocks {
            ingress.recycle_feedback_buffer(block);
        }
        assert_eq!(
            ingress.feedback_pool.lock_unpoisoned().len(),
            FEEDBACK_POOL_BLOCKS
        );
    }
    #[test]
    fn manual_feedback_attenuation_is_ordered_and_rejects_invalid_values() {
        let (ingress, rx) = collector();
        let capture = ingress.capture(Instant::now()).unwrap();
        for attenuation in [0, 13, 31] {
            ingress
                .configure_batch(
                    7,
                    Some(&capture),
                    &[TciCommand::SetPureSignalAttenuation(attenuation)],
                )
                .unwrap();
            let TxCommand::PureSignalOwnerConfig(c) = rx.commands.try_recv().unwrap() else {
                panic!("missing ordered feedback setting")
            };
            assert_eq!(c.view().0.desired.pure_signal_attenuation_db, attenuation);
            assert!(!c.view().0.desired.tx_enabled);
            assert!(!c.view().1.enabled);
        }
        assert_eq!(
            ingress.configure_batch(
                7,
                Some(&capture),
                &[TciCommand::SetPureSignalAttenuation(32)]
            ),
            Err(Fault::InvalidConfig)
        );
        assert!(rx.commands.try_recv().is_err());
        ingress.release();
        assert_eq!(
            ingress.configure_batch(
                7,
                Some(&capture),
                &[TciCommand::SetPureSignalAttenuation(0)]
            ),
            Err(Fault::GenerationMismatch)
        );
    }
    #[test]
    fn explicit_host_drive_ceiling_survives_lower_startup_and_ordered_settings() {
        let (ingress, rx) = crate::tx_thread::feedback_owner::tests::collector();
        assert_eq!(ingress.set_drive_ceiling(0), Err(Fault::InvalidConfig));
        assert_eq!(ingress.set_drive_ceiling(101), Err(Fault::InvalidConfig));
        ingress.set_drive_ceiling(100).unwrap();
        assert_eq!(ingress.set_drive_ceiling(100), Err(Fault::InvalidConfig));
        ready(&ingress); // starts at 5 W, with a separately supplied 100 W limit
        assert_eq!(
            ingress.drive_limit_message(),
            "saturn_tx_drive_limit:0,100;"
        );
        assert_eq!(ingress.set_drive_ceiling(5), Err(Fault::InvalidConfig));
        let capture = ingress.capture(Instant::now()).unwrap();
        for watts in [3, 100, 5] {
            ingress
                .configure_batch(7, Some(&capture), &[TciCommand::SetTxDrive(watts)])
                .unwrap();
            let TxCommand::PureSignalOwnerConfig(c) = rx.commands.try_recv().unwrap() else {
                panic!("missing ordered power setting")
            };
            assert_eq!(c.view().0.desired.tx_drive, watts);
        }
        assert_eq!(
            ingress.configure_batch(7, Some(&capture), &[TciCommand::SetTxDrive(101)]),
            Err(Fault::InvalidConfig)
        );
        assert_eq!(
            ingress.drive_limit_message(),
            "saturn_tx_drive_limit:0,100;"
        );
        assert!(rx.commands.try_recv().is_err());
    }
    #[test]
    fn separate_drive_reduction_and_restore_keep_original_reviewed_ceiling() {
        let (ingress, rx) = collector();
        let capture = ingress.capture(Instant::now()).unwrap();
        ingress
            .configure_batch(7, Some(&capture), &[TciCommand::SetTxDrive(3)])
            .unwrap();
        ingress
            .configure_batch(7, Some(&capture), &[TciCommand::SetTxDrive(5)])
            .unwrap();
        assert_eq!(
            ingress.configure_batch(7, Some(&capture), &[TciCommand::SetTxDrive(6)]),
            Err(Fault::InvalidConfig)
        );
        for expected in [3, 5] {
            let TxCommand::PureSignalOwnerConfig(c) = rx.commands.try_recv().unwrap() else {
                panic!("missing settings")
            };
            assert_eq!(c.view().0.desired.tx_drive, expected);
        }
        assert!(rx.commands.try_recv().is_err());
    }
    #[test]
    fn fault_ack_ingress_rejects_viewers_incomplete_cleanup_and_mixed_release() {
        for case in [
            "viewer",
            "cleanup",
            "authority",
            "blocked",
            "mixed",
            "empty",
        ] {
            let (ingress, rx) = collector();
            {
                let mut v = ingress.view.lock_unpoisoned();
                let status = &mut v.as_mut().unwrap().status;
                status.primary_fault = Some(Fault::QueueOverrun);
                status.stop_id = 8;
                if case == "cleanup" {
                    status.cleanup_fault = Some(Fault::CleanupTimeout);
                }
                if case == "authority" {
                    status.authority_fault = Some(Fault::TxAuthority);
                }
                if case == "blocked" {
                    status.phase = Phase::Blocked;
                }
            }
            let capture = ingress.capture(Instant::now()).unwrap();
            let before = ingress.cancel.settings_epoch.current();
            let message = if case == "mixed" {
                "saturn_tx_fault_ack:0,1,8;trx:0,false;"
            } else if case == "empty" {
                "saturn_tx_fault_ack:"
            } else {
                "saturn_tx_fault_ack:0,1,8;"
            };
            assert!(ingress.fault_ack_message(
                if case == "viewer" { 8 } else { 7 },
                message,
                Some(&capture)
            ));
            assert!(rx.commands.try_recv().is_err(), "{case}");
            if case == "mixed" {
                assert_ne!(before, ingress.cancel.settings_epoch.current());
            }
            assert_eq!(
                ingress
                    .view
                    .lock_unpoisoned()
                    .as_ref()
                    .unwrap()
                    .status
                    .primary_fault,
                Some(Fault::QueueOverrun)
            );
        }
    }
    #[test]
    fn ps_capabilities_distinguish_measurement_only_and_single_fit() {
        let (measurement, _) = collector();
        assert_eq!(
            measurement.ps_control_caps(),
            "saturn_ps_control_caps:0,measurement-only;"
        );
        assert!(measurement
            .owner_status_message()
            .unwrap()
            .contains("saturn_ps_mode:0,1,off;"));
        let (single, _) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&single);
        assert_eq!(single.ps_control_caps(), "saturn_ps_control_caps:0,single;");
        assert!(single
            .owner_status_message()
            .unwrap()
            .contains("saturn_ps_mode:0,1,off;"));
    }
    #[test]
    fn idle_fault_latch_cannot_certify_host_receive_readiness() {
        for field in ["primary", "authority", "cleanup"] {
            let (ingress, _) = crate::tx_thread::feedback_owner::tests::collector();
            ready(&ingress);
            assert_eq!(ingress.host_readiness(), ("shared-ready-off", true));
            {
                let mut view = ingress.view.lock_unpoisoned();
                let status = &mut view.as_mut().unwrap().status;
                match field {
                    "primary" => status.primary_fault = Some(Fault::QueueOverrun),
                    "authority" => status.authority_fault = Some(Fault::TxAuthority),
                    _ => status.cleanup_fault = Some(Fault::CleanupTimeout),
                }
            }
            assert_eq!(ingress.host_readiness(), ("shared-ready-off", false));
        }
    }
    #[test]
    fn single_fit_settings_are_queued_without_arm_and_not_published_before_commit() {
        let (ingress, rx) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&ingress);
        let capture = ingress.capture(Instant::now()).unwrap();
        ingress
            .configure_batch(7, Some(&capture), &[TciCommand::SetPureSignalEnabled(true)])
            .unwrap();
        let TxCommand::PureSignalOwnerConfig(_) = rx.commands.try_recv().unwrap() else {
            panic!("calibration configuration must not be an authorized arm")
        };
        let tail = ingress.settings_tail.lock_unpoisoned();
        assert!(tail.as_ref().unwrap().config.enabled);
        assert!(!tail.as_ref().unwrap().config.automatic);
        assert!(rx.commands.try_recv().is_err());
        assert!(ingress
            .owner_status_message()
            .unwrap()
            .contains("saturn_ps_mode:0,1,off;"));
    }
    #[test]
    fn live_and_stop_publication_do_not_wait_for_a_radio_model_reader() {
        let (ingress, _) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&ingress);
        let (stamp, mut status, installed) = {
            let view = ingress.view.lock_unpoisoned();
            let v = view.as_ref().unwrap();
            (v.stamp, v.status, v.installed.clone())
        };
        for phase in [Phase::Active, Phase::ReadyOff] {
            status.phase = phase;
            status.stop_id += 1;
            let guard = ingress.model.lock_unpoisoned();
            let worker = ingress.clone();
            let values = installed.clone();
            let (done, rx) = std::sync::mpsc::channel();
            let join = std::thread::spawn(move || {
                worker.publish(stamp, status, values, TxAudioSource::Tci);
                done.send(()).unwrap();
            });
            let published = rx.recv_timeout(Duration::from_secs(1)).is_ok();
            drop(guard);
            join.join().unwrap();
            assert!(
                published,
                "phase publication waited for a held model mutex: {phase:?}"
            );
            assert_eq!(
                ingress.view.lock_unpoisoned().as_ref().unwrap().phase,
                phase
            );
            // A deferred phase projection is retried even though lifecycle
            // publication already advanced. No field or sample is restamped.
            ingress.publish(stamp, status, installed.clone(), TxAudioSource::Tci);
            assert_eq!(
                ingress.model.lock_unpoisoned().desired.tx_enabled,
                phase == Phase::Active
            );
        }
    }
    #[test]
    fn auto_requires_startup_opt_in_and_configuring_it_never_arms() {
        let (strict, rx) = crate::tx_thread::feedback_owner::tests::collector();
        ready(&strict);
        let c = strict.capture(Instant::now()).unwrap();
        assert!(strict.auto_mode_caps().is_none());
        assert_eq!(
            strict.configure_batch(7, Some(&c), &[TciCommand::SetPureSignalMode(2)]),
            Err(Fault::InvalidConfig)
        );
        assert!(rx.commands.try_recv().is_err());
        assert_eq!(strict.enable_auto_mode(), Err(Fault::InvalidConfig));

        let (ingress, rx) = crate::tx_thread::feedback_owner::tests::collector();
        ingress.enable_auto_mode().unwrap();
        assert_eq!(ingress.enable_auto_mode(), Err(Fault::InvalidConfig));
        ready(&ingress);
        assert_eq!(
            ingress.auto_mode_caps(),
            Some("saturn_ps_auto_caps:0,continuous-wdsp-v1;")
        );
        let c = ingress.capture(Instant::now()).unwrap();
        ingress
            .configure_batch(7, Some(&c), &[TciCommand::SetPureSignalMode(2)])
            .unwrap();
        assert!(matches!(
            rx.commands.try_recv().unwrap(),
            TxCommand::PureSignalOwnerConfig(_)
        ));
        let tail = ingress.settings_tail.lock_unpoisoned();
        let config = tail.as_ref().unwrap().config;
        assert!(config.enabled && config.automatic);
        assert!(rx.commands.try_recv().is_err());
        assert!(ingress
            .owner_status_message()
            .unwrap()
            .contains("saturn_ps_mode:0,1,off;"));
    }
    #[test]
    fn measurement_only_cannot_advertise_auto_adaptation() {
        let (ingress, _) = collector();
        assert_eq!(ingress.enable_auto_mode(), Err(Fault::InvalidConfig));
        assert!(ingress.auto_mode_caps().is_none());
    }
    #[test]
    fn diagnostic_wire_limits_authority_and_no_ordinary_ptt_or_ps_enable() {
        let (ingress, rx) = collector();
        let capture = ingress.capture(Instant::now()).unwrap();
        for value in ["0", "1001", "18446744073709551615", "-1", "abc"] {
            assert!(ingress.measurement_message(
                7,
                &format!("saturn_ps_measurement:0,start,{value};"),
                Some(&capture)
            ));
            assert!(rx.commands.try_recv().is_err());
        }
        assert!(ingress.measurement_message(
            8,
            "saturn_ps_measurement:0,start,500;",
            Some(&capture)
        ));
        assert!(rx.commands.try_recv().is_err());
        assert_eq!(
            ingress.tci(7, Some(&capture), &TciCommand::SetTxEnabled(true)),
            Err(Fault::InvalidConfig)
        );
        assert_eq!(
            ingress.configure_batch(7, Some(&capture), &[TciCommand::SetPureSignalEnabled(true)]),
            Err(Fault::InvalidConfig)
        );
        assert!(rx.commands.try_recv().is_err());
        ingress.measurement_message(7, "saturn_ps_measurement:0,start,500;", Some(&capture));
        let TxCommand::MeasurementOnly(request) = rx.commands.try_recv().unwrap() else {
            panic!("explicit diagnostic envelope missing")
        };
        assert_eq!(request.duration_ns, 500_000_000);
        assert_eq!(request.requested_at, capture.received_at);
        assert!(request.credential.valid());
        assert_eq!(request.recorder.context().feedback_attenuation_db, Some(31));
        assert!(request.diagnostic.is_some());
        drop(request);
        assert!(ingress
            .measurement_host
            .get()
            .unwrap()
            .status()
            .contains("Rejected"));
    }
    #[test]
    fn timing_wire_is_explicit_ssb_only_and_preserves_stimulus_in_sealed_request() {
        use super::super::feedback_owner::Stimulus;
        let (ingress, rx) = collector();
        let mut capture = ingress.capture(Instant::now()).unwrap();
        for command in [
            "saturn_ps_measurement:0,start,500,unknown;",
            "saturn_ps_measurement:0,start,500,saturn-timing-multisine-v1,extra;",
        ] {
            ingress.measurement_message(7, command, Some(&capture));
            assert!(rx.commands.try_recv().is_err());
        }
        let command = "saturn_ps_measurement:0,start,500,saturn-timing-multisine-v1;";
        for mode in [DemodMode::Am, DemodMode::Sam, DemodMode::Fm] {
            capture.model.desired.mode = mode;
            ingress.measurement_message(7, command, Some(&capture));
            assert!(rx.commands.try_recv().is_err());
        }
        capture.model.desired.mode = DemodMode::Lsb;
        ingress.measurement_message(8, command, Some(&capture));
        assert!(rx.commands.try_recv().is_err());
        ingress.measurement_message(7, command, Some(&capture));
        let TxCommand::MeasurementOnly(request) = rx.commands.try_recv().unwrap() else {
            panic!("missing timing action")
        };
        assert_eq!(request.stimulus, Stimulus::Timing);
        assert_eq!(request.duration_ns, 500_000_000);
        assert_eq!(request.requested_at, capture.received_at);
        assert!(!capture.config.enabled && !capture.config.automatic);
        assert!(request.credential.valid());
        drop(request);
        assert!(ingress
            .measurement_host
            .get()
            .unwrap()
            .status()
            .contains("Rejected"));
    }
    #[test]
    fn native_two_tone_wire_requires_ssb_operator_and_seals_original_action() {
        use super::super::feedback_owner::Stimulus;
        let (ingress, rx) = collector();
        let mut capture = ingress.capture(Instant::now()).unwrap();
        let message = "saturn_ps_measurement:0,start,500,wdsp-postgen-two-tone-v1;";
        for mode in [DemodMode::Am, DemodMode::Sam, DemodMode::Fm] {
            capture.model.desired.mode = mode;
            ingress.measurement_message(7, message, Some(&capture));
            assert!(rx.commands.try_recv().is_err());
        }
        capture.model.desired.mode = DemodMode::Usb;
        ingress.measurement_message(8, message, Some(&capture));
        assert!(rx.commands.try_recv().is_err());
        ingress.measurement_message(7, message, Some(&capture));
        let TxCommand::MeasurementOnly(request) = rx.commands.try_recv().unwrap() else {
            panic!("missing sealed native source")
        };
        assert_eq!(request.stimulus, Stimulus::NativeTwoTone);
        assert_eq!(request.requested_at, capture.received_at);
        assert!(request.credential.valid());
        assert!(
            super::super::ps_measurement::stimulus_json(request.stimulus)
                .contains("not tone audio")
        );
    }
    #[test]
    fn full_diagnostic_queue_preserves_rejection_and_cancels_output() {
        let (ingress, rx) = collector();
        for _ in 0..16 {
            ingress
                .commands
                .try_send(TxCommand::ModelChanged)
                .ok()
                .unwrap();
        }
        let before = ingress.cancel.settings_epoch.current();
        ingress.measurement_message(
            7,
            "saturn_ps_measurement:0,start,500;",
            ingress.capture(Instant::now()).as_ref(),
        );
        assert_ne!(ingress.cancel.settings_epoch.current(), before);
        assert!(ingress
            .measurement_host
            .get()
            .unwrap()
            .status()
            .contains("Rejected"));
        assert_eq!(rx.commands.try_iter().count(), 16);
        assert_eq!(ingress.take_fault(), Some(Fault::QueueOverrun));
    }
    #[test]
    fn mixed_diagnostic_batch_never_swallows_release_or_queues_arm() {
        for release in [
            "trx:0,false",
            "trx:false",
            "trx:0,off,tci",
            "trx:0,no",
            "trx:0,0",
        ] {
            let (ingress, rx) = collector();
            let before = ingress.cancel.settings_epoch.current();
            ingress.measurement_message(
                7,
                &format!("saturn_ps_measurement:0,start,500;{release};trx:0,true;"),
                ingress.capture(Instant::now()).as_ref(),
            );
            assert_ne!(ingress.cancel.settings_epoch.current(), before, "{release}");
            assert!(rx.commands.try_recv().is_err());
        }
    }
    #[test]
    fn viewer_reply_overflow_revokes_only_viewer_not_operator_output() {
        let (ingress, rx) = collector();
        let (clients, _) = ingress.operator.fixture_registry();
        let viewer = clients.lock_unpoisoned().get(&8).unwrap().outbound.clone();
        for _ in 0..64 {
            assert!(viewer.enqueue_transaction_reply("fixture".into()));
        }
        let before = ingress.cancel.settings_epoch.current();
        ingress.measurement_message(8, "saturn_ps_measurement:0,start,500;", None);
        assert_eq!(ingress.cancel.settings_epoch.current(), before);
        assert!(ingress.operator.settings_permission(7).is_ok());
        assert!(ingress.operator.settings_permission(8).is_err());
        assert!(rx.commands.try_recv().is_err());
        assert_eq!(ingress.take_fault(), None);
    }
    #[cfg(not(saturn_bridge_stub_native))]
    #[test]
    fn generated_trial_actual_websocket_dispatch_advertises_and_queues_sealed_action() {
        let socket = SocketFixture::new();
        let model = Arc::new(Mutex::new(crate::tx_owner_stop::tests::model()));
        let fence = Arc::new(OutputFence::new_open_for_offline_trial());
        fence.close();
        let (ingress, rx) =
            SharedIngress::channels(socket.issuer(), Cancel::fixture(fence), model.clone());
        ready(&ingress);
        ingress.view.lock_unpoisoned().as_mut().unwrap().installed = None;
        ingress.enable_tagged_pcm_review().unwrap();
        ready(&ingress);
        ingress
            .view
            .lock_unpoisoned()
            .as_mut()
            .unwrap()
            .installed
            .as_mut()
            .unwrap()
            .1
            .enabled = true;
        ingress
            .enable_two_tone_trials(TwoToneLimits {
                frequency_hz: 7_200_000,
                max_ms: 45000,
            })
            .unwrap();
        let mut client = socket.connect(ingress, model);
        client.text("saturn_ps_two_tone:0,41,start,300;");
        let deadline = Instant::now() + Duration::from_secs(2);
        let command = loop {
            if let Ok(c) = rx.commands.try_recv() {
                break c;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        let TxCommand::PureSignalAuthorizedArm(mut arm) = command else {
            panic!("wrong production dispatch")
        };
        assert!(arm.credential.valid());
        assert_eq!(arm.trial.as_ref().unwrap().duration_ns, 300_000_000);
        while !client
            .diagnostics
            .iter()
            .any(|s| s == "saturn_ps_two_tone:0,41,queued;")
            || !client
                .diagnostics
                .iter()
                .any(|s| s == "saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;")
        {
            client.drain();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        while let Ok(command) = socket.legacy.try_recv() {
            assert!(
                !matches!(
                    command,
                    TciCommand::SetTxEnabled(true) | TciCommand::MicAudioFrame(_)
                ),
                "no parallel legacy TX path"
            );
        }
        arm.trial.as_mut().unwrap().reject(Fault::CleanupPending);
    }
    #[cfg(not(saturn_bridge_stub_native))]
    #[test]
    fn actual_websocket_diagnostic_capability_and_queued_request_use_existing_operator_boundary() {
        let socket = SocketFixture::new();
        let model = Arc::new(Mutex::new(crate::tx_owner_stop::tests::model()));
        let fence = Arc::new(OutputFence::new_open_for_offline_trial());
        fence.close();
        let (ingress, rx) =
            SharedIngress::channels(socket.issuer(), Cancel::fixture(fence), model.clone());
        let directory = super::super::ps_measurement::tests::directory();
        ingress
            .enable_measurement_only(
                super::super::ps_measurement::Host::open(&directory, 7_200_000).unwrap(),
            )
            .unwrap();
        // Match production startup: media selection follows the owner's first
        // publication, before an installed configuration is exposed.
        ready(&ingress);
        ingress.view.lock_unpoisoned().as_mut().unwrap().installed = None;
        ingress.enable_tagged_pcm_review().unwrap();
        ready(&ingress);
        let mut client = socket.connect(ingress, model);
        client.text("saturn_ps_measurement:0,start,300;");
        let deadline = Instant::now() + Duration::from_secs(2);
        let command = loop {
            if let Ok(c) = rx.commands.try_recv() {
                break c;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        let TxCommand::MeasurementOnly(request) = command else {
            panic!("wrong diagnostic command")
        };
        assert_eq!(request.duration_ns, 300_000_000);
        assert!(request.credential.valid());
        while !client.diagnostics.iter().any(|s| s.contains(",queued;"))
            || !client
                .diagnostics
                .iter()
                .any(|s| s.starts_with("saturn_ps_measurement_caps:0,true,1000,rx-bypass,"))
        {
            client.drain();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(client
            .diagnostics
            .iter()
            .any(|s| s.starts_with("saturn_ps_measurement_caps:0,true,1000,rx-bypass,")));
        assert!(client
            .diagnostics
            .iter()
            .any(|s| s == "saturn_ps_control_caps:0,measurement-only;"));
        // Connection bookkeeping still uses the ordinary control mailbox;
        // the diagnostic must not enqueue a legacy TX command there.
        assert!(matches!(
            socket.legacy.try_recv(),
            Ok(TciCommand::ClientConnected)
        ));
        assert!(socket.legacy.try_recv().is_err());
        drop(request);
        client.close();
    }
}
impl SharedIngress {
    /// Typed diagnostic entry. Ordinary trx, settings and reconnect never
    /// select it. The optional live diagnostic command uses the SAME sealed
    /// credential/age checks, with a separate host-only mode and export ticket.
    pub(crate) fn measurement_only(
        &self,
        client: u64,
        capture: &Capture,
        duration_ns: u64,
        recorder: saturn_bridge::puresignal::measurement::MeasurementRecorder,
    ) -> Result<(), Fault> {
        let request = self.measurement_request(client, capture, duration_ns, recorder)?;
        self.send_command(TxCommand::MeasurementOnly(request))
    }
    fn measurement_request(
        &self,
        client: u64,
        capture: &Capture,
        duration_ns: u64,
        recorder: saturn_bridge::puresignal::measurement::MeasurementRecorder,
    ) -> Result<super::ps_owner_queue::AuthorizedMeasurement, Fault> {
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
        super::ps_owner_queue::AuthorizedMeasurement::new(
            credential,
            duration_ns,
            capture.received_at,
            recorder,
        )
    }
    pub(crate) fn enable_measurement_only(
        &self,
        host: Arc<super::ps_measurement::Host>,
    ) -> Result<(), Fault> {
        if self.auto_mode_supported.load(Ordering::Acquire) {
            return Err(Fault::InvalidConfig);
        }
        self.measurement_host
            .set(host)
            .map_err(|_| Fault::InvalidConfig)
    }
    pub(crate) fn measurement_only_mode(&self) -> bool {
        self.measurement_host.get().is_some()
    }
    pub(crate) fn measurement_frequency_hz(&self) -> u32 {
        self.measurement_host.get().map_or(0, |h| h.frequency_hz)
    }
    pub(crate) fn measurement_duration_ms(&self) -> u64 {
        self.measurement_host
            .get()
            .map_or(super::ps_measurement::MAX_DURATION_MS, |h| {
                h.capture_mode.max_duration_ms()
            })
    }
    pub(crate) fn measurement_capture_mode(&self) -> &'static str {
        self.measurement_host
            .get()
            .map_or("unavailable", |h| h.capture_mode.label())
    }
    /// A diagnostic action must occupy a standalone control message. No mixed
    /// settings/PTT batch can hide or refresh the captured operator action.
    pub(crate) fn measurement_message(
        &self,
        client: u64,
        text: &str,
        capture: Option<&Capture>,
    ) -> bool {
        if !text
            .split(';')
            .any(|part| part.trim().split(':').next() == Some("saturn_ps_measurement"))
        {
            return false;
        }
        let reply = |status: &str| {
            self.operator.settings_reply(
                client,
                format!("saturn_ps_measurement:0,{status};"),
                || self.release(),
            )
        };
        if self.operator.settings_permission(client).is_err() {
            // An unauthorized viewer cannot cancel the operator by filling
            // its own reply queue. Revoke only that viewer on backpressure.
            self.operator.settings_reply(
                client,
                "saturn_ps_measurement:0,0,rejected,TxAuthority;".into(),
                || {},
            );
            return true;
        }
        // Never let a malformed diagnostic batch swallow an ordinary release.
        if text.split(';').any(|s| {
            s.trim().split_once(':').is_some_and(|(name, args)| {
                let args: Vec<_> = args.split(',').map(str::trim).collect();
                let enabled = if args.len() >= 2 {
                    args.get(1)
                } else {
                    args.first()
                };
                name.eq_ignore_ascii_case("trx")
                    && enabled.is_some_and(|v| crate::tci::parse_tci_bool(v) == Some(false))
            })
        }) {
            self.release();
        }
        let Some(host) = self.measurement_host.get() else {
            reply("0,rejected,DiagnosticDisabled");
            return true;
        };
        let parts: Vec<_> = text
            .split(';')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .collect();
        let args: Vec<_> = parts
            .first()
            .and_then(|s| s.strip_prefix("saturn_ps_measurement:"))
            .unwrap_or("")
            .split(',')
            .collect();
        if parts.len() != 1 || args.first() != Some(&"0") {
            reply("0,rejected,InvalidCommand");
            return true;
        }
        match args.as_slice() {
            ["0", "status"] => {
                reply(&host.status());
            }
            ["0", "stop"] => {
                self.release();
                reply("0,stop-requested");
            }
            ["0", "export", value] => {
                match value
                    .parse::<u64>()
                    .map_err(|_| "InvalidCaptureId".into())
                    .and_then(|id| host.export(id))
                {
                    Ok(path) => {
                        reply(&format!("{value},exported,{path}"));
                    }
                    Err(error) => {
                        reply(&format!("{value},export-rejected,{error}"));
                    }
                }
            }
            ["0", "start", value]
            | ["0", "start", value, "saturn-timing-multisine-v1"]
            | ["0", "start", value, "wdsp-postgen-two-tone-v1"] => {
                use saturn_bridge::puresignal::measurement::{CaptureContext, MeasurementRecorder};
                let request = (|| {
                    let ms = value.parse::<u64>().map_err(|_| Fault::InvalidConfig)?;
                    let c = capture.ok_or(Fault::CleanupPending)?;
                    use super::feedback_owner::Stimulus;
                    use crate::radio_model::DemodMode;
                    let stimulus = if args.len() == 4 {
                        if !matches!(c.model.desired.mode, DemodMode::Usb | DemodMode::Lsb) {
                            return Err(Fault::InvalidConfig);
                        }
                        if args[3] == "wdsp-postgen-two-tone-v1" {
                            Stimulus::NativeTwoTone
                        } else {
                            Stimulus::Timing
                        }
                    } else if matches!(c.model.desired.mode, DemodMode::Am | DemodMode::Sam) {
                        Stimulus::AmCarrier
                    } else {
                        Stimulus::TwoTone
                    };
                    if ms == 0
                        || ms > host.capture_mode.max_duration_ms()
                        || c.config.enabled
                        || c.config.automatic
                        || c.model.desired.pure_signal_enabled
                        || c.model.desired.tx_drive > super::ps_measurement::MAX_DRIVE_WATTS
                        || c.model.desired.tx_frequency_hz != host.frequency_hz
                    {
                        return Err(Fault::InvalidConfig);
                    }
                    let context = CaptureContext {
                        source_pin: env!("SATURN_BRIDGE_SOURCE_PIN").into(),
                        native_identity: format!(
                            "WDSP210 archive={} header={}",
                            env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"),
                            env!("SATURN_BRIDGE_WDSP_HEADER_SHA256")
                        ),
                        feedback_attenuation_db: Some(c.model.desired.pure_signal_attenuation_db),
                        commanded_drive: Some(c.model.desired.tx_drive),
                        physical_adc: c.config.physical_adc,
                    };
                    let recorder = match host.capture_mode {
                        super::ps_measurement::CaptureMode::Raw => MeasurementRecorder::new(
                            super::ps_measurement::CAPACITY_BLOCKS,
                            context,
                        )?,
                        super::ps_measurement::CaptureMode::SummaryOnly => {
                            if stimulus == Stimulus::Timing {
                                MeasurementRecorder::timing_summary(context)?
                            } else {
                                MeasurementRecorder::stimulus_summary(context)?
                            }
                        }
                    };
                    let mut request =
                        self.measurement_request(client, c, ms * 1_000_000, recorder)?;
                    request.stimulus = stimulus;
                    let ticket =
                        host.reserve(client, self.operator.clone(), self.cancel.clone(), stimulus)?;
                    if !ticket.queued() {
                        return Err(Fault::TxAuthority);
                    }
                    request.diagnostic = Some(ticket);
                    Ok(request)
                })();
                match request {
                    Ok(request) => {
                        if let Err(error) =
                            self.commands.try_send(TxCommand::MeasurementOnly(request))
                        {
                            let command = match error {
                                mpsc::TrySendError::Full(c)
                                | mpsc::TrySendError::Disconnected(c) => c,
                            };
                            if let TxCommand::MeasurementOnly(request) = &command {
                                if let Some(ticket) = &request.diagnostic {
                                    ticket.reject(Fault::QueueOverrun);
                                }
                            }
                            self.stop(Fault::QueueOverrun);
                        }
                    }
                    Err(fault) => {
                        reply(&format!("0,rejected,{fault:?}"));
                    }
                }
            }
            _ => {
                reply("0,rejected,InvalidCommand");
            }
        }
        true
    }
    pub(super) fn settings_epoch(&self) -> Arc<SettingsEpoch> {
        self.cancel.settings_epoch.clone()
    }
    pub(crate) fn channels(
        operator: OperatorIngress,
        cancel: Cancel,
        model: Arc<Mutex<RadioModel>>,
    ) -> (Self, Receivers) {
        let (commands, crx) = super::owner_mailbox::sync_channel(16);
        let (audio, arx) = super::owner_mailbox::sync_channel(16);
        let (feedback, frx) = super::owner_mailbox::sync_channel(FEEDBACK_QUEUE_BLOCKS);
        (
            Self {
                signal_capture: Arc::new(OnceLock::new()),
                commands,
                audio,
                feedback,
                feedback_pool: Arc::new(Mutex::new(
                    (0..FEEDBACK_POOL_BLOCKS)
                        .map(|_| Box::new(PairBlock::default()))
                        .collect(),
                )),
                operator: Arc::new(operator),
                cancel,
                view: Arc::new(Mutex::new(None)),
                fault: Arc::new(Mutex::new(None)),
                model,
                model_publication: Arc::new(Mutex::new(None)),
                selected: Arc::new(AtomicU8::new(0)),
                next_settings: Arc::new(AtomicU64::new(0)),
                two_tone_limits: Arc::new(OnceLock::new()),
                generated_trials_only: Arc::new(AtomicBool::new(false)),
                two_tone_pending: Arc::new(AtomicBool::new(false)),
                settings_tail: Arc::new(Mutex::new(None)),
                media: Arc::new(crate::tci::ps_media::RemoteMedia::default()),
                owner_status: Arc::new(Mutex::new((0, String::new(), String::new()))),
                measurement_host: Arc::new(OnceLock::new()),
                drive_ceiling: Arc::new(OnceLock::new()),
                auto_mode_supported: Arc::new(AtomicBool::new(false)),
                #[cfg(feature = "managed-puresignal")]
                rf_monitor: Arc::new(OnceLock::new()),
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
        if let Some((snapshot, config)) = &installed {
            self.drive_ceiling.get_or_init(|| snapshot.desired.tx_drive);
            let key = (config.id, phase, source);
            let mut projected = self.model_publication.lock_unpoisoned();
            let changed = projected.as_ref().is_none_or(|old| old.0 != config.id);
            if projected.as_ref() != Some(&key) {
                // Configuration commits still publish the complete model before
                // their terminal receipt. Those commits occur with output stopped.
                // Repeated live status must never wait behind a UI/RX model reader.
                // A phase-only projection is retried until successful; authoritative
                // lifecycle/media publication below proceeds independently.
                let model = if changed {
                    Some(self.model.lock_unpoisoned())
                } else {
                    match self.model.try_lock() {
                        Ok(model) => Some(model),
                        Err(std::sync::TryLockError::Poisoned(error)) => Some(error.into_inner()),
                        Err(std::sync::TryLockError::WouldBlock) => None,
                    }
                };
                if let Some(mut m) = model {
                    if changed {
                        // Commit fields carried by ordered owner transactions only.
                        // Repeated status publication must not restore stale RX
                        // volume/ADC/display/client state.
                        macro_rules! copy {($($field:ident),* $(,)?)=>{$(m.desired.$field=snapshot.desired.$field;)*};}
                        copy!(
                            mode,
                            vfo_a_hz,
                            vfo_b_hz,
                            active_vfo,
                            split_enabled,
                            rx_antenna,
                            rx_attenuation_db,
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
                    *projected = Some(key);
                }
            }
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
            self.operator.media_reply(
                format!("{text}{}", self.ps_mode_message().unwrap_or_default()),
                || self.release(),
            );
        }
    }
    pub(crate) fn owner_status_message(&self) -> Option<String> {
        let published = self.owner_status.lock_unpoisoned();
        let text = (!published.2.is_empty()).then(|| published.2.clone());
        drop(published);
        text.map(|text| format!("{text}{}", self.ps_mode_message().unwrap_or_default()))
    }
    /// Committed configuration, not proof of a fit or physical RF correction.
    fn ps_mode_message(&self) -> Option<String> {
        let view = self.view.lock_unpoisoned();
        let (model, config) = view.as_ref()?.installed.as_ref()?;
        let mode = if !config.enabled {
            "off"
        } else if config.automatic {
            "auto"
        } else {
            "single"
        };
        let status = view.as_ref()?.status;
        Some(format!("saturn_ps_mode:0,{},{};saturn_tx_fault_ack_state:0,{},{};saturn_tx_committed_drive:0,{},{};tx_puresignal_attenuation:0,{};",
            config.id, mode, status.stop_id, ack_eligible(status), config.id, model.desired.tx_drive, model.desired.pure_signal_attenuation_db))
    }
    pub(crate) fn ps_control_caps(&self) -> String {
        format!(
            "saturn_ps_control_caps:0,{};",
            if self.measurement_only_mode() {
                "measurement-only"
            } else {
                "single"
            }
        )
    }
    pub(crate) fn drive_limit_message(&self) -> String {
        format!(
            "saturn_tx_drive_limit:0,{};",
            self.drive_ceiling.get().copied().unwrap_or(1)
        )
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
            stop_id: v.status.stop_id,
            ack_eligible: ack_eligible(v.status),
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
        self.host_readiness().0
    }
    pub(crate) fn host_readiness(&self) -> (&'static str, bool) {
        let view = self.view.lock_unpoisoned();
        let clean = view.as_ref().is_some_and(|v| {
            v.clean
                && v.installed.is_some()
                && v.phase == Phase::ReadyOff
                && v.status.primary_fault.is_none()
                && v.status.authority_fault.is_none()
                && v.status.cleanup_fault.is_none()
        });
        let phase = match view.as_ref().map(|v| v.phase) {
            Some(Phase::Active) if self.cancel.output_open() => "shared-keyed",
            Some(Phase::AwaitingKey) if self.cancel.output_open() => "shared-armed",
            Some(Phase::ReadyOff) => "shared-ready-off",
            Some(_) => "shared-stopping",
            None => "shared-unavailable",
        };
        (phase, clean)
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
            let detected = SystemTime::now();
            let notification_started = Instant::now();
            self.stop(Fault::QueueOverrun);
            queue_fault_report("command-ingress", detected, notification_started);
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
                let detected = SystemTime::now();
                let notification_started = Instant::now();
                self.stop(Fault::QueueOverrun);
                queue_fault_report("audio-ingress", detected, notification_started);
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
                let detected = SystemTime::now();
                let notification_started = Instant::now();
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
                queue_fault_report("feedback-ingress", detected, notification_started);
                self.recycle_feedback_buffer(block);
                Err(Fault::QueueOverrun)
            }
        }
    }
    pub(crate) fn take_feedback_buffer(&self) -> Result<Box<PairBlock>, Fault> {
        // This mutex protects only pop/push of preallocated boxes. Neither
        // holder performs I/O, DSP, queue work or cancellation while holding it.
        // try_lock conflated brief recycling contention with actual exhaustion,
        // spuriously faulting a healthy feedback stream. Release never takes it.
        let block = self.feedback_pool.lock_unpoisoned().pop();
        block.ok_or_else(|| {
            eprintln!("shared TX queue fault boundary=feedback-pool-exhausted fault=QueueOverrun");
            Fault::QueueOverrun
        })
    }
    pub(crate) fn recycle_feedback_buffer(&self, mut block: Box<PairBlock>) {
        block.pairs = 0;
        let mut pool = self.feedback_pool.lock_unpoisoned();
        if pool.len() < FEEDBACK_POOL_BLOCKS {
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
                if self.measurement_only_mode() || self.generated_trials_only() {
                    return Err(Fault::InvalidConfig);
                }
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
                if matches!(command, TciCommand::SetPureSignalMode(3..)) {
                    return Err(Fault::InvalidConfig);
                }
                if matches!(command, TciCommand::SetPureSignalMode(2))
                    && !self.auto_mode_supported.load(Ordering::Acquire)
                {
                    return Err(Fault::InvalidConfig);
                }
                let result = if matches!(command, TciCommand::SetTxDrive(v)
                    if self.drive_ceiling.get().is_none_or(|limit| v > limit))
                {
                    Err(Fault::InvalidConfig)
                } else {
                    apply_config(command, &mut model, &mut config, &mut source)
                };
                if let Err(fault) = result {
                    // Variant only: SATP/control arguments may contain secrets.
                    let debug = format!("{command:?}");
                    let name = debug.split(['(', '{', ' ']).next().unwrap_or("Unknown");
                    eprintln!("shared TX settings rejected transaction={id} command={name} fault={fault:?} drive_ceiling={:?}", self.drive_ceiling.get());
                    return Err(fault);
                }
            }
            if self.measurement_only_mode() && (config.enabled || config.automatic) {
                return Err(Fault::InvalidConfig);
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
                    eprintln!("saturn-bridge: shared settings enqueue rejected: client={client} transaction={id} config={} boundary=command-mailbox fault=QueueOverrun", config.id);
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
            | SetPureSignalMode(_)
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
        // Normal receive controls use the same ordered stop/configure barrier
        // as tuning. The separately configured RX BYPASS feedback path remains
        // owned by acquisition preparation/restoration, not these settings.
        SetRxAntenna(v) if (1..=3).contains(&v) => m.desired.rx_antenna = v,
        SetRxAttenuation(v) if v <= 31 => m.desired.rx_attenuation_db = v,
        // ADC/sample-rate geometry still needs its own acquisition contract.
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
        // The immutable reviewed ceiling is checked before applying the batch.
        SetTxDrive(v) => m.desired.tx_drive = v.max(1),
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
        SetPureSignalEnabled(v) => {
            ps.enabled = v;
            ps.automatic = false; // this UI contract explicitly requests one fit, never Auto
        }
        SetPureSignalMode(mode) if mode <= 2 => {
            ps.enabled = mode != 0;
            ps.automatic = mode == 2;
        }
        SetPureSignalAttenuation(v) if v <= 31 => m.desired.pure_signal_attenuation_db = v,
        ResetPureSignal => {}
        SatpControl { ref action, .. } if action == "tci" || action == "satp" => {
            *source = if action == "satp" {
                TxAudioSource::Satp
            } else {
                TxAudioSource::Tci
            };
            m.satp.source = *source;
        }
        _ => return Err(Fault::InvalidConfig), // unsupported controls cannot reach legacy
    }
    Ok(())
}
