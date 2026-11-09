//! Explicit diagnostic admission through the actual scheduled owner, matched
//! WDSP and existing P2 / direct acquisition adapters. LOOPBACK/FILES ONLY.
use super::*;
use crate::{
    p2::session::managed_feedback::tests::Fixture as P2Fixture,
    tci::{ps_authorization::OperatorIngress, TciCommand},
    tx_audio::{TxAudioFrame, TxAudioSamples, TxAudioSource},
    tx_thread::{
        feedback_owner::Link,
        production_owner::integration_tests::Wave,
        ps_owner_loop::{OwnerLoop, Timing},
    },
    wdsp::ps_native::FullNative,
    xdma_rx::managed_feedback::tests as direct_rx,
    xdma_tx_radio::offline_fixture::DirectFixture,
};
use saturn_bridge::puresignal::{
    controller::{Config, Controller, Qualification},
    measurement::{CaptureContext, MeasurementRecorder, OverloadQualification},
    tx_lifecycle::{Limits, Phase, RunMode, TxLifecycle},
    Backend, Fault, OverloadSource, Owner,
};

fn recorder(capacity: usize) -> MeasurementRecorder {
    MeasurementRecorder::new(
        capacity,
        CaptureContext {
            source_pin: "synthetic-offline-fixture-not-hardware".into(),
            native_identity: format!(
                "WDSP211 archive={}",
                env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256")
            ),
            feedback_attenuation_db: Some(9),
            commanded_drive: Some(5),
            physical_adc: OverloadSource::Adc1,
        },
    )
    .unwrap()
}

#[test]
#[ignore = "serial matched-native synthetic diagnostic; loopback/ordinary files only"]
fn measurement_only_actual_acquisition_owner_native_both_backends() {
    run_measurement_only(false, false);
}

#[test]
#[ignore = "serial real-native explicit control-handler diagnostic; loopback/files only"]
fn measurement_only_actual_control_handler_native_both_backends() {
    run_measurement_only(true, false);
}

#[test]
#[ignore = "serial matched native capture through existing acquisition/output owners; loopback/files only"]
fn measurement_only_shared_signal_capture_actual_boundaries_both_backends() {
    run_measurement_only(true, true);
}

