#!/usr/bin/env python3
"""Record exactly which WDSP a Saturn Bridge build contains.

Writes one JSON manifest that pins the native dependency, so a build on any
machine (a developer PC, a cross build, the G2's staged build) can be compared
with another. It reads, and never changes, the sources and the archive that
scripts/build-wdsp2-linux-arm.sh produced.

  wdsp-pin-manifest.py --native-src DIR --build-dir DIR [--bridge BINARY]
                       [--cc CC] [--target-cpu CPU] [--output FILE]

  --native-src  the directory holding OpenHPSDR-wdsp/ and pihpsdr/ (the
                installer's target/native-src)
  --build-dir   the build helper's output (WDSP2_BUILD_DIR), holding libwdsp.a
                and the patched sources it was built from
  --bridge      optionally also record a built saturn-bridge binary

It records:
  * the expected commits, read from the installer (the single source of truth)
    and the commits actually checked out, and whether they match;
  * SHA-256 of every upstream source file and of every patched file the archive
    was compiled from, so a changed patch is visible;
  * the compile options used by the build helper, the compiler identity and the
    FFTW version;
  * SHA-256 of libwdsp.a and of each object in it, its machine type, and
    whether every symbol the installer requires is defined.

Exit status is 1 if a pin does not match or a required symbol is missing.
Hashes of the archive and objects are only comparable between builds made with
the same compiler and flags; the source hashes are comparable always. A
compiler that emits debug information also embeds the build directory, so with
such a compiler the objects only match if the build path matches too. (Checked
2026-10-09: GCC without -g gave bit-identical objects and archive from two
different directories; a Zig/clang cross build did not.)
"""
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
INSTALLER = HERE.parent.parent / "scripts" / "install-saturn-bridge.sh"
BUILD_HELPER = HERE / "build-wdsp2-linux-arm.sh"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command, **kwargs):
    return subprocess.run(command, check=False, capture_output=True, text=True, **kwargs)


def installer_value(name: str) -> str:
    text = INSTALLER.read_text()
    match = re.search(rf'^{name}="\${{{name}:-([^}}]+)}}"', text, re.MULTILINE)
    if not match:
        raise SystemExit(f"cannot find {name} in {INSTALLER}")
    return match.group(1)


def required_symbols() -> list[str]:
    text = INSTALLER.read_text()
    block = re.search(r"verify_wdsp2_archive\(\).*?for symbol in(.*?)\n\s*do\n", text, re.DOTALL)
    if not block:
        raise SystemExit("cannot find the required-symbol list in the installer")
    return block.group(1).replace("\\", " ").split()


def git_head(path: Path):
    result = run(["git", "-C", str(path), "rev-parse", "HEAD"])
    return result.stdout.strip() if result.returncode == 0 else None


def tree_hashes(root: Path, suffixes) -> dict:
    return {
        str(file.relative_to(root)): sha256(file)
        for file in sorted(root.rglob("*"))
        if file.is_file() and file.suffix in suffixes and ".git" not in file.parts
    }


def compile_options() -> dict:
    """The flags the build helper uses, read from the helper itself."""
    text = BUILD_HELPER.read_text()
    flags = re.search(r"cflags=\((.*?)\)\n", text)
    return {
        "source": "scripts/build-wdsp2-linux-arm.sh",
        "cflags": flags.group(1).split() if flags else None,
        "nnr_model_sources_use": "-O1 instead of -O3 (pure data)",
        "default_target_cpu": re.search(r'TARGET_CPU="\$\{SATURN_WDSP_TARGET_CPU:-([^}]+)}"', text).group(1),
        "helper_sha256": sha256(BUILD_HELPER),
    }


