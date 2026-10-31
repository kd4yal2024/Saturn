//! Noncritical observer. Acquisition copies into preallocated storage with
//! try_lock; only this worker performs FFT/formatting. No TX authority/native DSP.
use crate::{sync_ext::MutexExt, tci::TciFrontend};
use saturn_bridge::{
    puresignal::{Backend, Fence, PairBlock},
    rf_spectrum::{Analysis, Analyzer, FFT_SIZE, RATE},
};
use std::{
    fmt::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
mod archive;
pub(crate) mod legacy;

static CLOCK: OnceLock<(Instant, u128)> = OnceLock::new();
pub(crate) fn clock() -> (u128, u64) {
    let (origin, boot) = CLOCK.get_or_init(|| {
        (
            Instant::now(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
    });
    (
        *boot,
        origin.elapsed().as_nanos().min(u64::MAX as u128) as u64,
    )
}
fn observed(origin: Instant, ns: u64) -> Option<u64> {
    let clock_origin = CLOCK.get()?.0;
    origin
        .checked_add(Duration::from_nanos(ns))?
        .checked_duration_since(clock_origin)
        .and_then(|d| u64::try_from(d.as_nanos()).ok())
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) center_hz: u32,
    pub(crate) tones: [f64; 2],
    pub(crate) attenuation_db: u8,
    pub(crate) commanded_watts: u8,
    pub(crate) ps_requested: bool,
    pub(crate) mode: crate::radio_model::DemodMode,
    pub(crate) stimulus: crate::tx_thread::feedback_owner::Stimulus,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            center_hz: 0,
            tones: [700., 1900.],
            attenuation_db: 31,
            commanded_watts: 0,
            ps_requested: false,
            mode: crate::radio_model::DemodMode::Usb,
            stimulus: Default::default(),
        }
    }
}
impl Settings {
    pub(crate) fn model(model: &crate::radio_model::RadioModel) -> Self {
        let d = &model.desired;
        Self {
            center_hz: d.tx_frequency_hz,
            tones: [
                f64::from(d.tx_two_tone_freq1_hz),
                f64::from(d.tx_two_tone_freq2_hz),
            ],
            attenuation_db: d.pure_signal_attenuation_db,
            commanded_watts: d.tx_drive,
            ps_requested: d.pure_signal_enabled,
            mode: d.mode,
            stimulus: Default::default(),
        }
    }
}
struct Window {
    rf: Vec<f32>,
    reference: Vec<f32>,
    arrival_ns: Vec<u64>,
    pairs: usize,
    fence: Fence,
    next: u64,
    first_ns: u64,
    last_ns: u64,
    backend: Backend,
    overload_clear: bool,
    settings: Settings,
}
impl Default for Window {
    fn default() -> Self {
        Self {
            rf: vec![0.; FFT_SIZE * 2],
            reference: vec![0.; FFT_SIZE * 2],
            arrival_ns: vec![0; FFT_SIZE],
            pairs: 0,
            fence: Fence::default(),
            next: 0,
            first_ns: 0,
            last_ns: 0,
            backend: Backend::DirectXdma,
            overload_clear: false,
            settings: Settings::default(),
        }
    }
}
struct Pending {
    active: Window,
    ready: Window,
    available: bool,
    invalid: bool,
    settings: Settings,
}
#[derive(Clone)]
pub(crate) struct Input {
    pending: Arc<Mutex<Pending>>,
    drops: Arc<AtomicU64>,
    faulted: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    suspended: Arc<AtomicBool>,
}
impl Input {
    pub(crate) fn new() -> Self {
        clock();
        Self {
            pending: Arc::new(Mutex::new(Pending {
                active: Window::default(),
                ready: Window::default(),
                available: false,
                invalid: true,
                settings: Settings::default(),
            })),
            drops: Arc::new(AtomicU64::new(0)),
            faulted: Arc::new(AtomicBool::new(false)),
            revision: Arc::new(AtomicU64::new(0)),
            suspended: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) fn configure(&self, settings: Settings) {
        let mut p = self.pending.lock_unpoisoned();
        if p.settings != settings {
            self.revision.fetch_add(1, Ordering::AcqRel);
            p.active.pairs = 0;
            p.available = false;
            p.settings = settings;
        }
    }
    pub(crate) fn prepare(&self, settings: Settings) {
        self.configure(settings);
        self.activate();
    }
    pub(crate) fn activate(&self) {
        let mut p = self.pending.lock_unpoisoned();
        p.active.pairs = 0;
        p.available = false;
        p.invalid = false;
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.faulted.store(false, Ordering::Release);
        self.suspended.store(false, Ordering::Release);
    }
    pub(crate) fn invalidate(&self) {
        self.suspended.store(true, Ordering::Release);
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.faulted.store(true, Ordering::Release);
        if let Ok(mut p) = self.pending.try_lock() {
            p.invalid = true;
            p.active.pairs = 0;
            p.available = false;
        }
    }
    pub(crate) fn faulted(&self) -> bool {
        self.faulted.load(Ordering::Acquire) || self.suspended.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub(crate) fn push(&self, block: &PairBlock, origin: Instant) {
        self.push_inner(block, origin, None);
    }
    pub(crate) fn push_scoped(
        &self,
        block: &PairBlock,
        origin: Instant,
        scope: crate::tx_thread::feedback_owner::Scope,
    ) {
        self.push_inner(
            block,
            origin,
            Some((scope.frequency_hz, scope.attenuation_db, scope.stimulus)),
        );
    }
    fn push_inner(
        &self,
        block: &PairBlock,
        origin: Instant,
        route: Option<(u32, u8, crate::tx_thread::feedback_owner::Stimulus)>,
    ) {
        if self.suspended.load(Ordering::Acquire) {
            return;
        }
        if block.pairs == 0
            || block.pairs > 1024
            || block.meta.rate_hz != RATE
            || !block.arrival_times_recorded
            || block
                .meta
                .interval
                .end
                .checked_sub(block.meta.interval.first)
                != Some(block.pairs as u64)
        {
            self.invalidate();
            return;
        }
        let Some(first) = observed(origin, block.meta.observed_ns) else {
            self.invalidate();
            return;
        };
        let Some(last) = block.arrival_ns[..block.pairs]
            .iter()
            .max()
            .and_then(|n| observed(origin, *n))
        else {
            self.invalidate();
            return;
        };
        let Ok(mut p) = self.pending.try_lock() else {
            self.drops.fetch_add(1, Ordering::Relaxed);
            self.faulted.store(true, Ordering::Release);
            return;
        };
        if self.faulted.swap(false, Ordering::AcqRel) || p.invalid {
            p.active.pairs = 0;
            p.available = false;
            p.invalid = false;
        }
        if p.active.pairs != 0
            && (p.active.fence != block.meta.fence
                || p.active.next != block.meta.interval.first
                || p.active.backend != block.meta.backend)
        {
            p.active.pairs = 0;
            p.available = false;
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
        let settings = if let Some((center_hz, attenuation_db, stimulus)) = route {
            Settings {
                center_hz,
                attenuation_db,
                stimulus,
                ..p.settings
            }
        } else {
            p.settings
        };
        if p.active.pairs != 0 && p.active.settings != settings {
            p.active.pairs = 0;
            p.available = false;
        }
        // A window may span multiple acquisition blocks; it may NOT span a gap,
        // a generation/configuration change or a drop. Excess from a block is
        // retained in the next window with its ORIGINAL observation time.
        let mut consumed = 0;
        while consumed < block.pairs {
            if p.active.pairs == 0 {
                p.active.fence = block.meta.fence;
                p.active.first_ns = first;
                p.active.backend = block.meta.backend;
                p.active.overload_clear = true;
                p.active.settings = settings;
            }
            let n = (FFT_SIZE - p.active.pairs).min(block.pairs - consumed);
            let start = p.active.pairs * 2;
            let source = consumed * 2;
            for i in 0..n * 2 {
                p.active.rf[start + i] = block.rf[source + i] as f32;
                p.active.reference[start + i] = block.tx[source + i] as f32;
            }
            let dest_pair = p.active.pairs;
            for i in 0..n {
                let Some(at) = observed(origin, block.arrival_ns[consumed + i]) else {
                    self.invalidate();
                    return;
                };
                if at < first {
                    self.invalidate();
                    return;
                }
                p.active.arrival_ns[dest_pair + i] = at;
            }
            p.active.pairs += n;
            consumed += n;
            p.active.next = block.meta.interval.first + consumed as u64;
            p.active.last_ns = last;
            p.active.overload_clear &= block.meta.overload.iter().enumerate().all(|(i, b)| {
                b.is_some_and(|b| {
                    b.source
                        == [
                            saturn_bridge::puresignal::OverloadSource::Adc1,
                            saturn_bridge::puresignal::OverloadSource::Ddc0Clamp,
                            saturn_bridge::puresignal::OverloadSource::Ddc1Clamp,
                        ][i]
                        && b.observed_ns >= *block.arrival_ns[..block.pairs].iter().max().unwrap()
                        && b.certify(
                            block.meta.interval,
                            block
                                .meta
                                .observed_ns
                                .max(*block.arrival_ns[..block.pairs].iter().max().unwrap()),
                        )
                        .is_ok()
                })
            });
            if p.active.pairs == FFT_SIZE {
                if p.available {
                    self.drops.fetch_add(1, Ordering::Relaxed);
                }
                let Pending { active, ready, .. } = &mut *p;
                std::mem::swap(active, ready);
                p.available = true;
                p.active.pairs = 0;
            }
        }
    }
}

pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub(crate) fn spawn(tci: Arc<TciFrontend>, input: Input) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let handle = thread::Builder::new()
            .name("ps-rf-monitor".into())
            .spawn(move || {
                let mut analyzer = Analyzer::default();
                let mut frame = Window::default();
                let mut serial = 0u64;
                let mut archive = archive::Archive::from_env();
                while !end.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(100)); // <=10 Hz, no FFT on TX/acquisition thread
                    let revision;
                    {
                        let mut p = input.pending.lock_unpoisoned();
                        if !p.available || p.invalid || input.faulted() {
                            continue;
                        }
                        std::mem::swap(&mut p.ready, &mut frame);
                        p.available = false;
                        revision = input.revision.load(Ordering::Acquire);
                    }
                    let (_, now) = clock();
                    if now.saturating_sub(frame.first_ns) > 500_000_000 {
                        continue;
                    }
                    let Ok(mut rf) = analyzer.analyze(&frame.rf, frame.settings.tones) else {
                        continue;
                    };
                    let Ok(reference) = analyzer.analyze(&frame.reference, frame.settings.tones)
                    else {
                        continue;
                    };
                    qualify_pair(&mut rf, &reference);
                    qualify_stimulus(&mut rf, frame.settings.stimulus);
                    serial += 1;
                    let text = serialize(
                        &frame,
                        &rf,
                        &reference,
                        serial,
                        input.drops.load(Ordering::Relaxed),
                    );
                    if let Some(archive) = &mut archive {
                        let _ = archive.record(&frame, serial, &text);
                    }
                    if !input.faulted() && input.revision.load(Ordering::Acquire) == revision {
                        tci.publish_rf_spectrum(text);
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
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
fn qualify_pair(rf: &mut Analysis, reference: &Analysis) {
    if !reference.reason.starts_with("relative estimate") || rf.tone_hz != reference.tone_hz {
        rf.reason = "reference fundamentals unavailable or sideband mismatch";
        for p in &mut rf.products {
            p.dbc = None;
            p.upper_bound_dbc = None;
            p.reason = rf.reason;
        }
    }
}
fn qualify_stimulus(rf: &mut Analysis, stimulus: crate::tx_thread::feedback_owner::Stimulus) {
    use crate::tx_thread::feedback_owner::Stimulus;
    let reason = match stimulus {
        Stimulus::Timing => "timing multisine; two-tone IMD unavailable",
        Stimulus::AmCarrier => "AM silent carrier; two-tone IMD unavailable",
        _ => return,
    };
    rf.reason = reason;
    for p in &mut rf.products {
        p.dbc = None;
        p.upper_bound_dbc = None;
        p.reason = reason;
    }
}
fn serialize(w: &Window, a: &Analysis, reference: &Analysis, serial: u64, drops: u64) -> String {
    let (boot, now) = clock();
    let s = w.settings;
    let mut out=format!("saturn_ps_spectrum:{{\"format\":\"saturn-rf-spectrum-v1\",\"boot\":\"{boot}\",\"serial\":\"{serial}\",\"generation\":\"{}\",\"config\":\"{}\",\"txEpoch\":\"{}\",\"backend\":\"{:?}\",\"source\":\"ReturnedRfIq\",\"observedFirstNs\":\"{}\",\"observedLastNs\":\"{}\",\"publishedNs\":\"{now}\",\"rateHz\":{RATE},\"fftSize\":{FFT_SIZE},\"binHz\":{:.6},\"rbwHz\":{:.6},\"window\":\"Blackman-Harris 4-term\",\"centerHz\":{},\"attenuationCommandDb\":{},\"commandedWatts\":{},\"psRequested\":{},\"overload\":\"{}\",\"continuity\":\"contiguous\",\"diagnosticDrops\":\"{drops}\",\"reference\":\"weaker fundamental; not PEP\",\"rfImdQualified\":false,\"reason\":\"{}\",\"componentPeak\":{:.9},\"clippedComponents\":{},\"toneBalanceDb\":{:.4},\"toneHz\":[{:.3},{:.3}],\"toneDbfs\":[{:.4},{:.4}],\"referenceToneDbfs\":[{:.4},{:.4}],\"products\":[",
        w.fence.generation,w.fence.config,w.fence.tx_epoch,w.backend,w.first_ns,w.last_ns,
        a.bin_hz,a.rbw_hz,s.center_hz,s.attenuation_db,s.commanded_watts,s.ps_requested,
        if w.overload_clear { "clear" } else { "unknown" },a.reason,a.component_peak,a.clipped_components,a.tone_balance_db,
        a.tone_hz[0],a.tone_hz[1],a.tone_dbfs[0],a.tone_dbfs[1],reference.tone_dbfs[0],reference.tone_dbfs[1]);
    for (i, p) in a.products.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        let number = |v: Option<f64>| v.map_or("null".into(), |v| format!("{v:.4}"));
        write!(
            out,
            "{{\"order\":{},\"offsetHz\":{:.3},\"dbc\":{},\"upperBoundDbc\":{},\"reason\":\"{}\"}}",
            p.order,
            p.offset_hz,
            number(p.dbc),
            number(p.upper_bound_dbc),
            p.reason
        )
        .unwrap();
    }
    // Original 16k analysis is never decimated. Publish only visible +/-8 kHz
    // bins for the noncritical viewer, with the exact bin origin/spacing.
    let low = (FFT_SIZE as f64 / 2. - 8000. / a.bin_hz).ceil() as usize;
    let high = (FFT_SIZE as f64 / 2. + 8000. / a.bin_hz).floor() as usize;
    write!(
        out,
        "],\"stimulus\":\"{}\",\"requestedStimulus\":{},\"firstBinHz\":{:.6},\"binsDbfs\":[",
        s.stimulus.label(),
        crate::tx_thread::ps_measurement::stimulus_json(s.stimulus),
        (low as f64 - FFT_SIZE as f64 / 2.) * a.bin_hz
    )
    .unwrap();
    for (i, value) in a.bins_dbfs[low..=high].iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        write!(out, "{value:.2}").unwrap();
    }
    write!(out,"],\"sourcePin\":\"{}\",\"sourceBoot\":\"{}\",\"firmwareBuild\":\"not supplied by this observer\"}}",env!("SATURN_BRIDGE_SOURCE_PIN"),w.fence.boot).unwrap();
    // TCI uses ';' as a record delimiter. Escape it INSIDE JSON strings before
    // adding the single terminal delimiter (including the reference label).
    let mut out = out.replace(';', "\\u003b");
    out.push(';');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use saturn_bridge::puresignal::{Interval, MAX_PAIRS};
    fn block(first: u64, generation: u64, origin: Instant) -> PairBlock {
        let mut block = PairBlock::default();
        block.pairs = MAX_PAIRS;
        block.meta.fence = Fence {
            boot: 1,
            generation,
            config: 1,
            tx_epoch: generation,
        };
        block.meta.backend = Backend::DirectXdma;
        block.meta.interval = Interval {
            fence: block.meta.fence,
            first,
            end: first + MAX_PAIRS as u64,
        };
        block.meta.observed_ns = origin.elapsed().as_nanos() as u64;
        block.arrival_times_recorded = true;
        block.arrival_ns.fill(block.meta.observed_ns);
        for i in 0..MAX_PAIRS {
            for (hz, amp) in [(700., 0.15), (1900., 0.1), (-500., 0.001)] {
                let phase = std::f64::consts::TAU * hz * (first as f64 + i as f64) / RATE as f64;
                block.rf[i * 2] += amp * phase.cos();
                block.rf[i * 2 + 1] += amp * phase.sin();
                if hz > 0. {
                    block.tx[i * 2] += amp * phase.cos();
                    block.tx[i * 2 + 1] += amp * phase.sin();
                }
            }
        }
        block
    }
    fn fill(input: &Input, origin: Instant, generation: u64) {
        for i in 0..16 {
            input.push(&block(i * 1024, generation, origin), origin);
        }
    }
    #[test]
    fn preallocated_observer_retains_raw_pair_identity_oldest_time_and_unknown_overload() {
        let input = Input::new();
        let origin = Instant::now();
        input.prepare(Settings {
            center_hz: 7_200_000,
            commanded_watts: 5,
            ..Settings::default()
        });
        fill(&input, origin, 1);
        let p = input.pending.lock().unwrap();
        assert!(p.available);
        assert_eq!(p.ready.pairs, FFT_SIZE);
        assert!(!p.ready.overload_clear);
        assert!(p.ready.first_ns <= p.ready.last_ns);
        assert_ne!(p.ready.rf[0], p.ready.reference[0]);
        let mut analyzer = Analyzer::default();
        let rf = analyzer.analyze(&p.ready.rf, [700., 1900.]).unwrap();
        let reference = analyzer.analyze(&p.ready.reference, [700., 1900.]).unwrap();
        assert!((rf.products[0].dbc.unwrap() + 40.).abs() < 0.3);
        let wire = serialize(&p.ready, &rf, &reference, 1, 0);
        assert!(wire.len() < 24_000);
        assert!(wire.contains("\"overload\":\"unknown\""));
        if let Some(directory) = std::env::var_os("SATURN_RF_MONITOR_FIXTURE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            assert!(directory.is_absolute() && !directory.starts_with("/dev"));
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("synthetic-rf-spectrum.txt"), wire).unwrap();
            std::fs::write(
                directory.join("SYNTHETIC-NOT-RF.txt"),
                "Generated test pairs; no hardware, RF or measured IMD evidence.\n",
            )
            .unwrap();
        }
    }
    #[test]
    fn gaps_generation_changes_configuration_and_slow_analyzer_never_stitch_windows() {
        let input = Input::new();
        let origin = Instant::now();
        input.push(&block(0, 1, origin), origin);
        input.push(&block(2048, 1, origin), origin);
        assert_eq!(input.pending.lock().unwrap().active.pairs, 1024);
        input.push(&block(3072, 2, origin), origin);
        assert_eq!(input.pending.lock().unwrap().active.pairs, 1024);
        input.configure(Settings {
            center_hz: 14_167_000,
            ..Settings::default()
        });
        assert_eq!(input.pending.lock().unwrap().active.pairs, 0);
        fill(&input, origin, 3);
        assert!(input.pending.lock().unwrap().available);
        fill(&input, origin, 4);
        assert_eq!(input.pending.lock().unwrap().ready.fence.generation, 4);
        assert!(input.drops.load(Ordering::Relaxed) >= 2);
        input.invalidate();
        fill(&input, origin, 5);
        assert!(!input.pending.lock().unwrap().available);
        input.prepare(Settings::default());
        fill(&input, origin, 6);
        assert!(input.pending.lock().unwrap().available);
    }
    #[test]
    fn producer_never_waits_for_slow_analyzer_and_dropped_block_retires_partial_capture() {
        let input = Input::new();
        let origin = Instant::now();
        input.push(&block(0, 1, origin), origin);
        let lock = input.pending.lock().unwrap();
        input.push(&block(1024, 1, origin), origin);
        drop(lock);
        assert_eq!(input.drops.load(Ordering::Relaxed), 1);
        input.push(&block(2048, 1, origin), origin);
        assert_eq!(input.pending.lock().unwrap().active.pairs, 1024);
        assert!(!input.pending.lock().unwrap().available);
    }
    #[test]
    fn p2_existing_pair_decoder_taps_the_same_observer_without_changing_owner_delivery() {
        use crate::tx_thread::{
            feedback_owner::tests::{collector, scope},
            feedback_stream::{tests::packet, Stream},
        };
        let (ingress, rx) = collector();
        let input = Input::new();
        let origin = Instant::now();
        input.configure(Settings {
            center_hz: 7_200_000,
            ..Settings::default()
        });
        ingress.rf_monitor.set(input.clone()).ok().unwrap();
        let token = scope(1);
        let mut stream = Stream::new(token, Backend::P2, origin, ingress.clone()).unwrap();
        for i in 0..138 {
            stream.p2(Some(token), &packet(i), Instant::now()).unwrap();
            if (i + 1) % 8 == 0 {
                let block = rx.feedback.try_recv().unwrap();
                assert_eq!(block.pairs, 952);
                ingress.recycle_feedback_buffer(block);
            }
        }
        let p = input.pending.lock().unwrap();
        assert!(p.available);
        assert_eq!(p.ready.backend, Backend::P2);
        assert_eq!(p.ready.rf[0], 0.125);
        assert_eq!(p.ready.reference[0], 0.25);
        drop(p);
        drop(stream);
        assert!(input.suspended.load(Ordering::Acquire));
    }
    #[test]
    fn missing_or_opposite_reference_never_certifies_rf_products() {
        let mut a = Analyzer::default();
        let input = Input::new();
        let origin = Instant::now();
        fill(&input, origin, 1);
        let p = input.pending.lock().unwrap();
        let mut rf = a.analyze(&p.ready.rf, [700., 1900.]).unwrap();
        let reference = a.analyze(&vec![0.; FFT_SIZE * 2], [700., 1900.]).unwrap();
        qualify_pair(&mut rf, &reference);
        assert!(rf
            .products
            .iter()
            .all(|p| p.dbc.is_none() && p.upper_bound_dbc.is_none()));
        assert!(rf.reason.contains("reference fundamentals unavailable"));
    }
    #[test]
    fn capture_route_metadata_comes_from_programmed_acquisition_scope_not_requested_model() {
        let input = Input::new();
        let origin = Instant::now();
        input.configure(Settings {
            center_hz: 14_200_000,
            attenuation_db: 2,
            ..Settings::default()
        });
        let mut scope = crate::tx_thread::feedback_owner::tests::scope(1);
        scope.stimulus = crate::tx_thread::feedback_owner::Stimulus::Timing;
        input.push_scoped(&block(0, 1, origin), origin, scope);
        let p = input.pending.lock().unwrap();
        assert_eq!(p.active.settings.center_hz, scope.frequency_hz);
        assert_eq!(p.active.settings.attenuation_db, scope.attenuation_db);
        assert_eq!(p.active.settings.stimulus, scope.stimulus);
    }
    #[test]
    fn timing_scope_survives_periodic_model_updates_and_never_exports_two_tone_imd() {
        use crate::tx_thread::feedback_owner::{tests::scope, Stimulus};
        let input = Input::new();
        let origin = Instant::now();
        let mut scope = scope(1);
        scope.stimulus = Stimulus::Timing;
        for i in 0..16 {
            input.configure(Settings::default()); // runtime's independent model snapshot
            input.push_scoped(&block(i * 1024, 1, origin), origin, scope);
        }
        let p = input.pending.lock().unwrap();
        assert!(p.available);
        assert_eq!(p.ready.settings.stimulus, Stimulus::Timing);
        let mut analyzer = Analyzer::default();
        let mut rf = analyzer.analyze(&p.ready.rf, [700., 1900.]).unwrap();
        let reference = analyzer.analyze(&p.ready.reference, [700., 1900.]).unwrap();
        assert!(rf.products[0].dbc.is_some()); // positive control: actual tones in this synthetic test
        qualify_stimulus(&mut rf, p.ready.settings.stimulus);
        assert!(rf
            .products
            .iter()
            .all(|p| p.dbc.is_none() && p.upper_bound_dbc.is_none()));
        let wire = serialize(&p.ready, &rf, &reference, 1, 0);
        assert!(wire.contains("\"stimulus\":\"saturn-timing-multisine-v1\""));
        assert!(wire.contains("\"repeatMs\":1000"));
        assert!(wire.contains("\"dbc\":null"));
        drop(p);
        let normal = crate::tx_thread::feedback_owner::tests::scope(2);
        input.push_scoped(&block(0, 2, origin), origin, normal);
        assert_eq!(
            input.pending.lock().unwrap().active.settings.stimulus,
            Stimulus::Unspecified
        );
    }
}
