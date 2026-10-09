#!/usr/bin/env python3
"""Tests for owner_reader.py: the read-only reader of the acquisition owner's cached telemetry.

  python3 tools/browser-comparison/test_owner_reader.py
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
READER = os.environ.get("OWNER_READER_UNDER_TEST", os.path.join(HERE, "owner_reader.py"))


def write_doc(directory, updated_ms=None, pid=None, extra=None, raw=None):
    path = os.path.join(directory, "perf.json")
    if raw is not None:
        with open(path, "w") as fh:
            fh.write(raw)
        return path
    doc = {"schema_version": 1, "updated_at_ms": int(time.time() * 1000) if updated_ms is None else updated_ms,
           "source": "saturn-bridge", "backend": "xdma",
           "metrics": {"pid": os.getpid() if pid is None else pid, "client": 1, "tci_nodelay_enabled": 1,
                       "tci_nodelay_confirmed_total": 2, "tci_nodelay_failed_total": 0}}
    if extra:
        doc["metrics"].update(extra)
    with open(path, "w") as fh:
        json.dump(doc, fh)
    return path


def read(path, *extra):
    out = subprocess.run([sys.executable, READER, "--once", "--path", path, *extra], capture_output=True, text=True, timeout=30)
    assert out.returncode == 0, out.stderr
    return json.loads(out.stdout)


class Reader(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="owner-reader-")
        self.addCleanup(lambda: __import__("shutil").rmtree(self.dir, ignore_errors=True))

    def test_the_age_is_computed_on_this_machine_from_the_documents_own_timestamp(self):
        path = write_doc(self.dir, updated_ms=int(time.time() * 1000) - 1500)
        answer = read(path)
        self.assertTrue(answer["ok"])
        self.assertEqual(answer["piSourceAgeMs"], answer["piReadAtMs"] - answer["piSourceUpdatedAtMs"])
        self.assertTrue(1400 <= answer["piSourceAgeMs"] <= 4000, answer["piSourceAgeMs"])

    def test_a_stale_document_is_reported_stale_and_kept_never_refreshed(self):
        stale = int(time.time() * 1000) - 86_400_000
        path = write_doc(self.dir, updated_ms=stale)
        first, second = read(path), read(path)
        for answer in (first, second):
            self.assertTrue(answer["ok"])
            self.assertEqual(answer["piSourceUpdatedAtMs"], stale, "the document's own timestamp must never be refreshed")
            self.assertEqual(answer["document"]["updated_at_ms"], stale)
            self.assertGreater(answer["piSourceAgeMs"], 86_000_000)

    def test_the_original_document_is_preserved_exactly(self):
        path = write_doc(self.dir, extra={"audio_dropped_s": 0.25, "build_git_sha": "abc"})
        answer = read(path)
        with open(path, "rb") as fh:
            raw = fh.read()
        self.assertEqual(answer["document"], json.loads(raw))
        self.assertEqual(answer["documentSha256"], hashlib.sha256(raw).hexdigest())

    def test_a_missing_file_is_unavailable_not_zero(self):
        answer = read(os.path.join(self.dir, "nope.json"))
        self.assertFalse(answer["ok"])
        self.assertNotIn("document", answer)
        self.assertNotIn("piSourceAgeMs", answer)

    def test_unparsable_and_non_finite_documents_are_unavailable(self):
        for raw in ("{ not json", '{"updated_at_ms": NaN, "metrics": {}}', '{"updated_at_ms": 1, "metrics": {"x": Infinity}}', ""):
            with self.subTest(raw=raw):
                answer = read(write_doc(self.dir, raw=raw))
                self.assertFalse(answer["ok"], raw)

    def test_a_document_without_a_timestamp_or_metrics_is_unavailable(self):
        for raw in ('{"metrics": {}}', '{"updated_at_ms": "soon", "metrics": {}}', '{"updated_at_ms": 5}', '{"updated_at_ms": true, "metrics": {}}', "[1,2]"):
            with self.subTest(raw=raw):
                self.assertFalse(read(write_doc(self.dir, raw=raw))["ok"], raw)

    def test_the_owner_process_identity_is_read_from_proc(self):
        answer = read(write_doc(self.dir))   # the document names this test process as the owner
        owner = answer["owner"]
        self.assertEqual(owner["pid"], os.getpid())
        self.assertTrue(owner["alive"])
        self.assertIsInstance(owner["startTicks"], int)
        self.assertGreater(owner["clockTicksPerSecond"], 0)
        with open("/proc/sys/kernel/random/boot_id") as fh:
            self.assertEqual(owner["bootId"], fh.read().strip())

    def _copied_sleep(self):
        """A running process whose executable is a file we control, so its hash is known independently."""
        import shutil
        source = shutil.which("sleep")
        copy = os.path.join(self.dir, "sleep-copy")
        shutil.copy(source, copy)
        os.chmod(copy, 0o755)
        child = subprocess.Popen([copy, "30"])
        self.addCleanup(child.kill)
        time.sleep(0.2)
        with open(copy, "rb") as fh:
            return child, copy, hashlib.sha256(fh.read()).hexdigest()

    def test_the_running_executable_is_hashed(self):
        child, copy, expected = self._copied_sleep()
        owner = read(write_doc(self.dir, pid=child.pid))["owner"]
        self.assertTrue(owner["alive"], owner)
        self.assertEqual(owner["pid"], child.pid)
        self.assertEqual(owner["exe"], copy)
        self.assertEqual(owner["exeSha256"], expected)

    def test_the_running_image_is_hashed_even_after_its_file_was_replaced_on_disk(self):
        child, copy, expected = self._copied_sleep()
        os.unlink(copy)                       # the running image survives; the path now names nothing
        with open(copy, "w") as fh:
            fh.write("a different file at the same path\n")
        owner = read(write_doc(self.dir, pid=child.pid))["owner"]
        self.assertEqual(owner["exeSha256"], expected, "the hash must be of the running image, not of the file now on disk")
        self.assertEqual(owner["exe"], copy)

    def test_the_start_ticks_are_the_kernels_start_time_of_that_process(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        self.addCleanup(child.kill)
        time.sleep(0.2)
        with open(f"/proc/{child.pid}/stat") as fh:
            stat = fh.read()
        independent = int(stat[stat.rindex(")") + 2:].split()[19])   # field 22: starttime, after the command name
        self.assertEqual(read(write_doc(self.dir, pid=child.pid))["owner"]["startTicks"], independent)

    def test_a_command_name_containing_parentheses_and_spaces_does_not_confuse_the_start_time(self):
        import shutil
        weird = os.path.join(self.dir, "a) (b c")
        shutil.copy(shutil.which("sleep"), weird)
        os.chmod(weird, 0o755)
        child = subprocess.Popen([weird, "30"])
        self.addCleanup(child.kill)
        time.sleep(0.2)
        with open(f"/proc/{child.pid}/stat") as fh:
            stat = fh.read()
        independent = int(stat[stat.rindex(")") + 2:].split()[19])
        self.assertEqual(read(write_doc(self.dir, pid=child.pid))["owner"]["startTicks"], independent)

    def test_a_boolean_in_the_pid_field_is_not_taken_for_process_one(self):
        # JSON true is an integer in Python; reading it as pid 1 would report init's identity as the owner's.
        answer = read(write_doc(self.dir, pid=True))
        self.assertTrue(answer["ok"])
        self.assertFalse(answer["owner"]["alive"])
        self.assertIsNone(answer["owner"]["startTicks"])
        self.assertIsNone(answer["owner"]["exeSha256"])
        self.assertIn("no process id", answer["owner"]["exeError"], "a malformed pid is reported as naming no process, not as a dead one")

    def test_a_cached_executable_hash_is_never_reused_for_a_process_that_started_later_under_the_same_pid(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("owner_reader_under_test", READER)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        ticks, hashes, calls = iter([111, 111, 222]), iter(["a" * 64, "b" * 64]), []
        module.proc_start_ticks = lambda pid: next(ticks)
        module.sha256_file = lambda path: calls.append(path) or next(hashes)
        pid = os.getpid()
        first = module.owner_identity(pid, None)["exeSha256"]
        again = module.owner_identity(pid, None)["exeSha256"]      # same start time: same process, the cached hash is correct
        later = module.owner_identity(pid, None)["exeSha256"]      # different start time: a different process that reused the pid
        self.assertEqual((first, again, later), ("a" * 64, "a" * 64, "b" * 64))
        self.assertEqual(len(calls), 2, "the same process is hashed once, a new process is hashed again")

    def test_a_process_that_is_not_running_is_reported_not_alive(self):
        child = subprocess.Popen([sys.executable, "-c", "pass"])
        child.wait()
        owner = read(write_doc(self.dir, pid=child.pid))["owner"]
        self.assertFalse(owner["alive"])
        self.assertIsNone(owner["exeSha256"])
        self.assertIsNone(owner["startTicks"])

    def test_a_document_that_names_no_process_has_no_identity(self):
        owner = read(write_doc(self.dir, pid=0))["owner"]
        self.assertFalse(owner["alive"])
        self.assertIsNone(owner["exeSha256"])

    def test_a_restart_changes_the_process_start_identity(self):
        a = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        self.addCleanup(a.kill)
        time.sleep(0.05)
        b = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        self.addCleanup(b.kill)
        ta = read(write_doc(self.dir, pid=a.pid))["owner"]["startTicks"]
        tb = read(write_doc(self.dir, pid=b.pid))["owner"]["startTicks"]
        self.assertNotEqual((a.pid, ta), (b.pid, tb))

    def test_it_never_writes_to_the_directory_it_reads(self):
        path = write_doc(self.dir)
        before = (sorted(os.listdir(self.dir)), os.stat(path).st_mtime_ns, open(path, "rb").read())
        read(path)
        read(path, "--service", "no-such-unit")
        after = (sorted(os.listdir(self.dir)), os.stat(path).st_mtime_ns, open(path, "rb").read())
        self.assertEqual(before, after)

    def test_the_persistent_protocol_answers_each_request_and_quits(self):
        path = write_doc(self.dir)
        proc = subprocess.Popen([sys.executable, READER, "--path", path], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(proc.kill)
        answers = []
        for _ in range(3):
            proc.stdin.write("read\n")
            proc.stdin.flush()
            answers.append(json.loads(proc.stdout.readline()))
        proc.stdin.write("quit\n")
        proc.stdin.flush()
        self.assertEqual(proc.wait(timeout=10), 0)
        self.assertTrue(all(a["ok"] for a in answers))
        self.assertEqual(len({a["owner"]["pid"] for a in answers}), 1)

    def test_it_exits_cleanly_when_the_session_closes(self):
        proc = subprocess.Popen([sys.executable, READER, "--path", write_doc(self.dir)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(proc.kill)
        proc.stdin.close()
        self.assertEqual(proc.wait(timeout=10), 0)

    def test_it_ignores_unknown_commands_without_answering_them(self):
        proc = subprocess.Popen([sys.executable, READER, "--path", write_doc(self.dir)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(proc.kill)
        proc.stdin.write("write\nread\n")
        proc.stdin.flush()
        first = json.loads(proc.stdout.readline())
        self.assertTrue(first["ok"])   # the first line out is the answer to "read"; "write" produced nothing
        proc.stdin.close()
        self.assertEqual(proc.wait(timeout=10), 0)

    def test_the_reader_has_no_way_to_reach_hardware(self):
        source = open(READER).read()
        for forbidden in ("/dev/xdma", "xdma_user", "mmap", "ioctl", "saturn-bridge --", "Popen(", "os.system", "shell=True"):
            self.assertNotIn(forbidden, source.split('"""', 2)[2], forbidden)
        # the only subprocess it can start is systemctl show, read-only, and only when asked
        self.assertIn('["systemctl", "show"', source)

    def test_it_needs_nothing_beyond_the_standard_library(self):
        source = open(READER).read()
        imports = {line.split()[1].split(".")[0] for line in source.splitlines() if line.startswith("import ")}
        self.assertTrue(imports <= {"argparse", "hashlib", "json", "os", "subprocess", "sys", "time"}, imports)


if __name__ == "__main__":
    unittest.main(verbosity=1)
