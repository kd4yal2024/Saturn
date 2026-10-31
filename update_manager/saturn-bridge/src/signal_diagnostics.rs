//! Observation-only capture taps and offline analysis. No native calls, device
//! access, TX authority, signal modification, fitting or calibration.
//!
//! Producers use try_lock and preallocated slots. Analysis, formatting and
//! export belong to a consumer, never an owner/render/DMA turn.
use crate::diagnostic_fft::SpectrumFft;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Point {
    PreWdspPcm,
    WdspIq,
    BeforeOutputIq,
    ReturnedReferenceIq,
    ReturnedRfIq,
    Timeline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Backend {
    P2,
    DirectXdma,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Evidence {
    Unknown,
    Clear,
    Fault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TimeDomain {
    BridgeIngress,
    HostProcessing,
    OfflineFixture,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Event {
    Samples,
    OperatorRequest,
    SourceSelected,
    FifoPrepared,
    KeyAsserted,
    ControlChanged,
    OutputCompleted,
    StopRequested,
    CleanupCompleted,
    Rejected,
}

/// All IDs/times originate at the capture point. Consumers never substitute
/// receipt time or a newer generation. None remains unknown, not clear/zero.
#[derive(Clone, Copy, Debug)]
pub struct Meta {
    pub point: Point,
    pub backend: Backend,
    pub event: Event,
    pub session: u128,
    pub generation: u64,
    pub config: u128,
    pub tx_epoch: u64,
    pub observed_first_ns: Option<u64>,
    pub observed_last_ns: Option<u64>,
    pub recorded_ns: u64,
    pub time_domain: TimeDomain,
    pub rate_hz: u32,
    pub units_per_full_scale: f64,
    pub attenuation_command_db: Option<u8>,
    pub continuity: Evidence,
    pub overload: Evidence,
    /// Snapshot only; not proof of continuously keyed state or physical RF.
    pub keyed: Option<bool>,
    /// Filled only at a real completion boundary. P2 socket completion is NOT
    /// H2C/DMA completion, and an offline file completion is NOT radio output.
    pub requested_bytes: Option<u64>,
    pub completed_bytes: Option<u64>,
}
#[derive(Debug)]
pub struct Record {
    pub meta: Meta,
    pub samples: Vec<f64>,
}
struct Slots {
    free: Vec<Record>,
    ready: VecDeque<Record>,
    remaining: usize,
}
struct Inner {
    slots: Mutex<Slots>,
    max_samples: usize,
    dropped: AtomicU64,
    budget_exhausted: AtomicU64,
}
#[derive(Clone)]
pub struct CaptureTap(Arc<Inner>);
impl CaptureTap {
    pub fn new(
        capacity: usize,
        max_samples: usize,
        record_budget: usize,
        template: Meta,
    ) -> Result<Self, &'static str> {
        if !(1..=32).contains(&capacity)
            || !(1..=16384).contains(&max_samples)
            || !(1..=8192).contains(&record_budget)
        {
            return Err("invalid bounded capture capacity");
        }
        let free = (0..capacity)
            .map(|_| Record {
                meta: template,
                samples: Vec::with_capacity(max_samples),
            })
            .collect();
        Ok(Self(Arc::new(Inner {
            slots: Mutex::new(Slots {
                free,
                ready: VecDeque::with_capacity(capacity),
                remaining: record_budget,
            }),
            max_samples,
            dropped: AtomicU64::new(0),
            budget_exhausted: AtomicU64::new(0),
        })))
    }
    /// No waiting, allocation, FFT, logging or file I/O. Malformed/nonfinite
    /// samples are still retained unchanged (within the declared size bound).
    pub fn try_capture(&self, meta: Meta, samples: &[f64]) -> bool {
        self.try_capture_iter(meta, samples.iter().copied())
    }
    pub fn try_capture_iter(
        &self,
        meta: Meta,
        samples: impl ExactSizeIterator<Item = f64>,
    ) -> bool {
        if samples.len() > self.0.max_samples {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let Ok(mut slots) = self.0.slots.try_lock() else {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if slots.remaining == 0 {
            self.0.budget_exhausted.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let Some(mut record) = slots.free.pop() else {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        record.meta = meta;
        record.samples.clear();
        record.samples.extend(samples);
        slots.ready.push_back(record);
        slots.remaining -= 1;
        true
    }
    /// Consumer only. Analysis/export occurs AFTER releasing the queue lock.
    pub fn consume_one<T>(&self, consume: impl FnOnce(&Record) -> T) -> Option<T> {
        let record = self.0.slots.lock().ok()?.ready.pop_front()?;
        let result = consume(&record);
        if let Ok(mut slots) = self.0.slots.lock() {
            slots.free.push(record);
        }
        Some(result)
    }
    pub fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }
    pub fn budget_exhausted(&self) -> u64 {
        self.0.budget_exhausted.load(Ordering::Relaxed)
    }
}

/// Exact f64 sample bits plus explicit metadata; no lossy IQ/WAV conversion.
/// Deliberately small standalone format so analysis adds no JSON/Rust crates.
pub fn write_record(record: &Record, mut out: impl Write) -> io::Result<()> {
    let m = record.meta;
    out.write_all(b"SATDIAG1")?;
    out.write_all(&[
        m.point as u8,
        m.backend as u8,
        m.event as u8,
        m.time_domain as u8,
        m.continuity as u8,
        m.overload as u8,
        m.keyed.map_or(2, u8::from),
        m.attenuation_command_db.unwrap_or(255),
    ])?;
    out.write_all(&m.session.to_le_bytes())?;
    out.write_all(&m.generation.to_le_bytes())?;
    out.write_all(&m.config.to_le_bytes())?;
    for n in [
        m.tx_epoch,
        m.observed_first_ns.unwrap_or(u64::MAX),
        m.observed_last_ns.unwrap_or(u64::MAX),
        m.recorded_ns,
        m.requested_bytes.unwrap_or(u64::MAX),
        m.completed_bytes.unwrap_or(u64::MAX),
    ] {
        out.write_all(&n.to_le_bytes())?;
    }
    out.write_all(&m.rate_hz.to_le_bytes())?;
    out.write_all(&(record.samples.len() as u32).to_le_bytes())?;
    out.write_all(&m.units_per_full_scale.to_le_bytes())?;
    for sample in &record.samples {
        out.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}
pub fn read_record(mut input: impl Read) -> io::Result<Record> {
    fn bad() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid/oversized diagnostic record",
        )
    }
    let mut header = [0; 120];
    input.read_exact(&mut header)?;
    if &header[..8] != b"SATDIAG1" {
        return Err(bad());
    }
    let point = match header[8] {
        0 => Point::PreWdspPcm,
        1 => Point::WdspIq,
        2 => Point::BeforeOutputIq,
        3 => Point::ReturnedReferenceIq,
        4 => Point::ReturnedRfIq,
        5 => Point::Timeline,
        _ => return Err(bad()),
    };
    let backend = match header[9] {
        0 => Backend::P2,
        1 => Backend::DirectXdma,
        _ => return Err(bad()),
    };
    let event = match header[10] {
        0 => Event::Samples,
        1 => Event::OperatorRequest,
        2 => Event::SourceSelected,
        3 => Event::FifoPrepared,
        4 => Event::KeyAsserted,
        5 => Event::ControlChanged,
        6 => Event::OutputCompleted,
        7 => Event::StopRequested,
        8 => Event::CleanupCompleted,
        9 => Event::Rejected,
        _ => return Err(bad()),
    };
    let time_domain = match header[11] {
        0 => TimeDomain::BridgeIngress,
        1 => TimeDomain::HostProcessing,
        2 => TimeDomain::OfflineFixture,
        _ => return Err(bad()),
    };
    let evidence = |v| match v {
        0 => Ok(Evidence::Unknown),
        1 => Ok(Evidence::Clear),
        2 => Ok(Evidence::Fault),
        _ => Err(bad()),
    };
    let n = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
    let opt = |v| if v == u64::MAX { None } else { Some(v) };
    let rate_hz = u32::from_le_bytes(header[104..108].try_into().unwrap());
    let count = u32::from_le_bytes(header[108..112].try_into().unwrap()) as usize;
    if count > 16384 || (header[15] != 255 && header[15] > 31) {
        return Err(bad());
    }
    let meta = Meta {
        point,
        backend,
        event,
        time_domain,
        session: u128::from_le_bytes(header[16..32].try_into().unwrap()),
        generation: n(32),
        config: u128::from_le_bytes(header[40..56].try_into().unwrap()),
        tx_epoch: n(56),
        observed_first_ns: opt(n(64)),
        observed_last_ns: opt(n(72)),
        recorded_ns: n(80),
        requested_bytes: opt(n(88)),
        completed_bytes: opt(n(96)),
        rate_hz,
        units_per_full_scale: f64::from_le_bytes(header[112..120].try_into().unwrap()),
        continuity: evidence(header[12])?,
        overload: evidence(header[13])?,
        keyed: match header[14] {
            0 => Some(false),
            1 => Some(true),
            2 => None,
            _ => return Err(bad()),
        },
        attenuation_command_db: if header[15] == 255 {
            None
        } else {
            Some(header[15])
        },
    };
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let mut b = [0; 8];
        input.read_exact(&mut b)?;
        samples.push(f64::from_le_bytes(b));
    }
    let mut extra = [0];
    if input.read(&mut extra)? != 0 {
        return Err(bad());
    }
    Ok(Record { meta, samples })
}

#[derive(Debug)]
pub struct Analysis {
    pub tone_estimates_resolved: bool,
    pub component_peak: f64,
    pub rms: f64,
    pub dc_magnitude: f64,
    pub components_at_full_scale: usize,
    pub silent: bool,
    pub used_frames: usize,
    pub raw_frames: usize,
    pub bin_hz: f64,
    pub tone_hz: [f64; 2],
    pub tone_amplitude: [f64; 2],
    pub opposite_amplitude: [f64; 2],
    /// fftshifted two-sided amplitude, Hann coherent-gain corrected; PCM is
    /// still two-sided here. Tone estimates are one-sided for real PCM only.
    pub spectrum_dbfs: Vec<Option<f64>>,
}
pub fn analyze(record: &Record, tone_hz: [f64; 2]) -> Result<Analysis, &'static str> {
    let m = record.meta;
    let real = m.point == Point::PreWdspPcm;
    if m.event != Event::Samples || m.point == Point::Timeline {
        return Err("not a sample record");
    }
    let stride = if real { 1 } else { 2 };
    if record.samples.is_empty()
        || !record.samples.len().is_multiple_of(stride)
        || record.samples.len() > 16384
        || m.rate_hz == 0
        || !m.units_per_full_scale.is_finite()
        || m.units_per_full_scale <= 0.
        || record.samples.iter().any(|x| !x.is_finite())
        || tone_hz
            .iter()
            .any(|f| !f.is_finite() || f.abs() >= m.rate_hz as f64 / 2.)
        || tone_hz[0] == 0.
        || tone_hz[1] == 0.
        || tone_hz[0] == tone_hz[1]
        || (real && tone_hz.iter().any(|f| *f < 0.))
    {
        return Err("invalid samples/rate/scaling/tones");
    }
    if [m.observed_first_ns, m.observed_last_ns]
        .into_iter()
        .flatten()
        .any(|v| v > m.recorded_ns)
    {
        return Err("invalid original observation interval");
    }
    if let (Some(first), Some(last)) = (m.observed_first_ns, m.observed_last_ns) {
        if first > last || last > m.recorded_ns {
            return Err("invalid original observation interval");
        }
    }
    let raw_frames = record.samples.len() / stride;
    let size = 1usize << (usize::BITS - 1 - raw_frames.leading_zeros());
    if size < 32 {
        return Err("insufficient FFT window");
    }
    let scale = m.units_per_full_scale;
    if record
        .samples
        .iter()
        .any(|v| (v / scale).abs() > f32::MAX as f64 / (size as f64 * 2.))
    {
        return Err("analysis precision overflow; raw retained");
    }
    let mut iq = Vec::with_capacity(size * 2);
    let mut energy = 0.;
    let mut peak = 0_f64;
    let mut clips = 0;
    let mut dc = [0.; 2];
    for frame in record.samples.chunks_exact(stride) {
        let i = frame[0] / scale;
        let q = if real { 0. } else { frame[1] / scale };
        if !i.is_finite()
            || !q.is_finite()
            || i.abs() > f32::MAX as f64
            || q.abs() > f32::MAX as f64
        {
            return Err("analysis precision overflow; raw retained");
        }
        peak = peak.max(i.abs()).max(q.abs());
        clips += usize::from(i.abs() >= 1.) + usize::from(!real && q.abs() >= 1.);
        energy += i * i + q * q;
        dc[0] += i;
        dc[1] += q;
        if iq.len() < size * 2 {
            iq.extend([i as f32, q as f32]);
        }
    }
    if !energy.is_finite() {
        return Err("analysis energy overflow; raw retained");
    }
    let weights: Vec<f64> = (0..size)
        .map(|n| {
            (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / (size - 1) as f64).cos()) as f32 as f64
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    let magnitude = |hz: f64| {
        let mut a = [0.; 2];
        for (n, p) in iq.chunks_exact(2).enumerate() {
            let phase = std::f64::consts::TAU * hz * n as f64 / m.rate_hz as f64;
            a[0] += weights[n] * (f64::from(p[0]) * phase.cos() + f64::from(p[1]) * phase.sin());
            a[1] += weights[n] * (f64::from(p[1]) * phase.cos() - f64::from(p[0]) * phase.sin());
        }
        a[0].hypot(a[1]) / sum * if real { 2. } else { 1. }
    };
    let tone_amplitude = tone_hz.map(magnitude);
    let opposite_amplitude = tone_hz.map(|hz| magnitude(-hz));
    let mut fft = SpectrumFft::new(size);
    let mut bins = vec![0.; size];
    fft.transform_db(&iq, &mut bins);
    let spectrum_dbfs = bins
        .into_iter()
        .map(|db| {
            let amp = (10_f64.powf(db as f64 / 20.) - 1e-8).max(0.) * size as f64 / sum;
            (amp > 0.).then(|| 20. * amp.log10())
        })
        .collect();
    Ok(Analysis {
        tone_estimates_resolved: (tone_hz[0] - tone_hz[1]).abs()
            >= 4. * m.rate_hz as f64 / size as f64,
        component_peak: peak,
        rms: (energy / raw_frames as f64).sqrt(),
        dc_magnitude: dc[0].hypot(dc[1]) / raw_frames as f64,
        components_at_full_scale: clips,
        silent: peak == 0.,
        used_frames: size,
        raw_frames,
        bin_hz: m.rate_hz as f64 / size as f64,
        tone_hz,
        tone_amplitude,
        opposite_amplitude,
        spectrum_dbfs,
    })
}
fn number(v: Option<f64>) -> String {
    v.filter(|n| n.is_finite())
        .map_or("null".into(), |n| n.to_string())
}
fn optional(v: Option<u64>) -> String {
    v.map_or("null".into(), |n| format!("\"{n}\""))
}
/// Controlled enum labels/numbers only; no arbitrary strings or credentials.
pub fn report_json(record: &Record, tones: [f64; 2]) -> String {
    let m = record.meta;
    let base=format!("\"format\":\"saturn-signal-diagnostics-v1\",\"session\":\"{}\",\"generation\":\"{}\",\"config\":\"{}\",\"point\":\"{:?}\",\"backend\":\"{:?}\",\"event\":\"{:?}\",\"timeDomain\":\"{:?}\",\"observedFirstNs\":{},\"observedLastNs\":{},\"recordedNs\":\"{}\",\"rateHz\":{},\"unitsPerFullScale\":{},\"attenuationCommandDb\":{},\"continuity\":\"{:?}\",\"overload\":\"{:?}\",\"keyedSnapshot\":{},\"requestedBytes\":{},\"completedBytes\":{},\"rfOutputVerified\":false,\"rfImdQualified\":false,\"continuousKeyedStateVerified\":false,\"deliveryAgeVerified\":false",
        m.session,m.generation,m.config,m.point,m.backend,m.event,m.time_domain,
        optional(m.observed_first_ns),optional(m.observed_last_ns),m.recorded_ns,m.rate_hz,
        number(Some(m.units_per_full_scale)),m.attenuation_command_db.map_or("null".into(),|v|v.to_string()),
        m.continuity,m.overload,m.keyed.map_or("null".into(),|v|v.to_string()),optional(m.requested_bytes),optional(m.completed_bytes));
    let base = format!(
        "{base},\"txEpoch\":\"{}\",\"sourceIdentity\":\"requires-original-capture-sidecar\"",
        m.tx_epoch
    );
    match analyze(record, tones) {
        Err(reason) => {
            format!("{{{base},\"analysisAvailable\":false,\"analysisReason\":\"{reason}\"}}")
        }
        Ok(a) => {
            let base = format!(
                "{base},\"toneEstimatesResolved\":{}",
                a.tone_estimates_resolved
            );
            let bins = a
                .spectrum_dbfs
                .iter()
                .map(|v| number(*v))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{base},\"analysisAvailable\":true,\"componentPeak\":{},\"rms\":{},\"dcMagnitude\":{},\"componentsAtFullScale\":{},\"silent\":{},\"rawFrames\":{},\"fftFrames\":{},\"binHz\":{},\"window\":\"symmetric-Hann\",\"fftNormalization\":\"abs(FFT)/sum(window); fftshift; two-sided\",\"toneNormalization\":\"Hann DTFT / sum(window); real PCM x2 only\",\"toneHz\":[{},{}],\"toneAmplitude\":[{},{}],\"oppositeAmplitude\":[{},{}],\"spectrumDbfs\":[{bins}]}}",
                a.component_peak,a.rms,a.dc_magnitude,a.components_at_full_scale,a.silent,a.raw_frames,a.used_frames,a.bin_hz,
                a.tone_hz[0],a.tone_hz[1],a.tone_amplitude[0],a.tone_amplitude[1],a.opposite_amplitude[0],a.opposite_amplitude[1])
        }
    }
}

#[cfg(test)]
mod tests;
