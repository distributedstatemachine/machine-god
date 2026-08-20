#!/usr/bin/env python3
"""Collect reproducible bootstrap timing and binary-size evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
import time
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_once(binary: Path) -> tuple[int, int]:
    start = time.perf_counter_ns()
    completed = subprocess.run(
        [str(binary)],
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return time.perf_counter_ns() - start, completed.returncode


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

    for _ in range(args.warmup):
        _, returncode = run_once(binary)
        if returncode != 0:
            raise SystemExit(f"warmup exited {returncode}")

    samples = []
    for _ in range(args.runs):
        elapsed_ns, returncode = run_once(binary)
        if returncode != 0:
            raise SystemExit(f"benchmark exited {returncode}")
        samples.append(elapsed_ns)

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
        "binary": {
            "path": str(binary),
            "bytes": binary.stat().st_size,
            "sha256": sha256(binary),
        },
        "command": [str(binary)],
        "warmup": args.warmup,
        "samples_ns": samples,
        "median_ns": int(statistics.median(samples)),
        "p95_ns": ordered[p95_index],
    }
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

