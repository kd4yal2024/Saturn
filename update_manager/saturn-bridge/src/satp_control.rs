//! Operator-owned SATP audio lease. Secrets never enter snapshots or status files.
use crate::tx_audio::TxAudioSource;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    io::{self, Read},
    net::SocketAddr,
    time::{Duration, Instant},
};

const LEASE_TIME: Duration = Duration::from_secs(5);
#[derive(Clone)]
struct Lease {
    owner: u64,
    key: [u8; 32],
    expires: Instant,
    stream: Option<(SocketAddr, u32, u32)>,
    counter: Option<u64>,
    replay_high: Option<u64>,
    replay_bits: u64,
    audio_floor: u64,
}

#[derive(Clone)]
pub struct SatpControl {
    pub enabled: bool,
    pub source: TxAudioSource,
    pub generation: u64,
    pub pending_source: bool,
    pub last_progress: Option<Instant>,
    pub peak_db: f32,
    pub buffer_frames: u32,
    pub missing_packets: u64,
    pub loss_timeout: Duration,
    lease: Option<Lease>,
}
impl std::fmt::Debug for SatpControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SatpControl")
            .field("source", &self.source)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl Default for SatpControl {
    fn default() -> Self {
        Self {
            enabled: false,
            source: TxAudioSource::Tci,
            generation: 0,
            pending_source: false,
            last_progress: None,
            peak_db: -120.0,
            buffer_frames: 0,
            missing_packets: 0,
            loss_timeout: Duration::from_millis(250),
            lease: None,
        }
    }
}
impl SatpControl {
    pub fn pair(&mut self, owner: u64, now: Instant) -> io::Result<String> {
        let mut key = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
        self.install(owner, key, now);
        Ok(key.iter().map(|b| format!("{b:02x}")).collect())
    }
    fn install(&mut self, owner: u64, key: [u8; 32], now: Instant) {
        self.revoke();
        self.lease = Some(Lease {
            owner,
            key,
            expires: now + LEASE_TIME,
            stream: None,
            counter: None,
            replay_high: None,
            replay_bits: 0,
            audio_floor: 0,
        });
    }
    pub fn revoke(&mut self) {
        self.lease = None;
        self.last_progress = None;
        self.peak_db = -120.0;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn revoke_owner(&mut self, owner: u64) {
        if self.lease.as_ref().is_some_and(|l| l.owner == owner) {
            self.revoke();
        }
    }
    pub fn paired(&self, owner: u64, now: Instant) -> bool {
        self.lease
            .as_ref()
            .is_some_and(|l| l.owner == owner && now < l.expires)
    }
    pub fn live(&self, now: Instant) -> bool {
        self.lease.as_ref().is_some_and(|l| now < l.expires)
    }
    pub fn renew(&mut self, owner: u64, now: Instant) -> bool {
        if !self.paired(owner, now) {
            return false;
        }
        self.lease.as_mut().unwrap().expires = now + LEASE_TIME;
        true
    }
    pub fn ready(&self, now: Instant) -> bool {
        self.enabled
            && self.live(now)
            && self
                .last_progress
                .is_some_and(|at| now.saturating_duration_since(at) < self.loss_timeout)
    }
    pub fn permits_tx(&self, generated: bool, now: Instant) -> bool {
        !self.pending_source && (generated || self.source == TxAudioSource::Tci || self.ready(now))
    }
    pub fn can_configure(&self, phase: crate::radio_model::TxPhase, tx_enabled: bool) -> bool {
        phase == crate::radio_model::TxPhase::Rx && !tx_enabled && !self.pending_source
    }
    /// Verify the full header and payload before accepting any session/timeline changes.
    /// Reordered packets may fill the jitter buffer but must not refresh liveness.
    pub fn authenticate(&mut self, bytes: &[u8], from: SocketAddr, now: Instant) -> bool {
        if bytes.len() != 576 || &bytes[..4] != b"SAT1" || bytes[4] != 2 {
            return false;
        }
        let Some(lease) = self.lease.as_mut().filter(|l| now < l.expires) else {
            return false;
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&lease.key).expect("HMAC key length");
        mac.update(&bytes[..544]);
        if mac.verify_slice(&bytes[544..]).is_err() {
            return false;
        }
        let session = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let stream = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let identity = (from, session, stream);
        if lease.stream.is_some_and(|bound| bound != identity) {
            return false;
        }
        lease.stream = Some(identity);
        let counter = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
        if counter < lease.audio_floor {
            return false;
        }
        match lease.replay_high {
            None => {
                lease.replay_high = Some(counter);
                lease.replay_bits = 1;
            }
            Some(high) if counter > high => {
                let advance = (counter - high) / 128;
                if advance == 0 {
                    return false;
                }
                lease.replay_bits = if advance >= 64 {
                    1
                } else {
                    (lease.replay_bits << advance) | 1
                };
                lease.replay_high = Some(counter);
            }
            Some(high) => {
                let behind = (high - counter) / 128;
                if behind >= 64 || lease.replay_bits & (1u64 << behind) != 0 {
                    return false;
                }
                lease.replay_bits |= 1u64 << behind;
            }
        }
        true
    }
    pub fn begin_tx_epoch(&mut self) {
        if let Some(lease) = self.lease.as_mut() {
            lease.audio_floor = lease
                .counter
                .map_or(0, |counter| counter.saturating_add(128));
        }
    }
    pub fn progress(&mut self, counter: u64, peak: f32, now: Instant) {
        if let Some(lease) = self.lease.as_mut().filter(|l| now < l.expires) {
            if lease.counter.is_none_or(|previous| counter > previous) {
                lease.counter = Some(counter);
                self.last_progress = Some(now);
                self.peak_db = 20.0 * peak.max(0.000001).log10();
            }
        }
    }
    pub fn message(&self, owner: u64, now: Instant) -> String {
        let age = self
            .last_progress
            .map(|at| now.saturating_duration_since(at).as_millis() as u64)
            .unwrap_or(u64::MAX);
        let health = if !self.live(now) {
            "unpaired"
        } else if self.last_progress.is_none() {
            "waiting"
        } else if !self.ready(now) {
            "lost"
        } else if age > 50 {
            "degraded"
        } else {
            "healthy"
        };
        format!(
            "saturn_satp_state:{},{},{},{},{},{:.1},{},{},{},{};",
            self.source.as_str(),
            self.enabled,
            self.paired(owner, now),
            self.ready(now),
            health,
            self.peak_db,
            self.buffer_frames,
            self.missing_packets,
            self.generation,
            self.pending_source
        )
    }
}

/// Called by either backend with the model locked; source changes are acknowledged
/// only by the TX worker after it has flushed/reconfigured the idle pipeline.
pub fn handle(
    action: &str,
    owner: u64,
    model: &mut crate::radio_model::RadioModel,
    tci: &crate::tci::TciFrontend,
    tx: &std::sync::mpsc::Sender<crate::tx_thread::TxCommand>,
) {
    if !tci.is_operator(owner) {
        return;
    }
    let now = Instant::now();
    let error = if action == "renew" {
        if model.satp.renew(owner, now) {
            None
        } else {
            Some("lease_expired")
        }
    } else if !model
        .satp
        .can_configure(model.desired.tx_phase, model.desired.tx_enabled)
    {
        Some("rx_required")
    } else if action == "pair" {
        if !model.satp.enabled {
            Some("receiver_disabled")
        } else {
            match model.satp.pair(owner, now) {
                Ok(key) => {
                    tci.publish_satp_reply(owner, format!("saturn_satp_key:{key};"));
                    None
                }
                Err(_) => Some("pair_failed"),
            }
        }
    } else if let Some(source) = TxAudioSource::parse(action) {
        if source == TxAudioSource::Satp
            && (!model.satp.paired(owner, now) || !model.satp.ready(now))
        {
            Some("native_audio_not_ready")
        } else {
            model.satp.pending_source = true;
            if tx
                .send(crate::tx_thread::TxCommand::SelectAudioSource(source))
                .is_err()
            {
                model.satp.pending_source = false;
                Some("tx_worker_unavailable")
            } else {
                None
            }
        }
    } else {
        Some("invalid_command")
    };
    if let Some(error) = error {
        tci.publish_satp_reply(owner, format!("saturn_satp_error:{error};"));
    }
    tci.publish_satp_reply(owner, model.satp.message(owner, now));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signed(counter: u64, key: &[u8; 32]) -> Vec<u8> {
        let mut b = vec![0u8; 544];
        b[..4].copy_from_slice(b"SAT1");
        b[4] = 2;
        b[20..28].copy_from_slice(&counter.to_le_bytes());
        let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
        m.update(&b);
        b.extend_from_slice(&m.finalize().into_bytes());
        b
    }
    #[test]
    fn authentication_binds_sender_and_rejects_tampering() {
        let now = Instant::now();
        let mut c = SatpControl::default();
        c.install(1, [7; 32], now);
        let addr = "127.0.0.1:1234".parse().unwrap();
        let b = signed(128, &[7; 32]);
        assert!(c.authenticate(&b, addr, now));
        assert!(!c.authenticate(&b, "127.0.0.1:1235".parse().unwrap(), now));
        let mut bad = b.clone();
        bad[40] ^= 1;
        assert!(!c.authenticate(&bad, addr, now));
        assert!(!c.authenticate(&signed(128, &[8; 32]), addr, now));
        assert!(!c.authenticate(&b[..544], addr, now));
    }
    #[test]
    fn only_owner_renews_and_replay_cannot_keep_audio_alive() {
        let now = Instant::now();
        let mut c = SatpControl::default();
        c.enabled = true;
        c.install(1, [7; 32], now);
        c.progress(128, 0.5, now);
        assert!(c.ready(now));
        assert!(!c.renew(2, now));
        c.progress(128, 0.5, now + Duration::from_secs(1));
        assert!(!c.ready(now + Duration::from_secs(1)));
        assert!(!c.renew(1, now + LEASE_TIME));
        assert!(!c.live(now + LEASE_TIME));
        c.revoke_owner(1);
        assert!(!c.live(now));
    }
    #[test]
    fn source_switch_and_tx_admission_fail_closed() {
        use crate::radio_model::TxPhase;
        let now = Instant::now();
        let mut c = SatpControl::default();
        assert!(c.can_configure(TxPhase::Rx, false));
        assert!(!c.can_configure(TxPhase::Armed, false));
        assert!(!c.can_configure(TxPhase::Keyed, true));
        assert!(c.permits_tx(false, now));
        c.source = TxAudioSource::Satp;
        assert!(!c.permits_tx(false, now));
        c.enabled = true;
        c.install(1, [7; 32], now);
        assert!(!c.permits_tx(false, now));
        c.progress(0, 0.25, now);
        assert!(c.permits_tx(false, now));
        c.pending_source = true;
        assert!(!c.permits_tx(false, now));
        assert!(!c.permits_tx(true, now));
        c.pending_source = false;
        c.revoke_owner(2);
        assert!(c.permits_tx(false, now));
        c.revoke_owner(1);
        assert!(!c.permits_tx(false, now));
    }
    #[test]
    fn re_pairing_rejects_old_credentials_and_session_takeover() {
        let now = Instant::now();
        let mut c = SatpControl::default();
        c.install(1, [7; 32], now);
        let addr = "127.0.0.1:1234".parse().unwrap();
        let b = signed(128, &[7; 32]);
        assert!(c.authenticate(&b, addr, now));
        let mut takeover = signed(256, &[7; 32]);
        takeover[8] = 1;
        let mut mac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
        mac.update(&takeover[..544]);
        takeover[544..].copy_from_slice(&mac.finalize().into_bytes());
        assert!(!c.authenticate(&takeover, addr, now));
        c.install(1, [8; 32], now);
        assert!(!c.authenticate(&b, addr, now));
        assert!(c.authenticate(&signed(0, &[8; 32]), addr, now));
        assert!(!format!("{c:?}").contains("key"));
        assert!(!c.message(2, now).contains(&"08".repeat(32)));
    }

    #[test]
    fn bounded_reordering_is_allowed_once_and_ptt_discards_pre_arm_audio() {
        let now = Instant::now();
        let mut c = SatpControl::default();
        c.install(1, [7; 32], now);
        let addr = "127.0.0.1:1234".parse().unwrap();
        assert!(c.authenticate(&signed(256, &[7; 32]), addr, now));
        c.progress(256, 0.25, now);
        assert!(c.authenticate(&signed(128, &[7; 32]), addr, now));
        assert!(!c.authenticate(&signed(128, &[7; 32]), addr, now));
        c.begin_tx_epoch();
        assert!(!c.authenticate(&signed(0, &[7; 32]), addr, now));
        assert!(c.authenticate(&signed(384, &[7; 32]), addr, now));
    }
}
