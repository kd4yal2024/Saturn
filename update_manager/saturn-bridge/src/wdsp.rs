use std::collections::VecDeque;
use std::error::Error;
use std::ffi::c_char;
use std::fmt;
use std::sync::Once;

use crate::radio_model::{AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel};

const WDSP_RX_CHANNEL: i32 = 0;
const WDSP_TX_CHANNEL: i32 = 1;
const WDSP_AUDIO_RATE_HZ: u32 = 48_000;
const WDSP_DSP_SIZE: usize = 64;
const WDSP_AUDIO_FRAME_FLOATS: usize = 2048;
const WDSP_ANR_TAPS: i32 = 64;
const WDSP_ANR_DELAY: i32 = 16;
const WDSP_NR_POSITION_POST_AGC: i32 = 1;
const WDSP_NR2_GAIN_METHOD: i32 = 2;
const WDSP_NR2_NPE_METHOD: i32 = 0;
const WDSP_NR2_TRAIN_ZETA_THRESH: f64 = -0.5;
const WDSP_NR2_TRAIN_T2: f64 = 0.2;
const WDSP_NR4_WHITENING_FACTOR: f32 = 0.0;
const WDSP_NR4_NOISE_SCALING_TYPE: i32 = 0;

// NoiseBlanker defaults (from pihpsdr receiver.c)
const NB_TAU: f64 = 0.00001;
const NB_HANGTIME: f64 = 0.00001;
const NB_ADVTIME: f64 = 0.00001;

// AGC modes (wdsp values)
const AGC_OFF: i32 = 0;
#[allow(dead_code)]
const AGC_MEDIUM: i32 = 3;
const AGC_FAST: i32 = 4;

// RXA meter type indices (from wdsp/RXA.h)
const RXA_S_AV: i32 = 1; // average S-meter value in dBm

static LOAD_RNNR_MODEL: Once = Once::new();

