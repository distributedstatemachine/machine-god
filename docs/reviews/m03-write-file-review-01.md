# Milestone 03 native `write_file` review 01

Status: **FORMAL REVIEW CANDIDATE — formal reviews, seal, and delivery are
pending**

## Base and prior delivery

- Exact base: `bc042536eb3a40d75ccf4d1fe52032b31defac04`.
- Contract commit: `3ee52fd8393bfb86f11048eaa6c624bd18a78798`.
- Contract feature CI `32626410935`: **GREEN** on exact contract commit.
- Contract benchmark evidence `32626410931`: **GREEN** on exact contract
  commit.
- `grep_files` final-record feature CI `32624393377` and benchmark evidence
  `32624393386`: GREEN on exact base.
- `grep_files` final-record main CI `32624645663` and benchmark evidence
  `32624645667`: GREEN on exact base.
- Integration branch: `agent/m03-write-file`.

## Parallel ownership

- Production owner `agent/m03-write-file-prod` owns native production,
  exports, prepared-root wiring, and reference-host composition.
- Independent-test owner `agent/m03-write-file-tests` owns direct, engine,
  host, portability, and deterministic fault/race evidence.
- Documentation owner `agent/m03-write-file-docs` owns the normative contract,
  maintained indexes and plan status, and this review record.

The three component branches started from the exact contract commit and owned
non-overlapping files. Their commits are not behavior candidates in isolation.
The integration branch has composed all three without overwriting one another;
formal review remains pending until final local-gate evidence freezes one exact
SHA.

## Frozen boundary

The normative contract is [`write-file.md`](../write-file.md). It freezes:

- strict `path` and `content` input with independent 4,096-byte path, 49,152-byte
  raw content, and 65,536-byte serialized argument limits;
- exact `FilesystemAccess::Write` preparation and canonical execution agreement;
- Linux/macOS retained-descriptor, no-follow, existing-parent-only execution;
- private same-parent staging, bounded chunked writes, final mode, file and
  directory durability, `NOREPLACE` creation, and atomic-path replacement;
- exact modes, output, redacted errors, cancellation, ambiguity, and cleanup;
- disclosed target, parent, and cleanup races rather than inode-CAS or perfect
  identity-safe-unlink claims; and
- six-tool alphabetical reference-host composition.

The contract deliberately rejects parent creation, external paths, symlink or
special targets, target-content reads, no-op equality reads, ACL/xattr/ownership
preservation, automatic retry after ambiguous commit, and complete fx
equivalence.

## Required independent evidence

Production and test owners remained separate. The composed branch supplies the
following candidate evidence; formal same-SHA review remains pending:

- [x] Exact public symbols, constants, tool/schema descriptions, strict
  arguments, result shape, open errors, tool errors, and redacted debug/display.
- [x] Exact and one-over path, component, raw-content, serialized-argument,
  write-chunk, temporary-attempt, and serialized-result boundaries.
- [x] Effect-free normalized preparation and exact
  `FilesystemAccess::Write` policy/execution agreement, including engine denial
  before any filesystem mutation.
- [x] Empty/NUL/Unicode/maximum content, create, replace, identical replacement,
  atomic descriptor visibility, hostile-umask `0644`, rwx preservation, and
  special-bit stripping.
- [x] Missing parents, every symlink position, final special objects, retained-
  root replacement/removal, and unchanged outside sentinels.
- [x] Deterministic target appearance/replacement, parent identity change/move,
  staged-name replacement, eight collisions, collision preservation, cleanup
  swap protection, and residue handling.
- [x] Injected write/chmod/file-sync/rename/directory-sync failures establish
  unchanged-target precommit behavior and post-rename commit ambiguity.
- [x] Cancellation at every stated boundary, inert-until-poll/drop behavior,
  engine same-poll post-effect recovery, and absence of detached work.
- [x] Exact six-tool alphabetical host catalog, original-plus-five-clone
  workspace identity, Linux/macOS behavior, FreeBSD/WASI compilation, and an
  active unsupported-target construction test.

## Formal gates

Before delivery, one exact composed SHA must pass local Rust 1.94.1 formatting,
warnings-denied workspace Clippy, workspace and documentation tests, repository
Python and compatibility checks, dependency policy/audit, Linux/macOS native
tests, FreeBSD/WASI checks, a clean locked release smoke, and three fresh
same-SHA adversarial tracks for correctness/API, filesystem/robustness, and
performance/concurrency. Every finding restarts all three tracks. A later
documentation-only seal or delivery record is exempt from another adversarial
cycle under the user's instruction, but exact feature and `main` workflows are
still required.

## Pending lineage

- Exact delivered base: `bc042536eb3a40d75ccf4d1fe52032b31defac04`
- Frozen contract commit: `3ee52fd8393bfb86f11048eaa6c624bd18a78798`
- Contract feature CI: `32626410935` — **GREEN** on exact contract commit
- Contract benchmark evidence: `32626410931` — **GREEN** on exact contract
  commit
- Production component: `e9b3ad8e8bd3ab831d93178eea583b09782f5f69`,
  composed as `c0d555b`
