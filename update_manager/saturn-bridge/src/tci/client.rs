use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::error::Error as WsError;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_hdr_with_config, Message};

use crate::radio_model::{NoiseReductionMode, RadioModel};
use crate::sync_ext::MutexExt;
use crate::tx_codec::{tx_codec_frame_is_stale, TxCodecDecoder, TxCodecRuntimeFlags, TxMicCodec};

use super::*;

#[derive(Clone, Debug)]
pub(crate) struct ClientState {
    pub(crate) iq_stream_enabled: bool,
    pub(crate) audio_stream_enabled: bool,
    pub(crate) audio_sample_rate_hz: u32,
    pub(crate) audio_frame_float_count: u32,
    pub(crate) audio_channels: u32,
    pub(crate) audio_seq_gap_count: u64,
    pub(crate) tx_uplink_degraded: bool,
    pub(crate) tx_mic_browser_last_seq: u32,
    pub(crate) tx_mic_browser_dropped_count: u64,
    pub(crate) tx_uplink_buffered_bytes: u64,
    pub(crate) tx_uplink_buffered_high_watermark_bytes: u64,
    pub(crate) tx_mic_last_arrived_seq: u32,
    pub(crate) tx_mic_seq_gap_count: u64,
    pub(crate) tx_mic_last_arrived_at: Option<Instant>,
    pub(crate) tx_codec_caps: BTreeSet<TxMicCodec>,
    pub(crate) tx_codec_active: TxMicCodec,
    pub(crate) tx_codec_negotiated_at: Option<Instant>,
    pub(crate) tx_codec_runtime_flags: TxCodecRuntimeFlags,
    pub(crate) tx_codec_decoder: Arc<Mutex<TxCodecDecoder>>,
    pub(crate) tx_codec_degraded: bool,
    pub(crate) tx_codec_decode_error_count: u64,
    pub(crate) tx_codec_decode_error_window_started_at: Option<Instant>,
    pub(crate) tx_codec_decode_error_window_count: u64,
    pub(crate) tx_codec_stale_drop_count: u64,
    pub(crate) tx_codec_release_flush_count: u64,
    pub(crate) split: Option<SplitClientMetadata>,
    /// Lane declared by the websocket request path (`/control`, `/media`) at
    /// connect time, before the in-band `session_lane` command arrives. The
    /// same-origin proxy dials with these paths; direct clients use `/` and
    /// stay on the legacy any-lane behavior.
    pub(crate) connect_lane_hint: Option<SplitSocketKind>,
}

impl Default for ClientState {
    fn default() -> Self {
        Self::with_tx_codec_runtime_flags(TxCodecRuntimeFlags::default())
    }
}

impl ClientState {
    pub(crate) fn with_tx_codec_runtime_flags(tx_codec_runtime_flags: TxCodecRuntimeFlags) -> Self {
        Self {
            iq_stream_enabled: false,
            audio_stream_enabled: false,
            audio_sample_rate_hz: 48_000,
            audio_frame_float_count: 2048,
            audio_channels: 2,
            audio_seq_gap_count: 0,
            tx_uplink_degraded: false,
            tx_mic_browser_last_seq: 0,
            tx_mic_browser_dropped_count: 0,
            tx_uplink_buffered_bytes: 0,
            tx_uplink_buffered_high_watermark_bytes: 0,
            tx_mic_last_arrived_seq: 0,
            tx_mic_seq_gap_count: 0,
            tx_mic_last_arrived_at: None,
            tx_codec_caps: BTreeSet::from([TxMicCodec::Pcm]),
            tx_codec_active: TxMicCodec::Pcm,
            tx_codec_negotiated_at: None,
            tx_codec_runtime_flags,
            tx_codec_decoder: Arc::new(Mutex::new(TxCodecDecoder::new_with_flags(
                TxMicCodec::Pcm,
                tx_codec_runtime_flags,
            ))),
            tx_codec_degraded: false,
            tx_codec_decode_error_count: 0,
            tx_codec_decode_error_window_started_at: None,
            tx_codec_decode_error_window_count: 0,
            tx_codec_stale_drop_count: 0,
            tx_codec_release_flush_count: 0,
            split: None,
            connect_lane_hint: None,
        }
    }
}

