//! Independent v4.4 wire specimens; do not construct these with production encoders.
use super::*;

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|byte| {
            assert_eq!(byte.len(), 2);
            u8::from_str_radix(byte, 16).unwrap()
        })
        .collect()
}

#[test]
fn discovery_protocol_is_distinct_from_firmware() {
    let bytes = hex(include_str!("fixtures/discovery-saturn-v44.hex"));
    let reply = parse_discovery_reply(&bytes).unwrap();
    assert_eq!(reply.state_code, 2);
    assert_eq!(reply.mac_address, [0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    assert_eq!(reply.device_code, 10);
    assert_eq!(reply.protocol_version, 44);
    assert_eq!(reply.firmware_version, 31);
    assert_eq!(reply.ddc_count, 10);
    assert_eq!(reply.frequency_format, 1);
    assert_eq!(reply.data_format_flags, 0);
    assert_eq!(reply.beta_version, 46);
    assert_eq!(reply.saturn_app_version(), Some(46));
}

#[test]
fn generic_beta_and_format_capabilities_are_not_saturn_app_version() {
    let bytes = hex(include_str!("fixtures/discovery-generic-v44.hex"));
    let reply = parse_discovery_reply(&bytes).unwrap();
    assert_eq!(reply.state_code, 3);
    assert_eq!(reply.device_code, 5);
    assert_eq!(reply.protocol_version, 44);
    assert_eq!(reply.firmware_version, 20);
    assert_eq!(reply.ddc_count, 8);
    assert_eq!(reply.frequency_format, 0);
    assert_eq!(reply.data_format_flags, 0x1f);
    assert_eq!(reply.beta_version, 7);
    assert_eq!(reply.saturn_app_version(), None);
}

#[test]
fn legacy_discovery_remains_accepted_without_upgrading_its_advertisement() {
    let mut bytes = hex(include_str!("fixtures/discovery-saturn-v44.hex"));
    bytes[12] = 43;
    bytes[13] = 27;
    bytes[20] = 4;
    let reply = parse_discovery_reply(&bytes).unwrap();
    assert_eq!(reply.protocol_version, 43);
    assert_eq!(reply.firmware_version, 27);
    assert_eq!(reply.ddc_count, 4);
}

#[test]
fn discovery_preserves_unknown_capability_bytes_as_metadata() {
    let mut bytes = hex(include_str!("fixtures/discovery-saturn-v44.hex"));
    bytes[21] = 17;
    bytes[22] = 0xe0;
    let reply = parse_discovery_reply(&bytes).unwrap();
    assert_eq!(reply.frequency_format, 17);
    assert_eq!(reply.data_format_flags, 0xe0);
}

#[test]
fn discovery_requires_a_complete_fixed_reply() {
    let mut bytes = hex(include_str!("fixtures/discovery-saturn-v44.hex"));
    for length in 0..60 {
        assert!(parse_discovery_reply(&bytes[..length]).is_none());
    }
    bytes.push(0);
    assert!(parse_discovery_reply(&bytes).is_none());
}

#[test]
fn observed_status_reports_protocol_firmware_and_saturn_build_separately() {
    let mut model =
        crate::radio_model::RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    model.apply_discovery(
        parse_discovery_reply(&hex(include_str!("fixtures/discovery-saturn-v44.hex"))).unwrap(),
    );
    let status = model.status_line();
    assert!(status.contains("proto:44 p2app:46"), "{status}");
    assert!(
        status.contains("fw:31 ddcs:10 freq_format:1 data_formats:0x00 beta:46"),
        "{status}"
    );
    model.apply_discovery(
        parse_discovery_reply(&hex(include_str!("fixtures/discovery-generic-v44.hex"))).unwrap(),
    );
    let status = model.status_line();
    assert!(status.contains("proto:44 p2app:n/a"), "{status}");
    assert!(
        status.contains("fw:20 ddcs:8 freq_format:0 data_formats:0x1f beta:7"),
        "{status}"
    );
}

#[test]
fn signed_24_bit_samples_and_full_packet_tail_match_independent_values() {
    let bytes = hex(include_str!("fixtures/ddc24-edge-values.hex"));
    let frame = parse_ddc_iq_frame(&bytes, 6).unwrap();
    assert_eq!(frame.ddc_index, 6);
    assert_eq!(frame.sequence, 0x01020304);
    assert_eq!(frame.bits_per_sample, 24);
    assert_eq!(frame.sample_count, 238);
    assert_eq!(frame.iq_samples.len(), 476);
    assert_eq!(frame.payload_len, 1428);
    assert_eq!(
        &frame.iq_samples[..8],
        &[
            -1.,
            8388607. / 8388608.,
            1. / 8388608.,
            -1. / 8388608.,
            0.5,
            -0.5,
            0.,
            0.
        ]
    );
    assert!(frame.iq_samples[8..474].iter().all(|v| *v == 0.));
    assert_eq!(&frame.iq_samples[474..], &[0.75, -0.75]);
    assert!(frame.approx_meter_dbm.is_finite());
}

#[test]
fn paired_feedback_uses_the_same_valid_24_bit_wire_format() {
    let bytes = hex(include_str!("fixtures/ddc24-edge-values.hex"));
    let frame = parse_ddc_iq_frame(&bytes, 0).unwrap();
    let paired = split_puresignal_samples(&frame).unwrap();
    assert_eq!(paired.rx_feedback.len(), 238);
    assert_eq!(paired.tx_reference.len(), 238);
    assert_eq!(&paired.rx_feedback[..2], &[-1., 8388607. / 8388608.]);
    assert_eq!(&paired.tx_reference[..2], &[1. / 8388608., -1. / 8388608.]);
    assert_eq!(&paired.tx_reference[236..], &[0.75, -0.75]);
}

#[test]
fn unsupported_width_never_decodes_as_24_bit() {
    let mut bytes = hex(include_str!("fixtures/ddc24-edge-values.hex"));
    for width in [0u16, 8, 16, 32, 64, u16::MAX] {
        bytes[12..14].copy_from_slice(&width.to_be_bytes());
        assert!(parse_ddc_iq_frame(&bytes, 6).is_none(), "width={width}");
    }
}

#[test]
fn sample_count_must_be_nonempty_and_fit_the_payload() {
    let mut bytes = hex(include_str!("fixtures/ddc24-edge-values.hex"));
    for count in [0u16, 239, u16::MAX] {
        bytes[14..16].copy_from_slice(&count.to_be_bytes());
        assert!(parse_ddc_iq_frame(&bytes, 6).is_none(), "count={count}");
    }
    for count in [1u16, 237, 238] {
        bytes[14..16].copy_from_slice(&count.to_be_bytes());
        assert_eq!(
            parse_ddc_iq_frame(&bytes, 6).unwrap().iq_samples.len(),
            usize::from(count) * 2
        );
    }
}

#[test]
fn ddc_packet_lengths_cannot_be_silently_truncated_or_padded() {
    let bytes = hex(include_str!("fixtures/ddc24-edge-values.hex"));
    for length in [0, 1, 15, 16, 1443] {
        assert!(parse_ddc_iq_frame(&bytes[..length], 6).is_none());
    }
    let mut oversized = bytes;
    oversized.push(0);
    assert!(parse_ddc_iq_frame(&oversized, 6).is_none());
}

#[test]
fn high_priority_v44_fifo_and_adc_peak_fields_match_the_wire_specimen() {
    let bytes = hex(include_str!("fixtures/high-priority-v44.hex"));
    let status = parse_high_priority_from_sdr(&bytes).unwrap();
    assert_eq!(status.sequence, 42);
    assert_eq!(status.ptt_bits, 0x13);
    assert_eq!(status.adc_overflows, 2);
    assert_eq!(status.exciter_power, 0x0102);
    assert_eq!(status.forward_power, 0x0304);
    assert_eq!(status.reverse_power, 0x0506);
    assert_eq!(status.fifo_flags, 0x0a);
    assert_eq!(status.ddc_fifo_samples, 0x1112);
    assert_eq!(status.mic_fifo_samples, 0x2122);
    assert_eq!(status.duc_fifo_samples, 0x3132);
    assert_eq!(status.speaker_fifo_samples, 0x4142);
    // Rust uses one-based ADC names; the wire specification calls these ADC0/1.
    assert_eq!(status.adc1_peak, 32767);
    assert_eq!(status.adc2_peak, 32768);
    assert_eq!(status.supply_voltage, 0x5152);
    assert_eq!(status.user_analog1, 0x6162);
    assert_eq!(status.user_analog2, 0x7172);
    assert_eq!(status.user_io_bits, 0x12);
}

#[test]
fn duc_specific_zero_sequence_matches_independent_wire_specimen() {
    let bytes = hex(include_str!("fixtures/duc-specific-v44.hex"));
    assert_eq!(build_duc_specific_packet(31, 12).as_slice(), bytes);
}
