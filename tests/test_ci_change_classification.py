import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
CLASSIFIER = REPOSITORY_ROOT / "scripts" / "classify_ci_changes.py"
ZERO_SHA = "0" * 40


class GitRepository:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.git("init", "-b", "main")
        self.git("config", "user.name", "CI Classifier Test")
        self.git("config", "user.email", "ci-classifier@example.invalid")

    def git(self, *arguments: str) -> str:
        return subprocess.run(
            ["git", *arguments],
            cwd=self.path,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        ).stdout.strip()

    def write(self, relative: str, contents: str) -> None:
        destination = self.path / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(contents, encoding="utf-8")

    def commit(self, message: str) -> str:
        self.git("add", "-A")
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD")


class CiChangeClassificationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.repository_path = Path(self.temporary_directory.name) / "repository"
        self.repository_path.mkdir()
        self.repository = GitRepository(self.repository_path)
        self.repository.write("src/lib.rs", "pub fn baseline() {}\n")
        self.repository.write("README.md", "# Project\n")
        self.initial = self.repository.commit("initial")

    def run_classifier(
        self,
        event: str = "push",
        before: str | None = None,
        head: str | None = None,
        base: str | None = None,
        default_ref: str | None = None,
        cwd: Path | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        output_file = Path(self.temporary_directory.name) / "github-output"
        if output_file.exists():
            output_file.unlink()
        arguments = [
            sys.executable,
            str(CLASSIFIER),
            "--event",
            event,
            "--output",
            str(output_file),
        ]
        for flag, value in (
            ("--before", before),
            ("--head", head),
            ("--base", base),
            ("--default-ref", default_ref),
        ):
            if value is not None:
                arguments.extend((flag, value))
        completed = subprocess.run(
            arguments,
            cwd=cwd or self.repository_path,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        output = output_file.read_text(encoding="utf-8") if output_file.exists() else ""
        return completed, output

    def assert_result(self, output: str, *, full: bool, docs_only: bool) -> None:
        self.assertEqual(
            output,
            f"full={'true' if full else 'false'}\n"
            f"docs_only={'true' if docs_only else 'false'}\n",
        )

    def test_docs_only_push_uses_cheap_gate(self) -> None:
        self.repository.write("README.md", "# Updated\n")
        self.repository.write("docs/guide/setup.md", "# Setup\n")
        head = self.repository.commit("docs")
        completed, output = self.run_classifier(before=self.initial, head=head)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=False, docs_only=True)
        self.assertIn("only cheap documentation (2 path(s))", completed.stdout)

    def test_contract_document_exceptions_use_full_gate(self) -> None:
        for relative in (
            "docs/core-api.md",
            "docs/testkit.md",
            "docs/compatibility.md",
        ):
            with self.subTest(relative=relative):
                self.repository.git("reset", "--hard", self.initial)
                self.repository.write(relative, "changed\n")
                head = self.repository.commit(f"change {relative}")
                completed, output = self.run_classifier(before=self.initial, head=head)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assert_result(output, full=True, docs_only=False)

    def test_mixed_and_unknown_paths_use_full_gate(self) -> None:
        cases = (
            ("mixed", ("docs/guide.md", "crates/core/src/lib.rs")),
            ("unknown markdown", ("CONTRIBUTING.md",)),
            ("non-markdown docs", ("docs/schema.json",)),
        )
        for name, paths in cases:
            with self.subTest(name=name):
                self.repository.git("reset", "--hard", self.initial)
                for relative in paths:
                    self.repository.write(relative, "changed\n")
                head = self.repository.commit(name)
                completed, output = self.run_classifier(before=self.initial, head=head)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assert_result(output, full=True, docs_only=False)
                self.assertIn("full-gate path", completed.stdout)

    def test_normal_push_requires_before_to_be_an_ancestor(self) -> None:
        self.repository.write("docs/normal.md", "normal\n")
        head = self.repository.commit("normal push")
        completed, output = self.run_classifier(before=self.initial, head=head)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=False, docs_only=True)
        self.assertIn("normal push range", completed.stdout)

    def test_new_branch_uses_default_ref_merge_base(self) -> None:
        self.repository.git("update-ref", "refs/remotes/origin/main", self.initial)
        self.repository.git("switch", "-c", "feature")
        self.repository.write("docs/new-branch.md", "new branch\n")
        head = self.repository.commit("new branch docs")
        completed, output = self.run_classifier(
            before=ZERO_SHA,
            head=head,
            default_ref="refs/remotes/origin/main",
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=False, docs_only=True)
        self.assertIn("new branch from default-ref merge base", completed.stdout)

    def test_pull_request_uses_merge_base_not_base_tip(self) -> None:
        self.repository.git("switch", "-c", "feature")
        self.repository.write("docs/pr.md", "pull request\n")
        head = self.repository.commit("feature docs")
        self.repository.git("switch", "main")
        self.repository.write("src/main.rs", "fn main() {}\n")
        base = self.repository.commit("main advances")
        completed, output = self.run_classifier(
            event="pull_request", base=base, head=head
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=False, docs_only=True)
        self.assertIn("pull-request merge-base range", completed.stdout)

    def test_force_unreachable_and_git_failure_fail_closed(self) -> None:
        self.repository.git("switch", "-c", "other", self.initial)
        self.repository.write("docs/other.md", "other\n")
        unrelated = self.repository.commit("unrelated")
        self.repository.git("switch", "main")
        self.repository.write("docs/head.md", "head\n")
        head = self.repository.commit("head")

        cases = (
            ("force", unrelated, self.repository_path),
            ("unreachable", "f" * 40, self.repository_path),
            ("git failure", self.initial, Path(self.temporary_directory.name)),
        )
        for name, before, cwd in cases:
            with self.subTest(name=name):
                completed, output = self.run_classifier(before=before, head=head, cwd=cwd)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assert_result(output, full=True, docs_only=False)
                self.assertIn("uncertain change set", completed.stdout)

    def test_rename_from_code_to_docs_cannot_hide_code_change(self) -> None:
        (self.repository_path / "docs").mkdir()
        self.repository.git("mv", "src/lib.rs", "docs/moved.md")
        head = self.repository.commit("move code into docs")
        completed, output = self.run_classifier(before=self.initial, head=head)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=True, docs_only=False)
        self.assertIn("'src/lib.rs'", completed.stdout)

    def test_empty_diff_fails_closed(self) -> None:
        completed, output = self.run_classifier(before=self.initial, head=self.initial)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assert_result(output, full=True, docs_only=False)
        self.assertIn("empty normal push range", completed.stdout)

    def test_dispatch_unknown_event_and_missing_inputs_run_full_gate(self) -> None:
        for event, before, head in (
            ("workflow_dispatch", None, None),
            ("schedule", None, None),
            ("push", None, self.initial),
        ):
            with self.subTest(event=event):
                completed, output = self.run_classifier(event=event, before=before, head=head)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assert_result(output, full=True, docs_only=False)

    def test_output_format_is_exact_and_environment_output_is_supported(self) -> None:
        self.repository.write("docs/output.md", "output\n")
        head = self.repository.commit("output")
        output_file = Path(self.temporary_directory.name) / "environment-output"
        environment = os.environ.copy()
        environment["GITHUB_OUTPUT"] = str(output_file)
        completed = subprocess.run(
            [
                sys.executable,
                str(CLASSIFIER),
                "--event",
                "push",
                "--before",
                self.initial,
                "--head",
                head,
            ],
            cwd=self.repository_path,
            env=environment,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            completed.stdout,
            "CI change classification: full=false docs_only=true; "
            "normal push range contains only cheap documentation (1 path(s))\n",
        )
        self.assertEqual(output_file.read_text(encoding="utf-8"), "full=false\ndocs_only=true\n")


if __name__ == "__main__":
    unittest.main()
