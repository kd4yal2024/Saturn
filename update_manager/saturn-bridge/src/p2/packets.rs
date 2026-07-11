use crate::p2::ports::P2PortMap;

pub const DISCOVERY_PACKET_SIZE: usize = 60;
pub const DISCOVERY_REPLY_SIZE: usize = 60;
pub const GENERAL_PACKET_SIZE: usize = 60;
pub const DUC_SPECIFIC_PACKET_SIZE: usize = 60;
pub const DDC_SPECIFIC_PACKET_SIZE: usize = 1444;
pub const HIGH_PRIORITY_TO_SDR_PACKET_SIZE: usize = 1444;
pub const HIGH_PRIORITY_FROM_SDR_PACKET_SIZE: usize = 60;
pub const DDC_IQ_PACKET_SIZE: usize = 1444;
pub const DUC_IQ_PACKET_SIZE: usize = 1444;
pub const DUC_IQ_SAMPLES: usize = 240;

#[derive(Clone, Copy, Debug)]
pub struct DdcSetup {
    pub ddc_index: u8,
    pub adc: u8,
    pub sample_rate_khz: u16,
    pub sample_size_bits: u8,
}

#[derive(Clone, Debug)]
pub struct HighPriorityToSdr {
    pub run: bool,
    pub tx: bool,
    pub ddc_phase_words: [u32; 10],
    pub duc_phase_word: u32,
    pub tx_drive: u8,
    pub cat_port: u16,
    pub alex_tx_word: u16,
    pub alex_rx_word: u16,
    pub alex_rx2_filter_word: u16,
    pub alex_rx1_filter_word: u16,
    pub rx2_attenuation_db: u8,
    pub rx1_attenuation_db: u8,
}

#[derive(Clone, Debug)]
pub struct DiscoveryReply {
    pub state_code: u8,
    pub mac_address: [u8; 6],
    pub device_code: u8,
    pub protocol_version: u8,
    pub p2app_version: u8,
}

#[derive(Clone, Debug)]
pub struct HighPriorityFromSdr {
    pub sequence: u32,
    pub ptt_bits: u8,
    pub adc_overflows: u8,
    pub exciter_power: u16,
    pub forward_power: u16,
    pub reverse_power: u16,
    pub fifo_flags: u8,
    pub ddc_fifo_samples: u16,
    pub mic_fifo_samples: u16,
    pub duc_fifo_samples: u16,
    pub speaker_fifo_samples: u16,
    pub adc1_peak: u16,
    pub adc2_peak: u16,
    pub supply_voltage: u16,
    pub user_analog1: u16,
    pub user_analog2: u16,
    pub user_io_bits: u8,
}

#[derive(Clone, Debug)]
pub struct DdcIqFrame {
    pub ddc_index: u8,
    pub sequence: u32,
    pub bits_per_sample: u16,
    pub sample_count: u16,
    pub payload_len: usize,
    pub iq_samples: Vec<f32>,
    pub approx_meter_dbm: f32,
}

#[derive(Clone, Debug)]
pub struct PureSignalSamples {
    pub tx_reference: Vec<f64>,
    pub rx_feedback: Vec<f64>,
}

pub fn split_puresignal_samples(frame: &DdcIqFrame) -> Option<PureSignalSamples> {
    if frame.ddc_index != 0 || frame.sample_count < 2 || !frame.sample_count.is_multiple_of(2) {
        return None;
    }

    let pair_count = frame.sample_count as usize / 2;
    let mut tx_reference = Vec::with_capacity(pair_count * 2);
    let mut rx_feedback = Vec::with_capacity(pair_count * 2);
    for synchronized_pair in frame.iq_samples.chunks_exact(4) {
        rx_feedback.push(synchronized_pair[0] as f64);
        rx_feedback.push(synchronized_pair[1] as f64);
        tx_reference.push(synchronized_pair[2] as f64);
        tx_reference.push(synchronized_pair[3] as f64);
    }
    Some(PureSignalSamples {
        tx_reference,
        rx_feedback,
    })
}

pub fn build_discovery_request() -> [u8; DISCOVERY_PACKET_SIZE] {
    let mut packet = [0u8; DISCOVERY_PACKET_SIZE];
    packet[4] = 2;
    packet
}

