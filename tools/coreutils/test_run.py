import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import run


class CoreutilsComparison(unittest.TestCase):
    def test_incompatible_guest_is_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "cat").write_bytes(b"not an ELF")
            item = run.run_case({"bin": "cat"}, root, [], None, tmp, 1)
            self.assertEqual(item["status"], "skipped")
            self.assertIn("ELF", item["reason"])

    def test_unexpected_exit_and_wrong_reference(self):
        executable = Path(sys.executable)
        if not run.is_x86_64_elf(executable):
            self.skipTest("requires x86-64 Linux Python")
        with tempfile.TemporaryDirectory() as tmp:
            case = {"bin": executable.name, "args": ["-c", "print('text')"]}
            native = run.run_case(case, executable.parent, [], None, tmp, 2)
            self.assertEqual(native["status"], "passed")
            native["result"]["stdout_b64"] = ""
            reference = {case["bin"]: native}
            row = run.run_case(case, executable.parent, [], reference, tmp, 2)
            self.assertEqual(row["status"], "failed")
            self.assertIn("stdout", row["reason"])
            native["binary_sha256"] = "wrong"
            row = run.run_case(case, executable.parent, [], reference, tmp, 2)
            self.assertEqual(row["status"], "skipped")
            self.assertIn("differs", row["reason"])
            error = run.run_case(dict(case, args=["-c", "raise SystemExit(1)"]),
                                 executable.parent, [], None, tmp, 2)
            self.assertEqual(error["status"], "failed")

    def test_missing_programs_are_in_the_denominator(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "report.json"
            ref = root / "native.json"
            ref.write_text(json.dumps({
                "schema": run.SCHEMA, "backend": "native",
                "corpus_sha256": run.digest(run.CASES), "cases": [],
            }))
            result = subprocess.run([
                sys.executable, run.__file__, "--backend", "qemu",
                "--runner", sys.executable, "--bin-dir", tmp, "--baseline", str(ref),
                "--output", str(output),
            ], check=False, capture_output=True)
            self.assertEqual(result.returncode, 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["skipped"], len(json.loads(run.CASES.read_text())))
            self.assertEqual(report["percent_passed"], 0)


if __name__ == "__main__":
    unittest.main()
