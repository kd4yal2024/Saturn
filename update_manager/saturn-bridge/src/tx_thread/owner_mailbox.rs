//! Fixed-capacity owner ingress. The active consumer never waits for a producer.
//! Producers serialize only a preallocated deque move. No sample
//! processing, formatting, I/O or payload destruction occurs under that lock.
//! The PCM-only overflow path uses a constant-time envelope comparison.
//! A contended receive is temporarily Empty, not overflow or disconnection.
//! Original envelopes/ages remain queued; lifecycle deadlines still run first.
use std::{
    cell::Cell,
    collections::VecDeque,
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{TryRecvError, TrySendError},
        Arc, Mutex, TryLockError,
    },
};

pub(crate) use crate::thread_wake::Wake;

struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    capacity: usize,
    receiver_alive: AtomicBool,
    senders: AtomicUsize,
    wake: Arc<Wake>,
}
pub(crate) struct SyncSender<T> {
    inner: Arc<Inner<T>>,
}
impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        self.inner.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: self.inner.clone(),
        }
    }
}
pub(crate) enum Receiver<T> {
    Owned(OwnedReceiver<T>),
    // Older deterministic fixtures retain their existing channels. Production
    // construction has no standard-channel receiver or fallback branch.
    #[cfg(test)]
    Fixture(std::sync::mpsc::Receiver<T>),
}
pub(crate) struct OwnedReceiver<T> {
    inner: Arc<Inner<T>>,
    // The serialized owner may move this endpoint, but cannot share it.
    consumer: PhantomData<Cell<()>>,
}

#[cfg(test)]
pub(crate) fn sync_channel<T>(capacity: usize) -> (SyncSender<T>, Receiver<T>) {
    sync_channel_with_wake(capacity, Arc::new(Wake::default()))
}
pub(crate) fn sync_channel_with_wake<T>(
    capacity: usize,
    wake: Arc<Wake>,
) -> (SyncSender<T>, Receiver<T>) {
    assert!(capacity > 0);
    let inner = Arc::new(Inner {
        queue: Mutex::new(VecDeque::with_capacity(capacity)),
        capacity,
        receiver_alive: AtomicBool::new(true),
        senders: AtomicUsize::new(1),
        wake,
    });
    (
        SyncSender {
            inner: inner.clone(),
        },
        Receiver::Owned(OwnedReceiver {
            inner,
            consumer: PhantomData,
        }),
    )
}
impl<T> SyncSender<T> {
    /// Live stateless PCM may skip its oldest queued block under pressure.
    /// The caller must compare only envelope metadata, and may not replace a
    /// control barrier, another source or another output generation. Return the
    /// skipped payload so its destruction/counters happen AFTER releasing the
    /// queue lock. All other mailboxes retain strict FIFO/full behavior.
    pub(crate) fn try_send_latest(
        &self,
        message: T,
        live_limit: usize,
        replaces: fn(&T, &T) -> bool,
    ) -> Result<Option<T>, TrySendError<T>> {
        let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        if !self.inner.receiver_alive.load(Ordering::Acquire) {
            return Err(TrySendError::Disconnected(message));
        }
        let limit = live_limit.clamp(1, self.inner.capacity);
        let skipped = if queue.len() >= limit {
            if !queue.front().is_some_and(|old| replaces(old, &message)) {
                return Err(TrySendError::Full(message));
            }
            queue.pop_front()
        } else {
            None
        };
        queue.push_back(message);
        drop(queue);
        self.inner.wake.notify();
        Ok(skipped)
    }
    pub(crate) fn try_send(&self, message: T) -> Result<(), TrySendError<T>> {
        self.send_inner(message, || {})
    }
    fn send_inner(&self, message: T, after_lock: impl FnOnce()) -> Result<(), TrySendError<T>> {
        // Ingress producer context; the active owner only calls try_recv.
        // Startup's owner-local bootstrap may enqueue while RF is off.
        let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        after_lock(); // production is an empty closure, optimized away
        if !self.inner.receiver_alive.load(Ordering::Acquire) {
            return Err(TrySendError::Disconnected(message));
        }
        if queue.len() == self.inner.capacity {
            return Err(TrySendError::Full(message));
        }
        queue.push_back(message);
        drop(queue);
        self.inner.wake.notify();
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn send_with_pause(
        &self,
        message: T,
        after_lock: impl FnOnce(),
    ) -> Result<(), TrySendError<T>> {
        self.send_inner(message, after_lock)
    }
}
impl<T> Drop for SyncSender<T> {
    fn drop(&mut self) {
        // One atomic decrement chooses exactly one final sender, even when
        // the final two handles close concurrently. Publish before the wake.
        if self.inner.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.wake.notify();
        }
    }
}
impl<T> Receiver<T> {
    #[cfg(test)]
    pub(crate) fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }
    pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
        let inner = match self {
            Self::Owned(receiver) => &receiver.inner,
            #[cfg(test)]
            Self::Fixture(receiver) => return receiver.try_recv(),
        };
        let mut queue = match inner.queue.try_lock() {
            Ok(queue) => queue,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(TryRecvError::Empty),
        };
        if let Some(message) = queue.pop_front() {
            return Ok(message);
        }
        if inner.senders.load(Ordering::Acquire) == 0 {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }
}
impl<T> Drop for OwnedReceiver<T> {
    fn drop(&mut self) {
        self.inner.receiver_alive.store(false, Ordering::Release);
    }
}
#[cfg(test)]
impl<T> From<std::sync::mpsc::Receiver<T>> for Receiver<T> {
    fn from(receiver: std::sync::mpsc::Receiver<T>) -> Self {
        Self::Fixture(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Weak},
        thread,
        time::Duration,
    };

