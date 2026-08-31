from pathlib import Path
import re
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
BENCHMARK_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "bench.yml"
PATHS_FILTER_PIN = "ceb8a2b8f2d89434be7ff52d3de7ec3738c5cc9d"
FOCUSED_FILTERS = {
    "core_api_docs": "docs/core-api.md",
    "testkit_docs": "docs/testkit.md",
    "compatibility_docs": "docs/compatibility.md",
    "vision_docs": "docs/vision.md",
}


def job(workflow: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [a-z0-9_-]+:\n|\Z)",
        workflow,
    )
    if match is None:
        raise AssertionError(f"workflow has no {name!r} job")
    return match.group(0)


class CiChangeClassificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.ci = CI_WORKFLOW.read_text(encoding="utf-8")
        cls.benchmark = BENCHMARK_WORKFLOW.read_text(encoding="utf-8")

    def test_both_workflows_use_the_pinned_established_filter(self) -> None:
        for workflow in (self.ci, self.benchmark):
            with self.subTest(workflow=workflow.splitlines()[0]):
                classifier = job(workflow, "change-classification")
                self.assertEqual(
                    classifier.count(f"uses: dorny/paths-filter@{PATHS_FILTER_PIN}"),
                    1,
                )
                self.assertIn("id: paths", classifier)
                self.assertIn("base: ${{ github.ref }}", classifier)
                self.assertIn("predicate-quantifier: some-with-excludes", classifier)
                self.assertIn("fetch-depth: 0", classifier)
                self.assertIn("timeout-minutes: 5", classifier)
                self.assertNotIn("classify_ci_changes.py", classifier)
                self.assertNotIn("list-files:", classifier)

        self.assertIn("pull-requests: read", self.ci)
        self.assertFalse((REPOSITORY_ROOT / "scripts/classify_ci_changes.py").exists())
        self.assertFalse((REPOSITORY_ROOT / "scripts/bounded_subprocess.py").exists())

    def test_filters_declare_one_documentation_boundary(self) -> None:
        expected = (
            "documentation:\n"
            "              - 'README.md'\n"
            "              - 'docs/**/*.md'\n"
            "            non_documentation:\n"
            "              - '**'\n"
            "              - '!README.md'\n"
            "              - '!docs/**/*.md'"
        )
        for workflow in (self.ci, self.benchmark):
            classifier = job(workflow, "change-classification")
            self.assertIn(expected, classifier)

        classifier = job(self.ci, "change-classification")
        for output, path in FOCUSED_FILTERS.items():
            with self.subTest(output=output):
                self.assertIn(f"            {output}:\n", classifier)
                self.assertIn(f"              - '{path}'\n", classifier)

    def test_route_defaults_every_non_docs_only_case_to_full(self) -> None:
        for workflow in (self.ci, self.benchmark):
            classifier = job(workflow, "change-classification")
            self.assertIn("id: route", classifier)
            self.assertIn(
                "DOCUMENTATION: ${{ steps.paths.outputs.documentation }}",
                classifier,
            )
            self.assertIn(
                "NON_DOCUMENTATION: ${{ steps.paths.outputs.non_documentation }}",
                classifier,
            )
            self.assertIn("full=true", classifier)
            self.assertIn("docs_only=false", classifier)
            self.assertIn("true:false", classifier)
            self.assertIn('echo "full=${full}" >> "${GITHUB_OUTPUT}"', classifier)
            self.assertIn(
                'echo "docs_only=${docs_only}" >> "${GITHUB_OUTPUT}"',
                classifier,
            )
            self.assertIn("full: ${{ steps.route.outputs.full }}", classifier)
            self.assertIn(
                "docs_only: ${{ steps.route.outputs.docs_only }}", classifier
            )

        for output in FOCUSED_FILTERS:
            self.assertIn(
                f"{output}: ${{{{ steps.route.outputs.{output} }}}}",
                job(self.ci, "change-classification"),
            )

    def test_oversized_or_malformed_pull_request_fails_closed(self) -> None:
        classifier = job(self.ci, "change-classification")
        self.assertIn("EVENT_NAME: ${{ github.event_name }}", classifier)
        self.assertIn(
            "PULL_REQUEST_CHANGED_FILES: "
            "${{ github.event.pull_request.changed_files }}",
            classifier,
        )
        self.assertIn('if [[ "${EVENT_NAME}" == "pull_request" ]]', classifier)
        self.assertIn(
            '[[ "${PULL_REQUEST_CHANGED_FILES}" =~ ^[0-9]{1,4}$ ]]',
            classifier,
        )
        self.assertIn("10#${PULL_REQUEST_CHANGED_FILES} <= 3000", classifier)
        self.assertIn(
            '[[ "${valid}" == "true" && "${complete_file_list}" == "true" ]]',
            classifier,
        )

    def test_ci_keeps_focused_docs_light_and_mixed_changes_full(self) -> None:
        documentation = job(self.ci, "documentation-policy")
        self.assertIn("needs: change-classification", documentation)
        self.assertIn("if: ${{ always() }}", documentation)
        self.assertIn("python3 scripts/check_documentation.py", documentation)
        self.assertIn("tests/test_documentation_policy.py", documentation)

        for output in FOCUSED_FILTERS:
            with self.subTest(output=output):
                self.assertIn(
                    "needs.change-classification.outputs."
                    f"{output} == 'true'",
                    documentation,
                )

        for heavy_job in (
            "quality",
            "security",
            "native-target-tests",
            "unsupported-native-tools",
        ):
            self.assertIn(
                "if: ${{ needs.change-classification.outputs.full == 'true' }}",
                job(self.ci, heavy_job),
            )

        gate = job(self.ci, "ci-gate")
        self.assertIn("name: CI gate", gate)
        self.assertIn("if: ${{ always() }}", gate)
        self.assertIn("CLASSIFICATION_RESULT", gate)
        self.assertIn("DOCUMENTATION_RESULT", gate)
        self.assertIn("DOCS_ONLY", gate)
        self.assertIn('expected="skipped"', gate)
        self.assertIn('expected="success"', gate)

    def test_benchmark_keeps_a_stable_non_artifact_docs_gate(self) -> None:
        for evidence_job in ("bootstrap-evidence", "pinned-upstream-evidence"):
            self.assertIn(
                "if: ${{ needs.change-classification.outputs.full == 'true' }}",
                job(self.benchmark, evidence_job),
            )

        classifier = job(self.benchmark, "change-classification")
        self.assertIn("github.event_name", classifier)
        self.assertIn("workflow_dispatch", classifier)

        gate = job(self.benchmark, "benchmark-gate")
        self.assertIn("name: Benchmark gate", gate)
        self.assertIn("if: ${{ always() }}", gate)
        self.assertIn("DOCS_ONLY", gate)
        self.assertIn('expected="skipped"', gate)
        self.assertIn('expected="success"', gate)
        self.assertIn("no artifacts were produced", gate)


if __name__ == "__main__":
    unittest.main()
