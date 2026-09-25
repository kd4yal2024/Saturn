//! Server-side RX spectrum rows for WAN display clients.
//!
//! A WAN browser needs only a few thousand fresh IQ pairs per display row, but
//! the full-rate Direct-XDMA IQ stream is ~24.6 Mbit/s. Clients that opt in via
//! `saturn_display:spectrum,<fft>,<interval>;` receive quantized dB rows
//! computed here instead. LAN clients never opt in and keep raw IQ unchanged.
//!
//! The RX runtime loop's only job is a lock-free ring write. The worker
//! thread snapshots the newest pairs at each client-group deadline, skipping
//! the deadline rather than blocking when the ring is lapped or retuned.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::sync_ext::MutexExt;
use crate::tci::TciFrontend;
use crate::xdma_telemetry::LatencyHistogram;

pub(crate) const SPECTRUM_MIN_FFT_SIZE: u32 = 256;
pub(crate) const SPECTRUM_MAX_FFT_SIZE: u32 = 4096;
pub(crate) const SPECTRUM_MIN_INTERVAL_MS: u32 = 33;
pub(crate) const SPECTRUM_MAX_INTERVAL_MS: u32 = 250;
pub(crate) const SPECTRUM_DB_OFFSET: f32 = -160.0;
pub(crate) const SPECTRUM_DB_STEP: f32 = 0.625;
pub(crate) const SPECTRUM_FORMAT_U8_DB: u32 = 0x5301;
pub(crate) const TCI_STREAM_SPECTRUM_ROW: u32 = 16;

/// Ring capacity in complex pairs (~170 ms at 384 kHz). The margin above
/// `SPECTRUM_MAX_FFT_SIZE` covers the largest writer chunk that may be in
/// flight before the head store publishes it.
const RING_CAPACITY_PAIRS: u64 = 65_536;
const RING_WRITER_MARGIN_PAIRS: u64 = 16_384;
const WORKER_IDLE_POLL: Duration = Duration::from_millis(20);

/// Round down to a power of two and clamp to the WAN display contract.
pub(crate) fn clamp_spectrum_fft_size(requested: u32) -> u32 {
    let clamped = requested.clamp(SPECTRUM_MIN_FFT_SIZE, SPECTRUM_MAX_FFT_SIZE);
    1 << (31 - clamped.leading_zeros())
}

pub(crate) fn clamp_spectrum_interval_ms(requested: u32) -> u32 {
    requested.clamp(SPECTRUM_MIN_INTERVAL_MS, SPECTRUM_MAX_INTERVAL_MS)
}

/// One computed display row, shared by every client in the same
/// (fft_size, interval) group. The per-client sequence is assigned when the
/// writer dequeues the row, so replaced rows never consume credit.
#[derive(Debug)]
pub(crate) struct SpectrumRow {
    pub(crate) span_hz: u32,
    pub(crate) fft_size: u32,
    pub(crate) interval: Duration,
    pub(crate) center_hz: u64,
    pub(crate) captured_at: Instant,
    pub(crate) capture_to_enqueue_us: u32,
    pub(crate) server_ms: u32,
    pub(crate) codes: Box<[u8]>,
}

impl SpectrumRow {
    pub(crate) fn is_stale(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.captured_at) > self.interval * 2
    }
}

/// Radix-2 FFT that reproduces `remote-web/src/dsp/fft.ts` exactly: the same
/// Hann window, bit-reversal, butterfly order, fftshift and dB formula. JS
/// computes in f64 and stores into Float32Arrays, so this does the same.
pub(crate) struct SpectrumFft {
    size: usize,
    window: Box<[f32]>,
    bit_reverse: Box<[u32]>,
    cos_table: Box<[f32]>,
    sin_table: Box<[f32]>,
    scratch_re: Box<[f32]>,
    scratch_im: Box<[f32]>,
}

