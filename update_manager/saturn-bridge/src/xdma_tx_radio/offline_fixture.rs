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
    pub(crate) fn prepare_legacy_capture(&self, worker: crate::tx_signal_capture::Worker) {
        let mut state = self.radio.state.lock().unwrap();
        state.shutdown().unwrap();
        state.capture_worker = Some(worker);
        state.completed_write_fifo_fixture = true;
        state.legacy_streaming_fifo_fixture = true;
    }
    pub(crate) fn finish_capture(&self) {
        let worker = self.radio.state.lock().unwrap().capture_worker.take();
        if let Some(w) = worker {
            w.finish();
        }
    }
    pub(crate) fn fail_dma_writes(&self) {
        self.radio.state.lock().unwrap().dma = File::open(self.directory.join("duc.bin")).unwrap();
    }
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
            capture_worker: None,
            shared_submission: None,
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
            completed_write_fifo_fixture: false,
            legacy_streaming_fifo_fixture: false,
        };
        let radio = Arc::new(DirectXdmaTxRadio {
            state: Mutex::new(state),
            monitor: crate::tx_monitor::TxMonitor::offline_disabled(),
            offline_output_fence: Some(Arc::new(
                crate::tx_owner_stop::OutputFence::new_open_for_offline_trial(),
            )),
            test_key_clock: None,
            monitor_observation: Mutex::new((0, 0., 0)),
            rx_headphone_observation: Mutex::new((0, 0., 0.)),
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
    pub(crate) fn with_key_clock() -> Self {
        let mut fixture = Self::new();
        let now = Instant::now();
        Arc::get_mut(&mut fixture.radio).unwrap().test_key_clock = Some(Mutex::new((now, now)));
        fixture
    }
    pub(crate) fn enable_completed_write_fifo(&self) {
        self.radio
            .state
            .lock()
            .unwrap()
            .completed_write_fifo_fixture = true;
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
    /// Optional file-I/O model: occupancy reflects frames the actual producer
    /// has written, rather than claiming full occupancy before its first write.
    /// This is not a model or qualification of hardware DMA timing or draining.
    pub(crate) fn step_written_fifo(&self) {
        let Ok(state) = self.radio.state.try_lock() else {
            return;
        };
        let words = if state.keyed {
            DUC_PREFILL_TARGET_WORDS
        } else if state.stream_active {
            let first = state
                .active_session
                .as_ref()
                .map_or(0, |s| s.frames_written_start);
            ((state.frames_written.saturating_sub(first) as usize) * DUC_FIFO_WORDS_PER_FRAME)
                .min(state.fifo_depth_words)
        } else {
            0
        };
        self.write(DUC_FIFO_MONITOR_REGISTER, words as u32);
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

#[test]
fn cooperative_direct_writer_obeys_fifo_space_and_final_cancellation_without_a_timer() {
    let f = DirectFixture::new();
    let iq = [0.2; DUC_FRAME_IQ_FLOATS];
    f.write(DUC_FIFO_MONITOR_REGISTER, 4096);
    assert!(!f.radio.offline_iq_generation(0, &iq).unwrap());
    assert_eq!(f.dma_writes(), 0);
    f.fifo_ready();
    assert!(f.radio.offline_iq_generation(0, &iq).unwrap());
    assert_eq!(f.dma_writes(), 1);
    f.radio.offline_output_fence().unwrap().close();
    assert!(f.radio.offline_iq_generation(0, &iq).is_err());
    assert_eq!(f.dma_writes(), 1);
}

#[test]
fn managed_key_refills_partial_fifo_before_live_iq_and_key() {
    exercise_managed_prefill(
        crate::pa_calibration::PaCalibration::Legacy,
        7_200_000,
        5,
        18,
    );
}

#[test]
fn calibrated_profile_reaches_actual_key_and_complete_stop() {
    exercise_managed_prefill(
        crate::pa_calibration::PaCalibration::Bands(crate::pa_calibration::G2_STARTING_GAINS),
        27_426_000,
        40,
        68,
    );
}

fn exercise_managed_prefill(
    profile: crate::pa_calibration::PaCalibration,
    frequency: u32,
    watts: u8,
    expected_drive: u8,
) {
    let fixture = DirectFixture::with_key_clock();
    let mut model = crate::tx_owner_stop::tests::model();
    model.desired.tx_pa_calibration = profile;
    model.desired.tx_frequency_hz = frequency;
    model.desired.tx_drive = watts;
    let iq = [0.2; DUC_FRAME_IQ_FLOATS];
    let mut stage = DirectKeyStage::default();
    fixture.write(DUC_FIFO_MONITOR_REGISTER, 0);
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert_eq!(fixture.radio.state.lock().unwrap().frames_written, 11);
    assert_eq!(fixture.read(DAC_CONTROL_REGISTER), dac_control_word(0));
    assert_eq!(
        fixture.read(RF_GPIO_REGISTER) & (MOX_BIT | TX_ENABLE_BIT),
        0
    );
    // Model the first real H2C batch arriving, not an already-full FIFO.
    fixture.write(
        DUC_FIFO_MONITOR_REGISTER,
        11 * DUC_FIFO_WORDS_PER_FRAME as u32,
    );
    fixture.radio.set_test_key_time(500_000);
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert_eq!(
        fixture.radio.state.lock().unwrap().frames_written,
        19,
        "the managed producer must write the missing eight frames"
    );
    fixture.fifo_ready();
    fixture.radio.set_test_key_time(1_000_000);
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert_eq!(fixture.radio.state.lock().unwrap().frames_written, 20);
    let mut bytes = [0; DUC_FRAME_BYTES];
    fixture.dma.read_exact_at(&mut bytes, 0).unwrap();
    assert!(
        bytes.iter().any(|v| *v != 0),
        "first live IQ must reach the H2C sink"
    );
    fixture.radio.set_test_key_time(1_500_000);
    assert!(fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert!(fixture.radio.state.lock().unwrap().keyed);
    assert_eq!(
        fixture.read(RF_GPIO_REGISTER) & (MOX_BIT | TX_ENABLE_BIT),
        MOX_BIT | TX_ENABLE_BIT
    );
    assert_eq!(
        fixture.read(DAC_CONTROL_REGISTER),
        dac_control_word(expected_drive)
    );
    fixture.radio.state.lock().unwrap().shutdown().unwrap();
    fixture.assert_off();
}

#[test]
fn managed_key_honors_dma_visibility_delay_and_cancellation() {
    let fixture = DirectFixture::with_key_clock();
    let model = crate::tx_owner_stop::tests::model();
    let iq = [0.2; DUC_FRAME_IQ_FLOATS];
    let mut stage = DirectKeyStage::default();
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    fixture.write(
        DUC_FIFO_MONITOR_REGISTER,
        11 * DUC_FIFO_WORDS_PER_FRAME as u32,
    );
    fixture.radio.set_test_key_time(499_999);
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert_eq!(fixture.radio.state.lock().unwrap().frames_written, 11);
    fixture.radio.set_test_key_time(500_000);
    assert!(!fixture
        .radio
        .offline_key_generation(0, &model, &mut stage, &iq)
        .unwrap());
    assert_eq!(fixture.radio.state.lock().unwrap().frames_written, 19);
    fixture.radio.offline_output_fence().unwrap().cancel();
    fixture.fifo_ready();
    fixture.radio.set_test_key_time(1_000_000);
    let before = fixture.dma_writes();
    assert_eq!(
        fixture
            .radio
            .offline_key_generation(0, &model, &mut stage, &iq),
        Err(saturn_bridge::puresignal::Fault::GenerationMismatch)
    );
    assert_eq!(
        fixture.dma_writes(),
        before,
        "closed generation must not refill or key"
    );
    assert_eq!(fixture.read(DAC_CONTROL_REGISTER), dac_control_word(0));
    assert_eq!(
        fixture.read(RF_GPIO_REGISTER) & (MOX_BIT | TX_ENABLE_BIT),
        0
    );
    assert!(fixture.radio.offline_stop_and_readback().unwrap());
    fixture.assert_off();
}

#[test]
fn managed_key_never_keys_on_prefill_fault_or_late_underrun() {
    for case in [
        "overflow",
        "threshold",
        "no-progress",
        "final-underflow",
        "final-low",
        "no-live-room",
    ] {
        let fixture = DirectFixture::with_key_clock();
        let model = crate::tx_owner_stop::tests::model();
        let iq = [0.2; DUC_FRAME_IQ_FLOATS];
        let mut stage = DirectKeyStage::default();
        assert!(!fixture
            .radio
            .offline_key_generation(0, &model, &mut stage, &iq)
            .unwrap());
        let value = match case {
            "overflow" => 1 << 31,
            "threshold" => 1 << 30,
            "no-progress" => 0,
            "no-live-room" => 4096,
            _ => DUC_PREFILL_TARGET_WORDS as u32,
        };
        fixture.write(DUC_FIFO_MONITOR_REGISTER, value);
        fixture.radio.set_test_key_time(500_000);
        let result = if case.starts_with("final-") {
            assert!(!fixture
                .radio
                .offline_key_generation(0, &model, &mut stage, &iq)
                .unwrap());
            fixture.write(
                DUC_FIFO_MONITOR_REGISTER,
                if case == "final-underflow" {
                    DUC_PREFILL_TARGET_WORDS as u32 | (1 << 29)
                } else {
                    (DUC_PREFILL_MINIMUM_WORDS - 1) as u32
                },
            );
            fixture.radio.set_test_key_time(1_000_000);
            fixture
                .radio
                .offline_key_generation(0, &model, &mut stage, &iq)
        } else if case == "no-progress" {
            for turn in 1..DUC_PREFILL_MAX_ATTEMPTS {
                fixture.radio.set_test_key_time(turn as u64 * 500_000);
                assert!(!fixture
                    .radio
                    .offline_key_generation(0, &model, &mut stage, &iq)
                    .unwrap());
            }
            fixture
                .radio
                .set_test_key_time(DUC_PREFILL_MAX_ATTEMPTS as u64 * 500_000);
            let before = fixture.dma_writes();
            let failed = fixture
                .radio
                .offline_key_generation(0, &model, &mut stage, &iq);
            assert_eq!(before, DUC_PREFILL_MAX_ATTEMPTS as u64);
            assert_eq!(
                fixture.dma_writes(),
                before,
                "attempt exhaustion must not write again"
            );
            failed
        } else {
            fixture
                .radio
                .offline_key_generation(0, &model, &mut stage, &iq)
        };
        assert_eq!(
            result,
            Err(saturn_bridge::puresignal::Fault::TxAuthority),
            "{case}"
        );
        fixture.assert_off();
    }
}
