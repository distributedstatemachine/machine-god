#!/usr/bin/env python3
"""Validate the repository's bounded, single-source documentation policy."""

from __future__ import annotations

import argparse
import re
import sys
from array import array
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import unquote, urlsplit


PLAN_PATH = Path("docs/implementation-plan.md")
START_MARKER = "<!-- canonical-live-status:start -->"
END_MARKER = "<!-- canonical-live-status:end -->"
MAX_PLAN_LINES = 600
MAX_MARKDOWN_BYTES = 262_144

GOVERNED_OVERVIEWS = (
    Path("README.md"),
    Path("docs/README.md"),
    Path("docs/reviews/README.md"),
    Path("docs/architecture.md"),
    Path("docs/security.md"),
    Path("docs/performance.md"),
    Path("docs/native-reference-host.md"),
    Path("docs/cli.md"),
    Path("docs/ask-cli.md"),
    Path("docs/ask-user-question.md"),
    Path("docs/session-cli.md"),
    Path("docs/native-session-inspection.md"),
    Path("docs/session-store.md"),
)

REQUIRED_LIVE_FIELDS = {
    "Delivered slices": re.compile(r"`[1-9][0-9]*`"),
    "Delivered main": re.compile(r"`[0-9a-f]{40}`"),
    "Main CI": re.compile(r"`[0-9]+` \(`GREEN`\)"),
    "Main Benchmark evidence": re.compile(r"`[0-9]+` \(`GREEN`\)"),
    "Active branch": re.compile(r"`agent/m[0-9]{2}-[a-z0-9-]+`"),
    "Active phase": re.compile(r"`[^`]+`"),
    "Next gate": re.compile(r"`[^`]+`"),
}

MARKDOWN_LINK_RE = re.compile(r"!?\[[^\]]*\]\(\s*(<[^>]+>|[^)\s]+)")
FENCE_OPEN_RE = re.compile(r"^[ ]{0,3}(`{3,}|~{3,})(.*)$")
FENCE_CLOSE_RE = re.compile(r"^[ ]{0,3}(`{3,}|~{3,})[ \t]*$")
BLOCK_QUOTE_PREFIX_RE = re.compile(r"[ ]{0,3}>[ \t]?")
BACKTICK_RUN_RE = re.compile(r"`+")
HTML_COMMENT_OPEN_RE = re.compile(r"<!--")
ACTIONS_RUN_ID_RE = re.compile(
    r"\b(?:github\s+actions|actions|workflow)(?:\s+run)?(?:\s+id)?"
    r"\s*(?:(?:is|was)\s+|[:#]\s*)?`?[0-9]{6,12}\b|"
    r"\b(?:ci|benchmark(?:\s+evidence)?)\s+run\s+`?[0-9]{6,12}\b|"
    r"/actions/runs/[0-9]+\b",
    re.IGNORECASE,
)
LIVE_STATUS_HEADER_RE = re.compile(
    r"^#{1,6}\s+(?:(?:current|live|delivery|implementation)\s+)?status\b",
    re.IGNORECASE | re.MULTILINE,
)
TOP_LEVEL_STATUS_RE = re.compile(r"^Status:\s+\S", re.IGNORECASE | re.MULTILINE)
DELIVERED_COUNT_RE = re.compile(
    r"\bdelivered[- ]slice count\b|\bdelivered count\b|"
    r"\bdelivered slices?\s*:\s*(?:[0-9]+|[a-z-]+)\b|"
    r"\b(?:[0-9]+|(?:one|two|three|four|five|six|seven|eight|nine|ten|"
    r"eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|"
    r"nineteen|twenty|thirty|forty)(?:-[a-z]+)?)\s+delivered(?:\s+bounded)?"
    r"\s+slices?\b|"
    r"\bslices?\s+[0-9]+\s+(?:is|are)\s+delivered\b|"
    r"\bdelivered\s+slice[- ]?[0-9]+\b",
    re.IGNORECASE,
)
REVISION_RE = r"`?(?<![0-9a-f])[0-9a-f]{7,40}(?![0-9a-f])`?"
DELIVERY_LINEAGE_RE = re.compile(
    rf"\b(?:candidate|delivery|delivered(?:-main)?|review(?:ed)?|commit|base|tree|"
    rf"revision)\b[^\n]{{0,120}}{REVISION_RE}|"
    rf"{REVISION_RE}[^\n]{{0,40}}\b(?:candidate|delivery|review|commit|tree|revision)\b|"
    rf"{REVISION_RE}\s*/\s*{REVISION_RE}",
    re.IGNORECASE,
)

