use std::fmt;

use crate::p2::packets::{DdcIqFrame, DiscoveryReply, HighPriorityFromSdr};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DemodMode {
    Usb,
    Lsb,
    Cwu,
    Cwl,
    Am,
    Sam,
    Fm,
    Wfm,
    DigU,
    DigL,
    Unknown,
}

impl Default for DemodMode {
    fn default() -> Self {
        Self::Usb
    }
}

impl fmt::Display for DemodMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Usb => "USB",
            Self::Lsb => "LSB",
            Self::Cwu => "CWU",
            Self::Cwl => "CWL",
            Self::Am => "AM",
            Self::Sam => "SAM",
            Self::Fm => "FM",
            Self::Wfm => "WFM",
            Self::DigU => "DIGU",
            Self::DigL => "DIGL",
            Self::Unknown => "UNKNOWN",
        };
        f.write_str(text)
    }
}

impl DemodMode {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "USB" => Self::Usb,
            "LSB" => Self::Lsb,
            "CWU" => Self::Cwu,
            "CWL" => Self::Cwl,
            "AM" => Self::Am,
            "SAM" => Self::Sam,
            "FM" | "NFM" => Self::Fm,
            "WFM" | "WBFM" | "FM_STEREO" => Self::Wfm,
            "DIGU" => Self::DigU,
            "DIGL" => Self::DigL,
            _ => Self::Unknown,
        }
    }

    pub fn default_filter_band(self) -> (i32, i32) {
        match self {
            Self::Lsb | Self::DigL => (-3000, -300),
            Self::Usb | Self::Unknown => (250, 3000),
            Self::DigU => (300, 3000),
            Self::Cwl => (-800, -200),
            Self::Cwu => (200, 800),
            Self::Am | Self::Sam => (-4000, 4000),
            Self::Fm => (-6000, 6000),
            Self::Wfm => (-90_000, 90_000),
        }
    }

    pub fn default_tx_filter_band(self) -> (i32, i32) {
        match self {
            Self::Lsb => (-3000, -300),
            Self::DigL => (-3000, 0),
            Self::Usb | Self::Unknown => (50, 3050),
            Self::DigU => (0, 3000),
            Self::Cwl => (-800, -200),
            Self::Cwu => (200, 800),
            Self::Am | Self::Sam => (-3000, 3000),
            Self::Fm | Self::Wfm => (-3000, 3000),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WbfmDeemphasis {
    Off,
    #[default]
    NorthAmerica,
    Europe,
}

impl fmt::Display for WbfmDeemphasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "OFF",
            Self::NorthAmerica => "NA_75US",
            Self::Europe => "EU_50US",
        })
    }
}

impl WbfmDeemphasis {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "0" | "OFF" | "NONE" => Self::Off,
            "2" | "EU" | "EUROPE" | "EU_50US" | "50US" => Self::Europe,
            _ => Self::NorthAmerica,
        }
    }

    #[cfg(wdsp_has_wbfm)]
    pub fn wdsp_values(self) -> (i32, i32) {
        match self {
            Self::Off => (0, 0),
            Self::NorthAmerica => (1, 0),
            Self::Europe => (1, 1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoiseReductionMode {
    Off,
    Nr1,
    Nr2,
    Nr3,
    Nr4,
}

impl Default for NoiseReductionMode {
    fn default() -> Self {
        Self::Off
    }
}

impl fmt::Display for NoiseReductionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Off => "OFF",
            Self::Nr1 => "NR1",
            Self::Nr2 => "NR2",
            Self::Nr3 => "NR3",
            Self::Nr4 => "NR4",
        };
        f.write_str(text)
    }
}

impl NoiseReductionMode {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "0" | "OFF" => Self::Off,
            "1" | "NR1" | "ANR" => Self::Nr1,
            "2" | "NR2" | "EMNR" => Self::Nr2,
            "3" | "NR3" | "RNNR" => Self::Nr3,
            "4" | "NR4" | "SBNR" => Self::Nr4,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Nr2GainMethod {
    Gaussian,
    GaussianLog,
    #[default]
    Gamma,
    Trained,
}

impl fmt::Display for Nr2GainMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Gaussian => "GAUSSIAN",
            Self::GaussianLog => "GAUSSIAN_LOG",
            Self::Gamma => "GAMMA",
            Self::Trained => "TRAINED",
        })
    }
}

impl Nr2GainMethod {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "0" | "GAUSSIAN" | "GAUSSIAN_LINEAR" => Self::Gaussian,
            "1" | "GAUSSIAN_LOG" | "LOG" => Self::GaussianLog,
            "3" | "TRAINED" => Self::Trained,
            _ => Self::Gamma,
        }
    }

    pub fn wdsp_value(self) -> i32 {
        match self {
            Self::Gaussian => 0,
            Self::GaussianLog => 1,
            Self::Gamma => 2,
            Self::Trained => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Nr2NpeMethod {
    #[default]
    Osms,
    Mmse,
    Nstat,
}

impl fmt::Display for Nr2NpeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Osms => "OSMS",
            Self::Mmse => "MMSE",
            Self::Nstat => "NSTAT",
        })
    }
}

