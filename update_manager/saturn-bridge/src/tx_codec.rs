#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TxMicCodec {
    Pcm,
    OpusNb,
    OpusWb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxDecodeError {
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

#[derive(Clone, Debug)]
pub struct TxCodecDecoder {
    codec: TxMicCodec,
}

impl TxCodecDecoder {
    pub fn new(codec: TxMicCodec) -> Self {
        Self { codec }
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
            TxMicCodec::OpusNb | TxMicCodec::OpusWb => Err(TxDecodeError::UnsupportedCodec),
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
            Err(TxDecodeError::UnsupportedCodec)
        );
    }
}