REFERENCE_HOST_INVENTORY_EXEMPTIONS = {
    PLAN_PATH,
}
REFERENCE_HOST_CONTRACT_PATH = Path("docs/native-reference-host.md")
CARDINAL_UNIT_RE = (
    r"(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|"
    r"thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen)"
)
CARDINAL_TENS_RE = r"(?:twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety)"
COUNT_WORD_RE = (
    rf"(?:[0-9]+|a\s+dozen|{CARDINAL_TENS_RE}(?:-(?:one|two|three|four|five|"
    rf"six|seven|eight|nine))?|{CARDINAL_UNIT_RE})"
)
INVENTORY_COUNT_NOUN_PATTERN = (
    rf"\b{COUNT_WORD_RE}-(?:tool|clone|entry|built-in)\b|"
    rf"\b{COUNT_WORD_RE}\s+(?:ToolSpec|tool-spec)\s+"
    rf"(?:values?|objects?)\b|"
    rf"\b{COUNT_WORD_RE}\s+"
    rf"(?:[A-Za-z][A-Za-z0-9_-]*\s+){{0,5}}"
    rf"(?:tools?(?=\s*(?:$|[.,;:!?)]+|\b(?:are|is|were|was|that|which|who|"
    rf"run|execute|remain|exposed|registered|provided|installed|owned|supplied|"
    rf"listed|share|shares|shared|make|makes|made|use|uses|used|in|across)\b))|"
    rf"entries|clones?|built-ins?)\b|"
    rf"\b{COUNT_WORD_RE}\s+tool\s+(?:schemas?|specs?)\b"
)
REFERENCE_HOST_INVENTORY_COUNT_RE = re.compile(
    INVENTORY_COUNT_NOUN_PATTERN,
    re.IGNORECASE,
)
INVENTORY_COUNT_TOKEN_RE = re.compile(rf"\b{COUNT_WORD_RE}\b", re.IGNORECASE)
REFERENCE_HOST_CONTEXT_RE = re.compile(
    r"\breference[- ]host\b|\bNativeReferenceHost\b",
    re.IGNORECASE,
)
REFERENCE_HOST_CATALOG_CONTEXT_RE = re.compile(
    r"\b(?:tool|reference)\s+catalog\b|"
    r"\breference[- ]host\s+catalog\b|docs/native-reference-host\.md",
    re.IGNORECASE,
)
OPERATIONAL_QUANTIFIER_RE = re.compile(
    r"\b(?:at\s+most|up\s+to|no\s+more\s+than|maximum\s+of)\b",
    re.IGNORECASE,
)
OPERATIONAL_SCOPE_RE = re.compile(
    r"\b(?:concurrency|execution|scheduler|queue)\s+(?:limit|capacity)\b|"
    r"\b(?:active|executing|concurrent|in[- ]flight|queued|running)\s+"
    r"(?:set|pool|batch)\b",
    re.IGNORECASE,
)
OPERATIONAL_COUNT_RE = re.compile(
    r"\b(?:active|executing|concurrent|in[- ]flight|queued|running|Waker)\b",
    re.IGNORECASE,
)
INVENTORY_VERB_RE = re.compile(
    r"\b(?:register(?:s|ed)?|expos(?:e|es|ed)|include(?:s|d)?|"
    r"compris(?:e|es|ed)|provid(?:e|es|ed)|"
    r"install(?:s|ed)?|owns?|suppl(?:y|ies|ied)|lists?|totals?|has|"
    r"contain(?:s|ed)?)\b|"
    r"\b(?:make|makes|made)\s+up\b|\bcomposed\s+of\b",
    re.IGNORECASE,
)
OPERATIONAL_VERB_RE = re.compile(
    r"\b(?:accepts?|allows?|permits?|batches?|executes?|runs?|schedules?|queues?|"
    r"records?)\b",
    re.IGNORECASE,
)
OPERATIONAL_MODAL_AFTER_RE = re.compile(
    r"^\s*(?:may|can|must|will)\s+(?:execute|run|queue|schedule)\b",
    re.IGNORECASE,
)
STRONG_OPERATIONAL_AFTER_RE = re.compile(
    r"\b(?:per\s+[A-Za-z][A-Za-z-]*|"
    r"for\s+each\s+(?:supplied\s+)?Waker|"
    r"while\s+(?:a\s+)?calls?\s+is\s+active)\b",
    re.IGNORECASE,
)
RELATIVE_OPERATIONAL_AFTER_RE = re.compile(
    r"^\s*(?:that|which|who)\b", re.IGNORECASE
)
POSSESSIVE_INVENTORY_RE = re.compile(
    r"(?:NativeReferenceHost|reference[- ]host|host|catalog)'s\s*$",
    re.IGNORECASE,
)
INVENTORY_TOTAL_RE = re.compile(
    rf"\b(?:register(?:s|ed)?|expos(?:e|es|ed)|include(?:s|d)?|"
    rf"provid(?:e|es|ed)|contain(?:s|ed)?|has)\s+"
    rf"a\s+total\s+of\s+{COUNT_WORD_RE}\b",
    re.IGNORECASE,
)
INVENTORY_BARE_COUNT_RE = re.compile(
    rf"\b(?:register(?:s|ed)?|expos(?:e|es|ed)|include(?:s|d)?|"
    rf"provid(?:e|es|ed)|contain(?:s|ed)?|has)\s+"
    rf"(?:exactly\s+)?{COUNT_WORD_RE}(?=\s*(?:$|[.,;!?]))",
    re.IGNORECASE,
)
INVENTORY_NUMBER_OF_RE = re.compile(
    rf"\bnumber\s+of\s+(?:tools?(?=\s+(?:is|was|are|were|exposed|registered|"
    rf"provided|installed|owned|supplied|listed|in|across)\b)|entries|built-ins?|"
    rf"ToolSpec\s+(?:values?|objects?))"
    rf"\b[^.!?]{{0,120}}\b(?:is|was|equals?|totals?)\s+{COUNT_WORD_RE}\b",
    re.IGNORECASE,
)
REFERENCE_HOST_INVENTORY_SHORTHAND_RE = re.compile(
    rf"\b{COUNT_WORD_RE}-(?:tool|clone)\b[^.!?]{{0,160}}"
    rf"(?:\bcatalog\b|\breference[- ]host\b|\bhost\b)|"
    rf"(?:\bcatalog\b|\breference[- ]host\b|\bhost\b)"
    rf"[^.!?]{{0,160}}\b{COUNT_WORD_RE}-(?:tool|clone)\b|"
    rf"\bcomposition\b[^.!?]{{0,80}}\bcontain(?:s|ed)?\b"
    rf"[^.!?]{{0,80}}\b{COUNT_WORD_RE}\s+"
    rf"(?:[A-Za-z][A-Za-z0-9_-]*\s+){{0,5}}tools?\b|"
    rf"\bhost\b[^.!?]{{0,80}}\bhas\b[^.!?]{{0,40}}"
    rf"\b{COUNT_WORD_RE}\s+workspace(?:-backed)?\s+tools?\b|"
    rf"\b{COUNT_WORD_RE}\s+workspace(?:-backed)?\s+tools?\b"
    rf"[^.!?]{{0,120}}\b(?:share|shares|shared)\b[^.!?]{{0,80}}\bdescriptor\b|"
    rf"\b{COUNT_WORD_RE}\s+descriptor-backed\s+tools\b|"
    rf"\b{COUNT_WORD_RE}\s+identity-preserving\s+clones\b|"
    rf"\bdescriptor\b[^.!?]{{0,80}}\b{COUNT_WORD_RE}\s+clones\b"
    rf"[^.!?]{{0,80}}\b{COUNT_WORD_RE}\s+tools\b|"
    rf"\bworkspace\s+descriptor\b[^.!?]{{0,80}}\bdistribut(?:e|es|ed)\b"
    rf"[^.!?]{{0,80}}\b{COUNT_WORD_RE}\s+clones\b",
    re.IGNORECASE,
)
MARKDOWN_HEADING_RE = re.compile(
    r"^[ ]{0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$"
)
SETEXT_HEADING_RE = re.compile(r"^[ ]{0,3}(=+|-+)[ \t]*$")
CLAUSE_SPLIT_RE = re.compile(r"\s+(?:and|but|whereas)\s+|\s*;\s*", re.IGNORECASE)
SENTENCE_BOUNDARY_RE = re.compile(r"(?<=[.!?])\s+(?=[A-Z])")
SENTENCE_CONTEXT_RE = re.compile(
    r"^\s*(?:(?:However|Additionally|Therefore|Consequently|Moreover),?\s+)?"
    r"(?:It|Its|This\s+(?:host|catalog|composition)|"
    r"The\s+(?:host|catalog|composition))\b",
    re.IGNORECASE,
)
SENTENCE_ABBREVIATIONS = ("e.g.", "i.e.", "etc.", "vs.", "mr.", "mrs.", "dr.")


