#!/usr/bin/env python3
"""Collect reproducible bootstrap timing and binary-size evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import stat
import subprocess
import time
from pathlib import Path
from typing import BinaryIO


def bounded_sha256(source: BinaryIO, expected_bytes: int) -> str:
    digest = hashlib.sha256()
    remaining = expected_bytes
    while remaining:
        chunk = source.read(min(remaining, 1024 * 1024))
        if not chunk:
            raise RuntimeError("binary became shorter while inspected")
        digest.update(chunk)
        remaining -= len(chunk)
    if source.read(1):
        raise RuntimeError("binary became longer while inspected")
    return digest.hexdigest()


def stat_identity(metadata: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def binary_record(binary: Path) -> dict[str, object]:
    before_path = binary.lstat()
    open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    open_flags |= getattr(os, "O_CLOEXEC", 0)
    open_flags |= getattr(os, "O_NOFOLLOW", 0)
    open_flags |= getattr(os, "O_NONBLOCK", 0)
    descriptor = os.open(binary, open_flags)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or not before.st_mode & 0o111:
            raise RuntimeError(f"binary is not executable: {binary}")
        if stat_identity(before_path) != stat_identity(before):
            raise RuntimeError("binary path changed before inspection")
        if not os.access(binary, os.X_OK):
            raise RuntimeError(f"binary is not executable: {binary}")
        with os.fdopen(descriptor, "rb", buffering=0, closefd=False) as source:
            checksum = bounded_sha256(source, before.st_size)
        after = os.fstat(descriptor)
        path_after = binary.lstat()
        if stat_identity(before_path) != stat_identity(path_after):
            raise RuntimeError("binary path changed while inspected")
        if stat_identity(before) != stat_identity(after):
            raise RuntimeError("binary changed while inspected")
        return {"path": str(binary), "bytes": before.st_size, "sha256": checksum}
    finally:
        os.close(descriptor)


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
    collected_binary = binary_record(binary)
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
