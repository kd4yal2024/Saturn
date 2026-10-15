//! SIMULATED existing TX authority. No socket, DMA, GPIO, RF or physical readback.
use crate::puresignal::{tx_lifecycle::*, Backend, Fault};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub struct AuthorityTrace {
    pub events: Vec<&'static str>,
    pub output_inhibited: bool,
    pub drive: u32,
    pub receipt_after_polls: u32,
    pub stall: bool,
    pub stale_receipt_once: bool,
    pub invalid_receipt: bool,
    pub key_error: bool,
    pub release_tickets: Vec<u64>,
    pub arm_boundary: Option<(ReleaseHandle, Fault)>,
    pub health_release: Option<ReleaseHandle>,
}
pub struct SimAuthority {
    pub trace: Rc<RefCell<AuthorityTrace>>,
    stop: Option<StopRequest>,
    polls: u32,
    arm: Option<(FreshTxAction, u64)>,
}
impl SimAuthority {
    pub fn new(trace: Rc<RefCell<AuthorityTrace>>) -> Self {
        trace.borrow_mut().output_inhibited = true;
        Self {
            trace,
            stop: None,
            polls: 0,
            arm: None,
        }
    }
}
impl TxAuthority for SimAuthority {
    fn release_received(&mut self, ticket: u64, _: u64) -> Result<(), Fault> {
        let mut t = self.trace.borrow_mut();
        t.events.push("priority-release");
        t.release_tickets.push(ticket);
        t.output_inhibited = true;
        self.arm = None;
        Ok(())
    }
    fn begin_stop(&mut self, request: StopRequest) -> Result<(), Fault> {
        let mut t = self.trace.borrow_mut();
        t.events.push("stop-fence");
        t.output_inhibited = true; // immediately reject new/queued-generation writes
        self.arm = None;
        self.stop = Some(request);
        self.polls = 0;
        Ok(())
    }
    fn poll_stop(&mut self, id: u64) -> Result<Option<StopReceipt>, Fault> {
        let mut t = self.trace.borrow_mut();
        self.polls += 1;
        if t.stall || self.polls <= t.receipt_after_polls {
            return Ok(None);
        }
        let stop = self.stop.expect("stop before receipt");
        assert_eq!(id, stop.id);
        let mut receipt = StopReceipt {
            id,
            output_barrier_closed: true,
            in_flight_writes_drained: true,
            queued_output_discarded: true,
            unkey_operation_completed: true,
            evidence: if stop.backend == Backend::P2 {
                StopEvidence::P2CommandBurstOnly
            } else {
                StopEvidence::DirectControlReadback
            },
        };
        if t.stale_receipt_once {
            t.stale_receipt_once = false;
            receipt.id = id.saturating_sub(1);
        } else if t.invalid_receipt {
            receipt.in_flight_writes_drained = false;
        } else {
            t.events.push("writers-drained-and-queue-discarded");
            t.drive = 0;
            t.events.push("unkey-complete-simulated");
        }
        Ok(Some(receipt))
    }
    fn begin_arm(&mut self, action: FreshTxAction, generation: u64) -> Result<(), Fault> {
        if let Some((release, fault)) = self.trace.borrow_mut().arm_boundary.take() {
            release.release();
            return Err(fault);
        }
        self.trace
            .borrow_mut()
            .events
            .push("fresh-authorized-arm-simulated");
        self.arm = Some((action, generation));
        Ok(())
    }
    fn poll_health(&mut self) -> Result<(), Fault> {
        if let Some(release) = self.trace.borrow_mut().health_release.take() {
            release.release();
        }
        Ok(())
    }
    fn poll_key(&mut self, now: u64) -> Result<Option<KeyReceipt>, Fault> {
        let mut t = self.trace.borrow_mut();
        if t.key_error {
            return Err(Fault::TxAuthority);
        }
        Ok(self.arm.take().map(|(action, output_generation)| {
            t.events.push("key-complete-simulated");
            t.drive = 1; // fixture safe starting drive; NOT a production drive policy
            t.output_inhibited = false;
            KeyReceipt {
                action_sequence: action.sequence,
                output_generation,
                keyed_ns: now,
            }
        }))
    }
}
