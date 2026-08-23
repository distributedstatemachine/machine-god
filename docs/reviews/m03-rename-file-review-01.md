# Milestone 03 native `rename_file` review 01

Status: **IMPLEMENTATION IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `3d76f2e844312e7f3e809524cb72c1a7957975ff`.
- Integration branch: `agent/m03-rename-file`.
- Normative contract: [`rename-file.md`](../rename-file.md).
- Base feature CI `32665981665` and benchmark `32665981641` are green.
- Base main CI `32666261656` and benchmark `32666261525` are green.
- Both benchmark workflows retain two nonexpired exact-SHA artifacts.
- Frozen contract commit:
  `19cad7d10a8fc885e2e70a7345fc0ba27d76872a`.
- Exact contract benchmark workflow `32667647846` is green with both jobs and
  two exact-SHA artifacts. Contract CI `32667647822` is still in progress and
  is not claimed as green.
- Production composes at
  `d8f73676fcfce2cead385fa5b36598da989abe8f`.
- Independent evidence composes at
  `1dab9a0dfcb4ec2d204625c744171ae923cca458`.
- Exact composed local gates and formal review have not yet completed.

This documentation-only contract is exempt from adversarial review under the
user's explicit instruction. Its workflows freeze documentation only; they are
not implementation, behavior, equivalence, performance, or delivery evidence.

## Frozen feature

The slice moves one confined regular file to one absent confined destination.
It freezes strict bounded two-path input, a typed two-endpoint
`FilesystemRename` capability, existing no-follow parents, two-pass identity
validation, exactly one no-replace rename, postcommit destination-identity
verification, bounded one- or two-parent durability, redacted fixed errors,
Linux/macOS execution, nine-tool host composition, and explicit final-window
race and retained-parent limitations.

It adds no overwrite, parent creation, directory or symlink move, external
path, content read, enumeration, staging, copy/delete fallback, dependency,
CLI behavior, benchmark workload, performance claim, or fx-equivalence claim.

## Parallel ownership

- Production owns core authority, native implementation/exports, deterministic
  evidence seams, retained-root composition, and reference-host registration.
- Independent evidence owns direct, private, race, engine, host, core-contract,
  unsupported-target, cancellation, bounds, and redaction tests.
- Documentation owns the normative contract, implementation plan, and this
  exact-SHA lineage record.

Owners work in isolated worktrees on non-overlapping files. Only the composed
integration SHA can become a formal behavior candidate.

## Required evidence

- [ ] Exact public constants, schema, descriptions, result, construction
  taxonomy, error codes/messages/kinds/retryability, and redaction.
- [ ] Exact/one-over path, component, argument, and result bounds for both
  requested and canonical endpoints.
- [ ] Effect-free preparation, strict canonical direct execution, exact typed
  two-path policy input, denial before lookup, and policy/execution agreement.
- [ ] Same-parent and cross-parent regular-file moves preserve file identity
  and content without reading content, creating parents, or leaving residue.
- [ ] Existing destination of every entry type is never replaced; missing
  parents, invalid types, ancestor/final symlinks, and outside sentinels fail
  closed without blocking.
- [ ] Initial and final linked-root, parent, source, and destination-absence
  evidence; retained-root changes, moved retained parents, and final source
  replacement race behavior.
- [ ] Exactly one real no-replace rename, no retry after `EINTR`, definitive
  precommit failures, postcommit identity, same-parent one-sync, and distinct
  source-then-destination sync with both attempted and 16-call bounds.
- [ ] Cancellation around every authority operation, at final pre-rename, and
  after real rename; inert-until-poll, drop, and core same-poll unknown-result
  recovery.
- [ ] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported-target,
  nine-tool catalog, eight-clone identity, no-unsafe, dependency, compatibility,
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
