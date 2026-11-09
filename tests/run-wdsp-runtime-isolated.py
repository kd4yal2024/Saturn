#!/usr/bin/env python3
"""Run real-WDSP loopback/file runtime fixtures in separate native processes.

Build the managed Bridge test binary with tests/support/puresignal-native-observer.c
linked first. Pass that test executable, not the installed Bridge. No hardware
fixtures or browser processes are started by this runner.
"""
import argparse
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    listing = subprocess.check_output([binary, "--list"], text=True, timeout=30)
    names = [line.removesuffix(": test") for line in listing.splitlines()
             if line.endswith(": test") and "actual_run_inner_" in line
             and not any(word in line for word in
                         ("browser", "45_second", "measurement_client"))]
    if not names:
        parser.error("no actual-runtime tests found; supply the managed native test executable")
    with args.log.open("w") as log:
        for name in names:
            print("RUN", name, flush=True)
            log.write("\nFRESH PROCESS " + name + "\n")
            log.flush()
            subprocess.run([binary, name, "--exact", "--ignored", "--nocapture",
                            "--test-threads=1"], stdout=log,
                           stderr=subprocess.STDOUT, check=True, timeout=150)
    print(f"Passed {len(names)} isolated runtime cases; physical RF untested")


if __name__ == "__main__":
    main()
