from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from scripts.generate_terminal_unicode_data import (
    ARRAYS,
    SUPPORTED_UPSTREAM_COMMIT,
    extract_array,
    render,
    validate_upstream_commit,
)


ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "scripts" / "generate_terminal_unicode_data.py"


def fixture() -> str:
    entries = {
        "Range": ".{ .first = 0x1100, .last = 0x115F },",
        "TrieNode": ".{ .edge_start = 0, .edge_len = 1, .terminal = false },",
        "TrieEdge": ".{ .codepoint = 0x1F600, .child = 1 },",
    }
    return "//! Unicode fixture provenance\n//! sha256: fixture\n\n" + "\n".join(
        f"pub const {name} = [_]{kind}{{\n    {entries[kind]}\n}};"
        for name, kind in ARRAYS
    )


class TerminalUnicodeGeneratorTests(unittest.TestCase):
    def test_all_tables_transliterate_deterministically(self) -> None:
        generated = render(fixture())
        self.assertEqual(generated, render(fixture()))
        self.assertIn("// Unicode fixture provenance\n// sha256: fixture", generated)
        self.assertIn(f"// {SUPPORTED_UPSTREAM_COMMIT}:", generated)
        for name, kind in ARRAYS:
            self.assertIn(f"static {name.upper()}: [{kind}; 1]", generated)
        self.assertIn("Range { first: 0x1100, last: 0x115F },", generated)
        self.assertIn("TrieNode { edge_start: 0, edge_len: 1, terminal: false },", generated)
        self.assertIn("TrieEdge { codepoint: 0x1F600, child: 1 },", generated)
        self.assertIn("MAX_RGI_SEQUENCE_CODEPOINTS: usize = 10;", generated)
        self.assertEqual(generated.count("#[rustfmt::skip]"), len(ARRAYS))
        self.assertTrue(generated.endswith("\n"))

    def test_upstream_revision_must_match_supported_generator_provenance(self) -> None:
        validate_upstream_commit(SUPPORTED_UPSTREAM_COMMIT)
        for commit in ("f" * 40, "", SUPPORTED_UPSTREAM_COMMIT.upper()):
            with self.subTest(commit=commit), self.assertRaisesRegex(
                ValueError, "terminal Unicode generator supports"
            ):
                validate_upstream_commit(commit)

    def test_missing_array_is_rejected(self) -> None:
        for name, _ in ARRAYS:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, name):
                render(fixture().replace(f"pub const {name} =", f"pub const absent_{name} ="))

    def test_multiple_entries_and_empty_array_have_exact_lengths(self) -> None:
        source = fixture()
        body = extract_array(source, "wide_ranges")
        source = source.replace(body, body + "\n" + body, 1)
        self.assertIn("static WIDE_RANGES: [Range; 2]", render(source))
        source = fixture().replace(body, "", 1)
        self.assertIn("static WIDE_RANGES: [Range; 0]", render(source))

    def test_cli_check_detects_drift_without_overwriting_output(self) -> None:
        with tempfile.TemporaryDirectory(prefix="terminal-unicode-test-") as temporary:
            source = Path(temporary) / "input.zig"
            output = Path(temporary) / "output.rs"
            source.write_text(fixture(), encoding="utf-8")
            command = [sys.executable, str(GENERATOR), str(source), str(output)]
            subprocess.run(command, check=True, capture_output=True, timeout=10)
            expected = output.read_bytes()
            self.assertEqual(expected.decode(), render(fixture()))
            result = subprocess.run(
                [*command, "--check"], check=False, capture_output=True, timeout=10
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_bytes(), expected)
            output.write_bytes(expected + b"// stale\n")
            result = subprocess.run(
                [*command, "--check"], check=False, capture_output=True, timeout=10
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"generated terminal Unicode data is stale", result.stderr)
            self.assertEqual(output.read_bytes(), expected + b"// stale\n")


if __name__ == "__main__":
    unittest.main()
