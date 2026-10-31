use super::*;
fn meta(point: Point) -> Meta {
    Meta {
        point,
        backend: Backend::DirectXdma,
        event: Event::Samples,
        session: 7,
        generation: 11,
        config: 13,
        tx_epoch: 17,
        observed_first_ns: Some(100),
        observed_last_ns: Some(200),
        recorded_ns: 300,
        time_domain: TimeDomain::OfflineFixture,
        rate_hz: 48000,
        units_per_full_scale: 1.,
        attenuation_command_db: Some(31),
        continuity: Evidence::Clear,
        overload: Evidence::Unknown,
        keyed: Some(false),
        requested_bytes: None,
        completed_bytes: None,
    }
}
fn signal(real: bool, sign: f64) -> Record {
    let mut m = meta(if real {
        Point::PreWdspPcm
    } else {
        Point::WdspIq
    });
    m.rate_hz = 48000;
    let samples = (0..4096)
        .flat_map(|n| {
            let p = std::f64::consts::TAU * n as f64 / 48000.;
            if real {
                vec![0.2 * (700. * p).sin() + 0.1 * (1900. * p).sin()]
            } else {
                vec![
                    0.2 * (700. * p).cos() + 0.1 * (1900. * p).cos(),
                    sign * (0.2 * (700. * p).sin() + 0.1 * (1900. * p).sin()),
                ]
            }
        })
        .collect();
    Record { meta: m, samples }
}
#[test]
fn real_pcm_amplitudes_and_complex_sidebands_are_not_conflated() {
    let pcm = analyze(&signal(true, 1.), [700., 1900.]).unwrap();
    for (actual, expected) in pcm.tone_amplitude.into_iter().zip([0.2, 0.1]) {
        assert!((actual - expected).abs() < 1e-5);
    }
    for sign in [-1., 1.] {
        let iq = analyze(&signal(false, sign), [sign * 700., sign * 1900.]).unwrap();
        assert!((iq.tone_amplitude[0] - 0.2).abs() < 1e-5);
        assert!((iq.tone_amplitude[1] - 0.1).abs() < 1e-5);
        assert!(iq.opposite_amplitude.iter().all(|v| *v < 1e-5));
        let max = iq
            .spectrum_dbfs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let hz = (max as f64 - iq.used_frames as f64 / 2.) * iq.bin_hz;
        assert!((hz - sign * 700.).abs() < iq.bin_hz);
    }
}
#[test]
fn dc_clipping_silence_and_explicit_scaling() {
    let mut r = signal(false, 1.);
    for pair in r.samples.chunks_exact_mut(2) {
        pair[0] += 0.4;
    }
    let a = analyze(&r, [700., 1900.]).unwrap();
    assert!((a.dc_magnitude - 0.4).abs() < 0.003);
    r.samples.fill(0.);
    assert!(analyze(&r, [700., 1900.]).unwrap().silent);
    r.samples[1] = 1.01;
    assert_eq!(
        analyze(&r, [700., 1900.]).unwrap().components_at_full_scale,
        1
    );
    let mut r = signal(true, 1.);
    let bits: Vec<_> = r.samples.iter().map(|v| v.to_bits()).collect();
    r.meta.units_per_full_scale = 2.;
    assert!((analyze(&r, [700., 1900.]).unwrap().tone_amplitude[0] - 0.1).abs() < 1e-5);
    assert_eq!(
        bits,
        r.samples.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}
#[test]
fn unsuitable_samples_are_preserved_not_sanitized() {
    let mut r = signal(false, 1.);
    r.samples[0] = f64::from_bits(0x7ff8000000000031);
    r.samples[1] = -0.;
    let mut bytes = Vec::new();
    write_record(&r, &mut bytes).unwrap();
    let restored = read_record(&bytes[..]).unwrap();
    assert_eq!(restored.samples[0].to_bits(), r.samples[0].to_bits());
    assert_eq!(restored.samples[1].to_bits(), (-0_f64).to_bits());
    assert!(analyze(&restored, [700., 1900.]).is_err());
    assert!(report_json(&restored, [700., 1900.]).contains("\"analysisAvailable\":false"));
    r.samples.pop();
    assert!(analyze(&r, [700., 1900.]).is_err());
}
#[test]
fn malformed_files_empty_evidence_and_invalid_metadata_fail() {
    assert!(read_record(&[][..]).is_err());
    let mut r = signal(false, 1.);
    let mut bytes = Vec::new();
    write_record(&r, &mut bytes).unwrap();
    bytes[108..112].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_record(&bytes[..]).is_err());
    r.meta.observed_last_ns = Some(400);
    assert!(analyze(&r, [700., 1900.]).is_err());
    r.meta.observed_last_ns = Some(200);
    r.meta.units_per_full_scale = 0.;
    assert!(analyze(&r, [700., 1900.]).is_err());
    r.samples.clear();
    assert!(analyze(&r, [700., 1900.]).is_err());
}
#[test]
fn saturated_queue_and_busy_consumer_do_not_wait_or_hide_drops() {
    let m = meta(Point::PreWdspPcm);
    let tap = CaptureTap::new(1, 64, 8, m).unwrap();
    assert!(tap.try_capture(m, &[0.5; 64]));
    assert!(!tap.try_capture(m, &[0.2; 64]));
    assert_eq!(tap.dropped(), 1);
    tap.consume_one(|r| {
        assert_eq!(r.samples[0], 0.5);
        assert!(!tap.try_capture(m, &[0.3; 64]));
    })
    .unwrap();
    assert_eq!(tap.dropped(), 2);
    assert!(tap.try_capture(m, &[0.4; 64]));
    let _guard = tap.0.slots.lock().unwrap();
    assert!(!tap.try_capture(m, &[0.; 64]));
    assert_eq!(tap.dropped(), 3);
}
#[test]
fn budget_generation_and_age_survive_delayed_analysis() {
    let m = meta(Point::PreWdspPcm);
    let tap = CaptureTap::new(2, 64, 2, m).unwrap();
    assert!(tap.try_capture(m, &[0.; 64]));
    let mut newer = m;
    newer.generation += 1;
    newer.recorded_ns = 800;
    assert!(tap.try_capture(newer, &[0.; 64]));
    tap.consume_one(|r| {
        assert_eq!(r.meta.generation, 11);
        assert_eq!(r.meta.observed_first_ns, Some(100));
        assert_eq!(r.meta.recorded_ns, 300);
    })
    .unwrap();
    assert!(!tap.try_capture(newer, &[0.; 64]));
    assert_eq!(tap.budget_exhausted(), 1);
    assert_eq!(tap.consume_one(|r| r.meta.generation), Some(12));
}
#[test]
fn no_event_or_keyed_snapshot_claims_physical_output_or_continuity() {
    let mut r = signal(true, 1.);
    r.meta.event = Event::OutputCompleted;
    r.meta.point = Point::Timeline;
    r.meta.keyed = Some(true);
    r.meta.requested_bytes = Some(512);
    r.meta.completed_bytes = Some(256);
    r.samples.clear();
    let json = report_json(&r, [700., 1900.]);
    for field in [
        "rfOutputVerified",
        "rfImdQualified",
        "continuousKeyedStateVerified",
        "deliveryAgeVerified",
    ] {
        assert!(json.contains(&format!("\"{field}\":false")));
    }
    assert!(json.contains("\"requestedBytes\":\"512\""));
    assert!(json.contains("\"completedBytes\":\"256\""));
}

#[test]
fn worker_analysis_does_not_hold_the_queue_lock() {
    let m = meta(Point::PreWdspPcm);
    let tap = CaptureTap::new(2, 64, 4, m).unwrap();
    assert!(tap.try_capture(m, &[0.; 64]));
    tap.consume_one(|_| {
        assert!(tap.try_capture(m, &[0.2; 64]));
    })
    .unwrap();
    assert_eq!(tap.dropped(), 0);
}

#[test]
fn unresolved_short_windows_and_future_partial_intervals_are_explicit() {
    let mut r = signal(false, 1.);
    r.samples.truncate(256);
    assert!(!analyze(&r, [700., 1900.]).unwrap().tone_estimates_resolved);
    r.meta.observed_last_ns = None;
    r.meta.observed_first_ns = Some(400);
    assert!(analyze(&r, [700., 1900.]).is_err());
}
