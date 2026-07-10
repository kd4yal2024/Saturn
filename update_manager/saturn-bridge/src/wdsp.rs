use std::collections::VecDeque;
use std::error::Error;
#[cfg(wdsp_has_rnnr_sbnr)]
use std::ffi::c_char;
use std::fmt;
#[cfg(wdsp_has_rnnr_sbnr)]
use std::sync::Once;
use std::time::{Duration, Instant};

use crate::radio_model::{AgcMode, DemodMode, NoiseBlankerMode, NoiseReductionMode, RadioModel};

const WDSP_RX_CHANNEL: i32 = 0;
const WDSP_TX_CHANNEL: i32 = 1;
const WDSP_AUDIO_RATE_HZ: u32 = 48_000;
const WDSP_DSP_SIZE: usize = 64;
pub const TX_MIC_SAMPLES_PER_DSP_BLOCK: usize = 512;
const WDSP_TX_DSP_SIZE: usize = 2048;
const WDSP_TX_DSP_RATE_HZ: u32 = 96_000;
pub const WDSP_TX_IQ_RATE_HZ: u32 = 192_000;
const WDSP_TX_OUTPUT_SAMPLES: usize =
    TX_MIC_SAMPLES_PER_DSP_BLOCK * (WDSP_TX_IQ_RATE_HZ as usize / WDSP_AUDIO_RATE_HZ as usize);
const WDSP_AUDIO_FRAME_FLOATS: usize = 512;
const WDSP_ANR_TAPS: i32 = 64;
const WDSP_ANR_DELAY: i32 = 16;
const WDSP_ANR_GAIN: f64 = 0.0002;
const WDSP_ANR_LEAKAGE: f64 = 0.00005;
const WDSP_ANF_TAPS: i32 = 64;
const WDSP_ANF_DELAY: i32 = 16;
const WDSP_ANF_GAIN: f64 = 0.00012;
const WDSP_ANF_LEAKAGE: f64 = 0.00008;
const WDSP_NR_POSITION_POST_AGC: i32 = 1;
const WDSP_NR2_GAIN_METHOD: i32 = 2;
const WDSP_NR2_NPE_METHOD: i32 = 0;
const WDSP_NR2_TRAIN_ZETA_THRESH: f64 = -0.5;
const WDSP_NR2_TRAIN_T2: f64 = 0.2;
#[allow(dead_code)]
const WDSP_NR4_WHITENING_FACTOR: f32 = 0.0;
#[allow(dead_code)]
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
const TXA_MIC_PK: i32 = 0;
const TXA_MIC_AV: i32 = 1;
const TXA_COMP_PK: i32 = 10;
const TXA_COMP_AV: i32 = 11;
const TXA_ALC_PK: i32 = 12;
const TXA_ALC_AV: i32 = 13;
const TXA_ALC_GAIN: i32 = 14;
const TXA_OUT_PK: i32 = 15;
const TXA_OUT_AV: i32 = 16;

