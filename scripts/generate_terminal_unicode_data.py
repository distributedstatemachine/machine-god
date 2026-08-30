#!/usr/bin/env python3
"""Transliterate fx's generated Unicode terminal tables into Rust.

The input is the pinned fx `src/core/shared/unicode_display_data.zig` file.
This mechanical converter deliberately understands only that generated file's
small declaration grammar; it performs no network access.
"""

from __future__ import annotations

import argparse
import pathlib
import re


ARRAYS = (
    ("wide_ranges", "Range"),
    ("emoji_presentation_ranges", "Range"),
    ("emoji_modifier_ranges", "Range"),
    ("variation_bases", "Range"),
    ("rgi_trie_nodes", "TrieNode"),
    ("rgi_trie_edges", "TrieEdge"),
)


def extract_array(source: str, name: str) -> str:
    match = re.search(
        rf"pub const {re.escape(name)} = \[_\]\w+\{{\n(.*?)\n\}};",
        source,
        flags=re.DOTALL,
    )
    if match is None:
        raise ValueError(f"missing generated array: {name}")
    return match.group(1)


def convert_entries(body: str, item_type: str) -> list[str]:
    entries = []
    for raw in body.splitlines():
        line = raw.strip()
        if not line:
            continue
        converted = line.replace(".{", f"{item_type} {{")
        converted = re.sub(r"\.([a-z_]+)\s*=", r"\1:", converted)
        entries.append(f"    {converted}")
    return entries


def render(source: str) -> str:
    output = [
        "// SPDX-License-Identifier: Apache-2.0",
        "//",
        "// Mechanically transliterated from vercel-labs/fx at revision",
        "// b1774fbf6c7602b503026f96f6e960e946c692ef:",
        "// src/core/shared/unicode_display_data.zig.",
        "// The upstream provenance and Unicode input hashes are retained below.",
    ]
    for line in source.splitlines():
        if line.startswith("//!"):
            output.append("//" + line[3:])
        elif line == "":
            if len(output) > 6:
                break
        else:
            break
    output.extend(
        [
            "",
            "#[derive(Clone, Copy)]",
            "pub(super) struct Range {",
            "    pub(super) first: u32,",
            "    pub(super) last: u32,",
            "}",
            "",
            "#[derive(Clone, Copy)]",
            "pub(super) struct TrieNode {",
            "    pub(super) edge_start: u32,",
            "    pub(super) edge_len: u16,",
            "    pub(super) terminal: bool,",
            "}",
            "",
            "#[derive(Clone, Copy)]",
            "pub(super) struct TrieEdge {",
            "    pub(super) codepoint: u32,",
            "    pub(super) child: u32,",
            "}",
            "",
        ]
    )
    for name, item_type in ARRAYS:
        entries = convert_entries(extract_array(source, name), item_type)
        output.append("#[rustfmt::skip]")
        output.append(
            f"pub(super) static {name.upper()}: [{item_type}; {len(entries)}] = ["
        )
        output.extend(entries)
        output.extend(["];"])
        output.append("")
    output.append("pub(super) const MAX_RGI_SEQUENCE_CODEPOINTS: usize = 10;")
    output.append("")
    return "\n".join(output)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = render(args.input.read_text(encoding="utf-8"))
    if args.check:
        if args.output.read_text(encoding="utf-8") != generated:
            raise SystemExit("generated terminal Unicode data is stale")
    else:
        args.output.write_text(generated, encoding="utf-8")


if __name__ == "__main__":
    main()
