# Milestone 03 native `edit_file` review 01

Status: **IN PROGRESS — contract-only kickoff; production not composed**

## Base and kickoff

- Exact delivered base:
  `242adfed4be717baf7cd07275aae40ec8a3637f6`.
- Integration branch: `agent/m03-edit-file`.
- Normative contract: [`edit-file.md`](../edit-file.md).
- Contract kickoff: this documentation-only commit; record its exact SHA only
  after commit.
- Contract feature CI: pending push and exact-SHA workflow.
- Contract benchmark evidence: pending push and exact-SHA workflow.

This kickoff changes documentation only. Per the user's explicit instruction,
it is exempt from adversarial review. It does not establish implementation,
independent evidence, composed behavior, compatibility, performance, or
delivery.

## Frozen boundary

The contract freezes:

- strict effect-free `{path,old_string,new_string}` preparation;
- `FilesystemAccess::Edit`, serialized as `edit`, rather than widening the
  delivered no-read `Write` authority;
- independent 4,096-byte/256-component path, 49,152-byte old/new/preimage/
  postimage, 65,536-byte serialized-input, 8,192-byte chunk, eight-name,
  16-interruption, 31-entropy-call-per-name, and 393,216 matcher-work limits;
- valid-UTF-8 existing content, exact byte matching, overlapping ambiguity,
  NUL acceptance, empty replacement, and exact success path/byte count;
- Linux/macOS retained-descriptor, no-follow, existing-regular-file-only,
  same-parent staged atomic replacement with original ordinary rwx bits;
- bounded target/content/parent/staged-name revalidation, cancellation,
  cleanup, race disclosure, and post-rename ambiguity semantics; and
- exact seven-tool alphabetical reference-host composition.

Preparation remains effect-free, so this slice deliberately defers pinned fx's
preapproval preimage read and computed diff. It also defers external paths,
parent or file creation, binary/alternate-encoding edit, regex/patch/range/
multi-edit, metadata preservation beyond ordinary rwx bits, CLI changes,
non-Linux/macOS hardening, compatibility promotion, benchmark changes, and any
performance or equivalence claim.

## Parallel ownership

Three owners start from the exact contract kickoff and use isolated worktrees
with non-overlapping ownership. They must not overwrite or revert one another.

### Production owner

Owns native/core production, exports, private staging extraction when needed,
prepared-root wiring, and reference-host composition:

- `crates/machine-god-core/src/permission.rs`
- `crates/machine-god-core/tests/contracts.rs`
- `crates/machine-god-native/src/edit_file.rs`
- `crates/machine-god-native/src/lib.rs`
- `crates/machine-god-native/src/workspace.rs`
- `crates/machine-god-native/src/reference_host.rs`
- narrowly extracted package-private staging modules and only the mechanical
  `write_file.rs` changes required to use them

The owner should extract and reuse already-reviewed staging/publication
mechanics where practical instead of independently reproducing the hardened
writer pipeline. Every observable `write_file` behavior must remain unchanged.

### Independent-test owner

Owns black-box, engine, host, portability, and deterministic fault/race
evidence without editing production:

- `crates/machine-god-native/tests/edit_file.rs`
- `crates/machine-god-native/tests/edit_file_engine.rs`
- `crates/machine-god-native/tests/edit_file_unsupported.rs`
- edit-specific host catalog and descriptor-clone assertions in
  `crates/machine-god-native/tests/reference_host.rs`

If existing integration-test locations differ, the owner may use the matching
repository test paths but must not edit production or the documentation-owned
files. Tests must exercise public behavior and production-used deterministic
seams rather than restating private mappers.

### Documentation owner

Owns the normative contract, review lineage, maintained plan status, and
documentation indexes:

- `docs/edit-file.md`
- `docs/reviews/m03-edit-file-review-01.md`
- `docs/implementation-plan.md`
- `docs/README.md`
- `docs/reviews/README.md`

Production, independent-test, and documentation commits are component inputs,
not behavior candidates in isolation. Formal review begins only after all are
composed and the exact branch is clean and locally green.

## Required independent evidence

- [ ] Exact public symbols, constants, schema/property descriptions,
  construction/tool errors, serialized forms, and redacted debug/display.
- [ ] Strict requested/canonical input, path and component boundaries,
  independent text and serialized limits, empty old string, identical strings,
  and preparation with zero filesystem effects.
