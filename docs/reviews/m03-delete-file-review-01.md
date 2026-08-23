# Milestone 03 native `delete_file` review 01

Status: **CONTRACT FROZEN — implementation and formal review pending**

## Base and contract gate

- Exact delivered base:
  `719a9bded86fd7ce394d482798b9064c736f43ab`.
- Integration branch: `agent/m03-delete-file`.
- Normative contract: [`delete-file.md`](../delete-file.md).
- Contract commit: this documentation-only contract commit; exact SHA is
  recorded by the coordinator after creation.
- Contract CI and benchmark evidence: pending push of the exact contract SHA.

The exact base is green under feature CI `32651168514` across all six jobs and
feature benchmark workflow `32651168515` across both jobs with two nonexpired
exact-SHA artifacts. `main` was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to the exact base and is green
under main CI `32651488265` across all six jobs and main benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

This contract commit is documentation-only and is exempt from adversarial
review under the user's explicit instruction. Its workflows establish only the
frozen documentation boundary, not implementation, behavior, compatibility,
performance, equivalence, or delivery.

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

- [ ] Exact public API, constants, schema, descriptions, result, construction
  taxonomy, fixed tool errors, retryability, `Display`, and redaction.
- [ ] Exact/one-over requested and canonical path bytes, 256 components,
  65,536-byte serialized arguments, and 16,384-byte serialized result.
- [ ] Effect-free strict preparation, denied execution before lookup, exact
  `Delete` capability, and canonical policy/direct-execution agreement.
- [ ] Native regular-file and empty-directory deletion without content reads or
  enumeration; nonempty directory and root removal rejection.
- [ ] Missing ancestors/targets, all symlink positions, FIFO/socket/device/
  other special objects, and outside sentinels fail closed without blocking.
- [ ] Retained-root changes, complete parent/target rewalk and revalidation,
  moved retained parent, final different-entry race, hard links/open
  descriptors, and post-success pathname recreation.
- [ ] Root/intermediate-open and ordinal `fstat`/`statat` faults, exact type
  flags, one actual `unlinkat`, post-delete checkpoint, and parent-sync faults
  all route through the real production path.
- [ ] `unlinkat` success, definitive failures, and `EINTR` have exact mappings;
  interruption makes no second delete call; parent sync has one cumulative
  16-interruption ceiling and postcommit failures are nonretryable ambiguity.
- [ ] Cancellation is covered through both traversal/validation passes, at the
  exact final pre-delete boundary, and after a real delete, plus inert-until-
  poll, drop, and core same-poll unknown-result recovery.
- [ ] The exact eight-tool alphabetical catalog and original-plus-seven-clone
  retained identity are green without changing CLI bytes.
- [ ] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported-target,
  seven-tool regression, no-unsafe, docs, dependency, compatibility, and fresh
  release-smoke gates are green.

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
