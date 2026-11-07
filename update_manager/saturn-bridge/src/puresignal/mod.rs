//! M2 offline core: one serialized native owner, two bounded paired decoders.
//! No socket, register, DMA, RF/PTT, firmware, or driver operations live here.
pub mod adapters;
#[cfg(feature = "offline-puresignal-policy-proposal")]
pub mod admission_proposal;
pub mod controller;
pub mod measurement;
#[cfg(feature = "offline-puresignal-tx-lifecycle")]
pub mod tx_lifecycle;

pub const RATE_HZ: u32 = 192_000;
pub const NATIVE_PAIRS: usize = 1024;
pub const MAX_PAIRS: usize = NATIVE_PAIRS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    P2,
    DirectXdma,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Bridge,
    ExternalP2Client,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    NativeUnverified,
    TopologyUnverified,
    NoController,
    ExternalController,
    NotKeyed,
    Settling,
    GenerationMismatch,
    ConfigChanged,
    WrongBackend,
    WrongRate,
    WrongRoute,
    UnequalLength,
    NonFinite,
    WrongFormat,
    Stale,
    FutureObservation,
    SequenceGap,
    Duplicate,
    Reordered,
    DmaFraming,
    FifoLoss,
    QueueOverrun,
    ObservationIncomplete,
    CounterReset,
    CounterInexact,
    AdcOverrange,
    DdcClamp,
    ClipStatusUnknown,
    SevereOverdrive,
    FeedbackLevel,
    CleanupPending,
    CleanupTimeout,
    TxAuthority,
    SolutionRejected,
    NativeFailure,
    InvalidConfig,
}

/// Acquisition fence attached before queueing, not reconstructed at consumption.
/// IDs map to canonical strings in the reviewed wire contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fence {
    pub boot: u128,
    pub generation: u64,
    pub config: u128,
    pub tx_epoch: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub fence: Fence,
    pub first: u64,
    pub end: u64,
}
impl Interval {
    pub fn contains(self, capture: Self) -> bool {
        self.fence == capture.fence
            && self.first <= capture.first
            && self.end >= capture.end
            && self.first < self.end
            && capture.first < capture.end
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverloadSource {
    Adc1,
    Adc2,
    Ddc0Clamp,
    Ddc1Clamp,
}
/// Only a qualified, nonresetting counter bracket can certify a clear interval.
/// A wider bracket with a hit invalidates conservatively, without locating it.
#[derive(Clone, Copy, Debug)]
pub struct CounterBracket {
    pub source: OverloadSource,
    pub interval: Interval,
    pub observed_ns: u64,
    pub counter_generation: u64,
    pub end_counter_generation: u64,
    pub start_count: u64,
    pub end_count: u64,
    /// Source-qualified width, not inferred from the observed value.
    pub width_bits: Option<u8>,
    /// Must cover the entire bracket; unknown is not equivalent to false.
    pub saturated: Option<bool>,
    pub overflow: Option<bool>,
    pub complete: bool,
    pub valid: bool,
}
impl CounterBracket {
    pub fn certify(self, capture: Interval, now_ns: u64) -> Result<(), Fault> {
        if !self.valid {
            return Err(Fault::ClipStatusUnknown);
        }
        if self.observed_ns > now_ns {
            return Err(Fault::FutureObservation);
        }
        if !self.complete || !self.interval.contains(capture) {
            return Err(Fault::ObservationIncomplete);
        }
        if self.counter_generation != self.end_counter_generation
            || self.end_count < self.start_count
        {
            return Err(Fault::CounterReset);
        }
        let maximum = match self.width_bits {
            Some(1..=63) => (1_u64 << self.width_bits.unwrap()) - 1,
            Some(64) => u64::MAX,
            _ => return Err(Fault::CounterInexact),
        };
        if self.saturated != Some(false)
            || self.overflow != Some(false)
            || self.start_count >= maximum
            || self.end_count >= maximum
        {
            return Err(Fault::CounterInexact);
        }
        if self.end_count != self.start_count {
            return Err(match self.source {
                OverloadSource::Adc1 | OverloadSource::Adc2 => Fault::AdcOverrange,
                _ => Fault::DdcClamp,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BlockMeta {
    pub fence: Fence,
    pub backend: Backend,
    pub rate_hz: u32,
    pub route: u128,
    pub interval: Interval,
    pub observed_ns: u64,
    pub sequence: Option<u32>,
    /// Contiguous original P2 datagrams beginning at `sequence`; zero for XDMA.
    pub sequence_packets: u16,
    // Exactly the selected physical ADC and both reserved DDCs, never cached
    // instantaneous booleans. None is unknown, not zero overload events.
    pub overload: [Option<CounterBracket>; 3],
}
impl Default for BlockMeta {
    fn default() -> Self {
        Self {
            fence: Fence::default(),
            backend: Backend::P2,
            rate_hz: RATE_HZ,
            route: 0,
            interval: Interval {
                fence: Fence::default(),
                first: 0,
                end: 0,
            },
            observed_ns: 0,
            sequence: None,
            sequence_packets: 0,
            overload: [None; 3],
        }
    }
}
/// Reused bounded buffer; no vectors allocated on decoder/controller hot paths.
pub struct PairBlock {
    pub meta: BlockMeta,
    /// Existing acquisition owner's gain/layout request serial, stamped before
    /// enqueue. Zero means unavailable, never a matching Auto ATT completion.
    pub acquisition_revision: u64,
    pub pairs: usize,
    pub tx: Box<[f64; MAX_PAIRS * 2]>,
    pub rf: Box<[f64; MAX_PAIRS * 2]>,
    /// Conservative original host arrival for each pair, not FPGA/RF time.
    /// Kept through Direct XDMA frame assembly rather than replacing it with
    /// the last read's time. False means the source did not supply this detail.
    pub arrival_ns: Box<[u64; MAX_PAIRS]>,
    pub arrival_times_recorded: bool,
}
impl Default for PairBlock {
    fn default() -> Self {
        Self {
            meta: BlockMeta::default(),
            acquisition_revision: 0,
            pairs: 0,
            tx: Box::new([0.0; MAX_PAIRS * 2]),
            rf: Box::new([0.0; MAX_PAIRS * 2]),
            arrival_ns: Box::new([0; MAX_PAIRS]),
            arrival_times_recorded: false,
        }
    }
}
impl PairBlock {
    pub fn copy_from(&mut self, other: &Self) {
        self.meta = other.meta;
        self.acquisition_revision = other.acquisition_revision;
        self.pairs = other.pairs;
        self.tx[..other.pairs * 2].copy_from_slice(&other.tx[..other.pairs * 2]);
        self.rf[..other.pairs * 2].copy_from_slice(&other.rf[..other.pairs * 2]);
        self.arrival_ns[..other.pairs].copy_from_slice(&other.arrival_ns[..other.pairs]);
        self.arrival_times_recorded = other.arrival_times_recorded;
    }
}

#[cfg(test)]
mod tests;
