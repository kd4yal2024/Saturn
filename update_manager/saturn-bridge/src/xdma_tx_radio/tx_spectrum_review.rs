//! Generated signals through real WDSP and final byte packing; never opens a device.
use super::*;
use crate::{p2::packets::build_duc_iq_packet, radio_model::DemodMode, wdsp::WdspTxEngine};

fn model(mode: DemodMode) -> RadioModel {
    let mut m = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    m.desired.mode = mode;
    (m.desired.tx_filter_low_hz, m.desired.tx_filter_high_hz) = mode.default_tx_filter_band();
    m.desired.tx_mic_gain_db = 0.;
    m.desired.tx_noise_gate_enabled = false;
    m
}

fn render(engine: &mut WdspTxEngine, pcm_tones: bool) -> Vec<f32> {
    let mut retained = Vec::new();
    for block in 0..120 {
        let pcm: Vec<f32> = (0..512)
            .map(|i| {
                let n = (block * 512 + i) as f64;
                if pcm_tones {
                    (0.2 * ((std::f64::consts::TAU * 700. * n / 48000.).sin()
                        + (std::f64::consts::TAU * 1900. * n / 48000.).sin()))
                        as f32
                } else {
                    0.
                }
            })
            .collect();
        let iq = engine.offline_render_block(&pcm).unwrap();
        assert!(iq.iter().all(|v| v.is_finite()));
        if block >= 100 {
            retained.extend(iq);
        }
    }
    retained
}

fn magnitude(iq: &[f32], hz: f64) -> f64 {
    let mut sum = [0.; 2];
    for (n, pair) in iq.chunks_exact(2).enumerate() {
        let phase = std::f64::consts::TAU * hz * n as f64 / 192000.;
        sum[0] += f64::from(pair[0]) * phase.cos() + f64::from(pair[1]) * phase.sin();
        sum[1] += f64::from(pair[1]) * phase.cos() - f64::from(pair[0]) * phase.sin();
    }
    sum[0].hypot(sum[1]) / (iq.len() / 2) as f64
}

fn pack_and_decode(iq: &[f32]) -> Vec<f32> {
    let mut decoded = Vec::new();
    for (sequence, frame) in iq.chunks_exact(DUC_FRAME_IQ_FLOATS).enumerate() {
        let mut direct = [0u8; DUC_FRAME_BYTES];
        encode_iq_frame(&mut direct, frame);
        let p2 = build_duc_iq_packet(sequence as u32, frame);
        // Actual P2_app InDUCIQ.c copies payload[3..6], then [0..3].
        for (word, wire) in direct.chunks_exact(6).zip(p2[4..].chunks_exact(6)) {
            assert_eq!(&word[..3], &wire[3..]);
            assert_eq!(&word[3..], &wire[..3]);
            let signed =
                |b: &[u8]| (i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8) as f32 / 8_388_607.;
            // Reconstruct logical I,Q from the hardware's Q,I byte stream.
            decoded.extend([signed(&word[3..]), signed(&word[..3])]);
        }
    }
    assert!(decoded.len() > 40000);
    decoded
}

fn check_tones(label: &str, mode: DemodMode, iq: &[f32]) -> bool {
    let iq = pack_and_decode(iq);
    let sign = if mode == DemodMode::Lsb { 1. } else { -1. };
    let a = magnitude(&iq, sign * 700.);
    let b = magnitude(&iq, sign * 1900.);
    let opposite_a = magnitude(&iq, -sign * 700.);
    let opposite_b = magnitude(&iq, -sign * 1900.);
    let peak = iq.iter().fold(0_f32, |p, v| p.max(v.abs()));
    let clipped = iq.iter().filter(|v| v.abs() >= 0.999999).count();
    eprintln!("OFFLINE final H2C {label} {mode:?}: tones={a:.8},{b:.8} opposite={opposite_a:.8},{opposite_b:.8} component_peak={peak:.8} clipped_components={clipped} p2_bytes_identical=true RF_unmeasured=true");
    a > 0.05
        && b > 0.05
        && opposite_a < a * 0.02
        && opposite_b < b * 0.02
        && (a / b - 1.).abs() < 0.15
        && clipped == 0
}

