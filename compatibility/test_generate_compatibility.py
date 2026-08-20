from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/generate_compatibility.py"
FIXTURE = ROOT / "compatibility/fixtures/upstream"
UNSUPPORTED_TOOL_FIXTURE = ROOT / "compatibility/fixtures/unsupported_tool_expression.zig"
POLICY = ROOT / "compatibility/policy.json"

SPEC = importlib.util.spec_from_file_location("generate_compatibility", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import {SCRIPT}")
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


class CompatibilityGeneratorTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.temporary = Path(self.temporary_directory.name)
        self.upstream = self.temporary / "upstream"
        shutil.copytree(FIXTURE, self.upstream)
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "fixture")
        self.commit = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.lock = {
            "repository": "https://github.com/vercel-labs/fx.git",
            "commit": self.commit,
        }

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def source(self, commit: str | None = None) -> object:
        return GENERATOR.GitSnapshot(self.upstream, commit or self.commit)

    def fixture_inventory(self, source: object | None = None) -> dict[str, object]:
        return GENERATOR.build_inventory(
            source or self.source(),
            self.lock,
            GENERATOR.load_policy(POLICY),
            lock_path="fixture.lock",
        )

    def commit_changes(self, message: str) -> str:
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD").stdout.decode().strip()

    def test_fixture_extracts_all_inventory_surfaces_comment_aware(self) -> None:
        inventory = self.fixture_inventory()
        surfaces = inventory["surfaces"]

        self.assertEqual(
            [item["token"] for item in surfaces["top_level_cli_commands"]["items"]],
            ["help", "ask"],
        )
        self.assertEqual(
            {item["kind"] for item in surfaces["slash_command_kinds"]["items"]},
            {"help", "quit"},
        )
        self.assertEqual(
            [item["name"] for item in surfaces["builtin_tool_names"]["items"]],
            ["read_file", "terminal"],
        )
        modules = {
            module["path"]: module["exports"]
            for module in surfaces["sdk_exports"]["modules"]
        }
        self.assertEqual(
            modules["sdk/fx-sdk.js"],
            ["fxSdkApiVersion", "supportsJspi", "createFxAgent"],
        )
        self.assertEqual(
            modules["sdk/node.js"],
            ["supportsJspi", "publicValue", "libfxApiVersion", "createFxAgent"],
        )
        flattened_exports = {name for exports in modules.values() for name in exports}
        self.assertNotIn("commentedFake", flattened_exports)
        self.assertNotIn("stringFake", flattened_exports)
        self.assertNotIn("templateFake", flattened_exports)
        self.assertEqual(
            surfaces["e2e_owners"]["classification_counts"],
            {"training": 1, "verification_only": 1, "intentional_exclusion": 1},
        )
        self.assertIn("fixture.lock", GENERATOR.render_docs(inventory))

    def test_canonical_commit_bytes_ignore_crlf_worktree_and_head_races(self) -> None:
        baseline = self.fixture_inventory()
        source = self.source()
        commands = self.upstream / "src/builtins/commands.zig"
        commands.write_bytes(commands.read_bytes().replace(b"\n", b"\r\n"))
        tools = self.upstream / "src/builtins/tools.zig"
        tools.write_text("not valid Zig\n", encoding="utf-8")
        (self.upstream / "tests/e2e/orphan.test.ts").write_text(
            "// unclassified worktree file\n", encoding="utf-8"
        )
        raced_head = self.commit_changes("mutate checkout after snapshot selection")
        self.assertNotEqual(raced_head, self.commit)

        after_race = self.fixture_inventory(source)
        self.assertEqual(after_race, baseline)
        commands_source = next(
            item
            for item in baseline["upstream"]["source_files"]
            if item["path"] == "src/builtins/commands.zig"
        )
        canonical = self.git(
            "cat-file", "blob", f"{self.commit}:src/builtins/commands.zig"
        ).stdout
        self.assertEqual(commands_source["sha256"], hashlib.sha256(canonical).hexdigest())
        self.assertNotEqual(
            commands_source["sha256"], hashlib.sha256(commands.read_bytes()).hexdigest()
        )

    def test_expected_source_symlink_mode_is_rejected(self) -> None:
        target = self.git("hash-object", "-w", "--stdin", input_bytes=b"fx-sdk.js").stdout
        object_id = target.decode().strip()
        self.git(
            "update-index",
            "--add",
            "--cacheinfo",
            f"120000,{object_id},sdk/node.js",
        )
        self.git("commit", "-q", "-m", "replace source with symlink")
        commit = self.git("rev-parse", "HEAD").stdout.decode().strip()
        source = self.source(commit)
        with self.assertRaisesRegex(GENERATOR.InventoryError, "regular blob"):
            source.text("sdk/node.js")

    def test_unsupported_tool_registry_expression_is_rejected(self) -> None:
        text = UNSUPPORTED_TOOL_FIXTURE.read_text(encoding="utf-8")
        with self.assertRaisesRegex(GENERATOR.InventoryError, "registry expression"):
            GENERATOR.extract_builtin_tools(text)

    def test_unsupported_javascript_export_is_rejected(self) -> None:
        with self.assertRaisesRegex(GENERATOR.InventoryError, "export syntax"):
            GENERATOR.extract_js_exports('export * from "./other.js";\n')

    def test_e2e_owner_coverage_rejects_an_unclassified_file(self) -> None:
        (self.upstream / "tests/e2e/orphan.test.ts").write_text(
            "// unclassified\n", encoding="utf-8"
        )
        commit = self.commit_changes("add orphan owner")
        with self.assertRaisesRegex(GENERATOR.InventoryError, "unclassified"):
            GENERATOR.extract_e2e_owners(self.source(commit))

    def test_e2e_owner_coverage_rejects_duplicate_classification(self) -> None:
        corpus_path = self.upstream / "scripts/pgso/corpus.json"
        corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
        corpus["verification_scenarios"].append(
            {
                "name": "verify-cli-again",
                "argv": ["bun", "test", "./cli.test.ts"],
                "test_file": "cli.test.ts",
            }
        )
        corpus_path.write_text(json.dumps(corpus), encoding="utf-8")
        commit = self.commit_changes("duplicate owner")
        with self.assertRaisesRegex(GENERATOR.InventoryError, "multiple PGSO classifications"):
            GENERATOR.extract_e2e_owners(self.source(commit))

    def test_markdown_helpers_escape_dynamic_text_and_code(self) -> None:
        escaped_text = GENERATOR.markdown_text("[link](https://invalid)|<tag>*")
        self.assertNotIn("[link]", escaped_text)
        self.assertNotIn("<tag>", escaped_text)
        self.assertIn("\\|", escaped_text)
        escaped_code = GENERATOR.markdown_code("value`|tail")
        self.assertTrue(escaped_code.startswith("``"))
        self.assertIn("&#124;", escaped_code)

    def test_cli_drift_check_uses_network_free_canonical_fixture(self) -> None:
        lock = self.temporary / "upstream.lock"
        lock.write_text(
            "repository=https://github.com/vercel-labs/fx.git\n"
            f"commit={self.commit}\n",
            encoding="utf-8",
        )
        inventory = self.temporary / "inventory.json"
        docs = self.temporary / "compatibility.md"
        base_command = [
            sys.executable,
            str(SCRIPT),
            "--upstream",
            str(self.upstream),
            "--lock",
            str(lock),
            "--policy",
            str(POLICY),
            "--inventory",
            str(inventory),
            "--docs",
            str(docs),
        ]

        generated = subprocess.run(base_command, check=False, capture_output=True, text=True)
        self.assertEqual(generated.returncode, 0, generated.stderr)
        clean = subprocess.run(
            [*base_command, "--check"], check=False, capture_output=True, text=True
        )
        self.assertEqual(clean.returncode, 0, clean.stderr)

        docs.write_text(docs.read_text(encoding="utf-8") + "drift\n", encoding="utf-8")
        drifted = subprocess.run(
            [*base_command, "--check"], check=False, capture_output=True, text=True
        )
        self.assertEqual(drifted.returncode, 1)
        self.assertIn("stale", drifted.stderr)

        commands = self.upstream / "src/builtins/commands.zig"
        commands.write_bytes(commands.read_bytes().replace(b"\n", b"\r\n"))
        canonical = subprocess.run(
            base_command, check=False, capture_output=True, text=True
        )
        self.assertEqual(canonical.returncode, 0, canonical.stderr)

    def git(
        self, *args: str, input_bytes: bytes | None = None
    ) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            ["git", "-C", str(self.upstream), *args],
            input=input_bytes,
            check=True,
            capture_output=True,
        )


if __name__ == "__main__":
    unittest.main()
