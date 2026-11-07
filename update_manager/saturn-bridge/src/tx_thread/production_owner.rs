//! Explicit production-owner construction. No default limits are invented and
//! no legacy/native consumer is constructed as a fallback on any failure.
use super::{
    feedback_owner::Link,
    ps_ingress::SharedIngress,
    ps_owner_loop::{OwnerLoop, Timing},
};
use crate::{
    p2::session::P2Session, radio_model::RadioModel, sync_ext::MutexExt, tci::TciFrontend,
    tx_audio::TxAudioSource, tx_owner_stop::OwnerStopAdapter, wdsp::ps_native::FullNative,
    xdma_tx_radio::DirectXdmaTxRadio,
};
use saturn_bridge::puresignal::admission_proposal::DraftLimits;
use saturn_bridge::puresignal::{
    controller::{Config, Controller, Qualification},
    tx_lifecycle::{Limits, Phase, TxLifecycle},
    Backend, Fault, OverloadSource, Owner,
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
pub(crate) struct Options {
    pub(crate) config: Config,
    pub(crate) timing: Timing,
    pub(crate) limits: Limits,
    pub(crate) max_watts: u8,
    pub(crate) feedback_topology_reviewed: bool,
    pub(crate) managed_rf_permitted: bool,
    pub(crate) operational_policy: Option<DraftLimits>,
    pub(crate) startup_timeout_ns: u64,
    pub(crate) feedback_attenuation_db: u8,
    pub(crate) measurement_only: bool,
    pub(crate) generated_trials_only: bool,
}
impl Options {
    pub(crate) fn from_env() -> Result<Self, String> {
        // Startup selection is host-only. A browser URL/configuration cannot
        // manufacture topology review or RF permission.
        fn number<T: std::str::FromStr>(name: &str) -> Result<T, String> {
            std::env::var(name)
                .map_err(|_| format!("explicit {name} required"))?
                .parse()
                .map_err(|_| format!("invalid {name}"))
        }
        fn flag(name: &str) -> Result<bool, String> {
            match std::env::var(name).as_deref() {
                Ok("1") => Ok(true),
                Ok("0") | Err(std::env::VarError::NotPresent) => Ok(false),
                _ => Err(format!("{name} must be 0 or 1")),
            }
        }
        let options = Self {
            config: Config {
                id: 1,
                route: 1,
                physical_adc: OverloadSource::Adc1,
                enabled: false,
                automatic: false,
                hardware_peak: number("SATURN_PS_HARDWARE_PEAK")?,
                requested_delay_seconds: number("SATURN_PS_DELAY_SECONDS")?,
                loop_delay_seconds: number("SATURN_PS_LOOP_DELAY_SECONDS")?,
                settling_ns: number("SATURN_PS_SETTLING_NS")?,
                stale_after_ns: number("SATURN_PS_FEEDBACK_MAX_AGE_NS")?,
            },
            timing: Timing {
                mic_age_ns: number("SATURN_PS_MIC_MAX_AGE_NS")?,
                source_stall_ns: number("SATURN_PS_SOURCE_STALL_NS")?,
                tx_duration_ns: number("SATURN_PS_TX_DURATION_NS")?,
            },
            limits: Limits {
                authority_timeout_ns: number("SATURN_PS_AUTHORITY_TIMEOUT_NS")?,
                cleanup_timeout_ns: number("SATURN_PS_CLEANUP_TIMEOUT_NS")?,
            },
            max_watts: number("SATURN_PS_MAX_WATTS")?,
            startup_timeout_ns: number("SATURN_PS_STARTUP_TIMEOUT_NS")?,
            feedback_attenuation_db: number("SATURN_PS_FEEDBACK_ATTENUATION_DB")?,
            measurement_only: flag("SATURN_PS_MEASUREMENT_ONLY")?,
            generated_trials_only: flag("SATURN_PS_GENERATED_TRIALS_ONLY")?,
            feedback_topology_reviewed: flag("SATURN_PS_FEEDBACK_TOPOLOGY_REVIEWED")?,
            managed_rf_permitted: flag("SATURN_PS_MANAGED_RF_PERMITTED")?,
            operational_policy: match std::env::var("SATURN_PS_ADMISSION").as_deref() {
                Err(std::env::VarError::NotPresent) | Ok("strict") => None,
                Ok("operational") => Some(DraftLimits {
                    active_reference_fraction: number("SATURN_PS_ACTIVE_REFERENCE_FRACTION")?,
                    feedback_rms_floor: number("SATURN_PS_FEEDBACK_RMS_FLOOR")?,
                    component_limit: number("SATURN_PS_COMPONENT_LIMIT")?,
                }),
                _ => return Err("SATURN_PS_ADMISSION must be strict or operational".into()),
            },
        };
        if !options.config.hardware_peak.is_finite()
            || options.config.hardware_peak <= 0.
            || options.config.hardware_peak > 1.
            || !options.config.requested_delay_seconds.is_finite()
            || options.config.requested_delay_seconds.abs() > 0.1
            || !options.config.loop_delay_seconds.is_finite()
            || options.config.loop_delay_seconds < 0.
            || options.config.settling_ns > 10_000_000_000
            || options.config.stale_after_ns == 0
            || options.config.stale_after_ns > 1_000_000_000
            || options.max_watts == 0
            || options.max_watts > 100
            || options.startup_timeout_ns == 0
            || options.feedback_attenuation_db > 31
            || options.timing.mic_age_ns == 0
            || options.timing.source_stall_ns == 0
            || options.limits.authority_timeout_ns == 0
            || options.limits.cleanup_timeout_ns == 0
        {
            return Err("invalid explicit managed lifecycle limits".into());
        }
        if cfg!(saturn_bridge_stub_native) || env!("SATURN_BRIDGE_SOURCE_PIN") == "unverified" {
            return Err("managed startup requires a pinned real-native executable".into());
        }
        if !options.feedback_topology_reviewed {
            return Err("managed startup requires reviewed feedback topology attestation; default is unavailable".into());
        }
        if (options.measurement_only || options.managed_rf_permitted)
            && std::env::var("SATURN_PS_FEEDBACK_INPUT").as_deref() != Ok("rx-bypass")
        {
            return Err("explicit SATURN_PS_FEEDBACK_INPUT=rx-bypass required".into());
        }
        if options.generated_trials_only
            && (options.measurement_only
                || options.max_watts > 5
                || std::env::var("SATURN_PS_GENERATED_TRIALS").as_deref() != Ok("1"))
        {
            return Err("generated-only mode requires explicit generated trials, <=5 commanded W, and non-MeasurementOnly owner".into());
        }
        if options.measurement_only
            && (options.operational_policy.is_some()
                || options.max_watts > super::ps_measurement::MAX_DRIVE_WATTS
                || options.timing.tx_duration_ns
                    > super::ps_measurement::CaptureMode::from_env()?.max_duration_ms() * 1_000_000)
        {
            return Err(
                "MeasurementOnly requires strict admission, <=5 commanded W and the selected capture-mode duration bound"
                    .into(),
            );
        }
        Ok(options)
    }
}
#[derive(Clone)]
pub(crate) enum Radio {
    P2(Arc<P2Session>),
    Direct(Arc<DirectXdmaTxRadio>),
}
pub(crate) struct Runtime {
    _signal_capture: Option<crate::tx_signal_capture::shared::Worker>,
    pub(crate) ingress: SharedIngress,
    pub(crate) link: Link,
    pub(crate) origin: Instant,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) done: mpsc::Receiver<Result<(), Fault>>,
    pub(crate) worker: Option<JoinHandle<()>>,
}
pub(crate) fn spawn(
    radio: Radio,
    tci: Arc<TciFrontend>,
    model: Arc<Mutex<RadioModel>>,
    source: TxAudioSource,
    options: Options,
    rf_gate: bool,
    keepalive_period: std::time::Duration,
) -> Result<Runtime, Fault> {
    if keepalive_period.is_zero() {
        return Err(Fault::InvalidConfig);
    }
    // Default off, separately reviewed from MeasurementOnly. No inferred
    // frequency, feedback limits, RF permission or automatic PS enablement.
    let two_tone_limits = match std::env::var("SATURN_PS_GENERATED_TRIALS").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("0") => None,
        Ok("1") if !options.measurement_only => {
            match std::env::var("SATURN_PS_GENERATED_OPERATIONAL").as_deref() {
                Ok("1") => Some(super::ps_ingress::TwoToneLimits {
                    frequency_hz: 0,
                    max_ms: 0,
                }),
                Ok("0") | Err(std::env::VarError::NotPresent) => {
                    Some(super::ps_ingress::TwoToneLimits {
                        frequency_hz: std::env::var("SATURN_PS_GENERATED_FREQUENCY_HZ")
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .ok_or(Fault::InvalidConfig)?,
                        max_ms: if options.timing.tx_duration_ns == 0 {
                            45_000
                        } else {
                            45_000.min(options.timing.tx_duration_ns / 1_000_000)
                        },
                    })
                }
                _ => return Err(Fault::InvalidConfig),
            }
        }
        _ => return Err(Fault::InvalidConfig),
    };
    let measurement_host = if options.measurement_only {
        let path = std::env::var_os("SATURN_PS_MEASUREMENT_DIR").ok_or(Fault::InvalidConfig)?;
        let frequency = std::env::var("SATURN_PS_MEASUREMENT_FREQUENCY_HZ")
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(Fault::InvalidConfig)?;
        Some(
            super::ps_measurement::Host::open_mode(
                std::path::Path::new(&path),
                frequency,
                super::ps_measurement::CaptureMode::from_env().map_err(|_| Fault::InvalidConfig)?,
            )
            .map_err(|_| Fault::InvalidConfig)?,
        )
    } else {
        None
    };
    let origin = Instant::now();
    // Optional diagnostics cannot prevent radio startup. MeasurementOnly's
    // explicitly requested recorder retains its own strict evidence contract.
    let signal_capture = crate::tx_signal_capture::shared::Worker::from_env(origin)
        .unwrap_or_else(|error| {
            eprintln!("saturn-bridge: optional shared diagnostics unavailable: {error}; radio startup continues");
            None
        });
    let diagnostic = signal_capture.as_ref().map(|w| w.producer());
    let link = Link::new();
    let thread_link = link.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let (done_tx, done) = mpsc::sync_channel(1);
    let operator = tci.shared_operator_ingress();
    let worker = thread::Builder::new()
        .name("saturn-shared-tx".into())
        .spawn(move || {
            let result = (|| -> Result<(), Fault> {
                use super::TxRadio;
                let priority = match &radio {
                    Radio::P2(r) => r.realtime_priority(),
                    Radio::Direct(r) => r.realtime_priority(),
                };
                // Capture the original process mask before pinning the owner.
                // Native workers retain SCHED_OTHER on the other allowed CPUs,
                // including every channel recreation during cleanup.
                let worker_policy = crate::wdsp::native_worker_scheduling::WorkerPolicy::capture(
                    priority.is_some(),
                )?;
                #[cfg(not(test))]
                super::configure_tx_thread_scheduling(priority).map_err(|_| Fault::TxAuthority)?;
                #[cfg(test)]
                let _ = priority; // fixture evidence never claims installed RT scheduling
                                  // Construct the !Send native engine inside its sole owner thread.
                let mut snapshot = model.lock_unpoisoned().clone();
                snapshot.desired.pure_signal_attenuation_db = options.feedback_attenuation_db;
                let native = FullNative::matched_with_worker_policy(&snapshot, worker_policy)?;
                let mut iq = VecDeque::new();
                let mut mic = VecDeque::new();
                let (mut output, backend) = match &radio {
                    Radio::P2(session) => (
                        OwnerStopAdapter::p2(session.clone(), &snapshot, &mut iq, &mut mic)?,
                        Backend::P2,
                    ),
                    Radio::Direct(radio) => (
                        OwnerStopAdapter::direct(radio.clone(), &mut iq, &mut mic)?,
                        Backend::DirectXdma,
                    ),
                };
                output.enable_production_handoff(
                    &snapshot,
                    options.max_watts,
                    rf_gate && options.managed_rf_permitted,
                )?;
                output.attach_feedback_owner(thread_link, options.config.route)?;
                if let Some(limits) = options
                    .operational_policy
                    .filter(|_| !options.measurement_only)
                {
                    output.enable_auto_attenuation(
                        options.config.hardware_peak,
                        limits.component_limit,
                        options.config.stale_after_ns,
                    )?;
                }
                // Strict default retained. Missing overload coverage is never
                // fabricated; operational policy limits remain a separate review.
                let boot = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| Fault::InvalidConfig)?
                    .as_nanos();
                let qualification = Qualification {
                    native_abi_verified: true,
                    feedback_topology_verified: options.feedback_topology_reviewed,
                };
                let controller = match options.operational_policy {
                    None => Controller::new(native, Owner::Bridge, backend, boot, qualification, 4),
                    Some(limits) => Controller::new_offline_admission_proposal(
                        native,
                        Owner::Bridge,
                        backend,
                        boot,
                        qualification,
                        4,
                        limits,
                    )?,
                };
                let life = TxLifecycle::new(controller, output, backend, options.limits)?;
                #[cfg(feature = "managed-puresignal")]
                let correction_model = model.clone();
                let (mut owner, ingress) =
                    OwnerLoop::with_shared_ingress(life, operator, model, origin, options.timing)?;
                ingress.set_drive_ceiling(options.max_watts)?;
                if options.operational_policy.is_some() && !options.measurement_only {
                    ingress.enable_auto_mode()?;
                    ingress.enable_auto_attenuation()?;
                }
                if let Some(p) = diagnostic {
                    owner.attach_signal_capture(p)?;
                }
                if let Some(host) = measurement_host {
                    ingress.enable_measurement_only(host)?;
                }
                if let Some(limits) = two_tone_limits {
                    ingress.enable_two_tone_trials(limits)?;
                }
                if options.generated_trials_only {
                    ingress.restrict_to_generated_trials()?;
                }
                ingress.enable_tagged_pcm_review()?;
                // Parent attaches acquisition BEFORE accepting any client.
                startup_tx
                    .send(Ok(ingress.clone()))
                    .map_err(|_| Fault::TxAuthority)?;
                let mut bootstrapped = false;
                let mut admitted_configuration = false;
                let mut last_keepalive = origin;
                #[cfg(feature = "managed-puresignal")]
                let mut correction_poll = crate::correction_monitor::Poller::new();
                owner.run(&thread_stop, |owner, _| {
                    admitted_configuration |= ingress.startup_ready();
                    if !admitted_configuration
                        && origin.elapsed().as_nanos() > u128::from(options.startup_timeout_ns)
                    {
                        ingress.stop(Fault::CleanupTimeout);
                        thread_stop.store(true, Ordering::Release);
                    }
                    if !bootstrapped && owner.life.status().phase == Phase::ReadyOff {
                        if ingress
                            .bootstrap(snapshot.clone(), options.config, source)
                            .is_ok()
                        {
                            bootstrapped = true;
                        }
                    }
                    if last_keepalive.elapsed() >= keepalive_period {
                        if let Err(fault) = owner.life.offline_owner().keyed_keepalive() {
                            if fault != Fault::GenerationMismatch {
                                ingress.stop(fault);
                            }
                        }
                        last_keepalive = Instant::now();
                    }
                    #[cfg(feature = "managed-puresignal")]
                    if matches!(
                        owner.life.status().phase,
                        Phase::ReadyOff | Phase::AwaitingKey | Phase::Active
                    ) {
                        if let Some(input) = correction_poll.due() {
                            if let Ok(model) = correction_model.try_lock() {
                                let auto_att =
                                    owner.life.offline_owner().auto_attenuation_status(false);
                                let keyed =
                                    owner.life.status().phase == Phase::Active || auto_att.keyed;
                                owner.life.offline_native_snapshot(|n| {
                                    n.observe_correction(&input, &model, keyed, auto_att.applied_db)
                                });
                            }
                        }
                    }
                });
                #[cfg(test)]
                eprintln!("shared owner completed counters={:?}", owner.counters);
                let status = owner.life.status();
                if status.phase != Phase::ReadyOff
                    || status.primary_fault.is_some()
                    || status.cleanup_fault.is_some()
                    || status.authority_fault.is_some()
                {
                    return Err(status
                        .primary_fault
                        .or(status.cleanup_fault)
                        .or(status.authority_fault)
                        .unwrap_or(Fault::TxAuthority));
                }
                Ok(())
            })();
            if let Err(fault) = result {
                let _ = startup_tx.try_send(Err(fault));
            }
            let _ = done_tx.send(result);
        })
        .map_err(|_| Fault::TxAuthority)?;
    let startup_budget = options.startup_timeout_ns;
    let ingress = match startup_rx.recv_timeout(std::time::Duration::from_nanos(startup_budget)) {
        Ok(Ok(ingress)) => ingress,
        Ok(Err(fault)) => {
            stop.store(true, Ordering::Release);
            worker.join().map_err(|_| Fault::TxAuthority)?;
            return Err(fault);
        }
        Err(_) => {
            stop.store(true, Ordering::Release);
            // Startup is unarmed and no acquisition scope has been installed.
            // Do not detach cold native construction on timeout. This join is
            // not a claim of bounded hardware startup latency.
            worker.join().map_err(|_| Fault::TxAuthority)?;
            return Err(Fault::CleanupTimeout);
        }
    };
    if tci.install_shared_tx(ingress.clone()).is_err() {
        ingress.release();
        stop.store(true, Ordering::Release);
        worker.join().map_err(|_| Fault::TxAuthority)?;
        return Err(Fault::TxAuthority);
    }
    Ok(Runtime {
        _signal_capture: signal_capture,
        ingress,
        link,
        origin,
        stop,
        done,
        worker: Some(worker),
    })
}
impl Runtime {
    pub(crate) fn request_stop(&self) {
        self.ingress.release();
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.request_stop();
        // Normal orchestration keeps acquisition alive through completion and
        // takes this handle itself. A premature drop must still not detach the
        // owner; absent restoration it will fail/retain native state, not report
        // clean. Immediate output inhibition happens before this join.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(all(test, not(saturn_bridge_stub_native)))]
#[path = "production_tests.rs"]
pub(crate) mod integration_tests;

#[cfg(all(test, not(saturn_bridge_stub_native)))]
mod tests {
    use super::*;
    use crate::{
        config::BridgeConfig,
        tx_owner_stop::tests::{model, p2_separate_lanes},
        xdma_tx_radio::offline_fixture::DirectFixture,
    };
    #[test]
    #[ignore = "explicit matched-native fixture, run serially with the real reviewed archive"]
    fn matched_production_startup_single_owner_and_completed_shutdown_both_backends() {
        startup_fixture(false);
    }
    #[test]
    #[ignore = "explicit generated-only matched-native production constructor; no hardware or RF"]
    fn matched_production_generated_only_single_owner_and_shutdown_both_backends() {
        startup_fixture(true);
    }
    fn startup_fixture(generated_only: bool) {
        struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Environment {
            fn drop(&mut self) {
                for (name, value) in &self.0 {
                    if let Some(value) = value {
                        std::env::set_var(name, value);
                    } else {
                        std::env::remove_var(name);
                    }
                }
            }
        }
        let _environment = Environment(
            [
                "SATURN_PS_GENERATED_TRIALS",
                "SATURN_PS_GENERATED_FREQUENCY_HZ",
            ]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect(),
        );
        std::env::set_var(
            "SATURN_PS_GENERATED_TRIALS",
            if generated_only { "1" } else { "0" },
        );
        if generated_only {
            std::env::set_var("SATURN_PS_GENERATED_FREQUENCY_HZ", "7200000");
        }
        // Same local FFT planner cap as the existing combined native replay.
        // Not applied by production startup; no installed latency claim.
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        for direct in [false, true] {
            let mut initial = model();
            initial.desired.tx_enabled = false;
            initial.desired.tx_drive = 5;
            initial.desired.running = true;
            let shared_model = Arc::new(Mutex::new(initial.clone()));
            let frontend_config = BridgeConfig {
                tci_bind_addr: "127.0.0.1:0".parse().unwrap(),
                ..BridgeConfig::default()
            };
            let (front, _commands) =
                TciFrontend::reserve_shared_startup(&frontend_config, shared_model.clone(), false)
                    .unwrap();
            assert!(
                front.admit_shared_clients().is_err(),
                "reserved startup has no installed shared owner"
            );
            let front = Arc::new(front);
            let p2_fixture = (!direct).then(p2_separate_lanes);
            let direct_fixture = direct.then(DirectFixture::new);
            if let Some(fixture) = &direct_fixture {
                fixture.fifo_ready();
            }
            let radio = match &p2_fixture {
                Some((session, _, _)) => Radio::P2(session.clone()),
                None => Radio::Direct(direct_fixture.as_ref().unwrap().radio.clone()),
            };
            let options = Options {
                config: Config {
                    id: 1,
                    route: 1,
                    physical_adc: OverloadSource::Adc1,
                    enabled: false,
                    automatic: false,
                    hardware_peak: if generated_only { 0.67 } else { 0.6121 },
                    requested_delay_seconds: 0.,
                    settling_ns: 1_000_000,
                    loop_delay_seconds: 0.,
                    stale_after_ns: 150_000_000,
                },
                timing: Timing {
                    mic_age_ns: 150_000_000,
                    source_stall_ns: 500_000_000,
                    tx_duration_ns: 10_000_000_000,
                },
                // Cold FFT planning on a local fixture, NOT a hardware stop bound.
                limits: Limits {
                    authority_timeout_ns: 30_000_000_000,
                    cleanup_timeout_ns: 30_000_000_000,
                },
                // Fixture attestation exercises the positive constructor, not
                // physical RXC1 topology verification. RF remains forbidden.
                max_watts: if generated_only { 5 } else { 100 },
                feedback_topology_reviewed: true,
                managed_rf_permitted: false,
                operational_policy: generated_only.then_some(DraftLimits {
                    active_reference_fraction: 0.1,
                    feedback_rms_floor: 0.01,
                    component_limit: 0.95,
                }),
                startup_timeout_ns: 60_000_000_000,
                feedback_attenuation_db: 9,
                measurement_only: false,
                generated_trials_only: generated_only,
            };
            let mut runtime = spawn(
                radio,
                front.clone(),
                shared_model.clone(),
                crate::tx_audio::TxAudioSource::Tci,
                options,
                false,
                std::time::Duration::from_millis(200),
            )
            .unwrap();
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while !runtime.ingress.startup_ready() {
                assert!(Instant::now() < deadline, "startup did not complete");
                thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(!shared_model.lock_unpoisoned().desired.tx_enabled);
            assert_eq!(
                runtime.ingress.drive_limit_message(),
                format!("saturn_tx_drive_limit:0,{};", options.max_watts)
            );
            assert_eq!(shared_model.lock_unpoisoned().desired.tx_drive, 5);
            assert_eq!(runtime.ingress.generated_trials_only(), generated_only);
            assert!(
                crate::wdsp::WdspTxEngine::try_new(&initial).is_err(),
                "parallel legacy native owner"
            );
            front.admit_shared_clients().unwrap();
            runtime.request_stop();
            assert!(runtime
                .done
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .is_ok());
            runtime.worker.take().unwrap().join().unwrap();
            assert!(runtime.link.restored().unwrap());
            println!("matched production constructor backend={} strict_policy={} generated_only={} topology_fixture_attestation=true rf_permission=false shutdown_complete=true legacy_native_rejected=true", if direct { "DirectXdma" } else { "P2" }, options.operational_policy.is_none(), generated_only);
        }
    }
}
