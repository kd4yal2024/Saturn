//! Read-only complex two-tone measurements. No RF, WDSP or control operations.
//! Input is ONE contiguous raw RF-feedback window, not display-smoothed IQ.
use crate::diagnostic_fft::SpectrumFft;

pub const FFT_SIZE: usize = 16_384;
pub const RATE: u32 = 192_000;
pub const PRODUCT_ORDERS: [usize; 3] = [3, 5, 7];
const HALF_BAND: isize = 5;
const FLOOR_MARGIN_DB: f64 = 10.;
fn window(i: usize, n: usize) -> f64 {
    let p = std::f64::consts::TAU * i as f64 / (n - 1) as f64;
    0.35875 - 0.48829 * p.cos() + 0.14128 * (2. * p).cos() - 0.01168 * (3. * p).cos()
}
#[derive(Clone, Debug)]
pub struct Product {
    pub order: usize,
    pub offset_hz: f64,
    /// Relative to the WEAKER fundamental, not PEP. None is not zero distortion.
    pub dbc: Option<f64>,
    pub upper_bound_dbc: Option<f64>,
    pub reason: &'static str,
}
#[derive(Clone, Debug)]
pub struct Analysis {
    pub reason: &'static str,
    pub tone_hz: [f64; 2],
    pub tone_dbfs: [f64; 2],
    pub tone_balance_db: f64,
    pub component_peak: f64,
    pub clipped_components: usize,
    pub rbw_hz: f64,
    pub bin_hz: f64,
    pub bins_dbfs: Vec<f32>,
    pub products: Vec<Product>,
}
pub struct Analyzer {
    fft: SpectrumFft,
    db: Vec<f32>,
    powers: Vec<f64>,
    coherent_gain: f64,
    enbw_bins: f64,
}
impl Default for Analyzer {
    fn default() -> Self {
        let weights: Vec<_> = (0..FFT_SIZE).map(|i| window(i, FFT_SIZE)).collect();
        let sum = weights.iter().sum::<f64>();
        Self {
            fft: SpectrumFft::with_window(FFT_SIZE, window),
            db: vec![0.; FFT_SIZE],
            powers: vec![0.; FFT_SIZE],
            coherent_gain: sum / FFT_SIZE as f64,
            enbw_bins: FFT_SIZE as f64 * weights.iter().map(|w| w * w).sum::<f64>() / (sum * sum),
        }
    }
}
impl Analyzer {
    fn bin(hz: f64) -> isize {
        (hz * FFT_SIZE as f64 / f64::from(RATE)).round() as isize + FFT_SIZE as isize / 2
    }
    fn band(&self, hz: f64) -> Option<f64> {
        let k = Self::bin(hz);
        if k - HALF_BAND < 0 || k + HALF_BAND >= FFT_SIZE as isize {
            return None;
        }
        Some(
            (-HALF_BAND..=HALF_BAND)
                .map(|d| self.powers[(k + d) as usize])
                .sum::<f64>()
                / self.enbw_bins,
        )
    }
    fn floor(&self, hz: f64, excluded: &[f64]) -> Option<f64> {
        let k = Self::bin(hz);
        let mut noise: Vec<_> = (-80..=80)
            .filter(|d: &isize| d.abs() > 10)
            .map(|d| k + d)
            .filter(|i| *i >= 0 && *i < FFT_SIZE as isize)
            .filter(|i| excluded.iter().all(|f| (*i - Self::bin(*f)).abs() > 10))
            .map(|i| self.powers[i as usize])
            .collect();
        if noise.len() < 24 {
            return None;
        }
        noise.sort_by(f64::total_cmp);
        // Median / ln(2) estimates mean exponential-bin power. It is a noise
        // estimate, not a calibrated uncertainty bound or spur rejection test.
        Some(
            noise[noise.len() / 2] / std::f64::consts::LN_2 * (2 * HALF_BAND + 1) as f64
                / self.enbw_bins,
        )
    }
    pub fn analyze(&mut self, iq: &[f32], tones: [f64; 2]) -> Result<Analysis, &'static str> {
        if iq.len() != FFT_SIZE * 2 || iq.iter().any(|v| !v.is_finite()) {
            return Err("invalid or incomplete complex window");
        }
        let bin_hz = f64::from(RATE) / FFT_SIZE as f64;
        if tones
            .iter()
            .any(|v| !v.is_finite() || *v < 100. || *v > 20_000.)
            || (tones[1] - tones[0]).abs() < bin_hz * 24.
        {
            return Err("fundamentals unresolved or unsupported");
        }
        self.fft.transform_db(iq, &mut self.db);
        if self.db.iter().any(|v| !v.is_finite()) {
            return Err("FFT overflow or invalid spectrum");
        }
        let gain_db = 20. * self.coherent_gain.log10();
        for (p, db) in self.powers.iter_mut().zip(&mut self.db) {
            *db -= gain_db as f32;
            *p = 10f64.powf(f64::from(*db) / 10.);
        }
        let pos: f64 = tones.iter().map(|f| self.band(*f).unwrap_or(0.)).sum();
        let neg: f64 = tones.iter().map(|f| self.band(-*f).unwrap_or(0.)).sum();
        let sign = if neg > pos { -1. } else { 1. };
        let mut tone_hz = tones.map(|f| sign * f);
        tone_hz.sort_by(f64::total_cmp);
        let tone_power = tone_hz.map(|f| self.band(f).unwrap_or(0.));
        let db = |p: f64| 10. * p.max(1e-30).log10();
        let tone_dbfs = tone_power.map(db);
        let mut offsets = tone_hz.to_vec();
        let spacing = tone_hz[1] - tone_hz[0];
        for order in PRODUCT_ORDERS {
            let n = ((order - 1) / 2) as f64;
            offsets.extend([tone_hz[0] - n * spacing, tone_hz[1] + n * spacing]);
        }
        let component_peak = iq.iter().fold(0f64, |p, x| p.max(f64::from(x.abs())));
        let clipped_components = iq.iter().filter(|v| v.abs() >= 0.999_999).count();
        let tone_good = tone_hz.iter().zip(tone_power).all(|(f, p)| {
            self.floor(*f, &offsets)
                .is_some_and(|noise| db(p / noise.max(1e-30)) >= 20.)
                && p > 1e-12
        });
        let reason = if clipped_components != 0 {
            "sample clipping"
        } else if !tone_good {
            "missing or noise-limited fundamental"
        } else if db(pos.max(neg) / pos.min(neg).max(1e-30)) < 20. {
            "ambiguous sideband"
        } else {
            "relative estimate; RF path and overload coverage require verification"
        };
        let usable = clipped_components == 0 && tone_good && reason != "ambiguous sideband";
        let reference = tone_power[0].min(tone_power[1]);
        let products = offsets[2..]
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                let mut product = Product {
                    order: PRODUCT_ORDERS[i / 2],
                    offset_hz: f,
                    dbc: None,
                    upper_bound_dbc: None,
                    reason,
                };
                if !usable {
                    return product;
                }
                let Some(p) = self.band(f) else {
                    product.reason = "out of band";
                    return product;
                };
                let Some(noise) = self.floor(f, &offsets) else {
                    product.reason = "noise estimate unavailable";
                    return product;
                };
                if db(p / noise.max(1e-30)) < FLOOR_MARGIN_DB {
                    product.upper_bound_dbc =
                        Some(db(noise.max(1e-30) / reference) + FLOOR_MARGIN_DB);
                    product.reason = "below local measurement floor (estimated upper limit)";
                } else {
                    product.dbc = Some(db((p - noise).max(1e-30) / reference));
                    product.reason = "relative estimate";
                }
                product
            })
            .collect();
        Ok(Analysis {
            reason,
            tone_hz,
            tone_dbfs,
            tone_balance_db: (tone_dbfs[0] - tone_dbfs[1]).abs(),
            component_peak,
            clipped_components,
            rbw_hz: bin_hz * self.enbw_bins,
            bin_hz,
            bins_dbfs: self.db.clone(),
            products,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(sign: f64, product: Option<(f64, f64)>) -> Vec<f32> {
        (0..FFT_SIZE)
            .flat_map(|i| {
                let mut value = [0.; 2];
                for (f, a) in [(700., 0.15), (1900., 0.10)].into_iter().chain(product) {
                    let phase = std::f64::consts::TAU * sign * f * i as f64 / RATE as f64;
                    value[0] += a * phase.cos();
                    value[1] += a * phase.sin();
                }
                value.map(|v| v as f32)
            })
            .collect()
    }
    #[test]
    fn known_imd_uses_weaker_tone_and_mirrors_both_sidebands() {
        for sign in [1., -1.] {
            let mut analyzer = Analyzer::default();
            let result = analyzer
                .analyze(
                    &signal(sign, Some((-500., 0.1 * 10f64.powf(-40. / 20.)))),
                    [700., 1900.],
                )
                .unwrap();
            let imd = result
                .products
                .iter()
                .find(|p| (p.offset_hz - sign * -500.).abs() < 1.)
                .unwrap();
            assert!((imd.dbc.unwrap() + 40.).abs() < 0.3, "{imd:?}");
            assert!((result.rbw_hz - 23.5).abs() < 0.2);
            assert!((result.tone_balance_db - 3.52).abs() < 0.1);
        }
    }
    #[test]
    fn low_products_are_limits_not_fake_precision_and_clipped_or_missing_is_unavailable() {
        let mut a = Analyzer::default();
        let r = a.analyze(&signal(1., None), [700., 1900.]).unwrap();
        assert!(r
            .products
            .iter()
            .all(|p| p.dbc.is_none() && p.upper_bound_dbc.is_some()));
        let mut clipped = signal(1., None);
        clipped[0] = 1.;
        assert!(a
            .analyze(&clipped, [700., 1900.])
            .unwrap()
            .products
            .iter()
            .all(|p| p.dbc.is_none()));
        assert!(a
            .analyze(&vec![0.; FFT_SIZE * 2], [700., 1900.])
            .unwrap()
            .products
            .iter()
            .all(|p| p.dbc.is_none()));
        clipped[0] = f32::NAN;
        assert!(a.analyze(&clipped, [700., 1900.]).is_err());
        assert!(a.analyze(&signal(1., None)[..2000], [700., 1900.]).is_err());
    }
    #[test]
    fn all_six_products_recover_known_relative_levels_including_minus_65_db() {
        for sign in [1., -1.] {
            for (i, f) in [-500., 3100., -1700., 4300., -2900., 5500.]
                .into_iter()
                .enumerate()
            {
                let expected = [-30., -35., -45., -50., -60., -65.][i];
                let mut a = Analyzer::default();
                let r = a
                    .analyze(
                        &signal(sign, Some((f, 0.1 * 10f64.powf(expected / 20.)))),
                        [700., 1900.],
                    )
                    .unwrap();
                let p = r
                    .products
                    .iter()
                    .find(|p| (p.offset_hz - sign * f).abs() < 0.1)
                    .unwrap();
                assert!(
                    (p.dbc.expect("known product resolved") - expected).abs() < 0.5,
                    "{p:?}"
                );
            }
        }
    }
}