unsafe extern "C" {
    fn OpenChannel(
        channel: i32,
        in_size: i32,
        dsp_size: i32,
        input_samplerate: i32,
        dsp_rate: i32,
        output_samplerate: i32,
        channel_type: i32,
        state: i32,
        tdelayup: f64,
        tslewup: f64,
        tdelaydown: f64,
        tslewdown: f64,
        bfo: i32,
    );
    fn CloseChannel(channel: i32);
    fn SetChannelState(channel: i32, state: i32, dmode: i32) -> i32;
    fn fexchange0(channel: i32, input: *const f64, output: *mut f64, error: *mut i32);

    // RX DSP
    fn SetRXAMode(channel: i32, mode: i32);
    fn RXASetPassband(channel: i32, low_hz: f64, high_hz: f64);
    fn RXASetNC(channel: i32, nc: i32);
    fn RXASetMP(channel: i32, mp: i32);
    fn SetRXABandpassWindow(channel: i32, wintype: i32);
    fn SetRXABandpassRun(channel: i32, run: i32);
    fn SetRXAAMDSBMode(channel: i32, sbmode: i32);
    fn SetRXAPanelRun(channel: i32, run: i32);
    fn SetRXAPanelSelect(channel: i32, select: i32);
    fn SetRXAPanelCopy(channel: i32, copy: i32);
    fn SetRXAPanelGain1(channel: i32, gain: f64);

    // Noise Blanker 1 (preemptive) — must call create_anbEXT before Set* functions
    #[allow(improper_ctypes)]
    fn create_anbEXT(
        id: i32, run: i32, buffsize: i32, samplerate: f64,
        tau: f64, hangtime: f64, advtime: f64, backtau: f64, threshold: f64,
    );
    fn destroy_anbEXT(id: i32);
    fn SetEXTANBRun(id: i32, run: i32);
    fn SetEXTANBTau(id: i32, tau: f64);
    fn SetEXTANBHangtime(id: i32, time: f64);
    fn SetEXTANBAdvtime(id: i32, time: f64);
    fn SetEXTANBThreshold(id: i32, thresh: f64);

    // Noise Blanker 2 (interpolating) — must call create_nobEXT before Set* functions
    #[allow(improper_ctypes)]
    fn create_nobEXT(
        id: i32, run: i32, mode: i32, buffsize: i32, samplerate: f64,
        slewtime: f64, hangtime: f64, advtime: f64, backtau: f64, threshold: f64,
    );
    fn destroy_nobEXT(id: i32);
    fn SetEXTNOBRun(id: i32, run: i32);
    fn SetEXTNOBMode(id: i32, mode: i32);
    fn SetEXTNOBTau(id: i32, tau: f64);
    fn SetEXTNOBHangtime(id: i32, time: f64);
    fn SetEXTNOBAdvtime(id: i32, time: f64);
    fn SetEXTNOBThreshold(id: i32, thresh: f64);

    // Auto Notch Filter
    fn SetRXAANFRun(channel: i32, run: i32);
    fn SetRXAANFPosition(channel: i32, position: i32);

    // Noise Reduction
    fn SetRXAANRRun(channel: i32, run: i32);
    fn SetRXAANRVals(channel: i32, taps: i32, delay: i32, gain: f64, leakage: f64);
    fn SetRXAANRPosition(channel: i32, position: i32);
    fn SetRXAEMNRRun(channel: i32, run: i32);
    fn SetRXAEMNRgainMethod(channel: i32, method: i32);
    fn SetRXAEMNRnpeMethod(channel: i32, method: i32);
    fn SetRXAEMNRaeRun(channel: i32, run: i32);
    fn SetRXAEMNRPosition(channel: i32, position: i32);
    fn SetRXAEMNRtrainZetaThresh(channel: i32, thresh: f64);
    fn SetRXAEMNRtrainT2(channel: i32, t2: f64);
    fn SetRXAEMNRpost2Run(channel: i32, run: i32);
    fn SetRXAEMNRpost2Nlevel(channel: i32, nlevel: f64);
    fn SetRXAEMNRpost2Factor(channel: i32, factor: f64);
    fn SetRXAEMNRpost2Rate(channel: i32, tc: f64);
    fn SetRXAEMNRpost2Taper(channel: i32, taper: i32);
    fn SetRXARNNRRun(channel: i32, run: i32);
    fn RNNRloadModel(file_path: *const c_char);
    fn SetRXARNNRPosition(channel: i32, position: i32);
    fn SetRXASBNRRun(channel: i32, run: i32);
    fn SetRXASBNRreductionAmount(channel: i32, amount: f32);
    fn SetRXASBNRsmoothingFactor(channel: i32, factor: f32);
    fn SetRXASBNRwhiteningFactor(channel: i32, factor: f32);
    fn SetRXASBNRnoiseRescale(channel: i32, factor: f32);
    fn SetRXASBNRpostFilterThreshold(channel: i32, threshold: f32);
    fn SetRXASBNRnoiseScalingType(channel: i32, noise_scaling_type: i32);
    fn SetRXASBNRPosition(channel: i32, position: i32);

    // AGC
    fn SetRXAAGCMode(channel: i32, mode: i32);
    fn SetRXAAGCSlope(channel: i32, slope: i32);
    fn SetRXAAGCTop(channel: i32, max_agc: f64);
    fn SetRXAAGCAttack(channel: i32, attack: i32);
    fn SetRXAAGCHang(channel: i32, hang: i32);
    fn SetRXAAGCDecay(channel: i32, decay: i32);
    fn SetRXAAGCHangThreshold(channel: i32, hangthreshold: i32);
    fn SetRXAAGCMaxInputLevel(channel: i32, level: f64);

    // Meter
    fn GetRXAMeter(channel: i32, mt: i32) -> f64;

    // TX DSP
    fn SetTXAMode(channel: i32, mode: i32);
    fn SetTXABandpassWindow(channel: i32, wintype: i32);
    fn SetTXABandpassRun(channel: i32, run: i32);
    fn SetTXABandpassFreqs(channel: i32, f_low: f64, f_high: f64);
    fn SetTXACFIRRun(channel: i32, run: i32);
    fn SetTXAAMSQRun(channel: i32, run: i32);
    fn SetTXAALCSt(channel: i32, state: i32);
    fn SetTXAALCAttack(channel: i32, attack: i32);
    fn SetTXAALCDecay(channel: i32, decay: i32);
    fn SetTXAPreGenMode(channel: i32, mode: i32);
    fn SetTXAPreGenToneMag(channel: i32, mag: f64);
    fn SetTXAPreGenToneFreq(channel: i32, freq: f64);
    fn SetTXAPreGenRun(channel: i32, run: i32);
    fn SetTXAPanelRun(channel: i32, run: i32);
    fn SetTXAPanelSelect(channel: i32, select: i32);
    fn SetTXAPanelGain1(channel: i32, gain: f64);
    fn SetTXAPostGenRun(channel: i32, run: i32);

    // Graphic EQ (10-band) — RX and TX
    fn SetRXAEQRun(channel: i32, run: i32);
    fn SetRXAGrphEQ10(channel: i32, rxeq: *mut i32);
    fn SetTXAEQRun(channel: i32, run: i32);
    fn SetTXAGrphEQ10(channel: i32, txeq: *mut i32);

    // CFC (Continuous Frequency Compression)
    fn SetTXACFCOMPRun(channel: i32, run: i32);
    fn SetTXACFCOMPPosition(channel: i32, pos: i32);
    fn SetTXACFCOMPprofile(channel: i32, nfreqs: i32, f: *const f64, g: *const f64, e: *const f64);
    fn SetTXACFCOMPPrecomp(channel: i32, precomp: f64);
    fn SetTXACFCOMPPeqRun(channel: i32, run: i32);
    fn SetTXACFCOMPPrePeq(channel: i32, prepeq: f64);

    // PostGen — two-tone test signal generator (mode 2 = two-tone)
    fn SetTXAPostGenMode(channel: i32, mode: i32);
    fn SetTXAPostGenTTMag(channel: i32, mag1: f64, mag2: f64);
    fn SetTXAPostGenTTFreq(channel: i32, freq1: f64, freq2: f64);
}

#[derive(Debug)]
pub enum WdspError {
    UnsupportedInputSampleRate(u32),
}

