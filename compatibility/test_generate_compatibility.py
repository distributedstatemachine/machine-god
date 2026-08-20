from __future__ import annotations

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
POLICY = ROOT / "compatibility/policy.json"

SPEC = importlib.util.spec_from_file_location("generate_compatibility", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import {SCRIPT}")
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


class CompatibilityGeneratorTest(unittest.TestCase):
    def fixture_inventory(self) -> dict[str, object]:
        lock = {
            "repository": "https://github.com/vercel-labs/fx.git",
            "commit": "1" * 40,
        }
        return GENERATOR.build_inventory(
            FIXTURE,
            lock,
            GENERATOR.load_policy(POLICY),
            lock_path="fixture.lock",
        )

    def test_fixture_extracts_all_inventory_surfaces(self) -> None:
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
            surfaces["e2e_owners"]["classification_counts"],
            {"training": 1, "verification_only": 1, "intentional_exclusion": 1},
        )
        self.assertIn("fixture.lock", GENERATOR.render_docs(inventory))

    def test_e2e_owner_coverage_rejects_an_unclassified_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory) / "upstream"
            shutil.copytree(FIXTURE, upstream)
            (upstream / "tests/e2e/orphan.test.ts").write_text("// unclassified\n", encoding="utf-8")
            with self.assertRaisesRegex(GENERATOR.InventoryError, "unclassified"):
                GENERATOR.extract_e2e_owners(upstream)

    def test_e2e_owner_coverage_rejects_duplicate_classification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory) / "upstream"
            shutil.copytree(FIXTURE, upstream)
            corpus_path = upstream / "scripts/pgso/corpus.json"
            corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
            corpus["verification_scenarios"].append(
                {
                    "name": "verify-cli-again",
                    "argv": ["bun", "test", "./cli.test.ts"],
                    "test_file": "cli.test.ts",
                }
            )
            corpus_path.write_text(json.dumps(corpus), encoding="utf-8")
            with self.assertRaisesRegex(GENERATOR.InventoryError, "multiple PGSO classifications"):
                GENERATOR.extract_e2e_owners(upstream)

    def test_cli_drift_check_uses_a_network_free_git_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            upstream = temporary / "upstream"
            shutil.copytree(FIXTURE, upstream)
            self.git(upstream, "init", "-q")
            self.git(upstream, "config", "user.name", "Fixture")
            self.git(upstream, "config", "user.email", "fixture@example.invalid")
            self.git(upstream, "add", ".")
            self.git(upstream, "commit", "-q", "-m", "fixture")
            commit = self.git(upstream, "rev-parse", "HEAD").stdout.strip()
            lock = temporary / "upstream.lock"
            lock.write_text(
                "repository=https://github.com/vercel-labs/fx.git\n" f"commit={commit}\n",
                encoding="utf-8",
            )
            inventory = temporary / "inventory.json"
            docs = temporary / "compatibility.md"
            base_command = [
                sys.executable,
                str(SCRIPT),
                "--upstream",
                str(upstream),
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

            commands = upstream / "src/builtins/commands.zig"
            commands.write_text(commands.read_text(encoding="utf-8") + "// dirty\n", encoding="utf-8")
            dirty = subprocess.run(
                [*base_command, "--check"], check=False, capture_output=True, text=True
            )
            self.assertEqual(dirty.returncode, 2)
            self.assertIn("local changes", dirty.stderr)

    @staticmethod
    def git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(root), *args],
            check=True,
            capture_output=True,
            text=True,
        )


if __name__ == "__main__":
    unittest.main()
