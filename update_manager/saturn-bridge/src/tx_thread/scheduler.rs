//! Scheduling driver shared by the installed TX loop and the gated owner loop.
//! The consumer owns its queues/native state; the driver never creates a DSP
//! owner or selects a backend. Legacy timing is retained; the shared owner may
//! provide a notified idle wait. Active timing is common to both drivers.
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
    drive_with_idle_wait(stop, &mut service, || {
        thread::sleep(Duration::from_millis(1))
    })
}

/// Only an inactive empty turn may park. Active timing and work draining retain
/// the installed policy; a wakeup is not permission to render or key output.
pub(super) fn drive_with_idle_wait(
    stop: &AtomicBool,
    mut service: impl FnMut() -> Turn,
    mut idle_wait: impl FnMut(),
) -> bool {
    while !stop.load(Ordering::Relaxed) {
        match service() {
            Turn::Exit => return true,
            Turn::Continue { worked, active } => {
                if !worked {
                    if active {
                        thread::sleep(super::TX_ACTIVE_IDLE_SLEEP);
                    } else {
                        idle_wait();
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_wait_is_only_used_for_inactive_empty_turns() {
        let stop = AtomicBool::new(false);
        let mut turns = [
            Turn::Continue {
                worked: false,
                active: false,
            },
            Turn::Continue {
                worked: true,
                active: false,
            },
            Turn::Continue {
                worked: false,
                active: true,
            },
            Turn::Exit,
        ]
        .into_iter();
        let mut waits = 0;
        assert!(drive_with_idle_wait(
            &stop,
            || turns.next().unwrap(),
            || waits += 1
        ));
        assert_eq!(waits, 1);
        assert!(turns.next().is_none());
    }
    #[test]
    fn external_stop_is_checked_after_an_idle_wakeup() {
        let stop = AtomicBool::new(false);
        let mut turns = 0;
        assert!(!drive_with_idle_wait(
            &stop,
            || {
                turns += 1;
                Turn::Continue {
                    worked: false,
                    active: false,
                }
            },
            || stop.store(true, Ordering::Release)
        ));
        assert_eq!(turns, 1);
    }
}