impl fmt::Display for WdspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInputSampleRate(rate) => {
                write!(f, "unsupported WDSP input sample rate {rate} Hz")
            }
        }
    }
}

impl Error for WdspError {}

// ── RX Engine ────────────────────────────────────────────────────────────────

pub struct WdspRxEngine {
    channel_id: i32,
    input_sample_rate_hz: u32,
    mode: DemodMode,
    volume_db: f64,
    noise_reduction_mode: NoiseReductionMode,
    noise_reduction_level: f64,
    nb_mode: NoiseBlankerMode,
    nb_threshold: f64,
    anf_enabled: bool,
    agc_mode: AgcMode,
    filter_low_hz: i32,
    filter_high_hz: i32,
    input_complex_samples: usize,
    output_audio_frames: usize,
    input_buffer: Vec<f64>,
    output_buffer: Vec<f64>,
    pending_iq: VecDeque<f64>,
    pending_audio: VecDeque<f32>,
    frame_float_count: usize,
    last_meter_dbm: Option<f32>,
    nb_initialized: bool,
    rx_eq_enabled: bool,
    rx_eq_bands: [i32; 11],
}

impl WdspRxEngine {
    pub fn new(model: &RadioModel) -> Result<Self, WdspError> {
        let mut engine = Self {
            channel_id: WDSP_RX_CHANNEL,
            input_sample_rate_hz: 0,
            mode: DemodMode::Unknown,
            volume_db: -10.0,
            noise_reduction_mode: NoiseReductionMode::Off,
            noise_reduction_level: 100.0,
            nb_mode: NoiseBlankerMode::Off,
            nb_threshold: 4.95,
            anf_enabled: false,
            agc_mode: AgcMode::Medium,
            filter_low_hz: 0,
            filter_high_hz: 0,
            input_complex_samples: 0,
            output_audio_frames: 0,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
            pending_iq: VecDeque::new(),
            pending_audio: VecDeque::new(),
            frame_float_count: WDSP_AUDIO_FRAME_FLOATS,
            last_meter_dbm: None,
            nb_initialized: false,
            rx_eq_enabled: false,
            rx_eq_bands: [0i32; 11],
        };
        engine.reconfigure(model)?;
        Ok(engine)
    }

    pub fn smeter_dbm(&self) -> Option<f32> {
        self.last_meter_dbm
    }

