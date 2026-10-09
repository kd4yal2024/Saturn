#!/usr/bin/env python3
"""Self-test for wdsp-pin-manifest.py: no network and no real WDSP needed."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("manifest", HERE / "wdsp-pin-manifest.py")
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)


class PinManifestTests(unittest.TestCase):
    def test_pins_and_symbols_come_from_the_installer(self):
        self.assertEqual(manifest.installer_value("SATURN_WDSP2_REF"), "b02d5bac675dd2f33ec2bab2b339f79a597c47dd")
        self.assertEqual(
            manifest.installer_value("SATURN_PIHPSDR_PORT_REF"), "974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1"
        )
        symbols = manifest.required_symbols()
        self.assertEqual(len(symbols), 16)
        self.assertIn("pscc", symbols)
        self.assertIn("SetRXAWBFMdmph", symbols)

    def test_compile_options_follow_the_build_helper(self):
        options = manifest.compile_options()
        self.assertIn("-O3", options["cflags"])
        self.assertEqual(options["default_target_cpu"], "cortex-a72")

    def test_a_changed_source_changes_the_tree_hash(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "a.c"
            source.write_text("int x;\n")
            before = manifest.tree_hashes(Path(root), {".c"})
            source.write_text("int y;\n")
            self.assertNotEqual(before, manifest.tree_hashes(Path(root), {".c"}))

    def test_an_archive_missing_a_required_symbol_is_reported_and_fails(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "native-src" / "OpenHPSDR-wdsp" / "wdsp 2.10" / "Source").mkdir(parents=True)
            (root / "native-src" / "pihpsdr" / "wdsp").mkdir(parents=True)
            for name in ("linux_port.c", "linux_port.h"):
                (root / "native-src" / "pihpsdr" / "wdsp" / name).write_text("/* shim */\n")
            build = root / "build"
            build.mkdir()
            (build / "only.c").write_text("int pscc(void) { return 0; }\n")
            subprocess.run(["cc", "-c", "-o", str(build / "only.o"), str(build / "only.c")], check=True)
            subprocess.run(["ar", "rcs", str(build / "libwdsp.a"), str(build / "only.o")], check=True)
            output = root / "manifest.json"
            result = subprocess.run(
                [sys.executable, str(HERE / "wdsp-pin-manifest.py"), "--native-src", str(root / "native-src"),
                 "--build-dir", str(build), "--output", str(output)],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 1)
            report = json.loads(output.read_text())
            archive = report["archive"]
            self.assertEqual(archive["members"], 1)
            self.assertNotIn("pscc", archive["missing_required_symbols"])
            self.assertIn("SetRXAWBFMdmph", archive["missing_required_symbols"])
            # Not git checkouts, so the pins cannot match either.
            self.assertFalse(report["pins"]["match"])
            self.assertIn("missing symbols", result.stderr)


if __name__ == "__main__":
    unittest.main()