@dataclass(frozen=True)
class DocumentationStats:
    markdown_files: int = 0
    fence_lines: int = 0
    relative_links: int = 0
    unique_relative_targets: int = 0


@dataclass
class ValidationContext:
    """Cache bounded text and one inert-block scan per maintained file."""

    root: Path
    errors: list[str]
    texts: dict[Path, str | None] = field(default_factory=dict)
    scans: dict[Path, tuple[str, int, bool] | None] = field(default_factory=dict)

    def read(self, path: Path) -> str | None:
        if path not in self.texts:
            self.texts[path] = _read(path, self.root, self.errors)
        return self.texts[path]

    def scan(self, path: Path) -> tuple[str, int, bool] | None:
        if path not in self.scans:
            text = self.read(path)
            self.scans[path] = (
                None if text is None else _scan_markdown_inert_blocks(text)
            )
        return self.scans[path]


def markdown_files(root: Path) -> list[Path]:
    """Return maintained Markdown files, excluding checkout/build state."""

    ignored_parts = {".bench", ".git", "target"}
    return sorted(
        path
        for path in root.rglob("*.md")
        if ignored_parts.isdisjoint(path.relative_to(root).parts)
    )


def _read(path: Path, root: Path, errors: list[str]) -> str | None:
    try:
        with path.open("rb") as source:
            data = source.read(MAX_MARKDOWN_BYTES + 1)
        if len(data) > MAX_MARKDOWN_BYTES:
            errors.append(
                f"{path.relative_to(root)}: exceeds the "
                f"{MAX_MARKDOWN_BYTES}-byte Markdown ceiling"
            )
            return None
        return data.decode("utf-8")
    except (OSError, UnicodeError) as error:
        errors.append(f"{path.relative_to(root)}: cannot read UTF-8: {error}")
        return None