    pub fn sync_model(&mut self, model: &RadioModel) -> Result<(), WdspError> {
        if model.desired.ddc0_sample_rate_khz as u32 * 1000 != self.input_sample_rate_hz {
            self.reconfigure(model)?;
            return Ok(());
        }

        if model.desired.mode != self.mode {
            self.mode = model.desired.mode;
            unsafe { SetRXAMode(self.channel_id, wdsp_mode(self.mode)); }
            self.apply_agc();
        }

        if (model.desired.rx_volume_db - self.volume_db).abs() > f64::EPSILON {
            self.volume_db = model.desired.rx_volume_db;
            unsafe { SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db)); }
        }

        if model.desired.rx_noise_reduction_mode != self.noise_reduction_mode
            || (model.desired.rx_noise_reduction_level - self.noise_reduction_level).abs() > f64::EPSILON
        {
            self.noise_reduction_mode = model.desired.rx_noise_reduction_mode;
            self.noise_reduction_level = model.desired.rx_noise_reduction_level;
            self.apply_noise_reduction();
        }

        if model.desired.nb_mode != self.nb_mode
            || (model.desired.nb_threshold - self.nb_threshold).abs() > f64::EPSILON
        {
            self.nb_mode = model.desired.nb_mode;
            self.nb_threshold = model.desired.nb_threshold;
            self.apply_noise_blanker();
        }

        if model.desired.anf_enabled != self.anf_enabled {
            self.anf_enabled = model.desired.anf_enabled;
            self.apply_anf();
        }

        if model.desired.agc_mode != self.agc_mode {
            self.agc_mode = model.desired.agc_mode;
            self.apply_agc();
        }

        if model.desired.filter_low_hz != self.filter_low_hz
            || model.desired.filter_high_hz != self.filter_high_hz
        {
            self.filter_low_hz = model.desired.filter_low_hz;
            self.filter_high_hz = model.desired.filter_high_hz;
            unsafe {
                RXASetPassband(self.channel_id, self.filter_low_hz as f64, self.filter_high_hz as f64);
            }
        }

        if model.desired.rx_eq_enabled != self.rx_eq_enabled
            || model.desired.rx_eq_bands != self.rx_eq_bands
        {
            self.rx_eq_enabled = model.desired.rx_eq_enabled;
            self.rx_eq_bands = model.desired.rx_eq_bands;
            unsafe {
                SetRXAGrphEQ10(self.channel_id, self.rx_eq_bands.as_mut_ptr());
                SetRXAEQRun(self.channel_id, self.rx_eq_enabled as i32);
            }
        }

        Ok(())
    }

    pub fn push_iq(&mut self, iq_samples: &[f32]) -> Vec<Vec<f32>> {
        for sample in iq_samples {
            self.pending_iq.push_back(*sample as f64);
        }

        let mut ready_frames = Vec::new();
        let needed_floats = self.input_complex_samples * 2;

        while self.pending_iq.len() >= needed_floats {
            for sample in &mut self.input_buffer {
                *sample = self.pending_iq.pop_front().unwrap_or(0.0);
            }

            let mut error = 0;
            unsafe {
                fexchange0(
                    self.channel_id,
                    self.input_buffer.as_ptr(),
                    self.output_buffer.as_mut_ptr(),
                    &mut error,
                );
            }

            if error != 0 {
                eprintln!("saturn-bridge: wdsp fexchange0 error {error}");
                continue;
            }

            self.last_meter_dbm = Some(unsafe { GetRXAMeter(self.channel_id, RXA_S_AV) } as f32);

            // HF radio is mono — always duplicate the left channel to both outputs.
            for sample_pair in self.output_buffer.chunks_exact(2) {
                let mono = (sample_pair[0] as f32).clamp(-1.0, 1.0);
                self.pending_audio.push_back(mono);
                self.pending_audio.push_back(mono);
            }

            while self.pending_audio.len() >= self.frame_float_count {
                let mut frame = Vec::with_capacity(self.frame_float_count);
                for _ in 0..self.frame_float_count {
                    frame.push(self.pending_audio.pop_front().unwrap_or(0.0));
                }
                ready_frames.push(frame);
            }
        }

        ready_frames
    }

    pub fn audio_sample_rate_hz(&self) -> u32 {
        WDSP_AUDIO_RATE_HZ
    }

    pub fn set_audio_frame_float_count(&mut self, frame_float_count: usize) -> usize {
        let normalized = frame_float_count.clamp(256, 8192) & !1usize;
        if normalized != self.frame_float_count {
            self.frame_float_count = normalized;
            self.pending_audio.clear();
        }
        normalized
    }

    pub fn reset_stream_buffers(&mut self) {
        self.pending_iq.clear();
        self.pending_audio.clear();
        self.input_buffer.fill(0.0);
        self.output_buffer.fill(0.0);
    }

    fn reconfigure(&mut self, model: &RadioModel) -> Result<(), WdspError> {
        let input_sample_rate_hz = model.desired.ddc0_sample_rate_khz as u32 * 1000;
        let ratio = input_sample_rate_hz / WDSP_AUDIO_RATE_HZ;
        if ratio == 0 || input_sample_rate_hz % WDSP_AUDIO_RATE_HZ != 0 || !ratio.is_power_of_two() {
            return Err(WdspError::UnsupportedInputSampleRate(input_sample_rate_hz));
        }

        if self.input_sample_rate_hz != 0 {
            unsafe { CloseChannel(self.channel_id); }
        }

        LOAD_RNNR_MODEL.call_once(|| unsafe {
            RNNRloadModel(std::ptr::null());
        });

        self.input_sample_rate_hz = input_sample_rate_hz;
        self.mode = model.desired.mode;
        self.volume_db = model.desired.rx_volume_db;
        self.noise_reduction_mode = model.desired.rx_noise_reduction_mode;
        self.noise_reduction_level = model.desired.rx_noise_reduction_level;
        self.nb_mode = model.desired.nb_mode;
        self.nb_threshold = model.desired.nb_threshold;
        self.anf_enabled = model.desired.anf_enabled;
        self.agc_mode = model.desired.agc_mode;
        self.filter_low_hz = model.desired.filter_low_hz;
        self.filter_high_hz = model.desired.filter_high_hz;
        self.input_complex_samples = (ratio as usize) * WDSP_DSP_SIZE;
        self.output_audio_frames = self.input_complex_samples / ratio as usize;
        self.input_buffer = vec![0.0; self.input_complex_samples * 2];
        self.output_buffer = vec![0.0; self.output_audio_frames * 2];
        self.pending_iq.clear();
        self.pending_audio.clear();

        unsafe {
            OpenChannel(
                self.channel_id,
                self.input_complex_samples as i32,
                WDSP_DSP_SIZE as i32,
                self.input_sample_rate_hz as i32,
                WDSP_AUDIO_RATE_HZ as i32,
                WDSP_AUDIO_RATE_HZ as i32,
                0,
                1,
                0.010,
                0.025,
                0.0,
                0.010,
                1,
            );
            SetRXABandpassWindow(self.channel_id, 1);
            SetRXABandpassRun(self.channel_id, 1);
            SetRXAAMDSBMode(self.channel_id, 0);
            SetRXAPanelRun(self.channel_id, 1);
            SetRXAPanelSelect(self.channel_id, 3); // use both I and Q input
            SetRXAPanelCopy(self.channel_id, 1);   // copy I→Q so both speakers get audio
            SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db));
            RXASetNC(self.channel_id, 2048);
            RXASetMP(self.channel_id, 0);
            SetRXAMode(self.channel_id, wdsp_mode(self.mode));
            RXASetPassband(self.channel_id, self.filter_low_hz as f64, self.filter_high_hz as f64);

            // Create (or re-create) the EXT NB objects for this channel
            if self.nb_initialized {
                destroy_anbEXT(self.channel_id);
                destroy_nobEXT(self.channel_id);
            }
            let sr = self.input_sample_rate_hz as f64;
            create_anbEXT(self.channel_id, 1, WDSP_DSP_SIZE as i32, sr,
                0.0001, 0.0001, 0.0001, 0.05, 20.0);
            create_nobEXT(self.channel_id, 1, 0, WDSP_DSP_SIZE as i32, sr,
                0.0001, 0.0001, 0.0001, 0.05, 20.0);
        }
        self.nb_initialized = true;
        unsafe { SetRXAEQRun(self.channel_id, 0); }
        self.apply_agc();
        self.apply_noise_blanker();
        self.apply_anf();
        self.apply_noise_reduction();
        Ok(())
    }

    fn apply_agc(&self) {
        let agc_mode = if self.agc_mode == AgcMode::Off {
            AGC_OFF
        } else {
            match self.mode {
                DemodMode::DigU | DemodMode::DigL | DemodMode::Cwl | DemodMode::Cwu => AGC_FAST,
                _ => self.agc_mode.wdsp_value(),
            }
        };
        unsafe {
            SetRXAAGCMode(self.channel_id, agc_mode);
            SetRXAAGCSlope(self.channel_id, 35);
            SetRXAAGCTop(self.channel_id, 80.0);
            SetRXAAGCMaxInputLevel(self.channel_id, 1.0);
            match agc_mode {
                AGC_OFF => {}
                AGC_FAST => {
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 0);
                    SetRXAAGCDecay(self.channel_id, 50);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
                1 => {
                    // LONG
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 2000);
                    SetRXAAGCDecay(self.channel_id, 2000);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
                2 => {
                    // SLOW
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 1000);
                    SetRXAAGCDecay(self.channel_id, 500);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
                _ => {
                    // MEDIUM
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 0);
                    SetRXAAGCDecay(self.channel_id, 250);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
            }
        }
    }

    fn apply_noise_blanker(&self) {
        let threshold = self.nb_threshold;
        unsafe {
            SetEXTANBTau(self.channel_id, NB_TAU);
            SetEXTANBHangtime(self.channel_id, NB_HANGTIME);
            SetEXTANBAdvtime(self.channel_id, NB_ADVTIME);
            SetEXTANBThreshold(self.channel_id, threshold);
            SetEXTANBRun(self.channel_id, (self.nb_mode == NoiseBlankerMode::Nb1) as i32);

            SetEXTNOBMode(self.channel_id, 0); // zero mode
            SetEXTNOBTau(self.channel_id, NB_TAU);
            SetEXTNOBHangtime(self.channel_id, NB_HANGTIME);
            SetEXTNOBAdvtime(self.channel_id, NB_ADVTIME);
            SetEXTNOBThreshold(self.channel_id, threshold);
            SetEXTNOBRun(self.channel_id, (self.nb_mode == NoiseBlankerMode::Nb2) as i32);
        }
    }

    fn apply_anf(&self) {
        unsafe {
            SetRXAANFRun(self.channel_id, self.anf_enabled as i32);
            SetRXAANFPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
        }
    }

    fn apply_noise_reduction(&self) {
        let level = self.noise_reduction_level.clamp(0.0, 100.0);
        unsafe {
            SetRXAANRVals(
                self.channel_id,
                WDSP_ANR_TAPS,
                WDSP_ANR_DELAY,
                anr_gain_for_level(level),
                anr_leakage_for_level(level),
            );
            SetRXAANRPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
            SetRXAANRRun(
                self.channel_id,
                matches!(self.noise_reduction_mode, NoiseReductionMode::Nr1) as i32,
            );

            SetRXAEMNRPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
            SetRXAEMNRgainMethod(self.channel_id, WDSP_NR2_GAIN_METHOD);
            SetRXAEMNRnpeMethod(self.channel_id, WDSP_NR2_NPE_METHOD);
            SetRXAEMNRtrainZetaThresh(self.channel_id, WDSP_NR2_TRAIN_ZETA_THRESH);
            SetRXAEMNRtrainT2(self.channel_id, WDSP_NR2_TRAIN_T2);
            SetRXAEMNRaeRun(self.channel_id, 1);
            SetRXAEMNRpost2Taper(self.channel_id, nr2_taper_for_level(level));
            SetRXAEMNRpost2Nlevel(self.channel_id, nr2_nlevel_for_level(level));
            SetRXAEMNRpost2Factor(self.channel_id, nr2_factor_for_level(level));
            SetRXAEMNRpost2Rate(self.channel_id, nr2_rate_for_level(level));
            SetRXAEMNRpost2Run(
                self.channel_id,
                (matches!(self.noise_reduction_mode, NoiseReductionMode::Nr2) && level >= 1.0) as i32,
            );
            SetRXAEMNRRun(
                self.channel_id,
                matches!(self.noise_reduction_mode, NoiseReductionMode::Nr2) as i32,
            );

            SetRXARNNRPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
            SetRXARNNRRun(
                self.channel_id,
                matches!(self.noise_reduction_mode, NoiseReductionMode::Nr3) as i32,
            );

            SetRXASBNRreductionAmount(self.channel_id, nr4_reduction_amount_for_level(level));
            SetRXASBNRsmoothingFactor(self.channel_id, nr4_smoothing_factor_for_level(level));
            SetRXASBNRwhiteningFactor(self.channel_id, WDSP_NR4_WHITENING_FACTOR);
            SetRXASBNRnoiseRescale(self.channel_id, nr4_noise_rescale_for_level(level));
            SetRXASBNRpostFilterThreshold(self.channel_id, nr4_post_threshold_for_level(level));
            SetRXASBNRnoiseScalingType(self.channel_id, WDSP_NR4_NOISE_SCALING_TYPE);
            SetRXASBNRPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
            SetRXASBNRRun(
                self.channel_id,
                matches!(self.noise_reduction_mode, NoiseReductionMode::Nr4) as i32,
            );
        }
    }
}