impl Nr2NpeMethod {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "1" | "MMSE" => Self::Mmse,
            "2" | "NSTAT" => Self::Nstat,
            _ => Self::Osms,
        }
    }

    pub fn wdsp_value(self) -> i32 {
        match self {
            Self::Osms => 0,
            Self::Mmse => 1,
            Self::Nstat => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoiseBlankerMode {
    Off,
    Nb1,
    Nb2,
}

impl Default for NoiseBlankerMode {
    fn default() -> Self {
        Self::Off
    }
}

impl fmt::Display for NoiseBlankerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Off => "OFF",
            Self::Nb1 => "NB1",
            Self::Nb2 => "NB2",
        };
        f.write_str(text)
    }
}

impl NoiseBlankerMode {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "0" | "OFF" => Self::Off,
            "1" | "NB1" | "NB" => Self::Nb1,
            "2" | "NB2" | "NOB" => Self::Nb2,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgcMode {
    Off,
    Long,
    Slow,
    Medium,
    Fast,
}

impl Default for AgcMode {
    fn default() -> Self {
        Self::Medium
    }
}

impl fmt::Display for AgcMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Off => "OFF",
            Self::Long => "LONG",
            Self::Slow => "SLOW",
            Self::Medium => "MEDIUM",
            Self::Fast => "FAST",
        };
        f.write_str(text)
    }
}

