# Milestone 03 native `glob_files` review 01

Status: **CANDIDATE — composition, local gates, three formal reviews, and
remote exact-SHA delivery pending**

## Candidate

- Exact base: `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`
- Production component: **PENDING — isolated production SHA will be recorded
  only after composition**
- Independent-test component: **PENDING — isolated test SHA will be recorded
  only after composition**
- Documentation component: **PENDING — isolated documentation SHA will be
  recorded only after composition**
- First fully composed behavior candidate: **PENDING**
- Local-gate precursor: **PENDING**
- Formal review head shared by all three tracks: **PENDING**
- Finding-fix and rereview heads: **PENDING if required**
- Documentation seal: **PENDING**
- Feature and `main` delivery evidence: **PENDING**
- Delivery branch: `agent/m03-glob-files`
- Required toolchain: Rust and Cargo 1.94.1 exactly

No pending SHA above is inferred, abbreviated, or invented. The coordinator
will replace a placeholder only after the named component or composed state
exists. This eighteenth bounded Milestone 03 candidate adds a read-only
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
  100,000 non-dot entries, 16 MiB of aggregate raw entry-name bytes, and child-
  directory traversal depth 256. The selected root is depth 0; a directory at
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

The coordinator will compose the isolated commits onto
`agent/m03-glob-files` without reverting unrelated work. Production behavior,
independent evidence, and documentation must be present together before formal
review begins. Isolated component SHAs remain explicitly pending until that
composition exists.

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
and release-binary smoke gates also remain required by the delivery workflow.
The current `machine-god` CLI bytes must be exercised through the freshly built
`target/release/machine-god` binary even though this candidate adds no CLI
behavior. No local result is claimed until it is recorded against an exact
composed SHA.

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

**PENDING.** No formal adversarial review has begun, and no track is represented
as green. Findings, fixes, rejected rationales, exact rereview SHA, and track
outcomes will be recorded here after the composed behavior candidate passes its
local gates.

## Remote delivery gates

**PENDING.** The composed branch must be pushed without force, wait for feature
CI and benchmark-evidence workflows that report the exact pushed SHA, and be
fast-forwarded to `main` only after all local and adversarial gates are green.
Exact `main` CI and benchmark-evidence workflows must then report that same
integrated SHA. Benchmark workflow success is delivery evidence only and does
not create a product-performance claim. No package publication or GitHub
release is authorized.
