# Milestone 03 native `file_info` review 01

Status: **DELIVERED — replacement reviews and exact feature and `main`
workflows green**

## Candidate

- Base: `41859e5b232f1d8d285af2df082529125f8c004a`
- Production component: `5c2d129a3755dca0c8f7913b27614b70352fe2a4`
- Production-only composed head: `1d93a650afdd022964b657fc93352872b8d380df`
- Independent-test component: `ca0091c181d8ffbecda008ee0f981516dc5cff7b`
- Production-and-test composed head: `f228c06bbda5d01b50905c66f378a2b29e0560bf`
- Documentation component: `b5d30d617378d09ec24552844c6233ef25ba1aa4`
- First fully composed candidate: `8ceef6d2c4193902c5750baea663dfe5bc396863`
- Isolated test-evidence documentation follow-up:
  `039dd03ce285c46a677062e18e8953776afcdc6d`
- Composed local-gate precursor: `0973acfaa41b198f952fbdc204ee3d3cc462f2f4`
- First formal review head: `8399ec78450d258570e376e0989639d3f70fc976`
- Documentation finding record: `9dbd1881e511f30998af66ff643a6ccb6757e04b`
- Isolated finding-test hardening:
  `7f2a2924b8e30abdfe572f54dfc51c1bd605a649`
- Composed finding-fix behavior: `b69ec4b9dc46c4d43202a2c0c5ba499fa8fbd071`
- Replacement local-gate precursor:
  `d445eb384ceafad1f107e78d456c93aecc0bc5a2`
- Replacement review head: `4193eccea7d46929a4f2e789f20f508200a346a0`
- Documentation seal and integrated `main` head:
  `60dd54f273afc7e62fb4b3cc1fb1a347d739998b`
- Delivery branch: `agent/m03-file-info`
- Toolchain: Rust and Cargo 1.94.1 exactly

This delivered seventeenth bounded Milestone 03 slice adds the
read-only Linux/macOS `file_info` library tool, a distinct
`FilesystemAccess::Metadata` authorization kind, independent direct and engine
tests, reference-host registration beside `list_files` and `read_file`, and the
maintained contract pages. It does not alter CLI behavior, complete the native
tool inventory or Milestone 03, change the pinned fx inventory or Zig benchmark
setup, or make a compatibility or product-performance claim.

## Candidate behavior to review

- `FileInfoTool::open` accepts an explicitly injected absolute workspace root,
  performs the same lexical host-root cleanup as the existing confined tools,
  and retains one no-follow directory descriptor on Linux and macOS. Other
  targets return a fixed unsupported-platform construction failure.
- Effect-free preflight accepts exactly one required string `path`, bounds it
  to 4,096 UTF-8 bytes, applies the same lexical normalization and character
  rejection as `read_file`, and gives policy and execution the exact same
  normalized path under `FilesystemAccess::Metadata`. A nonempty request made
  only of current-directory components normalizes to `.` so the retained root
  itself can be inspected.
- Each execution acquires a fresh `.` descriptor and validates that exact
  acquired root identity as linked: Linux rejects zero link count, while macOS
  matches its device, inode, and directory type through a descriptor-relative
  parent/name lookup. Stable rename retains identity; removal before acquisition
  or validation is unavailable.
- Allowed execution walks only ancestor directories descriptor-relatively and
  no-follow. It uses one no-follow metadata lookup for the final component and
  does not open that component, so final symlinks report themselves and FIFO,
  socket, device, or other special files cannot block the call.
- The exact structured result contains `path`, `kind`, checked `size_bytes`,
  one `modified` object with signed Unix seconds and validated nanoseconds, and
  a nullable lexical regular-file `extension`. File kind is one of `file`,
  `directory`, `symlink`, or `other`.
- Dotfile and trailing-dot extension behavior is frozen: `.bashrc` and `foo.`
  have no extension, `.config.json` has `json`, and `archive.tar.gz` has `gz`.
  Nonregular objects always have no extension.
- The retained root identity survives host-path rename or replacement.
  Already-opened ancestor identities survive rename. Non-root output uses one
  final no-follow `statat` snapshot; `.` uses one final `fstat` after root
  validation. The contract makes no preflight-time, content, target, or
  continued-existence snapshot claim.
- Construction, preparation, lookup, metadata-validation, and cancellation
  failures use fixed redacted diagnostics. The execution future is inert before
  poll, checks cancellation at bounded syscall boundaries, and detaches no work.
- The result contains only data derived from the one bounded input path and one
  fixed-width metadata record. Its conservative worst-case serialized content
  remains below 17 KiB and core's lower configured result limit still applies.
- The composed native host supplies exactly three workspace tools:
  `file_info`, `list_files`, and `read_file`; core exposes their catalog in
  deterministic alphabetical order. Prepared roots distribute the original
  retained descriptor plus two clones of one workspace identity across all
  three.

## Parallel implementation ownership

Three isolated worktrees own non-overlapping surfaces from exact base
`41859e5b232f1d8d285af2df082529125f8c004a`:

- production owns core permission/API changes, the native tool, native exports,
  and reference-host/root-bundle composition;
- independent tests own direct and real-engine black-box tests; and
- documentation owns the normative contract, maintained indexes and guides,
  plan/status updates, and this review record.

