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


def static_library(root: Path, name: str, version_string: str) -> Path:
    """A static archive that embeds FFTW's version string, as libfftw3.a does."""
    source = root / f"{name}.c"
    source.write_text(f'const char fftw_version[] = "{version_string}";\n')
    subprocess.run(["cc", "-c", "-o", str(root / f"{name}.o"), str(source)], check=True)
    library = root / f"lib{name}.a"
    subprocess.run(["ar", "rcs", str(library), str(root / f"{name}.o")], check=True)
    return library


class FftwIdentityTests(unittest.TestCase):
    def test_the_version_is_read_from_the_linked_library_not_the_host(self):
        with tempfile.TemporaryDirectory() as root:
            library = static_library(Path(root), "fftw3", "fftw-3.3.10")
            report = manifest.fftw_report([library], "3.3.8")
            self.assertEqual(report["version"], "3.3.10")
            self.assertIn("embedded in the supplied FFTW library", report["version_source"])
            self.assertEqual(report["host_pkg_config_version"], "3.3.8")
            self.assertTrue(report["differs_from_host"])
            self.assertFalse(report["conflict"])
            self.assertEqual(len(report["libraries"][0]["sha256"]), 64)

    def test_a_simd_suffix_is_not_part_of_the_version(self):
        with tempfile.TemporaryDirectory() as root:
            library = static_library(Path(root), "fftw3", "fftw-3.3.8-sse2-avx")
            self.assertEqual(manifest.fftw_report([library], None)["version"], "3.3.8")

    def test_without_a_library_the_host_answer_is_labelled_as_the_host(self):
        report = manifest.fftw_report([], "3.3.8")
        self.assertEqual(report["version"], "3.3.8")
        self.assertIn("the host's FFTW", report["version_source"])
        self.assertIn("--fftw-lib", report["version_source"])
        unknown = manifest.fftw_report([], None)
        self.assertIsNone(unknown["version"])
        self.assertEqual(unknown["version_source"], "unknown")

    def test_libraries_that_disagree_are_a_conflict_with_no_single_version(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            first = static_library(root, "fftw3", "fftw-3.3.10")
            second = static_library(root, "fftw3f", "fftw-3.3.8")
            report = manifest.fftw_report([first, second], "3.3.10")
            self.assertTrue(report["conflict"])
            self.assertIsNone(report["version"])

    def test_a_path_that_merely_contains_a_version_is_not_a_version(self):
        with tempfile.TemporaryDirectory() as root:
            library = static_library(Path(root), "fftw3", "/build/fftw-9.9.9")
            report = manifest.fftw_report([library], "3.3.8")
            self.assertEqual(report["libraries"][0]["embedded_versions"], [])
            self.assertIn("embed no readable version", report["version_source"])


class BridgeBindingTests(unittest.TestCase):
    def test_the_bridge_is_checked_against_the_expected_commit_and_is_not_bound_to_the_archive(self):
        expected = manifest.installer_value("SATURN_WDSP2_REF")
        with tempfile.TemporaryDirectory() as root:
            good = Path(root) / "good"
            good.write_bytes(b"\x7fELF....wdsp2-2.10\x00" + expected.encode() + b"\x00")
            bad = Path(root) / "bad"
            bad.write_bytes(b"\x7fELF....wdsp2-2.10\x00" + b"0" * 40 + b"\x00")
            self.assertTrue(manifest.bridge_report(good, expected)["embeds_wdsp_commit"])
            self.assertFalse(manifest.bridge_report(bad, expected)["embeds_wdsp_commit"])
            self.assertFalse(manifest.bridge_report(good, expected)["bound_to_archive"])


class ManifestScopeTests(unittest.TestCase):
    def test_a_manifest_states_its_limits_and_where_each_field_came_from(self):
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
            library = static_library(root, "fftw3", "fftw-3.3.10")
            output = root / "manifest.json"
            subprocess.run(
                [sys.executable, str(HERE / "wdsp-pin-manifest.py"), "--native-src", str(root / "native-src"),
                 "--build-dir", str(build), "--fftw-lib", str(library), "--output", str(output)],
                capture_output=True, text=True,
            )
            report = json.loads(output.read_text())
            self.assertEqual(report["schema"], "saturn-wdsp-pin-manifest-v2")
            self.assertEqual(report["build"]["fftw_version"], "3.3.10")
            self.assertIn("embedded", report["build"]["fftw_version_source"])
            self.assertEqual(report["build"]["fftw"]["libraries"][0]["embedded_versions"], ["3.3.10"])
            self.assertTrue(any("already-installed" in line for line in report["limits"]))
            self.assertIn("build.fftw", report["field_sources"])


if __name__ == "__main__":
    unittest.main()
