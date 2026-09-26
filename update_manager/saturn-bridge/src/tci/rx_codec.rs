//! Shared RX Opus encoders, one per channel profile. The DSP callback only
//! offers bounded PCM; libopus runs on workers and never on the RX thread.
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_uchar, c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::sync_ext::MutexExt;

use super::{ClientOutbound, OutboundMessage};

const RATE_HZ: u32 = 48_000;
const FRAME_SAMPLES: usize = 960; // 20 ms
const MAX_QUEUED_FRAMES: usize = FRAME_SAMPLES * 4; // 80 ms
const INPUT_GAP_RESET: Duration = Duration::from_millis(60);
const MAX_PACKET_BYTES: usize = 4000;
const OPUS_APPLICATION_AUDIO: c_int = 2049;
const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
const OPUS_SET_INBAND_FEC_REQUEST: c_int = 4012;
const OPUS_SET_DTX_REQUEST: c_int = 4016;

struct OpusLibrary {
    handle: *mut c_void,
    create: unsafe extern "C" fn(c_int, c_int, c_int, *mut c_int) -> *mut c_void,
    destroy: unsafe extern "C" fn(*mut c_void),
    encode: unsafe extern "C" fn(*mut c_void, *const f32, c_int, *mut c_uchar, c_int) -> c_int,
    ctl: unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int,
}
unsafe impl Send for OpusLibrary {}

impl OpusLibrary {
    fn load() -> Option<Self> {
        let name = CString::new("libopus.so.0").ok()?;
        let handle = unsafe { dlopen(name.as_ptr(), 2) };
        if handle.is_null() {
            return None;
        }
        let symbols = (|| unsafe {
            Some(Self {
                handle,
                create: symbol(handle, "opus_encoder_create")?,
                destroy: symbol(handle, "opus_encoder_destroy")?,
                encode: symbol(handle, "opus_encode_float")?,
                ctl: symbol(handle, "opus_encoder_ctl")?,
            })
        })();
        if symbols.is_none() {
            unsafe {
                dlclose(handle);
            }
        }
        symbols
    }
}
impl Drop for OpusLibrary {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.handle);
        }
    }
}

struct Encoder {
    library: OpusLibrary,
    state: *mut c_void,
}
unsafe impl Send for Encoder {}
impl Encoder {
    fn new(channels: u32) -> Option<Self> {
        let library = OpusLibrary::load()?;
        let mut error = 0;
        let state = unsafe {
            (library.create)(
                RATE_HZ as c_int,
                channels as c_int,
                OPUS_APPLICATION_AUDIO,
                &mut error,
            )
        };
        if state.is_null() || error != 0 {
            return None;
        }
        let encoder = Self { library, state };
        let bitrate = if channels == 1 { 24_000 } else { 80_000 };
        let settings_ok = unsafe {
            (encoder.library.ctl)(state, OPUS_SET_BITRATE_REQUEST, bitrate) == 0
                && (encoder.library.ctl)(state, OPUS_SET_INBAND_FEC_REQUEST, 0) == 0
                && (encoder.library.ctl)(state, OPUS_SET_DTX_REQUEST, 0) == 0
        };
        if settings_ok {
            Some(encoder)
        } else {
            None
        }
    }

    fn encode(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        let mut packet = vec![0u8; MAX_PACKET_BYTES];
        let length = unsafe {
            (self.library.encode)(
                self.state,
                pcm.as_ptr(),
                FRAME_SAMPLES as c_int,
                packet.as_mut_ptr(),
                MAX_PACKET_BYTES as c_int,
            )
        };
        if length <= 0 {
            return None;
        }
        packet.truncate(length as usize);
        Some(packet)
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            (self.library.destroy)(self.state);
        }
    }
}

