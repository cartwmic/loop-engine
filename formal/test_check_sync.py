#!/usr/bin/env python3
"""Tests for formal/check-sync.py. formal/check.sh runs this, so the `checks` job does too.

Each test builds a temp tree with a copy of formal/check-sync.py and of every file
COVERS names. The hashes in the temp COVERS are computed from those copies, so the
tests never depend on uncommitted edits in the real tree. The real tree is never
edited. Standard library only.
"""

import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

FORMAL = pathlib.Path(__file__).resolve().parent
ROOT = FORMAL.parent
TARGET = "crates/loop-integrations/src/sqlite.rs"


def covered_paths():
    paths = []
    for line in (FORMAL / "COVERS").read_text().splitlines():
        text = line.strip()
        if text and not text.startswith("#"):
            paths.append(text.split()[0])
    return paths


class SyncCheckTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.tree = pathlib.Path(self.temp.name)
        (self.tree / "formal").mkdir()
        shutil.copy(FORMAL / "check-sync.py", self.tree / "formal" / "check-sync.py")
        lines = []
        for path in covered_paths():
            (self.tree / path).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / path, self.tree / path)
            digest = subprocess.run(
                ["git", "hash-object", "--", path],
                cwd=self.tree,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            lines.append(f"{path} {digest}")
        (self.tree / "formal" / "COVERS").write_text("\n".join(lines) + "\n")

    def tearDown(self):
        self.temp.cleanup()

    def sync(self, *args):
        return subprocess.run(
            [sys.executable, str(self.tree / "formal" / "check-sync.py"), *args],
            capture_output=True,
            text=True,
        )

    def test_copy_is_in_sync(self):
        result = self.sync()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_changed_covered_file_exits_1_and_is_named(self):
        with (self.tree / TARGET).open("a") as source:
            source.write("\n// sync test edit\n")
        result = self.sync()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        changed = [line for line in result.stderr.splitlines() if "(changed)" in line]
        self.assertEqual(len(changed), 1, result.stderr)
        self.assertIn(TARGET, changed[0])

    def test_update_then_check_passes(self):
        with (self.tree / TARGET).open("a") as source:
            source.write("\n// sync test edit\n")
        self.assertEqual(self.sync("--update").returncode, 0)
        result = self.sync()
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
