use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tungstenite::error::Error as WsError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_with_config, Message};

use crate::config::BridgeConfig;
use crate::radio_model::{AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel};

#[derive(Clone, Debug)]
pub struct TciMicFrame {
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum TciCommand {
    SetVfoA(u32),
    SetVfoB(u32),
    SetIqCenter(u32),
    SetMode(DemodMode),
    SetFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxAdc(u8),
    SetRxAntenna(u8),
    SetRxVolume(f64),
    SetRxNoiseReductionMode(NoiseReductionMode),
    SetRxNoiseReductionEnabled(bool),
    SetRxNoiseReductionLevel(f64),
    SetRxAnrVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetIqSampleRate(u32),
    SetIqStreaming,
    RequestSmeter,
    SaturnPing {
        nonce: String,
        sent_at: String,
    },
    SetAudioStreaming(bool),
    SetAudioSampleRate(u32),
    SetAudioFrameSamples(u32),
    SetAudioChannels(u32),
    SetTxEnabled(bool),
    SetNoiseBlankerMode(NoiseBlankerMode),
    SetNoiseBlankerThreshold(f64),
    SetAnfEnabled(bool),
    SetRxAnfVals {
        taps: Option<i32>,
        delay: Option<i32>,
        gain: Option<f64>,
        leakage: Option<f64>,
    },
    SetAgcMode(AgcMode),
    SetAgcGain(f64),
    SetTxDrive(u8),
    SetTxMicGain(f64),
    SetTxFilterBand {
        low_hz: i32,
        high_hz: i32,
    },
    SetRxEqEnabled(bool),
    SetRxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxEqEnabled(bool),
    SetTxEqBand {
        band: usize,
        gain_db: i32,
    },
    SetTxCfcEnabled(bool),
    SetTxCfcPrecomp(f64),
    SetTxCfcBand {
        band: usize,
        gain_db: f64,
    },
    SetTxTwoToneTest(bool),
    SetTxTwoToneFreq1(f64),
    SetTxTwoToneFreq2(f64),
    SetTxTwoToneLevelDb(f64),
    SetTxTwoToneInvertLsb(bool),
    SetTxTwoToneDelayMs(u16),
    SetTxNoiseGateEnabled(bool),
    SetTxNoiseGateThreshold(f64),
    SetRxFftSize(u32),
    SetRxLowLatency(bool),
    SetTxFftSize(u32),
    SetTxLowLatency(bool),
    MicAudioFrame(TciMicFrame),
    ClientConnected,
    ClientDisconnected,
}

#[derive(Clone, Debug)]
enum OutboundMessage {
    Text(String),
    IqFrame {
        receiver: u32,
        sample_rate: u32,
        iq_samples: Vec<f32>,
    },
    TxIqFrame {
        receiver: u32,
        sample_rate: u32,
        iq_samples: Vec<f32>,
    },
    AudioFrame {
        receiver: u32,
        sample_rate: u32,
        audio_samples: Vec<f32>,
    },
}

#[derive(Default)]
struct ClientState {
    iq_stream_enabled: bool,
    audio_stream_enabled: bool,
    audio_sample_rate_hz: u32,
    audio_frame_float_count: u32,
    audio_channels: u32,
}

const MAX_TCI_INBOUND_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_TCI_INBOUND_FRAME_BYTES: usize = 256 * 1024;
const MAX_TCI_MIC_FLOAT_SAMPLES: usize = 32_768;

fn tx_power_trip_fault_message(forward_watts: f32, limit_watts: f32) -> String {
    format!("tx_fault:0,power_trip,{forward_watts:.1},{limit_watts:.1};")
}

fn remote_client_role_message(client_id: u64) -> String {
    format!("remote_client_role:0,operator,{client_id};")
}

pub struct TciFrontend {
    command_rx: Receiver<TciCommand>,
    outbound_tx: Arc<Mutex<Option<(u64, SyncSender<OutboundMessage>)>>>,
    client_state: Arc<Mutex<ClientState>>,
    drop_count: Arc<AtomicU64>,
    tx_power_meter_scale: f32,
    remote_tx_rf_enabled: bool,
    _accept_thread: JoinGuard,
}

#[derive(Clone, Copy, Debug)]
pub struct TciClientSnapshot {
    pub active: bool,
    pub iq_stream_enabled: bool,
    pub audio_stream_enabled: bool,
    pub outbound_drops: u64,
}

struct JoinGuard {
    #[allow(dead_code)]
    handle: thread::JoinHandle<()>,
}

impl TciFrontend {
    pub fn bind(config: &BridgeConfig, radio_model: Arc<Mutex<RadioModel>>) -> io::Result<Self> {
        let listener = TcpListener::bind(config.tci_bind_addr)?;
        listener.set_nonblocking(true)?;

        let (command_tx, command_rx) = mpsc::channel();
        let outbound_tx = Arc::new(Mutex::new(None));
        let client_state = Arc::new(Mutex::new(ClientState::default()));
        let active_client_id = Arc::new(AtomicU64::new(0));
        let drop_count = Arc::new(AtomicU64::new(0));
        let remote_tx_rf_enabled = config.remote_tx_rf_enabled;

        let outbound_slot = outbound_tx.clone();
        let client_flags = client_state.clone();
        let latest_client = active_client_id.clone();
        let radio_model = radio_model.clone();
        let handle = thread::spawn(move || loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let client_id = latest_client.fetch_add(1, Ordering::SeqCst) + 1;
                    if client_id > 1 {
                        println!("saturn-bridge: replacing prior TCI client with {addr}");
                    } else {
                        println!("saturn-bridge: TCI client connected from {addr}");
                    }

                    let command_tx = command_tx.clone();
                    let outbound_slot = outbound_slot.clone();
                    let client_flags = client_flags.clone();
                    let latest_client = latest_client.clone();
                    let radio_model = radio_model.clone();

                    thread::spawn(move || {
                        handle_client(
                            stream,
                            addr,
                            client_id,
                            &command_tx,
                            &outbound_slot,
                            &client_flags,
                            &latest_client,
                            &radio_model,
                            remote_tx_rf_enabled,
                        );
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    eprintln!("saturn-bridge: TCI listener error: {error}");
                    thread::sleep(Duration::from_millis(250));
                }
            }
        });

