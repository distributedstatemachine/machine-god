# Milestone 03 native `grep_files` review 01

Status: **IN PROGRESS — fully composed behavior and exact local gates are green;
formal behavior reviews, seal, and delivery evidence are pending**

## Candidate lineage

- Exact base: `f6aa458bb875d6cb26565adc878703fe140916d3`
- Tree-identical integration kickoff:
  `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`
- Production owner: `agent/m03-grep-files-prod`
- Independent-test owner: `agent/m03-grep-files-tests`
- Documentation owner: `agent/m03-grep-files-docs`
- Integration owner: `agent/m03-grep-files`
- Production component: `27eec2f3c25ffecd1ba8ff3c0a4fe0129dbeeac3`
- Initial independent-test component: `6eaee93398de8fbf6e87e77cf4d3e7de56e2a8cb`
- Documentation component: `b04151a7d958875118eebddd67526d74e2ea9526`
- First composed production head: `9057feb24fd3f24657148ca8e78198b88c9dbab4`
- Initial composed production-and-test head:
  `44e33d7e24c6650a1e375cd095eb9efae31f4e78`
- Reference-host fixture fix and focused production/test head:
  `bdbb677161322e249aea95a12bfb1b2169ff5b48`
- First fully composed behavior candidate:
  `42e4793b27902da7390dc54ef6bedb169da7e1bc`
- Local-gate precursor: `45ad91fa2689250c47c79d2105f5e3c261cea638`
- Formal correctness/API review SHA: **PENDING**
- Formal security/abuse review SHA: **PENDING**
- Formal performance/concurrency review SHA: **PENDING**
- Confirmed-finding fixes and rereviews: **PENDING**
- Exact behavior SHA with all three review tracks green: **PENDING**
- Documentation seal: **PENDING**
- Feature CI and benchmark-evidence runs: **PENDING**
- Integrated `main` SHA: **PENDING**
- Exact `main` CI and benchmark-evidence runs: **PENDING**
- Required toolchain: Rust and Cargo 1.94.1 exactly

The three isolated component SHAs and all integration heads above were observed
directly. Initial production-and-test head `44e33d7` required its reference-host
fixture fix; focused production-and-test composition is green through
`bdbb677`. Maintained documentation first composes at `42e4793`; the bounded
lint and cross-target correction composes at local-gate precursor `45ad91f`.
No pending SHA or workflow identifier may be inferred
from a branch tip, tree identity, or another component. Record it only after the
named artifact exists and was observed. Production, independent tests, and
maintained docs are parallel, non-overlapping ownership slices and must compose
without overwriting one another.

The exact base is the final `glob_files` documentation record. It passed feature
CI `32611623653` and feature benchmark evidence `32611623655`. GitHub did not
materialize workflows for its first `main` event, so the tree-identical,
non-behavior grep kickoff marker `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`
passed feature CI `32612424382` and feature benchmark evidence `32612424383`,
was fast-forwarded to `main`, and passed exact main CI `32612662260` and main
benchmark evidence `32612662203`. Neither documentation-only handoff record nor
tree-identical marker required another adversarial cycle after the preceding
`glob_files` behavior was green.

For this nineteenth slice, the maintained behavior in
[`grep-files.md`](../grep-files.md) must be present in the exact behavior SHA
reviewed by all three tracks. After that behavior is green, the user's explicit
instruction exempts a later documentation-only seal or final delivery record
from another adversarial review. Those commits still require exact feature and
`main` workflows, and this record must report them before delivery is called
complete.

## Candidate behavior to review

- A Linux/macOS `GrepFilesTool` retains one explicitly injected absolute
  workspace descriptor. Construction and every selected/traversed component
  are no-follow, close-on-exec, nonblocking, and type-checked under the existing
  fresh-root liveness rule.
- Strict, effect-free preflight accepts exactly required `pattern` plus optional
  `path`, `include`, `case_insensitive`, `mode`, `head_limit`, `offset`, and
  `context_lines`; it rejects unknowns and prepares all eight canonical values
  with defaults `.`, `null`, `false`, `matches`, `100`, `0`, and `0`.
- Policy receives distinct `FilesystemAccess::SearchContent` at the exact
  normalized selected path. It is not `Read`, `Metadata`, `Enumerate`, or
  `EnumerateRecursive` and grants no mutation or external/symlink-target
  authority.
- The selected path may be one regular file or directory. Directory traversal
  is iterative, hidden-inclusive, fully no-follow, sorted and deterministic for
  a stable tree. Only regular files are opened. Symlinks and specials are never
  followed or read.
- `include` uses the delivered bytewise glob grammar and work accounting before
  content open. It filters regular candidates but does not prune traversal.
- Content matching is literal and worst-case linear. Case-insensitive mode folds
  ASCII only. One result represents each matching LF-delimited line and records
  the first matching byte offset.
- A candidate is eligible text only when its complete observed content is no
  more than 204,800 bytes, valid UTF-8, and NUL-free. Oversized and non-text
  files are skipped with disclosed aggregate statistics. Other candidate
  failures fail the whole call.
- Context is taken from the same validated file buffer. Match excerpts are
  UTF-8-safe, at most 4,096 bytes, and contain the complete first match. Context
  records are complete bounded prefixes; `context_truncated` separates omitted
  requested context from top-level page truncation.