def _validate_live_status(context: ValidationContext, files: list[Path]) -> None:
    root = context.root
    errors = context.errors
    plan = root / PLAN_PATH
    text = context.read(plan)
    if text is None:
        return

    line_count = len(text.splitlines())
    if line_count > MAX_PLAN_LINES:
        errors.append(
            f"{PLAN_PATH}: {line_count} lines exceeds the {MAX_PLAN_LINES}-line ceiling"
        )

    marker_locations: dict[str, list[Path]] = {START_MARKER: [], END_MARKER: []}
    for path in files:
        candidate = context.read(path)
        if candidate is None:
            continue
        for marker in marker_locations:
            marker_locations[marker].extend([path] * candidate.count(marker))

    for marker, locations in marker_locations.items():
        if locations != [plan]:
            rendered = ", ".join(str(path.relative_to(root)) for path in locations)
            errors.append(
                f"canonical marker {marker!r} must occur exactly once in {PLAN_PATH}; "
                f"found [{rendered}]"
            )

    if text.count(START_MARKER) != 1 or text.count(END_MARKER) != 1:
        return
    start = text.index(START_MARKER) + len(START_MARKER)
    end = text.index(END_MARKER)
    if start >= end:
        errors.append(f"{PLAN_PATH}: canonical live-status markers are out of order")
        return

    fields: dict[str, list[str]] = {}
    for line in text[start:end].splitlines():
        match = re.fullmatch(r"- ([^:]+): (.+)", line)
        if match:
            fields.setdefault(match.group(1), []).append(match.group(2))

    if set(fields) != set(REQUIRED_LIVE_FIELDS):
        missing = sorted(set(REQUIRED_LIVE_FIELDS) - set(fields))
        extra = sorted(set(fields) - set(REQUIRED_LIVE_FIELDS))
        errors.append(
            f"{PLAN_PATH}: live fields differ; missing={missing}, extra={extra}"
        )
    for name, pattern in REQUIRED_LIVE_FIELDS.items():
        values = fields.get(name, [])
        if len(values) != 1:
            errors.append(f"{PLAN_PATH}: live field {name!r} must occur exactly once")
        elif pattern.fullmatch(values[0]) is None:
            errors.append(
                f"{PLAN_PATH}: live field {name!r} has invalid value {values[0]!r}"
            )

    delivered_values = fields.get("Delivered slices", [])
    if len(delivered_values) == 1:
        delivered = int(delivered_values[0].strip("`"))
        inventory = [
            int(match.group(1))
            for match in re.finditer(r"^\|\s*([0-9]+)\s*\|", text, re.MULTILINE)
        ]
        expected = list(range(1, delivered + 1))
        if inventory != expected:
            errors.append(
                f"{PLAN_PATH}: delivered inventory must contain exactly slices "
                f"1..{delivered}; found {inventory}"
            )


