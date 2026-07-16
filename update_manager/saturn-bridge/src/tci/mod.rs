use std::collections::BTreeMap;
use std::io;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::BridgeConfig;
use crate::radio_model::{NoiseReductionMode, RadioModel};
use crate::sync_ext::MutexExt;
use crate::tx_codec::TxCodecRuntimeFlags;

mod client;
mod outbound;
mod protocol;
mod session_pair;

#[cfg(test)]
mod tests;

pub(crate) use client::*;
pub(crate) use outbound::*;
pub(crate) use protocol::*;
pub(crate) use session_pair::*;

pub struct TciFrontend {
    clients: ClientRegistry,
    operator_client_id: Arc<AtomicU64>,
    operator_control_at: Arc<Mutex<Option<Instant>>>,
    drop_count: Arc<AtomicU64>,
    display_rate_limited_count: AtomicU64,
    // TX media priority is derived from the bridge's authoritative
    // TX intent/armed/keyed state, not from a browser command. While active,
    // RX binary frames are suppressed on the media lane so uplink mic owns the
    // media TCP send buffer. Cleared automatically when TX returns to RX,
    // eliminating the stuck-flag class of bug. Single source of truth.
    tx_media_priority_active: AtomicBool,
    tx_power_meter_scale: f32,
    remote_tx_rf_enabled: bool,
    display_frame_interval: Duration,
    last_display_frame_at: Mutex<Option<Instant>>,
    rx_audio_transport_rate_hz: u32,
    rx_audio_transport_channels: u32,
    _accept_thread: JoinGuard,
}

#[derive(Clone, Copy, Debug)]
pub struct TciClientSnapshot {
    pub active: bool,
    pub iq_stream_enabled: bool,
    pub audio_stream_enabled: bool,
    pub outbound_drops: u64,
    pub safety_enqueue_to_write_p50_us: u64,
    pub safety_enqueue_to_write_p95_us: u64,
    pub safety_enqueue_to_write_p99_us: u64,
    pub control_enqueue_to_write_p50_us: u64,
    pub control_enqueue_to_write_p95_us: u64,
    pub control_enqueue_to_write_p99_us: u64,
    pub display_replaced_per_sec: u64,
    pub display_dropped_per_sec: u64,
    pub audio_dropped_per_sec: u64,
    pub audio_seq_gap_count: u64,
    pub audio_panic_drain_count: u64,
    pub send_blocked_ms: u64,
    pub outbound_high_watermark_bytes: u64,
    pub outbound_queued_bytes: u64,
    pub tcp_outq_high_watermark_bytes: u64,
    pub display_rate_limited_per_sec: u64,
    pub safety_queue_depth_overflow_count: u64,
    pub split_control_clients: u64,
    pub split_media_clients: u64,
    pub split_paired_sessions: u64,
    pub tx_uplink_degraded: bool,
    pub tx_mic_browser_dropped_count: u64,
    pub tx_uplink_buffered_bytes: u64,
    pub tx_uplink_buffered_high_watermark_bytes: u64,
    pub tx_mic_last_arrived_seq: u32,
    pub tx_mic_seq_gap_count: u64,
    pub tx_mic_age_ms: u64,
    pub tx_media_priority_active: bool,
    pub tx_codec_decode_error_count: u64,
    pub tx_codec_stale_drop_count: u64,
    pub tx_codec_release_flush_count: u64,
}

struct JoinGuard {
    #[allow(dead_code)]
    handle: thread::JoinHandle<()>,
}