        Ok(Self {
            command_rx,
            outbound_tx,
            client_state,
            drop_count,
            tx_power_meter_scale: config.tx_power_meter_scale,
            remote_tx_rf_enabled,
            _accept_thread: JoinGuard { handle },
        })
    }

    pub fn try_recv_command(&self) -> Option<TciCommand> {
        self.command_rx.try_recv().ok()
    }

    pub fn client_snapshot(&self) -> TciClientSnapshot {
        let flags = self.client_state.lock().unwrap();
        TciClientSnapshot {
            active: self.outbound_tx.lock().unwrap().is_some(),
            iq_stream_enabled: flags.iq_stream_enabled,
            audio_stream_enabled: flags.audio_stream_enabled,
            outbound_drops: self.drop_count.load(Ordering::Relaxed),
        }
    }

    pub fn publish_radio_state(&self, model: &RadioModel) {
        self.send_text(format!("vfo:0,0,{};", model.desired.vfo_a_hz));
        self.send_text(format!("vfo:0,1,{};", model.desired.vfo_b_hz));
        self.send_text(format!("dds:0,{};", model.desired.iq_center_hz));
        self.send_text(format!("rx_adc:0,{};", model.desired.ddc0_adc));
        self.send_text(format!(
            "rx_antenna:0,{};",
            model.desired.rx_antenna.max(1).min(3)
        ));
        self.send_text(format!(
            "iq_samplerate:{};",
            model.desired.ddc0_sample_rate_khz as u32 * 1000
        ));
        self.send_text(format!("modulation:0,{};", model.desired.mode));
        self.send_text(format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db));
        self.send_text(format!(
            "rx_nr:0,{};",
            model.desired.rx_noise_reduction_mode != NoiseReductionMode::Off
        ));
        self.send_text(format!(
            "rx_nr_mode:0,{};",
            model.desired.rx_noise_reduction_mode
        ));
        self.send_text(format!(
            "rx_nr_level:0,{:.0};",
            model.desired.rx_noise_reduction_level
        ));
        self.send_text(format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ));
        self.send_text(format!("rx_nb:0,{};", model.desired.nb_mode));
        self.send_text(format!(
            "rx_nb_threshold:0,{:.2};",
            model.desired.nb_threshold
        ));
        self.send_text(format!("rx_anr_taps:0,{};", model.desired.rx_anr_taps));
        self.send_text(format!("rx_anr_delay:0,{};", model.desired.rx_anr_delay));
        self.send_text(format!("rx_anr_gain:0,{:.6};", model.desired.rx_anr_gain));
        self.send_text(format!(
            "rx_anr_leakage:0,{:.6};",
            model.desired.rx_anr_leakage
        ));
        self.send_text(format!("rx_anf:0,{};", model.desired.anf_enabled));
        self.send_text(format!("rx_anf_taps:0,{};", model.desired.rx_anf_taps));
        self.send_text(format!("rx_anf_delay:0,{};", model.desired.rx_anf_delay));
        self.send_text(format!("rx_anf_gain:0,{:.6};", model.desired.rx_anf_gain));
        self.send_text(format!(
            "rx_anf_leakage:0,{:.6};",
            model.desired.rx_anf_leakage
        ));
        self.send_text(format!("rx_agc:0,{};", model.desired.agc_mode));
        self.send_text(format!("rx_agc_gain:0,{:.0};", model.desired.agc_gain));
        self.send_text(format!("tx_drive:0,{};", model.desired.tx_drive));
        self.send_text(format!(
            "remote_tx_rf_enabled:0,{};",
            self.remote_tx_rf_enabled
        ));
        self.send_text(format!(
            "tx_mic_gain:0,{:.1};",
            model.desired.tx_mic_gain_db
        ));
        self.send_text(format!("trx:0,{};", model.desired.tx_enabled));
        self.send_text(format!("tx_state:0,{};", model.desired.tx_phase));
        self.send_text(format!(
            "tx_filter_band:0,{},{};",
            model.desired.tx_filter_low_hz, model.desired.tx_filter_high_hz
        ));
        self.send_text(format!("rx_eq_enable:0,{};", model.desired.rx_eq_enabled));
        self.send_text(format!("tx_eq_enable:0,{};", model.desired.tx_eq_enabled));
        for i in 1..=10 {
            self.send_text(format!(
                "rx_eq_band:0,{},{};",
                i, model.desired.rx_eq_bands[i]
            ));
            self.send_text(format!(
                "tx_eq_band:0,{},{};",
                i, model.desired.tx_eq_bands[i]
            ));
        }
        self.send_text(format!("tx_cfc_enable:0,{};", model.desired.cfc_enabled));
        self.send_text(format!(
            "tx_cfc_precomp:0,{:.1};",
            model.desired.cfc_precomp_db
        ));
        for i in 0..10 {
            self.send_text(format!(
                "tx_cfc_band:0,{},{:.1};",
                i + 1,
                model.desired.cfc_bands[i]
            ));
        }
        self.send_text(format!("tx_two_tone:0,{};", model.desired.two_tone_enabled));
        self.send_text(format!(
            "tx_two_tone_freq1:0,{:.0};",
            model.desired.tx_two_tone_freq1_hz
        ));
        self.send_text(format!(
            "tx_two_tone_freq2:0,{:.0};",
            model.desired.tx_two_tone_freq2_hz
        ));
        self.send_text(format!(
            "tx_two_tone_level_db:0,{:.1};",
            model.desired.tx_two_tone_level_db
        ));
        self.send_text(format!(
            "tx_two_tone_invert_lsb:0,{};",
            model.desired.tx_two_tone_invert_lsb
        ));
        self.send_text(format!(
            "tx_two_tone_delay_ms:0,{};",
            model.desired.tx_two_tone_delay_ms
        ));
        self.send_text(format!(
            "tx_noise_gate:0,{};",
            model.desired.tx_noise_gate_enabled
        ));
        self.send_text(format!(
            "tx_noise_gate_threshold:0,{:.1};",
            model.desired.tx_noise_gate_threshold_db
        ));
        self.send_text(format!("rx_fft_size:0,{};", model.desired.rx_fft_size));
        self.send_text(format!(
            "rx_low_latency:0,{};",
            model.desired.rx_low_latency
        ));
        self.send_text(format!("tx_fft_size:0,{};", model.desired.tx_fft_size));
        self.send_text(format!(
            "tx_low_latency:0,{};",
            model.desired.tx_low_latency
        ));
        self.send_text("tune:0,false;".to_string());
        self.publish_telemetry(model);
    }

    pub fn publish_telemetry(&self, model: &RadioModel) {
        if let Some(meter_dbm) = model.observed.ddc0_meter_dbm {
            self.send_text(format!("rx_smeter:0,0,{meter_dbm:.1};"));
        }
        if let Some(packet) = model.observed.high_priority.as_ref() {
            let fwd_watts =
                saturn_adc_to_watts(packet.forward_power, 32, self.tx_power_meter_scale);
            let rev_watts =
                saturn_adc_to_watts(packet.reverse_power, 28, self.tx_power_meter_scale);
            self.send_text(format!("tx_power:0,{:.1};", fwd_watts));
            self.send_text(format!(
                "swr:0,{:.2};",
                calculate_swr_watts(fwd_watts, rev_watts)
            ));
        }
        let drops = self.drop_count.swap(0, Ordering::Relaxed);
        if drops > 0 {
            self.send_text(format!("rx_drops:{drops};"));
        }
    }

    pub fn publish_saturn_pong(&self, nonce: &str, sent_at: &str) {
        self.send_text(format!("saturn_pong:{nonce},{sent_at};"));
    }

    pub fn publish_tx_power_trip(&self, forward_watts: f32, limit_watts: f32) {
        self.send_text(tx_power_trip_fault_message(forward_watts, limit_watts));
    }

    pub fn publish_iq_frame(&self, sample_rate_hz: u32, iq_samples: &[f32]) {
        if !self.is_iq_stream_enabled() {
            return;
        }

        self.send_message(OutboundMessage::IqFrame {
            receiver: 0,
            sample_rate: sample_rate_hz,
            iq_samples: iq_samples.to_vec(),
        });
    }

    pub fn publish_tx_iq_frame(&self, sample_rate_hz: u32, iq_samples: &[f32]) {
        if !self.is_iq_stream_enabled() {
            return;
        }

        self.send_message(OutboundMessage::TxIqFrame {
            receiver: 0,
            sample_rate: sample_rate_hz,
            iq_samples: iq_samples.to_vec(),
        });
    }

    pub fn publish_audio_started(&self, sample_rate_hz: u32) {
        self.send_text("audio_start:0;".to_string());
        self.send_text(format!("audio_samplerate:{sample_rate_hz};"));
    }

    pub fn publish_audio_stopped(&self) {
        self.send_text("audio_stop:0;".to_string());
    }

    pub fn publish_audio_frame(&self, sample_rate_hz: u32, audio_samples: &[f32]) {
        if !self.is_audio_stream_enabled() {
            return;
        }

        self.send_message(OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: sample_rate_hz,
            audio_samples: audio_samples.to_vec(),
        });
    }

    fn is_iq_stream_enabled(&self) -> bool {
        self.client_state.lock().unwrap().iq_stream_enabled
    }

    fn is_audio_stream_enabled(&self) -> bool {
        self.client_state.lock().unwrap().audio_stream_enabled
    }

    fn send_text(&self, text: String) {
        self.send_message(OutboundMessage::Text(text));
    }

    fn send_message(&self, message: OutboundMessage) {
        if let Some((_, sender)) = self.outbound_tx.lock().unwrap().as_ref().cloned() {
            match sender.try_send(message) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.drop_count.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }
}

fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    client_id: u64,
    command_tx: &Sender<TciCommand>,
    outbound_slot: &Arc<Mutex<Option<(u64, SyncSender<OutboundMessage>)>>>,
    client_flags: &Arc<Mutex<ClientState>>,
    latest_client: &Arc<AtomicU64>,
    radio_model: &Arc<Mutex<RadioModel>>,
    remote_tx_rf_enabled: bool,
) {
    let _ = stream.set_nonblocking(true);
    match accept_with_config(stream, Some(tci_websocket_config())) {
        Ok(mut websocket) => {
            let (client_tx, client_rx) = mpsc::sync_channel::<OutboundMessage>(256);
            {
                let mut slot = outbound_slot.lock().unwrap();
                if latest_client.load(Ordering::SeqCst) != client_id {
                    let _ = websocket.close(None);
                    return;
                }
                *slot = Some((client_id, client_tx.clone()));
            }
            reset_client_state(client_flags);

            for message in initial_snapshot_messages(
                &radio_model.lock().unwrap(),
                remote_tx_rf_enabled,
                client_id,
            ) {
                let _ = client_tx.send(OutboundMessage::Text(message));
            }

            let _ = command_tx.send(TciCommand::ClientConnected);

            let mut superseded = false;
            loop {
                if latest_client.load(Ordering::SeqCst) != client_id {
                    superseded = true;
                    let _ = websocket.close(None);
                    break;
                }

                let mut pending_flush = false;
                let mut client_closed = false;
                for _ in 0..64 {
                    match websocket.read() {
                        Ok(message) => {
                            if !handle_incoming_message(message, command_tx, client_flags) {
                                client_closed = true;
                                break;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                            client_closed = true;
                            break;
                        }
                        Err(error) => {
                            eprintln!(
                                "saturn-bridge: TCI websocket read error from {addr}: {error}"
                            );
                            client_closed = true;
                            break;
                        }
                    }
                }
                if client_closed {
                    break;
                }

                loop {
                    match client_rx.try_recv() {
                        Ok(message) => {
                            match send_outbound(&mut websocket, message) {
                                Ok(()) => {
                                    pending_flush = true;
                                }
                                Err(WsError::Io(error))
                                    if error.kind() == io::ErrorKind::WouldBlock =>
                                {
                                    pending_flush = true;
                                    break;
                                }
                                Err(error) => {
                                    eprintln!("saturn-bridge: TCI websocket send error to {addr}: {error}");
                                    pending_flush = true;
                                    break;
                                }
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
                    }
                }

                if pending_flush {
                    match websocket.flush() {
                        Ok(()) => {}
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => break,
                        Err(error) => {
                            eprintln!(
                                "saturn-bridge: TCI websocket flush error for {addr}: {error}"
                            );
                            break;
                        }
                    }
                }

                thread::sleep(Duration::from_millis(2));
            }

            clear_active_client(outbound_slot, client_flags, latest_client, client_id);
            if superseded {
                println!("saturn-bridge: TCI client {addr} superseded by a newer session");
            } else {
                println!("saturn-bridge: TCI client disconnected from {addr}");
                let _ = command_tx.send(TciCommand::ClientDisconnected);
            }
        }
        Err(error) => {
            eprintln!("saturn-bridge: TCI websocket accept failed from {addr}: {error}");
        }
    }
}

fn reset_client_state(client_flags: &Arc<Mutex<ClientState>>) {
    let mut flags = client_flags.lock().unwrap();
    flags.iq_stream_enabled = false;
    flags.audio_stream_enabled = false;
    flags.audio_sample_rate_hz = 48_000;
    flags.audio_frame_float_count = 2048;
    flags.audio_channels = 2;
}

fn clear_active_client(
    outbound_slot: &Arc<Mutex<Option<(u64, SyncSender<OutboundMessage>)>>>,
    client_flags: &Arc<Mutex<ClientState>>,
    latest_client: &Arc<AtomicU64>,
    client_id: u64,
) {
    if latest_client.load(Ordering::SeqCst) != client_id {
        return;
    }

    {
        let mut slot = outbound_slot.lock().unwrap();
        if slot.as_ref().map(|(active_id, _)| *active_id) == Some(client_id) {
            *slot = None;
        }
    }
    {
        let mut flags = client_flags.lock().unwrap();
        flags.iq_stream_enabled = false;
        flags.audio_stream_enabled = false;
    }
}

fn initial_snapshot_messages(
    model: &RadioModel,
    remote_tx_rf_enabled: bool,
    client_id: u64,
) -> Vec<String> {
    vec![
        "ready;".to_string(),
        remote_client_role_message(client_id),
        format!("vfo:0,0,{};", model.desired.vfo_a_hz),
        format!("vfo:0,1,{};", model.desired.vfo_b_hz),
        format!("dds:0,{};", model.desired.iq_center_hz),
        format!("rx_adc:0,{};", model.desired.ddc0_adc),
        format!("rx_antenna:0,{};", model.desired.rx_antenna.max(1).min(3)),
        format!(
            "iq_samplerate:{};",
            model.desired.ddc0_sample_rate_khz as u32 * 1000
        ),
        format!("modulation:0,{};", model.desired.mode),
        format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db),
        format!(
            "rx_nr:0,{};",
            model.desired.rx_noise_reduction_mode != NoiseReductionMode::Off
        ),
        format!("rx_nr_mode:0,{};", model.desired.rx_noise_reduction_mode),
        format!(
            "rx_nr_level:0,{:.0};",
            model.desired.rx_noise_reduction_level
        ),
        format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ),
        format!("rx_nb:0,{};", model.desired.nb_mode),
        format!("rx_nb_threshold:0,{:.2};", model.desired.nb_threshold),
        format!("rx_anr_taps:0,{};", model.desired.rx_anr_taps),
        format!("rx_anr_delay:0,{};", model.desired.rx_anr_delay),
        format!("rx_anr_gain:0,{:.6};", model.desired.rx_anr_gain),
        format!("rx_anr_leakage:0,{:.6};", model.desired.rx_anr_leakage),
        format!("rx_anf:0,{};", model.desired.anf_enabled),
        format!("rx_anf_taps:0,{};", model.desired.rx_anf_taps),
        format!("rx_anf_delay:0,{};", model.desired.rx_anf_delay),
        format!("rx_anf_gain:0,{:.6};", model.desired.rx_anf_gain),
        format!("rx_anf_leakage:0,{:.6};", model.desired.rx_anf_leakage),
        format!("rx_agc:0,{};", model.desired.agc_mode),
        format!("rx_agc_gain:0,{:.0};", model.desired.agc_gain),
        format!("tx_drive:0,{};", model.desired.tx_drive),
        format!("remote_tx_rf_enabled:0,{remote_tx_rf_enabled};"),
        format!("tx_mic_gain:0,{:.1};", model.desired.tx_mic_gain_db),
        format!("trx:0,{};", model.desired.tx_enabled),
        format!("tx_state:0,{};", model.desired.tx_phase),
        format!(
            "tx_filter_band:0,{},{};",
            model.desired.tx_filter_low_hz, model.desired.tx_filter_high_hz
        ),
        format!("rx_eq_enable:0,{};", model.desired.rx_eq_enabled),
        format!("tx_eq_enable:0,{};", model.desired.tx_eq_enabled),
    ]
    .into_iter()
    .chain((1..=10).flat_map(|i| {
        vec![
            format!("rx_eq_band:0,{},{};", i, model.desired.rx_eq_bands[i]),
            format!("tx_eq_band:0,{},{};", i, model.desired.tx_eq_bands[i]),
        ]
    }))
    .chain([
        format!("tx_cfc_enable:0,{};", model.desired.cfc_enabled),
        format!("tx_cfc_precomp:0,{:.1};", model.desired.cfc_precomp_db),
    ])
    .chain(
        (0..10usize).map(|i| format!("tx_cfc_band:0,{},{:.1};", i + 1, model.desired.cfc_bands[i])),
    )
    .chain([
        format!("tx_two_tone:0,{};", model.desired.two_tone_enabled),
        format!(
            "tx_two_tone_freq1:0,{:.0};",
            model.desired.tx_two_tone_freq1_hz
        ),
        format!(
            "tx_two_tone_freq2:0,{:.0};",
            model.desired.tx_two_tone_freq2_hz
        ),
        format!(
            "tx_two_tone_level_db:0,{:.1};",
            model.desired.tx_two_tone_level_db
        ),
        format!(
            "tx_two_tone_invert_lsb:0,{};",
            model.desired.tx_two_tone_invert_lsb
        ),
        format!(
            "tx_two_tone_delay_ms:0,{};",
            model.desired.tx_two_tone_delay_ms
        ),
        format!("tx_noise_gate:0,{};", model.desired.tx_noise_gate_enabled),
        format!(
            "tx_noise_gate_threshold:0,{:.1};",
            model.desired.tx_noise_gate_threshold_db
        ),
        format!("rx_fft_size:0,{};", model.desired.rx_fft_size),
        format!("rx_low_latency:0,{};", model.desired.rx_low_latency),
        format!("tx_fft_size:0,{};", model.desired.tx_fft_size),
        format!("tx_low_latency:0,{};", model.desired.tx_low_latency),
        "tune:0,false;".to_string(),
        "audio_samplerate:48000;".to_string(),
    ])
    .collect()
}

fn handle_incoming_message(
    message: Message,
    command_tx: &Sender<TciCommand>,
    client_state: &Arc<Mutex<ClientState>>,
) -> bool {
    match message {
        Message::Text(text) => {
            for command in text
                .split(';')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                parse_tci_command(command, command_tx, client_state);
            }
            true
        }
        Message::Binary(data) => {
            if let Some(frame) = parse_tci_mic_frame(&data) {
                let _ = command_tx.send(TciCommand::MicAudioFrame(frame));
            }
            true
        }
        Message::Ping(payload) => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
            let _ = payload;
            true
        }
        Message::Close(_) => false,
        _ => true,
    }
}

