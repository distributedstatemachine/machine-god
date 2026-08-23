# Milestone 03 native `delete_file` review 01

Status: **IN PROGRESS — formal cycle 1 not green; remediation in progress**

## Base and contract gate

- Exact delivered base:
  `719a9bded86fd7ce394d482798b9064c736f43ab`.
- Integration branch: `agent/m03-delete-file`.
- Normative contract: [`delete-file.md`](../delete-file.md).
- Contract commit: `78ed6292386f86e5807bcf72591d6cb5d9f45c45`.
- Exact contract CI: workflow `32652361712`, green across all six jobs.
- Exact contract benchmark evidence: workflow `32652361692`, green across both
  jobs with two nonexpired exact-SHA artifacts.

The exact base is green under feature CI `32651168514` across all six jobs and
feature benchmark workflow `32651168515` across both jobs with two nonexpired
exact-SHA artifacts. `main` was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to the exact base and is green
under main CI `32651488265` across all six jobs and main benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

This contract commit is documentation-only and is exempt from adversarial
review under the user's explicit instruction. Its workflows establish only the
frozen documentation boundary, not implementation, behavior, compatibility,
performance, equivalence, or delivery. Production and independent evidence are
now composed through exact local-gate precursor
`5e340155f9a38b81a2812942d6ad0a796164beb5`.

## Frozen boundary

The contract freezes:

- library-only deletion of exactly one existing confined regular file or empty
  directory, with no recursion, root deletion, content read, enumeration,
  creation, symlink following, external path, or CLI change;
- strict `{"path":string}` input, delivered mutation-path normalization, exact
  `FilesystemAccess::Delete`, and independent requested/canonical path,
  component, serialized-input, and serialized-result limits;
- exact path-only success, public symbols, construction errors, fixed redacted
  tool errors, retryability, and Linux/macOS platform scope;
- retained-root and parent descriptor traversal, no-follow metadata, complete
  parent/target identity and type revalidation, and a final cancellation check
  immediately before exactly one type-directed `unlinkat`;
- no `unlinkat` retry after `EINTR`, cancellation-ignoring bounded parent sync,
  and explicit commit-ambiguity recovery semantics;
- the portable final check-to-delete different-entry race, retained-parent
  movement, hard-link/open-descriptor survival, and pathname recreation; and
- exact eight-tool alphabetical reference-host composition with one additional
  retained descriptor clone.

There is no staging, ACL, chmod, content buffer, rename, recursive walk, or
directory enumeration. The slice adds no new dependency, benchmark workload,
compatibility promotion, product-performance claim, or fx-equivalence claim.

## Parallel ownership

- Production owns native implementation, exports, prepared-root/reference-host
  wiring, and any narrow existing-`Delete` contract regression.
- Independent tests own direct, engine, host, portability, fault, race, bounds,
  cancellation, and unsupported-target evidence.
- Documentation owns the normative contract, plan/index status, and this
  lineage record.

Owners use isolated worktrees and non-overlapping files. Component commits are
not behavior candidates independently; the integration branch must compose all
required behavior and evidence before formal review.

## Required production evidence seams

The real execution path must expose statically dispatched, deterministic seams
for:

- initial and final root/intermediate opens;
- ordinal-aware descriptor `fstat` and no-follow target `statat` calls;
- checkpoints after initial validation and after final validation;
- the actual `unlinkat` flags and syscall outcome;
- the checkpoint immediately after a real deletion; and
- parent `fsync`, including cumulative interruption outcomes.

Production behavior must remain identical with the no-op evidence type. No
global mutable seam, release-path dynamic dispatch, unbounded retry, or
test-only alternate executor is accepted.

## Required independent evidence

- [x] Exact public API, constants, schema, descriptions, result, construction
  taxonomy, fixed tool errors, retryability, `Display`, and redaction.
- [x] Exact/one-over requested and canonical path bytes, 256 components,
  65,536-byte serialized arguments, and 16,384-byte serialized result.
- [x] Effect-free strict preparation, denied execution before lookup, exact
  `Delete` capability, and canonical policy/direct-execution agreement.
- [x] Native regular-file and empty-directory deletion without content reads or
  enumeration; nonempty directory and root removal rejection.
- [x] Missing ancestors/targets, all symlink positions, FIFO/socket/device/
  other special objects, and outside sentinels fail closed without blocking.
- [x] Retained-root changes, complete parent/target rewalk and revalidation,
  moved retained parent, final different-entry race, hard links/open
  descriptors, and post-success pathname recreation.
- [x] Root/intermediate-open and ordinal `fstat`/`statat` faults, exact type
  flags, one actual `unlinkat`, post-delete checkpoint, and parent-sync faults
  all route through the real production path.