impl Drop for WdspRxEngine {
    fn drop(&mut self) {
        if self.input_sample_rate_hz != 0 {
            unsafe {
                if self.nb_initialized {
                    destroy_anbEXT(self.channel_id);
                    destroy_nobEXT(self.channel_id);
                }
                CloseChannel(self.channel_id);
            }
        }
    }
}

// ── TX Engine ────────────────────────────────────────────────────────────────

// 240 samples per DUC IQ packet (from P2 protocol spec)
pub const DUC_IQ_SAMPLES_PER_PACKET: usize = 240;

// CFC default frequency bands (Hz) — same as pihpsdr transmitter.c
const CFC_FREQS: [f64; 10] = [50.0, 100.0, 200.0, 500.0, 1000.0, 1500.0, 2500.0, 3000.0, 5000.0, 8000.0];

pub struct WdspTxEngine {
    channel_id: i32,
    mode: DemodMode,
    mic_gain_db: f64,
    filter_low_hz: i32,
    filter_high_hz: i32,
    tx_active: bool,
    input_buffer: Vec<f64>,
    output_buffer: Vec<f64>,
    pending_mic: VecDeque<f64>,
    pub pending_iq: VecDeque<f32>,
    tx_eq_enabled: bool,
    tx_eq_bands: [i32; 11],
    cfc_enabled: bool,
    cfc_precomp_db: f64,
    cfc_bands: [f64; 10],
    two_tone_enabled: bool,
}