pub fn build_general_packet(port_map: &P2PortMap) -> [u8; GENERAL_PACKET_SIZE] {
    let mut packet = [0u8; GENERAL_PACKET_SIZE];
    packet[4] = 0;
    write_u16_be(&mut packet, 5, port_map.ddc_specific);
    write_u16_be(&mut packet, 7, port_map.duc_specific);
    write_u16_be(&mut packet, 9, port_map.high_priority_to_sdr);
    write_u16_be(&mut packet, 11, port_map.high_priority_from_sdr);
    write_u16_be(&mut packet, 13, port_map.speaker_audio);
    write_u16_be(&mut packet, 15, port_map.duc_iq);
    write_u16_be(&mut packet, 17, port_map.ddc_iq_base);
    write_u16_be(&mut packet, 19, port_map.mic_audio);
    write_u16_be(&mut packet, 21, port_map.wideband_base);
    packet[37] = 0x08;
    packet[38] = 1;
    // Match piHPSDR's Saturn path: keep the PA enabled unless a higher-level
    // policy explicitly disables it.
    packet[58] = 0x01;
    packet[59] = 0x03;
    packet
}

pub fn build_duc_specific_packet(
    sequence: u32,
    adc1_tx_attenuation_db: u8,
    adc0_tx_attenuation_db: u8,
) -> [u8; DUC_SPECIFIC_PACKET_SIZE] {
    let mut packet = [0u8; DUC_SPECIFIC_PACKET_SIZE];
    write_u32_be(&mut packet, 0, sequence);
    packet[4] = 1; // 1 DUC stream enabled
                   // Match piHPSDR's safe Saturn/P2 defaults for the TX-specific path.
                   // Byte 50 bit 2 must be set to keep Orion mic-PTT disabled.
    packet[50] = 0x04;
    packet[58] = adc1_tx_attenuation_db.min(31);
    packet[59] = adc0_tx_attenuation_db.min(31);
    packet
}

