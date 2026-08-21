#!/usr/bin/env python3
"""Collect reproducible bootstrap timing and binary-size evidence."""

from __future__ import annotations

import argparse
import json
import os
import platform
import secrets
import subprocess
from pathlib import Path

from upstream import (  # noqa: E402
    CONTAINMENT_ENVIRONMENT_KEY,
    PinnedExecutable,
    collect_and_publish_evidence,
    executable_identity,
    git_output,
    invocation_path,
    pin_executable,
    run_process,
    verify_executable_identity,
)


GIT_TIMEOUT_SECONDS = 10.0


def isolated_git_environment() -> dict[str, str]:
    path_value = os.environ.get("PATH")
    if not path_value:
        raise RuntimeError("PATH is required to invoke Git")
    return {
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "LANG": "C",
        "LC_ALL": "C",
        CONTAINMENT_ENVIRONMENT_KEY: secrets.token_hex(16),
        "NO_COLOR": "1",
        "PATH": path_value,
    }


def repository_head(
    cwd: Path,
    *,
    git: str = "git",
    timeout_seconds: float = GIT_TIMEOUT_SECONDS,
) -> str:
    environment = isolated_git_environment()
    resolved_git = invocation_path(git, environment["PATH"])
    git_sha = git_output(
        resolved_git,
        cwd,
        environment,
        timeout_seconds,
        "rev-parse",
        "--verify",
        "HEAD^{commit}",
    )
    if len(git_sha) != 40 or any(
        character not in "0123456789abcdef" for character in git_sha
    ):
        raise RuntimeError(f"machine-god HEAD is not a full Git SHA: {git_sha}")
    return git_sha


def run_once(
    pinned: PinnedExecutable,
    binary: Path,
    identity: dict[str, object],
    environment: dict[str, str],
) -> tuple[int, int]:
    verify_executable_identity(identity)
    pinned.verify()
    try:
        completed = run_process(
            [str(binary)],
            cwd=Path.cwd(),
            environment=environment,
            timeout_seconds=30.0,
            capture_output=False,
            executable_descriptor=(
                pinned.descriptor
                if pinned.method == "linux-sealed-memfd-fexecve"
                else None
            ),
            executable_path=pinned.execution_path,
        )
    except BaseException:
        try:
            pinned.verify()
            verify_executable_identity(identity)
        except BaseException:
            pass
        raise
    pinned.verify()
    verify_executable_identity(identity)
    return completed.elapsed_ns, completed.returncode


def integer_median(samples: list[int]) -> int:
    ordered = sorted(samples)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[middle]
    return (ordered[middle - 1] + ordered[middle]) // 2


def collect_evidence(binary: Path, runs: int, warmup: int) -> dict[str, object]:
    identity = executable_identity(binary)
    pinned = pin_executable(identity)
    environment = os.environ.copy()
    environment[CONTAINMENT_ENVIRONMENT_KEY] = secrets.token_hex(16)
    try:
        for _ in range(warmup):
            _, returncode = run_once(pinned, binary, identity, environment)
            if returncode != 0:
                raise SystemExit(f"warmup exited {returncode}")

        samples = []
        for _ in range(runs):
            elapsed_ns, returncode = run_once(pinned, binary, identity, environment)
            if returncode != 0:
                raise SystemExit(f"benchmark exited {returncode}")
            samples.append(elapsed_ns)

        collected_binary = {
            "path": identity["canonical_executable"],
            "bytes": identity["bytes"],
            "sha256": identity["sha256"],
        }
        ordered = sorted(samples)
        p95_index = min(len(ordered) - 1, (len(ordered) * 95 + 99) // 100 - 1)
        evidence = {
            "schema_version": 1,
            "classification": "bootstrap-infrastructure-only",
            "git_sha": repository_head(Path.cwd()),
            "host": {
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "python": platform.python_version(),
            },
            "binary": collected_binary,
            "command": [str(binary)],
            "warmup": warmup,
            "samples_ns": samples,
            "median_ns": integer_median(samples),
            "p95_ns": ordered[p95_index],
        }
        pinned.verify()
        verify_executable_identity(identity)
        pinned.close()
        pinned = None
    except BaseException:
        if pinned is not None:
            try:
                pinned.close()
            except BaseException:
                pass
        raise
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    args = parser.parse_args()

    try:
        binary = args.binary.resolve()
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"binary is not executable: {binary}")
        if args.runs < 10 or args.warmup < 1:
            parser.error("runs must be >= 10 and warmup must be >= 1")

        requested_output = args.output.absolute()
        output = requested_output.parent.resolve() / requested_output.name
        evidence = collect_and_publish_evidence(
            output,
            lambda: collect_evidence(binary, args.runs, args.warmup),
        )
    except (OSError, subprocess.SubprocessError, RuntimeError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    print(
        json.dumps(
            {
                "median_ns": evidence["median_ns"],
                "p95_ns": evidence["p95_ns"],
                "bytes": evidence["binary"]["bytes"],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
