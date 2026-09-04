from pathlib import Path
import os
import re
import subprocess
import tempfile
import textwrap
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
CI_ROUTE_INPUTS = (
    "DOCUMENTATION",
    "NON_DOCUMENTATION",
    "UNCLASSIFIED",
    "WORKSPACE_FALLBACK",
    "CI_INFRASTRUCTURE",
    "RUST_GLOBAL",
    "RUST_FORMAT",
    "CORE_ANY",
    "CORE_SOURCE",
    "TESTKIT_ANY",
    "TESTKIT_SOURCE",
    "NATIVE_ANY",
    "NATIVE_SOURCE",
    "CLI_ANY",
    "CLI_SOURCE",
    "TEST_SUPPORT",
    "BENCHMARK_TEST_INPUTS",
    "COMPATIBILITY_TEST_INPUTS",
    "CI_CLASSIFIER_TEST_INPUTS",
    "NATIVE_MANIFEST_TEST_INPUTS",
    "PROVISION_ZIG_TEST_INPUTS",
    "COMPATIBILITY_INPUTS",
    "DEPENDENCY_INPUTS",
    "DOCUMENTATION_POLICY",
    "CORE_API_DOCS",
    "TESTKIT_DOCS",
    "COMPATIBILITY_DOCS",
    "VISION_DOCS",
)
CI_ROUTE_OUTPUTS = (
    "documentation",
    "core",
    "testkit",
    "native",
    "cli",
    "format",
    "quality",
    "full_workspace",
    "benchmark_tests",
    "compatibility_tests",
    "ci_classifier_tests",
    "native_manifest_tests",
    "provision_zig_tests",
    "compatibility",
    "release_smoke",
    "dependency_audit",
    "native_matrix",
    "unsupported",
    "test_support",
    *FOCUSED_FILTERS,
)
BENCHMARK_ROUTE_INPUTS = (
    "DOCUMENTATION",
    "NON_DOCUMENTATION",
)


def job(workflow: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [a-z0-9_-]+:\n|\Z)",
        workflow,
    )
    if match is None:
        raise AssertionError(f"workflow has no {name!r} job")
    return match.group(0)


def step_script(workflow: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^      - name: {re.escape(name)}\n(?P<body>.*?)"
        r"(?=^      - (?:name:|uses:)|^  [a-z0-9_-]+:\n|\Z)",
        workflow,
    )
    if match is None:
        raise AssertionError(f"workflow has no {name!r} step")
    script = re.search(r"(?ms)^        run: \|\n(?P<body>.*)", match.group("body"))
    if script is None:
        raise AssertionError(f"workflow step {name!r} has no block script")
    return textwrap.dedent(script.group("body"))


def run_route(
    workflow: str,
    inputs: tuple[str, ...],
    values: dict[str, str],
) -> dict[str, str]:
    environment = os.environ.copy()
    environment.update({name: "false" for name in inputs})
    environment.update(values)
    with tempfile.NamedTemporaryFile() as output:
        environment["GITHUB_OUTPUT"] = output.name
        result = subprocess.run(
            ["bash", "-c", step_script(workflow, "Select the applicable gate")],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"route failed: stdout={result.stdout!r} stderr={result.stderr!r}"
            )
        output.seek(0)
        return dict(
            line.decode().rstrip("\n").split("=", 1)
            for line in output
            if b"=" in line
        )