- [x] `unlinkat` success, definitive failures, and `EINTR` have exact mappings;
  interruption makes no second delete call; parent sync has one cumulative
  16-interruption ceiling and postcommit failures are nonretryable ambiguity.
- [x] Cancellation is covered through both traversal/validation passes, at the
  exact final pre-delete boundary, and after a real delete, plus inert-until-
  poll, drop, and core same-poll unknown-result recovery.
- [x] The exact eight-tool alphabetical catalog and original-plus-seven-clone
  retained identity are green without changing CLI bytes.
- [x] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported-target,
  seven-tool regression, no-unsafe, docs, dependency, compatibility, and fresh
  release-smoke gates are green.

## Composed lineage and local gate

Production component `d31d0656528988f57caaecbacf15453f129ab27e`, independent
black-box component `2d2d8502a7b12ac6d9baeb983b06e604b58b2cde`, private
precommit component `e36cbd780d47fc847121ac3da75ec8a4649cd11e`, and private
commit-race components `f9464d736940b87a1e38948244d871d4497892eb`,
`829fff21c447dd7333b8042431932589e51f0801`, and
`af3d0ba7cd8ada7aa9b80794e786782dc5ba33df` compose through exact clean
precursor `5e340155f9a38b81a2812942d6ad0a796164beb5`. Deterministic evidence exposed
and closed the `ELOOP` and macOS `EPERM` type-race mappings before formal
review.

Under Rust and Cargo 1.94.1, formatting, workspace all-target/all-feature
warnings-denied Clippy, workspace tests, and two doctests are green. Focused
totals are 19 default-feature and 20 all-feature private tests, 19 direct, five
engine, and seven reference-host tests. Discovery reports 728 default-feature
tests, 778 all-feature tests, and zero benchmarks.

All 130 Python tests pass with eight expected macOS skips. Pinned-fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, cargo-deny 0.20.2,
cargo-audit 0.22.2, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity is 64 Markdown files, 445 inline links,
295 repository-relative links, and zero missing targets. Diff/no-unsafe/Cargo/
CLI checks are clean. A fresh locked arm64 Mach-O release CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`
and passes bare, help, and status smoke paths.

The tree-identical behavior marker
`7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf` became formal cycle 1's exact
candidate. All three fresh tracks reviewed detached clean worktrees at that
same SHA and reported **NOT GREEN**. No candidate or delivery claim is made.

## Formal adversarial cycle 1

Exact candidate: `7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf`.

1. Correctness/API: **NOT GREEN**, with two medium findings. Retained-root
   acquisition and linked-root metadata discarded `EACCES`/`EPERM`, violating
   the fixed nonretryable permission taxonomy in both validation phases. The
   macOS regular-file `EPERM` diagnostic metadata call also bypassed the frozen
   cancellation checks and could suppress precommit cancellation after a
   definitive failed unlink.
2. Filesystem/robustness: **NOT GREEN**, with two medium findings. It confirmed
   the retained-root permission mismatch independently and demonstrated that
   empty-flag `unlinkat` can remove a final-window symlink replacement, while
   the contract incorrectly characterized type changes as generally failing.
3. Performance/concurrency: **NOT GREEN**, with one medium finding. Serialized
   argument accounting preceded the requested path's 4,096-byte rejection, so
   a direct caller could force arbitrarily input-sized synchronous JSON string
   scanning before the fixed path bound fired.

The four unique remediations are: validate the requested path bound before
serialized-size accounting; preserve permission errors across retained-root
operations in either phase; make the macOS diagnostic metadata cancellation-
aware with cancellation precedence after a definitive noncommit; and freeze
and test the portable file-class replacement boundary for symlink, FIFO, and
socket entries with referent/sentinel preservation. A new candidate receives
the complete local gate and three entirely fresh same-SHA reviewers.

## Formal adversarial protocol

After all implementation and evidence compose into one exact behavior SHA,
three fresh agents review that same SHA for:

1. correctness and public API;
2. filesystem behavior and robustness; and
3. performance and concurrency.

Each track must explicitly report **GREEN** with zero findings or **NOT GREEN**
with every finding. Any finding is remediated, all local gates rerun, and all
three tracks restart with fresh agents on the same new SHA. A later docs-only
seal or delivery record is exempt from another adversarial cycle under the
user's instruction, while exact feature and `main` workflows remain mandatory.

## Pinned input and non-claims

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` was observed
to accept a regular file and empty directory but has broader pathname-based
behavior. Machine-god's stricter bounds, retained-descriptor confinement,
revalidation, cancellation, redacted errors, durability, and race disclosure
are deliberate differences.

The contract makes no compatibility promotion, benchmark-workload change,
product-performance claim, or fx-equivalence claim. Zig remains solely the
pinned upstream benchmark build input; the implementation remains Rust.
