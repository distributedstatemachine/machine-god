#!/usr/bin/env python3
"""Validate benchmark evidence before it is retained by CI."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--bootstrap", action="store_true")
    args = parser.parse_args()
    data = json.loads(args.evidence.read_text(encoding="utf-8"))

    if data.get("schema_version") != 1:
        raise SystemExit("unsupported benchmark schema")
    samples = data.get("samples_ns")
    if not isinstance(samples, list) or len(samples) < 10:
        raise SystemExit("benchmark evidence needs at least 10 samples")
    if any(not isinstance(value, int) or value <= 0 for value in samples):
        raise SystemExit("benchmark samples must be positive integer nanoseconds")
    if args.bootstrap and data.get("classification") != "bootstrap-infrastructure-only":
        raise SystemExit("bootstrap evidence must not claim product performance")
    binary = data.get("binary", {})
    if not isinstance(binary.get("bytes"), int) or binary["bytes"] <= 0:
        raise SystemExit("binary size is missing")
    if not isinstance(binary.get("sha256"), str) or len(binary["sha256"]) != 64:
        raise SystemExit("binary checksum is missing")
    print("benchmark evidence is valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

