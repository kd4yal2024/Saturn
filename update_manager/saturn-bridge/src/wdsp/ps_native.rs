//! Full TX engine wrapper. One engine owns mic DSP, pscc and cleanup.
//! Legacy PS controls/reset/stop loops are never used by this lifecycle port.
//! Explicit managed startup uses the build-matched constructor; Legacy stays default.
#![allow(dead_code)]
use super::*;
use crate::tx_thread::ps_owner_loop::ScheduledNative;
use saturn_bridge::puresignal::{
    controller::{Config, NativePs},
    tx_lifecycle::{CleanupProgress, CooperativeNative},
    Fault,
};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static CLAIMED: AtomicBool = AtomicBool::new(false);
#[cfg(all(test, feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
#[path = "ps_native_retained_review.rs"]
mod retained_review;
pub(crate) const MEASUREMENT_MIC_GAIN_DB: f64 = 0.;
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct Audit {
    pub(crate) exchanges: u64,
    pub(crate) feedback_blocks: u64,
    pub(crate) model_updates: u64,
    pub(crate) recreations: u64,
    pub(crate) cleanup_steps: u64,
    pub(crate) dexp_exchanges: u64,
}
pub(crate) struct FullNative {
    engine: Option<WdspTxEngine>,
    worker_policy: super::native_worker_scheduling::WorkerPolicy,
    model: RadioModel,
    automatic: bool,
    config: Option<(Config, f64)>,
    flags: fn() -> i32, // pinned native IQC atomics, verified by offline runner
    history: Option<usize>,
    cleanup: Option<(bool, u32)>,
    closing_worker: bool,
    quiescent: bool,
    measurement_gain: bool,
    diagnostic_source: crate::tx_thread::feedback_owner::Stimulus,
    audit: Audit,
    _single_owner: PhantomData<Rc<()>>, // !Send / !Sync
}
impl FullNative {
    #[cfg(all(feature = "managed-puresignal", saturn_bridge_stub_native))]
    pub(crate) fn matched(_: &RadioModel) -> Result<Self, Fault> {
        Err(Fault::NativeUnverified)
    }
    #[cfg(all(feature = "managed-puresignal", saturn_bridge_stub_native))]
    pub(crate) fn matched_with_worker_policy(
        _: &RadioModel,
        _: super::native_worker_scheduling::WorkerPolicy,
    ) -> Result<Self, Fault> {
        Err(Fault::NativeUnverified)
    }
    #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
    pub(crate) fn matched(model: &RadioModel) -> Result<Self, Fault> {
        Self::matched_with_worker_policy(
            model,
            super::native_worker_scheduling::WorkerPolicy::capture(false)?,
        )
    }
    #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
    pub(crate) fn matched_with_worker_policy(
        model: &RadioModel,
        worker_policy: super::native_worker_scheduling::WorkerPolicy,
    ) -> Result<Self, Fault> {
        unsafe extern "C" {
            fn saturn_shared_iqc_flags() -> i32;
            fn saturn_shared_satp_history() -> i32;
            fn GetWDSPVersion() -> i32;
        }
        fn flags() -> i32 {
            unsafe { saturn_shared_iqc_flags() }
        }
        let history = unsafe { saturn_shared_satp_history() };
        if history != 140 {
            return Err(Fault::NativeUnverified);
        }
        Self::offline_with_worker_policy(
            model,
            unsafe { GetWDSPVersion() },
            Some(history as usize),
            flags,
            worker_policy,
        )
    }
    pub(crate) fn offline(
        model: &RadioModel,
        version: i32,
        history: Option<usize>,
        flags: fn() -> i32,
    ) -> Result<Self, Fault> {
        Self::offline_with_worker_policy(
            model,
            version,
            history,
            flags,
            super::native_worker_scheduling::WorkerPolicy::capture(false)?,
        )
    }
    fn offline_with_worker_policy(
        model: &RadioModel,
        version: i32,
        history: Option<usize>,
        flags: fn() -> i32,
        worker_policy: super::native_worker_scheduling::WorkerPolicy,
    ) -> Result<Self, Fault> {
        if version != 210 || history.is_some_and(|h| h != 140) {
            return Err(Fault::NativeUnverified);
        }
        if model.desired.two_tone_enabled
            || matches!(model.desired.mode, DemodMode::Wfm | DemodMode::Unknown)
        {
            return Err(Fault::InvalidConfig);
        }
        if CLAIMED.swap(true, Ordering::SeqCst) {
            return Err(Fault::ExternalController);
        }
        let mut model = model.clone();
        model.desired.pure_signal_enabled = false;
        model.desired.two_tone_enabled = false;
        let mut engine = match worker_policy
            .construct(|| WdspTxEngine::new_shared_owner(&model))
            .and_then(|v| v)
        {
            Ok(engine) => engine,
            Err(fault) => {
                CLAIMED.store(false, Ordering::SeqCst);
                return Err(fault);
            }
        };
        engine.sync_model(&model);
        engine.set_active(true);
        Ok(Self {
            engine: Some(engine),
            worker_policy,
            model,
            automatic: false,
            config: None,
            flags,
            history,
            cleanup: None,
            closing_worker: false,
            quiescent: true,
            measurement_gain: false,
            diagnostic_source: Default::default(),
            audit: Audit::default(),
            _single_owner: PhantomData,
        })
    }
    pub(crate) fn audit(&self) -> Audit {
        self.audit
    }
    pub(crate) fn diagnostics(&self) -> TxDspDiagnostics {
        self.engine.as_ref().unwrap().diagnostics()
    }
    #[cfg(feature = "managed-puresignal")]
    pub(crate) fn observe_correction(
        &self,
        input: &crate::correction_monitor::Input,
        model: &RadioModel,
        keyed: bool,
        attenuation: Option<u8>,
    ) {
        if let Some(engine) = &self.engine {
            engine.observe_correction_with_attenuation(input, model, keyed, attenuation);
        }
    }
    fn exchange(&mut self, samples: &[f32]) -> Result<Vec<f32>, Fault> {
        #[cfg(test)]
        let started = std::time::Instant::now();
        self.audit.exchanges += 1;
        if self.engine.as_ref().unwrap().dexp_initialized
            && self.engine.as_ref().unwrap().dexp_active()
        {
            self.audit.dexp_exchanges += 1;
        }
        let result = self.engine.as_mut().unwrap().offline_render_block(samples);
        #[cfg(test)]
        if started.elapsed() > std::time::Duration::from_millis(2) {
            eprintln!(
                "native fixture exchange us={}",
                started.elapsed().as_micros()
            );
        }
        result
    }
    fn info(&self) -> [i32; 16] {
        let mut info = [0; 16];
        unsafe {
            GetPSInfo(WDSP_TX_CHANNEL, info.as_mut_ptr());
        }
        info
    }
    fn verify_generator(&self) -> Result<(), Fault> {
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            unsafe extern "C" {
                fn saturn_shared_generator_readback(values: *mut f64) -> i32;
            }
            let mut state = [f64::NAN; 6];
            if unsafe { saturn_shared_generator_readback(state.as_mut_ptr()) } != 1 {
                return Err(Fault::NativeUnverified);
            }
            if self.diagnostic_source == crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone {
                let sign = if self.model.desired.mode == DemodMode::Lsb {
                    -1.
                } else {
                    1.
                };
                if state != [1., 1., 0.48, 0.48, sign * 700., sign * 1900.] {
                    return Err(Fault::NativeFailure);
                }
            } else if state[0] != 0. {
                return Err(Fault::NativeFailure);
            }
            Ok(())
        }
        #[cfg(any(not(feature = "managed-puresignal"), saturn_bridge_stub_native))]
        Err(Fault::NativeUnverified)
    }
    #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
    fn mic_gain_readback(&self) -> f64 {
        unsafe extern "C" {
            fn saturn_shared_tx_mic_gain() -> f64;
        }
        unsafe { saturn_shared_tx_mic_gain() }
    }
    fn apply_config(c: Config) -> Result<f64, Fault> {
        let delay = unsafe {
            SetPSFeedbackRate(WDSP_TX_CHANNEL, 192000);
            SetPSHWPeak(WDSP_TX_CHANNEL, c.hardware_peak);
            SetPSMoxDelay(WDSP_TX_CHANNEL, 0.);
            SetPSLoopDelay(WDSP_TX_CHANNEL, c.loop_delay_seconds);
            SetPSTXDelay(WDSP_TX_CHANNEL, c.requested_delay_seconds)
        };
        if !delay.is_finite() {
            return Err(Fault::NativeFailure);
        }
        Ok(delay)
    }
}
impl NativePs for FullNative {
    fn bypass_tx_guard(&mut self) -> Result<(), Fault> {
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            self.verify_generator()?;
            unsafe extern "C" {
                fn saturn_shared_measurement_flags() -> i32;
            }
            let expected_gain = if self.measurement_gain {
                1.0
            } else {
                panel_gain_for_volume_db(self.model.desired.tx_mic_gain_db)
            };
            if !self.config.is_some_and(|(c, _)| !c.enabled && !c.automatic)
                || self.automatic
                || self.cleanup.is_some()
                || self.mic_gain_readback() != expected_gain
                || unsafe { saturn_shared_measurement_flags() } != 0
                || self.info()[14] != 0
            {
                return Err(Fault::NativeFailure);
            }
            return Ok(());
        }
        #[cfg(any(not(feature = "managed-puresignal"), saturn_bridge_stub_native))]
        Err(Fault::NativeUnverified)
    }
    fn measurement_only_guard(&mut self) -> Result<(), Fault> {
        self.bypass_tx_guard()?;
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            if !self.measurement_gain || self.mic_gain_readback() != 1.0 {
                return Err(Fault::NativeFailure);
            }
            Ok(())
        }
        #[cfg(any(not(feature = "managed-puresignal"), saturn_bridge_stub_native))]
        Err(Fault::NativeUnverified)
    }
    fn configure(&mut self, c: Config) -> Result<f64, Fault> {
        let delay = Self::apply_config(c)?;
        self.automatic = c.automatic;
        self.config = Some((c, delay));
        Ok(delay)
    }
    fn mox(&mut self, active: bool) {
        if active {
            self.quiescent = false;
            unsafe {
                SetPSControl(
                    WDSP_TX_CHANNEL,
                    0,
                    i32::from(!self.automatic),
                    i32::from(self.automatic),
                    0,
                );
            }
        }
        unsafe {
            SetPSMox(WDSP_TX_CHANNEL, i32::from(active));
        }
    }
    fn reset(&mut self) -> Result<(), Fault> {
        Err(Fault::NativeFailure)
    } // attached lifecycle must never call blocking reset
    fn process(&mut self, t: &mut [f64; 2048], r: &mut [f64; 2048]) -> Result<(), Fault> {
        self.quiescent = false;
        self.audit.feedback_blocks += 1;
        unsafe {
            pscc(WDSP_TX_CHANNEL, 1024, t.as_mut_ptr(), r.as_mut_ptr());
        }
        Ok(())
    }
    fn information(&mut self) -> Result<[i32; 16], Fault> {
        Ok(self.info())
    }
}
impl ScheduledNative for FullNative {
    fn startup_leveling_guard(&mut self) -> Result<bool, Fault> {
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            unsafe extern "C" {
                fn saturn_try_shared_leveling_flags(flags: *mut i32) -> i32;
            }
            if self.cleanup.is_some() {
                return Err(Fault::NativeFailure);
            }
            let mut flags = -1;
            match unsafe { saturn_try_shared_leveling_flags(&mut flags) } {
                0 => Ok(false), // Retry on a later block without refreshing its age.
                1 if flags == 0 => Ok(true),
                _ => Err(Fault::NativeFailure),
            }
        }
        #[cfg(any(not(feature = "managed-puresignal"), saturn_bridge_stub_native))]
        Err(Fault::NativeUnverified)
    }
    fn diagnostic_stimulus(
        &mut self,
        stimulus: crate::tx_thread::feedback_owner::Stimulus,
    ) -> Result<(), Fault> {
        use crate::tx_thread::feedback_owner::Stimulus;
        if !self.quiescent || self.cleanup.is_some() {
            return Err(Fault::CleanupPending);
        }
        if stimulus == Stimulus::NativeTwoTone
            && (!self.measurement_gain
                || !matches!(self.model.desired.mode, DemodMode::Usb | DemodMode::Lsb))
        {
            return Err(Fault::InvalidConfig);
        }
        unsafe {
            if stimulus == Stimulus::NativeTwoTone {
                SetTXAPostGenRun(WDSP_TX_CHANNEL, 0);
                let sign = if self.model.desired.mode == DemodMode::Lsb {
                    -1.
                } else {
                    1.
                };
                SetTXAPostGenMode(WDSP_TX_CHANNEL, 1);
                SetTXAPostGenTTMag(WDSP_TX_CHANNEL, 0.48, 0.48);
                SetTXAPostGenTTFreq(WDSP_TX_CHANNEL, sign * 700., sign * 1900.);
                SetTXAPostGenRun(WDSP_TX_CHANNEL, 1);
            } else if self.diagnostic_source == Stimulus::NativeTwoTone {
                SetTXAPostGenRun(WDSP_TX_CHANNEL, 0);
            }
        }
        self.diagnostic_source = stimulus;
        self.verify_generator()
    }
    fn measurement_mic_gain(&mut self, enabled: bool) -> Result<(), Fault> {
        #[cfg(any(not(feature = "managed-puresignal"), saturn_bridge_stub_native))]
        return Err(Fault::NativeUnverified);
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            if !self.quiescent || self.cleanup.is_some() {
                return Err(Fault::CleanupPending);
            }
            let mut temporary = self.model.clone();
            if enabled {
                temporary.desired.tx_mic_gain_db = MEASUREMENT_MIC_GAIN_DB;
            }
            // self.model remains the committed normal profile. Channel recreation
            // on release, fault or disconnect restores it before declaring clean.
            self.engine
                .as_mut()
                .ok_or(Fault::NativeFailure)?
                .sync_model(&temporary);
            #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
            if self.mic_gain_readback()
                != panel_gain_for_volume_db(temporary.desired.tx_mic_gain_db)
            {
                return Err(Fault::NativeFailure);
            }
            self.measurement_gain = enabled;
            eprintln!("saturn-bridge: measurement mic gain override={} effective_db={} normal_db={} native_readback_verified=true",
            enabled, temporary.desired.tx_mic_gain_db, self.model.desired.tx_mic_gain_db);
            Ok(())
        }
    }
    fn configure_model(&mut self, model: &RadioModel) -> Result<(), Fault> {
        if !self.quiescent
            || self.cleanup.is_some()
            || model.desired.two_tone_enabled
            || matches!(model.desired.mode, DemodMode::Wfm | DemodMode::Unknown)
        {
            return Err(Fault::CleanupPending);
        }
        let mut snapshot = model.clone();
        snapshot.desired.pure_signal_enabled = false;
        self.engine.as_mut().unwrap().sync_model(&snapshot);
        self.model = snapshot;
        self.measurement_gain = false;
        self.audit.model_updates += 1;
        Ok(())
    }
    fn render(&mut self, mono: &[f32]) -> Result<Vec<f32>, Fault> {
        if self.diagnostic_source == crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone {
            if mono.iter().any(|v| *v != 0.) {
                return Err(Fault::InvalidConfig);
            }
            self.verify_generator()?;
        }
        self.exchange(mono)
    }
    fn satp_history_samples(&self) -> Option<usize> {
        self.history
    }
}
impl CooperativeNative for FullNative {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
        if self.closing_worker {
            return Ok(());
        }
        self.exchange(&[0.; 512]).map(|_| ())
    }
    fn begin_cleanup(&mut self) -> Result<(), Fault> {
        self.mox(false);
        // Authority stop receipt already completed. Never run the generator
        // through cleanup or allow recreation to retain its action identity.
        if self.diagnostic_source == crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone {
            unsafe {
                SetTXAPostGenRun(WDSP_TX_CHANNEL, 0);
            }
        }
        self.diagnostic_source = Default::default();
        self.engine.as_mut().unwrap().clear_tx_buffers();
        self.cleanup = Some((false, 0));
        Ok(())
    }
    fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault> {
        let (resetting, stable) = self.cleanup.ok_or(Fault::NativeFailure)?;
        if self.closing_worker {
            #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
            {
                unsafe extern "C" {
                    fn saturn_channel_shutdown_poll(channel: i32) -> i32;
                }
                match unsafe { saturn_channel_shutdown_poll(WDSP_TX_CHANNEL) } {
                    0 => return Ok(CleanupProgress::Pending),
                    1 => (),
                    _ => return Err(Fault::NativeUnverified),
                }
            }
            // Receipt and fitter/IQC quiescence precede channel recreation.
            // This also removes ALL old TX filter/ALC/EQ/CFC/DEXP sample state.
            let engine = self.engine.as_mut().unwrap();
            self.worker_policy
                .construct(|| engine.recreate_channel(&self.model))?;
            engine.sync_model(&self.model);
            engine.set_active(true);
            #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
            if self.mic_gain_readback()
                != panel_gain_for_volume_db(self.model.desired.tx_mic_gain_db)
            {
                return Err(Fault::NativeFailure);
            }
            self.measurement_gain = false;
            self.diagnostic_source = Default::default();
            // An ordinary release/re-arm has no pending configuration. Restore
            // the shared controller's retained contract before declaring clean.
            if let Some((config, actual_delay)) = self.config {
                if Self::apply_config(config)? != actual_delay {
                    return Err(Fault::NativeFailure);
                }
            }
            self.audit.recreations += 1;
            self.cleanup = None;
            self.closing_worker = false;
            self.quiescent = true;
            return Ok(CleanupProgress::Complete);
        }

        let mut t = [0.; 2048];
        let mut r = [0.; 2048];
        unsafe {
            pscc(WDSP_TX_CHANNEL, 1024, t.as_mut_ptr(), r.as_mut_ptr());
        }
        self.exchange(&[0.; 512])?;
        self.audit.cleanup_steps += 1;
        let info = self.info();
        let flags = (self.flags)();
        if !(0..=3).contains(&flags) {
            return Err(Fault::NativeUnverified);
        }
        let done = if resetting {
            info[15] == 0 && info[14] == 0 && flags == 0
        } else {
            matches!(info[15], 0 | 1 | 8)
        };
        let stable = if done { stable + 1 } else { 0 };
        if resetting && stable >= 2 {
            self.closing_worker = true;
            return Ok(CleanupProgress::Pending);
        }
        if !resetting && stable >= 2 {
            unsafe {
                SetPSControl(WDSP_TX_CHANNEL, 1, 0, 0, 0);
            }
            self.cleanup = Some((true, 0));
        } else {
            self.cleanup = Some((resetting, stable));
        }
        Ok(CleanupProgress::Pending)
    }
}
impl Drop for FullNative {
    fn drop(&mut self) {
        if !self.quiescent || self.cleanup.is_some() || (self.flags)() != 0 {
            // Same fail-closed lifetime rule as TxLifecycle; never destroy a
            // native channel with possible fitter/IQC work outstanding.
            if let Some(engine) = self.engine.take() {
                std::mem::forget(engine);
            }
            eprintln!("offline full native retained before quiescence");
            return;
        }
        drop(self.engine.take());
        CLAIMED.store(false, Ordering::SeqCst);
    }
}

