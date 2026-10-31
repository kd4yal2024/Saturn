#!/usr/bin/env python3
"""Hash the tracked Bridge/browser source closure, independently of build output."""
import hashlib
import subprocess
import sys
from pathlib import Path


def source_pin(root):
    root = Path(root).resolve()
    scopes = ["update_manager/saturn-bridge", "update_manager/remote-web",
              "update_manager/templates/saturn-remote-next.html",
              "update_manager/docs/puresignal/native-abi.h"]
    paths = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z", "--", *scopes]).split(b"\0")
    records = []
    for raw in sorted(p for p in paths if p):
        name = raw.decode()
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("Source closure must contain regular files: " + name)
        records.append(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + name + "\n")
    if not records:
        raise RuntimeError("Empty source closure")
    return hashlib.sha256("".join(records).encode()).hexdigest()


if __name__ == "__main__":
    print(source_pin(sys.argv[1]))
