use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TxMicCodec {
    Pcm,
    OpusNb,
    OpusWb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxDecodeError {
    CodecDisabled,
    UnsupportedCodec,
    UnsupportedSampleType,
    PayloadSizeMismatch,
    PayloadTooShort,
}

pub const TX_MIC_CODEC_PCM_ID: u32 = 0;
pub const TX_MIC_CODEC_OPUS_NB_ID: u32 = 1;
pub const TX_MIC_CODEC_OPUS_WB_ID: u32 = 2;

pub const TX_SAMPLE_TYPE_LEGACY_FLOAT32: u32 = 0;
pub const TX_SAMPLE_TYPE_S16: u32 = 1;
pub const TX_SAMPLE_TYPE_FLOAT32: u32 = 3;
pub const TX_CODEC_STALE_FRAME_MAX_AGE: Duration = Duration::from_millis(150);
pub const TX_OPUS_FRAME_MS: u32 = 20;
pub const TX_OPUS_NB_SAMPLE_RATE_HZ: u32 = 16_000;
pub const TX_OPUS_WB_SAMPLE_RATE_HZ: u32 = 48_000;
pub const TX_OPUS_NB_BITRATE_BPS: u32 = 16_000;
pub const TX_OPUS_WB_BITRATE_BPS: u32 = 24_000;

impl TxMicCodec {
    pub fn from_tci(value: &str) -> Option<Self> {
        match value
            .trim()
            .trim_end_matches(';')
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str()
        {
            "0" | "pcm" | "s16" => Some(Self::Pcm),
            "1" | "opus_nb" | "opus_narrowband" => Some(Self::OpusNb),
            "2" | "opus_wb" | "opus_wideband" => Some(Self::OpusWb),
            _ => None,
        }
    }

    pub fn from_id(value: u32) -> Option<Self> {
        match value {
            TX_MIC_CODEC_PCM_ID => Some(Self::Pcm),
            TX_MIC_CODEC_OPUS_NB_ID => Some(Self::OpusNb),
            TX_MIC_CODEC_OPUS_WB_ID => Some(Self::OpusWb),
            _ => None,
        }
    }

    pub fn as_tci(self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::OpusNb => "opus_nb",
            Self::OpusWb => "opus_wb",
        }
    }
}

