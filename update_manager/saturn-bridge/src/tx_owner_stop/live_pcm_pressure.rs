//! Actual bounded ingress and owner adapters; loopback/ordinary files only.
use super::*;

fn exercise<N: ScheduledNative>(
    backend: Backend,
    case: &str,
    native: impl FnOnce(&RadioModel) -> N,
) {
    let p2 = (backend == Backend::P2).then(super::super::tests::p2_separate_lanes);
    if let Some((_, hp, iq)) = &p2 {
        hp.set_nonblocking(true).unwrap();
        iq.set_nonblocking(true).unwrap();
    }
    let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
    if let Some(d) = &direct {
        d.fifo_ready();
    }
    let (mut iq, mut mic) = queues();
    let mut m = model();
    m.desired.tx_drive = 100;
    m.desired.vfo_a_hz = 27_385_000;
    m.desired.tx_frequency_hz = 27_385_000;
    m.desired.tx_pa_calibration =
        crate::pa_calibration::PaCalibration::Bands(crate::pa_calibration::G2_STARTING_GAINS);
    m.desired.pure_signal_enabled = false;
    m.sync_vfo_routes();
    let mut authority = if let Some((s, _, _)) = &p2 {
        OwnerStopAdapter::p2(s.clone(), &m, &mut iq, &mut mic).unwrap()
    } else {
        OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic).unwrap()
    };
    authority.enable_offline_handoff(&m, 100).unwrap();
    let life = controller(native(&m), authority, backend);
    let origin = Instant::now();
    let issuer = OperatorIngress::fixture();
    let (clients, _operator) = issuer.fixture_registry();
    let outbound = clients.lock().unwrap().get(&7).unwrap().outbound.clone();
    let (mut r, ingress) = OwnerLoop::with_shared_ingress(
        life,
        issuer,
        Arc::new(Mutex::new(m.clone())),
        origin,
        Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 3_000_000_000,
        },
    )
    .unwrap();
    ingress.set_drive_ceiling(100).unwrap();
    let mut stage = 0;
    let mut next_audio = 0;
    let mut at = 0;
    let mut packets = 0;
    let mut old_capture = None;
    let mut rearmed = false;
    let frame = || TxAudioFrame {
        enqueued_at: Instant::now(),
        source: TxAudioSource::Tci,
        samples: TxAudioSamples::Dynamic(
            (0..512)
                .map(|i| 0.4 * (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin())
                .collect(),
        ),
        channels: 1,
        sample_rate_hz: 48000,
    };
    r.run(&AtomicBool::new(false), |r, now| {
        while outbound.next_message(false).is_some() {}
        assert!(
            now < 3_000_000_000,
            "{backend:?} {case} stage={stage} {:?}",
            r.life.status()
        );
        if let Some((_, hp, iq)) = &p2 {
            let mut b = [0; 2048];
            while hp.recv(&mut b).is_ok() {}
            while iq.recv(&mut b).is_ok() {}
        }
        match stage {
            0 if r.life.status().phase == Phase::ReadyOff => {
                let mut config = cfg(11);
                config.enabled = false;
                config.automatic = false;
                ingress
                    .bootstrap(m.clone(), config, TxAudioSource::Tci)
                    .unwrap();
                stage = 1;
            }
            1 if ingress.startup_ready() => {
                let c = ingress.capture(Instant::now()).unwrap();
                ingress
                    .tci(7, Some(&c), &crate::tci::TciCommand::SetTxEnabled(true))
                    .unwrap();
                stage = 2;
            }
            2 if r.life.status().phase == Phase::Active => {
                let c = ingress.capture(Instant::now()).unwrap();
                packets = r.counters.iq_packets;
                // Keep the consumer paused while 48 real ingress blocks arrive.
                // This deterministically exceeds the unchanged 16-slot bound.
                for _ in 0..48 {
                    ingress.enqueue_audio(&c, frame()).unwrap();
                }
                assert!(ingress.audio_drop_count() >= 32);
                old_capture = Some(c);
                at = now;
                stage = 3;
                if case == "release" {
                    ingress.release();
                }
                if case == "stale" {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            3 if matches!(case, "healthy" | "old-generation") && now - at > 250_000_000 => {
                assert_eq!(r.life.status().phase, Phase::Active);
                assert_eq!(r.life.status().primary_fault, None);
                assert!(
                    r.counters.iq_packets > packets + 20,
                    "nonvacuous resumed output"
                );
                ingress.release();
                stage = 4;
            }
            3 if matches!(case, "release" | "stale")
                && r.life.status().phase == Phase::ReadyOff =>
            {
                stage = 4;
            }
            4 if r.life.status().phase == Phase::ReadyOff => {
                assert!(r.life.status().authority_completed_ns.is_some());
                assert!(r.life.status().cleanup_completed_ns.is_some());
                assert_eq!(
                    r.life.status().primary_fault,
                    if case == "stale" {
                        Some(Fault::Stale)
                    } else {
                        None
                    }
                );
                if case == "old-generation" && !rearmed {
                    rearmed = true;
                    let c = ingress.capture(Instant::now()).unwrap();
                    ingress
                        .tci(7, Some(&c), &crate::tci::TciCommand::SetTxEnabled(true))
                        .unwrap();
                    stage = 5;
                } else {
                    ingress.shutdown().unwrap();
                    stage = 7;
                }
            }
            5 if r.life.status().phase == Phase::Active => {
                let drops = ingress.audio_drop_count();
                assert_eq!(
                    ingress
                        .enqueue_audio(old_capture.as_ref().unwrap(), frame())
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::PermissionDenied
                );
                assert_eq!(ingress.audio_drop_count(), drops);
                assert_eq!(r.life.status().primary_fault, None);
                packets = r.counters.iq_packets;
                at = now;
                stage = 6;
            }
            6 if now - at > 50_000_000 => {
                assert!(r.counters.iq_packets > packets);
                assert_eq!(r.life.status().primary_fault, None);
                ingress.release();
                stage = 4;
            }
            _ => {}
        }
        if matches!(r.life.status().phase, Phase::AwaitingKey | Phase::Active)
            && now >= next_audio
            && (stage == 2
                || (stage == 3 && matches!(case, "healthy" | "old-generation"))
                || matches!(stage, 5 | 6))
        {
            let c = ingress.capture(Instant::now()).unwrap();
            ingress.enqueue_audio(&c, frame()).unwrap();
            next_audio = now + 10_666_667;
        }
    });
    assert_eq!(stage, 7);
    assert!(r.life.status().cleanup_completed_ns.is_some());
    if let Some(d) = direct {
        d.assert_off();
    }
    eprintln!("live_pcm_pressure backend={backend:?} case={case} drops={} cleanup=complete physical_rf=false",ingress.audio_drop_count());
}

#[test]
fn live_pcm_pressure_recovers_and_preserves_release_age_and_generation_both_backends() {
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in ["healthy", "release", "stale", "old-generation"] {
            exercise(backend, case, |_| Fake {
                trace: Rc::new(RefCell::new(Trace::default())),
            });
        }
    }
}

#[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
#[test]
#[ignore = "serial real WDSP 100-commanded-W voice PCM burst, file/loopback only; no RF"]
fn live_pcm_pressure_real_wdsp_output_and_cleanup_both_backends() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in ["healthy", "release", "old-generation"] {
            exercise(backend, case, |m| {
                crate::wdsp::ps_native::FullNative::matched(m).unwrap()
            });
        }
    }
}
