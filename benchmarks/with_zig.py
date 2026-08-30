#!/usr/bin/env python3
"""Run the pinned upstream harness with an ephemeral exact Zig toolchain."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.dont_write_bytecode = True
os.environ["PYTHONDONTWRITEBYTECODE"] = "1"

from scripts.provision_zig import (  # noqa: E402
    ProvisionError,
    host_spec,
    provisioned_zig,
)


def default_cache_root() -> Path:
    return Path(tempfile.gettempdir()) / f"machine-god-zig-{os.getuid()}"


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cache-root",
        type=Path,
        default=default_cache_root(),
        help="checksum-verified archive cache (default: a private OS temporary path)",
    )
    parser.add_argument(
        "--validate-evidence",
        type=Path,
        help="validate the collected evidence before removing the exact Zig toolchain",
    )
    parser.add_argument("--expected-git-sha")
    parser.add_argument("--expected-runner-class")
    parser.add_argument("--fx-binary", type=Path)
    parser.add_argument("--machine-god-binary", type=Path)
    parser.add_argument(
        "upstream_arguments",
        nargs=argparse.REMAINDER,
        help="arguments forwarded to benchmarks/upstream.py after --",
    )
    options = parser.parse_args(arguments)
    if options.upstream_arguments[:1] == ["--"]:
        options.upstream_arguments = options.upstream_arguments[1:]
    validation_values = (
        options.validate_evidence,
        options.expected_git_sha,
        options.expected_runner_class,
        options.fx_binary,
        options.machine_god_binary,
    )
    if any(value is not None for value in validation_values) and not all(
        value is not None for value in validation_values
    ):
        parser.error("evidence validation requires all five validation options")
    return options


def upstream_command(zig: Path, arguments: Sequence[str]) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/upstream.py"),
        "--zig",
        str(zig),
        *arguments,
    ]


def validation_command(options: argparse.Namespace) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/check.py"),
        str(options.validate_evidence),
        "--expected-git-sha",
        options.expected_git_sha,
        "--expected-runner-class",
        options.expected_runner_class,
        "--fx-binary",
        str(options.fx_binary.resolve(strict=True)),
        "--machine-god-binary",
        str(options.machine_god_binary.resolve(strict=True)),
    ]


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if arguments is None else arguments)
    try:
        with provisioned_zig(options.cache_root, host_spec()) as zig:
            completed = subprocess.run(
                upstream_command(zig, options.upstream_arguments), check=False
            )
            if completed.returncode == 0 and options.validate_evidence is not None:
                completed = subprocess.run(validation_command(options), check=False)
    except (OSError, ProvisionError) as error:
        print(f"could not run pinned upstream benchmark: {error}", file=sys.stderr)
        return 1
    return completed.returncode if 0 <= completed.returncode <= 125 else 1


if __name__ == "__main__":
    raise SystemExit(main())
