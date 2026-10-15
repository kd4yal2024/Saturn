//! Actual loopback WebSocket handler, no listeners/services beyond this fixture.
use super::*;
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use tungstenite::{Message, WebSocket};
pub(crate) struct SocketFixture {
    listener: TcpListener,
    clients: ClientRegistry,
    operator: Arc<AtomicU64>,
    control: Arc<Mutex<Option<Instant>>>,
    sender: TciCommandMailboxSender,
    pub(crate) legacy: TciCommandMailboxReceiver,
}
pub(crate) struct SocketClient {
    socket: WebSocket<TcpStream>,
    worker: Option<JoinHandle<()>>,
    pub(crate) transactions: Vec<String>,
    pub(crate) media_token: Option<[u8; 16]>,
    pub(crate) diagnostics: Vec<String>,
}
impl SocketFixture {
    /// Real external browser ingress, test-only and loopback-only. No service,
    /// hardware discovery, or installed startup route can call this fixture.
    pub(crate) fn browser_listener(
        &self,
        shared: crate::tx_thread::ps_ingress::SharedIngress,
        model: Arc<Mutex<RadioModel>>,
    ) -> BrowserListener {
        self.browser_listener_profile(shared, model, false)
    }
    pub(crate) fn browser_ui_listener(
        &self,
        shared: crate::tx_thread::ps_ingress::SharedIngress,
        model: Arc<Mutex<RadioModel>>,
    ) -> BrowserListener {
        self.browser_listener_profile(shared, model, true)
    }
    fn browser_listener_profile(
        &self,
        shared: crate::tx_thread::ps_ingress::SharedIngress,
        model: Arc<Mutex<RadioModel>>,
        fixture_tx_enabled: bool,
    ) -> BrowserListener {
        let listener = self.listener.try_clone().unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let clients = self.clients.clone();
        let operator = self.operator.clone();
        let control = self.control.clone();
        let commands = self.sender.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let streams = Arc::new(Mutex::new(Vec::<TcpStream>::new()));
        let worker_streams = streams.clone();
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            let mut id = 0;
            while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                let (stream, peer) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("browser fixture accept {e}"),
                };
                assert!(peer.ip().is_loopback());
                id += 1;
                worker_streams
                    .lock_unpoisoned()
                    .push(stream.try_clone().unwrap());
                let clients = clients.clone();
                let operator = operator.clone();
                let control = control.clone();
                let commands = commands.clone();
                let shared = shared.clone();
                let model = model.clone();
                workers.push(thread::spawn(move || {
                    stream.set_nonblocking(true).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    let mut request = [0; 8192];
                    loop {
                        if stream
                            .peek(&mut request)
                            .is_ok_and(|n| request[..n].windows(4).any(|w| w == b"\r\n\r\n"))
                        {
                            break;
                        }
                        if Instant::now() >= deadline {
                            return;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    handle_client(
                        stream,
                        peer,
                        id,
                        &commands,
                        &clients,
                        &operator,
                        &control,
                        &model,
                        &Arc::new(AtomicU64::new(0)),
                        &Arc::new(FullRateIqTransportStats::default()),
                        fixture_tx_enabled, // test-only capability; outputs remain loopback/regular files
                        TxCodecRuntimeFlags {
                            opus_decode_enabled: false,
                        },
                        (true, 0),
                        &DisplayTransport::default(),
                        &Arc::new(Mutex::new(Some(shared))),
                    );
                }));
            }
            for stream in worker_streams.lock_unpoisoned().drain(..) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        BrowserListener {
            url,
            stop,
            streams,
            worker: Some(worker),
        }
    }
    pub(crate) fn new() -> Self {
        let (sender, legacy) = tci_command_mailbox();
        Self {
            listener: TcpListener::bind("127.0.0.1:0").unwrap(),
            clients: Arc::new(Mutex::new(BTreeMap::new())),
            operator: Arc::new(AtomicU64::new(0)),
            control: Arc::new(Mutex::new(None)),
            sender,
            legacy,
        }
    }
    pub(crate) fn issuer(&self) -> ps_authorization::OperatorIngress {
        ps_authorization::OperatorIngress::from_registry(
            self.clients.clone(),
            self.operator.clone(),
        )
    }
    pub(crate) fn connect(
        &self,
        shared: crate::tx_thread::ps_ingress::SharedIngress,
        model: Arc<Mutex<RadioModel>>,
    ) -> SocketClient {
        self.connect_lane(shared, model, 1, "/tci")
    }
    pub(crate) fn connect_lane(
        &self,
        shared: crate::tx_thread::ps_ingress::SharedIngress,
        model: Arc<Mutex<RadioModel>>,
        id: u64,
        path: &str,
    ) -> SocketClient {
        let listener = self.listener.try_clone().unwrap();
        let addr = listener.local_addr().unwrap();
        let clients = self.clients.clone();
        let operator = self.operator.clone();
        let control = self.control.clone();
        let commands = self.sender.clone();
        let worker = thread::spawn(move || {
            let (stream, peer) = listener.accept().unwrap();
            // Existing handler starts with nonblocking accept. Prequeue the
            // fixture's complete handshake; do not change production handshake
            // behavior as part of TX ingress migration.
            stream.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut request = [0; 8192];
            loop {
                if stream
                    .peek(&mut request)
                    .is_ok_and(|n| request[..n].windows(4).any(|w| w == b"\r\n\r\n"))
                {
                    break;
                }
                assert!(Instant::now() < deadline, "fixture handshake not queued");
                thread::sleep(Duration::from_millis(1));
            }
            let drops = Arc::new(AtomicU64::new(0));
            let stats = Arc::new(FullRateIqTransportStats::default());
            let slot = Arc::new(Mutex::new(Some(shared)));
            handle_client(
                stream,
                peer,
                id,
                &commands,
                &clients,
                &operator,
                &control,
                &model,
                &drops,
                &stats,
                false,
                TxCodecRuntimeFlags {
                    opus_decode_enabled: false,
                },
                (true, 0),
                &DisplayTransport::default(),
                &slot,
            );
        });
        let socket = tungstenite::client(
            format!("ws://{addr}{path}"),
            TcpStream::connect(addr).unwrap(),
        )
        .unwrap()
        .0;
        socket
            .get_ref()
            .set_write_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        SocketClient {
            socket,
            worker: Some(worker),
            transactions: Vec::new(),
            media_token: None,
            diagnostics: Vec::new(),
        }
    }
}
pub(crate) struct BrowserListener {
    pub(crate) url: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    streams: Arc<Mutex<Vec<TcpStream>>>,
    worker: Option<JoinHandle<()>>,
}
impl BrowserListener {
    pub(crate) fn interrupt_connections(&self) {
        for stream in self.streams.lock_unpoisoned().drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
impl Drop for BrowserListener {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl SocketClient {
    pub(crate) fn drain(&mut self) {
        if self.worker.is_none() {
            return;
        }
        self.socket.get_mut().set_nonblocking(true).unwrap();
        for _ in 0..128 {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    // The real snapshot may batch multiple TCI commands in
                    // one WebSocket message. Observe each command, as clients do.
                    for command in text.split(';').filter(|s| !s.is_empty()) {
                        if command.starts_with("saturn_ps_measurement") {
                            self.diagnostics.push(format!("{command};"));
                        } else if command.starts_with("saturn_tx_generation:psg1,") {
                            let value = command.split(',').nth(1).unwrap();
                            self.media_token = if value == "off" {
                                None
                            } else {
                                Some(std::array::from_fn(|i| {
                                    u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap()
                                }))
                            };
                        } else if command.starts_with("tx_config:") {
                            self.transactions.push(format!("{command};"));
                        }
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break
                }
                Err(e) => panic!("fixture websocket reader {e:?}"),
            }
        }
        self.socket.get_mut().set_nonblocking(false).unwrap();
    }
    pub(crate) fn text(&mut self, text: &str) {
        self.socket.send(Message::Text(text.into())).unwrap();
    }
    pub(crate) fn mic(&mut self, samples: &[f32], sequence: u32) {
        let mut bytes = build_tci_audio_frame(0, 48000, 1, samples, sequence);
        bytes[24..28].copy_from_slice(&2u32.to_le_bytes());
        self.socket.send(Message::Binary(bytes.into())).unwrap();
    }
    pub(crate) fn tagged_mic(
        &mut self,
        token: [u8; 16],
        samples: &[f32],
        sequence: u32,
        opus: bool,
    ) -> Vec<u8> {
        let bytes = if opus {
            crate::tx_thread::ps_media::tests::frame(token, sequence, true)
        } else {
            let mut b = build_tci_audio_frame(0, 48000, 1, samples, sequence);
            b[24..28].copy_from_slice(&2u32.to_le_bytes());
            let n = (b.len() - 64) as u32;
            b[40..44].copy_from_slice(&n.to_le_bytes());
            crate::tci::ps_media::tag(&mut b, token);
            b
        };
        self.binary(bytes.clone());
        bytes
    }
    pub(crate) fn binary(&mut self, bytes: Vec<u8>) {
        self.socket.send(Message::Binary(bytes.into())).unwrap();
    }
    pub(crate) fn close(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.get_mut().shutdown(std::net::Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Drop for SocketClient {
    fn drop(&mut self) {
        self.close();
    }
}