#[cfg(wdsp_has_rnnr_sbnr)]
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
        id: i32,
        run: i32,
        buffsize: i32,
        samplerate: f64,
        tau: f64,
        hangtime: f64,
        advtime: f64,
        backtau: f64,
        threshold: f64,
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
        id: i32,
        run: i32,
        mode: i32,
        buffsize: i32,
        samplerate: f64,
        slewtime: f64,
        hangtime: f64,
        advtime: f64,
        backtau: f64,
        threshold: f64,
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
    fn SetRXAANFVals(channel: i32, taps: i32, delay: i32, gain: f64, leakage: f64);
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
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXARNNRRun(channel: i32, run: i32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn RNNRloadModel(file_path: *const c_char);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXARNNRPosition(channel: i32, position: i32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRRun(channel: i32, run: i32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRreductionAmount(channel: i32, amount: f32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRsmoothingFactor(channel: i32, factor: f32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRwhiteningFactor(channel: i32, factor: f32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRnoiseRescale(channel: i32, factor: f32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRpostFilterThreshold(channel: i32, threshold: f32);
    #[cfg(wdsp_has_rnnr_sbnr)]
    fn SetRXASBNRnoiseScalingType(channel: i32, noise_scaling_type: i32);
    #[cfg(wdsp_has_rnnr_sbnr)]
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
    fn SetTXAAMSQThreshold(channel: i32, threshold: f64);
    fn SetTXAAMSQMutedGain(channel: i32, db_level: f64);
    fn SetTXAALCSt(channel: i32, state: i32);
    fn SetTXAALCAttack(channel: i32, attack: i32);
    fn SetTXAALCDecay(channel: i32, decay: i32);
    fn SetTXAALCMaxGain(channel: i32, maxgain: f64);
    fn SetTXALevelerSt(channel: i32, state: i32);
    fn SetTXALevelerAttack(channel: i32, attack: i32);
    fn SetTXALevelerDecay(channel: i32, decay: i32);
    fn SetTXALevelerTop(channel: i32, maxgain: f64);
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
    fn TXASetNC(channel: i32, nc: i32);
    fn TXASetMP(channel: i32, mp: i32);
    fn GetTXAMeter(channel: i32, mt: i32) -> f64;

    // DEXP — downward expander / noise gate (operates on mic input buffer)
    #[allow(dead_code)]
    fn SetDEXPRun(id: i32, run: i32);
    #[allow(dead_code)]
    fn SetDEXPDetectorTau(id: i32, tau: f64);
    #[allow(dead_code)]
    fn SetDEXPAttackTime(id: i32, time: f64);
    #[allow(dead_code)]
    fn SetDEXPReleaseTime(id: i32, time: f64);
    #[allow(dead_code)]
    fn SetDEXPHoldTime(id: i32, time: f64);
    #[allow(dead_code)]
    fn SetDEXPExpansionRatio(id: i32, ratio: f64);
    #[allow(dead_code)]
    fn SetDEXPHysteresisRatio(id: i32, ratio: f64);
    #[allow(dead_code)]
    fn SetDEXPAttackThreshold(id: i32, thresh: f64);
    #[allow(dead_code)]
    fn SetDEXPLowCut(id: i32, lowcut: f64);
    #[allow(dead_code)]
    fn SetDEXPHighCut(id: i32, highcut: f64);
    #[allow(dead_code)]
    fn SetDEXPRunSideChannelFilter(id: i32, run: i32);
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
    anf_taps: i32,
    anf_delay: i32,
    anf_gain: f64,
    anf_leakage: f64,
    agc_mode: AgcMode,
    agc_gain: f64,
    anr_taps: i32,
    anr_delay: i32,
    anr_gain: f64,
    anr_leakage: f64,
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
    rx_fft_size: u32,
    rx_low_latency: bool,
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
            anf_taps: WDSP_ANF_TAPS,
            anf_delay: WDSP_ANF_DELAY,
            anf_gain: WDSP_ANF_GAIN,
            anf_leakage: WDSP_ANF_LEAKAGE,
            agc_mode: AgcMode::Medium,
            agc_gain: 80.0,
            anr_taps: WDSP_ANR_TAPS,
            anr_delay: WDSP_ANR_DELAY,
            anr_gain: WDSP_ANR_GAIN,
            anr_leakage: WDSP_ANR_LEAKAGE,
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
            rx_fft_size: model.desired.rx_fft_size,
            rx_low_latency: model.desired.rx_low_latency,
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
            unsafe {
                SetRXAMode(self.channel_id, wdsp_mode(self.mode));
            }
            self.apply_agc();
        }

        if (model.desired.rx_volume_db - self.volume_db).abs() > f64::EPSILON {
            self.volume_db = model.desired.rx_volume_db;
            unsafe {
                SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db));
            }
        }

        if model.desired.rx_noise_reduction_mode != self.noise_reduction_mode
            || (model.desired.rx_noise_reduction_level - self.noise_reduction_level).abs()
                > f64::EPSILON
            || model.desired.rx_anr_taps != self.anr_taps
            || model.desired.rx_anr_delay != self.anr_delay
            || (model.desired.rx_anr_gain - self.anr_gain).abs() > f64::EPSILON
            || (model.desired.rx_anr_leakage - self.anr_leakage).abs() > f64::EPSILON
        {
            self.noise_reduction_mode = model.desired.rx_noise_reduction_mode;
            self.noise_reduction_level = model.desired.rx_noise_reduction_level;
            self.anr_taps = model.desired.rx_anr_taps;
            self.anr_delay = model.desired.rx_anr_delay;
            self.anr_gain = model.desired.rx_anr_gain;
            self.anr_leakage = model.desired.rx_anr_leakage;
            self.apply_noise_reduction();
        }

        if model.desired.nb_mode != self.nb_mode
            || (model.desired.nb_threshold - self.nb_threshold).abs() > f64::EPSILON
        {
            self.nb_mode = model.desired.nb_mode;
            self.nb_threshold = model.desired.nb_threshold;
            self.apply_noise_blanker();
        }

        if model.desired.anf_enabled != self.anf_enabled
            || model.desired.rx_anf_taps != self.anf_taps
            || model.desired.rx_anf_delay != self.anf_delay
            || (model.desired.rx_anf_gain - self.anf_gain).abs() > f64::EPSILON
            || (model.desired.rx_anf_leakage - self.anf_leakage).abs() > f64::EPSILON
        {
            self.anf_enabled = model.desired.anf_enabled;
            self.anf_taps = model.desired.rx_anf_taps;
            self.anf_delay = model.desired.rx_anf_delay;
            self.anf_gain = model.desired.rx_anf_gain;
            self.anf_leakage = model.desired.rx_anf_leakage;
            self.apply_anf();
        }

        if model.desired.agc_mode != self.agc_mode {
            self.agc_mode = model.desired.agc_mode;
            self.apply_agc();
        }

        if (model.desired.agc_gain - self.agc_gain).abs() > f64::EPSILON {
            self.agc_gain = model.desired.agc_gain;
            self.apply_agc();
        }

        if model.desired.filter_low_hz != self.filter_low_hz
            || model.desired.filter_high_hz != self.filter_high_hz
        {
            self.filter_low_hz = model.desired.filter_low_hz;
            self.filter_high_hz = model.desired.filter_high_hz;
            unsafe {
                RXASetPassband(
                    self.channel_id,
                    self.filter_low_hz as f64,
                    self.filter_high_hz as f64,
                );
            }
        }

        if model.desired.rx_fft_size != self.rx_fft_size
            || model.desired.rx_low_latency != self.rx_low_latency
        {
            self.rx_fft_size = model.desired.rx_fft_size;
            self.rx_low_latency = model.desired.rx_low_latency;
            unsafe {
                RXASetNC(self.channel_id, self.rx_fft_size as i32);
                RXASetMP(self.channel_id, self.rx_low_latency as i32);
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

    pub fn reset_audio_packetizer(&mut self) {
        self.pending_audio.clear();
    }

    fn reconfigure(&mut self, model: &RadioModel) -> Result<(), WdspError> {
        let input_sample_rate_hz = model.desired.ddc0_sample_rate_khz as u32 * 1000;
        let ratio = input_sample_rate_hz / WDSP_AUDIO_RATE_HZ;
        if ratio == 0 || input_sample_rate_hz % WDSP_AUDIO_RATE_HZ != 0 || !ratio.is_power_of_two()
        {
            return Err(WdspError::UnsupportedInputSampleRate(input_sample_rate_hz));
        }

        if self.input_sample_rate_hz != 0 {
            unsafe {
                CloseChannel(self.channel_id);
            }
        }

        #[cfg(wdsp_has_rnnr_sbnr)]
        LOAD_RNNR_MODEL.call_once(|| unsafe {
            RNNRloadModel(std::ptr::null());
        });

        self.input_sample_rate_hz = input_sample_rate_hz;
        self.rx_fft_size = model.desired.rx_fft_size;
        self.rx_low_latency = model.desired.rx_low_latency;
        self.mode = model.desired.mode;
        self.volume_db = model.desired.rx_volume_db;
        self.noise_reduction_mode = model.desired.rx_noise_reduction_mode;
        self.noise_reduction_level = model.desired.rx_noise_reduction_level;
        self.nb_mode = model.desired.nb_mode;
        self.nb_threshold = model.desired.nb_threshold;
        self.anf_enabled = model.desired.anf_enabled;
        self.anf_taps = model.desired.rx_anf_taps;
        self.anf_delay = model.desired.rx_anf_delay;
        self.anf_gain = model.desired.rx_anf_gain;
        self.anf_leakage = model.desired.rx_anf_leakage;
        self.agc_mode = model.desired.agc_mode;
        self.agc_gain = model.desired.agc_gain;
        self.anr_taps = model.desired.rx_anr_taps;
        self.anr_delay = model.desired.rx_anr_delay;
        self.anr_gain = model.desired.rx_anr_gain;
        self.anr_leakage = model.desired.rx_anr_leakage;
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
            SetRXAPanelCopy(self.channel_id, 1); // copy I→Q so both speakers get audio
            SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db));
            RXASetNC(self.channel_id, model.desired.rx_fft_size as i32);
            // Thetis defaults to Low_Latency (MP=1) for both RX and TX.
            // Linear phase (MP=0) adds NC/2 samples of group delay;
            // minimum phase (MP=1) cuts that drastically.
            RXASetMP(self.channel_id, self.rx_low_latency as i32);
            SetRXAMode(self.channel_id, wdsp_mode(self.mode));
            RXASetPassband(
                self.channel_id,
                self.filter_low_hz as f64,
                self.filter_high_hz as f64,
            );

            // Create (or re-create) the EXT NB objects for this channel
            if self.nb_initialized {
                destroy_anbEXT(self.channel_id);
                destroy_nobEXT(self.channel_id);
            }
            let sr = self.input_sample_rate_hz as f64;
            create_anbEXT(
                self.channel_id,
                1,
                WDSP_DSP_SIZE as i32,
                sr,
                0.0001,
                0.0001,
                0.0001,
                0.05,
                20.0,
            );
            create_nobEXT(
                self.channel_id,
                1,
                0,
                WDSP_DSP_SIZE as i32,
                sr,
                0.0001,
                0.0001,
                0.0001,
                0.05,
                20.0,
            );
        }
        self.nb_initialized = true;
        unsafe {
            SetRXAEQRun(self.channel_id, 0);
        }
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
            SetRXAAGCTop(self.channel_id, self.agc_gain.clamp(0.0, 100.0));
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
            SetEXTANBRun(
                self.channel_id,
                (self.nb_mode == NoiseBlankerMode::Nb1) as i32,
            );

            SetEXTNOBMode(self.channel_id, 0); // zero mode
            SetEXTNOBTau(self.channel_id, NB_TAU);
            SetEXTNOBHangtime(self.channel_id, NB_HANGTIME);
            SetEXTNOBAdvtime(self.channel_id, NB_ADVTIME);
            SetEXTNOBThreshold(self.channel_id, threshold);
            SetEXTNOBRun(
                self.channel_id,
                (self.nb_mode == NoiseBlankerMode::Nb2) as i32,
            );
        }
    }

    fn apply_anf(&self) {
        unsafe {
            SetRXAANFVals(
                self.channel_id,
                self.anf_taps,
                self.anf_delay,
                self.anf_gain,
                self.anf_leakage,
            );
            SetRXAANFRun(self.channel_id, self.anf_enabled as i32);
            SetRXAANFPosition(self.channel_id, WDSP_NR_POSITION_POST_AGC);
        }
    }

    fn apply_noise_reduction(&self) {
        let level = self.noise_reduction_level.clamp(0.0, 100.0);
        let emnr_active = matches!(self.noise_reduction_mode, NoiseReductionMode::Nr2)
            || (!cfg!(wdsp_has_rnnr_sbnr)
                && matches!(
                    self.noise_reduction_mode,
                    NoiseReductionMode::Nr3 | NoiseReductionMode::Nr4
                ));
        unsafe {
            SetRXAANRVals(
                self.channel_id,
                self.anr_taps,
                self.anr_delay,
                self.anr_gain,
                self.anr_leakage,
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
            SetRXAEMNRpost2Run(self.channel_id, (emnr_active && level >= 1.0) as i32);
            SetRXAEMNRRun(self.channel_id, emnr_active as i32);

            #[cfg(wdsp_has_rnnr_sbnr)]
            {
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
const CFC_FREQS: [f64; 10] = [
    50.0, 100.0, 200.0, 500.0, 1000.0, 1500.0, 2500.0, 3000.0, 5000.0, 8000.0,
];

pub struct WdspTxEngine {
    channel_id: i32,
    mode: DemodMode,
    mic_gain_db: f64,
    filter_low_hz: i32,
    filter_high_hz: i32,
    tx_fft_size: u32,
    tx_low_latency: bool,
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
    two_tone_freq1_hz: f64,
    two_tone_freq2_hz: f64,
    two_tone_level_db: f64,
    two_tone_invert_lsb: bool,
    two_tone_delay_ms: u16,
    two_tone_started_at: Option<Instant>,
    two_tone_second_active: bool,
    noise_gate_enabled: bool,
    noise_gate_threshold_db: f64,
    last_input_peak: f32,
    last_output_peak: f32,
    total_input_samples: u64,
    total_output_pairs: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct TxDspDiagnostics {
    pub input_peak: f32,
    pub output_peak: f32,
    pub total_input_samples: u64,
    pub total_output_pairs: u64,
    pub pending_mic_floats: usize,
    pub pending_iq_floats: usize,
    pub mic_peak_db: f64,
    pub mic_avg_db: f64,
    pub comp_peak_db: f64,
    pub comp_avg_db: f64,
    pub alc_peak_db: f64,
    pub alc_avg_db: f64,
    pub alc_gain_db: f64,
    pub out_peak_db: f64,
    pub out_avg_db: f64,
}

impl WdspTxEngine {
    pub fn new(model: &RadioModel) -> Self {
        let mut engine = Self {
            channel_id: WDSP_TX_CHANNEL,
            mode: model.desired.mode,
            mic_gain_db: model.desired.tx_mic_gain_db,
            filter_low_hz: model.desired.tx_filter_low_hz,
            filter_high_hz: model.desired.tx_filter_high_hz,
            tx_fft_size: model.desired.tx_fft_size,
            tx_low_latency: model.desired.tx_low_latency,
            tx_active: false,
            // Match piHPSDR's Protocol 2 TX shape: 512 mic samples in,
            // 2048 IQ pairs out at 192 kHz.
            input_buffer: vec![0.0f64; TX_MIC_SAMPLES_PER_DSP_BLOCK * 2],
            output_buffer: vec![0.0f64; WDSP_TX_OUTPUT_SAMPLES * 2],
            pending_mic: VecDeque::new(),
            pending_iq: VecDeque::new(),
            tx_eq_enabled: false,
            tx_eq_bands: [0i32; 11],
            cfc_enabled: false,
            cfc_precomp_db: 0.0,
            cfc_bands: [0.0f64; 10],
            two_tone_enabled: false,
            two_tone_freq1_hz: model.desired.tx_two_tone_freq1_hz,
            two_tone_freq2_hz: model.desired.tx_two_tone_freq2_hz,
            two_tone_level_db: model.desired.tx_two_tone_level_db,
            two_tone_invert_lsb: model.desired.tx_two_tone_invert_lsb,
            two_tone_delay_ms: model.desired.tx_two_tone_delay_ms,
            two_tone_started_at: None,
            two_tone_second_active: false,
            noise_gate_enabled: model.desired.tx_noise_gate_enabled,
            noise_gate_threshold_db: model.desired.tx_noise_gate_threshold_db,
            last_input_peak: 0.0,
            last_output_peak: 0.0,
            total_input_samples: 0,
            total_output_pairs: 0,
        };
        engine.open_channel();
        engine
    }

    fn open_channel(&mut self) {
        unsafe {
            OpenChannel(
                self.channel_id,
                TX_MIC_SAMPLES_PER_DSP_BLOCK as i32, // in_size
                WDSP_TX_DSP_SIZE as i32,             // dsp_size
                WDSP_AUDIO_RATE_HZ as i32,           // input_samplerate (mic = 48 kHz)
                WDSP_TX_DSP_RATE_HZ as i32,          // dsp_rate
                WDSP_TX_IQ_RATE_HZ as i32,           // output_samplerate (P2 TX IQ = 192 kHz)
                1,                                   // channel_type = TX
                0,                                   // state = stopped (don't run yet)
                0.010,
                0.025,
                0.0,
                0.010,
                1,
            );
            SetTXABandpassWindow(self.channel_id, 1);
            SetTXABandpassRun(self.channel_id, 1);
            SetTXACFIRRun(self.channel_id, 1);
            SetTXAAMSQThreshold(self.channel_id, self.noise_gate_threshold_db);
            SetTXAAMSQMutedGain(self.channel_id, -140.0);
            SetTXAAMSQRun(self.channel_id, self.noise_gate_enabled as i32);
            SetTXAALCAttack(self.channel_id, 1);
            SetTXAALCDecay(self.channel_id, 10);
            SetTXAALCMaxGain(self.channel_id, 0.0);
            SetTXAALCSt(self.channel_id, 1);
            SetTXALevelerAttack(self.channel_id, 1);
            SetTXALevelerDecay(self.channel_id, 500);
            SetTXALevelerTop(self.channel_id, 8.0);
            // Match piHPSDR: leveler only runs when CFC is enabled
            SetTXALevelerSt(self.channel_id, self.cfc_enabled as i32);
            SetTXAPreGenMode(self.channel_id, 0);
            SetTXAPreGenToneMag(self.channel_id, 0.0);
            SetTXAPreGenToneFreq(self.channel_id, 0.0);
            SetTXAPreGenRun(self.channel_id, 0);
            SetTXAPanelRun(self.channel_id, 1);
            SetTXAPanelSelect(self.channel_id, 2); // use mic I channel (left)
            SetTXAPostGenRun(self.channel_id, 0);
            SetTXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.mic_gain_db));
            SetTXAEQRun(self.channel_id, 0);
            TXASetNC(self.channel_id, self.tx_fft_size as i32);
            TXASetMP(self.channel_id, self.tx_low_latency as i32);
            SetTXABandpassFreqs(
                self.channel_id,
                self.filter_low_hz as f64,
                self.filter_high_hz as f64,
            );
            SetTXAMode(self.channel_id, wdsp_mode(self.mode));

            // CFC — initialize with flat profile, disabled
            let gains = [0.0f64; 10];
            let post_eq = [0.0f64; 10];
            SetTXACFCOMPprofile(
                self.channel_id,
                10,
                CFC_FREQS.as_ptr(),
                gains.as_ptr(),
                post_eq.as_ptr(),
            );
            SetTXACFCOMPPosition(self.channel_id, 0);
            SetTXACFCOMPPrecomp(self.channel_id, 0.0);
            SetTXACFCOMPPeqRun(self.channel_id, 0);
            SetTXACFCOMPPrePeq(self.channel_id, 0.0);
            SetTXACFCOMPRun(self.channel_id, 0);

            // Two-tone test generator — configure Thetis-style continuous two-tone, leave disabled
            SetTXAPostGenMode(self.channel_id, 1);
            SetTXAPostGenTTMag(
                self.channel_id,
                two_tone_magnitude(0.0),
                two_tone_magnitude(0.0),
            );
            SetTXAPostGenTTFreq(self.channel_id, 700.0, 1900.0);
            SetTXAPostGenRun(self.channel_id, 0);
        }
    }

    pub fn sync_model(&mut self, model: &RadioModel) {
        if model.desired.mode != self.mode {
            self.mode = model.desired.mode;
            unsafe {
                SetTXAMode(self.channel_id, wdsp_mode(self.mode));
            }
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
            unsafe {
                SetTXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.mic_gain_db));
            }
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
                SetTXACFCOMPprofile(
                    self.channel_id,
                    10,
                    CFC_FREQS.as_ptr(),
                    self.cfc_bands.as_ptr(),
                    post_eq.as_ptr(),
                );
                SetTXACFCOMPPrecomp(self.channel_id, self.cfc_precomp_db);
                SetTXACFCOMPPeqRun(self.channel_id, self.cfc_enabled as i32);
                SetTXACFCOMPRun(self.channel_id, self.cfc_enabled as i32);
                // Leveler only runs when CFC is active (matches piHPSDR)
                SetTXALevelerSt(self.channel_id, self.cfc_enabled as i32);
            }
        }

        if model.desired.tx_noise_gate_enabled != self.noise_gate_enabled
            || (model.desired.tx_noise_gate_threshold_db - self.noise_gate_threshold_db).abs()
                > f64::EPSILON
        {
            self.noise_gate_enabled = model.desired.tx_noise_gate_enabled;
            self.noise_gate_threshold_db = model.desired.tx_noise_gate_threshold_db;
            unsafe {
                SetTXAAMSQThreshold(self.channel_id, self.noise_gate_threshold_db);
                SetTXAAMSQRun(self.channel_id, self.noise_gate_enabled as i32);
            }
        }

        if model.desired.tx_fft_size != self.tx_fft_size
            || model.desired.tx_low_latency != self.tx_low_latency
        {
            self.tx_fft_size = model.desired.tx_fft_size;
            self.tx_low_latency = model.desired.tx_low_latency;
            unsafe {
                TXASetNC(self.channel_id, self.tx_fft_size as i32);
                TXASetMP(self.channel_id, self.tx_low_latency as i32);
            }
        }

        if model.desired.two_tone_enabled != self.two_tone_enabled {
            self.two_tone_enabled = model.desired.two_tone_enabled;
            self.two_tone_started_at = self.two_tone_enabled.then(Instant::now);
            self.two_tone_second_active =
                self.two_tone_enabled && model.desired.tx_two_tone_delay_ms == 0;
            unsafe {
                SetTXAPostGenRun(self.channel_id, self.two_tone_enabled as i32);
            }
        }

        if (model.desired.tx_two_tone_freq1_hz - self.two_tone_freq1_hz).abs() > f64::EPSILON
            || (model.desired.tx_two_tone_freq2_hz - self.two_tone_freq2_hz).abs() > f64::EPSILON
            || model.desired.tx_two_tone_invert_lsb != self.two_tone_invert_lsb
            || model.desired.mode != self.mode
        {
            self.two_tone_freq1_hz = model.desired.tx_two_tone_freq1_hz;
            self.two_tone_freq2_hz = model.desired.tx_two_tone_freq2_hz;
            self.two_tone_invert_lsb = model.desired.tx_two_tone_invert_lsb;
            let (freq1_hz, freq2_hz) = signed_two_tone_freqs(
                self.mode,
                self.two_tone_invert_lsb,
                self.two_tone_freq1_hz,
                self.two_tone_freq2_hz,
            );
            unsafe {
                SetTXAPostGenTTFreq(self.channel_id, freq1_hz, freq2_hz);
            }
        }

        if (model.desired.tx_two_tone_level_db - self.two_tone_level_db).abs() > f64::EPSILON
            || model.desired.tx_two_tone_delay_ms != self.two_tone_delay_ms
            || model.desired.two_tone_enabled != self.two_tone_enabled
        {
            self.two_tone_level_db = model.desired.tx_two_tone_level_db;
            self.two_tone_delay_ms = model.desired.tx_two_tone_delay_ms;
            if self.two_tone_enabled {
                self.apply_two_tone_magnitudes();
            }
        } else if self.two_tone_enabled {
            self.apply_two_tone_magnitudes();
        }
    }

    fn apply_two_tone_magnitudes(&mut self) {
        let mag = two_tone_magnitude(self.two_tone_level_db);
        let second_ready = self.two_tone_delay_ms == 0
            || self
                .two_tone_started_at
                .map(|started| {
                    started.elapsed() >= Duration::from_millis(u64::from(self.two_tone_delay_ms))
                })
                .unwrap_or(false);
        if second_ready != self.two_tone_second_active {
            self.two_tone_second_active = second_ready;
        }
        unsafe {
            SetTXAPostGenTTMag(
                self.channel_id,
                mag,
                if self.two_tone_second_active {
                    mag
                } else {
                    0.0
                },
            );
        }
    }

    fn clear_tx_buffers(&mut self) {
        self.pending_mic.clear();
        self.pending_iq.clear();
        self.input_buffer.fill(0.0);
        self.output_buffer.fill(0.0);
        self.last_input_peak = 0.0;
        self.last_output_peak = 0.0;
        self.total_input_samples = 0;
        self.total_output_pairs = 0;
    }

    /// Enable or disable the TX DSP chain. PTT is a control-plane event for
    /// the remote bridge; never wait for WDSP slew-down on release or the main
    /// loop cannot send the immediate RX-recovery high-priority packet.
    pub fn set_active(&mut self, active: bool) {
        if active == self.tx_active {
            return;
        }
        self.tx_active = active;
        if active {
            self.clear_tx_buffers();
            unsafe {
                SetChannelState(self.channel_id, 1, 0);
            } // run, no wait
        } else {
            // Do not stop the WDSP TX channel here. On the G2 test path,
            // stopping/restarting the channel after the first MOX cycle can
            // leave later mic frames producing zero TX IQ until the bridge
            // process restarts. The TX thread stops RF and DUC packet output
            // separately; while tx_active is false, no fexchange0 calls run.
            self.clear_tx_buffers();
            self.two_tone_enabled = false;
            self.two_tone_started_at = None;
            self.two_tone_second_active = false;
        }
    }

    /// Push mono mic audio samples (f32, normalized ±1.0) through the TX DSP
    /// chain. IQ output is accumulated in `self.pending_iq`; callers should
    /// drain it in 240-sample (DUC_IQ_SAMPLES_PER_PACKET) batches.
    pub fn push_mic(&mut self, samples: &[f32]) {
        if !self.tx_active {
            return;
        }
        let input_peak = samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        self.last_input_peak = input_peak;
        self.total_input_samples = self
            .total_input_samples
            .saturating_add(samples.len() as u64);

        // Expand mono mic to stereo interleaved [L, R] where L=mic, R=0.0
        for s in samples {
            self.pending_mic.push_back(*s as f64); // I (left = mic)
            self.pending_mic.push_back(0.0); // Q (right = zero)
        }

        let needed = TX_MIC_SAMPLES_PER_DSP_BLOCK * 2;
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
            let mut output_peak = 0.0f32;
            for sample_pair in self.output_buffer.chunks_exact(2) {
                let i_sample = sample_pair[0] as f32;
                let q_sample = sample_pair[1] as f32;
                output_peak = output_peak.max(i_sample.abs()).max(q_sample.abs());
                self.pending_iq.push_back(i_sample); // I
                self.pending_iq.push_back(q_sample); // Q
            }
            self.last_output_peak = output_peak;
            self.total_output_pairs = self
                .total_output_pairs
                .saturating_add(WDSP_TX_OUTPUT_SAMPLES as u64);
        }
    }

    pub fn diagnostics(&self) -> TxDspDiagnostics {
        unsafe {
            TxDspDiagnostics {
                input_peak: self.last_input_peak,
                output_peak: self.last_output_peak,
                total_input_samples: self.total_input_samples,
                total_output_pairs: self.total_output_pairs,
                pending_mic_floats: self.pending_mic.len(),
                pending_iq_floats: self.pending_iq.len(),
                mic_peak_db: GetTXAMeter(self.channel_id, TXA_MIC_PK),
                mic_avg_db: GetTXAMeter(self.channel_id, TXA_MIC_AV),
                comp_peak_db: GetTXAMeter(self.channel_id, TXA_COMP_PK),
                comp_avg_db: GetTXAMeter(self.channel_id, TXA_COMP_AV),
                alc_peak_db: GetTXAMeter(self.channel_id, TXA_ALC_PK),
                alc_avg_db: GetTXAMeter(self.channel_id, TXA_ALC_AV),
                alc_gain_db: GetTXAMeter(self.channel_id, TXA_ALC_GAIN),
                out_peak_db: GetTXAMeter(self.channel_id, TXA_OUT_PK),
                out_avg_db: GetTXAMeter(self.channel_id, TXA_OUT_AV),
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.tx_active
    }
}

fn two_tone_magnitude(level_db: f64) -> f64 {
    0.49999 * 10f64.powf(level_db.clamp(-40.0, 0.0) / 20.0)
}

fn mode_inverts_two_tone(mode: DemodMode) -> bool {
    matches!(mode, DemodMode::Lsb | DemodMode::DigL | DemodMode::Cwl)
}

fn signed_two_tone_freqs(
    mode: DemodMode,
    invert_lsb: bool,
    freq1_hz: f64,
    freq2_hz: f64,
) -> (f64, f64) {
    if invert_lsb && mode_inverts_two_tone(mode) {
        (-freq1_hz, -freq2_hz)
    } else {
        (freq1_hz, freq2_hz)
    }
}

impl Drop for WdspTxEngine {
    fn drop(&mut self) {
        unsafe {
            CloseChannel(self.channel_id);
        }
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

#[allow(dead_code)]
fn nr4_reduction_amount_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.2) as f32
}

#[allow(dead_code)]
fn nr4_smoothing_factor_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.4) as f32
}

#[allow(dead_code)]
fn nr4_noise_rescale_for_level(level_percent: f64) -> f32 {
    (level_percent.clamp(0.0, 100.0) * 0.04) as f32
}

#[allow(dead_code)]
fn nr4_post_threshold_for_level(level_percent: f64) -> f32 {
    (-10.0 + level_percent.clamp(0.0, 100.0) * 0.14) as f32
}

#[cfg(test)]
mod tests {
    use super::{
        nr2_factor_for_level, nr2_nlevel_for_level, nr2_rate_for_level, nr2_taper_for_level,
        nr4_post_threshold_for_level, nr4_reduction_amount_for_level, panel_gain_for_volume_db,
    };

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