impl SpectrumFft {
    pub(crate) fn new(size: usize) -> Self {
        assert!(size.is_power_of_two() && size >= 2);
        let mut window = vec![0.0f32; size];
        let mut bit_reverse = vec![0u32; size];
        for (i, (w, reversed)) in window.iter_mut().zip(bit_reverse.iter_mut()).enumerate() {
            *w = (0.5 - 0.5 * ((2.0 * std::f64::consts::PI * i as f64) / (size - 1) as f64).cos())
                as f32;
            let mut j = 0usize;
            let mut bit = 0;
            while (1usize << bit) < size {
                j = (j << 1) | ((i >> bit) & 1);
                bit += 1;
            }
            *reversed = j as u32;
        }
        let half = size / 2;
        let mut cos_table = vec![0.0f32; half];
        let mut sin_table = vec![0.0f32; half];
        for i in 0..half {
            let angle = (-2.0 * std::f64::consts::PI * i as f64) / size as f64;
            cos_table[i] = angle.cos() as f32;
            sin_table[i] = angle.sin() as f32;
        }
        Self {
            size,
            window: window.into_boxed_slice(),
            bit_reverse: bit_reverse.into_boxed_slice(),
            cos_table: cos_table.into_boxed_slice(),
            sin_table: sin_table.into_boxed_slice(),
            scratch_re: vec![0.0; size].into_boxed_slice(),
            scratch_im: vec![0.0; size].into_boxed_slice(),
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// `iq` must hold exactly `size` contiguous interleaved pairs (stride 1).
    /// Writes fftshifted dB bins into `out`.
    pub(crate) fn transform_db(&mut self, iq: &[f32], out: &mut [f32]) {
        let size = self.size;
        assert_eq!(iq.len(), size * 2);
        assert_eq!(out.len(), size);
        for i in 0..size {
            let w = f64::from(self.window[i]);
            let target = self.bit_reverse[i] as usize;
            self.scratch_re[target] = (f64::from(iq[i * 2]) * w) as f32;
            self.scratch_im[target] = (f64::from(iq[i * 2 + 1]) * w) as f32;
        }

        let mut span = 2;
        while span <= size {
            let half = span >> 1;
            let table_step = size / span;
            let mut start = 0;
            while start < size {
                for i in 0..half {
                    let twiddle = i * table_step;
                    let even = start + i;
                    let odd = even + half;
                    let odd_re = f64::from(self.scratch_re[odd]);
                    let odd_im = f64::from(self.scratch_im[odd]);
                    let tw_re = f64::from(self.cos_table[twiddle]);
                    let tw_im = f64::from(self.sin_table[twiddle]);
                    let rot_re = odd_re * tw_re - odd_im * tw_im;
                    let rot_im = odd_re * tw_im + odd_im * tw_re;
                    let even_re = f64::from(self.scratch_re[even]);
                    let even_im = f64::from(self.scratch_im[even]);
                    self.scratch_re[odd] = (even_re - rot_re) as f32;
                    self.scratch_im[odd] = (even_im - rot_im) as f32;
                    self.scratch_re[even] = (even_re + rot_re) as f32;
                    self.scratch_im[even] = (even_im + rot_im) as f32;
                }
                start += span;
            }
            span <<= 1;
        }

        let half = size / 2;
        for (i, bin) in out.iter_mut().enumerate() {
            let shifted = (i + half) % size;
            let re = f64::from(self.scratch_re[shifted]);
            let im = f64::from(self.scratch_im[shifted]);
            let magnitude = re.hypot(im) / size as f64;
            *bin = (20.0 * (magnitude + 1e-8).log10()) as f32;
        }
    }
}

pub(crate) fn quantize_db(db: f32) -> u8 {
    let code = ((db - SPECTRUM_DB_OFFSET) / SPECTRUM_DB_STEP).round();
    if code.is_nan() {
        0
    } else {
        code.clamp(0.0, 255.0) as u8
    }
}

/// Single-producer ring of the newest IQ pairs. The RX loop is the only
/// writer and never blocks; the worker validates its copy against the head
/// afterwards and discards it if the writer lapped the window.
pub(crate) struct LatestIqRing {
    samples: Box<[AtomicU32]>,
    head: AtomicU64,
    valid_from: AtomicU64,
    center_hz: AtomicU64,
}

impl LatestIqRing {
    pub(crate) fn new(center_hz: u64) -> Self {
        Self {
            samples: (0..RING_CAPACITY_PAIRS * 2)
                .map(|_| AtomicU32::new(0))
                .collect(),
            head: AtomicU64::new(0),
            valid_from: AtomicU64::new(0),
            center_hz: AtomicU64::new(center_hz),
        }
    }

    /// Writer only. Appends interleaved IQ pairs.
    pub(crate) fn push(&self, iq: &[f32]) {
        let pairs = iq.chunks_exact(2);
        let total = pairs.len() as u64;
        let skip = total.saturating_sub(RING_CAPACITY_PAIRS - RING_WRITER_MARGIN_PAIRS);
        let head = self.head.load(Ordering::Relaxed);
        for (index, pair) in (head + skip..).zip(pairs.skip(skip as usize)) {
            let slot = ((index % RING_CAPACITY_PAIRS) * 2) as usize;
            self.samples[slot].store(pair[0].to_bits(), Ordering::Relaxed);
            self.samples[slot + 1].store(pair[1].to_bits(), Ordering::Relaxed);
        }
        self.head.store(head + total, Ordering::Release);
    }

    /// Writer only. Samples before this point belong to another RF center or
    /// an interrupted stream and must never reach a display row.
    pub(crate) fn restart(&self, center_hz: u64) {
        self.center_hz.store(center_hz, Ordering::Release);
        self.valid_from
            .store(self.head.load(Ordering::Relaxed), Ordering::Release);
    }

    /// Reader only. Copies the newest `pairs` contiguous pairs into `out` and
    /// returns their RF center, or `None` if not enough valid samples exist or
    /// the writer overwrote the window during the copy.
    pub(crate) fn snapshot(&self, pairs: usize, out: &mut Vec<f32>) -> Option<u64> {
        let valid_from = self.valid_from.load(Ordering::Acquire);
        let center_hz = self.center_hz.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        let pairs = pairs as u64;
        if head < valid_from.saturating_add(pairs) {
            return None;
        }
        let start = head - pairs;
        out.clear();
        for index in start..head {
            let slot = ((index % RING_CAPACITY_PAIRS) * 2) as usize;
            out.push(f32::from_bits(self.samples[slot].load(Ordering::Relaxed)));
            out.push(f32::from_bits(
                self.samples[slot + 1].load(Ordering::Relaxed),
            ));
        }
        std::sync::atomic::fence(Ordering::Acquire);
        let head_after = self.head.load(Ordering::Acquire);
        let lapped = head_after + RING_WRITER_MARGIN_PAIRS - start > RING_CAPACITY_PAIRS;
        let restarted = self.valid_from.load(Ordering::Acquire) != valid_from;
        (!lapped && !restarted).then_some(center_hz)
    }
}

#[derive(Debug, Default)]
pub(crate) struct SpectrumWorkerStats {
    pub(crate) rows_computed: AtomicU64,
    pub(crate) captures_skipped: AtomicU64,
    pub(crate) fft_latency: Mutex<LatencyHistogram>,
}

pub(crate) struct SpectrumWorker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    pub(crate) stats: Arc<SpectrumWorkerStats>,
}

impl SpectrumWorker {
    pub(crate) fn spawn(
        tci: Arc<TciFrontend>,
        ring: Arc<LatestIqRing>,
        span_hz: u32,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(SpectrumWorkerStats::default());
        let worker_stop = Arc::clone(&stop);
        let worker_stats = Arc::clone(&stats);
        let handle = thread::Builder::new()
            .name("display-spectrum".to_string())
            .spawn(move || run_worker(&tci, &ring, span_hz, &worker_stop, &worker_stats))?;
        Ok(Self {
            stop,
            handle: Some(handle),
            stats,
        })
    }
}

impl Drop for SpectrumWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(
    tci: &TciFrontend,
    ring: &LatestIqRing,
    span_hz: u32,
    stop: &AtomicBool,
    stats: &SpectrumWorkerStats,
) {
    let epoch = Instant::now();
    let mut transforms: BTreeMap<u32, SpectrumFft> = BTreeMap::new();
    let mut deadlines: BTreeMap<(u32, u32), Instant> = BTreeMap::new();
    let mut iq = Vec::with_capacity(SPECTRUM_MAX_FFT_SIZE as usize * 2);
    let mut db = Vec::with_capacity(SPECTRUM_MAX_FFT_SIZE as usize);
    while !stop.load(Ordering::Acquire) {
        let groups = tci.spectrum_groups();
        if groups.is_empty() {
            deadlines.clear();
            thread::sleep(WORKER_IDLE_POLL);
            continue;
        }
        deadlines.retain(|group, _| groups.contains(group));
        let now = Instant::now();
        let mut next_wake = now + WORKER_IDLE_POLL;
        for &(fft_size, interval_ms) in &groups {
            let interval = Duration::from_millis(u64::from(interval_ms));
            let due = deadlines.entry((fft_size, interval_ms)).or_insert(now);
            if now >= *due {
                // Skip missed deadlines instead of bursting to catch up.
                *due = (*due + interval).max(now);
                let captured_at = Instant::now();
                match ring.snapshot(fft_size as usize, &mut iq) {
                    Some(center_hz) => {
                        let fft = transforms
                            .entry(fft_size)
                            .or_insert_with(|| SpectrumFft::new(fft_size as usize));
                        db.resize(fft.size(), 0.0);
                        fft.transform_db(&iq, &mut db);
                        let codes = db.iter().map(|&value| quantize_db(value)).collect();
                        stats
                            .fft_latency
                            .lock_unpoisoned()
                            .record(captured_at.elapsed());
                        stats.rows_computed.fetch_add(1, Ordering::Relaxed);
                        tci.publish_spectrum_row(Arc::new(SpectrumRow {
                            span_hz,
                            fft_size,
                            interval,
                            center_hz,
                            captured_at,
                            capture_to_enqueue_us: captured_at
                                .elapsed()
                                .as_micros()
                                .min(u128::from(u32::MAX))
                                as u32,
                            server_ms: captured_at.duration_since(epoch).as_millis() as u32,
                            codes,
                        }));
                    }
                    None => {
                        stats.captures_skipped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            next_wake = next_wake.min(*due);
        }
        let sleep_for = next_wake.saturating_duration_since(Instant::now());
        if !sleep_for.is_zero() {
            thread::sleep(sleep_for.min(WORKER_IDLE_POLL));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(pairs: usize, bin: f64, amplitude: f32) -> Vec<f32> {
        (0..pairs)
            .flat_map(|n| {
                let phase = 2.0 * std::f64::consts::PI * bin * n as f64 / pairs as f64;
                [
                    amplitude * phase.cos() as f32,
                    amplitude * phase.sin() as f32,
                ]
            })
            .collect()
    }

    /// Reads the flat `fftSize` / `inputIqFloat32` / `expectedDb` fixture that
    /// `remote-web/scripts/generate-fft-parity-fixture.mjs` writes from the
    /// browser's real `fft.ts`. Hand-parsed: the bridge has no JSON dependency.
    fn parity_fixture(path: &std::path::Path) -> (usize, Vec<f32>, Vec<f32>) {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let array = |key: &str| -> Vec<f32> {
            let start = text.find(&format!("\"{key}\":[")).expect(key) + key.len() + 4;
            let end = start + text[start..].find(']').expect(key);
            text[start..end]
                .split(',')
                .map(|value| value.trim().parse::<f64>().expect(key) as f32)
                .collect()
        };
        let size_at = text.find("\"fftSize\":").expect("fftSize") + 10;
        let size_end = size_at + text[size_at..].find(',').expect("fftSize");
        let size = text[size_at..size_end].trim().parse().expect("fftSize");
        (size, array("inputIqFloat32"), array("expectedDb"))
    }

    fn assert_browser_parity(path: &std::path::Path) {
        let (size, iq, expected) = parity_fixture(path);
        assert_eq!(iq.len(), size * 2);
        assert_eq!(expected.len(), size);
        let mut fft = SpectrumFft::new(size);
        let mut actual = vec![0.0; size];
        fft.transform_db(&iq, &mut actual);
        let worst = actual
            .iter()
            .zip(&expected)
            .map(|(a, e)| (a - e).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst <= 0.01,
            "{}: worst bin error {worst} dB",
            path.display()
        );
    }

    #[test]
    fn fft_matches_browser_fft_ts_fixtures() {
        let testdata = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata");
        let mut paths = vec![testdata.join("fft_parity_2048.json")];
        let capture = testdata.join("fft_parity_2048_capture.json");
        if capture.exists() {
            paths.push(capture);
        }
        if let Some(extra) = std::env::var_os("SATURN_FFT_PARITY_FIXTURE") {
            paths = vec![extra.into()];
        }
        for path in paths {
            assert_browser_parity(&path);
        }
    }

    #[test]
    fn fft_size_and_interval_clamp_to_contract() {
        assert_eq!(clamp_spectrum_fft_size(0), 256);
        assert_eq!(clamp_spectrum_fft_size(3000), 2048);
        assert_eq!(clamp_spectrum_fft_size(2048), 2048);
        assert_eq!(clamp_spectrum_fft_size(16384), 4096);
        assert_eq!(clamp_spectrum_interval_ms(1), 33);
        assert_eq!(clamp_spectrum_interval_ms(50), 50);
        assert_eq!(clamp_spectrum_interval_ms(1000), 250);
    }

    #[test]
    fn tone_lands_in_fftshifted_bin_like_browser() {
        let size = 1024;
        let mut fft = SpectrumFft::new(size);
        let mut out = vec![0.0; size];
        fft.transform_db(&tone(size, 100.0, 0.5), &mut out);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        // fftshift puts DC at size/2, positive frequencies above it.
        assert_eq!(peak, size / 2 + 100);
        // Hann coherent gain 0.5 on amplitude 0.5 => -12 dBFS.
        assert!((out[peak] - -12.04).abs() < 0.1, "{}", out[peak]);
    }

    #[test]
    fn silence_floors_at_minus_160_db() {
        let mut fft = SpectrumFft::new(256);
        let mut out = vec![0.0; 256];
        fft.transform_db(&vec![0.0; 512], &mut out);
        assert!(out.iter().all(|&db| (db - -160.0).abs() < 1e-3));
        assert!(out.iter().all(|&db| quantize_db(db) == 0));
    }

    #[test]
    fn quantization_round_trip_is_within_half_step() {
        // Code 255 is -0.625 dB; a Hann-windowed full-scale tone peaks near
        // -6 dBFS, so the top of the range saturates only on overload.
        for tenth in -1600..=-7 {
            let db = tenth as f32 / 10.0;
            let decoded = SPECTRUM_DB_OFFSET + f32::from(quantize_db(db)) * SPECTRUM_DB_STEP;
            assert!(
                (decoded - db).abs() <= SPECTRUM_DB_STEP / 2.0 + 1e-4,
                "{db}"
            );
        }
        assert_eq!(quantize_db(10.0), 255);
        assert_eq!(quantize_db(-200.0), 0);
        assert_eq!(quantize_db(f32::NAN), 0);
    }

    #[test]
    fn ring_snapshot_returns_newest_contiguous_pairs() {
        let ring = LatestIqRing::new(14_200_000);
        let input: Vec<f32> = (0..4000).map(|v| v as f32).collect();
        ring.push(&input[..1000]);
        ring.push(&input[1000..]);
        let mut out = Vec::new();
        assert_eq!(ring.snapshot(256, &mut out), Some(14_200_000));
        assert_eq!(out, input[4000 - 512..]);
        assert_eq!(ring.snapshot(4096, &mut out), None);
    }

    #[test]
    fn ring_restart_discards_samples_from_previous_center() {
        let ring = LatestIqRing::new(7_000_000);
        ring.push(&vec![1.0; 2048]);
        ring.restart(7_100_000);
        let mut out = Vec::new();
        assert_eq!(ring.snapshot(256, &mut out), None);
        ring.push(&vec![2.0; 512]);
        assert_eq!(ring.snapshot(256, &mut out), Some(7_100_000));
        assert!(out.iter().all(|&v| v == 2.0));
    }

    #[test]
    fn ring_wraps_without_losing_order() {
        let ring = LatestIqRing::new(0);
        let chunk: Vec<f32> = (0..8192).map(|v| v as f32).collect();
        for _ in 0..40 {
            ring.push(&chunk);
        }
        let mut out = Vec::new();
        assert!(ring.snapshot(4096, &mut out).is_some());
        assert_eq!(out, chunk[8192 - 8192..]);
    }

    #[test]
    fn stale_row_is_older_than_two_intervals() {
        let captured_at = Instant::now();
        let row = SpectrumRow {
            span_hz: 384_000,
            fft_size: 256,
            interval: Duration::from_millis(50),
            center_hz: 0,
            captured_at,
            capture_to_enqueue_us: 0,
            server_ms: 0,
            codes: vec![0; 256].into_boxed_slice(),
        };
        assert!(!row.is_stale(captured_at + Duration::from_millis(100)));
        assert!(row.is_stale(captured_at + Duration::from_millis(101)));
    }
}
