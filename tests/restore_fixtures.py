"""Real restore state for tests: a stand-in host layout, and pending restore transactions left by the restore helper's own crash points.

Shared by the lock-tool and update-broker tests, which both need a transaction that is genuinely pending (settings half-applied, or a repository restore with its
staging directory) rather than a hand-written record."""
import hashlib
import json
import os
import stat
import subprocess
import sys
from pathlib import Path

HELPER = Path(__file__).resolve().parents[1] / "update_manager/scripts/saturn-restore-transaction.py"

ARCHIVE_STATE_SCHEMA = '{"format":"saturn-persistent-state","schema_version":1,"state_schema_version":1}\n'


def tree_digest(root: Path) -> str:
    """Every path under root with its type, mode and content (or link target): equal digests mean nothing was written, removed or replaced."""
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        info = path.lstat()
        digest.update(f"{path.relative_to(root)}|{stat.filemode(info.st_mode)}|".encode())
        if stat.S_ISLNK(info.st_mode):
            digest.update(os.readlink(path).encode())
        elif stat.S_ISREG(info.st_mode):
            digest.update(path.read_bytes())
        digest.update(b"\n")
    return digest.hexdigest()


class Host:
    """A stand-in for the host's state, built the way the restore helper's own test builds it, plus the helper's failpoints to leave real transactions pending."""

    def __init__(self, root: Path):
        self.root = root
        self.state, self.scripts = root / "state", root / "scripts"
        self.pihpsdr, self.deskhpsdr = root / "pihpsdr", root / "deskhpsdr"
        self.archive, self.current, self.source = root / "archive", root / "current-repo", root / "source-repo"
        for directory in (self.state, self.scripts, self.pihpsdr, self.deskhpsdr, self.archive / "saturn-state",
                          self.current / ".git", self.current / "update_manager", self.source / ".git", self.source / "update_manager"):
            directory.mkdir(parents=True)
        (self.current / "source-version.txt").write_text("old source\n")
        (self.source / "source-version.txt").write_text("new source\n")
        (self.state / "repo_root.txt").write_text(f"{self.current}\n")
        (self.state / "remote_settings.json").write_text('{"value":"old"}\n')
        (self.archive / "saturn-state/state-schema.json").write_text(ARCHIVE_STATE_SCHEMA)
        (self.archive / "saturn-state/remote_settings.json").write_text('{"value":"new"}\n')
        files = []
        for path in sorted(self.archive.rglob("*")):
            if path.is_file():
                content = path.read_bytes()
                files.append({"inventory_id": "test", "archive_path": path.relative_to(self.archive).as_posix(), "size": len(content),
                              "mode": path.stat().st_mode & 0o777, "sha256": hashlib.sha256(content).hexdigest()})
        (self.archive / "manifest.json").write_text(json.dumps({"format": "saturn-settings-backup", "schema_version": 1, "files": files}))

    def helper(self, *arguments, failpoint=None, state=None):
        env = dict(os.environ, SATURN_RESTORE_TEST_AVAILABLE_BYTES="1073741824")
        if failpoint:
            env["SATURN_RESTORE_FAILPOINT"] = failpoint
        return subprocess.run([sys.executable, str(HELPER), *arguments], capture_output=True, text=True, env=env, timeout=60)

    def pending_settings(self):
        """A settings restore that died after its first file: the real helper's own crash point, leaving a pending transaction and a half-applied state."""
        result = self.helper("settings", "--state-root", str(self.state), "--archive-root", str(self.archive), "--scripts-root", str(self.scripts),
                             "--pihpsdr-root", str(self.pihpsdr), "--deskhpsdr-root", str(self.deskhpsdr), failpoint="settings_after_1")
        assert result.returncode == 97, result.stderr
        return self.transaction_ids()

    def pending_source(self):
        """A repository restore that died after copying the source: a staging directory and a pending transaction."""
        result = self.helper("source", "--state-root", str(self.state), "--source-root", str(self.source), "--current-repo-root", str(self.current),
                             "--repo-root-file", str(self.state / "repo_root.txt"), failpoint="source_after_copy")
        assert result.returncode == 97, result.stderr
        return self.transaction_ids()

    def transaction_ids(self):
        directory = self.state / "restore-transactions"
        return sorted(path.name for path in directory.iterdir()) if directory.is_dir() else []

    def statuses(self):
        return [json.loads((self.state / "restore-transactions" / name / "transaction.json").read_text())["status"] for name in self.transaction_ids()]

    def write_record(self, name, **fields):
        directory = self.state / "restore-transactions" / name
        directory.mkdir(parents=True, exist_ok=True)
        record = {"format": "saturn-restore-transaction", "schema_version": 1, "id": name, "kind": "settings", "entries": [], "status": "committed"}
        record.update(fields)
        record = {key: value for key, value in record.items() if value is not None}
        (directory / "transaction.json").write_text(json.dumps(record))