pub(crate) fn lane_hint_for_request_path(path: &str) -> Option<SplitSocketKind> {
    match path {
        "/control" => Some(SplitSocketKind::Control),
        "/media" => Some(SplitSocketKind::Media),
        _ => None,
    }
}

#[derive(Clone)]
pub(crate) struct ClientConnection {
    pub(crate) outbound: Arc<ClientOutbound>,
    pub(crate) state: ClientState,
}

pub(crate) type ClientRegistry = Arc<Mutex<BTreeMap<u64, ClientConnection>>>;

pub(crate) const MAX_TCI_INBOUND_MESSAGE_BYTES: usize = 256 * 1024;

pub(crate) const MAX_TCI_INBOUND_FRAME_BYTES: usize = 256 * 1024;

pub(crate) const TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT: u64 = 10;

pub(crate) const TX_CODEC_DECODE_ERROR_WINDOW: Duration = Duration::from_secs(1);

pub(crate) fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    client_id: u64,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    operator_control_at: &Arc<Mutex<Option<Instant>>>,
    radio_model: &Arc<Mutex<RadioModel>>,
    drop_count: &Arc<AtomicU64>,
    remote_tx_rf_enabled: bool,
    tx_codec_runtime_flags: TxCodecRuntimeFlags,
) {
    let _ = stream.set_nonblocking(true);
    let mut connect_lane_hint = None;
    let accept_result = accept_hdr_with_config(
        stream,
        |request: &Request, response: Response| {
            connect_lane_hint = lane_hint_for_request_path(request.uri().path());
            Ok(response)
        },
        Some(tci_websocket_config()),
    );
    match accept_result {
        Ok(mut websocket) => {
            let outbound = ClientOutbound::new();
            let (role, first_client, client_count) = register_client(
                clients,
                operator_client_id,
                client_id,
                outbound.clone(),
                tx_codec_runtime_flags,
                connect_lane_hint,
            );
            println!(
                "saturn-bridge: TCI client {client_id} assigned {} role ({client_count} connected){}",
                role.as_tci(),
                match connect_lane_hint {
                    Some(SplitSocketKind::Control) => " [control path]",
                    Some(SplitSocketKind::Media) => " [media path]",
                    None => "",
                }
            );

            // A media-lane socket must never carry text; the paired control
            // socket receives the snapshot instead.
            if connect_lane_hint != Some(SplitSocketKind::Media) {
                for message in initial_snapshot_messages(
                    &radio_model.lock_unpoisoned(),
                    remote_tx_rf_enabled,
                    client_id,
                    role,
                ) {
                    let drops = outbound.enqueue(OutboundMessage::Text(message));
                    drop_count.fetch_add(drops, Ordering::Relaxed);
                }
            }
            outbound.mark_writer_started();

            if first_client {
                let _ = command_tx.send(TciCommand::ClientConnected);
            }

            let mut bulk_pause_until: Option<Instant> = None;
            loop {
                let mut pending_flush = false;
                let mut client_closed = false;
                for _ in 0..64 {
                    match websocket.read() {
                        Ok(message) => {
                            if !handle_incoming_message(
                                message,
                                command_tx,
                                clients,
                                operator_client_id,
                                operator_control_at,
                                client_id,
                            ) {
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
                    let now = Instant::now();
                    if bulk_pause_until.map(|until| now >= until).unwrap_or(false) {
                        bulk_pause_until = None;
                    }

                    let tcp_outq_bytes = tcp_outq_bytes(websocket.get_ref()).unwrap_or(0);
                    outbound.record_tcp_outq_high_watermark(tcp_outq_bytes);
                    let tcp_outq_allows_bulk = bulk_allowed_for_tcp_outq(tcp_outq_bytes);
                    if !tcp_outq_allows_bulk {
                        outbound.record_send_blocked(Duration::from_millis(2));
                        bulk_pause_until =
                            Some(now + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS));
                    }

                    let allow_bulk = bulk_pause_until.is_none() && tcp_outq_allows_bulk;
                    let Some(item) = outbound.next_message(allow_bulk) else {
                        break;
                    };
                    let closes_client = matches!(&item.message, OutboundMessage::Close);
                    match send_outbound(&mut websocket, &item.message) {
                        Ok(()) => {
                            outbound.record_write(item.class, item.enqueued_at.elapsed());
                            pending_flush = true;
                            if closes_client {
                                client_closed = true;
                                break;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            pending_flush = true;
                            outbound.record_send_blocked(Duration::from_millis(2));
                            bulk_pause_until = Some(
                                Instant::now() + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS),
                            );
                            if item.class.is_never_drop() {
                                outbound.requeue_front(item);
                            } else {
                                outbound.record_bulk_send_drop(item.class);
                                drop_count.fetch_add(1, Ordering::Relaxed);
                            }
                            break;
                        }
                        Err(error) => {
                            eprintln!("saturn-bridge: TCI websocket send error to {addr}: {error}");
                            pending_flush = true;
                            client_closed = true;
                            break;
                        }
                    }
                }
                if client_closed {
                    break;
                }

                if pending_flush {
                    match websocket.flush() {
                        Ok(()) => {
                            if bulk_pause_until
                                .map(|until| Instant::now() >= until)
                                .unwrap_or(false)
                            {
                                bulk_pause_until = None;
                            }
                        }
                        Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                            outbound.record_send_blocked(Duration::from_millis(2));
                            bulk_pause_until = Some(
                                Instant::now() + Duration::from_millis(BULK_BACKPRESSURE_PAUSE_MS),
                            );
                        }
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

            let disconnect = unregister_client(clients, operator_client_id, client_id);
            if disconnect.was_operator {
                *operator_control_at.lock_unpoisoned() = None;
                let _ = command_tx.send(TciCommand::SetTxEnabled(false));
            }
            if disconnect.split_media_loss_forces_rx {
                let _ = command_tx.send(TciCommand::SetTxEnabled(false));
            }
            if let Some(peer_id) = disconnect.split_closed_peer {
                println!(
                    "saturn-bridge: closed split peer media client {peer_id} after control disconnect"
                );
            }
            if let Some(promoted_id) = disconnect.promoted_operator {
                send_role_to_client(
                    clients,
                    promoted_id,
                    TciClientRole::Operator,
                    &format!("saturn-bridge: TCI promoted client {promoted_id} to operator"),
                );
            }
            if disconnect.remaining_clients == 0 {
                let _ = command_tx.send(TciCommand::ClientDisconnected);
            }
            println!(
                "saturn-bridge: TCI client {client_id} disconnected from {addr} ({} connected)",
                disconnect.remaining_clients
            );
        }
        Err(error) => {
            eprintln!("saturn-bridge: TCI websocket accept failed from {addr}: {error}");
        }
    }
}

pub(crate) struct ClientDisconnect {
    pub(crate) was_operator: bool,
    pub(crate) split_media_loss_forces_rx: bool,
    pub(crate) split_closed_peer: Option<u64>,
    pub(crate) promoted_operator: Option<u64>,
    pub(crate) remaining_clients: usize,
}

pub(crate) fn register_client(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    client_id: u64,
    outbound: Arc<ClientOutbound>,
    tx_codec_runtime_flags: TxCodecRuntimeFlags,
    connect_lane_hint: Option<SplitSocketKind>,
) -> (TciClientRole, bool, usize) {
    let mut clients = clients.lock_unpoisoned();
    let first_client = clients.is_empty();
    let mut state = ClientState::with_tx_codec_runtime_flags(tx_codec_runtime_flags);
    state.connect_lane_hint = connect_lane_hint;
    clients.insert(client_id, ClientConnection { outbound, state });

    let current_operator = operator_client_id.load(Ordering::SeqCst);
    let role = if current_operator == 0 || !clients.contains_key(&current_operator) {
        operator_client_id.store(client_id, Ordering::SeqCst);
        TciClientRole::Operator
    } else {
        TciClientRole::Viewer
    };
    (role, first_client, clients.len())
}

pub(crate) fn unregister_client(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    client_id: u64,
) -> ClientDisconnect {
    let mut clients = clients.lock_unpoisoned();
    let current_operator = operator_client_id.load(Ordering::SeqCst);
    let split_media_loss_forces_rx =
        split_media_client_paired_with_operator_in_clients(&clients, current_operator, client_id);
    let split_closed_peer =
        queue_split_media_peer_close_for_control_in_clients(&clients, client_id);
    clients.remove(&client_id);

    let was_operator = current_operator == client_id;
    let mut promoted_operator = None;
    if was_operator {
        if let Some((&next_operator, _)) = clients
            .iter()
            .find(|(_, client)| !client_is_split_media(client))
        {
            operator_client_id.store(next_operator, Ordering::SeqCst);
            promoted_operator = Some(next_operator);
        } else {
            operator_client_id.store(0, Ordering::SeqCst);
        }
    }

    ClientDisconnect {
        was_operator,
        split_media_loss_forces_rx,
        split_closed_peer,
        promoted_operator,
        remaining_clients: clients.len(),
    }
}

pub(crate) fn send_role_to_client(
    clients: &ClientRegistry,
    client_id: u64,
    role: TciClientRole,
    log_message: &str,
) {
    if let Some(outbound) = clients
        .lock_unpoisoned()
        .get(&client_id)
        .map(|client| client.outbound.clone())
    {
        let _ = outbound.enqueue(OutboundMessage::SafetyText(remote_client_role_message(
            client_id, role,
        )));
        println!("{log_message}");
    }
}

pub(crate) fn initial_snapshot_messages(
    model: &RadioModel,
    remote_tx_rf_enabled: bool,
    client_id: u64,
    role: TciClientRole,
) -> Vec<String> {
    vec![
        "ready;".to_string(),
        remote_client_role_message(client_id, role),
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
        format!("rx_nr2_gain_method:0,{};", model.desired.rx_nr2_gain_method),
        format!("rx_nr2_npe_method:0,{};", model.desired.rx_nr2_npe_method),
        format!(
            "rx_nr2_post_filter:0,{};",
            model.desired.rx_nr2_post_filter_enabled
        ),
        format!("rx_wbfm_supported:0,{};", crate::wdsp::wbfm_supported()),
        format!("rx_wbfm_deemphasis:0,{};", model.desired.rx_wbfm_deemphasis),
        format!(
            "rx_wbfm_stereo:0,{};",
            model.observed.rx_wbfm_stereo_detected
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
        format!(
            "tx_phase_rotator:0,{};",
            model.desired.tx_phase_rotator_enabled
        ),
        format!(
            "tx_phase_rotator_auto:0,{};",
            model.desired.tx_phase_rotator_auto
        ),
        format!(
            "tx_phase_rotator_corner:0,{:.0};",
            model.desired.tx_phase_rotator_corner_hz
        ),
        format!("tx_puresignal:0,{};", model.desired.pure_signal_enabled),
        format!(
            "tx_puresignal_auto_attenuate:0,{};",
            model.desired.pure_signal_auto_attenuate
        ),
        format!(
            "tx_puresignal_attenuation:0,{};",
            model.desired.pure_signal_attenuation_db
        ),
        format!(
            "tx_puresignal_state:0,{};",
            model.observed.pure_signal_state
        ),
        format!(
            "tx_puresignal_feedback:0,{};",
            model.observed.pure_signal_feedback_level
        ),
        format!(
            "tx_puresignal_calibration_count:0,{};",
            model.observed.pure_signal_calibration_count
        ),
        format!(
            "tx_puresignal_correcting:0,{};",
            model.observed.pure_signal_correcting
        ),
        format!(
            "tx_puresignal_max_tx:0,{:.4};",
            model.observed.pure_signal_max_tx
        ),
        format!(
            "tx_puresignal_feedback_packets:0,{};",
            model.observed.pure_signal_feedback_packets
        ),
        format!(
            "tx_puresignal_feedback_gaps:0,{};",
            model.observed.pure_signal_feedback_gaps
        ),
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

pub(crate) fn handle_incoming_message(
    message: Message,
    command_tx: &Sender<TciCommand>,
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    operator_control_at: &Arc<Mutex<Option<Instant>>>,
    client_id: u64,
) -> bool {
    let current_operator_client_id = operator_client_id.load(Ordering::SeqCst);
    let is_operator = current_operator_client_id == client_id;
    match message {
        Message::Text(text) => {
            let received_at = Instant::now();
            for command in text
                .split(';')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                parse_tci_command_with_roles(
                    command,
                    command_tx,
                    clients,
                    client_id,
                    operator_client_id.load(Ordering::SeqCst) == client_id,
                    Some(operator_client_id),
                );
            }
            if operator_client_id.load(Ordering::SeqCst) == client_id {
                *operator_control_at.lock_unpoisoned() = Some(received_at);
            }
            true
        }
        Message::Binary(data) => {
            if is_operator
                || split_media_client_can_supply_mic(
                    clients,
                    current_operator_client_id,
                    client_id,
                    Instant::now(),
                )
            {
                match parse_tci_mic_frame_result_for_client(clients, client_id, &data) {
                    Ok(frame) => {
                        if tx_codec_frame_is_stale(frame.received_at, Instant::now()) {
                            record_client_tx_codec_stale_drop(clients, client_id);
                            return true;
                        }
                        record_client_tx_mic_frame(
                            clients,
                            client_id,
                            frame.sequence,
                            frame.received_at,
                        );
                        let _ = command_tx.send(TciCommand::MicAudioFrame(frame));
                    }
                    Err(TciMicFrameParseError::NotMicFrame) => {}
                    Err(_) => {
                        let action = record_client_tx_codec_decode_error(clients, client_id);
                        if action.force_rx {
                            send_safety_text_to_client_or_control(
                                clients,
                                client_id,
                                tx_codec_decode_fault_message(action.count, action.limit),
                            );
                            let _ = command_tx.send(TciCommand::SetTxEnabled(false));
                        }
                    }
                }
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

pub(crate) fn set_client_iq_stream_enabled(
    clients: &ClientRegistry,
    client_id: u64,
    enabled: bool,
) -> bool {
    let mut clients = clients.lock_unpoisoned();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.iq_stream_enabled = enabled;
    }
    // Mirror to the paired media client so binary IQ frames have
    // a destination on the media lane. client_wants_outbound_message then
    // routes RX IQ to the media client only.
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.iq_stream_enabled = enabled;
        }
    }
    clients
        .values()
        .any(|client| client.state.iq_stream_enabled)
}

pub(crate) fn set_client_audio_stream_enabled(
    clients: &ClientRegistry,
    client_id: u64,
    enabled: bool,
) -> bool {
    let mut clients = clients.lock_unpoisoned();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_stream_enabled = enabled;
    }
    // Mirror to the paired media client.
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_stream_enabled = enabled;
        }
    }
    clients
        .values()
        .any(|client| client.state.audio_stream_enabled)
}

pub(crate) fn set_client_audio_sample_rate(
    clients: &ClientRegistry,
    client_id: u64,
    sample_rate_hz: u32,
) {
    let mut clients = clients.lock_unpoisoned();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_sample_rate_hz = sample_rate_hz;
    }
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_sample_rate_hz = sample_rate_hz;
        }
    }
}

pub(crate) fn set_client_audio_frame_float_count(
    clients: &ClientRegistry,
    client_id: u64,
    sample_count: u32,
) {
    let mut clients = clients.lock_unpoisoned();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_frame_float_count = sample_count;
    }
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_frame_float_count = sample_count;
        }
    }
}

