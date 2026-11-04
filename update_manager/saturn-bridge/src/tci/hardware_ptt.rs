//! Rear PTT is an edge-triggered operator action, not a remembered TX setting.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const PTT_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PRESS_DEBOUNCE: Duration = Duration::from_millis(8);
const INPUT_DEADLINE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub(crate) struct HardwarePttPermit {
    pub(crate) id: u64,
    pub(crate) context: Option<(u64, u64)>,
    live: Arc<AtomicBool>,
    claimed: Arc<AtomicBool>,
    operator: Option<(Arc<AtomicU64>, u64)>,
}
impl HardwarePttPermit {
    pub(crate) fn new(id: u64, context: Option<(u64, u64)>) -> Self {
        Self {
            id,
            context,
            live: Arc::new(AtomicBool::new(true)),
            claimed: Arc::new(AtomicBool::new(false)),
            operator: None,
        }
    }
    fn for_operator(mut self, operator: Arc<AtomicU64>, client: u64) -> Self {
        self.operator = Some((operator, client));
        self
    }
    pub(crate) fn live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
            && self
                .operator
                .as_ref()
                .is_none_or(|(operator, client)| operator.load(Ordering::SeqCst) == *client)
    }
    pub(crate) fn claim(&self) -> bool {
        self.live()
            && self
                .claimed
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            && self.live()
    }
    /// Invalidation precedes any queue work. Clones already queued see it too.
    pub(crate) fn cancel(&self) -> bool {
        self.live.store(false, Ordering::SeqCst);
        self.claimed.swap(false, Ordering::SeqCst)
    }
}

#[derive(Default)]
pub(crate) struct HardwarePttInput {
    raw: Option<bool>,
    changed: Option<Instant>,
    observed: Option<Instant>,
    emitted: bool,
    blocked: bool,
}
impl HardwarePttInput {
    pub(crate) fn observe(&mut self, pressed: bool, now: Instant) {
        if self.raw.is_none() {
            self.blocked = pressed;
        }
        if self.raw != Some(pressed) {
            self.changed = Some(now);
        }
        self.raw = Some(pressed);
        self.observed = Some(now);
        if !pressed {
            self.blocked = false;
        }
    }
    pub(crate) fn inhibit(&mut self) {
        self.blocked = self.raw == Some(true);
    }
    pub(crate) fn tick(&mut self, now: Instant) -> Option<bool> {
        let fresh = self
            .observed
            .is_some_and(|at| now.saturating_duration_since(at) <= INPUT_DEADLINE);
        if !fresh {
            self.inhibit();
        }
        let down = self.raw == Some(true) && fresh && !self.blocked;
        if self.emitted && !down {
            self.emitted = false;
            return Some(false);
        }
        if !self.emitted
            && down
            && self
                .changed
                .is_some_and(|at| now.saturating_duration_since(at) >= PRESS_DEBOUNCE)
        {
            self.emitted = true;
            return Some(true);
        }
        None
    }
}

#[derive(Default)]
pub(crate) struct HardwarePttRouting {
    pub(crate) input: HardwarePttInput,
    pub(crate) next: u64,
    pub(crate) owner: u64,
    pub(crate) lease: Option<HardwarePttPermit>,
    pub(crate) last_poll: Option<Instant>,
}

