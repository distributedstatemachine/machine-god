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

## Rereview

Reviewed commit: `865193b1d0e9393c2beecf42c0491a980ee63dae`.

A second fresh adversarial review found and resolved four additional fail-open
paths:

- Git replacement refs could substitute a different commit or blob while the
  recorded pin and object ID remained unchanged. Git plumbing now runs with
  replacement objects disabled and a sanitized `GIT_*` environment, and every
  returned blob is independently hashed using the repository object format.
  Hostile commit-replacement, blob-replacement, and byte-substitution tests
  prove the recorded identities and canonical bytes remain coupled.
- Optional Zig registry fields were recognized only when their values matched
  the supported grammar, so dynamic or repeated assignments could silently
  degrade to defaults. Every known optional assignment is now found first at
  top-level depth, then required to occur once at most and use its supported
  literal form. Regression cases cover aliases, hidden-help flags, and
  presentation categories.
- Multi-declarator JavaScript exports recorded only their first binding.
  Variable export declarations are now structurally split on top-level commas,
  validate every declarator, and retain every exported identifier.
- JavaScript regular-expression contents could be mistaken for export syntax.
  The scanner now masks regular-expression literals with escape and character
  class handling while preserving division operators; tests combine injected
  fake exports, control-statement regexes, division, flags, and multiple
  exported declarators.

Rejected after rereview remediation: none.

## Final security rereview

Reviewed commit: `9bf6227eaf7c4cd449f370c0c25d688dfa970840`.

The final security pass found two related Zig lexical gaps. Payload text inside
quoted identifiers remained searchable, and multiline-string lines were masked
only when their `\\` prefix was the first non-whitespace text on a line. Either
form could hide a fake registry declaration that was accepted when the real
registry was absent.

Quoted-identifier payloads are now replaced by offset-preserving neutral
characters while their delimiters remain available to structural parsing;
identifier values are recovered from the original source only after the masked
syntax matches the supported grammar. Every Zig multiline-string prefix is now
recognized at any valid token position and blanked through end-of-line. A lone
backslash outside an already-recognized literal fails closed. Hostile tests
prove neither quoted identifiers nor same-line multiline strings can provide a
replacement registry, while a supported quoted identifier remains extractable.

Rejected after final security rereview remediation: none.

## Final performance rereview

Reviewed commit: `613ed0a639adc8e10452b72a26e592bb584ce38b`.

The performance pass confirmed superlinear behavior on synthetic registries.
Built-in tool extraction rebuilt the complete Zig source mask and rescanned the
complete file for every registered tool. JavaScript extraction rescanned the
mask prefix to calculate structural depth for every export and allocated a
suffix copy for several positional matches. Duplicate checks also used repeated
linear list searches.

Each source is now masked once and receives one shared structural-depth map.
ToolSpec declarations are indexed in a single pass over that shared Zig mask,
and their non-overlapping bodies reuse slices of the shared depth map.
JavaScript export patterns match against absolute positions, and top-level
depth checks are constant-time lookups. Uniqueness and ordered export
deduplication now use sets. A bounded scaling regression covers 2,000
JavaScript exports, while a 500-tool regression asserts exactly one full-source
mask and depth-map construction. All prior hostile fail-closed cases remain in
the same focused suite.

Rejected after final performance rereview remediation: none.

## Final scope-security rereview

Reviewed commit: `58ade4290d46ad3d85334ed1a10e1fc82cabd80b`.

The final scope review confirmed that the generic Zig initializer lookup
accepted matching declarations at any structural depth. A nested namespace
could therefore provide `TopLevelKind`, a command registry, or the built-in
tool registry even when the required production declaration was absent from
file scope.

Initializer lookup now reuses the full-source structural-depth map and requires
every matching contract declaration and its opening initializer brace to be at
depth zero. Its matching closing brace must return through exactly that scope;
registry entries and indexed ToolSpec bodies apply the corresponding local and
top-level scope checks. Structural depth calculation now uses a typed delimiter
stack, so a mismatched bracket cannot manufacture an apparent depth-zero
position. Nested enum, command-registry, and tool-registry decoys are covered by
hostile regressions. Shared mask/depth maps and the linear scaling bounds remain
intact.

Rejected after final scope-security rereview remediation: none.
