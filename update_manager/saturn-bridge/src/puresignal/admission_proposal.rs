//! UNREVIEWED policy experiment, compiled only by explicit offline opt-in.
//! Missing telemetry is not evidence of a clear capture. No hardware operations.
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct DraftLimits {
    /// RMS reference / verified HW peak above which absent RF is a fault.
    pub active_reference_fraction: f64,
    /// Normalized IQ amplitude, NOT watts or an ADC noise-floor calibration.
    pub feedback_rms_floor: f64,
    /// Component rail guard, independently of complex envelope magnitude.
    pub component_limit: f64,
}
impl DraftLimits {
    pub fn validate(self) -> Result<(), Fault> {
        if !self.active_reference_fraction.is_finite()
            || !(0.005..=1.0).contains(&self.active_reference_fraction)
            || !self.feedback_rms_floor.is_finite()
            || !(0.0..1.0).contains(&self.feedback_rms_floor)
            || self.feedback_rms_floor == 0.0
            || !self.component_limit.is_finite()
            || !(0.5..1.0).contains(&self.component_limit)
        {
            return Err(Fault::InvalidConfig);
        }
        Ok(())
    }
    pub fn check_window(self, tx: &[f64], rf: &[f64], peak: f64) -> Result<(), Fault> {
        self.validate()?;
        if tx.len() != NATIVE_PAIRS * 2 || rf.len() != tx.len() {
            return Err(Fault::UnequalLength);
        }
        if tx.iter().chain(rf).any(|v| !v.is_finite()) {
            return Err(Fault::NonFinite);
        }
        if tx.iter().chain(rf).any(|v| v.abs() >= self.component_limit) {
            return Err(Fault::SevereOverdrive);
        }
        let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / NATIVE_PAIRS as f64).sqrt();
        if rms(tx) / peak >= self.active_reference_fraction && rms(rf) < self.feedback_rms_floor {
            return Err(Fault::FeedbackLevel);
        }
        // Do not manufacture excitation or pause the native clock for silence.
        // Pinned WDSP putSample rejects zero RF/TX and reference below .005 HW
        // peak. Bucket completion and successful fitting remain native decisions.
        Ok(())
    }
}

/// Current-epoch positive evidence can stop adaptation without certifying any
/// particular window clear. A wider bracket with a hit stops conservatively.
/// Counter uncertainty alone is measurement-unavailable, not a fabricated hit.
pub fn check_operational_evidence(
    meta: BlockMeta,
    adc: OverloadSource,
    now: u64,
) -> Result<(), Fault> {
    for (entry, source) in
        meta.overload
            .into_iter()
            .zip([adc, OverloadSource::Ddc0Clamp, OverloadSource::Ddc1Clamp])
    {
        let Some(b) = entry else { continue };
        if b.source != source {
            return Err(Fault::WrongRoute);
        }
        if b.observed_ns > now {
            return Err(Fault::FutureObservation);
        }
        if b.valid
            && b.interval.fence == meta.fence
            && b.interval.first < b.interval.end
            && b.counter_generation == b.end_counter_generation
            && b.end_count > b.start_count
        {
            return Err(match source {
                OverloadSource::Adc1 | OverloadSource::Adc2 => Fault::AdcOverrange,
                _ => Fault::DdcClamp,
            });
        }
    }
    Ok(())
}

/// Only this full, exact check can certify a measured interval. The draft never
/// relaxes CounterBracket::certify, even when operational admission is allowed.
pub fn certify_measurement(
    meta: BlockMeta,
    capture: Interval,
    adc: OverloadSource,
    now: u64,
) -> Result<(), Fault> {
    for (entry, source) in
        meta.overload
            .into_iter()
            .zip([adc, OverloadSource::Ddc0Clamp, OverloadSource::Ddc1Clamp])
    {
        let b = entry.ok_or(Fault::ClipStatusUnknown)?;
        if b.source != source {
            return Err(Fault::WrongRoute);
        }
        b.certify(capture, now)?;
    }
    Ok(())
}

/// Adapter translates an EXISTING positive ADC overflow bit or known clamp
/// delta. This is an event, never a clear bracket or a claim of sample coverage.
#[derive(Clone, Copy, Debug)]
pub struct HazardObservation {
    /// Owner decision fence, not a newly inferred FPGA sample epoch. A positive
    /// report whose RF time cannot be assigned is conservatively associated with
    /// the current eligible session. Do not claim capture interval coverage.
    pub fence: Fence,
    pub source: OverloadSource,
    /// Oldest host arrival of the positive report, not browser receipt or RF time.
    pub observed_ns: u64,
    pub valid: bool,
}