- Independent-test component: `59a06a34c25afce4289d7c7b9d214cef9d89a8e8`,
  composed as `c4c5ce6`
- Documentation component: `9285fe900dbc019edeb26b89c97a8fda6855447b`,
  composed as `de46c3e`
- Retained-root fixture correction: `85099337520a4215ee3d2a24b638dfbd8c8ca187`
- Deterministic seam hardening: `1d30ff906017fbb592062dde0f44ae71c492e1d3`,
  composed as `a9a7c99`
- Core same-poll recovery regression:
  `8b5847f355e685a145557e98b1719cf1e154ae83`
- Full-pipeline fault and phase evidence:
  `c7fdef2d65a3f498673dde470a09bbda4a547b59`
- Local-gate-green behavior precursor:
  `072bd69eb6f73944d1db00363da0f965f09dda9f`
- First exact behavior candidate: the tree-identical marker immediately after
  the local-evidence commit; its exact SHA is supplied to every formal track
  and will be retained by the documentation-only seal
- Three formal adversarial reviews: **PENDING**
- Behavior-green SHA: **PENDING**
- Documentation seal: **PENDING**
- Feature CI and benchmark evidence: **PENDING**
- Fast-forward `main` and exact workflows: **PENDING**

Every placeholder must be replaced only with directly observed evidence. A
branch tip, tree identity, another component's checks, or an earlier review
cannot be used to infer an identifier or green status.

The two contract workflows validate the documentation kickoff only. They do
not satisfy any production, independent-test, composed behavior, adversarial,
feature-delivery, or `main` gate.

## Preformal evidence audit

Two read-only preformal audits inspected exact composed SHA
`85099337520a4215ee3d2a24b638dfbd8c8ca187`. They found no production
confinement, atomicity, durability-classification, bound, or liveness defect,
but both correctly reported that mapper-only and indirect tests did not prove
several fault, race, cancellation, and unsupported-target branches through the
real pipeline. These audits are not formal adversarial tracks and do not count
toward the required same-SHA green cycle.

The confirmed evidence gaps are closed before the first formal candidate:

- `a9a7c99` proves exact collision counts, same-mode inode replacement, final-
  parent identity changes, bounded partial/interrupted writes, cancellation
  between partial writes, and file/directory-sync retry and error semantics
  through production-used helpers;
- `8b5847f` drives the real tool through the engine's same-poll post-effect
  cancellation path and proves live committed bytes plus the exact durable
  `tool_result_unknown` placeholder;
- `c7fdef2` drives both `fchmod` stages, write, staged-file sync, create and
  replace rename, staged-name tampering, traversal cancellation, final
  prepublish cancellation, and post-rename parent-sync failure through the
  production pipeline, and also classifies `/dev/null` as a rejected device
  target; and
- `a717e22` corrects the maintained contract, plan, and review lineage from the
  contract-only five-tool/pending state to the composed six-tool candidate.

Final exact local gates and the three formal same-SHA review tracks remain
pending; no preformal result is promoted into delivery evidence.

## Local gate results

Exact composed precursor `072bd69eb6f73944d1db00363da0f965f09dda9f`
is green under Rust and Cargo 1.94.1 exactly:

- formatting and workspace/all-target/all-feature warnings-denied Clippy pass;
- the workspace all-target/all-feature inventory and gate pass 651 tests plus
  two doctests;
- focused evidence passes 23 private `write_file` tests, 25 direct integration
  tests, five real-engine tests including same-poll recovery, seven reference-
  host tests, and three prepared-root tests;
- the repository Python gate runs 129 tests: 121 pass and eight expected macOS
  skips, with zero failures or errors;
- a fresh credential-stripped checkout of pinned fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes compatibility-inventory
  generation check;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings;
- Linux no-default native Clippy passes with warnings denied; FreeBSD
  no-default library/tests compile and library Clippy passes with only two
  pre-existing test-only `glob_files` warnings; WASI builds the dedicated
  unsupported-target test with only the pre-existing `read_file` dead-code
  warning, and Node's WASI runner actively passes that exact test 1/1;
- a fresh locked release build passes and produces an arm64 Mach-O CLI with
  SHA-256 `57025124a24a636e7fc9639ca5a0f53c75690d3610667d11cc87f2eaeda7f494`;
  executing that binary directly prints the exact version/API line and exits
  zero; and
- 60 Markdown files contain 428 inline links, including 278 repository-relative
  links with zero missing; whole-feature diff checks pass, no unsafe Rust was
  added, and the exact precursor worktree is clean.

These are local precursor results, not formal-review or remote-delivery
evidence. The immediately following tree-identical marker freezes the exact
first formal behavior candidate supplied independently to all three fresh
review tracks.

## Explicit nonclaims

This contract adds no CLI behavior, parent creation, external-path access,
symlink-target mutation, target-content read, append/patch operation,
ownership/ACL/xattr preservation, non-Linux/macOS hardened execution,
benchmark workload, compatibility-status change, fx-equivalence claim, or
product-performance claim. Zig remains only the pinned fx benchmark build
input; machine-god and this tool remain Rust.
