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
            "DIGU" => Self::DigU,
            "DIGL" => Self::DigL,
            _ => Self::Unknown,
        }
    }

    pub fn default_filter_band(self) -> (i32, i32) {
        match self {
            Self::Lsb | Self::DigL => (-3000, -300),
            Self::Usb | Self::DigU | Self::Unknown => (300, 3000),
            Self::Cwl => (-800, -200),
            Self::Cwu => (200, 800),
            Self::Am | Self::Sam => (-4000, 4000),
            Self::Fm => (-8000, 8000),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesiredRadioState {
    pub running: bool,
    pub tx_enabled: bool,
    pub rx_ddc_index: u8,
    pub vfo_a_hz: u32,
    pub vfo_b_hz: u32,
    pub iq_center_hz: u32,
    pub tx_frequency_hz: u32,
    pub ddc0_adc: u8,
    pub rx_antenna: u8,
    pub mode: DemodMode,
    pub rx_volume_db: f64,
    pub filter_low_hz: i32,
    pub filter_high_hz: i32,
    pub ddc0_sample_rate_khz: u16,
    pub ddc0_sample_size_bits: u8,
}

#[derive(Clone, Debug, Default)]
pub struct ObservedRadioState {
    pub discovery: Option<DiscoveryReply>,
    pub high_priority: Option<HighPriorityFromSdr>,
    pub last_ddc0_frame: Option<DdcIqFrame>,
    pub ddc0_packets: u64,
    pub high_priority_packets: u64,
    pub ddc0_meter_dbm: Option<f32>,
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
    ) -> Self {
        Self {
            desired: DesiredRadioState {
                running: true,
                tx_enabled: false,
                rx_ddc_index: rx_ddc_index.min(9),
                vfo_a_hz: ddc0_frequency_hz,
                vfo_b_hz: ddc0_frequency_hz,
                iq_center_hz: ddc0_frequency_hz,
                tx_frequency_hz: ddc0_frequency_hz,
                ddc0_adc: ddc0_adc.min(2),
                rx_antenna: 1,
                mode: DemodMode::Usb,
                rx_volume_db: -20.0,
                filter_low_hz: 300,
                filter_high_hz: 3000,
                ddc0_sample_rate_khz,
                ddc0_sample_size_bits,
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
            self.observed.ddc0_meter_dbm = Some(frame.approx_meter_dbm);
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
            "vfoA={} vfoB={} dds={} ddc={} adc={} rxant=ANT{} mode={} vol={:.1}dB filter={}..{} rate={}k {} | {} | {} | counters hp={} ddc{}={}",
            self.desired.vfo_a_hz,
            self.desired.vfo_b_hz,
            self.desired.iq_center_hz,
            self.desired.rx_ddc_index,
            adc_label(self.desired.ddc0_adc),
            self.desired.rx_antenna.max(1).min(3),
            self.desired.mode,
            self.desired.rx_volume_db,
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
