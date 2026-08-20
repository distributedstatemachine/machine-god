import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks"))

from upstream import (  # noqa: E402
    ALLOWED_MACHINE_OUTPUTS,
    EXPECTED_RUST_VERSION,
    EXPECTED_ZIG_VERSION,
    ProcessTimeout,
    UpstreamLock,
    check_machine_cleanliness,
    command_plan,
    parse_upstream_lock,
    prepare_upstream,
    run_process,
    sha256_file,
    unavailable_workloads,
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
    def base_environment(self, root: Path) -> dict[str, str]:
        return {
            "HOME": str(root / ".bench/scratch/home"),
            "LANG": "C",
            "LC_ALL": "C",
            "NO_COLOR": "1",
            "PATH": "/usr/bin:/bin",
            "TMPDIR": str(root / ".bench/scratch/tmp"),
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
            "returncode": 0,
            "stdout_sha256": "0" * 64,
            "stderr_sha256": "1" * 64,
        }

    def valid_upstream_evidence(
        self,
        root: Path = Path("/checkout"),
        *,
        fx_root: Path | None = None,
        scratch: Path | None = None,
    ) -> dict[str, object]:
        lock_path = ROOT / "benchmarks/upstream.lock"
        lock = parse_upstream_lock(lock_path)
        fx_root = fx_root or root / ".bench/fx"
        scratch = scratch or root / ".bench/scratch"
        fx_binary = fx_root / "zig-out/bin/fx"
        machine_binary = scratch / "machine-target/release/machine-god"
        base = self.base_environment(root)
        git_environment = {
            **base,
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
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
                "version": "git version 2",
            },
            "zig": {
                "command": ["/usr/bin/zig", "version"],
                "executable": "/usr/bin/zig",
                "required_version": EXPECTED_ZIG_VERSION,
                "version": EXPECTED_ZIG_VERSION,
            },
            "rustc": {
                "command": ["/usr/bin/rustc", "+1.94.1", "--version"],
                "executable": "/usr/bin/rustc",
                "required_version": EXPECTED_RUST_VERSION,
                "version": "rustc 1.94.1 (test 2026-01-01)",
            },
            "cargo": {
                "command": ["/usr/bin/cargo", "+1.94.1", "--version"],
                "executable": "/usr/bin/cargo",
                "required_version": EXPECTED_RUST_VERSION,
                "version": "cargo 1.94.1 (test 2026-01-01)",
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
        samples = [{"elapsed_ns": value, "returncode": 0} for value in range(1, 11)]
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
            plan["machine_god_build"], machine_environment, 10.0, root
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
            implementations.append(
                {
                    "project": project,
                    "status": "measured",
                    "command": [str(binary)],
                    "cwd": str(root),
                    "environment": environment,
                    "timeout_seconds": 1.0,
                    "warmup": 1,
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
                    "git_sha": "3" * 40,
                    "dirty": False,
                    "allowed_output_directories": list(ALLOWED_MACHINE_OUTPUTS),
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
                    "description": "harness smoke path",
                    "equivalence": "non-equivalent",
                    "claim_eligible": False,
                    "reason": "the programs execute different bootstrap behavior",
                    "implementations": implementations,
                },
                *unavailable_workloads(fx_binary),
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

    def test_rejects_aggregate_not_derived_from_raw_samples(self) -> None:
        evidence = self.valid_upstream_evidence()
        evidence["workloads"][0]["implementations"][0]["p95_ns"] = 9
        with self.assertRaises(ValueError):
            validate_upstream_evidence(evidence)

    def test_schema_two_checker_binds_sha_and_both_actual_binaries(self) -> None:
        (ROOT / ".bench").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / ".bench") as directory:
            temporary = Path(directory)
            evidence = self.valid_upstream_evidence(
                ROOT,
                fx_root=temporary / "fx",
                scratch=temporary / "scratch",
            )
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
            evidence_path = temporary / "upstream.json"
            evidence_path.write_text(
                json.dumps(evidence), encoding="utf-8"
            )
            command = [
                sys.executable,
                str(ROOT / "benchmarks/check.py"),
                str(evidence_path),
                "--expected-git-sha",
                "3" * 40,
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
            machine_binary.write_bytes(b"tampered")
            tampered = subprocess.run(
                command, check=False, capture_output=True, text=True
            )
        self.assertNotEqual(tampered.returncode, 0)

    def test_process_timeout_terminates_child_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "child-finished"
            child = (
                "import pathlib,time; time.sleep(0.4); "
                f"pathlib.Path({str(marker)!r}).write_text('bad')"
            )
            parent = (
                "import subprocess,sys,time; "
                f"subprocess.Popen([sys.executable,'-c',{child!r}]); time.sleep(5)"
            )
            with self.assertRaises(ProcessTimeout):
                run_process(
                    [sys.executable, "-c", parent],
                    cwd=Path(directory),
                    environment=os.environ,
                    timeout_seconds=0.1,
                )
            time.sleep(0.5)
            self.assertFalse(marker.exists())

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
            check_machine_cleanliness(root, "git", environment, 2.0)
            (root / ".cargo").mkdir()
            (root / ".cargo/config.toml").write_text(
                "[build]\nrustflags=['-Ctarget-cpu=native']\n", encoding="utf-8"
            )
            with self.assertRaises(RuntimeError):
                check_machine_cleanliness(root, "git", environment, 2.0)

    def test_failed_harness_removes_stale_evidence(self) -> None:
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
        self.assertFalse(evidence_path.exists())


if __name__ == "__main__":
    unittest.main()