pub(crate) fn set_client_audio_channels(clients: &ClientRegistry, client_id: u64, channels: u32) {
    let mut clients = clients.lock_unpoisoned();
    if let Some(client) = clients.get_mut(&client_id) {
        client.state.audio_channels = channels;
    }
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            media.state.audio_channels = channels;
        }
    }
}

pub(crate) fn set_client_audio_seq_gap_count(clients: &ClientRegistry, client_id: u64, gaps: u64) {
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.audio_seq_gap_count = gaps;
    }
}

pub(crate) fn set_client_tx_uplink_stats(
    clients: &ClientRegistry,
    client_id: u64,
    degraded: bool,
    last_seq: u32,
    dropped_count: u64,
    buffered_bytes: u64,
    high_watermark_bytes: u64,
) {
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.tx_uplink_degraded = degraded;
        client.state.tx_mic_browser_last_seq = last_seq;
        client.state.tx_mic_browser_dropped_count = dropped_count;
        client.state.tx_uplink_buffered_bytes = buffered_bytes;
        client.state.tx_uplink_buffered_high_watermark_bytes =
            high_watermark_bytes.max(buffered_bytes);
    }
}

pub(crate) fn reset_client_tx_codec_state(
    client: &mut ClientConnection,
    caps: BTreeSet<TxMicCodec>,
    selected: Option<TxMicCodec>,
    now: Instant,
) {
    client.state.tx_codec_caps = caps;
    client.state.tx_codec_decode_error_window_started_at = None;
    client.state.tx_codec_decode_error_window_count = 0;
    if let Some(codec) = selected {
        client.state.tx_codec_active = codec;
        client.state.tx_codec_negotiated_at = Some(now);
        client.state.tx_codec_decoder = Arc::new(Mutex::new(TxCodecDecoder::new_with_flags(
            codec,
            client.state.tx_codec_runtime_flags,
        )));
        client.state.tx_codec_degraded = false;
    } else {
        client.state.tx_codec_negotiated_at = None;
    }
}