#[cfg(all(test, feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
mod stimulus_tests {
    use super::*;

    #[test]
    #[ignore = "serial real WDSP ordinary microphone bypass; no devices or RF"]
    fn full_native_bypass_keeps_normal_mic_gain_and_measurement_remains_explicit() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        for gain_db in [-12., 0., 6.] {
            let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
            model.desired.tx_mic_gain_db = gain_db;
            let mut native = FullNative::matched(&model).unwrap();
            native.configure(timing_config(0.)).unwrap();
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(gain_db)
            );
            assert!(!native.measurement_gain);
            native
                .bypass_tx_guard()
                .expect("ordinary PS-off microphone must not require measurement gain");
            unsafe {
                SetTXAPanelGain1(WDSP_TX_CHANNEL, 0.0);
            }
            assert_eq!(native.bypass_tx_guard(), Err(Fault::NativeFailure));
            unsafe {
                SetTXAPanelGain1(WDSP_TX_CHANNEL, panel_gain_for_volume_db(gain_db));
            }
            native.bypass_tx_guard().unwrap();
            assert_eq!(native.measurement_only_guard(), Err(Fault::NativeFailure));
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(gain_db)
            );
            native.measurement_mic_gain(true).unwrap();
            native.measurement_only_guard().unwrap();
            native.measurement_mic_gain(false).unwrap();
            native.bypass_tx_guard().unwrap();
            assert_eq!(native.measurement_only_guard(), Err(Fault::NativeFailure));
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(gain_db)
            );
            let mut peak = 0f32;
            for block in 0..64 {
                let input: Vec<f32> = (0..512)
                    .map(|i| {
                        (std::f32::consts::TAU * 700. * (block * 512 + i) as f32 / 48000.).sin()
                            * 0.2
                    })
                    .collect();
                peak = native
                    .render(&input)
                    .unwrap()
                    .iter()
                    .fold(peak, |a, b| a.max(b.abs()));
            }
            assert!(
                peak > 0.001,
                "normal PS-off voice must produce nonzero IQ at {gain_db} dB"
            );
            assert_eq!(native.information().unwrap()[14], 0);
            let mut enabled = timing_config(0.);
            enabled.enabled = true;
            native.configure(enabled).unwrap();
            assert_eq!(native.bypass_tx_guard(), Err(Fault::NativeFailure));
            native.configure(timing_config(0.)).unwrap();
            native.begin_cleanup().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            native.bypass_tx_guard().unwrap();
            assert_eq!(native.measurement_only_guard(), Err(Fault::NativeFailure));
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(gain_db)
            );
        }
    }

    fn timing_config(delay: f64) -> Config {
        Config {
            id: 1,
            route: 1,
            physical_adc: saturn_bridge::puresignal::OverloadSource::Adc1,
            enabled: false,
            automatic: false,
            hardware_peak: 0.4,
            requested_delay_seconds: delay,
            settling_ns: 0,
            loop_delay_seconds: 0.,
            stale_after_ns: 100_000_000,
        }
    }

    #[test]
    #[ignore = "serial matched WDSP delay/reference readback, offline only; no devices or RF"]
    fn full_native_timing_contract_routes_signed_delay_and_scales_reference() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
            fn saturn_shared_ps_delay_readback(tx: *mut f64, rf: *mut f64, peak: *mut f64) -> i32;
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        let mut native = FullNative::matched(&model).unwrap();
        native.measurement_mic_gain(true).unwrap();
        for requested in [0., 13.7e-6, -13.7e-6, 100e-6, -100e-6] {
            let actual = native.configure(timing_config(requested)).unwrap();
            let (mut tx, mut rf, mut peak) = (f64::NAN, f64::NAN, f64::NAN);
            assert_eq!(
                unsafe { saturn_shared_ps_delay_readback(&mut tx, &mut rf, &mut peak) },
                1
            );
            assert!(
                (peak - 0.4).abs() < 1e-12,
                "HWPeak controls returned-reference normalization"
            );
            assert!((tx - rf - actual).abs() < 1e-12);
            assert!(
                (actual - requested).abs() < 21e-9,
                "native delay quantization"
            );
            if requested >= 0. {
                assert_eq!(rf, 0.);
                assert!(tx >= 0.);
            } else {
                assert_eq!(tx, 0.);
                assert!(rf > 0.);
            }
            native.measurement_only_guard().unwrap();
            assert_eq!(native.audit().feedback_blocks, 0);
            assert_eq!(native.info()[14], 0);
        }
    }

    #[test]
    #[ignore = "explicit finite local PCM fixture through full native TX; no devices, socket or RF"]
    fn full_native_timing_probe_reaches_iq_with_fitting_disabled() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        let input = std::path::PathBuf::from(
            std::env::var_os("SATURN_PS_TIMING_PCM").expect("explicit local timing fixture"),
        );
        let metadata = std::fs::symlink_metadata(&input).unwrap();
        assert!(metadata.is_file() && metadata.len() == 48_000 * 4);
        let bytes = std::fs::read(input).unwrap();
        let pcm: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert!(pcm.iter().all(|v| v.is_finite() && v.abs() <= 0.4));
        let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        model.desired.mode = DemodMode::Usb;
        let mut native = FullNative::matched(&model).unwrap();
        native.configure(timing_config(0.)).unwrap();
        native.measurement_mic_gain(true).unwrap();
        let mut output = Vec::new();
        for chunk in pcm.chunks(512) {
            let mut block = [0.; 512];
            block[..chunk.len()].copy_from_slice(chunk);
            native.measurement_only_guard().unwrap();
            output.extend(native.render(&block).unwrap());
        }
        native.measurement_only_guard().unwrap();
        assert!(output.iter().all(|v| v.is_finite()));
        let steady = &output[output.len() / 4..output.len() * 3 / 4];
        let peak = steady.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
        let rms =
            (steady.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / steady.len() as f64).sqrt();
        assert!(
            peak > 0.02 && rms > 0.005,
            "timing source must survive native TX filtering"
        );
        assert_eq!(native.audit().feedback_blocks, 0, "no fitter submission");
        assert_eq!(native.info()[14], 0, "no correction applied");
        let directory = std::path::PathBuf::from(
            std::env::var_os("SATURN_PS_TIMING_OUTPUT_DIR").expect("new offline output directory"),
        );
        std::fs::create_dir(&directory).unwrap();
        let raw: Vec<u8> = output.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(directory.join("wdsp-output-iq.f32le"), raw).unwrap();
        std::fs::write(directory.join("native-result.json"),format!("{{\"offlineOnly\":true,\"physicalRfExecuted\":false,\"fittingEnabled\":false,\"correctionApplied\":false,\"sourcePin\":\"{}\",\"rateHz\":192000,\"interleavedComponents\":{},\"steadyComponentPeak\":{peak},\"steadyComponentRms\":{rms}}}\n",env!("SATURN_BRIDGE_SOURCE_PIN"),output.len())).unwrap();
        native.begin_cleanup().unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while native.cleanup_step().unwrap() != CleanupProgress::Complete {
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    #[ignore = "serial matched real WDSP, no devices or RF"]
    fn full_native_measurement_gain_is_unity_and_cleanup_restores_normal_profile() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        let mut reference_peak: Option<f32> = None;
        for normal_db in [-12., 0., 6.] {
            let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
            model.desired.tx_mic_gain_db = normal_db;
            let mut native = FullNative::matched(&model).unwrap();
            native
                .configure(Config {
                    id: 1,
                    route: 1,
                    physical_adc: saturn_bridge::puresignal::OverloadSource::Adc1,
                    enabled: false,
                    automatic: false,
                    hardware_peak: 0.4,
                    requested_delay_seconds: 0.,
                    settling_ns: 0,
                    loop_delay_seconds: 0.,
                    stale_after_ns: 100_000_000,
                })
                .unwrap();
            assert!(native.measurement_only_guard().is_err());
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(normal_db)
            );
            assert!(!native.measurement_gain);
            native.measurement_mic_gain(true).unwrap();
            assert_eq!(native.mic_gain_readback(), 1.0);
            assert_eq!(native.model.desired.tx_mic_gain_db, normal_db);
            assert_eq!(
                native.model.desired.tx_noise_gate_enabled,
                model.desired.tx_noise_gate_enabled
            );
            // Rejected pre-key admission restores the quiescent profile.
            native.measurement_mic_gain(false).unwrap();
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(normal_db)
            );
            native.measurement_mic_gain(true).unwrap();
            native.measurement_only_guard().unwrap();
            let mut peak = 0.0_f32;
            for block in 0..120 {
                let mic: Vec<f32> = (0..512)
                    .map(|i| {
                        let n = (block * 512 + i) as f64;
                        (0.2 * ((std::f64::consts::TAU * 700. * n / 48000.).sin()
                            + (std::f64::consts::TAU * 1900. * n / 48000.).sin()))
                            as f32
                    })
                    .collect();
                let iq = native.render(&mic).unwrap();
                assert!(iq.iter().all(|s| s.is_finite()));
                if block >= 100 {
                    peak = iq.iter().fold(peak, |p, s| p.max(s.abs()));
                }
            }
            assert!(peak > 0.05, "measurement stimulus must reach native IQ");
            if let Some(reference) = reference_peak {
                assert!(
                    (peak - reference).abs() < 0.001,
                    "normal gain changed diagnostic IQ: {normal_db} {peak} {reference}"
                );
            } else {
                reference_peak = Some(peak);
            }
            native.measurement_only_guard().unwrap();
            native.begin_cleanup().unwrap();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(!native.measurement_gain);
            assert_eq!(
                native.mic_gain_readback(),
                panel_gain_for_volume_db(normal_db)
            );
            assert_eq!(native.model.desired.tx_mic_gain_db, normal_db);
        }
    }

    #[test]
    #[ignore = "serial matched native single-fit control/gain readback; no hardware or RF"]
    fn full_native_generated_trial_unity_preserves_single_fit_and_restores_profile() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
            fn saturn_shared_measurement_flags() -> i32;
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        model.desired.tx_mic_gain_db = -12.;
        let mut native = FullNative::matched(&model).unwrap();
        let mut config = timing_config(0.);
        config.enabled = true;
        config.automatic = false;
        native.configure(config).unwrap();
        for _ in 0..2 {
            native.measurement_mic_gain(true).unwrap();
            assert_eq!(native.mic_gain_readback(), 1.);
            assert!(
                native.measurement_only_guard().is_err(),
                "single-fit must not masquerade as MeasurementOnly"
            );
            native.mox(true);
            let flags = unsafe { saturn_shared_measurement_flags() };
            assert_eq!(
                flags & 0b10011,
                0b10001,
                "native manual fit + MOX, never auto"
            );
            for _ in 0..12 {
                native.render(&[0.1; 512]).unwrap();
            }
            native.begin_cleanup().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(native.mic_gain_readback(), panel_gain_for_volume_db(-12.));
            assert!(!native.measurement_gain);
            assert_eq!(unsafe { saturn_shared_measurement_flags() }, 0);
            assert!(native.config.unwrap().0.enabled);
        }
        assert_eq!(
            native.audit().feedback_blocks,
            0,
            "no synthetic correction-performance claim"
        );
    }

    #[test]
    #[ignore = "serial matched real WDSP, no devices or RF"]
    fn full_native_two_tone_and_am_silent_carrier_have_actual_iq() {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        // Fixture planning limit only; installed/native planning is unchanged.
        unsafe {
            fftw_set_timelimit(0.01);
        }
        for mode in [
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Am,
            DemodMode::Sam,
        ] {
            let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
            model.desired.mode = mode;
            let (low, high) = mode.default_tx_filter_band();
            model.desired.tx_filter_low_hz = low;
            model.desired.tx_filter_high_hz = high;
            model.desired.tx_mic_gain_db = 0.;
            let mut native = FullNative::matched(&model).unwrap();
            let mut retained = Vec::new();
            for block in 0..120 {
                let mic: Vec<f32> = (0..512)
                    .map(|i| {
                        if matches!(mode, DemodMode::Am | DemodMode::Sam) {
                            0.
                        } else {
                            let n = (block * 512 + i) as f64;
                            (0.2 * ((std::f64::consts::TAU * 700. * n / 48000.).sin()
                                + (std::f64::consts::TAU * 1900. * n / 48000.).sin()))
                                as f32
                        }
                    })
                    .collect();
                let iq = native.render(&mic).unwrap();
                assert_eq!(iq.len(), 4096);
                assert!(iq.iter().all(|s| s.is_finite()));
                if block >= 100 {
                    retained.extend(iq);
                }
            }
            let peak = retained.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
            assert!(peak > 0.05, "non-vacuous native output {mode:?}: {peak}");
            let pairs = retained.len() / 2;
            let rms =
                (retained.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / pairs as f64).sqrt();
            if matches!(mode, DemodMode::Am | DemodMode::Sam) {
                let dc = retained.chunks_exact(2).fold([0.; 2], |mut s, v| {
                    s[0] += f64::from(v[0]);
                    s[1] += f64::from(v[1]);
                    s
                });
                let dc_magnitude = dc[0].hypot(dc[1]) / pairs as f64;
                assert!(
                    (0.45..0.55).contains(&dc_magnitude),
                    "AM digital carrier, not RF watts: {dc_magnitude}"
                );
                assert!(
                    (rms - dc_magnitude).abs() < 0.005,
                    "silence must not depend on voice"
                );
            } else {
                // WDSP's native output convention precedes the existing Q,I
                // hardware packing: USB is negative here, LSB positive.
                let sign = if mode == DemodMode::Lsb { 1. } else { -1. };
                let magnitude = |frequency: f64| {
                    let s = retained
                        .chunks_exact(2)
                        .enumerate()
                        .fold([0.; 2], |mut s, (i, v)| {
                            let phase =
                                sign * std::f64::consts::TAU * frequency * i as f64 / 192000.;
                            s[0] += f64::from(v[0]) * phase.cos() + f64::from(v[1]) * phase.sin();
                            s[1] += f64::from(v[1]) * phase.cos() - f64::from(v[0]) * phase.sin();
                            s
                        });
                    s[0].hypot(s[1]) / pairs as f64
                };
                let a = magnitude(700.);
                let b = magnitude(1900.);
                eprintln!(
                    "native spectrum {mode:?}: tones {a} {b}, opposite {} {}",
                    magnitude(-700.),
                    magnitude(-1900.)
                );
                assert!(
                    a > 0.05 && b > 0.05,
                    "both intended tones must reach native IQ: {mode:?} {a} {b}"
                );
                assert!((a / b - 1.).abs() < 0.15, "unequal fundamentals: {a} {b}");
            }
            eprintln!(
                "OFFLINE native {mode:?} component_peak={peak} complex_rms={rms}; not RF power/IMD"
            );
            native.begin_cleanup().unwrap();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}