impl WdspTxEngine {
    pub fn new(model: &RadioModel) -> Self {
        let mut engine = Self {
            channel_id: WDSP_TX_CHANNEL,
            mode: model.desired.mode,
            mic_gain_db: model.desired.tx_mic_gain_db,
            filter_low_hz: model.desired.tx_filter_low_hz,
            filter_high_hz: model.desired.tx_filter_high_hz,
            tx_active: false,
            // DSP block size: 64 complex samples → 128 f64 values each direction
            input_buffer: vec![0.0f64; WDSP_DSP_SIZE * 2],
            output_buffer: vec![0.0f64; WDSP_DSP_SIZE * 2],
            pending_mic: VecDeque::new(),
            pending_iq: VecDeque::new(),
            tx_eq_enabled: false,
            tx_eq_bands: [0i32; 11],
            cfc_enabled: false,
            cfc_precomp_db: 0.0,
            cfc_bands: [0.0f64; 10],
            two_tone_enabled: false,
        };
        engine.open_channel();
        engine
    }

    fn open_channel(&mut self) {
        unsafe {
            OpenChannel(
                self.channel_id,
                WDSP_DSP_SIZE as i32,          // in_size
                WDSP_DSP_SIZE as i32,          // dsp_size
                WDSP_AUDIO_RATE_HZ as i32,     // input_samplerate (mic = 48 kHz)
                WDSP_AUDIO_RATE_HZ as i32,     // dsp_rate
                WDSP_AUDIO_RATE_HZ as i32,     // output_samplerate (TX IQ = 48 kHz)
                1,                             // channel_type = TX
                0,                             // state = stopped (don't run yet)
                0.010, 0.025, 0.0, 0.010,
                1,
            );
            SetTXABandpassWindow(self.channel_id, 1);
            SetTXABandpassRun(self.channel_id, 1);
            SetTXACFIRRun(self.channel_id, 1);
            SetTXAAMSQRun(self.channel_id, 0);
            SetTXAALCAttack(self.channel_id, 1);
            SetTXAALCDecay(self.channel_id, 10);
            SetTXAALCSt(self.channel_id, 1);
            SetTXAPreGenMode(self.channel_id, 0);
            SetTXAPreGenToneMag(self.channel_id, 0.0);
            SetTXAPreGenToneFreq(self.channel_id, 0.0);
            SetTXAPreGenRun(self.channel_id, 0);
            SetTXAPanelRun(self.channel_id, 1);
            SetTXAPanelSelect(self.channel_id, 2); // use mic I channel (left)
            SetTXAPostGenRun(self.channel_id, 0);
            SetTXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.mic_gain_db));
            SetTXAEQRun(self.channel_id, 0);
            SetTXABandpassFreqs(
                self.channel_id,
                self.filter_low_hz as f64,
                self.filter_high_hz as f64,
            );
            SetTXAMode(self.channel_id, wdsp_mode(self.mode));

