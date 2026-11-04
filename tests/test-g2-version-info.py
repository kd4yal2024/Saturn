#!/usr/bin/env python3
"""Exercise the real read-only report with runtime and release fixtures."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "update_manager/scripts/g2-version-info.sh"
BUILD = "3eb0f729238f228e02da49c7fc80089f8a1d879a"


class VersionReportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="saturn-version-report-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.repo = self.root / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        self.release = self.root / BUILD
        (self.release / "bin").mkdir(parents=True)
        self.executable = self.release / "bin/saturn-bridge"
        self.executable.write_bytes(b"fixture executable; must never be run\n")
        self.digest = hashlib.sha256(self.executable.read_bytes()).hexdigest()
        self.manifest = {"source": {"commit": BUILD}, "components": [{
            "name": "saturn-bridge", "path": "bin/saturn-bridge", "version": "0.1.0",
            "source_commit": BUILD, "sha256": self.digest,
        }]}
        self.data = {"perf": {
            "service": {"name": "saturn-bridge.service", "main_pid": 42,
                        "running_exe_sha256": self.digest},
            "workload": {"selected_app": "xdma", "startup_mode": "direct-xdma",
                         "current_target_abs": str(self.executable), "panel_mode": None},
            "app_telemetry": {"pid_matches_service": True, "current": {
                "app": "saturn-bridge", "pid": 42,
                "gauges": {"bridge": {"build_git_sha": BUILD}},
                "fpga": {"available": False},
            }},
        }}
        self.make_script(self.bin / "curl", '''while (($#)); do
 if [[ "$1" == "-o" ]]; then cp "$SATURN_VERSION_FIXTURE" "$2"; exit 0; fi
 shift
done
exit 1
''')
        self.make_script(self.bin / "systemctl", "printf 'inactive\\n'; exit 3\n")
        self.make_script(self.bin / "journalctl", "echo 'STALE SENTINEL FPGA identity'; exit 0\n")
        self.make_script(self.repo / "scripts/detect-front-panel.sh", "printf 'NONE\\n'\n")
        self.make_script(self.repo / "scripts/detect-lcd-profile.sh", "printf '%s\\n' 'module_family=cm4' 'storage_variant=unknown' 'profile=cm4-7' 'size=7' 'size_source=config'\n")

    @staticmethod
    def make_script(path, body):
        path.write_text("#!/usr/bin/env bash\n" + body)
        path.chmod(0o755)

    def report(self):
        (self.release / "release-manifest.json").write_text(json.dumps(self.manifest))
        fixture = self.root / "perf.json"
        fixture.write_text(json.dumps(self.data))
        env = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
               "SATURN_VERSION_FIXTURE": str(fixture), "SATURN_ACTIVE_REPO_ROOT": str(self.repo)}
        result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True,
                                capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("STALE SENTINEL", result.stdout)
        return result.stdout

    def test_old_bridge_uses_hash_matched_running_release_not_source_checkout(self):
        text = self.report()
        self.assertIn("App version: 0.1.0", text)
        self.assertIn("App build: " + BUILD, text)
        self.assertIn("Installed release: " + BUILD + " (executable hash verified)", text)
        self.assertIn("Panel mode: not applicable (Direct XDMA", text)
        self.assertIn("FPGA identity: not published by this runtime", text)
        self.assertIn("Storage variant: not identified", text)

    def test_new_owner_metadata_reports_identity_without_live_clock_claim(self):
        current = self.data["perf"]["app_telemetry"]["current"]
        current["version"] = "0.1.1"
        current["fpga"] = {"available": True, "product": "Saturn", "product_version": 2,
            "firmware_name": "Saturn, full function", "build_identity_status": "identified",
            "firmware_display": "1.31.002", "rx_filter": "22/Q24 saturated",
            "build_id_hex": "0x53460004", "identity_source": "acquisition-owner-session-cache"}
        text = self.report()
        self.assertIn("App version: 0.1.1", text)
        self.assertIn("FPGA firmware: 1.31.002", text)
        self.assertIn("RX filter: 22/Q24 saturated", text)
        self.assertIn("Radio PCB: G2 PCB V2", text)
        self.assertIn("Codec family: TLV320AIC23B", text)
        self.assertIn("captured at session start", text)
        self.assertNotIn("FPGA clocks: all present", text)

    def test_mismatched_manifest_hash_does_not_supply_a_version(self):
        self.manifest["components"][0]["sha256"] = "0" * 64
        text = self.report()
        self.assertIn("App version: not published by this runtime", text)
        self.assertNotIn("Installed release:", text)

    def test_running_process_hash_must_match_manifest(self):
        self.data["perf"]["service"]["running_exe_sha256"] = "0" * 64
        text = self.report()
        self.assertIn("App version: not published by this runtime", text)
        self.assertNotIn("executable hash verified", text)

    def test_pid_mismatch_cannot_supply_hardware_identity(self):
        telemetry = self.data["perf"]["app_telemetry"]
        telemetry["pid_matches_service"] = False
        telemetry["current"]["fpga"] = {"available": True, "product": "Saturn", "product_version": 3}
        text = self.report()
        self.assertIn("snapshot PID does not match", text)
        self.assertIn("Radio PCB: unknown", text)
        self.assertNotIn("Radio PCB: G2 PCB V3", text)

    def test_p2_report_preserves_runtime_version_and_panel_mode(self):
        self.data["perf"]["workload"].update(selected_app="p2", startup_mode="p2", panel_mode="headless")
        self.data["perf"]["app_telemetry"]["current"].update(app="p2app", version=52)
        text = self.report()
        self.assertIn("App version: 52", text)
        self.assertIn("Panel mode: headless", text)
        self.assertNotIn("Panel mode: not applicable", text)


if __name__ == "__main__":
    unittest.main()
