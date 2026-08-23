# Milestone 03 native `grep_files` review 01

Status: **IN PROGRESS — formal fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is green with zero findings
across correctness/API, filesystem/robustness, and performance/concurrency;
this documentation-only seal needs no further adversarial review, while remote
workflow IDs and final delivery remain pending**

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
- Second production remediation:
  `ac5d7726411744e4f85344edf966d26a3cdb0a26`
- Composed second production remediation head:
  `d67221021aa173299f8f2e99d2574a15870cd5c8`
- Second documentation remediation:
  `7ad0863885d28b7b7a1d6f89d35f525cdd2dd3fa`
- Fully composed second-fix local-gate precursor:
  `b498ba06fa808dc9453a7644727cf8166b6f8e87`
- Second-fix local gates: **GREEN** on exact `b498ba0`
- Second replacement formal behavior-review candidate:
  `5aeddc1b4cb210b00cb967b938db8d5232062916`
- Second replacement correctness/API review: **GREEN — zero findings** on exact
  `5aeddc1`
- Second replacement security/filesystem-robustness review: **GREEN — zero
  findings** on exact `5aeddc1`
- Second replacement performance/concurrency review: **NOT GREEN — one MEDIUM
  and two LOW findings** on exact `5aeddc1`
- Third production remediation:
  `8777825b1b8b8c97dd4eb4bb31c0d8dbed9a7741`
- Composed third production remediation head:
  `ab1c13385a475ac34e8df2180e8c4cbb3b0ee3e9`
- Independent-test remediation:
  `dcf57ad35150b86c84a3f6c1127d9e379f3840fc`
- Composed production-and-test remediation head:
  `d7526d4dcd7f41be1b8d4c95d640da061088517c`
- Review-findings documentation remediation:
  `44afb232f2b8418c0b61eec7d1dab46bbe8e3667`
- Composed production, test, and documentation remediation head:
  `f08c5f2e35befb5e533ef4bb80a4b342dc5ffa46`
- Exact-toolchain lint follow-up:
  `1f13f9ae04ee3307d13a363ed28b156d7ee2421f`
- Fully composed third-remediation local-gate precursor:
  `a8f61794ee5e279558856220b5789526b908015a`
- Third-remediation local gates: **GREEN** on exact `a8f6179`
- Formal third-cycle behavior-review candidate:
  `0bfe68a9692837187c057b5b4efa08ebe3dee058`
- Third-cycle correctness/API review: **NOT GREEN — one LOW documentation
  contract mismatch** on exact `0bfe68a`
- Third-cycle security/filesystem-robustness review: **GREEN — zero findings**
  on exact `0bfe68a`
- Third-cycle performance/concurrency review: **NOT GREEN — the same one LOW
  documentation contract mismatch** on exact `0bfe68a`
- Third-cycle confirmed production defects: **zero**
- Documentation wording-remediation component:
  `993b618bf78d30f6a68f3b248b572e33e4de1126`
- Composed wording-remediation head:
  `f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`
- Exact f87 documentation gates: **GREEN**
- Behavior tree: **unchanged from `a8f6179` except for documentation**
- Formal fourth-cycle exact behavior-review SHA:
  `8e5fccea1b12483d2de2448e7a4ece0dc840ba39`
- Fourth-cycle correctness/API review: **GREEN — zero findings** on exact
  `8e5fcce`
- Fourth-cycle security/filesystem-robustness review: **GREEN — zero findings**
  on exact `8e5fcce`
- Fourth-cycle performance/concurrency review: **GREEN — zero findings** on
  exact `8e5fcce`
- Exact behavior SHA with all three review tracks green:
  `8e5fccea1b12483d2de2448e7a4ece0dc840ba39`
- Documentation-only seal: **THIS COMMIT — explicitly exempt from another
  adversarial review; self-SHA cannot be embedded**
