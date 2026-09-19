#!/usr/bin/env python3
"""Guard/CLI regressions; real pinned numerical tests live in benchmark-hbres."""
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
HELPER = ROOT / "update_manager/saturn-bridge/scripts/wdsp_hbres_index.py"
BUILD = HELPER.with_name("build-wdsp2-linux-arm.sh")
spec = importlib.util.spec_from_file_location("wdsp_hbres_index", HELPER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class IndexGuards(unittest.TestCase):
    def test_unknown_source_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unexpected WDSP"):
            module.candidate("unrecognized source")

    def test_exact_transformation_and_crlf(self):
        # Synthetic fixture verifies the transformation mechanics, not the pin.
        source = "\n".join(module.REPLACEMENTS) + "\n"
        expected = "\n".join(module.REPLACEMENTS.values()) + "\n"
        with patch.object(module, "SOURCE_SHA256", module.digest(source)), \
             patch.object(module, "OPTIMIZED_SHA256", module.digest(expected)):
            self.assertEqual(module.candidate(source), expected)
            self.assertEqual(module.candidate(source.replace("\n", "\r\n")), expected)
            with self.assertRaises(ValueError):
                module.candidate(source + "/* drift */")
            with self.assertRaises(ValueError):
                module.candidate(expected)  # Double application must fail.

    def test_missing_expression_rejected_even_with_matching_input_digest(self):
        source = "\n".join(list(module.REPLACEMENTS)[1:])
        with patch.object(module, "SOURCE_SHA256", module.digest(source)):
            with self.assertRaisesRegex(ValueError, "expression changed"):
                module.candidate(source)

    def test_output_guard_rejects_changed_transform(self):
        source = "\n".join(module.REPLACEMENTS)
        with patch.object(module, "SOURCE_SHA256", module.digest(source)):
            with self.assertRaisesRegex(ValueError, "optimized.*digest"):
                module.candidate(source)

    def test_cli_failure_does_not_overwrite_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "reshb.c"
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
            (source / "reshb.c").write_text("unexpected source")
            for name in ["linux_port.c", "linux_port.h"]:
                (port / name).touch()
            archive = build / "libwdsp.a"
            archive.write_bytes(b"existing archive")
            pkg = bins / "pkg-config"
            pkg.write_text("#!/bin/sh\nexit 0\n")
            pkg.chmod(0o755)
            result = subprocess.run(["bash", str(BUILD)], capture_output=True, env={
                **os.environ, "PATH": f"{bins}:{os.environ['PATH']}",
                "WDSP2_SOURCE_DIR": str(source), "PIHPSDR_WDSP_DIR": str(port),
                "WDSP2_BUILD_DIR": str(build),
            })
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"Unexpected WDSP", result.stderr)
            self.assertEqual(archive.read_bytes(), b"existing archive")


if __name__ == "__main__":
    unittest.main()
