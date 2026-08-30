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
            self.assertEqual(16, stats.markdown_files)
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

    def test_ci_change_policy_rejects_mutable_delivery_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_minimal_repository(root)
            relative = Path("docs/ci-change-classification.md")
            (root / relative).write_text(
                "# Durable policy\n\n"
                "Status: current delivery is complete.\n"
                "Workflow ID 12345678901.\n"
                "Delivered slices: 999\n",
                encoding="utf-8",
            )

            errors, _ = check_documentation.validate_repository(root)

            self.assertIn(
                f"{relative}: must not contain mutable top-level Status prose",
                errors,
            )
            self.assertIn(
                f"{relative}: must not contain GitHub Actions run IDs", errors
            )
            self.assertIn(
                f"{relative}: must not contain a delivered-count phrase", errors
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