impl AgcMode {
    pub fn from_tci(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "0" | "OFF" => Self::Off,
            "1" | "LONG" => Self::Long,
            "2" | "SLOW" => Self::Slow,
            "3" | "MEDIUM" | "MED" => Self::Medium,
            "4" | "FAST" => Self::Fast,
            _ => Self::Medium,
        }
    }

    pub fn wdsp_value(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::Long => 1,
            Self::Slow => 2,
            Self::Medium => 3,
            Self::Fast => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TxPhase {
    #[default]
    Rx,
    Armed,
    Keyed,
}

impl fmt::Display for TxPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rx => write!(f, "rx"),
            Self::Armed => write!(f, "armed"),
            Self::Keyed => write!(f, "keyed"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PureSignalState {
    #[default]
    Off,
    Waiting,
    Calibrating,
    Correcting,
    Fault,
}

impl fmt::Display for PureSignalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Waiting => write!(f, "waiting"),
            Self::Calibrating => write!(f, "calibrating"),
            Self::Correcting => write!(f, "correcting"),
            Self::Fault => write!(f, "fault"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesiredRadioState {
    pub running: bool,
    pub tx_enabled: bool,
    pub tx_phase: TxPhase,
    pub rx_ddc_index: u8,
    pub vfo_a_hz: u32,
    pub vfo_b_hz: u32,
    pub iq_center_hz: u32,
    pub tx_frequency_hz: u32,
    pub ddc0_adc: u8,
    pub rx_antenna: u8,
    pub mode: DemodMode,
    pub rx_volume_db: f64,
    pub rx_noise_reduction_mode: NoiseReductionMode,
    pub rx_noise_reduction_level: f64,
    pub rx_nr2_gain_method: Nr2GainMethod,
    pub rx_nr2_npe_method: Nr2NpeMethod,
    pub rx_nr2_post_filter_enabled: bool,
    pub rx_wbfm_deemphasis: WbfmDeemphasis,
    pub nb_mode: NoiseBlankerMode,
    pub nb_threshold: f64,
    pub rx_anr_taps: i32,
    pub rx_anr_delay: i32,
    pub rx_anr_gain: f64,
    pub rx_anr_leakage: f64,
    pub anf_enabled: bool,
    pub rx_anf_taps: i32,
    pub rx_anf_delay: i32,
    pub rx_anf_gain: f64,
    pub rx_anf_leakage: f64,
    pub agc_mode: AgcMode,
    pub agc_gain: f64,
    pub filter_low_hz: i32,
    pub filter_high_hz: i32,
    pub ddc0_sample_rate_khz: u16,
    pub ddc0_sample_size_bits: u8,
    /// Remote TX power target in watts. The P2 drive byte is derived from this
    /// target in the high-priority packet builder.
    pub tx_drive: u8,
    pub tx_mic_gain_db: f64,
    pub tx_filter_low_hz: i32,
    pub tx_filter_high_hz: i32,
    pub rx_eq_enabled: bool,
    pub rx_eq_bands: [i32; 11],
    pub tx_eq_enabled: bool,
    pub tx_eq_bands: [i32; 11],
    pub cfc_enabled: bool,
    pub cfc_precomp_db: f64,
    pub cfc_bands: [f64; 10],
    pub tx_phase_rotator_enabled: bool,
    pub tx_phase_rotator_auto: bool,
    pub tx_phase_rotator_corner_hz: f64,
    pub pure_signal_enabled: bool,
    pub pure_signal_auto_attenuate: bool,
    pub pure_signal_attenuation_db: u8,
    pub two_tone_enabled: bool,
    pub tx_two_tone_freq1_hz: f64,
    pub tx_two_tone_freq2_hz: f64,
    pub tx_two_tone_level_db: f64,
    pub tx_two_tone_invert_lsb: bool,
    pub tx_two_tone_delay_ms: u16,
    pub rx_fft_size: u32,
    pub rx_low_latency: bool,
    pub tx_fft_size: u32,
    pub tx_low_latency: bool,
    pub tx_noise_gate_enabled: bool,
    pub tx_noise_gate_threshold_db: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ObservedRadioState {
    pub discovery: Option<DiscoveryReply>,
    pub high_priority: Option<HighPriorityFromSdr>,
    pub last_ddc0_frame: Option<DdcIqFrame>,
    pub ddc0_packets: u64,
    pub high_priority_packets: u64,
    pub ddc0_meter_dbm: Option<f32>,
    pub rx_wbfm_stereo_detected: bool,
    pub pure_signal_state: PureSignalState,
    pub pure_signal_feedback_level: i32,
    pub pure_signal_calibration_count: i32,
    pub pure_signal_correcting: bool,
    pub pure_signal_max_tx: f64,
    pub pure_signal_feedback_packets: u64,
    pub pure_signal_feedback_gaps: u64,
}

#[derive(Clone, Debug)]
pub struct RadioModel {
    pub desired: DesiredRadioState,
    pub observed: ObservedRadioState,
}

impl RadioModel {
    pub fn new(
        rx_ddc_index: u8,
        ddc0_frequency_hz: u32,
        ddc0_adc: u8,
        ddc0_sample_rate_khz: u16,
        ddc0_sample_size_bits: u8,
        rx_fft_size: u32,
        rx_low_latency: bool,
        tx_fft_size: u32,
        tx_low_latency: bool,
    ) -> Self {
        let mode = DemodMode::Usb;
        let (filter_low_hz, filter_high_hz) = mode.default_filter_band();
        let (tx_filter_low_hz, tx_filter_high_hz) = mode.default_tx_filter_band();
        Self {
            desired: DesiredRadioState {
                running: false,
                tx_enabled: false,
                tx_phase: TxPhase::Rx,
                rx_ddc_index: rx_ddc_index.min(9),
                vfo_a_hz: ddc0_frequency_hz,
                vfo_b_hz: ddc0_frequency_hz,
                iq_center_hz: ddc0_frequency_hz,
                tx_frequency_hz: ddc0_frequency_hz,
                ddc0_adc: ddc0_adc.min(2),
                rx_antenna: 1,
                mode,
                rx_volume_db: -10.0,
                rx_noise_reduction_mode: NoiseReductionMode::Off,
                rx_noise_reduction_level: 0.0,
                rx_nr2_gain_method: Nr2GainMethod::Gamma,
                rx_nr2_npe_method: Nr2NpeMethod::Osms,
                rx_nr2_post_filter_enabled: true,
                rx_wbfm_deemphasis: WbfmDeemphasis::NorthAmerica,
                nb_mode: NoiseBlankerMode::Off,
                nb_threshold: 4.95,
                rx_anr_taps: 64,
                rx_anr_delay: 16,
                rx_anr_gain: 0.0002,
                rx_anr_leakage: 0.00005,
                anf_enabled: false,
                rx_anf_taps: 64,
                rx_anf_delay: 16,
                rx_anf_gain: 0.00012,
                rx_anf_leakage: 0.00008,
                agc_mode: AgcMode::Medium,
                agc_gain: 80.0,
                filter_low_hz,
                filter_high_hz,
                ddc0_sample_rate_khz,
                ddc0_sample_size_bits,
                tx_drive: 10,
                tx_mic_gain_db: -12.0,
                tx_filter_low_hz,
                tx_filter_high_hz,
                rx_eq_enabled: false,
                rx_eq_bands: [0i32; 11],
                tx_eq_enabled: false,
                tx_eq_bands: [0i32; 11],
                cfc_enabled: false,
                cfc_precomp_db: 0.0,
                cfc_bands: [0.0f64; 10],
                tx_phase_rotator_enabled: false,
                tx_phase_rotator_auto: false,
                tx_phase_rotator_corner_hz: 338.0,
                pure_signal_enabled: false,
                pure_signal_auto_attenuate: true,
                pure_signal_attenuation_db: 0,
                two_tone_enabled: false,
                tx_two_tone_freq1_hz: 700.0,
                tx_two_tone_freq2_hz: 1900.0,
                tx_two_tone_level_db: 0.0,
                tx_two_tone_invert_lsb: true,
                tx_two_tone_delay_ms: 0,
                rx_fft_size,
                rx_low_latency,
                tx_fft_size,
                tx_low_latency,
                tx_noise_gate_enabled: true,
                tx_noise_gate_threshold_db: -30.0,
            },
            observed: ObservedRadioState::default(),
        }
    }

    pub fn apply_discovery(&mut self, reply: DiscoveryReply) {
        self.observed.discovery = Some(reply);
    }

    pub fn apply_high_priority(&mut self, packet: HighPriorityFromSdr) {
        self.observed.high_priority_packets += 1;
        self.observed.high_priority = Some(packet);
    }

    pub fn apply_ddc_frame(&mut self, frame: DdcIqFrame) {
        if frame.ddc_index == self.desired.rx_ddc_index {
            self.observed.ddc0_packets += 1;
            self.observed.last_ddc0_frame = Some(frame);
        }
    }

    pub fn status_line(&self) -> String {
        let discovery = self
            .observed
            .discovery
            .as_ref()
            .map(|reply| {
                format!(
                    "disc=state:{} dev:{} proto:{} p2app:{} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    reply.state_code,
                    reply.device_code,
                    reply.protocol_version,
                    reply.p2app_version,
                    reply.mac_address[0],
                    reply.mac_address[1],
                    reply.mac_address[2],
                    reply.mac_address[3],
                    reply.mac_address[4],
                    reply.mac_address[5]
                )
            })
            .unwrap_or_else(|| "disc=pending".to_string());

        let hp = self
            .observed
            .high_priority
            .as_ref()
            .map(|packet| {
                format!(
                    "hp_seq={} ptt=0x{:02x} adc=0x{:02x} fifo=0x{:02x} exc={} fwd={} rev={} vcc={} ddc/mic/duc/spk={}/{}/{}/{} peaks={}/{} aux={}/{} io=0x{:02x}",
                    packet.sequence,
                    packet.ptt_bits,
                    packet.adc_overflows,
                    packet.fifo_flags,
                    packet.exciter_power,
                    packet.forward_power,
                    packet.reverse_power,
                    packet.supply_voltage,
                    packet.ddc_fifo_samples,
                    packet.mic_fifo_samples,
                    packet.duc_fifo_samples,
                    packet.speaker_fifo_samples,
                    packet.adc1_peak,
                    packet.adc2_peak,
                    packet.user_analog1,
                    packet.user_analog2,
                    packet.user_io_bits
                )
            })
            .unwrap_or_else(|| "hp=waiting".to_string());

        let ddc = self
            .observed
            .last_ddc0_frame
            .as_ref()
            .map(|frame| {
                format!(
                    "ddc{}_seq={} samples={} bits={} payload={} meter={:.1}dBm",
                    self.desired.rx_ddc_index,
                    frame.sequence,
                    frame.sample_count,
                    frame.bits_per_sample,
                    frame.payload_len,
                    frame.approx_meter_dbm
                )
            })
            .unwrap_or_else(|| format!("ddc{}=waiting", self.desired.rx_ddc_index));

        format!(
            "vfoA={} vfoB={} dds={} ddc={} adc={} rxant=ANT{} mode={} vol={:.1}dB nr={}({:.0}%) nb={} anf={} agc={} top={:.0} tx={} target={}W filter={}..{} rate={}k {} | {} | {} | counters hp={} ddc{}={}",
            self.desired.vfo_a_hz,
            self.desired.vfo_b_hz,
            self.desired.iq_center_hz,
            self.desired.rx_ddc_index,
            adc_label(self.desired.ddc0_adc),
            self.desired.rx_antenna.max(1).min(3),
            self.desired.mode,
            self.desired.rx_volume_db,
            self.desired.rx_noise_reduction_mode,
            self.desired.rx_noise_reduction_level,
            self.desired.nb_mode,
            if self.desired.anf_enabled { "ON" } else { "OFF" },
            self.desired.agc_mode,
            self.desired.agc_gain,
            if self.desired.tx_enabled { "TX" } else { "RX" },
            self.desired.tx_drive,
            self.desired.filter_low_hz,
            self.desired.filter_high_hz,
            self.desired.ddc0_sample_rate_khz,
            discovery,
            hp,
            ddc,
            self.observed.high_priority_packets,
            self.desired.rx_ddc_index,
            self.observed.ddc0_packets
        )
    }
}

fn adc_label(adc: u8) -> &'static str {
    match adc {
        1 => "ADC2",
        2 => "TXSAMPLES",
        _ => "ADC1",
    }
}
