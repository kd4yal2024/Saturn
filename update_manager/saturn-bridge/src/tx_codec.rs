use std::ffi::CString;
use std::fmt;
use std::os::raw::{c_char, c_int, c_uchar, c_void};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TxMicCodec {
    Pcm,
    OpusNb,
    OpusWb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxDecodeError {
    CodecMismatch,
    CodecDisabled,
    OpusBackendUnavailable,
    OpusDecoderCreateFailed,
    OpusDecodeFailed,
    UnsupportedCodec,
    UnsupportedSampleType,
    PayloadSizeMismatch,
    PayloadTooShort,
    SampleCountMismatch,
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
pub const TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;
pub const TX_OPUS_NB_BITRATE_BPS: u32 = 16_000;
pub const TX_OPUS_WB_BITRATE_BPS: u32 = 24_000;
pub const TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES: usize =
    (TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ as usize * TX_OPUS_FRAME_MS as usize) / 1000;

#[cfg(test)]
pub(crate) const TX_OPUS_WB_TEST_PACKET: [u8; 60] = [
    0x78, 0x82, 0x88, 0x8e, 0x19, 0xf8, 0x0e, 0x09, 0x82, 0x91, 0x57, 0x70, 0x44, 0xff, 0x1c, 0x9d,
    0x5f, 0xe8, 0x6a, 0xd3, 0x3a, 0x01, 0x26, 0x5d, 0x01, 0xa0, 0x3a, 0x9f, 0x2e, 0x7c, 0x79, 0xdc,
    0x7b, 0x82, 0xf0, 0x21, 0x95, 0x56, 0xc2, 0xe5, 0xc1, 0xe2, 0x9d, 0x67, 0x7b, 0x9a, 0xc6, 0x24,
    0xf9, 0x83, 0x72, 0xa9, 0x43, 0xc3, 0x54, 0x5e, 0x98, 0xfd, 0xa0, 0x5e,
];

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
    pub encoder_sample_rate_hz: u32,
    pub decode_output_sample_rate_hz: u32,
    pub bitrate_bps: u32,
    pub frame_ms: u32,
    pub encoder_frame_samples: usize,
    pub decode_output_frame_samples: usize,
    pub fec_enabled: bool,
    pub dtx_enabled: bool,
}

impl TxOpusDecoderConfig {
    pub fn for_codec(codec: TxMicCodec) -> Option<Self> {
        match codec {
            TxMicCodec::OpusNb => Some(Self {
                codec,
                encoder_sample_rate_hz: TX_OPUS_NB_SAMPLE_RATE_HZ,
                decode_output_sample_rate_hz: TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
                bitrate_bps: TX_OPUS_NB_BITRATE_BPS,
                frame_ms: TX_OPUS_FRAME_MS,
                encoder_frame_samples: (TX_OPUS_NB_SAMPLE_RATE_HZ as usize
                    * TX_OPUS_FRAME_MS as usize)
                    / 1000,
                decode_output_frame_samples: TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
                fec_enabled: true,
                dtx_enabled: false,
            }),
            TxMicCodec::OpusWb => Some(Self {
                codec,
                encoder_sample_rate_hz: TX_OPUS_WB_SAMPLE_RATE_HZ,
                decode_output_sample_rate_hz: TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
                bitrate_bps: TX_OPUS_WB_BITRATE_BPS,
                frame_ms: TX_OPUS_FRAME_MS,
                encoder_frame_samples: (TX_OPUS_WB_SAMPLE_RATE_HZ as usize
                    * TX_OPUS_FRAME_MS as usize)
                    / 1000,
                decode_output_frame_samples: TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
                fec_enabled: true,
                dtx_enabled: false,
            }),
            TxMicCodec::Pcm => None,
        }
    }
}

struct OpusDynamicLibrary {
    handle: *mut c_void,
    opus_decoder_create:
        unsafe extern "C" fn(fs: i32, channels: c_int, error: *mut c_int) -> *mut c_void,
    opus_decoder_destroy: unsafe extern "C" fn(st: *mut c_void),
    opus_decode_float: unsafe extern "C" fn(
        st: *mut c_void,
        data: *const c_uchar,
        len: i32,
        pcm: *mut f32,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int,
}

unsafe impl Send for OpusDynamicLibrary {}

impl OpusDynamicLibrary {
    fn load() -> Result<Self, TxDecodeError> {
        let path =
            CString::new("libopus.so.0").map_err(|_| TxDecodeError::OpusBackendUnavailable)?;
        let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(TxDecodeError::OpusBackendUnavailable);
        }

        let opus_decoder_create = match unsafe { load_symbol(handle, "opus_decoder_create") } {
            Ok(symbol) => symbol,
            Err(err) => {
                unsafe { dlclose(handle) };
                return Err(err);
            }
        };
        let opus_decoder_destroy = match unsafe { load_symbol(handle, "opus_decoder_destroy") } {
            Ok(symbol) => symbol,
            Err(err) => {
                unsafe { dlclose(handle) };
                return Err(err);
            }
        };
        let opus_decode_float = match unsafe { load_symbol(handle, "opus_decode_float") } {
            Ok(symbol) => symbol,
            Err(err) => {
                unsafe { dlclose(handle) };
                return Err(err);
            }
        };

        Ok(Self {
            handle,
            opus_decoder_create,
            opus_decoder_destroy,
            opus_decode_float,
        })
    }
}

impl Drop for OpusDynamicLibrary {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { dlclose(self.handle) };
        }
    }
}