- [ ] Exact `FilesystemAccess::Edit` capability and canonical policy/execution
  agreement, including denial before target read and strict direct execution.
- [ ] Beginning/middle/end, Unicode, NUL, empty replacement, complete deletion,
  exact size boundaries, zero/one/two matches, and overlapping ambiguity.
- [ ] KMP-like linear work accounting at exact/one-over public cap with
  cancellation checks no farther apart than the contract permits. Public legal
  input stays below the cap; private injected-budget tests prove the defensive
  exact/one-over boundary and fixed `edit_file_match_work_exceeded` error.
- [ ] Existing regular-file-only behavior, missing target/parents, invalid UTF-
  8, oversize/growing content, ancestor/final symlinks, and all special types.
- [ ] Exact original ordinary-rwx preservation under hostile umask, inode
  replacement, old-descriptor/new-path visibility, hard-link behavior, and
  deliberate nonpreservation of other metadata.
- [ ] Retained-root changes and deterministic target identity/mode/content,
  staged-name, and parent races through the real production pipeline.
- [ ] Every read/match/construction/write/chmod/file-sync/rename/parent-sync
  fault with unchanged-target precommit and nonretryable postcommit ambiguity.
- [ ] Cumulative interruption bounds, entropy partial progress and exhaustion,
  eight collisions, collision preservation, cleanup swaps, held-descriptor mode
  reset, and disclosed residue dual-failure behavior.
- [ ] Cancellation during both reads, matching, construction, traversal,
  entropy/staging, final verification, immediately before rename, unpolled/
  drop, and engine same-poll durable recovery.
- [ ] Exact seven-tool alphabetical host catalog, original-plus-six-clone
  descriptor identity, complete `write_file` regression, Linux/macOS native
  behavior, FreeBSD/WASI compilation, and active unsupported behavior.
- [ ] Private production-helper evidence proves the exact 16,384-byte
  serialized-result guard because every public success payload is much smaller.

## Formal gates

Focused suites run first. One exact composed behavior SHA must then pass Rust
and Cargo 1.94.1 formatting, workspace all-target/all-feature warnings-denied
Clippy, workspace and documentation tests, repository Python and pinned-fx
compatibility checks, dependency policy/audit, Linux/macOS native execution,
FreeBSD/WASI checks, documentation links, no-unsafe/diff checks, and a freshly
built locked release CLI smoke.

After those gates, three fresh agents independently inspect the same exact SHA:

1. correctness and public API;
2. filesystem confinement, atomicity, durability, and robustness; and
3. performance, bounded work, cancellation, and concurrency.

Every confirmed finding is fixed and restarts all three tracks with fresh
agents on one new exact behavior SHA. Delivery requires all three green, exact
feature-branch workflows, a no-force fast-forward of `main`, and exact `main`
workflows. Documentation-only kickoff, seal, and delivery records are exempt
from another adversarial cycle under the user's instruction, while their own
exact remote workflows remain required.

## Pending lineage

- Exact base: `242adfed4be717baf7cd07275aae40ec8a3637f6`
- Contract kickoff SHA: pending direct observation after commit
- Contract feature CI: pending
- Contract benchmark evidence: pending
- Production component: pending
- Independent-test component: pending
- Documentation component after production: pending
- Local-gate behavior precursor: pending
- Exact formal behavior candidate: pending
- Correctness/API track: pending
- Filesystem/robustness track: pending
- Performance/concurrency track: pending
- Behavior-green SHA: pending
- Documentation seal: pending
- Exact feature CI and benchmark evidence: pending
- No-force fast-forward `main`: pending
- Exact `main` CI and benchmark evidence: pending

Every identifier and result must be replaced only with directly observed
evidence. A component branch, tree-similar predecessor, earlier slice, or docs-
only kickoff cannot stand in for a composed exact behavior SHA or green gate.

## Pinned input and deliberate differences

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` confirms
only the `path`, `old_string`, and `new_string` compatibility target. Its 4 MiB
limits, permissive unknown fields, non-overlapping ambiguity, invalid-UTF-8 byte
editing, external paths, textual result, and preapproval diff behavior are not
adopted. Zig remains only that pinned upstream benchmark's build input; all
machine-god product implementation remains Rust.
