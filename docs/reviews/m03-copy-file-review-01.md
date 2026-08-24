# Milestone 03 native `copy_file` review 01

Status: **PORTABILITY REMEDIATION REVIEW PENDING**

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

## Cycle 1 remediation and replacement gate

Exact production and evidence remediation
`53f4ee947c82033a08a2ff943f23f52c475189d7`, tree
`4bdb07a30950584d71260e70e263aafcccfff710`, closes every cycle-1 finding:

1. A successful exclusive staged open now transfers immediately into an
   infallible cleanup guard before cancellation or metadata validation. The
   guard pins the created inode with its descriptor, accepts an initially
   unknown recorded identity, and unlinks only when the held descriptor and
   no-follow pathname still identify the same regular file. Deterministic
   cancellation and rejected-metadata regressions prove no tool-owned residue,
   while a replaced pathname is preserved.
2. Precommit native operations now use check-before/raw-call/check-after
   cancellation ordering. Publication remains exactly once and is never
   retried, including after `EINTR`; postcommit verification and durability
   continue to ignore caller cancellation.
3. Postcommit source verification freshly rewalks the requested source parent
   from the retained workspace root and compares the parent and source
   identities. A moved-and-replaced source parent therefore returns the fixed
   ambiguous-commit result after publication while destination-parent sync is
   still attempted independently.
4. Independent evidence exercises exact and one-over serialized argument and
   result guards. Empty, one-byte, and exact-16-MiB inputs observe the same one
   reusable 64-KiB streaming buffer through copy, both staged hashes, and the
   published hash without allocating a source-sized fixture or content buffer.

The complete replacement local gate is green under Rust and Cargo 1.94.1:

- focused evidence passes 25 private pipeline/race tests, 24 direct tests, five
  engine tests, seven reference-host tests, and one core capability contract;
- formatting, workspace all-target/all-feature warnings-denied Clippy, default
  workspace tests, all-target/all-feature workspace tests, and both doctests
  pass; discovery lists 834 default and 882 all-feature tests with zero
  benchmarks;
- all 130 Python tests pass with eight expected platform skips, and the
  generated compatibility inventory matches pinned upstream
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- cargo-deny 0.20.2 reports advisories, bans, licenses, and sources green, while
  cargo-audit 0.22.2 scans 175 dependencies with no vulnerability finding;
- relevant Rust-only Linux and FreeBSD cross-target checks pass. The dedicated
  WASI unsupported test builds, and Node 22.22.0 actively executes it 1/1;
- 68 Markdown files contain 483 links, including 333 relative file links, with
  zero missing targets before this record update. The diff is clean, adds no
  unsafe Rust, and changes no Cargo metadata or CLI source; and
- the fresh locked release CLI retains SHA-256
  `1e8c5aefd32ab12f201c1527b38f86ef31463c80be1f75a4901f9e00930f3c24`
  and passes bare, help, and status smoke paths.

This is local remediation evidence, not a green review or delivery claim. A
tree-identical cycle-2 candidate and three fresh same-SHA zero-finding reviews
remain required before exact feature and main delivery gates.

## Formal review cycle 2

Exact tree-identical candidate
`ad4af0c2c642cc315724a3515bacd9aa70cbe17f`, tree
`9e09fd7ba5b486847b8302629193f3e665831d81`, is **GREEN** with zero findings in
all three fresh tracks. Every reviewer independently verified the SHA and tree,
used a fresh detached read-only worktree, left it clean, and reviewed the same
immutable behavior.

Correctness/API passed 25 private pipeline/race tests, 24 direct tests, five
engine tests, seven reference-host tests, and the exact core capability
contract under Rust/Cargo 1.94.1. It audited strict schema and canonical direct
arguments, exact two-endpoint authority, public bounds, fixed redacted errors,
cancellation semantics, cleanup ownership, postcommit ambiguity, unsupported
targets, and ten-tool host composition.

Filesystem/robustness passed the 25 private and 24 direct suites under the same
toolchain. It audited descriptor-relative no-follow confinement, retained root,
parent, source, and stage identities, replacement-safe cleanup, ordinary-mode
and macOS ACL handling, streaming and retry bounds, exact-one no-replace
publication and nonretryable `EINTR`, fresh postcommit source rewalking,
destination integrity, and bounded destination-parent durability.