fn parse_tci_command(
    command: &str,
    command_tx: &Sender<TciCommand>,
    client_state: &Arc<Mutex<ClientState>>,
) {
    let Some((name, rest)) = command.split_once(':') else {
        return;
    };

    let args: Vec<&str> = rest.split(',').collect();
    match name.to_ascii_lowercase().as_str() {
        "vfo" => {
            if args.len() >= 3 {
                if let Ok(freq_hz) = args[2].trim().parse::<u32>() {
                    let which = args[1].trim();
                    let _ = match which {
                        "0" => command_tx.send(TciCommand::SetVfoA(freq_hz)),
                        "1" => command_tx.send(TciCommand::SetVfoB(freq_hz)),
                        _ => Ok(()),
                    };
                }
            }
        }
        "dds" => {
            if args.len() >= 2 {
                if let Ok(freq_hz) = args[1].trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqCenter(freq_hz));
                }
            }
        }
        "saturn_ping" => {
            if args.len() >= 2 {
                let nonce = sanitize_token(args[0], 32);
                let sent_at = sanitize_token(args[1], 32);
                if !nonce.is_empty() && !sent_at.is_empty() {
                    let _ = command_tx.send(TciCommand::SaturnPing { nonce, sent_at });
                }
            }
        }
        "modulation" => {
            if args.len() >= 2 {
                let _ = command_tx.send(TciCommand::SetMode(DemodMode::from_tci(args[1])));
            }
        }
        "rx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_volume" => {
            let volume_arg = if args.len() >= 3 {
                args.get(2)
            } else if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(volume_text) = volume_arg {
                if let Ok(volume_db) = volume_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxVolume(volume_db));
                }
            }
        }
        "rx_nr_mode" | "nr_mode" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetRxNoiseReductionMode(
                    NoiseReductionMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nr" | "nr" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionEnabled(enabled));
                }
            }
        }
        "rx_nr_level" | "nr_level" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxNoiseReductionLevel(level));
                }
            }
        }
        "rx_anr_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anr_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnrVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_adc" => {
            let adc_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(adc_text) = adc_arg {
                if let Ok(adc) = adc_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAdc(adc.min(2)));
                }
            }
        }
        "rx_antenna" => {
            let antenna_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(antenna_text) = antenna_arg {
                if let Ok(antenna) = antenna_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAntenna(antenna.clamp(1, 3)));
                }
            }
        }
        "iq_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetIqSampleRate(rate_hz));
                }
            }
        }
        "iq_start" => {
            client_state.lock().unwrap().iq_stream_enabled = true;
            println!("saturn-bridge: TCI iq_start requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "iq_stop" => {
            client_state.lock().unwrap().iq_stream_enabled = false;
            println!("saturn-bridge: TCI iq_stop requested");
            let _ = command_tx.send(TciCommand::SetIqStreaming);
        }
        "audio_start" => {
            client_state.lock().unwrap().audio_stream_enabled = true;
            println!("saturn-bridge: TCI audio_start requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(true));
        }
        "audio_stop" => {
            client_state.lock().unwrap().audio_stream_enabled = false;
            println!("saturn-bridge: TCI audio_stop requested");
            let _ = command_tx.send(TciCommand::SetAudioStreaming(false));
        }
        "audio_samplerate" => {
            if let Some(rate_text) = args.first() {
                if let Ok(rate_hz) = rate_text.trim().parse::<u32>() {
                    client_state.lock().unwrap().audio_sample_rate_hz = rate_hz;
                    let _ = command_tx.send(TciCommand::SetAudioSampleRate(rate_hz));
                }
            }
        }
        "audio_stream_samples" => {
            if let Some(sample_text) = args.first() {
                if let Ok(sample_count) = sample_text.trim().parse::<u32>() {
                    client_state.lock().unwrap().audio_frame_float_count = sample_count;
                    let _ = command_tx.send(TciCommand::SetAudioFrameSamples(sample_count));
                }
            }
        }
        "audio_stream_channels" => {
            if let Some(channel_text) = args.first() {
                if let Ok(channels) = channel_text.trim().parse::<u32>() {
                    client_state.lock().unwrap().audio_channels = channels;
                    let _ = command_tx.send(TciCommand::SetAudioChannels(channels));
                }
            }
        }
        "audio_stream_sample_type" => {}
        "rx_smeter" | "s_meter" | "smeter" => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
        }
        "trx" => {
            // trx:0,true or trx:0,true,tci — PTT on/off
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    println!("saturn-bridge: TCI trx requested -> {}", enabled);
                    let _ = command_tx.send(TciCommand::SetTxEnabled(enabled));
                }
            }
        }
        "rx_nb" => {
            // rx_nb:0,mode — NB mode: 0=OFF, 1=NB1, 2=NB2
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetNoiseBlankerMode(
                    NoiseBlankerMode::from_tci(mode_text),
                ));
            }
        }
        "rx_nb_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh) = thresh_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetNoiseBlankerThreshold(thresh));
                }
            }
        }
        "rx_anf" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetAnfEnabled(enabled));
                }
            }
        }
        "rx_anf_taps" => {
            let taps_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(taps_text) = taps_arg {
                if let Ok(taps) = taps_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: Some(taps),
                        delay: None,
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_delay" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay) = delay_text.trim().parse::<i32>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: Some(delay),
                        gain: None,
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: Some(gain),
                        leakage: None,
                    });
                }
            }
        }
        "rx_anf_leakage" => {
            let leakage_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(leakage_text) = leakage_arg {
                if let Ok(leakage) = leakage_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetRxAnfVals {
                        taps: None,
                        delay: None,
                        gain: None,
                        leakage: Some(leakage),
                    });
                }
            }
        }
        "rx_agc" => {
            let mode_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(mode_text) = mode_arg {
                let _ = command_tx.send(TciCommand::SetAgcMode(AgcMode::from_tci(mode_text)));
            }
        }
        "rx_agc_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetAgcGain(gain));
                }
            }
        }
        "tx_drive" => {
            let drive_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(drive_text) = drive_arg {
                if let Ok(drive) = drive_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetTxDrive(drive));
                }
            }
        }
        "tx_mic_gain" => {
            let gain_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(gain_text) = gain_arg {
                if let Ok(gain_db) = gain_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxMicGain(gain_db));
                }
            }
        }
        "tx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) =
                    (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
                {
                    let _ = command_tx.send(TciCommand::SetTxFilterBand { low_hz, high_hz });
                }
            }
        }
        "rx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetRxEqEnabled(enabled));
                }
            }
        }
        "tx_eq_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxEqEnabled(enabled));
                }
            }
        }
        "rx_eq_band" => {
            // rx_eq_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB integers
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetRxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_eq_band" => {
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<i32>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxEqBand { band, gain_db });
                    }
                }
            }
        }
        "tx_cfc_enable" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxCfcEnabled(enabled));
                }
            }
        }
        "tx_cfc_precomp" => {
            let precomp_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(precomp_text) = precomp_arg {
                if let Ok(db) = precomp_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxCfcPrecomp(db));
                }
            }
        }
        "tx_cfc_band" => {
            // tx_cfc_band:0,band_idx,gain_db  — band_idx 1-10, gain_db in dB
            if args.len() >= 3 {
                if let (Ok(band), Ok(gain_db)) = (
                    args[1].trim().parse::<usize>(),
                    args[2].trim().parse::<f64>(),
                ) {
                    if band >= 1 && band <= 10 {
                        let _ = command_tx.send(TciCommand::SetTxCfcBand { band, gain_db });
                    }
                }
            }
        }
        "tx_two_tone" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneTest(enabled));
                }
            }
        }
        "tx_two_tone_freq1" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq1(freq_hz));
                }
            }
        }
        "tx_two_tone_freq2" => {
            let freq_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(freq_text) = freq_arg {
                if let Ok(freq_hz) = freq_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneFreq2(freq_hz));
                }
            }
        }
        "tx_two_tone_level_db" => {
            let level_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(level_text) = level_arg {
                if let Ok(level_db) = level_text.trim().parse::<f64>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneLevelDb(level_db));
                }
            }
        }
        "tx_two_tone_invert_lsb" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneInvertLsb(enabled));
                }
            }
        }
        "tx_two_tone_delay_ms" => {
            let delay_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(delay_text) = delay_arg {
                if let Ok(delay_ms) = delay_text.trim().parse::<u16>() {
                    let _ = command_tx.send(TciCommand::SetTxTwoToneDelayMs(delay_ms));
                }
            }
        }
        "tx_noise_gate" => {
            let enabled_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(enabled_text) = enabled_arg {
                if let Some(enabled) = parse_tci_bool(enabled_text) {
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateEnabled(enabled));
                }
            }
        }
        "tx_noise_gate_threshold" => {
            let thresh_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(thresh_text) = thresh_arg {
                if let Ok(thresh_db) = thresh_text.trim().parse::<f64>() {
                    let clamped = thresh_db.clamp(-80.0, 0.0);
                    let _ = command_tx.send(TciCommand::SetTxNoiseGateThreshold(clamped));
                }
            }
        }
        "rx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetRxFftSize(size));
                }
            }
        }
        "rx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetRxLowLatency(val));
                }
            }
        }
        "tx_fft_size" => {
            let size_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = size_arg {
                if let Ok(size) = text.trim().parse::<u32>() {
                    let _ = command_tx.send(TciCommand::SetTxFftSize(size));
                }
            }
        }
        "tx_low_latency" => {
            let val_arg = if args.len() >= 2 {
                args.get(1)
            } else {
                args.first()
            };
            if let Some(text) = val_arg {
                if let Some(val) = parse_tci_bool(text) {
                    let _ = command_tx.send(TciCommand::SetTxLowLatency(val));
                }
            }
        }
        _ => {}
    }
}

