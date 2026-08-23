# Milestone 03 native `grep_files` review 01

Status: **IN PROGRESS — first replacement candidate
`ae87bf1454b1527b2e55ed5e517c21fd7410c980` is NOT GREEN; production and
documentation fixes, second replacement reviews, the behavior-green SHA, seal,
and delivery remain pending**

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
- First formal behavior-review candidate:
  `355a11a6055b0053dff80e71011d7633e8a6ce97`
- First-cycle correctness/API review: **NOT GREEN** on exact `355a11a`
- First-cycle security/filesystem-robustness review: **NOT GREEN** on exact
  `355a11a`
- First-cycle performance/concurrency review: **NOT GREEN** on exact `355a11a`
- Isolated production remediation:
  `012f14d273b15085713fba9092e93486d4e6f0e4`
- Composed production remediation head:
  `35defb5c7cee021064411535070b9ecd62387e2f`
- Isolated independent-test remediation:
  `646286203ab665e9dc9d0a86f7de6d036b7c5c86`
- Composed production-and-test remediation head:
  `58550734b77a5a44c4b9452438e34f265013c40b`
- Isolated documentation remediation:
  `771a3e34816d2d67cd1e08d73abdac7c807313a3`
- Composed production, test, and documentation remediation head:
  `3cd282fafb26bb069ac73407fde0fd30c7d1ff82`
- Isolated deterministic production-evidence component:
  `842d36ae5f7cc8aa5a3011a41bba209e0f35172c`
- Composed deterministic-evidence head:
  `630acbb384f2e1b79b6916a10baaac26acafbf41`
- Isolated unsupported-target test component:
  `325350ce558a7e6f21ef4cf2d4d030e30cc4f740`
- Final code/test local-gate precursor:
  `275d263dd3c7981e66f6a0f90f3779c271eb4cc3`
- Replacement local gates: **GREEN** on exact `275d263`
- First replacement formal behavior-review candidate:
  `ae87bf1454b1527b2e55ed5e517c21fd7410c980`
- First replacement correctness/API review: **NOT GREEN — LOW** on exact
  `ae87bf1`
- First replacement security/filesystem-robustness review: **NOT GREEN — LOW**
  on exact `ae87bf1`
- First replacement performance/concurrency review: **NOT GREEN — MEDIUM and
  LOW** on exact `ae87bf1`
- Second replacement correctness/API review SHA: **PENDING**
- Second replacement security/filesystem-robustness review SHA: **PENDING**
- Second replacement performance/concurrency review SHA: **PENDING**
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
Maintained documentation records that all three first-cycle reviews of exact
candidate `355a11a` are NOT GREEN. The five isolated remediation/evidence
components above compose in order through `35defb5`, `5855073`, `3cd282f`,
`630acbb`, and final local-gate precursor `275d263`. First replacement formal
candidate `ae87bf1` records those gates and is NOT GREEN under all three fresh
tracks. Every listed identifier was observed directly; no second-replacement,
seal, or workflow identifier may be inferred from a branch tip, tree identity,
or another component. Production,
independent tests, and maintained docs remain non-overlapping ownership slices
and must compose without overwriting one another.

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
  a stable tree. Stable specials are skipped without open; any raced nonblocking
  special open is authoritatively rejected, and no special or symlink target is
  read or followed.
- `include` is compiled and preparsed once per call, uses the delivered bytewise
  glob grammar, and charges complete parse plus match work before content open.
  It filters regular candidates but does not prune traversal. Selected-file
  filtering occurs after no-follow stat classification and before content open.
  A slashful selected-file rejection consumes one charged cancellation-checked
  include unit. An excluded selected file consumes fixed pattern-table and
  include work but no candidate, content-byte, or per-file matching work. An
  included selected file opens and is revalidated before those latter budgets.
- Content matching is literal and worst-case linear. Pattern-table construction
  consumes fixed literal work before selected-root resolution. Case-insensitive
  mode folds ASCII only. One result represents each matching LF-delimited line
  and records the first matching byte offset.
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
  67,108,864, context 5, 100,000 entries, 16 MiB entry names, 10,000 candidates,
  64 MiB aggregate content, 8 Mi include steps, 256 Mi content-match steps,
  depth 256, 8 KiB aggregate result paths, 8 KiB aggregate result text, and a
  48 KiB complete serialized `ToolOutput`.
- A fired scan/work cap fails without partial output. Output omission occurs
  only after complete scanning and is represented by list truncation/next
  semantics; count has neither field.
- Every complete descendant path is length-checked before allocation,
  entry-kind dispatch, or include matching.
