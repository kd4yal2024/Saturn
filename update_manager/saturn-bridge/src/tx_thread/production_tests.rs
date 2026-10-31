//! Production constructor + actual TCI listener/PCM ingress + acquisition
//! layout/demux + matched full native/owner loop. LOOPBACK/REGULAR FILES ONLY.
use super::*;
use crate::{
    config::BridgeConfig, p2::session::managed_feedback::tests::Fixture as P2Fixture,
    xdma_rx::managed_feedback::tests as direct_rx, xdma_tx_radio::offline_fixture::DirectFixture,
};
use std::{
    net::{TcpListener, TcpStream},
    time::Duration,
};
use tungstenite::{Message, WebSocket};

pub(crate) struct Client {
    ws: WebSocket<TcpStream>,
    token: Option<[u8; 16]>,
    sequence: u32,
    pub(crate) transactions: Vec<String>,
    pub(crate) binary_frames: usize,
    pub(crate) iq_frames: usize,
    pub(crate) spectrum_frames: usize,
    pub(crate) vfo_a_reports: usize,
    pub(crate) iq_pairs_per_frame: Vec<usize>,
    pub(crate) audio_frames: usize,
    pub(crate) rx_meter_dbm: Vec<f32>,
    pub(crate) native_applied: Vec<bool>,
    pub(crate) native_calibrations: Vec<i32>,
    pub(crate) two_tone_results: Vec<String>,
}
impl Client {
    pub(crate) fn open(address: std::net::SocketAddr) -> Self {
        Self::open_lane(address, true)
    }
    pub(crate) fn open_unpaired(address: std::net::SocketAddr) -> Self {
        Self::open_lane(address, false)
    }
    fn open_lane(address: std::net::SocketAddr, control_only: bool) -> Self {
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.set_nodelay(true).unwrap(); // local fixture client; not a production TCP setting change
        let ws = tungstenite::client(format!("ws://{address}/tci"), stream)
            .unwrap()
            .0;
        let mut client = Self {
            ws,
            token: None,
            sequence: 0,
            transactions: Vec::new(),
            binary_frames: 0,
            iq_frames: 0,
            spectrum_frames: 0,
            vfo_a_reports: 0,
            iq_pairs_per_frame: Vec::new(),
            audio_frames: 0,
            rx_meter_dbm: Vec::new(),
            native_applied: Vec::new(),
            native_calibrations: Vec::new(),
            two_tone_results: Vec::new(),
        };
        if control_only {
            client.text("session_open:production-fixture,operator;session_lane:production-fixture,control;tx_codec_caps:pcm;");
        } else {
            client.text("session_open:production-fixture,operator;tx_codec_caps:pcm;");
        }
        client.ws.get_mut().set_nonblocking(true).unwrap();
        client
    }
    pub(crate) fn text(&mut self, text: &str) {
        self.ws.send(Message::Text(text.into())).unwrap();
    }
    pub(crate) fn drain(&mut self) {
        for _ in 0..128 {
            match self.ws.read() {
                Ok(Message::Text(text)) if text.starts_with("saturn_tx_generation:psg1,") => {
                    let encoded = text.trim_end_matches(';').split(',').nth(1).unwrap();
                    let token = if encoded == "off" {
                        None
                    } else {
                        Some(std::array::from_fn(|i| {
                            u8::from_str_radix(&encoded[i * 2..i * 2 + 2], 16).unwrap()
                        }))
                    };
                    if token != self.token {
                        self.sequence = 0;
                    }
                    self.token = token;
                }
                Ok(Message::Text(text)) if text.starts_with("tx_config:") => {
                    self.transactions.push(text.to_string())
                }
                Ok(Message::Text(text)) if text.starts_with("vfo:0,0,") => {
                    self.vfo_a_reports += 1;
                }
                Ok(Message::Text(text)) if text.starts_with("rx_smeter:0,0,") => {
                    self.rx_meter_dbm.push(
                        text.trim_end_matches(';')
                            .rsplit(',')
                            .next()
                            .unwrap()
                            .parse()
                            .unwrap(),
                    );
                }
                Ok(Message::Text(text)) if text.starts_with("saturn_ps_two_tone:") => {
                    self.two_tone_results.push(text.to_string());
                }
                Ok(Message::Text(text)) if text.starts_with("saturn_ps_correction:") => {
                    self.native_applied
                        .push(text.contains("\"nativeApplied\":true"));
                    let count = text
                        .split("\"calibrations\":")
                        .nth(1)
                        .unwrap()
                        .split(',')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    self.native_calibrations.push(count);
                }
                Ok(Message::Binary(bytes)) => {
                    self.binary_frames += 1;
                    if bytes.len() >= 64 {
                        match u32::from_le_bytes(bytes[24..28].try_into().unwrap()) {
                            0 => {
                                self.iq_frames += 1;
                                self.iq_pairs_per_frame.push(
                                    u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize
                                        / 2,
                                );
                            }
                            1 | 17 => self.audio_frames += 1,
                            16 => self.spectrum_frames += 1,
                            _ => {}
                        }
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break
                }
                Err(error) => panic!("actual client read {error}"),
            }
        }
    }
    pub(crate) fn mic(&mut self) {
        let samples: Vec<_> = (0..512)
            .map(|i| (std::f32::consts::TAU * 700. * i as f32 / 48000.).sin() * 0.3)
            .collect();
        self.pcm(&samples);
    }
    pub(crate) fn zero_keepalive(&mut self) {
        self.pcm(&[0.; 512]);
    }
    fn pcm(&mut self, samples: &[f32]) {
        let Some(token) = self.token else {
            return;
        };
        let mut bytes = crate::tci::build_tci_audio_frame(0, 48000, 1, &samples, self.sequence);
        self.sequence += 1;
        bytes[24..28].copy_from_slice(&2u32.to_le_bytes());
        let size = (bytes.len() - 64) as u32;
        bytes[40..44].copy_from_slice(&size.to_le_bytes());
        crate::tci::ps_media::tag(&mut bytes, token);
        self.ws.send(Message::Binary(bytes.into())).unwrap();
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.ws.get_mut().shutdown(std::net::Shutdown::Both);
    }
}
pub(crate) struct Wave {
    random: u32,
    sequence: u32,
}
impl Wave {
    pub(crate) fn new() -> Self {
        Self {
            random: 0x53460004,
            sequence: 0,
        }
    }
    fn uniform(&mut self) -> f64 {
        self.random = self.random.wrapping_mul(1664525).wrapping_add(1013904223);
        f64::from(self.random) / 4294967296.
    }
    fn pair(&mut self) -> [u8; 12] {
        let r = 0.001 + 0.999 * self.uniform();
        let angle = std::f64::consts::TAU * self.uniform();
        let feedback = 0.5 * r * (1. - 0.25 * r * r);
        let phase = angle + 0.18 * r * r;
        let mut result = [0; 12];
        for (sample, dest) in [
            feedback * phase.cos(),
            feedback * phase.sin(),
            0.6121 * r * angle.cos(),
            0.6121 * r * angle.sin(),
        ]
        .into_iter()
        .zip(result.chunks_exact_mut(3))
        {
            let v = (sample * 8388608.).round() as i32;
            dest.copy_from_slice(&v.to_be_bytes()[1..]);
        }
        result
    }
    pub(crate) fn p2(&mut self) -> Vec<u8> {
        let mut packet = vec![0; 1444];
        packet[..4].copy_from_slice(&self.sequence.to_be_bytes());
        self.sequence += 1;
        packet[12..14].copy_from_slice(&24u16.to_be_bytes());
        packet[14..16].copy_from_slice(&238u16.to_be_bytes());
        for dest in packet[16..].chunks_exact_mut(12) {
            dest.copy_from_slice(&self.pair());
        }
        packet
    }
    pub(crate) fn direct(&mut self) -> Vec<u8> {
        let mut bytes = vec![0; 136 * 256];
        for frame in bytes.chunks_exact_mut(136) {
            frame[..4].copy_from_slice(&0x10001fu32.to_le_bytes());
            frame[7] = 0x80;
            for i in 0..4 {
                let pair = self.pair();
                frame[8 + i * 16..14 + i * 16].copy_from_slice(&pair[..6]);
                frame[16 + i * 16..22 + i * 16].copy_from_slice(&pair[6..]);
            }
        }
        bytes
    }
}
#[test]
#[ignore = "explicit serial real-native production fixture; no hardware"]
fn production_combined_listener_pcm_acquisition_matched_native_release_rearm() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    } // LOCAL FIXTURE ONLY
    for direct in [false, true] {
        let mut initial = crate::tx_owner_stop::tests::model();
        initial.desired.tx_enabled = false;
        initial.desired.tx_drive = 5;
        if direct {
            initial.desired.rx_ddc_index = 6;
            initial.desired.ddc0_sample_rate_khz = 384;
        }
        let model = Arc::new(Mutex::new(initial.clone()));
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let config = BridgeConfig {
            tci_bind_addr: address,
            remote_tx_rf_enabled: true,
            ..BridgeConfig::default()
        };
        let (front, _commands) =
            TciFrontend::reserve_shared_startup(&config, model.clone(), false).unwrap();
        let front = Arc::new(front);
        let p2 = (!direct).then(P2Fixture::new);
        let files = direct.then(DirectFixture::new);
        let mut rx = files.as_ref().map(|f| {
            f.fifo_ready();
            f.write(0x100c, 0x100000);
            f.write(0x1010, 1 << 30);
            f.write(0, 42);
            f.write(4, 43);
            direct_rx::session(&f.register_path())
        });
        let radio = if let Some(p) = &p2 {
            Radio::P2(p.session.clone())
        } else {
            Radio::Direct(files.as_ref().unwrap().radio.clone())
        };
        let options = Options {
            config: Config {
                id: 1,
                route: 20,
                physical_adc: OverloadSource::Adc1,
                enabled: false,
                automatic: false,
                hardware_peak: 0.6121,
                requested_delay_seconds: 0.,
                loop_delay_seconds: 0.,
                settling_ns: 0,
                stale_after_ns: 250_000_000,
            },
            timing: Timing {
                mic_age_ns: 150_000_000,
                source_stall_ns: 500_000_000,
                tx_duration_ns: 20_000_000_000,
            },
            limits: Limits {
                authority_timeout_ns: 2_000_000_000,
                cleanup_timeout_ns: 3_000_000_000,
            },
            max_watts: 5,
            feedback_topology_reviewed: true,
            managed_rf_permitted: true,
            startup_timeout_ns: 10_000_000_000,
            feedback_attenuation_db: 9,
            measurement_only: false,
            generated_trials_only: false,
            // Explicit synthetic-waveform fixture policy. NOT production defaults.
            operational_policy: Some(DraftLimits {
                active_reference_fraction: 0.1,
                feedback_rms_floor: 1e-6,
                component_limit: 8388607. / 8388608.,
            }),
        };
        let mut runtime = spawn(
            radio,
            front.clone(),
            model.clone(),
            TxAudioSource::Tci,
            options,
            true,
            Duration::from_millis(20),
        )
        .unwrap();
        if let Some(p) = &p2 {
            p.session
                .attach_managed_feedback(
                    runtime.link.clone(),
                    runtime.ingress.clone(),
                    model.clone(),
                    runtime.origin,
                )
                .unwrap();
        }
        if let Some(rx) = &mut rx {
            rx.attach_managed_feedback(
                runtime.link.clone(),
                runtime.ingress.clone(),
                runtime.origin,
            )
            .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut client = None;
        let mut next_mic = Instant::now();
        let mut stage = 0;
        let mut wave = Wave::new();
        let mut sequence = 0;
        let mut old_scope = None;
        let mut corrected = 0;
        let mut old_fault_checked = false;
        loop {
            assert!(
                Instant::now() < deadline,
                "production fixture stage={stage} status={:?}",
                runtime.ingress.lifecycle_status()
            );
            if let Some(p) = &p2 {
                p.service().unwrap();
                p.drain_output();
            }
            if let Some(rx) = &mut rx {
                direct_rx::service(rx).unwrap();
            }
            if let Some(client) = &mut client {
                Client::drain(client);
            }
            let status = runtime.ingress.lifecycle_status().unwrap();
            assert!(
                status.primary_fault.is_none()
                    && status.authority_fault.is_none()
                    && status.cleanup_fault.is_none(),
                "{status:?}"
            );
            match stage {
                0 if runtime.ingress.startup_ready() => {
                    assert!(crate::wdsp::WdspTxEngine::try_new(&initial).is_err());
                    front.admit_shared_clients().unwrap();
                    let mut c = Client::open(address);
                    assert!(!model.lock_unpoisoned().desired.pure_signal_enabled);
                    c.text("tx_puresignal:0,true;");
                    c.text("rx_adc:0,1;"); // real pre-queue unsupported-ADC rejection
                    client = Some(c);
                    stage = 10;
                }
                10 if runtime.ingress.startup_ready()
                    && client
                        .as_ref()
                        .unwrap()
                        .transactions
                        .iter()
                        .any(|s| s == "tx_config:0,1,applied,2;") =>
                {
                    assert!(client
                        .as_ref()
                        .unwrap()
                        .transactions
                        .iter()
                        .any(|s| s == "tx_config:0,2,rejected,InvalidConfig,0;"));
                    assert_eq!(model.lock_unpoisoned().desired.rx_antenna, 1);
                    client.as_mut().unwrap().text("trx:0,true;");
                    stage = 1;
                }
                1 | 3 if status.phase == Phase::Active => {
                    let scope = runtime.link.desired().scope.unwrap();
                    if old_scope.is_none() {
                        old_scope = Some(scope);
                    }
                    if stage == 3 && !old_fault_checked {
                        assert_ne!(scope.output, old_scope.unwrap().output);
                        assert!(!runtime
                            .ingress
                            .stop_feedback(old_scope.unwrap().output, Fault::SequenceGap));
                        old_fault_checked = true;
                    }
                    if let Some(p) = &p2 {
                        p.receive(&wave.p2());
                    }
                    if let Some(rx) = &mut rx {
                        assert_eq!(
                            direct_rx::consume(
                                rx,
                                &wave.direct(),
                                &mut sequence,
                                Some(scope),
                                Instant::now()
                            ),
                            2048
                        );
                    }
                    if status.correction_applied == Some(true) {
                        corrected += 1;
                        client.as_mut().unwrap().text("trx:0,false;");
                        stage += 1;
                    }
                }
                2 if runtime.ingress.startup_ready() => {
                    assert!(runtime.link.restored().unwrap());
                    let s = runtime.ingress.lifecycle_status().unwrap();
                    assert!(s.cleanup_completed_ns.is_some() && s.authority_completed_ns.is_some());
                    if let Some(f) = &files {
                        f.assert_off();
                        assert_eq!(f.read(0x100c), 0x100000);
                        assert_eq!((f.read(0), f.read(4)), (42, 43));
                    }
                    wave = Wave::new();
                    client.as_mut().unwrap().text("trx:0,true;");
                    stage = 3;
                }
                4 if runtime.ingress.startup_ready() => {
                    assert_eq!(corrected, 2);
                    assert!(old_fault_checked);
                    break;
                }
                _ => {}
            }
            if matches!(stage, 1 | 3) && Instant::now() >= next_mic {
                client.as_mut().unwrap().mic();
                next_mic = Instant::now() + Duration::from_nanos(512_000_000_000 / 48_000);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!runtime
            .ingress
            .stop_feedback(old_scope.unwrap().output, Fault::SequenceGap));
        drop(client);
        runtime.request_stop();
        // Complete acquisition restoration cooperatively while the owner drains.
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(p) = &p2 {
                p.service().unwrap();
                p.drain_output();
            }
            if let Some(rx) = &mut rx {
                direct_rx::service(rx).unwrap();
            }
            if let Ok(result) = runtime.done.try_recv() {
                assert!(result.is_ok(), "{result:?}");
                break;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        runtime.worker.take().unwrap().join().unwrap();
        println!("production combined backend={} corrected_generations=2 tagged_pcm_actual_listener=true actual_acquisition_demux=true matched_observer=true layout_restored=true physical_rf_qualified=false",if direct {"DirectXdma"} else {"P2"});
    }
}
