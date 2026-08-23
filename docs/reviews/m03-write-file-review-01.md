# Milestone 03 native `write_file` review 01

Status: **FROZEN CONTRACT — production, independent tests, composition, formal
reviews, seal, and delivery are pending**

## Base and prior delivery

- Exact base: `bc042536eb3a40d75ccf4d1fe52032b31defac04`.
- `grep_files` final-record feature CI `32624393377` and benchmark evidence
  `32624393386`: GREEN on exact base.
- `grep_files` final-record main CI `32624645663` and benchmark evidence
  `32624645667`: GREEN on exact base.
- Integration branch: `agent/m03-write-file`.

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

Production and test owners must be separate. Deterministic seams must exercise
target appearance, target replacement, parent movement, staged-name replacement,
temporary collisions and cleanup, every precommit fault, directory-sync
ambiguity, and cancellation before the irreversible rename boundary. Tests must
also prove exact schema/caps, empty and NUL content, mode behavior under hostile
umask, old-descriptor/new-path atomic visibility, engine policy ordering,
post-effect recovery, cross-target compilation, active unsupported behavior,
and exact host catalog/descriptor identity.

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

- Contract commit: **PENDING**
- Production component: **PENDING**
- Independent-test component: **PENDING**
- Documentation component: **PENDING**
- First exact behavior candidate: **PENDING**
- Three formal adversarial reviews: **PENDING**
- Behavior-green SHA: **PENDING**
- Documentation seal: **PENDING**
- Feature CI and benchmark evidence: **PENDING**
- Fast-forward `main` and exact workflows: **PENDING**

