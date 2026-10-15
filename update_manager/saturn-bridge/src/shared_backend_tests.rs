//! SAME run_inner orchestration with external loopback/file I/O only.
//! The independent client driver never services acquisition or the owner.
use super::*;
use crate::{
    p2::session::managed_feedback::tests::Fixture as P2Fixture,
    tx_thread::{
        feedback_owner::Link,
        production_owner::integration_tests::{Client, Wave},
        ps_ingress::SharedIngress,
    },
    xdma_rx::managed_feedback::tests as direct_rx,
    xdma_tx_radio::offline_fixture::DirectFixture,
};
use saturn_bridge::puresignal::{
    admission_proposal::DraftLimits,
    controller::Config,
    tx_lifecycle::{LifecycleStatus, Limits, Phase},
    OverloadSource,
};
use std::{
    net::{TcpListener, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    ShutdownActive,
    AcquisitionFailure,
    SatpBindFailure,
    AttachmentFailure,
    RestoreFailure,
    MeasurementClient,
}
struct Io {
    direct: bool,
    case: Case,
    address: std::net::SocketAddr,
    p2: Option<Arc<P2Fixture>>,
    normal_p2: Option<UdpSocket>,
    files: Option<DirectFixture>,
    stop: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
    ingress: Option<SharedIngress>,
    link: Option<Link>,
    driver: Option<thread::JoinHandle<usize>>,
    status: Option<LifecycleStatus>,
    wave: Wave,
    sequence: u64,
    restore_hold: Option<Instant>,
    held_turns: usize,
    received_turns: usize,
    injected: bool,
    expected_rx: Option<crate::rx_state::RxSettings>,
    measurement_last_receive: Instant,
}
fn options() -> Options {
    Options {
        config: Config {
            id: 1,
            route: 20,
            physical_adc: OverloadSource::Adc1,
            enabled: false,
            automatic: false,
            hardware_peak: 0.6121,
            requested_delay_seconds: 0.,
            loop_delay_seconds: 0.,
            settling_ns: 0,
            stale_after_ns: 250_000_000,
        },
        timing: crate::tx_thread::ps_owner_loop::Timing {
            mic_age_ns: 150_000_000,
            source_stall_ns: 500_000_000,
            tx_duration_ns: 20_000_000_000,
        },
        limits: Limits {
            authority_timeout_ns: 2_000_000_000,
            cleanup_timeout_ns: 3_000_000_000,
        },
        max_watts: 5,
        feedback_topology_reviewed: true,
        managed_rf_permitted: true,
        startup_timeout_ns: 10_000_000_000,
        feedback_attenuation_db: 9,
        measurement_only: false,
        // Synthetic fixture only. These are NOT approved G2 feedback limits.
        operational_policy: Some(DraftLimits {
            active_reference_fraction: 0.1,
            feedback_rms_floor: 1e-6,
            component_limit: 8388607. / 8388608.,
        }),
    }
}
impl RuntimeIo for Io {
    fn open(
        &mut self,
        config: &mut BridgeConfig,
        backend: RadioBackend,
        model: &Arc<Mutex<RadioModel>>,
    ) -> Result<(Radio, Acquisition), Box<dyn Error>> {
        if let Some(expected) = &self.expected_rx {
            let initial = model.lock_unpoisoned();
            assert_eq!(&crate::rx_state::capture(&initial), expected);
            assert_eq!(config.ddc0_frequency_hz, initial.desired.iq_center_hz);
            assert!(!initial.desired.tx_enabled && initial.desired.tx_phase == TxPhase::Rx);
            assert!(!initial.desired.pure_signal_enabled);
            if backend == RadioBackend::Xdma {
                assert_eq!(initial.desired.rx_ddc_index, 6);
            }
        }
        if self.direct {
            let f = DirectFixture::new();
            f.fifo_ready();
            f.write(0x100c, 0x100000);
            f.write(0x1010, 1 << 30);
            f.write(0, 42);
            f.write(4, 43);
            if self.case == Case::AttachmentFailure {
                f.write(0x4004, 0x53460003);
            }
            let rx = direct_rx::session(&f.register_path());
            let radio = Radio::Direct(f.radio.clone());
            self.files = Some(f);
            Ok((radio, Acquisition::Direct(rx)))
        } else {
            let (p, normal) = loop {
                let p = Arc::new(P2Fixture::new());
                let Some(port) = p.feedback.local_addr()?.port().checked_add(2) else {
                    continue;
                };
                if let Ok(normal) = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)) {
                    break (p, normal);
                }
            };
            self.normal_p2 = Some(normal);
            let radio = Radio::P2(p.session.clone());
            self.p2 = Some(p.clone());
            Ok((radio, Acquisition::P2(p.session.clone())))
        }
    }
    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    fn started(&mut self, runtime: &production_owner::Runtime, _: &Arc<TciFrontend>) {
        self.ingress = Some(runtime.ingress.clone());
        self.link = Some(runtime.link.clone());
        let check_persistence = self.expected_rx.is_some();
        let (ingress, link, stop, fail, joined, p2, address, case) = (
            runtime.ingress.clone(),
            runtime.link.clone(),
            self.stop.clone(),
            self.fail.clone(),
            self.joined.clone(),
            self.p2.clone(),
            self.address,
            self.case,
        );
        self.driver = Some(thread::spawn(move || {
            struct StopOnExit(Arc<AtomicBool>);
            impl Drop for StopOnExit {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _stop_on_exit = StopOnExit(stop.clone());
            let end = Instant::now() + Duration::from_secs(25);
            let mut client: Option<Client> = None;
            let mut wave = Wave::new();
            let mut next_mic = Instant::now();
            let mut next_feedback = Instant::now();
            if case == Case::MeasurementClient {
                while !ingress.startup_ready() {
                    assert!(Instant::now() < end, "diagnostic startup timeout");
                    thread::sleep(Duration::from_millis(1));
                }
                let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts/ps-measurement-client.mjs");
                let mut child = std::process::Command::new("node")
                    .arg(script)
                    .args([
                        "--run",
                        "--confirm-ant1-dummy-load-lp700-attended",
                        "--duration-ms=500",
                    ])
                    .arg(format!("--url=ws://{address}/tci"))
                    .arg(format!("--source-pin={}", env!("SATURN_BRIDGE_SOURCE_PIN")))
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                loop {
                    let status = ingress.lifecycle_status().unwrap();
                    assert_ne!(status.correction_applied, Some(true));
                    if let Some(p) = &p2 {
                        if status.phase == Phase::Active && Instant::now() >= next_feedback {
                            p.feedback.send_to(&wave.p2(), p.destination).unwrap();
                            next_feedback = Instant::now() + Duration::from_millis(2);
                        }
                    }
                    if child.try_wait().unwrap().is_some() {
                        break;
                    }
                    if Instant::now() >= end {
                        child.kill().unwrap(); // LOCAL Node fixture only, never a radio/native owner
                        panic!("diagnostic client exceeded offline test bound");
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                let output = child.wait_with_output().unwrap();
                println!(
                    "diagnostic client stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(output.status.success(), "diagnostic Node client failed");
                assert!(String::from_utf8_lossy(&output.stdout).contains("\"event\":\"exported\""));
                assert!(link.restored().unwrap());
                stop.store(true, Ordering::Release);
                while !joined.load(Ordering::Acquire) {
                    assert!(Instant::now() < end);
                    thread::sleep(Duration::from_millis(1));
                }
                return 0; // diagnostic capture is not a corrected generation
            }
            let mut stage = 0;
            let mut corrected = 0;
            while !joined.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < end,
                    "runtime client {case:?} stage={stage} {:?}",
                    ingress.lifecycle_status()
                );
                if matches!(case, Case::SatpBindFailure | Case::AttachmentFailure) {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                if let Some(c) = &mut client {
                    c.drain();
                }
                let status = ingress.lifecycle_status().unwrap();
                match stage {
                    0 if ingress.startup_ready() => {
                        // run_inner alone opens admission. No test-side admit.
                        let mut c = Client::open_unpaired(address);
                        if check_persistence {
                            // Actual ingress settings, not a direct test-side model edit.
                            c.text("vfo:0,0,7300000;");
                        }
                        c.text("iq_start:0;audio_start:0;");
                        client = Some(c);
                        stage = 1;
                    }
                    1 if client.as_ref().unwrap().iq_frames > 0
                        && client.as_ref().unwrap().audio_frames > 0 =>
                    {
                        println!(
                            "run_inner received IQ and audio: iq_frames={} audio_frames={}",
                            client.as_ref().unwrap().iq_frames,
                            client.as_ref().unwrap().audio_frames
                        );
                        client.as_mut().unwrap().text("tx_puresignal:0,true;");
                        stage = 2;
                    }
                    2 if ingress.startup_ready()
                        && client
                            .as_ref()
                            .unwrap()
                            .transactions
                            .iter()
                            .any(|s| s == "tx_config:0,1,applied,2;") =>
                    {
                        client.as_mut().unwrap().text("trx:0,true;");
                        stage = 3;
                    }
                    3 | 5
                        if status.phase == Phase::Active
                            && status.correction_applied == Some(true) =>
                    {
                        corrected += 1;
                        if stage == 3 && case == Case::ShutdownActive {
                            client.as_mut().unwrap().text("trx:0,false;");
                            stage = 4;
                        } else {
                            if case == Case::AcquisitionFailure {
                                fail.store(true, Ordering::Release);
                            } else {
                                stop.store(true, Ordering::Release);
                            }
                            stage = 6;
                        }
                    }
                    4 if ingress.startup_ready() => {
                        assert!(link.restored().unwrap());
                        assert!(
                            status.authority_completed_ns.is_some()
                                && status.cleanup_completed_ns.is_some()
                        );
                        wave = Wave::new();
                        client.as_mut().unwrap().text("trx:0,true;");
                        stage = 5;
                    }
                    _ => {}
                }
                if matches!(stage, 3 | 5) {
                    if let Some(p) = &p2 {
                        if status.phase == Phase::Active && Instant::now() >= next_feedback {
                            p.feedback.send_to(&wave.p2(), p.destination).unwrap();
                            next_feedback = Instant::now() + Duration::from_millis(2);
                        }
                    }
                    if Instant::now() >= next_mic {
                        client.as_mut().unwrap().mic();
                        next_mic = Instant::now() + Duration::from_nanos(512_000_000_000 / 48_000);
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
            let startup_failure = matches!(case, Case::SatpBindFailure | Case::AttachmentFailure);
            assert_eq!(stage, if startup_failure { 0 } else { 6 });
            assert_eq!(
                corrected,
                if startup_failure {
                    0
                } else if case == Case::ShutdownActive {
                    2
                } else {
                    1
                }
            );
            corrected
        }));
    }
    fn before_receive(&mut self, acquisition: &mut Acquisition) -> Result<bool, Box<dyn Error>> {
        let ingress = self.ingress.as_ref().unwrap();
        let link = self.link.as_ref().unwrap();
        if self.fail.swap(false, Ordering::AcqRel) && !self.injected {
            self.injected = true;
            return Err(std::io::Error::other("injected acquisition read failure").into());
        }
        let request = link.desired();
        if self.case == Case::MeasurementClient && request.scope.is_some() {
            if self.measurement_last_receive.elapsed() < Duration::from_millis(4) {
                return Ok(false);
            }
            self.measurement_last_receive = Instant::now();
        }
        if self.case != Case::SatpBindFailure
            && request.serial > 0
            && request.scope.is_none()
            && (self.stop_requested() || self.injected)
            && !link.restored().unwrap_or(false)
        {
            if self.case == Case::RestoreFailure {
                if !self.injected {
                    let Acquisition::Direct(rx) = acquisition else {
                        unreachable!();
                    };
                    direct_rx::corrupt_next_layout_readback(
                        rx,
                        &self.files.as_ref().unwrap().register_path(),
                    );
                    self.injected = true;
                }
            } else {
                let until = *self
                    .restore_hold
                    .get_or_insert(Instant::now() + Duration::from_millis(80));
                if Instant::now() < until {
                    self.held_turns += 1;
                    assert!(ingress
                        .lifecycle_status()
                        .unwrap()
                        .cleanup_completed_ns
                        .is_none());
                    assert!(ingress
                        .lifecycle_status()
                        .unwrap()
                        .authority_completed_ns
                        .is_none());
                    return Ok(false); // external I/O temporarily unavailable; lifecycle keeps ticking
                }
            }
        }
        match acquisition {
            Acquisition::P2(p) => {
                self.p2.as_ref().unwrap().drain_output();
                // Normal receiver source uses the existing socket decoder.
                if request.scope.is_some() {
                    return Ok(true);
                }
                let normal = self.normal_p2.as_ref().unwrap();
                let mut packet = vec![0u8; 1444];
                packet[..4].copy_from_slice(&(self.sequence as u32).to_be_bytes());
                packet[12..14].copy_from_slice(&24u16.to_be_bytes());
                packet[14..16].copy_from_slice(&238u16.to_be_bytes());
                normal.send_to(&packet, self.p2.as_ref().unwrap().destination)?;
                let _ = p;
            }
            Acquisition::Direct(rx) => {
                let mut frame = vec![0u8; 72]; // DDC6 x8 complex words @384k
                frame[..4].copy_from_slice(&0x100000u32.to_le_bytes());
                frame[7] = 0x80;
                let normal = frame.repeat(128);
                let paired = self.wave.direct();
                direct_rx::reader_turn(rx, &normal, &paired[..136 * 128], self.sequence);
            }
        }
        self.sequence += 1;
        self.received_turns += 1;
        Ok(true)
    }
    fn joined(&mut self, runtime: &production_owner::Runtime) {
        assert!(
            runtime.worker.is_none(),
            "owner must be joined before result publication"
        );
        self.status = runtime.ingress.lifecycle_status();
        self.joined.store(true, Ordering::Release);
    }
}
fn exercise(direct: bool, case: Case) {
    exercise_with_state(direct, case, false);
}
fn exercise_with_state(direct: bool, case: Case, persistent: bool) {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    } // test-only; no hardware latency claim
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let occupied = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = BridgeConfig {
        tci_bind_addr: address,
        ddc0_frequency_hz: 7_200_000,
        remote_tx_rf_enabled: true,
        satp_enabled: case == Case::SatpBindFailure,
        satp_bind_addr: occupied.local_addr().unwrap(),
        ..BridgeConfig::default()
    };
    let mut io = Io {
        direct,
        case,
        address,
        p2: None,
        normal_p2: None,
        files: None,
        stop: Arc::new(AtomicBool::new(false)),
        fail: Arc::new(AtomicBool::new(false)),
        joined: Arc::new(AtomicBool::new(false)),
        ingress: None,
        link: None,
        driver: None,
        status: None,
        wave: Wave::new(),
        sequence: 0,
        restore_hold: None,
        held_turns: 0,
        received_turns: 0,
        injected: false,
        expected_rx: None,
        measurement_last_receive: Instant::now(),
    };
    let directory = std::env::temp_dir().join(format!(
        "saturn-runtime-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let store = crate::rx_state::RxStateStore::new(directory.join("receive-state"));
    if persistent {
        let mut saved = crate::tx_owner_stop::tests::model();
        saved.desired.vfo_a_hz = 7_210_000;
        saved.desired.vfo_b_hz = 7_215_000;
        saved.desired.rx_antenna = 2;
        saved.desired.rx_attenuation_db = 12;
        saved.sync_vfo_routes();
        let settings = crate::rx_state::capture(&saved);
        store.save(&settings).unwrap();
        io.expected_rx = Some(settings);
    }
    let mut selected_options = options();
    if case == Case::MeasurementClient {
        selected_options.measurement_only = true;
        selected_options.operational_policy = None;
        selected_options.timing.tx_duration_ns = 1_000_000_000;
        let captures = crate::tx_thread::ps_measurement::tests::directory();
        // Explicit isolated ignored-test process; no live service reads these.
        unsafe {
            std::env::set_var("SATURN_PS_MEASUREMENT_DIR", &captures);
            std::env::set_var("SATURN_PS_MEASUREMENT_FREQUENCY_HZ", "7200000");
        }
    }
    let result = run_inner_with_state_store(
        config,
        if direct {
            RadioBackend::Xdma
        } else {
            RadioBackend::P2
        },
        selected_options,
        &directory.join("ready.json"),
        &directory.join("perf.json"),
        &mut io,
        persistent.then(|| store.clone()),
    );
    assert!(io.joined.load(Ordering::Acquire));
    let corrected = io.driver.take().unwrap().join().unwrap();
    if persistent {
        let restored = store.load().unwrap().unwrap().fields;
        assert_eq!(restored.vfo_a_hz, Some(7_300_000));
        assert_eq!(restored.vfo_b_hz, Some(7_215_000));
        assert_eq!(restored.rx_antenna, Some(2));
        assert_eq!(restored.rx_attenuation_db, Some(12));
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("tx_enabled") && !text.contains("pure_signal"));
    }
    if let Some(p) = &io.p2 {
        let mut bytes = [0u8; 2048];
        let mut last = None;
        while let Ok(size) = p.hp.recv(&mut bytes) {
            assert!(size > 4);
            last = Some(bytes[4] & 3);
        }
        assert_eq!(
            last,
            Some(0),
            "actual runtime final P2 command clears RUN and MOX"
        );
    }
    let status = io.status.unwrap();
    assert_eq!(
        result.is_ok(),
        matches!(case, Case::ShutdownActive | Case::MeasurementClient),
        "{case:?} {result:?} {status:?}"
    );
    if case == Case::RestoreFailure {
        assert!(result
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("readback mismatch"));
        assert_eq!(status.phase, Phase::Blocked);
        assert!(status.authority_fault.is_some() || status.cleanup_fault.is_some());
        assert!(status.cleanup_completed_ns.is_none());
        assert!(io.link.as_ref().unwrap().restored().is_err());
        let f = io.files.as_ref().unwrap();
        f.assert_off();
        assert_eq!(
            f.read(0x100c),
            0x100000,
            "best effort restore is not certified completion"
        );
    } else {
        assert_eq!(status.phase, Phase::ReadyOff, "{status:?}");
        assert!(status.authority_completed_ns.is_some() && status.cleanup_completed_ns.is_some());
        assert!(io.link.as_ref().unwrap().restored().unwrap());
        if !matches!(
            case,
            Case::SatpBindFailure | Case::AttachmentFailure | Case::MeasurementClient
        ) {
            assert!(io.held_turns > 2 && io.received_turns > 10);
        }
        if let Some(f) = &io.files {
            f.assert_off();
            assert_eq!(f.read(0x100c), 0x100000);
            assert_eq!((f.read(0), f.read(4)), (42, 43));
        }
    }
    if case == Case::AcquisitionFailure {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("injected acquisition read failure"));
    }
    println!("run_inner backend={} case={case:?} corrected_generations={corrected} measurement_captures={} receive_turns={} restoration_wait_turns={} owner_joined=true phase={:?} physical_rf_qualified=false", if direct {"DirectXdma"} else {"P2"}, usize::from(case == Case::MeasurementClient), io.received_turns, io.held_turns, status.phase);
}
#[test]
#[ignore = "serial actual runtime + Node tagged-PCM MeasurementOnly client; loopback/files only"]
fn actual_run_inner_measurement_client_both_backends() {
    assert!(
        env!("SATURN_BRIDGE_SOURCE_PIN").len() == 64,
        "compile with explicit offline source-content pin"
    );
    for direct in [false, true] {
        exercise(direct, Case::MeasurementClient);
    }
}
#[test]
#[ignore = "serial matched-native actual runtime orchestration; loopback/files only"]
fn actual_run_inner_receive_release_rearm_active_shutdown_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::ShutdownActive);
    }
}
#[test]
#[ignore = "serial matched-native receive-state integration; loopback/files only"]
fn actual_run_inner_preserves_receive_settings_both_backends() {
    for direct in [false, true] {
        exercise_with_state(direct, Case::ShutdownActive, true);
    }
}
#[test]
#[ignore = "serial matched-native receive-state error flush; loopback/files only"]
fn actual_run_inner_acquisition_failure_flushes_receive_settings_both_backends() {
    for direct in [false, true] {
        exercise_with_state(direct, Case::AcquisitionFailure, true);
    }
}
#[test]
#[ignore = "serial matched-native actual runtime orchestration; loopback/files only"]
fn actual_run_inner_acquisition_error_restores_and_joins_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::AcquisitionFailure);
    }
}
#[test]
#[ignore = "serial matched-native actual runtime orchestration; loopback/files only"]
fn actual_run_inner_satp_startup_error_stops_and_joins_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::SatpBindFailure);
    }
}
#[test]
#[ignore = "serial matched-native actual runtime attachment failure; ordinary files only"]
fn actual_run_inner_attachment_error_stops_and_joins() {
    exercise(true, Case::AttachmentFailure);
}
#[test]
#[ignore = "serial matched-native constructor failure; no hardware"]
fn actual_production_spawn_native_lease_failure_is_joined() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    let initial = crate::tx_owner_stop::tests::model();
    let held = crate::wdsp::WdspTxEngine::try_new(&initial).unwrap();
    let model = Arc::new(Mutex::new(initial.clone()));
    let config = BridgeConfig {
        tci_bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..BridgeConfig::default()
    };
    let (front, _) = TciFrontend::reserve_shared_startup(&config, model.clone(), false).unwrap();
    let fixture = P2Fixture::new();
    let result = production_owner::spawn(
        Radio::P2(fixture.session.clone()),
        Arc::new(front),
        model,
        config.tx_audio_source,
        options(),
        false,
        Duration::from_millis(20),
    );
    assert!(matches!(result, Err(Fault::ExternalController)));
    drop(held);
    let fresh = crate::wdsp::WdspTxEngine::try_new(&initial).unwrap();
    drop(fresh);
    println!("production spawn native lease refused; failure path joined; fresh native lease available; no client admission or RF");
}
#[test]
#[ignore = "isolated matched-native failure test; uncertain native retained, never reported clean"]
fn actual_run_inner_failed_restoration_cannot_report_clean_or_detach_owner() {
    exercise(true, Case::RestoreFailure);
}
