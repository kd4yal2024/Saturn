import copy
import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "rx-soak.py"
SPEC = importlib.util.spec_from_file_location("saturn_rx_soak", SCRIPT)
RX_SOAK = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(RX_SOAK)


def make_perf(adc_reports=0, adc_bits=0):
    routing = {
        "ddc": [
            {
                "id": 2,
                "enabled": True,
                "sample_rate_khz": 384,
                "interleaved": False,
            }
        ],
        "wideband": {"adc1_enabled": False, "adc2_enabled": False},
    }
    app = {
        "app": "p2",
        "version": 51,
        "pid": 123,
        "state": {
            "tx_mode": False,
            "pure_signal_enabled": False,
            "exit_requested": False,
            "thread_error": False,
        },
        "fpga": {
            "firmware_version": 29,
            "date_code_hex": "09122026",
            "fallback_config": False,
            "all_clocks_present": True,
        },
        "routing": routing,
        "counters": {
            name: 0
            for name in RX_SOAK.BASE_LOSS_COUNTERS
            + RX_SOAK.CONTROLLED_ONLY_LOSS_COUNTERS
        },
        "gauges": {
            "adc": {"overflow_bits": adc_bits, "peak1": 32767, "peak2": 140},
            "fpga_fifo_v29": {
                "available": True,
                "status": "available",
                "build_id": RX_SOAK.EXPECTED_BUILD_ID,
                "snapshot_valid": True,
                "snapshot_timeout_count": 0,
                "snapshot_generation": 10,
            },
        },
    }
    app["counters"]["adc_overflow_events"] = adc_reports
    return {
        "app_telemetry": {"snapshot_readable": True, "current": app},
        "service": {"main_pid": 123},
        "workload": {
            "selected_app": "p2",
            "panel_mode": "off",
            "current_target": "/opt/saturn-go/p23-apps/p2app",
            "startup_mode": "headless",
            "workload_key": "p2|mode=headless|panel=off",
        },
        "xdma": {
            "present": True,
            "pcie": {"current_link_speed": "5.0 GT/s PCIe", "current_link_width": "1"},
        },
        "network": {},
    }


class RXSoakProfileTests(unittest.TestCase):
    def test_controlled_profile_gates_adc_reports_and_live_bits(self):
        baseline = make_perf()
        sample = make_perf(adc_reports=6, adc_bits=1)

        problems = RX_SOAK.gate_violations(
            sample, baseline, None, "controlled"
        )

        self.assertIn("ADC overflow bits became nonzero", problems)
        self.assertIn("adc_overflow_events changed by 6", problems)

    def test_antenna_profile_records_but_does_not_gate_adc(self):
        baseline = make_perf()
        sample = make_perf(adc_reports=6, adc_bits=1)

        problems = RX_SOAK.gate_violations(
            sample, baseline, None, "antenna"
        )

        self.assertEqual([], problems)

    def test_antenna_profile_requires_adc_observation_fields(self):
        sample = make_perf()
        del sample["app_telemetry"]["current"]["counters"]["adc_overflow_events"]

        problems = RX_SOAK.gate_violations(
            sample, None, None, "antenna"
        )

        self.assertIn("required ADC report counter is missing", problems)

    def test_antenna_profile_rejects_adc_counter_rollback(self):
        baseline = make_perf(adc_reports=10)
        sample = make_perf(adc_reports=2)

        problems = RX_SOAK.gate_violations(
            sample, baseline, None, "antenna"
        )

        self.assertIn("adc_overflow_events moved backward by -8", problems)

    def test_antenna_profile_keeps_speaker_loss_as_hard_gate(self):
        baseline = make_perf()
        sample = copy.deepcopy(baseline)
        sample["app_telemetry"]["current"]["counters"]["fifo_speaker_under_events"] = 1

        problems = RX_SOAK.gate_violations(
            sample, baseline, None, "antenna"
        )

        self.assertIn("fifo_speaker_under_events changed by 1", problems)

    def test_explicit_dual_receiver_workload_is_accepted_and_frozen(self):
        baseline = make_perf()
        baseline["app_telemetry"]["current"]["routing"]["ddc"].append({
            "id": 3,
            "enabled": True,
            "sample_rate_khz": 384,
            "interleaved": False,
        })
        sample = copy.deepcopy(baseline)

        problems = RX_SOAK.gate_violations(
            sample, baseline, None, "antenna",
            ((2, 384, False), (3, 384, False)),
        )

        self.assertEqual([], problems)

    def test_default_workload_rejects_unexpected_second_receiver(self):
        sample = make_perf()
        sample["app_telemetry"]["current"]["routing"]["ddc"].append({
            "id": 3,
            "enabled": True,
            "sample_rate_khz": 384,
            "interleaved": False,
        })

        problems = RX_SOAK.gate_violations(
            sample, None, None, "controlled"
        )

        self.assertTrue(any("RX routing/workload" in problem for problem in problems))

    def test_existing_output_is_rejected_before_modification(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "collection_status.json"
            marker.write_text("original\n", encoding="utf-8")

            with self.assertRaisesRegex(SystemExit, "output directory must be empty"):
                RX_SOAK.main([
                    "--profile", "antenna",
                    "--output", directory,
                    "--duration", "0.01",
                    "--interval", "5",
                ])

            self.assertEqual("original\n", marker.read_text(encoding="utf-8"))

    def test_sub_five_second_interval_is_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaisesRegex(SystemExit, "2"):
                RX_SOAK.parse_args([
                    "--output", "/tmp/not-used",
                    "--interval", "1",
                ])


if __name__ == "__main__":
    unittest.main()
