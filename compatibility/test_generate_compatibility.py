from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock


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

    def test_commit_replacement_cannot_change_pinned_snapshot_bytes(self) -> None:
        baseline = self.fixture_inventory()
        module = self.upstream / "sdk/fx-sdk.js"
        module.write_text(
            module.read_text(encoding="utf-8")
            + "\nexport const replacementCommitFake = true;\n",
            encoding="utf-8",
        )
        replacement = self.commit_changes("hostile replacement commit")
        self.git("checkout", "--quiet", "--detach", self.commit)
        self.git("replace", self.commit, replacement)

        ordinary_git = self.git("show", f"{self.commit}:sdk/fx-sdk.js").stdout
        self.assertIn(b"replacementCommitFake", ordinary_git)
        self.assertEqual(self.fixture_inventory(), baseline)

    def test_blob_replacement_cannot_change_pinned_snapshot_bytes(self) -> None:
        path = "sdk/node.js"
        expected = (self.upstream / path).read_bytes()
        original = self.git("rev-parse", f"{self.commit}:{path}").stdout.decode().strip()
        replacement = self.git(
            "hash-object",
            "-w",
            "--stdin",
            input_bytes=b"export const replacementBlobFake = true;\n",
        ).stdout.decode().strip()
        self.git("replace", original, replacement)

        self.assertIn(b"replacementBlobFake", self.git("cat-file", "blob", original).stdout)
        mode, object_id, actual = self.source().blob(path)
        self.assertEqual(mode, "100644")
        self.assertEqual(object_id, original)
        self.assertEqual(actual, expected)

    def test_blob_bytes_must_hash_to_recorded_object_id(self) -> None:
        source = self.source()
        canonical_git = source._git

        def tampered_git(*args: str) -> bytes:
            result = canonical_git(*args)
            if args[:2] == ("cat-file", "blob"):
                return result + b"hostile mutation"
            return result

        source._git = tampered_git
        with self.assertRaisesRegex(GENERATOR.InventoryError, "expected Git blob"):
            source.blob("sdk/node.js")

    def test_git_plumbing_strips_credentials_and_disables_transport(self) -> None:
        hostile_environment = {
            "ALL_PROXY": "http://proxy.invalid",
            "GH_TOKEN": "gh-secret",
            "GITHUB_TOKEN": "github-secret",
            "GIT_ASKPASS": "/hostile/askpass",
            "GIT_CONFIG_GLOBAL": "/hostile/gitconfig",
            "HTTP_PROXY": "http://proxy.invalid",
            "HTTPS_PROXY": "http://proxy.invalid",
            "SSH_ASKPASS": "/hostile/ssh-askpass",
            "SSH_AUTH_SOCK": "/hostile/agent.sock",
            "UPSTREAM_ACCESS_TOKEN": "upstream-secret",
        }
        with (
            mock.patch.dict(os.environ, hostile_environment, clear=False),
            mock.patch(
                "subprocess.run", wraps=subprocess.run
            ) as run,
        ):
            self.source().text("sdk/node.js")

        forbidden = set(hostile_environment) - {"GIT_CONFIG_GLOBAL"}
        for call in run.call_args_list:
            environment = call.kwargs["env"]
            self.assertTrue(forbidden.isdisjoint(environment))
            self.assertEqual(environment["GIT_CONFIG_GLOBAL"], os.devnull)
            self.assertEqual(environment["GIT_NO_LAZY_FETCH"], "1")
            self.assertEqual(environment["GIT_TERMINAL_PROMPT"], "0")
            command = call.args[0]
            self.assertIn("credential.helper=", command)
            self.assertIn("http.extraHeader=", command)
            self.assertIn("http.proxy=", command)
            self.assertIn("protocol.allow=never", command)

    def test_missing_promisor_blob_fails_without_lazy_fetch(self) -> None:
        remote = self.temporary / "remote.git"
        promisor = self.temporary / "promisor"
        subprocess.run(
            ["git", "clone", "--quiet", "--bare", str(self.upstream), str(remote)],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            ["git", "-C", str(remote), "config", "uploadpack.allowFilter", "true"],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            [
                "git",
                "clone",
                "--quiet",
                "--filter=blob:none",
                "--no-checkout",
                remote.as_uri(),
                str(promisor),
            ],
            check=True,
            capture_output=True,
        )
        object_id = self.git(
            "rev-parse", f"{self.commit}:sdk/node.js"
        ).stdout.decode().strip()
        missing = subprocess.run(
            [
                "git",
                "--no-lazy-fetch",
                "-C",
                str(promisor),
                "rev-list",
                "--objects",
                "--missing=print",
                self.commit,
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertIn(f"?{object_id}", missing)

        source = GENERATOR.GitSnapshot(promisor, self.commit)
        with self.assertRaisesRegex(GENERATOR.InventoryError, "cat-file blob"):
            source.blob("sdk/node.js")

        missing_after = subprocess.run(
            [
                "git",
                "--no-lazy-fetch",
                "-C",
                str(promisor),
                "rev-list",
                "--objects",
                "--missing=print",
                self.commit,
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertIn(f"?{object_id}", missing_after)

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

    def test_quoted_zig_identifier_cannot_supply_a_fake_registry(self) -> None:
        text = (
            'const @"pub const all = [_]tool_dispatch.Tool{ fake, }" = 1;\n'
            'pub const fake = ToolSpec{ .name = "fake" };\n'
        )
        with self.assertRaisesRegex(GENERATOR.InventoryError, "cannot find upstream"):
            GENERATOR.extract_builtin_tools(text)

    def test_same_line_zig_multiline_string_cannot_supply_a_fake_registry(self) -> None:
        text = (
            "const decoy = \\\\pub const all = [_]tool_dispatch.Tool{ fake, }\n"
            'pub const fake = ToolSpec{ .name = "fake" };\n'
        )
        with self.assertRaisesRegex(GENERATOR.InventoryError, "cannot find upstream"):
            GENERATOR.extract_builtin_tools(text)

    def test_nested_zig_enum_cannot_replace_top_level_contract(self) -> None:
        specs = (
            "pub const Namespace = struct {\n"
            "    pub const TopLevelKind = enum { help, };\n"
            "};\n"
        )
        registry = (
            "pub const top_level_specs = [_]TopLevelSpec{\n"
            '    .{ .kind = .help, .token = "help", .usage = "help", '
            '.summary = "Show help" },\n'
            "};\n"
        )
        with self.assertRaisesRegex(GENERATOR.InventoryError, "declared at top level"):
            GENERATOR.extract_top_level_commands(specs, registry)

    def test_nested_zig_command_registry_cannot_replace_top_level_registry(self) -> None:
        specs = "pub const TopLevelKind = enum { help, };\n"
        registry = (
            "pub const Namespace = struct {\n"
            "    pub const top_level_specs = [_]TopLevelSpec{\n"
            '        .{ .kind = .help, .token = "help", .usage = "help", '
            '.summary = "Show help" },\n'
            "    };\n"
            "};\n"
        )
        with self.assertRaisesRegex(GENERATOR.InventoryError, "declared at top level"):
            GENERATOR.extract_top_level_commands(specs, registry)

    def test_nested_zig_tool_registry_cannot_replace_top_level_registry(self) -> None:
        text = (
            "pub const Namespace = struct {\n"
            "    pub const all = [_]tool_dispatch.Tool{ fake, };\n"
            '    pub const fake = ToolSpec{ .name = "fake" };\n'
            "};\n"
        )
        with self.assertRaisesRegex(GENERATOR.InventoryError, "declared at top level"):
            GENERATOR.extract_builtin_tools(text)

    def test_quoted_zig_identifiers_remain_extractable(self) -> None:
        specs = 'pub const TopLevelKind = enum { @"help", };\n'
        registry = (
            "pub const top_level_specs = [_]TopLevelSpec{\n"
            '    .{ .@"kind" = .@"help", .@"token" = "help", '
            '.@"usage" = "help", .@"summary" = "Show help" },\n'
            "};\n"
        )
        commands = GENERATOR.extract_top_level_commands(specs, registry)
        self.assertEqual(commands[0]["kind"], "help")

    def test_large_tool_registry_masks_and_indexes_source_once(self) -> None:
        count = 500
        registry = ",\n".join(f"tool_{index}" for index in range(count)) + ","
        declarations = "\n".join(
            f'pub const tool_{index} = ToolSpec{{ .name = "tool_{index}" }};'
            for index in range(count)
        )
        text = (
            "pub const all = [_]tool_dispatch.Tool{\n"
            f"{registry}\n"
            "};\n"
            f"{declarations}\n"
        )
        with (
            mock.patch.object(
                GENERATOR, "source_mask", wraps=GENERATOR.source_mask
            ) as masks,
            mock.patch.object(
                GENERATOR,
                "structural_depth_map",
                wraps=GENERATOR.structural_depth_map,
            ) as depth_maps,
        ):
            tools = GENERATOR.extract_builtin_tools(text)
        self.assertEqual(len(tools), count)
        self.assertEqual(masks.call_count, 1)
        self.assertEqual(depth_maps.call_count, 1)

    def test_large_javascript_export_extraction_scales_with_input(self) -> None:
        def javascript_exports(count: int) -> str:
            return "\n".join(
                f"export const value{index} = {index};" for index in range(count)
            )

        def fastest_elapsed(text: str) -> float:
            elapsed = []
            for _ in range(3):
                started = time.perf_counter()
                GENERATOR.extract_js_exports(text)
                elapsed.append(time.perf_counter() - started)
            return min(elapsed)

        small = javascript_exports(500)
        large = javascript_exports(2_000)
        small_elapsed = fastest_elapsed(small)
        large_elapsed = fastest_elapsed(large)
        self.assertEqual(len(GENERATOR.extract_js_exports(large)), 2_000)
        self.assertLess(large_elapsed, 2.0)
        self.assertLess(large_elapsed, small_elapsed * 8 + 0.05)

    def test_unsupported_javascript_export_is_rejected(self) -> None:
        with self.assertRaisesRegex(GENERATOR.InventoryError, "export syntax"):
            GENERATOR.extract_js_exports('export * from "./other.js";\n')

    def test_javascript_multi_declarators_are_all_exported(self) -> None:
        exports = GENERATOR.extract_js_exports(
            "export const first = 1, second = call(1, 2), third = { value: 3 };\n"
            "export let fourth, fifth = 5;\n"
        )
        self.assertEqual(exports, ["first", "second", "third", "fourth", "fifth"])

    def test_javascript_regex_literals_cannot_inject_exports(self) -> None:
        exports = GENERATOR.extract_js_exports(
            "const decoy = /export const regexFake = true/gi;\n"
            "const ratio = total / divisor;\n"
            "if (ready) /export function controlFake() {}/.test(value);\n"
            "export const real = total / divisor, pattern = /a[/]b/gi;\n"
        )
        self.assertEqual(exports, ["real", "pattern"])

    def test_ambiguous_javascript_regex_after_brace_fails_closed(self) -> None:
        with self.assertRaisesRegex(GENERATOR.InventoryError, "ambiguous JavaScript slash"):
            GENERATOR.extract_js_exports(
                "if (ready) { work(); }\n"
                "/export const blockFake = true/.test(value);\n"
                "export const real = 1;\n"
            )

    def test_known_optional_fields_reject_dynamic_values_and_duplicates(self) -> None:
        specs = (
            FIXTURE / "src/core/slash_commands/command_specs.zig"
        ).read_text(encoding="utf-8")
        commands = (FIXTURE / "src/builtins/commands.zig").read_text(encoding="utf-8")
        cases = [
            (
                "dynamic aliases",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    '.aliases = &.{ "--help", "-h" },',
                    ".aliases = buildAliases(),",
                ),
                r"unsupported \.aliases",
            ),
            (
                "duplicate aliases",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    '.aliases = &.{ "--help", "-h" },',
                    '.aliases = &.{ "--help", "-h" }, .aliases = &.{},',
                ),
                r"repeats \.aliases",
            ),
            (
                "dynamic quoted aliases",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    '.aliases = &.{ "--help", "-h" },',
                    '.@"aliases" = buildAliases(),',
                ),
                r"unsupported \.aliases",
            ),
            (
                "mixed duplicate aliases",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    '.aliases = &.{ "--help", "-h" },',
                    '.aliases = &.{ "--help", "-h" }, .@"aliases" = &.{},',
                ),
                r"repeats \.aliases",
            ),
            (
                "dynamic hidden flag",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    ".hidden_from_top_level_help = true,",
                    ".hidden_from_top_level_help = isHidden(),",
                ),
                r"unsupported \.hidden_from_top_level_help",
            ),
            (
                "duplicate hidden flag",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    ".hidden_from_top_level_help = true,",
                    ".hidden_from_top_level_help = true, "
                    ".hidden_from_top_level_help = false,",
                ),
                r"repeats \.hidden_from_top_level_help",
            ),
            (
                "dynamic quoted hidden flag",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    ".hidden_from_top_level_help = true,",
                    '.@"hidden_from_top_level_help" = isHidden(),',
                ),
                r"unsupported \.hidden_from_top_level_help",
            ),
            (
                "mixed duplicate hidden flag",
                GENERATOR.extract_top_level_commands,
                commands.replace(
                    ".hidden_from_top_level_help = true,",
                    '.hidden_from_top_level_help = true, '
                    '.@"hidden_from_top_level_help" = false,',
                ),
                r"repeats \.hidden_from_top_level_help",
            ),
            (
                "dynamic presentation category",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general",
                    ".presentation_category = chooseCategory()",
                    1,
                ),
                r"unsupported \.presentation_category",
            ),
            (
                "duplicate presentation category",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general",
                    ".presentation_category = .general, "
                    ".presentation_category = .advanced",
                    1,
                ),
                r"repeats \.presentation_category",
            ),
            (
                "dynamic quoted presentation category",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general",
                    '.@"presentation_category" = chooseCategory()',
                    1,
                ),
                r"unsupported \.presentation_category",
            ),
            (
                "mixed duplicate presentation category",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general",
                    '.presentation_category = .general, '
                    '.@"presentation_category" = .advanced',
                    1,
                ),
                r"repeats \.presentation_category",
            ),
            (
                "dynamic quoted argument flag",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general }",
                    '.presentation_category = .general, '
                    '.@"has_args" = determineArgs() }',
                    1,
                ),
                r"unsupported \.has_args",
            ),
            (
                "mixed duplicate argument flag",
                GENERATOR.extract_slash_commands,
                commands.replace(
                    ".presentation_category = .general }",
                    ".presentation_category = .general, .has_args = true, "
                    '.@"has_args" = false }',
                    1,
                ),
                r"repeats \.has_args",
            ),
        ]
        for label, extractor, mutated, error in cases:
            with self.subTest(label=label), self.assertRaisesRegex(
                GENERATOR.InventoryError, error
            ):
                extractor(specs, mutated)

    def test_quoted_and_unquoted_known_fields_share_duplicate_detection(self) -> None:
        known_fields = (
            "kind",
            "token",
            "aliases",
            "usage",
            "summary",
            "hidden_from_top_level_help",
            "command",
            "presentation_category",
            "has_args",
            "name",
        )
        for field in known_fields:
            duplicate_mask = f'.{field} = true, .@"{field}" = false,'
            duplicate_depths = GENERATOR.structural_depth_map(duplicate_mask)
            with self.subTest(field=field), self.assertRaisesRegex(
                GENERATOR.InventoryError, rf"repeats \.{field}"
            ):
                GENERATOR.field_assignment(duplicate_mask, duplicate_depths, field)

            dynamic_mask = f'.@"{field}" = dynamicValue(),'
            dynamic_depths = GENERATOR.structural_depth_map(dynamic_mask)
            with self.subTest(field=field), self.assertRaisesRegex(
                GENERATOR.InventoryError, rf"unsupported \.{field}"
            ):
                GENERATOR.field_match(
                    dynamic_mask, dynamic_depths, field, r"true|false"
                )

            escaped_field = f"{field[:-1]}\\x{ord(field[-1]):02x}"
            for expression in (
                f'.@"{escaped_field}" = dynamicValue(),',
                f'.{field} = true, .@"{escaped_field}" = false,',
            ):
                with self.subTest(
                    field=field, expression=expression
                ), self.assertRaisesRegex(
                    GENERATOR.InventoryError, "escape-bearing quoted Zig identifier"
                ):
                    GENERATOR.source_mask(expression, "zig")

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
