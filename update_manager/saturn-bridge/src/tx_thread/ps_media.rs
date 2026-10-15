//! Explicit offline remote-media contract; never selected by installed startup.
//! PSG1 tags precede decoding. Opus banks are separate encoder/decoder streams,
//! not a claimed finite memory horizon for an ordinary continuous Opus stream.
use crate::tci::parse_tci_mic_frame_parts;
use crate::{
    sync_ext::MutexExt,
    tx_codec::{TxCodecDecoder, TxCodecRuntimeFlags, TxMicCodec},
    tx_thread::ps_owner_loop::IngressStamp,
};
use hmac::{Hmac, Mac};
use saturn_bridge::puresignal::Fault;
use sha2::Sha256;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

pub(crate) const MAGIC: u32 = 0x50534731;
pub(crate) const BANK_PACKETS: u32 = 4;
const MAX_AGE: Duration = Duration::from_millis(150);
const CAPS: &str = "saturn_tx_media_caps:psg1,pcm,opus_wb,bank4;";
#[derive(Clone, Copy)]
struct Offer {
    token: [u8; 16],
    stamp: IngressStamp,
}
impl Offer {
    fn matches(self, s: IngressStamp) -> bool {
        self.stamp.output == s.output
            && s.output.is_some()
            && self.stamp.revision == s.revision
            && self.stamp.release == s.release
            && self.stamp.fence.boot == s.fence.boot
            && self.stamp.fence.config == s.fence.config
        // Acquisition/feedback epochs advance at key receipt; the output
        // generation does not. They remain on each queued Audio stamp but
        // must not rotate the browser token/reset its sequence at key receipt.
    }
}
struct Publication {
    offer: Option<Offer>,
    next: u64,
}
#[derive(Default)]
struct History {
    token: Option<[u8; 16]>,
    client: u64,
    next: u32,
    codec: Option<TxMicCodec>,
    decoder: Option<TxCodecDecoder>,
    floor: Option<Instant>,
}
pub(crate) struct RemoteMedia {
    enabled: AtomicBool,
    pcm_only: AtomicBool,
    secret: Option<[u8; 32]>,
    publication: Mutex<Publication>,
    history: Mutex<History>,
}
impl Default for RemoteMedia {
    fn default() -> Self {
        let mut secret = [0; 32];
        let secret = std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut secret))
            .ok()
            .map(|_| secret);
        Self {
            enabled: AtomicBool::new(false),
            pcm_only: AtomicBool::new(false),
            secret,
            publication: Mutex::new(Publication {
                offer: None,
                next: 0,
            }),
            history: Mutex::new(History::default()),
        }
    }
}
impl RemoteMedia {
    pub(crate) fn enable_tagged_pcm_review(&self) -> Result<(), Fault> {
        if self.enabled() {
            return Err(Fault::InvalidConfig);
        }
        self.pcm_only.store(true, Ordering::SeqCst);
        self.enable_offline_review()
    }
    pub(crate) fn pcm_only(&self) -> bool {
        self.pcm_only.load(Ordering::SeqCst)
    }
    pub(crate) fn enable_offline_review(&self) -> Result<(), Fault> {
        if self.secret.is_none() {
            return Err(Fault::NativeUnverified);
        }
        self.enabled
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Fault::InvalidConfig)?;
        Ok(())
    }
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub(crate) fn caps(&self) -> &'static str {
        if self.pcm_only() {
            "saturn_tx_media_caps:psg1,pcm,owner1,transactions1;"
        } else {
            CAPS
        }
    }
    /// Owner publication never takes the decoder/history lock. RF cancellation
    /// likewise does not wait for decoding, library creation, or a socket write.
    pub(crate) fn publish(
        &self,
        stamp: IngressStamp,
        tci_active: bool,
    ) -> Result<Option<String>, Fault> {
        if !self.enabled() {
            return Ok(None);
        }
        let mut p = self.publication.lock_unpoisoned();
        if !tci_active || stamp.output.is_none() {
            let serial = p.next;
            return Ok(p
                .offer
                .take()
                .map(|_| format!("saturn_tx_generation:psg1,off,{serial};")));
        }
        if p.offer.is_some_and(|o| o.matches(stamp)) {
            return Ok(None);
        }
        p.next = p.next.checked_add(1).ok_or(Fault::InvalidConfig)?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.secret.as_ref().ok_or(Fault::NativeUnverified)?)
                .map_err(|_| Fault::NativeUnverified)?;
        mac.update(&p.next.to_le_bytes());
        let digest = mac.finalize().into_bytes();
        let token: [u8; 16] = digest[..16].try_into().unwrap();
        p.offer = Some(Offer { token, stamp });
        Ok(Some(format!(
            "saturn_tx_generation:psg1,{},{};",
            hex(token),
            p.next
        )))
    }
    pub(crate) fn token(&self, stamp: IngressStamp) -> Option<[u8; 16]> {
        self.publication
            .lock_unpoisoned()
            .offer
            .filter(|o| o.matches(stamp))
            .map(|o| o.token)
    }
    pub(crate) fn decode(
        &self,
        stamp: IngressStamp,
        client: u64,
        arrived: Instant,
        data: &[u8],
        now: Instant,
    ) -> Result<(Vec<f32>, Instant), Fault> {
        let expected = self.token(stamp).ok_or(Fault::GenerationMismatch)?;
        if data.len() < 64
            || u32::from_le_bytes(data[60..64].try_into().unwrap()) != MAGIC
            || data[44..60] != expected
        {
            return Err(Fault::GenerationMismatch);
        }
        // Decode is never invoked for wrong generations, malformed bounds,
        // invalid formats or discontinuities; no automatic PCM fallback/PLC.
        let parts = parse_tci_mic_frame_parts(data).map_err(|_| Fault::WrongFormat)?;
        if self.pcm_only() && parts.codec != TxMicCodec::Pcm {
            return Err(Fault::WrongFormat);
        }
        if u32::from_le_bytes(data[..4].try_into().unwrap()) != 0 {
            return Err(Fault::WrongRoute);
        }
        if parts.sample_rate_hz != 48000
            || parts.channels != 1
            || u32::from_le_bytes(data[28..32].try_into().unwrap()) != 1
            || parts.codec == TxMicCodec::OpusNb
            || parts.declared_payload_bytes == 0
            || data.len() != 64 + parts.declared_payload_bytes
            || parts.sample_count > 1024
        {
            return Err(Fault::WrongFormat);
        }
        if parts.codec == TxMicCodec::OpusWb
            && (parts.sample_count != 960 || parts.sample_type != 1)
        {
            return Err(Fault::WrongFormat);
        }
        let mut h = self.history.lock_unpoisoned();
        if h.token != Some(expected) {
            *h = History {
                token: Some(expected),
                client,
                ..History::default()
            };
        }
        if h.client != client {
            return Err(Fault::WrongRoute);
        }
        if h.next != parts.sequence || parts.sequence == u32::MAX {
            return Err(Fault::SequenceGap);
        }
        if h.codec.is_some_and(|c| c != parts.codec) {
            return Err(Fault::WrongFormat);
        }
        if arrived > now {
            return Err(Fault::FutureObservation);
        }
        let opus = parts.codec == TxMicCodec::OpusWb;
        let reset = h.decoder.is_none() || (opus && parts.sequence % BANK_PACKETS == 0);
        let floor = if reset || !opus {
            arrived
        } else {
            h.floor.ok_or(Fault::NativeUnverified)?.min(arrived)
        };
        if now.saturating_duration_since(floor) > MAX_AGE {
            return Err(Fault::Stale);
        }
        if reset {
            h.decoder = Some(TxCodecDecoder::new_with_flags(
                parts.codec,
                TxCodecRuntimeFlags {
                    opus_decode_enabled: true,
                },
            ));
        }
        let samples = h
            .decoder
            .as_mut()
            .ok_or(Fault::NativeUnverified)?
            .decode(
                parts.sample_type,
                parts.sample_count,
                parts.payload,
                parts.declared_payload_bytes,
            )
            .map_err(|_| Fault::WrongFormat)?;
        if samples.iter().any(|s| !s.is_finite()) {
            return Err(Fault::NonFinite);
        }
        // A stop/publication may have raced decoding. Never rebind its output.
        if self.token(stamp) != Some(expected) {
            return Err(Fault::GenerationMismatch);
        }
        h.next = h.next.checked_add(1).ok_or(Fault::SequenceGap)?;
        h.codec = Some(parts.codec);
        h.floor = Some(floor);
        Ok((samples, floor))
    }
}
pub(crate) fn hex(token: [u8; 16]) -> String {
    token.iter().map(|b| format!("{b:02x}")).collect()
}
#[cfg(test)]
pub(crate) fn tag(data: &mut [u8], token: [u8; 16]) {
    data[44..60].copy_from_slice(&token);
    data[60..64].copy_from_slice(&MAGIC.to_le_bytes());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tx_owner_stop::OutputStamp;
    fn stamp() -> IngressStamp {
        IngressStamp {
            fence: saturn_bridge::puresignal::Fence {
                boot: 1,
                generation: 1,
                config: 11,
                tx_epoch: 1,
            },
            revision: 4,
            release: 2,
            output: Some(OutputStamp {
                generation: 3,
                revision: 4,
            }),
        }
    }
    pub(crate) fn frame(token: [u8; 16], seq: u32, opus: bool) -> Vec<u8> {
        let mut data = if opus {
            let payload = crate::tx_codec::TX_OPUS_WB_TEST_PACKET;
            let mut f = vec![0; 64];
            f.extend_from_slice(&payload);
            f[8..12].copy_from_slice(&1u32.to_le_bytes());
            f[20..24].copy_from_slice(&960u32.to_le_bytes());
            f[36..40].copy_from_slice(&2u32.to_le_bytes());
            f
        } else {
            crate::tci::build_tci_audio_frame(0, 48000, 1, &[0.3; 512], seq)
        };
        data[4..8].copy_from_slice(&48000u32.to_le_bytes());
        data[24..28].copy_from_slice(&2u32.to_le_bytes());
        data[28..32].copy_from_slice(&1u32.to_le_bytes());
        data[32..36].copy_from_slice(&seq.to_le_bytes());
        let payload = (data.len() - 64) as u32;
        data[40..44].copy_from_slice(&payload.to_le_bytes());
        tag(&mut data, token);
        data
    }
    fn active() -> (RemoteMedia, IngressStamp, [u8; 16]) {
        let media = RemoteMedia::default();
        assert!(!media.enabled());
        media.enable_offline_review().unwrap();
        let s = stamp();
        media.publish(s, true).unwrap();
        let token = media.token(s).unwrap();
        (media, s, token)
    }
    #[test]
    fn tagged_pcm_profile_advertises_no_banked_opus_and_rejects_it_before_history() {
        let media = RemoteMedia::default();
        media.enable_tagged_pcm_review().unwrap();
        assert_eq!(
            media.caps(),
            "saturn_tx_media_caps:psg1,pcm,owner1,transactions1;"
        );
        assert!(!media.caps().contains("opus"));
        let s = stamp();
        media.publish(s, true).unwrap();
        let token = media.token(s).unwrap();
        let now = Instant::now();
        assert_eq!(
            media
                .decode(s, 7, now, &frame(token, 0, true), now)
                .unwrap_err(),
            Fault::WrongFormat
        );
        assert!(media.history.lock().unwrap().decoder.is_none());
        assert_eq!(
            media
                .decode(s, 7, now, &frame(token, 0, false), now)
                .unwrap()
                .0
                .len(),
            512
        );
    }
    #[test]
    fn media_offers_retire_and_never_reuse_across_rearm_or_restart() {
        let (m, mut s, t) = active();
        assert!(m.enable_offline_review().is_err());
        s.fence.tx_epoch += 1;
        s.fence.generation += 1;
        assert_eq!(m.publish(s, true).unwrap(), None);
        assert_eq!(m.token(s), Some(t));
        m.publish(s, false).unwrap();
        assert_eq!(m.token(s), None);
        s.revision += 1;
        s.release += 1;
        s.output = Some(OutputStamp {
            generation: 4,
            revision: 5,
        });
        m.publish(s, true).unwrap();
        assert_ne!(m.token(s), Some(t));
        assert_ne!(
            active().2,
            t,
            "fresh boot secret does not repeat a generation"
        );
    }
    #[test]
    fn media_old_or_untagged_frames_cannot_mutate_decoder_or_sequence() {
        let (m, s, t) = active();
        let now = Instant::now();
        let mut old = frame(t, 0, true);
        old[44] ^= 1;
        assert_eq!(
            m.decode(s, 7, now, &old, now).unwrap_err(),
            Fault::GenerationMismatch
        );
        old[60..64].fill(0);
        assert_eq!(
            m.decode(s, 7, now, &old, now).unwrap_err(),
            Fault::GenerationMismatch
        );
        assert!(m.history.lock().unwrap().decoder.is_none());
        assert_eq!(
            m.decode(s, 7, now, &frame(t, 0, true), now)
                .unwrap()
                .0
                .len(),
            960
        );
    }
    #[test]
    fn media_opus_history_keeps_oldest_observation_until_explicit_bank_reset() {
        let (m, s, t) = active();
        let start = Instant::now();
        for seq in 0..8 {
            let at = start + Duration::from_millis(20 * seq as u64);
            let (decoded, oldest) = m.decode(s, 7, at, &frame(t, seq, true), at).unwrap();
            assert_eq!(decoded.len(), 960);
            assert_eq!(
                oldest,
                start + Duration::from_millis(20 * (seq / 4 * 4) as u64)
            );
            if seq % 4 == 0 {
                let mut cold = TxCodecDecoder::new_with_flags(
                    TxMicCodec::OpusWb,
                    TxCodecRuntimeFlags {
                        opus_decode_enabled: true,
                    },
                );
                assert_eq!(
                    decoded,
                    cold.decode(1, 960, &crate::tx_codec::TX_OPUS_WB_TEST_PACKET, 60)
                        .unwrap(),
                    "bank really replaces all decoder state, not just its timestamp"
                );
            }
        }
    }
    #[test]
    fn media_fresh_arrival_does_not_hide_stale_decoder_history() {
        let (m, s, t) = active();
        let start = Instant::now();
        m.decode(s, 7, start, &frame(t, 0, true), start).unwrap();
        let later = start + Duration::from_millis(200);
        assert_eq!(
            m.decode(s, 7, later, &frame(t, 1, true), later)
                .unwrap_err(),
            Fault::Stale
        );
        assert_eq!(m.history.lock().unwrap().next, 1);
    }
    #[test]
    fn media_sequence_gap_reorder_wrap_codec_or_lane_switch_cannot_resume_history() {
        let (m, s, t) = active();
        let now = Instant::now();
        m.decode(s, 7, now, &frame(t, 0, true), now).unwrap();
        for seq in [0, 2, u32::MAX] {
            assert_eq!(
                m.decode(s, 7, now, &frame(t, seq, true), now).unwrap_err(),
                Fault::SequenceGap
            );
        }
        assert_eq!(
            m.decode(s, 8, now, &frame(t, 1, true), now).unwrap_err(),
            Fault::WrongRoute
        );
        assert_eq!(
            m.decode(s, 7, now, &frame(t, 1, false), now).unwrap_err(),
            Fault::WrongFormat
        );
        assert_eq!(m.history.lock().unwrap().next, 1);
    }
    #[test]
    fn media_rearm_never_rebinds_retained_opus_history() {
        let (m, mut s, t) = active();
        let old = Instant::now();
        m.decode(s, 7, old, &frame(t, 0, true), old).unwrap();
        m.publish(s, false).unwrap();
        s.release += 1;
        s.revision += 1;
        s.output = Some(OutputStamp {
            generation: 4,
            revision: 5,
        });
        m.publish(s, true).unwrap();
        let fresh = m.token(s).unwrap();
        let now = old + Duration::from_millis(300);
        assert_eq!(
            m.decode(s, 7, now, &frame(t, 1, true), now).unwrap_err(),
            Fault::GenerationMismatch
        );
        assert_eq!(
            m.decode(s, 7, now, &frame(fresh, 0, true), now).unwrap().1,
            now
        );
    }
    #[test]
    fn media_publication_and_retirement_never_wait_for_decoder_lock() {
        let (m, mut s, _) = active();
        let held = m.history.lock().unwrap();
        m.publish(s, false).unwrap();
        s.output = Some(OutputStamp {
            generation: 4,
            revision: 5,
        });
        s.revision += 1;
        m.publish(s, true).unwrap();
        drop(held);
    }
    #[test]
    fn media_malformed_or_future_frames_are_rejected_before_decode() {
        let (m, s, t) = active();
        let now = Instant::now();
        let mut f = frame(t, 0, true);
        f.push(0);
        assert_eq!(
            m.decode(s, 7, now, &f, now).unwrap_err(),
            Fault::WrongFormat
        );
        f.pop();
        assert_eq!(
            m.decode(s, 7, now + Duration::from_millis(1), &f, now)
                .unwrap_err(),
            Fault::FutureObservation
        );
        assert!(m.history.lock().unwrap().decoder.is_none());
    }
}
