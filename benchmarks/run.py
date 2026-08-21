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
    executable_identity,
    pin_executable,
    run_process,
    verify_executable_identity,
)


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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"binary is not executable: {binary}")
    if args.runs < 10 or args.warmup < 1:
        parser.error("runs must be >= 10 and warmup must be >= 1")

    identity = executable_identity(binary)
    pinned = pin_executable(identity)
    environment = os.environ.copy()
    environment[CONTAINMENT_ENVIRONMENT_KEY] = secrets.token_hex(16)
    try:
        for _ in range(args.warmup):
            _, returncode = run_once(pinned, binary, identity, environment)
            if returncode != 0:
                raise SystemExit(f"warmup exited {returncode}")

        samples = []
        for _ in range(args.runs):
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
            "git_sha": subprocess.check_output(
                ["git", "rev-parse", "HEAD"], text=True
            ).strip(),
            "host": {
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "python": platform.python_version(),
            },
            "binary": collected_binary,
            "command": [str(binary)],
            "warmup": args.warmup,
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

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
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