def _fence_opener(line: str) -> tuple[str, int] | None:
    """Return one CommonMark-compatible fenced-code opener."""

    match = FENCE_OPEN_RE.match(line)
    if match is None:
        return None
    token, info = match.groups()
    if token[0] == "`" and "`" in info:
        return None
    return token[0], len(token)


def _is_fence_closer(line: str, open_fence: tuple[str, int]) -> bool:
    """Return whether a line closes the exact active fence family."""

    match = FENCE_CLOSE_RE.match(line)
    if match is None:
        return False
    token = match.group(1)
    return token[0] == open_fence[0] and len(token) >= open_fence[1]


def _block_quote_content(line: str) -> tuple[int, int, str]:
    """Return block-quote depth, content offset, and container-free content."""

    depth = 0
    cursor = 0
    while True:
        match = BLOCK_QUOTE_PREFIX_RE.match(line, cursor)
        if match is None:
            break
        depth += 1
        cursor = match.end()
    return depth, cursor, line[cursor:]


def _backtick_runs(text: str) -> tuple[array, array, array]:
    """Index paragraph-local exact backtick runs in compact numeric arrays."""

    starts = array("I")
    ends = array("I")
    next_same = array("i")

    def add_segment(segment_start: int, segment_end: int) -> None:
        first_index = len(starts)
        for match in BACKTICK_RUN_RE.finditer(text, segment_start, segment_end):
            starts.append(match.start())
            ends.append(match.end())
            next_same.append(-1)
        nearest: dict[int, int] = {}
        for index in range(len(starts) - 1, first_index - 1, -1):
            length = ends[index] - starts[index]
            next_same[index] = nearest.get(length, -1)
            nearest[length] = index

    segment_start = 0
    offset = 0
    for raw_line in text.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        line_start = offset
        offset += len(raw_line)
        _, _, block_content = _block_quote_content(content)
        is_block_boundary = (
            not content.strip()
            or _fence_opener(block_content) is not None
            or FENCE_CLOSE_RE.match(block_content) is not None
        )
        if is_block_boundary:
            add_segment(segment_start, line_start)
            segment_start = offset
    add_segment(segment_start, len(text))
    return starts, ends, next_same


def _scan_markdown_inert_blocks(text: str) -> tuple[str, int, bool]:
    """Strip fences/comments with linear, shared Markdown state."""

    run_starts, run_ends, next_same_run = _backtick_runs(text)
    comment_starts = [match.start() for match in HTML_COMMENT_OPEN_RE.finditer(text)]
    prose: list[str] = []
    open_fence: tuple[str, int, int] | None = None
    unclosed_fence = False
    in_comment = False
    fence_lines = 0
    run_index = 0
    code_span_end: int | None = None
    comment_index = 0
    offset = 0

    for raw_line in text.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        newline = raw_line[len(content) :]
        line_start = offset
        line_end = line_start + len(content)
        offset += len(raw_line)

        while (
            run_index < len(run_starts)
            and run_ends[run_index] <= line_start
        ):
            run_index += 1
        while (
            comment_index < len(comment_starts)
            and comment_starts[comment_index] < line_start
        ):
            comment_index += 1

        quote_depth, _, block_content = _block_quote_content(content)
        if open_fence is not None:
            required_quote_depth = open_fence[2]
            if required_quote_depth > 0 and quote_depth < required_quote_depth:
                unclosed_fence = True
                open_fence = None
            else:
                if (
                    quote_depth == required_quote_depth
                    and _is_fence_closer(
                        block_content, (open_fence[0], open_fence[1])
                    )
                ):
                    open_fence = None
                    fence_lines += 1
                while (
                    run_index < len(run_starts)
                    and run_starts[run_index] < line_end
                ):
                    run_index += 1
                prose.append(newline)
                continue

        if code_span_end is not None and code_span_end <= line_start:
            code_span_end = None
        continues_code_span = code_span_end is not None
        if not in_comment and not continues_code_span:
            opener = _fence_opener(block_content)
            if opener is not None:
                open_fence = (opener[0], opener[1], quote_depth)
                fence_lines += 1
                while (
                    run_index < len(run_starts)
                    and run_starts[run_index] < line_end
                ):
                    run_index += 1
                prose.append(newline)
                continue

        visible: list[str] = []
        cursor = line_start
        while cursor < line_end:
            if code_span_end is not None:
                segment_end = min(code_span_end, line_end)
                visible.append(text[cursor:segment_end])
                cursor = segment_end
                if cursor >= code_span_end:
                    code_span_end = None
                continue
            if in_comment:
                end = text.find("-->", cursor, line_end)
                if end < 0:
                    cursor = line_end
                    break
                in_comment = False
                cursor = end + 3
                continue

            while (
                run_index < len(run_starts)
                and run_ends[run_index] <= cursor
            ):
                run_index += 1
            while (
                comment_index < len(comment_starts)
                and comment_starts[comment_index] < cursor
            ):
                comment_index += 1

            run_start = (
                run_starts[run_index]
                if run_index < len(run_starts) and run_starts[run_index] < line_end
                else line_end
            )
            comment_start = (
                comment_starts[comment_index]
                if comment_index < len(comment_starts)
                and comment_starts[comment_index] < line_end
                else line_end
            )

            if run_start < comment_start and run_index < len(run_starts):
                close_index = next_same_run[run_index]
                if close_index < 0:
                    segment_end = run_ends[run_index]
                    run_index += 1
                else:
                    code_span_end = run_ends[close_index]
                    run_index = close_index + 1
                    segment_end = min(code_span_end, line_end)
                visible.append(text[cursor:segment_end])
                cursor = segment_end
                if code_span_end is not None and cursor >= code_span_end:
                    code_span_end = None
                continue
            if comment_start < line_end:
                visible.append(text[cursor:comment_start])
                in_comment = True
                cursor = comment_start + 4
                comment_index += 1
                continue

            visible.append(text[cursor:line_end])
            cursor = line_end

        prose.append("".join(visible) + newline)

    return "".join(prose), fence_lines, unclosed_fence or open_fence is not None