struct OpusDynamicDecoder {
    library: OpusDynamicLibrary,
    decoder: *mut c_void,
}

unsafe impl Send for OpusDynamicDecoder {}

impl OpusDynamicDecoder {
    fn new(output_sample_rate_hz: u32) -> Result<Self, TxDecodeError> {
        let library = OpusDynamicLibrary::load()?;
        let mut error: c_int = 0;
        let decoder =
            unsafe { (library.opus_decoder_create)(output_sample_rate_hz as i32, 1, &mut error) };
        if decoder.is_null() || error != 0 {
            return Err(TxDecodeError::OpusDecoderCreateFailed);
        }
        Ok(Self { library, decoder })
    }

    fn decode_float(&mut self, payload: &[u8], output: &mut [f32]) -> Result<usize, TxDecodeError> {
        if payload.is_empty() {
            return Err(TxDecodeError::PayloadTooShort);
        }
        let decoded = unsafe {
            (self.library.opus_decode_float)(
                self.decoder,
                payload.as_ptr(),
                payload.len() as i32,
                output.as_mut_ptr(),
                output.len() as c_int,
                0,
            )
        };
        if decoded < 0 {
            return Err(TxDecodeError::OpusDecodeFailed);
        }
        Ok(decoded as usize)
    }
}

impl Drop for OpusDynamicDecoder {
    fn drop(&mut self) {
        if !self.decoder.is_null() {
            unsafe { (self.library.opus_decoder_destroy)(self.decoder) };
        }
    }
}

pub struct TxOpusDecoder {
    config: TxOpusDecoderConfig,
    backend: OpusDynamicDecoder,
}

impl fmt::Debug for TxOpusDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxOpusDecoder")
            .field("config", &self.config)
            .field("backend", &"libopus.so.0")
            .finish()
    }
}

impl TxOpusDecoder {
    pub fn new(
        config: TxOpusDecoderConfig,
        flags: TxCodecRuntimeFlags,
    ) -> Result<Self, TxDecodeError> {
        if !flags.opus_decode_enabled {
            return Err(TxDecodeError::CodecDisabled);
        }
        Ok(Self {
            backend: OpusDynamicDecoder::new(config.decode_output_sample_rate_hz)?,
            config,
        })
    }

