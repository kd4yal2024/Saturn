use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

use crate::radio_model::{DemodMode, RadioModel};

const WDSP_RX_CHANNEL: i32 = 0;
const WDSP_AUDIO_RATE_HZ: u32 = 48_000;
const WDSP_DSP_SIZE: usize = 64;
const WDSP_AUDIO_FRAME_FLOATS: usize = 2048;

const AGC_MEDIUM: i32 = 3;
const AGC_FAST: i32 = 4;

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
    fn fexchange0(channel: i32, input: *const f64, output: *mut f64, error: *mut i32);
    fn SetRXAMode(channel: i32, mode: i32);
    fn RXASetPassband(channel: i32, low_hz: f64, high_hz: f64);
    fn RXASetNC(channel: i32, nc: i32);
    fn RXASetMP(channel: i32, mp: i32);
    fn SetRXABandpassWindow(channel: i32, wintype: i32);
    fn SetRXABandpassRun(channel: i32, run: i32);
    fn SetRXAAMDSBMode(channel: i32, sbmode: i32);
    fn SetRXAPanelRun(channel: i32, run: i32);
    fn SetRXAPanelSelect(channel: i32, select: i32);
    fn SetRXAPanelGain1(channel: i32, gain: f64);
    fn SetRXAAGCMode(channel: i32, mode: i32);
    fn SetRXAAGCSlope(channel: i32, slope: i32);
    fn SetRXAAGCTop(channel: i32, max_agc: f64);
    fn SetRXAAGCAttack(channel: i32, attack: i32);
    fn SetRXAAGCHang(channel: i32, hang: i32);
    fn SetRXAAGCDecay(channel: i32, decay: i32);
    fn SetRXAAGCHangThreshold(channel: i32, hangthreshold: i32);
    fn SetRXAAGCMaxInputLevel(channel: i32, level: f64);
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

pub struct WdspRxEngine {
    channel_id: i32,
    input_sample_rate_hz: u32,
    mode: DemodMode,
    volume_db: f64,
    filter_low_hz: i32,
    filter_high_hz: i32,
    input_complex_samples: usize,
    output_audio_frames: usize,
    input_buffer: Vec<f64>,
    output_buffer: Vec<f64>,
    pending_iq: VecDeque<f64>,
    pending_audio: VecDeque<f32>,
    frame_float_count: usize,
}

impl WdspRxEngine {
    pub fn new(model: &RadioModel) -> Result<Self, WdspError> {
        let mut engine = Self {
            channel_id: WDSP_RX_CHANNEL,
            input_sample_rate_hz: 0,
            mode: DemodMode::Unknown,
            volume_db: -20.0,
            filter_low_hz: 0,
            filter_high_hz: 0,
            input_complex_samples: 0,
            output_audio_frames: 0,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
            pending_iq: VecDeque::new(),
            pending_audio: VecDeque::new(),
            frame_float_count: WDSP_AUDIO_FRAME_FLOATS,
        };
        engine.reconfigure(model)?;
        Ok(engine)
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
            self.apply_default_agc();
        }

        if (model.desired.rx_volume_db - self.volume_db).abs() > f64::EPSILON {
            self.volume_db = model.desired.rx_volume_db;
            unsafe {
                SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db));
            }
        }

        if model.desired.filter_low_hz != self.filter_low_hz || model.desired.filter_high_hz != self.filter_high_hz {
            self.filter_low_hz = model.desired.filter_low_hz;
            self.filter_high_hz = model.desired.filter_high_hz;
            unsafe {
                RXASetPassband(self.channel_id, self.filter_low_hz as f64, self.filter_high_hz as f64);
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
                continue;
            }

            for sample in &self.output_buffer {
                self.pending_audio
                    .push_back((*sample as f32).clamp(-1.0, 1.0));
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

    fn reconfigure(&mut self, model: &RadioModel) -> Result<(), WdspError> {
        let input_sample_rate_hz = model.desired.ddc0_sample_rate_khz as u32 * 1000;
        let ratio = input_sample_rate_hz / WDSP_AUDIO_RATE_HZ;
        if ratio == 0 || input_sample_rate_hz % WDSP_AUDIO_RATE_HZ != 0 || !ratio.is_power_of_two() {
            return Err(WdspError::UnsupportedInputSampleRate(input_sample_rate_hz));
        }

        if self.input_sample_rate_hz != 0 {
            unsafe {
                CloseChannel(self.channel_id);
            }
        }

        self.input_sample_rate_hz = input_sample_rate_hz;
        self.mode = model.desired.mode;
        self.volume_db = model.desired.rx_volume_db;
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
            SetRXAPanelSelect(self.channel_id, 3);
            SetRXAPanelGain1(self.channel_id, panel_gain_for_volume_db(self.volume_db));
            RXASetNC(self.channel_id, 2048);
            RXASetMP(self.channel_id, 0);
            SetRXAMode(self.channel_id, wdsp_mode(self.mode));
            RXASetPassband(self.channel_id, self.filter_low_hz as f64, self.filter_high_hz as f64);
        }
        self.apply_default_agc();
        Ok(())
    }

    fn apply_default_agc(&self) {
        let agc_mode = default_agc_mode(self.mode);
        unsafe {
            SetRXAAGCMode(self.channel_id, agc_mode);
            SetRXAAGCSlope(self.channel_id, 35);
            SetRXAAGCTop(self.channel_id, 80.0);
            SetRXAAGCMaxInputLevel(self.channel_id, 1.0);
            match agc_mode {
                AGC_FAST => {
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 0);
                    SetRXAAGCDecay(self.channel_id, 50);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
                _ => {
                    SetRXAAGCAttack(self.channel_id, 2);
                    SetRXAAGCHang(self.channel_id, 0);
                    SetRXAAGCDecay(self.channel_id, 250);
                    SetRXAAGCHangThreshold(self.channel_id, 100);
                }
            }
        }
    }
}

impl Drop for WdspRxEngine {
    fn drop(&mut self) {
        if self.input_sample_rate_hz != 0 {
            unsafe {
                CloseChannel(self.channel_id);
            }
        }
    }
}

fn wdsp_mode(mode: DemodMode) -> i32 {
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

fn default_agc_mode(mode: DemodMode) -> i32 {
    match mode {
        DemodMode::DigU | DemodMode::DigL | DemodMode::Cwl | DemodMode::Cwu => AGC_FAST,
        _ => AGC_MEDIUM,
    }
}

fn panel_gain_for_volume_db(volume_db: f64) -> f64 {
    if volume_db <= -39.5 {
        0.0
    } else if volume_db >= 0.0 {
        1.0
    } else {
        10.0f64.powf(0.05 * volume_db)
    }
}