/// Parse a TCI binary frame that contains TX mic audio from the client.
/// Frame layout: 64-byte header + f32 LE samples.
///   header[20..24] = sample_count (u32 LE)
///   header[24..28] = stream_type  (u32 LE); must be 2 (TX mic)
///   header[28..32] = channels     (u32 LE); 1=mono, 2=stereo
///
/// stream_type == 1 is intentionally excluded: it is the RX audio type used by
/// the server→client direction and must not be fed into the TX DSP path.
fn parse_tci_mic_frame(data: &[u8]) -> Option<TciMicFrame> {
    if data.len() < 68 {
        return None; // need at least header + 1 sample
    }
    let sample_rate_hz = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let stream_type = u32::from_le_bytes(data[24..28].try_into().ok()?);
    if stream_type != 2 {
        return None;
    }
    let raw_channels = u32::from_le_bytes(data[28..32].try_into().ok()?);
    let channels = match raw_channels {
        0 | 1 => 1,
        _ => 2,
    };
    let sample_count = u32::from_le_bytes(data[20..24].try_into().ok()?) as usize;
    if sample_count == 0 || sample_count > MAX_TCI_MIC_FLOAT_SAMPLES {
        return None;
    }
    let payload = &data[64..];
    if payload.len() < sample_count * 4 {
        return None;
    }
    let mut samples = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let bytes: [u8; 4] = payload[i * 4..i * 4 + 4].try_into().ok()?;
        samples.push(f32::from_le_bytes(bytes));
    }
    Some(TciMicFrame {
        sample_rate_hz,
        channels,
        samples,
    })
}

