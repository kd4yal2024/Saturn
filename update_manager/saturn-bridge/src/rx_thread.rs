//! Dedicated RX receive + DSP thread.
//!
//! Owns the P2 receive path and the WDSP RX engine, mirroring the
//! `saturn-tx` thread pattern: the main thread stays a pure control
//! plane (TCI commands, model reconciliation, TX safety) and can no
//! longer stall IQ ingestion — e.g. a WDSP filter reconfigure that
//! rebuilds a long impulse response now only delays this thread while
//! the socket buffer absorbs the burst.
//!
//! High-priority status packets arrive on the same socket; they are
//! forwarded to the main thread as [`RxEvent::HighPriority`] because
//! power-trip enforcement and telemetry publishing are control-plane
//! responsibilities.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::p2::packets::{split_puresignal_samples, HighPriorityFromSdr};
use crate::p2::session::{P2Event, P2Session};
use crate::radio_model::RadioModel;
use crate::sync_ext::MutexExt;
use crate::tci::TciFrontend;
use crate::tx_thread::TxCommand;
use crate::wdsp::WdspRxEngine;

/// How long to yield the shared socket to a discovery exchange.
const RX_DISCOVERY_YIELD: Duration = Duration::from_millis(2);

pub enum RxCommand {
    /// RX-relevant model parameters changed — re-sync the WDSP RX chain.
    ModelChanged,
    ResetAudioPacketizer,
    /// Value must already be normalized via `wdsp::normalize_audio_frame_float_count`.
    SetAudioFrameFloatCount(usize),
    ResetStreamBuffers,
}

pub enum RxEvent {
    HighPriority(HighPriorityFromSdr),
    /// Unrecoverable receive-path failure; the main thread should exit.
    Fatal(String),
}

/// Shared per-second counters for the main thread's diag status line.
#[derive(Default)]
pub struct RxStats {
    pub ddc_packets: AtomicU64,
    pub audio_frames: AtomicU64,
    pub audio_samples: AtomicU64,
}

pub fn spawn(
    session: Arc<P2Session>,
    radio_model: Arc<Mutex<RadioModel>>,
    tci: Arc<TciFrontend>,
    wdsp: WdspRxEngine,
    command_rx: Receiver<RxCommand>,
    event_tx: Sender<RxEvent>,
    tx_cmd_tx: Sender<TxCommand>,
    tx_requested: Arc<AtomicBool>,
    stats: Arc<RxStats>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("saturn-rx".into())
        .spawn(move || {
            run(
                session,
                radio_model,
                tci,
                wdsp,
                command_rx,
                event_tx,
                tx_cmd_tx,
                tx_requested,
                stats,
                stop_flag,
            );
        })
        .expect("failed to spawn RX thread")
}

