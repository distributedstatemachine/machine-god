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
        "upstream_arguments",
        nargs=argparse.REMAINDER,
        help="arguments forwarded to benchmarks/upstream.py after --",
    )
    options = parser.parse_args(arguments)
    if options.upstream_arguments[:1] == ["--"]:
        options.upstream_arguments = options.upstream_arguments[1:]
    return options


def upstream_command(zig: Path, arguments: Sequence[str]) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/upstream.py"),
        "--zig",
        str(zig),
        *arguments,
    ]


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if arguments is None else arguments)
    try:
        with provisioned_zig(options.cache_root, host_spec()) as zig:
            completed = subprocess.run(
                upstream_command(zig, options.upstream_arguments), check=False
            )
    except (OSError, ProvisionError) as error:
        print(f"could not run pinned upstream benchmark: {error}", file=sys.stderr)
        return 1
    return completed.returncode if 0 <= completed.returncode <= 125 else 1


if __name__ == "__main__":
    raise SystemExit(main())
