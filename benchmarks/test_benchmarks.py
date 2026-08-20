import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks"))

from upstream import (  # noqa: E402
    ALLOWED_MACHINE_OUTPUTS,
    CONTAINMENT_ENVIRONMENT_KEY,
    EXPECTED_RUST_VERSION,
    EXPECTED_ZIG_VERSION,
    ProcessTimeout,
    UpstreamLock,
    canonical_git_entries_sha256,
    canonical_manifest_sha256,
    check_machine_cleanliness,
    command_plan,
    executable_identity,
    finalize_successful_process,
    invocation_path,
    linux_containment_preflight,
    machine_tree_command,
    materialize_machine_source,
    parse_upstream_lock,
    prepare_upstream,
    run_process,
    sha256_file,
    source_tree_sha256,
    unavailable_workloads,
    validate_upstream_evidence,
)
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
                "sha256": "a" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 1,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "version": "git version 2",
            },
            "zig": {
                "command": ["/usr/bin/zig", "version"],
                "executable": "/usr/bin/zig",
                "sha256": "b" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 2,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "required_version": EXPECTED_ZIG_VERSION,
                "version": EXPECTED_ZIG_VERSION,
            },
            "rustc": {
                "command": ["/usr/bin/rustc", "+1.94.1", "--version"],
                "executable": "/usr/bin/rustc",
                "sha256": "c" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 3,
                "mtime_ns": 1,
                "ctime_ns": 1,
                "required_version": EXPECTED_RUST_VERSION,
                "version": "rustc 1.94.1 (test 2026-01-01)",
            },
            "cargo": {
                "command": ["/usr/bin/cargo", "+1.94.1", "--version"],
                "executable": "/usr/bin/cargo",
                "sha256": "d" * 64,
                "bytes": 1,
                "mode": 0o755,
                "device": 1,
                "inode": 4,
                "mtime_ns": 1,
                "ctime_ns": 1,
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
            implementations.append(
                {
                    "project": project,
                    "status": "measured",
                    "command": [str(binary)],
                    "cwd": str(machine_source),
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
            completed = run_process(
                [resolved],
                cwd=temporary,
                environment=environment,
                timeout_seconds=1.0,
                expected_executable=identity,
            )
            self.assertEqual(completed.stdout, b"good")

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
