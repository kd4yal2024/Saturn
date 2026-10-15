//! Wire/DMA replay adapters. Lifetime-scoped fence must be supplied by the sole
//! acquisition owner during a configuration transaction. This is NOT permission
//! to start a second XDMA reader or to issue the proposed register configuration.
use super::*;

pub const PS_RATE_WORD: u32 = 0x1f;
pub const PS_WITH_DDC6_RATE_WORD: u32 = 0x10_001f;
pub const P2_PAIR_COUNT: usize = 119;
pub const XDMA_PAIR_COUNT: usize = 4;

/// Explicit desired layout, to be applied/rolled back by the existing owner in
/// a future integration transaction after RXC1 qualification. Ordinary DDC6
/// rate/tuning/attenuation must be saved and restored, not replaced blindly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedbackPlan {
    pub rate_word: u32,
    pub feedback_ddc: u8,
    pub reference_ddc: u8,
    pub feedback_p2_source: u8,
    pub reference_p2_source: u8,
    pub feedback_register_source: u8,
    pub reference_register_source: u8,
    pub shared_lo: bool,
    pub independent_lo_debug: bool,
    pub feedback_rate_hz: u32,
    pub feedback_bits: u16,
    pub retain_ddc6: bool,
}
impl FeedbackPlan {
    pub fn adc1(retain_ddc6: bool) -> Self {
        Self {
            rate_word: if retain_ddc6 {
                PS_WITH_DDC6_RATE_WORD
            } else {
                PS_RATE_WORD
            },
            feedback_ddc: 0,
            reference_ddc: 1,
            feedback_p2_source: 0,
            reference_p2_source: 2,
            feedback_register_source: 0,
            reference_register_source: 3,
            shared_lo: true,
            independent_lo_debug: false,
            feedback_rate_hz: RATE_HZ,
            feedback_bits: 24,
            retain_ddc6,
        }
    }
}

