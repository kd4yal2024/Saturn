//! Real NativePs port for the standalone offline process ONLY. No radio I/O.
//! Never imported by the production library/binary. Header/archive identity is
//! verified by run-puresignal-offline-native.sh before linking this process.
use crate::puresignal::{
    controller::{Config, NativePs},
    Fault, NATIVE_PAIRS,
};
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const CHANNEL: i32 = 1;
static TAKEN: AtomicBool = AtomicBool::new(false);
unsafe extern "C" {
    fn GetWDSPVersion() -> i32;
    fn fftw_set_timelimit(seconds: f64);
    fn OpenChannel(
        channel: i32,
        input: i32,
        dsp: i32,
        input_rate: i32,
        dsp_rate: i32,
        output_rate: i32,
        kind: i32,
        state: i32,
        delay_up: f64,
        slew_up: f64,
        delay_down: f64,
        slew_down: f64,
        bfo: i32,
    );
    fn CloseChannel(channel: i32);
    fn SetChannelState(channel: i32, state: i32, wait: i32) -> i32;
    fn SetPSMox(channel: i32, mox: i32);
    fn SetPSControl(channel: i32, reset: i32, mancal: i32, auto: i32, turnon: i32);
    fn SetPSFeedbackRate(channel: i32, rate: i32);
    fn SetPSHWPeak(channel: i32, peak: f64);
    fn SetPSTXDelay(channel: i32, seconds: f64) -> f64;
    fn SetPSMoxDelay(channel: i32, seconds: f64);
    fn SetPSLoopDelay(channel: i32, seconds: f64);
    fn pscc(channel: i32, pairs: i32, tx: *mut f64, rf: *mut f64);
    fn fexchange0(channel: i32, mic: *const f64, out: *mut f64, error: *mut i32);
    fn GetPSInfo(channel: i32, info: *mut i32);
    fn saturn_offline_iqc_flags() -> i32;
}