- Execution is inert until first poll, performs bounded synchronous work, checks
  cancellation around every authority-bearing operation, at fixed intervals
  through line indexing and matching, and before every serialization-trimming
  attempt. Slashful candidate splitting checks at most every 1,024 candidate
  bytes, and both recursive and non-recursive dynamic-programming branches
  retain cancellation checks. Execution owns all descriptors/buffers and
  detaches nothing.
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
  interactions, root rename/removal, deterministic pre-poll growth/removal/
  substitution, cancellation intervals, unpolled/drop ownership, and
  diagnostic redaction. Public synchronous-future tests prove pre-poll growth,
  removal, and special substitution. Deterministic internal classifiers prove
  post-observation `NOENT` omission and non-`NOENT` error mapping, growth and
  aggregate-content overflow with oversized accounting, raced nonregular-open
  rejection, line-index cancellation intervals, and a check before every
  serialization-trimming iteration. Second-replacement evidence must
  additionally prove the charged, cancellation-checked slashful selected-file
  rejection plus at-most-1,024-byte cancellation intervals while splitting
  slashful candidates and through both dynamic-programming branches. These
  deterministic seams replace flaky sleep-based race tests.
- The dedicated `grep_files_unsupported` integration target compiles for
  `wasm32-wasip1` with its constructor test active rather than cfg-elided. It
  exercises the reachable public boundary: `GrepFilesTool::open` returns the
  exact redacted `UnsupportedPlatform` failure without touching its private
  path. Execution is uninhabited and unreachable through safe public code on
  that target because construction cannot produce a tool; no fabricated unsafe
  instance is required or permitted as evidence.
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

Exact final code/test local-gate precursor
`275d263dd3c7981e66f6a0f90f3779c271eb4cc3` is green under Rust and Cargo
1.94.1 exactly:

- formatting and workspace/all-target/all-feature warnings-denied Clippy pass;
- the workspace gate passes 589 non-documentation tests plus two doctests, the
  explicit documentation gate passes 2/2, and the native private-library gate
  passes 72/72;
- focused coverage includes 39 direct `grep_files` integration tests and four
  real-engine tests;
- the repository Python gate runs 129 tests: 121 pass and eight expected macOS
  skips, with no failure or error;
- a fresh exact upstream fx checkout at
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes compatibility-inventory
  generation check; a clean locked release build and all eight accepted CLI
  bare/version/help/status forms pass exact-output, empty-stderr, and no-create
  checks;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings;
- Linux and FreeBSD no-default native Clippy pass with warnings denied; WASI
  no-default and all-feature checks pass with only the pre-existing
  `read_file::check_cancellation` dead-code warning; the dedicated unsupported-
  target test compiles to a WASI artifact containing its exact test function,
  closure, private sentinel, expected fixed diagnostics, and harness `main`, so
  it is active rather than a zero-test cfg elision; and
- 58 Markdown files contain 420 inline links, including 270 repository-relative
  links with zero missing; first-review-to-precursor, post-documentation, and
  final-test diff checks pass with a clean exact-SHA worktree.

These are local exact-SHA results, not second-replacement or remote-delivery
evidence. First replacement candidate `ae87bf1` is NOT GREEN. No replacement
cycle is green until all three reviewers report on the same exact behavior SHA.

## Explicit nonclaims

This candidate adds no regex or Unicode case folding, binary search, alternate
encoding, external path, symlink-target search, Git/ignore/subprocess behavior,
index/cache/watch/snapshot, CLI command, benchmark workload, compatibility-
status change, fx-equivalence claim, or product-performance claim. Zig remains
only the pinned upstream benchmark build input; the product is Rust. The
combined native-tool checklist and Milestone 03 remain open.

## Findings and resolution

All three first-cycle tracks reviewed exact candidate
`355a11a6055b0053dff80e71011d7633e8a6ce97` and are **NOT GREEN**.
Every confirmed production fix and required regression/evidence item below is
implemented in the recorded remediation lineage and locally green at exact
`275d263dd3c7981e66f6a0f90f3779c271eb4cc3`. The track headings preserve the
historical first-cycle verdicts; formal closure still requires all three
second replacement reviewers to approve one exact behavior SHA.

### Correctness/API track — NOT GREEN

- **MEDIUM — confirmed:** descendant paths were allocated and retained before
  the documented 4,096-byte full-path check. The implemented remediation uses
  checked full workspace-relative length before allocation, entry-kind dispatch,
  or include matching, with exact-bound and one-over regressions.
- **MEDIUM — confirmed:** the accepted `offset <= 100000` could reject a
  non-null `next_offset` emitted from a successful result with more than 100,000
  matching lines. The implemented remediation raises the bound to 67,108,864,
  which covers every reusable continuation under the 64 MiB aggregate-content
  bound and required nonempty pattern; a regression consumes a continuation
  beyond the old ceiling.
- **LOW — confirmed:** the integration test file's Linux/macOS file-level gate
  removed purported unsupported-platform constructor/execution tests before
  they could compile on an unsupported target. The dedicated WASI integration
  target now compiles an active test of the reachable constructor boundary and
  its exact redacted failure. Execution remains safely unreachable because the
  failed constructor cannot produce an instance.