The coordinator will compose those component commits onto
`agent/m03-file-info` without reverting unrelated work. Behavior and its
documentation must be present together before formal review begins.

All 34 focused direct, real-engine, root-preparation, and reference-host tests
are green at production-and-test composed head `f228c06`. The documentation
component and its test-evidence follow-up compose through `0973acf`, where the
required local gates below are green. The formal-review SHA is the
documentation-only descendant that records this evidence; reviewers receive
that exact SHA directly.

## Required adversarial tracks

Three fresh agents must review the same exact composed behavior SHA:

1. correctness, public API, strict schema, result shape, bounds, tests, and
   Linux/macOS portability;
2. filesystem authority, no-follow traversal, special-file behavior, metadata
   validation, redaction, cancellation, and race semantics; and
3. reference-host composition, policy/execution agreement, documentation,
   deferred scope, CLI non-change, and benchmark/compatibility non-claims.

Every confirmed finding must be fixed and rereviewed until all three tracks are
green on the same exact behavior SHA. Per the user's delivery instruction, a
later documentation-only seal or delivery-evidence commit does not require an
additional adversarial cycle after production behavior is green.

## First formal review results

All three tracks reviewed exact candidate `8399ec7`. The security and resource
track was green. The correctness/API and documentation/evidence tracks were not
green, so this candidate cannot be delivered.

The correctness/API track found no production-logic defect, but confirmed
three independent-evidence gaps:

- decorated final-root symlink rejection and decorated real-root acceptance
  were not frozen by direct `FileInfoTool::open` regressions;
- the maximum-result test used an unescaped directory path, a null extension,
  and only the core 64 KiB ceiling rather than proving the documented escaping-
  heavy result remains below 17 KiB; and
- signed pre-epoch output plus invalid size/nanosecond conversion boundaries
  lacked direct evidence.

The documentation/evidence track confirmed two low-severity inconsistencies:

- the implementation plan said the three tools receive one descriptor instance
  even though they receive the original plus two clones of one identity; and
- the README still called the already-green local gates pending.

Documentation record `9dbd188` corrects the two documentation statements.
Independently owned hardening `7f2a292` adds the required decorated-root,
escaping-heavy exact result-bound, signed pre-epoch, and pure invalid-metadata
conversion evidence. It composes as behavior head `b69ec4b`, bringing the
focused direct/engine/root/host suite from 34 to 36 tests plus five private unit
tests. Required replacement local gates are green at `d445eb3`. All three
replacement tracks reviewed the same exact documentation-only descendant
`4193ecc` and are green.

## Replacement rereview results

All three fresh replacement tracks are green on exact candidate `4193ecc`:

- **Correctness, API, portability, and independent evidence — GREEN.** The
  decorated-root regressions pass, the successful escape-heavy 4,096-byte path
  with non-null extension serializes below 17 KiB, signed pre-epoch output is
  preserved, and the production-used pure conversion helper covers public
  integer endpoints plus negative size and invalid nanoseconds.
- **Filesystem security, resources, and concurrency — GREEN.** The finding
  hardening does not regress authority, descriptor-relative no-follow access,
  root liveness, TOCTOU boundaries, special-file behavior, redaction, bounds,
  cancellation, descriptor ownership, or permission/execution agreement.
- **Documentation, evidence, compatibility, and performance — GREEN.** The
  three descriptor instances are described as the original plus two clones of
  one retained identity, local status is current, all maintained links and
  lineage resolve, CLI/benchmark/workflow surfaces remain unchanged, and the
  Rust-product/Zig-upstream-only boundary is intact.

Per the user's instruction, documentation-only seals and delivery-evidence
records that follow the already-green behavior do not receive another
adversarial cycle. Exact feature and `main` delivery gates remained mandatory
and are green below.

## Local and remote delivery gates

The replacement's 36 focused integration tests and five private unit tests,
formatting, workspace/all-target/all-feature
warnings-denied Clippy, workspace tests, documentation tests, dependency policy
and vulnerability checks, pinned compatibility-inventory check, and
release-binary bare/help/status smoke checks are green under Rust and Cargo
1.94.1 exactly at local-gate precursor `d445eb3`. The 129-test repository
Python gate is green with eight expected macOS skips. The first candidate's
Python gate initially had one
load-induced two-second timeout while a release build ran in parallel; that
same regression reran green in isolation before the complete suite was rerun
serially. This slice changes no benchmark behavior.

Documentation seal `60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact
feature CI run `32605071080` on successful retry attempt 2 and exact feature
benchmark-evidence run `32605071063`. It was fast-forwarded without force to
`main` at that same exact SHA and passed exact main CI run `32606050292` and
exact main benchmark-evidence run `32606050294`. All four workflows report exact
seal SHA `60dd54f273afc7e62fb4b3cc1fb1a347d739998b`. Benchmark success is
delivery evidence only and makes no product-performance claim.

This documentation-only commit is the final delivery record. It is explicitly
exempt from another adversarial review because exact production behavior was
already green on all three replacement tracks; its own exact feature and `main`
workflows are reported at handoff. No package or GitHub release is authorized.

The benchmark workflow may continue obtaining Zig only to build the pinned
upstream fx reference. `file_info` adds no benchmark workload or performance
claim, and machine-god remains a Rust product.
