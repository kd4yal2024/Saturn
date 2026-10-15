//! Bounded observation only. No RF authority, native calls, fitting, resampling,
//! normalization or calibration is performed here. Export is outside TX turns.
use super::*;
use std::collections::VecDeque;
use std::fmt::Write;

pub const SIGNED24_DENOMINATOR: f64 = 8_388_608.0;

#[derive(Clone, Debug)]
pub struct CaptureContext {
    pub source_pin: String,
    pub native_identity: String,
    pub feedback_attenuation_db: Option<u8>,
    /// Commanded drive code, NOT calibrated RF power or measured attenuation.
    pub commanded_drive: Option<u8>,
    pub physical_adc: OverloadSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeMeasurementState {
    pub fitting_enabled: bool,
    pub automatic_enabled: bool,
    pub correction_applied: bool,
    /// False means intent only; it cannot certify native MeasurementOnly.
    pub evidence_verified: bool,
}
impl NativeMeasurementState {
    pub fn disabled_verified(self) -> bool {
        self.evidence_verified
            && !self.fitting_enabled
            && !self.automatic_enabled
            && !self.correction_applied
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverloadQualification {
    Clear,
    Clipped,
    Unknown,
}
#[derive(Clone, Copy, Debug)]
pub struct Levels {
    /// sqrt(mean(I²+Q²)); normalized converter units, not dBm/watts.
    pub complex_rms: f64,
    pub component_peak: f64,
    pub envelope_peak: f64,
    pub envelope_minimum: f64,
}
#[derive(Debug)]
pub struct Capture {
    pub context: CaptureContext,
    pub meta: BlockMeta,
    pub recorded_ns: u64,
    pub native_state: NativeMeasurementState,
    pub admission: Result<(), Fault>,
    /// Preserve the controller's original rejection even if recording discovers
    /// a second defect such as nonfinite data or unverified native state.
    pub original_admission: Result<(), Fault>,
    pub overload: OverloadQualification,
    pub overload_reasons: [Option<Fault>; 3],
    pub reference: Vec<f64>,
    pub rf: Vec<f64>,
    pub original_arrival_ns: Option<Vec<u64>>,
    pub reference_levels: Option<Levels>,
    pub rf_levels: Option<Levels>,
}
fn levels(samples: &[f64]) -> Option<Levels> {
    if samples.is_empty()
        || !samples.len().is_multiple_of(2)
        || samples.iter().any(|v| !v.is_finite())
    {
        return None;
    }
    let mut energy = 0.;
    let mut component_peak: f64 = 0.;
    let mut envelope_peak: f64 = 0.;
    let mut envelope_minimum = f64::INFINITY;
    for iq in samples.chunks_exact(2) {
        let e = iq[0].hypot(iq[1]);
        energy += e * e;
        component_peak = component_peak.max(iq[0].abs()).max(iq[1].abs());
        envelope_peak = envelope_peak.max(e);
        envelope_minimum = envelope_minimum.min(e);
    }
    let complex_rms = (energy / (samples.len() / 2) as f64).sqrt();
    if !complex_rms.is_finite() {
        return None;
    }
    Some(Levels {
        complex_rms,
        component_peak,
        envelope_peak,
        envelope_minimum,
    })
}
fn capture(
    context: &CaptureContext,
    block: &PairBlock,
    now: u64,
    native_state: NativeMeasurementState,
    mut admission: Result<(), Fault>,
) -> Capture {
    let original_admission = admission;
    // Malformed lengths remain evidence, bounded to the storage actually present.
    let pairs = block.pairs.min(MAX_PAIRS);
    let reference = block.tx[..pairs * 2].to_vec();
    let rf = block.rf[..pairs * 2].to_vec();
    if block.pairs == 0 || block.pairs > MAX_PAIRS {
        admission = Err(Fault::UnequalLength);
    } else if reference.iter().chain(&rf).any(|v| !v.is_finite()) {
        admission = Err(Fault::NonFinite);
    } else if !native_state.disabled_verified() {
        admission = Err(Fault::NativeUnverified);
    } else if block.meta.observed_ns > now
        || (block.arrival_times_recorded && block.arrival_ns[..pairs].iter().any(|v| *v > now))
    {
        admission = Err(Fault::FutureObservation);
    }
    let expected = [
        context.physical_adc,
        OverloadSource::Ddc0Clamp,
        OverloadSource::Ddc1Clamp,
    ];
    let latest = if block.arrival_times_recorded {
        block.arrival_ns[..pairs]
            .iter()
            .copied()
            .max()
            .unwrap_or(block.meta.observed_ns)
    } else {
        block.meta.observed_ns
    };
    let reasons = std::array::from_fn(|i| match block.meta.overload[i] {
        None => Some(Fault::ClipStatusUnknown),
        Some(b) if b.source != expected[i] => Some(Fault::WrongRoute),
        Some(b) if b.observed_ns < latest => Some(Fault::ObservationIncomplete),
        Some(b) => b.certify(block.meta.interval, now).err(),
    });
    let overload = if reasons
        .iter()
        .any(|r| matches!(r, Some(Fault::AdcOverrange | Fault::DdcClamp)))
    {
        OverloadQualification::Clipped
    } else if reasons.iter().any(Option::is_some) {
        OverloadQualification::Unknown
    } else {
        OverloadQualification::Clear
    };
    Capture {
        context: context.clone(),
        meta: block.meta,
        recorded_ns: now,
        native_state,
        admission,
        original_admission,
        overload,
        overload_reasons: reasons,
        reference_levels: levels(&reference),
        rf_levels: levels(&rf),
        reference,
        rf,
        original_arrival_ns: block
            .arrival_times_recorded
            .then(|| block.arrival_ns[..pairs].to_vec()),
    }
}

pub struct MeasurementRecorder {
    context: CaptureContext,
    capacity: usize,
    records: VecDeque<Capture>,
    /// Dedicated extra slot retains the first queue-fault block too. Later
    /// arrivals after cancellation count as loss, never silently overwrite data.
    overflow_record: Option<Capture>,
    pub lost_blocks: u64,
    terminal_fault: Option<Fault>,
    cleanup_completed_ns: Option<u64>,
    finalized: bool,
}
impl MeasurementRecorder {
    pub fn context(&self) -> &CaptureContext {
        &self.context
    }
    pub fn new(capacity_blocks: usize, context: CaptureContext) -> Result<Self, Fault> {
        if !(1..=4096).contains(&capacity_blocks)
            || context.source_pin.is_empty()
            || context.native_identity.is_empty()
            || context.feedback_attenuation_db.is_some_and(|v| v > 31)
            || !matches!(
                context.physical_adc,
                OverloadSource::Adc1 | OverloadSource::Adc2
            )
        {
            return Err(Fault::InvalidConfig);
        }
        Ok(Self {
            context,
            capacity: capacity_blocks,
            records: VecDeque::with_capacity(capacity_blocks),
            overflow_record: None,
            lost_blocks: 0,
            terminal_fault: None,
            cleanup_completed_ns: None,
            finalized: false,
        })
    }
    pub fn record(
        &mut self,
        block: &PairBlock,
        recorded_ns: u64,
        native_state: NativeMeasurementState,
        admission: Result<(), Fault>,
    ) -> Result<(), Fault> {
        if self.cleanup_completed_ns.is_some() {
            return Err(Fault::CleanupPending);
        }
        if self.records.len() == self.capacity {
            if self.overflow_record.is_none() {
                let mut overflow =
                    capture(&self.context, block, recorded_ns, native_state, admission);
                // Overflow is the recorder's failure, not the controller's
                // admission result. Keep the original result captured above.
                overflow.admission = Err(Fault::QueueOverrun);
                self.overflow_record = Some(overflow);
            } else {
                self.lost_blocks = self.lost_blocks.saturating_add(1);
            }
            return Err(Fault::QueueOverrun);
        }
        self.records.push_back(capture(
            &self.context,
            block,
            recorded_ns,
            native_state,
            admission,
        ));
        Ok(())
    }
    pub fn records(&self) -> &VecDeque<Capture> {
        &self.records
    }
    pub fn overflow_record(&self) -> Option<&Capture> {
        self.overflow_record.as_ref()
    }
    /// Record stop/cleanup outcome as well as per-block admission. Acquisition
    /// faults can arise before decoding a PairBlock; they must not disappear
    /// merely because the last successfully decoded block was continuous.
    pub fn finish(&mut self, fault: Option<Fault>, cleanup_completed_ns: Option<u64>) {
        self.terminal_fault = self.terminal_fault.or(fault);
        self.cleanup_completed_ns = cleanup_completed_ns;
        self.finalized = true;
    }
    /// Acknowledgment clears the live fault latch, not historical measurement
    /// evidence. Keep the first diagnostic fault before allowing acknowledgment.
    pub fn note_fault(&mut self, fault: Fault) {
        self.terminal_fault.get_or_insert(fault);
    }
    /// No clearing/streaming on the active path; generation retirement controls
    /// recorder replacement. Slow export cannot postpone release processing.
    pub fn export_json(&self) -> String {
        let retention_complete = self.overflow_record.is_none() && self.lost_blocks == 0;
        let mut out = format!("{{\"format\":\"saturn-feedback-measurement-v1\",\"rfImdQualified\":false,\"recordingComplete\":{},\"retentionComplete\":{},\"captureFinished\":{},\"cleanupCompletedNs\":{},\"terminalFault\":{},\"lostBlocks\":{},\"captures\":[",
                              retention_complete && !self.records.is_empty() && self.finalized && self.cleanup_completed_ns.is_some() && self.terminal_fault.is_none(),
                              retention_complete, self.finalized,
                              self.cleanup_completed_ns.map_or_else(|| "null".into(), |v| v.to_string()),
                              optional_debug(self.terminal_fault), self.lost_blocks);
        for (i, c) in self
            .records
            .iter()
            .chain(self.overflow_record.iter())
            .enumerate()
        {
            if i != 0 {
                out.push(',');
            }
            out.push_str(&c.export_json());
        }
        out.push_str("]}");
        out
    }
}
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c <= '\u{1f}' => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn optional_debug<T: std::fmt::Debug>(v: Option<T>) -> String {
    v.map_or_else(|| "null".into(), |x| quoted(&format!("{x:?}")))
}
fn fence_json(f: Fence) -> String {
    format!(
        "{{\"boot\":\"{}\",\"generation\":{},\"config\":\"{}\",\"txEpoch\":{}}}",
        f.boot, f.generation, f.config, f.tx_epoch
    )
}
fn interval_json(i: Interval) -> String {
    format!(
        "{{\"fence\":{},\"first\":{},\"endExclusive\":{}}}",
        fence_json(i.fence),
        i.first,
        i.end
    )
}
fn bits_json(samples: &[f64]) -> String {
    format!(
        "[{}]",
        samples
            .iter()
            .map(|v| format!("\"{:016x}\"", v.to_bits()))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn levels_json(value: Option<Levels>) -> String {
    value.map_or_else(|| "null".into(), |v| format!(
        "{{\"units\":\"normalized-complex-converter\",\"complexRms\":{},\"componentPeak\":{},\"envelopePeak\":{},\"envelopeMinimum\":{}}}",
        v.complex_rms, v.component_peak, v.envelope_peak, v.envelope_minimum))
}
impl Capture {
    pub fn export_json(&self) -> String {
        let overload = self.meta.overload.iter().enumerate().map(|(i, bracket)| {
            let evidence = match bracket {
                None => "null".into(),
                Some(b) => format!("{{\"source\":{},\"interval\":{},\"observedNs\":{},\"counterGeneration\":\"{}\",\"endCounterGeneration\":\"{}\",\"startCount\":\"{}\",\"endCount\":\"{}\",\"widthBits\":{},\"saturated\":{},\"overflow\":{},\"complete\":{},\"valid\":{}}}",
                    quoted(&format!("{:?}",b.source)), interval_json(b.interval), b.observed_ns,
                    b.counter_generation,b.end_counter_generation,b.start_count,b.end_count,
                    b.width_bits.map_or_else(||"null".into(),|x|x.to_string()),
                    b.saturated.map_or_else(||"null".into(),|x|x.to_string()),
                    b.overflow.map_or_else(||"null".into(),|x|x.to_string()),b.complete,b.valid),
            };
            format!("{{\"evidence\":{evidence},\"reason\":{}}}", optional_debug(self.overload_reasons[i]))
        }).collect::<Vec<_>>().join(",");
        let arrivals = self
            .original_arrival_ns
            .as_ref()
            .map_or_else(|| "null".into(), |v| format!("{v:?}"));
        let mut out = format!("{{\"sourcePin\":{},\"nativeIdentity\":{},\"backend\":{},\"fence\":{},\"route\":\"{}\",\"rateHz\":{},\"pairCount\":{},\"interval\":{},\"oldestObservedNs\":{},\"recordedNs\":{},\"arrivalNs\":{},\"timestampDomain\":\"bridge-monotonic-ingress-not-RF-time\",\"feedbackAttenuationCommandDb\":{},\"commandedDriveCode\":{},\"lp700Measurement\":null,\"sequence\":{},\"admissionFault\":{},\"overloadQualification\":{},\"overload\":[{}],\"native\":{{\"fittingEnabled\":{},\"automaticEnabled\":{},\"correctionApplied\":{},\"verified\":{}}},\"scaling\":{{\"encoding\":\"IEEE754-f64-bits-hex-interleaved-IQ\",\"converter\":\"signed24/8388608\",\"normalizationApplied\":false,\"absoluteRfCalibration\":null}},\"referenceF64Bits\":{},\"rfF64Bits\":{}}}",
            quoted(&self.context.source_pin),quoted(&self.context.native_identity),quoted(&format!("{:?}",self.meta.backend)),
            fence_json(self.meta.fence),self.meta.route,self.meta.rate_hz,self.reference.len()/2,
            interval_json(self.meta.interval),self.meta.observed_ns,self.recorded_ns,arrivals,
            self.context.feedback_attenuation_db.map_or_else(||"null".into(),|x|x.to_string()),
            self.context.commanded_drive.map_or_else(||"null".into(),|x|x.to_string()),
            self.meta.sequence.map_or_else(||"null".into(),|x|x.to_string()), optional_debug(self.admission.err()),
            quoted(&format!("{:?}",self.overload)),overload,self.native_state.fitting_enabled,
            self.native_state.automatic_enabled,self.native_state.correction_applied,self.native_state.evidence_verified,
            bits_json(&self.reference),bits_json(&self.rf));
        out.pop();
        write!(out, ",\"originalAdmissionFault\":{},\"physicalAdc\":{},\"referenceLevels\":{},\"rfLevels\":{},\"overloadProvenUnclipped\":{}}}",
            optional_debug(self.original_admission.err()), quoted(&format!("{:?}",self.context.physical_adc)),
            levels_json(self.reference_levels), levels_json(self.rf_levels), self.overload == OverloadQualification::Clear).unwrap();
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelayValidity {
    ResolvedIntegerLag,
    Ambiguous,
    BoundaryPeak,
}
#[derive(Clone, Copy, Debug)]
pub struct DelayEstimate {
    /// Positive: RF[n + lag] matches reference[n], i.e. RF arrives later.
    /// This is not yet a WDSP SetPSTXDelay sign/calibration recommendation.
    pub rf_lag_samples: i32,
    pub rf_lag_seconds: f64,
    pub correlation_magnitude: f64,
    pub competing_magnitude: f64,
    pub validity: DelayValidity,
    /// Integer sampling quantization only. Total measurement uncertainty is
    /// unknown until repeatability/source clock/path errors are qualified.
    pub quantization_half_sample_seconds: f64,
    pub total_uncertainty_seconds: Option<f64>,
}
/// Mean-subtracted complex correlation. No two-tone IMD or correction fit.
/// Only accepted continuous captures can yield a delay; unknown overload stays
/// unknown in the capture and does not become a proven-unclipped delay claim.
pub fn estimate_integer_delay(c: &Capture, max_lag: usize) -> Result<DelayEstimate, Fault> {
    c.admission?;
    if c.overload == OverloadQualification::Clipped {
        return Err(c
            .overload_reasons
            .iter()
            .flatten()
            .find(|f| matches!(f, Fault::AdcOverrange | Fault::DdcClamp))
            .copied()
            .unwrap_or(Fault::ClipStatusUnknown));
    }
    let n = c.reference.len() / 2;
    if n < 32
        || max_lag == 0
        || max_lag > n / 4
        || c.meta.rate_hz == 0
        || c.rf.len() != c.reference.len()
    {
        return Err(Fault::InvalidConfig);
    }
    if c.reference.iter().chain(&c.rf).any(|v| !v.is_finite()) {
        return Err(Fault::NonFinite);
    }
    let mut scores = Vec::with_capacity(max_lag * 2 + 1);
    for lag in -(max_lag as i32)..=max_lag as i32 {
        let first = if lag < 0 { -lag as usize } else { 0 };
        let end = if lag > 0 { n - lag as usize } else { n };
        let length = (end - first) as f64;
        let (mut txi, mut txq, mut rfi, mut rfq) = (0., 0., 0., 0.);
        for i in first..end {
            let j = (i as i32 + lag) as usize;
            txi += c.reference[2 * i];
            txq += c.reference[2 * i + 1];
            rfi += c.rf[2 * j];
            rfq += c.rf[2 * j + 1];
        }
        txi /= length;
        txq /= length;
        rfi /= length;
        rfq /= length;
        let (mut re, mut im, mut te, mut fe) = (0., 0., 0., 0.);
        for i in first..end {
            let j = (i as i32 + lag) as usize;
            let a = c.reference[2 * i] - txi;
            let b = c.reference[2 * i + 1] - txq;
            let x = c.rf[2 * j] - rfi;
            let y = c.rf[2 * j + 1] - rfq;
            re += a * x + b * y;
            im += a * y - b * x;
            te += a * a + b * b;
            fe += x * x + y * y;
        }
        let denominator = te.sqrt() * fe.sqrt();
        let score = if denominator > 0. {
            re.hypot(im) / denominator
        } else {
            0.
        };
        if !score.is_finite() {
            return Err(Fault::NonFinite);
        }
        scores.push((lag, score));
    }
    let (lag, score) = *scores.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    if score < 0.5 {
        return Err(Fault::FeedbackLevel);
    }
    let competing = scores
        .iter()
        .filter(|(l, _)| (l - lag).abs() > 1)
        .map(|(_, s)| *s)
        .fold(0., f64::max);
    let validity = if lag.unsigned_abs() as usize == max_lag {
        DelayValidity::BoundaryPeak
    } else if competing >= score * 0.9 {
        DelayValidity::Ambiguous
    } else {
        DelayValidity::ResolvedIntegerLag
    };
    Ok(DelayEstimate {
        rf_lag_samples: lag,
        rf_lag_seconds: lag as f64 / c.meta.rate_hz as f64,
        correlation_magnitude: score,
        competing_magnitude: competing,
        validity,
        quantization_half_sample_seconds: 0.5 / c.meta.rate_hz as f64,
        total_uncertainty_seconds: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> CaptureContext {
        CaptureContext {
            source_pin: "synthetic-fixture".into(),
            native_identity: "fake-native-not-hardware".into(),
            feedback_attenuation_db: Some(31),
            commanded_drive: Some(1),
            physical_adc: OverloadSource::Adc1,
        }
    }
    fn off() -> NativeMeasurementState {
        NativeMeasurementState {
            fitting_enabled: false,
            automatic_enabled: false,
            correction_applied: false,
            evidence_verified: true,
        }
    }
    fn block() -> PairBlock {
        let mut b = PairBlock::default();
        b.pairs = 128;
        b.meta.interval.end = 128;
        b.meta.observed_ns = 10;
        b.arrival_times_recorded = true;
        b.arrival_ns[..128].fill(10);
        b.tx[..256].fill(0.25);
        b.rf[..256].fill(0.125);
        b
    }
    #[test]
    fn measurement_raw_bad_samples_and_unknown_coverage_are_preserved() {
        let mut r = MeasurementRecorder::new(2, context()).unwrap();
        let mut b = block();
        b.rf[1] = f64::from_bits(0x7ff8_0000_0000_1234);
        r.record(&b, 100, off(), Err(Fault::Stale)).unwrap();
        let c = &r.records()[0];
        assert_eq!(c.rf[1].to_bits(), b.rf[1].to_bits());
        assert_eq!(c.admission, Err(Fault::NonFinite));
        assert_eq!(c.original_admission, Err(Fault::Stale));
        assert_eq!(c.overload, OverloadQualification::Unknown);
        assert_eq!(c.meta.observed_ns, 10);
        assert_eq!(c.original_arrival_ns.as_ref().unwrap()[0], 10);
        assert!(r.export_json().contains("7ff8000000001234"));
        assert!(r.export_json().contains("\"rfImdQualified\":false"));
    }
    #[test]
    fn measurement_capacity_fault_retains_trigger_without_eviction() {
        let mut r = MeasurementRecorder::new(1, context()).unwrap();
        let b = block();
        r.record(&b, 20, off(), Ok(())).unwrap();
        assert_eq!(r.record(&b, 30, off(), Ok(())), Err(Fault::QueueOverrun));
        assert_eq!(r.records()[0].recorded_ns, 20);
        assert_eq!(
            r.overflow_record().unwrap().admission,
            Err(Fault::QueueOverrun)
        );
        assert_eq!(r.record(&b, 40, off(), Ok(())), Err(Fault::QueueOverrun));
        assert_eq!(r.lost_blocks, 1);
        assert!(r.export_json().contains("\"recordingComplete\":false"));
    }
    #[test]
    fn measurement_overflow_preserves_original_controller_rejection_in_export() {
        for admission in [
            Ok(()),
            Err(Fault::Stale),
            Err(Fault::GenerationMismatch),
            Err(Fault::NonFinite),
        ] {
            let mut r = MeasurementRecorder::new(1, context()).unwrap();
            let mut b = block();
            r.record(&b, 20, off(), Ok(())).unwrap();
            if admission == Err(Fault::NonFinite) {
                b.rf[0] = f64::NAN;
            }
            assert_eq!(r.record(&b, 30, off(), admission), Err(Fault::QueueOverrun));
            let overflow = r.overflow_record().unwrap();
            assert_eq!(overflow.original_admission, admission);
            assert_eq!(overflow.admission, Err(Fault::QueueOverrun));
            assert_eq!(overflow.rf[0].to_bits(), b.rf[0].to_bits());
            assert_eq!(r.records().len(), 1);
            let json = r.export_json();
            let original = admission
                .err()
                .map_or_else(|| "null".into(), |fault| format!("\"{fault:?}\""));
            assert!(json.contains(&format!("\"originalAdmissionFault\":{original}")));
            assert!(json.contains("\"admissionFault\":\"QueueOverrun\""));
        }
    }
    #[test]
    fn measurement_storage_completion_never_hides_decoder_fault_or_unfinished_cleanup() {
        let mut r = MeasurementRecorder::new(2, context()).unwrap();
        r.record(&block(), 20, off(), Ok(())).unwrap();
        assert!(r.export_json().contains("\"recordingComplete\":false"));
        assert!(r.export_json().contains("\"retentionComplete\":true"));
        r.finish(Some(Fault::DmaFraming), None);
        assert!(r.export_json().contains("\"cleanupCompletedNs\":null"));
        r.finish(None, Some(30));
        assert!(r.export_json().contains("\"terminalFault\":\"DmaFraming\""));
        assert!(r.export_json().contains("\"recordingComplete\":false"));
        let mut good = MeasurementRecorder::new(2, context()).unwrap();
        good.finish(None, Some(30));
        assert!(good.export_json().contains("\"recordingComplete\":false"));
        assert_eq!(
            good.record(&block(), 40, off(), Ok(())),
            Err(Fault::CleanupPending)
        );
        assert!(good.records().is_empty());
        let mut good = MeasurementRecorder::new(2, context()).unwrap();
        good.record(&block(), 20, off(), Ok(())).unwrap();
        good.finish(None, Some(30));
        assert!(good.export_json().contains("\"recordingComplete\":true"));
    }
    #[test]
    fn measurement_old_saturated_wrong_source_and_future_counters_stay_unknown() {
        let mut b = block();
        for (i, source) in [
            OverloadSource::Adc1,
            OverloadSource::Ddc0Clamp,
            OverloadSource::Ddc1Clamp,
        ]
        .into_iter()
        .enumerate()
        {
            b.meta.overload[i] = Some(CounterBracket {
                source,
                interval: b.meta.interval,
                observed_ns: 20,
                counter_generation: 1,
                end_counter_generation: 1,
                start_count: 0,
                end_count: 0,
                width_bits: Some(32),
                saturated: Some(false),
                overflow: Some(false),
                complete: true,
                valid: true,
            });
        }
        assert_eq!(
            capture(&context(), &b, 30, off(), Ok(())).overload,
            OverloadQualification::Clear
        );
        for change in 0..4 {
            let original = b.meta.overload[0].unwrap();
            let c = b.meta.overload[0].as_mut().unwrap();
            match change {
                0 => c.observed_ns = 1,
                1 => c.saturated = Some(true),
                2 => c.source = OverloadSource::Adc2,
                _ => c.observed_ns = 40,
            }
            assert_eq!(
                capture(&context(), &b, 30, off(), Ok(())).overload,
                OverloadQualification::Unknown
            );
            b.meta.overload[0] = Some(original);
        }
        b.meta.overload[0].as_mut().unwrap().end_count = 1;
        assert_eq!(
            capture(&context(), &b, 30, off(), Ok(())).overload,
            OverloadQualification::Clipped
        );
        let bracket = b.meta.overload[0].as_mut().unwrap();
        bracket.width_bits = Some(64);
        bracket.start_count = 9_007_199_254_740_993;
        bracket.end_count = 9_007_199_254_740_994;
        assert!(capture(&context(), &b, 30, off(), Ok(()))
            .export_json()
            .contains("\"startCount\":\"9007199254740993\""));
    }
    fn noisy(lag: i32) -> Capture {
        let mut b = block();
        let mut state = 1234u32;
        for v in &mut b.tx[..256] {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *v = (state as f64 / u32::MAX as f64 - 0.5) * 0.5;
        }
        b.rf[..256].fill(0.);
        for i in 0..128 {
            let j = i as i32 + lag;
            if (0..128).contains(&j) {
                b.rf[2 * j as usize] = b.tx[2 * i] * 0.5;
                b.rf[2 * j as usize + 1] = b.tx[2 * i + 1] * 0.5;
            }
        }
        capture(&context(), &b, 20, off(), Ok(()))
    }
    #[test]
    fn measurement_delay_sign_positive_and_negative_is_explicit_not_wdsp_setting() {
        for lag in [-7, 7] {
            let d = estimate_integer_delay(&noisy(lag), 16).unwrap();
            assert_eq!(d.rf_lag_samples, lag);
            assert_eq!(d.validity, DelayValidity::ResolvedIntegerLag);
            assert_eq!(d.total_uncertainty_seconds, None);
            assert_eq!(d.rf_lag_seconds, lag as f64 / 192000.);
        }
    }
    #[test]
    fn measurement_periodic_signal_cannot_resolve_delay() {
        let mut b = block();
        for i in 0..128 {
            b.tx[2 * i] = (i as f64 * 0.5).cos() * 0.25;
            b.tx[2 * i + 1] = (i as f64 * 0.5).sin() * 0.25;
        }
        b.rf[..256].copy_from_slice(&b.tx[..256]);
        let c = capture(&context(), &b, 20, off(), Ok(()));
        assert_ne!(
            estimate_integer_delay(&c, 16).unwrap().validity,
            DelayValidity::ResolvedIntegerLag
        );
    }
    #[test]
    fn measurement_stale_capture_and_native_intent_cannot_be_qualified() {
        let b = block();
        let c = capture(&context(), &b, 20, off(), Err(Fault::Stale));
        assert_eq!(estimate_integer_delay(&c, 16).unwrap_err(), Fault::Stale);
        let mut n = off();
        n.evidence_verified = false;
        assert_eq!(
            capture(&context(), &b, 20, n, Ok(())).admission,
            Err(Fault::NativeUnverified)
        );
    }
    #[test]
    fn measurement_exact_signed24_scaling_both_adapter_paths_and_json() {
        use super::super::adapters::{DirectXdmaAdapter, P2Adapter, PS_RATE_WORD};
        let f = Fence {
            boot: 1,
            generation: 2,
            config: 3,
            tx_epoch: 4,
        };
        let packed = |v: i32| v.to_be_bytes()[1..].to_vec();
        let values = [-8_388_608i32, 8_388_607, 1, -1];
        for backend in [Backend::P2, Backend::DirectXdma] {
            let mut b = PairBlock::default();
            if backend == Backend::P2 {
                let mut packet = vec![0; 1444];
                packet[12..14].copy_from_slice(&24u16.to_be_bytes());
                packet[14..16].copy_from_slice(&238u16.to_be_bytes());
                for (i, v) in values.iter().enumerate() {
                    packet[16 + i * 3..19 + i * 3].copy_from_slice(&packed(*v));
                }
                P2Adapter::new(f, 1)
                    .decode(0, &packet, 10, [None; 3], &mut b)
                    .unwrap();
            } else {
                let mut frame = vec![0; 72];
                frame[..4].copy_from_slice(&PS_RATE_WORD.to_le_bytes());
                frame[7] = 0x80;
                for (i, v) in values.iter().enumerate() {
                    let offset = if i < 2 { 8 + i * 3 } else { 16 + (i - 2) * 3 };
                    frame[offset..offset + 3].copy_from_slice(&packed(*v));
                }
                let mut adapter = DirectXdmaAdapter::new(f, 1, false);
                for (i, word) in frame.chunks_exact(8).enumerate() {
                    adapter
                        .push_word(
                            word.try_into().unwrap(),
                            10 + i as u64,
                            [None; 3],
                            &mut b,
                            &mut [0.; 16],
                        )
                        .unwrap();
                }
            }
            assert_eq!(b.meta.backend, backend);
            assert_eq!(&b.rf[..2], &[-1., 8_388_607. / SIGNED24_DENOMINATOR]);
            assert_eq!(
                &b.tx[..2],
                &[1. / SIGNED24_DENOMINATOR, -1. / SIGNED24_DENOMINATOR]
            );
            assert_eq!(b.arrival_ns[0], 10);
            let mut ctx = context();
            ctx.source_pin = "fixture-quoted\"\\\n".into();
            let mut r = MeasurementRecorder::new(1, ctx).unwrap();
            r.record(&b, 30, off(), Ok(())).unwrap();
            assert_eq!(r.records()[0].meta.fence, f);
            assert_eq!(r.records()[0].overload, OverloadQualification::Unknown);
            println!("MEASUREMENT_JSON {}", r.export_json());
        }
    }
}
