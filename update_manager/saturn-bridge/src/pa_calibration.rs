//! PA target-to-drive conversion shared by P2 and Direct XDMA.
//! Gain values are attenuations in the Thetis PA-profile convention: a larger
//! value commands less drive. They are not measured RF gain or output watts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum PaCalibration {
    #[default]
    Legacy,
    Bands([f64; 12]),
}

pub(crate) const BAND_NAMES: [&str; 12] = [
    "160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "11m", "10m", "6m",
];
#[cfg(test)]
pub(crate) const G2_STARTING_GAINS: [f64; 12] = [
    47.9, 50.5, 50.8, 50.8, 50.9, 50.9, 50.5, 47.0, 47.9, 46.5, 46.5, 44.6,
];

pub(crate) fn band_index(frequency: u32) -> Option<usize> {
    // Extended PA calibration regions, with an independent 11 m entry. Gaps
    // outside these regions retain the legacy mapping rather than borrowing
    // an unrelated band's calibration.
    const REGIONS: [(u32, u32); 12] = [
        (1_800_000, 2_750_000),
        (2_750_000, 5_250_000),
        (5_250_000, 7_000_000),
        (7_000_000, 8_700_000),
        (8_700_000, 12_075_000),
        (12_075_000, 16_209_000),
        (16_209_000, 19_584_000),
        (19_584_000, 23_170_000),
        (23_170_000, 26_495_000),
        (26_495_000, 28_000_000),
        (28_000_000, 29_700_001),
        (50_000_000, 54_000_001),
    ];
    REGIONS
        .iter()
        .position(|&(low, end)| frequency >= low && frequency < end)
}

impl PaCalibration {
    pub(crate) fn valid(self) -> bool {
        match self {
            Self::Legacy => true,
            Self::Bands(gains) => gains
                .iter()
                .all(|g| g.is_finite() && (40.0..=80.0).contains(g)),
        }
    }
    pub(crate) fn parse(text: &str) -> Option<Self> {
        if text == "legacy" {
            return Some(Self::Legacy);
        }
        let mut fields = text.split('|');
        let mut gains = [0.; 12];
        for gain in &mut gains {
            *gain = fields.next()?.parse().ok()?;
        }
        if fields.next().is_some() {
            return None;
        }
        let value = Self::Bands(gains);
        value.valid().then_some(value)
    }
    pub(crate) fn wire(self) -> String {
        match self {
            Self::Legacy => "legacy".into(),
            Self::Bands(gains) => gains
                .iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>()
                .join("|"),
        }
    }
    pub(crate) fn gain(self, frequency: u32) -> Option<f64> {
        match self {
            Self::Bands(gains) if self.valid() => Some(gains[band_index(frequency)?]),
            _ => None,
        }
    }
    // The third argument calibrates the Legacy curve's 100-W endpoint. It is
    // not a physical DAC ceiling: the per-band profile replaces that curve.
    // Both policies clamp requested watts to 100 and encoded drive to u8.
    pub(crate) fn drive(self, watts: u8, frequency: u32, legacy_drive_at_100w: u8) -> u8 {
        if watts == 0 {
            return 0;
        }
        match self.gain(frequency) {
            Some(gain) => {
                // Thetis: target_dBm - PA_gain -> volts into 50 ohms;
                // radio volume /0.8, network factor 1.02, then truncate.
                let dbm = 10.0 * (f64::from(watts.min(100)) * 1000.0).log10() - gain;
                let volume = (10.0_f64.powf(dbm * 0.1) * 0.05).sqrt() / 0.8;
                (255.0 * (volume.min(1.0) * 1.02).min(1.0))
                    .floor()
                    .clamp(1.0, 255.0) as u8
            }
            None => legacy_drive(watts, legacy_drive_at_100w),
        }
    }
}

pub(crate) fn legacy_drive(watts: u8, full_scale: u8) -> u8 {
    if watts == 0 {
        return 0;
    }
    let target = f64::from(watts.min(100));
    let curve = [
        (0.0, 0.0),
        (5.0, 18.0),
        (10.0, 24.0),
        (25.0, 34.0),
        (50.0, 46.0),
        (100.0, 68.0),
    ];
    let pair = curve.windows(2).find(|w| target <= w[1].0).unwrap();
    let drive =
        pair[0].1 + (pair[1].1 - pair[0].1) * (target - pair[0].0) / (pair[1].0 - pair[0].0);
    (drive * f64::from(full_scale.max(1)) / 68.0)
        .round()
        .clamp(1.0, f64::from(full_scale.max(1))) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_thetis_g2_profile_and_extended_eleven_metres() {
        let p = PaCalibration::Bands(G2_STARTING_GAINS);
        assert_eq!(p.drive(40, 7_200_000, 68), 41);
        assert_eq!(p.drive(10, 7_200_000, 68), 20);
        assert_eq!(p.drive(40, 27_426_000, 68), 68);
        assert_eq!(p.drive(10, 27_426_000, 68), 34);
        assert_eq!(p.drive(40, 28_400_000, 68), 68);
        let mut gains = G2_STARTING_GAINS;
        gains[9] += 6.0;
        let reduced = PaCalibration::Bands(gains);
        assert_eq!(reduced.drive(40, 27_426_000, 68), 34);
        assert_eq!(reduced.drive(40, 28_400_000, 68), 68);
    }
    #[test]
    fn strict_complete_profile_and_wire_roundtrip() {
        let p = PaCalibration::Bands(G2_STARTING_GAINS);
        assert_eq!(PaCalibration::parse(&p.wire()), Some(p));
        assert_eq!(PaCalibration::parse("legacy"), Some(PaCalibration::Legacy));
        for bad in [
            "",
            "46.5",
            "NaN|50|50|50|50|50|50|50|50|50|50|50",
            "39|50|50|50|50|50|50|50|50|50|50|50",
            "81|50|50|50|50|50|50|50|50|50|50|50",
        ] {
            assert!(PaCalibration::parse(bad).is_none());
        }
    }
    #[test]
    fn bounded_monotonic_and_legacy_fallback() {
        assert_eq!(PaCalibration::Legacy.drive(40, 27_426_000, 68), 41);
        for profile in [
            PaCalibration::Legacy,
            PaCalibration::Bands(G2_STARTING_GAINS),
            PaCalibration::Bands([40.; 12]),
            PaCalibration::Bands([80.; 12]),
        ] {
            for frequency in [0, 7_200_000, 27_385_000, 28_000_000, 40_000_000, 50_000_000] {
                assert_eq!(profile.drive(0, frequency, 68), 0);
                for watts in 1..100 {
                    assert!(
                        profile.drive(watts, frequency, 68)
                            <= profile.drive(watts + 1, frequency, 68)
                    );
                }
                assert_eq!(
                    profile.drive(255, frequency, 68),
                    profile.drive(100, frequency, 68)
                );
            }
        }
        assert_eq!(
            PaCalibration::Bands(G2_STARTING_GAINS).drive(40, 40_000_000, 68),
            41
        );
        for (hz, band) in [
            (26_494_999, 8),
            (26_495_000, 9),
            (27_999_999, 9),
            (28_000_000, 10),
            (29_700_000, 10),
            (54_000_000, 11),
        ] {
            assert_eq!(band_index(hz), Some(band));
        }
        assert_eq!(band_index(29_700_001), None);
        assert_eq!(legacy_drive(100, 120), 120);
    }
}