fn tci_websocket_config() -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(MAX_TCI_INBOUND_MESSAGE_BYTES),
        max_frame_size: Some(MAX_TCI_INBOUND_FRAME_BYTES),
        ..WebSocketConfig::default()
    }
}

fn send_outbound(
    websocket: &mut tungstenite::WebSocket<std::net::TcpStream>,
    message: OutboundMessage,
) -> Result<(), WsError> {
    match message {
        OutboundMessage::Text(text) => websocket.send(Message::Text(text)),
        OutboundMessage::IqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(build_tci_iq_frame(
            receiver,
            sample_rate,
            &iq_samples,
        ))),
        OutboundMessage::TxIqFrame {
            receiver,
            sample_rate,
            iq_samples,
        } => websocket.send(Message::Binary(build_tci_tx_iq_frame(
            receiver,
            sample_rate,
            &iq_samples,
        ))),
        OutboundMessage::AudioFrame {
            receiver,
            sample_rate,
            audio_samples,
        } => websocket.send(Message::Binary(build_tci_audio_frame(
            receiver,
            sample_rate,
            &audio_samples,
        ))),
    }
}

fn build_tci_iq_frame(receiver: u32, sample_rate: u32, iq_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 0, 2)
}

fn build_tci_tx_iq_frame(receiver: u32, sample_rate: u32, iq_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, iq_samples, 3, 2)
}

