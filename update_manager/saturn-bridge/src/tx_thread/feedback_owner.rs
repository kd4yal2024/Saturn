//! Bounded handoff to the EXISTING acquisition owner. No reader, RF control or
//! polling is created here. Layout acknowledgments are host evidence only.
use crate::sync_ext::MutexExt;
use crate::tx_owner_stop::OutputStamp;
use saturn_bridge::puresignal::{Fault, Fence};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    pub(crate) fence: Fence,
    pub(crate) route: u128,
    pub(crate) frequency_hz: u32,
    pub(crate) attenuation_db: u8,
    pub(crate) output: OutputStamp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) serial: u64,
    pub(crate) scope: Option<Scope>,
}
struct State {
    desired: Request,
    completed: u64,
    failed: bool,
    restore_allowed: bool,
}
#[derive(Clone)]
pub(crate) struct Link(Arc<Mutex<State>>);
impl Link {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            desired: Request {
                serial: 0,
                scope: None,
            },
            completed: 0,
            failed: false,
            restore_allowed: true,
        })))
    }
    pub(crate) fn request(&self, scope: Option<Scope>) -> Result<Request, Fault> {
        let mut state = self.0.lock_unpoisoned();
        if state.failed {
            return Err(Fault::TxAuthority);
        }
        let serial = state
            .desired
            .serial
            .checked_add(1)
            .ok_or(Fault::GenerationMismatch)?;
        let unchanged_off = scope.is_none()
            && state.desired.scope.is_none()
            && state.completed == state.desired.serial;
        state.desired = Request { serial, scope };
        if unchanged_off {
            state.completed = serial;
        }
        state.restore_allowed = unchanged_off;
        Ok(state.desired)
    }
    pub(crate) fn desired(&self) -> Request {
        self.0.lock_unpoisoned().desired
    }
    pub(crate) fn preparing(&self) -> bool {
        let state = self.0.lock_unpoisoned();
        !state.failed && state.desired.scope.is_some() && state.completed != state.desired.serial
    }
    pub(crate) fn permit_restoration(&self, request: Request) {
        let mut state = self.0.lock_unpoisoned();
        if state.desired == request && request.scope.is_none() {
            state.restore_allowed = true;
        }
    }
    pub(crate) fn may_apply(&self, request: Request) -> bool {
        let state = self.0.lock_unpoisoned();
        !state.failed
            && state.desired == request
            && (request.scope.is_some() || state.restore_allowed)
    }
    /// An old asynchronous completion cannot acknowledge a later arm/restore.
    pub(crate) fn complete(&self, request: Request) -> bool {
        let mut state = self.0.lock_unpoisoned();
        if state.failed
            || state.desired != request
            || (request.scope.is_none() && !state.restore_allowed)
        {
            return false;
        }
        state.completed = request.serial;
        true
    }
    pub(crate) fn prepared(&self, stamp: OutputStamp) -> Result<bool, Fault> {
        let state = self.0.lock_unpoisoned();
        if state.failed {
            return Err(Fault::TxAuthority);
        }
        Ok(state.completed == state.desired.serial
            && state.desired.scope.is_some_and(|s| s.output == stamp))
    }
    pub(crate) fn restored(&self) -> Result<bool, Fault> {
        let state = self.0.lock_unpoisoned();
        if state.failed {
            return Err(Fault::TxAuthority);
        }
        Ok(state.desired.scope.is_none() && state.completed == state.desired.serial)
    }
    pub(crate) fn fail(&self) {
        self.0.lock_unpoisoned().failed = true;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn scope(generation: u64) -> Scope {
        Scope {
            fence: Fence::default(),
            route: 1,
            frequency_hz: 7_200_000,
            attenuation_db: 9,
            output: OutputStamp {
                generation,
                revision: generation,
            },
        }
    }
    #[test]
    fn late_layout_completion_cannot_cross_release_or_rearm() {
        let link = Link::new();
        let old = link.request(Some(scope(1))).unwrap();
        assert!(!link.prepared(scope(1).output).unwrap());
        let restore = link.request(None).unwrap();
        assert!(!link.may_apply(restore));
        assert!(!link.complete(restore));
        link.permit_restoration(restore);
        assert!(link.may_apply(restore));
        assert!(!link.complete(old));
        assert!(!link.restored().unwrap());
        assert!(link.complete(restore));
        assert!(link.restored().unwrap());
        let fresh = link.request(Some(scope(2))).unwrap();
        assert!(!link.complete(restore));
        assert!(link.complete(fresh));
        assert!(!link.prepared(scope(1).output).unwrap());
        assert!(link.prepared(scope(2).output).unwrap());
    }
    #[test]
    fn failed_acquisition_never_acknowledges_clear_or_prepared() {
        let link = Link::new();
        let request = link.request(Some(scope(1))).unwrap();
        link.fail();
        assert!(!link.complete(request));
        assert!(link.prepared(scope(1).output).is_err());
        assert!(link.restored().is_err());
    }
    pub(crate) fn collector() -> (
        crate::tx_thread::ps_ingress::SharedIngress,
        crate::tx_thread::ps_ingress::Receivers,
    ) {
        use crate::{
            tci::ps_authorization::OperatorIngress,
            tx_owner_stop::OutputFence,
            tx_thread::{ps_ingress::SharedIngress, ps_owner_loop::Cancel},
        };
        let fence = Arc::new(OutputFence::new_open_for_offline_trial());
        fence.close();
        SharedIngress::channels(
            OperatorIngress::fixture(),
            Cancel::fixture(fence),
            Arc::new(Mutex::new(crate::tx_owner_stop::tests::model())),
        )
    }
}
