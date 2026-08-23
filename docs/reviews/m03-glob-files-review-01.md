# Milestone 03 native `glob_files` review 01

Status: **DELIVERED — first review found a high matcher-work bound defect; fix,
replacement local gates, three same-SHA rereviews, and exact feature and `main`
delivery gates are green**

## Candidate

- Exact base: `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`
- Production component: `a5d1399a5d05fcf056e8c1951fef92211824df25`
- Production schema-text correction: `f1584f5047a729c3a5647d2b9b59704b1f565e1b`
- Independent-test component: `948994d5cfee6693579f114a6f7a2ce50fd91258`
- Documentation component: `f2f0fc13a98922cc0aa9dc8643ef4620bce78eb9`
- Composed production head: `df3017cdae2508d4c82f85e4f1480760dc351906`
- Composed production-and-test head: `c9eccb7f09b67ddf4b7e9123519abcc013ca5532`
- First fully composed behavior candidate: `60070d899b7ac298960f6d01826d3876cf8b5835`
- Initial local-gate precursor: `60070d899b7ac298960f6d01826d3876cf8b5835`
- Initial formal review head: `1f5de6ac45e292f38b1853ab0a6212de70f4cc51`
- Isolated matcher-work fix: `f3fd13b33e30e084295ae722ec48b469292932a4`
- Isolated independent finding regression: `825bbd36d4173ca3393859f38bd301ad98aaddac`
- Composed matcher-work fix: `fe33ba944ebdcfada36d6299d451dbb2818a341e`
- Composed finding regression: `aba282128b1ae8ab586a925e96a25bc27c1db020`
- Replacement local-gate precursor: `4171a4a8811a98888b7e4e161281a1216564746f`
- Replacement formal rereview head shared by all three tracks:
  `523df85822a27102d7e7100e274e3bad7b25494f`
- Documentation seal: `35c853605077f2ac700f4be1dd79eabd2ace4dd4`
- Feature CI: `32610950593` — success on the exact documentation seal
- Feature benchmark evidence: `32610950594` — success on the exact seal
- `main` CI: `32611208411` — success on the exact integrated seal
- `main` benchmark evidence: `32611208415` — success on the exact integrated seal
- Delivery branch: `agent/m03-glob-files`
- Required toolchain: Rust and Cargo 1.94.1 exactly

Every delivery SHA and run above was observed at the named state; none is
inferred or invented. This delivered eighteenth bounded Milestone 03 slice adds a read-only
Linux/macOS `glob_files` library tool, the distinct
`FilesystemAccess::EnumerateRecursive` permission kind, direct and real-engine
independent tests, reference-host/root-bundle composition as a fourth workspace
tool, and maintained documentation. It does not alter CLI behavior, complete
the native-tool inventory or Milestone 03, change benchmark data or workflows,
or make a compatibility or product-performance claim.

## Candidate behavior to review

- `GlobFilesTool::open` accepts one explicitly injected absolute workspace,
  applies the established lexical host-root cleanup, and retains a no-follow
  directory descriptor on Linux/macOS. Other targets return the fixed
  unsupported-platform construction failure.
- Strict effect-free preflight accepts exactly
  `{pattern:string,path?:string,mode?:"matches"|"count"}`, defaults path to `.`
  and mode to `matches`, rejects unknowns, independently bounds requested and
  normalized path/pattern forms to 4,096 UTF-8 bytes, and applies the
  [`file_info`](../file-info.md) path confinement rule.
- Pattern normalization uses only `/` separators, collapses repeats and exact
  `.` segments, and rejects empty normalized, absolute, parent-traversing, or
  forbidden input. Backslash, square brackets, and braces are literal.
- Matching is bytewise: `?` is one byte, `*` is zero or more bytes within one
  component, and only an exact `**` segment spans zero or more components.
  Slash-free patterns match basenames recursively; slashful patterns match the
  candidate relative to the selected search root.
- Successful preflight prepares exact normalized arguments with both defaults
  explicit and asks policy for
  `FilesystemAccess::EnumerateRecursive` at the normalized selected subtree.
  Pattern and mode attenuate output and do not broaden or replace that
  recursive-enumeration authority. The existing one-level `Enumerate` remains
  distinct.
- Execution reacquires and validates the retained workspace through fresh `.`
  liveness exactly as `file_info`, opens the selected root and descendants
  descriptor-relatively and no-follow, and uses an iterative traversal. It
  fully reads and validates each directory, then bytewise sorts its entries
  before processing. Hidden entries are included.
- Only regular files and final symlinks are candidates. Directories are
  traversal-only, specials are ignored, symlinks are never descended through,
  and no content or link target is read. Results use full workspace-relative
  paths.