impl super::TciFrontend {
    pub(crate) fn hardware_ptt_poll_due(&self, now: Instant) -> bool {
        let mut p = self.hardware_ptt.lock().unwrap_or_else(|e| e.into_inner());
        if p.last_poll
            .is_some_and(|at| now.saturating_duration_since(at) < PTT_POLL_INTERVAL)
        {
            return false;
        }
        p.last_poll = Some(now);
        true
    }
    pub(crate) fn observe_hardware_ptt(&self, pressed: bool, now: Instant) {
        self.hardware_ptt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .input
            .observe(pressed, now);
        self.service_hardware_ptt(now);
    }
    pub(crate) fn service_hardware_ptt(&self, now: Instant) {
        use super::TciCommandSink;
        let owner = self.operator_client_id.load(Ordering::SeqCst);
        let mut p = self.hardware_ptt.lock().unwrap_or_else(|e| e.into_inner());
        if p.lease.is_some() && p.owner != owner {
            p.input.inhibit();
        }
        let Some(down) = p.input.tick(now) else {
            return;
        };
        if down {
            if owner == 0 {
                return;
            }
            let Some(id) = p.next.checked_add(1) else {
                return;
            };
            p.next = id;
            #[cfg(not(feature = "offline-puresignal-owner-adapters"))]
            let context = None;
            #[cfg(feature = "offline-puresignal-owner-adapters")]
            let context = if let Some(shared) = self
                .shared_tx
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                let Some(c) = shared.capture(now) else {
                    return;
                };
                if !shared.host_readiness().1 {
                    return;
                }
                Some(c.hardware_ptt_context())
            } else {
                None
            };
            let lease = HardwarePttPermit::new(id, context)
                .for_operator(self.operator_client_id.clone(), owner);
            let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
            let Some(client) = clients.get_mut(&owner) else {
                return;
            };
            client.state.hardware_ptt = Some(lease.clone());
            p.owner = owner;
            p.lease = Some(lease);
            drop(clients);
            super::send_safety_text_to_client_or_control(
                &self.clients,
                owner,
                format!("saturn_ptt:0,true,{id};"),
            );
        } else if let Some(lease) = p.lease.take() {
            let claimed = lease.cancel();
            if claimed {
                // Immediate shared fence closure; Legacy receives the existing
                // priority release, which also cancels queued arms and audio.
                #[cfg(feature = "offline-puresignal-owner-adapters")]
                if let Some(shared) = self
                    .shared_tx
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                {
                    shared.release();
                }
                let _ = self
                    .hardware_commands
                    .send(super::TciCommand::SetTxEnabled(false));
            }
            super::send_safety_text_to_client_or_control(
                &self.clients,
                p.owner,
                format!("saturn_ptt:0,false,{};", lease.id),
            );
            p.owner = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_startup_needs_release_then_a_debounced_fresh_press() {
        let t = Instant::now();
        let mut p = HardwarePttInput::default();
        p.observe(true, t);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE), None);
        p.observe(false, t + PRESS_DEBOUNCE);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE), None);
        p.observe(true, t + PRESS_DEBOUNCE);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE), None);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE * 2), Some(true));
        p.observe(false, t + PRESS_DEBOUNCE * 2);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE * 2), Some(false));
    }
    #[test]
    fn missing_status_and_operator_change_cannot_rekey_a_held_switch() {
        let t = Instant::now();
        let mut p = HardwarePttInput::default();
        p.observe(false, t);
        p.observe(true, t);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE), Some(true));
        assert_eq!(p.tick(t + INPUT_DEADLINE + PRESS_DEBOUNCE), Some(false));
        p.observe(true, t + INPUT_DEADLINE + PRESS_DEBOUNCE);
        assert_eq!(p.tick(t + INPUT_DEADLINE + PRESS_DEBOUNCE * 2), None);
        let next = t + INPUT_DEADLINE + PRESS_DEBOUNCE * 3;
        p.observe(false, next);
        p.observe(true, next);
        assert_eq!(p.tick(next + PRESS_DEBOUNCE), Some(true));
        p.inhibit();
        assert_eq!(p.tick(next + PRESS_DEBOUNCE), Some(false));
        assert_eq!(p.tick(next + PRESS_DEBOUNCE), None);
    }
    #[test]
    fn short_bounce_never_emits_a_press_and_release_never_waits() {
        let t = Instant::now();
        let mut p = HardwarePttInput::default();
        p.observe(false, t);
        p.observe(true, t);
        p.observe(false, t + Duration::from_millis(2));
        assert_eq!(p.tick(t + PRESS_DEBOUNCE), None);
        p.observe(true, t + PRESS_DEBOUNCE);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE * 2), Some(true));
        p.observe(false, t + PRESS_DEBOUNCE * 2);
        assert_eq!(p.tick(t + PRESS_DEBOUNCE * 2), Some(false));
    }
    #[test]
    fn permit_is_single_use_and_cancellation_reaches_queued_clones() {
        let p = HardwarePttPermit::new(1, Some((2, 3)));
        let queued = p.clone();
        assert!(p.claim());
        assert!(!p.claim());
        assert!(p.cancel());
        assert!(!queued.live());
        assert!(!queued.claim());
        assert!(!p.cancel());
    }
}