struct Queue {
    samples: VecDeque<f32>,
    recipients: Vec<Weak<ClientOutbound>>,
    // A recipient that just joined waits here until the audio queued before
    // it joined (the `usize`, in samples) has been drained by the worker, so
    // it is promoted into `recipients` having heard nothing that predates
    // it. This is what lets a join/leave never touch `samples` itself:
    // continuity for the existing audience and "no pre-join audio" for the
    // newcomer are both satisfied without a shared flush.
    joining: Vec<(Weak<ClientOutbound>, usize)>,
    last_offer_at: Option<Instant>,
}
impl Queue {
    /// Advances every pending joiner's backlog by `removed` and promotes any
    /// that reach zero into `recipients`. Must be called with exactly how
    /// many samples just left the front of `self.samples`, from the *same*
    /// critical section that removed them -- a worker's `frame_len` drain,
    /// or `offer`'s drop-oldest-on-overflow eviction. Both remove audio from
    /// the front without necessarily delivering it anywhere, so both must
    /// count against a joiner's backlog, or a joiner staged against audio
    /// that overflow later silently discarded would wait for backlog that no
    /// longer exists.
    fn account_for_front_removal(&mut self, removed: usize) {
        let mut promoted = Vec::new();
        self.joining.retain_mut(|(recipient, remaining)| {
            *remaining = remaining.saturating_sub(removed);
            let ready = *remaining == 0;
            if ready {
                promoted.push(recipient.clone());
            }
            !ready
        });
        self.recipients.extend(promoted);
    }

    /// Discards all queued audio and immediately promotes every pending
    /// joiner: with the queue empty, none of them has any backlog left to
    /// wait out, so making them wait for a `usize` count of samples that no
    /// longer exists would just delay them for no reason.
    fn clear_samples(&mut self) {
        self.samples.clear();
        for (recipient, _) in self.joining.drain(..) {
            self.recipients.push(recipient);
        }
    }
}

pub(crate) struct RxOpusIngress {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    channels: u32,
    failed: Arc<AtomicBool>,
    tx_priority: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    overflow_samples: AtomicU64,
    contention_drops: AtomicU64,
}
impl RxOpusIngress {
    fn start(channels: u32, tx_priority: Arc<AtomicBool>) -> Option<Self> {
        if !(1..=2).contains(&channels) {
            return None;
        }
        let encoder = Encoder::new(channels)?;
        let shared = Arc::new((
            Mutex::new(Queue {
                samples: VecDeque::with_capacity(MAX_QUEUED_FRAMES * channels as usize),
                recipients: Vec::new(),
                joining: Vec::new(),
                last_offer_at: None,
            }),
            Condvar::new(),
        ));
        let failed = Arc::new(AtomicBool::new(false));
        let worker_shared = shared.clone();
        let worker_failed = failed.clone();
        let worker_tx_priority = tx_priority.clone();
        let epoch = Arc::new(AtomicU64::new(0));
        let worker_epoch = epoch.clone();
        thread::Builder::new()
            .name("rx-opus".into())
            .spawn(move || {
                worker_loop(
                    worker_shared,
                    worker_failed,
                    worker_tx_priority,
                    worker_epoch,
                    encoder,
                    channels,
                );
            })
            .ok()?;
        Some(Self {
            shared,
            channels,
            failed,
            tx_priority,
            epoch,
            overflow_samples: AtomicU64::new(0),
            contention_drops: AtomicU64::new(0),
        })
    }