    #[test]
    fn notification_before_registration_is_retained_without_creating_a_message() {
        let wake = Arc::new(Wake::default());
        let (tx, rx) = sync_channel_with_wake::<u64>(1, wake.clone());
        wake.notify();
        let worker_wake = wake.clone();
        let (done, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            worker_wake.register_current();
            worker_wake.wait(Duration::from_secs(1));
            done.send(rx.try_recv()).unwrap();
        });
        let outcome = result.recv_timeout(Duration::from_millis(200));
        wake.notify(); // always release/join even if the regression loses a wake
        worker.join().unwrap();
        assert_eq!(outcome.unwrap(), Err(TryRecvError::Empty));
        drop(tx);
    }

    #[test]
    fn each_shared_mailbox_wakes_between_arming_and_park_without_restamping() {
        for source in 0..3 {
            let wake = Arc::new(Wake::default());
            let channels: Vec<_> = (0..3)
                .map(|_| sync_channel_with_wake(2, wake.clone()))
                .collect();
            let (senders, mut receivers): (Vec<_>, Vec<_>) = channels.into_iter().unzip();
            let receiver = receivers.remove(source);
            let worker_wake = wake.clone();
            let (armed, entering) = mpsc::channel();
            let resume = Arc::new(AtomicBool::new(false));
            let proceed = resume.clone();
            let (done, result) = mpsc::channel();
            let worker = thread::spawn(move || {
                worker_wake.register_current();
                worker_wake.wait_before_park(Duration::from_secs(1), || {
                    armed.send(()).unwrap();
                    let deadline = std::time::Instant::now() + Duration::from_secs(1);
                    while !proceed.load(Ordering::Acquire) {
                        assert!(std::time::Instant::now() < deadline);
                        thread::yield_now();
                    }
                });
                done.send(receiver.try_recv()).unwrap();
            });
            entering.recv_timeout(Duration::from_secs(1)).unwrap();
            senders[source].try_send((source, 42u64, 123u64)).unwrap();
            resume.store(true, Ordering::Release);
            let outcome = result.recv_timeout(Duration::from_millis(200));
            wake.notify();
            worker.join().unwrap();
            assert_eq!(outcome.unwrap(), Ok((source, 42, 123)));
        }
    }

    #[test]
    fn last_sender_disconnect_is_visible_before_the_idle_receiver_wakes() {
        let wake = Arc::new(Wake::default());
        let (sender, receiver) = sync_channel_with_wake::<u64>(1, wake.clone());
        let clone = sender.clone();
        drop(clone);
        let worker_wake = wake.clone();
        let (armed, entering) = mpsc::channel();
        let resume = Arc::new(AtomicBool::new(false));
        let proceed = resume.clone();
        let (done, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            worker_wake.register_current();
            worker_wake.wait_before_park(Duration::from_secs(1), || {
                armed.send(()).unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(1);
                while !proceed.load(Ordering::Acquire) {
                    assert!(std::time::Instant::now() < deadline);
                    thread::yield_now();
                }
            });
            done.send(receiver.try_recv()).unwrap();
        });
        entering.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(sender);
        resume.store(true, Ordering::Release);
        let outcome = result.recv_timeout(Duration::from_millis(200));
        wake.notify();
        worker.join().unwrap();
        assert_eq!(outcome.unwrap(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn concurrent_final_senders_publish_disconnect_and_wake_the_parked_owner() {
        for _ in 0..64 {
            let wake = Arc::new(Wake::default());
            let (first, receiver) = sync_channel_with_wake::<u64>(1, wake.clone());
            let second = first.clone();
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let (entered, ready) = mpsc::channel();
            let (done, result) = mpsc::channel();
            let worker_wake = wake.clone();
            let worker = thread::spawn(move || {
                worker_wake.register_current();
                worker_wake.wait_before_park(Duration::from_secs(1), || entered.send(()).unwrap());
                done.send(receiver.try_recv()).unwrap();
            });
            ready.recv_timeout(Duration::from_secs(1)).unwrap();
            thread::scope(|scope| {
                for sender in [first, second] {
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        drop(sender);
                    });
                }
                barrier.wait();
            });
            let outcome = result.recv_timeout(Duration::from_millis(200));
            wake.notify(); // join even if a regression missed disconnection
            worker.join().unwrap();
            assert_eq!(outcome.unwrap(), Err(TryRecvError::Disconnected));
        }
    }

    #[test]
    fn latest_pcm_retains_bounds_original_ages_and_generation_barriers() {
        let (tx, rx) = sync_channel(2);
        let same = |a: &(u64, u64), b: &(u64, u64)| a.0 == b.0;
        tx.try_send((7, 10)).unwrap();
        tx.try_send((7, 20)).unwrap();
        assert_eq!(tx.try_send_latest((7, 30), 2, same), Ok(Some((7, 10))));
        assert_eq!(
            tx.try_send_latest((8, 40), 2, same),
            Err(TrySendError::Full((8, 40)))
        );
        assert_eq!(rx.try_recv(), Ok((7, 20)));
        assert_eq!(rx.try_recv(), Ok((7, 30)));
        drop(rx);
        assert_eq!(
            tx.try_send_latest((7, 50), 2, same),
            Err(TrySendError::Disconnected((7, 50)))
        );
    }

    #[test]
    fn capacity_fifo_and_disconnect_preserve_every_envelope() {
        let (tx, rx) = sync_channel(2);
        let second = tx.clone();
        tx.try_send((1, 100)).unwrap();
        second.try_send((2, 50)).unwrap();
        assert_eq!(tx.try_send((3, 0)), Err(TrySendError::Full((3, 0))));
        drop(tx);
        drop(second);
        assert_eq!(rx.try_recv(), Ok((1, 100)));
        assert_eq!(rx.try_recv(), Ok((2, 50)));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }
    #[test]
    fn receiver_drop_returns_the_original_unsent_envelope() {
        let (tx, rx) = sync_channel(1);
        drop(rx);
        assert_eq!(
            tx.try_send((7, 900)),
            Err(TrySendError::Disconnected((7, 900)))
        );
    }
    #[test]
    fn receiver_close_during_a_paused_enqueue_cannot_accept_the_old_message() {
        let (tx, rx) = sync_channel(1);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            tx.send_with_pause((8, 123), || {
                entered_tx.send(()).unwrap();
                let _ = resume_rx.recv_timeout(Duration::from_secs(2));
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(rx);
        resume_tx.send(()).unwrap();
        assert_eq!(
            producer.join().unwrap(),
            Err(TrySendError::Disconnected((8, 123)))
        );
    }
    #[test]
    fn paused_producer_cannot_hold_the_consumer_and_old_age_is_not_refreshed() {
        let (tx, rx) = sync_channel(1);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            tx.send_with_pause((42, 100), || {
                entered_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (returned_tx, returned_rx) = mpsc::channel();
        let consumer = thread::spawn(move || {
            let result = rx.try_recv();
            returned_tx.send((rx, result)).unwrap();
        });
        let result = returned_rx.recv_timeout(Duration::from_secs(2));
        // Always release/join the producer even if a regression blocked receive.
        resume_tx.send(()).unwrap();
        producer.join().unwrap().unwrap();
        consumer.join().unwrap();
        let (rx, outcome) = result.expect("consumer waited for paused producer");
        assert_eq!(outcome, Err(TryRecvError::Empty));
        assert_eq!(
            rx.try_recv(),
            Ok((42, 100)),
            "retained original observation"
        );
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }
    #[test]
    fn payload_destruction_happens_after_queue_lock_is_released() {
        struct Payload {
            inner: Weak<Inner<Payload>>,
            dropped: Arc<AtomicBool>,
        }
        impl Drop for Payload {
            fn drop(&mut self) {
                let inner = self.inner.upgrade().unwrap();
                assert!(
                    inner.queue.try_lock().is_ok(),
                    "payload dropped under queue lock"
                );
                self.dropped.store(true, Ordering::Release);
            }
        }
        let (tx, rx) = sync_channel(1);
        let dropped = Arc::new(AtomicBool::new(false));
        tx.try_send(Payload {
            inner: Arc::downgrade(&tx.inner),
            dropped: dropped.clone(),
        })
        .ok()
        .unwrap();
        drop(rx.try_recv().unwrap());
        assert!(dropped.load(Ordering::Acquire));
    }
}
