use std::io;

/// Consumer boundary for native transmit PCM.
///
/// Phase 0E intentionally provides only the null implementation. An XDMA
/// implementation must be reviewed separately and must not be added to the
/// SATP networking code.
pub trait TxAudioSink: Send {
    fn write_frames(&mut self, samples: &[f32]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

#[derive(Default)]
pub struct NullTxAudioSink {
    frames: u64,
    flushes: u64,
}

impl NullTxAudioSink {
    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    #[cfg(test)]
    pub fn flushes(&self) -> u64 {
        self.flushes
    }
}

impl TxAudioSink for NullTxAudioSink {
    fn write_frames(&mut self, samples: &[f32]) -> io::Result<()> {
        self.frames = self.frames.saturating_add(samples.len() as u64);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes = self.flushes.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{NullTxAudioSink, TxAudioSink};

    #[test]
    fn null_sink_counts_without_forwarding_audio() {
        let mut sink = NullTxAudioSink::default();
        sink.write_frames(&[0.1; 128]).unwrap();
        sink.flush().unwrap();
        assert_eq!(sink.frames(), 128);
        assert_eq!(sink.flushes(), 1);
    }
}