    pub(crate) fn offer(&self, samples: &[f32], recipients: Vec<Arc<ClientOutbound>>) -> bool {
        if self.failed.load(Ordering::Acquire) || self.tx_priority.load(Ordering::Acquire) {
            return false;
        }
        let (mutex, wake) = &*self.shared;
        let Ok(mut queue) = mutex.try_lock() else {
            self.contention_drops.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let max_samples = MAX_QUEUED_FRAMES * self.channels as usize;
        let now = Instant::now();
        if queue
            .last_offer_at
            .map(|last| now.saturating_duration_since(last) > INPUT_GAP_RESET)
            .unwrap_or(false)
        {
            queue.clear_samples();
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        queue.last_offer_at = Some(now);
        // Audience membership changes without touching `samples`: a departure
        // needs no special handling (it just stops receiving), and a join is
        // staged in `joining` rather than flushing the shared queue -- doing
        // that used to cost every *existing* listener up to 80 ms of audio
        // every time any client (dis)connected, including the second half of
        // a split-session pair arriving after the first.
        queue.recipients.retain(|existing| {
            recipients
                .iter()
                .any(|wanted| existing.ptr_eq(&Arc::downgrade(wanted)))
        });
        queue.joining.retain(|(existing, _)| {
            recipients
                .iter()
                .any(|wanted| existing.ptr_eq(&Arc::downgrade(wanted)))
        });
        // Audio already queued at this instant predates the newcomer; the
        // worker promotes it into `recipients` once that much has drained, so
        // it never hears a frame built from before it joined. Nothing to wait
        // out (an empty queue, the common case for the very first join) means
        // joining immediately rather than waiting for the worker's next
        // drain, which would otherwise cost a real listener one wasted cycle
        // for no reason.
        let pending_before = queue.samples.len();
        for wanted in &recipients {
            let weak = Arc::downgrade(wanted);
            let already_known = queue.recipients.iter().any(|r| r.ptr_eq(&weak))
                || queue.joining.iter().any(|(r, _)| r.ptr_eq(&weak));
            if !already_known {
                if pending_before == 0 {
                    queue.recipients.push(weak);
                } else {
                    queue.joining.push((weak, pending_before));
                }
            }
        }
        // Keep whole channel-aligned frames and always discard oldest audio.
        let length = samples.len() / self.channels as usize * self.channels as usize;
        let take = length.min(max_samples);
        let incoming = &samples[length - take..length];
        let overflow = queue
            .samples
            .len()
            .saturating_add(length)
            .saturating_sub(max_samples);
        self.overflow_samples
            .fetch_add(overflow as u64, Ordering::Relaxed);
        let overflow = queue
            .samples
            .len()
            .saturating_add(take)
            .saturating_sub(max_samples);
        for _ in 0..overflow {
            queue.samples.pop_front();
        }
        queue.account_for_front_removal(overflow);
        queue.samples.extend(incoming.iter().copied());
        wake.notify_one();
        true
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    fn clear_pending(&self) {
        let mut queue = self.shared.0.lock_unpoisoned();
        queue.clear_samples();
        queue.last_offer_at = None;
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
    pub(crate) fn drop_counts(&self) -> (u64, u64) {
        (
            self.overflow_samples.load(Ordering::Relaxed),
            self.contention_drops.load(Ordering::Relaxed),
        )
    }
}

/// Tracks encode failures so one bad frame cannot permanently disable a
/// shared encoder for every current and future client on this channel
/// profile. Only a sustained run of failures — which in practice means the
/// encoder or library state is actually broken, not a single glitch frame —
/// is treated as fatal.
struct EncodeHealth {
    consecutive_failures: u32,
}
impl EncodeHealth {
    const MAX_CONSECUTIVE_FAILURES: u32 = 8;
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
        }
    }
    /// Returns true once the failure run should be treated as permanent.
    fn record(&mut self, encoded: bool) -> bool {
        if encoded {
            self.consecutive_failures = 0;
            false
        } else {
            self.consecutive_failures += 1;
            self.consecutive_failures >= Self::MAX_CONSECUTIVE_FAILURES
        }
    }
}

fn worker_loop(
    shared: Arc<(Mutex<Queue>, Condvar)>,
    failed: Arc<AtomicBool>,
    tx_priority: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    mut encoder: Encoder,
    channels: u32,
) {
    let frame_len = FRAME_SAMPLES * channels as usize;
    let mut health = EncodeHealth::new();
    loop {
        let (frame, recipients, frame_epoch) = {
            let (mutex, wake) = &*shared;
            let mut queue = mutex.lock_unpoisoned();
            while queue.samples.len() < frame_len {
                queue = wake
                    .wait(queue)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            let drained = queue.samples.drain(..frame_len).collect::<Vec<_>>();
            // Snapshot who is live *before* accounting for this drain: a
            // joiner this exact drain promotes must wait for the next frame,
            // never the one that still contains the audio they were staged
            // to avoid.
            let recipients = queue.recipients.clone();
            queue.account_for_front_removal(frame_len);
            (drained, recipients, epoch.load(Ordering::Acquire))
        };
        if tx_priority.load(Ordering::Acquire) {
            continue;
        }
        let Some(packet) = encoder.encode(&frame) else {
            eprintln!(
                "saturn-bridge: RX Opus encode failed (channels={channels}, consecutive_failures={})",
                health.consecutive_failures + 1
            );
            if health.record(false) {
                failed.store(true, Ordering::Release);
                break;
            }
            continue;
        };
        health.record(true);
        for outbound in recipients {
            if tx_priority.load(Ordering::Acquire) || epoch.load(Ordering::Acquire) != frame_epoch {
                break;
            }
            if let Some(outbound) = outbound.upgrade() {
                outbound.enqueue(OutboundMessage::OpusAudioFrame {
                    receiver: 0,
                    sample_rate: RATE_HZ,
                    channels,
                    packet: packet.clone(),
                    sequence: 0,
                });
            }
        }
    }
}

pub(crate) struct RxOpusTransport {
    mono: Option<RxOpusIngress>,
    stereo: Option<RxOpusIngress>,
}
static RX_OPUS_TRANSPORT: OnceLock<RxOpusTransport> = OnceLock::new();
static RX_OPUS_TX_PRIORITY: OnceLock<Arc<AtomicBool>> = OnceLock::new();
fn tx_priority_flag() -> Arc<AtomicBool> {
    RX_OPUS_TX_PRIORITY
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}
impl RxOpusTransport {
    pub(crate) fn global() -> &'static Self {
        RX_OPUS_TRANSPORT.get_or_init(|| {
            let tx_priority = tx_priority_flag();
            Self {
                mono: RxOpusIngress::start(1, tx_priority.clone()),
                stereo: RxOpusIngress::start(2, tx_priority.clone()),
            }
        })
    }
    pub(crate) fn current() -> Option<&'static Self> {
        RX_OPUS_TRANSPORT.get()
    }
    pub(crate) fn profile(&self, channels: u32) -> Option<&RxOpusIngress> {
        match channels {
            1 => self.mono.as_ref(),
            2 => self.stereo.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn available(&self, channels: u32) -> bool {
        self.profile(channels)
            .map(|profile| !profile.failed())
            .unwrap_or(false)
    }
    fn clear_pending(&self) {
        if let Some(mono) = &self.mono {
            mono.clear_pending();
        }
        if let Some(stereo) = &self.stereo {
            stereo.clear_pending();
        }
    }
    pub(crate) fn drop_counts(&self) -> (u64, u64) {
        let (m_overflow, m_contention) = self
            .mono
            .as_ref()
            .map(RxOpusIngress::drop_counts)
            .unwrap_or_default();
        let (s_overflow, s_contention) = self
            .stereo
            .as_ref()
            .map(RxOpusIngress::drop_counts)
            .unwrap_or_default();
        (m_overflow + s_overflow, m_contention + s_contention)
    }
}

pub(crate) fn set_rx_opus_tx_priority(active: bool) {
    let priority = tx_priority_flag();
    let previous = priority.swap(active, Ordering::AcqRel);
    if active && !previous {
        if let Some(transport) = RxOpusTransport::current() {
            transport.clear_pending();
        }
    }
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}
unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let address = dlsym(handle, name.as_ptr());
    if address.is_null() {
        return None;
    }
    Some(std::mem::transmute_copy::<*mut c_void, T>(&address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_encode_failure_does_not_kill_a_shared_encoder() {
        let mut health = EncodeHealth::new();
        for _ in 0..EncodeHealth::MAX_CONSECUTIVE_FAILURES - 1 {
            assert!(!health.record(false));
        }
        // A success before the threshold resets the run; one bad frame (or a
        // few, scattered) must never take Opus away from every client.
        assert!(!health.record(true));
        for _ in 0..EncodeHealth::MAX_CONSECUTIVE_FAILURES - 1 {
            assert!(!health.record(false));
        }
    }

    #[test]
    fn sustained_encode_failure_is_reported_as_permanent() {
        let mut health = EncodeHealth::new();
        for _ in 0..EncodeHealth::MAX_CONSECUTIVE_FAILURES - 1 {
            assert!(!health.record(false));
        }
        assert!(health.record(false));
        // Stays permanent from the caller's perspective: the worker breaks
        // out of its loop on the first `true`, so a further record() call
        // never happens in production, but the guard itself keeps failing.
        assert!(health.record(false));
    }

    fn test_ingress(channels: u32) -> RxOpusIngress {
        RxOpusIngress {
            shared: Arc::new((
                Mutex::new(Queue {
                    samples: VecDeque::new(),
                    recipients: Vec::new(),
                    joining: Vec::new(),
                    last_offer_at: None,
                }),
                Condvar::new(),
            )),
            channels,
            failed: Arc::new(AtomicBool::new(false)),
            tx_priority: Arc::new(AtomicBool::new(false)),
            epoch: Arc::new(AtomicU64::new(0)),
            overflow_samples: AtomicU64::new(0),
            contention_drops: AtomicU64::new(0),
        }
    }

    #[test]
    fn existing_listener_keeps_its_queued_audio_when_someone_else_joins() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&[1.0, 2.0, 3.0], vec![a.clone()]));
        // A second client joining -- e.g. the media half of a split pair
        // arriving after the control half -- must not cost `a` a single
        // sample of what is already queued for it.
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[4.0], vec![a.clone(), b.clone()]));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(queue.samples, VecDeque::from(vec![1.0, 2.0, 3.0, 4.0]));
        assert_eq!(queue.recipients.len(), 1, "b is staged, not live yet");
        assert_eq!(queue.joining.len(), 1);
        assert_eq!(
            queue.joining[0].1, 3,
            "b waits out the 3 samples that predate it"
        );
    }

    #[test]
    fn joining_recipient_is_promoted_only_once_its_pre_join_backlog_drains() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&[1.0, 2.0, 3.0], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        {
            let mut queue = ingress.shared.0.lock_unpoisoned();
            queue.account_for_front_removal(2);
            assert_eq!(
                queue.recipients.len(),
                1,
                "still short of the 3-sample backlog"
            );
            assert_eq!(queue.joining[0].1, 1);
            queue.account_for_front_removal(1);
        }
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(
            queue.recipients.len(),
            2,
            "b is live once its backlog fully drained"
        );
        assert!(queue.joining.is_empty());
    }

    #[test]
    fn departing_recipient_is_dropped_without_touching_the_queue_or_others() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[1.0, 2.0], vec![a.clone(), b.clone()]));
        assert!(ingress.offer(&[3.0], vec![a.clone()]));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(queue.samples, VecDeque::from(vec![1.0, 2.0, 3.0]));
        assert_eq!(queue.recipients.len(), 1);
        assert!(queue.recipients[0].ptr_eq(&Arc::downgrade(&a)));
    }

    #[test]
    fn joiner_is_excluded_from_the_frame_that_drains_its_own_backlog() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        // Exactly one full frame already queued for `a` before `b` joins.
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        assert_eq!(
            ingress.shared.0.lock_unpoisoned().joining[0].1,
            FRAME_SAMPLES
        );

        // Mirrors worker_loop's corrected order: snapshot recipients for the
        // frame about to be drained *before* accounting for that drain.
        let first_frame_recipients = ingress.shared.0.lock_unpoisoned().recipients.clone();
        assert_eq!(
            first_frame_recipients.len(),
            1,
            "b must not receive the frame that still contains its own pre-join backlog"
        );
        {
            let mut queue = ingress.shared.0.lock_unpoisoned();
            queue.samples.drain(..FRAME_SAMPLES);
            queue.account_for_front_removal(FRAME_SAMPLES);
        }
        let second_frame_recipients = ingress.shared.0.lock_unpoisoned().recipients.clone();
        assert_eq!(
            second_frame_recipients.len(),
            2,
            "b is live starting the next, fully post-join frame"
        );
    }

    #[test]
    fn partial_frame_pre_join_backlog_delays_promotion_to_the_next_full_frame() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES / 2], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        assert_eq!(
            ingress.shared.0.lock_unpoisoned().joining[0].1,
            FRAME_SAMPLES / 2
        );
        // Top the queue up to one full frame with fresh, post-join audio.
        assert!(ingress.offer(&vec![2.0; FRAME_SAMPLES / 2], vec![a.clone(), b.clone()]));
        let mixed_frame_recipients = ingress.shared.0.lock_unpoisoned().recipients.clone();
        assert_eq!(
            mixed_frame_recipients.len(),
            1,
            "this frame still mixes pre-join audio with post-join audio; b must wait"
        );
        {
            let mut queue = ingress.shared.0.lock_unpoisoned();
            queue.samples.drain(..FRAME_SAMPLES);
            queue.account_for_front_removal(FRAME_SAMPLES);
        }
        assert_eq!(
            ingress.shared.0.lock_unpoisoned().recipients.len(),
            2,
            "b is live once a fully clean frame begins"
        );
    }

    #[test]
    fn overflow_eviction_counts_against_a_joiners_backlog() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&vec![1.0; MAX_QUEUED_FRAMES], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        assert_eq!(
            ingress.shared.0.lock_unpoisoned().joining[0].1,
            MAX_QUEUED_FRAMES
        );
        // New audio overflows the already-full queue, evicting old samples
        // from the front -- audio that predates b and that nobody, old or
        // new, will ever receive. That must count against b's backlog too,
        // or b would wait for audio that no longer exists anywhere.
        assert!(ingress.offer(&vec![2.0; FRAME_SAMPLES], vec![a.clone(), b.clone()]));
        assert_eq!(
            ingress.shared.0.lock_unpoisoned().joining[0].1,
            MAX_QUEUED_FRAMES - FRAME_SAMPLES,
            "evicted samples must not still count as backlog b has to wait out"
        );
    }

    #[test]
    fn input_gap_immediately_releases_pending_joiners() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        assert_eq!(ingress.shared.0.lock_unpoisoned().joining.len(), 1);
        {
            let mut queue = ingress.shared.0.lock_unpoisoned();
            queue.last_offer_at = Some(Instant::now() - INPUT_GAP_RESET - Duration::from_millis(1));
        }
        // The gap wipes the queue: there is no backlog left for b to wait
        // out, so it must not still be staged as "waiting" afterward.
        assert!(ingress.offer(&vec![3.0; FRAME_SAMPLES / 2], vec![a.clone(), b.clone()]));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert!(queue.joining.is_empty());
        assert_eq!(queue.recipients.len(), 2);
    }

    #[test]
    fn tx_priority_clear_immediately_releases_pending_joiners() {
        let ingress = test_ingress(1);
        let a = ClientOutbound::new();
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES], vec![a.clone()]));
        let b = ClientOutbound::new();
        assert!(ingress.offer(&[], vec![a.clone(), b.clone()]));
        assert_eq!(ingress.shared.0.lock_unpoisoned().joining.len(), 1);
        ingress.clear_pending();
        let queue = ingress.shared.0.lock_unpoisoned();
        assert!(queue.joining.is_empty());
        assert_eq!(
            queue.recipients.len(),
            2,
            "b is released once the backlog it was waiting for is gone"
        );
    }

    #[test]
    fn opus_queue_is_bounded_and_drops_oldest() {
        let ingress = test_ingress(2);
        assert!(ingress.offer(&vec![1.0; MAX_QUEUED_FRAMES * 2], Vec::new()));
        assert!(ingress.offer(&vec![2.0; FRAME_SAMPLES * 2], Vec::new()));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(queue.samples.len(), MAX_QUEUED_FRAMES * 2);
        assert_eq!(queue.samples[0], 1.0);
        assert_eq!(queue.samples[queue.samples.len() - 1], 2.0);
        assert_eq!(ingress.drop_counts(), ((FRAME_SAMPLES * 2) as u64, 0));
        assert!(!ingress.offer(&[0.0, 0.0], Vec::new()));
        assert_eq!(ingress.drop_counts(), ((FRAME_SAMPLES * 2) as u64, 1));
    }

    #[test]
    fn input_gap_discards_partial_packet_before_resume() {
        let ingress = test_ingress(1);
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES / 2], Vec::new()));
        {
            let mut queue = ingress.shared.0.lock_unpoisoned();
            queue.last_offer_at = Some(Instant::now() - INPUT_GAP_RESET - Duration::from_millis(1));
        }
        assert!(ingress.offer(&vec![2.0; FRAME_SAMPLES / 2], Vec::new()));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(queue.samples.len(), FRAME_SAMPLES / 2);
        assert!(queue.samples.iter().all(|&sample| sample == 2.0));
    }

    #[test]
    fn tx_priority_discards_queued_partial_audio_and_blocks_offers() {
        let tx_priority = Arc::new(AtomicBool::new(false));
        let ingress = RxOpusIngress {
            shared: Arc::new((
                Mutex::new(Queue {
                    samples: VecDeque::new(),
                    recipients: Vec::new(),
                    joining: Vec::new(),
                    last_offer_at: None,
                }),
                Condvar::new(),
            )),
            channels: 1,
            failed: Arc::new(AtomicBool::new(false)),
            tx_priority: tx_priority.clone(),
            epoch: Arc::new(AtomicU64::new(0)),
            overflow_samples: AtomicU64::new(0),
            contention_drops: AtomicU64::new(0),
        };
        assert!(ingress.offer(&vec![1.0; FRAME_SAMPLES / 2], Vec::new()));
        tx_priority.store(true, Ordering::Release);
        ingress.clear_pending();
        assert_eq!(ingress.epoch.load(Ordering::Acquire), 1);
        assert!(!ingress.offer(&vec![9.0; FRAME_SAMPLES / 2], Vec::new()));
        assert!(ingress.shared.0.lock_unpoisoned().samples.is_empty());
        tx_priority.store(false, Ordering::Release);
        assert!(ingress.offer(&vec![2.0; FRAME_SAMPLES / 2], Vec::new()));
        let queue = ingress.shared.0.lock_unpoisoned();
        assert_eq!(queue.samples.len(), FRAME_SAMPLES / 2);
        assert!(queue.samples.iter().all(|&sample| sample == 2.0));
    }

    #[test]
    fn real_opus_packets_decode_as_twenty_millisecond_audio() {
        for channels in [1u32, 2] {
            let Some(mut encoder) = Encoder::new(channels) else {
                return;
            };
            let pcm = (0..FRAME_SAMPLES * channels as usize)
                .map(|index| ((index / channels as usize) as f32 * 0.02).sin() * 0.2)
                .collect::<Vec<_>>();
            let packet = encoder
                .encode(&pcm)
                .expect("libopus encoder must accept 20 ms PCM");
            assert!(!packet.is_empty());
            let frame = super::super::build_tci_opus_audio_frame(0, RATE_HZ, channels, &packet, 1);
            assert_eq!(frame.len(), 64 + packet.len());
            assert_eq!(
                u32::from_le_bytes(frame[20..24].try_into().unwrap()),
                packet.len() as u32
            );

            let library = OpusLibrary::load().unwrap();
            let create: unsafe extern "C" fn(c_int, c_int, *mut c_int) -> *mut c_void =
                unsafe { symbol(library.handle, "opus_decoder_create").unwrap() };
            let destroy: unsafe extern "C" fn(*mut c_void) =
                unsafe { symbol(library.handle, "opus_decoder_destroy").unwrap() };
            let decode: unsafe extern "C" fn(
                *mut c_void,
                *const c_uchar,
                c_int,
                *mut f32,
                c_int,
                c_int,
            ) -> c_int = unsafe { symbol(library.handle, "opus_decode_float").unwrap() };
            let mut error = 0;
            let decoder = unsafe { create(RATE_HZ as c_int, channels as c_int, &mut error) };
            assert_eq!(error, 0);
            assert!(!decoder.is_null());
            let mut output = vec![0.0f32; FRAME_SAMPLES * channels as usize];
            let decoded = unsafe {
                decode(
                    decoder,
                    packet.as_ptr(),
                    packet.len() as c_int,
                    output.as_mut_ptr(),
                    FRAME_SAMPLES as c_int,
                    0,
                )
            };
            unsafe {
                destroy(decoder);
            }
            assert_eq!(decoded, FRAME_SAMPLES as c_int);
            assert!(output.iter().any(|sample| sample.abs() > 0.01));
        }
    }
}
