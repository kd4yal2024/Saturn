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
    model: RadioModel,
    automatic: bool,
    config: Option<(Config, f64)>,
    flags: fn() -> i32, // pinned native IQC atomics, verified by offline runner
    history: Option<usize>,
    cleanup: Option<(bool, u32)>,
    quiescent: bool,
    audit: Audit,
    _single_owner: PhantomData<Rc<()>>, // !Send / !Sync
}
impl FullNative {
    #[cfg(all(feature = "managed-puresignal", saturn_bridge_stub_native))]
    pub(crate) fn matched(_: &RadioModel) -> Result<Self, Fault> {
        Err(Fault::NativeUnverified)
    }
    #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
    pub(crate) fn matched(model: &RadioModel) -> Result<Self, Fault> {
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
        Self::offline(
            model,
            unsafe { GetWDSPVersion() },
            Some(history as usize),
            flags,
        )
    }
    pub(crate) fn offline(
        model: &RadioModel,
        version: i32,
        history: Option<usize>,
        flags: fn() -> i32,
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
        let mut engine = match WdspTxEngine::new_shared_owner(&model) {
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
            model,
            automatic: false,
            config: None,
            flags,
            history,
            cleanup: None,
            quiescent: true,
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
    fn measurement_only_guard(&mut self) -> Result<(), Fault> {
        #[cfg(all(feature = "managed-puresignal", not(saturn_bridge_stub_native)))]
        {
            unsafe extern "C" {
                fn saturn_shared_measurement_flags() -> i32;
            }
            if !self.config.is_some_and(|(c, _)| !c.enabled && !c.automatic)
                || self.automatic
                || self.cleanup.is_some()
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
        self.audit.model_updates += 1;
        Ok(())
    }
    fn render(&mut self, mono: &[f32]) -> Result<Vec<f32>, Fault> {
        self.exchange(mono)
    }
    fn satp_history_samples(&self) -> Option<usize> {
        self.history
    }
}
impl CooperativeNative for FullNative {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
        self.exchange(&[0.; 512]).map(|_| ())
    }
    fn begin_cleanup(&mut self) -> Result<(), Fault> {
        self.mox(false);
        self.engine.as_mut().unwrap().clear_tx_buffers();
        self.cleanup = Some((false, 0));
        Ok(())
    }
    fn cleanup_step(&mut self) -> Result<CleanupProgress, Fault> {
        let (resetting, stable) = self.cleanup.ok_or(Fault::NativeFailure)?;
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
            // Receipt and fitter/IQC quiescence precede channel recreation.
            // This also removes ALL old TX filter/ALC/EQ/CFC/DEXP sample state.
            let engine = self.engine.as_mut().unwrap();
            engine.recreate_channel(&self.model);
            engine.sync_model(&self.model);
            engine.set_active(true);
            // An ordinary release/re-arm has no pending configuration. Restore
            // the shared controller's retained contract before declaring clean.
            if let Some((config, actual_delay)) = self.config {
                if Self::apply_config(config)? != actual_delay {
                    return Err(Fault::NativeFailure);
                }
            }
            self.audit.recreations += 1;
            self.cleanup = None;
            self.quiescent = true;
            return Ok(CleanupProgress::Complete);
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
