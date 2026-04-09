use std::io;
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tungstenite::error::Error as WsError;
use tungstenite::{accept, Message};

use crate::config::BridgeConfig;
use crate::radio_model::{DemodMode, RadioModel};

#[derive(Clone, Debug)]
pub enum TciCommand {
    SetVfoA(u32),
    SetVfoB(u32),
    SetIqCenter(u32),
    SetMode(DemodMode),
    SetFilterBand { low_hz: i32, high_hz: i32 },
    SetRxAdc(u8),
    SetRxAntenna(u8),
    SetRxVolume(f64),
    SetIqSampleRate(u32),
    SetIqStreaming,
    RequestSmeter,
    SetAudioStreaming(bool),
    SetAudioSampleRate(u32),
    SetAudioFrameSamples(u32),
    SetAudioChannels(u32),
}

#[derive(Clone, Debug)]
enum OutboundMessage {
    Text(String),
    IqFrame {
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

pub struct TciFrontend {
    command_rx: Receiver<TciCommand>,
    outbound_tx: Arc<Mutex<Option<SyncSender<OutboundMessage>>>>,
    client_state: Arc<Mutex<ClientState>>,
    _accept_thread: JoinGuard,
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

        let outbound_slot = outbound_tx.clone();
        let client_flags = client_state.clone();
        let handle = thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        println!("saturn-bridge: TCI client connected from {addr}");
                        let _ = stream.set_nonblocking(true);
                        match accept(stream) {
                            Ok(mut websocket) => {
                                let (client_tx, client_rx) = mpsc::sync_channel::<OutboundMessage>(256);
                                {
                                    let mut slot = outbound_slot.lock().unwrap();
                                    *slot = Some(client_tx.clone());
                                }
                                {
                                    let mut flags = client_flags.lock().unwrap();
                                    flags.iq_stream_enabled = false;
                                    flags.audio_stream_enabled = false;
                                    flags.audio_sample_rate_hz = 48_000;
                                    flags.audio_frame_float_count = 2048;
                                    flags.audio_channels = 2;
                                }

                                for message in initial_snapshot_messages(&radio_model.lock().unwrap()) {
                                    let _ = client_tx.send(OutboundMessage::Text(message));
                                }

                                loop {
                                    let mut pending_flush = false;
                                    match websocket.read() {
                                        Ok(message) => {
                                            if !handle_incoming_message(message, &command_tx, &client_flags) {
                                                break;
                                            }
                                        }
                                        Err(WsError::Io(error))
                                            if error.kind() == io::ErrorKind::WouldBlock =>
                                        {
                                        }
                                        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => break,
                                        Err(error) => {
                                            eprintln!("saturn-bridge: TCI websocket read error: {error}");
                                            break;
                                        }
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
                                                        eprintln!(
                                                            "saturn-bridge: TCI websocket send error: {error}"
                                                        );
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
                                            Err(WsError::Io(error))
                                                if error.kind() == io::ErrorKind::WouldBlock => {}
                                            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => break,
                                            Err(error) => {
                                                eprintln!(
                                                    "saturn-bridge: TCI websocket flush error: {error}"
                                                );
                                                break;
                                            }
                                        }
                                    }

                                    thread::sleep(Duration::from_millis(10));
                                }
                            }
                            Err(error) => {
                                eprintln!("saturn-bridge: TCI websocket accept failed: {error}");
                            }
                        }

                        {
                            let mut slot = outbound_slot.lock().unwrap();
                            *slot = None;
                        }
                        {
                            let mut flags = client_flags.lock().unwrap();
                            flags.iq_stream_enabled = false;
                            flags.audio_stream_enabled = false;
                        }
                        println!("saturn-bridge: TCI client disconnected");
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(error) => {
                        eprintln!("saturn-bridge: TCI listener error: {error}");
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        });

