//! Explicit ordinary files only. Never calls DirectTxState::open(), environment
//! device discovery, ensure_p2app_inactive(), codec worker or memory locking.
use super::*;
use std::sync::{mpsc, Arc};
pub(crate) struct DirectFixture {
    pub(crate) radio: Arc<DirectXdmaTxRadio>,
    registers: File,
    dma: File,
    directory: PathBuf,
}
impl DirectFixture {
    pub(crate) fn register_path(&self) -> PathBuf {
        self.directory.join("registers.bin")
    }
    pub(crate) fn write(&self, offset: u64, value: u32) {
        self.registers
            .write_all_at(&value.to_le_bytes(), offset)
            .unwrap();
    }
    pub(crate) fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("saturn-ps-owner-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("registers.bin");
        let registers = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        registers.set_len(0xc008).unwrap();
        let write = |offset, value: u32| {
            registers
                .write_all_at(&value.to_le_bytes(), offset)
                .unwrap();
        };
        write(0xc000, (1 << 25) | (4 << 20) | (31 << 4) | 0xf);
        write(0xc004, (1 << 16) | 2);
        write(0x4004, 0x53460004);
        write(DAC_CONTROL_REGISTER, dac_control_word(23));
        write(TX_CONFIG_REGISTER, keyed_stream_config(0));
        write(RF_GPIO_REGISTER, MOX_BIT | TX_ENABLE_BIT);
        write(
            ALEX_TX_FILTER_TX_ANTENNA_REGISTER,
            u32::from(ALEX_TX_RELAY_BIT),
        );
        let dma = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.join("duc.bin"))
            .unwrap();
        let state = DirectTxState {
            registers: XdmaRegisterDevice::open_peripheral(&path).unwrap(),
            dma: dma.try_clone().unwrap(),
            buffer: AlignedBuffer::new(DMA_BUFFER_BYTES).unwrap(),
            fifo_depth_words: 4096,
            rf_tx_qualified: true,
            stream_active: true,
            keyed: true,
            dma_writes: 0,
            frames_written: 0,
            fifo_lwm: usize::MAX,
            fifo_hwm: 0,
            fifo_faults: 0,
            fifo_startup_underflows: 0,
            forward_watts: 0.,
            reverse_watts: 0.,
            swr: 1.,
            power_meter_scale: 1.,
            sessions_started: 0,
            sessions_completed: 0,
            mux_resets: 0,
            active_session: None,
            last_session: None,
        };
        let radio = Arc::new(DirectXdmaTxRadio {
            state: Mutex::new(state),
            monitor: crate::tx_monitor::TxMonitor::offline_disabled(),
            offline_output_fence: Some(Arc::new(
                crate::tx_owner_stop::OutputFence::new_open_for_offline_trial(),
            )),
        });
        Self {
            radio,
            registers,
            dma,
            directory,
        }
    }
    pub(crate) fn read(&self, offset: u64) -> u32 {
        let mut b = [0; 4];
        self.registers.read_exact_at(&mut b, offset).unwrap();
        u32::from_le_bytes(b)
    }
    pub(crate) fn fifo_ready(&self) {
        // Ordinary-file fixture occupancy; NOT a hardware FIFO timing model.
        self.registers
            .write_all_at(
                &(DUC_PREFILL_TARGET_WORDS as u32).to_le_bytes(),
                DUC_FIFO_MONITOR_REGISTER,
            )
            .unwrap();
    }
    pub(crate) fn assert_off(&self) {
        assert_eq!(self.read(DAC_CONTROL_REGISTER), dac_control_word(0));
        assert_eq!(
            self.read(TX_CONFIG_REGISTER)
                & (TX_AMPLITUDE_MASK
                    | TX_OUTPUT_GATE_BIT
                    | DUC_STREAM_ENABLE_BIT
                    | DUC_MUX_RESET_BIT),
            0
        );
        assert_eq!(
            self.read(RF_GPIO_REGISTER) & (MOX_BIT | TX_ENABLE_BIT | TX_RELAY_DISABLE_BIT),
            TX_RELAY_DISABLE_BIT
        );
        assert_eq!(
            self.read(ALEX_TX_FILTER_TX_ANTENNA_REGISTER) & u32::from(ALEX_TX_RELAY_BIT),
            0
        );
        assert_ne!(self.read(FIFO_RESET_REGISTER) & DUC_FIFO_RESET_BIT, 0);
    }
    pub(crate) fn damage_register_file(&self) {
        self.registers.set_len(0).unwrap();
    }
    pub(crate) fn dma_len(&self) -> u64 {
        self.dma.metadata().unwrap().len()
    }
    pub(crate) fn dma_writes(&self) -> u64 {
        self.radio.state.lock().unwrap().dma_writes
    }
    /// Pauses an actual owner write while it holds the final output permit and
    /// state mutex. This models an in-flight kernel completion, not RF latency.
    pub(crate) fn paused_write(
        &self,
    ) -> (
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        thread::JoinHandle<Result<(), String>>,
    ) {
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let radio = self.radio.clone();
        let worker = thread::spawn(move || {
            radio.with_state(|state| {
                started.send(()).unwrap();
                wait.recv().unwrap();
                state.write_frame_batch(&[0.1; DUC_FRAME_IQ_FLOATS])
            })
        });
        (ready, release, worker)
    }
}
impl Drop for DirectFixture {
    fn drop(&mut self) {
        // Exact validated fixture files only; no recursive/broad deletion.
        for name in ["registers.bin", "duc.bin"] {
            std::fs::remove_file(self.directory.join(name)).unwrap();
        }
        std::fs::remove_dir(&self.directory).unwrap();
    }
}