    pub fn decode(
        &mut self,
        sample_type: u32,
        sample_count: usize,
        payload: &[u8],
        declared_payload_bytes: usize,
    ) -> Result<Vec<f32>, TxDecodeError> {
        if sample_type != TX_SAMPLE_TYPE_S16 {
            return Err(TxDecodeError::UnsupportedSampleType);
        }
        if sample_count != self.config.decode_output_frame_samples {
            return Err(TxDecodeError::SampleCountMismatch);
        }
        let payload_bytes = if declared_payload_bytes == 0 {
            payload.len()
        } else {
            declared_payload_bytes
        };
        if payload_bytes == 0 {
            return Err(TxDecodeError::PayloadTooShort);
        }
        if payload_bytes > payload.len() {
            return Err(TxDecodeError::PayloadTooShort);
        }

        let mut output = vec![0.0f32; self.config.decode_output_frame_samples];
        let decoded_samples = self
            .backend
            .decode_float(&payload[..payload_bytes], &mut output)?;
        if decoded_samples != self.config.decode_output_frame_samples {
            return Err(TxDecodeError::SampleCountMismatch);
        }
        output.truncate(decoded_samples);
        Ok(output)
    }

    #[cfg(test)]
    fn output_sample_rate_hz(&self) -> u32 {
        self.config.decode_output_sample_rate_hz
    }

    #[cfg(test)]
    fn output_frame_samples(&self) -> usize {
        self.config.decode_output_frame_samples
    }
}

pub struct TxCodecDecoder {
    codec: TxMicCodec,
    flags: TxCodecRuntimeFlags,
    opus: Option<TxOpusDecoder>,
}

impl fmt::Debug for TxCodecDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxCodecDecoder")
            .field("codec", &self.codec)
            .field("flags", &self.flags)
            .field("opus", &self.opus.as_ref().map(|_| "initialized"))
            .finish()
    }
}

impl TxCodecDecoder {
    #[cfg(test)]
    pub fn new(codec: TxMicCodec) -> Self {
        Self::new_with_flags(codec, TxCodecRuntimeFlags::default())
    }

    pub fn new_with_flags(codec: TxMicCodec, flags: TxCodecRuntimeFlags) -> Self {
        Self {
            codec,
            flags,
            opus: None,
        }
    }

    pub fn codec(&self) -> TxMicCodec {
        self.codec
    }

    pub fn decode(
        &mut self,
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
                if self.opus.is_none() {
                    self.opus = Some(TxOpusDecoder::new(config, self.flags)?);
                }
                self.opus
                    .as_mut()
                    .ok_or(TxDecodeError::OpusDecoderCreateFailed)?
                    .decode(sample_type, sample_count, payload, declared_payload_bytes)
            }
        }
    }
}

const RTLD_NOW: c_int = 2;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

