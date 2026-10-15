//! Generation-tagged framing and bounded delivery, owned by the existing RX
//! acquisition consumer. No RF access, native engine or independent reader.
use super::{feedback_owner::Scope, ps_ingress::SharedIngress};
use saturn_bridge::puresignal::{
    adapters::{DirectXdmaAdapter, P2Adapter},
    Backend, Fault, PairBlock,
};
use std::time::Instant;

pub(crate) struct Stream {
    scope: Scope,
    backend: Backend,
    direct: DirectXdmaAdapter,
    p2: P2Adapter,
    scratch: Option<Box<PairBlock>>,
    accumulated: Option<Box<PairBlock>>,
    origin: Instant,
    ingress: SharedIngress,
}
impl Stream {
    pub(crate) fn new(
        scope: Scope,
        backend: Backend,
        origin: Instant,
        ingress: SharedIngress,
    ) -> Result<Self, Fault> {
        let scratch = ingress.take_feedback_buffer()?;
        let accumulated = if backend == Backend::DirectXdma {
            match ingress.take_feedback_buffer() {
                Ok(block) => Some(block),
                Err(fault) => {
                    ingress.recycle_feedback_buffer(scratch);
                    return Err(fault);
                }
            }
        } else {
            None
        };
        Ok(Self {
            scope,
            backend,
            direct: DirectXdmaAdapter::new(scope.fence, scope.route, true),
            p2: P2Adapter::new(scope.fence, scope.route),
            scratch: Some(scratch),
            accumulated,
            origin,
            ingress,
        })
    }
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    fn observed(&self, at: Instant) -> Result<u64, Fault> {
        at.checked_duration_since(self.origin)
            .and_then(|d| u64::try_from(d.as_nanos()).ok())
            .ok_or(Fault::Stale)
    }
    /// A token acquired BEFORE read/enqueue is required; old reads are discarded,
    /// never rebound to this scope. Observation age is not replaced at dequeue.
    pub(crate) fn p2(
        &mut self,
        token: Option<Scope>,
        packet: &[u8],
        observed: Instant,
    ) -> Result<(), Fault> {
        if token != Some(self.scope) {
            return Ok(());
        }
        if self.backend != Backend::P2 {
            return Err(Fault::WrongFormat);
        }
        let at = self.observed(observed)?;
        self.p2.decode(
            0,
            packet,
            at,
            [None; 3],
            self.scratch.as_mut().ok_or(Fault::QueueOverrun)?,
        )?;
        let replacement = self.ingress.take_feedback_buffer()?;
        let block = self
            .scratch
            .replace(replacement)
            .ok_or(Fault::QueueOverrun)?;
        self.ingress.feedback(block)
    }
    pub(crate) fn direct(
        &mut self,
        token: Option<Scope>,
        bytes: &[u8],
        observed: Instant,
        iq: &mut Vec<f32>,
    ) -> Result<(), Fault> {
        if token != Some(self.scope) {
            return Ok(());
        }
        if self.backend != Backend::DirectXdma || !bytes.len().is_multiple_of(8) {
            return Err(Fault::DmaFraming);
        }
        let at = self.observed(observed)?;
        let mut ddc6 = [0.; 16];
        let scratch = self.scratch.as_mut().ok_or(Fault::QueueOverrun)?;
        for word in bytes.chunks_exact(8) {
            if self
                .direct
                .push_word(word.try_into().unwrap(), at, [None; 3], scratch, &mut ddc6)?
            {
                iq.extend(ddc6.iter().map(|v| *v as f32));
                let block = self.accumulated.as_mut().ok_or(Fault::QueueOverrun)?;
                if block.pairs == 0 {
                    block.meta = scratch.meta;
                    block.arrival_times_recorded = scratch.arrival_times_recorded;
                }
                let start = block.pairs * 2;
                let end = start + scratch.pairs * 2;
                block.tx[start..end].copy_from_slice(&scratch.tx[..scratch.pairs * 2]);
                block.rf[start..end].copy_from_slice(&scratch.rf[..scratch.pairs * 2]);
                block.arrival_ns[block.pairs..block.pairs + scratch.pairs]
                    .copy_from_slice(&scratch.arrival_ns[..scratch.pairs]);
                block.arrival_times_recorded &= scratch.arrival_times_recorded;
                block.pairs += scratch.pairs;
                block.meta.observed_ns = block.meta.observed_ns.min(scratch.meta.observed_ns);
                block.meta.interval.end = scratch.meta.interval.end;
                if block.pairs == 1024 {
                    let replacement = self.ingress.take_feedback_buffer()?;
                    let complete = self.accumulated.replace(replacement).unwrap();
                    self.ingress.feedback(complete)?;
                }
            }
        }
        Ok(())
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        // Retained frame/assembly history is destroyed; only storage is reused.
        if let Some(block) = self.accumulated.take() {
            self.ingress.recycle_feedback_buffer(block);
        }
        if let Some(block) = self.scratch.take() {
            self.ingress.recycle_feedback_buffer(block);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tx_thread::feedback_owner::tests::{collector, scope};
    pub(crate) fn direct_frames(count: usize) -> Vec<u8> {
        let mut frame = vec![0; 136];
        frame[..4].copy_from_slice(&0x10001fu32.to_le_bytes());
        frame[7] = 0x80;
        for pair in 0..4 {
            frame[8 + pair * 16] = 0x10; // RF I = 0.125
            frame[16 + pair * 16] = 0x20; // ref I = 0.25
        }
        for pair in 0..8 {
            frame[72 + pair * 8] = 0x30;
        } // DDC6 I = 0.375
        frame.repeat(count)
    }
    pub(crate) fn packet(sequence: u32) -> Vec<u8> {
        let mut packet = vec![0; 1444];
        packet[..4].copy_from_slice(&sequence.to_be_bytes());
        packet[12..14].copy_from_slice(&24u16.to_be_bytes());
        packet[14..16].copy_from_slice(&238u16.to_be_bytes());
        for pair in 0..119 {
            packet[16 + pair * 12] = 0x10;
            packet[22 + pair * 12] = 0x20;
        }
        packet
    }
    #[test]
    fn direct_production_assembly_preserves_oldest_read_age_and_drops_old_scope() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::DirectXdma, origin, ingress.clone()).unwrap();
        let bytes = direct_frames(256);
        let mut iq = Vec::new();
        stream
            .direct(Some(scope(0)), &bytes, origin, &mut iq)
            .unwrap();
        assert!(iq.is_empty());
        assert!(rx.feedback.try_recv().is_err());
        stream
            .direct(Some(s), &bytes[..8], origin, &mut iq)
            .unwrap();
        stream
            .direct(
                Some(s),
                &bytes[8..],
                origin + std::time::Duration::from_secs(1),
                &mut iq,
            )
            .unwrap();
        let block = rx.feedback.try_recv().unwrap();
        assert_eq!(block.pairs, 1024);
        assert_eq!(block.meta.observed_ns, 0);
        assert!(block.arrival_times_recorded);
        assert_eq!(&block.arrival_ns[..4], &[0; 4]);
        assert_eq!(block.arrival_ns[4], 1_000_000_000);
        assert_eq!(block.meta.fence, s.fence);
        assert!(block.meta.overload.iter().all(Option::is_none));
        assert_eq!((block.rf[0], block.tx[0], iq[0]), (0.125, 0.25, 0.375));
        assert_eq!(iq.len(), 4096);
        ingress.recycle_feedback_buffer(block);
        drop(stream);
        for _ in 0..4 {
            assert!(ingress.take_feedback_buffer().is_ok());
        }
    }
    #[test]
    fn p2_storage_returns_after_poison_and_old_packet_cannot_rebind() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::P2, origin, ingress.clone()).unwrap();
        stream.p2(Some(scope(0)), &packet(0), origin).unwrap();
        assert!(rx.feedback.try_recv().is_err());
        stream.p2(Some(s), &packet(0), origin).unwrap();
        let block = rx.feedback.try_recv().unwrap();
        assert_eq!((block.pairs, block.rf[0], block.tx[0]), (119, 0.125, 0.25));
        assert!(block.meta.overload.iter().all(Option::is_none));
        ingress.recycle_feedback_buffer(block);
        assert_eq!(
            stream.p2(Some(s), &packet(2), origin),
            Err(Fault::SequenceGap)
        );
        drop(stream);
        for _ in 0..4 {
            assert!(ingress.take_feedback_buffer().is_ok());
        }
    }
}