pub(crate) fn set_client_tx_codec_caps(
    clients: &ClientRegistry,
    client_id: u64,
    caps: BTreeSet<TxMicCodec>,
) -> Option<TxMicCodec> {
    let mut clients = clients.lock_unpoisoned();
    let flags = clients
        .get(&client_id)
        .map(|client| client.state.tx_codec_runtime_flags)
        .unwrap_or_default();
    let selected = select_tx_codec(&caps, flags);
    let now = Instant::now();
    if let Some(client) = clients.get_mut(&client_id) {
        reset_client_tx_codec_state(client, caps.clone(), selected, now);
    }
    // Codec negotiation happens on the control lane, but TX mic
    // binary frames arrive on the paired media lane. Mirror the accepted state
    // so the media client owns the decoder that will actually consume frames.
    if let Some(media_id) = split_paired_media_client_id(&clients, client_id) {
        if let Some(media) = clients.get_mut(&media_id) {
            reset_client_tx_codec_state(media, caps, selected, now);
        }
    }
    selected
}

pub(crate) fn send_text_to_client(clients: &ClientRegistry, client_id: u64, text: String) {
    if let Some(outbound) = clients
        .lock()
        .unwrap()
        .get(&client_id)
        .map(|client| client.outbound.clone())
    {
        let _ = outbound.enqueue(OutboundMessage::Text(text));
    }
}

