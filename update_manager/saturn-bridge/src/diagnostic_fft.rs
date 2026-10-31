//! Existing display FFT, shared with offline TX diagnostics. Numerical display behavior is unchanged.
/// Radix-2 FFT that reproduces `remote-web/src/dsp/fft.ts` exactly: the same
/// Hann window, bit-reversal, butterfly order, fftshift and dB formula. JS
/// computes in f64 and stores into Float32Arrays, so this does the same.
pub struct SpectrumFft {
    size: usize,
    window: Box<[f32]>,
    bit_reverse: Box<[u32]>,
    cos_table: Box<[f32]>,
    sin_table: Box<[f32]>,
    scratch_re: Box<[f32]>,
    scratch_im: Box<[f32]>,
}

impl SpectrumFft {
    pub fn new(size: usize) -> Self {
        Self::with_window(size, |i, n| {
            0.5 - 0.5 * ((2.0 * std::f64::consts::PI * i as f64) / (n - 1) as f64).cos()
        })
    }

    /// Analysis-only caller-selected window. Existing display callers retain Hann.
    pub fn with_window(size: usize, value: impl Fn(usize, usize) -> f64) -> Self {
        assert!(size.is_power_of_two() && size >= 2);
        let mut window = vec![0.0f32; size];
        let mut bit_reverse = vec![0u32; size];
        for (i, (w, reversed)) in window.iter_mut().zip(bit_reverse.iter_mut()).enumerate() {
            *w = value(i, size) as f32;
            let mut j = 0usize;
            let mut bit = 0;
            while (1usize << bit) < size {
                j = (j << 1) | ((i >> bit) & 1);
                bit += 1;
            }
            *reversed = j as u32;
        }
        let half = size / 2;
        let mut cos_table = vec![0.0f32; half];
        let mut sin_table = vec![0.0f32; half];
        for i in 0..half {
            let angle = (-2.0 * std::f64::consts::PI * i as f64) / size as f64;
            cos_table[i] = angle.cos() as f32;
            sin_table[i] = angle.sin() as f32;
        }
        Self {
            size,
            window: window.into_boxed_slice(),
            bit_reverse: bit_reverse.into_boxed_slice(),
            cos_table: cos_table.into_boxed_slice(),
            sin_table: sin_table.into_boxed_slice(),
            scratch_re: vec![0.0; size].into_boxed_slice(),
            scratch_im: vec![0.0; size].into_boxed_slice(),
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// `iq` must hold exactly `size` contiguous interleaved pairs (stride 1).
    /// Writes fftshifted dB bins into `out`.
    pub fn transform_db(&mut self, iq: &[f32], out: &mut [f32]) {
        let size = self.size;
        assert_eq!(iq.len(), size * 2);
        assert_eq!(out.len(), size);
        for i in 0..size {
            let w = f64::from(self.window[i]);
            let target = self.bit_reverse[i] as usize;
            self.scratch_re[target] = (f64::from(iq[i * 2]) * w) as f32;
            self.scratch_im[target] = (f64::from(iq[i * 2 + 1]) * w) as f32;
        }

        let mut span = 2;
        while span <= size {
            let half = span >> 1;
            let table_step = size / span;
            let mut start = 0;
            while start < size {
                for i in 0..half {
                    let twiddle = i * table_step;
                    let even = start + i;
                    let odd = even + half;
                    let odd_re = f64::from(self.scratch_re[odd]);
                    let odd_im = f64::from(self.scratch_im[odd]);
                    let tw_re = f64::from(self.cos_table[twiddle]);
                    let tw_im = f64::from(self.sin_table[twiddle]);
                    let rot_re = odd_re * tw_re - odd_im * tw_im;
                    let rot_im = odd_re * tw_im + odd_im * tw_re;
                    let even_re = f64::from(self.scratch_re[even]);
                    let even_im = f64::from(self.scratch_im[even]);
                    self.scratch_re[odd] = (even_re - rot_re) as f32;
                    self.scratch_im[odd] = (even_im - rot_im) as f32;
                    self.scratch_re[even] = (even_re + rot_re) as f32;
                    self.scratch_im[even] = (even_im + rot_im) as f32;
                }
                start += span;
            }
            span <<= 1;
        }

        let half = size / 2;
        for (i, bin) in out.iter_mut().enumerate() {
            let shifted = (i + half) % size;
            let re = f64::from(self.scratch_re[shifted]);
            let im = f64::from(self.scratch_im[shifted]);
            let magnitude = re.hypot(im) / size as f64;
            *bin = (20.0 * (magnitude + 1e-8).log10()) as f32;
        }
    }
}
