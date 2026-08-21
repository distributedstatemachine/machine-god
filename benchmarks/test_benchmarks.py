import hashlib
import io
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks"))

from upstream import (  # noqa: E402
    ALLOWED_MACHINE_OUTPUTS,
    BOOTSTRAP_DESCRIPTION,
    BOOTSTRAP_REASON,
    CONTAINMENT_ENVIRONMENT_KEY,
    EXPECTED_RUST_VERSION,
    EXPECTED_ZIG_VERSION,
    LinuxProcessInfo,
    LinuxProcessSupervisor,
    MachineStatusEntry,
    ProcessTimeout,
    UpstreamLock,
    acquire_output_lock,
    bounded_sha256_file,
    canonical_git_entries_sha256,
    canonical_manifest_sha256,
    check_machine_cleanliness,
    command_plan,
    collect_and_publish_evidence,
    executable_identity,
    finalize_successful_process,
    invocation_path,
    linux_containment_preflight,
    machine_tree_command,
    materialize_machine_source,
    parse_upstream_lock,
    parse_porcelain_v1_z,
    prepare_upstream,
    run_measurement,
    run_process,
    sha256_file,
    source_tree_sha256,
    unavailable_workloads,
    validate_binary_file,
    validate_upstream_evidence,
    write_evidence_atomic,
)
import check as benchmark_check  # noqa: E402
import run as benchmark_run  # noqa: E402
import upstream  # noqa: E402


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
        return self.run_checker_text(json.dumps(evidence))

    def run_checker_text(self, evidence: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            path.write_text(evidence, encoding="utf-8")
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

    def test_checker_rejects_non_object_json_without_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence_path = Path(directory) / "evidence.json"
            evidence_path.write_text("[]\n", encoding="utf-8")
            completed = subprocess.run(
                [sys.executable, str(ROOT / "benchmarks/check.py"), str(evidence_path)],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(completed.stderr, "benchmark evidence must be an object\n")
        self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_invalid_json_without_traceback(self) -> None:
        completed = self.run_checker_text("{\n")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("invalid benchmark evidence", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_duplicate_members_and_nonfinite_numbers(self) -> None:
        cases = (
            '{"schema_version":2,"schema_version":2}',
            '{"schema_version":2,"source":{"machine_god":{},"machine_god":{}}}',
            '{"schema_version":NaN}',
            '{"schema_version":Infinity}',
            '{"schema_version":-Infinity}',
            '{"schema_version":1e9999}',
        )
        for evidence in cases:
            with self.subTest(evidence=evidence):
                completed = self.run_checker_text(evidence)
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("invalid benchmark evidence", completed.stderr)
                self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_command_binary_mismatch(self) -> None:
        evidence = self.valid_evidence()
        evidence["command"] = ["different-binary"]
        completed = self.run_checker(evidence)
        self.assertNotEqual(completed.returncode, 0)

    def test_checker_rejects_nul_paths_without_traceback(self) -> None:
        mutations = (
            ("binary.path", lambda data: data["binary"].__setitem__("path", "bad\0path")),
            ("command[0]", lambda data: data["command"].__setitem__(0, "bad\0path")),
        )
        for field, mutate in mutations:
            with self.subTest(field=field):
                evidence = self.valid_evidence()
                mutate(evidence)
                completed = self.run_checker(evidence)

                self.assertNotEqual(completed.returncode, 0)
                self.assertEqual(
                    completed.stderr,
                    f"{field} is not a valid filesystem path\n",
                )
                self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_missing_supplied_binary_without_traceback(self) -> None:
        evidence = self.valid_evidence()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            missing_binary = root / "missing-binary"
            evidence["binary"]["path"] = str(missing_binary)
            evidence["command"] = [str(missing_binary)]
            evidence_path = root / "evidence.json"
            evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(evidence_path),
                    "--bootstrap",
                    "--binary",
                    str(missing_binary),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("failed to inspect supplied binary", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_non_regular_supplied_binary_before_hashing(self) -> None:
        evidence = self.valid_evidence()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            non_regular_binary = root / "binary-directory"
            non_regular_binary.mkdir()
            evidence["binary"]["path"] = str(non_regular_binary)
            evidence["command"] = [str(non_regular_binary)]
            evidence_path = root / "evidence.json"
            evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(evidence_path),
                    "--bootstrap",
                    "--binary",
                    str(non_regular_binary),
                ],
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )

        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(
            completed.stderr,
            "supplied binary is not a regular file\n",
        )
        self.assertNotIn("Traceback", completed.stderr)

    def test_checker_rejects_non_executable_regular_supplied_binary(self) -> None:
        evidence = self.valid_evidence()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "non-executable-binary"
            binary.write_bytes(b"test executable")
            binary.chmod(0o600)
            evidence["binary"] = {
                "path": str(binary),
                "bytes": binary.stat().st_size,
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }
            evidence["command"] = [str(binary)]
            evidence_path = root / "evidence.json"
            evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(evidence_path),
                    "--bootstrap",
                    "--binary",
                    str(binary),
                ],
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )

        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(completed.stderr, "supplied binary is not executable\n")
        self.assertNotIn("Traceback", completed.stderr)

    def test_binary_hash_reads_only_declared_size_and_one_eof_byte(self) -> None:
        source = io.BytesIO(b"expected-unbounded-extra-data")

        with self.assertRaisesRegex(ValueError, "became longer"):
            benchmark_check.file_sha256(source, len(b"expected"))

        self.assertEqual(source.tell(), len(b"expected") + 1)

    def test_checker_rejects_undeclared_and_malformed_schema_one_fields(self) -> None:
        mutations = (
            lambda data: data.__setitem__("performance_claim", "100x"),
            lambda data: data["host"].__setitem__("winner", "machine-god"),
            lambda data: data.__setitem__("binary", None),
            lambda data: data.__setitem__("binary", []),
            lambda data: data["binary"].__setitem__("result", "faster"),
            lambda data: data["command"].append("--extra"),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_evidence()
                mutate(evidence)
                completed = self.run_checker(evidence)
                self.assertNotEqual(completed.returncode, 0)
                self.assertNotIn("Traceback", completed.stderr)

    def test_checker_handles_huge_integer_samples_with_exact_arithmetic(self) -> None:
        evidence = self.valid_evidence()
        huge = 10**4000
        evidence["samples_ns"] = [huge] * 10
        evidence["median_ns"] = huge
        evidence["p95_ns"] = huge
        completed = self.run_checker(evidence)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_collector_emits_exact_large_integer_median_accepted_by_checker(self) -> None:
        lower_middle = 10**4000 + 1
        upper_middle = lower_middle + 1
        samples = [upper_middle, lower_middle] * 5

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "test-binary"
            binary.write_bytes(b"test executable")
            binary.chmod(0o755)
            evidence_path = root / "evidence.json"
            run_results = [(1, 0), *((sample, 0) for sample in samples)]

            with (
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        str(ROOT / "benchmarks/run.py"),
                        "--binary",
                        str(binary),
                        "--output",
                        str(evidence_path),
                        "--runs",
                        "10",
                        "--warmup",
                        "1",
                    ],
                ),
                mock.patch.object(
                    benchmark_run, "run_once", side_effect=run_results
                ),
                mock.patch.object(
                    benchmark_run.subprocess,
                    "check_output",
                    return_value="1" * 40 + "\n",
                ),
                mock.patch("builtins.print"),
            ):
                self.assertEqual(benchmark_run.main(), 0)

            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
            self.assertEqual(evidence["median_ns"], lower_middle)
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/check.py"),
                    str(evidence_path),
                    "--bootstrap",
                    "--binary",
                    str(binary),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_checker_binds_binary_and_expected_sha(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "test-binary"
            binary.write_bytes(b"test executable")
            binary.chmod(0o755)
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
    def base_environment(self, scratch: Path) -> dict[str, str]:
        return {
            "HOME": str(scratch / "home"),
            "LANG": "C",
            "LC_ALL": "C",
            CONTAINMENT_ENVIRONMENT_KEY: "a" * 32,
            "NO_COLOR": "1",
            "PATH": "/usr/bin:/bin",
            "TMPDIR": str(scratch / "tmp"),
        }

    def command_record(
        self,
        command: list[str],
        environment: dict[str, str],
        timeout: float,
        cwd: Path,
    ) -> dict[str, object]:
        return {
            "command": command,
            "cwd": str(cwd),
            "environment": environment,
            "timeout_seconds": timeout,
            "elapsed_ns": 10,
            "setup_ns": 1,
            "supervision_ns": 1,
            "cleanup_ns": 1,
            "returncode": 0,
            "stdout_sha256": "0" * 64,
            "stderr_sha256": "1" * 64,
        }

    def executable_record(
        self, path: Path, byte_count: int, checksum: str
    ) -> dict[str, object]:
        return {
            "executable": str(path),
            "canonical_executable": str(path),
            "sha256": checksum,
            "bytes": byte_count,
            "mode": 0o755,
            "device": 1,
            "inode": 2,
            "mtime_ns": 3,
            "ctime_ns": 4,
            "invocation_mode": 0o755,
            "invocation_device": 1,
            "invocation_inode": 2,
            "invocation_mtime_ns": 3,
            "invocation_ctime_ns": 4,
            "invocation_link_target": "",
        }

    def binary_record(self, path: Path) -> dict[str, object]:
        content = path.read_bytes()
        return {
            "path": str(path),
            "bytes": len(content),
            "sha256": hashlib.sha256(content).hexdigest(),
        }

    def test_schema_two_binary_validation_rejects_non_regular_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary-directory"
            binary.mkdir()
            record = {"path": str(binary), "bytes": 1, "sha256": "0" * 64}

            with (
                mock.patch.object(upstream, "bounded_sha256_file") as hash_file,
                self.assertRaises(ValueError),
            ):
                validate_binary_file(record, binary, "build.binary")

            hash_file.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "executable mode regression requires POSIX")
    def test_schema_two_binary_validation_rejects_non_executable_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "non-executable-binary"
            binary.write_bytes(b"test executable")
            binary.chmod(0o600)

            with self.assertRaisesRegex(ValueError, "is not executable"):
                validate_binary_file(
                    self.binary_record(binary), binary, "build.binary"
                )

    def test_schema_two_binary_hash_is_bounded_to_declared_size(self) -> None:
        source = io.BytesIO(b"expected-unbounded-extra-data")

        with self.assertRaisesRegex(ValueError, "became longer"):
            bounded_sha256_file(source, len(b"expected"))

        self.assertEqual(source.tell(), len(b"expected") + 1)

    def test_schema_two_binary_validation_closes_descriptor_on_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"test executable")
            binary.chmod(0o755)
            record = self.binary_record(binary)
            record["sha256"] = "0" * 64
            opened_descriptors: list[int] = []
            original_open = os.open

            def tracking_open(path: object, flags: int) -> int:
                descriptor = original_open(path, flags)
                opened_descriptors.append(descriptor)
                return descriptor

            with (
                mock.patch.object(upstream.os, "open", side_effect=tracking_open),
                self.assertRaisesRegex(ValueError, "sha256"),
            ):
                validate_binary_file(record, binary, "build.binary")

            self.assertEqual(len(opened_descriptors), 1)
            with self.assertRaises(OSError):
                os.fstat(opened_descriptors[0])

    @unittest.skipUnless(os.name == "posix", "pathname replacement requires POSIX")
    def test_schema_two_binary_validation_rejects_path_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "binary"
            displaced = root / "displaced-binary"
            replacement = root / "replacement-binary"
            binary.write_bytes(b"original")
            replacement.write_bytes(b"replaced")
            binary.chmod(0o755)
            replacement.chmod(0o755)
            record = self.binary_record(binary)
            original_hash = upstream.bounded_sha256_file

            def hash_then_replace(source: object, expected_bytes: int) -> str:
                checksum = original_hash(source, expected_bytes)
                binary.rename(displaced)
                replacement.rename(binary)
                return checksum

            with (
                mock.patch.object(
                    upstream,
                    "bounded_sha256_file",
                    side_effect=hash_then_replace,
                ),
                self.assertRaisesRegex(ValueError, "path.*changed"),
            ):
                validate_binary_file(record, binary, "build.binary")

    def valid_upstream_evidence(
        self,
        root: Path = Path("/checkout"),
        *,
        fx_root: Path | None = None,
        scratch: Path | None = None,
        machine_sha: str = "3" * 40,
        git_tree: str = "5" * 40,
    ) -> dict[str, object]:
        lock_path = ROOT / "benchmarks/upstream.lock"
        lock = parse_upstream_lock(lock_path)
        fx_root = fx_root or root / ".bench/fx"
        scratch = scratch or root / ".bench/scratch"
        fx_binary = fx_root / "zig-out/bin/fx"
        machine_binary = scratch / "machine-target/release/machine-god"
        machine_source = scratch / "machine-source"
        machine_manifest = scratch / "machine-source-manifest.json"
        base = self.base_environment(scratch)
        git_environment = {
            **base,
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_NO_REPLACE_OBJECTS": "1",
            "GIT_TERMINAL_PROMPT": "0",
        }
        fx_environment = {
            **base,
            "ZIG_GLOBAL_CACHE_DIR": str(scratch / "zig-global-cache"),
            "ZIG_LOCAL_CACHE_DIR": str(fx_root / ".zig-cache"),
        }
        machine_environment = {
            **base,
            "CARGO_HOME": str(scratch / "cargo-home"),
            "CARGO_INCREMENTAL": "0",
            "CARGO_TARGET_DIR": str(scratch / "machine-target"),
            "RUSTUP_HOME": "/toolchains/rustup",
        }
        tool_environment = {
            **base,
            "CARGO_HOME": str(scratch / "cargo-home"),
            "RUSTUP_HOME": "/toolchains/rustup",
        }
        tools = {
            "git": {
                "command": ["/usr/bin/git", "--version"],
                "executable": "/usr/bin/git",
                "canonical_executable": "/usr/bin/git",
                "sha256": "a" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 1,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "invocation_mode": 0o755,
                "invocation_device": 1,
                "invocation_inode": 1,
                "invocation_mtime_ns": 1,
                "invocation_ctime_ns": 1,
                "invocation_link_target": "",
                "version": "git version 2",
            },
            "zig": {
                "command": ["/usr/bin/zig", "version"],
                "executable": "/usr/bin/zig",
                "canonical_executable": "/usr/bin/zig",
                "sha256": "b" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 2,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "invocation_mode": 0o755,
                "invocation_device": 1,
                "invocation_inode": 2,
                "invocation_mtime_ns": 1,
                "invocation_ctime_ns": 1,
                "invocation_link_target": "",
                "required_version": EXPECTED_ZIG_VERSION,
                "version": EXPECTED_ZIG_VERSION,
            },
            "rustc": {
                "command": ["/usr/bin/rustc", "+1.94.1", "--version"],
                "executable": "/usr/bin/rustc",
                "canonical_executable": "/usr/bin/rustc",
                "sha256": "c" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 3,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "invocation_mode": 0o755,
                "invocation_device": 1,
                "invocation_inode": 3,
                "invocation_mtime_ns": 1,
                "invocation_ctime_ns": 1,
                "invocation_link_target": "",
                "required_version": EXPECTED_RUST_VERSION,
                "version": "rustc 1.94.1 (e408947bf 2026-03-25)",
            },
            "cargo": {
                "command": ["/usr/bin/cargo", "+1.94.1", "--version"],
                "executable": "/usr/bin/cargo",
                "canonical_executable": "/usr/bin/cargo",
                "sha256": "d" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 4,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "invocation_mode": 0o755,
                "invocation_device": 1,
                "invocation_inode": 4,
                "invocation_mtime_ns": 1,
                "invocation_ctime_ns": 1,
                "invocation_link_target": "",
                "required_version": EXPECTED_RUST_VERSION,
                "version": "cargo 1.94.1 (29ea6fb6a 2026-03-24)",
            },
        }
        plan = command_plan(
            root,
            fx_root,
            lock,
            git=tools["git"]["executable"],
            zig=tools["zig"]["executable"],
            cargo=tools["cargo"]["executable"],
        )
        preparation = [
            self.command_record(plan[name], git_environment, 5.0, root)
            for name in ("clone", "fetch", "checkout")
        ]
        entries = [
            {
                "path": "source.txt",
                "mode": "100644",
                "object": "7" * 40,
                "bytes": 6,
                "sha256": "8" * 64,
            }
        ]
        materialization = {
            "method": "git-ls-tree-cat-file",
            "source_dir": str(machine_source),
            "manifest_path": str(machine_manifest),
            "manifest_sha256": canonical_manifest_sha256(entries),
            "git_entries_sha256": canonical_git_entries_sha256(entries),
            "git_tree": git_tree,
            "source_tree_sha256": canonical_manifest_sha256(entries),
            "entries": entries,
            "listing_command": self.command_record(
                machine_tree_command(tools["git"]["executable"], machine_sha),
                git_environment,
                5.0,
                root,
            ),
        }
        samples = [
            {
                "elapsed_ns": value,
                "setup_ns": 1,
                "supervision_ns": 1,
                "cleanup_ns": 1,
                "returncode": 0,
            }
            for value in range(1, 11)
        ]
        fx_build = self.command_record(plan["fx_build"], fx_environment, 10.0, fx_root)
        fx_build.update(
            {
                "project": "fx",
                "profile": "ReleaseSafe",
                "binary": {
                    "path": str(fx_binary),
                    "bytes": 1,
                    "sha256": "2" * 64,
                },
            }
        )
        machine_build = self.command_record(
            plan["machine_god_build"], machine_environment, 10.0, machine_source
        )
        machine_build.update(
            {
                "project": "machine-god",
                "profile": "release",
                "binary": {
                    "path": str(machine_binary),
                    "bytes": 1,
                    "sha256": "3" * 64,
                },
            }
        )
        implementations = []
        for project, binary, environment in (
            ("fx", fx_binary, {**base, "FX_BENCH": "1"}),
            ("machine-god", machine_binary, base),
        ):
            checksum = "2" * 64 if project == "fx" else "3" * 64
            implementations.append(
                {
                    "project": project,
                    "status": "measured",
                    "command": [str(binary)],
                    "cwd": str(machine_source),
                    "environment": environment,
                    "timeout_seconds": 1.0,
                    "warmup": 1,
                    "executable_identity": self.executable_record(binary, 1, checksum),
                    "pinned_executable": {
                        "method": "private-copy",
                        "sha256": checksum,
                        "bytes": 1,
                        "mode": 0o500,
                        "device": 5,
                        "inode": 6,
                        "seals": 0,
                    },
                    "samples": samples,
                    "median_ns": 5,
                    "p95_ns": 10,
                }
            )
        return {
            "schema_version": 2,
            "classification": "bootstrap-infrastructure-only",
            "claim_eligible": False,
            "generated_at_utc": "2026-08-20T00:00:00Z",
            "runner_class": "test-runner-x86_64",
            "timeouts_seconds": {"fetch": 5.0, "build": 10.0, "sample": 1.0},
            "source": {
                "machine_god": {
                    "git_sha": machine_sha,
                    "dirty": False,
                    "repository_root": str(root),
                    "allowed_output_directories": list(ALLOWED_MACHINE_OUTPUTS),
                    "materialization": materialization,
                },
                "fx": {
                    "repository": lock.repository,
                    "locked_commit": lock.commit,
                    "verified_commit": lock.commit,
                    "lock_path": str(lock_path),
                    "lock_sha256": sha256_file(lock_path),
                    "fresh_checkout": True,
                    "hooks_disabled": True,
                    "preparation_commands": preparation,
                },
            },
            "host": {
                "system": "TestOS",
                "release": "1",
                "machine": "test64",
                "python": "3.14",
                "cpu_count": 1,
                "cpu_model": "Test CPU",
                "runner": {
                    "class": "test-runner-x86_64",
                    "github_actions": False,
                    "image_os": "test-image",
                    "image_version": "1",
                    "runner_os": "TestOS",
                    "runner_arch": "test64",
                },
            },
            "tools": tools,
            "tool_environment": tool_environment,
            "builds": [fx_build, machine_build],
            "environment_policy": {
                "inherits_parent_environment": False,
                "allowlisted_environment_only": True,
            },
            "workloads": [
                {
                    "id": "bootstrap-exit",
                    "description": BOOTSTRAP_DESCRIPTION,
                    "equivalence": "non-equivalent",
                    "claim_eligible": False,
                    "reason": BOOTSTRAP_REASON,
                    "implementations": implementations,
                },
                *unavailable_workloads(fx_binary, machine_binary),
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
        hardened_prefix = [
            "git",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "protocol.ext.allow=never",
        ]
        self.assertEqual(plan["clone"][:7], hardened_prefix)
        self.assertEqual(plan["fetch"][:7], hardened_prefix)
        self.assertEqual(plan["checkout"][:7], hardened_prefix)
        self.assertEqual(plan["clone"][-2:], [lock.repository, "/repo/.bench/fx"])
        self.assertEqual(plan["fetch"][-2:], ["origin", lock.commit])
        self.assertEqual(plan["checkout"][-2:], ["--detach", lock.commit])
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
        lock_path = ROOT / "benchmarks/upstream.lock"
        validate_upstream_evidence(
            self.valid_upstream_evidence(),
            expected_lock=parse_upstream_lock(lock_path),
            expected_lock_sha256=sha256_file(lock_path),
        )

    def test_accepts_canonical_generated_at_utc_boundaries(self) -> None:
        timestamps = (
            "0001-01-01T00:00:00Z",
            "2026-08-20T00:00:00.000001Z",
            "9999-12-31T23:59:59.999999Z",
        )
        for timestamp in timestamps:
            with self.subTest(timestamp=timestamp):
                evidence = self.valid_upstream_evidence()
                evidence["generated_at_utc"] = timestamp
                validate_upstream_evidence(evidence)

    def test_rejects_noncanonical_generated_at_utc(self) -> None:
        timestamps = (
            "machine-god is 100x faster",
            "2026-08-20 00:00:00Z",
            "2026-08-20T00:00:00+00:00",
            "2026-08-20T04:00:00+04:00",
            "2026-08-20T00:00:00.1Z",
            "2026-08-20T00:00:00.000000Z",
            "2026-02-30T00:00:00Z",
            "2026-08-20T00:00:00Z machine-god won",
        )
        for timestamp in timestamps:
            with self.subTest(timestamp=timestamp):
                evidence = self.valid_upstream_evidence()
                evidence["generated_at_utc"] = timestamp
                with self.assertRaisesRegex(ValueError, "canonical UTC timestamp"):
                    validate_upstream_evidence(evidence)

    def test_executable_identity_runtime_error_is_controlled_validation_error(self) -> None:
        evidence = self.valid_upstream_evidence()
        expected_binaries = {
            "fx": Path("/tmp/fx"),
            "machine-god": Path("/tmp/machine-god"),
        }
        with (
            mock.patch.object(upstream, "validate_binary_file"),
            mock.patch.object(
                upstream,
                "executable_identity",
                side_effect=RuntimeError("identity changed while inspected"),
            ),
            self.assertRaisesRegex(
                ValueError, "fx executable identity is unreadable"
            ) as raised,
        ):
            validate_upstream_evidence(
                evidence,
                expected_binaries=expected_binaries,
            )

        self.assertIsNone(raised.exception.__cause__)

    def test_rejects_false_comparison_claim(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["workloads"][0]["equivalence"] = "equivalent"
        evidence["workloads"][0]["claim_eligible"] = True
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_undeclared_claim_and_measurement_fields(self) -> None:
        mutations = (
            lambda data: data.__setitem__(
                "performance_claim", "machine-god is 100x faster"
            ),
            lambda data: data["workloads"][0].__setitem__(
                "comparison", {"claim_eligible": True}
            ),
            lambda data: data["workloads"][0]["implementations"][0].__setitem__(
                "winner", "fx"
            ),
            lambda data: data["workloads"][1].__setitem__("samples", []),
            lambda data: data["workloads"][1].__setitem__("median_ns", 1),
            lambda data: data["workloads"][2].__setitem__(
                "result", {"winner": "machine-god", "speedup": 100}
            ),
            lambda data: data["workloads"][3].__setitem__(
                "machine_god_available", True
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_claim_bearing_workload_prose(self) -> None:
        mutations = (
            lambda data: data["workloads"][0].__setitem__(
                "description", "machine-god is 100x faster than fx"
            ),
            lambda data: data["workloads"][0].__setitem__(
                "reason", "these measurements prove a product performance win"
            ),
            lambda data: data["workloads"][1].__setitem__(
                "description", "equivalent help benchmark"
            ),
            lambda data: data["workloads"][2].__setitem__(
                "reason", "machine-god status measured at 1 ns"
            ),
            lambda data: data["workloads"][1]["implementations"][1].__setitem__(
                "reason", "machine-god won this benchmark"
            ),
            lambda data: data["workloads"][3]["implementations"][0].__setitem__(
                "reason", "fx is slower"
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_undeclared_nested_evidence_fields(self) -> None:
        mutations = (
            lambda data: data["source"].__setitem__("performance_claim", "100x"),
            lambda data: data["source"]["machine_god"].__setitem__(
                "performance_claim", "100x"
            ),
            lambda data: data["source"]["machine_god"]["materialization"].__setitem__(
                "winner", "machine-god"
            ),
            lambda data: data["source"]["machine_god"]["materialization"][
                "entries"
            ][0].__setitem__("result", "faster"),
            lambda data: data["source"]["machine_god"]["materialization"][
                "listing_command"
            ].__setitem__("median_ns", 1),
            lambda data: data["source"]["fx"].__setitem__("equivalent", True),
            lambda data: data["source"]["fx"]["preparation_commands"][0].__setitem__(
                "winner", "machine-god"
            ),
            lambda data: data["host"].__setitem__("performance_claim", "100x"),
            lambda data: data["host"]["runner"].__setitem__("result", "faster"),
            lambda data: data["tools"].__setitem__("winner", "machine-god"),
            lambda data: data["tools"]["cargo"].__setitem__(
                "performance_claim", "100x"
            ),
            lambda data: data["builds"][0].__setitem__("winner", "fx"),
            lambda data: data["builds"][0]["binary"].__setitem__(
                "result", "faster"
            ),
            lambda data: data["workloads"][0]["implementations"][0][
                "executable_identity"
            ].__setitem__("performance_claim", "100x"),
            lambda data: data["workloads"][0]["implementations"][0][
                "pinned_executable"
            ].__setitem__("winner", "fx"),
            lambda data: data["workloads"][0]["implementations"][0]["samples"][
                0
            ].__setitem__("comparison", {"winner": "machine-god"}),
            lambda data: data["workloads"][0]["implementations"][0]["samples"][
                0
            ].__setitem__("median_ns", 1),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_malformed_tools_and_claim_bearing_version_suffixes(self) -> None:
        mutations = (
            lambda data: data.__setitem__("tools", []),
            lambda data: data["tools"]["git"].__setitem__(
                "version", "git version 2 machine-god is faster"
            ),
            lambda data: data["tools"]["rustc"].__setitem__(
                "version", "rustc 1.94.1 (e408947bf 2026-03-25) machine-god is faster"
            ),
            lambda data: data["tools"]["cargo"].__setitem__(
                "version", "cargo 1.94.1 (29ea6fb6a 2026-03-24) winner=machine-god"
            ),
        )
        canonical = parse_upstream_lock(ROOT / "benchmarks/upstream.lock")
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence, expected_lock=canonical)

    def test_every_malformed_scalar_type_is_a_controlled_validation_error(self) -> None:
        def scalar_paths(value: object, path: tuple[object, ...] = ()):
            if isinstance(value, dict):
                for key, child in value.items():
                    yield from scalar_paths(child, (*path, key))
            elif isinstance(value, list):
                for index, child in enumerate(value):
                    yield from scalar_paths(child, (*path, index))
            else:
                yield path

        template = self.valid_upstream_evidence()
        paths = list(scalar_paths(template))
        self.assertGreater(len(paths), 100)
        for path in paths:
            with self.subTest(path=path):
                evidence = self.valid_upstream_evidence()
                target: object = evidence
                for part in path[:-1]:
                    target = target[part]  # type: ignore[index]
                target[path[-1]] = {}  # type: ignore[index]
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_every_scalar_rejects_all_alternate_json_types(self) -> None:
        def scalar_paths(value: object, path: tuple[object, ...] = ()):
            if isinstance(value, dict):
                for key, child in value.items():
                    yield from scalar_paths(child, (*path, key))
            elif isinstance(value, list):
                for index, child in enumerate(value):
                    yield from scalar_paths(child, (*path, index))
            else:
                yield path, value

        template = self.valid_upstream_evidence()
        paths = list(scalar_paths(template))
        self.assertGreater(len(paths), 100)
        for path, original in paths:
            if isinstance(original, bool):
                replacements: tuple[object, ...] = (None, 0, 1, 0.0, 1.0, "", [], {})
            elif isinstance(original, str):
                replacements = (None, False, True, 0, 0.0, [], {})
            elif isinstance(original, int):
                replacements = (None, False, True, float(original), "", [], {})
            elif isinstance(original, float):
                replacements = (None, False, True, "", [], {})
            else:
                replacements = ({},)
            for replacement in replacements:
                with self.subTest(path=path, replacement=replacement):
                    evidence = self.valid_upstream_evidence()
                    target: object = evidence
                    for part in path[:-1]:
                        target = target[part]  # type: ignore[index]
                    target[path[-1]] = replacement  # type: ignore[index]
                    with self.assertRaises(ValueError):
                        validate_upstream_evidence(evidence)

    def test_environment_policy_rejects_integer_boolean_lookalikes(self) -> None:
        for field, value in (
            ("inherits_parent_environment", 0),
            ("allowlisted_environment_only", 1),
        ):
            with self.subTest(field=field):
                evidence = self.valid_upstream_evidence()
                evidence["environment_policy"][field] = value
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_measurement_numbers_reject_boolean_lookalikes(self) -> None:
        for implementation_index in (0, 1):
            evidence = self.valid_upstream_evidence()
            measurement = evidence["workloads"][0]["implementations"][
                implementation_index
            ]
            measurement["timeout_seconds"] = True
            with self.subTest(implementation=implementation_index, field="timeout"):
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_huge_numbers_use_controlled_exact_validation(self) -> None:
        for timeout_name in ("fetch", "build", "sample"):
            evidence = self.valid_upstream_evidence()
            evidence["timeouts_seconds"][timeout_name] = 10**4000
            with self.subTest(timeout=timeout_name):
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

        evidence = self.valid_upstream_evidence()
        huge = 10**4000
        for measurement in evidence["workloads"][0]["implementations"]:
            for sample in measurement["samples"]:
                sample["elapsed_ns"] = huge
            measurement["median_ns"] = huge
            measurement["p95_ns"] = huge
        validate_upstream_evidence(evidence)

        for aggregate in ("median_ns", "p95_ns"):
            evidence = self.valid_upstream_evidence()
            measurement = evidence["workloads"][0]["implementations"][0]
            for sample in measurement["samples"]:
                sample["elapsed_ns"] = 1
            measurement["median_ns"] = 1
            measurement["p95_ns"] = 1
            measurement[aggregate] = True
            with self.subTest(aggregate=aggregate):
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_records_implemented_local_commands_without_measurements(self) -> None:
        evidence = self.valid_upstream_evidence()
        machine_binary = evidence["builds"][1]["binary"]["path"]
        for workload, command in (
            (evidence["workloads"][1], [machine_binary, "help"]),
            (
                evidence["workloads"][2],
                [machine_binary, "status", "--json"],
            ),
        ):
            self.assertEqual(workload["equivalence"], "non-equivalent")
            self.assertIs(workload["claim_eligible"], False)
            self.assertEqual(
                [item["status"] for item in workload["implementations"]],
                ["not-measured", "not-measured"],
            )
            self.assertEqual(workload["implementations"][1]["command"], command)
            self.assertNotIn("samples", workload["implementations"][0])
            self.assertNotIn("samples", workload["implementations"][1])

    def test_rejects_implemented_workload_schema_drift(self) -> None:
        mutations = (
            lambda data: data["workloads"][1].__setitem__(
                "equivalence", "unimplemented"
            ),
            lambda data: data["workloads"][1].__setitem__("claim_eligible", True),
            lambda data: data["workloads"][1]["implementations"][0].__setitem__(
                "status", "measured"
            ),
            lambda data: data["workloads"][1]["implementations"][1].__setitem__(
                "command", ["machine-god", "--help"]
            ),
            lambda data: data["workloads"][2]["implementations"][0].__setitem__(
                "samples", []
            ),
            lambda data: data["workloads"][2]["implementations"][1].__setitem__(
                "median_ns", 1
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_unimplemented_workload_schema_drift(self) -> None:
        mutations = (
            lambda data: data["workloads"][3].__setitem__(
                "equivalence", "non-equivalent"
            ),
            lambda data: data["workloads"][3].__setitem__("claim_eligible", True),
            lambda data: data["workloads"][4]["implementations"][0].__setitem__(
                "command", ["fx", "session", "--json"]
            ),
            lambda data: data["workloads"][4]["implementations"][1].__setitem__(
                "status", "not-measured"
            ),
            lambda data: data["workloads"][5]["implementations"][1].__setitem__(
                "command", ["machine-god", "background", "--json"]
            ),
            lambda data: data["workloads"][5]["implementations"][0].__setitem__(
                "samples", []
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_unverified_upstream_commit(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["source"]["fx"]["verified_commit"] = "5" * 40
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_noncanonical_lock_repository_or_commit(self) -> None:
        lock_path = ROOT / "benchmarks/upstream.lock"
        canonical = parse_upstream_lock(lock_path)
        for field, value in (
            ("repository", "https://example.test/hostile.git"),
            ("locked_commit", "5" * 40),
        ):
            with self.subTest(field=field):
                evidence = self.valid_upstream_evidence()
                evidence["source"]["fx"][field] = value
                if field == "locked_commit":
                    evidence["source"]["fx"]["verified_commit"] = value
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence, expected_lock=canonical)

    def test_rejects_altered_profile_build_path_or_measurement_command(self) -> None:
        mutations = (
            lambda data: data["builds"][0].__setitem__("profile", "ReleaseFast"),
            lambda data: data["builds"][1].__setitem__("command", ["true"]),
            lambda data: data["tools"]["zig"].__setitem__("command", ["/usr/bin/true"]),
            lambda data: data["builds"][0]["binary"].__setitem__("path", "/tmp/fx"),
            lambda data: data["workloads"][0]["implementations"][1].__setitem__(
                "command", ["/tmp/machine-god"]
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_hostile_build_environment_and_runner_identity(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["builds"][1]["environment"]["RUSTFLAGS"] = "-C target-cpu=native"
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)
        evidence = self.valid_upstream_evidence()
        evidence["host"]["runner"]["class"] = "different-runner"
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_substituted_toolchain_and_scratch_cache_paths(self) -> None:
        mutations = (
            lambda data: data["builds"][1]["environment"].__setitem__(
                "CARGO_HOME", "/attacker/cargo"
            ),
            lambda data: data["builds"][1]["environment"].__setitem__(
                "RUSTUP_HOME", "/attacker/rustup"
            ),
            lambda data: data["builds"][1]["environment"].__setitem__(
                "CARGO_TARGET_DIR", "/attacker/target"
            ),
            lambda data: data["builds"][0]["environment"].__setitem__(
                "ZIG_GLOBAL_CACHE_DIR", "/attacker/zig"
            ),
            lambda data: data["tool_environment"].__setitem__(
                "CARGO_HOME", "/attacker/cargo"
            ),
            lambda data: data["source"]["machine_god"]["materialization"].__setitem__(
                "source_dir", "/attacker/source"
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_rejects_aggregate_not_derived_from_raw_samples(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["workloads"][0]["implementations"][0]["p95_ns"] = 9
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_rejects_missing_or_invalid_outside_timing_durations(self) -> None:
        mutations = (
            lambda data: data["builds"][0].pop("setup_ns"),
            lambda data: data["builds"][1].__setitem__("cleanup_ns", -1),
            lambda data: data["workloads"][0]["implementations"][0]["samples"][
                0
            ].__setitem__("supervision_ns", True),
            lambda data: data["workloads"][0]["implementations"][0][
                "pinned_executable"
            ].__setitem__("sha256", "f" * 64),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                evidence = self.valid_upstream_evidence()
                mutate(evidence)
                with self.assertRaises(ValueError):
                    validate_upstream_evidence(evidence)

    def test_schema_two_checker_binds_sha_and_both_actual_binaries(self) -> None:
        (ROOT / ".bench").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / ".bench") as directory:
            temporary = Path(directory)
            machine_sha = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
            ).strip()
            git_tree = subprocess.check_output(
                ["git", "rev-parse", f"{machine_sha}^{{tree}}"], cwd=ROOT, text=True
            ).strip()
            evidence = self.valid_upstream_evidence(
                ROOT,
                fx_root=temporary / "fx",
                scratch=temporary / "scratch",
                machine_sha=machine_sha,
                git_tree=git_tree,
            )
            scratch = temporary / "scratch"
            (scratch / "home").mkdir(parents=True)
            (scratch / "tmp").mkdir()
            git_environment = {
                **self.base_environment(scratch),
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
            }
            git = invocation_path("git", os.environ["PATH"])
            python = str(Path(sys.executable).resolve())
            identities = {
                "git": executable_identity(Path(git)),
                "zig": executable_identity(Path(python)),
                "rustc": executable_identity(Path(python)),
                "cargo": executable_identity(Path(python)),
            }
            version_commands = {
                "git": [git, "--version"],
                "zig": [python, "version"],
                "rustc": [python, "+1.94.1", "--version"],
                "cargo": [python, "+1.94.1", "--version"],
            }
            for name, identity in identities.items():
                evidence["tools"][name].update(identity)
                evidence["tools"][name]["command"] = version_commands[name]
            lock = parse_upstream_lock(ROOT / "benchmarks/upstream.lock")
            plan = command_plan(
                ROOT,
                temporary / "fx",
                lock,
                git=git,
                zig=python,
                cargo=python,
            )
            evidence["source"]["fx"]["preparation_commands"] = [
                self.command_record(plan[name], git_environment, 5.0, ROOT)
                for name in ("clone", "fetch", "checkout")
            ]
            evidence["builds"][0]["command"] = plan["fx_build"]
            evidence["builds"][1]["command"] = plan["machine_god_build"]
            fx_binary = temporary / "fx/zig-out/bin/fx"
            machine_binary = temporary / "scratch/machine-target/release/machine-god"
            for binary, index in ((fx_binary, 0), (machine_binary, 1)):
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(f"binary-{index}".encode())
                binary.chmod(0o755)
                evidence["builds"][index]["binary"] = {
                    "path": str(binary),
                    "bytes": binary.stat().st_size,
                    "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                }
                identity = executable_identity(binary)
                measurement = evidence["workloads"][0]["implementations"][index]
                measurement["executable_identity"] = identity
                measurement["pinned_executable"].update(
                    {
                        "sha256": identity["sha256"],
                        "bytes": identity["bytes"],
                    }
                )
            machine_source = scratch / "machine-source"
            materialization = materialize_machine_source(
                ROOT,
                machine_source,
                scratch / "machine-source-manifest.json",
                machine_sha,
                git,
                environment=git_environment,
                timeout_seconds=5.0,
                expected_executable=identities["git"],
            )
            evidence["source"]["machine_god"]["materialization"] = materialization
            evidence_path = temporary / "upstream.json"
            evidence_path.write_text(
                json.dumps(evidence), encoding="utf-8"
            )
            command = [
                sys.executable,
                str(ROOT / "benchmarks/check.py"),
                str(evidence_path),
                "--expected-git-sha",
                machine_sha,
                "--expected-runner-class",
                "test-runner-x86_64",
                "--fx-binary",
                str(fx_binary),
                "--machine-god-binary",
                str(machine_binary),
            ]
            completed = subprocess.run(
                command, check=False, capture_output=True, text=True
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            checker_arguments = command[1:]
            runtime_error_driver = f"""
import runpy
import sys
from pathlib import Path
import upstream

actual_executable_identity = upstream.executable_identity
fx_binary = Path({str(fx_binary)!r}).resolve()

def fail_fx_identity(path):
    if Path(path).resolve() == fx_binary:
        raise RuntimeError("identity changed while inspected")
    return actual_executable_identity(path)

upstream.executable_identity = fail_fx_identity
sys.argv = {checker_arguments!r}
runpy.run_path(sys.argv[0], run_name="__main__")
"""
            controlled_identity_failure = subprocess.run(
                [sys.executable, "-c", runtime_error_driver],
                cwd=ROOT,
                env={**os.environ, "PYTHONPATH": str(ROOT / "benchmarks")},
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(controlled_identity_failure.returncode, 0)
            self.assertEqual(
                controlled_identity_failure.stderr,
                "fx executable identity is unreadable\n",
            )
            self.assertNotIn("Traceback", controlled_identity_failure.stderr)
            machine_binary.write_bytes(b"tampered")
            tampered = subprocess.run(
                command, check=False, capture_output=True, text=True
            )
        self.assertNotEqual(tampered.returncode, 0)

    def test_materialized_commit_isolated_from_worktree_mutation_during_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(
                ["git", "-C", str(root), "config", "user.email", "test@example.test"],
                check=True,
            )
            source_input = root / "input.txt"
            source_input.write_text("recorded", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "input.txt"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "source"], check=True)
            commit = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            scratch = root / ".bench"
            scratch.mkdir()
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "d" * 32
            if sys.platform.startswith("linux"):
                linux_containment_preflight()
            environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
            environment["GIT_CONFIG_NOSYSTEM"] = "1"
            environment["GIT_NO_REPLACE_OBJECTS"] = "1"
            environment["GIT_TERMINAL_PROMPT"] = "0"
            materialization = materialize_machine_source(
                root,
                scratch / "machine-source",
                scratch / "machine-source-manifest.json",
                commit,
                "git",
                environment=environment,
                timeout_seconds=2.0,
            )
            mutator = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import pathlib,time; time.sleep(0.05); "
                    f"pathlib.Path({str(source_input)!r}).write_text('hostile')",
                ]
            )
            completed = run_process(
                [
                    sys.executable,
                    "-c",
                    "import pathlib,time; time.sleep(0.2); "
                    "print(pathlib.Path('input.txt').read_text())",
                ],
                cwd=scratch / "machine-source",
                environment=environment,
                timeout_seconds=2.0,
            )
            mutator.wait(timeout=2)
            self.assertEqual(completed.stdout.decode().strip(), "recorded")
            self.assertEqual(
                source_tree_sha256(scratch / "machine-source"),
                materialization["source_tree_sha256"],
            )

    def test_materialization_ignores_export_attributes_and_rejects_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(
                ["git", "-C", str(root), "config", "user.email", "test@example.test"],
                check=True,
            )
            (root / ".gitattributes").write_text(
                "ignored.txt export-ignore\nsubstituted.txt export-subst\n",
                encoding="utf-8",
            )
            (root / "ignored.txt").write_text("must remain", encoding="utf-8")
            substitution = "$Format:%H$\n"
            (root / "substituted.txt").write_text(substitution, encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "attributes"], check=True)
            commit = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            scratch = root / ".bench"
            scratch.mkdir()
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "f" * 32
            materialization = materialize_machine_source(
                root,
                scratch / "machine-source",
                scratch / "machine-source-manifest.json",
                commit,
                "git",
                environment=environment,
                timeout_seconds=2.0,
            )
            self.assertEqual(
                (scratch / "machine-source/ignored.txt").read_text(encoding="utf-8"),
                "must remain",
            )
            self.assertEqual(
                (scratch / "machine-source/substituted.txt").read_text(encoding="utf-8"),
                substitution,
            )
            self.assertEqual(materialization["method"], "git-ls-tree-cat-file")

            link = root / "link"
            link.symlink_to("ignored.txt")
            subprocess.run(["git", "-C", str(root), "add", "link"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "link"], check=True)
            link_commit = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            with self.assertRaisesRegex(RuntimeError, "unsupported mode/type"):
                materialize_machine_source(
                    root,
                    scratch / "linked-source",
                    scratch / "linked-manifest.json",
                    link_commit,
                    "git",
                    environment=environment,
                    timeout_seconds=2.0,
                )

    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "detached descendant containment is enforced by Linux /proc",
    )
    def test_process_timeout_terminates_child_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "child-finished"
            parent = "\n".join(
                (
                    "import os, pathlib, time",
                    "first = os.fork()",
                    "if first == 0:",
                    "    os.setsid()",
                    "    second = os.fork()",
                    "    if second == 0:",
                    "        os.environ.clear()",
                    "        time.sleep(0.4)",
                    f"        pathlib.Path({str(marker)!r}).write_text('bad')",
                    "        time.sleep(5)",
                    "    os._exit(0)",
                    "time.sleep(5)",
                )
            )
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "b" * 32
            with self.assertRaises(ProcessTimeout):
                run_process(
                    [sys.executable, "-c", parent],
                    cwd=Path(directory),
                    environment=environment,
                    timeout_seconds=0.1,
                )
            time.sleep(0.5)
            self.assertFalse(marker.exists())

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux containment regression")
    def test_linux_containment_does_not_read_descendant_environments(self) -> None:
        linux_containment_preflight()
        environment = os.environ.copy()
        environment[CONTAINMENT_ENVIRONMENT_KEY] = "1" * 32
        original_read_bytes = Path.read_bytes

        def reject_environ(path: Path) -> bytes:
            if path.name == "environ":
                raise PermissionError("hostile proc mount")
            return original_read_bytes(path)

        with mock.patch.object(Path, "read_bytes", reject_environ):
            completed = run_process(
                [sys.executable, "-c", "pass"],
                cwd=Path.cwd(),
                environment=environment,
                timeout_seconds=1.0,
            )
        self.assertEqual(completed.returncode, 0)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux containment regression")
    def test_success_reaps_short_lived_detached_grandchild(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "grandchild.pid"
            command = "\n".join(
                (
                    "import os, pathlib",
                    "first = os.fork()",
                    "if first == 0:",
                    "    os.setsid()",
                    "    second = os.fork()",
                    "    if second == 0:",
                    f"        pathlib.Path({str(marker)!r}).write_text(str(os.getpid()))",
                    "        os.close(1)",
                    "        os.close(2)",
                    "        os._exit(0)",
                    "    os._exit(0)",
                    "os._exit(0)",
                )
            )
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "6" * 32
            completed = run_process(
                [sys.executable, "-c", command],
                cwd=Path(directory),
                environment=environment,
                timeout_seconds=1.0,
            )
            self.assertEqual(completed.returncode, 0)
            adopted_pid = int(marker.read_text(encoding="utf-8"))
            self.assertFalse(Path(f"/proc/{adopted_pid}").exists())
            with self.assertRaises(ChildProcessError):
                os.waitpid(adopted_pid, os.WNOHANG)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux containment regression")
    def test_supervisor_constructor_failure_launches_no_process(self) -> None:
        linux_containment_preflight()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "launched"
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "7" * 32
            with (
                mock.patch.object(
                    LinuxProcessSupervisor,
                    "__init__",
                    side_effect=RuntimeError("injected constructor failure"),
                ),
                self.assertRaisesRegex(RuntimeError, "constructor failure"),
            ):
                run_process(
                    [
                        sys.executable,
                        "-c",
                        f"import pathlib; pathlib.Path({str(marker)!r}).write_text('bad')",
                    ],
                    cwd=Path(directory),
                    environment=environment,
                    timeout_seconds=1.0,
                )
            self.assertFalse(marker.exists())

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux containment regression")
    def test_attach_and_finalizer_failures_cleanup_detached_descendants(self) -> None:
        linux_containment_preflight()
        for failure in ("attach", "finalizer"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                marker = Path(directory) / "leaked"
                command = "\n".join(
                    (
                        "import os, pathlib, time",
                        "first = os.fork()",
                        "if first == 0:",
                        "    os.setsid()",
                        "    second = os.fork()",
                        "    if second == 0:",
                        "        os.close(1)",
                        "        os.close(2)",
                        "        time.sleep(0.4)",
                        f"        pathlib.Path({str(marker)!r}).write_text('bad')",
                        "        os._exit(0)",
                        "    os._exit(0)",
                        "time.sleep(0.05)",
                    )
                )
                environment = os.environ.copy()
                environment[CONTAINMENT_ENVIRONMENT_KEY] = "8" * 32

                def failed_attach(
                    supervisor: LinuxProcessSupervisor,
                    root_pid: int,
                    descriptor: int | None = None,
                ) -> None:
                    del supervisor, root_pid, descriptor
                    time.sleep(0.1)
                    raise RuntimeError("injected attach failure")

                patcher = (
                    mock.patch.object(LinuxProcessSupervisor, "attach_root", failed_attach)
                    if failure == "attach"
                    else mock.patch.object(
                        upstream,
                        "finalize_successful_process",
                        side_effect=RuntimeError("injected finalizer failure"),
                    )
                )
                expected_error = "attach" if failure == "attach" else "finalizer"
                with patcher, self.assertRaisesRegex(RuntimeError, expected_error):
                    run_process(
                        [sys.executable, "-c", command],
                        cwd=Path(directory),
                        environment=environment,
                        timeout_seconds=1.0,
                    )
                time.sleep(0.5)
                self.assertFalse(marker.exists())

    def test_pid_reuse_does_not_expand_recorded_ancestry(self) -> None:
        supervisor = object.__new__(LinuxProcessSupervisor)
        supervisor.root_pid = 100
        supervisor.root_identity = (100, 10)
        supervisor.owner_pid = 999
        supervisor.baseline_children = set()
        supervisor._known = {(100, 10): 1, (200, 20): 2}
        supervisor._lock = upstream.threading.Lock()
        table = {
            100: LinuxProcessInfo(100, 1, "S", 11),
            101: LinuxProcessInfo(101, 100, "S", 30),
            200: LinuxProcessInfo(200, 1, "S", 21),
            201: LinuxProcessInfo(201, 200, "S", 40),
        }
        with (
            mock.patch.object(upstream, "linux_process_table", return_value=table),
            mock.patch.object(supervisor, "_open_pidfd") as open_pidfd,
        ):
            supervisor.refresh()
        open_pidfd.assert_not_called()
        self.assertEqual(set(supervisor._known), {(100, 10), (200, 20)})

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux containment regression")
    def test_linux_containment_preflight_fails_closed_without_proc(self) -> None:
        environment = os.environ.copy()
        environment[CONTAINMENT_ENVIRONMENT_KEY] = "2" * 32
        with (
            mock.patch.object(upstream, "_LINUX_PREFLIGHT_COMPLETE", False),
            mock.patch.object(
                upstream,
                "linux_process_table",
                side_effect=RuntimeError("unreadable proc"),
            ),
            self.assertRaisesRegex(RuntimeError, "unreadable proc"),
        ):
            run_process(
                [sys.executable, "-c", "pass"],
                cwd=Path.cwd(),
                environment=environment,
                timeout_seconds=1.0,
            )

    def test_elapsed_time_excludes_delayed_containment_scan(self) -> None:
        environment = os.environ.copy()
        environment[CONTAINMENT_ENVIRONMENT_KEY] = "3" * 32

        def delayed_finalize(process: object, supervisor: object) -> None:
            time.sleep(0.2)
            finalize_successful_process(process, supervisor)

        started = time.monotonic_ns()
        with mock.patch.object(
            upstream, "finalize_successful_process", side_effect=delayed_finalize
        ):
            completed = run_process(
                [sys.executable, "-c", "pass"],
                cwd=Path.cwd(),
                environment=environment,
                timeout_seconds=1.0,
            )
        wall_elapsed = time.monotonic_ns() - started
        self.assertLess(completed.elapsed_ns, 150_000_000)
        self.assertGreater(wall_elapsed, 190_000_000)
        self.assertGreater(completed.cleanup_ns, 190_000_000)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux timing regression")
    def test_measurement_excludes_delayed_attach_and_descendant_scan(self) -> None:
        linux_containment_preflight()
        environment = os.environ.copy()
        environment[CONTAINMENT_ENVIRONMENT_KEY] = "5" * 32
        original_attach = LinuxProcessSupervisor.attach_root
        original_settle = LinuxProcessSupervisor.settle_and_reap_adopted

        def delayed_attach(
            supervisor: LinuxProcessSupervisor,
            root_pid: int,
            descriptor: int | None = None,
        ) -> None:
            time.sleep(0.2)
            original_attach(supervisor, root_pid, descriptor)

        def delayed_settle(
            supervisor: LinuxProcessSupervisor,
            settle_seconds: float = 0.25,
        ) -> set[int]:
            time.sleep(0.2)
            return original_settle(supervisor, settle_seconds)

        started = time.monotonic_ns()
        with (
            mock.patch.object(LinuxProcessSupervisor, "attach_root", delayed_attach),
            mock.patch.object(
                LinuxProcessSupervisor, "settle_and_reap_adopted", delayed_settle
            ),
        ):
            completed = run_process(
                [sys.executable, "-c", "pass"],
                cwd=Path.cwd(),
                environment=environment,
                timeout_seconds=1.0,
                capture_output=False,
            )
        wall_elapsed = time.monotonic_ns() - started

        self.assertLess(completed.elapsed_ns, 150_000_000)
        self.assertGreater(completed.supervision_ns, 190_000_000)
        self.assertGreater(completed.cleanup_ns, 190_000_000)
        self.assertGreater(wall_elapsed, 390_000_000)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux timing regression")
    def test_gated_submillisecond_samples_wait_for_observer_registration(self) -> None:
        linux_containment_preflight()
        environment = os.environ.copy()
        environment[CONTAINMENT_ENVIRONMENT_KEY] = "c" * 32
        original_register = upstream.LinuxExitObserver._register_pidfd
        registrations = 0

        def delayed_registration(
            observer: upstream.LinuxExitObserver,
            poller: object,
            descriptor: int,
        ) -> None:
            nonlocal registrations
            registrations += 1
            time.sleep(0.05)
            original_register(observer, poller, descriptor)

        executable = invocation_path("true", os.environ["PATH"])
        identity = executable_identity(Path(executable))
        with mock.patch.object(
            upstream.LinuxExitObserver,
            "_register_pidfd",
            delayed_registration,
        ):
            measurement = run_measurement(
                "prearmed-true",
                [executable],
                Path.cwd(),
                environment,
                warmup=1,
                runs=10,
                timeout_seconds=1.0,
                expected_executable=identity,
            )

        self.assertEqual(registrations, 11)
        self.assertEqual(len(measurement["samples"]), 10)
        self.assertNotIn("discarded_pre_registration_exits", measurement)
        self.assertLess(
            max(sample["elapsed_ns"] for sample in measurement["samples"]),
            50_000_000,
        )

    @unittest.skipUnless(os.name == "posix", "pinned executable regression requires POSIX")
    def test_replacement_during_sample_cannot_change_executed_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            marker = temporary / "result"
            invocation = temporary / "measured-tool"
            if sys.platform.startswith("linux"):
                original = Path(invocation_path("sh", os.environ["PATH"])).resolve()
                replacement = Path(
                    invocation_path("false", os.environ["PATH"])
                ).resolve()
                command = [
                    str(invocation),
                    "-c",
                    f"sleep 0.2; printf good > {marker}",
                ]
            else:
                original = temporary / "original"
                replacement = temporary / "replacement"
                original.write_text(
                    f"#!/bin/sh\nsleep 0.2\nprintf good > {marker}\n",
                    encoding="utf-8",
                )
                replacement.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
                original.chmod(0o755)
                replacement.chmod(0o755)
                command = [str(invocation)]
            invocation.symlink_to(original)
            identity = executable_identity(invocation)
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "d" * 32

            def replace_invocation() -> None:
                time.sleep(0.05)
                invocation.unlink()
                invocation.symlink_to(replacement)

            replacer = threading.Thread(target=replace_invocation)
            replacer.start()
            try:
                with self.assertRaisesRegex(RuntimeError, "identity changed"):
                    run_measurement(
                        "identity-swap",
                        command,
                        temporary,
                        environment,
                        warmup=0,
                        runs=1,
                        timeout_seconds=1.0,
                        expected_executable=identity,
                    )
            finally:
                replacer.join(1.0)
            self.assertFalse(replacer.is_alive())
            self.assertEqual(marker.read_text(encoding="utf-8"), "good")

    @unittest.skipUnless(os.name == "posix", "executable symlink regression requires POSIX")
    def test_tool_identity_survives_symlink_swap_and_detects_target_change(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            good = temporary / "good-tool"
            bad = temporary / "bad-tool"
            link = temporary / "tool"
            good.write_text("#!/bin/sh\nsleep 0.2\nprintf good\n", encoding="utf-8")
            bad.write_text("#!/bin/sh\nprintf bad\n", encoding="utf-8")
            good.chmod(0o755)
            bad.chmod(0o755)
            link.symlink_to(good)
            resolved = invocation_path(str(link), os.environ.get("PATH", ""))
            identity = executable_identity(Path(resolved))
            link.unlink()
            link.symlink_to(bad)
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "4" * 32
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                run_process(
                    [resolved],
                    cwd=temporary,
                    environment=environment,
                    timeout_seconds=1.0,
                    expected_executable=identity,
                )

            identity = executable_identity(good)
            mutator = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import pathlib,time; time.sleep(0.05); "
                    f"pathlib.Path({str(good)!r}).write_text('#!/bin/sh\\nprintf changed\\n')",
                ]
            )
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                run_process(
                    [str(good)],
                    cwd=temporary,
                    environment=environment,
                    timeout_seconds=1.0,
                    expected_executable=identity,
                )
            mutator.wait(timeout=1)

    def test_executable_identity_bounds_growth_and_closes_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "growing-tool"
            executable.write_bytes(b"#!/bin/sh\n")
            executable.chmod(0o755)
            initial_size = executable.stat().st_size
            opened_descriptors: list[int] = []
            read_sizes: list[int] = []
            original_open = os.open
            original_hash = upstream.bounded_sha256_file

            def tracking_open(path: object, flags: int) -> int:
                descriptor = original_open(path, flags)
                opened_descriptors.append(descriptor)
                return descriptor

            def grow_then_hash(source: object, expected_bytes: int) -> str:
                with executable.open("ab") as destination:
                    destination.write(b"growing data that must not be streamed to EOF")

                class CountingSource:
                    def read(self, size: int) -> bytes:
                        read_sizes.append(size)
                        return source.read(size)

                return original_hash(CountingSource(), expected_bytes)

            with (
                mock.patch.object(upstream.os, "open", side_effect=tracking_open),
                mock.patch.object(
                    upstream,
                    "bounded_sha256_file",
                    side_effect=grow_then_hash,
                ),
                self.assertRaisesRegex(RuntimeError, "became longer"),
            ):
                executable_identity(executable)

            self.assertEqual(read_sizes, [initial_size, 1])
            self.assertEqual(len(opened_descriptors), 1)
            with self.assertRaises(OSError):
                os.fstat(opened_descriptors[0])

    @unittest.skipUnless(os.name == "posix", "target replacement requires POSIX")
    def test_executable_identity_rejects_canonical_target_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            invocation = root / "tool"
            target = root / "target"
            displaced = root / "displaced"
            replacement = root / "replacement"
            target.write_bytes(b"#!/bin/sh\n")
            replacement.write_bytes(b"#!/bin/sh\n")
            target.chmod(0o755)
            replacement.chmod(0o755)
            invocation.symlink_to(target)
            original_hash = upstream.bounded_sha256_file

            def hash_then_replace(source: object, expected_bytes: int) -> str:
                checksum = original_hash(source, expected_bytes)
                target.rename(displaced)
                replacement.rename(target)
                return checksum

            with (
                mock.patch.object(
                    upstream,
                    "bounded_sha256_file",
                    side_effect=hash_then_replace,
                ),
                self.assertRaisesRegex(RuntimeError, "target path changed"),
            ):
                executable_identity(invocation)

    def test_executable_identity_rejects_content_change_during_hash(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "changing-tool"
            executable.write_bytes(b"#!/bin/sh\n")
            executable.chmod(0o755)
            original_hash = upstream.bounded_sha256_file

            def hash_then_change(source: object, expected_bytes: int) -> str:
                checksum = original_hash(source, expected_bytes)
                executable.write_bytes(b"#!/bin/zsh")
                return checksum

            with (
                mock.patch.object(
                    upstream,
                    "bounded_sha256_file",
                    side_effect=hash_then_change,
                ),
                self.assertRaisesRegex(RuntimeError, "changed while inspected"),
            ):
                executable_identity(executable)

    @unittest.skipUnless(os.name == "posix", "setsid regression requires POSIX")
    def test_timeout_cleanup_is_bounded_when_detached_child_holds_pipe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pid_path = Path(directory) / "detached.pid"
            child = (
                "import os,pathlib,time; "
                f"pathlib.Path({str(pid_path)!r}).write_text(str(os.getpid())); time.sleep(5)"
            )
            parent = (
                "import subprocess,sys,time; "
                f"subprocess.Popen([sys.executable,'-c',{child!r}],start_new_session=True); "
                "time.sleep(5)"
            )
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "e" * 32
            started = time.monotonic()
            with self.assertRaises(ProcessTimeout):
                run_process(
                    [sys.executable, "-c", parent],
                    cwd=Path(directory),
                    environment=environment,
                    timeout_seconds=0.3,
                )
            self.assertLess(time.monotonic() - started, 3.0)
            if pid_path.exists():
                try:
                    os.kill(int(pid_path.read_text()), signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_fresh_upstream_rejects_preexisting_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            upstream = root / "fx"
            upstream.mkdir()
            with self.assertRaises(RuntimeError):
                prepare_upstream(
                    root,
                    upstream,
                    UpstreamLock("https://example.test/fx.git", "a" * 40, "0.16.0"),
                    {},
                    "git",
                    environment=os.environ,
                    timeout_seconds=1.0,
                )

    def test_machine_cleanliness_rejects_untracked_inputs_but_allows_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(
                ["git", "-C", str(root), "config", "user.email", "test@example.test"],
                check=True,
            )
            tracked = root / "tracked"
            tracked.write_text("ok", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "tracked"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "test"], check=True)
            (root / ".gitignore").write_text("/.bench/\n/target/\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", ".gitignore"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "ignore"], check=True)
            (root / "target").mkdir()
            (root / "target/output").write_text("ok", encoding="utf-8")
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "c" * 32
            check_machine_cleanliness(root, "git", environment, 2.0)
            (root / ".cargo").mkdir()
            (root / ".cargo/config.toml").write_text(
                "[build]\nrustflags=['-Ctarget-cpu=native']\n", encoding="utf-8"
            )
            with self.assertRaises(RuntimeError):
                check_machine_cleanliness(root, "git", environment, 2.0)

    def test_parses_porcelain_z_rename_copy_and_literal_paths(self) -> None:
        status = (
            b"?? outside -> target/input\0"
            b"!! outside\n -> target/ignored\0"
            b"R  target/renamed\0tracked-old\0"
            b"C  target/copied\0tracked-source\0"
        )
        self.assertEqual(
            parse_porcelain_v1_z(status),
            [
                MachineStatusEntry("??", b"outside -> target/input", None),
                MachineStatusEntry("!!", b"outside\n -> target/ignored", None),
                MachineStatusEntry("R ", b"target/renamed", b"tracked-old"),
                MachineStatusEntry("C ", b"target/copied", b"tracked-source"),
            ],
        )
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            parse_porcelain_v1_z(b"R  target/renamed\0")

    def test_machine_cleanliness_rejects_hostile_literal_names_and_renames(self) -> None:
        hostile_cases = (
            ("untracked-arrow", "outside -> target/input", False),
            ("ignored-arrow", "ignored -> target/input", True),
            ("untracked-newline", "outside\n -> target/input", False),
        )
        for label, relative, ignored in hostile_cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                subprocess.run(["git", "init", "-q", str(root)], check=True)
                subprocess.run(
                    ["git", "-C", str(root), "config", "user.name", "Test"], check=True
                )
                subprocess.run(
                    ["git", "-C", str(root), "config", "user.email", "test@example.test"],
                    check=True,
                )
                ignore = f"/{relative.split('/', 1)[0]}/\n" if ignored else ""
                (root / ".gitignore").write_text(ignore, encoding="utf-8")
                (root / "tracked").write_text("tracked", encoding="utf-8")
                subprocess.run(["git", "-C", str(root), "add", "."], check=True)
                subprocess.run(["git", "-C", str(root), "commit", "-qm", "base"], check=True)
                hostile = root / relative
                hostile.parent.mkdir(parents=True)
                hostile.write_text("hostile", encoding="utf-8")
                environment = os.environ.copy()
                environment[CONTAINMENT_ENVIRONMENT_KEY] = "9" * 32
                with self.assertRaises(RuntimeError):
                    check_machine_cleanliness(root, "git", environment, 2.0)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(
                ["git", "-C", str(root), "config", "user.email", "test@example.test"],
                check=True,
            )
            (root / "tracked-old").write_text("tracked", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "tracked-old"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "base"], check=True)
            (root / "target").mkdir()
            subprocess.run(
                ["git", "-C", str(root), "mv", "tracked-old", "target/renamed"],
                check=True,
            )
            environment = os.environ.copy()
            environment[CONTAINMENT_ENVIRONMENT_KEY] = "0" * 32
            with self.assertRaises(RuntimeError):
                check_machine_cleanliness(root, "git", environment, 2.0)

    def test_failed_harness_preserves_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            evidence_path = temporary / "evidence.json"
            evidence_path.write_text("stale", encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/upstream.py"),
                    "--output",
                    str(evidence_path),
                    "--upstream-dir",
                    str(temporary / "fx"),
                    "--scratch-dir",
                    str(temporary / "scratch"),
                    "--zig",
                    "definitely-not-zig",
                    "--fetch-timeout",
                    "0.1",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(completed.returncode, 0)
            self.assertEqual(evidence_path.read_text(encoding="utf-8"), "stale")

    def test_concurrent_failure_does_not_remove_successful_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "evidence.json"
            success_started = threading.Event()
            producer_order: list[str] = []
            failures: list[tuple[str, BaseException]] = []

            def successful_producer() -> dict[str, object]:
                producer_order.append("success")
                success_started.set()
                time.sleep(0.15)
                return {"writer": "success"}

            def delayed_failure() -> dict[str, object]:
                producer_order.append("failure")
                time.sleep(0.05)
                raise RuntimeError("delayed failure")

            def publish_success() -> None:
                try:
                    collect_and_publish_evidence(
                        output, successful_producer, lock_timeout_seconds=1.0
                    )
                except BaseException as error:
                    failures.append(("success", error))

            def publish_failure() -> None:
                try:
                    collect_and_publish_evidence(
                        output, delayed_failure, lock_timeout_seconds=1.0
                    )
                except BaseException as error:
                    failures.append(("failure", error))

            successful_thread = threading.Thread(target=publish_success)
            failing_thread = threading.Thread(target=publish_failure)
            successful_thread.start()
            self.assertTrue(success_started.wait(1.0))
            failing_thread.start()
            successful_thread.join(2.0)
            failing_thread.join(2.0)

            self.assertFalse(successful_thread.is_alive())
            self.assertFalse(failing_thread.is_alive())
            self.assertEqual(producer_order, ["success", "failure"])
            self.assertEqual(len(failures), 1)
            self.assertEqual(failures[0][0], "failure")
            self.assertIsInstance(failures[0][1], RuntimeError)
            self.assertEqual(str(failures[0][1]), "delayed failure")
            self.assertEqual(
                json.loads(output.read_text(encoding="utf-8")),
                {"writer": "success"},
            )
            self.assertFalse(output.with_name(f".{output.name}.lock").exists())

    def test_output_lock_fails_bounded_without_deleting_preexisting_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "evidence.json"
            lock_path = output.with_name(f".{output.name}.lock")
            lock_path.write_text("not ours", encoding="utf-8")

            started = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, "locked by another invocation"):
                acquire_output_lock(output, timeout_seconds=0.05)

            self.assertLess(time.monotonic() - started, 0.5)
            self.assertEqual(lock_path.read_text(encoding="utf-8"), "not ours")

    def test_atomic_evidence_resists_symlinks_files_and_temp_collisions(self) -> None:
        evidence = {"schema_version": 2, "value": "new"}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            victim = root / "victim"
            victim.write_text("do not overwrite", encoding="utf-8")
            output = root / "evidence.json"
            output.symlink_to(victim)
            predictable = root / f".{output.name}.{os.getpid()}.partial"
            predictable.symlink_to(victim)

            write_evidence_atomic(output, evidence)

            self.assertEqual(victim.read_text(encoding="utf-8"), "do not overwrite")
            self.assertFalse(output.is_symlink())
            self.assertEqual(json.loads(output.read_text(encoding="utf-8")), evidence)
            self.assertTrue(predictable.is_symlink())

            output.write_text("stale regular file", encoding="utf-8")
            write_evidence_atomic(output, evidence)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8")), evidence)

            collision = root / f".{output.name}.collision.partial"
            collision.write_text("attacker file", encoding="utf-8")
            with mock.patch.object(
                upstream.tempfile,
                "_get_candidate_names",
                return_value=iter(("collision", "exclusive")),
            ):
                write_evidence_atomic(output, evidence)
            self.assertEqual(collision.read_text(encoding="utf-8"), "attacker file")
            self.assertFalse(root.joinpath(f".{output.name}.exclusive.partial").exists())

    def test_atomic_evidence_creates_output_directory_and_preserves_symlink_victim(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "nested/results/evidence.json"
            write_evidence_atomic(output, {"ok": True})
            self.assertEqual(json.loads(output.read_text(encoding="utf-8")), {"ok": True})
            self.assertEqual(list(output.parent.glob(f".{output.name}.*.partial")), [])

            victim = root / "victim"
            victim.write_text("safe", encoding="utf-8")
            output.unlink()
            output.symlink_to(victim)
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "benchmarks/upstream.py"),
                    "--output",
                    str(output),
                    "--upstream-dir",
                    str(root / "fx"),
                    "--scratch-dir",
                    str(root / "scratch"),
                    "--zig",
                    "definitely-not-zig",
                    "--fetch-timeout",
                    "0.1",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(completed.returncode, 0)
            self.assertEqual(victim.read_text(encoding="utf-8"), "safe")
            self.assertTrue(output.is_symlink())


if __name__ == "__main__":
    unittest.main()
