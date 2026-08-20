#!/usr/bin/env python3
"""Generate the pinned fx compatibility inventory and its human-readable view.

The generator is deliberately offline. It consumes a local checkout whose HEAD
matches ``benchmarks/upstream.lock`` and rejects relevant uncommitted changes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
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


class InventoryError(RuntimeError):
    """Raised when an upstream source no longer matches the expected contract."""


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def read_text(root: Path, relative: str) -> str:
    path = root / relative
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise InventoryError(f"cannot read upstream source {relative}: {error}") from error


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
    if not repository.startswith("https://github.com/") or not repository.endswith(".git"):
        raise InventoryError("upstream repository must be a canonical HTTPS GitHub .git URL")
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit):
        raise InventoryError("upstream commit must be a lowercase full Git object ID")
    return values


def git_output(checkout: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", "-C", str(checkout), *args],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise InventoryError(f"git {' '.join(args)} failed for {checkout}: {detail}")
    return completed.stdout.strip()


def verify_checkout(checkout: Path, commit: str) -> None:
    actual = git_output(checkout, "rev-parse", "HEAD")
    if actual != commit:
        raise InventoryError(f"upstream checkout HEAD is {actual}, expected pinned commit {commit}")
    e2e_paths = [
        path.relative_to(checkout).as_posix()
        for path in (checkout / "tests/e2e").glob("*.test.ts")
    ]
    relevant_paths = [*SOURCE_FILES, *e2e_paths]
    dirty = git_output(
        checkout,
        "status",
        "--porcelain",
        "--untracked-files=all",
        "--",
        *relevant_paths,
    )
    if dirty:
        raise InventoryError(f"upstream compatibility sources have local changes:\n{dirty}")


def find_matching_brace(text: str, opening: int) -> int:
    if opening >= len(text) or text[opening] != "{":
        raise InventoryError("internal parser error: expected opening brace")
    depth = 0
    in_string = False
    escaped = False
    line_comment = False
    block_comment_depth = 0
    index = opening
    while index < len(text):
        character = text[index]
        following = text[index + 1] if index + 1 < len(text) else ""
        if line_comment:
            if character == "\n":
                line_comment = False
        elif block_comment_depth:
            if character == "/" and following == "*":
                block_comment_depth += 1
                index += 1
            elif character == "*" and following == "/":
                block_comment_depth -= 1
                index += 1
        elif in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
        elif character == "/" and following == "/":
            line_comment = True
            index += 1
        elif character == "/" and following == "*":
            block_comment_depth = 1
            index += 1
        elif character == '"':
            in_string = True
        elif character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return index
            if depth < 0:
                break
        index += 1
    raise InventoryError("unterminated Zig initializer")


def initializer_body(text: str, declaration_pattern: str, label: str) -> str:
    match = re.search(declaration_pattern, text)
    if match is None:
        raise InventoryError(f"cannot find upstream {label}")
    opening = text.find("{", match.start(), match.end())
    if opening < 0:
        opening = text.find("{", match.end())
    if opening < 0:
        raise InventoryError(f"cannot find initializer for upstream {label}")
    closing = find_matching_brace(text, opening)
    return text[opening + 1 : closing]


def root_struct_blocks(body: str) -> list[str]:
    blocks: list[str] = []
    for match in re.finditer(r"(?m)^    \.\{", body):
        opening = match.end() - 1
        closing = find_matching_brace(body, opening)
        blocks.append(body[opening + 1 : closing])
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


def parse_enum(text: str, name: str) -> list[str]:
    body = initializer_body(text, rf"pub\s+const\s+{re.escape(name)}\s*=\s*enum\s*\{{", name)
    values: list[str] = []
    for raw_line in body.splitlines():
        line = raw_line.split("//", 1)[0].strip()
        if not line:
            continue
        if not line.endswith(","):
            raise InventoryError(f"unsupported {name} member syntax: {line}")
        values.append(zig_identifier(line[:-1]))
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


def string_field(block: str, field: str, required: bool = True) -> str | None:
    match = re.search(rf"\.{re.escape(field)}\s*=\s*\"((?:\\.|[^\"\\])*)\"", block)
    if match is None:
        if required:
            raise InventoryError(f"registry entry is missing .{field}")
        return None
    return decode_zig_string(match.group(1))


def identifier_field(block: str, field: str, required: bool = True) -> str | None:
    match = re.search(
        rf"\.{re.escape(field)}\s*=\s*\.(?P<value>@\"[A-Za-z_][A-Za-z0-9_]*\"|[A-Za-z_][A-Za-z0-9_]*)",
        block,
    )
    if match is None:
        if required:
            raise InventoryError(f"registry entry is missing .{field}")
        return None
    return zig_identifier(match.group("value"))


def string_list_field(block: str, field: str) -> list[str]:
    match = re.search(rf"\.{re.escape(field)}\s*=\s*&\.\{{(?P<body>.*?)\}}", block, re.DOTALL)
    if match is None:
        return []
    return [
        decode_zig_string(raw)
        for raw in re.findall(r'"((?:\\.|[^"\\])*)"', match.group("body"))
    ]


def require_unique(values: list[str], label: str) -> None:
    duplicates = sorted({value for value in values if values.count(value) > 1})
    if duplicates:
        raise InventoryError(f"duplicate {label}: {', '.join(duplicates)}")


def extract_top_level_commands(specs_text: str, registry_text: str) -> list[dict[str, Any]]:
    enum_values = parse_enum(specs_text, "TopLevelKind")
    body = initializer_body(
        registry_text,
        r"pub\s+const\s+top_level_specs\s*=\s*\[_\]TopLevelSpec\s*\{",
        "top-level command registry",
    )
    commands: list[dict[str, Any]] = []
    for block in root_struct_blocks(body):
        commands.append(
            {
                "kind": identifier_field(block, "kind"),
                "token": string_field(block, "token"),
                "aliases": string_list_field(block, "aliases"),
                "usage": string_field(block, "usage"),
                "summary": string_field(block, "summary"),
                "hidden_from_help": bool(
                    re.search(r"\.hidden_from_top_level_help\s*=\s*true", block)
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


def extract_slash_commands(specs_text: str, registry_text: str) -> list[dict[str, Any]]:
    enum_values = parse_enum(specs_text, "SlashKind")
    body = initializer_body(
        registry_text,
        r"pub\s+const\s+slash_specs\s*=\s*\[_\]SlashSpec\s*\{",
        "slash command registry",
    )
    commands: list[dict[str, Any]] = []
    for block in root_struct_blocks(body):
        commands.append(
            {
                "kind": identifier_field(block, "kind"),
                "command": string_field(block, "command"),
                "aliases": string_list_field(block, "aliases"),
                "presentation_category": identifier_field(
                    block, "presentation_category", required=False
                ),
                "has_arguments": bool(re.search(r"\.has_args\s*=\s*true", block)),
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


def extract_builtin_tools(text: str) -> list[dict[str, str]]:
    body = initializer_body(
        text,
        r"pub\s+const\s+all\s*=\s*\[_\]tool_dispatch\.Tool\s*\{",
        "built-in tool registry",
    )
    identifiers = re.findall(r"(?m)^\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*$", body)
    require_unique(identifiers, "built-in tool registry identifier")
    if not identifiers:
        raise InventoryError("built-in tool registry is empty")
    tools: list[dict[str, str]] = []
    for identifier in identifiers:
        tool_body = initializer_body(
            text,
            rf"pub\s+const\s+{re.escape(identifier)}\s*=\s*ToolSpec\s*\{{",
            f"built-in tool {identifier}",
        )
        tools.append({"identifier": identifier, "name": str(string_field(tool_body, "name"))})
    require_unique([tool["name"] for tool in tools], "built-in tool name")
    return tools


def extract_js_exports(text: str) -> list[str]:
    found: list[tuple[int, str]] = []
    declaration = re.compile(
        r"\bexport\s+(?:async\s+)?(?:function|class|const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)"
    )
    for match in declaration.finditer(text):
        found.append((match.start(), match.group(1)))
    for match in re.finditer(r"\bexport\s*\{(?P<body>.*?)\}\s*;", text, re.DOTALL):
        for entry in match.group("body").split(","):
            cleaned = re.sub(r"/\*.*?\*/|//[^\n]*", "", entry, flags=re.DOTALL).strip()
            if not cleaned:
                continue
            parts = re.split(r"\s+as\s+", cleaned)
            exported = parts[-1].strip()
            if not re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$]*", exported):
                raise InventoryError(f"unsupported JavaScript export entry {cleaned!r}")
            found.append((match.start(), exported))
    found.sort(key=lambda item: item[0])
    exports: list[str] = []
    for _, name in found:
        if name not in exports:
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
        entrypoints.append({"specifier": specifier, "targets": targets})
    return entrypoints


def extract_sdk_exports(upstream: Path) -> dict[str, Any]:
    try:
        package = json.loads(read_text(upstream, SDK_PACKAGE))
    except json.JSONDecodeError as error:
        raise InventoryError(f"invalid {SDK_PACKAGE}: {error}") from error
    if not isinstance(package, dict):
        raise InventoryError(f"{SDK_PACKAGE} root must be an object")
    modules = []
    for relative in SDK_MODULES:
        modules.append({"path": relative, "exports": extract_js_exports(read_text(upstream, relative))})
    known_modules = {module["path"] for module in modules}
    for entrypoint in package_entrypoints(package):
        for target in entrypoint["targets"].values():
            module_path = f"sdk/{target.removeprefix('./')}"
            if module_path not in known_modules:
                raise InventoryError(f"SDK entrypoint targets unparsed module {target}")
    name = package.get("name")
    if not isinstance(name, str) or not name:
        raise InventoryError("SDK package name must be non-empty text")
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
        if test_file in owners:
            raise InventoryError(f"duplicate PGSO {classification} owner {test_file}")
        owners[test_file] = {
            "file": test_file,
            "classification": classification,
            "scenario": name,
            "reason": None,
        }
    return owners


def extract_e2e_owners(upstream: Path) -> list[dict[str, Any]]:
    try:
        corpus = json.loads(read_text(upstream, E2E_CORPUS))
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
            raise InventoryError(f"E2E owners have multiple PGSO classifications: {', '.join(overlap)}")
        combined.update(mapping)
    actual = sorted(path.name for path in (upstream / "tests/e2e").glob("*.test.ts"))
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
        if not isinstance(difference, dict) or difference.get("status") != "intentional_difference":
            raise InventoryError("every intentional difference must use intentional_difference status")
        for field in ("id", "surface", "notes"):
            if not isinstance(difference.get(field), str) or not difference[field]:
                raise InventoryError(f"intentional difference needs non-empty {field}")
    return policy


def source_provenance(upstream: Path) -> tuple[list[dict[str, str]], dict[str, Any]]:
    sources = [{"path": relative, "sha256": sha256_file(upstream / relative)} for relative in SOURCE_FILES]
    e2e_files = sorted(path.name for path in (upstream / "tests/e2e").glob("*.test.ts"))
    e2e_digest = sha256_bytes(("\n".join(e2e_files) + "\n").encode())
    return sources, {
        "path_glob": "tests/e2e/*.test.ts",
        "member_count": len(e2e_files),
        "member_names_sha256": e2e_digest,
    }


def build_inventory(
    upstream: Path,
    lock: dict[str, str],
    policy: dict[str, Any],
    *,
    lock_path: str = "benchmarks/upstream.lock",
) -> dict[str, Any]:
    specs_text = read_text(upstream, COMMAND_SPECS)
    registry_text = read_text(upstream, COMMAND_REGISTRY)
    top_level = extract_top_level_commands(specs_text, registry_text)
    slash = extract_slash_commands(specs_text, registry_text)
    tools = extract_builtin_tools(read_text(upstream, TOOL_REGISTRY))
    sdk = extract_sdk_exports(upstream)
    e2e = extract_e2e_owners(upstream)
    source_files, source_set = source_provenance(upstream)
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


def markdown_cell(value: object) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ")


def code_list(values: list[str]) -> str:
    return ", ".join(f"`{value}`" for value in values) if values else "—"


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
        f"Comparison target: [`vercel-labs/fx` commit `{commit}`]({repository_page}/commit/{commit}),",
        f"pinned by `{upstream['lock_path']}`.",
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
        "| deferred | The surface is intentionally scheduled after the core engine and native host. |",
        "| intentional difference | The upstream behavior or implementation shape is explicitly not a compatibility target. |",
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
            f"| {labels[name]} | {status_labels[surface['status']]} | "
            f"{markdown_cell(surface['milestone'])} | {markdown_cell(surface['notes'])} |"
        )

    lines.extend(
        [
            "",
            "## Top-level CLI commands",
            "",
            f"Source: `{COMMAND_SPECS}` and `{COMMAND_REGISTRY}` ({surfaces['top_level_cli_commands']['count']} commands).",
            "",
            "| Kind | Token | Aliases | Hidden from help |",
            "| --- | --- | --- | --- |",
        ]
    )
    for command in surfaces["top_level_cli_commands"]["items"]:
        lines.append(
            f"| `{command['kind']}` | `{command['token']}` | {code_list(command['aliases'])} | "
            f"{'yes' if command['hidden_from_help'] else 'no'} |"
        )

    lines.extend(
        [
            "",
            "## Slash command kinds",
            "",
            f"Source: `{COMMAND_SPECS}` and `{COMMAND_REGISTRY}` ({surfaces['slash_command_kinds']['count']} kinds).",
            "",
            "| Kind | Command | Aliases | Presentation category | Accepts arguments |",
            "| --- | --- | --- | --- | --- |",
        ]
    )
    for command in surfaces["slash_command_kinds"]["items"]:
        category = command["presentation_category"]
        lines.append(
            f"| `{command['kind']}` | `{command['command']}` | {code_list(command['aliases'])} | "
            f"{f'`{category}`' if category else 'internal subcommand'} | "
            f"{'yes' if command['has_arguments'] else 'no'} |"
        )

    tool_surface = surfaces["builtin_tool_names"]
    lines.extend(
        [
            "",
            "## Built-in tools",
            "",
            f"Source: `{TOOL_REGISTRY}` ({tool_surface['count']} production registry entries).",
            "",
            code_list([tool["name"] for tool in tool_surface["items"]]) + ".",
            "",
            "## SDK exports",
            "",
            f"Package: `{surfaces['sdk_exports']['package']}`. Source: `{SDK_PACKAGE}` and the mapped modules.",
            "",
            "| Package entrypoint | Conditions and modules |",
            "| --- | --- |",
        ]
    )
    for entrypoint in surfaces["sdk_exports"]["entrypoints"]:
        targets = ", ".join(
            f"`{condition}` → `{target}`" for condition, target in entrypoint["targets"].items()
        )
        lines.append(f"| `{entrypoint['specifier']}` | {targets} |")
    lines.extend(["", "| Module | Named exports |", "| --- | --- |"])
    for module in surfaces["sdk_exports"]["modules"]:
        lines.append(f"| `{module['path']}` | {code_list(module['exports'])} |")

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
            f"Counts: {counts['training']} training, {counts['verification_only']} verification-only, "
            f"and {counts['intentional_exclusion']} intentional exclusions ({e2e_surface['count']} total).",
            "",
            "| Owner file | Upstream classification | Scenario or exclusion reason |",
            "| --- | --- | --- |",
        ]
    )
    for owner in e2e_surface["items"]:
        detail = owner["scenario"] if owner["scenario"] is not None else owner["reason"]
        classification = owner["classification"].replace("_", "-")
        lines.append(
            f"| `{owner['file']}` | {classification} | {markdown_cell(detail)} |"
        )

    lines.extend(["", "## Intentional differences", ""])
    for difference in inventory["intentional_differences"]:
        lines.append(
            f"- `{difference['id']}` ({difference['surface']}): {difference['notes']}"
        )

    lines.extend(
        [
            "",
            "## Regeneration and drift check",
            "",
            "The generator performs no network access. Point it at a clean checkout of the pinned",
            "commit; it verifies `HEAD` and the relevant worktree paths before reading sources.",
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
        verify_checkout(upstream, lock["commit"])
        lock_display = (
            "benchmarks/upstream.lock"
            if args.lock.resolve() == DEFAULT_LOCK
            else args.lock.name
        )
        inventory = build_inventory(upstream, lock, policy, lock_path=lock_display)
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
