//! Managed feedback remains on P2Session's one receive socket/owner. The scope
//! is captured before recv_from; queued reads cannot gain a newer generation.
use super::*;
use crate::tx_thread::{
    feedback_owner::{Link, Request, Scope},
    feedback_stream::Stream,
    ps_ingress::SharedIngress,
};
use saturn_bridge::puresignal::{Backend, Fault};

pub(super) struct Managed {
    pub(super) link: Link,
    pub(super) ingress: SharedIngress,
    pub(super) stream: Option<Stream>,
    completed: Request,
    model: Arc<Mutex<RadioModel>>,
    origin: Instant,
}
impl P2Session {
    pub(crate) fn attach_managed_feedback(
        &self,
        link: Link,
        ingress: SharedIngress,
        model: Arc<Mutex<RadioModel>>,
        origin: Instant,
    ) -> io::Result<()> {
        let mut slot = self.managed_feedback.lock_unpoisoned();
        if slot.is_some() {
            return Err(io::Error::other("P2 feedback owner already attached"));
        }
        *slot = Some(Managed {
            completed: link.desired(),
            link,
            ingress,
            stream: None,
            model,
            origin,
        });
        self.managed_acquisition.store(true, Ordering::Release);
        Ok(())
    }
    pub(super) fn service_managed_layout(&self, managed: &mut Managed) -> io::Result<()> {
        let desired = managed.link.desired();
        if desired == managed.completed {
            return Ok(());
        }
        if !managed.link.may_apply(desired) {
            return Ok(());
        }
        managed.stream = None; // retire decoder/history before any layout change
                               // Retire queued bridge datagrams, not just Rust decoder state. This
                               // does not prove RF acquisition age or drain packets still in transit.
        self.socket.set_nonblocking(true)?;
        let mut discard = [0; 2048];
        let drained = (|| -> io::Result<bool> {
            for _ in 0..256 {
                match self.socket.recv_from(&mut discard) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(true),
                    Err(e) => return Err(e),
                }
            }
            Ok(false)
        })();
        self.socket.set_nonblocking(false)?;
        if !drained? {
            // Continue this same request on the next acquisition turn. Keep
            // the decoder retired and do not publish preparation/restoration
            // completion until the queue is actually empty. Lifecycle stop
            // and cleanup deadlines remain in force while this yields.
            return Ok(());
        }
        let model = managed.model.lock_unpoisoned().clone();
        if model.desired.rx_ddc_index < 2 {
            return Err(io::Error::other("P2 shared feedback reserves DDC0/1"));
        }
        let mut setups = vec![DdcSetup {
            ddc_index: model.desired.rx_ddc_index,
            adc: model.desired.ddc0_adc,
            sample_rate_khz: model.desired.ddc0_sample_rate_khz,
            sample_size_bits: model.desired.ddc0_sample_size_bits,
        }];
        let mut mask = 1 << model.desired.rx_ddc_index;
        if desired.scope.is_some() {
            setups.extend([
                DdcSetup {
                    ddc_index: 0,
                    adc: 0,
                    sample_rate_khz: 192,
                    sample_size_bits: 24,
                },
                DdcSetup {
                    ddc_index: 1,
                    adc: 2,
                    sample_rate_khz: 192,
                    sample_size_bits: 24,
                },
            ]);
            mask |= 3;
        }
        // TX ADC1 attenuation is part of the same fixed route/generation.
        // Normal receive setup used 31 dB while PS was off; restore that safe
        // value after the unkey barrier. ADC2 TX attenuation stays at 31 dB.
        let attenuation = desired.scope.map_or(31, |scope| scope.attenuation_db);
        let sequence = self.duc_specific_sequence.fetch_add(1, Ordering::Relaxed);
        let duc = build_duc_specific_packet(sequence, 31, attenuation);
        if self
            .socket
            .send_to(&duc, self.target_addr(self.config.port_map.duc_specific))?
            != duc.len()
        {
            return Err(io::ErrorKind::WriteZero.into());
        }
        // Complete UDP command submission only, not FPGA acknowledgment or RF-off.
        let packet = build_ddc_specific_packet(mask, &setups, desired.scope.is_some());
        // This acquisition owner can change ONLY DDC layout, not RF/HP or IQ.
        // The matching Link completion remains a mandatory key/restore barrier.
        let written = self
            .socket
            .send_to(&packet, self.target_addr(self.config.port_map.ddc_specific))?;
        if written != packet.len() {
            return Err(io::ErrorKind::WriteZero.into());
        }
        if let Some(scope) = desired.scope {
            managed.stream = Some(
                Stream::new(scope, Backend::P2, managed.origin, managed.ingress.clone())
                    .map_err(|f| io::Error::other(format!("P2 feedback storage: {f:?}")))?
                    .with_revision(desired.serial),
            );
        }
        managed.completed = desired;
        managed.link.complete(desired);
        Ok(())
    }
}
impl Managed {
    pub(super) fn token(&self) -> Option<Scope> {
        self.stream.as_ref().map(Stream::scope)
    }
    pub(super) fn fault(&self, fault: Fault) {
        self.ingress.stop(fault);
        self.link.fail();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) struct Fixture {
        pub(crate) session: Arc<P2Session>,
        pub(crate) feedback: UdpSocket,
        pub(crate) layout: UdpSocket,
        pub(crate) hp: UdpSocket,
        pub(crate) status: UdpSocket,
        pub(crate) duc: UdpSocket,
        pub(crate) destination: SocketAddr,
    }
    impl Fixture {
        pub(crate) fn new() -> Self {
            let feedback = UdpSocket::bind("127.0.0.1:0").unwrap();
            let layout = UdpSocket::bind("127.0.0.1:0").unwrap();
            let hp = UdpSocket::bind("127.0.0.1:0").unwrap();
            let status = UdpSocket::bind("127.0.0.1:0").unwrap();
            let duc = UdpSocket::bind("127.0.0.1:0").unwrap();
            for socket in [&layout, &hp, &duc] {
                socket.set_nonblocking(true).unwrap();
            }
            let mut config = BridgeConfig {
                client_bind_addr: "127.0.0.1:0".parse().unwrap(),
                radio_command_addr: hp.local_addr().unwrap(),
                ..BridgeConfig::default()
            };
            config.port_map.ddc_iq_base = feedback.local_addr().unwrap().port();
            config.port_map.ddc_specific = layout.local_addr().unwrap().port();
            config.port_map.high_priority_to_sdr = hp.local_addr().unwrap().port();
            config.port_map.high_priority_from_sdr = status.local_addr().unwrap().port();
            config.port_map.duc_iq = duc.local_addr().unwrap().port();
            config.port_map.duc_specific = duc.local_addr().unwrap().port();
            let session = Arc::new(P2Session::bind_offline_fenced(config).unwrap());
            let destination = session.socket.local_addr().unwrap();
            Self {
                session,
                feedback,
                layout,
                hp,
                status,
                duc,
                destination,
            }
        }
        pub(crate) fn service(&self) -> io::Result<()> {
            let _receive = self.session.receive_lock.lock_unpoisoned();
            let mut slot = self.session.managed_feedback.lock_unpoisoned();
            self.session.service_managed_layout(slot.as_mut().unwrap())
        }
        pub(crate) fn receive(&self, bytes: &[u8]) {
            self.feedback.send_to(bytes, self.destination).unwrap();
            assert!(matches!(
                self.session.recv_event().unwrap(),
                Some(P2Event::ManagedFeedback)
            ));
        }
        pub(crate) fn drain_output(&self) {
            self.drain_output_count();
        }
        pub(crate) fn drain_output_count(&self) -> u64 {
            let mut bytes = [0; 2048];
            while let Ok(n) = self.hp.recv(&mut bytes) {
                if n == 1444 && bytes[4] & 2 != 0 {
                    assert_eq!(
                        &bytes[1434..1436],
                        &[0x18, 0x00],
                        "actual shared key/keepalive must select external RX BYPASS"
                    );
                }
            }
            let mut packets = 0;
            while let Ok(n) = self.duc.recv(&mut bytes) {
                if n == 1444 {
                    packets += 1;
                }
            }
            packets
        }
    }
    #[test]
    fn p2_queue_retirement_yields_without_premature_completion() {
        use crate::tx_thread::{
            feedback_owner::tests::{collector, scope},
            feedback_stream::tests::packet,
        };
        let fixture = Fixture::new();
        let link = Link::new();
        let (ingress, _) = collector();
        let model = Arc::new(Mutex::new(crate::tx_owner_stop::tests::model()));
        fixture
            .session
            .attach_managed_feedback(link.clone(), ingress, model, Instant::now())
            .unwrap();
        let bytes: libc::c_int = 1024 * 1024;
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    fixture.session.socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    (&bytes as *const libc::c_int).cast(),
                    std::mem::size_of_val(&bytes) as _,
                )
            },
            0
        );
        let first = scope(1);
        link.request(Some(first)).unwrap();
        for n in 0..300 {
            fixture
                .feedback
                .send_to(&packet(n), fixture.destination)
                .unwrap();
        }
        fixture.service().unwrap();
        assert!(!link.prepared(first.output).unwrap());
        fixture.service().unwrap();
        assert!(link.prepared(first.output).unwrap());
        let restore = link.request(None).unwrap();
        link.permit_restoration(restore);
        for n in 300..600 {
            fixture
                .feedback
                .send_to(&packet(n), fixture.destination)
                .unwrap();
        }
        fixture.service().unwrap();
        assert!(!link.restored().unwrap());
        fixture.service().unwrap();
        assert!(link.restored().unwrap());
    }
    #[test]
    fn p2_existing_socket_layout_queue_retirement_demux_and_restore() {
        use crate::tx_thread::{
            feedback_owner::tests::{collector, scope},
            feedback_stream::tests::packet,
        };
        let fixture = Fixture::new();
        let link = Link::new();
        let (ingress, deliveries) = collector();
        let model = Arc::new(Mutex::new(crate::tx_owner_stop::tests::model()));
        fixture
            .session
            .attach_managed_feedback(link.clone(), ingress.clone(), model, Instant::now())
            .unwrap();
        let first = scope(1);
        link.request(Some(first)).unwrap();
        // Old bridge datagrams are retired before changing layout.
        fixture
            .feedback
            .send_to(&packet(90), fixture.destination)
            .unwrap();
        fixture.service().unwrap();
        assert!(link.prepared(first.output).unwrap());
        let mut layout = [0; 1444];
        assert_eq!(fixture.layout.recv(&mut layout).unwrap(), 1444);
        assert_eq!(u16::from_le_bytes(layout[7..9].try_into().unwrap()), 7);
        assert_eq!((layout[17], layout[23], layout[1363]), (0, 2, 2));
        let mut duc = [0; 60];
        assert_eq!(fixture.duc.recv(&mut duc).unwrap(), 60);
        assert_eq!((duc[58], duc[59]), (31, 9));
        for n in 0..8 {
            fixture.receive(&packet(n));
        }
        let block = deliveries.feedback.try_recv().unwrap();
        assert_eq!(block.meta.fence, first.fence);
        assert_eq!(block.pairs, 952);
        assert_eq!(block.meta.sequence_packets, 8);
        assert!(block.meta.overload.iter().all(Option::is_none));
        ingress.recycle_feedback_buffer(block);
        fixture.receive(&packet(9));
        assert!(deliveries.feedback.try_recv().is_err());
        assert!(fixture
            .session
            .managed_feedback
            .lock_unpoisoned()
            .as_ref()
            .unwrap()
            .stream
            .is_none());
        let restore = link.request(None).unwrap();
        fixture.service().unwrap();
        assert!(fixture.layout.recv(&mut layout).is_err()); // backend-stop barrier
        link.permit_restoration(restore);
        fixture.service().unwrap();
        assert!(link.restored().unwrap());
        assert_eq!(fixture.layout.recv(&mut layout).unwrap(), 1444);
        assert_eq!(u16::from_le_bytes(layout[7..9].try_into().unwrap()), 4);
        assert_eq!(layout[1363], 0);
        assert_eq!(fixture.duc.recv(&mut duc).unwrap(), 60);
        assert_eq!((duc[58], duc[59]), (31, 31));
        link.request(Some(scope(2))).unwrap();
        fixture.service().unwrap();
        for n in 0..8 {
            fixture.receive(&packet(n));
        }
        let block = deliveries.feedback.try_recv().unwrap();
        assert_eq!(block.meta.interval.first, 0);
        assert_eq!(block.meta.fence, scope(2).fence);
        ingress.recycle_feedback_buffer(block);
    }
    #[test]
    fn real_shared_bind_bootstraps_receive_before_closing_and_never_allows_legacy_tx() {
        let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
        sink.set_nonblocking(true).unwrap();
        let port = sink.local_addr().unwrap().port();
        let mut config = BridgeConfig {
            client_bind_addr: "127.0.0.1:0".parse().unwrap(),
            radio_command_addr: sink.local_addr().unwrap(),
            enable_discovery: false,
            ..BridgeConfig::default()
        };
        config.port_map.ddc_specific = port;
        config.port_map.duc_specific = port;
        config.port_map.high_priority_to_sdr = port;
        config.port_map.duc_iq = port;
        let mut model = crate::tx_owner_stop::tests::model();
        model.desired.tx_enabled = false;
        let shared = Arc::new(Mutex::new(model.clone()));
        let session = P2Session::bind_shared(config.clone(), &shared).unwrap();
        let mut packet = [0; 2048];
        let mut setup_count = 0;
        while sink.recv(&mut packet).is_ok() {
            setup_count += 1;
        }
        assert_eq!(setup_count, 3);
        assert!(session.offline_output_fence().unwrap().is_closed());
        assert!(session.send_high_priority(&model).is_err());
        assert!(session.send_duc_iq(&[0.2; 480]).is_err());
        session.shared_receive_keepalive(&model).unwrap();
        assert_eq!(sink.recv(&mut packet).unwrap(), 1444);
        assert_eq!(packet[4] & 2, 0);
        assert_eq!(packet[345], 0);
        session.shared_stop_run(&model).unwrap();
        assert_eq!(sink.recv(&mut packet).unwrap(), 1444);
        assert_eq!(packet[4] & 3, 0);
        shared.lock_unpoisoned().desired.tx_enabled = true;
        assert!(P2Session::bind_shared(config, &shared).is_err());
    }
}
