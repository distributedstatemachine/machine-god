#!/usr/bin/env python3
"""Generate the pinned fx compatibility inventory and its human-readable view.

The generator is deliberately offline. It consumes a local checkout whose HEAD
matches ``benchmarks/upstream.lock`` and rejects relevant uncommitted changes.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LOCK = ROOT / "benchmarks/upstream.lock"
DEFAULT_POLICY = ROOT / "compatibility/policy.json"
DEFAULT_INVENTORY = ROOT / "compatibility/inventory.json"
DEFAULT_DOCS = ROOT / "docs/compatibility.md"

COMMAND_SPECS = "src/core/slash_commands/command_specs.zig"
COMMAND_REGISTRY = "src/builtins/commands.zig"
TOOL_REGISTRY = "src/builtins/tools.zig"
SDK_PACKAGE = "sdk/package.json"
SDK_MODULES = ("sdk/fx-sdk.js", "sdk/node.js", "sdk/browser.js")
E2E_CORPUS = "scripts/pgso/corpus.json"
SOURCE_FILES = (
    COMMAND_SPECS,
    COMMAND_REGISTRY,
    TOOL_REGISTRY,
    SDK_PACKAGE,
    *SDK_MODULES,
    E2E_CORPUS,
)
SURFACE_NAMES = (
    "top_level_cli_commands",
    "slash_command_kinds",
    "builtin_tool_names",
    "sdk_exports",
    "e2e_owners",
)
ALLOWED_STATUSES = {"planned", "deferred", "intentional_difference"}
REGULAR_BLOB_MODES = {"100644", "100755"}
ZIG_IDENTIFIER_PATTERN = r'(?:@"[A-Za-z_][A-Za-z0-9_]*"|[A-Za-z_][A-Za-z0-9_]*)'
JS_IDENTIFIER_PATTERN = r"[A-Za-z_$][A-Za-z0-9_$]*"
JS_VARIABLE_EXPORT = re.compile(r"export\s+(?P<kind>const|let|var)\s+")
JS_EXPORT_LIST_START = re.compile(r"export\s*\{")
JS_EXPORT_FROM = re.compile(r'from\s*(?P<quote>")')
CLI_TOKEN = re.compile(r"[a-z][a-z0-9-]*")
CLI_ALIAS = re.compile(r"(?:-{1,2})?[A-Za-z0-9][A-Za-z0-9-]*")
SLASH_COMMAND = re.compile(r"/[a-z0-9][a-z0-9_-]*(?: [a-z0-9][a-z0-9_-]*)*")
TOOL_NAME = re.compile(r"[a-z][a-z0-9_]*")
E2E_FILE = re.compile(r"[a-z0-9][a-z0-9-]*\.test\.ts")
SCENARIO_NAME = re.compile(r"[a-z0-9][a-z0-9-]*")
PACKAGE_NAME = re.compile(r"(?:@[a-z0-9][a-z0-9._-]*/)?[a-z0-9][a-z0-9._-]*")
PACKAGE_SPECIFIER = re.compile(r"\.|\./[A-Za-z0-9][A-Za-z0-9._/-]*")
PACKAGE_CONDITION = re.compile(r"[A-Za-z][A-Za-z0-9_-]*")
PACKAGE_TARGET = re.compile(r"\./[A-Za-z0-9][A-Za-z0-9._/-]*\.js")


class InventoryError(RuntimeError):
    """Raised when an upstream source no longer matches the expected contract."""


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require_pattern(value: str, pattern: re.Pattern[str], label: str) -> str:
    if pattern.fullmatch(value) is None:
        raise InventoryError(f"unsupported {label} {value!r}")
    return value


class GitSnapshot:
    """Canonical bytes and tree metadata from one immutable Git commit."""

    def __init__(self, repository: Path, commit: str, *, require_head: bool = True) -> None:
        self.repository = repository.resolve()
        self.commit = commit
        self._blobs: dict[str, tuple[str, str, bytes]] = {}
        object_format = self._git("rev-parse", "--show-object-format").decode(
            "ascii", errors="strict"
        ).strip()
        if object_format not in {"sha1", "sha256"}:
            raise InventoryError(f"unsupported Git object format {object_format!r}")
        self.object_format = object_format
        resolved = self._git("rev-parse", "--verify", f"{commit}^{{commit}}").decode(
            "ascii", errors="strict"
        ).strip()
        if resolved != commit:
            raise InventoryError(f"pinned object resolves to {resolved}, expected {commit}")
        if require_head:
            head = self._git("rev-parse", "HEAD").decode("ascii", errors="strict").strip()
            if head != commit:
                raise InventoryError(
                    f"upstream checkout HEAD is {head}, expected pinned commit {commit}"
                )

    def _git(self, *args: str) -> bytes:
        environment = {
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_NO_LAZY_FETCH": "1",
            "GIT_NO_REPLACE_OBJECTS": "1",
            "GIT_OPTIONAL_LOCKS": "0",
            "GIT_TERMINAL_PROMPT": "0",
            "HOME": os.devnull,
            "LANG": "C",
            "LC_ALL": "C",
            "PATH": os.environ.get("PATH", os.defpath),
            "XDG_CONFIG_HOME": os.devnull,
        }
        completed = subprocess.run(
            [
                "git",
                "--no-replace-objects",
                "-c",
                "core.askPass=",
                "-c",
                "credential.helper=",
                "-c",
                "http.extraHeader=",
                "-c",
                "http.proxy=",
                "-c",
                "protocol.allow=never",
                "-C",
                str(self.repository),
                *args,
            ],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
        )
        if completed.returncode != 0:
            detail = completed.stderr.decode("utf-8", errors="replace").strip()
            raise InventoryError(
                f"git {' '.join(args)} failed for {self.repository}: {detail}"
            )
        return completed.stdout

    @staticmethod
    def _parse_tree_record(record: bytes) -> tuple[str, str, str, str]:
        try:
            metadata, raw_path = record.split(b"\t", 1)
            mode, object_type, object_id = metadata.decode("ascii").split(" ")
            path = raw_path.decode("utf-8", errors="strict")
        except (UnicodeError, ValueError) as error:
            raise InventoryError("unsupported Git tree record") from error
        if re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", object_id) is None:
            raise InventoryError(f"invalid Git object ID for {path}")
        return mode, object_type, object_id, path

    @staticmethod
    def _require_regular_blob(mode: str, object_type: str, path: str) -> None:
        if object_type != "blob" or mode not in REGULAR_BLOB_MODES:
            raise InventoryError(
                f"upstream source {path} must be a regular blob, found mode={mode} "
                f"type={object_type}"
            )

    def blob(self, path: str) -> tuple[str, str, bytes]:
        cached = self._blobs.get(path)
        if cached is not None:
            return cached
        output = self._git("ls-tree", "-z", "--full-tree", self.commit, "--", path)
        records = [record for record in output.split(b"\0") if record]
        if len(records) != 1:
            raise InventoryError(
                f"upstream commit must contain exactly one tree entry for {path}, "
                f"found {len(records)}"
            )
        mode, object_type, object_id, actual_path = self._parse_tree_record(records[0])
        if actual_path != path:
            raise InventoryError(f"Git tree returned {actual_path!r} for requested {path!r}")
        self._require_regular_blob(mode, object_type, path)
        data = self._git("cat-file", "blob", object_id)
        digest = hashlib.new(self.object_format)
        digest.update(f"blob {len(data)}\0".encode("ascii"))
        digest.update(data)
        if digest.hexdigest() != object_id:
            raise InventoryError(
                f"canonical bytes for {path} hash to {digest.hexdigest()}, "
                f"expected Git blob {object_id}"
            )
        cached = (mode, object_id, data)
        self._blobs[path] = cached
        return cached

    def text(self, path: str) -> str:
        try:
            return self.blob(path)[2].decode("utf-8", errors="strict")
        except UnicodeError as error:
            raise InventoryError(f"upstream source {path} is not valid UTF-8") from error

    def root_files(self, directory: str, suffix: str) -> list[str]:
        output = self._git(
            "ls-tree", "-r", "-z", "--full-tree", self.commit, "--", directory
        )
        prefix = f"{directory.rstrip('/')}/"
        names: list[str] = []
        for raw_record in output.split(b"\0"):
            if not raw_record:
                continue
            mode, object_type, _, path = self._parse_tree_record(raw_record)
            if not path.startswith(prefix):
                raise InventoryError(f"Git tree path escaped requested directory: {path}")
            relative = path[len(prefix) :]
            if "/" in relative or not relative.endswith(suffix):
                continue
            self._require_regular_blob(mode, object_type, path)
            names.append(relative)
        require_unique(names, f"{directory} tree member")
        return sorted(names)


def read_text(source: GitSnapshot, relative: str) -> str:
    return source.text(relative)


def parse_lock(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        raise InventoryError(f"cannot read upstream lock {path}: {error}") from error
    for line_number, raw_line in enumerate(lines, 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise InventoryError(f"{path}:{line_number}: expected key=value")
        key, value = (part.strip() for part in line.split("=", 1))
        if not key or not value:
            raise InventoryError(f"{path}:{line_number}: empty lock key or value")
        if key in values:
            raise InventoryError(f"{path}:{line_number}: duplicate lock key {key}")
        values[key] = value
    repository = values.get("repository", "")
    commit = values.get("commit", "")
    if repository != "https://github.com/vercel-labs/fx.git":
        raise InventoryError("upstream repository must be canonical vercel-labs/fx HTTPS URL")
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit):
        raise InventoryError("upstream commit must be a lowercase full Git object ID")
    return values


def source_mask(text: str, language: str) -> str:
    """Blank comments and literal contents while preserving offsets and newlines."""

    if language not in {"zig", "js"}:
        raise InventoryError(f"unsupported source language {language}")
    masked = list(text)
    index = 0
    js_expression_ended = False
    js_pending_control_parenthesis = False
    js_parentheses: list[bool] = []
    while index < len(text):
        following = text[index + 1] if index + 1 < len(text) else ""
        if language == "zig" and text[index] == "\\":
            if following != "\\":
                raise InventoryError("unsupported Zig backslash outside a literal")
            masked[index] = masked[index + 1] = " "
            index += 2
            while index < len(text) and text[index] not in "\r\n":
                masked[index] = " "
                index += 1
            continue
        if text[index] == "/" and following == "/":
            masked[index] = masked[index + 1] = " "
            index += 2
            while index < len(text) and text[index] not in "\r\n":
                masked[index] = " "
                index += 1
            continue
        if text[index] == "/" and following == "*":
            if language == "zig":
                raise InventoryError("Zig does not support block comments")
            masked[index] = masked[index + 1] = " "
            index += 2
            while index < len(text):
                following = text[index + 1] if index + 1 < len(text) else ""
                if text[index] == "*" and following == "/":
                    masked[index] = masked[index + 1] = " "
                    index += 2
                    break
                else:
                    if text[index] not in "\r\n":
                        masked[index] = " "
                    index += 1
            else:
                raise InventoryError("unterminated block comment")
            continue
        if language == "zig" and text[index] == "@" and following == '"':
            index += 2
            payload_start = index
            while index < len(text):
                character = text[index]
                if character in "\r\n":
                    raise InventoryError("unterminated quoted Zig identifier")
                if character == "\\":
                    raise InventoryError(
                        "escape-bearing quoted Zig identifiers are unsupported"
                    )
                if character == '"':
                    payload = text[payload_start:index]
                    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", payload):
                        masked[payload_start:index] = list(payload)
                    index += 1
                    break
                else:
                    masked[index] = "x"
                index += 1
            else:
                raise InventoryError("unterminated quoted Zig identifier")
            continue
        if text[index] in {'"', "'"}:
            quote = text[index]
            index += 1
            escaped = False
            while index < len(text):
                character = text[index]
                if character not in "\r\n":
                    masked[index] = " "
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == quote:
                    masked[index] = quote
                    index += 1
                    break
                index += 1
            else:
                raise InventoryError("unterminated string or character literal")
            if language == "js":
                js_expression_ended = True
            continue
        if language == "js" and text[index] == "`":
            masked[index] = " "
            index += 1
            escaped = False
            while index < len(text):
                character = text[index]
                if character not in "\r\n":
                    masked[index] = " "
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == "`":
                    index += 1
                    break
                index += 1
            else:
                raise InventoryError("unterminated JavaScript template literal")
            js_expression_ended = True
            continue
        if language == "js" and text[index] == "/":
            if js_expression_ended:
                previous = index - 1
                while previous >= 0 and masked[previous].isspace():
                    previous -= 1
                if previous >= 0 and masked[previous] == "}":
                    raise InventoryError(
                        "ambiguous JavaScript slash after a closing brace"
                    )
                js_expression_ended = False
                index += 1
                continue
            masked[index] = " "
            index += 1
            escaped = False
            in_character_class = False
            while index < len(text):
                character = text[index]
                if character in "\r\n":
                    raise InventoryError("unterminated JavaScript regular expression literal")
                masked[index] = " "
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == "[":
                    in_character_class = True
                elif character == "]" and in_character_class:
                    in_character_class = False
                elif character == "/" and not in_character_class:
                    index += 1
                    while index < len(text) and (
                        text[index].isascii() and text[index].isalpha()
                    ):
                        masked[index] = " "
                        index += 1
                    break
                index += 1
            else:
                raise InventoryError("unterminated JavaScript regular expression literal")
            js_expression_ended = True
            continue
        if language == "js" and (text[index].isalpha() or text[index] in "_$"):
            end = index + 1
            while end < len(text) and (
                text[end].isalnum() or text[end] in "_$"
            ):
                end += 1
            word = text[index:end]
            js_pending_control_parenthesis = word in {
                "catch",
                "for",
                "if",
                "switch",
                "while",
                "with",
            }
            js_expression_ended = word not in {
                "await",
                "case",
                "delete",
                "do",
                "else",
                "in",
                "instanceof",
                "new",
                "of",
                "return",
                "throw",
                "typeof",
                "void",
                "yield",
            } and not js_pending_control_parenthesis
            index = end
            continue
        if language == "js" and text[index].isdigit():
            end = index + 1
            while end < len(text) and (text[end].isalnum() or text[end] in "._"):
                end += 1
            js_expression_ended = True
            index = end
            continue
        if language == "js" and text[index] == "(":
            js_parentheses.append(js_pending_control_parenthesis)
            js_pending_control_parenthesis = False
            js_expression_ended = False
        elif language == "js" and text[index] == ")":
            control_parenthesis = js_parentheses.pop() if js_parentheses else False
            js_expression_ended = not control_parenthesis
        elif language == "js" and text[index] in "]}":
            js_expression_ended = True
        elif language == "js" and text[index] in "[{,;:=!?&|+*-%^~<>.":
            js_expression_ended = False
            js_pending_control_parenthesis = False
        elif language == "js" and not text[index].isspace():
            raise InventoryError(
                f"unsupported JavaScript token while scanning literals: {text[index]!r}"
            )
        index += 1
    return "".join(masked)


def find_matching_brace(mask: str, opening: int) -> int:
    if opening >= len(mask) or mask[opening] != "{":
        raise InventoryError("internal parser error: expected opening brace")
    depth = 0
    for index in range(opening, len(mask)):
        character = mask[index]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return index
            if depth < 0:
                break
    raise InventoryError("unterminated Zig initializer")


def initializer_body(
    text: str,
    language: str,
    declaration_pattern: str,
    label: str,
    *,
    masked_source: str | None = None,
    source_depths: list[int] | None = None,
) -> tuple[str, str]:
    mask = masked_source if masked_source is not None else source_mask(text, language)
    depths = source_depths if source_depths is not None else structural_depth_map(mask)
    if len(depths) != len(mask) + 1:
        raise InventoryError("internal parser error: invalid structural depth map")
    matches = list(re.finditer(declaration_pattern, mask))
    if any(depths[match.start()] != 0 for match in matches):
        raise InventoryError(f"upstream {label} must be declared at top level")
    if len(matches) > 1:
        raise InventoryError(f"multiple upstream declarations for {label}")
    match = matches[0] if matches else None
    if match is None:
        raise InventoryError(f"cannot find upstream {label}")
    opening = mask.find("{", match.start(), match.end())
    if opening < 0:
        raise InventoryError(f"matched upstream {label} has no initializer")
    if depths[opening] != 0:
        raise InventoryError(f"upstream {label} initializer escaped top-level scope")
    closing = find_matching_brace(mask, opening)
    if depths[closing] != 1:
        raise InventoryError(f"upstream {label} initializer escaped top-level scope")
    return text[opening + 1 : closing], mask[opening + 1 : closing]


def skip_space(mask: str, index: int) -> int:
    while index < len(mask) and mask[index].isspace():
        index += 1
    return index


def root_struct_blocks(body: str, mask: str) -> list[tuple[str, str, list[int]]]:
    depths = structural_depth_map(mask)
    blocks: list[tuple[str, str, list[int]]] = []
    index = skip_space(mask, 0)
    while index < len(mask):
        if not mask.startswith(".{", index):
            excerpt = body[index : index + 40].splitlines()[0]
            raise InventoryError(f"unsupported registry entry syntax near {excerpt!r}")
        opening = index + 1
        if depths[opening] != 0:
            raise InventoryError("registry entry escaped its initializer scope")
        closing = find_matching_brace(mask, opening)
        if depths[closing] != 1:
            raise InventoryError("registry entry escaped its initializer scope")
        block_start = opening + 1
        base_depth = depths[block_start]
        block_depths = [
            depth - base_depth for depth in depths[block_start : closing + 1]
        ]
        blocks.append(
            (
                body[block_start:closing],
                mask[block_start:closing],
                block_depths,
            )
        )
        index = skip_space(mask, closing + 1)
        if index >= len(mask) or mask[index] != ",":
            raise InventoryError("registry entries must be comma-terminated struct literals")
        index = skip_space(mask, index + 1)
    if not blocks:
        raise InventoryError("upstream registry contains no root struct entries")
    return blocks


def zig_identifier(raw: str) -> str:
    value = raw.strip()
    quoted = re.fullmatch(r'@"([A-Za-z_][A-Za-z0-9_]*)"', value)
    if quoted:
        return quoted.group(1)
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", value):
        raise InventoryError(f"unsupported Zig identifier {raw!r}")
    return value


def parse_enum(
    text: str,
    name: str,
    *,
    masked_source: str | None = None,
    source_depths: list[int] | None = None,
) -> list[str]:
    body, mask = initializer_body(
        text,
        "zig",
        rf"pub\s+const\s+{re.escape(name)}\s*=\s*enum\s*\{{",
        name,
        masked_source=masked_source,
        source_depths=source_depths,
    )
    if mask.rstrip() and not mask.rstrip().endswith(","):
        raise InventoryError(f"{name} members must be comma-terminated")
    values: list[str] = []
    offset = 0
    for masked_part in mask.split(","):
        original_part = body[offset : offset + len(masked_part)]
        offset += len(masked_part) + 1
        if not masked_part.strip():
            continue
        stripped = masked_part.strip()
        start = len(masked_part) - len(masked_part.lstrip())
        raw_value = original_part[start : start + len(stripped)]
        if re.fullmatch(ZIG_IDENTIFIER_PATTERN, stripped) is None:
            raise InventoryError(f"unsupported {name} member syntax: {raw_value}")
        values.append(zig_identifier(raw_value))
    require_unique(values, name)
    if not values:
        raise InventoryError(f"{name} is empty")
    return values


def decode_zig_string(raw: str) -> str:
    try:
        value = json.loads(f'"{raw}"')
    except json.JSONDecodeError as error:
        raise InventoryError(f"unsupported Zig string literal {raw!r}") from error
    if not isinstance(value, str):
        raise InventoryError("decoded Zig string is not text")
    return value


def field_assignment(
    mask: str, depths: list[int], field: str
) -> re.Match[str] | None:
    escaped = re.escape(field)
    candidates = re.finditer(rf'(?:\.{escaped}|\.@"{escaped}")\s*=\s*', mask)
    matches = [match for match in candidates if depths[match.start()] == 0]
    if len(matches) > 1:
        raise InventoryError(f"registry entry repeats .{field}")
    return matches[0] if matches else None


def field_match(
    mask: str, depths: list[int], field: str, value_pattern: str
) -> re.Match[str] | None:
    assignment = field_assignment(mask, depths, field)
    if assignment is None:
        return None
    match = re.compile(rf"(?P<value>{value_pattern})").match(mask, assignment.end())
    if match is None:
        raise InventoryError(f"unsupported .{field} value expression")
    return match


def structural_depth_map(mask: str) -> list[int]:
    depths = [0] * (len(mask) + 1)
    stack: list[str] = []
    matching = {"}": "{", "]": "[", ")": "("}
    for index, character in enumerate(mask):
        depths[index] = len(stack)
        if character in "{[(":
            stack.append(character)
        elif character in "}])":
            if not stack or stack[-1] != matching[character]:
                raise InventoryError("unbalanced registry initializer")
            stack.pop()
    if stack:
        raise InventoryError("unbalanced registry initializer")
    depths[len(mask)] = 0
    return depths


def parse_string_at(text: str, opening: int) -> tuple[str, int]:
    if opening >= len(text) or text[opening] != '"':
        raise InventoryError("internal parser error: expected string literal")
    index = opening + 1
    escaped = False
    while index < len(text):
        character = text[index]
        if escaped:
            escaped = False
        elif character == "\\":
            escaped = True
        elif character == '"':
            return decode_zig_string(text[opening + 1 : index]), index + 1
        index += 1
    raise InventoryError("unterminated Zig string literal")


def require_field_terminator(mask: str, index: int, field: str) -> None:
    cursor = skip_space(mask, index)
    if cursor < len(mask) and mask[cursor] != ",":
        raise InventoryError(f"unsupported .{field} value expression")


def string_field(
    block: str,
    mask: str,
    depths: list[int],
    field: str,
    required: bool = True,
) -> str | None:
    match = field_match(mask, depths, field, r'"')
    if match is None:
        if required:
            raise InventoryError(f"registry entry is missing .{field}")
        return None
    value, end = parse_string_at(block, match.start("value"))
    require_field_terminator(mask, end, field)
    return value


def identifier_field(
    block: str,
    mask: str,
    depths: list[int],
    field: str,
    required: bool = True,
) -> str | None:
    match = field_match(
        mask,
        depths,
        field,
        rf"\.(?P<identifier>{ZIG_IDENTIFIER_PATTERN})(?![A-Za-z0-9_])",
    )
    if match is None:
        if required:
            raise InventoryError(f"registry entry is missing .{field}")
        return None
    identifier = block[match.start("identifier") : match.end("identifier")]
    require_field_terminator(mask, match.end("value"), field)
    return zig_identifier(identifier)


def string_list_field(
    block: str, mask: str, depths: list[int], field: str
) -> list[str]:
    match = field_match(mask, depths, field, r"&\.\{")
    if match is None:
        return []
    opening = match.end("value") - 1
    closing = find_matching_brace(mask, opening)
    require_field_terminator(mask, closing + 1, field)
    list_body = block[opening + 1 : closing]
    list_mask = mask[opening + 1 : closing]
    parts = list_mask.split(",")
    originals = []
    offset = 0
    for part in parts:
        originals.append(list_body[offset : offset + len(part)])
        offset += len(part) + 1
    values: list[str] = []
    for original_part, masked_part in zip(originals, parts, strict=True):
        stripped = masked_part.strip()
        if not stripped:
            continue
        if re.fullmatch(r'"\s*"', stripped) is None:
            raise InventoryError(f"unsupported .{field} list expression")
        leading = len(masked_part) - len(masked_part.lstrip())
        value, end = parse_string_at(original_part, leading)
        if masked_part[end:].strip():
            raise InventoryError(f"unsupported .{field} list expression")
        values.append(value)
    return values


def boolean_field(
    block: str,
    mask: str,
    depths: list[int],
    field: str,
    default: bool = False,
) -> bool:
    match = field_match(mask, depths, field, r"true|false")
    if match is None:
        return default
    require_field_terminator(mask, match.end("value"), field)
    return match.group("value") == "true"


def require_unique(values: list[str], label: str) -> None:
    seen: set[str] = set()
    duplicates: set[str] = set()
    for value in values:
        if value in seen:
            duplicates.add(value)
        else:
            seen.add(value)
    if duplicates:
        raise InventoryError(f"duplicate {label}: {', '.join(sorted(duplicates))}")


def extract_top_level_commands(
    specs_text: str,
    registry_text: str,
    *,
    specs_masked: str | None = None,
    registry_masked: str | None = None,
    specs_depths: list[int] | None = None,
    registry_depths: list[int] | None = None,
) -> list[dict[str, Any]]:
    enum_values = parse_enum(
        specs_text,
        "TopLevelKind",
        masked_source=specs_masked,
        source_depths=specs_depths,
    )
    body, mask = initializer_body(
        registry_text,
        "zig",
        r"pub\s+const\s+top_level_specs\s*=\s*\[_\]TopLevelSpec\s*\{",
        "top-level command registry",
        masked_source=registry_masked,
        source_depths=registry_depths,
    )
    commands: list[dict[str, Any]] = []
    for block, block_mask, depths in root_struct_blocks(body, mask):
        kind = str(identifier_field(block, block_mask, depths, "kind"))
        token = str(string_field(block, block_mask, depths, "token"))
        aliases = string_list_field(block, block_mask, depths, "aliases")
        require_pattern(token, CLI_TOKEN, "top-level command token")
        for alias in aliases:
            require_pattern(alias, CLI_ALIAS, "top-level command alias")
        commands.append(
            {
                "kind": kind,
                "token": token,
                "aliases": aliases,
                "usage": string_field(block, block_mask, depths, "usage"),
                "summary": string_field(block, block_mask, depths, "summary"),
                "hidden_from_help": boolean_field(
                    block, block_mask, depths, "hidden_from_top_level_help"
                ),
            }
        )
    kinds = [str(command["kind"]) for command in commands]
    tokens = [str(command["token"]) for command in commands]
    require_unique(kinds, "top-level command kind")
    require_unique(tokens, "top-level command token")
    if set(kinds) != set(enum_values):
        raise InventoryError(
            "top-level command specs do not exactly cover TopLevelKind: "
            f"enum={sorted(enum_values)}, specs={sorted(kinds)}"
        )
    return commands


def extract_slash_commands(
    specs_text: str,
    registry_text: str,
    *,
    specs_masked: str | None = None,
    registry_masked: str | None = None,
    specs_depths: list[int] | None = None,
    registry_depths: list[int] | None = None,
) -> list[dict[str, Any]]:
    enum_values = parse_enum(
        specs_text,
        "SlashKind",
        masked_source=specs_masked,
        source_depths=specs_depths,
    )
    body, mask = initializer_body(
        registry_text,
        "zig",
        r"pub\s+const\s+slash_specs\s*=\s*\[_\]SlashSpec\s*\{",
        "slash command registry",
        masked_source=registry_masked,
        source_depths=registry_depths,
    )
    commands: list[dict[str, Any]] = []
    for block, block_mask, depths in root_struct_blocks(body, mask):
        kind = str(identifier_field(block, block_mask, depths, "kind"))
        command = str(string_field(block, block_mask, depths, "command"))
        aliases = string_list_field(block, block_mask, depths, "aliases")
        require_pattern(command, SLASH_COMMAND, "slash command")
        for alias in aliases:
            require_pattern(alias, SLASH_COMMAND, "slash command alias")
        commands.append(
            {
                "kind": kind,
                "command": command,
                "aliases": aliases,
                "presentation_category": identifier_field(
                    block,
                    block_mask,
                    depths,
                    "presentation_category",
                    required=False,
                ),
                "has_arguments": boolean_field(
                    block, block_mask, depths, "has_args"
                ),
            }
        )
    kinds = [str(command["kind"]) for command in commands]
    require_unique(kinds, "slash command kind")
    if set(kinds) != set(enum_values):
        raise InventoryError(
            "slash specs do not exactly cover SlashKind: "
            f"enum={sorted(enum_values)}, specs={sorted(kinds)}"
        )
    return commands


def index_tool_spec_blocks(
    text: str,
    mask: str,
    depths: list[int],
    identifiers: list[str],
) -> dict[str, tuple[str, str, list[int]]]:
    wanted = set(identifiers)
    matches: dict[str, list[re.Match[str]]] = {identifier: [] for identifier in wanted}
    declaration_pattern = re.compile(
        r"pub\s+const\s+(?P<identifier>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*"
        r"ToolSpec\s*\{"
    )
    for match in declaration_pattern.finditer(mask):
        identifier = match.group("identifier")
        if identifier in wanted:
            matches[identifier].append(match)

    blocks: dict[str, tuple[str, str, list[int]]] = {}
    for identifier in identifiers:
        candidates = matches[identifier]
        if len(candidates) > 1:
            raise InventoryError(f"multiple upstream declarations for built-in tool {identifier}")
        if not candidates:
            raise InventoryError(f"cannot find upstream built-in tool {identifier}")
        declaration = candidates[0]
        if depths[declaration.start()] != 0:
            raise InventoryError(f"built-in tool {identifier} must be declared at top level")
        opening = mask.find("{", declaration.start(), declaration.end())
        if opening < 0:
            raise InventoryError(f"cannot find initializer for built-in tool {identifier}")
        if depths[opening] != 0:
            raise InventoryError(
                f"built-in tool {identifier} initializer escaped top-level scope"
            )
        closing = find_matching_brace(mask, opening)
        if depths[closing] != 1:
            raise InventoryError(
                f"built-in tool {identifier} initializer escaped top-level scope"
            )
        block_start = opening + 1
        base_depth = depths[block_start]
        blocks[identifier] = (
            text[block_start:closing],
            mask[block_start:closing],
            [depth - base_depth for depth in depths[block_start : closing + 1]],
        )
    return blocks


def extract_builtin_tools(text: str) -> list[dict[str, str]]:
    source_masked = source_mask(text, "zig")
    source_depths = structural_depth_map(source_masked)
    body, mask = initializer_body(
        text,
        "zig",
        r"pub\s+const\s+all\s*=\s*\[_\]tool_dispatch\.Tool\s*\{",
        "built-in tool registry",
        masked_source=source_masked,
        source_depths=source_depths,
    )
    if mask.rstrip() and not mask.rstrip().endswith(","):
        raise InventoryError("built-in tool registry entries must be comma-terminated")
    identifiers: list[str] = []
    offset = 0
    for masked_part in mask.split(","):
        original_part = body[offset : offset + len(masked_part)]
        offset += len(masked_part) + 1
        if not masked_part.strip():
            continue
        expression = masked_part.strip()
        if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", expression) is None:
            raise InventoryError(
                f"unsupported built-in tool registry expression {original_part.strip()!r}"
            )
        identifiers.append(expression)
    require_unique(identifiers, "built-in tool registry identifier")
    if not identifiers:
        raise InventoryError("built-in tool registry is empty")
    tool_blocks = index_tool_spec_blocks(
        text, source_masked, source_depths, identifiers
    )
    tools: list[dict[str, str]] = []
    for identifier in identifiers:
        tool_body, tool_mask, tool_depths = tool_blocks[identifier]
        name = str(string_field(tool_body, tool_mask, tool_depths, "name"))
        require_pattern(name, TOOL_NAME, "built-in tool name")
        tools.append({"identifier": identifier, "name": name})
    require_unique([tool["name"] for tool in tools], "built-in tool name")
    return tools


def javascript_statement_end(mask: str, start: int) -> int:
    depth = 0
    for index in range(start, len(mask)):
        character = mask[index]
        if character in "{[(":
            depth += 1
        elif character in "}])":
            depth -= 1
            if depth < 0:
                raise InventoryError("unbalanced JavaScript export declaration")
        elif character == ";" and depth == 0:
            return index
    raise InventoryError("JavaScript variable export must end with a semicolon")


def split_javascript_declarators(text: str, mask: str) -> list[tuple[str, str]]:
    parts: list[tuple[str, str]] = []
    depth = 0
    start = 0
    for index, character in enumerate(mask):
        if character in "{[(":
            depth += 1
        elif character in "}])":
            depth -= 1
            if depth < 0:
                raise InventoryError("unbalanced JavaScript variable export")
        elif character == "," and depth == 0:
            parts.append((text[start:index], mask[start:index]))
            start = index + 1
    if depth != 0:
        raise InventoryError("unbalanced JavaScript variable export")
    parts.append((text[start:], mask[start:]))
    return parts


def javascript_variable_exports(
    text: str, mask: str, position: int
) -> list[str] | None:
    declaration = JS_VARIABLE_EXPORT.match(mask, position)
    if declaration is None:
        return None
    body_start = declaration.end()
    body_end = javascript_statement_end(mask, body_start)
    body = text[body_start:body_end]
    body_mask = mask[body_start:body_end]
    names: list[str] = []
    for original, masked in split_javascript_declarators(body, body_mask):
        declarator = re.fullmatch(
            rf"\s*(?P<name>{JS_IDENTIFIER_PATTERN})(?![A-Za-z0-9_$])"
            r"(?P<initializer>\s*=\s*[\s\S]+)?\s*",
            masked,
        )
        if declarator is None or (
            declaration.group("kind") == "const"
            and declarator.group("initializer") is None
        ):
            raise InventoryError(
                f"unsupported JavaScript variable export {original.strip()!r}"
            )
        names.append(declarator.group("name"))
    if not names:
        raise InventoryError("JavaScript variable export must declare a name")
    return names


def extract_js_exports(text: str) -> list[str]:
    mask = source_mask(text, "js")
    depths = structural_depth_map(mask)
    export_positions = [match.start() for match in re.finditer(r"\bexport\b", mask)]
    found: list[str] = []
    declaration_pattern = re.compile(
        rf"export\s+(?:async\s+function|function|class)\s+"
        rf"(?P<name>{JS_IDENTIFIER_PATTERN})(?![A-Za-z0-9_$])"
    )
    for position in export_positions:
        if depths[position] != 0:
            raise InventoryError("JavaScript exports must be top-level declarations")
        declaration = declaration_pattern.match(mask, position)
        if declaration is not None:
            found.append(declaration.group("name"))
            continue
        variable_exports = javascript_variable_exports(text, mask, position)
        if variable_exports is not None:
            found.extend(variable_exports)
            continue
        list_start = JS_EXPORT_LIST_START.match(mask, position)
        if list_start is None:
            excerpt = text[position : position + 60].splitlines()[0]
            raise InventoryError(f"unsupported JavaScript export syntax near {excerpt!r}")
        opening = list_start.end() - 1
        closing = find_matching_brace(mask, opening)
        body = text[opening + 1 : closing]
        body_mask = mask[opening + 1 : closing]
        offset = 0
        list_exports: list[str] = []
        for masked_part in body_mask.split(","):
            original_part = body[offset : offset + len(masked_part)]
            offset += len(masked_part) + 1
            cleaned = masked_part.strip()
            if not cleaned:
                continue
            entry = re.fullmatch(
                rf"(?P<local>{JS_IDENTIFIER_PATTERN})(?:\s+as\s+"
                rf"(?P<exported>{JS_IDENTIFIER_PATTERN}))?",
                cleaned,
            )
            if entry is None:
                raise InventoryError(
                    f"unsupported JavaScript export entry {original_part.strip()!r}"
                )
            list_exports.append(entry.group("exported") or entry.group("local"))
        if not list_exports:
            raise InventoryError("JavaScript export list must not be empty")
        cursor = skip_space(mask, closing + 1)
        from_match = JS_EXPORT_FROM.match(mask, cursor)
        if from_match is not None:
            quote = from_match.start("quote")
            module, cursor = parse_string_at(text, quote)
            if re.fullmatch(r"\./[A-Za-z0-9][A-Za-z0-9._/-]*\.js", module) is None:
                raise InventoryError(f"unsupported JavaScript re-export module {module!r}")
            cursor = skip_space(mask, cursor)
        if cursor >= len(mask) or mask[cursor] != ";":
            raise InventoryError("JavaScript export list must end with a semicolon")
        found.extend(list_exports)
    exports: list[str] = []
    seen: set[str] = set()
    for name in found:
        if re.fullmatch(JS_IDENTIFIER_PATTERN, name) is None:
            raise InventoryError(f"unsupported JavaScript export identifier {name!r}")
        if name not in seen:
            seen.add(name)
            exports.append(name)
    if not exports:
        raise InventoryError("SDK module has no named exports")
    return exports


def package_entrypoints(package: dict[str, Any]) -> list[dict[str, Any]]:
    raw_exports = package.get("exports")
    if not isinstance(raw_exports, dict) or not raw_exports:
        raise InventoryError("sdk/package.json has no exports map")
    entrypoints: list[dict[str, Any]] = []
    for specifier, target in raw_exports.items():
        if not isinstance(specifier, str):
            raise InventoryError("SDK export specifier must be text")
        require_pattern(specifier, PACKAGE_SPECIFIER, "SDK package specifier")
        targets: dict[str, str]
        if isinstance(target, str):
            targets = {"default": target}
        elif isinstance(target, dict) and all(
            isinstance(condition, str) and isinstance(value, str)
            for condition, value in target.items()
        ):
            targets = dict(target)
        else:
            raise InventoryError(f"unsupported SDK exports map for {specifier}")
        for condition, value in targets.items():
            require_pattern(condition, PACKAGE_CONDITION, "SDK export condition")
            require_pattern(value, PACKAGE_TARGET, "SDK module target")
        entrypoints.append({"specifier": specifier, "targets": targets})
    return entrypoints


def extract_sdk_exports(source: GitSnapshot) -> dict[str, Any]:
    try:
        package = json.loads(read_text(source, SDK_PACKAGE))
    except json.JSONDecodeError as error:
        raise InventoryError(f"invalid {SDK_PACKAGE}: {error}") from error
    if not isinstance(package, dict):
        raise InventoryError(f"{SDK_PACKAGE} root must be an object")
    modules = []
    for relative in SDK_MODULES:
        modules.append(
            {
                "path": relative,
                "exports": extract_js_exports(read_text(source, relative)),
            }
        )
    known_modules = {module["path"] for module in modules}
    for entrypoint in package_entrypoints(package):
        for target in entrypoint["targets"].values():
            module_path = f"sdk/{target.removeprefix('./')}"
            if module_path not in known_modules:
                raise InventoryError(f"SDK entrypoint targets unparsed module {target}")
    name = package.get("name")
    if not isinstance(name, str) or not name:
        raise InventoryError("SDK package name must be non-empty text")
    require_pattern(name, PACKAGE_NAME, "SDK package name")
    return {
        "package": name,
        "entrypoints": package_entrypoints(package),
        "modules": modules,
    }


def scenario_owner_map(entries: Any, classification: str) -> dict[str, dict[str, Any]]:
    if not isinstance(entries, list):
        raise InventoryError(f"PGSO {classification} scenarios must be an array")
    owners: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise InventoryError(f"PGSO {classification} scenario must be an object")
        test_file = entry.get("test_file")
        if test_file is None:
            continue
        name = entry.get("name")
        if not isinstance(test_file, str) or not isinstance(name, str):
            raise InventoryError(f"PGSO {classification} scenario has invalid owner metadata")
        require_pattern(test_file, E2E_FILE, "E2E owner file")
        require_pattern(name, SCENARIO_NAME, "PGSO scenario name")
        if test_file in owners:
            raise InventoryError(f"duplicate PGSO {classification} owner {test_file}")
        owners[test_file] = {
            "file": test_file,
            "classification": classification,
            "scenario": name,
            "reason": None,
        }
    return owners


def extract_e2e_owners(source: GitSnapshot) -> list[dict[str, Any]]:
    try:
        corpus = json.loads(read_text(source, E2E_CORPUS))
    except json.JSONDecodeError as error:
        raise InventoryError(f"invalid {E2E_CORPUS}: {error}") from error
    if not isinstance(corpus, dict):
        raise InventoryError("PGSO corpus root must be an object")
    owner_maps = [
        scenario_owner_map(corpus.get("scenarios"), "training"),
        scenario_owner_map(corpus.get("verification_scenarios"), "verification_only"),
    ]
    exclusions = corpus.get("intentional_exclusions")
    if not isinstance(exclusions, dict):
        raise InventoryError("PGSO intentional_exclusions must be an object")
    excluded: dict[str, dict[str, Any]] = {}
    for test_file, reason in exclusions.items():
        if not isinstance(test_file, str) or not isinstance(reason, str) or not reason:
            raise InventoryError("PGSO intentional exclusion must have a non-empty reason")
        require_pattern(test_file, E2E_FILE, "E2E owner file")
        excluded[test_file] = {
            "file": test_file,
            "classification": "intentional_exclusion",
            "scenario": None,
            "reason": reason,
        }
    owner_maps.append(excluded)
    combined: dict[str, dict[str, Any]] = {}
    for mapping in owner_maps:
        overlap = sorted(set(combined).intersection(mapping))
        if overlap:
            raise InventoryError(
                f"E2E owners have multiple PGSO classifications: {', '.join(overlap)}"
            )
        combined.update(mapping)
    actual = source.root_files("tests/e2e", ".test.ts")
    classified = sorted(combined)
    if actual != classified:
        missing = sorted(set(actual) - set(classified))
        stale = sorted(set(classified) - set(actual))
        raise InventoryError(
            "PGSO owner coverage does not match root E2E files; "
            f"unclassified={missing}, stale={stale}"
        )
    return [combined[name] for name in actual]


def load_policy(path: Path) -> dict[str, Any]:
    try:
        policy = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise InventoryError(f"cannot read compatibility policy {path}: {error}") from error
    if not isinstance(policy, dict) or policy.get("schema_version") != 1:
        raise InventoryError("compatibility policy must use schema_version 1")
    surfaces = policy.get("surfaces")
    if not isinstance(surfaces, dict) or set(surfaces) != set(SURFACE_NAMES):
        raise InventoryError(
            f"compatibility policy surfaces must be exactly {', '.join(SURFACE_NAMES)}"
        )
    for name, surface in surfaces.items():
        if not isinstance(surface, dict) or surface.get("status") not in ALLOWED_STATUSES:
            raise InventoryError(f"compatibility policy surface {name} has invalid status")
        for field in ("milestone", "notes"):
            if not isinstance(surface.get(field), str) or not surface[field]:
                raise InventoryError(f"compatibility policy surface {name} needs non-empty {field}")
    differences = policy.get("intentional_differences")
    if not isinstance(differences, list) or not differences:
        raise InventoryError("compatibility policy needs intentional_differences")
    for difference in differences:
        if (
            not isinstance(difference, dict)
            or difference.get("status") != "intentional_difference"
        ):
            raise InventoryError(
                "every intentional difference must use intentional_difference status"
            )
        for field in ("id", "surface", "notes"):
            if not isinstance(difference.get(field), str) or not difference[field]:
                raise InventoryError(f"intentional difference needs non-empty {field}")
    return policy


def source_provenance(source: GitSnapshot) -> tuple[list[dict[str, str]], dict[str, Any]]:
    sources = []
    for relative in SOURCE_FILES:
        mode, object_id, data = source.blob(relative)
        sources.append(
            {
                "path": relative,
                "mode": mode,
                "git_blob": object_id,
                "sha256": sha256_bytes(data),
            }
        )
    e2e_files = source.root_files("tests/e2e", ".test.ts")
    e2e_digest = sha256_bytes(("\n".join(e2e_files) + "\n").encode())
    return sources, {
        "path_glob": "tests/e2e/*.test.ts",
        "member_count": len(e2e_files),
        "member_names_sha256": e2e_digest,
    }


def build_inventory(
    source: GitSnapshot,
    lock: dict[str, str],
    policy: dict[str, Any],
    *,
    lock_path: str = "benchmarks/upstream.lock",
) -> dict[str, Any]:
    specs_text = read_text(source, COMMAND_SPECS)
    registry_text = read_text(source, COMMAND_REGISTRY)
    specs_masked = source_mask(specs_text, "zig")
    registry_masked = source_mask(registry_text, "zig")
    specs_depths = structural_depth_map(specs_masked)
    registry_depths = structural_depth_map(registry_masked)
    top_level = extract_top_level_commands(
        specs_text,
        registry_text,
        specs_masked=specs_masked,
        registry_masked=registry_masked,
        specs_depths=specs_depths,
        registry_depths=registry_depths,
    )
    slash = extract_slash_commands(
        specs_text,
        registry_text,
        specs_masked=specs_masked,
        registry_masked=registry_masked,
        specs_depths=specs_depths,
        registry_depths=registry_depths,
    )
    tools = extract_builtin_tools(read_text(source, TOOL_REGISTRY))
    sdk = extract_sdk_exports(source)
    e2e = extract_e2e_owners(source)
    source_files, source_set = source_provenance(source)
    surface_policy = policy["surfaces"]
    return {
        "schema_version": 1,
        "upstream": {
            "repository": lock["repository"],
            "commit": lock["commit"],
            "lock_path": lock_path,
            "source_files": source_files,
            "source_sets": {"e2e_owner_files": source_set},
        },
        "surfaces": {
            "top_level_cli_commands": {
                **surface_policy["top_level_cli_commands"],
                "source": [COMMAND_SPECS, COMMAND_REGISTRY],
                "count": len(top_level),
                "items": top_level,
            },
            "slash_command_kinds": {
                **surface_policy["slash_command_kinds"],
                "source": [COMMAND_SPECS, COMMAND_REGISTRY],
                "count": len(slash),
                "items": slash,
            },
            "builtin_tool_names": {
                **surface_policy["builtin_tool_names"],
                "source": [TOOL_REGISTRY],
                "count": len(tools),
                "items": tools,
            },
            "sdk_exports": {
                **surface_policy["sdk_exports"],
                "source": [SDK_PACKAGE, *SDK_MODULES],
                **sdk,
            },
            "e2e_owners": {
                **surface_policy["e2e_owners"],
                "source": [E2E_CORPUS, "tests/e2e/*.test.ts"],
                "count": len(e2e),
                "classification_counts": {
                    classification: sum(
                        owner["classification"] == classification for owner in e2e
                    )
                    for classification in (
                        "training",
                        "verification_only",
                        "intentional_exclusion",
                    )
                },
                "items": e2e,
            },
        },
        "intentional_differences": policy["intentional_differences"],
    }


def markdown_text(value: object) -> str:
    escaped = html.escape(str(value), quote=False).replace("\r", " ").replace("\n", " ")
    for character in "\\`*_{}[]()#+-.!|":
        escaped = escaped.replace(character, f"\\{character}")
    return escaped


def markdown_code(value: object) -> str:
    escaped = html.escape(str(value), quote=False).replace("\r", " ").replace("\n", " ")
    escaped = escaped.replace("|", "&#124;")
    longest_run = max((len(run) for run in re.findall(r"`+", escaped)), default=0)
    delimiter = "`" * (longest_run + 1)
    if escaped.startswith(("`", " ")) or escaped.endswith(("`", " ")):
        escaped = f" {escaped} "
    return f"{delimiter}{escaped}{delimiter}"


def code_list(values: list[str]) -> str:
    return ", ".join(markdown_code(value) for value in values) if values else "—"


def render_docs(inventory: dict[str, Any]) -> str:
    upstream = inventory["upstream"]
    surfaces = inventory["surfaces"]
    repository_page = upstream["repository"].removesuffix(".git")
    commit = upstream["commit"]
    lines = [
        "<!-- Generated by scripts/generate_compatibility.py; do not edit by hand. -->",
        "",
        "# Compatibility",
        "",
        f"Comparison target: [{markdown_code('vercel-labs/fx')} commit "
        f"{markdown_code(commit)}]({repository_page}/commit/{commit}),",
        f"pinned by {markdown_code(upstream['lock_path'])}.",
        "",
        "The inventory records upstream names and ownership boundaries; it is not a claim that",
        "machine-god already implements them. `compatibility/inventory.json` is the canonical",
        "machine-readable artifact.",
        "",
        "## Status semantics",
        "",
        "| Status | Meaning |",
        "| --- | --- |",
        "| planned | Compatibility work is expected in the named milestone. |",
        "| deferred | The surface is intentionally scheduled after the core engine and "
        "native host. |",
        "| intentional difference | The upstream behavior or implementation shape is "
        "explicitly not a compatibility target. |",
        "",
        "## Surface plan",
        "",
        "| Surface | Status | Milestone | Notes |",
        "| --- | --- | --- | --- |",
    ]
    labels = {
        "top_level_cli_commands": "Top-level CLI commands",
        "slash_command_kinds": "Slash command kinds",
        "builtin_tool_names": "Built-in tool names",
        "sdk_exports": "SDK exports",
        "e2e_owners": "E2E owners",
    }
    status_labels = {
        "planned": "planned",
        "deferred": "deferred",
        "intentional_difference": "intentional difference",
    }
    for name in SURFACE_NAMES:
        surface = surfaces[name]
        lines.append(
            f"| {markdown_text(labels[name])} | "
            f"{markdown_text(status_labels[surface['status']])} | "
            f"{markdown_text(surface['milestone'])} | {markdown_text(surface['notes'])} |"
        )

    lines.extend(
        [
            "",
            "## Top-level CLI commands",
            "",
            f"Source: {markdown_code(COMMAND_SPECS)} and {markdown_code(COMMAND_REGISTRY)} "
            f"({markdown_text(surfaces['top_level_cli_commands']['count'])} commands).",
            "",
            "| Kind | Token | Aliases | Hidden from help |",
            "| --- | --- | --- | --- |",
        ]
    )
    for command in surfaces["top_level_cli_commands"]["items"]:
        lines.append(
            f"| {markdown_code(command['kind'])} | {markdown_code(command['token'])} | "
            f"{code_list(command['aliases'])} | "
            f"{'yes' if command['hidden_from_help'] else 'no'} |"
        )

    lines.extend(
        [
            "",
            "## Slash command kinds",
            "",
            f"Source: {markdown_code(COMMAND_SPECS)} and {markdown_code(COMMAND_REGISTRY)} "
            f"({markdown_text(surfaces['slash_command_kinds']['count'])} kinds).",
            "",
            "| Kind | Command | Aliases | Presentation category | Accepts arguments |",
            "| --- | --- | --- | --- | --- |",
        ]
    )
    for command in surfaces["slash_command_kinds"]["items"]:
        category = command["presentation_category"]
        lines.append(
            f"| {markdown_code(command['kind'])} | {markdown_code(command['command'])} | "
            f"{code_list(command['aliases'])} | "
            f"{markdown_code(category) if category else 'internal subcommand'} | "
            f"{'yes' if command['has_arguments'] else 'no'} |"
        )

    tool_surface = surfaces["builtin_tool_names"]
    lines.extend(
        [
            "",
            "## Built-in tools",
            "",
            f"Source: {markdown_code(TOOL_REGISTRY)} "
            f"({markdown_text(tool_surface['count'])} production registry entries).",
            "",
            code_list([tool["name"] for tool in tool_surface["items"]]) + ".",
            "",
            "## SDK exports",
            "",
            f"Package: {markdown_code(surfaces['sdk_exports']['package'])}. Source: "
            f"{markdown_code(SDK_PACKAGE)} and the mapped modules.",
            "",
            "| Package entrypoint | Conditions and modules |",
            "| --- | --- |",
        ]
    )
    for entrypoint in surfaces["sdk_exports"]["entrypoints"]:
        targets = ", ".join(
            f"{markdown_code(condition)} → {markdown_code(target)}"
            for condition, target in entrypoint["targets"].items()
        )
        lines.append(f"| {markdown_code(entrypoint['specifier'])} | {targets} |")
    lines.extend(["", "| Module | Named exports |", "| --- | --- |"])
    for module in surfaces["sdk_exports"]["modules"]:
        lines.append(f"| {markdown_code(module['path'])} | {code_list(module['exports'])} |")

    e2e_surface = surfaces["e2e_owners"]
    counts = e2e_surface["classification_counts"]
    lines.extend(
        [
            "",
            "## Major E2E owners",
            "",
            "Upstream defines each root `tests/e2e/*.test.ts` file as one deterministic E2E",
            "owner in its PGSO corpus. The inventory validates that every file has exactly one",
            "classification and that the corpus has no stale owners.",
            "",
            f"Counts: {markdown_text(counts['training'])} training, "
            f"{markdown_text(counts['verification_only'])} verification-only, "
            f"and {markdown_text(counts['intentional_exclusion'])} intentional exclusions "
            f"({markdown_text(e2e_surface['count'])} total).",
            "",
            "| Owner file | Upstream classification | Scenario or exclusion reason |",
            "| --- | --- | --- |",
        ]
    )
    for owner in e2e_surface["items"]:
        detail = owner["scenario"] if owner["scenario"] is not None else owner["reason"]
        classification = owner["classification"].replace("_", "-")
        lines.append(
            f"| {markdown_code(owner['file'])} | {markdown_text(classification)} | "
            f"{markdown_text(detail)} |"
        )

    lines.extend(["", "## Intentional differences", ""])
    for difference in inventory["intentional_differences"]:
        lines.append(
            f"- {markdown_code(difference['id'])} ({markdown_text(difference['surface'])}): "
            f"{markdown_text(difference['notes'])}"
        )

    lines.extend(
        [
            "",
            "## Regeneration and drift check",
            "",
            "The generator performs no network access. Point it at a Git checkout whose `HEAD` is",
            "the pinned commit. It reads and hashes canonical regular-blob bytes from that commit",
            "with Git object plumbing; worktree changes, symlinks, and line-ending filters cannot",
            "change the inventory.",
            "",
            "```sh",
            f"git clone {upstream['repository']} /tmp/fx-compatibility",
            f"git -C /tmp/fx-compatibility checkout --detach {commit}",
            "python3 scripts/generate_compatibility.py --upstream /tmp/fx-compatibility",
            "python3 scripts/generate_compatibility.py --upstream /tmp/fx-compatibility --check",
            "```",
            "",
            "`--check` exits nonzero when either generated artifact differs. Fixture-based Python",
            "tests exercise the same parsers and drift path without cloning or network access.",
            "",
        ]
    )
    return "\n".join(lines)


def json_document(value: dict[str, Any]) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def check_artifact(path: Path, expected: str) -> bool:
    try:
        actual = path.read_text(encoding="utf-8")
    except OSError:
        actual = ""
    if actual == expected:
        return True
    print(f"generated compatibility artifact is stale: {path}", file=sys.stderr)
    return False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", type=Path, required=True, help="local checkout of pinned fx")
    parser.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--inventory", type=Path, default=DEFAULT_INVENTORY)
    parser.add_argument("--docs", type=Path, default=DEFAULT_DOCS)
    parser.add_argument("--check", action="store_true", help="fail if generated files differ")
    args = parser.parse_args(argv)

    try:
        lock = parse_lock(args.lock)
        policy = load_policy(args.policy)
        upstream = args.upstream.resolve()
        source = GitSnapshot(upstream, lock["commit"])
        lock_display = (
            "benchmarks/upstream.lock"
            if args.lock.resolve() == DEFAULT_LOCK
            else args.lock.name
        )
        inventory = build_inventory(source, lock, policy, lock_path=lock_display)
        inventory_text = json_document(inventory)
        docs_text = render_docs(inventory)
        if args.check:
            valid = check_artifact(args.inventory, inventory_text)
            valid = check_artifact(args.docs, docs_text) and valid
            return 0 if valid else 1
        args.inventory.parent.mkdir(parents=True, exist_ok=True)
        args.docs.parent.mkdir(parents=True, exist_ok=True)
        args.inventory.write_text(inventory_text, encoding="utf-8")
        args.docs.write_text(docs_text, encoding="utf-8")
    except InventoryError as error:
        print(f"compatibility generation failed: {error}", file=sys.stderr)
        return 2
    print(f"generated {args.inventory} and {args.docs}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