impl TciFrontend {
    /// Returns the frontend plus the command stream fed by client threads.
    /// The receiver stays outside the frontend so the frontend itself is
    /// `Sync` and can be shared with the RX thread behind an `Arc`.
    pub fn bind(
        config: &BridgeConfig,
        radio_model: Arc<Mutex<RadioModel>>,
    ) -> io::Result<(Self, Receiver<TciCommand>)> {
        let listener = TcpListener::bind(config.tci_bind_addr)?;
        listener.set_nonblocking(true)?;

        let (command_tx, command_rx) = mpsc::channel();
        let clients = Arc::new(Mutex::new(BTreeMap::new()));
        let next_client_id = Arc::new(AtomicU64::new(0));
        let operator_client_id = Arc::new(AtomicU64::new(0));
        let operator_control_at = Arc::new(Mutex::new(None));
        let drop_count = Arc::new(AtomicU64::new(0));
        let remote_tx_rf_enabled = config.remote_tx_rf_enabled;
        let tx_codec_runtime_flags = TxCodecRuntimeFlags {
            opus_decode_enabled: config.tx_opus_decode_enabled,
        };

        let client_registry = clients.clone();
        let next_client = next_client_id.clone();
        let operator_client = operator_client_id.clone();
        let operator_control = operator_control_at.clone();
        let drop_counter = drop_count.clone();
        let radio_model = radio_model.clone();
        let handle = thread::spawn(move || loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let client_id = next_client.fetch_add(1, Ordering::SeqCst) + 1;
                    println!(
                        "saturn-bridge: TCI websocket client {client_id} connected from {addr}"
                    );

                    let command_tx = command_tx.clone();
                    let clients = client_registry.clone();
                    let operator_client_id = operator_client.clone();
                    let operator_control_at = operator_control.clone();
                    let drop_count = drop_counter.clone();
                    let radio_model = radio_model.clone();
                    let tx_codec_runtime_flags = tx_codec_runtime_flags;

                    thread::spawn(move || {
                        handle_client(
                            stream,
                            addr,
                            client_id,
                            &command_tx,
                            &clients,
                            &operator_client_id,
                            &operator_control_at,
                            &radio_model,
                            &drop_count,
                            remote_tx_rf_enabled,
                            tx_codec_runtime_flags,
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

        let frontend = Self {
            clients,
            operator_client_id,
            operator_control_at,
            drop_count,
            display_rate_limited_count: AtomicU64::new(0),
            tx_media_priority_active: AtomicBool::new(false),
            tx_power_meter_scale: config.tx_power_meter_scale,
            remote_tx_rf_enabled,
            display_frame_interval: display_frame_interval_for_limit(config.display_frame_limit_hz),
            last_display_frame_at: Mutex::new(None),
            rx_audio_transport_rate_hz: config.rx_audio_transport_rate_hz,
            rx_audio_transport_channels: config.rx_audio_transport_channels.clamp(1, 2),
            _accept_thread: JoinGuard { handle },
        };
        Ok((frontend, command_rx))
    }

    /// Set the bridge's authoritative TX media-priority state.
    /// Called from the main loop from TX intent/armed/keyed state. While true,
    /// RX binary frames (IqFrame, TxIqFrame, AudioFrame) are suppressed on
    /// the media lane so uplink mic owns the media TCP send buffer. The flag
    /// cannot drift out of sync because there is no browser-owned state to
    /// forget to clear.
    pub fn set_tx_media_priority_active(&self, active: bool) {
        self.tx_media_priority_active
            .store(active, Ordering::Relaxed);
    }

    pub fn tx_media_priority_active(&self) -> bool {
        self.tx_media_priority_active.load(Ordering::Relaxed)
    }

    pub fn has_split_paired_session(&self) -> bool {
        let clients = self.clients.lock_unpoisoned();
        split_paired_session_count(&clients) > 0
    }

    pub fn last_operator_control_at(&self) -> Option<Instant> {
        *self.operator_control_at.lock_unpoisoned()
    }

    pub fn clear_split_release_window(&self) {
        let operator_client_id = self.operator_client_id.load(Ordering::SeqCst);
        set_split_media_ignore_until(&self.clients, operator_client_id, None);
    }

    pub fn mark_split_released(&self, now: Instant) {
        let operator_client_id = self.operator_client_id.load(Ordering::SeqCst);
        flush_client_tx_codec_decode_queue(&self.clients, operator_client_id);
        set_split_media_ignore_until(
            &self.clients,
            operator_client_id,
            Some(now + SPLIT_RELEASE_IGNORE_WINDOW),
        );
        // Source-of-truth release: bridge clears media priority here so the
        // next send_message call lifts RX media suppression on the media lane.
        // Replaces the previous per-client clear_tx_media_priority_active; the
        // stuck-flag class of bug cannot recur because there is no per-client
        // flag to drift out of sync.
        self.set_tx_media_priority_active(false);
    }

    pub fn client_snapshot(&self) -> TciClientSnapshot {
        let clients = self.clients.lock_unpoisoned();
        let now = Instant::now();
        let mut safety_latencies_us = Vec::new();
        let mut control_latencies_us = Vec::new();
        let mut display_replaced_per_sec = 0u64;
        let mut display_dropped_per_sec = 0u64;
        let mut audio_dropped_per_sec = 0u64;
        let mut audio_panic_drain_count = 0u64;
        let mut send_blocked_ms = 0u64;
        let mut outbound_high_watermark_bytes = 0u64;
        let mut outbound_queued_bytes = 0u64;
        let mut tcp_outq_high_watermark_bytes = 0u64;
        let mut safety_queue_depth_overflow_count = 0u64;

        for client in clients.values() {
            let delta = client.outbound.drain_stats();
            safety_latencies_us.extend(delta.safety_latencies_us);
            control_latencies_us.extend(delta.control_latencies_us);
            display_replaced_per_sec =
                display_replaced_per_sec.saturating_add(delta.display_replaced);
            display_dropped_per_sec = display_dropped_per_sec.saturating_add(delta.display_dropped);
            audio_dropped_per_sec = audio_dropped_per_sec.saturating_add(delta.audio_dropped);
            audio_panic_drain_count =
                audio_panic_drain_count.saturating_add(delta.audio_panic_drain);
            send_blocked_ms = send_blocked_ms.saturating_add(delta.send_blocked_ms);
            outbound_high_watermark_bytes =
                outbound_high_watermark_bytes.max(delta.outbound_high_watermark_bytes);
            outbound_queued_bytes = outbound_queued_bytes.max(client.outbound.queued_bytes());
            tcp_outq_high_watermark_bytes =
                tcp_outq_high_watermark_bytes.max(delta.tcp_outq_high_watermark_bytes);
            safety_queue_depth_overflow_count =
                safety_queue_depth_overflow_count.saturating_add(delta.safety_queue_depth_overflow);
        }

        TciClientSnapshot {
            active: !clients.is_empty(),
            iq_stream_enabled: clients
                .values()
                .any(|client| client.state.iq_stream_enabled),
            audio_stream_enabled: clients
                .values()
                .any(|client| client.state.audio_stream_enabled),
            outbound_drops: self.drop_count.load(Ordering::Relaxed),
            safety_enqueue_to_write_p50_us: percentile_us(&mut safety_latencies_us, 50),
            safety_enqueue_to_write_p95_us: percentile_us(&mut safety_latencies_us, 95),
            safety_enqueue_to_write_p99_us: percentile_us(&mut safety_latencies_us, 99),
            control_enqueue_to_write_p50_us: percentile_us(&mut control_latencies_us, 50),
            control_enqueue_to_write_p95_us: percentile_us(&mut control_latencies_us, 95),
            control_enqueue_to_write_p99_us: percentile_us(&mut control_latencies_us, 99),
            display_replaced_per_sec,
            display_dropped_per_sec,
            audio_dropped_per_sec,
            audio_seq_gap_count: clients
                .values()
                .map(|client| client.state.audio_seq_gap_count)
                .sum(),
            audio_panic_drain_count,
            send_blocked_ms,
            outbound_high_watermark_bytes,
            outbound_queued_bytes,
            tcp_outq_high_watermark_bytes,
            display_rate_limited_per_sec: self
                .display_rate_limited_count
                .swap(0, Ordering::Relaxed),
            safety_queue_depth_overflow_count,
            split_control_clients: split_lane_client_count(&clients, SplitSocketKind::Control),
            split_media_clients: split_lane_client_count(&clients, SplitSocketKind::Media),
            split_paired_sessions: split_paired_session_count(&clients),
            tx_uplink_degraded: clients
                .values()
                .any(|client| client.state.tx_uplink_degraded),
            tx_mic_browser_dropped_count: clients
                .values()
                .map(|client| client.state.tx_mic_browser_dropped_count)
                .sum(),
            tx_uplink_buffered_bytes: clients
                .values()
                .map(|client| client.state.tx_uplink_buffered_bytes)
                .max()
                .unwrap_or(0),
            tx_uplink_buffered_high_watermark_bytes: clients
                .values()
                .map(|client| client.state.tx_uplink_buffered_high_watermark_bytes)
                .max()
                .unwrap_or(0),
            tx_mic_last_arrived_seq: clients
                .values()
                .map(|client| client.state.tx_mic_last_arrived_seq)
                .max()
                .unwrap_or(0),
            tx_mic_seq_gap_count: clients
                .values()
                .map(|client| client.state.tx_mic_seq_gap_count)
                .sum(),
            tx_mic_age_ms: clients
                .values()
                .filter_map(|client| client.state.tx_mic_last_arrived_at)
                .map(|arrived_at| now.saturating_duration_since(arrived_at).as_millis() as u64)
                .max()
                .unwrap_or(0),
            tx_media_priority_active: self.tx_media_priority_active(),
            tx_codec_decode_error_count: clients
                .values()
                .map(|client| client.state.tx_codec_decode_error_count)
                .sum(),
            tx_codec_stale_drop_count: clients
                .values()
                .map(|client| client.state.tx_codec_stale_drop_count)
                .sum(),
            tx_codec_release_flush_count: clients
                .values()
                .map(|client| client.state.tx_codec_release_flush_count)
                .sum(),
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
            "rx_nr2_gain_method:0,{};",
            model.desired.rx_nr2_gain_method
        ));
        self.send_text(format!(
            "rx_nr2_npe_method:0,{};",
            model.desired.rx_nr2_npe_method
        ));
        self.send_text(format!(
            "rx_nr2_post_filter:0,{};",
            model.desired.rx_nr2_post_filter_enabled
        ));
        self.send_text(format!(
            "rx_wbfm_supported:0,{};",
            crate::wdsp::wbfm_supported()
        ));
        self.send_text(format!(
            "rx_wbfm_deemphasis:0,{};",
            model.desired.rx_wbfm_deemphasis
        ));
        self.send_text(format!(
            "rx_wbfm_stereo:0,{};",
            model.observed.rx_wbfm_stereo_detected
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
        self.send_text(format!(
            "tx_phase_rotator:0,{};",
            model.desired.tx_phase_rotator_enabled
        ));
        self.send_text(format!(
            "tx_phase_rotator_auto:0,{};",
            model.desired.tx_phase_rotator_auto
        ));
        self.send_text(format!(
            "tx_phase_rotator_corner:0,{:.0};",
            model.desired.tx_phase_rotator_corner_hz
        ));
        self.send_text(format!(
            "tx_puresignal:0,{};",
            model.desired.pure_signal_enabled
        ));
        self.send_text(format!(
            "tx_puresignal_auto_attenuate:0,{};",
            model.desired.pure_signal_auto_attenuate
        ));
        self.send_text(format!(
            "tx_puresignal_attenuation:0,{};",
            model.desired.pure_signal_attenuation_db
        ));
        self.send_text(format!(
            "tx_puresignal_state:0,{};",
            model.observed.pure_signal_state
        ));
        self.send_text(format!(
            "tx_puresignal_feedback:0,{};",
            model.observed.pure_signal_feedback_level
        ));
        self.send_text(format!(
            "tx_puresignal_calibration_count:0,{};",
            model.observed.pure_signal_calibration_count
        ));
        self.send_text(format!(
            "tx_puresignal_correcting:0,{};",
            model.observed.pure_signal_correcting
        ));
        self.send_text(format!(
            "tx_puresignal_max_tx:0,{:.4};",
            model.observed.pure_signal_max_tx
        ));
        self.send_text(format!(
            "tx_puresignal_feedback_packets:0,{};",
            model.observed.pure_signal_feedback_packets
        ));
        self.send_text(format!(
            "tx_puresignal_feedback_gaps:0,{};",
            model.observed.pure_signal_feedback_gaps
        ));
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

    pub fn publish_puresignal_status(&self, model: &RadioModel) {
        self.send_text(format!(
            "tx_puresignal_attenuation:0,{};",
            model.desired.pure_signal_attenuation_db
        ));
        self.send_text(format!(
            "tx_puresignal_state:0,{};",
            model.observed.pure_signal_state
        ));
        self.send_text(format!(
            "tx_puresignal_feedback:0,{};",
            model.observed.pure_signal_feedback_level
        ));
        self.send_text(format!(
            "tx_puresignal_calibration_count:0,{};",
            model.observed.pure_signal_calibration_count
        ));
        self.send_text(format!(
            "tx_puresignal_correcting:0,{};",
            model.observed.pure_signal_correcting
        ));
        self.send_text(format!(
            "tx_puresignal_max_tx:0,{:.4};",
            model.observed.pure_signal_max_tx
        ));
        self.send_text(format!(
            "tx_puresignal_feedback_packets:0,{};",
            model.observed.pure_signal_feedback_packets
        ));
        self.send_text(format!(
            "tx_puresignal_feedback_gaps:0,{};",
            model.observed.pure_signal_feedback_gaps
        ));
    }

    pub fn publish_telemetry(&self, model: &RadioModel) {
        self.send_text(format!(
            "rx_wbfm_stereo:0,{};",
            model.observed.rx_wbfm_stereo_detected
        ));
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

    pub fn publish_scheduler_telemetry(&self, snapshot: &TciClientSnapshot) {
        self.send_text(format!(
            "remote_backpressure:0,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{};",
            snapshot.safety_enqueue_to_write_p50_us,
            snapshot.safety_enqueue_to_write_p95_us,
            snapshot.safety_enqueue_to_write_p99_us,
            snapshot.control_enqueue_to_write_p50_us,
            snapshot.control_enqueue_to_write_p95_us,
            snapshot.control_enqueue_to_write_p99_us,
            snapshot.display_replaced_per_sec,
            snapshot.display_dropped_per_sec,
            snapshot.audio_dropped_per_sec,
            snapshot.audio_seq_gap_count,
            snapshot.audio_panic_drain_count,
            snapshot.send_blocked_ms,
            snapshot.outbound_high_watermark_bytes,
            snapshot.safety_queue_depth_overflow_count,
            snapshot.tcp_outq_high_watermark_bytes,
            snapshot.display_rate_limited_per_sec,
            snapshot.outbound_queued_bytes
        ));
    }

    pub fn publish_tx_uplink_telemetry(&self, snapshot: &TciClientSnapshot) {
        self.send_text(format!(
            "remote_tx_uplink:0,{},{},{},{},{},{},{},{},{},{};",
            if snapshot.tx_uplink_degraded { 1 } else { 0 },
            snapshot.tx_mic_browser_dropped_count,
            snapshot.tx_uplink_buffered_bytes,
            snapshot.tx_uplink_buffered_high_watermark_bytes,
            snapshot.tx_mic_last_arrived_seq,
            snapshot.tx_mic_seq_gap_count,
            snapshot.tx_mic_age_ms,
            snapshot.tx_codec_decode_error_count,
            snapshot.tx_codec_stale_drop_count,
            snapshot.tx_codec_release_flush_count
        ));
    }

    pub fn publish_saturn_pong(&self, client_id: u64, nonce: &str, sent_at: &str) {
        self.send_text_to(client_id, format!("saturn_pong:{nonce},{sent_at};"));
    }

    pub fn publish_tx_power_trip(&self, forward_watts: f32, limit_watts: f32) {
        self.send_safety_text(tx_power_trip_fault_message(forward_watts, limit_watts));
    }

    pub fn publish_tx_uplink_late(&self, age_ms: u64, limit_ms: u64) {
        self.send_safety_text(tx_uplink_late_fault_message(age_ms, limit_ms));
    }

    pub fn publish_tx_control_watchdog(&self, silence_ms: u64, limit_ms: u64) {
        self.send_safety_text(tx_control_watchdog_fault_message(silence_ms, limit_ms));
    }

    pub fn publish_iq_frame(&self, sample_rate_hz: u32, iq_samples: &[f32]) {
        if !self.is_iq_stream_enabled() {
            return;
        }
        if !self.should_publish_display_frame() {
            self.display_rate_limited_count
                .fetch_add(1, Ordering::Relaxed);
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
        if !self.should_publish_display_frame() {
            self.display_rate_limited_count
                .fetch_add(1, Ordering::Relaxed);
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
        self.send_text(format!(
            "audio_samplerate:{};",
            self.audio_transport_sample_rate(sample_rate_hz)
        ));
    }

    pub fn publish_audio_stopped(&self) {
        self.send_text("audio_stop:0;".to_string());
    }

    pub fn publish_audio_frame(&self, sample_rate_hz: u32, audio_samples: &[f32]) {
        if !self.is_audio_stream_enabled() {
            return;
        }

        let (transport_rate_hz, transport_channels, transport_samples) =
            shape_rx_audio_for_transport(
                audio_samples,
                sample_rate_hz,
                2,
                self.audio_transport_sample_rate(sample_rate_hz),
                self.rx_audio_transport_channels,
            );

        self.send_message(OutboundMessage::AudioFrame {
            receiver: 0,
            sample_rate: transport_rate_hz,
            channels: transport_channels,
            audio_samples: transport_samples,
            sequence: 0,
        });
    }

    fn audio_transport_sample_rate(&self, source_rate_hz: u32) -> u32 {
        self.rx_audio_transport_rate_hz
            .clamp(8_000, source_rate_hz.max(8_000).min(48_000))
    }

    fn is_iq_stream_enabled(&self) -> bool {
        self.clients
            .lock()
            .unwrap()
            .values()
            .any(|client| client.state.iq_stream_enabled)
    }

    fn should_publish_display_frame(&self) -> bool {
        if self.display_frame_interval.is_zero() {
            return true;
        }
        let now = Instant::now();
        let mut last = self.last_display_frame_at.lock_unpoisoned();
        if last
            .map(|sent_at| now.duration_since(sent_at) >= self.display_frame_interval)
            .unwrap_or(true)
        {
            *last = Some(now);
            true
        } else {
            false
        }
    }

    fn is_audio_stream_enabled(&self) -> bool {
        self.clients
            .lock()
            .unwrap()
            .values()
            .any(|client| client.state.audio_stream_enabled)
    }

    fn send_text(&self, text: String) {
        self.send_message(OutboundMessage::Text(text));
    }

    fn send_safety_text(&self, text: String) {
        self.send_message(OutboundMessage::SafetyText(text));
    }

    fn send_text_to(&self, client_id: u64, text: String) {
        self.send_message_to(client_id, OutboundMessage::Text(text));
    }

    fn send_message_to(&self, client_id: u64, message: OutboundMessage) {
        if let Some(outbound) = self
            .clients
            .lock()
            .unwrap()
            .get(&client_id)
            .map(|client| client.outbound.clone())
        {
            let drops = outbound.enqueue(message);
            self.drop_count.fetch_add(drops, Ordering::Relaxed);
        }
    }

    fn send_message(&self, message: OutboundMessage) {
        let tx_media_priority_active = self.tx_media_priority_active();
        let clients = self.clients.lock_unpoisoned();
        for client in clients.values() {
            if !client_wants_outbound_message(client, &message, tx_media_priority_active) {
                continue;
            }
            let drops = client.outbound.enqueue(message.clone());
            self.drop_count.fetch_add(drops, Ordering::Relaxed);
        }
    }
}
