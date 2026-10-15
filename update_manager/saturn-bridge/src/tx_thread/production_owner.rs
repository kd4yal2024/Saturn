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
            || options.timing.tx_duration_ns == 0
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
        Ok(options)
    }
}
#[derive(Clone)]
pub(crate) enum Radio {
    P2(Arc<P2Session>),
    Direct(Arc<DirectXdmaTxRadio>),
}
pub(crate) struct Runtime {
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
    let origin = Instant::now();
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
                #[cfg(not(test))]
                super::configure_tx_thread_scheduling(priority).map_err(|_| Fault::TxAuthority)?;
                #[cfg(test)]
                let _ = priority; // fixture evidence never claims installed RT scheduling
                                  // Construct the !Send native engine inside its sole owner thread.
                let mut snapshot = model.lock_unpoisoned().clone();
                snapshot.desired.pure_signal_attenuation_db = options.feedback_attenuation_db;
                let native = FullNative::matched(&snapshot)?;
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
                let (mut owner, ingress) =
                    OwnerLoop::with_shared_ingress(life, operator, model, origin, options.timing)?;
                ingress.enable_tagged_pcm_review()?;
                // Parent attaches acquisition BEFORE accepting any client.
                startup_tx
                    .send(Ok(ingress.clone()))
                    .map_err(|_| Fault::TxAuthority)?;
                let mut bootstrapped = false;
                let mut admitted_configuration = false;
                let mut last_keepalive = origin;
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
                });
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
                    hardware_peak: 0.6121,
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
                max_watts: 5,
                feedback_topology_reviewed: true,
                managed_rf_permitted: false,
                operational_policy: None,
                startup_timeout_ns: 60_000_000_000,
                feedback_attenuation_db: 9,
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
            println!("matched production constructor backend={} strict_policy=true topology_fixture_attestation=true rf_permission=false shutdown_complete=true legacy_native_rejected=true", if direct { "DirectXdma" } else { "P2" });
        }
    }
}
