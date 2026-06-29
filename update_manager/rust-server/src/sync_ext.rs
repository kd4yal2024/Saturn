//! Synchronization helpers for long-running Saturn Go state.
//!
//! A panicking worker thread can poison a mutex. Saturn Go should report that
//! event, but update/status paths still need access to the guarded state so the
//! operator can recover without a process crash.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};

static MUTEX_POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub trait MutexExt<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            if !MUTEX_POISON_WARNED.swap(true, Ordering::Relaxed) {
                eprintln!("saturn-go: WARN mutex poisoned; recovering guarded state");
            }
            poisoned.into_inner()
        })
    }
}

impl<T> MutexExt<T> for Arc<Mutex<T>> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.as_ref().lock_unpoisoned()
    }
}

#[cfg(test)]
mod tests {
    use super::MutexExt;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn lock_unpoisoned_recovers_guarded_state() {
        let value = Arc::new(Mutex::new(1usize));
        let poison_target = Arc::clone(&value);

        let _ = thread::spawn(move || {
            let mut guard = poison_target
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = 2;
            panic!("intentional mutex poison for lock_unpoisoned test");
        })
        .join();

        let mut recovered = value.lock_unpoisoned();
        assert_eq!(*recovered, 2);
        *recovered = 3;
        drop(recovered);

        assert_eq!(*value.lock_unpoisoned(), 3);
    }
}