#[allow(clippy::too_many_arguments)]
fn run(
    session: Arc<P2Session>,
    radio_model: Arc<Mutex<RadioModel>>,
    tci: Arc<TciFrontend>,
    mut wdsp: WdspRxEngine,
    command_rx: Receiver<RxCommand>,
    event_tx: Sender<RxEvent>,
    tx_cmd_tx: Sender<TxCommand>,
    tx_requested: Arc<AtomicBool>,
    stats: Arc<RxStats>,
    stop_flag: Arc<AtomicBool>,
) {
    println!("saturn-bridge: RX thread started");
    while !stop_flag.load(Ordering::Relaxed) {
        loop {
            match command_rx.try_recv() {
                Ok(command) => {
                    if !handle_command(command, &mut wdsp, &radio_model, &event_tx) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if session.discovery_exclusive_active() {
            thread::sleep(RX_DISCOVERY_YIELD);
            continue;
        }

        match session.recv_event() {
            Ok(Some(event)) => handle_event(
                event,
                &mut wdsp,
                &radio_model,
                &tci,
                &event_tx,
                &tx_cmd_tx,
                &tx_requested,
                &stats,
            ),
            // The socket read timeout paces this loop; no extra sleep.
            Ok(None) => {}
            Err(error) => {
                let _ = event_tx.send(RxEvent::Fatal(format!("P2 receive failed: {error}")));
                return;
            }
        }
    }
}

/// Returns false when the thread should exit (fatal DSP error already reported).
fn handle_command(
    command: RxCommand,
    wdsp: &mut WdspRxEngine,
    radio_model: &Arc<Mutex<RadioModel>>,
    event_tx: &Sender<RxEvent>,
) -> bool {
    match command {
        RxCommand::ModelChanged => {
            let mut model = radio_model.lock_unpoisoned();
            if let Err(error) = wdsp.sync_model(&model) {
                let _ = event_tx.send(RxEvent::Fatal(format!("WDSP RX sync failed: {error}")));
                return false;
            }
            model.observed.rx_wbfm_stereo_detected = wdsp.wbfm_stereo_detected();
        }
        RxCommand::ResetAudioPacketizer => wdsp.reset_audio_packetizer(),
        RxCommand::SetAudioFrameFloatCount(count) => {
            wdsp.set_audio_frame_float_count(count);
        }
        RxCommand::ResetStreamBuffers => wdsp.reset_stream_buffers(),
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    event: P2Event,
    wdsp: &mut WdspRxEngine,
    radio_model: &Arc<Mutex<RadioModel>>,
    tci: &TciFrontend,
    event_tx: &Sender<RxEvent>,
    tx_cmd_tx: &Sender<TxCommand>,
    tx_requested: &AtomicBool,
    stats: &RxStats,
) {
    match event {
        P2Event::HighPriorityFromSdr(packet) => {
            let _ = event_tx.send(RxEvent::HighPriority(packet));
        }
        P2Event::DdcIq(frame) => {
            let (pure_signal_enabled, tx_enabled, rx_ddc_index, ddc0_sample_rate_khz) = {
                let model = radio_model.lock_unpoisoned();
                (
                    model.desired.pure_signal_enabled,
                    model.desired.tx_enabled,
                    model.desired.rx_ddc_index,
                    model.desired.ddc0_sample_rate_khz,
                )
            };
            let tx_active = tx_requested.load(Ordering::Relaxed) || tx_enabled;

            // Phase 0B B1 §2.2: stop the RX WDSP channel across MOX instead
            // of starving it mid-stream (WDSP Guide §3.3). Both calls are
            // idempotent; transitions are detected here because DDC frames
            // keep arriving throughout TX.
            if tx_active {
                wdsp.suspend_for_tx();
            } else {
                wdsp.resume_from_tx();
            }

            if frame.ddc_index == 0 && pure_signal_enabled && tx_active {
                if let Some(samples) = split_puresignal_samples(&frame) {
                    let _ = tx_cmd_tx.send(TxCommand::PureSignalFeedback {
                        sequence: frame.sequence,
                        tx_reference: samples.tx_reference,
                        rx_feedback: samples.rx_feedback,
                        received_at: std::time::Instant::now(),
                    });
                }
            } else if frame.ddc_index == rx_ddc_index {
                stats.ddc_packets.fetch_add(1, Ordering::Relaxed);
                let sample_rate_hz = ddc0_sample_rate_khz as u32 * 1000;
                tci.publish_iq_frame(sample_rate_hz, &frame.iq_samples);
                // Keep display IQ live during MOX. The RX WDSP channel is
                // suspended (state 0) while tx_active, so AGC/NR neither pump
                // on local TX energy nor slew to maximum on zero input;
                // push_iq is additionally self-guarded while suspended.
                if !tx_active {
                    for audio_frame in wdsp.push_iq(&frame.iq_samples) {
                        stats.audio_frames.fetch_add(1, Ordering::Relaxed);
                        stats
                            .audio_samples
                            .fetch_add(audio_frame.len() as u64, Ordering::Relaxed);
                        tci.publish_audio_frame(wdsp.audio_sample_rate_hz(), &audio_frame);
                    }
                }
                let mut model = radio_model.lock_unpoisoned();
                if let Some(dbm) = wdsp.smeter_dbm() {
                    model.observed.ddc0_meter_dbm = Some(dbm);
                }
                model.observed.rx_wbfm_stereo_detected = wdsp.wbfm_stereo_detected();
                model.apply_ddc_frame(frame);
            }
        }
    }
}
