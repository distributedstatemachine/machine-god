#!/usr/bin/env python3
"""Validate the repository's bounded, single-source documentation policy."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlsplit


PLAN_PATH = Path("docs/implementation-plan.md")
START_MARKER = "<!-- canonical-live-status:start -->"
END_MARKER = "<!-- canonical-live-status:end -->"
MAX_PLAN_LINES = 600

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
FENCE_RE = re.compile(r"^[ ]{0,3}(`{3,}|~{3,})")
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
COUNT_WORD_RE = (
    r"(?:[0-9]+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|"
    r"twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|"
    r"twenty)"
)
INVENTORY_COUNT_NOUN_PATTERN = (
    rf"\b{COUNT_WORD_RE}-(?:tool|clone|entry)\b|"
    rf"\b{COUNT_WORD_RE}\s+(?:ToolSpec|tool-spec)\s+"
    rf"(?:values?|objects?)\b|"
    rf"\b{COUNT_WORD_RE}\s+"
    rf"(?:[A-Za-z][A-Za-z0-9_-]*\s+){{0,5}}"
    rf"(?:tools?(?!\s+(?:calls?|catalog|events?|outputs?|results?|schemas?|specs?)\b)|"
    rf"entries|clones?)\b"
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
    r"install(?:s|ed)?|owns?|suppl(?:y|ies|ied)|lists?|totals?)\b|"
    r"\b(?:make|makes|made)\s+up\b|\bcomposed\s+of\b",
    re.IGNORECASE,
)
OPERATIONAL_VERB_RE = re.compile(
    r"\b(?:accepts?|allows?|permits?|batches?|executes?|runs?|schedules?|queues?)\b",
    re.IGNORECASE,
)
OPERATIONAL_AFTER_RE = re.compile(
    r"\b(?:active|executing|concurrent(?:ly)?|in[- ]flight|queued|running|"
    r"at\s+once|per\s+(?:call|invocation|turn|round|request|batch)|"
    r"while\s+(?:a\s+)?calls?\s+is\s+active|Waker|scheduler|queue|"
    r"calls?|invocations?)\b",
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


@dataclass(frozen=True)
class DocumentationStats:
    markdown_files: int = 0
    fence_lines: int = 0
    relative_links: int = 0
    unique_relative_targets: int = 0


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
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        errors.append(f"{path.relative_to(root)}: cannot read UTF-8: {error}")
        return None


def _validate_live_status(root: Path, files: list[Path], errors: list[str]) -> None:
    plan = root / PLAN_PATH
    text = _read(plan, root, errors)
    if text is None:
        return

    line_count = len(text.splitlines())
    if line_count > MAX_PLAN_LINES:
        errors.append(
            f"{PLAN_PATH}: {line_count} lines exceeds the {MAX_PLAN_LINES}-line ceiling"
        )

    marker_locations: dict[str, list[Path]] = {START_MARKER: [], END_MARKER: []}
    for path in files:
        candidate = _read(path, root, errors)
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


def _scan_markdown_inert_blocks(text: str) -> tuple[str, int, bool]:
    """Strip fenced blocks and HTML comments with shared Markdown state."""

    prose: list[str] = []
    open_fence: tuple[str, int] | None = None
    in_comment = False
    fence_lines = 0
    for raw_line in text.splitlines():
        if open_fence is not None:
            match = FENCE_RE.match(raw_line)
            if match is not None:
                fence_lines += 1
                token = match.group(1)
                if token[0] == open_fence[0] and len(token) >= open_fence[1]:
                    open_fence = None
            prose.append("")
            continue

        if not in_comment:
            match = FENCE_RE.match(raw_line)
            if match is not None:
                token = match.group(1)
                open_fence = (token[0], len(token))
                fence_lines += 1
                prose.append("")
                continue

        visible: list[str] = []
        cursor = 0
        while cursor < len(raw_line):
            if in_comment:
                end = raw_line.find("-->", cursor)
                if end < 0:
                    cursor = len(raw_line)
                    break
                in_comment = False
                cursor = end + 3
                continue

            start = _find_html_comment_start(raw_line, cursor)
            if start < 0:
                visible.append(raw_line[cursor:])
                break
            visible.append(raw_line[cursor:start])
            in_comment = True
            cursor = start + 4

        if in_comment and not visible:
            prose.append("")
        else:
            prose.append("".join(visible))
    return "\n".join(prose), fence_lines, open_fence is not None


def _find_html_comment_start(line: str, cursor: int) -> int:
    """Find a comment opener outside a same-line Markdown code span."""

    position = cursor
    while True:
        comment = line.find("<!--", position)
        if comment < 0:
            return -1
        tick = line.find("`", position)
        if tick < 0 or comment < tick:
            return comment

        run_end = tick + 1
        while run_end < len(line) and line[run_end] == "`":
            run_end += 1
        delimiter = line[tick:run_end]
        close = line.find(delimiter, run_end)
        if close < 0:
            return -1
        position = close + len(delimiter)


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


def _validate_governed_overviews(root: Path, errors: list[str]) -> None:
    for relative in GOVERNED_OVERVIEWS:
        path = root / relative
        text = _read(path, root, errors)
        if text is None:
            continue
        prose = _prose_without_fenced_blocks(text)
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
    root: Path, files: list[Path], errors: list[str]
) -> None:
    """Keep mutable reference-host inventory facts in their canonical contract."""

    for path in files:
        relative = path.relative_to(root)
        if relative in REFERENCE_HOST_INVENTORY_EXEMPTIONS:
            continue
        if len(relative.parts) >= 2 and relative.parts[:2] == ("docs", "reviews"):
            continue

        text = _read(path, root, errors)
        if text is None:
            continue
        prose = _prose_without_fenced_blocks(text)
        if relative == REFERENCE_HOST_CONTRACT_PATH:
            prose, section_count = _without_unique_markdown_section(
                prose, "Tool catalog"
            )
            if section_count != 1:
                errors.append(
                    f"{relative}: must contain exactly one Tool catalog section; "
                    f"found {section_count}"
                )
        has_inventory = any(
            _contains_reference_host_inventory(paragraph)
            for paragraph in re.split(r"\n\s*\n", prose)
        )

        if has_inventory:
            errors.append(
                f"{relative}: reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog"
            )


def _contains_reference_host_inventory(paragraph: str) -> bool:
    """Recognize inventory statements without confusing operational limits."""

    normalized = re.sub(r"\s+", " ", paragraph).strip()
    if not normalized:
        return False

    for sentence in re.split(r"(?<=[.!?])\s+", normalized):
        sentence_has_host = REFERENCE_HOST_CONTEXT_RE.search(sentence) is not None
        sentence_has_catalog = (
            REFERENCE_HOST_CATALOG_CONTEXT_RE.search(sentence) is not None
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
                sentence_has_host or REFERENCE_HOST_CONTEXT_RE.search(clause) is not None
            )
            has_catalog_context = (
                sentence_has_catalog
                or REFERENCE_HOST_CATALOG_CONTEXT_RE.search(clause) is not None
            )
            if (
                (has_host_context or has_catalog_context)
                and nonoperational_counts
            ):
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

    return False


def _counted_noun_is_operational(clause: str, match: re.Match[str]) -> bool:
    """Classify one counted noun from its bounded local clause context."""

    before = clause[: match.start()].rsplit(",", 1)[-1][-120:]
    counted = match.group(0)
    after = clause[match.end() :].split(",", 1)[0][:120]
    if OPERATIONAL_COUNT_RE.search(counted) is not None:
        return True
    if (
        OPERATIONAL_QUANTIFIER_RE.search(before) is not None
        or OPERATIONAL_SCOPE_RE.search(before) is not None
    ):
        return True
    if INVENTORY_VERB_RE.search(before) is not None:
        return False
    return (
        OPERATIONAL_VERB_RE.search(before) is not None
        or OPERATIONAL_AFTER_RE.search(after) is not None
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
    root: Path, files: list[Path], errors: list[str]
) -> DocumentationStats:
    fence_lines = 0
    relative_links = 0
    unique_targets: set[Path] = set()

    for path in files:
        text = _read(path, root, errors)
        if text is None:
            continue

        _, file_fence_lines, unclosed_fence = _scan_markdown_inert_blocks(text)
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
    files = markdown_files(root)
    _validate_live_status(root, files, errors)
    _validate_governed_overviews(root, errors)
    _validate_reference_host_inventory(root, files, errors)
    stats = _validate_markdown(root, files, errors)
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