- Feature CI and benchmark-evidence workflow IDs: **PENDING**
- Integrated `main` SHA: **PENDING**
- Exact `main` CI and benchmark-evidence workflow IDs: **PENDING**
- Final delivery SHA: **PENDING**
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
tracks. Second production remediation `ac5d772` composes at `d672210`; second
documentation remediation `7ad0863` composes with it at fully composed exact
local-gate precursor `b498ba0`. Formal second replacement candidate `5aeddc1`
has correctness/API and filesystem/robustness green with zero findings, while
performance/concurrency is not green with one medium and two low findings.
Third production remediation `8777825` composes at `ab1c133`; independent
regression `dcf57ad` composes at `d7526d4`; review-findings documentation
`44afb23` composes at `f08c5f2`; and exact-toolchain lint follow-up `1f13f9a`
produces fully composed local-gate precursor `a8f6179`. Its exact local,
cross-target, dependency, link, compatibility, release, and CLI-smoke gates are
green. Formal third-cycle candidate `0bfe68a` has filesystem/robustness green
with zero findings. Correctness/API and performance/concurrency are not green
only for the same low documentation contract mismatch; reviewers confirmed
zero production defects. Isolated wording remediation `993b618` composes at
exact `f87f6be`; its documentation gates are green, and its behavior tree
remains `a8f6179` except for documentation. Formal fourth-cycle exact behavior
SHA `8e5fcce` is green with zero findings across all three fresh tracks. All
historical findings are closed, including the attempted-read-window storage
wording. This documentation-only seal is exempt from further adversarial review;
feature and `main` workflow IDs and the final delivery SHA remain pending.
Every listed identifier was observed directly; no production/test/documentation
remediation, replacement rereview, behavior-green, seal, or workflow identifier
may be inferred from a branch tip, tree identity, or another component.
Production, independent tests, and maintained docs remain non-overlapping
ownership slices and must compose without overwriting one another.

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
- One content buffer is local to one scan. Initialized storage is acquired or
  grown before reads in attempted-read windows of at most 8 KiB and has a high-
  water length no greater than the 204,801-byte file-plus-witness cap. Logical
  length, the visible file slice, and charged content-byte counters advance only
  by bytes actually read. The buffer logically resets between files; reentrant
  scans do not share it, reset exposes no stale bytes, and actual per-file and
  aggregate overflow witnesses remain charged.
- Context is taken from the same validated logical file view of that reusable
  buffer. Match excerpts are UTF-8-safe, at most 4,096 bytes, and contain the
  complete first match. Retained output owns its bytes before buffer reuse.
  Context records are complete bounded prefixes; `context_truncated` separates
  omitted requested context from top-level page truncation.
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
  bytes, and both recursive and non-recursive dynamic-programming branches route
  through the injectable scan-local cancellation checker. Execution owns all
  descriptors and its one content buffer and detaches nothing.
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
  serialization-trimming iteration. Replacement remediation and rereview
  evidence must additionally prove the charged, cancellation-checked slashful
  selected-file rejection plus at-most-1,024-byte cancellation intervals while
  splitting slashful candidates and through both dynamic-programming branches.
  Exact candidate `5aeddc1` did not supply recursive-branch proof. Third
  production remediation `8777825` routes the recursive branch through the
  injected checker and supplies a deterministic recursive regression. These
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

Fully composed second-fix local-gate precursor
`b498ba06fa808dc9453a7644727cf8166b6f8e87` is green under Rust and Cargo
1.94.1 exactly:

- formatting and workspace/all-target/all-feature warnings-denied Clippy pass;
- the workspace gate passes 592 non-documentation tests plus two doctests, and
  the native private-library gate passes 75/75;
- focused coverage remains 39 direct `grep_files` integration tests and four
  real-engine tests;
- the repository Python gate runs 129 tests: 121 pass and eight expected macOS
  skips, with zero failures or errors;
- a fresh credential-stripped upstream fx checkout at
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes
  `generate_compatibility.py --check`;
- `cargo clean` removes 12,335 files / 1.5 GiB before a locked release CLI build
  passes in 48.85 seconds; all eight bare, help, version, status, and JSON-status
  assertions exit zero with byte-exact stdout, empty stderr, and no config or
  state creation;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings under `--no-fetch`;
- Linux and FreeBSD no-default native Clippy pass with warnings denied; WASI
  no-default and all-feature checks pass with only the pre-existing
  `read_file::check_cancellation` dead-code warning; the dedicated unsupported-
  target test compiles to an active WASI artifact containing its exact test,
  closure, private sentinel, fixed diagnostics, and harness `main`; and
- 58 Markdown files contain 420 inline links, including 270 repository-relative
  links with zero missing; second-fix diff checks pass with a clean exact-SHA
  worktree.

Fully composed third-remediation local-gate precursor
`a8f61794ee5e279558856220b5789526b908015a` is green under Rust and Cargo
1.94.1 exactly:

- formatting and workspace/all-target/all-feature warnings-denied Clippy pass;
- the combined workspace gate passes 598 non-documentation tests plus two
  doctests, the private `grep_files` gate passes 25/25, the direct integration
  gate passes 40/40, and four real-engine tests pass;
- the repository Python gate discovers two files and runs 129 tests: 121 pass
  and eight expected macOS skips, with zero failures or errors;
- a fresh credential-stripped upstream fx checkout at
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes
  `generate_compatibility.py --check`;
- a clean locked release CLI build passes and produces an arm64 Mach-O binary
  with SHA-256
  `e4d4246b501c524121d1f4af270a662f11e96b33dd4cb8c8bc1be40142a5ebe0`; all
  eight bare, help, version, status, and JSON-status forms exit zero with byte-
  exact stdout, empty stderr, and no config or state creation;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings under `--no-fetch`;
- Linux and FreeBSD no-default native Clippy pass with warnings denied; WASI
  no-default and all-feature checks pass with only the pre-existing
  `read_file::check_cancellation` dead-code warning; the dedicated unsupported-
  target test compiles to a 12,789,126-byte active WASI artifact containing its
  exact test, closure, private sentinel, fixed diagnostics, and harness `main`;
  and
- 58 Markdown files contain 420 inline links, including 270 repository-relative
  links with zero missing; all remediation and whole-lineage diff checks pass
  in an exact detached clean worktree.

These local results are third-remediation precursor evidence, not remote-
delivery evidence. First replacement candidate `ae87bf1` remains historically
NOT GREEN. Formal second replacement candidate `5aeddc1` remains historically
not green because its performance/concurrency review confirmed one medium and
two low findings. The remediation does not become review-green until all three
replacement tracks approve one later exact behavior SHA.

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
replacement reviewers to approve one exact behavior SHA.

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
  work from later candidate, content-byte, and per-file matching budgets and is
  composed at `b498ba0`.

### First replacement security/filesystem-robustness track — NOT GREEN

- **LOW — confirmed:** the first-cycle resolution text claimed a deep FIFO
  regression, while the deep special-kind fixture constructs symlinks. The
  corrected evidence statement names the deep symlink/special-kind branch and
  retains the separate stable shallow FIFO and pre-poll substitution coverage.
  The correction is composed at `b498ba0`; no production filesystem defect was
  found in this track.

### First replacement performance/concurrency track — NOT GREEN

- **MEDIUM — confirmed:** slashful candidate splitting and false
  dynamic-programming cells could traverse bounded work without the documented
  at-most-1,024-byte cancellation interval. Composed production at `d672210`
  makes splitting cancellation-aware at that interval and checks both recursive
  and non-recursive dynamic-programming branches. The existing deterministic
  checker regression exercises the non-recursive false-cell path; it does not
  route the recursive branch through the injected checker, as the later second
  replacement review found.
- **LOW — confirmed:** slashful include rejection for a selected file returned
  without a charged or local cancellation-checked include decision. Composed
  production charges exactly one decision unit and checks cancellation, with
  deterministic exact-work evidence.

Production and documentation corrections, independent regressions, and exact
local gates are composed and green at `b498ba0`.

### Second replacement correctness/API track — GREEN

Exact candidate `5aeddc1b4cb210b00cb967b938db8d5232062916` is **GREEN** with
zero findings on the correctness/API track.

### Second replacement security/filesystem-robustness track — GREEN

Exact candidate `5aeddc1b4cb210b00cb967b938db8d5232062916` is **GREEN** with
zero findings on the security/filesystem-robustness track.

### Second replacement performance/concurrency track — NOT GREEN

- **MEDIUM — confirmed:** each eligible file can allocate a fresh 204,801-byte
  content buffer even when the file is empty. A 10,000-empty-file diagnostic
  accumulated approximately 2,048,010,000 allocated bytes and observed 6.10
  seconds. The allocation total demonstrates amplification; the observed time
  is diagnostic only and is not a contractual timing or product-performance
  result. Third production remediation `8777825` creates one scan-local buffer.
  It acquires or grows initialized storage before reads in attempted-read
  windows of at most 8 KiB, never beyond the 204,801-byte high-water cap, and
  logically resets it between files. Only logical length, visible bytes, and
  charged content bytes advance by bytes actually read. It retains exact file
  and aggregate overflow-witness accounting, isolates reentrant scans, and
  composes at `ab1c133`. Private tests cover empty reuse, maximum-to-empty/tiny
  stale-byte exclusion, interrupt/error/cancellation semantics, and reentrant
  isolation; independent regression `dcf57ad`, composed at `d7526d4`, covers
  one maximum file followed by many empty and tiny files.
