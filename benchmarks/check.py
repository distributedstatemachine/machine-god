#!/usr/bin/env python3
"""Validate benchmark evidence before it is retained by CI."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise SystemExit(f"{field} must be a non-empty string")
    return value


def is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--bootstrap", action="store_true")
    parser.add_argument("--expected-git-sha")
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    data = json.loads(args.evidence.read_text(encoding="utf-8"))

    if not is_integer(data.get("schema_version")) or data["schema_version"] != 1:
        raise SystemExit("unsupported benchmark schema")
    git_sha = require_text(data.get("git_sha"), "git_sha")
    if len(git_sha) not in (40, 64) or any(character not in "0123456789abcdef" for character in git_sha.lower()):
        raise SystemExit("git_sha must be a hexadecimal Git object ID")
    if args.expected_git_sha and git_sha != args.expected_git_sha:
        raise SystemExit("benchmark git_sha does not match the expected CI SHA")
    host = data.get("host")
    if not isinstance(host, dict):
        raise SystemExit("host metadata is missing")
    for field in ("system", "release", "machine", "python"):
        require_text(host.get(field), f"host.{field}")
    command = data.get("command")
    if not isinstance(command, list) or not command:
        raise SystemExit("command must be a non-empty list")
    for index, argument in enumerate(command):
        require_text(argument, f"command[{index}]")
    warmup = data.get("warmup")
    if not isinstance(warmup, int) or isinstance(warmup, bool) or warmup < 1:
        raise SystemExit("warmup must be a positive integer")
    samples = data.get("samples_ns")
    if not isinstance(samples, list) or len(samples) < 10:
        raise SystemExit("benchmark evidence needs at least 10 samples")
    if any(not is_integer(value) or value <= 0 for value in samples):
        raise SystemExit("benchmark samples must be positive integer nanoseconds")
    ordered = sorted(samples)
    expected_median = int(statistics.median(samples))
    p95_index = min(len(ordered) - 1, (len(ordered) * 95 + 99) // 100 - 1)
    median_ns = data.get("median_ns")
    p95_ns = data.get("p95_ns")
    if not is_integer(median_ns) or median_ns != expected_median:
        raise SystemExit("median_ns does not match samples")
    if not is_integer(p95_ns) or p95_ns != ordered[p95_index]:
        raise SystemExit("p95_ns does not match samples")
    if args.bootstrap and data.get("classification") != "bootstrap-infrastructure-only":
        raise SystemExit("bootstrap evidence must not claim product performance")
    binary = data.get("binary", {})
    if not is_integer(binary.get("bytes")) or binary["bytes"] <= 0:
        raise SystemExit("binary size is missing")
    checksum = binary.get("sha256")
    if not isinstance(checksum, str) or len(checksum) != 64 or any(
        character not in "0123456789abcdef" for character in checksum.lower()
    ):
        raise SystemExit("binary checksum is missing")
    recorded_binary = Path(require_text(binary.get("path"), "binary.path")).resolve()
    recorded_command = Path(command[0]).resolve()
    if recorded_command != recorded_binary:
        raise SystemExit("command executable does not match binary.path")
    if args.binary:
        actual_binary = args.binary.resolve()
        if recorded_binary != actual_binary:
            raise SystemExit("recorded binary path does not match supplied binary")
        if actual_binary.stat().st_size != binary["bytes"]:
            raise SystemExit("binary size does not match evidence")
        if file_sha256(actual_binary) != checksum:
            raise SystemExit("binary checksum does not match evidence")
    print("benchmark evidence is valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
