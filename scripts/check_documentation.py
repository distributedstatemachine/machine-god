#!/usr/bin/env python3
"""Validate the repository's bounded, single-source documentation policy."""

from __future__ import annotations

import argparse
import re
import sys
from array import array
from collections.abc import Iterator
from dataclasses import dataclass, field
from html import unescape as unescape_html
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

FENCE_OPEN_RE = re.compile(r"^[ ]{0,3}(`{3,}|~{3,})(.*)$")
FENCE_CLOSE_RE = re.compile(r"^[ ]{0,3}(`{3,}|~{3,})[ \t]*$")
BLOCK_QUOTE_PREFIX_RE = re.compile(r"[ ]{0,3}>[ ]?")
LIST_MARKER_RE = re.compile(r"([ ]{0,3})([-+*]|[0-9]{1,9}[.)])")
THEMATIC_BREAK_RE = re.compile(r"[ ]{0,3}(?:(?:\*[ ]*){3,}|(?:_[ ]*){3,}|(?:-[ ]*){3,})$")
BACKTICK_RUN_RE = re.compile(r"`+")
HTML_COMMENT_OPEN_RE = re.compile(r"<!--")
PARAGRAPH_ATX_RE = re.compile(r"^[ ]{0,3}#{1,6}(?:[ \t]+|$)")
HTML_RAW_BLOCK_RE = re.compile(
    r"^[ ]{0,3}</?(?:address|article|aside|base|basefont|blockquote|body|caption|"
    r"center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption|"
    r"figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe|legend|"
    r"li|link|main|menu|menuitem|nav|noframes|ol|optgroup|option|p|param|search|"
    r"section|summary|table|tbody|td|tfoot|th|thead|title|tr|track|ul)"
    r"(?:[ \t]|/?>|$)|"
    r"^[ ]{0,3}<(?:script|pre|style|textarea)(?:[ \t]|>|$)",
    re.IGNORECASE,
)
HTML_DECLARATION_BLOCK_RE = re.compile(
    r"^[ ]{0,3}(?:<!--|<\?|<![A-Z]|<!\[CDATA\[)"
)
HTML_ENTITY_RE = re.compile(
    r"&(?:#[xX][0-9A-Fa-f]{1,8}|#[0-9]{1,8}|[A-Za-z][A-Za-z0-9]{1,31});"
)
ACTIONS_RUN_ID_RE = re.compile(
    r"\b(?:github\s+actions|actions|workflow)(?:\s+run)?(?:\s+id)?"
    r"\s*(?:(?:is|was)\s+|[:#]\s*)?`?[0-9]{6,12}\b|"
    r"\b(?:ci|benchmark(?:\s+evidence)?)\s+run\s+`?[0-9]{6,12}\b|"
    r"/actions/runs/[0-9]+\b",
    re.IGNORECASE,
)
LIVE_STATUS_HEADER_RE = re.compile(
    r"^#{1,6}\s+(?:(?:current|live|delivery|implementation)\s+status|status)"
    r"\s*#*\s*$",
    re.IGNORECASE | re.MULTILINE,
)
TOP_LEVEL_STATUS_RE = re.compile(
    r"^Status:\s+(?=.*\b(?:deliver(?:ed|y)|integrated|milestone|slice|workflow|"
    r"candidate|pending)\b)\S",
    re.IGNORECASE | re.MULTILINE,
)
DELIVERED_COUNT_RE = re.compile(
    r"\bdelivered[- ]slice count\s*(?:is|:)\s*(?:[0-9]+|[a-z-]+)\b|"
    r"\bdelivered count\s*(?:is|:)\s*(?:[0-9]+|[a-z-]+)\b|"
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
    rf"\b(?:candidate|delivery(?:\s+seal)?|delivered(?:-main)?|review(?:ed)?|"
    rf"exact\s+base|main\s+sha|tree)\b[^\n]{{0,120}}{REVISION_RE}|"
    rf"{REVISION_RE}[^\n]{{0,40}}\b(?:candidate|delivery(?:\s+seal)?|review|"
    rf"exact\s+base|main\s+sha|tree)\b|"
    rf"{REVISION_RE}\s*/\s*{REVISION_RE}",
    re.IGNORECASE,
)
NON_PLAN_LIVE_FIELD_RE = re.compile(
    r"^[ ]{0,3}(?:#{1,6}[ \t]+)?"
    r"(Main CI|Main Benchmark evidence|Active branch|Active phase|Next gate):"
    r"[ \t]+\S",
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
    rf"entries|clones?|built-ins?|capabilit(?:y|ies))\b|"
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
    r"retr(?:y|ies|ied))\b",
    re.IGNORECASE,
)
OPERATIONAL_MODAL_AFTER_RE = re.compile(
    r"^\s*(?:may|can|must|will)\s+(?:execute|run|queue|schedule)\b",
    re.IGNORECASE,
)
STRONG_OPERATIONAL_AFTER_RE = re.compile(
    r"(?:^\s*(?:as\s+)?active\b)|"
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
    rf"(?:exactly\s+|at\s+most\s+|up\s+to\s+|no\s+more\s+than\s+|"
    rf"not\s+more\s+than\s+|fewer\s+than\s+|"
    rf"a\s+maximum\s+of\s+)?"
    rf"{COUNT_WORD_RE}(?=\s*(?:$|[.,;!?]))",
    re.IGNORECASE,
)
INVENTORY_IMPLIED_CATALOG_COUNT_RE = re.compile(
    rf"\bcatalog\s+of\s+(?:exactly\s+|at\s+most\s+|up\s+to\s+|"
    rf"no\s+more\s+than\s+|a\s+maximum\s+of\s+)?"
    rf"{COUNT_WORD_RE}(?=\s*(?:$|[.,;!?]))",
    re.IGNORECASE,
)
INVENTORY_NUMBER_OF_RE = re.compile(
    rf"(?:\b(?:number|count)\s+of\s+(?:tools?(?=\s+(?:is|was|are|were|exposed|"
    rf"registered|provided|installed|owned|supplied|listed|in|across)\b)|entries|"
    rf"built-ins?|ToolSpec\s+(?:values?|objects?))\b|"
    rf"\btool\s+count\s+of\b)"
    rf"[^.!?]{{0,120}}\b(?:is|was|equals?|totals?)\s+{COUNT_WORD_RE}\b",
    re.IGNORECASE,
)
INVENTORY_TOOL_COUNT_RE = re.compile(
    rf"\btool\s+count\b[^.!?]{{0,120}}"
    rf"(?:\b(?:is|was|equals?|totals?)\s+|\bof\s+){COUNT_WORD_RE}\b",
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
    r"^\s*(?:(?:[A-Za-z][A-Za-z-]*)(?:\s+[A-Za-z][A-Za-z-]*){0,3},\s+){0,2}"
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


@dataclass(frozen=True)
class MarkdownScan:
    """One bounded scan projected for prose policy and rendered links."""

    policy_prose: str
    classifier_prose: str
    link_markup: str
    fence_lines: int
    unclosed_fence: bool


@dataclass
class ValidationContext:
    """Cache bounded text and one inert-block scan per maintained file."""

    root: Path
    errors: list[str]
    texts: dict[Path, str | None] = field(default_factory=dict)
    scans: dict[Path, MarkdownScan | None] = field(default_factory=dict)

    def read(self, path: Path) -> str | None:
        if path not in self.texts:
            self.texts[path] = _read(path, self.root, self.errors)
        return self.texts[path]

    def scan(self, path: Path) -> MarkdownScan | None:
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


ContainerFrame = tuple[str, int]
ContainerPath = tuple[ContainerFrame, ...]


@dataclass(frozen=True)
class ContainerState:
    """Ordered container ownership plus cached blank-line behavior."""

    frames: ContainerPath
    blank_frames: ContainerPath
    list_only: bool


EMPTY_CONTAINER = ContainerState((), (), False)


@dataclass(frozen=True)
class ParsedContainerLine:
    state: ContainerState
    content_offset: int
    content: str

    @property
    def path(self) -> ContainerPath:
        return self.state.frames


def _container_state(frames: ContainerPath) -> ContainerState:
    """Cache the list prefix that may own a following physical blank line."""

    if not frames:
        return EMPTY_CONTAINER
    leading_lists = 0
    while leading_lists < len(frames) and frames[leading_lists][0] == "list":
        leading_lists += 1
    blank_frames = frames if leading_lists == len(frames) else frames[:leading_lists]
    return ContainerState(frames, blank_frames, leading_lists == len(frames))


def _match_container_path(
    line: str, expected: ContainerPath
) -> tuple[ContainerPath, int]:
    """Match the longest ordered quote/list prefix from one expanded line."""

    matched: list[ContainerFrame] = []
    cursor = 0
    for frame in expected:
        kind, width = frame
        if kind == "quote":
            marker = BLOCK_QUOTE_PREFIX_RE.match(line, cursor)
            if marker is None:
                break
            cursor = marker.end()
        else:
            if line[cursor : cursor + width] != " " * width:
                break
            cursor += width
        matched.append(frame)
    return tuple(matched), cursor


def _list_marker(
    line: str, cursor: int
) -> tuple[int, int, str, bool] | None:
    """Return continuation width, content offset, marker, and emptiness."""

    marker = LIST_MARKER_RE.match(line, cursor)
    if marker is None:
        return None
    token = marker.group(2)
    marker_end = marker.end()
    marker_width = marker_end - cursor
    if marker_end == len(line):
        return marker_width + 1, marker_end, token, True
    if line[marker_end] != " ":
        return None

    padding_end = marker_end
    while padding_end < len(line) and line[padding_end] == " ":
        padding_end += 1
    if padding_end == len(line):
        return marker_width + 1, padding_end, token, True

    padding = padding_end - marker_end
    effective_padding = padding if padding <= 4 else 1
    return (
        marker_width + effective_padding,
        marker_end + effective_padding,
        token,
        False,
    )


def _parse_container_line(
    line: str, ambient: ContainerState
) -> ParsedContainerLine:
    """Parse a bounded ordered CommonMark quote/list container prefix."""

    if not line.strip():
        frames = ambient.blank_frames
        state = (
            EMPTY_CONTAINER
            if not frames
            else ContainerState(frames, frames, True)
        )
        return ParsedContainerLine(state, len(line), "")

    matched, cursor = _match_container_path(line, ambient.frames)
    path = list(matched)
    while True:
        quote = BLOCK_QUOTE_PREFIX_RE.match(line, cursor)
        if quote is not None:
            path.append(("quote", 0))
            cursor = quote.end()
            continue
        marker = _list_marker(line, cursor)
        if marker is not None:
            width, content_offset, _, empty = marker
            path.append(("list", width))
            cursor = content_offset
            if empty:
                break
            continue
        break
    state = _container_state(tuple(path))
    return ParsedContainerLine(state, cursor, line[cursor:])


def _backtick_runs(text: str) -> tuple[array, array, array, bytearray]:
    """Index paragraph-local exact and outside-code escaped backtick runs."""

    starts = array("I")
    ends = array("I")
    next_same = array("i")
    escaped = bytearray()

    def add_segment(segment_start: int, segment_end: int) -> None:
        first_index = len(starts)
        for match in BACKTICK_RUN_RE.finditer(text, segment_start, segment_end):
            slash = match.start() - 1
            slash_count = 0
            while slash >= segment_start and text[slash] == "\\":
                slash_count += 1
                slash -= 1
            starts.append(match.start())
            ends.append(match.end())
            next_same.append(-1)
            escaped.append(slash_count % 2)
        nearest: dict[int, int] = {}
        for index in range(len(starts) - 1, first_index - 1, -1):
            length = ends[index] - starts[index]
            next_same[index] = nearest.get(length, -1)
            nearest[length] = index

    segment_start = 0
    offset = 0
    ambient = EMPTY_CONTAINER
    for raw_line in text.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        line_start = offset
        offset += len(raw_line)
        paragraph_continues, _ = _paragraph_line_continuation(
            content,
            ambient,
            html_blocks_interrupt=False,
        )
        parsed = _parse_container_line(content, ambient)
        ambient = parsed.state
        fence_boundary = (
            _fence_opener(parsed.content) is not None
            or FENCE_CLOSE_RE.match(parsed.content) is not None
        )
        declaration_block = HTML_DECLARATION_BLOCK_RE.match(parsed.content)
        html_leaf_boundary = (
            HTML_RAW_BLOCK_RE.match(parsed.content) is not None
            or (
                declaration_block is not None
                and not parsed.content.lstrip(" ").startswith("<!--")
            )
        )
        leaf_boundary = (
            PARAGRAPH_ATX_RE.match(parsed.content) is not None
            or SETEXT_HEADING_RE.match(parsed.content) is not None
            or THEMATIC_BREAK_RE.match(parsed.content) is not None
            or html_leaf_boundary
        )
        if not content.strip() or fence_boundary:
            add_segment(segment_start, line_start)
            segment_start = offset
        elif leaf_boundary:
            add_segment(segment_start, line_start)
            add_segment(line_start, offset)
            segment_start = offset
        elif not paragraph_continues:
            add_segment(segment_start, line_start)
            segment_start = line_start
    add_segment(segment_start, len(text))
    return starts, ends, next_same, escaped


def _paragraph_line_continuation(
    line: str,
    owner: ContainerState,
    *,
    html_blocks_interrupt: bool = True,
) -> tuple[bool, int]:
    """Classify one physical line before extending an owning paragraph."""

    if not line.strip():
        return False, 0
    matched, owner_offset = _match_container_path(line, owner.frames)
    explicit_owner = matched == owner.frames
    candidate_offset = owner_offset if explicit_owner else 0
    candidate = line[candidate_offset:]
    if (
        _fence_opener(candidate) is not None
        or PARAGRAPH_ATX_RE.match(candidate) is not None
        or SETEXT_HEADING_RE.match(candidate) is not None
        or THEMATIC_BREAK_RE.match(candidate) is not None
        or BLOCK_QUOTE_PREFIX_RE.match(candidate) is not None
        or (
            html_blocks_interrupt
            and (
                HTML_RAW_BLOCK_RE.match(candidate) is not None
                or HTML_DECLARATION_BLOCK_RE.match(candidate) is not None
            )
        )
    ):
        return False, candidate_offset

    marker = _list_marker(candidate, 0)
    if marker is not None:
        _, _, token, empty = marker
        ordered = token[0].isdigit()
        if not empty and (not ordered or int(token[:-1]) == 1):
            return False, candidate_offset
    return True, candidate_offset


def _blank_for_links(value: str) -> str:
    """Blank inert inline code without joining surrounding link syntax."""

    return re.sub(r"[^\r\n]", " ", value)


def _scan_markdown_inert_blocks(source: str) -> MarkdownScan:
    """Project bounded Markdown once with ordered container ownership."""

    text = source.expandtabs(4)
    run_starts, run_ends, next_same_run, escaped_runs = _backtick_runs(text)
    comment_starts = array(
        "I", (match.start() for match in HTML_COMMENT_OPEN_RE.finditer(text))
    )
    policy: list[str] = []
    links: list[str] = []
    ambient = EMPTY_CONTAINER
    open_fence: tuple[str, int, ContainerState] | None = None
    block_comment: ContainerState | None = None
    inline_comment: ContainerState | None = None
    inline_pending: list[str] = []
    inline_newlines: list[str] = []
    unclosed_fence = False
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
        closing_owner: ContainerState | None = None

        while run_index < len(run_starts) and run_ends[run_index] <= line_start:
            run_index += 1
        while (
            comment_index < len(comment_starts)
            and comment_starts[comment_index] < line_start
        ):
            comment_index += 1

        if open_fence is not None:
            fence_state = open_fence[2]
            blank_list_continuation = not content.strip() and fence_state.list_only
            if blank_list_continuation:
                policy.append(newline)
                links.append(newline)
                continue
            matched, fence_offset = _match_container_path(
                content, fence_state.frames
            )
            if matched == fence_state.frames:
                fence_content = content[fence_offset:]
                if _is_fence_closer(
                    fence_content, (open_fence[0], open_fence[1])
                ):
                    open_fence = None
                    fence_lines += 1
                while (
                    run_index < len(run_starts)
                    and run_starts[run_index] < line_end
                ):
                    run_index += 1
                policy.append(newline)
                links.append(newline)
                continue
            unclosed_fence = True
            open_fence = None

        if block_comment is not None:
            blank_list_continuation = not content.strip() and block_comment.list_only
            if blank_list_continuation:
                policy.append(newline)
                links.append(newline)
                ambient = block_comment
                continue
            matched, comment_offset = _match_container_path(
                content, block_comment.frames
            )
            if matched == block_comment.frames:
                end = text.find("-->", line_start + comment_offset, line_end)
                if end < 0:
                    policy.append(newline)
                    links.append(newline)
                    ambient = block_comment
                    continue
                block_comment = None
                ambient = _container_state(matched)
                cursor = end + 3
            else:
                block_comment = None
                cursor = line_start
        elif inline_comment is not None:
            continuation, owner_offset = _paragraph_line_continuation(
                content, inline_comment
            )
            end = (
                text.find("-->", line_start + owner_offset, line_end)
                if continuation
                else -1
            )
            if not continuation:
                restored = "".join(inline_pending)
                policy.append(restored)
                links.append(restored)
                inline_pending.clear()
                inline_newlines.clear()
                inline_comment = None
                cursor = line_start
            elif end >= 0:
                policy.extend(inline_newlines)
                links.extend(inline_newlines)
                inline_pending.clear()
                inline_newlines.clear()
                closing_owner = inline_comment
                inline_comment = None
                cursor = end + 3
            else:
                inline_pending.append(raw_line)
                inline_newlines.append(newline)
                continue
        else:
            cursor = line_start

        if closing_owner is not None:
            parsed = ParsedContainerLine(
                closing_owner,
                owner_offset,
                content[owner_offset:],
            )
        else:
            parsed = _parse_container_line(content, ambient)
        ambient = parsed.state
        content_start = line_start + parsed.content_offset
        if code_span_end is not None and code_span_end <= line_start:
            code_span_end = None
        if block_comment is None and inline_comment is None and code_span_end is None:
            opener = _fence_opener(parsed.content)
            if opener is not None and cursor <= content_start:
                open_fence = (opener[0], opener[1], parsed.state)
                fence_lines += 1
                while (
                    run_index < len(run_starts)
                    and run_starts[run_index] < line_end
                ):
                    run_index += 1
                policy.append(newline)
                links.append(newline)
                continue

        defer_inline_line = False
        while cursor < line_end:
            if code_span_end is not None:
                segment_end = min(code_span_end, line_end)
                segment = text[cursor:segment_end]
                policy.append(segment)
                links.append(_blank_for_links(segment))
                cursor = segment_end
                if cursor >= code_span_end:
                    code_span_end = None
                continue

            while run_index < len(run_starts) and run_ends[run_index] <= cursor:
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
                if escaped_runs[run_index]:
                    segment_end = run_ends[run_index]
                    segment = text[cursor:segment_end]
                    policy.append(segment)
                    links.append(segment)
                    cursor = segment_end
                    run_index += 1
                    continue
                close_index = next_same_run[run_index]
                if close_index < 0:
                    segment_end = run_ends[run_index]
                    segment = text[cursor:segment_end]
                    policy.append(segment)
                    links.append(segment)
                    cursor = segment_end
                    run_index += 1
                    continue
                code_span_end = run_ends[close_index]
                run_index = close_index + 1
                segment_end = min(code_span_end, line_end)
                policy.append(text[cursor:segment_end])
                links.append(text[cursor:run_start])
                links.append(_blank_for_links(text[run_start:segment_end]))
                cursor = segment_end
                if cursor >= code_span_end:
                    code_span_end = None
                continue

            if comment_start < line_end:
                prefix = text[cursor:comment_start]
                logical_prefix = text[content_start:comment_start]
                block_candidate = not logical_prefix.strip()
                if block_candidate and len(logical_prefix) > 3:
                    literal_end = comment_start + 4
                    literal = text[cursor:literal_end]
                    policy.append(literal)
                    links.append(literal)
                    cursor = literal_end
                    comment_index += 1
                    continue
                policy.append(prefix)
                links.append(prefix)
                end = text.find("-->", comment_start + 4, line_end)
                comment_index += 1
                if end >= 0:
                    cursor = end + 3
                    continue
                if block_candidate:
                    block_comment = parsed.state
                    cursor = line_end
                    break
                if (
                    PARAGRAPH_ATX_RE.match(parsed.content) is not None
                    or HTML_RAW_BLOCK_RE.match(parsed.content) is not None
                    or HTML_DECLARATION_BLOCK_RE.match(parsed.content) is not None
                ):
                    literal = text[comment_start:line_end]
                    policy.append(literal)
                    links.append(literal)
                    cursor = line_end
                    break
                inline_comment = parsed.state
                inline_pending.append(text[comment_start:line_end] + newline)
                inline_newlines.append(newline)
                defer_inline_line = True
                cursor = line_end
                break

            segment = text[cursor:line_end]
            policy.append(segment)
            links.append(segment)
            cursor = line_end

        if not defer_inline_line:
            policy.append(newline)
            links.append(newline)

    if inline_pending:
        restored = "".join(inline_pending)
        policy.append(restored)
        links.append(restored)

    policy_prose = "".join(policy)
    return MarkdownScan(
        policy_prose=policy_prose,
        classifier_prose=_normalize_policy_markup(policy_prose),
        link_markup="".join(links),
        fence_lines=fence_lines,
        unclosed_fence=unclosed_fence or open_fence is not None,
    )


def _prose_without_fenced_blocks(text: str) -> str:
    """Return policy prose while fenced blocks and HTML comments stay inert."""

    return _scan_markdown_inert_blocks(text).policy_prose


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


def _validate_live_ledger_ownership(
    context: ValidationContext, files: list[Path]
) -> None:
    """Keep mutable delivery status out of every durable non-review document."""

    root = context.root
    errors = context.errors
    review_index = Path("docs/reviews/README.md")
    for path in files:
        relative = path.relative_to(root)
        if relative == PLAN_PATH:
            continue
        if (
            len(relative.parts) >= 2
            and relative.parts[:2] == ("docs", "reviews")
            and relative != review_index
        ):
            continue
        scan = context.scan(path)
        if scan is None:
            continue
        prose = scan.classifier_prose
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
        ambient = EMPTY_CONTAINER
        reserved_fields: dict[str, str] = {}
        for line in prose.splitlines():
            parsed = _parse_container_line(line, ambient)
            ambient = parsed.state
            reserved = NON_PLAN_LIVE_FIELD_RE.match(parsed.content)
            if reserved is not None:
                reserved_fields.setdefault(
                    reserved.group(1).casefold(), reserved.group(1)
                )
        for key in sorted(reserved_fields):
            field = reserved_fields[key]
            errors.append(
                f"{relative}: must not contain canonical live-status field "
                f"{field!r}"
            )


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
        prose = scan.classifier_prose
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
                if INVENTORY_IMPLIED_CATALOG_COUNT_RE.search(clause) is not None:
                    return True
                for tool_count in INVENTORY_TOOL_COUNT_RE.finditer(clause):
                    after = clause[
                        tool_count.end() : min(len(clause), tool_count.end() + 120)
                    ].split(",", 1)[0]
                    if STRONG_OPERATIONAL_AFTER_RE.search(after) is None:
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


@dataclass(frozen=True)
class LinkSyntaxIndex:
    """Compact forward indexes used by rendered-link recognition."""

    escaped: bytearray
    next_angle_close: array
    next_angle_open: array
    next_line_break: array
    next_nonspace: array
    next_single_quote: array
    next_double_quote: array
    next_paren_close: array


def _next_unescaped(markup: str, escaped: bytearray, token: str) -> array:
    result = array("i", [-1]) * (len(markup) + 1)
    nearest = -1
    for index in range(len(markup) - 1, -1, -1):
        if markup[index] == token and not escaped[index]:
            nearest = index
        result[index] = nearest
    return result


def _link_syntax_index(markup: str) -> LinkSyntaxIndex:
    """Index escapes and bounded lookahead without rescanning suffixes."""

    escaped = bytearray(len(markup))
    slash_run = 0
    for index, token in enumerate(markup):
        if token == "\\":
            slash_run += 1
            continue
        if slash_run % 2 and token.isascii() and not token.isalnum():
            escaped[index] = 1
        slash_run = 0

    next_nonspace = array("i", [-1]) * (len(markup) + 1)
    nearest_nonspace = -1
    for index in range(len(markup) - 1, -1, -1):
        if not markup[index].isspace():
            nearest_nonspace = index
        next_nonspace[index] = nearest_nonspace

    return LinkSyntaxIndex(
        escaped=escaped,
        next_angle_close=_next_unescaped(markup, escaped, ">"),
        next_angle_open=_next_unescaped(markup, escaped, "<"),
        next_line_break=_next_unescaped(markup, bytearray(len(markup)), "\n"),
        next_nonspace=next_nonspace,
        next_single_quote=_next_unescaped(markup, escaped, "'"),
        next_double_quote=_next_unescaped(markup, escaped, '"'),
        next_paren_close=_next_unescaped(markup, escaped, ")"),
    )


def _skip_link_whitespace(
    syntax: LinkSyntaxIndex, cursor: int, limit: int
) -> int | None:
    """Skip link whitespace unless it contains a blank physical line."""

    if cursor >= limit:
        return limit
    next_nonspace = syntax.next_nonspace[cursor]
    end = limit if next_nonspace < 0 or next_nonspace >= limit else next_nonspace
    first_break = syntax.next_line_break[cursor]
    if first_break >= 0 and first_break < end:
        second_break = syntax.next_line_break[first_break + 1]
        if second_break >= 0 and second_break < end:
            return None
    return end


def _title_end(
    markup: str,
    syntax: LinkSyntaxIndex,
    cursor: int,
    limit: int,
) -> int | None:
    delimiter = markup[cursor]
    if delimiter == '"':
        end = syntax.next_double_quote[cursor + 1]
    elif delimiter == "'":
        end = syntax.next_single_quote[cursor + 1]
    elif delimiter == "(":
        end = syntax.next_paren_close[cursor + 1]
    else:
        return None
    if end < 0 or end >= limit:
        return None
    first_break = syntax.next_line_break[cursor + 1]
    if first_break >= 0 and first_break < end:
        second_break = syntax.next_line_break[first_break + 1]
        if second_break >= 0 and second_break < end:
            return None
    return end + 1


def _inline_link_target(
    markup: str,
    syntax: LinkSyntaxIndex,
    open_paren: int,
) -> tuple[str, int] | None:
    """Parse one complete inline destination and optional title."""

    cursor = _skip_link_whitespace(syntax, open_paren + 1, len(markup))
    if cursor is None or cursor >= len(markup):
        return None

    if markup[cursor] == "<" and not syntax.escaped[cursor]:
        angle_end = syntax.next_angle_close[cursor + 1]
        nested_angle = syntax.next_angle_open[cursor + 1]
        line_break = syntax.next_line_break[cursor + 1]
        if (
            angle_end < 0
            or (nested_angle >= 0 and nested_angle < angle_end)
            or (line_break >= 0 and line_break < angle_end)
        ):
            return None
        target = markup[cursor : angle_end + 1]
        cursor = angle_end + 1
    else:
        target_start = cursor
        depth = 0
        while cursor < len(markup):
            token = markup[cursor]
            if syntax.escaped[cursor]:
                cursor += 1
                continue
            if token == "(" and depth < 32:
                depth += 1
            elif token == "(":
                return None
            elif token == ")":
                if depth == 0:
                    return markup[target_start:cursor], cursor + 1
                depth -= 1
            elif token.isspace():
                break
            elif token == "<" or ord(token) < 0x20:
                return None
            cursor += 1
        if cursor == target_start or cursor >= len(markup):
            return None
        target = markup[target_start:cursor]

    cursor = _skip_link_whitespace(syntax, cursor, len(markup))
    if cursor is None or cursor >= len(markup):
        return None
    if markup[cursor] == ")" and not syntax.escaped[cursor]:
        return target, cursor + 1
    title_end = _title_end(markup, syntax, cursor, len(markup))
    if title_end is None:
        return None
    cursor = _skip_link_whitespace(syntax, title_end, len(markup))
    if (
        cursor is None
        or cursor >= len(markup)
        or markup[cursor] != ")"
        or syntax.escaped[cursor]
    ):
        return None
    return target, cursor + 1


def _normalize_reference_label(label: str) -> str:
    rendered: list[str] = []
    cursor = 0
    while cursor < len(label):
        if (
            label[cursor] == "\\"
            and cursor + 1 < len(label)
            and label[cursor + 1].isascii()
            and not label[cursor + 1].isalnum()
        ):
            cursor += 1
        rendered.append(label[cursor])
        cursor += 1
    return " ".join("".join(rendered).split()).casefold()


def _reference_definition(
    content: str,
) -> tuple[str, str, bool] | None:
    """Parse one single-line CommonMark reference definition."""

    cursor = len(content) - len(content.lstrip(" "))
    if cursor > 3 or cursor >= len(content) or content[cursor] != "[":
        return None
    label_start = cursor + 1
    cursor = label_start
    escaped = False
    while cursor < len(content):
        token = content[cursor]
        if escaped:
            escaped = False
        elif token == "\\":
            escaped = True
        elif token == "[":
            return None
        elif token == "]":
            break
        cursor += 1
    if cursor >= len(content) or cursor - label_start > 999:
        return None
    label = _normalize_reference_label(content[label_start:cursor])
    cursor += 1
    if not label or cursor >= len(content) or content[cursor] != ":":
        return None
    cursor += 1
    while cursor < len(content) and content[cursor] in " \t":
        cursor += 1
    if cursor >= len(content):
        return None

    if content[cursor] == "<":
        target_start = cursor
        cursor += 1
        escaped = False
        while cursor < len(content):
            token = content[cursor]
            if escaped:
                escaped = False
            elif token == "\\":
                escaped = True
            elif token == ">":
                break
            elif token in "\r\n<":
                return None
            cursor += 1
        if cursor >= len(content):
            return None
        cursor += 1
        target = content[target_start:cursor]
    else:
        target_start = cursor
        depth = 0
        escaped = False
        while cursor < len(content):
            token = content[cursor]
            if escaped:
                escaped = False
            elif token == "\\":
                escaped = True
            elif token == "(" and depth < 32:
                depth += 1
            elif token == "(":
                return None
            elif token == ")" and depth:
                depth -= 1
            elif token.isspace():
                break
            elif token == "<" or ord(token) < 0x20:
                return None
            cursor += 1
        if cursor == target_start or depth:
            return None
        target = content[target_start:cursor]

    while cursor < len(content) and content[cursor] in " \t":
        cursor += 1
    has_title = cursor < len(content)
    if has_title:
        delimiter = content[cursor]
        closer = {"\"": "\"", "'": "'", "(": ")"}.get(delimiter)
        if closer is None:
            return None
        cursor += 1
        escaped = False
        while cursor < len(content):
            token = content[cursor]
            if escaped:
                escaped = False
            elif token == "\\":
                escaped = True
            elif token == closer:
                cursor += 1
                break
            cursor += 1
        else:
            return None
        if content[cursor:].strip():
            return None
    return label, target, has_title


def _reference_title_line(content: str) -> bool:
    """Recognize one complete continuation title for a link definition."""

    cursor = len(content) - len(content.lstrip(" "))
    if cursor > 3 or cursor >= len(content):
        return False
    delimiter = content[cursor]
    closer = {"\"": "\"", "'": "'", "(": ")"}.get(delimiter)
    if closer is None:
        return False
    cursor += 1
    escaped = False
    while cursor < len(content):
        token = content[cursor]
        if escaped:
            escaped = False
        elif token == "\\":
            escaped = True
        elif token == closer:
            return not content[cursor + 1 :].strip()
        cursor += 1
    return False


def _reference_definitions(markup: str) -> tuple[dict[str, str], str]:
    definitions: dict[str, str] = {}
    chunks: list[str] = []
    ambient = EMPTY_CONTAINER
    pending_title_owner: ContainerState | None = None
    for raw_line in markup.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        newline = raw_line[len(content) :]
        parsed = _parse_container_line(content, ambient)
        ambient = parsed.state
        if pending_title_owner is not None:
            same_owner = parsed.path == pending_title_owner.frames
            pending_title_owner = None
            if same_owner and _reference_title_line(parsed.content):
                chunks.append(" " * len(content) + newline)
                continue
        definition = _reference_definition(parsed.content)
        if definition is None:
            chunks.append(raw_line)
            continue
        label, target, has_title = definition
        definitions.setdefault(label, target)
        chunks.append(" " * len(content) + newline)
        if not has_title:
            pending_title_owner = parsed.state
    return definitions, "".join(chunks)


def _without_indented_code(markup: str) -> str:
    """Blank indented code while preserving paragraph continuation lines."""

    chunks: list[str] = []
    ambient = EMPTY_CONTAINER
    paragraph_open = False
    indented_owner: ContainerState | None = None
    for raw_line in markup.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        newline = raw_line[len(content) :]
        if indented_owner is not None:
            if not content.strip():
                chunks.append(" " * len(content) + newline)
                continue
            matched, owner_offset = _match_container_path(
                content, indented_owner.frames
            )
            if (
                matched == indented_owner.frames
                and content[owner_offset:].startswith("    ")
            ):
                chunks.append(" " * len(content) + newline)
                continue
            indented_owner = None

        parsed = _parse_container_line(content, ambient)
        ambient = parsed.state
        if not content.strip():
            paragraph_open = False
            chunks.append(raw_line)
            continue
        if parsed.content.startswith("    ") and not paragraph_open:
            indented_owner = parsed.state
            chunks.append(" " * len(content) + newline)
            continue

        chunks.append(raw_line)
        paragraph_open = not (
            _fence_opener(parsed.content) is not None
            or PARAGRAPH_ATX_RE.match(parsed.content) is not None
            or SETEXT_HEADING_RE.match(parsed.content) is not None
            or THEMATIC_BREAK_RE.match(parsed.content) is not None
            or HTML_RAW_BLOCK_RE.match(parsed.content) is not None
            or HTML_DECLARATION_BLOCK_RE.match(parsed.content) is not None
        )
    return "".join(chunks)


def _without_html_blocks_for_links(markup: str) -> str:
    """Blank CommonMark HTML block contents where Markdown links are inert."""

    chunks: list[str] = []
    ambient = EMPTY_CONTAINER
    html_owner: ContainerState | None = None
    html_end: str | None = None
    for raw_line in markup.splitlines(keepends=True):
        content = raw_line.rstrip("\r\n")
        newline = raw_line[len(content) :]
        if html_owner is not None:
            matched, owner_offset = _match_container_path(
                content, html_owner.frames
            )
            if matched != html_owner.frames:
                html_owner = None
                html_end = None
            elif html_end is None and not content.strip():
                html_owner = None
                chunks.append(raw_line)
                continue
            else:
                chunks.append(" " * len(content) + newline)
                candidate = content[owner_offset:].casefold()
                if html_end is not None and html_end in candidate:
                    html_owner = None
                    html_end = None
                continue

        parsed = _parse_container_line(content, ambient)
        ambient = parsed.state
        candidate = parsed.content
        raw = HTML_RAW_BLOCK_RE.match(candidate)
        declaration = HTML_DECLARATION_BLOCK_RE.match(candidate)
        if raw is None and declaration is None:
            chunks.append(raw_line)
            continue

        stripped = candidate.lstrip(" ")
        end_marker: str | None = None
        lowered = stripped.casefold()
        for tag in ("script", "pre", "style", "textarea"):
            if re.match(rf"<{tag}(?:[ \t]|>|$)", lowered):
                end_marker = f"</{tag}>"
                break
        if lowered.startswith("<?"):
            end_marker = "?>"
        elif lowered.startswith("<![cdata["):
            end_marker = "]]>"
        elif re.match(r"<![a-z]", lowered):
            end_marker = ">"

        chunks.append(" " * len(content) + newline)
        if end_marker is None:
            html_owner = parsed.state
        elif end_marker not in lowered:
            html_owner = parsed.state
            html_end = end_marker
    return "".join(chunks)


def _reference_suffix(
    markup: str,
    syntax: LinkSyntaxIndex,
    cursor: int,
) -> tuple[str, int] | None:
    if cursor >= len(markup) or markup[cursor] != "[" or syntax.escaped[cursor]:
        return None
    label_start = cursor + 1
    cursor = label_start
    while cursor < len(markup) and cursor - label_start <= 999:
        if markup[cursor] == "]" and not syntax.escaped[cursor]:
            return markup[label_start:cursor], cursor + 1
        if markup[cursor] == "[" and not syntax.escaped[cursor]:
            return None
        cursor += 1
    return None


def _markdown_link_targets(markup: str) -> Iterator[str]:
    """Yield complete rendered inline and reference-link targets."""

    markup = _without_html_blocks_for_links(_without_indented_code(markup))
    definitions, markup = _reference_definitions(markup)
    syntax = _link_syntax_index(markup)
    opener_positions = array("I")
    opener_generations = array("I")
    opener_images = bytearray()
    opener_nested = bytearray()
    generation = 0
    cursor = 0
    while cursor < len(markup):
        token = markup[cursor]
        if token == "[" and not syntax.escaped[cursor]:
            if opener_nested:
                opener_nested[-1] = 1
            opener_positions.append(cursor)
            opener_generations.append(generation)
            opener_images.append(
                cursor > 0
                and markup[cursor - 1] == "!"
                and not syntax.escaped[cursor - 1]
            )
            opener_nested.append(0)
            cursor += 1
            continue
        if (
            token != "]"
            or syntax.escaped[cursor]
            or not opener_positions
        ):
            cursor += 1
            continue

        opener = opener_positions.pop()
        opener_generation = opener_generations.pop()
        image = bool(opener_images.pop())
        nested_label = bool(opener_nested.pop())
        active = image or opener_generation == generation
        if cursor - opener - 1 > 999 or not active:
            cursor += 1
            continue

        inline = None
        if cursor + 1 < len(markup) and markup[cursor + 1] == "(":
            inline = _inline_link_target(markup, syntax, cursor + 1)
        if inline is not None:
            target, cursor = inline
            if target:
                yield target
            if not image:
                generation += 1
            continue

        suffix = _reference_suffix(markup, syntax, cursor + 1)
        if suffix is not None:
            reference_label, suffix_end = suffix
            if reference_label:
                key = _normalize_reference_label(reference_label)
            elif not nested_label:
                key = _normalize_reference_label(markup[opener + 1 : cursor])
            else:
                key = ""
            target = definitions.get(key)
            if target is not None:
                yield target
                cursor = suffix_end
                if not image:
                    generation += 1
                continue
            cursor = suffix_end
            continue

        target = (
            definitions.get(
                _normalize_reference_label(markup[opener + 1 : cursor])
            )
            if definitions and not nested_label
            else None
        )
        if target is not None:
            yield target
            if not image:
                generation += 1
        cursor += 1


def _policy_code_mask(prose: str) -> bytearray:
    starts, ends, next_same, escaped = _backtick_runs(prose)
    mask = bytearray(len(prose))
    index = 0
    while index < len(starts):
        close_index = next_same[index]
        if escaped[index] or close_index < 0:
            index += 1
            continue
        start = starts[index]
        end = ends[close_index]
        mask[start:end] = b"\x01" * (end - start)
        index = close_index + 1
    return mask


def _policy_html_tag_mask(
    prose: str, code: bytearray, escaped: bytearray
) -> bytearray:
    """Mark complete inline HTML tags while leaving their visible text."""

    mask = bytearray(len(prose))
    cursor = 0
    while cursor < len(prose):
        if prose[cursor] != "<" or code[cursor] or escaped[cursor]:
            cursor += 1
            continue
        end = cursor + 1
        if end < len(prose) and prose[end] == "/":
            end += 1
        name_start = end
        while end < len(prose) and (
            prose[end].isalnum() or prose[end] in "-_"
        ):
            end += 1
        if (
            end == name_start
            or not prose[name_start].isascii()
            or not prose[name_start].isalpha()
            or (
                end < len(prose)
                and prose[end] not in " \t\r\n/>"
            )
        ):
            cursor += 1
            continue

        quote: str | None = None
        while end < len(prose):
            token = prose[end]
            if quote is not None:
                if token == quote:
                    quote = None
            elif token in "\"'":
                quote = token
            elif token == ">":
                end += 1
                mask[cursor:end] = b"\x01" * (end - cursor)
                break
            elif token == "<" or (
                token in "\r\n"
                and end + 1 < len(prose)
                and prose[end + 1] in "\r\n"
            ):
                break
            end += 1
        cursor = max(cursor + 1, end)
    return mask


def _delimiter_flanking(prose: str, start: int, end: int) -> tuple[bool, bool]:
    before = prose[start - 1] if start else "\n"
    after = prose[end] if end < len(prose) else "\n"
    before_space = before.isspace()
    after_space = after.isspace()
    before_punctuation = not before_space and not before.isalnum()
    after_punctuation = not after_space and not after.isalnum()
    left = not after_space and (
        not after_punctuation or before_space or before_punctuation
    )
    right = not before_space and (
        not before_punctuation or after_space or after_punctuation
    )
    return left, right


def _normalize_policy_markup(prose: str) -> str:
    """Project rendered classifier text without inline presentation wrappers."""

    prose = _without_indented_code(prose)
    definitions, prose = _reference_definitions(prose)
    syntax = _link_syntax_index(prose)
    code = _policy_code_mask(prose)
    html_tags = _policy_html_tag_mask(prose, code, syntax.escaped)
    removed = bytearray(len(prose))
    opener_positions = array("I")
    opener_generations = array("I")
    opener_images = bytearray()
    opener_nested = bytearray()
    generation = 0
    cursor = 0
    while cursor < len(prose):
        if code[cursor]:
            cursor += 1
            continue
        token = prose[cursor]
        if token == "[" and not syntax.escaped[cursor]:
            if opener_nested:
                opener_nested[-1] = 1
            opener_positions.append(cursor)
            opener_generations.append(generation)
            opener_images.append(
                cursor > 0
                and prose[cursor - 1] == "!"
                and not syntax.escaped[cursor - 1]
            )
            opener_nested.append(0)
            cursor += 1
            continue
        if (
            token != "]"
            or syntax.escaped[cursor]
            or not opener_positions
        ):
            cursor += 1
            continue

        opener = opener_positions.pop()
        opener_generation = opener_generations.pop()
        image = bool(opener_images.pop())
        nested_label = bool(opener_nested.pop())
        active = image or opener_generation == generation
        if cursor - opener - 1 > 999 or not active:
            cursor += 1
            continue

        syntax_end: int | None = None
        if cursor + 1 < len(prose) and prose[cursor + 1] == "(":
            inline = _inline_link_target(prose, syntax, cursor + 1)
            if inline is not None:
                _, syntax_end = inline
        if syntax_end is None:
            suffix = _reference_suffix(prose, syntax, cursor + 1)
            if suffix is not None:
                reference_label, suffix_end = suffix
                if reference_label:
                    key = _normalize_reference_label(reference_label)
                elif not nested_label:
                    key = _normalize_reference_label(prose[opener + 1 : cursor])
                else:
                    key = ""
                if key in definitions:
                    syntax_end = suffix_end
                else:
                    cursor = suffix_end
                    continue
        if syntax_end is None and definitions and not nested_label:
            key = _normalize_reference_label(prose[opener + 1 : cursor])
            if key in definitions:
                syntax_end = cursor + 1
        if syntax_end is None:
            cursor += 1
            continue

        removed[opener] = 1
        removed[cursor] = 1
        if image and opener:
            removed[opener - 1] = 1
        if syntax_end > cursor + 1:
            removed[cursor + 1 : syntax_end] = b"\x01" * (
                syntax_end - cursor - 1
            )
        if not image:
            generation += 1
        cursor = syntax_end

    delimiter_stacks: dict[str, array] = {}
    cursor = 0
    line_has_content = False
    while cursor < len(prose):
        if prose[cursor] in "\r\n":
            if not line_has_content:
                delimiter_stacks.clear()
            line_has_content = False
            cursor += 1
            continue
        if not prose[cursor].isspace():
            line_has_content = True
        if (
            code[cursor]
            or html_tags[cursor]
            or removed[cursor]
            or syntax.escaped[cursor]
        ):
            cursor += 1
            continue
        marker = prose[cursor]
        if marker not in "*_~":
            cursor += 1
            continue
        run_end = cursor + 1
        while (
            run_end < len(prose)
            and prose[run_end] == marker
            and not code[run_end]
            and not syntax.escaped[run_end]
        ):
            run_end += 1
        if marker == "~" and run_end - cursor < 2:
            cursor = run_end
            continue
        token = prose[cursor:run_end]
        can_open, can_close = _delimiter_flanking(prose, cursor, run_end)
        if marker == "_":
            before = prose[cursor - 1] if cursor else "\n"
            after = prose[run_end] if run_end < len(prose) else "\n"
            can_open = can_open and (not can_close or not before.isalnum())
            can_close = can_close and (not can_open or not after.isalnum())
        stack = delimiter_stacks.setdefault(token, array("I"))
        if can_close and stack:
            opener = stack.pop()
            removed[opener : opener + len(token)] = b"\x01" * len(token)
            removed[cursor:run_end] = b"\x01" * (run_end - cursor)
        elif can_open:
            stack.append(cursor)
        cursor = run_end

    rendered: list[str] = []
    cursor = 0
    while cursor < len(prose):
        token = prose[cursor]
        if code[cursor]:
            rendered.append(token)
            cursor += 1
            continue
        if html_tags[cursor] or removed[cursor]:
            if token in "\r\n":
                rendered.append(token)
            cursor += 1
            continue
        if token == "&" and not syntax.escaped[cursor]:
            entity = HTML_ENTITY_RE.match(prose, cursor)
            if entity is not None:
                rendered.append(unescape_html(entity.group(0)))
                cursor = entity.end()
                continue
        rendered.append(token)
        cursor += 1
    return "".join(rendered)


def _relative_link_target(raw_target: str) -> str | None:
    target = raw_target[1:-1] if raw_target.startswith("<") else raw_target
    rendered: list[str] = []
    cursor = 0
    escapable = " !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"
    while cursor < len(target):
        if (
            target[cursor] == "\\"
            and cursor + 1 < len(target)
            and target[cursor + 1] in escapable
        ):
            cursor += 1
        rendered.append(target[cursor])
        cursor += 1
    target = unquote("".join(rendered))
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

        fence_lines += scan.fence_lines
        if scan.unclosed_fence:
            errors.append(f"{path.relative_to(root)}: unclosed Markdown fence")

        decoded_targets: dict[str, str | None] = {}
        checked_targets: set[str] = set()
        for raw_target in _markdown_link_targets(scan.link_markup):
            if raw_target not in decoded_targets:
                decoded_targets[raw_target] = _relative_link_target(raw_target)
            target = decoded_targets[raw_target]
            if target is None:
                continue
            relative_links += 1
            if target in checked_targets:
                continue
            checked_targets.add(target)
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
    _validate_live_ledger_ownership(context, files)
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