unsafe fn load_symbol<T: Copy>(handle: *mut c_void, symbol: &str) -> Result<T, TxDecodeError> {
    let name = CString::new(symbol).map_err(|_| TxDecodeError::OpusBackendUnavailable)?;
    let ptr = dlsym(handle, name.as_ptr());
    if ptr.is_null() {
        return Err(TxDecodeError::OpusBackendUnavailable);
    }
    Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
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
        let mut decoder = TxCodecDecoder::new(TxMicCodec::Pcm);
        let samples = decoder
            .decode(TX_SAMPLE_TYPE_S16, 3, &payload, payload.len())
            .unwrap();
        assert_eq!(samples, vec![0.25, -0.5, 32767.0 / 32768.0]);
    }

    #[test]
    fn pcm_decoder_passes_through_float32_samples() {
        let payload = [0.25f32.to_le_bytes(), (-0.5f32).to_le_bytes()].concat();
        let mut decoder = TxCodecDecoder::new(TxMicCodec::Pcm);
        let samples = decoder
            .decode(TX_SAMPLE_TYPE_FLOAT32, 2, &payload, payload.len())
            .unwrap();
        assert_eq!(samples, vec![0.25, -0.5]);
    }

    #[test]
    fn pcm_decoder_rejects_bad_payload_lengths() {
        let mut decoder = TxCodecDecoder::new(TxMicCodec::Pcm);
        assert_eq!(
            decoder.decode(TX_SAMPLE_TYPE_S16, 3, &[0; 6], 4),
            Err(TxDecodeError::PayloadSizeMismatch)
        );
        assert_eq!(
            decoder.decode(TX_SAMPLE_TYPE_S16, 3, &[0; 4], 0),
            Err(TxDecodeError::PayloadTooShort)
        );
    }

    #[test]
    fn opus_decoder_is_not_enabled_yet() {
        let mut decoder = TxCodecDecoder::new(TxMicCodec::OpusWb);
        assert_eq!(
            decoder.decode(TX_SAMPLE_TYPE_S16, 3, &[0; 6], 6),
            Err(TxDecodeError::CodecDisabled)
        );
    }

    #[test]
    fn opus_profiles_match_defaults() {
        let nb = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusNb).unwrap();
        assert_eq!(nb.encoder_sample_rate_hz, 16_000);
        assert_eq!(nb.decode_output_sample_rate_hz, 48_000);
        assert_eq!(nb.bitrate_bps, 16_000);
        assert_eq!(nb.frame_ms, 20);
        assert_eq!(nb.encoder_frame_samples, 320);
        assert_eq!(nb.decode_output_frame_samples, 960);
        assert!(nb.fec_enabled);
        assert!(!nb.dtx_enabled);

        let wb = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusWb).unwrap();
        assert_eq!(wb.encoder_sample_rate_hz, 48_000);
        assert_eq!(wb.decode_output_sample_rate_hz, 48_000);
        assert_eq!(wb.bitrate_bps, 24_000);
        assert_eq!(wb.encoder_frame_samples, 960);
        assert_eq!(wb.decode_output_frame_samples, 960);
        assert!(TxOpusDecoderConfig::for_codec(TxMicCodec::Pcm).is_none());
    }

    #[test]
    fn opus_decoder_backend_is_gated_and_reports_output_invariant() {
        let config = TxOpusDecoderConfig::for_codec(TxMicCodec::OpusWb).unwrap();
        let decoder = TxOpusDecoder::new(
            config,
            TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            },
        );
        match decoder {
            Ok(decoder) => {
                assert_eq!(decoder.output_sample_rate_hz(), 48_000);
                assert_eq!(decoder.output_frame_samples(), 960);
            }
            Err(TxDecodeError::OpusBackendUnavailable) => {}
            Err(err) => panic!("unexpected Opus backend result: {err:?}"),
        }
    }

    #[test]
    fn opus_decoder_rejects_wrong_output_sample_count_before_backend_decode() {
        let mut decoder = TxCodecDecoder::new_with_flags(
            TxMicCodec::OpusWb,
            TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            },
        );
        let result = decoder.decode(TX_SAMPLE_TYPE_S16, 320, &[0xff, 0xff], 2);
        if result != Err(TxDecodeError::OpusBackendUnavailable) {
            assert_eq!(result, Err(TxDecodeError::SampleCountMismatch));
        }
    }

    #[test]
    fn opus_decoder_decodes_real_wideband_packet_when_enabled() {
        // 20 ms mono Opus packet generated locally with ffmpeg/libopus from a
        // 1 kHz sine at 48 kHz, 24 kbps, VOIP application, VBR off.
        let mut decoder = TxCodecDecoder::new_with_flags(
            TxMicCodec::OpusWb,
            TxCodecRuntimeFlags {
                opus_decode_enabled: true,
            },
        );
        match decoder.decode(
            TX_SAMPLE_TYPE_S16,
            TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
            &TX_OPUS_WB_TEST_PACKET,
            TX_OPUS_WB_TEST_PACKET.len(),
        ) {
            Ok(samples) => {
                assert_eq!(samples.len(), TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES);
                assert!(samples.iter().all(|sample| sample.is_finite()));
                let peak = samples
                    .iter()
                    .map(|sample| sample.abs())
                    .fold(0.0f32, f32::max);
                assert!(peak > 0.001);
            }
            Err(TxDecodeError::OpusBackendUnavailable) => {}
            Err(err) => panic!("unexpected Opus fixture decode result: {err:?}"),
        }
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
