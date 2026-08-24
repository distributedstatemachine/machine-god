# Milestone 03 native `copy_file` review 01

Status: **CONTRACT IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `226040780eb14dd72e86d0a002dc4bf61ba2ddfc`.
- Integration branch: `agent/m03-copy-file`.
- Normative contract: [`copy-file.md`](../copy-file.md).
- Base feature CI `32675981622` and benchmark `32675981593` are green.
- Base main CI `32676296870` and benchmark `32676296945` are green.
- Both benchmark workflows retain exactly two nonexpired exact-SHA artifacts.

This documentation-only contract is exempt from adversarial review under the
user's explicit instruction. Its workflows freeze documentation only; they are
not implementation, behavior, equivalence, performance, or delivery evidence.

## Frozen feature

The slice copies at most 16 MiB from one confined no-follow regular file to one
absent confined destination without modifying the source. It freezes strict
bounded two-path input, an exact two-endpoint `FilesystemCopy` capability,
existing no-follow parents, constant-memory 64 KiB binary streaming, source
and stage integrity checks, private destination-local staging, exactly one no-replace
publication, postcommit destination verification, bounded destination-parent
durability, redacted fixed errors, Linux/macOS execution, ten-tool host
composition, and explicit source-content and retained-parent race limitations.

It adds no overwrite, parent creation, directory or symlink copy, external path,
source-sized allocation, source mutation, dependency, CLI behavior, benchmark
workload, performance claim, or fx-equivalence claim.

## Parallel ownership

- Core and production own the typed authority, native implementation/exports,
  deterministic evidence seams, retained-root composition, and reference-host
  registration.
- Independent evidence owns direct, private, race, engine, host, core-contract,
  unsupported-target, cancellation, bounds, redaction, and allocation tests.
- Documentation owns the normative contract, implementation plan, maintained
  architecture/security/API pages, and this exact-SHA lineage record.

Owners use isolated worktrees or explicitly non-overlapping files. Only the
composed integration SHA can become a formal behavior candidate.

## Required evidence

- [ ] Exact public constants, schema, descriptions, result, construction
  taxonomy, error codes/messages/kinds/retryability, and redaction.
- [ ] Exact/one-over endpoint, component, argument, result, source-byte, chunk,
  I/O-call, interruption, entropy, temporary-attempt, and sync bounds.
- [ ] Effect-free preparation, strict canonical direct execution, exact typed
  two-path policy input, denial before lookup, and policy/execution agreement.
- [ ] Empty, text, binary, executable, exact-limit, same-parent, cross-parent,
  and confined cross-mount copies retain source bytes and ordinary mode while
  allocating no complete-content buffer.
- [ ] Existing destination of every entry type is never replaced; missing
  parents, invalid types, ancestor/final symlinks, and outside sentinels fail
  closed without blocking.
- [ ] Initial and final root, parent, source, destination, and stage identity;
  source mutation/replacement, destination appearance, stage replacement,
  hostile umask/ACL, moved retained parent, and final source-window behavior.
- [ ] Exact-one publication, no retry after `EINTR`, definitive precommit
  failures, postcommit identity/digest, destination-parent sync and 16-call
  bound, identity-safe cleanup, and cleanup dual failure.
- [ ] Cancellation around every authority and content operation, at final
  prepublication, and after real publication; inert-until-poll, drop, and core
  same-poll unknown-result recovery.
- [ ] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported target,
  ten-tool catalog, nine-clone identity, no-unsafe, dependency, compatibility,
  documentation, and fresh release-binary smoke evidence.

## Formal review protocol

After the complete local exact-SHA gate, create a tree-identical behavior
candidate and start three fresh reviewers against that same SHA:

1. correctness/API;
2. filesystem/robustness;
3. performance/concurrency.

Every finding is fixed, the complete local gate is rerun, and all three tracks
restart with fresh agents on one replacement SHA. Repeat until every track is
green with zero findings. Documentation-only seal and delivery-record commits
do not receive another adversarial cycle, but their exact feature and main
workflows remain required.
