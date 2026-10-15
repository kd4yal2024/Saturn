//! Cooperative layout changes by the existing RX owner, after its sole reader
//! acknowledges a pause. Epochs are captured before DMA reads, never at dequeue.
use super::*;
use crate::{
    sync_ext::MutexExt,
    tx_thread::{
        feedback_owner::{Link, Request, Scope},
        feedback_stream::Stream,
        ps_ingress::SharedIngress,
    },
};
use saturn_bridge::puresignal::{Backend, Fault};
use std::sync::atomic::AtomicU64;

pub(crate) struct ReaderGate {
    requested_pause: AtomicU64,
    paused: AtomicU64,
    scope: Mutex<Option<Scope>>,
}
impl ReaderGate {
    pub(crate) fn new() -> Self {
        Self {
            requested_pause: AtomicU64::new(0),
            paused: AtomicU64::new(0),
            scope: Mutex::new(None),
        }
    }
    pub(crate) fn service_pause(&self) -> bool {
        let serial = self.requested_pause.load(Ordering::SeqCst);
        if serial == 0 {
            return false;
        }
        self.paused.store(serial, Ordering::SeqCst);
        true
    }
    pub(crate) fn scope(&self) -> Option<Scope> {
        *self.scope.lock_unpoisoned()
    }
    fn pause(&self, serial: u64) -> bool {
        self.requested_pause.store(serial, Ordering::SeqCst);
        self.paused.load(Ordering::SeqCst) == serial
    }
    fn resume(&self, scope: Option<Scope>) {
        *self.scope.lock_unpoisoned() = scope;
        self.requested_pause.store(0, Ordering::SeqCst);
    }
}
#[derive(Clone, Copy)]
pub(super) struct Saved {
    rate: u32,
    input: u32,
    phase0: u32,
    phase1: u32,
    adc_control: u32,
    rx_filter: u32,
}
pub(crate) struct ManagedFeedback {
    link: Link,
    ingress: SharedIngress,
    origin: Instant,
    completed: Request,
    pub(super) saved: Option<Saved>,
    pub(super) stream: Option<Stream>,
    pub(super) discarded_iq: Vec<f32>,
    #[cfg(test)]
    before_readback: Option<Box<dyn FnOnce() + Send>>,
}
impl ManagedFeedback {
    pub(super) fn accepts(&self, scope: Scope) -> bool {
        self.link.desired().scope == Some(scope)
    }
    pub(super) fn data_fault(&self, scope: Scope, fault: Fault) {
        self.ingress.stop_feedback(scope.output, fault);
    }
}
impl OperationalRxSession {
    pub(crate) fn attach_managed_feedback(
        &mut self,
        link: Link,
        ingress: SharedIngress,
        origin: Instant,
    ) -> Result<(), XdmaError> {
        if self.identity.firmware_major != 1
            || self.identity.firmware_minor != 31
            || self.identity.user_version != 0x53460004
            || self.identity.pcb_version != 2
        {
            return Err(XdmaError::Incompatible(
                "managed Direct XDMA candidate is pinned to PCB2 RXC1 1.31.002 / 0x53460004".into(),
            ));
        }
        if self.managed_feedback.is_some() {
            return Err(XdmaError::Incompatible(
                "feedback owner already attached".into(),
            ));
        }
        self.managed_feedback = Some(ManagedFeedback {
            completed: link.desired(),
            link,
            ingress,
            origin,
            saved: None,
            stream: None,
            discarded_iq: Vec::with_capacity(DMA_MAX_READ_BYTES / 8 * 2),
            #[cfg(test)]
            before_readback: None,
        });
        Ok(())
    }
    /// Each call returns promptly while an in-flight read finishes. The TX
    /// owner continues servicing release and never waits on this reader.
    pub(crate) fn service_managed_feedback(&mut self) -> Result<(), XdmaError> {
        let Some(mut managed) = self.managed_feedback.take() else {
            return Ok(());
        };
        let desired = managed.link.desired();
        if desired == managed.completed
            || !managed.link.may_apply(desired)
            || !self.reader_gate.pause(desired.serial)
        {
            self.managed_feedback = Some(managed);
            return Ok(());
        }
        // Acknowledged pause means no read is in flight or can start. Retire
        // already queued buffers and partial frames before changing the layout.
        self.pool.clear_ready();
        self.last_dma_sequence = None;
        managed.stream = None;
        let operation = (|| -> Result<(), XdmaError> {
            let regs = self.registers.lock_unpoisoned();
            let saved = match managed.saved {
                Some(saved) => saved,
                None => {
                    let saved = Saved {
                        rate: regs.read_register(DDC_RATE_REGISTER)?,
                        input: regs.read_register(DDC_INPUT_SELECT_REGISTER)?,
                        phase0: regs.read_register(0)?,
                        phase1: regs.read_register(4)?,
                        adc_control: regs.read_register(ADC_ATTENUATION_REGISTER)?,
                        rx_filter: regs.read_register(ALEX_RX_FILTER_REGISTER)?,
                    };
                    if desired.scope.is_some() && saved.rate != direct_ddc_rate_word() {
                        return Err(XdmaError::Incompatible(
                            "managed feedback requires exclusive DDC6 receive layout".into(),
                        ));
                    }
                    managed.saved = Some(saved);
                    saved
                }
            };
            regs.write_register(
                DDC_INPUT_SELECT_REGISTER,
                saved.input & !DDC_STREAM_ENABLE_BIT,
            )?;
            // Preserve the existing RX-owner disable/reset settling sequence.
            // This owner is separate from TX release/stop; no DMA wait occurs.
            thread::sleep(Duration::from_millis(1));
            let reset = regs.read_register(FIFO_RESET_REGISTER)?;
            regs.write_register(FIFO_RESET_REGISTER, reset & !DDC_FIFO_RESET_BIT)?;
            regs.write_register(FIFO_RESET_REGISTER, reset | DDC_FIFO_RESET_BIT)?;
            let (rate, input, phase0, phase1) = if let Some(scope) = desired.scope {
                (
                    0x10_001f,
                    (saved.input & !0xf) | (3 << 2),
                    frequency_to_phase_word(scope.frequency_hz),
                    frequency_to_phase_word(scope.frequency_hz),
                )
            } else {
                (saved.rate, saved.input, saved.phase0, saved.phase1)
            };
            regs.write_register(0, phase0)?;
            regs.write_register(4, phase1)?;
            regs.write_register(DDC_RATE_REGISTER, rate)?;
            let adc_control = desired.scope.map_or(saved.adc_control, |scope| {
                (saved.adc_control & !(31 << 5)) | (u32::from(scope.attenuation_db) << 5)
            });
            regs.write_register(ADC_ATTENUATION_REGISTER, adc_control)?;
            // This host path supports external feedback on RX BYPASS only.
            // AXILite_Alex_SPI.v / saturnregisters.c define RX1 bit 11 as
            // PS sample select and bit 12 as BPF bypass. Preserve RX2, but
            // remove RX1 band filters, LNA, EXT1 and transverter selection.
            // Selecting DDC0's ADC alone does NOT select this physical input.
            let rx_filter = desired.scope.map_or(saved.rx_filter, |_| {
                (saved.rx_filter & 0xffff_0000) | 0x1800
            });
            regs.write_register(ALEX_RX_FILTER_REGISTER, rx_filter)?;
            regs.write_register(DDC_INPUT_SELECT_REGISTER, input)?;
            #[cfg(test)]
            if let Some(hook) = managed.before_readback.take() {
                hook();
            }
            if regs.read_register(DDC_RATE_REGISTER)? != rate
                || regs.read_register(DDC_INPUT_SELECT_REGISTER)? != input
                || regs.read_register(0)? != phase0
                || regs.read_register(4)? != phase1
                || regs.read_register(ADC_ATTENUATION_REGISTER)? != adc_control
                || regs.read_register(ALEX_RX_FILTER_REGISTER)? != rx_filter
            {
                return Err(XdmaError::Incompatible(
                    "managed feedback layout readback mismatch".into(),
                ));
            }
            drop(regs);
            if let Some(scope) = desired.scope {
                managed.stream = Some(
                    Stream::new(
                        scope,
                        Backend::DirectXdma,
                        managed.origin,
                        managed.ingress.clone(),
                    )
                    .map_err(|f| XdmaError::Incompatible(format!("feedback buffers: {f:?}")))?,
                );
            } else {
                managed.saved = None;
                let stats = self.parser.stats.clone();
                self.parser = DdcStreamParser::new(direct_ddc_rate_word());
                self.parser.stats = stats;
            }
            Ok(())
        })();
        if let Err(error) = operation {
            // Best-effort restoration is not an acknowledgment. Keep admission
            // failed and the reader paused if any layout operation was uncertain.
            if let Some(saved) = managed.saved {
                let regs = self.registers.lock_unpoisoned();
                for (offset, value) in [
                    (0, saved.phase0),
                    (4, saved.phase1),
                    (ADC_ATTENUATION_REGISTER, saved.adc_control),
                    (ALEX_RX_FILTER_REGISTER, saved.rx_filter),
                    (DDC_RATE_REGISTER, saved.rate),
                    (DDC_INPUT_SELECT_REGISTER, saved.input),
                ] {
                    let _ = regs.write_register(offset, value);
                }
            }
            managed.ingress.stop(Fault::DmaFraming);
            managed.link.fail();
            self.managed_feedback = Some(managed);
            return Err(error);
        }
        managed.completed = desired;
        // Cancellation racing application cannot mark a newer request ready.
        if managed.link.complete(desired) {
            self.reader_gate.resume(desired.scope);
        }
        self.managed_feedback = Some(managed);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// Same operational consumer/register transaction, manually instrumented
    /// read-completion boundary. No environment, /dev access or RT thread.
    pub(crate) fn session(path: &Path) -> OperationalRxSession {
        assert!(path.is_file());
        assert!(!path.starts_with("/dev"));
        let device = XdmaRegisterDevice::open_peripheral(path).unwrap();
        let identity = device.identity().clone();
        OperationalRxSession {
            fifo_policy: FifoStatusPolicy::for_identity(&identity),
            rx_counter_v31: rx_counter_v31::Owner::for_build_id(identity.user_version),
            identity,
            registers: Arc::new(Mutex::new(device)),
            parser: DdcStreamParser::new(direct_ddc_rate_word()),
            pool: Arc::new(OperationalRxBufferPool::new(3, false).unwrap()),
            reader_stats: Arc::new(Mutex::new(RxCaptureStats::default())),
            reader_stop: Arc::new(AtomicBool::new(false)),
            reader: None,
            reader_result: None,
            fifo_v29_telemetry: FpgaFifoV29Telemetry::default(),
            adc_v30_telemetry: FpgaAdcV30Telemetry::default(),
            per_ddc_almost_full_telemetry: PerDdcAlmostFullTelemetry::default(),
            fifo_monitor_threshold_words: 0,
            last_dma_sequence: None,
            frequency_hz: 7_200_000,
            rx_antenna: 1,
            rx_attenuation_db: 0,
            stopped: true,
            reader_gate: Arc::new(ReaderGate::new()),
            managed_feedback: None,
        }
    }
    pub(crate) fn service(session: &mut OperationalRxSession) -> Result<(), XdmaError> {
        session.service_managed_feedback()?; // request pause
        session.reader_gate.service_pause(); // previously in-flight read is finished
        session.service_managed_feedback()
    }
    pub(crate) fn publish(
        session: &OperationalRxSession,
        bytes: &[u8],
        sequence: u64,
        scope: Option<Scope>,
        observed: Instant,
        loss: bool,
    ) {
        let (mut buffer, dropped) = session.pool.acquire_for_reader().unwrap();
        assert_eq!(dropped, 0);
        buffer.as_mut_slice(bytes.len()).copy_from_slice(bytes);
        session.pool.publish(FilledRxBuffer {
            buffer,
            len: bytes.len(),
            sequence,
            feedback_scope: scope,
            observed_at: observed,
            completed_at: Instant::now(),
            host_loss_before: loss,
        });
    }
    /// Emulate only the sole reader's pause acknowledgment and DMA completion.
    /// The actual runtime calls read_iq; this helper NEVER services layout,
    /// consumes samples, pumps lifecycle, or acknowledges restoration.
    pub(crate) fn reader_turn(
        session: &OperationalRxSession,
        normal: &[u8],
        paired: &[u8],
        sequence: u64,
    ) {
        if session.reader_gate.service_pause() {
            return;
        }
        let observed = Instant::now();
        let scope = session.reader_gate.scope();
        publish(
            session,
            if scope.is_some() { paired } else { normal },
            sequence,
            scope,
            observed,
            false,
        );
    }
    pub(crate) fn corrupt_next_layout_readback(session: &mut OperationalRxSession, path: &Path) {
        assert!(path.is_file() && !path.starts_with("/dev"));
        let file = OpenOptions::new().write(true).open(path).unwrap();
        session.managed_feedback.as_mut().unwrap().before_readback = Some(Box::new(move || {
            file.write_all_at(&0u32.to_le_bytes(), DDC_RATE_REGISTER)
                .unwrap();
        }));
    }
    pub(crate) fn consume(
        session: &mut OperationalRxSession,
        bytes: &[u8],
        sequence: &mut u64,
        scope: Option<Scope>,
        observed: Instant,
    ) -> usize {
        let mut iq = Vec::new();
        let mut count = 0;
        for chunk in bytes.chunks(DMA_MAX_READ_BYTES) {
            publish(session, chunk, *sequence, scope, observed, false);
            *sequence += 1;
            let result = session.read_iq(&mut iq).unwrap();
            count += result.sample_pairs as usize;
        }
        count
    }
    #[test]
    fn a_previous_pause_ack_cannot_authorize_a_new_layout_write() {
        let gate = ReaderGate::new();
        assert!(!gate.pause(1));
        assert!(gate.service_pause());
        assert!(gate.pause(1));
        gate.resume(None);
        assert!(!gate.pause(2));
        assert!(gate.service_pause());
        assert!(gate.pause(2));
    }
    #[test]
    fn operational_layout_pause_demux_retirement_and_restore_are_generation_fenced() {
        use crate::{
            tx_thread::{
                feedback_owner::tests::{collector, scope},
                feedback_stream::tests::direct_frames,
            },
            xdma_tx_radio::offline_fixture::DirectFixture,
        };
        let files = DirectFixture::new();
        let initial = (DDC_STREAM_ENABLE_BIT | 0x2400, 42, 43);
        files.write(DDC_RATE_REGISTER, direct_ddc_rate_word());
        files.write(DDC_INPUT_SELECT_REGISTER, initial.0);
        files.write(0, initial.1);
        files.write(4, initial.2);
        files.write(ADC_ATTENUATION_REGISTER, 0xabcde);
        files.write(ALEX_RX_FILTER_REGISTER, 0x1234_4218);
        let mut rx = session(&files.register_path());
        let link = Link::new();
        let (ingress, deliveries) = collector();
        let origin = Instant::now();
        rx.attach_managed_feedback(link.clone(), ingress.clone(), origin)
            .unwrap();
        let old = scope(1);
        let request = link.request(Some(old)).unwrap();
        // A queued old read cannot survive the acknowledged pause.
        publish(&rx, &direct_frames(1), 0, None, origin, false);
        rx.service_managed_feedback().unwrap();
        assert_eq!(files.read(DDC_RATE_REGISTER), direct_ddc_rate_word());
        assert_eq!(rx.pool.ready_len(), 1);
        assert!(!link.prepared(old.output).unwrap());
        assert!(rx.reader_gate.service_pause());
        rx.service_managed_feedback().unwrap();
        assert_eq!(rx.pool.ready_len(), 0);
        assert!(link.prepared(old.output).unwrap());
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10001f);
        assert_eq!(
            files.read(DDC_INPUT_SELECT_REGISTER),
            (initial.0 & !15) | 12
        );
        assert_eq!(files.read(0), frequency_to_phase_word(old.frequency_hz));
        assert_eq!(files.read(4), files.read(0));
        assert_eq!(
            files.read(ADC_ATTENUATION_REGISTER),
            (0xabcde & !(31 << 5)) | (9 << 5)
        );
        assert_eq!(files.read(ALEX_RX_FILTER_REGISTER), 0x1234_1800);
        // Partial data cannot refresh its age when the rest of a DMA read arrives.
        let bytes = direct_frames(256);
        let mut iq = Vec::new();
        publish(&rx, &bytes[..8], 7, Some(old), origin, false);
        rx.read_iq(&mut iq).unwrap();
        let mut sequence = 8;
        assert_eq!(
            consume(
                &mut rx,
                &bytes[8..],
                &mut sequence,
                Some(old),
                origin + Duration::from_millis(300)
            ),
            2048
        );
        let block = deliveries.feedback.try_recv().unwrap();
        assert_eq!(block.meta.observed_ns, 0);
        assert_eq!(block.meta.fence, old.fence);
        assert_eq!(block.pairs, 1024);
        ingress.recycle_feedback_buffer(block);
        // Partial framing and a queued tail are both retired by restoration.
        publish(&rx, &bytes[..8], sequence, Some(old), origin, false);
        sequence += 1;
        rx.read_iq(&mut iq).unwrap();
        publish(&rx, &bytes[8..136], sequence, Some(old), origin, false);
        let restore = link.request(None).unwrap();
        service(&mut rx).unwrap();
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10001f);
        assert!(!link.restored().unwrap()); // no restoration before backend stop evidence
        link.permit_restoration(restore);
        service(&mut rx).unwrap();
        assert!(link.restored().unwrap());
        assert!(!link.complete(request));
        assert_eq!(files.read(DDC_RATE_REGISTER), direct_ddc_rate_word());
        assert_eq!(
            (
                files.read(DDC_INPUT_SELECT_REGISTER),
                files.read(0),
                files.read(4)
            ),
            initial
        );
        assert_eq!(files.read(ADC_ATTENUATION_REGISTER), 0xabcde);
        assert_eq!(files.read(ALEX_RX_FILTER_REGISTER), 0x1234_4218);
        // A fresh scope accepts no old scope buffered bytes or partial state.
        let fresh = scope(2);
        link.request(Some(fresh)).unwrap();
        service(&mut rx).unwrap();
        sequence = 20;
        assert_eq!(
            consume(
                &mut rx,
                &direct_frames(256),
                &mut sequence,
                Some(old),
                origin
            ),
            0
        );
        assert!(deliveries.feedback.try_recv().is_err());
        assert_eq!(
            consume(
                &mut rx,
                &direct_frames(256),
                &mut sequence,
                Some(fresh),
                origin
            ),
            2048
        );
        let block = deliveries.feedback.try_recv().unwrap();
        assert_eq!(block.meta.interval.first, 0);
        assert_eq!(block.meta.fence, fresh.fence);
        ingress.recycle_feedback_buffer(block);
    }
    #[test]
    fn operational_layout_io_failure_never_acknowledges_and_stays_paused() {
        use crate::{
            tx_thread::feedback_owner::tests::{collector, scope},
            xdma_tx_radio::offline_fixture::DirectFixture,
        };
        let files = DirectFixture::new();
        files.write(DDC_RATE_REGISTER, direct_ddc_rate_word());
        let mut rx = session(&files.register_path());
        let link = Link::new();
        let (ingress, _) = collector();
        rx.attach_managed_feedback(link.clone(), ingress, Instant::now())
            .unwrap();
        link.request(Some(scope(1))).unwrap();
        rx.service_managed_feedback().unwrap();
        rx.reader_gate.service_pause();
        files.damage_register_file();
        assert!(rx.service_managed_feedback().is_err());
        assert!(link.prepared(scope(1).output).is_err());
        assert!(rx.reader_gate.service_pause());
        assert!(link.restored().is_err());
    }
    #[test]
    fn external_bypass_readback_failure_blocks_key_and_restores_original_route() {
        use crate::{
            tx_thread::feedback_owner::tests::{collector, scope},
            xdma_tx_radio::offline_fixture::DirectFixture,
        };
        let files = DirectFixture::new();
        files.write(DDC_RATE_REGISTER, direct_ddc_rate_word());
        files.write(DDC_INPUT_SELECT_REGISTER, DDC_STREAM_ENABLE_BIT);
        files.write(ALEX_RX_FILTER_REGISTER, 0x1234_4218);
        let mut rx = session(&files.register_path());
        let link = Link::new();
        let (ingress, _) = collector();
        rx.attach_managed_feedback(link.clone(), ingress, Instant::now())
            .unwrap();
        let file = OpenOptions::new()
            .write(true)
            .open(files.register_path())
            .unwrap();
        rx.managed_feedback.as_mut().unwrap().before_readback = Some(Box::new(move || {
            file.write_all_at(&0x1234_1000u32.to_le_bytes(), ALEX_RX_FILTER_REGISTER)
                .unwrap();
        }));
        link.request(Some(scope(1))).unwrap();
        assert!(service(&mut rx).is_err());
        assert!(link.prepared(scope(1).output).is_err());
        assert!(link.restored().is_err());
        assert!(rx.reader_gate.service_pause());
        assert_eq!(files.read(ALEX_RX_FILTER_REGISTER), 0x1234_4218);
        assert_eq!(files.read(DDC_RATE_REGISTER), direct_ddc_rate_word());
    }

    #[test]
    fn operational_readback_mismatch_best_effort_restore_is_not_completion() {
        use crate::{
            tx_thread::feedback_owner::tests::{collector, scope},
            xdma_tx_radio::offline_fixture::DirectFixture,
        };
        let files = DirectFixture::new();
        files.write(DDC_RATE_REGISTER, direct_ddc_rate_word());
        files.write(DDC_INPUT_SELECT_REGISTER, DDC_STREAM_ENABLE_BIT);
        files.write(0, 42);
        files.write(4, 43);
        let mut rx = session(&files.register_path());
        let link = Link::new();
        let (ingress, _) = collector();
        rx.attach_managed_feedback(link.clone(), ingress, Instant::now())
            .unwrap();
        let file = OpenOptions::new()
            .write(true)
            .open(files.register_path())
            .unwrap();
        rx.managed_feedback.as_mut().unwrap().before_readback = Some(Box::new(move || {
            file.write_all_at(&0u32.to_le_bytes(), DDC_RATE_REGISTER)
                .unwrap();
        }));
        link.request(Some(scope(1))).unwrap();
        assert!(service(&mut rx).is_err());
        assert!(link.prepared(scope(1).output).is_err());
        assert!(link.restored().is_err());
        assert!(rx.reader_gate.service_pause());
        assert_eq!(files.read(DDC_RATE_REGISTER), direct_ddc_rate_word());
        assert_eq!((files.read(0), files.read(4)), (42, 43));
    }
    #[test]
    fn release_during_layout_write_does_not_complete_stale_prepare_or_restore_before_stop() {
        use crate::{
            tx_thread::feedback_owner::tests::{collector, scope},
            xdma_tx_radio::offline_fixture::DirectFixture,
        };
        let files = DirectFixture::new();
        files.write(DDC_RATE_REGISTER, direct_ddc_rate_word());
        files.write(DDC_INPUT_SELECT_REGISTER, DDC_STREAM_ENABLE_BIT);
        let mut rx = session(&files.register_path());
        let link = Link::new();
        let (ingress, _) = collector();
        rx.attach_managed_feedback(link.clone(), ingress, Instant::now())
            .unwrap();
        let cancelled = link.clone();
        rx.managed_feedback.as_mut().unwrap().before_readback = Some(Box::new(move || {
            cancelled.request(None).unwrap();
        }));
        let request = link.request(Some(scope(1))).unwrap();
        service(&mut rx).unwrap();
        assert!(!link.complete(request));
        assert!(!link.prepared(scope(1).output).unwrap());
        assert!(rx.reader_gate.service_pause());
        service(&mut rx).unwrap();
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10001f);
        link.permit_restoration(link.desired());
        service(&mut rx).unwrap();
        assert!(link.restored().unwrap());
        assert_eq!(files.read(DDC_RATE_REGISTER), direct_ddc_rate_word());
    }
}
