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
    acquisition_revision: u64,
    backend: Backend,
    direct: DirectXdmaAdapter,
    p2: P2Adapter,
    scratch: Option<Box<PairBlock>>,
    accumulated: Option<Box<PairBlock>>,
    origin: Instant,
    ingress: Option<SharedIngress>,
    #[cfg(feature = "managed-puresignal")]
    monitor: Option<crate::rf_monitor::Input>,
}
impl Stream {
    pub(crate) fn new(
        scope: Scope,
        backend: Backend,
        origin: Instant,
        ingress: SharedIngress,
    ) -> Result<Self, Fault> {
        #[cfg(feature = "managed-puresignal")]
        if let Some(monitor) = ingress.rf_monitor.get() {
            monitor.activate();
        }
        let scratch = ingress.take_feedback_buffer()?;
        let accumulated = {
            match ingress.take_feedback_buffer() {
                Ok(block) => Some(block),
                Err(fault) => {
                    ingress.recycle_feedback_buffer(scratch);
                    return Err(fault);
                }
            }
        };
        Ok(Self {
            scope,
            acquisition_revision: 0,
            backend,
            direct: DirectXdmaAdapter::new(scope.fence, scope.route, true),
            p2: P2Adapter::new(scope.fence, scope.route),
            scratch: Some(scratch),
            accumulated,
            origin,
            #[cfg(feature = "managed-puresignal")]
            monitor: ingress.rf_monitor.get().cloned(),
            ingress: Some(ingress),
        })
    }
    /// Observation-only stream: reuses decoders/storage, has NO SharedIngress
    /// and cannot submit a block to fitting, correction or TX lifecycle.
    #[cfg(feature = "managed-puresignal")]
    pub(crate) fn observer(
        scope: Scope,
        origin: Instant,
        monitor: crate::rf_monitor::Input,
    ) -> Self {
        Self {
            scope,
            acquisition_revision: 0,
            backend: Backend::DirectXdma,
            direct: DirectXdmaAdapter::new(scope.fence, scope.route, true),
            p2: P2Adapter::new(scope.fence, scope.route),
            scratch: Some(Box::new(PairBlock::default())),
            accumulated: Some(Box::new(PairBlock::default())),
            origin,
            ingress: None,
            monitor: Some(monitor),
        }
    }

