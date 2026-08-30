#!/usr/bin/env python3
"""Conservatively classify a GitHub Actions change set.

Only a small, explicit set of documentation paths may skip the full product
gate.  Missing or surprising Git data always selects the full gate.
"""

from __future__ import annotations

import argparse
import os
from pathlib import PurePosixPath
import subprocess
import sys
from dataclasses import dataclass
from typing import Sequence


DOC_EXCEPTIONS = {
    "docs/compatibility.md",
    "docs/core-api.md",
    "docs/testkit.md",
    "docs/vision.md",
}
KNOWN_STATUSES = {"A", "B", "D", "M", "T", "U", "X"}
REGULAR_BLOB_MODE = "100644"
MISSING_MODE = "000000"


class ClassificationError(RuntimeError):
    """A Git or input uncertainty that requires the full gate."""


@dataclass(frozen=True)
class Classification:
    full: bool
    docs_only: bool
    reason: str


def _run_git(arguments: Sequence[str]) -> bytes:
    try:
        completed = subprocess.run(
            ["git", *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        raise ClassificationError(f"could not execute git: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        if detail:
            detail = " ".join(detail.split())[:240]
            raise ClassificationError(f"git {' '.join(arguments[:2])} failed: {detail}")
        raise ClassificationError(f"git {' '.join(arguments[:2])} failed")
    return completed.stdout


def _validate_revision(revision: str | None, label: str) -> str:
    if not revision or revision.startswith("-") or "\x00" in revision:
        raise ClassificationError(f"{label} revision is missing or invalid")
    try:
        revision.encode("utf-8")
    except UnicodeEncodeError as error:
        raise ClassificationError(f"{label} revision is not UTF-8") from error
    return revision


def _resolve_commit(revision: str | None, label: str) -> str:
    revision = _validate_revision(revision, label)
    raw = _run_git(
        ["rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"]
    )
    try:
        resolved = raw.decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise ClassificationError(f"{label} revision did not resolve to a commit") from error
    if not resolved or any(character not in "0123456789abcdefABCDEF" for character in resolved):
        raise ClassificationError(f"{label} revision did not resolve to a commit")
    return resolved


def _is_ancestor(ancestor: str, descendant: str) -> bool:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    if completed.returncode == 0:
        return True
    if completed.returncode == 1:
        return False
    detail = completed.stderr.decode("utf-8", errors="replace").strip()
    detail = " ".join(detail.split())[:240]
    suffix = f": {detail}" if detail else ""
    raise ClassificationError(f"git ancestry check failed{suffix}")


def _merge_base(left: str, right: str) -> str:
    raw = _run_git(["merge-base", "--all", left, right])
    try:
        bases = raw.decode("ascii").splitlines()
    except UnicodeDecodeError as error:
        raise ClassificationError("merge base was not a commit ID") from error
    if len(bases) != 1:
        raise ClassificationError("revisions have no unique merge base")
    base = bases[0]
    if not base or any(character not in "0123456789abcdefABCDEF" for character in base):
        raise ClassificationError("merge base was not a commit ID")
    return base


def _push_range(before: str | None, head: str | None, default_ref: str | None) -> tuple[str, str, str]:
    resolved_head = _resolve_commit(head, "head")
    before = _validate_revision(before, "before")
    if set(before) == {"0"}:
        if len(before) != len(resolved_head):
            raise ClassificationError("new-branch sentinel has an invalid length")
        resolved_default = _resolve_commit(default_ref, "default ref")
        base = _merge_base(resolved_default, resolved_head)
        return base, resolved_head, "new branch from default-ref merge base"

    resolved_before = _resolve_commit(before, "before")
    if not _is_ancestor(resolved_before, resolved_head):
        raise ClassificationError("push before revision is not an ancestor of head")
    return resolved_before, resolved_head, "normal push range"


def _pull_request_range(base: str | None, head: str | None) -> tuple[str, str, str]:
    resolved_base = _resolve_commit(base, "base")
    resolved_head = _resolve_commit(head, "head")
    merge_base = _merge_base(resolved_base, resolved_head)
    return merge_base, resolved_head, "pull-request merge-base range"


def _changed_paths(base: str, head: str) -> list[tuple[str, str, str, str]]:
    raw = _run_git(
        [
            "diff",
            "--raw",
            "-z",
            "--no-renames",
            "--ignore-submodules=none",
            f"{base}..{head}",
            "--",
        ]
    )
    if not raw:
        return []
    fields = raw.split(b"\x00")
    if fields[-1] != b"":
        raise ClassificationError("git diff produced an unterminated record")
    fields.pop()
    if len(fields) % 2:
        raise ClassificationError("git diff produced a malformed raw record")

    changes: list[tuple[str, str, str, str]] = []
    for index in range(0, len(fields), 2):
        try:
            metadata = fields[index].decode("ascii")
            path = fields[index + 1].decode("utf-8")
        except UnicodeDecodeError as error:
            raise ClassificationError("git diff contained a non-UTF-8 record") from error
        metadata_fields = metadata.split()
        if len(metadata_fields) != 5 or not metadata_fields[0].startswith(":"):
            raise ClassificationError("git diff produced malformed raw metadata")
        old_mode = metadata_fields[0][1:]
        new_mode = metadata_fields[1]
        old_object = metadata_fields[2]
        new_object = metadata_fields[3]
        status = metadata_fields[4]
        if status not in KNOWN_STATUSES:
            raise ClassificationError(f"git diff contained unexpected status {status!r}")
        for mode in (old_mode, new_mode):
            if len(mode) != 6 or any(character not in "01234567" for character in mode):
                raise ClassificationError("git diff contained an invalid object mode")
        for object_id in (old_object, new_object):
            if not object_id or any(
                character not in "0123456789abcdefABCDEF" for character in object_id
            ):
                raise ClassificationError("git diff contained an invalid object ID")
        if not path or "\n" in path or "\r" in path or "\x00" in path:
            raise ClassificationError("git diff contained an invalid path")
        changes.append((status, old_mode, new_mode, path))
    return changes


def _is_cheap_documentation(path: str) -> bool:
    if path == "README.md":
        return True
    if path in DOC_EXCEPTIONS or not path.startswith("docs/") or not path.endswith(".md"):
        return False
    if "\\" in path:
        return False
    parts = PurePosixPath(path).parts
    return len(parts) >= 2 and parts[0] == "docs" and all(part not in {"", ".", ".."} for part in parts)


def _is_cheap_documentation_change(
    status: str, old_mode: str, new_mode: str, path: str
) -> bool:
    expected_modes = {
        "A": (MISSING_MODE, REGULAR_BLOB_MODE),
        "D": (REGULAR_BLOB_MODE, MISSING_MODE),
        "M": (REGULAR_BLOB_MODE, REGULAR_BLOB_MODE),
    }
    return expected_modes.get(status) == (old_mode, new_mode) and _is_cheap_documentation(path)


def _path_prefix_collision(changes: list[tuple[str, str, str, str]]) -> str | None:
    paths = {path for _status, _old_mode, _new_mode, path in changes}
    for path in paths:
        parts = path.split("/")
        for length in range(1, len(parts)):
            ancestor = "/".join(parts[:length])
            if ancestor in paths:
                return ancestor
    return None


def classify(
    event: str,
    before: str | None,
    head: str | None,
    base: str | None,
    default_ref: str | None,
) -> Classification:
    if event == "workflow_dispatch":
        return Classification(True, False, "workflow_dispatch always runs the full gate")
    try:
        if event == "push":
            start, end, range_reason = _push_range(before, head, default_ref)
        elif event in {"pull_request", "pull_request_target"}:
            start, end, range_reason = _pull_request_range(base, head)
        else:
            return Classification(True, False, f"unknown event {event!r}")
        changes = _changed_paths(start, end)
    except (ClassificationError, OSError) as error:
        return Classification(True, False, f"uncertain change set: {error}")

    if not changes:
        return Classification(True, False, f"empty {range_reason}")
    prefix_collision = _path_prefix_collision(changes)
    if prefix_collision is not None:
        return Classification(
            True,
            False,
            f"{range_reason} includes file/directory transition at {prefix_collision!r}",
        )
    non_docs = [
        path
        for status, old_mode, new_mode, path in changes
        if not _is_cheap_documentation_change(status, old_mode, new_mode, path)
    ]
    if non_docs:
        return Classification(
            True,
            False,
            f"{range_reason} includes full-gate path {non_docs[0]!r}",
        )
    return Classification(
        False,
        True,
        f"{range_reason} contains only cheap documentation ({len(changes)} path(s))",
    )


def _write_github_output(path: str, result: Classification) -> None:
    with open(path, "a", encoding="utf-8", newline="\n") as output:
        output.write(f"full={'true' if result.full else 'false'}\n")
        output.write(f"docs_only={'true' if result.docs_only else 'false'}\n")


def _parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", required=True)
    parser.add_argument("--before")
    parser.add_argument("--head")
    parser.add_argument("--base")
    parser.add_argument("--default-ref")
    parser.add_argument("--output")
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    options = _parse_arguments(sys.argv[1:] if arguments is None else arguments)
    result = classify(
        options.event,
        options.before,
        options.head,
        options.base,
        options.default_ref,
    )
    output_path = options.output or os.environ.get("GITHUB_OUTPUT")
    if output_path:
        try:
            _write_github_output(output_path, result)
        except OSError as error:
            print(f"CI change classification could not write GitHub output: {error}", file=sys.stderr)
            return 2
    print(
        "CI change classification: "
        f"full={'true' if result.full else 'false'} "
        f"docs_only={'true' if result.docs_only else 'false'}; {result.reason}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