/// Build a 1444-byte DUC IQ packet from up to 240 IQ pairs (interleaved I,Q f32).
/// Each sample is encoded as a signed 24-bit big-endian integer.
pub fn build_duc_iq_packet(sequence: u32, iq_samples: &[f32]) -> [u8; DUC_IQ_PACKET_SIZE] {
    let mut packet = [0u8; DUC_IQ_PACKET_SIZE];
    write_u32_be(&mut packet, 0, sequence);
    let pair_count = (iq_samples.len() / 2).min(DUC_IQ_SAMPLES);
    for i in 0..pair_count {
        let i_val = (iq_samples[i * 2].clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
        let q_val = (iq_samples[i * 2 + 1].clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
        let offset = 4 + i * 6;
        write_signed_24_be(&mut packet, offset, i_val);
        write_signed_24_be(&mut packet, offset + 3, q_val);
    }
    packet
}

pub fn build_ddc_specific_packet(
    enable_mask: u16,
    setups: &[DdcSetup],
    sync_ddc1_to_ddc0: bool,
) -> [u8; DDC_SPECIFIC_PACKET_SIZE] {
    let mut packet = [0u8; DDC_SPECIFIC_PACKET_SIZE];
    packet[4] = 2;
    write_u16_le(&mut packet, 7, enable_mask);
    for setup in setups {
        let offset = 17 + usize::from(setup.ddc_index.min(9)) * 6;
        packet[offset] = setup.adc.min(2);
        write_u16_be(&mut packet, offset + 1, setup.sample_rate_khz);
        packet[offset + 5] = setup.sample_size_bits;
    }
    if sync_ddc1_to_ddc0 {
        packet[1363] = 0x02;
    }
    packet
}

pub fn build_high_priority_to_sdr(
    state: &HighPriorityToSdr,
) -> [u8; HIGH_PRIORITY_TO_SDR_PACKET_SIZE] {
    let mut packet = [0u8; HIGH_PRIORITY_TO_SDR_PACKET_SIZE];

    if state.run {
        packet[4] |= 0x01;
    }
    if state.tx {
        packet[4] |= 0x02;
    }

    for (index, phase_word) in state.ddc_phase_words.iter().enumerate() {
        write_u32_be(&mut packet, 9 + index * 4, *phase_word);
    }

    write_u32_be(&mut packet, 329, state.duc_phase_word);
    packet[345] = state.tx_drive;
    write_u16_be(&mut packet, 1398, state.cat_port);
    write_u16_be(&mut packet, 1428, state.alex_tx_word);
    write_u16_be(&mut packet, 1430, state.alex_rx2_filter_word);
    write_u16_be(&mut packet, 1432, state.alex_rx_word);
    write_u16_be(&mut packet, 1434, state.alex_rx1_filter_word);
    packet[1442] = state.rx2_attenuation_db & 0x1f;
    packet[1443] = state.rx1_attenuation_db & 0x1f;
    packet
}

pub fn parse_discovery_reply(packet: &[u8]) -> Option<DiscoveryReply> {
    if packet.len() != DISCOVERY_REPLY_SIZE {
        return None;
    }

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&packet[5..11]);

    Some(DiscoveryReply {
        state_code: packet[4],
        mac_address: mac,
        device_code: packet[11],
        protocol_version: packet[13],
        p2app_version: packet[23],
    })
}

pub fn parse_high_priority_from_sdr(packet: &[u8]) -> Option<HighPriorityFromSdr> {
    if packet.len() != HIGH_PRIORITY_FROM_SDR_PACKET_SIZE {
        return None;
    }

    Some(HighPriorityFromSdr {
        sequence: read_u32_be(packet, 0),
        ptt_bits: packet[4],
        adc_overflows: packet[5],
        exciter_power: read_u16_be(packet, 6),
        forward_power: read_u16_be(packet, 14),
        reverse_power: read_u16_be(packet, 22),
        fifo_flags: packet[30],
        ddc_fifo_samples: read_u16_be(packet, 31),
        mic_fifo_samples: read_u16_be(packet, 33),
        duc_fifo_samples: read_u16_be(packet, 35),
        speaker_fifo_samples: read_u16_be(packet, 37),
        adc1_peak: read_u16_be(packet, 39),
        adc2_peak: read_u16_be(packet, 41),
        supply_voltage: read_u16_be(packet, 49),
        user_analog1: read_u16_be(packet, 55),
        user_analog2: read_u16_be(packet, 57),
        user_io_bits: packet[59],
    })
}

pub fn parse_ddc_iq_frame(packet: &[u8], ddc_index: u8) -> Option<DdcIqFrame> {
    if packet.len() != DDC_IQ_PACKET_SIZE {
        return None;
    }

    let sample_count = read_u16_be(packet, 14) as usize;
    let payload = &packet[16..];
    if payload.len() < sample_count.saturating_mul(6) {
        return None;
    }

    let mut iq_samples = Vec::with_capacity(sample_count * 2);
    let mut power_sum = 0.0f64;

    for sample_index in 0..sample_count {
        let offset = sample_index * 6;
        let i_sample = read_signed_24_be(&payload[offset..offset + 3]);
        let q_sample = read_signed_24_be(&payload[offset + 3..offset + 6]);
        let i_float = i_sample as f32 * 1.192_092_9e-7f32;
        let q_float = q_sample as f32 * 1.192_092_9e-7f32;
        iq_samples.push(i_float);
        iq_samples.push(q_float);
        power_sum += ((i_float * i_float) + (q_float * q_float)) as f64 * 0.5f64;
    }

    let avg_power = if sample_count == 0 {
        1.0e-20f64
    } else {
        (power_sum / sample_count as f64).max(1.0e-20f64)
    };
    let approx_meter_dbm = (10.0 * avg_power.log10() as f32) - 127.0;

    Some(DdcIqFrame {
        ddc_index,
        sequence: read_u32_be(packet, 0),
        bits_per_sample: read_u16_be(packet, 12),
        sample_count: sample_count as u16,
        payload_len: packet.len().saturating_sub(16),
        iq_samples,
        approx_meter_dbm,
    })
}

fn read_u16_be(packet: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([packet[offset], packet[offset + 1]])
}

#[cfg(test)]
fn read_u16_le(packet: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([packet[offset], packet[offset + 1]])
}

fn read_u32_be(packet: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    ])
}

fn read_signed_24_be(packet: &[u8]) -> i32 {
    let mut value = ((packet[0] as i32) << 16) | ((packet[1] as i32) << 8) | (packet[2] as i32);
    if (value & 0x0080_0000) != 0 {
        value |= !0x00FF_FFFF;
    }
    value
}

fn write_u16_be(packet: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_be_bytes();
    packet[offset..offset + 2].copy_from_slice(&bytes);
}

fn write_u16_le(packet: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    packet[offset..offset + 2].copy_from_slice(&bytes);
}

