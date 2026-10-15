//! Scheduling driver shared by the installed TX loop and the gated owner loop.
//! The consumer owns its queues/native state; the driver never creates a DSP
//! owner or selects a backend. Existing idle and active wait policy is retained.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

pub(crate) enum Turn {
    Continue { worked: bool, active: bool },
    Exit,
}

/// Returns true for explicit consumer exit, false for the external stop flag.
pub(super) fn drive(stop: &AtomicBool, mut service: impl FnMut() -> Turn) -> bool {
    while !stop.load(Ordering::Relaxed) {
        match service() {
            Turn::Exit => return true,
            Turn::Continue { worked, active } => {
                if !worked {
                    thread::sleep(if active {
                        super::TX_ACTIVE_IDLE_SLEEP
                    } else {
                        Duration::from_millis(1)
                    });
                }
            }
        }
    }
    false
}