        Ok(Self {
            command_rx,
            outbound_tx,
            client_state,
            _accept_thread: JoinGuard { handle },
        })
    }

    pub fn try_recv_command(&self) -> Option<TciCommand> {
        self.command_rx.try_recv().ok()
    }

    pub fn publish_radio_state(&self, model: &RadioModel) {
        self.send_text(format!("vfo:0,0,{};", model.desired.vfo_a_hz));
        self.send_text(format!("vfo:0,1,{};", model.desired.vfo_b_hz));
        self.send_text(format!("dds:0,{};", model.desired.iq_center_hz));
        self.send_text(format!("rx_adc:0,{};", model.desired.ddc0_adc));
        self.send_text(format!("rx_antenna:0,{};", model.desired.rx_antenna.max(1).min(3)));
        self.send_text(format!("iq_samplerate:{};", model.desired.ddc0_sample_rate_khz as u32 * 1000));
        self.send_text(format!("modulation:0,{};", model.desired.mode));
        self.send_text(format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db));
        self.send_text(format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ));
        self.send_text(format!("trx:0,{};", model.desired.tx_enabled));
        self.send_text("tune:0,false;".to_string());
        self.publish_telemetry(model);
    }

    pub fn publish_telemetry(&self, model: &RadioModel) {
        if let Some(meter_dbm) = model.observed.ddc0_meter_dbm {
            self.send_text(format!("rx_smeter:0,0,{meter_dbm:.1};"));
        }
        if let Some(packet) = model.observed.high_priority.as_ref() {
            self.send_text(format!("tx_power:0,{:.1};", packet.forward_power as f32));
            self.send_text(format!("swr:0,{:.2};", calculate_swr(packet.forward_power, packet.reverse_power)));
        }
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
        if let Some(sender) = self.outbound_tx.lock().unwrap().as_ref().cloned() {
            match sender.try_send(message) {
                Ok(()) => {}
                Err(TrySendError::Full(message)) => {
                    if matches!(message, OutboundMessage::Text(_)) {
                        eprintln!("saturn-bridge: dropping outbound TCI text due to websocket backpressure");
                    }
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }
}

fn initial_snapshot_messages(model: &RadioModel) -> Vec<String> {
    vec![
        "ready;".to_string(),
        format!("vfo:0,0,{};", model.desired.vfo_a_hz),
        format!("vfo:0,1,{};", model.desired.vfo_b_hz),
        format!("dds:0,{};", model.desired.iq_center_hz),
        format!("rx_adc:0,{};", model.desired.ddc0_adc),
        format!("rx_antenna:0,{};", model.desired.rx_antenna.max(1).min(3)),
        format!("iq_samplerate:{};", model.desired.ddc0_sample_rate_khz as u32 * 1000),
        format!("modulation:0,{};", model.desired.mode),
        format!("rx_volume:0,0,{:.1};", model.desired.rx_volume_db),
        format!(
            "rx_filter_band:0,{},{};",
            model.desired.filter_low_hz, model.desired.filter_high_hz
        ),
        format!("trx:0,{};", model.desired.tx_enabled),
        "tune:0,false;".to_string(),
        "audio_samplerate:48000;".to_string(),
    ]
}

fn handle_incoming_message(
    message: Message,
    command_tx: &Sender<TciCommand>,
    client_state: &Arc<Mutex<ClientState>>,
) -> bool {
    match message {
        Message::Text(text) => {
            for command in text.split(';').map(str::trim).filter(|part| !part.is_empty()) {
                parse_tci_command(command, command_tx, client_state);
            }
            true
        }
        Message::Binary(_) => true,
        Message::Ping(payload) => {
            let _ = command_tx.send(TciCommand::RequestSmeter);
            let _ = payload;
            true
        }
        Message::Close(_) => false,
        _ => true,
    }
}

fn parse_tci_command(command: &str, command_tx: &Sender<TciCommand>, client_state: &Arc<Mutex<ClientState>>) {
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
        "modulation" => {
            if args.len() >= 2 {
                let _ = command_tx.send(TciCommand::SetMode(DemodMode::from_tci(args[1])));
            }
        }
        "rx_filter_band" => {
            if args.len() >= 3 {
                if let (Ok(low_hz), Ok(high_hz)) = (args[1].trim().parse::<i32>(), args[2].trim().parse::<i32>())
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
        "rx_adc" => {
            let adc_arg = if args.len() >= 2 { args.get(1) } else { args.first() };
            if let Some(adc_text) = adc_arg {
                if let Ok(adc) = adc_text.trim().parse::<u8>() {
                    let _ = command_tx.send(TciCommand::SetRxAdc(adc.min(2)));
                }
            }
        }
        "rx_antenna" => {
            let antenna_arg = if args.len() >= 2 { args.get(1) } else { args.first() };
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
        _ => {}
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
        } => {
            websocket.send(Message::Binary(build_tci_iq_frame(
                receiver,
                sample_rate,
                &iq_samples,
            )))
        }
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
    fn builds_audio_frame_with_expected_header() {
        let frame = build_tci_audio_frame(0, 48_000, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(frame.len(), 64 + 16);
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(frame[24..28].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn swr_formula_is_reasonable() {
        assert!((calculate_swr(1000, 0) - 1.0).abs() < 0.01);
        assert!(calculate_swr(1000, 250) > 1.0);
    }
}
