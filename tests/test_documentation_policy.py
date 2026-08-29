from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts import check_documentation


class DocumentationPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.repo_root = Path(__file__).resolve().parents[1]

    def test_repository_satisfies_documentation_policy(self) -> None:
        errors, stats = check_documentation.validate_repository(self.repo_root)
        self.assertGreater(stats.markdown_files, 0)
        self.assertEqual([], errors, "\n".join(errors))

    def test_minimal_repository_is_accepted_without_persisted_counts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)

            errors, stats = check_documentation.validate_repository(root)

            self.assertEqual([], errors)
            self.assertEqual(15, stats.markdown_files)
            self.assertEqual(2, stats.fence_lines)
            self.assertEqual(1, stats.relative_links)
            self.assertNotIn("markdown=", (root / "docs/implementation-plan.md").read_text())

    def test_duplicate_marker_and_broken_markdown_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "README.md").write_text(
                "<!-- canonical-live-status:start -->\n"
                "## Live status\n"
                "The delivered count is 2. Workflow 12345678901.\n"
                "[missing](missing.md)\n"
                "```text\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)
            rendered = "\n".join(errors)

            self.assertIn("must occur exactly once", rendered)
            self.assertIn("GitHub Actions run IDs", rendered)
            self.assertIn("delivered-count phrase", rendered)
            self.assertIn("live status header", rendered)
            self.assertIn("unclosed Markdown fence", rendered)
            self.assertIn("missing relative link target", rendered)

    def test_session_contracts_are_governed(self) -> None:
        governed = set(check_documentation.GOVERNED_OVERVIEWS)

        self.assertTrue(
            {
                Path("docs/session-cli.md"),
                Path("docs/native-session-inspection.md"),
                Path("docs/session-store.md"),
            }.issubset(governed)
        )

    def test_each_session_contract_rejects_mutable_status_prose(self) -> None:
        session_contracts = (
            Path("docs/session-cli.md"),
            Path("docs/native-session-inspection.md"),
            Path("docs/session-store.md"),
        )
        for relative in session_contracts:
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                (root / relative).write_text(
                    "# Durable contract\n\nStatus: slice 32 is delivered.\n",
                    encoding="utf-8",
                )

                errors, _ = check_documentation.validate_repository(root)

                self.assertIn(
                    f"{relative}: must not contain mutable top-level Status prose",
                    errors,
                )

    def test_mutable_delivery_evidence_is_rejected_from_durable_docs(self) -> None:
        cases = (
            (
                "actions run",
                "The GitHub Actions run ID is `12345678901`.\n",
                "GitHub Actions run IDs",
            ),
            (
                "workflow id",
                "The exact workflow ID is `12345678901`.\n",
                "GitHub Actions run IDs",
            ),
            (
                "ci run",
                "The feature CI run `12345678901` passed.\n",
                "GitHub Actions run IDs",
            ),
            (
                "delivered count",
                "Delivered slices: 32\n",
                "delivered-count phrase",
            ),
            (
                "sha lineage",
                "Delivered from commit `0123456789abcdef0123456789abcdef01234567`.\n",
                "SHA-style delivery lineage",
            ),
        )
        for name, evidence, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/session-cli.md")
                (root / relative).write_text(
                    f"# Durable contract\n\n{evidence}", encoding="utf-8"
                )

                errors, _ = check_documentation.validate_repository(root)

                self.assertTrue(
                    any(str(relative) in error and expected in error for error in errors),
                    "\n".join(errors),
                )

    def test_live_evidence_is_allowed_in_plan_and_historical_reviews(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            history = root / "docs/reviews/m03-historical-review.md"
            history.parent.mkdir(parents=True, exist_ok=True)
            history.write_text(
                "# Historical review\n\n"
                "Status: accepted.\n"
                "Workflow ID `12345678901` reviewed commit "
                "`0123456789abcdef0123456789abcdef01234567`.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_reference_host_inventory_counts_are_rejected_outside_canonical_docs(
        self,
    ) -> None:
        claims = (
            "The fifteen-tool alphabetical reference catalog is stable.\n",
            "The same fifteen-tool catalog is stable.\n",
            "The reference host registers exactly fourteen tools.\n",
            "The reference host registers nineteen tools.\n",
            "Composition contains thirteen alphabetical tools.\n",
            "The host has exactly four workspace tools.\n",
            "Exactly twelve workspace-backed tools share the descriptor.\n",
            "There are twelve descriptor-backed tools.\n",
            "The root uses eleven identity-preserving clones.\n",
            "One descriptor plus three clones across those four tools.\n",
            "The twelve-tool production reference-host composition is stable.\n",
            "The workspace descriptor is distributed through eleven clones.\n",
            "The eleven-tool/ten-clone host is stable.\n",
            "The reference-host tool catalog contains nineteen entries.\n",
            "Nineteen built-in tools are exposed by NativeReferenceHost.\n",
            "NativeReferenceHost exposes nineteen built-in ToolSpec entries.\n",
            "The reference-host catalog size is nineteen.\n",
            "The catalog in `docs/native-reference-host.md` contains nineteen tools.\n",
            "NativeReferenceHost has nineteen tools.\n",
            "The tool catalog lists nineteen entries.\n",
            "NativeReferenceHost comprises nineteen tools.\n",
            "NativeReferenceHost provides nineteen built-in tools.\n",
            "Nineteen tools make up NativeReferenceHost.\n",
            "NativeReferenceHost totals nineteen tools.\n",
            "NativeReferenceHost includes nineteen tools.\n",
            "There are nineteen tools in NativeReferenceHost.\n",
            "NativeReferenceHost is composed of nineteen tools.\n",
            "NativeReferenceHost installs nineteen tools.\n",
            "NativeReferenceHost owns nineteen tools.\n",
            "NativeReferenceHost supplies nineteen tools.\n",
            "NativeReferenceHost's nineteen tools are deterministic.\n",
            "NativeReferenceHost exposes a nineteen-entry catalog.\n",
            "NativeReferenceHost registers nineteen ToolSpec values.\n",
            "NativeReferenceHost registers nineteen tools and permits two active "
            "tool calls.\n",
            "NativeReferenceHost registers nineteen tools, permitting two active "
            "tool calls.\n",
            "NativeReferenceHost permits two active tool calls; registers nineteen "
            "tools.\n",
            "NativeReferenceHost registers nineteen tools\n"
            "and permits two active tool calls.\n",
            "NativeReferenceHost registers at most nineteen tools.\n",
            "The reference-host catalog contains up to nineteen tools.\n",
            "NativeReferenceHost includes a maximum of nineteen ToolSpec objects.\n",
            "NativeReferenceHost has nineteen tools that execute concurrently.\n",
            "NativeReferenceHost's nineteen tools run concurrently.\n",
            "NativeReferenceHost provides nineteen built-ins.\n",
            "NativeReferenceHost exposes nineteen tool schemas.\n",
            "NativeReferenceHost registers a total of nineteen.\n",
            "NativeReferenceHost has thirty tools.\n",
            "NativeReferenceHost has a dozen tools.\n",
            "NativeReferenceHost is stable. It registers nineteen tools.\n",
            "NativeReferenceHost is stable.\n\nIt registers nineteen tools.\n",
            "NativeReferenceHost is stable.\n\nHowever, it registers nineteen tools.\n",
            "NativeReferenceHost is stable. Nevertheless, it registers nineteen "
            "tools.\n",
            "The catalog is defined in docs/native-reference-host.md. It contains "
            "nineteen tools.\n",
            "NativeReferenceHost (e.g. in production) registers nineteen tools.\n",
            "## NativeReferenceHost\n\nIt registers nineteen tools.\n",
            "The number of tools exposed by NativeReferenceHost is nineteen.\n",
            "NativeReferenceHost exposes nineteen.\n",
            "NativeReferenceHost exposes at most nineteen.\n",
            "The tool count of NativeReferenceHost is nineteen.\n",
        )
        for claim in claims:
            with self.subTest(claim=claim), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(
                    f"# Durable tool contract\n\n{claim}", encoding="utf-8"
                )

                errors, _ = check_documentation.validate_repository(root)

                self.assertIn(
                    f"{relative}: reference-host inventory counts belong only in "
                    "docs/native-reference-host.md#tool-catalog",
                    errors,
                )

    def test_reference_host_inventory_counts_are_allowed_in_canonical_history(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/native-reference-host.md").write_text(
                "# Reference host\n\n## Tool catalog\n\n"
                "The engine registers exactly fourteen tools.\n",
                encoding="utf-8",
            )
            review = root / "docs/reviews/m03-historical-review.md"
            review.parent.mkdir(parents=True, exist_ok=True)
            review.write_text(
                "# Historical review\n\n"
                "The thirteen alphabetical tools used twelve identity-preserving "
                "clones.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_reference_host_inventory_is_allowed_only_in_tool_catalog_section(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/native-reference-host.md")
            (root / relative).write_text(
                "# Reference host\n\n"
                "## Tool catalog\n\n"
                "The engine registers exactly fourteen tools.\n\n"
                "## Construction effects\n\n"
                "NativeReferenceHost has thirteen tools.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertIn(
                f"{relative}: reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog",
                errors,
            )

    def test_reference_host_catalog_section_must_be_unique_and_comment_safe(
        self,
    ) -> None:
        cases = (
            (
                "duplicate",
                "## Tool catalog\n\nNativeReferenceHost has thirteen tools.\n",
                "must contain exactly one Tool catalog section; found 2",
            ),
            (
                "comment",
                "<!--\n## Tool catalog\n-->\n"
                "NativeReferenceHost has thirteen tools.\n",
                "reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog",
            ),
        )
        for name, suffix, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/native-reference-host.md")
                (root / relative).write_text(
                    "# Reference host\n\n"
                    "## Tool catalog\n\n"
                    "The engine registers exactly fourteen tools.\n\n"
                    "## Construction effects\n\n"
                    f"{suffix}",
                    encoding="utf-8",
                )

                errors, _ = check_documentation.validate_repository(root)

                self.assertTrue(
                    any(str(relative) in error and expected in error for error in errors),
                    "\n".join(errors),
                )

    def test_setext_peer_heading_ends_tool_catalog_section(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/native-reference-host.md")
            (root / relative).write_text(
                "# Reference host\n\n"
                "## Tool catalog\n\n"
                "The engine registers exactly fourteen tools.\n\n"
                "Construction effects\n"
                "--------------------\n\n"
                "NativeReferenceHost has thirteen tools.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertIn(
                f"{relative}: reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog",
                errors,
            )

    def test_reference_host_inventory_policy_allows_unrelated_numeric_limits(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/tool-contract.md").write_text(
                "# Durable tool contract\n\n"
                "The executor accepts four active tools, retains twelve KiB, and "
                "permits two supplied-Waker clones.\n\n"
                "At most four workspace tools may execute concurrently.\n\n"
                "The executor makes two clones across four active tools, one per "
                "Waker.\n\n"
                "NativeReferenceHost may execute four workspace tools concurrently.\n\n"
                "The reference host permits two clones of the supplied Waker.\n\n"
                "NativeReferenceHost contains two supplied Waker clones while a "
                "call is active.\n\n"
                "NativeReferenceHost contains two clones while a call is active.\n\n"
                "NativeReferenceHost has at most four active tools.\n\n"
                "NativeReferenceHost has four active tools.\n\n"
                "NativeReferenceHost has a concurrency limit of four tools.\n\n"
                "NativeReferenceHost runs four tools at once.\n\n"
                "NativeReferenceHost queues four tools.\n\n"
                "NativeReferenceHost's active set contains four tools.\n\n"
                "NativeReferenceHost batches four tools per turn.\n\n"
                "NativeReferenceHost owns two clones for each Waker.\n\n"
                "NativeReferenceHost owns two clones while a call is active.\n\n"
                "NativeReferenceHost provides four tools per request.\n\n"
                "NativeReferenceHost records nineteen tool invocations before "
                "recycling them.\n\n"
                "NativeReferenceHost records nineteen tool retries, nineteen tool "
                "handles, and nineteen tool latencies.\n\n"
                "NativeReferenceHost marks nineteen tool entries active.\n\n"
                "NativeReferenceHost: four workspace tools may execute "
                "concurrently.\n\n"
                "At one scheduler checkpoint, four workspace tools may execute "
                "concurrently.\n\n"
                "Pipeline composition accepts four tools.\n\n"
                "Limits allow sixteen tool calls beside a one MiB tool catalog.\n\n"
                "```text\n"
                "The reference host registers exactly fourteen tools.\n"
                "```\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_html_comment_fence_cannot_hide_visible_inventory(self) -> None:
        payloads = (
            (
                "<!--\n```md\n-->\n"
                "NativeReferenceHost has nineteen tools.\n"
                "```\n",
                True,
            ),
            (
                "```text\n<!--\n```\n"
                "NativeReferenceHost has nineteen tools.\n"
                "-->\n```\n",
                True,
            ),
            (
                "> > <!--\n"
                "> NativeReferenceHost has nineteen tools.\n"
                "> > -->\n",
                False,
            ),
        )
        for payload, unclosed in payloads:
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(
                    f"# Durable tool contract\n\n{payload}",
                    encoding="utf-8",
                )

                errors, _ = check_documentation.validate_repository(root)
                rendered = "\n".join(errors)

                self.assertIn("reference-host inventory counts belong only", rendered)
                self.assertEqual(
                    unclosed,
                    f"{relative}: unclosed Markdown fence" in errors,
                    rendered,
                )

    def test_inline_code_comment_marker_cannot_hide_policy_prose(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/tool-contract.md")
            (root / relative).write_text(
                "# Durable tool contract\n\n"
                "The literal opener is `<!--`.\n"
                "NativeReferenceHost has nineteen tools.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertIn(
                f"{relative}: reference-host inventory counts belong only in "
                "docs/native-reference-host.md#tool-catalog",
                errors,
            )

    def test_multiline_code_span_and_unmatched_tick_preserve_comment_semantics(
        self,
    ) -> None:
        cases = (
            (
                "multiline code span",
                "The literal is `first\n<!--\nsecond`.\n"
                "NativeReferenceHost has nineteen tools.\n",
                True,
            ),
            (
                "unmatched literal",
                "The unmatched literal is ` before a real comment.\n"
                "<!--\nNativeReferenceHost has nineteen tools.\n-->\n",
                False,
            ),
        )
        for name, payload, rejected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(
                    f"# Durable tool contract\n\n{payload}", encoding="utf-8"
                )

                errors, _ = check_documentation.validate_repository(root)
                inventory_errors = [
                    error for error in errors
                    if "reference-host inventory counts belong only" in error
                ]

                self.assertEqual(rejected, bool(inventory_errors), "\n".join(errors))

    def test_fence_openers_and_closers_follow_markdown_syntax(self) -> None:
        cases = (
            (
                "invalid backtick info",
                "```bad`info\nNativeReferenceHost has nineteen tools.\n```\n",
                True,
                True,
            ),
            (
                "trailing closer text",
                "```text\n```not-a-close\n"
                "NativeReferenceHost has nineteen tools.\n```\n",
                False,
                False,
            ),
            (
                "quoted fence",
                "> ```text\n> NativeReferenceHost has nineteen tools.\n> ```\n",
                False,
                False,
            ),
            (
                "list fence",
                "- ```text\n  NativeReferenceHost has nineteen tools.\n  ```\n",
                False,
                False,
            ),
        )
        for name, payload, rejected, unclosed in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(
                    f"# Durable tool contract\n\n{payload}", encoding="utf-8"
                )

                errors, _ = check_documentation.validate_repository(root)
                inventory_errors = [
                    error for error in errors
                    if "reference-host inventory counts belong only" in error
                ]

                self.assertEqual(rejected, bool(inventory_errors), "\n".join(errors))
                self.assertEqual(
                    unclosed,
                    any("unclosed Markdown fence" in error for error in errors),
                    "\n".join(errors),
                )

    def test_inline_code_cannot_pair_across_fenced_block_boundaries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/tool-contract.md")
            (root / relative).write_text(
                "# Durable tool contract\n\n"
                "The unmatched literal starts `here.\n\n"
                "```text\n"
                "NativeReferenceHost has nineteen tools.\n"
                "```\n\n"
                "The other unmatched literal ends ` here.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_markdown_scanner_tokenizes_many_code_spans_once(self) -> None:
        class RecordingPattern:
            def __init__(self, pattern: object) -> None:
                self.pattern = pattern
                self.calls = 0

            def finditer(self, text: str):  # type: ignore[no-untyped-def]
                self.calls += 1
                return self.pattern.finditer(text)  # type: ignore[attr-defined]

        original = check_documentation.HTML_COMMENT_OPEN_RE
        recording = RecordingPattern(original)
        check_documentation.HTML_COMMENT_OPEN_RE = recording  # type: ignore[assignment]
        try:
            text = "`x`" * 65_536 + "<!-- hidden -->visible\n"
            prose, _, unclosed = check_documentation._scan_markdown_inert_blocks(text)
        finally:
            check_documentation.HTML_COMMENT_OPEN_RE = original

        self.assertEqual(1, recording.calls)
        self.assertFalse(unclosed)
        self.assertTrue(prose.endswith("visible\n"))

    def test_operational_classifier_slices_bounded_context_first(self) -> None:
        class SliceRecordingStr(str):
            widths: list[int] = []

            def __getitem__(self, key: object) -> str:
                if isinstance(key, slice):
                    start, stop, step = key.indices(len(self))
                    if step == 1:
                        self.widths.append(max(0, stop - start))
                return super().__getitem__(key)  # type: ignore[arg-type]

        clause = SliceRecordingStr(
            "x" * 100_000 + " NativeReferenceHost permits two active tools."
        )
        match = check_documentation.REFERENCE_HOST_INVENTORY_COUNT_RE.search(clause)
        self.assertIsNotNone(match)

        self.assertTrue(
            check_documentation._counted_noun_is_operational(clause, match)
        )
        self.assertLessEqual(max(clause.widths), 120)

    def test_markdown_input_ceiling_bounds_scanner_memory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/generated.md")
            (root / relative).write_bytes(
                b"x" * (check_documentation.MAX_MARKDOWN_BYTES + 1)
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual(
                [
                    f"{relative}: exceeds the "
                    f"{check_documentation.MAX_MARKDOWN_BYTES}-byte Markdown ceiling"
                ],
                errors,
            )

    def test_validation_scans_each_markdown_file_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            original = check_documentation._scan_markdown_inert_blocks
            calls = 0

            def recording(text: str) -> tuple[str, int, bool]:
                nonlocal calls
                calls += 1
                return original(text)

            check_documentation._scan_markdown_inert_blocks = recording
            try:
                errors, stats = check_documentation.validate_repository(root)
            finally:
                check_documentation._scan_markdown_inert_blocks = original

            self.assertEqual([], errors)
            self.assertEqual(stats.markdown_files, calls)

    def test_backtick_index_uses_compact_numeric_arrays(self) -> None:
        starts, ends, next_same = check_documentation._backtick_runs("`x` " * 10_000)

        self.assertEqual(20_000, len(starts))
        self.assertEqual(len(starts), len(ends))
        self.assertEqual(len(starts), len(next_same))
        self.assertEqual(4, starts.itemsize)
        self.assertEqual(4, ends.itemsize)
        self.assertEqual(4, next_same.itemsize)

    def test_durable_limits_hashes_and_policy_terms_are_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/session-store.md").write_text(
                "# Durable contract\n\n"
                "Workflow IDs are not product state. The byte ceiling is "
                "12,345,678,901 bytes.\n\n"
                "A content SHA-256 is "
                "`0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`.\n\n"
                "```text\n"
                "Status: examples may demonstrate rejected maintenance prose.\n"
                "Workflow ID 12345678901\n"
                "```\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def _write_minimal_repository(self, root: Path) -> None:
        for relative in check_documentation.GOVERNED_OVERVIEWS:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("# Evergreen\n", encoding="utf-8")

        (root / check_documentation.REFERENCE_HOST_CONTRACT_PATH).write_text(
            "# Reference host\n\n## Tool catalog\n",
            encoding="utf-8",
        )

        plan = root / check_documentation.PLAN_PATH
        plan.parent.mkdir(parents=True, exist_ok=True)
        plan.write_text(
            "# Implementation plan\n\n"
            f"{check_documentation.START_MARKER}\n"
            "- Delivered slices: `2`\n"
            "- Delivered main: `0123456789abcdef0123456789abcdef01234567`\n"
            "- Main CI: `123` (`GREEN`)\n"
            "- Main Benchmark evidence: `456` (`GREEN`)\n"
            "- Active branch: `agent/m03-docs-policy`\n"
            "- Active phase: `documentation maintenance`\n"
            "- Next gate: `run checks`\n"
            f"{check_documentation.END_MARKER}\n\n"
            "| Slice | Deliverable |\n"
            "| ---: | --- |\n"
            "| 1 | one |\n"
            "| 2 | [two](two.md) |\n\n"
            "```text\n"
            "bounded\n"
            "```\n",
            encoding="utf-8",
        )
        (root / "docs/two.md").write_text("# Two\n", encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
