#!/usr/bin/env python3
"""Build and measure machine-god beside the exact pinned fx revision.

This harness intentionally produces bootstrap infrastructure evidence, not a
product performance claim.  The current machine-god CLI does not implement the
fx local commands, so those workloads are retained as explicitly unimplemented
comparison cases.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence


EXPECTED_RUST_VERSION = "1.94.1"
EXPECTED_ZIG_VERSION = "0.16.0"
HEX_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


@dataclass(frozen=True)
class UpstreamLock:
    repository: str
    commit: str
    zig: str


def parse_upstream_lock(path: Path) -> UpstreamLock:
    """Parse the deliberately small key=value upstream lock format."""

    values: dict[str, str] = {}
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ValueError(f"{path}:{line_number}: expected key=value")
        key, value = (part.strip() for part in line.split("=", 1))
        if not key or not value:
            raise ValueError(f"{path}:{line_number}: key and value must be non-empty")
        if key in values:
            raise ValueError(f"{path}:{line_number}: duplicate key {key!r}")
        values[key] = value

    expected = {"repository", "commit", "zig"}
    missing = sorted(expected - values.keys())
    unknown = sorted(values.keys() - expected)
    if missing:
        raise ValueError(f"{path}: missing keys: {', '.join(missing)}")
    if unknown:
        raise ValueError(f"{path}: unknown keys: {', '.join(unknown)}")
    if not HEX_SHA_RE.fullmatch(values["commit"]):
        raise ValueError(f"{path}: commit must be a lowercase 40-character Git SHA")
    if values["zig"] != EXPECTED_ZIG_VERSION:
        raise ValueError(
            f"{path}: this harness requires zig={EXPECTED_ZIG_VERSION}, "
            f"found {values['zig']}"
        )
    return UpstreamLock(**values)


def command_plan(
    root: Path,
    upstream_dir: Path,
    lock: UpstreamLock,
    *,
    git: str = "git",
    zig: str = "zig",
    cargo: str = "cargo",
) -> dict[str, list[str]]:
    """Return source and build commands without executing them."""

    return {
        "clone": [
            git,
            "clone",
            "--filter=blob:none",
            "--no-checkout",
            lock.repository,
            str(upstream_dir),
        ],
        "fetch": [
            git,
            "-C",
            str(upstream_dir),
            "fetch",
            "--depth",
            "1",
            "origin",
            lock.commit,
        ],
        "checkout": [
            git,
            "-C",
            str(upstream_dir),
            "checkout",
            "--detach",
            lock.commit,
        ],
        "fx_build": [zig, "build", "-Doptimize=ReleaseSafe"],
        "machine_god_build": [
            cargo,
            f"+{EXPECTED_RUST_VERSION}",
            "build",
            "--locked",
            "--release",
            "-p",
            "machine-god-cli",
        ],
    }


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def require_command(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ValueError(f"{field} must be a non-empty list")
    for index, argument in enumerate(value):
        require_text(argument, f"{field}[{index}]")
    return value


def percentile_95(samples: Sequence[int]) -> int:
    ordered = sorted(samples)
    index = min(len(ordered) - 1, (len(ordered) * 95 + 99) // 100 - 1)
    return ordered[index]


def validate_upstream_evidence(data: Mapping[str, Any]) -> None:
    """Validate schema 2 upstream bootstrap evidence.

    The claim-eligibility checks are intentional: changing a label alone cannot
    turn this bootstrap harness into product comparison evidence.
    """

    if data.get("schema_version") != 2 or not is_integer(data.get("schema_version")):
        raise ValueError("unsupported upstream benchmark schema")
    if data.get("classification") != "bootstrap-infrastructure-only":
        raise ValueError("upstream harness evidence must be bootstrap-only")
    if data.get("claim_eligible") is not False:
        raise ValueError("bootstrap evidence must not be claim eligible")
    require_text(data.get("generated_at_utc"), "generated_at_utc")

    source = data.get("source")
    if not isinstance(source, dict):
        raise ValueError("source provenance is missing")
    machine_source = source.get("machine_god")
    fx_source = source.get("fx")
    if not isinstance(machine_source, dict) or not isinstance(fx_source, dict):
        raise ValueError("both source revisions are required")
    machine_sha = require_text(machine_source.get("git_sha"), "source.machine_god.git_sha")
    if not HEX_SHA_RE.fullmatch(machine_sha):
        raise ValueError("source.machine_god.git_sha must be a lowercase 40-character SHA")
    if machine_source.get("dirty") is not False:
        raise ValueError("machine-god source must be clean")
    repository = require_text(fx_source.get("repository"), "source.fx.repository")
    if not repository.startswith("https://"):
        raise ValueError("source.fx.repository must be an HTTPS URL")
    locked_commit = require_text(fx_source.get("locked_commit"), "source.fx.locked_commit")
    verified_commit = require_text(
        fx_source.get("verified_commit"), "source.fx.verified_commit"
    )
    if not HEX_SHA_RE.fullmatch(locked_commit) or verified_commit != locked_commit:
        raise ValueError("the verified fx commit must equal the locked 40-character SHA")
    preparation = fx_source.get("preparation_commands")
    if not isinstance(preparation, list) or not preparation:
        raise ValueError("source.fx.preparation_commands must be retained")
    for index, record in enumerate(preparation):
        validate_command_record(record, f"source.fx.preparation_commands[{index}]")

    host = data.get("host")
    if not isinstance(host, dict):
        raise ValueError("host metadata is missing")
    for field in ("system", "release", "machine", "python"):
        require_text(host.get(field), f"host.{field}")
    if not is_integer(host.get("cpu_count")) or host["cpu_count"] < 1:
        raise ValueError("host.cpu_count must be a positive integer")

    tools = data.get("tools")
    if not isinstance(tools, dict):
        raise ValueError("tool provenance is missing")
    for name in ("git", "zig", "rustc", "cargo"):
        tool = tools.get(name)
        if not isinstance(tool, dict):
            raise ValueError(f"tools.{name} is missing")
        require_command(tool.get("command"), f"tools.{name}.command")
        require_text(tool.get("executable"), f"tools.{name}.executable")
        require_text(tool.get("version"), f"tools.{name}.version")
    if tools["zig"].get("required_version") != EXPECTED_ZIG_VERSION:
        raise ValueError("tools.zig.required_version is not pinned to 0.16.0")
    if tools["zig"].get("version") != EXPECTED_ZIG_VERSION:
        raise ValueError("evidence was not built with Zig 0.16.0")
    for name in ("rustc", "cargo"):
        if tools[name].get("required_version") != EXPECTED_RUST_VERSION:
            raise ValueError(f"tools.{name}.required_version is not pinned to 1.94.1")
        if not tools[name]["version"].startswith(f"{name} {EXPECTED_RUST_VERSION} "):
            raise ValueError(f"evidence was not built with {name} {EXPECTED_RUST_VERSION}")

    builds = data.get("builds")
    if not isinstance(builds, list) or len(builds) != 2:
        raise ValueError("exactly two build records are required")
    projects: set[str] = set()
    for index, build in enumerate(builds):
        field = f"builds[{index}]"
        if not isinstance(build, dict):
            raise ValueError(f"{field} must be an object")
        projects.add(require_text(build.get("project"), f"{field}.project"))
        require_text(build.get("profile"), f"{field}.profile")
        validate_command_record(build, field)
        validate_binary(build.get("binary"), f"{field}.binary")
    if projects != {"fx", "machine-god"}:
        raise ValueError("build records must cover fx and machine-god")

    workloads = data.get("workloads")
    if not isinstance(workloads, list) or not workloads:
        raise ValueError("at least one workload is required")
    measured_projects: set[str] = set()
    saw_explicit_gap = False
    for index, workload in enumerate(workloads):
        field = f"workloads[{index}]"
        if not isinstance(workload, dict):
            raise ValueError(f"{field} must be an object")
        require_text(workload.get("id"), f"{field}.id")
        require_text(workload.get("description"), f"{field}.description")
        equivalence = workload.get("equivalence")
        if equivalence not in {"non-equivalent", "unimplemented"}:
            raise ValueError(f"{field}.equivalence makes an unsupported comparison claim")
        saw_explicit_gap = True
        if workload.get("claim_eligible") is not False:
            raise ValueError(f"{field} must not be claim eligible")
        require_text(workload.get("reason"), f"{field}.reason")
        implementations = workload.get("implementations")
        if not isinstance(implementations, list) or len(implementations) != 2:
            raise ValueError(f"{field} must describe fx and machine-god")
        implementation_projects: set[str] = set()
        for impl_index, implementation in enumerate(implementations):
            impl_field = f"{field}.implementations[{impl_index}]"
            if not isinstance(implementation, dict):
                raise ValueError(f"{impl_field} must be an object")
            project = require_text(implementation.get("project"), f"{impl_field}.project")
            implementation_projects.add(project)
            status = implementation.get("status")
            if status == "measured":
                validate_measurement(implementation, impl_field)
                measured_projects.add(project)
            elif status in {"not-measured", "unimplemented"}:
                if "samples" in implementation:
                    raise ValueError(f"{impl_field} must not contain samples")
                if status == "not-measured":
                    require_command(implementation.get("command"), f"{impl_field}.command")
                require_text(implementation.get("reason"), f"{impl_field}.reason")
            else:
                raise ValueError(f"{impl_field}.status is invalid")
        if implementation_projects != {"fx", "machine-god"}:
            raise ValueError(f"{field} must cover fx and machine-god")
    if measured_projects != {"fx", "machine-god"}:
        raise ValueError("bootstrap evidence must contain raw samples for both projects")
    if not saw_explicit_gap:
        raise ValueError("comparison gaps must be explicit")


def validate_command_record(record: object, field: str) -> None:
    if not isinstance(record, dict):
        raise ValueError(f"{field} must be an object")
    require_command(record.get("command"), f"{field}.command")
    require_text(record.get("cwd"), f"{field}.cwd")
    if not is_integer(record.get("elapsed_ns")) or record["elapsed_ns"] <= 0:
        raise ValueError(f"{field}.elapsed_ns must be a positive integer")
    if record.get("returncode") != 0 or not is_integer(record.get("returncode")):
        raise ValueError(f"{field}.returncode must be integer zero")
    for stream in ("stdout_sha256", "stderr_sha256"):
        checksum = require_text(record.get(stream), f"{field}.{stream}")
        if len(checksum) != 64 or any(character not in "0123456789abcdef" for character in checksum):
            raise ValueError(f"{field}.{stream} must be a lowercase SHA-256 digest")


def validate_binary(binary: object, field: str) -> None:
    if not isinstance(binary, dict):
        raise ValueError(f"{field} must be an object")
    require_text(binary.get("path"), f"{field}.path")
    if not is_integer(binary.get("bytes")) or binary["bytes"] <= 0:
        raise ValueError(f"{field}.bytes must be a positive integer")
    checksum = require_text(binary.get("sha256"), f"{field}.sha256")
    if len(checksum) != 64 or any(character not in "0123456789abcdef" for character in checksum):
        raise ValueError(f"{field}.sha256 must be a lowercase SHA-256 digest")


def validate_measurement(measurement: Mapping[str, Any], field: str) -> None:
    require_command(measurement.get("command"), f"{field}.command")
    require_text(measurement.get("cwd"), f"{field}.cwd")
    overrides = measurement.get("environment_overrides")
    if not isinstance(overrides, dict):
        raise ValueError(f"{field}.environment_overrides must be an object")
    for name, value in overrides.items():
        require_text(name, f"{field}.environment_overrides key")
        require_text(value, f"{field}.environment_overrides[{name!r}]")
    warmup = measurement.get("warmup")
    if not is_integer(warmup) or warmup < 1:
        raise ValueError(f"{field}.warmup must be a positive integer")
    samples = measurement.get("samples")
    if not isinstance(samples, list) or len(samples) < 10:
        raise ValueError(f"{field} needs at least 10 raw samples")
    elapsed: list[int] = []
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            raise ValueError(f"{field}.samples[{index}] must be an object")
        elapsed_ns = sample.get("elapsed_ns")
        if not is_integer(elapsed_ns) or elapsed_ns <= 0:
            raise ValueError(f"{field}.samples[{index}].elapsed_ns must be positive")
        if sample.get("returncode") != 0 or not is_integer(sample.get("returncode")):
            raise ValueError(f"{field}.samples[{index}].returncode must be integer zero")
        elapsed.append(elapsed_ns)
    if measurement.get("median_ns") != int(statistics.median(elapsed)):
        raise ValueError(f"{field}.median_ns does not match raw samples")
    if measurement.get("p95_ns") != percentile_95(elapsed):
        raise ValueError(f"{field}.p95_ns does not match raw samples")


def resolved_executable(command: str) -> str:
    executable = shutil.which(command)
    if executable is None:
        raise RuntimeError(f"required executable was not found: {command}")
    return str(Path(executable).resolve())


def tool_record(command: list[str], required_version: str | None = None) -> dict[str, object]:
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"tool version command failed ({' '.join(command)}): {detail}")
    version = completed.stdout.strip() or completed.stderr.strip()
    record: dict[str, object] = {
        "command": command,
        "executable": resolved_executable(command[0]),
        "version": version,
    }
    if required_version is not None:
        record["required_version"] = required_version
    return record


def verify_tool_versions(git: str, zig: str, rustc: str, cargo: str) -> dict[str, object]:
    tools = {
        "git": tool_record([git, "--version"]),
        "zig": tool_record([zig, "version"], EXPECTED_ZIG_VERSION),
        "rustc": tool_record(
            [rustc, f"+{EXPECTED_RUST_VERSION}", "--version"], EXPECTED_RUST_VERSION
        ),
        "cargo": tool_record(
            [cargo, f"+{EXPECTED_RUST_VERSION}", "--version"], EXPECTED_RUST_VERSION
        ),
    }
    if tools["zig"]["version"] != EXPECTED_ZIG_VERSION:
        raise RuntimeError(
            f"Zig {EXPECTED_ZIG_VERSION} is required; found {tools['zig']['version']}"
        )
    for name in ("rustc", "cargo"):
        version = str(tools[name]["version"])
        if not version.startswith(f"{name} {EXPECTED_RUST_VERSION} "):
            raise RuntimeError(f"{name} {EXPECTED_RUST_VERSION} is required; found {version}")
    return tools


def run_record(command: list[str], cwd: Path) -> dict[str, object]:
    start = time.perf_counter_ns()
    completed = subprocess.run(command, cwd=cwd, check=False, capture_output=True)
    elapsed_ns = time.perf_counter_ns() - start
    if completed.stdout:
        sys.stdout.buffer.write(completed.stdout)
        sys.stdout.buffer.flush()
    if completed.stderr:
        sys.stderr.buffer.write(completed.stderr)
        sys.stderr.buffer.flush()
    record = {
        "command": command,
        "cwd": str(cwd),
        "elapsed_ns": elapsed_ns,
        "returncode": completed.returncode,
        "stdout_sha256": sha256_bytes(completed.stdout),
        "stderr_sha256": sha256_bytes(completed.stderr),
    }
    if completed.returncode != 0:
        raise RuntimeError(f"command exited {completed.returncode}: {' '.join(command)}")
    return record


def git_output(git: str, cwd: Path, *arguments: str) -> str:
    return subprocess.check_output([git, *arguments], cwd=cwd, text=True).strip()


def prepare_upstream(
    root: Path,
    upstream_dir: Path,
    lock: UpstreamLock,
    plan: Mapping[str, list[str]],
    git: str,
) -> tuple[str, list[dict[str, object]]]:
    records: list[dict[str, object]] = []
    if upstream_dir.exists():
        if not (upstream_dir / ".git").exists():
            raise RuntimeError(f"upstream path exists but is not a Git checkout: {upstream_dir}")
        if git_output(git, upstream_dir, "status", "--porcelain"):
            raise RuntimeError(f"upstream checkout is dirty: {upstream_dir}")
        origin = git_output(git, upstream_dir, "remote", "get-url", "origin")
        if origin != lock.repository:
            raise RuntimeError(f"upstream origin is {origin!r}, expected {lock.repository!r}")
    else:
        upstream_dir.parent.mkdir(parents=True, exist_ok=True)
        records.append(run_record(plan["clone"], root))

    records.append(run_record(plan["fetch"], root))
    records.append(run_record(plan["checkout"], root))
    verified_commit = git_output(git, upstream_dir, "rev-parse", "HEAD")
    if verified_commit != lock.commit:
        raise RuntimeError(
            f"upstream checkout resolved to {verified_commit}, expected {lock.commit}"
        )
    if git_output(git, upstream_dir, "status", "--porcelain"):
        raise RuntimeError(f"upstream checkout is not clean after checkout: {upstream_dir}")
    return verified_commit, records


def binary_record(path: Path) -> dict[str, object]:
    resolved = path.resolve()
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise RuntimeError(f"build did not produce an executable binary: {resolved}")
    return {
        "path": str(resolved),
        "bytes": resolved.stat().st_size,
        "sha256": sha256_file(resolved),
    }


def run_measurement(
    project: str,
    command: list[str],
    cwd: Path,
    environment_overrides: Mapping[str, str],
    warmup: int,
    runs: int,
) -> dict[str, object]:
    environment = os.environ.copy()
    environment.update(environment_overrides)

    def run_once() -> dict[str, int]:
        start = time.perf_counter_ns()
        completed = subprocess.run(
            command,
            cwd=cwd,
            env=environment,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return {
            "elapsed_ns": time.perf_counter_ns() - start,
            "returncode": completed.returncode,
        }

    for _ in range(warmup):
        sample = run_once()
        if sample["returncode"] != 0:
            raise RuntimeError(f"{project} warmup exited {sample['returncode']}")
    samples = [run_once() for _ in range(runs)]
    failed = [sample for sample in samples if sample["returncode"] != 0]
    if failed:
        raise RuntimeError(f"{project} measured run exited {failed[0]['returncode']}")
    elapsed = [sample["elapsed_ns"] for sample in samples]
    return {
        "project": project,
        "status": "measured",
        "command": command,
        "cwd": str(cwd),
        "environment_overrides": dict(environment_overrides),
        "warmup": warmup,
        "samples": samples,
        "median_ns": int(statistics.median(elapsed)),
        "p95_ns": percentile_95(elapsed),
    }


def unavailable_workloads(fx_binary: Path) -> list[dict[str, object]]:
    definitions = (
        ("help", [str(fx_binary), "help"], "machine-god has no help command"),
        (
            "status-json",
            [str(fx_binary), "status", "--json"],
            "machine-god has no local status command or configuration model",
        ),
        (
            "doctor-json",
            [str(fx_binary), "doctor", "--json"],
            "machine-god has no local diagnostics command",
        ),
        (
            "sessions-json",
            [str(fx_binary), "sessions", "--json"],
            "machine-god has no persisted session list command",
        ),
        (
            "background-json",
            [str(fx_binary), "background", "--json"],
            "machine-god has no background-task list command",
        ),
    )
    workloads = []
    for identifier, command, reason in definitions:
        workloads.append(
            {
                "id": identifier,
                "description": f"Pinned fx local command: {' '.join(command[1:])}",
                "equivalence": "unimplemented",
                "claim_eligible": False,
                "reason": reason,
                "implementations": [
                    {
                        "project": "fx",
                        "status": "not-measured",
                        "command": command,
                        "reason": "an unpaired result would not be a comparison",
                    },
                    {
                        "project": "machine-god",
                        "status": "unimplemented",
                        "reason": reason,
                    },
                ],
            }
        )
    return workloads


def collect_evidence(args: argparse.Namespace) -> dict[str, object]:
    root = args.root.resolve()
    lock_path = args.lock.resolve()
    upstream_dir = args.upstream_dir.resolve()
    lock = parse_upstream_lock(lock_path)
    if args.runs < 10 or args.warmup < 1:
        raise ValueError("runs must be >= 10 and warmup must be >= 1")

    tools = verify_tool_versions(args.git, args.zig, args.rustc, args.cargo)
    machine_sha = git_output(args.git, root, "rev-parse", "HEAD")
    if not HEX_SHA_RE.fullmatch(machine_sha):
        raise RuntimeError(f"machine-god HEAD is not a full Git SHA: {machine_sha}")
    dirty = bool(git_output(args.git, root, "status", "--porcelain", "--untracked-files=no"))
    if dirty:
        raise RuntimeError("machine-god worktree is dirty; commit before collecting evidence")

    plan = command_plan(
        root,
        upstream_dir,
        lock,
        git=args.git,
        zig=args.zig,
        cargo=args.cargo,
    )
    verified_commit, preparation = prepare_upstream(
        root, upstream_dir, lock, plan, args.git
    )

    fx_build = run_record(plan["fx_build"], upstream_dir)
    fx_build.update(
        {
            "project": "fx",
            "profile": "ReleaseSafe",
            "binary": binary_record(upstream_dir / "zig-out/bin/fx"),
        }
    )
    machine_build = run_record(plan["machine_god_build"], root)
    machine_build.update(
        {
            "project": "machine-god",
            "profile": "release",
            "binary": binary_record(root / "target/release/machine-god"),
        }
    )

    fixture_home = args.fixture_home.resolve()
    fixture_home.mkdir(parents=True, exist_ok=True)
    common_environment = {"HOME": str(fixture_home), "LC_ALL": "C", "NO_COLOR": "1"}
    fx_binary = Path(str(fx_build["binary"]["path"]))
    machine_binary = Path(str(machine_build["binary"]["path"]))
    bootstrap = {
        "id": "bootstrap-exit",
        "description": "Launch each release binary through its current no-network bootstrap path",
        "equivalence": "non-equivalent",
        "claim_eligible": False,
        "reason": (
            "fx uses its FX_BENCH no-argument fast path while machine-god prints its "
            "bootstrap identity; these samples validate the harness and are not product-equivalent"
        ),
        "implementations": [
            run_measurement(
                "fx",
                [str(fx_binary)],
                root,
                {**common_environment, "FX_BENCH": "1"},
                args.warmup,
                args.runs,
            ),
            run_measurement(
                "machine-god",
                [str(machine_binary)],
                root,
                common_environment,
                args.warmup,
                args.runs,
            ),
        ],
    }

    evidence = {
        "schema_version": 2,
        "classification": "bootstrap-infrastructure-only",
        "claim_eligible": False,
        "generated_at_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "source": {
            "machine_god": {"git_sha": machine_sha, "dirty": False},
            "fx": {
                "repository": lock.repository,
                "locked_commit": lock.commit,
                "verified_commit": verified_commit,
                "lock_path": str(lock_path),
                "preparation_commands": preparation,
            },
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count() or 1,
        },
        "tools": tools,
        "builds": [fx_build, machine_build],
        "environment_policy": {
            "inherits_parent_environment": True,
            "evidence_records_only_non_secret_overrides": True,
        },
        "workloads": [bootstrap, *unavailable_workloads(fx_binary)],
    }
    validate_upstream_evidence(evidence)
    return evidence


def main() -> int:
    default_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=default_root)
    parser.add_argument("--lock", type=Path, default=default_root / "benchmarks/upstream.lock")
    parser.add_argument("--upstream-dir", type=Path, default=default_root / ".bench/fx")
    parser.add_argument("--fixture-home", type=Path, default=default_root / ".bench/home")
    parser.add_argument(
        "--output",
        type=Path,
        default=default_root / "benchmarks/results/upstream-bootstrap.json",
    )
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--git", default="git")
    parser.add_argument("--zig", default="zig")
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()

    try:
        evidence = collect_evidence(args)
    except (OSError, subprocess.SubprocessError, RuntimeError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(f"wrote validated bootstrap evidence to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
