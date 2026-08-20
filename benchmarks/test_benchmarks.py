import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class BenchmarkScriptsTest(unittest.TestCase):
    def test_checker_accepts_valid_bootstrap_evidence(self) -> None:
        evidence = {
            "schema_version": 1,
            "classification": "bootstrap-infrastructure-only",
            "samples_ns": [1] * 10,
            "binary": {"bytes": 1, "sha256": "0" * 64},
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            path.write_text(json.dumps(evidence), encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(path),
                    "--bootstrap",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()