pub(crate) fn send_safety_text_to_client_or_control(
    clients: &ClientRegistry,
    client_id: u64,
    text: String,
) {
    let outbound = {
        let clients = clients.lock_unpoisoned();
        let target_client_id = clients
            .get(&client_id)
            .and_then(|client| client.state.split.as_ref())
            .filter(|metadata| metadata.lane == Some(SplitSocketKind::Media))
            .and_then(|metadata| {
                split_session_pair_in_clients(&clients, &metadata.session_id)
                    .map(|pair| pair.control_client_id)
            })
            .unwrap_or(client_id);
        clients
            .get(&target_client_id)
            .map(|client| client.outbound.clone())
    };
    if let Some(outbound) = outbound {
        let _ = outbound.enqueue(OutboundMessage::SafetyText(text));
    }
}

pub(crate) fn reset_client_tx_uplink_attempt(clients: &ClientRegistry, client_id: u64) {
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.tx_uplink_degraded = false;
        client.state.tx_mic_browser_last_seq = 0;
        client.state.tx_mic_browser_dropped_count = 0;
        client.state.tx_uplink_buffered_bytes = 0;
        client.state.tx_uplink_buffered_high_watermark_bytes = 0;
        client.state.tx_mic_last_arrived_seq = 0;
        client.state.tx_mic_seq_gap_count = 0;
        client.state.tx_mic_last_arrived_at = None;
        client.state.tx_codec_decode_error_window_started_at = None;
        client.state.tx_codec_decode_error_window_count = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TxCodecDecodeFaultAction {
    pub(crate) force_rx: bool,
    pub(crate) count: u64,
    pub(crate) limit: u64,
}

pub(crate) fn record_client_tx_codec_decode_error(
    clients: &ClientRegistry,
    client_id: u64,
) -> TxCodecDecodeFaultAction {
    record_client_tx_codec_decode_error_at(clients, client_id, Instant::now())
}

pub(crate) fn record_client_tx_codec_decode_error_at(
    clients: &ClientRegistry,
    client_id: u64,
    now: Instant,
) -> TxCodecDecodeFaultAction {
    let limit = TX_CODEC_DECODE_ERROR_FORCE_RX_LIMIT;
    let mut count = 0;
    let mut force_rx = false;
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.tx_codec_decode_error_count =
            client.state.tx_codec_decode_error_count.saturating_add(1);
        let reset_window = client
            .state
            .tx_codec_decode_error_window_started_at
            .map(|started_at| {
                now.saturating_duration_since(started_at) > TX_CODEC_DECODE_ERROR_WINDOW
            })
            .unwrap_or(true);
        if reset_window {
            client.state.tx_codec_decode_error_window_started_at = Some(now);
            client.state.tx_codec_decode_error_window_count = 0;
        }
        client.state.tx_codec_decode_error_window_count = client
            .state
            .tx_codec_decode_error_window_count
            .saturating_add(1);
        count = client.state.tx_codec_decode_error_window_count;
        if count >= limit && !client.state.tx_codec_degraded {
            client.state.tx_codec_degraded = true;
            force_rx = true;
        }
    }
    TxCodecDecodeFaultAction {
        force_rx,
        count,
        limit,
    }
}