            // CFC — initialize with flat profile, disabled
            let gains = [0.0f64; 10];
            let post_eq = [0.0f64; 10];
            SetTXACFCOMPprofile(self.channel_id, 10, CFC_FREQS.as_ptr(), gains.as_ptr(), post_eq.as_ptr());
            SetTXACFCOMPPosition(self.channel_id, 0);
            SetTXACFCOMPPrecomp(self.channel_id, 0.0);
            SetTXACFCOMPPeqRun(self.channel_id, 0);
            SetTXACFCOMPPrePeq(self.channel_id, 0.0);
            SetTXACFCOMPRun(self.channel_id, 0);

            // Two-tone test generator — configure freqs, leave disabled
            SetTXAPostGenMode(self.channel_id, 2);
            SetTXAPostGenTTMag(self.channel_id, 0.49, 0.49);
            SetTXAPostGenTTFreq(self.channel_id, 700.0, 1900.0);
            SetTXAPostGenRun(self.channel_id, 0);
        }
    }

    pub fn sync_model(&mut self, model: &RadioModel) {
        if model.desired.mode != self.mode {
            self.mode = model.desired.mode;
            unsafe { SetTXAMode(self.channel_id, wdsp_mode(self.mode)); }
        }

        if model.desired.tx_filter_low_hz != self.filter_low_hz
            || model.desired.tx_filter_high_hz != self.filter_high_hz
        {
            self.filter_low_hz = model.desired.tx_filter_low_hz;
            self.filter_high_hz = model.desired.tx_filter_high_hz;
            unsafe {
                SetTXABandpassFreqs(
                    self.channel_id,
                    self.filter_low_hz as f64,
                    self.filter_high_hz as f64,
                );
            }
        }

        if (model.desired.tx_mic_gain_db - self.mic_gain_db).abs() > f64::EPSILON {
            self.mic_gain_db = model.desired.tx_mic_gain_db;
            unsafe { SetTXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.mic_gain_db)); }
        }

        if model.desired.tx_eq_enabled != self.tx_eq_enabled
            || model.desired.tx_eq_bands != self.tx_eq_bands
        {
            self.tx_eq_enabled = model.desired.tx_eq_enabled;
            self.tx_eq_bands = model.desired.tx_eq_bands;
            unsafe {
                SetTXAGrphEQ10(self.channel_id, self.tx_eq_bands.as_mut_ptr());
                SetTXAEQRun(self.channel_id, self.tx_eq_enabled as i32);
            }
        }

        if model.desired.cfc_enabled != self.cfc_enabled
            || (model.desired.cfc_precomp_db - self.cfc_precomp_db).abs() > f64::EPSILON
            || model.desired.cfc_bands != self.cfc_bands
        {
            self.cfc_enabled = model.desired.cfc_enabled;
            self.cfc_precomp_db = model.desired.cfc_precomp_db;
            self.cfc_bands = model.desired.cfc_bands;
            let post_eq = [0.0f64; 10];
            unsafe {
                SetTXACFCOMPprofile(self.channel_id, 10, CFC_FREQS.as_ptr(), self.cfc_bands.as_ptr(), post_eq.as_ptr());
                SetTXACFCOMPPrecomp(self.channel_id, self.cfc_precomp_db);
                SetTXACFCOMPPeqRun(self.channel_id, self.cfc_enabled as i32);
                SetTXACFCOMPRun(self.channel_id, self.cfc_enabled as i32);
            }
        }

        if model.desired.two_tone_enabled != self.two_tone_enabled {
            self.two_tone_enabled = model.desired.two_tone_enabled;
            unsafe {
                SetTXAPostGenRun(self.channel_id, self.two_tone_enabled as i32);
            }
        }
    }

    /// Enable or disable the TX DSP chain. Call with `true` when PTT goes
    /// active, `false` when PTT releases (waits for slew-down before returning).
    pub fn set_active(&mut self, active: bool) {
        if active == self.tx_active {
            return;
        }
        self.tx_active = active;
        if active {
            unsafe { SetChannelState(self.channel_id, 1, 0); } // run, no wait
        } else {
            unsafe { SetChannelState(self.channel_id, 0, 1); } // stop, wait for slew-down
            self.pending_mic.clear();
            self.pending_iq.clear();
        }
    }

    /// Push mono mic audio samples (f32, normalized ±1.0) through the TX DSP
    /// chain. IQ output is accumulated in `self.pending_iq`; callers should
    /// drain it in 240-sample (DUC_IQ_SAMPLES_PER_PACKET) batches.
    pub fn push_mic(&mut self, samples: &[f32]) {
        if !self.tx_active {
            return;
        }
        // Expand mono mic to stereo interleaved [L, R] where L=mic, R=0.0
        for s in samples {
            self.pending_mic.push_back(*s as f64); // I (left = mic)
            self.pending_mic.push_back(0.0);       // Q (right = zero)
        }

        let needed = WDSP_DSP_SIZE * 2;
        while self.pending_mic.len() >= needed {
            for sample in &mut self.input_buffer {
                *sample = self.pending_mic.pop_front().unwrap_or(0.0);
            }

            let mut error = 0;
            unsafe {
                fexchange0(
                    self.channel_id,
                    self.input_buffer.as_ptr(),
                    self.output_buffer.as_mut_ptr(),
                    &mut error,
                );
            }

            if error != 0 {
                eprintln!("saturn-bridge: wdsp TX fexchange0 error {error}");
                continue;
            }

            // Output is interleaved [I, Q] IQ pairs at TX sample rate
            for sample_pair in self.output_buffer.chunks_exact(2) {
                self.pending_iq.push_back(sample_pair[0] as f32); // I
                self.pending_iq.push_back(sample_pair[1] as f32); // Q
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.tx_active
    }
}

impl Drop for WdspTxEngine {
    fn drop(&mut self) {
        unsafe { CloseChannel(self.channel_id); }
    }
}

// ── Helper functions ─────────────────────────────────────────────────────────

pub fn wdsp_mode(mode: DemodMode) -> i32 {
    match mode {
        DemodMode::Lsb => 0,
        DemodMode::Usb => 1,
        DemodMode::Cwl => 3,
        DemodMode::Cwu => 4,
        DemodMode::Fm => 5,
        DemodMode::Am => 6,
        DemodMode::DigU => 7,
        DemodMode::DigL => 9,
        DemodMode::Sam => 10,
        DemodMode::Unknown => 1,
    }
}

fn panel_gain_for_volume_db(volume_db: f64) -> f64 {
    let clamped = volume_db.clamp(-40.0, 12.0);
    if clamped <= -39.5 {
        0.0
    } else {
        10.0f64.powf(0.05 * clamped)
    }
}

fn anr_gain_for_level(level_percent: f64) -> f64 {
    let normalized = level_percent.clamp(0.0, 100.0) / 100.0;
    1.0e-4 * 16.0f64.powf(normalized)
}

fn anr_leakage_for_level(level_percent: f64) -> f64 {
    let normalized = level_percent.clamp(0.0, 100.0) / 100.0;
    10.0f64.powf(-1.0 - 5.0 * normalized)
}

fn nr2_nlevel_for_level(level_percent: f64) -> f64 {
    level_percent.clamp(0.0, 100.0) * 0.3
}

fn nr2_factor_for_level(level_percent: f64) -> f64 {
    level_percent.clamp(0.0, 100.0) * 0.3
}

fn nr2_rate_for_level(level_percent: f64) -> f64 {
    level_percent.clamp(0.0, 100.0) * 0.1
}

fn nr2_taper_for_level(level_percent: f64) -> i32 {
    (4.0 + level_percent.clamp(0.0, 100.0) * 0.16).round() as i32
}

fn nr4_reduction_amount_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.2) as f32
}