- **LOW — confirmed:** the second-fix local-gate record reported 57 Markdown
  files, but the measured inventory is 58. This record corrects the count;
  review-findings documentation `44afb23` composes at `f08c5f2`.
- **LOW — confirmed:** the first replacement resolution claimed deterministic
  checker regressions for both dynamic-programming branches, but the recursive
  branch did not route through the injected checker. Third production
  remediation `8777825` routes both branches through one injectable checker and
  adds the deterministic recursive regression. Exact-toolchain lint follow-up
  `1f13f9a` preserves that evidence under warnings-denied Clippy.

Production, independent-test, review-findings documentation, and lint
remediation compose through `ab1c133`, `d7526d4`, `f08c5f2`, and exact fully
composed local-gate precursor `a8f6179`. Its exact local, cross-target,
dependency, link, compatibility, release, and CLI-smoke gates are green.

### Third-cycle correctness/API track — NOT GREEN

- **LOW — confirmed documentation mismatch:** exact candidate
  `0bfe68a9692837187c057b5b4efa08ebe3dee058` says initialized storage grows
  only as observed bytes require. Production instead acquires or grows storage
  before each read in an attempted window of at most 8 KiB. Logical length,
  visible file bytes, and charged content-byte counters advance only by bytes
  actually read. This is a contract-wording defect, not a production defect.

### Third-cycle security/filesystem-robustness track — GREEN

Exact candidate `0bfe68a9692837187c057b5b4efa08ebe3dee058` is **GREEN** with
zero findings on the security/filesystem-robustness track.

### Third-cycle performance/concurrency track — NOT GREEN

- **LOW — confirmed documentation mismatch:** this track confirmed the same
  attempted-read-window versus observed-byte storage-growth mismatch and zero
  production defects. Diagnostic allocator instrumentation over two 10,000-file
  boundary scans requested approximately 4,103,462,456 bytes and made 20,000
  allocations of exactly 204,801 bytes at `5aeddc1`, versus approximately
  7,459,007 requested bytes and zero maximum-sized allocations at `0bfe68a`.
  The maximum-plus-384 regression requested approximately 3,349,064 bytes and
  made one high-water allocation. Allocation and timing instrumentation is
  diagnostic only, not a contractual or product-performance result.

Isolated documentation wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`. On exact f87, formatting and two
doctests pass; 58 Markdown files contain 420 inline links, including 270
repository-relative links with zero missing; and diff, added-line-length, exact
11-file ownership, and clean-worktree checks pass. Its behavior tree remains
`a8f6179` except for documentation.

### Fourth-cycle correctness/API track — GREEN

Exact behavior candidate `8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is
**GREEN** with zero findings. The corrected contract states that initialized
storage is acquired or grown before reads in attempted windows of at most 8 KiB
while logical length, visible bytes, and charged content bytes advance only by
bytes actually read.

### Fourth-cycle security/filesystem-robustness track — GREEN

Exact behavior candidate `8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is
**GREEN** with zero findings. Descriptor confinement, type validation, race and
cancellation semantics, fixed redaction, and evidence wording have no open
finding.

### Fourth-cycle performance/concurrency track — GREEN

Exact behavior candidate `8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is
**GREEN** with zero findings. Scan-local buffer reuse, bounded attempted-read
windows, actual-read accounting, linear matcher work, cancellation intervals,
and output bounds have no open finding. Allocation and timing instrumentation
remains diagnostic only.

Exact 8e5 validation is green: formatting; warnings-denied workspace Clippy and
workspace tests; Linux/FreeBSD cross-target and WASI gates; two doctests; 25
private native tests; 40 direct `grep_files` tests; four engine tests; and 58
Markdown files with 420 inline links, including 270 repository-relative links
and zero missing. All historical correctness, filesystem, performance,
cancellation, evidence, inventory, and attempted-window wording findings are
closed. Exact behavior SHA `8e5fcce` is green across all three fresh tracks.

This commit is a documentation-only review seal. The user's explicit
instruction exempts it from another adversarial review after behavior is green.
Feature and `main` CI and benchmark-evidence workflow IDs, integrated `main`,
and the final delivery SHA remain **PENDING**.
