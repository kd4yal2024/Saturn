//! Native status is a build-specific contract, not an RF overload detector.
//! Reviewed WDSP 2.10 calcc.c SHA-256:
//! 1aa8470294640e7109824db7cc41786a02e21a5f0b9cf54bbbcfaf2df190ab66.
//! That source sets info[6].0 for a failed solution check and info[6].1
//! when upper-envelope collection must reset. Neither bit measures ADC rails.
use super::Fault;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeStatusContract {
    /// Preserve the prior fail-closed behavior for unreviewed/native test ports.
    #[default]
    LegacyFaultBits,
    ReviewedWdsp210,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationNotice {
    SolutionRejected,
    CollectorReset,
}

impl NativeStatusContract {
    pub fn interpret(self, bits: i32) -> Result<Option<CalibrationNotice>, Fault> {
        match self {
            Self::LegacyFaultBits if bits & 2 != 0 => Err(Fault::SevereOverdrive),
            Self::LegacyFaultBits if bits & 1 != 0 => Err(Fault::SolutionRejected),
            Self::LegacyFaultBits => Ok(None),
            Self::ReviewedWdsp210 if bits < 0 || bits & !3 != 0 => Err(Fault::NativeUnverified),
            Self::ReviewedWdsp210 if bits & 2 != 0 => Ok(Some(CalibrationNotice::CollectorReset)),
            Self::ReviewedWdsp210 if bits & 1 != 0 => Ok(Some(CalibrationNotice::SolutionRejected)),
            Self::ReviewedWdsp210 => Ok(None),
        }
    }
}
