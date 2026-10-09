use std::collections::VecDeque;
use std::mem::Discriminant;
use std::sync::mpsc::{SendError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

use crate::sync_ext::MutexExt;

use super::TciCommand;

pub(crate) const MAX_TCI_SAFETY_COMMANDS: usize = 16;
pub(crate) const MAX_TCI_CONTROL_COMMANDS: usize = 256;
pub(crate) const MAX_TCI_MIC_COMMANDS: usize = 8;

pub(crate) trait TciCommandSink {
    fn send(&self, command: TciCommand) -> Result<(), SendError<TciCommand>>;
}

impl TciCommandSink for Sender<TciCommand> {
    fn send(&self, command: TciCommand) -> Result<(), SendError<TciCommand>> {
        Sender::send(self, command)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TciCommandQueueSnapshot {
    pub(crate) safety_depth: usize,
    pub(crate) control_depth: usize,
    pub(crate) mic_depth: usize,
    pub(crate) total_depth: usize,
    pub(crate) high_watermark: usize,
    pub(crate) control_coalesced: u64,
    pub(crate) control_dropped: u64,
    pub(crate) mic_dropped: u64,
    pub(crate) safety_coalesced: u64,
    /// Queued arm requests removed because a release or disconnect arrived
    /// after them.
    pub(crate) arm_cancelled: u64,
    /// Queued microphone frames removed at the same boundary.
    pub(crate) mic_cancelled: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum CommandCoalesceKey {
    Variant(Discriminant<TciCommand>),
    RadioState(u64),
    RxEqBand(usize),
    TxEqBand(usize),
    TxCfcBand(usize),
}

#[derive(Default, Debug)]
struct TciCommandQueues {
    safety: VecDeque<TciCommand>,
    control: VecDeque<TciCommand>,
    mic: VecDeque<TciCommand>,
    high_watermark: usize,
    control_coalesced: u64,
    control_dropped: u64,
    mic_dropped: u64,
    safety_coalesced: u64,
    arm_cancelled: u64,
    mic_cancelled: u64,
}

impl TciCommandQueues {
    fn depth(&self) -> usize {
        self.safety.len() + self.control.len() + self.mic.len()
    }

    fn cancel_session_commands(&mut self) {
        let queued_arms = self.control.len();
        self.control
            .retain(|queued| !matches!(queued, TciCommand::SetTxEnabled(true)));
        self.arm_cancelled = self
            .arm_cancelled
            .saturating_add((queued_arms - self.control.len()) as u64);
        self.mic_cancelled = self.mic_cancelled.saturating_add(self.mic.len() as u64);
        self.mic.clear();
    }

    fn record_high_watermark(&mut self) {
        self.high_watermark = self.high_watermark.max(self.depth());
    }

    fn snapshot(&self) -> TciCommandQueueSnapshot {
        TciCommandQueueSnapshot {
            safety_depth: self.safety.len(),
            control_depth: self.control.len(),
            mic_depth: self.mic.len(),
            total_depth: self.depth(),
            high_watermark: self.high_watermark,
            control_coalesced: self.control_coalesced,
            control_dropped: self.control_dropped,
            mic_dropped: self.mic_dropped,
            safety_coalesced: self.safety_coalesced,
            arm_cancelled: self.arm_cancelled,
            mic_cancelled: self.mic_cancelled,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TciCommandMailboxSender {
    queues: Arc<Mutex<TciCommandQueues>>,
}

#[derive(Clone, Debug)]
pub(crate) struct TciCommandMailboxReceiver {
    queues: Arc<Mutex<TciCommandQueues>>,
}

pub(crate) fn tci_command_mailbox() -> (TciCommandMailboxSender, TciCommandMailboxReceiver) {
    let queues = Arc::new(Mutex::new(TciCommandQueues::default()));
    (
        TciCommandMailboxSender {
            queues: Arc::clone(&queues),
        },
        TciCommandMailboxReceiver { queues },
    )
}

impl TciCommandSink for TciCommandMailboxSender {
    fn send(&self, command: TciCommand) -> Result<(), SendError<TciCommand>> {
        self.enqueue(command);
        Ok(())
    }
}

impl TciCommandMailboxSender {
    fn enqueue(&self, command: TciCommand) {
        let mut queues = self.queues.lock_unpoisoned();
        if command_is_safety(&command) {
            // Safety commands are delivered ahead of control commands, so a
            // release that merely overtook an older arm would still let that
            // arm run afterwards. Everything queued before the release or
            // disconnect belongs to the session it ends: drop it here. Commands
            // enqueued afterwards, including a fresh arm, are unaffected.
            queues.cancel_session_commands();
            let discriminant = std::mem::discriminant(&command);
            if let Some(position) = queues
                .safety
                .iter()
                .position(|queued| std::mem::discriminant(queued) == discriminant)
            {
                queues.safety[position] = command;
                queues.safety_coalesced = queues.safety_coalesced.saturating_add(1);
            } else {
                if queues.safety.len() >= MAX_TCI_SAFETY_COMMANDS {
                    queues.safety.pop_front();
                }
                queues.safety.push_back(command);
            }
        } else if matches!(command, TciCommand::MicAudioFrame(_)) {
            if queues.mic.len() >= MAX_TCI_MIC_COMMANDS {
                queues.mic.pop_front();
                queues.mic_dropped = queues.mic_dropped.saturating_add(1);
            }
            queues.mic.push_back(command);
        } else if let Some(key) = command_coalesce_key(&command) {
            if let Some(position) = queues
                .control
                .iter()
                .position(|queued| command_coalesce_key(queued).as_ref() == Some(&key))
            {
                queues.control[position] = command;
                queues.control_coalesced = queues.control_coalesced.saturating_add(1);
            } else {
                enqueue_control(&mut queues, command);
            }
        } else {
            enqueue_control(&mut queues, command);
        }
        queues.record_high_watermark();
    }
}

fn enqueue_control(queues: &mut TciCommandQueues, command: TciCommand) {
    if queues.control.len() >= MAX_TCI_CONTROL_COMMANDS {
        queues.control.pop_front();
        queues.control_dropped = queues.control_dropped.saturating_add(1);
    }
    queues.control.push_back(command);
}

impl TciCommandMailboxReceiver {
    pub(crate) fn try_recv(&self) -> Result<TciCommand, TryRecvError> {
        let mut queues = self.queues.lock_unpoisoned();
        queues
            .safety
            .pop_front()
            .or_else(|| queues.control.pop_front())
            .or_else(|| queues.mic.pop_front())
            .ok_or(TryRecvError::Empty)
    }

    pub(crate) fn snapshot(&self) -> TciCommandQueueSnapshot {
        self.queues.lock_unpoisoned().snapshot()
    }
}

fn command_is_safety(command: &TciCommand) -> bool {
    matches!(
        command,
        TciCommand::SetTxEnabled(false) | TciCommand::ClientDisconnected
    )
}

fn command_coalesce_key(command: &TciCommand) -> Option<CommandCoalesceKey> {
    match command {
        TciCommand::SetRxEqBand { band, .. } => Some(CommandCoalesceKey::RxEqBand(*band)),
        TciCommand::SetTxEqBand { band, .. } => Some(CommandCoalesceKey::TxEqBand(*band)),
        TciCommand::SetTxCfcBand { band, .. } => Some(CommandCoalesceKey::TxCfcBand(*band)),
        TciCommand::RequestRadioState { client_id } => {
            Some(CommandCoalesceKey::RadioState(*client_id))
        }
        TciCommand::SetRxAnrVals { .. }
        | TciCommand::SetRxAnfVals { .. }
        | TciCommand::RequestSmeter
        | TciCommand::SaturnPing { .. }
        | TciCommand::SplitSessionOpen { .. }
        | TciCommand::SplitSessionLane { .. }
        | TciCommand::ResetPureSignal
        | TciCommand::MicAudioFrame(_)
        | TciCommand::ClientConnected
        | TciCommand::ClientDisconnected
        | TciCommand::SetTxEnabled(false) => None,
        _ => Some(CommandCoalesceKey::Variant(std::mem::discriminant(command))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_flood_keeps_tx_release_latency_below_ten_ms() {
        let (tx, rx) = tci_command_mailbox();
        for sequence in 1..=10_000 {
            tx.send(TciCommand::SetVfoA(7_100_000 + sequence)).unwrap();
            tx.send(TciCommand::MicAudioFrame(super::super::TciMicFrame {
                sample_rate_hz: 48_000,
                channels: 1,
                sequence,
                received_at: std::time::Instant::now(),
                samples: vec![0.0; 16],
            }))
            .unwrap();
        }
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();

        let started = std::time::Instant::now();
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(started.elapsed() < std::time::Duration::from_millis(10));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetVfoA(_))));
        // The release is a cancellation boundary: microphone frames that
        // arrived before it are not delivered after it.
        assert!(rx.try_recv().is_err());
        let snapshot = rx.snapshot();
        assert!(snapshot.total_depth <= MAX_TCI_CONTROL_COMMANDS + MAX_TCI_MIC_COMMANDS);
        assert_eq!(snapshot.mic_cancelled, MAX_TCI_MIC_COMMANDS as u64);
    }

    fn mic_frame(sequence: u32) -> TciCommand {
        TciCommand::MicAudioFrame(super::super::TciMicFrame {
            sample_rate_hz: 48_000,
            channels: 1,
            sequence,
            received_at: std::time::Instant::now(),
            samples: vec![0.0; 16],
        })
    }

    #[test]
    fn release_cancels_an_older_queued_arm() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();

        assert_eq!(rx.snapshot().arm_cancelled, 1);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(rx.try_recv().is_err(), "the older arm must not be delivered");
    }

    #[test]
    fn disconnect_cancels_an_older_queued_arm() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(TciCommand::ClientDisconnected).unwrap();

        assert_eq!(rx.snapshot().arm_cancelled, 1);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::ClientDisconnected)));
        assert!(rx.try_recv().is_err(), "the older arm must not be delivered");
    }

    #[test]
    fn a_new_arm_after_a_release_is_delivered() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();

        assert_eq!(rx.snapshot().arm_cancelled, 1);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_new_arm_after_a_disconnect_is_delivered() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(TciCommand::ClientDisconnected).unwrap();
        tx.send(TciCommand::ClientConnected).unwrap();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();

        assert!(matches!(rx.try_recv(), Ok(TciCommand::ClientDisconnected)));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::ClientConnected)));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn release_keeps_unrelated_control_commands_in_order() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetVfoA(7_100_000)).unwrap();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(TciCommand::RequestSmeter).unwrap();
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();

        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetVfoA(7_100_000))));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::RequestSmeter)));
        assert!(rx.try_recv().is_err());
        assert_eq!(rx.snapshot().arm_cancelled, 1);
    }

    #[test]
    fn release_without_a_queued_arm_cancels_nothing() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetVfoA(7_100_000)).unwrap();
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();

        let snapshot = rx.snapshot();
        assert_eq!(snapshot.arm_cancelled, 0);
        assert_eq!(snapshot.mic_cancelled, 0);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetVfoA(7_100_000))));
    }

    #[test]
    fn release_discards_older_microphone_frames_but_not_newer_ones() {
        let (tx, rx) = tci_command_mailbox();
        for sequence in 1..=3 {
            tx.send(mic_frame(sequence)).unwrap();
        }
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();
        tx.send(mic_frame(4)).unwrap();

        assert_eq!(rx.snapshot().mic_cancelled, 3);
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::MicAudioFrame(frame)) if frame.sequence == 4
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn an_arm_after_a_release_does_not_inherit_the_earlier_sessions_microphone() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();
        tx.send(mic_frame(1)).unwrap();
        tx.send(mic_frame(2)).unwrap();
        tx.send(TciCommand::SetTxEnabled(false)).unwrap();
        tx.send(TciCommand::SetTxEnabled(true)).unwrap();

        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(false))));
        assert!(matches!(rx.try_recv(), Ok(TciCommand::SetTxEnabled(true))));
        assert!(rx.try_recv().is_err());
        let snapshot = rx.snapshot();
        assert_eq!(snapshot.arm_cancelled, 1);
        assert_eq!(snapshot.mic_cancelled, 2);
    }

    #[test]
    fn mailbox_coalesces_control_and_bounds_noncoalesced_commands() {
        let (tx, rx) = tci_command_mailbox();
        for freq in 0..1_000u32 {
            tx.send(TciCommand::SetVfoA(freq)).unwrap();
        }
        for _ in 0..(MAX_TCI_CONTROL_COMMANDS + 10) {
            tx.send(TciCommand::RequestSmeter).unwrap();
        }

        let snapshot = rx.snapshot();
        assert_eq!(snapshot.control_depth, MAX_TCI_CONTROL_COMMANDS);
        assert_eq!(snapshot.control_coalesced, 999);
        assert_eq!(snapshot.control_dropped, 11);
        assert!(snapshot.total_depth <= MAX_TCI_CONTROL_COMMANDS);
    }

    #[test]
    fn mailbox_bounds_microphone_frames_and_keeps_newest() {
        let (tx, rx) = tci_command_mailbox();
        for sequence in 1..=20 {
            tx.send(TciCommand::MicAudioFrame(super::super::TciMicFrame {
                sample_rate_hz: 48_000,
                channels: 1,
                sequence,
                received_at: std::time::Instant::now(),
                samples: vec![0.0; 16],
            }))
            .unwrap();
        }
        let snapshot = rx.snapshot();
        assert_eq!(snapshot.mic_depth, MAX_TCI_MIC_COMMANDS);
        assert_eq!(snapshot.mic_dropped, 12);
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::MicAudioFrame(frame)) if frame.sequence == 13
        ));
    }

    #[test]
    fn mailbox_keeps_state_queries_for_distinct_clients() {
        let (tx, rx) = tci_command_mailbox();
        tx.send(TciCommand::RequestRadioState { client_id: 41 })
            .unwrap();
        tx.send(TciCommand::RequestRadioState { client_id: 41 })
            .unwrap();
        tx.send(TciCommand::RequestRadioState { client_id: 42 })
            .unwrap();

        let snapshot = rx.snapshot();
        assert_eq!(snapshot.control_depth, 2);
        assert_eq!(snapshot.control_coalesced, 1);

        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::RequestRadioState { client_id: 41 })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(TciCommand::RequestRadioState { client_id: 42 })
        ));
    }
}
