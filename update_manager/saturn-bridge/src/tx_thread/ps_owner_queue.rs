//! Next offline integration step for the EXISTING TxCommand channel. No live
//! socket producer selects it. The legacy run loop refuses this typed arm;
//! only the shared-lifecycle consumer below accepts the sealed credential.
#![allow(dead_code)] // offline integration only; no production caller
use super::TxCommand;
use crate::{tci::ps_authorization::Credential, tx_owner_stop::OwnerStopAdapter};
use saturn_bridge::puresignal::{
    tx_lifecycle::{CooperativeNative, Phase, TxLifecycle},
    Fault,
};
use std::sync::mpsc::{Receiver, TryRecvError};

// Public wrapper for the public command enum; the credential stays sealed and
// non-Clone. Neither the producer nor the consumer can refresh its revision.
pub struct AuthorizedArm {
    pub(super) credential: Credential,
    pub(super) trial: Option<super::ps_ingress::TwoToneTrial>,
    pub(super) hardware_ptt: Option<crate::tci::HardwarePttPermit>,
}
/// Sealed, explicit diagnostic request. No boolean arm or settings completion
/// can become this action at dequeue. Capacity/context are fixed before queue.
pub struct AuthorizedMeasurement {
    pub(super) credential: Credential,
    pub(super) duration_ns: u64,
    pub(super) requested_at: std::time::Instant,
    pub(super) recorder: saturn_bridge::puresignal::measurement::MeasurementRecorder,
    pub(super) diagnostic: Option<super::ps_measurement::Ticket>,
    pub(super) stimulus: super::feedback_owner::Stimulus,
}
impl AuthorizedMeasurement {
    pub(crate) fn new(
        credential: Credential,
        duration_ns: u64,
        requested_at: std::time::Instant,
        recorder: saturn_bridge::puresignal::measurement::MeasurementRecorder,
    ) -> Result<Self, Fault> {
        if duration_ns == 0
            || duration_ns > saturn_bridge::puresignal::tx_lifecycle::MAX_MEASUREMENT_DURATION_NS
        {
            return Err(Fault::InvalidConfig);
        }
        Ok(Self {
            credential,
            duration_ns,
            requested_at,
            recorder,
            diagnostic: None,
            stimulus: Default::default(),
        })
    }
}
impl AuthorizedArm {
    pub(crate) fn new(credential: Credential) -> Self {
        Self {
            credential,
            trial: None,
            hardware_ptt: None,
        }
    }
    pub(super) fn two_tone(credential: Credential, trial: super::ps_ingress::TwoToneTrial) -> Self {
        Self {
            credential,
            trial: Some(trial),
            hardware_ptt: None,
        }
    }
}

/// One periodic lifecycle step, then at most one existing-channel command.
/// Out-of-band release/revocation/deadlines run even on an empty command queue.
/// This bounded slice migrates ARM only. Mic/rmatch, source selection, model
/// changes, source watchdogs and the installed scheduler remain unmigrated.
pub(crate) fn service_next<N: CooperativeNative>(
    life: &mut TxLifecycle<N, OwnerStopAdapter<'_>>,
    commands: &Receiver<TxCommand>,
    now: u64,
) -> Result<bool, Fault> {
    life.tick(now)?; // never process an arm before out-of-band release
    let command = match commands.try_recv() {
        Ok(command) => command,
        Err(TryRecvError::Empty) => return Ok(false),
        Err(TryRecvError::Disconnected) => {
            life.offline_owner().disconnect_output();
            life.release_handle().release();
            // Do not call tick twice: the first tick may already have advanced
            // native cleanup. Notification stops output but performs no DSP;
            // the next scheduled turn progresses cleanup.
            life.notify_fault(Fault::TxAuthority, now)?;
            return Err(Fault::TxAuthority);
        }
    };
    let TxCommand::PureSignalAuthorizedArm(arm) = command else {
        // Legacy Arm has no ingress identity/generation; do not restamp it.
        // Other commands must migrate explicitly rather than silently invoking
        // the legacy native owner. Release is an out-of-band mailbox, not this.
        return Err(Fault::TxAuthority);
    };
    if arm.hardware_ptt.as_ref().is_some_and(|p| !p.live()) {
        return Ok(true);
    }
    // This small legacy arm harness cannot service a generated trial's
    // host deadline/replies. Only the actual scheduled owner may consume it.
    if arm.trial.is_some() {
        return Err(Fault::InvalidConfig);
    }
    let credential = arm.credential;
    if life.status().phase != Phase::ReadyOff {
        return Err(Fault::CleanupPending);
    }
    let action = life.offline_owner().authorize(credential)?;
    life.fresh_tx_action(action, now)?;
    Ok(true)
}
