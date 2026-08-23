# Milestone 03 native `write_file` review 01

Status: **FROZEN CONTRACT — production, independent tests, composition, formal
reviews, seal, and delivery are pending**

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

The three branches start from the exact contract commit and own non-overlapping
files. Their commits are not behavior candidates in isolation. The integration
owner must compose all three without overwriting one another before any formal
review begins.

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

Production and test owners remain separate. Every item below is pending:

- [ ] Exact public symbols, constants, tool/schema descriptions, strict
  arguments, result shape, open errors, tool errors, and redacted debug/display.
- [ ] Exact and one-over path, component, raw-content, serialized-argument,
  write-chunk, temporary-attempt, and serialized-result boundaries.
- [ ] Effect-free normalized preparation and exact
  `FilesystemAccess::Write` policy/execution agreement, including engine denial
  before any filesystem mutation.
- [ ] Empty/NUL/Unicode/maximum content, create, replace, identical replacement,
  atomic descriptor visibility, hostile-umask `0644`, rwx preservation, and
  special-bit stripping.
- [ ] Missing parents, every symlink position, final special objects, retained-
  root replacement/removal, and unchanged outside sentinels.
- [ ] Deterministic target appearance/replacement, parent identity change/move,
  staged-name replacement, eight collisions, collision preservation, cleanup
  swap protection, and residue handling.
- [ ] Injected write/chmod/file-sync/rename/directory-sync failures establish
  unchanged-target precommit behavior and post-rename commit ambiguity.
- [ ] Cancellation at every stated boundary, inert-until-poll/drop behavior,
  engine same-poll post-effect recovery, and absence of detached work.
- [ ] Exact six-tool alphabetical host catalog, original-plus-five-clone
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
- Production component: **PENDING**
- Independent-test component: **PENDING**
- Documentation component: **PENDING**
- First exact behavior candidate: **PENDING**
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

## Explicit nonclaims

This contract adds no CLI behavior, parent creation, external-path access,
symlink-target mutation, target-content read, append/patch operation,
ownership/ACL/xattr preservation, non-Linux/macOS hardened execution,
benchmark workload, compatibility-status change, fx-equivalence claim, or
product-performance claim. Zig remains only the pinned fx benchmark build
input; machine-god and this tool remain Rust.
