//! Read-only, optional WDSP AmpView observer. The sole native owner only copies
//! into preallocated storage. All formatting/network publication is off-owner.
use crate::{radio_model::RadioModel, tci::TciFrontend};
use std::{
    fmt::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

static INPUT: Mutex<Weak<Input>> = Mutex::new(Weak::new());
#[cfg(test)]
static PUBLISHED: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
#[cfg(test)]
pub(crate) fn published() -> [u64; 2] {
    std::array::from_fn(|i| PUBLISHED[i].load(Ordering::Relaxed))
}
const PERIOD: Duration = Duration::from_millis(500);
pub(crate) struct Raw {
    measured: [Vec<f64>; 4],
    correction: [Vec<f64>; 4],
    ns: i32,
    nc: i32,
    reference: f64,
    info: [i32; 16],
    flags: i32,
    observed: u64,
    frequency: u32,
    watts: u8,
    attenuation: u8,
    requested: bool,
    keyed: bool,
}
impl Default for Raw {
    fn default() -> Self {
        Self {
            measured: std::array::from_fn(|_| vec![0.; 4096]),
            correction: std::array::from_fn(|_| vec![0.; 512]),
            ns: 0,
            nc: 0,
            reference: 0.,
            info: [0; 16],
            flags: 0,
            observed: 0,
            frequency: 0,
            watts: 0,
            attenuation: 0,
            requested: false,
            keyed: false,
        }
    }
}
struct Pending {
    raw: Raw,
    ready: bool,
}
pub(crate) struct Input {
    slot: Mutex<Pending>,
    drops: AtomicU64,
}
impl Input {
    fn new() -> Self {
        Self {
            slot: Mutex::new(Pending {
                raw: Raw::default(),
                ready: false,
            }),
            drops: AtomicU64::new(0),
        }
    }
    /// Called only while the existing TX owner keeps its native channel alive.
    pub(crate) fn capture(&self, channel: i32, model: &RadioModel, keyed: bool) {
        #[cfg(not(saturn_bridge_stub_native))]
        {
            let Ok(mut slot) = self.slot.try_lock() else {
                self.drops.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let r = &mut slot.raw;
            let [x, ym, yc, ys] = &mut r.measured;
            let [xm, mc, xa, ac] = &mut r.correction;
            unsafe extern "C" {
                fn saturn_try_ps_display(
                    channel: i32,
                    x: *mut f64,
                    ym: *mut f64,
                    yc: *mut f64,
                    ys: *mut f64,
                    xm: *mut f64,
                    mc: *mut f64,
                    xa: *mut f64,
                    ac: *mut f64,
                    ns: *mut i32,
                    nc: *mut i32,
                    reference: *mut f64,
                    info: *mut i32,
                    flags: *mut i32,
                ) -> i32;
            }
            let result = unsafe {
                saturn_try_ps_display(
                    channel,
                    x.as_mut_ptr(),
                    ym.as_mut_ptr(),
                    yc.as_mut_ptr(),
                    ys.as_mut_ptr(),
                    xm.as_mut_ptr(),
                    mc.as_mut_ptr(),
                    xa.as_mut_ptr(),
                    ac.as_mut_ptr(),
                    &mut r.ns,
                    &mut r.nc,
                    &mut r.reference,
                    r.info.as_mut_ptr(),
                    &mut r.flags,
                )
            };
            if result != 1 {
                self.drops.fetch_add(1, Ordering::Relaxed);
                return;
            }
            // This timestamps the readback, NOT the calibration or RF capture.
            r.observed = crate::rf_monitor::clock().1;
            r.frequency = model.desired.tx_frequency_hz;
            r.watts = model.desired.tx_drive;
            r.attenuation = model.desired.pure_signal_attenuation_db;
            r.requested = model.desired.pure_signal_enabled;
            r.keyed = keyed;
            if slot.ready {
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
            slot.ready = true;
        }
        #[cfg(saturn_bridge_stub_native)]
        let _ = (channel, model, keyed);
    }
}
pub(crate) struct Poller {
    last: Instant,
}
impl Poller {
    pub(crate) fn new() -> Self {
        Self {
            last: Instant::now(),
        }
    }
    pub(crate) fn due(&mut self) -> Option<Arc<Input>> {
        if self.last.elapsed() < PERIOD {
            return None;
        }
        self.last = Instant::now();
        INPUT.try_lock().ok()?.upgrade()
    }
}
fn text(r: &Raw, backend: &str, serial: u64, drops: u64) -> Option<String> {
    let n = usize::try_from(r.ns).ok()?;
    if n > 4096 || r.nc != 512 || !r.reference.is_finite() || r.flags & !3 != 0 {
        return None;
    }
    if r.measured
        .iter()
        .any(|a| a[..n].iter().any(|v| !v.is_finite() || v.abs() > 1e6))
        || r.correction
            .iter()
            .any(|a| a.iter().any(|v| !v.is_finite() || v.abs() > 1e6))
    {
        return None;
    }
    let (boot, published) = crate::rf_monitor::clock();
    let mut s=format!("saturn_ps_correction:{{\"format\":\"saturn-ps-correction-v1\",\"boot\":\"{boot}\",\"serial\":\"{serial}\",\"snapshotNs\":\"{}\",\"publishedNs\":\"{published}\",\"sourcePin\":\"{}\",\"backend\":\"{backend}\",\"centerHz\":{},\"commandedWatts\":{},\"attenuationCommandDb\":{},\"psRequested\":{},\"keyed\":{},\"nativeApplied\":{},\"calibrations\":{},\"nativeState\":{},\"nativeSamples\":{n},\"phaseReferenceDeg\":{},\"diagnosticDrops\":\"{drops}\",\"curveAge\":\"unknown\",\"rfImdQualified\":false,\"nativeInfo\":{:?},\"measured\":[",
        r.observed,env!("SATURN_BRIDGE_SOURCE_PIN"),r.frequency,r.watts,r.attenuation,r.requested,r.keyed,r.flags&1!=0,r.info[5],r.info[15],r.reference,r.info);
    // Fixed stride sampling for scatter only. Raw native capacities are retained
    // unchanged; the UI states exactly how many points were transmitted.
    let count = n.min(1024);
    for k in 0..count {
        if k > 0 {
            s.push(',');
        }
        let i = k * n / count;
        write!(
            s,
            "[{:.6},{:.6},{:.6},{:.6}]",
            r.measured[0][i], r.measured[1][i], r.measured[2][i], r.measured[3][i]
        )
        .ok()?;
    }
    s.push_str("],\"correction\":[");
    if n > 0 {
        for i in 0..512 {
            if i > 0 {
                s.push(',');
            }
            write!(
                s,
                "[{:.6},{:.6},{:.6},{:.6}]",
                r.correction[0][i], r.correction[1][i], r.correction[2][i], r.correction[3][i]
            )
            .ok()?;
        }
    }
    s.push_str("]};");
    (s.len() <= 160_000).then_some(s)
}
pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub(crate) fn optional(
        tci: Arc<TciFrontend>,
        backend: &'static str,
    ) -> std::io::Result<Option<Self>> {
        if std::env::var("SATURN_PS_CORRECTION_MONITOR").as_deref() != Ok("1")
            || cfg!(saturn_bridge_stub_native)
        {
            return Ok(None);
        }
        let input = Arc::new(Input::new());
        {
            let mut registered = INPUT
                .try_lock()
                .map_err(|_| std::io::Error::other("correction observer registration busy"))?;
            if registered.upgrade().is_some() {
                return Err(std::io::Error::other(
                    "correction observer already installed",
                ));
            }
            *registered = Arc::downgrade(&input);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let handle = thread::Builder::new()
            .name("ps-correction".into())
            .spawn(move || {
                let mut raw = Raw::default();
                let mut serial = 0u64;
                while !end.load(Ordering::Acquire) {
                    let ready = if let Ok(mut slot) = input.slot.try_lock() {
                        if slot.ready {
                            std::mem::swap(&mut raw, &mut slot.raw);
                            slot.ready = false;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if ready {
                        serial = serial.saturating_add(1);
                        if let Some(message) =
                            text(&raw, backend, serial, input.drops.load(Ordering::Relaxed))
                        {
                            tci.publish_ps_correction(message);
                            #[cfg(test)]
                            PUBLISHED[usize::from(backend == "DirectXdma")]
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    thread::sleep(Duration::from_millis(25));
                }
            })?;
        // Advertise only after the real-native readback worker was created.
        // The caller owns the worker until after the TX owner is joined.
        Ok(Some(Self {
            stop,
            handle: Some(handle),
        }))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correction_display_bounds_unavailable_and_nonfinite() {
        let mut r = Raw::default();
        r.nc = 512;
        r.frequency = 7200000;
        let t = text(&r, "DirectXdma", 1, 0).unwrap();
        assert!(t.contains("\"measured\":[]"));
        assert!(t.contains("\"correction\":[]"));
        r.ns = 4097;
        assert!(text(&r, "P2", 2, 0).is_none());
        r.ns = 4096;
        r.measured[1][0] = f64::NAN;
        assert!(text(&r, "P2", 2, 0).is_none());
        r.measured[1][0] = 1.;
        let t = text(&r, "P2", 3, 2).unwrap();
        assert!(t.len() < 160000);
        assert!(t.contains("\"curveAge\":\"unknown\""));
        assert!(t.contains("\"nativeSamples\":4096"));
    }
    #[cfg(not(saturn_bridge_stub_native))]
    #[test]
    #[ignore = "serial real-WDSP in-memory fixture; no radio or devices"]
    fn correction_display_native_fit_matches_getpsdisp_and_skips_contention() {
        use crate::wdsp::WdspTxEngine;
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
            fn SetPSFeedbackRate(c: i32, r: i32);
            fn SetPSHWPeak(c: i32, p: f64);
            fn SetPSTXDelay(c: i32, d: f64) -> f64;
            fn SetPSMoxDelay(c: i32, d: f64);
            fn SetPSLoopDelay(c: i32, d: f64);
            fn SetPSControl(c: i32, reset: i32, manual: i32, auto: i32, on: i32);
            fn GetPSDisp(
                c: i32,
                x: *mut f64,
                ym: *mut f64,
                yc: *mut f64,
                ys: *mut f64,
                xm: *mut f64,
                mc: *mut f64,
                xa: *mut f64,
                ac: *mut f64,
                ns: *mut i32,
                nc: *mut i32,
                reference: *mut f64,
            );
        }
        unsafe {
            fftw_set_timelimit(0.01);
        }
        let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        model.desired.pure_signal_enabled = true;
        let mut engine = WdspTxEngine::new(&model);
        engine.sync_model(&model);
        engine.set_active(true);
        unsafe {
            SetPSFeedbackRate(1, 192000);
            SetPSHWPeak(1, 0.6121);
            SetPSTXDelay(1, 0.);
            SetPSMoxDelay(1, 0.);
            SetPSLoopDelay(1, 0.);
            SetPSControl(1, 0, 1, 0, 0);
        }
        engine.set_puresignal_mox(true);
        let input = Input::new();
        input.capture(1, &model, true);
        assert_eq!(
            input.slot.lock().unwrap().raw.ns,
            0,
            "never invent an unfitted curve"
        );
        let mut tx = vec![0.; 2048];
        let mut rf = vec![0.; 2048];
        let mut random = 0x53460004u32;
        let mut uniform = || {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            f64::from(random) / 4294967296.
        };
        let mut fitted = false;
        for _ in 0..2048 {
            for i in 0..1024 {
                let r = 0.001 + 0.999 * uniform();
                let a = std::f64::consts::TAU * uniform();
                let f = 0.5 * r * (1. - 0.25 * r * r);
                let p = a + 0.18 * r * r;
                tx[2 * i] = 0.6121 * r * a.cos();
                tx[2 * i + 1] = 0.6121 * r * a.sin();
                rf[2 * i] = f * p.cos();
                rf[2 * i + 1] = f * p.sin();
            }
            engine.push_puresignal_feedback(&tx, &rf);
            engine.offline_render_block(&[0.; 512]).unwrap();
            input.capture(1, &model, true);
            {
                let slot = input.slot.lock().unwrap();
                if slot.raw.info[5] > 0 && slot.raw.flags & 1 != 0 {
                    fitted = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(fitted, "positive-control fit must complete");
        let mut expected = Raw::default();
        let [x, ym, yc, ys] = &mut expected.measured;
        let [xm, mc, xa, ac] = &mut expected.correction;
        unsafe {
            GetPSDisp(
                1,
                x.as_mut_ptr(),
                ym.as_mut_ptr(),
                yc.as_mut_ptr(),
                ys.as_mut_ptr(),
                xm.as_mut_ptr(),
                mc.as_mut_ptr(),
                xa.as_mut_ptr(),
                ac.as_mut_ptr(),
                &mut expected.ns,
                &mut expected.nc,
                &mut expected.reference,
            );
        }
        {
            let slot = input.slot.lock().unwrap();
            let actual = &slot.raw;
            assert_eq!(actual.ns, 4096);
            assert_eq!(actual.nc, 512);
            assert_eq!(actual.measured, expected.measured);
            assert_eq!(actual.correction, expected.correction);
            assert_eq!(actual.reference, expected.reference);
            let text = text(actual, "DirectXdma", 1, 0).unwrap();
            assert!(text.contains("\"nativeApplied\":true"));
            if let Some(directory) = std::env::var_os("SATURN_CORRECTION_FIXTURE_OUT") {
                // Explicit offline artifact destination; never selected by production.
                std::fs::write(
                    std::path::Path::new(&directory).join("native-correction.txt"),
                    text,
                )
                .unwrap();
            }
        }
        let guard = input.slot.lock().unwrap();
        let before = input.drops.load(Ordering::Relaxed);
        input.capture(1, &model, true);
        assert_eq!(input.drops.load(Ordering::Relaxed), before + 1);
        drop(guard);
        engine.set_puresignal_mox(false);
        unsafe {
            SetPSControl(1, 1, 0, 0, 0);
        }
        for _ in 0..16 {
            engine.push_puresignal_feedback(&[0.; 2048], &[0.; 2048]);
            engine.offline_render_block(&[0.; 512]).unwrap();
        }
        engine.set_active(false);
        eprintln!("real WDSP correction observer: 4096 measured / 512 correction, public GetPSDisp buffers equal, input contention skipped; synthetic, no RF");
    }
}
