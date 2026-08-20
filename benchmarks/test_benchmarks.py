import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class BenchmarkScriptsTest(unittest.TestCase):
    def valid_evidence(self) -> dict[str, object]:
        return {
            "schema_version": 1,
            "classification": "bootstrap-infrastructure-only",
            "git_sha": "1" * 40,
            "host": {
                "system": "TestOS",
                "release": "1",
                "machine": "test64",
                "python": "3",
            },
            "command": ["test-binary"],
            "warmup": 1,
            "samples_ns": [1] * 10,
            "median_ns": 1,
            "p95_ns": 1,
            "binary": {"path": "test-binary", "bytes": 1, "sha256": "0" * 64},
        }

    def run_checker(self, evidence: dict[str, object]) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            path.write_text(json.dumps(evidence), encoding="utf-8")
            return subprocess.run(
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

    def test_checker_accepts_valid_bootstrap_evidence(self) -> None:
        completed = self.run_checker(self.valid_evidence())
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_checker_rejects_missing_provenance(self) -> None:
        evidence = self.valid_evidence()
        del evidence["git_sha"]
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_aggregate_mismatch(self) -> None:
        evidence = self.valid_evidence()
        evidence["median_ns"] = 2
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_non_hexadecimal_checksum(self) -> None:
        evidence = self.valid_evidence()
        evidence["binary"] = {"path": "test-binary", "bytes": 1, "sha256": "z" * 64}
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_boolean_numbers(self) -> None:
        evidence = self.valid_evidence()
        evidence["samples_ns"] = [True] * 10
        evidence["median_ns"] = True
        evidence["p95_ns"] = True
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_non_integer_schema_version(self) -> None:
        for invalid in (True, 1.0):
            with self.subTest(invalid=invalid):
                evidence = self.valid_evidence()
                evidence["schema_version"] = invalid
                completed = self.run_checker(evidence)
                self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_command_binary_mismatch(self) -> None:
        evidence = self.valid_evidence()
        evidence["command"] = ["different-binary"]
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_binds_binary_and_expected_sha(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "test-binary"
            binary.write_bytes(b"test executable")
            evidence = self.valid_evidence()
            evidence["command"] = [str(binary)]
            evidence["binary"] = {
                "path": str(binary),
                "bytes": binary.stat().st_size,
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }
            path = root / "evidence.json"
            path.write_text(json.dumps(evidence), encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(path),
                    "--bootstrap",
                    "--expected-git-sha",
                    "1" * 40,
                    "--binary",
                    str(binary),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()