    fn submit(&self, block: &PairBlock) {
        if let Some(c) = self.ingress.as_ref().and_then(|i| i.signal_capture.get()) {
            c.pairs(self.scope, block);
        }
        #[cfg(feature = "managed-puresignal")]
        if let Some(monitor) = &self.monitor {
            monitor.push_scoped(block, self.origin, self.scope);
        }
        #[cfg(not(feature = "managed-puresignal"))]
        let _ = block;
    }
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn with_revision(mut self, serial: u64) -> Self {
        self.acquisition_revision = serial;
        if let Some(block) = &mut self.scratch {
            block.acquisition_revision = serial;
        }
        if let Some(block) = &mut self.accumulated {
            block.acquisition_revision = serial;
        }
        self
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
        self.submit(self.scratch.as_ref().ok_or(Fault::QueueOverrun)?);
        let scratch = self.scratch.as_ref().ok_or(Fault::QueueOverrun)?;
        let batch = self.accumulated.as_mut().ok_or(Fault::QueueOverrun)?;
        if batch.pairs == 0 {
            batch.meta = scratch.meta;
            batch.acquisition_revision = self.acquisition_revision;
            batch.meta.sequence_packets = 0;
            batch.arrival_times_recorded = scratch.arrival_times_recorded;
        }
        let first = batch.pairs;
        let end = first + scratch.pairs;
        batch.tx[first * 2..end * 2].copy_from_slice(&scratch.tx[..scratch.pairs * 2]);
        batch.rf[first * 2..end * 2].copy_from_slice(&scratch.rf[..scratch.pairs * 2]);
        batch.arrival_ns[first..end].copy_from_slice(&scratch.arrival_ns[..scratch.pairs]);
        batch.arrival_times_recorded &= scratch.arrival_times_recorded;
        batch.pairs = end;
        batch.meta.observed_ns = batch.meta.observed_ns.min(scratch.meta.observed_ns);
        batch.meta.interval.end = scratch.meta.interval.end;
        batch.meta.sequence_packets += 1;
        // Packet-local counter evidence cannot certify this wider interval.
        batch.meta.overload = [None; 3];
        if batch.meta.sequence_packets as usize
            != saturn_bridge::puresignal::adapters::P2_BATCH_PACKETS
        {
            return Ok(());
        }
        let ingress = self.ingress.as_ref().ok_or(Fault::NoController)?;
        let replacement = ingress.take_feedback_buffer()?;
        let block = self
            .accumulated
            .replace(replacement)
            .ok_or(Fault::QueueOverrun)?;
        ingress.feedback(block)
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
                    block.acquisition_revision = self.acquisition_revision;
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
                    if let Some(c) = self.ingress.as_ref().and_then(|i| i.signal_capture.get()) {
                        c.pairs(self.scope, block);
                    }
                    #[cfg(feature = "managed-puresignal")]
                    if let Some(monitor) = &self.monitor {
                        monitor.push_scoped(block, self.origin, self.scope);
                    }
                    if let Some(ingress) = &self.ingress {
                        let replacement = ingress.take_feedback_buffer()?;
                        let complete = self.accumulated.replace(replacement).unwrap();
                        ingress.feedback(complete)?;
                    } else {
                        block.pairs = 0;
                    }
                }
            }
        }
        Ok(())
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        // Retained frame/assembly history is destroyed; only storage is reused.
        if let Some(ingress) = &self.ingress {
            if let Some(block) = self.accumulated.take() {
                ingress.recycle_feedback_buffer(block);
            }
            if let Some(block) = self.scratch.take() {
                ingress.recycle_feedback_buffer(block);
            }
        }
        #[cfg(feature = "managed-puresignal")]
        if let Some(monitor) = &self.monitor {
            monitor.invalidate();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tx_thread::feedback_owner::tests::{collector, scope};
    use crate::tx_thread::ps_ingress::{FEEDBACK_POOL_BLOCKS, FEEDBACK_QUEUE_BLOCKS};
    #[test]
    fn direct_dma_burst_keeps_original_ages_and_recovers_all_storage() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::DirectXdma, origin, ingress.clone())
            .unwrap()
            .with_revision(73);
        // Six native blocks (~32 ms), arriving in actual maximum DMA-sized
        // chunks, without any consumer scheduling between reads. The old
        // two-slot/four-box handoff failed on block three.
        let bytes = direct_frames(256 * 6);
        let mut iq = Vec::new();
        for chunk in bytes.chunks(32768) {
            stream.direct(Some(s), chunk, origin, &mut iq).unwrap();
        }
        for n in 0..6 {
            let block = rx.feedback.try_recv().unwrap();
            assert_eq!(block.pairs, 1024);
            assert_eq!(block.acquisition_revision, 73);
            assert_eq!(block.meta.observed_ns, 0);
            assert!(block.arrival_ns[..1024].iter().all(|v| *v == 0));
            assert_eq!(block.meta.interval.first, n * 1024);
            ingress.recycle_feedback_buffer(block);
        }
        assert!(rx.feedback.try_recv().is_err());
        drop(stream);
        let buffers: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert_eq!(buffers.len(), FEEDBACK_POOL_BLOCKS);
    }
    #[test]
    fn full_direct_queue_with_consumer_inflight_has_replacement_and_stays_bounded() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::DirectXdma, origin, ingress.clone()).unwrap();
        let bytes = direct_frames(256);
        let mut iq = Vec::new();
        stream.direct(Some(s), &bytes, origin, &mut iq).unwrap();
        let inflight = rx.feedback.try_recv().unwrap();
        for _ in 0..FEEDBACK_QUEUE_BLOCKS {
            stream.direct(Some(s), &bytes, origin, &mut iq).unwrap();
        }
        // Even with a consumer-held box, the replacement is available. Full
        // channel rejection recycles the rejected block, without allocating.
        assert_eq!(
            stream.direct(Some(s), &bytes, origin, &mut iq),
            Err(Fault::QueueOverrun)
        );
        ingress.recycle_feedback_buffer(inflight);
        for _ in 0..FEEDBACK_QUEUE_BLOCKS {
            ingress.recycle_feedback_buffer(rx.feedback.try_recv().unwrap());
        }
        assert!(rx.feedback.try_recv().is_err());
        drop(stream);
        let buffers: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert_eq!(buffers.len(), FEEDBACK_POOL_BLOCKS);
    }
    #[test]
    fn p2_queue_overflow_recovers_all_storage() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::P2, origin, ingress.clone()).unwrap();
        for n in 0..(FEEDBACK_QUEUE_BLOCKS * 8 + 7) {
            stream.p2(Some(s), &packet(n as u32), origin).unwrap();
        }
        assert_eq!(
            stream.p2(
                Some(s),
                &packet((FEEDBACK_QUEUE_BLOCKS * 8 + 7) as u32),
                origin
            ),
            Err(Fault::QueueOverrun)
        );
        for _ in 0..FEEDBACK_QUEUE_BLOCKS {
            ingress.recycle_feedback_buffer(rx.feedback.try_recv().unwrap());
        }
        drop(stream);
        let buffers: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert_eq!(buffers.len(), FEEDBACK_POOL_BLOCKS);
    }
    #[test]
    fn p2_batch_preserves_each_packet_age_sequence_and_pairs() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::P2, origin, ingress.clone())
            .unwrap()
            .with_revision(73);
        for n in 0..16 {
            let at = origin + std::time::Duration::from_millis(if n == 0 { 0 } else { 200 + n });
            stream.p2(Some(s), &packet(n as u32), at).unwrap();
        }
        for batch in 0..2 {
            let b = rx.feedback.try_recv().unwrap();
            assert_eq!(b.pairs, 952);
            assert_eq!(b.acquisition_revision, 73);
            assert_eq!(b.meta.sequence, Some(batch * 8));
            assert_eq!(b.meta.sequence_packets, 8);
            assert_eq!(b.meta.interval.first, batch as u64 * 952);
            assert_eq!(b.meta.interval.end, (batch as u64 + 1) * 952);
            assert_eq!(b.meta.observed_ns, if batch == 0 { 0 } else { 208_000_000 });
            for packet in 0..8 {
                let n = batch as u64 * 8 + packet as u64;
                let age = if n == 0 { 0 } else { (200 + n) * 1_000_000 };
                assert!(b.arrival_ns[packet * 119..(packet + 1) * 119]
                    .iter()
                    .all(|v| *v == age));
            }
            assert!(b.meta.overload.iter().all(Option::is_none));
            ingress.recycle_feedback_buffer(b);
        }
        drop(stream);
        let buffers: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert_eq!(buffers.len(), FEEDBACK_POOL_BLOCKS);
    }
    #[test]
    fn p2_gap_inside_partial_batch_cannot_publish_a_contiguous_block() {
        let (ingress, rx) = collector();
        let origin = Instant::now();
        let s = scope(1);
        let mut stream = Stream::new(s, Backend::P2, origin, ingress.clone()).unwrap();
        for n in 0..7 {
            stream.p2(Some(s), &packet(n), origin).unwrap();
        }
        assert!(rx.feedback.try_recv().is_err());
        assert_eq!(
            stream.p2(Some(s), &packet(8), origin),
            Err(Fault::SequenceGap)
        );
        assert!(rx.feedback.try_recv().is_err());
        drop(stream);
        let buffers: Vec<_> = (0..FEEDBACK_POOL_BLOCKS)
            .map(|_| ingress.take_feedback_buffer().unwrap())
            .collect();
        assert_eq!(buffers.len(), FEEDBACK_POOL_BLOCKS);
    }
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
        for _ in 0..FEEDBACK_POOL_BLOCKS {
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
        for n in 0..8 {
            stream.p2(Some(s), &packet(n), origin).unwrap();
        }
        let block = rx.feedback.try_recv().unwrap();
        assert_eq!((block.pairs, block.rf[0], block.tx[0]), (952, 0.125, 0.25));
        assert!(block.meta.overload.iter().all(Option::is_none));
        ingress.recycle_feedback_buffer(block);
        assert_eq!(
            stream.p2(Some(s), &packet(9), origin),
            Err(Fault::SequenceGap)
        );
        drop(stream);
        for _ in 0..FEEDBACK_POOL_BLOCKS {
            assert!(ingress.take_feedback_buffer().is_ok());
        }
    }
}