def _prose_without_fenced_blocks(text: str) -> str:
    """Return visible prose while fenced blocks and HTML comments stay inert."""

    prose, _, _ = _scan_markdown_inert_blocks(text)
    return prose


def _without_unique_markdown_section(text: str, title: str) -> tuple[str, int]:
    """Blank one uniquely named section through the next peer heading."""

    lines = text.splitlines()
    headings: list[tuple[int, int, str]] = []
    for index, line in enumerate(lines):
        atx = MARKDOWN_HEADING_RE.match(line)
        if atx is not None:
            headings.append(
                (index, len(atx.group(1)), atx.group(2).strip().casefold())
            )
            continue
        if index + 1 >= len(lines) or not line.strip():
            continue
        setext = SETEXT_HEADING_RE.match(lines[index + 1])
        if setext is not None:
            level = 1 if setext.group(1).startswith("=") else 2
            headings.append((index, level, line.strip().casefold()))
    matches = [heading for heading in headings if heading[2] == title.casefold()]
    if len(matches) != 1:
        return text, len(matches)

    start, section_level, _ = matches[0]
    end = len(lines)
    for index, level, _ in headings:
        if index > start and level <= section_level:
            end = index
            break
    lines[start:end] = [""] * (end - start)
    return "\n".join(lines), 1


def _validate_governed_overviews(context: ValidationContext) -> None:
    root = context.root
    errors = context.errors
    for relative in GOVERNED_OVERVIEWS:
        path = root / relative
        scan = context.scan(path)
        if scan is None:
            continue
        prose = scan[0]
        if ACTIONS_RUN_ID_RE.search(prose):
            errors.append(f"{relative}: must not contain GitHub Actions run IDs")
        if DELIVERED_COUNT_RE.search(prose):
            errors.append(f"{relative}: must not contain a delivered-count phrase")
        if LIVE_STATUS_HEADER_RE.search(prose):
            errors.append(f"{relative}: must not contain a live status header")
        if TOP_LEVEL_STATUS_RE.search(prose):
            errors.append(f"{relative}: must not contain mutable top-level Status prose")
        if DELIVERY_LINEAGE_RE.search(prose):
            errors.append(f"{relative}: must not contain SHA-style delivery lineage")


def _validate_reference_host_inventory(
    context: ValidationContext, files: list[Path]
) -> None:
    """Keep mutable reference-host inventory facts in their canonical contract."""

    root = context.root
    errors = context.errors
    for path in files:
        relative = path.relative_to(root)
        if relative in REFERENCE_HOST_INVENTORY_EXEMPTIONS:
            continue
        if len(relative.parts) >= 2 and relative.parts[:2] == ("docs", "reviews"):
            continue

        scan = context.scan(path)
        if scan is None:
            continue
        prose = scan[0]
        if relative == REFERENCE_HOST_CONTRACT_PATH:
            prose, section_count = _without_unique_markdown_section(
                prose, "Tool catalog"
            )
            if section_count != 1:
                errors.append(
                    f"{relative}: must contain exactly one Tool catalog section; "
                    f"found {section_count}"
                )
        has_inventory = _document_contains_reference_host_inventory(prose)

        if has_inventory:
            errors.append(
                f"{relative}: reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog"
            )


