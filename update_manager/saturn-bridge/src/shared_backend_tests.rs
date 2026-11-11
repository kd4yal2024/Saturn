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
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
};
#[path = "shared_backend_satp_tests.rs"]
mod satp_runtime;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    BaselineTwoTone,
    NativeKeepaliveGap,
    NativeKeepaliveMissing,
    AutoCorrection,
    AutoAttSingle,
    AutoAttLimit,
    AutoAttRelease,
    AutoAttMissing,
    AutoAttReconfigure,
    AutoAttDisconnect,
    AutoAttRange,
    AutoSustained,
    ReceiveMeter,
    BandRecall,
    FaultedReceiveTune,
    ReceiveSpectrum,
    ShutdownActive,
    RearPtt,
    RearPttBypass,
    MicrophoneBypass,
    AcquisitionFailure,
    SatpBindFailure,
    SatpPairing,
    AttachmentFailure,
    RestoreFailure,
    MeasurementClient,
    BrowserMeasurementClient,
    BrowserLongMeasurementClient,
    BrowserTimingMeasurementClient,
    StimulusClient,
}
impl Case {
    fn auto_attenuation(self) -> bool {
        matches!(
            self,
            Self::AutoCorrection
                | Self::AutoAttSingle
                | Self::AutoAttLimit
                | Self::AutoAttRelease
                | Self::AutoAttMissing
                | Self::AutoAttReconfigure
                | Self::AutoAttDisconnect
                | Self::AutoAttRange
        )
    }
    fn rear_ptt(self) -> bool {
        matches!(self, Self::RearPtt | Self::RearPttBypass)
    }
    fn microphone_bypass(self) -> bool {
        matches!(self, Self::RearPttBypass | Self::MicrophoneBypass)
    }
    fn native_two_tone(self) -> bool {
        matches!(
            self,
            Self::BaselineTwoTone | Self::NativeKeepaliveGap | Self::NativeKeepaliveMissing
        )
    }
    fn diagnostic(self) -> bool {
        matches!(
            self,
            Self::MeasurementClient
                | Self::BrowserMeasurementClient
                | Self::BrowserLongMeasurementClient
                | Self::BrowserTimingMeasurementClient
                | Self::StimulusClient
        )
    }
    fn long_diagnostic(self) -> bool {
        matches!(
            self,
            Self::StimulusClient | Self::BrowserLongMeasurementClient
        )
    }
}
struct Io {
    direct: bool,
    case: Case,
    address: std::net::SocketAddr,
    satp_address: std::net::SocketAddr,
    perf_path: std::path::PathBuf,
    p2: Option<Arc<P2Fixture>>,
    normal_p2: Option<UdpSocket>,
    files: Option<DirectFixture>,
    stop: Arc<AtomicBool>,
    stop_baseline: Arc<AtomicU64>,
    fail: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
    ingress: Option<SharedIngress>,
    link: Option<Link>,
    driver: Option<thread::JoinHandle<usize>>,
    status: Option<LifecycleStatus>,
    state_model: Option<Arc<Mutex<RadioModel>>>,
    wave: Wave,
    sequence: u64,
    restore_hold: Option<Instant>,
    held_turns: usize,
    pending_stop_checks: usize,
    received_turns: usize,
    injected: bool,
    expected_rx: Option<crate::rx_state::RxSettings>,
    measurement_last_receive: Instant,
    dma_burst_stage: usize,
    dma_burst_complete: Arc<AtomicBool>,
    output_packets: Arc<AtomicU64>,
    hardware_down: Arc<AtomicBool>,
    hardware_sent: Instant,
    auto_drop_feedback: Arc<AtomicBool>,
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
        generated_trials_only: false,
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
        self.state_model = Some(model.clone());
        model.lock_unpoisoned().desired.pure_signal_auto_attenuate = false; // each servo case enables it through actual TCI
        if self.direct && matches!(self.case, Case::MicrophoneBypass | Case::ReceiveMeter) {
            let mut m = model.lock_unpoisoned();
            m.desired.tx_monitor_available = true;
            m.desired.rx_headphones_available = true;
        }
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
            if self.case.diagnostic() {
                f.write(0x9004, 0);
                f.enable_completed_write_fifo();
            } else {
                f.fifo_ready();
            }
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
        if std::env::var("SATURN_PS_GENERATED_OPERATIONAL").as_deref() == Ok("1") {
            assert_eq!(
                runtime.ingress.two_tone_caps().as_deref(),
                Some("saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,100;")
            );
        }
        self.link = Some(runtime.link.clone());
        let state_model = self.state_model.as_ref().unwrap().clone();
        let direct_register_path = self.files.as_ref().map(|f| f.register_path());
        let check_persistence = self.expected_rx.is_some();
        let direct = self.direct;
        let perf_path = self.perf_path.clone();
        let dma_burst_complete = self.dma_burst_complete.clone();
        let output_packets = self.output_packets.clone();
        let hardware_down = self.hardware_down.clone();
        let (ingress, link, stop, stop_baseline, fail, joined, p2, address, case) = (
            runtime.ingress.clone(),
            runtime.link.clone(),
            self.stop.clone(),
            self.stop_baseline.clone(),
            self.fail.clone(),
            self.joined.clone(),
            self.p2.clone(),
            self.address,
            self.case,
        );
        let drop_feedback = self.auto_drop_feedback.clone();
        let satp_address = self.satp_address;
        self.driver = Some(thread::spawn(move || {
            struct StopOnExit(Arc<AtomicBool>);
            impl Drop for StopOnExit {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _stop_on_exit = StopOnExit(stop.clone());
            let end = Instant::now()
                + Duration::from_secs(if case == Case::BrowserLongMeasurementClient {
                    125
                } else if case.long_diagnostic() || case == Case::AutoSustained {
                    65
                } else {
                    25
                });
            let mut client: Option<Client> = None;
            let mut connected_at = None;
            let driver_started = Instant::now();
            let mut wave = Wave::new();
            let mut next_mic = Instant::now();
            let mut next_feedback = Instant::now();
            if case == Case::SatpPairing {
                while !ingress.startup_ready() {
                    assert!(Instant::now() < end, "SATP runtime startup timeout");
                    thread::sleep(Duration::from_millis(1));
                }
                satp_runtime::client(address, satp_address, &state_model, &ingress);
                stop.store(true, Ordering::Release);
                while !joined.load(Ordering::Acquire) {
                    assert!(Instant::now() < end, "SATP runtime shutdown timeout");
                    thread::sleep(Duration::from_millis(1));
                }
                return 0;
            }
            if case.diagnostic() {
                while !ingress.startup_ready() || !dma_burst_complete.load(Ordering::Acquire) {
                    assert!(Instant::now() < end, "diagnostic startup timeout");
                    thread::sleep(Duration::from_millis(1));
                }
                let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts/ps-measurement-client.mjs");
                let mut command = std::process::Command::new("node");
                if matches!(
                    case,
                    Case::BrowserMeasurementClient
                        | Case::BrowserLongMeasurementClient
                        | Case::BrowserTimingMeasurementClient
                ) {
                    command
                        .arg(
                            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("../remote-web/scripts/validate-puresignal-tab.mjs"),
                        )
                        .arg(format!("--offline-fixture=ws://{address}/tci"));
                    if case == Case::BrowserLongMeasurementClient {
                        command.arg("--measurement-duration-ms=45000");
                    }
                    if case == Case::BrowserTimingMeasurementClient {
                        command.arg("--timing-only");
                    }
                } else {
                    command.arg(script).args([
                        "--run",
                        "--confirm-ant1-dummy-load-lp700-attended",
                        "--commanded-watts=5",
                    ]);
                }
                if case == Case::StimulusClient {
                    command.args(["--summary-only", "--duration-ms=45000"]);
                } else {
                    command.arg("--duration-ms=500");
                }
                let mut child = command
                    .arg(format!("--url=ws://{address}/tci"))
                    .arg(format!("--source-pin={}", env!("SATURN_BRIDGE_SOURCE_PIN")))
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let diagnostic_started = Instant::now();
                loop {
                    let status = ingress.lifecycle_status().unwrap();
                    assert_ne!(status.correction_applied, Some(true));
                    if status.phase == Phase::Active {
                        assert_eq!(status.correction_applied, Some(false));
                    }
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
                if case == Case::BrowserTimingMeasurementClient {
                    let path = std::env::var_os("SATURN_PS_MEASUREMENT_DIR").unwrap();
                    let records: Vec<_> = std::fs::read_dir(path)
                        .unwrap()
                        .map(|r| r.unwrap().path())
                        .collect();
                    assert_eq!(records.len(), 1);
                    let evidence = std::fs::read_to_string(&records[0]).unwrap();
                    assert!(evidence.contains("\"version\":\"saturn-timing-multisine-v1\""));
                    assert!(evidence.contains("\"repeatMs\":1000"));
                    assert!(evidence.contains("\"terminalFault\":null"));
                    assert!(evidence.contains("\"twoToneImdAvailable\":false"));
                }
                if case == Case::StimulusClient {
                    assert!(
                        diagnostic_started.elapsed() >= Duration::from_secs(44),
                        "long test must not pass on a short pulse"
                    );
                    let path = std::env::var_os("SATURN_PS_MEASUREMENT_DIR").unwrap();
                    let records: Vec<_> = std::fs::read_dir(path)
                        .unwrap()
                        .map(|v| v.unwrap().path())
                        .collect();
                    assert_eq!(records.len(), 1);
                    let evidence = std::fs::read_to_string(&records[0]).unwrap();
                    assert!(evidence.contains("\"format\":\"saturn-two-tone-summary-v1\""));
                    assert!(evidence.contains("\"rawFeedbackCaptured\":false"));
                    assert!(evidence.contains("\"terminalFault\":null"));
                    assert!(!evidence.contains("referenceF64Bits") && evidence.len() < 4096);
                }
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
            let mut baseline_active = false;
            let mut baseline_since: Option<Instant> = None;
            let mut gap_tail_packets = None;
            let mut gap_completed = false;
            let mut auto_active_since: Option<Instant> = None;
            let mut bypass_active_since: Option<Instant> = None;
            let mut silent_ptt_started: Option<Instant> = None;
            let mut bypass_packets = 0;
            let mut auto_generations = std::collections::BTreeSet::new();
            let mut last_good_att = None;
            let require_monitor = case == Case::ShutdownActive
                && std::env::var("SATURN_PS_CORRECTION_MONITOR").as_deref() == Ok("1");
            let mut monitor_baseline = crate::correction_monitor::published()[usize::from(direct)];
            let mut leveling_since = None;
            let mut negative_started = false;
            while !joined.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < end,
                    "runtime client {case:?} stage={stage} {:?} meters={:?}",
                    ingress.lifecycle_status(),
                    client.as_ref().map(|c| &c.rx_meter_dbm)
                );
                if matches!(case, Case::SatpBindFailure | Case::AttachmentFailure) {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                if let Some(c) = &mut client {
                    c.drain();
                    if case.rear_ptt() {
                        for (down, id) in std::mem::take(&mut c.hardware_ptt) {
                            if down {
                                c.text(&format!("saturn_ptt:0,{id};"));
                            }
                        }
                    }
                }
                let status = ingress.lifecycle_status().unwrap();
                if case.auto_attenuation() {
                    if let Some(scope) = link.desired().scope {
                        if auto_generations.insert(scope.output.generation) {
                            assert_eq!(
                                scope.attenuation_db,
                                if stage >= 4 { last_good_att.expect("warm re-arm requires a completed previous key") } else { 31 },
                                "cold TX starts at maximum ATT; warm re-arm retains the completed key's ATT"
                            );
                        }
                        if status.phase == Phase::Active {
                            assert!(ingress.leveling_verified.load(Ordering::Acquire) > 0);
                            if case == Case::AutoAttLimit {
                                let att = ingress.auto_leveling_snapshot().unwrap();
                                assert_eq!(scope.attenuation_db, 31);
                                if auto_generations.len() > 1 {
                                    assert_eq!(att.phase, "retained");
                                    assert_eq!(att.applied_db, Some(31));
                                    assert_eq!(
                                        att.info, None,
                                        "old startup ratio is not a new measurement"
                                    );
                                } else {
                                    assert_eq!(att.phase, "limited");
                                    assert!(att.info.is_some_and(|level| level > 165));
                                }
                            } else {
                                assert!(scope.attenuation_db < 31, "servo must actually adjust");
                            }
                            assert!(link.prepared(scope.output).unwrap());
                            last_good_att = Some(scope.attenuation_db);
                        }
                    }
                }
                if case.auto_attenuation() && status.phase == Phase::AwaitingKey {
                    assert_ne!(
                        status.correction_applied,
                        Some(true),
                        "startup leveling must keep correction off"
                    );
                }
                if matches!(
                    case,
                    Case::AutoAttRelease
                        | Case::AutoAttMissing
                        | Case::AutoAttReconfigure
                        | Case::AutoAttDisconnect
                        | Case::AutoAttRange
                ) {
                    if ingress.auto_leveling_snapshot().is_some_and(|s| s.keyed) {
                        let at = leveling_since.get_or_insert_with(Instant::now);
                        if !negative_started
                            && (case == Case::AutoAttRange
                                || at.elapsed() >= Duration::from_millis(60))
                        {
                            assert_eq!(status.phase, Phase::AwaitingKey);
                            if case != Case::AutoAttRange {
                                assert!(
                                    ingress.leveling_verified.load(Ordering::Acquire) > 0,
                                    "real native disabled-state checks required"
                                );
                            }
                            if case == Case::AutoAttRange {
                                // Actual acquisition supplies saturated feedback at maximum ATT.
                            } else if case == Case::AutoAttDisconnect {
                                drop(client.take());
                            } else if case == Case::AutoAttMissing {
                                drop_feedback.store(true, Ordering::Release);
                            } else if case == Case::AutoAttReconfigure {
                                client.as_mut().unwrap().text("tx_mic_gain:0,1;");
                            } else {
                                client.as_mut().unwrap().text("trx:0,false;");
                            }
                            negative_started = true;
                            if case != Case::AutoAttRange {
                                stage = 12;
                            }
                        }
                    }
                    if (negative_started
                        || (case == Case::AutoAttRange && status.primary_fault.is_some()))
                        && status.phase == Phase::ReadyOff
                    {
                        assert!(
                            status.authority_completed_ns.is_some()
                                && status.cleanup_completed_ns.is_some()
                        );
                        assert_eq!(
                            status.primary_fault,
                            if case == Case::AutoAttRange {
                                Some(Fault::SevereOverdrive)
                            } else if case == Case::AutoAttMissing {
                                Some(Fault::Stale)
                            } else {
                                None
                            }
                        );
                        assert!(ingress.leveling_verified.load(Ordering::Acquire) > 0);
                        if case == Case::AutoAttReconfigure
                            && (!ingress.startup_ready()
                                || state_model.lock_unpoisoned().desired.tx_mic_gain_db != 1.)
                        {
                            continue;
                        }
                        stop.store(true, Ordering::Release);
                        while !joined.load(Ordering::Acquire) {
                            assert!(Instant::now() < end);
                            thread::sleep(Duration::from_millis(1));
                        }
                        return 0;
                    }
                }
                if case.native_two_tone() && status.phase == Phase::Active {
                    baseline_since.get_or_insert_with(Instant::now);
                }
                let gap_ms = baseline_since.map_or(0, |t| t.elapsed().as_millis());
                if case == Case::NativeKeepaliveGap && (350..400).contains(&gap_ms) {
                    gap_tail_packets.get_or_insert_with(|| output_packets.load(Ordering::Acquire));
                }
                if case == Case::NativeKeepaliveGap && gap_ms >= 400 && !gap_completed {
                    assert_eq!(
                        status.phase,
                        Phase::Active,
                        "short keepalive gap must not stop native output"
                    );
                    let before = gap_tail_packets.expect("must observe tail of keepalive pause");
                    assert!(
                        output_packets.load(Ordering::Acquire) > before + 5,
                        "native output must continue after retained input has drained"
                    );
                    gap_completed = true;
                }
                if case == Case::NativeKeepaliveMissing && status.primary_fault.is_some() {
                    assert_eq!(status.primary_fault, Some(Fault::Stale));
                    assert!(
                        gap_ms >= 600,
                        "existing source watchdog must not fire at the old 50 ms browser deadline"
                    );
                    if status.phase == Phase::ReadyOff {
                        assert!(
                            status.cleanup_completed_ns.is_some()
                                && status.authority_completed_ns.is_some()
                        );
                        stop.store(true, Ordering::Release);
                        while !joined.load(Ordering::Acquire) {
                            assert!(Instant::now() < end);
                            thread::sleep(Duration::from_millis(1));
                        }
                        return 0;
                    }
                }
                if case.microphone_bypass()
                    && matches!(status.phase, Phase::AwaitingKey | Phase::Active)
                {
                    let started = silent_ptt_started.get_or_insert_with(Instant::now);
                    assert!(
                        status.phase == Phase::Active || started.elapsed() < Duration::from_secs(1),
                        "ordinary PTT must key on fresh silence, before speech"
                    );
                    if status.phase == Phase::Active {
                        bypass_active_since.get_or_insert_with(Instant::now);
                    }
                }
                if case == Case::AutoSustained && status.phase == Phase::Active {
                    auto_active_since.get_or_insert_with(Instant::now);
                }
                match stage {
                    0 if ingress.startup_ready()
                        && (case != Case::ReceiveSpectrum
                            || driver_started.elapsed() >= Duration::from_millis(500)) =>
                    {
                        // run_inner alone opens admission. No test-side admit.
                        let mut c = Client::open_unpaired(address);
                        if check_persistence {
                            // Actual ingress settings, not a direct test-side model edit.
                            c.text("vfo:0,0,7300000;");
                            c.text("rx_antenna:0,3;");
                            c.text("rx_attenuation:0,20;tx_puresignal_attenuation:0,13;");
                        }
                        if case == Case::ReceiveSpectrum {
                            c.text("saturn_display:spectrum,1024,50;");
                        }
                        c.text("iq_start:0;audio_start:0;");
                        if direct && case == Case::ReceiveMeter {
                            c.text("rx_headphones:0,true;");
                        }
                        client = Some(c);
                        connected_at = Some(Instant::now());
                        stage = 1;
                    }
                    1 if (if case == Case::ReceiveSpectrum {
                        client.as_ref().unwrap().spectrum_frames >= 2
                    } else {
                        client.as_ref().unwrap().iq_frames > 0
                    }) && (case != Case::ReceiveSpectrum
                        || connected_at.unwrap().elapsed() >= Duration::from_millis(1500))
                        && client.as_ref().unwrap().audio_frames > 0
                        && (case != Case::ReceiveMeter
                            || client
                                .as_ref()
                                .unwrap()
                                .rx_meter_dbm
                                .iter()
                                .filter(|v| v.is_finite() && **v >= -400. && **v < 100.)
                                .count()
                                >= 2) =>
                    {
                        println!(
                            "run_inner received IQ and audio: iq_frames={} audio_frames={}",
                            client.as_ref().unwrap().iq_frames,
                            client.as_ref().unwrap().audio_frames
                        );
                        if case == Case::ReceiveSpectrum {
                            let spectrum_client = client.as_ref().unwrap();
                            assert!(spectrum_client.vfo_a_reports <= 3, "idle receive must not repeat full configuration snapshots: {} VFO reports", spectrum_client.vfo_a_reports);
                            assert!(
                                spectrum_client.rx_meter_dbm.len() >= 2,
                                "periodic meter updates must continue"
                            );
                            assert!(spectrum_client.monitor_caps_reports > 0, "late clients must receive monitor capabilities without a configuration change");
                            let perf = std::fs::read_to_string(&perf_path).unwrap();
                            assert!(perf.contains(concat!(
                                "\"app_version\":\"",
                                env!("CARGO_PKG_VERSION"),
                                "\""
                            )));
                            assert!(perf.contains("\"pure_signal_available\":true"));
                            assert_eq!(
                                perf.contains("\"fpga_identity_source\":\"acquisition-owner-session-cache\""),
                                direct,
                                "only Direct XDMA publishes this acquisition session's cached identity: {perf}"
                            );
                            assert!(!perf.contains("\"clock_mask\":"));
                            for field in [
                                "\"connections\":1",
                                "\"client\":1",
                                "\"iq\":1",
                                "\"audio\":1",
                                "\"frequency_hz\":7200000",
                                "\"tx_keyed\":false",
                                "\"tx_stream\":false",
                                "\"rf_safe\":null",
                                "\"rf_qualified\":false",
                            ] {
                                assert!(
                                    perf.contains(field),
                                    "missing actual shared-owner observation {field}: {perf}"
                                );
                            }
                            let rate = perf
                                .split("\"ddc_s\":")
                                .nth(1)
                                .unwrap()
                                .split(',')
                                .next()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap();
                            assert!(rate.is_finite() && rate > 0.0);
                            assert_eq!(
                                spectrum_client.iq_frames, 0,
                                "WAN client must receive spectrum rows without raw-IQ fallback"
                            );
                            assert_eq!(status.phase, Phase::ReadyOff);
                            assert!(perf.contains("\"shared_performance\":{\"scope\":"));
                            assert!(perf.contains("\"rx_exchange_p95_upper_us\":"));
                            assert!(!perf.contains("\"rx_exchange_count\":0"));
                            assert!(perf.contains("\"tx_render_count\":0"));
                            // Real subscription changes leave the passband S-meter
                            // alive, stop unrequested audio/rows and resume audio.
                            let c = client.as_mut().unwrap();
                            c.text("audio_stop:0;iq_stop:0;");
                            let drain = Instant::now();
                            while drain.elapsed() < Duration::from_millis(300) {
                                c.drain();
                                thread::sleep(Duration::from_millis(2));
                            }
                            let paused = (c.audio_frames, c.spectrum_frames, c.rx_meter_dbm.len());
                            let meter_only = Instant::now();
                            while meter_only.elapsed() < Duration::from_millis(1200) {
                                c.drain();
                                thread::sleep(Duration::from_millis(2));
                            }
                            assert_eq!((c.audio_frames, c.spectrum_frames), (paused.0, paused.1));
                            assert!(
                                c.rx_meter_dbm.len() > paused.2,
                                "control-only RX lost passband metering"
                            );
                            c.text("audio_start:0;iq_start:0;");
                            let resumed = Instant::now();
                            while c.audio_frames == paused.0 || c.spectrum_frames == paused.1 {
                                c.drain();
                                assert!(
                                    resumed.elapsed() < Duration::from_secs(3),
                                    "RX demand resume stalled"
                                );
                                thread::sleep(Duration::from_millis(2));
                            }
                            stop.store(true, Ordering::Release);
                            while !joined.load(Ordering::Acquire) {
                                assert!(Instant::now() < end);
                                thread::sleep(Duration::from_millis(1));
                            }
                            return 0; // no arm or key; actual row worker and TCI output
                        }
                        if case == Case::FaultedReceiveTune {
                            use std::os::unix::fs::FileExt;
                            ingress.stop(Fault::SevereOverdrive);
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while ingress.lifecycle_status().unwrap().phase != Phase::ReadyOff
                                || ingress.lifecycle_status().unwrap().primary_fault.is_none()
                            {
                                client.as_mut().unwrap().drain();
                                assert!(Instant::now() < deadline, "fault cleanup timed out");
                                thread::sleep(Duration::from_millis(2));
                            }
                            assert!(
                                !ingress.startup_ready(),
                                "fault must not become startup/TX readiness"
                            );
                            let register =
                                std::fs::File::open(direct_register_path.as_ref().unwrap())
                                    .unwrap();
                            for hz in [7_100_000u32, 3_899_000, 7_220_000] {
                                let c = client.as_mut().unwrap();
                                let applied_before = c
                                    .transactions
                                    .iter()
                                    .filter(|x| x.contains(",applied,"))
                                    .count();
                                c.text(&format!("vfo:0,0,{hz};dds:0,{hz};"));
                                let expected = ((u128::from(hz) * (1u128 << 32) + 61_440_000)
                                    / 122_880_000)
                                    as u32;
                                let deadline = Instant::now() + Duration::from_secs(3);
                                loop {
                                    c.drain();
                                    let mut bytes = [0; 4];
                                    register.read_exact_at(&mut bytes, 0x18).unwrap();
                                    let applied = c
                                        .transactions
                                        .iter()
                                        .filter(|x| x.contains(",applied,"))
                                        .count();
                                    if applied > applied_before
                                        && u32::from_le_bytes(bytes) == expected
                                    {
                                        break;
                                    }
                                    assert!(Instant::now()<deadline,"model committed but DDC6 did not retune: requested={hz} register={:#x} expected={expected:#x} transactions={:?}",u32::from_le_bytes(bytes),c.transactions);
                                    thread::sleep(Duration::from_millis(2));
                                }
                                let status = ingress.lifecycle_status().unwrap();
                                assert_eq!(status.phase, Phase::ReadyOff);
                                assert_eq!(status.primary_fault, Some(Fault::SevereOverdrive));
                                assert_eq!(state_model.lock_unpoisoned().desired.iq_center_hz, hz);
                                assert!(!state_model.lock_unpoisoned().desired.tx_enabled);
                            }
                            // Fix the return level while leaving the fault latched.
                            // The attenuation is then proved at the real acquisition
                            // register boundary on a fresh, separately authorized arm.
                            let c = client.as_mut().unwrap();
                            let applied = c
                                .transactions
                                .iter()
                                .filter(|x| x.contains(",applied,"))
                                .count();
                            c.text("tx_puresignal_attenuation:0,17;");
                            let deadline = Instant::now() + Duration::from_secs(3);
                            loop {
                                c.drain();
                                if c.transactions
                                    .iter()
                                    .filter(|x| x.contains(",applied,"))
                                    .count()
                                    > applied
                                {
                                    break;
                                }
                                assert!(
                                    Instant::now() < deadline,
                                    "faulted attenuation update rejected: {:?}",
                                    c.transactions
                                );
                                thread::sleep(Duration::from_millis(2));
                            }
                            assert_eq!(
                                state_model
                                    .lock_unpoisoned()
                                    .desired
                                    .pure_signal_attenuation_db,
                                17
                            );
                            let st = ingress.lifecycle_status().unwrap();
                            assert_eq!(st.primary_fault, Some(Fault::SevereOverdrive));
                            c.text(&format!("saturn_tx_fault_ack:0,1,{};", st.stop_id));
                            while ingress.lifecycle_status().unwrap().primary_fault.is_some() {
                                c.drain();
                                assert!(
                                    Instant::now() < deadline,
                                    "acknowledgment did not complete"
                                );
                                thread::sleep(Duration::from_millis(2));
                            }
                            c.text("trx:0,true;");
                            loop {
                                c.drain();
                                if let Some(scope) = link.desired().scope {
                                    if link.prepared(scope.output).unwrap_or(false) {
                                        assert_eq!(scope.attenuation_db, 17);
                                        let mut b = [0; 4];
                                        register.read_exact_at(&mut b, 0x2018).unwrap();
                                        assert_eq!((u32::from_le_bytes(b)>>5)&31,17,"feedback ATT must reach the acquisition-owned register");
                                        break;
                                    }
                                }
                                assert!(
                                    Instant::now() < deadline,
                                    "fresh feedback prepare did not complete"
                                );
                                thread::sleep(Duration::from_millis(2));
                            }
                            assert_eq!(
                                ingress.lifecycle_status().unwrap().phase,
                                Phase::AwaitingKey
                            );
                            c.text("trx:0,false;");
                            while ingress.lifecycle_status().unwrap().phase != Phase::ReadyOff {
                                c.drain();
                                assert!(Instant::now() < deadline, "release did not complete");
                                thread::sleep(Duration::from_millis(2));
                            }
                            stop.store(true, Ordering::Release);
                            while !joined.load(Ordering::Acquire) {
                                assert!(Instant::now() < end);
                                thread::sleep(Duration::from_millis(1));
                            }
                            return 0;
                        }
                        if case == Case::BandRecall {
                            let fixture = include_str!(
                                "../../remote-web/tests/fixtures/managed-band-recall.tsv"
                            );
                            let backend = if direct { "direct" } else { "p2" };
                            for row in fixture
                                .lines()
                                .skip(1)
                                .filter(|row| row.starts_with(backend))
                            {
                                let step: Vec<_> = row.split('\t').collect();
                                let frequency = step[2].parse::<u32>().unwrap();
                                let watts = step[3].parse::<u64>().unwrap();
                                let mic = step[4].parse::<f64>().unwrap();
                                let c = client.as_mut().unwrap();
                                let applied_before = c
                                    .transactions
                                    .iter()
                                    .filter(|s| s.contains(",applied,"))
                                    .count();
                                let messages = &step[5..];
                                let profile = crate::pa_calibration::PaCalibration::Bands(
                                    crate::pa_calibration::G2_STARTING_GAINS,
                                );
                                c.text(&format!("tx_pa_calibration:0,{};", profile.wire()));
                                for wire in messages {
                                    c.text(wire);
                                }
                                let expected = applied_before + messages.len() + 1;
                                let deadline = Instant::now() + Duration::from_secs(5);
                                loop {
                                    c.drain();
                                    assert!(
                                        !c.transactions.iter().any(|s| s.contains(",rejected,")),
                                        "band recall rejected: {:?}",
                                        c.transactions
                                    );
                                    let applied = c
                                        .transactions
                                        .iter()
                                        .filter(|s| s.contains(",applied,"))
                                        .count();
                                    if applied >= expected
                                        && ingress.lifecycle_status().unwrap().phase
                                            == Phase::ReadyOff
                                    {
                                        break;
                                    }
                                    assert!(
                                        Instant::now() < deadline,
                                        "band recall did not complete: {:?} {:?}",
                                        c.transactions,
                                        ingress.lifecycle_status()
                                    );
                                    thread::sleep(Duration::from_millis(2));
                                }
                                let m = state_model.lock_unpoisoned();
                                assert_eq!(m.desired.vfo_a_hz, frequency);
                                assert_eq!(m.desired.vfo_b_hz, frequency);
                                assert_eq!(m.desired.iq_center_hz, frequency);
                                assert_eq!(u64::from(m.desired.tx_drive), watts);
                                assert_eq!(m.desired.tx_pa_calibration, profile);
                                assert_eq!(f64::from(m.desired.tx_mic_gain_db), mic);
                                assert_eq!(
                                    m.desired.rx_antenna,
                                    if step[1] == "40m" { 2 } else { 1 }
                                );
                                assert_eq!(
                                    m.desired.rx_attenuation_db,
                                    if step[1] == "40m" { 10 } else { 0 }
                                );
                                assert!(!m.desired.tx_enabled);
                                println!("band_recall backend={} band={} frequency={} applied={} keyed=false", if direct {"DirectXdma"} else {"P2"}, step[1], m.desired.vfo_a_hz, applied_before + messages.len());
                            }
                            assert_eq!(status.phase, Phase::ReadyOff);
                            stop.store(true, Ordering::Release);
                            while !joined.load(Ordering::Acquire) {
                                assert!(Instant::now() < end);
                                thread::sleep(Duration::from_millis(1));
                            }
                            return 0;
                        }
                        if case == Case::ReceiveMeter {
                            if direct {
                                assert!(client.as_ref().unwrap().iq_pairs_per_frame.iter().all(|n| *n == 12_800), "shared Direct must deliver complete 384 kHz display frames, not rate-limited DMA snippets");
                            } else {
                                assert!(client.as_ref().unwrap().iq_pairs_per_frame.iter().all(|n| *n == 6_400), "shared P2 must batch the complete 192 kHz display stream, not discard small datagrams");
                            }
                            println!(
                                "run_inner live RX meter backend={} dbm={:?}",
                                if direct { "DirectXdma" } else { "P2" },
                                client.as_ref().unwrap().rx_meter_dbm
                            );
                            assert_eq!(status.phase, Phase::ReadyOff);
                            stop.store(true, Ordering::Release);
                            while !joined.load(Ordering::Acquire) {
                                assert!(Instant::now() < end);
                                thread::sleep(Duration::from_millis(1));
                            }
                            return 0; // receive-only: no arm, PCM or key command
                        }
                        if case.native_two_tone() {
                            client.as_mut().unwrap().text(
                                "saturn_ps_two_tone:0,1,start,2000,wdsp-postgen-two-tone-v1;",
                            );
                            stage = 11;
                            continue;
                        }
                        if case == Case::MicrophoneBypass && direct {
                            client.as_mut().unwrap().text(
                                "tx_monitor:0,true;tx_monitor_level:0,-18;rx_headphones:0,true;",
                            );
                        }
                        if case.microphone_bypass() {
                            // Exactly the normal voice profile that exposed the
                            // accidental MeasurementOnly unity-gain requirement.
                            client
                                .as_mut()
                                .unwrap()
                                .text("tx_mic_gain:0,-12;tx_puresignal:0,false;");
                        } else if matches!(case, Case::AutoCorrection | Case::AutoSustained) {
                            assert!(ingress.auto_mode_caps().is_some());
                            client.as_mut().unwrap().text(if case.auto_attenuation() {
                                "tx_puresignal_auto_attenuate:0,true;tx_puresignal_mode:0,auto;"
                            } else {
                                "tx_puresignal_mode:0,auto;"
                            });
                        } else if case.auto_attenuation() {
                            client
                                .as_mut()
                                .unwrap()
                                .text("tx_puresignal_auto_attenuate:0,true;tx_puresignal:0,true;");
                        } else {
                            client.as_mut().unwrap().text("tx_puresignal:0,true;");
                        }
                        stage = 2;
                    }
                    2 if ingress.startup_ready()
                        && client.as_ref().unwrap().transactions.iter().any(|s| {
                            s == if check_persistence {
                                // Frequency, antenna and attenuation were
                                // separate ordered transactions. Their
                                // completion cannot authorize the later arm.
                                "tx_config:0,4,applied,5;"
                            } else {
                                "tx_config:0,1,applied,2;"
                            }
                        }) =>
                    {
                        if case.rear_ptt() {
                            hardware_down.store(true, Ordering::Release);
                        } else {
                            client.as_mut().unwrap().text("trx:0,true;");
                        }
                        stage = 3;
                    }
                    3 | 5
                        if status.phase == Phase::Active
                            && if case.microphone_bypass() {
                                // Real tagged microphone input and completed backend
                                // output must continue through this generation.
                                status.correction_applied != Some(true)
                                    && output_packets.load(Ordering::Acquire) > bypass_packets + 30
                                    && bypass_active_since
                                        .is_some_and(|t| t.elapsed() >= Duration::from_millis(300))
                            } else {
                                status.correction_applied == Some(true)
                            }
                            && (!require_monitor
                                || crate::correction_monitor::published()[usize::from(direct)]
                                    > monitor_baseline)
                            && (case != Case::AutoSustained
                                || auto_active_since
                                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(15))) =>
                    {
                        if case == Case::MicrophoneBypass && direct {
                            let m = state_model.lock_unpoisoned();
                            assert_eq!(m.desired.tx_monitor_enabled, stage == 3);
                            assert_eq!(m.desired.rx_headphones_enabled, stage == 3);
                            assert_eq!(
                                m.desired.tx_monitor_level_db,
                                if stage == 3 { -18. } else { -60. }
                            );
                        }
                        corrected += 1;
                        if matches!(case, Case::AutoCorrection | Case::AutoSustained) {
                            assert!(ingress.owner_status_message().unwrap().contains(",auto;"));
                        }
                        if case == Case::AutoSustained {
                            assert!(
                                client
                                    .as_ref()
                                    .unwrap()
                                    .native_calibrations
                                    .iter()
                                    .copied()
                                    .max()
                                    .unwrap_or(0)
                                    >= 2,
                                "sustained Auto must refit within each generation"
                            );
                        }
                        if stage == 3
                            && matches!(
                                case,
                                Case::ShutdownActive
                                    | Case::RearPtt
                                    | Case::RearPttBypass
                                    | Case::MicrophoneBypass
                                    | Case::AutoCorrection
                                    | Case::AutoAttSingle
                                    | Case::AutoAttLimit
                                    | Case::AutoSustained
                            )
                        {
                            if case.rear_ptt() {
                                hardware_down.store(false, Ordering::Release);
                            } else {
                                client.as_mut().unwrap().text("trx:0,false;");
                            }
                            stage = 4;
                        } else {
                            stop_baseline.store(status.stop_id, Ordering::Release);
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
                        if case == Case::MicrophoneBypass && direct {
                            client.as_mut().unwrap().text(
                                "tx_monitor:0,false;rx_headphones:0,false;tx_monitor_level:0,-100;",
                            );
                        }
                        wave = Wave::new();
                        bypass_active_since = None;
                        silent_ptt_started = None;
                        bypass_packets = output_packets.load(Ordering::Acquire);
                        auto_active_since = None;
                        monitor_baseline =
                            crate::correction_monitor::published()[usize::from(direct)];
                        client.as_mut().unwrap().native_calibrations.clear();
                        if case.rear_ptt() {
                            // A delayed old press must not consume or fault the
                            // next physical generation after completed cleanup.
                            client.as_mut().unwrap().text("saturn_ptt:0,1;");
                            hardware_down.store(true, Ordering::Release);
                        } else {
                            client.as_mut().unwrap().text("trx:0,true;");
                        }
                        stage = 5;
                    }
                    _ => {}
                }
                if case.native_two_tone() && stage == 11 {
                    if status.phase == Phase::Active {
                        baseline_active = true;
                        assert_ne!(status.correction_applied, Some(true));
                    }
                    let c = client.as_ref().unwrap();
                    assert!(c.native_applied.iter().all(|applied| !*applied));
                    assert!(c.native_calibrations.iter().all(|count| *count == 0));
                    if baseline_active
                        && status.phase == Phase::ReadyOff
                        && c.two_tone_results
                            .iter()
                            .any(|r| r.contains(",complete,none;"))
                    {
                        if case == Case::NativeKeepaliveGap {
                            assert!(gap_completed);
                        }
                        assert!(
                            !c.native_applied.is_empty(),
                            "real WDSP disabled-state observation required"
                        );
                        stop.store(true, Ordering::Release);
                        while !joined.load(Ordering::Acquire) {
                            assert!(Instant::now() < end);
                            thread::sleep(Duration::from_millis(1));
                        }
                        return 0;
                    }
                }
                if matches!(stage, 3 | 5 | 11) {
                    if let Some(p) = &p2 {
                        if matches!(status.phase, Phase::AwaitingKey | Phase::Active)
                            && Instant::now() >= next_feedback
                        {
                            if case.auto_attenuation() {
                                if let Some(scope) = link.desired().scope {
                                    wave.attenuation(scope.attenuation_db);
                                    if case == Case::AutoAttRange {
                                        wave.amplify_feedback(1000.);
                                    } else if case == Case::AutoAttLimit {
                                        wave.amplify_feedback(18.);
                                    }
                                }
                            }
                            p.feedback.send_to(&wave.p2(), p.destination).unwrap();
                            next_feedback = Instant::now() + Duration::from_millis(2);
                        }
                    }
                    if Instant::now() >= next_mic {
                        if case.native_two_tone() {
                            let paused = match case {
                                Case::NativeKeepaliveGap => (250..400).contains(&gap_ms),
                                Case::NativeKeepaliveMissing => gap_ms >= 250,
                                _ => false,
                            };
                            if !paused {
                                client.as_mut().unwrap().zero_keepalive();
                            }
                        } else if case.microphone_bypass()
                            && bypass_active_since
                                .is_none_or(|t| t.elapsed() < Duration::from_millis(150))
                        {
                            // Key and sustain output before the first syllable.
                            client.as_mut().unwrap().zero_keepalive();
                        } else {
                            client.as_mut().unwrap().mic();
                        }
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
                } else if matches!(
                    case,
                    Case::ShutdownActive
                        | Case::RearPtt
                        | Case::RearPttBypass
                        | Case::MicrophoneBypass
                        | Case::AutoCorrection
                        | Case::AutoAttSingle
                        | Case::AutoAttLimit
                        | Case::AutoSustained
                ) {
                    2
                } else {
                    1
                }
            );
            corrected
        }));
    }
    fn before_receive(&mut self, acquisition: &mut Acquisition) -> Result<bool, Box<dyn Error>> {
        if self.case.rear_ptt() && self.hardware_sent.elapsed() >= Duration::from_millis(5) {
            let down = self.hardware_down.load(Ordering::Acquire);
            if let Some(files) = &self.files {
                files.write(0x4000, u32::from(down));
            }
            if let Some(p) = &self.p2 {
                let mut packet = [0u8; 60];
                packet[4] = u8::from(down);
                p.status.send_to(&packet, p.destination)?;
            }
            self.hardware_sent = Instant::now();
        }
        if self.case.diagnostic() {
            if self.dma_burst_stage == 0 && self.ingress.as_ref().unwrap().startup_ready() {
                // Three already completed small reads. No producer or wall
                // clock advances are needed to prove that all are available.
                match acquisition {
                    Acquisition::Direct(rx) => {
                        let mut frame = vec![0u8; 72];
                        frame[..4].copy_from_slice(&0x100000u32.to_le_bytes());
                        frame[7] = 0x80;
                        let bytes = frame.repeat(56);
                        for _ in 0..3 {
                            direct_rx::reader_turn(rx, &bytes, &[], self.sequence);
                            self.sequence += 1;
                        }
                    }
                    Acquisition::P2(_) => {
                        let mut packet = vec![0u8; 1444];
                        packet[12..14].copy_from_slice(&24u16.to_be_bytes());
                        packet[14..16].copy_from_slice(&238u16.to_be_bytes());
                        for _ in 0..3 {
                            packet[..4].copy_from_slice(&(self.sequence as u32).to_be_bytes());
                            self.normal_p2
                                .as_ref()
                                .unwrap()
                                .send_to(&packet, self.p2.as_ref().unwrap().destination)?;
                            self.sequence += 1;
                        }
                    }
                }
                self.dma_burst_stage = 1;
                return Ok(true);
            }
            if (1..=if self.direct { 3 } else { 4 }).contains(&self.dma_burst_stage) {
                return Ok(true); // consume retained completions, no new read
            }
        }
        if self.case.diagnostic() {
            if let Some(files) = &self.files {
                files.step_written_fifo();
            }
        }
        let ingress = self.ingress.as_ref().unwrap();
        let link = self.link.as_ref().unwrap();
        if self.expected_rx.is_some() {
            if let Some(scope) = link.desired().scope {
                assert_eq!(
                    scope.attenuation_db, 13,
                    "manual feedback attenuation reaches each fresh output generation"
                );
                if link.prepared(scope.output).unwrap_or(false) {
                    if let Some(files) = &self.files {
                        assert_eq!(
                            (files.read(0x2018) >> 5) & 31,
                            13,
                            "Direct feedback attenuation readback"
                        );
                    }
                }
            }
        }
        if self.fail.swap(false, Ordering::AcqRel) && !self.injected {
            self.injected = true;
            return Err(std::io::Error::other("injected acquisition read failure").into());
        }
        let request = link.desired();
        if self.auto_drop_feedback.load(Ordering::Acquire) && request.scope.is_some() {
            return Ok(false);
        }
        // Only the file-backed Direct reader needs synthetic read pacing.
        // P2 arrivals are already paced by the independent UDP producer; a
        // second 4 ms throttle here halves consumption and drops real packets
        // once the loopback receive socket fills during a longer replay.
        if self.direct && self.case.diagnostic() && request.scope.is_some() {
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
                    let status = ingress.lifecycle_status().unwrap();
                    let baseline = self.stop_baseline.load(Ordering::Acquire);
                    assert!(status.stop_id >= baseline, "status regressed: {status:?}");
                    // begin_stop publishes the acquisition request BEFORE
                    // turn() publishes lifecycle status. A retained completion
                    // from the previous stop is not a receipt for this stop.
                    // Only compare completion fields once this stop's status
                    // is published; require repeated checks below, so this
                    // cannot pass by observing only the older publication.
                    if status.stop_id > baseline {
                        assert_eq!(status.phase, Phase::AwaitingAuthority, "{status:?}");
                        assert!(status.cleanup_completed_ns.is_none(), "{status:?}");
                        assert!(status.authority_completed_ns.is_none(), "{status:?}");
                        self.pending_stop_checks += 1;
                    }
                    return Ok(false); // external I/O temporarily unavailable; lifecycle keeps ticking
                }
            }
        }
        match acquisition {
            Acquisition::P2(p) => {
                let packets = self.p2.as_ref().unwrap().drain_output_count();
                self.output_packets.fetch_add(packets, Ordering::Release);
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
                self.output_packets.store(
                    self.files.as_ref().unwrap().radio.snapshot().frames_written,
                    Ordering::Release,
                );
                let mut frame = vec![0u8; 72]; // DDC6 x8 complex words @384k
                frame[..4].copy_from_slice(&0x100000u32.to_le_bytes());
                frame[7] = 0x80;
                let normal = frame.repeat(128);
                if self.case.auto_attenuation() {
                    if link.preparing() {
                        direct_rx::reader_pause_only(rx);
                        return Ok(true);
                    }
                    if let Some(scope) = request.scope {
                        assert_eq!(
                            (self.files.as_ref().unwrap().read(0x2018) >> 5) & 31,
                            u32::from(scope.attenuation_db)
                        );
                        self.wave.attenuation(scope.attenuation_db);
                        if self.case == Case::AutoAttRange {
                            self.wave.amplify_feedback(1000.);
                        } else if self.case == Case::AutoAttLimit {
                            self.wave.amplify_feedback(18.);
                        }
                    }
                }
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
    fn wait_for_input(&mut self, wait: Option<Duration>) {
        let idle_stage = if self.direct { 3 } else { 4 };
        if (1..=idle_stage).contains(&self.dma_burst_stage) {
            assert_eq!(
                wait,
                if self.dma_burst_stage < idle_stage {
                    None
                } else {
                    Some(Duration::from_millis(1))
                },
                "queued DMA completion must not incur an idle sleep"
            );
            self.dma_burst_stage += 1;
            if self.dma_burst_stage > idle_stage {
                self.dma_burst_complete.store(true, Ordering::Release);
            }
        }
        if let Some(wait) = wait {
            thread::sleep(wait);
        }
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
        satp_enabled: matches!(case, Case::SatpBindFailure | Case::SatpPairing),
        satp_bind_addr: occupied.local_addr().unwrap(),
        ..BridgeConfig::default()
    };
    let _occupied = if case == Case::SatpPairing {
        drop(occupied);
        None
    } else {
        Some(occupied)
    };
    let mut io = Io {
        direct,
        case,
        address,
        satp_address: config.satp_bind_addr,
        perf_path: std::path::PathBuf::new(),
        p2: None,
        normal_p2: None,
        files: None,
        stop: Arc::new(AtomicBool::new(false)),
        stop_baseline: Arc::new(AtomicU64::new(0)),
        fail: Arc::new(AtomicBool::new(false)),
        joined: Arc::new(AtomicBool::new(false)),
        ingress: None,
        link: None,
        driver: None,
        status: None,
        state_model: None,
        wave: Wave::new(),
        sequence: 0,
        restore_hold: None,
        held_turns: 0,
        pending_stop_checks: 0,
        received_turns: 0,
        injected: false,
        expected_rx: None,
        measurement_last_receive: Instant::now(),
        dma_burst_stage: 0,
        dma_burst_complete: Arc::new(AtomicBool::new(false)),
        output_packets: Arc::new(AtomicU64::new(0)),
        hardware_down: Arc::new(AtomicBool::new(false)),
        hardware_sent: Instant::now() - Duration::from_secs(1),
        auto_drop_feedback: Arc::new(AtomicBool::new(false)),
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
    io.perf_path = directory.join("perf.json");
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
    if case == Case::AutoAttLimit {
        // Same host component margin as the reported G2 run, not a relaxed
        // full-scale fixture guard. Native fitting must pass at this margin.
        selected_options
            .operational_policy
            .as_mut()
            .unwrap()
            .component_limit = 0.95;
    }
    let previous_satp_status = std::env::var_os("SATURN_BRIDGE_SATP_STATUS_PATH");
    if case == Case::SatpPairing {
        unsafe {
            std::env::set_var(
                "SATURN_BRIDGE_SATP_STATUS_PATH",
                directory.join("satp-status.json"),
            );
        }
    }
    if std::env::var("SATURN_PS_GENERATED_OPERATIONAL").as_deref() == Ok("1") {
        selected_options.max_watts = 100;
        selected_options.timing.tx_duration_ns = 0;
    }
    if case.diagnostic() {
        selected_options.measurement_only = true;
        selected_options.operational_policy = None;
        selected_options.timing.tx_duration_ns = if case.long_diagnostic() {
            45_000_000_000
        } else {
            1_000_000_000
        };
        let captures = crate::tx_thread::ps_measurement::tests::directory();
        // Explicit isolated ignored-test process; no live service reads these.
        unsafe {
            std::env::set_var("SATURN_PS_MEASUREMENT_DIR", &captures);
            if case == Case::BrowserMeasurementClient {
                std::env::set_var("SATURN_TX_DIAGNOSTIC_DIRECTORY", captures.join("digital"));
            } else {
                std::env::remove_var("SATURN_TX_DIAGNOSTIC_DIRECTORY");
            }
            std::env::set_var("SATURN_PS_MEASUREMENT_FREQUENCY_HZ", "7200000");
            std::env::set_var(
                "SATURN_PS_MEASUREMENT_CAPTURE",
                if case.long_diagnostic() {
                    "summary-only"
                } else {
                    "raw"
                },
            );
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
    if case == Case::SatpPairing {
        unsafe {
            if let Some(old) = previous_satp_status {
                std::env::set_var("SATURN_BRIDGE_SATP_STATUS_PATH", old);
            } else {
                std::env::remove_var("SATURN_BRIDGE_SATP_STATUS_PATH");
            }
        }
    }
    let corrected = io.driver.take().unwrap().join().unwrap();
    if persistent {
        let restored = store.load().unwrap().unwrap().fields;
        assert_eq!(restored.vfo_a_hz, Some(7_300_000));
        assert_eq!(restored.vfo_b_hz, Some(7_215_000));
        assert_eq!(restored.rx_antenna, Some(3));
        assert_eq!(restored.rx_attenuation_db, Some(20));
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
    if direct && case == Case::ReceiveMeter {
        assert!(
            io.files
                .as_ref()
                .unwrap()
                .radio
                .rx_headphone_observation()
                .0
                > 0,
            "actual RX runtime must submit audio to the enabled headphone worker"
        );
    }
    if direct && case == Case::MicrophoneBypass {
        let (frames, peak, stops) = io.files.as_ref().unwrap().radio.monitor_observation();
        assert!(
            frames > 30 && peak > 0.001,
            "completed processed IQ reaches MON: {frames} {peak}"
        );
        assert!(stops >= 2, "release and shutdown invalidate monitor output");
    }
    let status = io.status.unwrap();
    assert_eq!(
        result.is_ok(),
        matches!(
            case,
            Case::ShutdownActive
                | Case::RearPtt
                | Case::RearPttBypass
                | Case::MicrophoneBypass
                | Case::AutoCorrection
                | Case::AutoAttSingle
                | Case::AutoAttLimit
                | Case::AutoAttRelease
                | Case::AutoAttReconfigure
                | Case::AutoAttDisconnect
                | Case::AutoSustained
                | Case::BaselineTwoTone
                | Case::NativeKeepaliveGap
                | Case::ReceiveMeter
                | Case::SatpPairing
                | Case::BandRecall
                | Case::FaultedReceiveTune
                | Case::ReceiveSpectrum
                | Case::MeasurementClient
                | Case::BrowserMeasurementClient
                | Case::BrowserLongMeasurementClient
                | Case::BrowserTimingMeasurementClient
                | Case::StimulusClient
        ),
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
            Case::SatpBindFailure
                | Case::BaselineTwoTone
                | Case::NativeKeepaliveGap
                | Case::NativeKeepaliveMissing
                | Case::ReceiveMeter
                | Case::SatpPairing
                | Case::BandRecall
                | Case::FaultedReceiveTune
                | Case::ReceiveSpectrum
                | Case::AttachmentFailure
                | Case::MeasurementClient
                | Case::BrowserMeasurementClient
                | Case::BrowserLongMeasurementClient
                | Case::BrowserTimingMeasurementClient
                | Case::StimulusClient
                | Case::AutoAttRelease
                | Case::AutoAttMissing
                | Case::AutoAttReconfigure
                | Case::AutoAttDisconnect
                | Case::AutoAttRange
        ) {
            assert!(io.held_turns > 2 && io.received_turns > 10);
            assert!(
                io.pending_stop_checks > 2,
                "no repeated current-stop checks"
            );
        }
        if let Some(f) = &io.files {
            f.assert_off();
            if case.native_two_tone() {
                assert!(f.radio.snapshot().last_session.unwrap().frames_written > 20);
                let bytes =
                    std::fs::read(f.register_path().parent().unwrap().join("duc.bin")).unwrap();
                assert!(
                    bytes.iter().any(|v| *v != 0),
                    "zero keepalive must produce actual nonzero native output"
                );
            }
            if persistent {
                assert_eq!(
                    f.read(0x2018) & 31,
                    20,
                    "normal RX attenuation survives feedback restore"
                );
                assert_eq!(
                    f.read(crate::xdma::ALEX_TX_FILTER_RX_ANTENNA_REGISTER),
                    crate::xdma::alex_receive_state_word(7_300_000, 3),
                    "selected normal RX antenna reaches the acquisition owner"
                );
            }
            if case.diagnostic() {
                assert!(io.dma_burst_complete.load(Ordering::Acquire));
                let tx = f.radio.snapshot();
                let session = tx
                    .last_session
                    .expect("measurement reached a real file-backed TX session");
                assert!(session.keyed, "the actual key method must complete");
                assert!(
                    session.frames_written > 20,
                    "seed + deficit refill + live + ongoing IQ"
                );
                assert!(session.dma_writes >= 4);
            }
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
    if case == Case::NativeKeepaliveMissing {
        assert_eq!(status.primary_fault, Some(Fault::Stale));
        assert!(status.authority_fault.is_none() && status.cleanup_fault.is_none());
    }
    println!("run_inner backend={} case={case:?} corrected_generations={corrected} measurement_transactions={} receive_turns={} restoration_wait_turns={} owner_joined=true phase={:?} physical_rf_qualified=false", if direct {"DirectXdma"} else {"P2"}, usize::from(case.diagnostic()), io.received_turns, io.held_turns, status.phase);
    if case == Case::BrowserMeasurementClient {
        let digital =
            std::path::PathBuf::from(std::env::var_os("SATURN_TX_DIAGNOSTIC_DIRECTORY").unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !digital.join("capture-summary.json").exists() {
            assert!(
                Instant::now() < deadline,
                "diagnostic worker did not finish"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let check = std::process::Command::new("python3")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts/review-shared-signal.py"),
            )
            .arg(&digital)
            .args(["--backend", if direct { "DirectXdma" } else { "P2" }])
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let json = String::from_utf8(check.stdout).unwrap();
        assert!(json.contains("wdsp-postgen-two-tone-v1"));
        assert!(json.contains("\"captureSetDrained\": true"));
        std::fs::write(digital.join("review.json"), json).unwrap();
        println!("native_source_runtime_capture backend={} directory={} verified_zero_pcm_nonzero_native_and_returned=true completed_h2c_checked={} physical_rf_qualified=false",if direct {"DirectXdma"} else {"P2"},digital.display(),direct);
    }
}
#[test]
#[ignore = "serial actual runtime operator controls, matched WDSP and file/loopback owners; no RF"]
fn actual_run_inner_operator_two_tone_policy_both_backends() {
    let names = [
        "SATURN_PS_GENERATED_TRIALS",
        "SATURN_PS_GENERATED_OPERATIONAL",
        "SATURN_PS_CORRECTION_MONITOR",
    ];
    let previous: Vec<_> = names.iter().map(|n| std::env::var_os(n)).collect();
    for name in names {
        std::env::set_var(name, "1");
    }
    for direct in [false, true] {
        exercise(direct, Case::BaselineTwoTone);
    }
    for (name, value) in names.into_iter().zip(previous) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}

#[test]
#[ignore = "serial actual-runtime PS-off native generator; zero input, real WDSP, file/loopback only"]
fn actual_run_inner_ps_off_two_tone_both_backends() {
    unsafe {
        std::env::set_var("SATURN_PS_GENERATED_TRIALS", "1");
        std::env::set_var("SATURN_PS_GENERATED_FREQUENCY_HZ", "7200000");
        std::env::set_var("SATURN_PS_CORRECTION_MONITOR", "1");
    }
    for direct in [false, true] {
        exercise(direct, Case::BaselineTwoTone);
    }
    unsafe {
        std::env::remove_var("SATURN_PS_GENERATED_TRIALS");
        std::env::remove_var("SATURN_PS_GENERATED_FREQUENCY_HZ");
        std::env::remove_var("SATURN_PS_CORRECTION_MONITOR");
    }
}
#[test]
#[ignore = "serial native-generator liveness: short keepalive pause vs expired source, files/loopback only"]
fn actual_run_inner_native_keepalive_liveness_both_backends() {
    unsafe {
        std::env::set_var("SATURN_PS_GENERATED_TRIALS", "1");
        std::env::set_var("SATURN_PS_GENERATED_FREQUENCY_HZ", "7200000");
        std::env::set_var("SATURN_PS_CORRECTION_MONITOR", "1");
    }
    for direct in [false, true] {
        exercise(direct, Case::NativeKeepaliveGap);
        exercise(direct, Case::NativeKeepaliveMissing);
    }
    unsafe {
        std::env::remove_var("SATURN_PS_GENERATED_TRIALS");
        std::env::remove_var("SATURN_PS_GENERATED_FREQUENCY_HZ");
        std::env::remove_var("SATURN_PS_CORRECTION_MONITOR");
    }
}
#[test]
#[ignore = "serial actual-runtime Auto adaptation, release/re-arm and native cleanup; loopback/files only"]
fn actual_run_inner_auto_release_rearm_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::AutoCorrection);
    }
}
#[test]
#[ignore = "serial real WDSP startup Auto ATT, both actual backends; loopback/files only"]
fn actual_run_inner_auto_attenuation_single_auto_and_cancellation() {
    for direct in [false, true] {
        for case in [
            Case::AutoAttSingle,
            Case::AutoCorrection,
            Case::AutoAttRelease,
            Case::AutoAttMissing,
            Case::AutoAttReconfigure,
            Case::AutoAttDisconnect,
            Case::AutoAttRange,
        ] {
            exercise(direct, case);
        }
    }
}
#[test]
#[ignore = "serial sustained Auto: two 15-second generations per backend, real native DSP, file/loopback only"]
fn actual_run_inner_sustained_auto_both_backends() {
    unsafe {
        std::env::set_var("SATURN_PS_CORRECTION_MONITOR", "1");
    }
    for direct in [false, true] {
        exercise(direct, Case::AutoSustained);
    }
    unsafe {
        std::env::remove_var("SATURN_PS_CORRECTION_MONITOR");
    }
}
#[test]
#[ignore = "serial real-WDSP receive-only runtime + live TCI meter; loopback/files only"]
fn actual_run_inner_live_receive_meter_both_backends() {
    // Silent normal-RX fixture: require recurring real WDSP readings and live
    // TCI delivery, including WDSP's -400 dB silence floor. This does not
    // certify an RF level or calibration.
    for direct in [false, true] {
        exercise(direct, Case::ReceiveMeter);
    }
}
#[test]
#[ignore = "45-second real-native uncorrected stimulus per backend; offline files/loopback only"]
fn actual_run_inner_45_second_stimulus_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::StimulusClient);
    }
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
#[ignore = "actual browser PS tab + real native run_inner; loopback/files only, no radio"]
fn actual_run_inner_browser_measurement_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::BrowserMeasurementClient);
    }
}
#[test]
#[ignore = "isolated Direct XDMA browser MeasurementOnly regression; files/loopback only, no radio"]
fn actual_run_inner_browser_measurement_direct_only() {
    exercise(true, Case::BrowserMeasurementClient);
}
#[test]
#[ignore = "Timing UI + real native/runtime, loopback and ordinary files only; no radio"]
fn actual_run_inner_browser_timing_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::BrowserTimingMeasurementClient);
    }
}
#[test]
#[ignore = "45-second Direct XDMA browser MeasurementOnly regression; files/loopback only, no radio"]
fn actual_run_inner_browser_long_measurement_direct_only() {
    exercise(true, Case::BrowserLongMeasurementClient);
}
#[test]
#[ignore = "45-second real Chrome two-tone AND AM per backend through actual runtime; files/loopback only"]
fn actual_run_inner_browser_long_measurement_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::BrowserLongMeasurementClient);
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
#[ignore = "serial real-WDSP rear PTT through actual runtime and backend status inputs; no hardware"]
fn actual_run_inner_rear_ptt_release_rearm_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::RearPtt);
    }
}
#[test]
#[ignore = "serial real-WDSP PS-off microphone and rear PTT; loopback/files only"]
fn actual_run_inner_ps_off_mic_and_rear_ptt_both_backends() {
    for direct in [false, true] {
        for case in [Case::RearPttBypass, Case::MicrophoneBypass] {
            exercise(direct, case);
        }
    }
}
#[test]
#[ignore = "serial actual runtime with read-only correction observer; no hardware"]
fn actual_run_inner_correction_monitor_both_backends() {
    std::env::set_var("SATURN_PS_CORRECTION_MONITOR", "1");
    for direct in [false, true] {
        let index = usize::from(direct);
        let before = crate::correction_monitor::published()[index];
        exercise(direct, Case::ShutdownActive);
        assert!(
            crate::correction_monitor::published()[index] > before,
            "real owner must deliver a native snapshot through the observer worker"
        );
    }
    std::env::remove_var("SATURN_PS_CORRECTION_MONITOR");
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

#[test]
#[ignore = "serial real-WDSP production runtime WAN spectrum/audio; loopback/files only"]
fn actual_run_inner_wan_spectrum_receive_both_backends() {
    for direct in [false, true] {
        exercise(direct, Case::ReceiveSpectrum);
    }
}

#[test]
#[ignore = "serial real WDSP RX audio to ordinary-file headphone adapter; no hardware"]
fn shared_rx_headphones_receive_processed_audio_without_browser_subscription_and_mute_for_tx() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    let mut m = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    m.desired.rx_headphones_available = true;
    m.desired.rx_volume_db = -9.;
    let config = BridgeConfig {
        tci_bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..BridgeConfig::default()
    };
    let (front, _) =
        TciFrontend::reserve_shared_startup(&config, Arc::new(Mutex::new(m.clone())), false)
            .unwrap();
    assert!(!front.media_demand().audio_stream_enabled);
    let radio = DirectFixture::new();
    let mut dsp = WdspRxEngine::new(&m).unwrap();
    // A real-sample 1 kHz tone in complex RX IQ: USB must produce nonzero audio.
    let iq: Vec<f32> = (0..12_288)
        .flat_map(|n| {
            [
                (0.1 * (std::f64::consts::TAU * 1000. * n as f64 / 384000.).cos()) as f32,
                0.,
            ]
        })
        .collect();
    let feed = |dsp: &mut WdspRxEngine, m: &RadioModel, transmitting| {
        for _ in 0..20 {
            publish_receive(
                &front,
                dsp,
                &iq,
                384000,
                m,
                Some(radio.radio.as_ref()),
                transmitting,
                None,
                None,
            );
        }
    };
    feed(&mut dsp, &m, false);
    assert_eq!(
        radio.radio.rx_headphone_observation().0,
        0,
        "default off must stay silent"
    );
    m.desired.rx_headphones_enabled = true;
    feed(&mut dsp, &m, false);
    let (frames, peak, volume) = radio.radio.rx_headphone_observation();
    assert!(
        frames > 0 && peak > 1e-5,
        "real processed audio reaches worker: frames={frames} peak={peak}"
    );
    assert_eq!(
        volume, -9.,
        "pass raw samples and one explicit RX volume to the existing worker"
    );
    feed(&mut dsp, &m, true);
    assert_eq!(
        radio.radio.rx_headphone_observation().0,
        frames,
        "RX audio is not submitted during TX"
    );
    m.desired.rx_headphones_enabled = false;
    feed(&mut dsp, &m, false);
    assert_eq!(
        radio.radio.rx_headphone_observation().0,
        frames,
        "Off stops RX submissions"
    );
    m.desired.rx_headphones_enabled = true;
    feed(&mut dsp, &m, false);
    assert!(
        radio.radio.rx_headphone_observation().0 > frames,
        "On resumes RX after TX/off"
    );
    let resumed = radio.radio.rx_headphone_observation().0;
    m.desired.rx_headphones_available = false;
    feed(&mut dsp, &m, false);
    assert_eq!(
        radio.radio.rx_headphone_observation().0,
        resumed,
        "unavailable local playback must not receive audio"
    );
}

#[test]
#[ignore = "serial actual runtime band-recall wire contract, matched WDSP, file/loopback owners; no RF"]
fn actual_run_inner_band_recall_80m_40m_roundtrip_both_backends() {
    for direct in [true, false] {
        exercise(direct, Case::BandRecall);
    }
}

#[test]
fn actual_run_inner_faulted_receive_tuning_programs_ddc6() {
    exercise(true, Case::FaultedReceiveTune);
}

#[test]
#[ignore = "serial matched WDSP; real orchestration, acquisition and TX adapters; file/loopback only"]
fn actual_run_inner_max_attenuation_unclipped_feedback_fits_and_rearms() {
    for direct in [false, true] {
        exercise(direct, Case::AutoAttLimit);
    }
}