- **MEDIUM — confirmed:** the required evidence inventory named deterministic
  growth, post-observation race, and mid-execution cancellation coverage that
  the first candidate did not supply. Public synchronous-future regressions now
  cover pre-poll substitution, growth, and removal; deterministic internal
  classifiers cover `NOENT`, growth/overflow accounting, raced nonregular
  rejection, and cancellation intervals without sleeps.

### Security/filesystem-robustness track — NOT GREEN

- **MEDIUM — confirmed:** the same descendant-path defect violated the promised
  fail-closed path bound before directory retention and special-entry handling.
  The pre-allocation checked-length remediation and adversarial deep directory,
  symlink/special-kind, and regular-file regressions are locally green. Separate
  stable shallow FIFO and pre-poll special-substitution coverage remains green;
  this does not claim that the deep fixture itself creates a FIFO.
- **MEDIUM — confirmed:** deterministic evidence for growth witnesses,
  post-observation `NOENT`, raced special replacement, and cancellation
  intervals was absent. Production now exposes deterministic internal
  classifiers for `NOENT` omission/error mapping, growth and aggregate overflow
  with oversized accounting, raced nonregular rejection, and fixed cancellation
  checks; their private tests replace sleep-based races.
- **LOW — confirmed:** selected-file include filtering occurred only after the
  file had already been content-opened. The implementation now performs
  no-follow stat classification, then include filtering, then content open only
  when selected, with a no-read regression for an excluded selected file.
- **LOW — confirmed:** maintained wording said special objects were never
  opened. Maintained wording now says stable specials are skipped without open,
  while a regular-to-special replacement can race into a nonblocking open whose
  authoritative type validation rejects it before read. Deterministic
  classification evidence covers that rejection; no special content or symlink
  target is read or followed.

### Performance/concurrency track — NOT GREEN

- **HIGH — confirmed:** every slashful include invocation reparsed the same
  4,096-byte pattern, allocated a segment vector, and recounted segments outside
  the include-work meter. Up to 100,000 visited regular entries could amplify
  this into multi-gigabyte allocation churn. The implementation compiles and
  preparses once per call, reuses the compiled form, and meters complete parse
  plus match work; exact-cap and reuse regressions are locally green.
- **HIGH — confirmed:** unchecked depth-256 prefixes could reach roughly 64 KiB
  and be recopied for every entry before the later regular-file-only path check,
  permitting multi-gigabyte prefix-copy amplification. Although correctness and
  filesystem reviewers rated the contract violation medium, remediation
  priority is normalized to **HIGH** for this resource-amplification lens. The
  checked pre-allocation full-path bound and hostile deep-tree evidence are now
  locally green.
- **MEDIUM — confirmed:** line-index construction had no cancellation token or
  fixed checks, and serialized-size trimming checked only before and after its
  repeated reconstruction/serialization loop. Fixed line-index byte-interval
  checks and a check before every trimming attempt are implemented and proven
  through deterministic private cancellation tests.

All first-cycle findings are fixed and their required deterministic evidence is
implemented and locally green at exact precursor `275d263`. First replacement
candidate `ae87bf1` nevertheless remains NOT GREEN under the findings below.

### First replacement correctness/API track — NOT GREEN

- **LOW — confirmed:** maintained selected-file budget ordering implied that an
  included file opened and revalidated before every literal-matcher charge, but
  production constructs the fixed pattern table before resolving the selected
  root. The normative correction distinguishes fixed once-per-call pattern-table
  work from later candidate, content-byte, and per-file matching budgets.

### First replacement security/filesystem-robustness track — NOT GREEN

- **LOW — confirmed:** the first-cycle resolution text claimed a deep FIFO
  regression, while the deep special-kind fixture constructs symlinks. The
  corrected evidence statement names the deep symlink/special-kind branch and
  retains the separate stable shallow FIFO and pre-poll substitution coverage.
  No production filesystem defect was found in this track.

### First replacement performance/concurrency track — NOT GREEN

- **MEDIUM — confirmed:** slashful candidate splitting and false
  dynamic-programming cells could traverse bounded work without the documented
  at-most-1,024-byte cancellation interval. The second replacement must make
  splitting cancellation-aware at that interval and check both recursive and
  non-recursive dynamic-programming branches, with deterministic checker
  regressions.
- **LOW — confirmed:** slashful include rejection for a selected file returned
  without a charged or local cancellation-checked include decision. The second
  replacement must charge exactly one decision unit and check cancellation,
  with deterministic exact-work evidence.

Production and documentation fixes, their independent regressions, local gates,
and all three second replacement reviews remain **PENDING**. Once one exact
replacement behavior SHA is green across all three tracks, a later
documentation-only seal or final delivery record needs no additional
adversarial review under the user's explicit instruction, but exact feature and
`main` workflow evidence is still required.