Performance/concurrency passed the 25 private, 24 direct, and five engine suites
plus focused warnings-denied native Clippy. It verified one 64-KiB allocation
reused at all four streaming sites, exact serialized guards, bounded source,
I/O, hash, entropy, staging, and sync work, precommit cancellation precedence,
source/destination/stage race outcomes, exact-one publication, independent
postcommit verification and sync, and the absence of deadlock or unbounded-work
paths. No reviewer found an actionable correctness, robustness, API,
performance, concurrency, or evidence defect.

Local behavior is sealed. Exact feature CI and benchmark workflows, exactly two
nonexpired feature-SHA benchmark artifacts, fast-forward integration, and exact
main workflows remain pending; no performance or fx-equivalence claim is made.

## First feature workflow attempt and lint remediation

Documentation seal `16b92ef1a409fdca78ddb86ce4ae7879b89e65d6`
passed exact feature benchmark workflow `32683596971`. Both jobs are green and
exactly two nonexpired artifacts bind that SHA:

- `bootstrap-benchmark-16b92ef1a409fdca78ddb86ce4ae7879b89e65d6`;
- `upstream-benchmark-16b92ef1a409fdca78ddb86ce4ae7879b89e65d6-ubuntu-24.04-x86_64`.

Exact feature CI `32683596986` passed all four native Linux/macOS matrices and
the dependency policy/audit job. Its quality job passed formatting but failed
warnings-denied Clippy before tests because the two Linux no-op ACL shims retain
the same fallible signatures as their macOS implementations. Linux-only Clippy
reported `unnecessary_wraps`; no runtime test, product behavior, dependency,
compatibility, benchmark, or performance assertion failed.

Exact portability remediation
`bb21c7aa91554b8958c69b15c2b93dba7aed2755`, tree
`c7fe63b030cc1de468c7694ce7e0c67c86866ab8`, adds only two narrowly scoped,
reasoned Linux `clippy::unnecessary_wraps` allowances. It changes no signature,
branch, syscall, result, test, dependency, CLI byte, or `copy_file` behavior.

The complete replacement local gate is green under Rust/Cargo 1.94.1. Linux
no-default warnings-denied Clippy passes in addition to the full native macOS
workspace Clippy gate. Formatting, default and all-target/all-feature workspace
tests, both doctest modes, 25 private, 24 direct, five engine, seven host, and
one core focused checks pass. Discovery remains 834/882 with zero benchmarks;
all 130 Python tests pass with eight expected platform skips. Pinned-fx
compatibility, cargo-deny 0.20.2, cargo-audit 0.22.2 over 175 dependencies,
Linux/FreeBSD checks, active Node 22.22.0 WASI 1/1, and documentation integrity
at 68/483/333/0 are green. The diff remains no-unsafe/no-Cargo/no-CLI, and the
fresh release CLI retains SHA-256
`1e8c5aefd32ab12f201c1527b38f86ef31463c80be1f75a4901f9e00930f3c24`
with bare/help/status smoke paths green.

Because the lint remediation touches production source after the cycle-2 seal,
a tree-identical cycle-3 candidate and three fresh same-SHA reviews remain
required before another feature push. The failed CI and green benchmark runs
are evidence for the first seal only and cannot deliver the replacement.

## Required evidence

- [x] Exact public constants, schema, descriptions, result, construction
  taxonomy, error codes/messages/kinds/retryability, and redaction.
- [x] Exact/one-over endpoint, component, argument, result, source-byte, chunk,
  I/O-call, interruption, entropy, temporary-attempt, and sync bounds.
- [x] Effect-free preparation, strict canonical direct execution, exact typed
  two-path policy input, denial before lookup, and policy/execution agreement.
- [x] Empty, text, binary, executable, exact-limit, same-parent, cross-parent,
  and confined cross-mount copies retain source bytes and ordinary mode while
  allocating no complete-content buffer.
- [x] Existing destination of every entry type is never replaced; missing
  parents, invalid types, ancestor/final symlinks, and outside sentinels fail
  closed without blocking.
- [x] Initial and final root, parent, source, destination, and stage identity;
  source mutation/replacement, destination appearance, stage replacement,
  hostile umask/ACL, moved retained parent, and final source-window behavior.
- [x] Exact-one publication, no retry after `EINTR`, definitive precommit
  failures, postcommit identity/digest, destination-parent sync and 16-call
  bound, identity-safe cleanup, and cleanup dual failure.
- [x] Cancellation around every authority and content operation, at final
  prepublication, and after real publication; inert-until-poll, drop, and core
  same-poll unknown-result recovery.
- [x] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported target,
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
