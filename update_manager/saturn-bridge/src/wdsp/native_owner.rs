//! Process-local channel-1 ownership, shared by legacy and managed engines.
//! This is not cross-process radio/XDMA ownership or installed-library proof.
use saturn_bridge::puresignal::Fault;
use std::sync::atomic::{AtomicBool, Ordering};
static OWNED: AtomicBool = AtomicBool::new(false);
pub(super) struct Lease;
impl Lease {
    pub(super) fn acquire() -> Result<Self, Fault> {
        OWNED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Fault::ExternalController)?;
        Ok(Self)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        OWNED.store(false, Ordering::SeqCst);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_owner_lease_refuses_a_second_owner_and_releases_after_drop() {
        let first = Lease::acquire().unwrap();
        assert!(matches!(Lease::acquire(), Err(Fault::ExternalController)));
        drop(first);
        let fresh = Lease::acquire().unwrap();
        drop(fresh);
    }
}
