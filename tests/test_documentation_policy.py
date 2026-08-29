from __future__ import annotations

import tempfile
import time
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

    def test_every_unlisted_durable_document_rejects_live_delivery_evidence(
        self,
    ) -> None:
        for relative in (
            Path("docs/unlisted-contract.md"),
            Path("docs/reviews/README.md"),
        ):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                (root / relative).write_text(
                    "# Durable contract\n\n"
                    "## Current status\n\n"
                    "Delivered slices: 999\n\n"
                    "Feature CI run `12345678901` passed.\n\n"
                    "Delivered from commit "
                    "`0123456789abcdef0123456789abcdef01234567`.\n",
                    encoding="utf-8",
                )

                errors, _ = check_documentation.validate_repository(root)
                rendered = "\n".join(errors)

                self.assertIn(
                    f"{relative}: must not contain a live status header", errors
                )
                self.assertIn("delivered-count phrase", rendered)
                self.assertIn("GitHub Actions run IDs", rendered)
                self.assertIn("SHA-style delivery lineage", rendered)

    def test_canonical_live_fields_are_reserved_to_the_plan(self) -> None:
        payload = (
            "Main CI: `123` (`GREEN`)\n\n"
            "- Main Benchmark evidence: pending\n\n"
            "Delivered main: pending\n\n"
            "> - > Active branch: `agent/m05-other`\n\n"
            "## Active phase: implementing memory\n\n"
            "1. Next gate: push main\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/unlisted.md")
            (root / relative).write_text(payload, encoding="utf-8")

            errors, _ = check_documentation.validate_repository(root)
            rendered = "\n".join(errors)

            for field in (
                "Main CI",
                "Main Benchmark evidence",
                "Delivered main",
                "Active branch",
                "Active phase",
                "Next gate",
            ):
                self.assertIn(
                    f"canonical live-status field '{field}'",
                    rendered,
                )

    def test_evergreen_prose_does_not_claim_canonical_live_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/unlisted.md").write_text(
                "# Evergreen\n\n"
                "Main CI uses Rust 1.94.1. The next gate is selected by policy.\n\n"
                "An active branch uses the documented naming convention.\n\n"
                "Current phase: durable documentation.\n\n"
                "| Main CI | exact-SHA workflow |\n"
                "| --- | --- |\n\n"
                "```text\nActive phase: example only\n```\n\n"
                "<!-- Next gate: hidden example -->\n",
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
            "NativeReferenceHost registers no more than nineteen.\n",
            "NativeReferenceHost registers not more than nineteen.\n",
            "NativeReferenceHost registers fewer than nineteen.\n",
            "NativeReferenceHost tool count is nineteen.\n",
            "NativeReferenceHost has a tool count of nineteen.\n",
            "The tool count for NativeReferenceHost equals nineteen.\n",
            "NativeReferenceHost tracks nineteen tools in its catalog.\n",
            "NativeReferenceHost labels nineteen built-in tools as stable.\n",
            "NativeReferenceHost records nineteen tools in the catalog.\n",
            "The reference host exposes a catalog of nineteen.\n",
            "The reference host exposes a catalog of at most nineteen.\n",
            "NativeReferenceHost exposes nineteen distinct capabilities.\n",
            "NativeReferenceHost is stable. However, in practice, it registers "
            "nineteen tools.\n",
            "NativeReferenceHost is stable. Even so, it registers nineteen tools.\n",
            "NativeReferenceHost is stable. As a result, it registers nineteen "
            "tools.\n",
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
                "NativeReferenceHost tracks nineteen active tools in flight.\n\n"
                "NativeReferenceHost labels nineteen tool entries active.\n\n"
                "NativeReferenceHost records nineteen tools per request.\n\n"
                "NativeReferenceHost retries nineteen tool entries.\n\n"
                "NativeReferenceHost retried nineteen tool entries.\n\n"
                "NativeReferenceHost may retry nineteen tool entries.\n\n"
                "NativeReferenceHost has a tool count of nineteen per request.\n\n"
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

    def test_escaped_html_comment_opener_remains_rendered(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/tool-contract.md")
            (root / relative).write_text(
                "\\<!-- NativeReferenceHost has nineteen tools. "
                "[broken](missing.md) -->\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)
            rendered = "\n".join(errors)

            self.assertIn("reference-host inventory counts belong only", rendered)
            self.assertIn("missing relative link target: missing.md", rendered)

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
            (
                "escaped opener before real code",
                "Escaped \\` then code ` <!--\n"
                "NativeReferenceHost has nineteen tools.\n--> ` after.\n",
                True,
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
            (
                "list then quote fence",
                "- > ```text\n  > NativeReferenceHost has nineteen tools.\n"
                "  > ```\n",
                False,
                False,
            ),
            (
                "nested list fence",
                "- - ```text\n    NativeReferenceHost has nineteen tools.\n"
                "    ```\n",
                False,
                False,
            ),
            (
                "quote list quote fence",
                "> - > ```text\n>   > NativeReferenceHost has nineteen tools.\n"
                ">   > ```\n",
                False,
                False,
            ),
            (
                "ambient list fence exit",
                "- item\n  ```text\nNativeReferenceHost has nineteen tools.\n"
                "  ```\n",
                True,
                True,
            ),
            (
                "blank list fence continuation",
                "- ```text\n  first\n\n  NativeReferenceHost has nineteen tools.\n"
                "  ```\n",
                False,
                False,
            ),
            (
                "tab list fence continuation",
                "-\t```text\n\tNativeReferenceHost has nineteen tools.\n\t```\n",
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
                    error
                    for error in errors
                    if "reference-host inventory counts belong only" in error
                ]

                self.assertEqual(rejected, bool(inventory_errors), "\n".join(errors))
                self.assertEqual(
                    unclosed,
                    any("unclosed Markdown fence" in error for error in errors),
                    "\n".join(errors),
                )

    def test_html_comments_follow_block_and_lazy_list_ownership(self) -> None:
        cases = (
            (
                "nested list outdent",
                "- - <!--\n  NativeReferenceHost has nineteen tools.\n  -->\n",
                True,
            ),
            (
                "ambient list block comment exit",
                "- item\n  <!--\nNativeReferenceHost has nineteen tools.\n"
                "  -->\n",
                True,
            ),
            (
                "lazy inline list comment",
                "- Text <!--\nNativeReferenceHost has nineteen tools.\n-->\n",
                False,
            ),
            (
                "nested list block comment",
                "- - <!--\n    NativeReferenceHost has nineteen tools.\n"
                "    -->\n",
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
                    error
                    for error in errors
                    if "reference-host inventory counts belong only" in error
                ]

                self.assertEqual(rejected, bool(inventory_errors), "\n".join(errors))

    def test_inline_comment_closes_only_inside_its_paragraph(self) -> None:
        cases = (
            (
                "valid root continuation",
                "Text <!--\nNativeReferenceHost has nineteen tools. --> suffix\n",
                False,
                False,
            ),
            (
                "valid lazy list continuation",
                "- Text <!--\nNativeReferenceHost has nineteen tools. -->\n",
                False,
                False,
            ),
            (
                "noninterrupting ordered two",
                "Text <!--\n2. NativeReferenceHost has nineteen tools. -->\n",
                False,
                False,
            ),
            (
                "blank interruption",
                "Text <!--\n\nNativeReferenceHost has nineteen tools.\n-->\n",
                True,
                False,
            ),
            (
                "ATX close interruption",
                "Text <!--\n# NativeReferenceHost has nineteen tools. -->\n",
                True,
                False,
            ),
            (
                "Setext interruption",
                "Text <!--\n===\nNativeReferenceHost has nineteen tools.\n-->\n",
                True,
                False,
            ),
            (
                "thematic interruption",
                "Text <!--\n***\nNativeReferenceHost has nineteen tools.\n-->\n",
                True,
                False,
            ),
            (
                "unordered list close interruption",
                "Text <!--\n- NativeReferenceHost has nineteen tools. -->\n",
                True,
                False,
            ),
            (
                "ordered one close interruption",
                "Text <!--\n1. NativeReferenceHost has nineteen tools. -->\n",
                True,
                False,
            ),
            (
                "fence close text is info",
                "Text <!--\n```md -->\n"
                "NativeReferenceHost has nineteen tools.\n```\n",
                False,
                False,
            ),
            (
                "ATX heading opener is line scoped",
                "# Title <!--\nNativeReferenceHost has nineteen tools. -->\n",
                True,
                False,
            ),
            (
                "HTML block interruption",
                "Text <!--\n"
                "<div>NativeReferenceHost has nineteen tools. --></div>\n",
                True,
                False,
            ),
        )
        for name, payload, rejected, unclosed in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(payload, encoding="utf-8")

                errors, _ = check_documentation.validate_repository(root)
                rendered = "\n".join(errors)

                self.assertEqual(
                    rejected,
                    "reference-host inventory counts belong only" in rendered,
                    rendered,
                )
                self.assertEqual(
                    unclosed,
                    f"{relative}: unclosed Markdown fence" in errors,
                    rendered,
                )

    def test_empty_list_items_preserve_nested_block_ownership(self) -> None:
        cases = (
            (
                "empty bullet fence outdent",
                "-\n  ```text\nNativeReferenceHost has nineteen tools.\n  ```\n",
                True,
                True,
            ),
            (
                "empty bullet fence retained",
                "-   \n  ```text\n  NativeReferenceHost has nineteen tools.\n  ```\n",
                False,
                False,
            ),
            (
                "empty ordered comment outdent",
                "1.\n   <!--\nNativeReferenceHost has nineteen tools.\n   -->\n",
                True,
                False,
            ),
            (
                "empty ordered comment retained",
                "1.   \n   <!--\n   NativeReferenceHost has nineteen tools.\n"
                "   -->\n",
                False,
                False,
            ),
            (
                "quote empty list outdent",
                "> -\n>   <!--\n"
                "> NativeReferenceHost has nineteen tools.\n>   -->\n",
                True,
                False,
            ),
        )
        for name, payload, rejected, unclosed in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(payload, encoding="utf-8")

                errors, _ = check_documentation.validate_repository(root)
                rendered = "\n".join(errors)

                self.assertEqual(
                    rejected,
                    "reference-host inventory counts belong only" in rendered,
                    rendered,
                )
                self.assertEqual(
                    unclosed,
                    f"{relative}: unclosed Markdown fence" in errors,
                    rendered,
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

    def test_inline_code_cannot_pair_across_paragraph_block_boundaries(self) -> None:
        payloads = (
            "Text starts `\n# Heading [real](missing-heading.md) `\n",
            "Text starts `\n- item [real](missing-list.md) `\n",
            "Text starts `\n> quote [real](missing-quote.md) `\n",
            "Text starts `\n<div>raw HTML</div>\n\n"
            "[real](missing-html.md) `\n",
            "Text starts `\n<?pi raw?>\n[real](missing-pi.md) `\n",
        )
        for payload in payloads:
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                relative = Path("docs/tool-contract.md")
                (root / relative).write_text(payload, encoding="utf-8")

                errors, _ = check_documentation.validate_repository(root)

                self.assertTrue(
                    any("missing relative link target" in error for error in errors),
                    "\n".join(errors),
                )

    def test_link_validation_uses_only_rendered_markdown_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/tool-contract.md").write_text(
                "# Durable tool contract\n\n"
                "[`two`](two.md) is real.\n\n"
                "The literal syntax is `[example](missing-inline.md)`.\n\n"
                "<!-- [example](missing-comment.md) -->\n\n"
                "```md\n[example](missing-fence.md)\n```\n",
                encoding="utf-8",
            )

            errors, stats = check_documentation.validate_repository(root)

            self.assertEqual([], errors)
            self.assertEqual(2, stats.relative_links)

    def test_markdown_link_target_scanner_preserves_supported_syntax(self) -> None:
        markup = (
            "[one](one.md) ![image](image.png) [[nested](nested.md) "
            "[angle](<dir/file name.md>) [title](target.md \"title\") "
            "[multiline](\n next.md) []() [empty-angle](<>) "
            "[unterminated](<tail"
        )

        targets = list(check_documentation._markdown_link_targets(markup))

        self.assertEqual(
            [
                "one.md",
                "image.png",
                "nested.md",
                "<dir/file name.md>",
                "target.md",
                "next.md",
                "<>",
            ],
            targets,
        )

    def test_rendered_link_grammar_handles_escapes_balance_and_completion(
        self,
    ) -> None:
        cases = (
            ("[a\\]](missing.md)", ["missing.md"]),
            ("[outer [inner]](missing.md)", ["missing.md"]),
            ("\\[literal](missing.md)", []),
            ("[x](dir/missing(and).md)", ["dir/missing(and).md"]),
            ("[x](<dir/missing file.md>)", ["<dir/missing file.md>"]),
            ("[x](missing.md \"title\")", ["missing.md"]),
            ("[x](missing.md 'title')", ["missing.md"]),
            ("[x](missing.md (title))", ["missing.md"]),
            ("[x](<foo.md>\"title\")", []),
            ("[x](missing.md\n\n\"title\")", []),
            ("[x](missing.md \"multi\n\nline\")", []),
            ("[x](missing-eof.md", []),
            ("[x](<missing-eof.md", []),
            ("[x](missing(unbalanced.md)", []),
        )
        for markup, expected in cases:
            with self.subTest(markup=markup):
                self.assertEqual(
                    expected,
                    list(check_documentation._markdown_link_targets(markup)),
                )

    def test_undefined_full_reference_does_not_fall_back_to_shortcut(self) -> None:
        markup = "[label][undefined]\n\n[label]: should-not-render.md\n"

        self.assertEqual(
            [], list(check_documentation._markdown_link_targets(markup))
        )
        self.assertIn(
            "[label][undefined]",
            check_documentation._normalize_policy_markup(markup),
        )

    def test_relative_link_targets_unescape_markdown_punctuation(self) -> None:
        self.assertEqual(
            "dir/a(b).md",
            check_documentation._relative_link_target(r"dir/a\(b\).md"),
        )
        self.assertEqual(
            r"dir/a\ b.md",
            check_documentation._relative_link_target(r"<dir/a\ b.md>"),
        )
        self.assertIsNone(
            check_documentation._relative_link_target(
                "https&#58;//example.invalid/path"
            )
        )
        self.assertEqual(
            "foo&bar.md",
            check_documentation._relative_link_target("foo&amp;bar.md"),
        )
        self.assertEqual(
            "foo\tbar.md",
            check_documentation._relative_link_target("foo%09bar.md"),
        )
        self.assertEqual(
            "foo\tbar.md",
            check_documentation._relative_link_target("foo&Tab;bar.md"),
        )
        self.assertEqual(
            "missing\0.md",
            check_documentation._relative_link_target("missing%00.md"),
        )
        self.assertEqual(
            [],
            list(check_documentation._markdown_link_targets("[x](\tmissing.md)")),
        )
        tab_cases = (
            ("[x](\tmissing.md)", []),
            ("[x](\n\tmissing.md)", ["missing.md"]),
            ("[x](foo.md\t \"title\")", []),
            ("[x](\nfoo.md\t \"title\")", []),
            ("[x](foo.md\n\t\"title\")", ["foo.md"]),
        )
        for markup, expected in tab_cases:
            with self.subTest(markup=markup):
                scan = check_documentation._scan_markdown_inert_blocks(markup)
                self.assertEqual(
                    expected,
                    list(check_documentation._markdown_link_targets(scan.link_markup)),
                )

    def test_invalid_decoded_relative_link_target_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/foo\tbar.md").write_text("# Tab name\n", encoding="utf-8")
            (root / "docs/tool-contract.md").write_text(
                "[percent tab](foo%09bar.md)\n"
                "[entity tab](foo&Tab;bar.md)\n"
                "[nul](missing%00.md)\n"
                "[literal tab](\tmissing.md)\n",
                encoding="utf-8",
            )

            errors, stats = check_documentation.validate_repository(root)

            self.assertEqual(
                ["docs/tool-contract.md: invalid relative link target"], errors
            )
            self.assertEqual(4, stats.relative_links)

    def test_reference_style_relative_links_are_validated(self) -> None:
        payloads = (
            "[real][key]\n\n[key]: missing-full.md\n",
            "[real][key]\n\n[key]:\n  missing-multiline.md\n",
            "[collapsed][]\n\n[collapsed]: missing-collapsed.md\n",
            "[shortcut]\n\n[shortcut]: missing-shortcut.md\n",
            "![image][asset]\n\n[asset]: missing-image.png\n",
        )
        for payload in payloads:
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                (root / "docs/tool-contract.md").write_text(
                    payload, encoding="utf-8"
                )

                errors, stats = check_documentation.validate_repository(root)

                self.assertTrue(
                    any("missing relative link target" in error for error in errors),
                    "\n".join(errors),
                )
                self.assertEqual(2, stats.relative_links)

    def test_reference_definition_continuation_title_is_inert(self) -> None:
        markup = (
            "[real][key]\n\n"
            "[key]: missing.md\n"
            "  \"[not-a-link](ignored.md)\"\n"
        )

        self.assertEqual(
            ["missing.md"],
            list(check_documentation._markdown_link_targets(markup)),
        )
        self.assertEqual(
            [],
            list(
                check_documentation._markdown_link_targets(
                    "[real][key]\n\n[key]: <ignored.md>\"title\"\n"
                )
            ),
        )

    def test_reference_definition_cannot_interrupt_a_paragraph(self) -> None:
        markup = (
            "NativeReferenceHost has\n"
            "[note]: nineteen&#32;tools.\n\n"
            "[use][note]\n"
        )

        self.assertEqual([], list(check_documentation._markdown_link_targets(markup)))
        self.assertIn(
            "NativeReferenceHost has\n[note]: nineteen tools.",
            check_documentation._normalize_policy_markup(markup),
        )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/unlisted.md").write_text(markup, encoding="utf-8")

            errors, _ = check_documentation.validate_repository(root)

            self.assertTrue(
                any("reference-host inventory counts belong only" in error for error in errors),
                "\n".join(errors),
            )

    def test_html_block_markdown_links_are_inert(self) -> None:
        markup = (
            "<div>\n[raw](ignored-one.md)\n</div>\n\n"
            "<script>[raw](ignored-two.md)</script>\n"
            "<x-widget data-kind='demo'>\n"
            "[raw](ignored-three.md)\n\n"
            "[real](checked.md)\n"
        )

        self.assertEqual(
            ["checked.md"],
            list(check_documentation._markdown_link_targets(markup)),
        )

    def test_list_owned_raw_html_blocks_survive_blank_lines(self) -> None:
        markup = (
            "- <script>\n\n"
            "  [script](ignored-script.md)\n"
            "  </script>\n\n"
            "- <?pi\n\n"
            "  [pi](ignored-pi.md)\n"
            "  ?>\n\n"
            "- <![CDATA[\n\n"
            "  [cdata](ignored-cdata.md)\n"
            "  ]]>\n\n"
            "[real](checked.md)\n"
        )

        self.assertEqual(
            ["checked.md"],
            list(check_documentation._markdown_link_targets(markup)),
        )

    def test_inline_titles_and_html_attributes_own_literal_delimiters(self) -> None:
        markup = (
            "[comment](comment.md \"<!--\") -->\n\n"
            "[tick](tick.md \"`\") `\n\n"
            "<span title=\"<!--\">[attribute-comment](attribute-comment.md)"
            "</span> -->\n\n"
            "<span title=\"`\">[attribute-tick](attribute-tick.md)</span> `\n"
        )
        scan = check_documentation._scan_markdown_inert_blocks(markup)

        self.assertEqual(
            [
                "comment.md",
                "tick.md",
                "attribute-comment.md",
                "attribute-tick.md",
            ],
            list(check_documentation._markdown_link_targets(scan.link_markup)),
        )

    def test_links_nested_in_image_alt_text_are_not_rendered(self) -> None:
        markup = "![[inner](ignored.md)](image.png)"

        self.assertEqual(
            ["image.png"],
            list(check_documentation._markdown_link_targets(markup)),
        )

    def test_indented_code_links_are_inert_but_paragraph_continuations_render(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/tool-contract.md").write_text(
                "    [root-code](missing-root-code.md)\n\n"
                "-     [same-line-list-code](missing-list-code.md)\n\n"
                "- item\n\n"
                "      [list-code](missing-nested-code.md)\n\n"
                ">     [quote-code](missing-quote-code.md)\n\n"
                "paragraph\n"
                "-     [new-container-code](missing-new-container-code.md)\n\n"
                "Paragraph\n"
                "    [rendered](missing-rendered.md)\n",
                encoding="utf-8",
            )

            errors, stats = check_documentation.validate_repository(root)
            rendered = "\n".join(errors)

            self.assertIn("missing-rendered.md", rendered)
            self.assertNotIn("missing-root-code.md", rendered)
            self.assertNotIn("missing-list-code.md", rendered)
            self.assertNotIn("missing-nested-code.md", rendered)
            self.assertNotIn("missing-quote-code.md", rendered)
            self.assertNotIn("missing-new-container-code.md", rendered)
            self.assertEqual(2, stats.relative_links)

    def test_classifier_uses_rendered_inline_presentation_text(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/unlisted.md")
            (root / relative).write_text(
                "**Main CI:** `999` (`GREEN`)\n\n"
                "[**Active phase:**](https://example.invalid) maintenance\n\n"
                "***Main Benchmark evidence:*** pending\n\n"
                "~~Next gate:~~ pending\n\n"
                "[**Active branch:**][branch] `agent/m05-example`\n\n"
                "<strong>Main&#32;CI:</strong> `999` (`GREEN`)\n\n"
                "## **Current status**\n\n"
                "## <span title=\">\">Current status</span>\n\n"
                "NativeReferenceHost has **nineteen tools**.\n\n"
                "<strong>NativeReferenceHost</strong> has "
                "nine&#116;een tools.\n\n"
                "[NativeReferenceHost](https://example.invalid) has "
                "[nineteen tools](https://example.invalid).\n\n"
                "[branch]: https://example.invalid\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)
            rendered = "\n".join(errors)

            self.assertIn("canonical live-status field 'Main CI'", rendered)
            self.assertIn("canonical live-status field 'Active phase'", rendered)
            self.assertIn(
                "canonical live-status field 'Main Benchmark evidence'", rendered
            )
            self.assertIn("canonical live-status field 'Next gate'", rendered)
            self.assertIn("canonical live-status field 'Active branch'", rendered)
            self.assertIn("live status header", rendered)
            self.assertIn("reference-host inventory counts belong only", rendered)

    def test_classifier_pairs_mixed_emphasis_runs(self) -> None:
        prose = check_documentation._normalize_policy_markup(
            "**Main *CI:*** pending\n"
            "Delivered **slices: *999***\r\n"
        )

        self.assertIn("Main CI: pending", prose)
        self.assertIn("Delivered slices: 999", prose)

    def test_classifier_applies_commonmark_backslash_escapes(self) -> None:
        self.assertEqual(
            "Main CI: pending",
            check_documentation._normalize_policy_markup(r"Main CI\: pending"),
        )
        self.assertEqual(
            r"Main CI\ : pending",
            check_documentation._normalize_policy_markup(r"Main CI\ : pending"),
        )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/unlisted.md").write_text(
                r"Main CI\: pending", encoding="utf-8"
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertTrue(
                any("canonical live-status field 'Main CI'" in error for error in errors),
                "\n".join(errors),
            )

    def test_inline_link_label_is_not_limited_like_a_reference_label(self) -> None:
        markup = "[" + "x" * 1_000 + "](missing.md)"

        self.assertEqual(
            ["missing.md"],
            list(check_documentation._markdown_link_targets(markup)),
        )
        self.assertEqual(
            "x" * 1_000,
            check_documentation._normalize_policy_markup(markup),
        )

    def test_classifier_treats_indented_code_as_inert(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/unlisted.md").write_text(
                "    **Main CI:** `999` (`GREEN`)\n\n"
                "    NativeReferenceHost has **nineteen tools**.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_classifier_preserves_escaped_and_code_literal_wrappers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            (root / "docs/unlisted.md").write_text(
                "\\*\\*Main CI:\\*\\* is literal.\n\n"
                "`**Active phase:** pending` is code.\n\n"
                "NativeReferenceHost has \\*\\*nineteen tools\\*\\* literally.\n\n"
                "NativeReferenceHost documents `nineteen tools` as syntax.\n\n"
                "\\<strong>Main CI:</strong> is escaped literal syntax.\n\n"
                "\\&#77;ain CI: is an escaped literal entity.\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertEqual([], errors)

    def test_pathological_markdown_scans_remain_bounded(self) -> None:
        ceiling = check_documentation.MAX_MARKDOWN_BYTES
        hostile_inputs = (
            "[" * ceiling,
            "\t" * ceiling,
            ("[x](< " * ceiling)[:ceiling],
            ("[" * (ceiling // 2) + "]" * (ceiling // 2))[:ceiling],
            ("- " * (ceiling // 4) + "x\n" + "\n" * ceiling)[:ceiling],
            (
                "- " * (ceiling // 16)
                + "```text\n"
                + "\n" * ceiling
            )[:ceiling],
            (
                "- " * (ceiling // 16)
                + "<!--\n"
                + "\n" * ceiling
            )[:ceiling],
        )
        started = time.monotonic()
        scans = [
            check_documentation._scan_markdown_inert_blocks(source)
            for source in hostile_inputs
        ]
        targets = list(
            check_documentation._markdown_link_targets(hostile_inputs[0])
        )
        elapsed = time.monotonic() - started

        self.assertEqual([], targets)
        self.assertFalse(scans[0].unclosed_fence)
        self.assertLess(elapsed, 5.0)

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
            scan = check_documentation._scan_markdown_inert_blocks(text)
        finally:
            check_documentation.HTML_COMMENT_OPEN_RE = original

        self.assertEqual(1, recording.calls)
        self.assertFalse(scan.unclosed_fence)
        self.assertTrue(scan.policy_prose.endswith("visible\n"))

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

    def test_repository_wide_documentation_bounds_fail_closed(self) -> None:
        cases = (
            (
                "filesystem discovery",
                "MAX_DOCUMENTATION_DISCOVERY_ENTRIES",
                1,
                "filesystem discovery exceeds",
            ),
            (
                "files",
                "MAX_MARKDOWN_FILES",
                14,
                "maintained Markdown file count exceeds",
            ),
            (
                "aggregate bytes",
                "MAX_MARKDOWN_TOTAL_BYTES",
                1,
                "aggregate Markdown input exceeds",
            ),
            (
                "expanded bytes",
                "MAX_EXPANDED_MARKDOWN_BYTES",
                1,
                "tab-expanded Markdown exceeds",
            ),
        )
        for name, constant, value, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self._write_minimal_repository(root)
                original = getattr(check_documentation, constant)
                setattr(check_documentation, constant, value)
                try:
                    errors, _ = check_documentation.validate_repository(root)
                finally:
                    setattr(check_documentation, constant, original)

                self.assertTrue(
                    any(expected in error for error in errors),
                    "\n".join(errors),
                )

    def test_ignored_trees_do_not_consume_discovery_budget(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            ignored = root / "target/generated"
            ignored.mkdir(parents=True)
            for index in range(128):
                (ignored / f"artifact-{index}.txt").write_text("x", encoding="utf-8")
            original = check_documentation.MAX_DOCUMENTATION_DISCOVERY_ENTRIES
            check_documentation.MAX_DOCUMENTATION_DISCOVERY_ENTRIES = 32
            try:
                errors, _ = check_documentation.validate_repository(root)
            finally:
                check_documentation.MAX_DOCUMENTATION_DISCOVERY_ENTRIES = original

            self.assertEqual([], errors)

    def test_aggregate_overflow_stops_reading_remaining_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = [root / f"{name}.md" for name in ("a", "b", "c")]
            for path in paths:
                path.write_text("xxxx", encoding="utf-8")
            original_limit = check_documentation.MAX_MARKDOWN_TOTAL_BYTES
            original_read = check_documentation._read
            calls: list[Path] = []

            def recording(
                path: Path,
                repository: Path,
                errors: list[str],
                byte_limit: int,
                *,
                aggregate_limit: bool,
            ) -> tuple[str, int] | None:
                calls.append(path)
                return original_read(
                    path,
                    repository,
                    errors,
                    byte_limit,
                    aggregate_limit=aggregate_limit,
                )

            check_documentation.MAX_MARKDOWN_TOTAL_BYTES = 4
            check_documentation._read = recording
            try:
                errors: list[str] = []
                context = check_documentation.ValidationContext(root, errors)
                for path in paths:
                    context.read(path)
            finally:
                check_documentation._read = original_read
                check_documentation.MAX_MARKDOWN_TOTAL_BYTES = original_limit

            self.assertEqual(paths[:2], calls)
            self.assertEqual(
                ["documentation: aggregate Markdown input exceeds the 4-byte ceiling"],
                errors,
            )

    def test_validation_scans_each_markdown_file_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            original = check_documentation._scan_markdown_inert_blocks
            calls = 0

            def recording(text: str) -> check_documentation.MarkdownScan:
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
        starts, ends, next_same, escaped = check_documentation._backtick_runs(
            "`x` " * 10_000
        )

        self.assertEqual(20_000, len(starts))
        self.assertEqual(len(starts), len(ends))
        self.assertEqual(len(starts), len(next_same))
        self.assertEqual(4, starts.itemsize)
        self.assertEqual(4, ends.itemsize)
        self.assertEqual(4, next_same.itemsize)
        self.assertEqual(bytes(20_000), escaped)

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
                "The plan owns the delivered-slice count. Status: accepted.\n\n"
                "The pinned baseline is "
                "[`b1774fbf6c7602b503026f96f6e960e946c692ef`]"
                "(https://github.com/vercel-labs/fx/commit/"
                "b1774fbf6c7602b503026f96f6e960e946c692ef).\n\n"
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
