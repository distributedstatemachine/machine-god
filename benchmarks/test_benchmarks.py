import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks"))

from upstream import (  # noqa: E402
    EXPECTED_RUST_VERSION,
    EXPECTED_ZIG_VERSION,
    UpstreamLock,
    command_plan,
    parse_upstream_lock,
    validate_upstream_evidence,
)


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


class UpstreamHarnessTest(unittest.TestCase):
    def command_record(self, command: list[str]) -> dict[str, object]:
        return {
            "command": command,
            "cwd": "/checkout",
            "elapsed_ns": 10,
            "returncode": 0,
            "stdout_sha256": "0" * 64,
            "stderr_sha256": "1" * 64,
        }

    def valid_upstream_evidence(self) -> dict[str, object]:
        samples = [{"elapsed_ns": value, "returncode": 0} for value in range(1, 11)]
        builds = []
        for project, profile, command in (
            ("fx", "ReleaseSafe", ["zig", "build", "-Doptimize=ReleaseSafe"]),
            (
                "machine-god",
                "release",
                ["cargo", "+1.94.1", "build", "--locked", "--release"],
            ),
        ):
            build = self.command_record(command)
            build.update(
                {
                    "project": project,
                    "profile": profile,
                    "binary": {
                        "path": f"/checkout/{project}",
                        "bytes": 1,
                        "sha256": "2" * 64,
                    },
                }
            )
            builds.append(build)
        return {
            "schema_version": 2,
            "classification": "bootstrap-infrastructure-only",
            "claim_eligible": False,
            "generated_at_utc": "2026-08-20T00:00:00Z",
            "source": {
                "machine_god": {"git_sha": "3" * 40, "dirty": False},
                "fx": {
                    "repository": "https://github.com/vercel-labs/fx.git",
                    "locked_commit": "4" * 40,
                    "verified_commit": "4" * 40,
                    "preparation_commands": [
                        self.command_record(["git", "fetch", "4" * 40])
                    ],
                },
            },
            "host": {
                "system": "TestOS",
                "release": "1",
                "machine": "test64",
                "python": "3.14",
                "cpu_count": 1,
            },
            "tools": {
                "git": {
                    "command": ["git"],
                    "executable": "/usr/bin/git",
                    "version": "git version 2",
                },
                "zig": {
                    "command": ["zig"],
                    "executable": "/usr/bin/zig",
                    "required_version": EXPECTED_ZIG_VERSION,
                    "version": EXPECTED_ZIG_VERSION,
                },
                "rustc": {
                    "command": ["rustc", "+1.94.1"],
                    "executable": "/usr/bin/rustc",
                    "required_version": EXPECTED_RUST_VERSION,
                    "version": "rustc 1.94.1 (test 2026-01-01)",
                },
                "cargo": {
                    "command": ["cargo", "+1.94.1"],
                    "executable": "/usr/bin/cargo",
                    "required_version": EXPECTED_RUST_VERSION,
                    "version": "cargo 1.94.1 (test 2026-01-01)",
                },
            },
            "builds": builds,
            "workloads": [
                {
                    "id": "bootstrap-exit",
                    "description": "harness smoke path",
                    "equivalence": "non-equivalent",
                    "claim_eligible": False,
                    "reason": "the programs execute different bootstrap behavior",
                    "implementations": [
                        {
                            "project": project,
                            "status": "measured",
                            "command": [f"/checkout/{project}"],
                            "cwd": "/checkout",
                            "environment_overrides": {"HOME": "/tmp/home"},
                            "warmup": 1,
                            "samples": samples,
                            "median_ns": 5,
                            "p95_ns": 10,
                        }
                        for project in ("fx", "machine-god")
                    ],
                }
            ],
        }

    def write_lock(self, contents: str) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        directory = tempfile.TemporaryDirectory()
        path = Path(directory.name) / "upstream.lock"
        path.write_text(contents, encoding="utf-8")
        return directory, path

    def test_parses_exact_upstream_lock(self) -> None:
        directory, path = self.write_lock(
            "# pinned source\n"
            "repository=https://github.com/vercel-labs/fx.git\n"
            "commit=b1774fbf6c7602b503026f96f6e960e946c692ef\n"
            "zig=0.16.0\n"
        )
        with directory:
            lock = parse_upstream_lock(path)
        self.assertEqual(lock.repository, "https://github.com/vercel-labs/fx.git")
        self.assertEqual(lock.commit, "b1774fbf6c7602b503026f96f6e960e946c692ef")
        self.assertEqual(lock.zig, "0.16.0")

    def test_rejects_incomplete_or_ambiguous_upstream_lock(self) -> None:
        invalid_locks = (
            "repository=https://example.test/fx.git\ncommit=" + "a" * 40 + "\n",
            "repository=https://example.test/fx.git\ncommit="
            + "a" * 40
            + "\ncommit="
            + "b" * 40
            + "\nzig=0.16.0\n",
            "repository=https://example.test/fx.git\ncommit="
            + "A" * 40
            + "\nzig=0.16.0\n",
            "repository=https://example.test/fx.git\ncommit="
            + "a" * 40
            + "\nzig=0.14.1\n",
        )
        for contents in invalid_locks:
            with self.subTest(contents=contents):
                directory, path = self.write_lock(contents)
                with directory, self.assertRaises(ValueError):
                    parse_upstream_lock(path)

    def test_constructs_pinned_clone_and_release_build_commands(self) -> None:
        lock = UpstreamLock(
            "https://github.com/vercel-labs/fx.git",
            "b1774fbf6c7602b503026f96f6e960e946c692ef",
            "0.16.0",
        )
        plan = command_plan(Path("/repo"), Path("/repo/.bench/fx"), lock)
        self.assertEqual(
            plan["clone"],
            [
                "git",
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                lock.repository,
                "/repo/.bench/fx",
            ],
        )
        self.assertEqual(
            plan["fetch"],
            [
                "git",
                "-C",
                "/repo/.bench/fx",
                "fetch",
                "--depth",
                "1",
                "origin",
                lock.commit,
            ],
        )
        self.assertEqual(
            plan["checkout"],
            ["git", "-C", "/repo/.bench/fx", "checkout", "--detach", lock.commit],
        )
        self.assertEqual(plan["fx_build"], ["zig", "build", "-Doptimize=ReleaseSafe"])
        self.assertEqual(
            plan["machine_god_build"],
            [
                "cargo",
                "+1.94.1",
                "build",
                "--locked",
                "--release",
                "-p",
                "machine-god-cli",
            ],
        )

    def test_accepts_provenance_complete_upstream_evidence(self) -> None:
        validate_upstream_evidence(self.valid_upstream_evidence())

    def test_rejects_false_comparison_claim(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["workloads"][0]["equivalence"] = "equivalent"
        evidence["workloads"][0]["claim_eligible"] = True
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_unverified_upstream_commit(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["source"]["fx"]["verified_commit"] = "5" * 40
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_aggregate_not_derived_from_raw_samples(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["workloads"][0]["implementations"][0]["p95_ns"] = 9
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_schema_two_checker_binds_machine_git_sha(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence_path = Path(directory) / "upstream.json"
            evidence_path.write_text(
                json.dumps(self.valid_upstream_evidence()), encoding="utf-8"
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(evidence_path),
                    "--expected-git-sha",
                    "3" * 40,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()