- Both modes complete the entire bounded traversal. The scan permits at most
  100,000 non-dot entries, 16 MiB of aggregate raw entry-name bytes, child-
  directory traversal depth 256, and 8,388,608 aggregate matcher-work steps.
  Steps meter slashful candidate splitting, pattern/DP-state visits, and inner
  component-byte matching. The selected root is depth 0; a directory at
  depth 256 is scanned and its regular/symlink children are eligible, while an
  attempted child-directory open at depth 257 is `scan_limit`. Any full
  workspace-relative candidate path over 4,096 bytes is also `scan_limit`.
  Firing a scan cap fails either mode without partial output.
- `matches` returns the longest globally bytewise-sorted prefix containing at
  most 100 paths and 16 KiB of aggregate raw path bytes. It never skips an
  omitted long path to retain a later short one. `truncated` is true exactly
  when an observed match is omitted. `count` returns the exact match count and
  has no truncation field.
- A stable tree is deterministic. There is no concurrent snapshot. A `NOENT`
  race after enumeration may omit an entry; other scan errors fail closed and
  use fixed redacted diagnostics. Invalid UTF-8 or forbidden entry names fail
  the complete call.
- Public bounds, constructor kinds/displays, and the complete fixed
  `glob_files_*` tool-error taxonomy are normative in
  [`glob-files.md`](../glob-files.md). No diagnostic reflects roots, provider
  input, entry or candidate names, operating-system text, or raw error numbers.
- The execution future is effect-free before first poll. Its first poll performs
  bounded synchronous work with cancellation checks around root acquisition,
  component opens, directory reads, classification/open operations, match
  accounting, and return. It cannot preempt one syscall and detaches no work.
- The native host registers exactly `file_info`, `glob_files`, `list_files`,
  and `read_file` in deterministic alphabetical catalog order. Prepared roots
  distribute the original retained workspace descriptor plus three clones of
  the same identity.
- The candidate adds no CLI behavior, external path, ignore/Git/subprocess
  behavior, mutation, content read, dependency, benchmark workload,
  performance claim, or fx-equivalence claim. Pinned input fields, enum values,
  and byte-matcher semantics are compatibility inputs only; deliberate strict,
  confinement, literal-syntax, output, and failure differences remain explicit.

## Parallel implementation ownership

Three isolated worktrees own non-overlapping surfaces from exact base
`bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`:

- production owns core permission/API changes and native production, including
  the tool, exports, and reference-host/root-bundle composition;
- independent tests own `glob_files` direct and real-engine coverage plus
  reference-host and prepared-root regression updates; and
- documentation owns the normative tool contract, maintained documentation
  indexes and guides, implementation-plan/status updates, and this review
  record.

The coordinator composed the initial isolated commits onto
`agent/m03-glob-files` without reverting unrelated work. Production behavior,
independent evidence, and documentation were present together at `60070d8`;
the first formal review ran on its local-evidence child `1f5de6a`. The accepted
finding fix and independent regression now compose through replacement
code-and-test head `4171a4a`.

## Required local gates

Focused direct, real-engine, reference-host, and prepared-root tests run first.
The exact composed candidate must then pass, under Rust and Cargo 1.94.1
exactly:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Repository Python, dependency-policy, vulnerability, compatibility-inventory,
and release-binary smoke gates are also required by the delivery workflow.
The current `machine-god` CLI bytes must be exercised through the freshly built
`target/release/machine-god` binary even though this candidate adds no CLI
behavior.

The initial 39 focused direct/engine/root/host integration tests and five
private matcher/schema unit tests were green at exact composed SHA `60070d8`.
After finding hardening, 40 focused integration tests and nine private matcher/
schema/budget unit tests are green at replacement code-and-test SHA `4171a4a`.
Formatting, workspace/all-target/all-feature warnings-denied Clippy, workspace
all-feature tests, documentation tests, dependency policy, the 1,225-advisory
vulnerability check over 175 dependencies, the fresh pinned-fx compatibility-
inventory check, and release-binary bare/help/status smoke are also green there
under Rust and Cargo 1.94.1 exactly. The 129-test repository Python gate is
green with eight expected macOS skips. Linux, FreeBSD, and WASI no-default-
feature native library checks pass; WASI retains the existing non-fatal
`read_file` `check_cancellation` dead-code warning. This slice changes no
benchmark behavior.

## Required adversarial tracks

After the exact composed behavior SHA passes the local gates, three fresh agents
must independently review that same SHA:

1. correctness, public API and schema, bytewise matcher semantics, both exact
   result modes, scan and output bounds, independent tests, and Linux/macOS
   portability;