fn run_measurement_only(wire: bool, capture_signals: bool) {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    } // LOCAL fixture only
    for backend in [Backend::P2, Backend::DirectXdma] {
        for case in [
            "expiry",
            "release",
            "disconnect",
            "missing",
            "queue",
            "config",
            "native",
            "gap",
            "delayed-request",
            "cancel-queued",
            "disconnect-queued",
        ] {
            if capture_signals && case != "expiry" {
                continue;
            }
            // Recorder-capacity injection remains in the typed fixture. The
            // live entry uses a fixed bounded capacity, never a client value.
            if wire && case == "queue" {
                continue;
            }
            let p2 = (backend == Backend::P2).then(P2Fixture::new);
            let direct = (backend == Backend::DirectXdma).then(DirectFixture::new);
            let mut rx = direct.as_ref().map(|f| {
                f.fifo_ready();
                f.write(0x100c, 0x100000);
                f.write(0x1010, 1 << 30);
                f.write(0, 42);
                f.write(4, 43);
                direct_rx::session(&f.register_path())
            });
            let (mut iq, mut mic) = super::tests::queues();
            let mut m = super::tests::model();
            m.desired.tx_drive = 5;
            m.desired.pure_signal_enabled = false;
            m.desired.pure_signal_attenuation_db = 9;
            if backend == Backend::DirectXdma {
                m.desired.rx_ddc_index = 6;
                m.desired.ddc0_sample_rate_khz = 384;
            }
            let model = Arc::new(Mutex::new(m.clone()));
            let mut output = if let Some(p) = &p2 {
                OwnerStopAdapter::p2(p.session.clone(), &m, &mut iq, &mut mic).unwrap()
            } else {
                OwnerStopAdapter::direct(direct.as_ref().unwrap().radio.clone(), &mut iq, &mut mic)
                    .unwrap()
            };
            output.enable_offline_handoff(&m, 5).unwrap();
            let link = Link::new();
            output.attach_feedback_owner(link.clone(), 20).unwrap();
            let native = FullNative::matched(&m).unwrap();
            let controller = Controller::new(
                native,
                Owner::Bridge,
                backend,
                88,
                Qualification {
                    native_abi_verified: true,
                    feedback_topology_verified: true,
                },
                4,
            );
            let life = TxLifecycle::new(
                controller,
                output,
                backend,
                Limits {
                    authority_timeout_ns: 2_000_000_000,
                    cleanup_timeout_ns: 3_000_000_000,
                },
            )
            .unwrap();
            let origin = Instant::now();
            let issuer = OperatorIngress::fixture();
            let (clients, operator) = issuer.fixture_registry();
            let (mut r, ingress) = OwnerLoop::with_shared_ingress(
                life,
                issuer,
                model,
                origin,
                Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 2_000_000_000,
                },
            )
            .unwrap();
            let signal_dir = capture_signals.then(|| {
                super::super::tx_thread::ps_measurement::tests::directory().join("shared-signal")
            });
            let signal_worker = signal_dir
                .as_ref()
                .map(|path| crate::tx_signal_capture::shared::Worker::start(path, origin).unwrap());
            if let Some(w) = &signal_worker {
                r.attach_signal_capture(w.producer()).unwrap();
            }
            let export_directory = wire.then(crate::tx_thread::ps_measurement::tests::directory);
            if let Some(directory) = &export_directory {
                ingress
                    .enable_measurement_only(
                        crate::tx_thread::ps_measurement::Host::open(
                            directory,
                            m.desired.tx_frequency_hz,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            let (legacy, legacy_rx) = std::sync::mpsc::channel();
            let control_at = Arc::new(Mutex::new(None));
            let dispatch_as = |client, text: &str| {
                assert!(crate::tci::handle_incoming_message_shared(
                    tungstenite::Message::Text(text.into()),
                    &legacy,
                    &clients,
                    &operator,
                    &control_at,
                    client,
                    Some(&ingress)
                ));
            };
            let dispatch = |text: &str| dispatch_as(7, text);
            if let Some(p) = &p2 {
                p.session
                    .attach_managed_feedback(
                        link.clone(),
                        ingress.clone(),
                        Arc::new(Mutex::new(m.clone())),
                        origin,
                    )
                    .unwrap();
            }
            if let Some(rx) = &mut rx {
                rx.attach_managed_feedback(link.clone(), ingress.clone(), origin)
                    .unwrap();
            }
            let config = Config {
                id: 10,
                route: 20,
                physical_adc: OverloadSource::Adc1,
                enabled: false,
                automatic: false,
                hardware_peak: 0.6121,
                requested_delay_seconds: 0.,
                settling_ns: 0,
                loop_delay_seconds: 0.,
                stale_after_ns: 250_000_000,
            };
            ingress
                .bootstrap(m.clone(), config, TxAudioSource::Tci)
                .unwrap();
            let mut stage = 0;
            let mut active_at = None;
            let mut next_mic = 0;
            let mut next_feedback = 0;
            let mut wave = Wave::new();
            let mut sequence = 0;
            let mut injected = false;
            let mut records = 0;
            let mut fresh_rearm = false;
            let mut snapshot_json = None;
            let mut queued_at = None;
            let mut capture_id = 1;
            r.run(&AtomicBool::new(false), |r, now| {
                assert!(now < 8_000_000_000, "diagnostic timeout {backend:?} {case} {:?}", r.life.status());
                if let Some(p) = &p2 { p.service().unwrap(); p.drain_output(); }
                if let Some(rx) = &mut rx { direct_rx::service(rx).unwrap(); }
                let s = r.life.status();
                assert_eq!(s.correction_applied == Some(true), false, "diagnostic applied a curve");
                if stage == 0 && ingress.startup_ready() {
                    let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                    for mismatch in ["adc", "drive", "attenuation"] {
                        let mut context = recorder(4).context().clone();
                        match mismatch {
                            "adc" => context.physical_adc = OverloadSource::Adc2,
                            "drive" => context.commanded_drive = Some(6),
                            "attenuation" => context.feedback_attenuation_db = Some(10),
                            _ => unreachable!(),
                        }
                        assert_eq!(ingress.measurement_only(7, &capture, 600_000_000,
                            MeasurementRecorder::new(4, context).unwrap()), Err(Fault::InvalidConfig));
                        assert!(r.life.measurement_records().is_none());
                    }
                    // Ordinary trx is not sufficient with PS disabled.
                    assert!(!m.desired.pure_signal_enabled);
                    if wire {
                        dispatch("trx:0,true;");
                        assert!(legacy_rx.try_recv().is_err(), "diagnostic PTT escaped to legacy");
                        dispatch("saturn_ps_measurement:0,start,600;");
                    } else {
                        ingress.measurement_only(7, &capture, 600_000_000,
                            recorder(if case == "queue" { 1 } else { 256 })).unwrap();
                    }
                    queued_at = Some(now);
                    match case {
                        "delayed-request" => std::thread::sleep(Duration::from_millis(180)),
                        "cancel-queued" => ingress.release(),
                        "disconnect-queued" => { crate::tci::unregister_client(&clients, &operator, 7); }
                        _ => (),
                    }
                    stage = 1;
                }
                if stage == 1 && matches!(case, "delayed-request" | "cancel-queued" | "disconnect-queued") {
                    assert!(!matches!(s.phase, Phase::Active | Phase::AwaitingKey), "stale queued action armed");
                    if now > queued_at.unwrap() && s.phase == Phase::ReadyOff {
                        assert!(r.counters.rejected > 0, "queued action wasn't consumed/rejected");
                        assert_eq!(r.counters.iq_packets, 0);
                        assert!(r.life.measurement_records().is_none());
                        ingress.shutdown().unwrap(); stage = 2;
                        println!("MeasurementOnly {backend:?} case={case} queued_authority_rejected=true output_packets=0");
                    }
                }
                if stage == 1 && matches!(s.phase, Phase::AwaitingKey | Phase::Active) {
                    assert_eq!(s.mode, RunMode::MeasurementOnly);
                    if now >= next_mic {
                        let at = origin + Duration::from_nanos(now);
                        let capture = ingress.capture(at).unwrap();
                        ingress.enqueue_audio(&capture, TxAudioFrame {
                            enqueued_at: at, source: TxAudioSource::Tci, channels: 1, sample_rate_hz: 48000,
                            samples: TxAudioSamples::Dynamic((0..512).map(|i|
                                (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.2).collect()),
                        }).unwrap();
                        next_mic = now + 10_000_000;
                    }
                    if s.phase == Phase::Active {
                        if let Some(files) = &direct {
                            assert_eq!(files.read(crate::xdma::ALEX_RX_FILTER_REGISTER) & 0xffff,
                                0x1800, "actual MeasurementOnly key must select external RX BYPASS");
                        }
                        let start = *active_at.get_or_insert(now);
                        records = r.life.measurement_records().unwrap().records().len();
                        if case != "missing" && now >= next_feedback && !injected {
                            let scope = link.desired().scope.unwrap();
                            if let Some(p) = &p2 { p.receive(&wave.p2()); }
                            if let Some(rx) = &mut rx {
                                direct_rx::consume(rx, &wave.direct(), &mut sequence,
                                    Some(scope), origin + Duration::from_nanos(now));
                            }
                            next_feedback = now + 10_000_000;
                        }
                        if records > 0 && now - start >= 25_000_000 && !injected {
                            match case {
                                "release" => { ingress.release(); injected = true; }
                                "disconnect" => {
                                    crate::tci::unregister_client(&clients, &operator, 7);
                                    injected = true;
                                }
                                "config" => {
                                    let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                                    ingress.tci(7, Some(&capture), &TciCommand::SetTxMicGain(1.)).unwrap();
                                    injected = true;
                                }
                                "native" => {
                                    unsafe extern "C" { fn SetPSAutomode(channel: i32, value: i32); }
                                    unsafe { SetPSAutomode(1, 1); } // fixture fault, no native MOX
                                    injected = true;
                                }
                                "gap" => {
                                    if let Some(p) = &p2 {
                                        let mut bytes = wave.p2();
                                        let seq = u32::from_be_bytes(bytes[..4].try_into().unwrap());
                                        bytes[..4].copy_from_slice(&(seq + 1).to_be_bytes());
                                        p.receive(&bytes);
                                    }
                                    if let Some(rx) = &mut rx {
                                        let scope = link.desired().scope.unwrap();
                                        let mut bytes = wave.direct(); bytes[7] = 0;
                                        direct_rx::consume(rx, &bytes, &mut sequence, Some(scope), origin + Duration::from_nanos(now));
                                    }
                                    injected = true;
                                }
                                _ => (),
                            }
                        }
                    }
                }
                if stage == 1 && active_at.is_some() && s.phase == Phase::ReadyOff {
                    let expected_fault = match case {
                        "disconnect" => Some(Fault::TxAuthority),
                        "missing" => Some(Fault::Stale),
                        "queue" => Some(Fault::QueueOverrun),
                        "native" => Some(Fault::NativeFailure),
                        "gap" => Some(if backend == Backend::P2 { Fault::SequenceGap } else { Fault::DmaFraming }),
                        _ => None,
                    };
                    assert_eq!(s.primary_fault, expected_fault, "wrong stop reason for {case}");
                    assert!(link.restored().unwrap());
                    assert!(s.cleanup_completed_ns.is_some());
                    let exported = if wire {
                        assert!(r.life.measurement_records().is_none(), "completed evidence not handed off");
                        dispatch_as(if case == "disconnect" { 8 } else { 7 },
                            &format!("saturn_ps_measurement:0,export,{capture_id};"));
                        let path=export_directory.as_ref().unwrap().join(format!("measurement-{capture_id}-1.json"));
                        let json=std::fs::read_to_string(path).unwrap();
                        if case != "missing" { assert!(!json.contains("\"captures\":[]")); }
                        assert!(json.contains("\"externalCouplingDb\":null"));
                        json
                    } else {
                        let evidence = r.life.take_measurement_records().unwrap().unwrap();
                        if case != "missing" { assert!(!evidence.records().is_empty()); }
                        assert!(evidence.records().iter().all(|b| b.native_state.disabled_verified()
                            && b.overload == OverloadQualification::Unknown));
                        if case == "queue" { assert!(evidence.overflow_record().is_some()); }
                        evidence.export_json()
                    };
                    let audit = r.life.offline_native_snapshot(|n| n.audit());
                    assert_eq!(audit.feedback_blocks, 0, "diagnostic called pscc fitting");
                    assert!(audit.exchanges > 0 && audit.recreations >= 2);
                    assert!(r.counters.iq_packets > 0, "no actual backend output control");
                    snapshot_json = Some(exported);
                    let exported = snapshot_json.as_ref().unwrap();
                    assert!(exported.contains("\"captureFinished\":true"));
                    assert!(!exported.contains("\"cleanupCompletedNs\":null"));
                    let terminal = expected_fault.map_or_else(|| "null".into(), |f| format!("\"{f:?}\""));
                    assert!(exported.contains(&format!("\"terminalFault\":{terminal}")), "missing terminal cause: {exported}");
                    if expected_fault.is_some() {
                        assert!(exported.contains("\"recordingComplete\":false"));
                    }
                    if case == "release" && !fresh_rearm {
                        assert_eq!(s.primary_fault, None);
                        let capture = ingress.capture(origin + Duration::from_nanos(now)).unwrap();
                        if wire {
                            capture_id += 1;
                            dispatch("saturn_ps_measurement:0,start,200;");
                        } else {
                            ingress.measurement_only(7, &capture, 200_000_000, recorder(256)).unwrap();
                        }
                        fresh_rearm = true; active_at = None; injected = false;
                        next_mic = now; next_feedback = now; wave = Wave::new();
                    } else {
                        ingress.shutdown().unwrap(); stage = 2;
                    }
                }
            });
            assert_eq!(stage, 2);
            if let Some(w) = signal_worker {
                w.finish();
                let path = signal_dir.as_ref().unwrap();
                let mut native = false;
                let mut pairs = false;
                let mut completed = false;
                for entry in std::fs::read_dir(path).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension().is_some_and(|e| e == "json") {
                        let text = std::fs::read_to_string(&path).unwrap();
                        native |= text.contains("\"kind\":\"Wdsp\"");
                        pairs |= text.contains("\"kind\":\"Pairs\"");
                        completed |= text.contains("\"kind\":\"H2c\"")
                            && text.contains("\"keyedSnapshot\":true");
                    }
                }
                assert!(native && pairs, "missing actual capture boundary");
                assert_eq!(
                    completed,
                    backend == Backend::DirectXdma,
                    "P2 is not an H2C write"
                );
                println!("shared_signal_capture backend={backend:?} directory={} native=true pairs=true h2c={completed}",path.display());
                if let Some(root) = std::env::var_os("SATURN_SHARED_CAPTURE_FIXTURE_DIR") {
                    let destination = std::path::PathBuf::from(root).join(format!("{backend:?}"));
                    std::fs::create_dir(&destination).unwrap();
                    for entry in std::fs::read_dir(path).unwrap() {
                        let entry = entry.unwrap();
                        std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
                    }
                }
            }
            if !matches!(
                case,
                "delayed-request" | "cancel-queued" | "disconnect-queued"
            ) {
                assert!(snapshot_json.is_some());
            }
            if let Some(d) = &direct {
                d.assert_off();
            }
            // Optional preserved synthetic evidence, after completed cleanup.
            // Never overwrite an earlier run or write from an active TX turn.
            if let (false, Some(directory), Some(json)) = (
                wire,
                std::env::var_os("SATURN_MEASUREMENT_EVIDENCE_DIR"),
                snapshot_json,
            ) {
                use std::io::Write;
                let path =
                    std::path::PathBuf::from(directory).join(format!("{backend:?}-{case}.json"));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .unwrap();
                file.write_all(json.as_bytes()).unwrap();
            }
            println!("MeasurementOnly {backend:?} control_handler={wire} case={case} records={records} fit_calls=0 correction=false cleanup=complete fresh_rearm={fresh_rearm} unknown_overload=true synthetic_not_RF_IMD=true");
        }
    }
}