pub struct P2Adapter {
    fence: Fence,
    route: u128,
    next_sequence: Option<u32>,
    next_pair: u64,
    poisoned: bool,
}
impl P2Adapter {
    pub fn new(fence: Fence, route: u128) -> Self {
        Self {
            fence,
            route,
            next_sequence: None,
            next_pair: 0,
            poisoned: false,
        }
    }
    /// DDC0 endpoint only: full 1444-byte packet, 24-bit, 238 IQ entries.
    /// P2 timestamp bytes are zero at this source pin; arrival is NOT a hardware
    /// capture timestamp and ordinals are generation-relative, not FPGA ticks.
    pub fn decode(
        &mut self,
        ddc: u8,
        bytes: &[u8],
        observed_ns: u64,
        overload: [Option<CounterBracket>; 3],
        out: &mut PairBlock,
    ) -> Result<(), Fault> {
        out.pairs = 0;
        if self.poisoned {
            return Err(Fault::GenerationMismatch);
        }
        let result = self.decode_inner(ddc, bytes, observed_ns, overload, out);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn decode_inner(
        &mut self,
        ddc: u8,
        bytes: &[u8],
        observed_ns: u64,
        overload: [Option<CounterBracket>; 3],
        out: &mut PairBlock,
    ) -> Result<(), Fault> {
        if ddc != 0 || bytes.len() != 1444 {
            return Err(Fault::WrongFormat);
        }
        if u16::from_be_bytes(bytes[12..14].try_into().unwrap()) != 24
            || u16::from_be_bytes(bytes[14..16].try_into().unwrap()) != 238
        {
            return Err(Fault::WrongFormat);
        }
        let sequence = u32::from_be_bytes(bytes[..4].try_into().unwrap());
        if let Some(next) = self.next_sequence {
            if sequence != next {
                return Err(if sequence == next.wrapping_sub(1) {
                    Fault::Duplicate
                } else if sequence.wrapping_sub(next) < 0x8000_0000 {
                    Fault::SequenceGap
                } else {
                    Fault::Reordered
                });
            }
        }
        for i in 0..P2_PAIR_COUNT {
            let pair = &bytes[16 + i * 12..28 + i * 12];
            out.rf[i * 2] = signed24(&pair[..3]);
            out.rf[i * 2 + 1] = signed24(&pair[3..6]);
            out.tx[i * 2] = signed24(&pair[6..9]);
            out.tx[i * 2 + 1] = signed24(&pair[9..12]);
        }
        out.pairs = P2_PAIR_COUNT;
        out.arrival_ns[..P2_PAIR_COUNT].fill(observed_ns);
        out.arrival_times_recorded = true;
        out.meta = self.meta(observed_ns, sequence, overload);
        self.next_sequence = Some(sequence.wrapping_add(1));
        self.next_pair += P2_PAIR_COUNT as u64;
        Ok(())
    }
    fn meta(
        &self,
        observed_ns: u64,
        sequence: u32,
        overload: [Option<CounterBracket>; 3],
    ) -> BlockMeta {
        BlockMeta {
            fence: self.fence,
            backend: Backend::P2,
            route: self.route,
            rate_hz: RATE_HZ,
            observed_ns,
            sequence: Some(sequence),
            overload,
            interval: Interval {
                fence: self.fence,
                first: self.next_pair,
                end: self.next_pair + P2_PAIR_COUNT as u64,
            },
        }
    }
}

pub struct DirectXdmaAdapter {
    fence: Fence,
    route: u128,
    retain_ddc6: bool,
    pending: [u8; 136],
    pending_words: usize,
    pending_oldest_ns: u64,
    next_pair: u64,
    poisoned: bool,
}
impl DirectXdmaAdapter {
    pub fn new(fence: Fence, route: u128, retain_ddc6: bool) -> Self {
        Self {
            fence,
            route,
            retain_ddc6,
            pending: [0; 136],
            pending_words: 0,
            pending_oldest_ns: 0,
            next_pair: 0,
            poisoned: false,
        }
    }
    /// Called once per already-byte-swapped 8-byte DMA word. Holds at most one
    /// frame (17 words with DDC6). Split reads carry only bounded partial state.
    /// False means incomplete, with out.pairs=0. No resync hides a framing fault.
    /// The SAME owner must call mark_loss on FIFO/DMA/host loss and retire this
    /// decoder; headers alone cannot detect a complete silently missing frame.
    pub fn push_word(
        &mut self,
        word: &[u8; 8],
        observed_ns: u64,
        overload: [Option<CounterBracket>; 3],
        out: &mut PairBlock,
        ddc6_iq: &mut [f64; 16],
    ) -> Result<bool, Fault> {
        out.pairs = 0;
        if self.poisoned {
            return Err(Fault::GenerationMismatch);
        }
        if self.pending_words == 0 {
            let expected = FeedbackPlan::adc1(self.retain_ddc6).rate_word;
            if word[7] != 0x80
                || word[4..7] != [0, 0, 0]
                || u32::from_le_bytes(word[..4].try_into().unwrap()) != expected
            {
                self.poisoned = true;
                return Err(Fault::DmaFraming);
            }
            self.pending_oldest_ns = observed_ns;
        } else {
            // Completion cannot rejuvenate words held across a stalled read.
            // Include the header conservatively; this is host arrival, not a
            // hardware acquisition timestamp. Minimum also avoids rejuvenation
            // if a caller supplies an out-of-order arrival timestamp.
            self.pending_oldest_ns = self.pending_oldest_ns.min(observed_ns);
        }
        let offset = self.pending_words * 8;
        self.pending[offset..offset + 8].copy_from_slice(word);
        self.pending_words += 1;
        let frame_words = if self.retain_ddc6 { 17 } else { 9 };
        if self.pending_words != frame_words {
            return Ok(false);
        }
        for i in 0..XDMA_PAIR_COUNT {
            let offset = 8 + i * 16;
            let pair = &self.pending[offset..offset + 16];
            out.rf[i * 2] = signed24(&pair[..3]);
            out.rf[i * 2 + 1] = signed24(&pair[3..6]);
            out.tx[i * 2] = signed24(&pair[8..11]);
            out.tx[i * 2 + 1] = signed24(&pair[11..14]);
        }
        if self.retain_ddc6 {
            for i in 0..8 {
                let offset = 72 + i * 8;
                ddc6_iq[i * 2] = signed24(&self.pending[offset..offset + 3]);
                ddc6_iq[i * 2 + 1] = signed24(&self.pending[offset + 3..offset + 6]);
            }
        } else {
            ddc6_iq.fill(0.0);
        }
        out.pairs = XDMA_PAIR_COUNT;
        out.arrival_ns[..XDMA_PAIR_COUNT].fill(self.pending_oldest_ns);
        out.arrival_times_recorded = true;
        out.meta = BlockMeta {
            fence: self.fence,
            backend: Backend::DirectXdma,
            rate_hz: RATE_HZ,
            route: self.route,
            observed_ns: self.pending_oldest_ns,
            sequence: None,
            overload,
            interval: Interval {
                fence: self.fence,
                first: self.next_pair,
                end: self.next_pair + XDMA_PAIR_COUNT as u64,
            },
        };
        self.next_pair += XDMA_PAIR_COUNT as u64;
        self.pending_words = 0;
        self.pending_oldest_ns = 0;
        Ok(true)
    }
    pub fn mark_loss(&mut self) {
        self.pending_words = 0;
        self.pending_oldest_ns = 0;
        self.poisoned = true;
    }
}

fn signed24(bytes: &[u8]) -> f64 {
    let word = ((bytes[0] as i32) << 24) | ((bytes[1] as i32) << 16) | ((bytes[2] as i32) << 8);
    (word >> 8) as f64 / 8_388_608.0
}