fn nr4_smoothing_factor_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.4) as f32
}

fn nr4_noise_rescale_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.04) as f32
}

fn nr4_post_threshold_for_level(level_percent: f64) -> f32 {
    (-10.0 + level_percent.clamp(0.0, 100.0) * 0.14) as f32
}

#[cfg(test)]
mod tests {
    use super::{
        anr_gain_for_level, anr_leakage_for_level, nr2_factor_for_level, nr2_nlevel_for_level,
        nr2_rate_for_level, nr2_taper_for_level, nr4_post_threshold_for_level,
        nr4_reduction_amount_for_level, panel_gain_for_volume_db,
    };

    #[test]
    fn anr_gain_scale_matches_expected_endpoints() {
        assert!((anr_gain_for_level(0.0) - 1.0e-4).abs() < 1.0e-12);
        assert!((anr_gain_for_level(100.0) - 16.0e-4).abs() < 1.0e-12);
    }

    #[test]
    fn anr_leakage_scale_matches_expected_endpoints() {
        assert!((anr_leakage_for_level(0.0) - 1.0e-1).abs() < 1.0e-12);
        assert!((anr_leakage_for_level(100.0) - 1.0e-6).abs() < 1.0e-12);
    }

    #[test]
    fn nr2_level_scale_matches_midpoint_defaults() {
        assert!((nr2_nlevel_for_level(50.0) - 15.0).abs() < 1.0e-12);
        assert!((nr2_factor_for_level(50.0) - 15.0).abs() < 1.0e-12);
        assert!((nr2_rate_for_level(50.0) - 5.0).abs() < 1.0e-12);
        assert_eq!(nr2_taper_for_level(50.0), 12);
    }

    #[test]
    fn nr4_level_scale_matches_midpoint_defaults() {
        assert!((nr4_reduction_amount_for_level(50.0) - 10.0).abs() < f32::EPSILON);
        assert!((nr4_post_threshold_for_level(50.0) + 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn panel_gain_supports_positive_monitor_gain() {
        assert!((panel_gain_for_volume_db(0.0) - 1.0).abs() < 1.0e-12);
        assert!(panel_gain_for_volume_db(12.0) > 3.9);
    }
}