- `matches`, `files_with_matches`, and `count` return the exact structured
  shapes in the normative contract. Matching totals remain exact for eligible
  text after a complete bounded scan. Both list modes implement exact bounded
  offset/head pagination, `next_offset`, and list-completeness `truncated`.
- One call is bounded by 4,096-byte input/result strings, head 100, offset
  100,000, context 5, 100,000 entries, 16 MiB entry names, 10,000 candidates,
  64 MiB aggregate content, 8 Mi include steps, 256 Mi content-match steps,
  depth 256, 8 KiB aggregate result paths, 8 KiB aggregate result text, and a
  48 KiB complete serialized `ToolOutput`.
- A fired scan/work cap fails without partial output. Output omission occurs
  only after complete scanning and is represented by list truncation/next
  semantics; count has neither field.
- Execution is inert until first poll, performs bounded synchronous work, checks
  cancellation around every authority-bearing operation and at bounded CPU
  intervals, owns all descriptors/buffers, and detaches nothing.
- Fixed constructor/tool errors retain and reflect no path, pattern, include,
  entry name, file bytes, match, metadata, OS diagnostic, or errno. Successful
  excerpts and paths are intentionally model-visible durable data.
- The reference host registers exactly five alphabetical tools:
  `file_info`, `glob_files`, `grep_files`, `list_files`, and `read_file`. It
  distributes one original workspace descriptor plus four clones of the same
  retained identity.

## Required adversarial tracks

Three fresh reviewers must inspect the same exact fully composed behavior SHA:

1. correctness/API and permission/preflight agreement;
2. security/abuse, descriptor confinement, redaction, malformed input and
   hostile-tree/content bounds; and
3. performance/concurrency, linear matcher work, allocation/output caps,
   cancellation, drop and race behavior.

Confirmed findings require isolated fixes, independent regressions, a new
composed candidate, replacement local gates, and same-SHA rereviews until all
three tracks are green. Rejected findings and their evidence remain in this
record.

## Required evidence before delivery

- Focused production and independent integration tests pass on Linux and macOS
  boundaries, including exact-cap and one-over cases.
- Permission denial causes no filesystem effect, and engine execution observes
  the exact normalized `SearchContent` path and canonical arguments.
- Tests cover file and directory roots, hidden entries, invalid names, every
  symlink position, FIFO/special substitution, candidate/read/matcher/output
  caps, valid/invalid UTF-8 and NUL, oversized/growing files, rejected empty and
  control-containing patterns, longest-valid patterns, adversarial repeated-
  prefix case-sensitive and ASCII-insensitive inputs, pagination/context
  interactions, root rename/removal, `NOENT` races,
  cancellation intervals, unpolled/drop ownership, and diagnostic redaction.
- Reference-host and prepared-root tests prove exactly five alphabetical tools
  share one opened workspace identity through the original descriptor plus four
  clones.
- Required exact-toolchain local gates pass: formatting, Clippy with warnings
  denied, workspace tests, and workspace doc tests. The freshly built release
  binary is exercised only to prove its existing bytes remain unchanged; this
  candidate adds no CLI path.
- Three formal adversarial tracks are green on one exact behavior SHA.
- The pushed feature branch passes exact-SHA CI and benchmark-evidence workflows.
- `main` is fast-forwarded without force and passes exact-SHA CI and benchmark-
  evidence workflows.

## Local gate results

Exact local-gate precursor `45ad91fa2689250c47c79d2105f5e3c261cea638`
is green under Rust and Cargo 1.94.1 exactly:

- formatting, workspace/all-target/all-feature warnings-denied Clippy, 571
  workspace unit/integration tests plus two doctests, explicit 2/2 workspace
  doctests, and 63/63 native private library tests pass;
- the focused behavior represented in the workspace suite includes 30 direct
  `grep_files`, four real-engine, seven reference-host, three prepared-root,
  and 58 core contract tests;
- the repository Python gate runs 129 tests: 121 pass and eight expected macOS
  skips, with no failure or error;
- a fresh exact upstream fx checkout at
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes compatibility-inventory
  generation check; a clean locked release build and all eight accepted CLI
  bare/version/help/status forms pass exact-output, empty-stderr, and no-create
  checks;
- cargo-deny 0.20.2 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings;
- Linux and FreeBSD no-default native Clippy pass with warnings denied; WASI
  no-default and all-feature checks pass with only the pre-existing
  `read_file::check_cancellation` dead-code warning; and
- 58 Markdown files contain 420 valid relative links with zero missing, while
  both base-to-candidate and kickoff-to-candidate diff checks pass.

## Explicit nonclaims

This candidate adds no regex or Unicode case folding, binary search, alternate
encoding, external path, symlink-target search, Git/ignore/subprocess behavior,
index/cache/watch/snapshot, CLI command, benchmark workload, compatibility-
status change, fx-equivalence claim, or product-performance claim. Zig remains
only the pinned upstream benchmark build input; the product is Rust. The
combined native-tool checklist and Milestone 03 remain open.

## Findings and resolution

**PENDING.** Formal review has not begun. No track may be marked green and no
finding may be described as resolved until the exact fully composed behavior
SHA exists and the named reviewer has reported on it.
