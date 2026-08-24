# Milestone 03 native `copy_file` review 01

Status: **IMPLEMENTATION IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `226040780eb14dd72e86d0a002dc4bf61ba2ddfc`.
- Integration branch: `agent/m03-copy-file`.
- Normative contract: [`copy-file.md`](../copy-file.md).
- Base feature CI `32675981622` and benchmark `32675981593` are green.
- Base main CI `32676296870` and benchmark `32676296945` are green.
- Both benchmark workflows retain exactly two nonexpired exact-SHA artifacts.

The documentation-only contract commit
`6021fb0d6b1cf668e1a339a2cd2f60ead8d555dd` is exempt from adversarial review
under the user's explicit instruction. Exact contract CI `32677160680` passed
all six jobs, and benchmark workflow `32677160652` passed both jobs while
retaining exactly two nonexpired exact-SHA artifacts. These runs freeze
documentation only; they are not implementation, behavior, equivalence,
performance, or delivery evidence.

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

## Exact local gate before review cycle 1

Production component `9ab8d90` and maintained documentation component
`622b9d4` compose at exact precursor
`622b9d4bfd9e3bbbe34165f5dd64c5b2bf7996d4`. The complete local gate is green
on that tree under Rust and Cargo 1.94.1:

- focused evidence passes 20 private pipeline/race tests, 24 direct tests, five
  engine tests, seven reference-host tests, and one core capability contract;
- formatting, workspace all-target/all-feature warnings-denied Clippy, default
  workspace tests, all-target/all-feature workspace tests, and both doctests
  pass; discovery lists 829 default and 877 all-feature tests with zero
  benchmarks;
- all 130 Python tests pass with eight expected macOS skips, and the generated
  compatibility inventory matches pinned upstream
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- cargo-deny 0.20.2 reports advisories, bans, licenses, and sources green, while
  cargo-audit 0.22.2 scans 175 dependencies with no vulnerability finding;
- relevant Rust-only Linux and FreeBSD cross-target checks pass. The dedicated
  WASI unsupported test builds, and Node 22.22.0 actively executes it 1/1;
- 68 Markdown files contain 483 links, including 333 relative file links, with
  zero missing targets. The diff is clean, adds no unsafe Rust, and changes no
  Cargo metadata or CLI source; and
- the fresh locked release CLI has SHA-256
  `1e8c5aefd32ab12f201c1527b38f86ef31463c80be1f75a4901f9e00930f3c24`
  and passes bare, help, and status smoke paths.

This precursor is local implementation evidence only. A tree-identical review
candidate, three fresh zero-finding reviews, and exact feature/main remote gates
remain required; none of this is a performance or fx-equivalence claim.

## Formal review cycle 1

Exact tree-identical behavior candidate
`38d0d801caf1174d6df951a03d5843d6c217eb1a`, tree
`f0eadf23e4cdfa6613f866ef5923806a8474cb0e`, is **NOT GREEN**. Reviewer-focused
checks passed, but checks passing does not resolve the following actionable
findings:

1. Correctness/API is **NOT GREEN** with one medium late-cleanup-ownership
   finding. After an exclusive staged open succeeds, cancellation can be
   observed before the staged descriptor and pathname are placed under the
   `StagedFile` drop guard. Initial staged metadata rejection has the same late-
   ownership gap. Either path can return precommit while leaving the tool-owned
   stage pathname behind.
2. Filesystem/robustness is **NOT GREEN** with the same medium initial-stage
   cleanup/residue finding. Remediation must establish cleanup ownership
   immediately after successful creation and route every later cancellation or
   validation failure through best-effort held-descriptor mode restoration and
   identity-checked unlink. Deterministic regressions must cover cancellation
   from the successful open and rejected initial staged metadata, while proving
   that a mismatched replacement is preserved.
3. Performance/concurrency is **NOT GREEN** with one medium postcommit source-
   parent rewalk finding and one low evidence finding. Postcommit source
   validation reuses the source-parent descriptor retained before publication,
   so a moved or replaced source parent can validate the stale directory rather
   than the source path currently reached from the retained root. Remediation
   must perform a fresh cancellation-ignoring root/parent rewalk after commit and
   treat any path/parent/source mismatch as ambiguity. Independent evidence must
   also exercise exact serialized argument/result bounds and demonstrate that
   allocation remains bounded independently of source size.

No remediation or behavior-green claim is made here. Production and independent
evidence remediation, the complete replacement local gate, and a fresh three-
track same-SHA review cycle remain pending. This documentation-only review
record is exempt from a separate adversarial cycle under the user's instruction;
the replacement behavior candidate is not exempt.

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
