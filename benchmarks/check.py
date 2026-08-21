#!/usr/bin/env python3
"""Validate bootstrap or pinned-upstream evidence before retention."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import shutil
import stat
import subprocess
from pathlib import Path
from typing import BinaryIO


def file_sha256(source: BinaryIO, expected_bytes: int) -> str:
    digest = hashlib.sha256()
    remaining = expected_bytes
    while remaining:
        chunk = source.read(min(remaining, 1024 * 1024))
        if not chunk:
            raise ValueError("supplied binary became shorter during inspection")
        digest.update(chunk)
        remaining -= len(chunk)
    if source.read(1):
        raise ValueError("supplied binary became longer during inspection")
    return digest.hexdigest()


def require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise SystemExit(f"{field} must be a non-empty string")
    return value


def is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def reject_duplicate_members(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for name, value in pairs:
        if name in result:
            raise ValueError(f"duplicate JSON object name: {name}")
        result[name] = value
    return result


def reject_nonfinite_constant(value: str) -> object:
    raise ValueError(f"non-finite JSON number: {value}")


def parse_finite_float(value: str) -> float:
    result = float(value)
    if not math.isfinite(result):
        raise ValueError(f"non-finite JSON number: {value}")
    return result


def integer_median(samples: list[int]) -> int:
    ordered = sorted(samples)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[middle]
    return (ordered[middle - 1] + ordered[middle]) // 2


def resolve_evidence_path(value: object, field: str) -> Path:
    text = require_text(value, field)
    try:
        return Path(text).resolve()
    except (OSError, RuntimeError, ValueError):
        raise SystemExit(f"{field} is not a valid filesystem path") from None


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
        data = json.loads(
            args.evidence.read_text(encoding="utf-8"),
            object_pairs_hook=reject_duplicate_members,
            parse_constant=reject_nonfinite_constant,
            parse_float=parse_finite_float,
        )
    except (OSError, UnicodeError, ValueError) as error:
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

    expected_schema_one_keys = {
        "schema_version",
        "classification",
        "git_sha",
        "host",
        "binary",
        "command",
        "warmup",
        "samples_ns",
        "median_ns",
        "p95_ns",
    }
    if set(data) != expected_schema_one_keys:
        raise SystemExit("schema 1 evidence fields are not canonical")
    if not is_integer(data.get("schema_version")) or data["schema_version"] != 1:
        raise SystemExit("unsupported benchmark schema")
    if data.get("classification") != "bootstrap-infrastructure-only":
        raise SystemExit("schema 1 evidence must be bootstrap-only")
    if args.fx_binary or args.machine_god_binary or args.expected_runner_class:
        raise SystemExit("schema 1 bootstrap evidence accepts only --binary")
    git_sha = require_text(data.get("git_sha"), "git_sha")
    if len(git_sha) not in (40, 64) or any(character not in "0123456789abcdef" for character in git_sha.lower()):
        raise SystemExit("git_sha must be a hexadecimal Git object ID")
    if args.expected_git_sha and git_sha != args.expected_git_sha:
        raise SystemExit("benchmark git_sha does not match the expected CI SHA")
    host = data.get("host")
    if not isinstance(host, dict) or set(host) != {
        "system",
        "release",
        "machine",
        "python",
    }:
        raise SystemExit("host metadata is missing")
    for field in ("system", "release", "machine", "python"):
        require_text(host.get(field), f"host.{field}")
    command = data.get("command")
    if not isinstance(command, list) or len(command) != 1:
        raise SystemExit("command must contain exactly one executable")
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
    expected_median = integer_median(samples)
    p95_index = min(len(ordered) - 1, (len(ordered) * 95 + 99) // 100 - 1)
    median_ns = data.get("median_ns")
    p95_ns = data.get("p95_ns")
    if not is_integer(median_ns) or median_ns != expected_median:
        raise SystemExit("median_ns does not match samples")
    if not is_integer(p95_ns) or p95_ns != ordered[p95_index]:
        raise SystemExit("p95_ns does not match samples")
    binary = data.get("binary")
    if not isinstance(binary, dict) or set(binary) != {"path", "bytes", "sha256"}:
        raise SystemExit("binary metadata fields are not canonical")
    if not is_integer(binary.get("bytes")) or binary["bytes"] <= 0:
        raise SystemExit("binary size is missing")
    checksum = binary.get("sha256")
    if not isinstance(checksum, str) or len(checksum) != 64 or any(
        character not in "0123456789abcdef" for character in checksum.lower()
    ):
        raise SystemExit("binary checksum is missing")
    recorded_binary = resolve_evidence_path(binary.get("path"), "binary.path")
    recorded_command = resolve_evidence_path(command[0], "command[0]")
    if recorded_command != recorded_binary:
        raise SystemExit("command executable does not match binary.path")
    if args.binary:
        try:
            actual_binary = args.binary.resolve()
        except (OSError, RuntimeError, ValueError) as error:
            raise SystemExit(f"supplied binary path is invalid: {error}") from None
        if recorded_binary != actual_binary:
            raise SystemExit("recorded binary path does not match supplied binary")
        open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
        open_flags |= getattr(os, "O_CLOEXEC", 0)
        open_flags |= getattr(os, "O_NOFOLLOW", 0)
        open_flags |= getattr(os, "O_NONBLOCK", 0)
        try:
            descriptor = os.open(actual_binary, open_flags)
        except (OSError, RuntimeError, ValueError) as error:
            raise SystemExit(f"failed to inspect supplied binary: {error}") from None
        try:
            actual_metadata = os.fstat(descriptor)
        except (OSError, RuntimeError, ValueError) as error:
            os.close(descriptor)
            raise SystemExit(f"failed to inspect supplied binary: {error}") from None
        if not stat.S_ISREG(actual_metadata.st_mode):
            os.close(descriptor)
            raise SystemExit("supplied binary is not a regular file")
        if actual_metadata.st_size != binary["bytes"]:
            os.close(descriptor)
            raise SystemExit("binary size does not match evidence")
        execute_bits = stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        lacks_posix_execute_mode = (
            os.name == "posix" and not actual_metadata.st_mode & execute_bits
        )
        if lacks_posix_execute_mode or not os.access(actual_binary, os.X_OK):
            os.close(descriptor)
            raise SystemExit("supplied binary is not executable")
        try:
            with os.fdopen(descriptor, "rb", closefd=False) as source:
                actual_checksum = file_sha256(source, binary["bytes"])
        except (OSError, RuntimeError, ValueError) as error:
            raise SystemExit(f"failed to inspect supplied binary: {error}") from None
        finally:
            os.close(descriptor)
        if actual_checksum != checksum:
            raise SystemExit("binary checksum does not match evidence")
    print("benchmark evidence is valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
