# Milestone 01 compatibility inventory review 01

Reviewed commit: `74550bec209857403c1677e539fa949cfe288646`.

Fresh correctness/API, security/abuse, and performance/concurrency reviews
confirmed the following findings and resolutions:

- Upstream parsing and provenance hashing read checkout files, so concurrent
  edits, clean/smudge filters, and platform line-ending conversion could make
  the inventory differ from the pinned commit. The generator now resolves the
  exact commit and reads every source through `git ls-tree` and `git cat-file`;
  hashes cover those canonical blob bytes and include their Git blob IDs.
- A checkout symlink or non-file entry could redirect a source read. Every
  parsed source and root E2E owner must now be a `100644` or `100755` blob;
  symlinks, gitlinks, trees, missing entries, and duplicate entries fail closed.
- Regex searches could recognize Zig declarations or fields inside comments
  and strings. Structural parsing now uses offset-preserving comment/string
  masks, top-level field depth, balanced initializers, exact enum coverage, and
  comma-terminated registry entries.
- The tool registry silently ignored expressions it did not understand. Its
  production registry now accepts only bare identifiers and rejects factory
  calls, conditionals, struct literals, and other unsupported expressions.
- JavaScript export discovery could count commented or quoted text and did not
  explicitly model re-exports. The SDK parser now recognizes named declaration
  exports, named export lists, and named `from` re-exports while rejecting
  wildcard, default, empty, malformed, and otherwise unsupported export forms.
- Extracted names and paths could carry syntax outside the documented upstream
  contracts, and generated Markdown interpolated dynamic text directly. Narrow
  grammars now validate CLI tokens and aliases, slash commands, tool names, SDK
  identifiers and entrypoints, and E2E owner/scenario names. All dynamic
  Markdown text and code spans pass through escaping helpers.
- Tests did not cover checkout filtering or immutable-object behavior. The
  fixture suite now commits real Git repositories and covers CRLF worktrees,
  worktree and `HEAD` mutation after snapshot selection, symlink modes,
  commented declarations, quoted fake exports, named re-exports, unsupported
  tool expressions, unsupported JavaScript exports, and Markdown metacharacters.
- CI ran parser unit tests but did not prove the retained artifacts against the
  real pinned upstream commit. The quality job now parses `upstream.lock`,
  fetches that exact public commit with prompting and credential helpers
  disabled, verifies detached `HEAD`, and runs the generator in `--check` mode.

Rejected after remediation: none.
