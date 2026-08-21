#!/usr/bin/env python3
"""Validate bootstrap or pinned-upstream evidence before retention."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import statistics
import subprocess
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
    parser.add_argument("--fx-binary", type=Path)
    parser.add_argument("--machine-god-binary", type=Path)
    parser.add_argument("--expected-runner-class")
    args = parser.parse_args()
    try:
        data = json.loads(args.evidence.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid benchmark evidence: {error}") from None
    if not isinstance(data, dict):
        raise SystemExit("benchmark evidence must be an object")

    if data.get("schema_version") == 2:
        if args.bootstrap:
            raise SystemExit("schema 2 upstream evidence does not use --bootstrap")
        if args.binary:
            raise SystemExit("schema 2 evidence binds both binaries in its build records")
        if not args.fx_binary or not args.machine_god_binary:
            raise SystemExit("schema 2 validation requires both actual binaries")
        if not args.expected_runner_class:
            raise SystemExit("schema 2 validation requires --expected-runner-class")
        if not args.expected_git_sha:
            raise SystemExit("schema 2 validation requires --expected-git-sha")
        try:
            from upstream import (
                canonical_git_entries_sha256,
                executable_identity,
                machine_tree_command,
                parse_upstream_lock,
                parse_git_tree_listing,
                sha256_file,
                validate_upstream_evidence,
                verify_executable_identity,
            )

            canonical_lock_path = Path(__file__).resolve().parent / "upstream.lock"
            canonical_lock = parse_upstream_lock(canonical_lock_path)
            canonical_root = Path(__file__).resolve().parents[1]
            discovered_git = shutil.which("git")
            if discovered_git is None:
                raise ValueError("git is required to bind the machine source tree")
            git = str(Path(discovered_git).resolve(strict=True))
            git_identity = executable_identity(Path(git))
            git_environment = {
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
                "HOME": str(canonical_root),
                "PATH": os.environ.get("PATH", ""),
            }
            expected_machine_tree = subprocess.check_output(
                [
                    git,
                    "-c",
                    "core.hooksPath=/dev/null",
                    "rev-parse",
                    f"{args.expected_git_sha}^{{tree}}",
                ],
                cwd=canonical_root,
                env=git_environment,
                text=True,
                timeout=10,
            ).strip()
            verify_executable_identity(git_identity)
            listing = subprocess.check_output(
                machine_tree_command(git, args.expected_git_sha),
                cwd=canonical_root,
                env=git_environment,
                timeout=10,
            )
            verify_executable_identity(git_identity)
            expected_machine_manifest_sha256 = canonical_git_entries_sha256(
                parse_git_tree_listing(listing)
            )
            validate_upstream_evidence(
                data,
                expected_lock=canonical_lock,
                expected_lock_path=canonical_lock_path,
                expected_lock_sha256=sha256_file(canonical_lock_path),
                expected_root=canonical_root,
                expected_runner_class=args.expected_runner_class,
                expected_machine_tree=expected_machine_tree,
                expected_machine_manifest_sha256=expected_machine_manifest_sha256,
                expected_binaries={
                    "fx": args.fx_binary,
                    "machine-god": args.machine_god_binary,
                },
            )
        except (OSError, subprocess.SubprocessError, TypeError, ValueError) as error:
            raise SystemExit(str(error)) from error
        if (
            args.expected_git_sha
            and data["source"]["machine_god"]["git_sha"] != args.expected_git_sha
        ):
            raise SystemExit("benchmark git_sha does not match the expected CI SHA")
        print("upstream benchmark evidence is valid")
        return 0

    if not is_integer(data.get("schema_version")) or data["schema_version"] != 1:
        raise SystemExit("unsupported benchmark schema")
    if args.fx_binary or args.machine_god_binary or args.expected_runner_class:
        raise SystemExit("schema 1 bootstrap evidence accepts only --binary")
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
