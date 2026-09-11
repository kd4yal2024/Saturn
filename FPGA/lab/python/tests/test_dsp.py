from __future__ import annotations

import sys
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from sfdr import measure_tone  # noqa: E402


class DspMeasurementsTest(unittest.TestCase):
    def test_complex_tone_frequency_and_level(self) -> None:
        sample_rate = 48_000.0
        count = 4096
        bin_number = 128
        amplitude = 0.5
        index = np.arange(count)
        tone = amplitude * np.exp(2j * np.pi * bin_number * index / count)

        result = measure_tone(tone, sample_rate, 1.0)

        self.assertAlmostEqual(result["fundamental_hz"], bin_number * sample_rate / count)
        self.assertAlmostEqual(result["fundamental_dbfs"], -6.0206, places=2)
        self.assertGreater(result["sfdr_dbc"], 100.0)

    def test_real_tone_frequency_and_level(self) -> None:
        sample_rate = 122_880_000.0
        count = 8192
        bin_number = 512
        amplitude = 0.25
        index = np.arange(count)
        tone = amplitude * np.sin(2 * np.pi * bin_number * index / count)

        result = measure_tone(tone, sample_rate, 1.0)

        self.assertAlmostEqual(result["fundamental_hz"], bin_number * sample_rate / count)
        self.assertAlmostEqual(result["fundamental_dbfs"], -12.0412, places=2)
        self.assertGreater(result["sfdr_dbc"], 100.0)


if __name__ == "__main__":
    unittest.main()
