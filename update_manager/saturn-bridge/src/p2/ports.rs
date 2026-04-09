pub const COMMAND_DISCOVERY_PORT: u16 = 1024;
pub const DDC_SPECIFIC_PORT: u16 = 1025;
pub const DUC_SPECIFIC_PORT: u16 = 1026;
pub const HIGH_PRIORITY_TO_SDR_PORT: u16 = 1027;
pub const SPEAKER_AUDIO_PORT: u16 = 1028;
pub const DUC_IQ_PORT: u16 = 1029;
pub const HIGH_PRIORITY_FROM_SDR_PORT: u16 = 1025;
pub const MIC_AUDIO_PORT: u16 = 1026;
pub const DDC_IQ_BASE_PORT: u16 = 1035;
pub const WIDEBAND_BASE_PORT: u16 = 1027;
pub const MAX_DDC_STREAMS: usize = 10;

#[derive(Clone, Copy, Debug)]
pub struct P2PortMap {
    pub ddc_specific: u16,
    pub duc_specific: u16,
    pub high_priority_to_sdr: u16,
    pub speaker_audio: u16,
    pub duc_iq: u16,
    pub high_priority_from_sdr: u16,
    pub mic_audio: u16,
    pub ddc_iq_base: u16,
    pub wideband_base: u16,
}

impl Default for P2PortMap {
    fn default() -> Self {
        Self {
            ddc_specific: DDC_SPECIFIC_PORT,
            duc_specific: DUC_SPECIFIC_PORT,
            high_priority_to_sdr: HIGH_PRIORITY_TO_SDR_PORT,
            speaker_audio: SPEAKER_AUDIO_PORT,
            duc_iq: DUC_IQ_PORT,
            high_priority_from_sdr: HIGH_PRIORITY_FROM_SDR_PORT,
            mic_audio: MIC_AUDIO_PORT,
            ddc_iq_base: DDC_IQ_BASE_PORT,
            wideband_base: WIDEBAND_BASE_PORT,
        }
    }
}

impl P2PortMap {
    pub fn ddc_index_from_source_port(&self, source_port: u16) -> Option<u8> {
        if source_port < self.ddc_iq_base {
            return None;
        }
        let delta = source_port - self.ddc_iq_base;
        if delta < MAX_DDC_STREAMS as u16 {
            Some(delta as u8)
        } else {
            None
        }
    }
}