pub(crate) fn record_client_tx_codec_stale_drop(clients: &ClientRegistry, client_id: u64) {
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.tx_codec_stale_drop_count =
            client.state.tx_codec_stale_drop_count.saturating_add(1);
    }
}

pub(crate) fn flush_client_tx_codec_decode_queue(
    clients: &ClientRegistry,
    operator_client_id: u64,
) -> bool {
    if operator_client_id == 0 {
        return false;
    }
    let mut clients = clients.lock_unpoisoned();
    let Some(operator) = clients.get_mut(&operator_client_id) else {
        return false;
    };
    operator.state.tx_codec_release_flush_count = operator
        .state
        .tx_codec_release_flush_count
        .saturating_add(1);
    true
}

pub(crate) fn next_wrapped_sequence(sequence: u32) -> u32 {
    let next = sequence.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

pub(crate) fn record_client_tx_mic_frame(
    clients: &ClientRegistry,
    client_id: u64,
    sequence: u32,
    received_at: Instant,
) {
    if let Some(client) = clients.lock_unpoisoned().get_mut(&client_id) {
        client.state.tx_mic_last_arrived_at = Some(received_at);
        if sequence == 0 {
            return;
        }
        if client.state.tx_mic_last_arrived_seq != 0
            && sequence != next_wrapped_sequence(client.state.tx_mic_last_arrived_seq)
        {
            client.state.tx_mic_seq_gap_count = client.state.tx_mic_seq_gap_count.saturating_add(1);
        }
        client.state.tx_mic_last_arrived_seq = sequence;
    }
}

pub(crate) fn tci_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_TCI_INBOUND_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_TCI_INBOUND_FRAME_BYTES))
}
