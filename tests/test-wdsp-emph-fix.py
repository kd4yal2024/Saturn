#!/usr/bin/env python3
"""Guards for the pinned WDSP 2.10 FM pre-emphasis resize fix."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "update_manager/saturn-bridge/scripts/wdsp_emph_fix.py"
BUILD = HELPER.with_name("build-wdsp2-linux-arm.sh")
spec = importlib.util.spec_from_file_location("wdsp_emph_fix", HELPER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class EmphasisFixGuards(unittest.TestCase):
    def test_unknown_source_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unexpected WDSP"):
            module.candidate("unrecognized source")

    def test_exact_transformation_and_crlf(self):
        source = "before\n" + module.OLD + "after\n"
        expected = "before\n" + module.NEW + "after\n"
        with patch.object(module, "SOURCE_SHA256", module.digest(source)), \
             patch.object(module, "PATCHED_SHA256", module.digest(expected)):
            self.assertEqual(module.candidate(source), expected)
            self.assertEqual(module.candidate(source.replace("\n", "\r\n")), expected)
            with self.assertRaises(ValueError):
                module.candidate(source + "/* drift */")
            with self.assertRaises(ValueError):
                module.candidate(expected)

    def test_missing_expression_rejected_even_with_matching_input_digest(self):
        source = "no filter resize here\n"
        with patch.object(module, "SOURCE_SHA256", module.digest(source)):
            with self.assertRaisesRegex(ValueError, "resize changed"):
                module.candidate(source)

    def test_output_guard_rejects_changed_transform(self):
        source = module.OLD
        with patch.object(module, "SOURCE_SHA256", module.digest(source)):
            with self.assertRaisesRegex(ValueError, "patched.*digest"):
                module.candidate(source)

    def test_cli_failure_does_not_overwrite_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "emph.c"
            source.write_bytes(b"not pinned\r\n")
            for args in [[], ["--check"]]:
                result = subprocess.run([sys.executable, str(HELPER), *args, str(source)],
                                        capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(source.read_bytes(), b"not pinned\r\n")

    def test_build_rejects_drift_before_removing_existing_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, port, build, bins = [root / name for name in ["src", "port", "build", "bin"]]
            for directory in [source, port, build, bins]:
                directory.mkdir()
            # Bypass the earlier resampler pin only to exercise the emph guard.
            (source / "reshb.c").write_text("resampler fixture\n")
            (source / "emph.c").write_text("unexpected source\n")
            for name in ["linux_port.c", "linux_port.h"]:
                (port / name).touch()
            archive = build / "libwdsp.a"
            archive.write_bytes(b"existing archive")
            pkg = bins / "pkg-config"
            pkg.write_text("#!/bin/sh\nexit 0\n")
            pkg.chmod(0o755)
            python = bins / "python3"
            python.write_text("#!/bin/sh\ncase \"$1\" in\n"
                              "*wdsp_hbres_index.py) exit 0;;\nesac\n"
                              f'exec "{sys.executable}" "$@"\n')
            python.chmod(0o755)
            result = subprocess.run(["bash", str(BUILD)], capture_output=True, env={
                **os.environ, "PATH": f"{bins}:{os.environ['PATH']}",
                "WDSP2_SOURCE_DIR": str(source), "PIHPSDR_WDSP_DIR": str(port),
                "WDSP2_BUILD_DIR": str(build),
            })
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"Unexpected WDSP emph.c", result.stderr)
            self.assertEqual(archive.read_bytes(), b"existing archive")


if __name__ == "__main__":
    unittest.main()
