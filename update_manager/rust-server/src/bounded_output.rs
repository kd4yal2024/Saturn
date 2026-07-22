use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::mpsc;

pub const LIVE_OUTPUT_CHANNEL_CAPACITY: usize = 128;

#[derive(Clone)]
pub struct BoundedOutputSender {
    sender: mpsc::Sender<String>,
    dropped: Arc<AtomicUsize>,
}

impl BoundedOutputSender {
    pub fn channel() -> (Self, mpsc::Receiver<String>) {
        let (sender, receiver) = mpsc::channel(LIVE_OUTPUT_CHANNEL_CAPACITY);
        (
            Self {
                sender,
                dropped: Arc::new(AtomicUsize::new(0)),
            },
            receiver,
        )
    }

    pub fn try_send(&self, line: String) {
        if self.sender.is_closed() {
            return;
        }

        let omitted = self.dropped.swap(0, Ordering::AcqRel);
        if omitted > 0 {
            let notice = backpressure_notice(omitted);
            if let Err(error) = self.sender.try_send(notice) {
                self.dropped.fetch_add(omitted, Ordering::AcqRel);
                if matches!(error, mpsc::error::TrySendError::Full(_)) {
                    self.dropped.fetch_add(1, Ordering::AcqRel);
                }
                return;
            }
        }

        if let Err(error) = self.sender.try_send(line) {
            if matches!(error, mpsc::error::TrySendError::Full(_)) {
                self.dropped.fetch_add(1, Ordering::AcqRel);
            }
        }
    }

    pub async fn send_terminal(&self, line: String) {
        let omitted = self.dropped.swap(0, Ordering::AcqRel);
        if omitted > 0 {
            let _ = tokio::time::timeout(
                Duration::from_millis(250),
                self.sender.send(backpressure_notice(omitted)),
            )
            .await;
        }
        let _ = tokio::time::timeout(Duration::from_millis(250), self.sender.send(line)).await;
    }
}

fn backpressure_notice(omitted: usize) -> String {
    format!(
        "[output backpressure: omitted {omitted} live line(s); the bounded run log contains the retained output]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_channel_reports_omitted_lines_before_terminal_output() {
        let (sender, mut receiver) = BoundedOutputSender::channel();
        for index in 0..(LIVE_OUTPUT_CHANNEL_CAPACITY + 5) {
            sender.try_send(format!("line-{index}"));
        }

        for _ in 0..LIVE_OUTPUT_CHANNEL_CAPACITY {
            receiver.recv().await.unwrap();
        }

        sender.send_terminal("Done".to_string()).await;
        let notice = receiver.recv().await.unwrap();
        assert!(notice.contains("output backpressure"));
        assert!(notice.contains("omitted 5 live line(s)"));
        assert_eq!(receiver.recv().await.as_deref(), Some("Done"));
    }
}