pub fn tx_codec_frame_is_stale(received_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(received_at) > TX_CODEC_STALE_FRAME_MAX_AGE
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxCodecRuntimeFlags {
    pub opus_decode_enabled: bool,
}

impl Default for TxCodecRuntimeFlags {
    fn default() -> Self {
        Self {
            opus_decode_enabled: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxOpusDecoderConfig {
    pub codec: TxMicCodec,
    pub sample_rate_hz: u32,
    pub bitrate_bps: u32,
    pub frame_ms: u32,
    pub frame_samples: usize,
    pub fec_enabled: bool,
    pub dtx_enabled: bool,
}

impl TxOpusDecoderConfig {
    pub fn for_codec(codec: TxMicCodec) -> Option<Self> {
        match codec {
            TxMicCodec::OpusNb => Some(Self {
                codec,
                sample_rate_hz: TX_OPUS_NB_SAMPLE_RATE_HZ,
                bitrate_bps: TX_OPUS_NB_BITRATE_BPS,
                frame_ms: TX_OPUS_FRAME_MS,
                frame_samples: (TX_OPUS_NB_SAMPLE_RATE_HZ as usize * TX_OPUS_FRAME_MS as usize)
                    / 1000,
                fec_enabled: true,
                dtx_enabled: false,
            }),
            TxMicCodec::OpusWb => Some(Self {
                codec,
                sample_rate_hz: TX_OPUS_WB_SAMPLE_RATE_HZ,
                bitrate_bps: TX_OPUS_WB_BITRATE_BPS,
                frame_ms: TX_OPUS_FRAME_MS,
                frame_samples: (TX_OPUS_WB_SAMPLE_RATE_HZ as usize * TX_OPUS_FRAME_MS as usize)
                    / 1000,
                fec_enabled: true,
                dtx_enabled: false,
            }),
            TxMicCodec::Pcm => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TxOpusDecoder;

impl TxOpusDecoder {
    pub fn new(
        _config: TxOpusDecoderConfig,
        flags: TxCodecRuntimeFlags,
    ) -> Result<Self, TxDecodeError> {
        if !flags.opus_decode_enabled {
            return Err(TxDecodeError::CodecDisabled);
        }
        // The real libopus backend is intentionally not wired in this
        // scaffold. Keeping construction gated prevents accidental TX payload
        // changes before force-RX/fallback acceptance tests exist.
        Err(TxDecodeError::UnsupportedCodec)
    }
}

#[derive(Clone, Debug)]
pub struct TxCodecDecoder {
    codec: TxMicCodec,
    flags: TxCodecRuntimeFlags,
}

impl TxCodecDecoder {
    pub fn new(codec: TxMicCodec) -> Self {
        Self::new_with_flags(codec, TxCodecRuntimeFlags::default())
    }

    pub fn new_with_flags(codec: TxMicCodec, flags: TxCodecRuntimeFlags) -> Self {
        Self { codec, flags }
    }

    pub fn decode(
        &self,
        sample_type: u32,
        sample_count: usize,
        payload: &[u8],
        declared_payload_bytes: usize,
    ) -> Result<Vec<f32>, TxDecodeError> {
        match self.codec {
            TxMicCodec::Pcm => {
                decode_pcm_payload(sample_type, sample_count, payload, declared_payload_bytes)
            }
            TxMicCodec::OpusNb | TxMicCodec::OpusWb => {
                let Some(config) = TxOpusDecoderConfig::for_codec(self.codec) else {
                    return Err(TxDecodeError::UnsupportedCodec);
                };
                let _ = (sample_type, sample_count, payload, declared_payload_bytes);
                TxOpusDecoder::new(config, self.flags)
                    .and_then(|_| Err(TxDecodeError::UnsupportedCodec))
            }
        }
    }
}

fn expected_pcm_payload_bytes(
    sample_type: u32,
    sample_count: usize,
) -> Result<usize, TxDecodeError> {
    match sample_type {
        TX_SAMPLE_TYPE_S16 => Ok(sample_count * 2),
        TX_SAMPLE_TYPE_LEGACY_FLOAT32 | TX_SAMPLE_TYPE_FLOAT32 => Ok(sample_count * 4),
        _ => Err(TxDecodeError::UnsupportedSampleType),
    }
}

fn decode_pcm_payload(
    sample_type: u32,
    sample_count: usize,
    payload: &[u8],
    declared_payload_bytes: usize,
) -> Result<Vec<f32>, TxDecodeError> {
    let expected_payload_bytes = expected_pcm_payload_bytes(sample_type, sample_count)?;
    if declared_payload_bytes != 0 && declared_payload_bytes != expected_payload_bytes {
        return Err(TxDecodeError::PayloadSizeMismatch);
    }
    if payload.len() < expected_payload_bytes {
        return Err(TxDecodeError::PayloadTooShort);
    }

    let mut samples = Vec::with_capacity(sample_count);
    match sample_type {
        TX_SAMPLE_TYPE_S16 => {
            for i in 0..sample_count {
                let bytes: [u8; 2] = payload[i * 2..i * 2 + 2]
                    .try_into()
                    .map_err(|_| TxDecodeError::PayloadTooShort)?;
                samples.push(i16::from_le_bytes(bytes) as f32 / 32768.0);
            }
        }
        TX_SAMPLE_TYPE_LEGACY_FLOAT32 | TX_SAMPLE_TYPE_FLOAT32 => {
            for i in 0..sample_count {
                let bytes: [u8; 4] = payload[i * 4..i * 4 + 4]
                    .try_into()
                    .map_err(|_| TxDecodeError::PayloadTooShort)?;
                samples.push(f32::from_le_bytes(bytes));
            }
        }
        _ => return Err(TxDecodeError::UnsupportedSampleType),
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codec_names_and_ids() {
        assert_eq!(TxMicCodec::from_tci("pcm"), Some(TxMicCodec::Pcm));
        assert_eq!(TxMicCodec::from_tci("opus-wb;"), Some(TxMicCodec::OpusWb));
        assert_eq!(TxMicCodec::from_id(2), Some(TxMicCodec::OpusWb));
        assert_eq!(TxMicCodec::OpusNb.as_tci(), "opus_nb");
        assert_eq!(TxMicCodec::from_tci("bad"), None);
        assert_eq!(TxMicCodec::from_id(99), None);
    }

    #[test]
    fn pcm_decoder_passes_through_s16_samples() {
        let payload = [0x00, 0x20, 0x00, 0xc0, 0xff, 0x7f];
        let samples = TxCodecDecoder::new(TxMicCodec::Pcm)
            .decode(TX_SAMPLE_TYPE_S16, 3, &payload, payload.len())
            .unwrap();
        assert_eq!(samples, vec![0.25, -0.5, 32767.0 / 32768.0]);
    }

    #[test]
    fn pcm_decoder_passes_through_float32_samples() {
        let payload = [0.25f32.to_le_bytes(), (-0.5f32).to_le_bytes()].concat();
        let samples = TxCodecDecoder::new(TxMicCodec::Pcm)
            .decode(TX_SAMPLE_TYPE_FLOAT32, 2, &payload, payload.len())
            .unwrap();
        assert_eq!(samples, vec![0.25, -0.5]);
    }

    #[test]
    fn pcm_decoder_rejects_bad_payload_lengths() {
        assert_eq!(
            TxCodecDecoder::new(TxMicCodec::Pcm).decode(TX_SAMPLE_TYPE_S16, 3, &[0; 6], 4),
            Err(TxDecodeError::PayloadSizeMismatch)
        );
        assert_eq!(
            TxCodecDecoder::new(TxMicCodec::Pcm).decode(TX_SAMPLE_TYPE_S16, 3, &[0; 4], 0),
            Err(TxDecodeError::PayloadTooShort)
        );
    }

    #[test]
    fn opus_decoder_is_not_enabled_yet() {
        assert_eq!(
            TxCodecDecoder::new(TxMicCodec::OpusWb).decode(TX_SAMPLE_TYPE_S16, 3, &[0; 6], 6),
            Err(TxDecodeError::CodecDisabled)
        );
    }

    #[test]
    fn opus_profiles_match_phase44_defaults() {
        let nb = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusNb).unwrap();
        assert_eq!(nb.sample_rate_hz, 16_000);
        assert_eq!(nb.bitrate_bps, 16_000);
        assert_eq!(nb.frame_ms, 20);
        assert_eq!(nb.frame_samples, 320);
        assert!(nb.fec_enabled);
        assert!(!nb.dtx_enabled);

        let wb = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusWb).unwrap();
        assert_eq!(wb.sample_rate_hz, 48_000);
        assert_eq!(wb.bitrate_bps, 24_000);
        assert_eq!(wb.frame_samples, 960);
        assert!(TxOpusDecoderConfig::for_codec(TxMicCodec::Pcm).is_none());
    }

    #[test]
    fn opus_decoder_backend_is_not_wired_even_when_flagged_on() {
        let config = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusWb).unwrap();
        assert_eq!(
            TxOpusDecoder::new(
                config,
                TxCodecRuntimeFlags {
                    opus_decode_enabled: true
                }
            )
            .unwrap_err(),
            TxDecodeError::UnsupportedCodec
        );
    }

    #[test]
    fn classifies_stale_decoded_frames_by_consume_age() {
        let received_at = Instant::now();
        assert!(!tx_codec_frame_is_stale(
            received_at,
            received_at + Duration::from_millis(150)
        ));
        assert!(tx_codec_frame_is_stale(
            received_at,
            received_at + Duration::from_millis(151)
        ));
    }
}