#[derive(Debug, Default)]
pub struct Stats {
    pub dsp_exchanges: u64,
    pub feedback_blocks: u64,
    pub resets: u64,
    pub reset_dsp_exchanges: u64,
    pub resets_during_fit: u64,
    pub final_iqc_flags: i32,
    pub closed: bool,
    pub blocking_resets: u64,
    pub cooperative_steps: u64,
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    pub cooperative_reset_pending: bool,
}
pub struct OfflineNative {
    stats: Rc<RefCell<Stats>>,
    // !Send + !Sync: all public WDSP operations belong to this one thread.
    _single_owner: PhantomData<Rc<()>>,
    automatic: bool,
    mic: Box<[f64; 1024]>,
    out: Box<[f64; 4096]>,
    zero_tx: Box<[f64; 2048]>,
    zero_rf: Box<[f64; 2048]>,
    #[cfg(feature = "offline-puresignal-tx-lifecycle")]
    cleanup: Option<(bool, u32, bool, u64)>, // reset phase, stable states, saw fit, DSP baseline
}
impl OfflineNative {
    #[cfg(feature = "offline-puresignal-owner-native")]
    pub fn feed_only(&mut self, tx: &mut [f64], rf: &mut [f64]) -> Result<(), Fault> {
        if tx.len() != NATIVE_PAIRS * 2 || rf.len() != NATIVE_PAIRS * 2 {
            return Err(Fault::UnequalLength);
        }
        unsafe {
            pscc(
                CHANNEL,
                NATIVE_PAIRS as i32,
                tx.as_mut_ptr(),
                rf.as_mut_ptr(),
            );
        }
        self.stats.borrow_mut().feedback_blocks += 1;
        Ok(())
    }
    #[cfg(feature = "offline-puresignal-owner-native")]
    pub fn latest_output(&self) -> Vec<f32> {
        self.out.iter().map(|v| *v as f32).collect()
    }
    #[cfg(feature = "offline-puresignal-owner-native")]
    pub fn render_mic(&mut self, mono: &[f32]) -> Result<Vec<f32>, Fault> {
        if mono.len() != 512 {
            return Err(Fault::InvalidConfig);
        }
        for (sample, pair) in mono.iter().zip(self.mic.chunks_exact_mut(2)) {
            pair[0] = f64::from(*sample);
            pair[1] = 0.;
        }
        self.advance_dsp()?;
        Ok(self.out.iter().map(|v| *v as f32).collect())
    }
    pub fn open(stats: Rc<RefCell<Stats>>) -> Self {
        assert!(
            !TAKEN.swap(true, Ordering::SeqCst),
            "only one offline TXA owner"
        );
        unsafe {
            assert_eq!(GetWDSPVersion(), 210);
            fftw_set_timelimit(0.01);
            OpenChannel(
                CHANNEL, 512, 2048, 48000, 96000, 192000, 1, 0, 0.01, 0.025, 0., 0.01, 1,
            );
            SetChannelState(CHANNEL, 1, 0); // DSP only; no RF device exists here
            SetPSMox(CHANNEL, 0);
        }
        Self {
            stats,
            _single_owner: PhantomData,
            automatic: false,
            mic: Box::new([0.; 1024]),
            out: Box::new([0.; 4096]),
            zero_tx: Box::new([0.; 2048]),
            zero_rf: Box::new([0.; 2048]),
            #[cfg(feature = "offline-puresignal-tx-lifecycle")]
            cleanup: None,
        }
    }
    fn advance_dsp(&mut self) -> Result<(), Fault> {
        let mut error = 0;
        unsafe {
            fexchange0(
                CHANNEL,
                self.mic.as_mut_ptr(),
                self.out.as_mut_ptr(),
                &mut error,
            );
        }
        self.stats.borrow_mut().dsp_exchanges += 1;
        if error != 0 || self.out.iter().any(|v| !v.is_finite()) {
            return Err(Fault::NativeFailure);
        }
        Ok(()) // discard native output, never write it to DUC/DMA/socket
    }
    fn idle_exchange(&mut self) -> Result<[i32; 16], Fault> {
        self.zero_tx.fill(0.);
        self.zero_rf.fill(0.);
        unsafe {
            pscc(
                CHANNEL,
                NATIVE_PAIRS as i32,
                self.zero_tx.as_mut_ptr(),
                self.zero_rf.as_mut_ptr(),
            );
        }
        self.advance_dsp()?;
        self.information()
    }
}
impl NativePs for OfflineNative {
    fn configure(&mut self, cfg: Config) -> Result<f64, Fault> {
        // Controller already retired/drained the previous epoch.
        self.automatic = cfg.automatic;
        unsafe {
            SetPSFeedbackRate(CHANNEL, 192000);
            SetPSHWPeak(CHANNEL, cfg.hardware_peak);
            SetPSMoxDelay(CHANNEL, 0.); // host settling gate supplies this delay
            SetPSLoopDelay(CHANNEL, cfg.loop_delay_seconds);
            Ok(SetPSTXDelay(CHANNEL, cfg.requested_delay_seconds))
        }
    }
    fn mox(&mut self, active: bool) {
        unsafe {
            if active {
                SetPSControl(
                    CHANNEL,
                    0,
                    i32::from(!self.automatic),
                    i32::from(self.automatic),
                    0,
                );
            }
            SetPSMox(CHANNEL, i32::from(active));
        }
    }
    fn reset(&mut self) -> Result<(), Fault> {
        self.stats.borrow_mut().blocking_resets += 1;
        // Do NOT stop TX DSP while its async fitter waits for IQC ramp ACK.
        // First drain the current fit WITHOUT a reset request. Under this sole
        // owner (no save/restore/external controls), stable WAIT/STAYON/RESET
        // follows completion of LCALC. GetPSInfo reports the pre-pscc state.
        self.mox(false);
        let deadline = Instant::now() + Duration::from_secs(3);
        let before = self.stats.borrow().dsp_exchanges;
        let mut stable = 0;
        let mut saw_fit = false;
        loop {
            let info = self.idle_exchange()?;
            saw_fit |= info[15] == 6;
            stable = if matches!(info[15], 0 | 1 | 8) {
                stable + 1
            } else {
                0
            };
            if stable >= 2 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Fault::NativeFailure);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        unsafe {
            SetPSControl(CHANNEL, 1, 0, 0, 0);
        }
        stable = 0;
        loop {
            let info = self.idle_exchange()?;
            let flags = unsafe { saturn_offline_iqc_flags() };
            stable = if info[15] == 0 && info[14] == 0 && flags == 0 {
                stable + 1
            } else {
                0
            };
            if stable >= 2 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Fault::NativeFailure);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut stats = self.stats.borrow_mut();
        stats.resets += 1;
        stats.resets_during_fit += u64::from(saw_fit);
        stats.reset_dsp_exchanges += stats.dsp_exchanges - before;
        stats.final_iqc_flags = unsafe { saturn_offline_iqc_flags() };
        Ok(())
    }
    fn process(&mut self, tx: &mut [f64; 2048], rf: &mut [f64; 2048]) -> Result<(), Fault> {
        unsafe {
            pscc(
                CHANNEL,
                NATIVE_PAIRS as i32,
                tx.as_mut_ptr(),
                rf.as_mut_ptr(),
            );
        }
        self.stats.borrow_mut().feedback_blocks += 1;
        self.advance_dsp()
    }
    fn information(&mut self) -> Result<[i32; 16], Fault> {
        let mut info = [0; 16];
        unsafe {
            GetPSInfo(CHANNEL, info.as_mut_ptr());
        }
        Ok(info)
    }
}
#[cfg(feature = "offline-puresignal-tx-lifecycle")]
impl crate::puresignal::tx_lifecycle::CooperativeNative for OfflineNative {
    fn advance_dsp_discard(&mut self) -> Result<(), Fault> {
        self.advance_dsp()
    }
    fn begin_cleanup(&mut self) -> Result<(), Fault> {
        // Called ONLY after the lifecycle's writer barrier / unkey receipt.
        // Do not call blocking NativePs::reset or stop TX DSP here.
        self.mox(false);
        self.mic.fill(0.); // subsequent fit/reset exchanges never reuse old mic input
        self.cleanup = Some((false, 0, false, self.stats.borrow().dsp_exchanges));
        self.stats.borrow_mut().cooperative_reset_pending = false;
        Ok(())
    }
    fn cleanup_step(&mut self) -> Result<crate::puresignal::tx_lifecycle::CleanupProgress, Fault> {
        use crate::puresignal::tx_lifecycle::CleanupProgress;
        let (resetting, stable, saw_fit, before) = self.cleanup.ok_or(Fault::NativeFailure)?;
        let info = self.idle_exchange()?; // exactly ONE exchange; owner regains control
        self.stats.borrow_mut().cooperative_steps += 1;
        let saw_fit = saw_fit || info[15] == 6;
        let flags = unsafe { saturn_offline_iqc_flags() };
        let stable = if (!resetting && matches!(info[15], 0 | 1 | 8))
            || (resetting && info[15] == 0 && info[14] == 0 && flags == 0)
        {
            stable + 1
        } else {
            0
        };
        if stable >= 2 && resetting {
            self.cleanup = None;
            let mut s = self.stats.borrow_mut();
            s.resets += 1;
            s.resets_during_fit += u64::from(saw_fit);
            s.reset_dsp_exchanges += s.dsp_exchanges - before;
            s.final_iqc_flags = flags;
            s.cooperative_reset_pending = false;
            return Ok(CleanupProgress::Complete);
        }
        if stable >= 2 {
            unsafe {
                SetPSControl(CHANNEL, 1, 0, 0, 0);
            }
            self.stats.borrow_mut().cooperative_reset_pending = true;
            self.cleanup = Some((true, 0, saw_fit, before));
        } else {
            self.cleanup = Some((resetting, stable, saw_fit, before));
        }
        Ok(CleanupProgress::Pending)
    }
}
impl Drop for OfflineNative {
    fn drop(&mut self) {
        if self.reset().is_err() {
            // This isolated diagnostic process must not free native objects
            // while a worker may still own them. No production abort policy.
            eprintln!("offline native quiescence failed; refusing CloseChannel");
            std::process::abort();
        }
        unsafe {
            SetChannelState(CHANNEL, 0, 0);
            CloseChannel(CHANNEL);
        }
        self.stats.borrow_mut().closed = true;
        TAKEN.store(false, Ordering::SeqCst);
    }
}