def _heading_at(lines: list[str], index: int) -> tuple[int, str, int] | None:
    """Return `(level, title, consumed_lines)` for one Markdown heading."""

    atx = MARKDOWN_HEADING_RE.match(lines[index])
    if atx is not None:
        return len(atx.group(1)), atx.group(2).strip(), 1
    if index + 1 >= len(lines) or not lines[index].strip():
        return None
    setext = SETEXT_HEADING_RE.match(lines[index + 1])
    if setext is None:
        return None
    level = 1 if setext.group(1).startswith("=") else 2
    return level, lines[index].strip(), 2


def _document_contains_reference_host_inventory(prose: str) -> bool:
    """Scan paragraphs while carrying their Markdown section context."""

    lines = prose.splitlines()
    section_context: dict[int, bool] = {}
    paragraph: list[str] = []
    carried_paragraph_context = False

    def inherited_context() -> bool:
        return any(section_context.values())

    def flush_paragraph() -> bool:
        nonlocal carried_paragraph_context
        if not paragraph:
            return False
        text = "\n".join(paragraph)
        anaphoric_context = (
            carried_paragraph_context
            and SENTENCE_CONTEXT_RE.search(text) is not None
        )
        active_context = inherited_context() or anaphoric_context
        has_inventory = _contains_reference_host_inventory(text, active_context)
        own_context = (
            REFERENCE_HOST_CONTEXT_RE.search(text) is not None
            or REFERENCE_HOST_CATALOG_CONTEXT_RE.search(text) is not None
        )
        carried_paragraph_context = own_context or anaphoric_context
        return has_inventory

    index = 0
    while index < len(lines):
        heading = _heading_at(lines, index)
        if heading is not None:
            if flush_paragraph():
                return True
            paragraph.clear()
            carried_paragraph_context = False
            level, title, consumed = heading
            parent_context = any(
                active for active_level, active in section_context.items()
                if active_level < level
            )
            section_context = {
                active_level: active
                for active_level, active in section_context.items()
                if active_level < level
            }
            own_context = (
                REFERENCE_HOST_CONTEXT_RE.search(title) is not None
                or REFERENCE_HOST_CATALOG_CONTEXT_RE.search(title) is not None
            )
            section_context[level] = parent_context or own_context
            if _contains_reference_host_inventory(title, parent_context or own_context):
                return True
            index += consumed
            continue
        if not lines[index].strip():
            if flush_paragraph():
                return True
            paragraph.clear()
        else:
            paragraph.append(lines[index])
        index += 1

    return flush_paragraph()


def _contains_reference_host_inventory(
    paragraph: str, inherited_context: bool = False
) -> bool:
    """Recognize inventory statements without confusing operational limits."""

    normalized = re.sub(r"\s+", " ", paragraph).strip()
    if not normalized:
        return False

    sentences: list[str] = []
    start = 0
    for boundary in SENTENCE_BOUNDARY_RE.finditer(normalized):
        prefix = normalized[max(0, boundary.start() - 8) : boundary.start()].casefold()
        if any(prefix.endswith(abbreviation) for abbreviation in SENTENCE_ABBREVIATIONS):
            continue
        sentences.append(normalized[start : boundary.start()])
        start = boundary.end()
    sentences.append(normalized[start:])

    carried_context = False
    for sentence in sentences:
        own_host_context = REFERENCE_HOST_CONTEXT_RE.search(sentence) is not None
        own_catalog_context = (
            REFERENCE_HOST_CATALOG_CONTEXT_RE.search(sentence) is not None
        )
        inherited_sentence_context = (
            carried_context and SENTENCE_CONTEXT_RE.search(sentence) is not None
        )
        sentence_has_host = (
            inherited_context or inherited_sentence_context or own_host_context
        )
        sentence_has_catalog = (
            inherited_context or inherited_sentence_context or own_catalog_context
        )

        for clause in CLAUSE_SPLIT_RE.split(sentence):
            counts = list(REFERENCE_HOST_INVENTORY_COUNT_RE.finditer(clause))
            nonoperational_counts = [
                match
                for match in counts
                if not _counted_noun_is_operational(clause, match)
            ]
            if (
                REFERENCE_HOST_INVENTORY_SHORTHAND_RE.search(clause) is not None
                and nonoperational_counts
            ):
                return True

            has_host_context = (
                sentence_has_host
                or REFERENCE_HOST_CONTEXT_RE.search(clause) is not None
            )
            has_catalog_context = (
                sentence_has_catalog
                or REFERENCE_HOST_CATALOG_CONTEXT_RE.search(clause) is not None
            )
            if (has_host_context or has_catalog_context) and nonoperational_counts:
                return True

            if has_host_context or has_catalog_context:
                if INVENTORY_NUMBER_OF_RE.search(clause) is not None:
                    return True
                for pattern in (INVENTORY_TOTAL_RE, INVENTORY_BARE_COUNT_RE):
                    for total in pattern.finditer(clause):
                        after = clause[
                            total.end() : min(len(clause), total.end() + 120)
                        ].split(",", 1)[0]
                        if STRONG_OPERATIONAL_AFTER_RE.search(after) is None:
                            return True

            has_count = INVENTORY_COUNT_TOKEN_RE.search(clause) is not None
            has_catalog_size = re.search(
                r"\bcatalog\s+size\b|\bsize\s+(?:is|of)\b",
                clause,
                re.IGNORECASE,
            )
            if (
                (has_host_context or has_catalog_context)
                and has_count
                and has_catalog_size is not None
            ):
                return True

        carried_context = own_host_context or own_catalog_context or (
            inherited_sentence_context and (sentence_has_host or sentence_has_catalog)
        )

    return False


