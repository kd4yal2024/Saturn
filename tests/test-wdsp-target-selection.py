#!/usr/bin/env python3
"""Exercise build entry points without native dependencies or appliance writes."""
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "update_manager/saturn-bridge/scripts/build-wdsp2-linux-arm.sh"
INSTALL = ROOT / "update_manager/scripts/install-saturn-bridge.sh"


class TargetSelection(unittest.TestCase):
    def target(self, arch, cpu=None):
        env = dict(os.environ, SATURN_WDSP_TARGET_ARCH=arch)
        env.pop("SATURN_WDSP_TARGET_CPU", None)
        if cpu is not None:
            env["SATURN_WDSP_TARGET_CPU"] = cpu
        return subprocess.run(["bash", str(BUILD), "--print-target"], env=env,
                              capture_output=True, text=True, timeout=5)

    def test_defaults_and_explicit_cpu(self):
        for arch in ("aarch64", "arm64", "armv7l", "armv8l", "x86_64", "amd64"):
            with self.subTest(arch=arch):
                result = self.target(arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                flag = "-mcpu=cortex-a72" if arch.startswith("arm") or arch == "aarch64" else "-march=x86-64"
                self.assertIn(flag, result.stdout)
        self.assertIn("-march=haswell", self.target("x86_64", "haswell").stdout)

    def test_invalid_targets_reject_before_source_or_build_access(self):
        for arch, cpu in (("mips", None), ("x86_64", "bad cpu")):
            with self.subTest(arch=arch, cpu=cpu):
                result = self.target(arch, cpu)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("source directory", result.stderr)

    def test_explicit_old_version_is_never_relabelled(self):
        for version in ("2.00", "2.10"):
            with self.subTest(version=version):
                result = subprocess.run(["bash", str(INSTALL)],
                    env=dict(os.environ, SATURN_BRIDGE_WDSP_FLAVOR=version),
                    capture_output=True, text=True, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("select 2.11 or wdsp2 explicitly", result.stderr)


if __name__ == "__main__":
    unittest.main()