#[test]
#[ignore = "serial real WDSP spectrum/packing diagnosis, no radio devices or RF"]
fn generated_pcm_tones_survive_final_h2c_packing_usb_and_lsb() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    for mode in [DemodMode::Usb, DemodMode::Lsb] {
        let m = model(mode);
        let mut engine = WdspTxEngine::try_new(&m).unwrap();
        engine.sync_model(&m);
        engine.set_active(true);
        assert!(check_tones(
            "generated-PCM",
            mode,
            &render(&mut engine, true)
        ));
    }
}

#[test]
#[ignore = "serial real WDSP spectrum/packing diagnosis, no radio devices or RF"]
fn native_postgen_tones_have_correct_sideband_on_startup_and_mode_changes() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    let mut failures = Vec::new();
    for first in [DemodMode::Usb, DemodMode::Lsb] {
        let mut m = model(first);
        let mut engine = WdspTxEngine::try_new(&m).unwrap();
        m.desired.two_tone_enabled = true;
        engine.sync_model(&m);
        engine.set_active(true);
        for mode in [first, DemodMode::Usb, DemodMode::Lsb] {
            m.desired.mode = mode;
            (m.desired.tx_filter_low_hz, m.desired.tx_filter_high_hz) =
                mode.default_tx_filter_band();
            engine.sync_model(&m);
            if !check_tones("native-PostGen", mode, &render(&mut engine, false)) {
                failures.push(format!("startup={first:?}, current={mode:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "incorrect native two-tone sideband: {failures:?}"
    );
}

#[test]
#[ignore = "serial real WDSP + actual loopback/file output diagnostic taps; no radio"]
fn diagnostic_taps_follow_real_wdsp_and_completed_backend_output() {
    use saturn_bridge::signal_diagnostics as d;
    use std::io::{Read, Write};
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    let directory = std::env::var_os("SATURN_SIGNAL_DIAG_EVIDENCE_DIR").map(PathBuf::from);
    if let Some(dir) = &directory {
        assert!(dir.is_absolute());
        std::fs::create_dir(dir).expect("new evidence directory required");
    }
    let mut json = Vec::new();
    for (generation, mode) in [DemodMode::Usb, DemodMode::Lsb].into_iter().enumerate() {
        let mut m = model(mode);
        let mut engine = WdspTxEngine::try_new(&m).unwrap();
        engine.sync_model(&m);
        engine.set_active(true);
        // Both backend fixtures receive the exact same real native output.
        let mut retained_pcm = Vec::with_capacity(8192);
        let mut iq = vec![];
        for block in 0..120 {
            let pcm: Vec<f32> = (0..512)
                .map(|i| {
                    let n = (block * 512 + i) as f64;
                    (0.2 * ((std::f64::consts::TAU * 700. * n / 48000.).sin()
                        + (std::f64::consts::TAU * 1900. * n / 48000.).sin()))
                        as f32
                })
                .collect();
            iq = engine.offline_render_block(&pcm).unwrap();
            if block >= 104 {
                retained_pcm.extend(pcm.iter().copied().map(f64::from));
            }
        }
        let sign = if mode == DemodMode::Lsb { 1. } else { -1. };
        for backend in [d::Backend::P2, d::Backend::DirectXdma] {
            let template = d::Meta {
                point: d::Point::PreWdspPcm,
                backend,
                event: d::Event::Samples,
                session: 1,
                generation: generation as u64 + 1,
                config: 1,
                tx_epoch: generation as u64 + 1,
                observed_first_ns: Some(0),
                observed_last_ns: Some(0),
                recorded_ns: 0,
                time_domain: d::TimeDomain::OfflineFixture,
                rate_hz: 48000,
                units_per_full_scale: 1.,
                attenuation_command_db: None,
                continuity: d::Evidence::Clear,
                overload: d::Evidence::Unknown,
                keyed: None,
                requested_bytes: None,
                completed_bytes: None,
            };
            let tap = d::CaptureTap::new(8, 8192, 16, template).unwrap();
            assert!(tap.try_capture(template, &retained_pcm));
            let mut meta = template;
            meta.point = d::Point::WdspIq;
            meta.rate_hz = 192000;
            assert!(tap.try_capture(meta, &iq.iter().map(|x| f64::from(*x)).collect::<Vec<_>>()));
            let frame = &iq[..DUC_FRAME_IQ_FLOATS];
            meta.point = d::Point::BeforeOutputIq;
            assert!(tap.try_capture(
                meta,
                &frame.iter().map(|x| f64::from(*x)).collect::<Vec<_>>()
            ));
            let mut expected = vec![0; DUC_FRAME_BYTES];
            encode_iq_frame(&mut expected, frame);
            let completed = match backend {
                d::Backend::DirectXdma => {
                    let fixture = offline_fixture::DirectFixture::new();
                    {
                        let mut state = fixture.radio.state.lock().unwrap();
                        state.write_frame_batch(frame).unwrap();
                        assert_eq!(state.dma_writes, 1);
                        // Read through the actual file-backed DMA fd, not the
                        // in-memory encoder buffer. No physical RF inference.
                        let mut actual = vec![0; DUC_FRAME_BYTES];
                        state.dma.read_exact_at(&mut actual, 0).unwrap();
                        assert_eq!(actual, expected);
                    }
                    if let Some(dir) = &directory {
                        std::fs::write(
                            dir.join(format!("{backend:?}-{mode:?}-output.bin")),
                            &expected,
                        )
                        .unwrap();
                    }
                    expected.len() as u64
                }
                d::Backend::P2 => {
                    let (session, _hp, socket) = crate::tx_owner_stop::tests::p2_separate_lanes();
                    socket
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    session.send_duc_iq(frame).unwrap();
                    let mut actual = [0; 2048];
                    let (count, _) = socket.recv_from(&mut actual).unwrap();
                    let packet = build_duc_iq_packet(0, frame);
                    assert_eq!(&actual[..count], packet);
                    if let Some(dir) = &directory {
                        std::fs::write(
                            dir.join(format!("{backend:?}-{mode:?}-output.bin")),
                            &actual[..count],
                        )
                        .unwrap();
                    }
                    count as u64
                }
            };
            meta.point = d::Point::Timeline;
            meta.event = d::Event::OutputCompleted;
            meta.requested_bytes = Some(completed);
            meta.completed_bytes = Some(completed);
            assert!(tap.try_capture(meta, &[]));
            // Explicit injected backend failure: failed/short completion never
            // acquires a successful/full receipt or changes signal samples.
            meta.event = d::Event::Rejected;
            meta.completed_bytes = Some(0);
            assert!(tap.try_capture(meta, &[]));
            let mut index = 0;
            while tap
                .consume_one(|r| {
                    let tones = if r.meta.point == d::Point::PreWdspPcm {
                        [700., 1900.]
                    } else {
                        [sign * 700., sign * 1900.]
                    };
                    if r.meta.point == d::Point::PreWdspPcm || r.meta.point == d::Point::WdspIq {
                        let a = d::analyze(r, tones).unwrap();
                        assert!(a.tone_amplitude.iter().all(|x| *x > 0.1));
                        assert_eq!(a.components_at_full_scale, 0);
                    }
                    let report = d::report_json(r, tones);
                    json.push(report);
                    if let Some(dir) = &directory {
                        let path = dir.join(format!("{backend:?}-{mode:?}-{index}.satdiag"));
                        let mut file = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .unwrap();
                        d::write_record(r, &mut file).unwrap();
                        file.flush().unwrap();
                        drop(file);
                        let mut bytes = vec![];
                        std::fs::File::open(&path)
                            .unwrap()
                            .read_to_end(&mut bytes)
                            .unwrap();
                        let copy = d::read_record(&bytes[..]).unwrap();
                        assert_eq!(
                            copy.samples.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                            r.samples.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                        );
                    }
                    index += 1;
                })
                .is_some()
            {}
            assert_eq!(index, 5);
            assert_eq!(tap.dropped(), 0);
        }
        // Native engine stays owned by this test and is released before the
        // next mode. This does not select or start a live TX owner.
        m.desired.tx_enabled = false;
        engine.set_active(false);
    }
    if let Some(dir) = &directory {
        std::fs::write(
            dir.join("diagnostics.json"),
            format!("[{}]\n", json.join(",")),
        )
        .unwrap();
    }
}