fn write_u32_be(packet: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_be_bytes();
    packet[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_signed_24_be(packet: &mut [u8], offset: usize, value: i32) {
    let clamped = value.clamp(-(1 << 23), (1 << 23) - 1);
    packet[offset] = ((clamped >> 16) & 0xFF) as u8;
    packet[offset + 1] = ((clamped >> 8) & 0xFF) as u8;
    packet[offset + 2] = (clamped & 0xFF) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2::ports::P2PortMap;

    #[test]
    fn general_packet_uses_expected_offsets() {
        let packet = build_general_packet(&P2PortMap::default());
        assert_eq!(packet[4], 0);
        assert_eq!(read_u16_be(&packet, 5), 1025);
        assert_eq!(read_u16_be(&packet, 17), 1035);
        assert_eq!(
            P2PortMap::default().ddc_index_from_source_port(1037),
            Some(2)
        );
        assert_eq!(packet[37], 0x08);
        assert_eq!(packet[38], 1);
        assert_eq!(packet[58], 0x01);
        assert_eq!(packet[59], 0x03);
    }

    #[test]
    fn high_priority_packet_builder_sets_run_and_frequency() {
        let packet = build_high_priority_to_sdr(&HighPriorityToSdr {
            run: true,
            tx: false,
            ddc_phase_words: [14_200_000, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            duc_phase_word: 14_200_000,
            tx_drive: 0,
            cat_port: 0,
            alex_tx_word: 0x0101,
            alex_rx_word: 0x0101,
            alex_rx2_filter_word: 0x0001,
            alex_rx1_filter_word: 0x0001,
            rx2_attenuation_db: 0,
            rx1_attenuation_db: 0,
        });
        assert_eq!(packet[4] & 0x01, 0x01);
        assert_eq!(read_u32_be(&packet, 9), 14_200_000);
        assert_eq!(read_u32_be(&packet, 329), 14_200_000);
        assert_eq!(read_u16_be(&packet, 1428), 0x0101);
        assert_eq!(read_u16_be(&packet, 1432), 0x0101);
    }

    #[test]
    fn ddc_iq_parser_reads_header() {
        let mut packet = [0u8; DDC_IQ_PACKET_SIZE];
        write_u32_be(&mut packet, 0, 42);
        write_u16_be(&mut packet, 12, 24);
        write_u16_be(&mut packet, 14, 238);
        let frame = parse_ddc_iq_frame(&packet, 0).expect("frame");
        assert_eq!(frame.sequence, 42);
        assert_eq!(frame.bits_per_sample, 24);
        assert_eq!(frame.sample_count, 238);
        assert_eq!(frame.payload_len, 1428);
        assert_eq!(frame.iq_samples.len(), 476);
    }

    #[test]
    fn puresignal_split_deinterleaves_feedback_and_tx_reference() {
        let frame = DdcIqFrame {
            ddc_index: 0,
            sequence: 9,
            bits_per_sample: 24,
            sample_count: 4,
            payload_len: 24,
            iq_samples: vec![0.1, 0.2, 0.3, 0.4, -0.1, -0.2, -0.3, -0.4],
            approx_meter_dbm: -10.0,
        };
        let samples = split_puresignal_samples(&frame).expect("PureSignal pair");
        assert_eq!(
            samples.rx_feedback,
            vec![0.1f32 as f64, 0.2f32 as f64, -0.1f32 as f64, -0.2f32 as f64]
        );
        assert_eq!(
            samples.tx_reference,
            vec![0.3f32 as f64, 0.4f32 as f64, -0.3f32 as f64, -0.4f32 as f64]
        );
    }

    #[test]
    fn ddc_specific_packet_sets_adc_and_rate() {
        let packet = build_ddc_specific_packet(
            1 << 2,
            &[DdcSetup {
                ddc_index: 2,
                adc: 1,
                sample_rate_khz: 48,
                sample_size_bits: 24,
            }],
            false,
        );
        assert_eq!(packet[4], 2);
        assert_eq!(read_u16_le(&packet, 7), 1 << 2);
        assert_eq!(read_u16_be(&packet, 30), 48);
        assert_eq!(packet[29], 1);
        assert_eq!(packet[34], 24);
    }

    #[test]
    fn ddc_specific_packet_configures_synchronized_puresignal_pair() {
        let packet = build_ddc_specific_packet(
            1,
            &[
                DdcSetup {
                    ddc_index: 0,
                    adc: 0,
                    sample_rate_khz: 192,
                    sample_size_bits: 24,
                },
                DdcSetup {
                    ddc_index: 1,
                    adc: 2,
                    sample_rate_khz: 192,
                    sample_size_bits: 24,
                },
            ],
            true,
        );
        assert_eq!(read_u16_le(&packet, 7), 1);
        assert_eq!(packet[17], 0);
        assert_eq!(packet[23], 2);
        assert_eq!(read_u16_be(&packet, 18), 192);
        assert_eq!(read_u16_be(&packet, 24), 192);
        assert_eq!(packet[1363], 0x02);
    }

    #[test]
    fn duc_specific_packet_uses_safe_tx_defaults() {
        let packet = build_duc_specific_packet(7, 31, 12);
        assert_eq!(read_u32_be(&packet, 0), 7);
        assert_eq!(packet[4], 1);
        assert_eq!(packet[50] & 0x04, 0x04);
        assert_eq!(packet[58], 31);
        assert_eq!(packet[59], 12);
    }
}
