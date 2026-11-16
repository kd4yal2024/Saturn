//! Coalesced single-consumer wake hints. Notifications carry no data/authority.
use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        OnceLock,
    },
    thread,
    time::Duration,
};

const PENDING: u8 = 1;
const PARKED: u8 = 2;

/// Coalesced hints only: this never dequeues/reorders a message or grants output.
/// One atomic word closes the notify-before-park race. The active consumer does
/// not wait; idle waits use Rust's retained unpark token and a bounded timeout.
#[derive(Default)]
pub(crate) struct Wake {
    state: AtomicU8,
    owner: OnceLock<thread::Thread>,
}
impl Wake {
    pub(crate) fn register_current(&self) {
        let current = thread::current();
        if let Err(current) = self.owner.set(current) {
            assert_eq!(
                self.owner.get().unwrap().id(),
                current.id(),
                "one parked owner"
            );
        }
    }
    pub(crate) fn notify(&self) {
        let previous = self.state.fetch_or(PENDING, Ordering::AcqRel);
        if previous & PARKED != 0 {
            if let Some(owner) = self.owner.get() {
                owner.unpark();
            }
        }
    }
    pub(crate) fn wait(&self, limit: Duration) {
        self.wait_inner(limit, || {});
    }
    #[cfg(test)]
    pub(crate) fn wait_before_park(&self, limit: Duration, before_park: impl FnOnce()) {
        self.wait_inner(limit, before_park);
    }
    fn wait_inner(&self, limit: Duration, before_park: impl FnOnce()) {
        let previous = self.state.swap(PARKED, Ordering::AcqRel);
        if previous & PENDING == 0 {
            before_park(); // empty in production; deterministic race seam in tests
            thread::park_timeout(limit);
        }
        // A concurrent notification remains pending for the next consumer turn.
        self.state.fetch_and(!PARKED, Ordering::AcqRel);
    }
}