def archive_report(archive: Path) -> dict:
    members = run(["ar", "t", str(archive)]).stdout.split()
    object_hashes = {}
    with tempfile.TemporaryDirectory() as scratch:
        run(["ar", "x", str(archive.resolve())], cwd=scratch)
        for member in sorted(members):
            object_hashes[member] = sha256(Path(scratch) / member)
        first = Path(scratch) / sorted(members)[0]
        header = run(["readelf", "-h", str(first)]).stdout
    machine = re.search(r"Machine:\s+(.+)", header)
    elf_class = re.search(r"Class:\s+(.+)", header)
    symbols = run(["nm", "-g", "--defined-only", str(archive)]).stdout
    defined = set(re.findall(r"\s(\S+)$", symbols, re.MULTILINE))
    missing = [name for name in required_symbols() if name not in defined]
    return {
        "path": str(archive),
        "sha256": sha256(archive),
        "bytes": archive.stat().st_size,
        "members": len(members),
        "machine": machine.group(1).strip() if machine else None,
        "elf_class": elf_class.group(1).strip() if elf_class else None,
        "object_sha256": object_hashes,
        "required_symbols": required_symbols(),
        "missing_required_symbols": missing,
    }


def bridge_report(binary: Path) -> dict:
    header = run(["readelf", "-h", str(binary)]).stdout
    machine = re.search(r"Machine:\s+(.+)", header)
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[(.+)\]", run(["readelf", "-d", str(binary)]).stdout)
    strings = run(["strings", "-a", str(binary)]).stdout
    return {
        "path": str(binary),
        "sha256": sha256(binary),
        "bytes": binary.stat().st_size,
        "machine": machine.group(1).strip() if machine else None,
        "dynamic_needed": needed,
        "embeds_wdsp_flavor_2_10": "wdsp2-2.10" in strings,
        "embeds_wdsp_commit": "b02d5bac675dd2f33ec2bab2b339f79a597c47dd" in strings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--native-src", type=Path, required=True)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--bridge", type=Path)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--target-cpu", default=None)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    wdsp_repo = args.native_src / "OpenHPSDR-wdsp"
    port_repo = args.native_src / "pihpsdr"
    source = wdsp_repo / "wdsp 2.10" / "Source"
    expected = {
        "wdsp": installer_value("SATURN_WDSP2_REF"),
        "pihpsdr_linux_port": installer_value("SATURN_PIHPSDR_PORT_REF"),
    }
    actual = {"wdsp": git_head(wdsp_repo), "pihpsdr_linux_port": git_head(port_repo)}
    pins_match = all(actual[key] == expected[key] for key in expected)

    cc_version = run([args.cc, "--version"]).stdout.splitlines()
    fftw = shutil.which("pkg-config") and run(["pkg-config", "--modversion", "fftw3"]).stdout.strip()
    archive = archive_report(args.build_dir / "libwdsp.a")
    manifest = {
        "schema": "saturn-wdsp-pin-manifest-v1",
        "pins": {
            "installer": str(INSTALLER.relative_to(HERE.parent.parent.parent)),
            "repositories": {
                "wdsp": "https://github.com/TAPR/OpenHPSDR-wdsp.git",
                "pihpsdr_linux_port": "https://github.com/dl1ycf/pihpsdr.git",
            },
            "expected": expected,
            "checked_out": actual,
            "match": pins_match,
        },
        "upstream_sources": {
            "wdsp_2_10_source": tree_hashes(source, {".c", ".h"}),
            "linux_port": {
                name: sha256(port_repo / "wdsp" / name) for name in ("linux_port.c", "linux_port.h")
            },
        },
        "patched_build_tree": tree_hashes(args.build_dir, {".c", ".h"}),
        "build": {
            "compile_options": compile_options(),
            "target_cpu": args.target_cpu,
            "compiler": cc_version[0] if cc_version else None,
            "compiler_machine": run([args.cc, "-dumpmachine"]).stdout.strip() or None,
            "fftw_version": fftw or None,
        },
        "archive": archive,
        "bridge": bridge_report(args.bridge) if args.bridge else None,
    }
    text = json.dumps(manifest, indent=1, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(text)
    else:
        sys.stdout.write(text)
    problems = []
    if not pins_match:
        problems.append(f"pins differ: expected {expected}, checked out {actual}")
    if archive["missing_required_symbols"]:
        problems.append(f"missing symbols: {archive['missing_required_symbols']}")
    for problem in problems:
        print(f"wdsp-pin-manifest: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