2. recursive filesystem authority, fresh-root liveness, descriptor-relative
   no-follow traversal, symlink/special handling, TOCTOU and race semantics,
   redaction, resource exhaustion, cancellation, and absence of detached work;
   and
3. core permission separation, prepared-policy/execution agreement,
   reference-host and prepared-root identity composition, documentation,
   deferred scope, CLI non-change, compatibility non-claim, and benchmark/
   performance non-claim.

Every confirmed finding must be fixed and rereviewed until all three tracks are
green on the same exact composed behavior SHA. A green track on a different
behavior SHA does not satisfy this gate. Per the user's explicit instruction,
a later documentation-only seal or delivery-evidence record is exempt from a
new adversarial cycle after production behavior is already green; exact feature
and `main` delivery gates remain mandatory for that later SHA.

## Review results

The first formal cycle reviewed exact SHA
`1f5de6ac45e292f38b1853ab0a6212de70f4cc51`:

- **Correctness/API/matcher semantics — GREEN.** The reviewer reproduced 29
  focused direct/engine tests and five private tests, differentially checked
  344,043 component and 1,596,221 path cases, and passed Linux, FreeBSD, WASI,
  and macOS target checks.
- **Filesystem and resource robustness — NOT GREEN.** The reviewer confirmed a
  high finding: the accepted 4,096-byte pattern, depth-256, and 100,000-entry
  bounds permitted roughly 13.2 billion unmetered path-matcher DP cells. A
  1,000-leaf release diagnostic took 12.88 seconds rather than the simple
  pattern's 0.253 seconds, and one synchronous poll could therefore monopolize
  its executor thread between cancellation checks.
- **Composition, documentation, and evidence — GREEN.** Permission separation,
  exact prepared arguments, exports, alphabetical catalog, original-plus-three-
  clone identity, tests, lineage, CLI non-change, and compatibility/benchmark/
  performance non-claims all passed.

The high finding was accepted. `f3fd13b` adds a public checked 8,388,608-step
aggregate matcher budget covering slashful candidate splitting, all pattern/
DP-state visits, and component-byte matcher transitions; overflow or the next
step fails both modes as `glob_files_scan_limit` without partial output. Its
nine private tests cover exact-cap, overflow, detailed accounting, semantics,
and 100,000 simple candidates. Independent commit `825bbd3` adds one depth-256,
64-leaf regression that was red before the fix in both modes while its simple
scan succeeded. The fix and regression compose through `aba2821`; `4171a4a`
also binds the exported public cap in the integration contract test.

All three fresh replacement tracks are green on exact SHA
`523df85822a27102d7e7100e274e3bad7b25494f`:

- **Correctness, API, matcher semantics, and portability — GREEN.** The track
  reproduced all 40 focused integration and nine private tests, passed 344,043
  component and 198,560 slashful differential cases, verified exact inclusive
  metering and overflow behavior, and passed Linux, FreeBSD, WASI, and macOS
  checks. Matcher semantics are unchanged and the both-mode regression fails
  closed without partial output.
- **Filesystem and resource robustness — GREEN.** The prior high finding is
  closed: one shared checked meter covers slashful splitting, every pattern/
  DP-state visit, every component transition, and slash-free matching. The
  practical depth-256 regression completed in 0.67 seconds including fixture
  setup, two successful simple scans, and both bounded failures. Fresh-root,
  no-follow traversal, races, invalid names, redaction, descriptor cleanup,
  cancellation/drop behavior, and portable stubs have no finding. Count mode's
  unnecessary bounded 100-path heap is recorded as an optimization opportunity,
  not a blocker.
- **Composition, documentation, and evidence — GREEN.** The new public cap,
  exact accounting contract, permission separation, prepared agreement,
  catalog/root identity, initial/fix/test/composed lineage, 40-plus-nine
  evidence, first-cycle rejection, non-claims, and maintained status all agree.

Per the user's instruction, documentation-only review seals and delivery-
evidence records after this exact behavior became green do not receive another
adversarial cycle. Exact feature and `main` gates remain mandatory.

## Remote delivery gates

**GREEN.** Documentation seal `35c853605077f2ac700f4be1dd79eabd2ace4dd4`
was pushed without force to `agent/m03-glob-files`. Feature CI run
`32610950593` and feature benchmark-evidence run `32610950594` succeeded on
that exact SHA. The branch was then fast-forwarded without force to `main`;
exact main CI run `32611208411` and main benchmark-evidence run `32611208415`
succeeded on the same integrated SHA. Benchmark workflow success is delivery
evidence only and does not create a product-performance claim. No package was
published and no GitHub release was created.

This documentation-only final delivery record is exempt from another
adversarial cycle because exact production behavior was already green under all
three replacement tracks. Its own exact feature and `main` workflows are
reported at handoff.