def _counted_noun_is_operational(clause: str, match: re.Match[str]) -> bool:
    """Classify one counted noun from its bounded local clause context."""

    before = clause[max(0, match.start() - 120) : match.start()].rsplit(",", 1)[-1]
    counted = match.group(0)
    after = clause[match.end() : min(len(clause), match.end() + 120)].split(",", 1)[0]
    if OPERATIONAL_COUNT_RE.search(counted) is not None:
        return True
    if STRONG_OPERATIONAL_AFTER_RE.search(after) is not None:
        return True
    if (
        OPERATIONAL_SCOPE_RE.search(before) is not None
    ):
        return True
    if RELATIVE_OPERATIONAL_AFTER_RE.search(after) is not None:
        return False
    if POSSESSIVE_INVENTORY_RE.search(before) is not None:
        return False
    if INVENTORY_VERB_RE.search(before) is not None:
        return False
    return (
        OPERATIONAL_VERB_RE.search(before) is not None
        or OPERATIONAL_MODAL_AFTER_RE.search(after) is not None
    )


def _relative_link_target(raw_target: str) -> str | None:
    target = raw_target[1:-1] if raw_target.startswith("<") else raw_target
    target = unquote(target.replace("\\ ", " "))
    if target.startswith("#"):
        return None
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc or target.startswith("/"):
        return None
    return parsed.path or None


def _validate_markdown(
    context: ValidationContext, files: list[Path]
) -> DocumentationStats:
    root = context.root
    errors = context.errors
    fence_lines = 0
    relative_links = 0
    unique_targets: set[Path] = set()

    for path in files:
        text = context.read(path)
        scan = context.scan(path)
        if text is None or scan is None:
            continue

        _, file_fence_lines, unclosed_fence = scan
        fence_lines += file_fence_lines
        if unclosed_fence:
            errors.append(f"{path.relative_to(root)}: unclosed Markdown fence")

        for match in MARKDOWN_LINK_RE.finditer(text):
            target = _relative_link_target(match.group(1))
            if target is None:
                continue
            relative_links += 1
            resolved = (path.parent / target).resolve()
            unique_targets.add(resolved)
            try:
                resolved.relative_to(root.resolve())
            except ValueError:
                errors.append(
                    f"{path.relative_to(root)}: relative link escapes repository: {target}"
                )
                continue
            if not resolved.exists():
                errors.append(
                    f"{path.relative_to(root)}: missing relative link target: {target}"
                )

    return DocumentationStats(
        markdown_files=len(files),
        fence_lines=fence_lines,
        relative_links=relative_links,
        unique_relative_targets=len(unique_targets),
    )


def validate_repository(root: Path) -> tuple[list[str], DocumentationStats]:
    root = root.resolve()
    errors: list[str] = []
    context = ValidationContext(root, errors)
    files = markdown_files(root)
    _validate_live_status(context, files)
    _validate_governed_overviews(context)
    _validate_reference_host_inventory(context, files)
    stats = _validate_markdown(context, files)
    return errors, stats


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root (defaults to the script's parent repository)",
    )
    args = parser.parse_args(argv)
    errors, stats = validate_repository(args.root)
    print(
        "documentation: "
        f"markdown={stats.markdown_files} "
        f"fences={stats.fence_lines} "
        f"relative_links={stats.relative_links} "
        f"unique_targets={stats.unique_relative_targets} "
        f"errors={len(errors)}"
    )
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