fn build_tci_audio_frame(receiver: u32, sample_rate: u32, audio_samples: &[f32]) -> Vec<u8> {
    build_tci_float_frame(receiver, sample_rate, audio_samples, 1, 2)
}

fn build_tci_float_frame(
    receiver: u32,
    sample_rate: u32,
    samples: &[f32],
    stream_type: u32,
    channels: u32,
) -> Vec<u8> {
    let mut frame = vec![0u8; 64 + samples.len() * 4];
    write_u32_le(&mut frame, 0, receiver);
    write_u32_le(&mut frame, 4, sample_rate);
    write_u32_le(&mut frame, 8, 3);
    write_u32_le(&mut frame, 12, 0);
    write_u32_le(&mut frame, 16, 0);
    write_u32_le(&mut frame, 20, samples.len() as u32);
    write_u32_le(&mut frame, 24, stream_type);
    write_u32_le(&mut frame, 28, channels);

    for (index, value) in samples.iter().enumerate() {
        let offset = 64 + index * 4;
        frame[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    frame
}

fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn parse_tci_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn sanitize_token(text: &str, max_len: usize) -> String {
    text.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .take(max_len)
        .collect()
}

/// Convert a Saturn G2 raw power ADC reading to watts.
/// Uses the ANAN-7000/Saturn 100 W PA calibration constants from pihpsdr:
///   ADC_REF = 5.0 V, coupling = 0.12 (fwd) / 0.12 (rev), fwd_offset = 32, rev_offset = 28.
/// Formula: V = ((raw - offset) / 4095) * 5.0;  watts = V² / 0.12
fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (raw as i32 - offset).max(0) as f32;
    let v = (corrected / 4095.0) * 5.0;
    ((v * v) / 0.12) * scale
}

fn calculate_swr_watts(fwd_watts: f32, rev_watts: f32) -> f32 {
    if fwd_watts <= 0.0 || rev_watts <= 0.0 || rev_watts >= fwd_watts {
        return 1.0;
    }
    let ratio = (rev_watts / fwd_watts).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}

#[allow(dead_code)]
fn calculate_swr(forward: u16, reverse: u16) -> f32 {
    if forward == 0 || reverse == 0 || reverse >= forward {
        return 1.0;
    }

    let ratio = (reverse as f32 / forward as f32).sqrt();
    if ratio >= 0.999 {
        99.0
    } else {
        ((1.0 + ratio) / (1.0 - ratio)).max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_iq_frame_with_expected_header() {
        let frame = build_tci_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 0);
    }

    #[test]
    fn builds_tx_iq_frame_with_distinct_stream_type() {
        let frame = build_tci_tx_iq_frame(0, 192_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 192_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn builds_audio_frame_with_expected_header() {
        let frame = build_tci_audio_frame(0, 48_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn rejects_oversized_tci_mic_frames() {
        let sample_count = (MAX_TCI_MIC_FLOAT_SAMPLES + 1) as u32;
        let mut frame = vec![0u8; 64];
        write_u32_le(&mut frame, 20, sample_count);
        write_u32_le(&mut frame, 24, 2);
        frame.resize(64 + sample_count as usize * 4, 0);
        assert!(parse_tci_mic_frame(&frame).is_none());
    }

    #[test]
    fn parses_mono_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 8];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 20, 2);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 1);
        frame[64..68].copy_from_slice(&0.25f32.to_le_bytes());
        frame[68..72].copy_from_slice(&(-0.5f32).to_le_bytes());

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn parses_stereo_tci_mic_frame_with_channel_metadata() {
        let mut frame = vec![0u8; 64 + 16];
        write_u32_le(&mut frame, 4, 48_000);
        write_u32_le(&mut frame, 20, 4);
        write_u32_le(&mut frame, 24, 2);
        write_u32_le(&mut frame, 28, 2);
        for (index, sample) in [0.25f32, -0.25, 0.5, -0.5].iter().enumerate() {
            let offset = 64 + index * 4;
            frame[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
        }

        let parsed = parse_tci_mic_frame(&frame).unwrap();
        assert_eq!(parsed.sample_rate_hz, 48_000);
        assert_eq!(parsed.channels, 2);
        assert_eq!(parsed.samples, vec![0.25, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn websocket_config_limits_inbound_message_size() {
        let config = tci_websocket_config();
        assert_eq!(config.max_message_size, Some(MAX_TCI_INBOUND_MESSAGE_BYTES));
        assert_eq!(config.max_frame_size, Some(MAX_TCI_INBOUND_FRAME_BYTES));
    }

    #[test]
    fn swr_formula_is_reasonable() {
        assert!((calculate_swr(1000, 0) - 1.0).abs() < 0.01);
        assert!(calculate_swr(1000, 250) > 1.0);
    }

    #[test]
    fn parses_boolish_tci_values() {
        assert_eq!(parse_tci_bool("true"), Some(true));
        assert_eq!(parse_tci_bool("0"), Some(false));
        assert_eq!(parse_tci_bool("bogus"), None);
    }

    #[test]
    fn parses_saturn_ping_command() {
        let (tx, rx) = mpsc::channel();
        let client_state = Arc::new(Mutex::new(ClientState::default()));

        parse_tci_command("saturn_ping:probe-1,123.456;", &tx, &client_state);

        match rx.try_recv().unwrap() {
            TciCommand::SaturnPing { nonce, sent_at } => {
                assert_eq!(nonce, "probe-1");
                assert_eq!(sent_at, "123.456");
            }
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn formats_tx_power_trip_fault_message() {
        assert_eq!(
            tx_power_trip_fault_message(126.34, 110.0),
            "tx_fault:0,power_trip,126.3,110.0;"
        );
    }

    #[test]
    fn formats_remote_client_role_message() {
        assert_eq!(
            remote_client_role_message(42),
            "remote_client_role:0,operator,42;"
        );
    }

    #[test]
    fn initial_snapshot_includes_remote_tx_rf_state() {
        let model = RadioModel::new(2, 14_200_000, 0, 192, 24, 2048, true, 4096, true);
        let disabled = initial_snapshot_messages(&model, false, 7);
        let enabled = initial_snapshot_messages(&model, true, 8);

        assert!(disabled.contains(&"remote_tx_rf_enabled:0,false;".to_string()));
        assert!(enabled.contains(&"remote_tx_rf_enabled:0,true;".to_string()));
        assert!(disabled.contains(&"remote_client_role:0,operator,7;".to_string()));
        assert!(enabled.contains(&"remote_client_role:0,operator,8;".to_string()));
    }
}