def run_step_script(
    workflow: str,
    name: str,
    values: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment.update(values)
    with tempfile.NamedTemporaryFile() as summary:
        environment["GITHUB_STEP_SUMMARY"] = summary.name
        return subprocess.run(
            ["bash", "-c", step_script(workflow, name)],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )


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
                self.assertIn("token: ''", classifier)
                self.assertEqual(classifier.count("ref: ${{ github.sha }}"), 2)
                self.assertIn("Require visible submodule changes", classifier)
                self.assertIn(
                    "--get-regexp "
                    "'^(diff\\.ignoresubmodules|submodule\\..*\\.ignore)$'",
                    classifier,
                )
                self.assertIn('reject_hidden_gitlinks "effective Git config"', classifier)
                self.assertIn(
                    'reject_hidden_gitlinks ".gitmodules" --file .gitmodules',
                    classifier,
                )
                self.assertIn(".gitmodules must be a regular file", classifier)
                self.assertIn("predicate-quantifier: some-with-excludes", classifier)
                self.assertIn("fetch-depth: 0", classifier)
                self.assertIn("timeout-minutes: 5", classifier)
                self.assertNotIn("classify_ci_changes.py", classifier)
                self.assertNotIn("list-files:", classifier)

        self.assertNotIn("pull-requests:", self.ci)
        self.assertIn(
            "base: ${{ github.event_name == 'pull_request' && "
            "github.event.pull_request.base.sha || github.event.before || github.sha }}",
            job(self.ci, "change-classification"),
        )
        self.assertIn(
            "base: ${{ github.event.before || github.sha }}",
            job(self.benchmark, "change-classification"),
        )
        self.assertFalse((REPOSITORY_ROOT / "scripts/classify_ci_changes.py").exists())
        self.assertFalse((REPOSITORY_ROOT / "scripts/bounded_subprocess.py").exists())

    def test_filters_declare_one_documentation_boundary(self) -> None:
        expected = (
            "documentation:\n"
            "              - 'AGENTS.md'\n"
            "              - 'README.md'\n"
            "              - 'docs/**/*.md'\n"
            "            non_documentation:\n"
            "              - '**'\n"
            "              - '!AGENTS.md'\n"
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

        self.assertIn("            unclassified:\n              - '**'", classifier)
        for admitted in (
            "!.github/workflows/**",
            "!benchmarks/**",
            "!compatibility/**",
            "!crates/**",
            "!crates/machine-god-cli/**",
            "!crates/machine-god-core/**",
            "!crates/machine-god-native/**",
            "!crates/machine-god-testkit/**",
            "!test-support/**",
        ):
            self.assertIn(f"              - '{admitted}'", classifier)
        for unsafe_exclusion in (
            "!.github/**",
            "!**/*.py",
            "!scripts/**",
            "!tests/**",
        ):
            self.assertNotIn(
                f"              - '{unsafe_exclusion}'", classifier
            )
        self.assertNotIn(
            "              - '!scripts/generate_terminal_unicode_data.py'",
            classifier,
        )
        self.assertIn("            workspace_fallback:\n", classifier)
        for path in (
            "scripts/check_documentation.py",
            "scripts/generate_compatibility.py",
            "scripts/provision_zig.py",
            "tests/test_ci_change_classification.py",
            "tests/test_documentation_policy.py",
            "tests/test_native_manifest.py",
            "tests/test_provision_zig.py",
        ):
            self.assertIn(f"              - '!{path}'", classifier)
        self.assertIn(
            "            compatibility_inputs:\n"
            "              - 'compatibility/**'\n"
            "              - 'benchmarks/upstream.lock'\n"
            "              - 'scripts/generate_compatibility.py'",
            classifier,
        )

    def test_gitlink_guard_behaviorally_rejects_gitmodules_symlinks(self) -> None:
        environment = os.environ.copy()
        environment["GIT_CONFIG_GLOBAL"] = os.devnull
        environment["GIT_CONFIG_NOSYSTEM"] = "1"
        for workflow in (self.ci, self.benchmark):
            with self.subTest(workflow=workflow.splitlines()[0]):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    target = root / "gitmodules-target"
                    target.write_text(
                        '[submodule "fixture"]\n\tpath = fixture\n',
                        encoding="utf-8",
                    )
                    (root / ".gitmodules").symlink_to(target.name)
                    result = subprocess.run(
                        [
                            "bash",
                            "-c",
                            step_script(workflow, "Require visible submodule changes"),
                        ],
                        cwd=root,
                        env=environment,
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(".gitmodules must be a regular file", result.stdout)

    def test_ci_route_uses_dependency_closure_and_fails_closed(self) -> None:
        false = {name: "false" for name in CI_ROUTE_OUTPUTS}
        all_concerns = {
            name: "true"
            for name in CI_ROUTE_OUTPUTS
            if name not in FOCUSED_FILTERS
        }
        cases = (
            (
                "documentation only",
                {
                    "DOCUMENTATION": "true",
                    "DOCUMENTATION_POLICY": "true",
                },
                {**false, "documentation": "true"},
            ),
            (
                "core source reaches every dependent",
                {
                    "NON_DOCUMENTATION": "true",
                    "CORE_ANY": "true",
                    "CORE_SOURCE": "true",
                },
                {
                    **false,
                    "core": "true",
                    "testkit": "true",
                    "native": "true",
                    "cli": "true",
                    "quality": "true",
                    "release_smoke": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                },
            ),
            (
                "core test stays in its owner",
                {
                    "NON_DOCUMENTATION": "true",
                    "CORE_ANY": "true",
                },
                {
                    **false,
                    "core": "true",
                    "quality": "true",
                    "native_matrix": "true",
                },
            ),
            (
                "testkit source reaches native test consumers",
                {
                    "NON_DOCUMENTATION": "true",
                    "TESTKIT_ANY": "true",
                    "TESTKIT_SOURCE": "true",
                },
                {
                    **false,
                    "testkit": "true",
                    "native": "true",
                    "quality": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                },
            ),
            (
                "native source reaches cli",
                {
                    "NON_DOCUMENTATION": "true",
                    "NATIVE_ANY": "true",
                    "NATIVE_SOURCE": "true",
                },
                {
                    **false,
                    "native": "true",
                    "cli": "true",
                    "quality": "true",
                    "release_smoke": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                },
            ),
            (
                "cli test stays in its owner",
                {
                    "NON_DOCUMENTATION": "true",
                    "CLI_ANY": "true",
                },
                {
                    **false,
                    "cli": "true",
                    "quality": "true",
                    "native_matrix": "true",
                },
            ),
            (
                "standalone test support checks itself and consumers",
                {
                    "NON_DOCUMENTATION": "true",
                    "TEST_SUPPORT": "true",
                },
                {
                    **false,
                    "core": "true",
                    "native": "true",
                    "quality": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                    "test_support": "true",
                },
            ),
            (
                "lockfile reaches every package audit and smoke",
                {
                    "NON_DOCUMENTATION": "true",
                    "RUST_GLOBAL": "true",
                    "NATIVE_MANIFEST_TEST_INPUTS": "true",
                    "DEPENDENCY_INPUTS": "true",
                },
                {
                    **false,
                    "core": "true",
                    "testkit": "true",
                    "native": "true",
                    "cli": "true",
                    "quality": "true",
                    "full_workspace": "true",
                    "native_manifest_tests": "true",
                    "release_smoke": "true",
                    "dependency_audit": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                },
            ),
            (
                "dependency policy does not run product tests",
                {
                    "NON_DOCUMENTATION": "true",
                    "DEPENDENCY_INPUTS": "true",
                },
                {**false, "dependency_audit": "true"},
            ),
            (
                "format policy does not run package tests",
                {
                    "NON_DOCUMENTATION": "true",
                    "RUST_FORMAT": "true",
                },
                {
                    **false,
                    "format": "true",
                    "quality": "true",
                    "test_support": "true",
                },
            ),
            (
                "benchmark Python owns benchmark inputs",
                {
                    "NON_DOCUMENTATION": "true",
                    "BENCHMARK_TEST_INPUTS": "true",
                },
                {**false, "benchmark_tests": "true", "quality": "true"},
            ),
            (
                "compatibility Python and pinned agreement share inputs",
                {
                    "NON_DOCUMENTATION": "true",
                    "COMPATIBILITY_TEST_INPUTS": "true",
                    "COMPATIBILITY_INPUTS": "true",
                },
                {
                    **false,
                    "compatibility_tests": "true",
                    "compatibility": "true",
                    "quality": "true",
                },
            ),
            (
                "native manifest owns the release panic probe",
                {
                    "NON_DOCUMENTATION": "true",
                    "NATIVE_ANY": "true",
                    "NATIVE_MANIFEST_TEST_INPUTS": "true",
                },
                {
                    **false,
                    "native": "true",
                    "native_manifest_tests": "true",
                    "quality": "true",
                    "native_matrix": "true",
                    "unsupported": "true",
                },
            ),
            (
                "Zig provisioner owns its focused test",
                {
                    "NON_DOCUMENTATION": "true",
                    "PROVISION_ZIG_TEST_INPUTS": "true",
                },
                {**false, "provision_zig_tests": "true", "quality": "true"},
            ),
            (
                "vision docs install native manifest prerequisites",
                {
                    "DOCUMENTATION": "true",
                    "DOCUMENTATION_POLICY": "true",
                    "VISION_DOCS": "true",
                    "NATIVE_MANIFEST_TEST_INPUTS": "true",
                },
                {
                    **false,
                    "documentation": "true",
                    "vision_docs": "true",
                },
            ),
            (
                "mixed docs and cli test",
                {
                    "DOCUMENTATION": "true",
                    "DOCUMENTATION_POLICY": "true",
                    "NON_DOCUMENTATION": "true",
                    "CLI_ANY": "true",
                },
                {
                    **false,
                    "documentation": "true",
                    "cli": "true",
                    "quality": "true",
                    "native_matrix": "true",
                },
            ),
            (
                "workflow change",
                {
                    "NON_DOCUMENTATION": "true",
                    "CI_INFRASTRUCTURE": "true",
                },
                {**false, **all_concerns, **dict.fromkeys(FOCUSED_FILTERS, "true")},
            ),
            (
                "new crate uses actual workspace fallback",
                {"NON_DOCUMENTATION": "true", "WORKSPACE_FALLBACK": "true"},
                {**false, **all_concerns, **dict.fromkeys(FOCUSED_FILTERS, "true")},
            ),
        )
        for name, inputs, expected in cases:
            with self.subTest(name=name):
                self.assertEqual(
                    run_route(self.ci, CI_ROUTE_INPUTS, inputs), expected
                )

        for name, inputs in (
            ("unknown path", {"NON_DOCUMENTATION": "true", "UNCLASSIFIED": "true"}),
            (
                "known and unknown paths",
                {
                    "NON_DOCUMENTATION": "true",
                    "CLI_ANY": "true",
                    "UNCLASSIFIED": "true",
                },
            ),
            (
                "unknown Python or non-Python script/test fixture",
                {"NON_DOCUMENTATION": "true", "UNCLASSIFIED": "true"},
            ),
            (
                "terminal Unicode generator without a checked input",
                {"NON_DOCUMENTATION": "true", "UNCLASSIFIED": "true"},
            ),
        ):
            with self.subTest(name=name):
                environment = {key: "false" for key in CI_ROUTE_INPUTS}
                environment.update(inputs)
                result = run_step_script(
                    self.ci, "Select the applicable gate", environment
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unclassified", result.stderr)

        malformed = run_route(
            self.ci,
            CI_ROUTE_INPUTS,
            {"CORE_ANY": "not-a-boolean"},
        )
        self.assertTrue(
            all(malformed[name] == "true" for name in CI_ROUTE_OUTPUTS),
            malformed,
        )

    def test_ci_keeps_focused_docs_and_selects_independent_jobs(self) -> None:
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

        rust_install = re.search(
            r"(?ms)- name: Install exact Rust toolchain for focused contract checks"
            r"(?P<body>.*?)(?=      - name:)",
            documentation,
        )
        self.assertIsNotNone(rust_install)
        assert rust_install is not None
        self.assertIn("core_api_docs", rust_install.group("body"))
        self.assertIn("testkit_docs", rust_install.group("body"))
        self.assertIn("vision_docs", rust_install.group("body"))
        self.assertIn('cargo +"${RUST_TOOLCHAIN}" fetch --locked', documentation)

        conditions = {
            "quality": "quality",
            "security": "dependency_audit",
            "native-target-tests": "native_matrix",
            "unsupported-native-tools": "unsupported",
        }
        for heavy_job, output in conditions.items():
            self.assertIn(
                f"if: ${{{{ needs.change-classification.outputs.{output} == 'true' }}}}",
                job(self.ci, heavy_job),
            )

        gate = job(self.ci, "ci-gate")
        self.assertIn("name: CI gate", gate)
        self.assertIn("if: ${{ always() }}", gate)
        self.assertIn("CLASSIFICATION_RESULT", gate)
        self.assertIn("DOCUMENTATION_RESULT", gate)
        self.assertIn("require_result", gate)
        for output in conditions.values():
            self.assertIn(output.upper(), gate)

        quality = job(self.ci, "quality")
        documentation_tests = re.search(
            r"(?ms)- name: Documentation tests\n(?P<body>.*?)(?=      - name:)",
            quality,
        )
        self.assertIsNotNone(documentation_tests)
        assert documentation_tests is not None
        self.assertNotIn("machine-god-cli", documentation_tests.group("body"))
        self.assertIn("--workspace", documentation_tests.group("body"))
        self.assertIn("native_manifest_tests == 'true'", quality)
        self.assertIn('cargo +"${RUST_TOOLCHAIN}" fetch --locked', quality)
        helper_manifest = "test-support/reentrant-waker/Cargo.toml"
        self.assertEqual(quality.count(f"--manifest-path {helper_manifest}"), 3)
        self.assertIn("fmt --manifest-path", quality)
        self.assertIn("clippy --locked --manifest-path", quality)
        self.assertIn("test --locked --manifest-path", quality)

    def test_ci_gate_requires_each_selected_job_independently(self) -> None:
        base = {
            "CLASSIFICATION_RESULT": "success",
            "DOCUMENTATION_RESULT": "success",
            "QUALITY": "true",
            "DEPENDENCY_AUDIT": "false",
            "NATIVE_MATRIX": "true",
            "UNSUPPORTED": "false",
            "QUALITY_RESULT": "success",
            "SECURITY_RESULT": "skipped",
            "NATIVE_RESULT": "success",
            "FREEBSD_RESULT": "skipped",
        }
        accepted = run_step_script(
            self.ci, "Require the applicable checks", base
        )
        self.assertEqual(accepted.returncode, 0, accepted.stderr)

        for name, updates in (
            ("selected skipped", {"QUALITY_RESULT": "skipped"}),
            ("unselected ran", {"SECURITY_RESULT": "success"}),
            ("invalid selector", {"NATIVE_MATRIX": "invalid"}),
            ("classification failed", {"CLASSIFICATION_RESULT": "failure"}),
            ("documentation failed", {"DOCUMENTATION_RESULT": "failure"}),
        ):
            with self.subTest(name=name):
                result = run_step_script(
                    self.ci,
                    "Require the applicable checks",
                    {**base, **updates},
                )
                self.assertNotEqual(result.returncode, 0)

    def test_apple_native_tests_serialize_only_test_threads(self) -> None:
        native_tests = job(self.ci, "native-target-tests")
        apple_condition = "if: ${{ endsWith(matrix.target, '-apple-darwin') }}"
        non_apple_condition = (
            "if: ${{ !endsWith(matrix.target, '-apple-darwin') }}"
        )
        serial_suffix = '--target "${{ matrix.target }}" -- --test-threads=1'

        self.assertEqual(native_tests.count(apple_condition), 1)
        self.assertEqual(native_tests.count(non_apple_condition), 1)
        self.assertEqual(native_tests.count(serial_suffix), 1)
        self.assertIn(
            "Test Apple target natively without shared process-table contention",
            native_tests,
        )

        non_apple_step = re.search(
            r"(?ms)^      - name: Test target natively\n(?P<body>.*?)"
            r"(?=^      - name:|\Z)",
            native_tests,
        )
        self.assertIsNotNone(non_apple_step)
        assert non_apple_step is not None
        self.assertIn(non_apple_condition, non_apple_step.group("body"))
        self.assertNotIn("--test-threads=1", non_apple_step.group("body"))

    def test_release_smoke_canonicalizes_its_temporary_workspace(self) -> None:
        release_smoke = step_script(self.ci, "Release smoke test")
        self.assertIn(
            'runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve()',
            release_smoke,
        )
        self.assertEqual(release_smoke.count("dir=runner_temp"), 5)
        self.assertIn(
            'workspace_root = (workspace_smoke_root / "primary workspace").resolve()',
            release_smoke,
        )

    def test_release_smoke_covers_empty_background_list_in_isolated_state(self) -> None:
        release_smoke = step_script(self.ci, "Release smoke test")
        for fragment in (
            'background_workspace = (background_root / "workspace").resolve()',
            '"XDG_STATE_HOME": str(background_state_root)',
            '"HOME": str(background_home)',
            'background_machine = run_background("background", "--json")',
            "'{\"kind\":\"background\",\"count\":0,'",
            "path.exists()",
            "background_state_root",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, release_smoke)

    def test_benchmark_keeps_a_stable_affected_evidence_gate(self) -> None:
        for evidence_job in ("bootstrap-evidence", "pinned-upstream-evidence"):
            self.assertIn(
                "if: ${{ needs.change-classification.outputs.evidence == 'true' }}",
                job(self.benchmark, evidence_job),
            )

        classifier = job(self.benchmark, "change-classification")
        self.assertIn("github.event_name", classifier)
        self.assertIn("workflow_dispatch", classifier)

        gate = job(self.benchmark, "benchmark-gate")
        self.assertIn("name: Benchmark gate", gate)
        self.assertIn("if: ${{ always() }}", gate)
        self.assertIn("EVIDENCE", gate)
        self.assertIn('expected="skipped"', gate)
        self.assertIn('expected="success"', gate)
        self.assertIn("no artifacts were produced", gate)

        cases = (
            ("docs", {"DOCUMENTATION": "true"}, "false"),
            (
                "runtime source",
                {"NON_DOCUMENTATION": "true"},
                "true",
            ),
            (
                "test only retains exact-sha evidence",
                {"NON_DOCUMENTATION": "true"},
                "true",
            ),
            (
                "classifier",
                {"NON_DOCUMENTATION": "true"},
                "true",
            ),
            ("unknown", {"NON_DOCUMENTATION": "true"}, "true"),
            (
                "manual",
                {"EVENT_NAME": "workflow_dispatch"},
                "true",
            ),
            ("malformed", {"NON_DOCUMENTATION": "invalid"}, "true"),
        )
        for name, inputs, expected in cases:
            with self.subTest(name=name):
                values = {"EVENT_NAME": "push", **inputs}
                self.assertEqual(
                    run_route(
                        self.benchmark,
                        BENCHMARK_ROUTE_INPUTS,
                        values,
                    ),
                    {"evidence": expected},
                )

    def test_benchmark_gate_requires_evidence_exactly_when_selected(self) -> None:
        base = {
            "CLASSIFICATION_RESULT": "success",
            "EVIDENCE": "true",
            "BOOTSTRAP_RESULT": "success",
            "UPSTREAM_RESULT": "success",
        }
        accepted = run_step_script(
            self.benchmark, "Require the applicable benchmark evidence", base
        )
        self.assertEqual(accepted.returncode, 0, accepted.stderr)

        docs = run_step_script(
            self.benchmark,
            "Require the applicable benchmark evidence",
            {
                **base,
                "EVIDENCE": "false",
                "BOOTSTRAP_RESULT": "skipped",
                "UPSTREAM_RESULT": "skipped",
            },
        )
        self.assertEqual(docs.returncode, 0, docs.stderr)
        self.assertIn("Documentation-only change", docs.stdout)

        for name, updates in (
            ("selected skipped", {"UPSTREAM_RESULT": "skipped"}),
            ("invalid selector", {"EVIDENCE": "invalid"}),
            ("classification failed", {"CLASSIFICATION_RESULT": "failure"}),
        ):
            with self.subTest(name=name):
                result = run_step_script(
                    self.benchmark,
                    "Require the applicable benchmark evidence",
                    {**base, **updates},
                )
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
