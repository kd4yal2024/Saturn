//! Fixed-capacity owner ingress. The active consumer never waits for a producer.
//! Producers serialize only a preallocated deque move. No callback, sample
//! processing, formatting, I/O or payload destruction occurs under that lock.
//! A contended receive is temporarily Empty, not overflow or disconnection.
//! Original envelopes/ages remain queued; lifecycle deadlines still run first.
use std::{
    cell::Cell,
    collections::VecDeque,
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{TryRecvError, TrySendError},
        Arc, Mutex, TryLockError, Weak,
    },
};

struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    capacity: usize,
    receiver_alive: AtomicBool,
    senders: Weak<()>,
}
pub(crate) struct SyncSender<T> {
    inner: Arc<Inner<T>>,
    live: Arc<()>,
}
impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            live: self.live.clone(),
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

pub(crate) fn sync_channel<T>(capacity: usize) -> (SyncSender<T>, Receiver<T>) {
    assert!(capacity > 0);
    let live = Arc::new(());
    let inner = Arc::new(Inner {
        queue: Mutex::new(VecDeque::with_capacity(capacity)),
        capacity,
        receiver_alive: AtomicBool::new(true),
        senders: Arc::downgrade(&live),
    });
    (
        SyncSender {
            inner: inner.clone(),
            live,
        },
        Receiver::Owned(OwnedReceiver {
            inner,
            consumer: PhantomData,
        }),
    )
}
impl<T> SyncSender<T> {
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
        if inner.senders.strong_count() == 0 {
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
    use std::{sync::mpsc, thread, time::Duration};

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
