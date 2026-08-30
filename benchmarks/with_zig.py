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


VALIDATED_UPSTREAM_OPTIONS = (
    "--output",
    "--runner-class",
    "--scratch-dir",
    "--upstream-dir",
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
    if any(
        argument.split("=", 1)[0] in {"--z", "--zi", "--zig"}
        for argument in options.upstream_arguments
    ):
        parser.error("the wrapper exclusively owns the forwarded --zig option")
    if options.validate_evidence is not None:
        bind_validation_to_collection(parser, options)
    return options


def forwarded_option(
    parser: argparse.ArgumentParser, arguments: Sequence[str], name: str
) -> str:
    values: list[str] = []
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument == name:
            if index + 1 == len(arguments):
                parser.error(f"forwarded {name} requires a value")
            values.append(arguments[index + 1])
            index += 2
            continue
        prefix = f"{name}="
        if argument.startswith(prefix):
            values.append(argument[len(prefix) :])
        index += 1
    if len(values) != 1 or not values[0]:
        parser.error(f"evidence validation requires exactly one forwarded {name}")
    return values[0]


def canonical_output_path(path: Path) -> Path:
    requested = path.absolute()
    return requested.parent.resolve() / requested.name


def bind_validation_to_collection(
    parser: argparse.ArgumentParser, options: argparse.Namespace
) -> None:
    forwarded = {
        name: forwarded_option(parser, options.upstream_arguments, name)
        for name in VALIDATED_UPSTREAM_OPTIONS
    }
    output = canonical_output_path(Path(forwarded["--output"]))
    upstream = Path(forwarded["--upstream-dir"]).resolve()
    scratch = Path(forwarded["--scratch-dir"]).resolve()
    if canonical_output_path(options.validate_evidence) != output:
        parser.error("validation evidence must be the forwarded collection output")
    if options.expected_runner_class != forwarded["--runner-class"]:
        parser.error("validation runner class must match the forwarded collection runner")
    if options.fx_binary.resolve() != upstream / "zig-out/bin/fx":
        parser.error("validation fx binary must belong to the forwarded upstream directory")
    if (
        options.machine_god_binary.resolve()
        != scratch / "machine-target/release/machine-god"
    ):
        parser.error(
            "validation machine-god binary must belong to the forwarded scratch directory"
        )
    options.validate_evidence = output
    options.fx_binary = upstream / "zig-out/bin/fx"
    options.machine_god_binary = scratch / "machine-target/release/machine-god"


def upstream_command(zig: Path, arguments: Sequence[str]) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/upstream.py"),
        *arguments,
        "--zig",
        str(zig),
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
