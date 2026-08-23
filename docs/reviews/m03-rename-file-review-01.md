# Milestone 03 native `rename_file` review 01

Status: **REMOTE TEST REMEDIATED; CYCLE 3 REVIEW PENDING**

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
- Exact composed local-gate precursor:
  `43847fe5fd405e8b1d28808f0495dac859ebab15` (tree
  `80cb9a17d9bb2c1151bc43b72faebcb305dd78c2`).
- Exact failed cycle-1 candidate:
  `2bc4f9a8ad809cd38a6b7b36488b27bf9bd531f6` (tree
  `44558a0e88019ad9063234642c08097b4123c5f2`).
- Exact cycle-1 remediation:
  `a3491cf8d5e6c388c896374e768794d06bf7be0b` (tree
  `0b195bdf29e7873a4d77169ec4d031491b1b336a`).
- Exact tree-identical cycle-2 candidate:
  `4f224a5447a61a76a3cdea5ced035c164240c02c` (tree
  `cb75dca76eeec80dc526946c9d39d6e3da882c68`).
- First cycle-2 documentation seal:
  `a03a57be82eca63a2cc471c308924608c1de6f8b`.
- Exact remote-test remediation:
  `2c771edf3d4385c0c94f2cbbee93427ea9e8b13a` (tree
  `5de94a6f90d5316ab84b7f9451e51b7cc25fd6a2`).
- All three cycle-1 tracks remain historically **NOT GREEN**. The remediation
  and its complete replacement local gate are green. All three fresh cycle-2
  tracks are **GREEN** with zero findings. The first feature workflow attempt
  exposed an unrelated pre-existing Linux test-fixture deadlock; its test-only
  remediation and complete replacement local gate are green. Cycle-3 review
  and replacement feature/main delivery have not completed.

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

- [x] Exact public constants, schema, descriptions, result, construction
  taxonomy, error codes/messages/kinds/retryability, and redaction.
- [x] Exact/one-over path, component, argument, and result bounds for both
  requested and canonical endpoints.
- [x] Effect-free preparation, strict canonical direct execution, exact typed
  two-path policy input, denial before lookup, and policy/execution agreement.
- [x] Same-parent and cross-parent regular-file moves preserve file identity
  and content without reading content, creating parents, or leaving residue.
- [x] Existing destination of every entry type is never replaced; missing
  parents, invalid types, ancestor/final symlinks, and outside sentinels fail
  closed without blocking.
- [x] Initial and final linked-root, parent, source, and destination-absence
  evidence; retained-root changes, moved retained parents, and final source
  replacement race behavior.
- [x] Exactly one real no-replace rename, no retry after `EINTR`, definitive
  precommit failures, postcommit identity, same-parent one-sync, and distinct
  source-then-destination sync with both attempted and 16-call bounds.
- [x] Cancellation around every authority operation, at final pre-rename, and
  after real rename; inert-until-poll, drop, and core same-poll unknown-result
  recovery.
- [x] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported-target,
  nine-tool catalog, eight-clone identity, no-unsafe, dependency, compatibility,
  documentation, and fresh release-binary smoke evidence.

## Exact composed local gate

Exact precursor `43847fe5fd405e8b1d28808f0495dac859ebab15`, tree
`80cb9a17d9bb2c1151bc43b72faebcb305dd78c2`, passes the complete local gate
under Rust and Cargo 1.94.1. Formatting, workspace all-target/all-feature
warnings-denied Clippy, workspace tests, and both doctests are green. Focused
evidence passes five private module tests, 16 direct tests, five real-engine
tests, seven reference-host tests, and one core serialization contract.
Discovery reports 767 default-feature tests, 817 all-feature tests, and zero
benchmarks.

All 130 repository Python tests pass with eight expected macOS skips. The
pinned fx compatibility inventory is unchanged at
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Local cargo-deny 0.19.9 accepts
the policy with only the established duplicate warnings, and cargo-audit
0.22.2 checks 1,225 cached advisories across 175 lockfile dependencies with no
finding. Linux no-default test-target check and warnings-denied Clippy pass.
FreeBSD no-default library/tests, library Clippy, and the dedicated unsupported
test type-check pass with only the established unrelated test-only
`glob_files` dead-code warnings. WASI builds the dedicated unsupported target
with only the established unrelated `read_file` warning, and Node 22.22.0
actively passes that test 1/1.

Documentation integrity covers 66 Markdown files, 462 inline links, 312
repository-relative links, and zero missing targets. The 22-file base diff is
clean, adds no unsafe Rust, Cargo metadata, dependency, or CLI source change.
A fresh locked 319,152-byte arm64 Mach-O release CLI has SHA-256
`76a86a025c19c338ce05e0de750a685cd2d439741787e3ce420344ac97c1f3c1` and
passes exact bare, help, human-status, and JSON-status smoke paths. These
results establish local readiness only, not review, delivery, compatibility
promotion, fx equivalence, or product performance.

## Formal review cycle 1

Exact candidate `2bc4f9a8ad809cd38a6b7b36488b27bf9bd531f6`, tree
`44558a0e88019ad9063234642c08097b4123c5f2`, is **NOT GREEN** in all three
fresh tracks. The consolidated evidence findings require retained tests for
the terminal source-replacement race across regular-file, symlink, FIFO, and
directory replacements; `EINTR` ambiguity without retry; injected errno
classification; postcommit destination-identity failure; same- and distinct-
parent sync behavior and the 16-call bound; ignored late cancellation; and
retained parents moved before the rename boundary. A separate low
documentation finding identified that the contract described only symlink and
special-file final replacements even though a directory entry can also be
moved in that window.

Exact remediation `a3491cf8d5e6c388c896374e768794d06bf7be0b`, tree
`0b195bdf29e7873a4d77169ec4d031491b1b336a`, expands the private module suite
from five to 15 tests to cover the requested terminal-race, syscall,
postcommit, durability, cancellation, and moved-parent matrix. The contract
clarifies the final directory-replacement race. The complete replacement local
gate below is green. The tree-identical cycle-2 behavior candidate and three
fresh green reviews are recorded below.

## Cycle 1 remediation local gate

Exact remediation `a3491cf8d5e6c388c896374e768794d06bf7be0b`, tree
`0b195bdf29e7873a4d77169ec4d031491b1b336a`, passes the complete replacement
local gate under Rust and Cargo 1.94.1. Formatting, workspace all-target and
all-feature warnings-denied Clippy, workspace tests, all-feature tests, and
both doctests are green. Focused evidence passes 15 private module tests, 16
direct tests, five real-engine tests, seven all-feature reference-host tests,
and one core serialization contract. Discovery reports 777 default-feature
tests, 827 all-feature tests, and zero benchmarks.

All 130 repository Python tests pass with eight expected macOS skips. The
pinned fx compatibility inventory is unchanged at
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Local cargo-deny 0.19.9 accepts
the policy with only the established duplicate warnings, and cargo-audit
0.22.2 checks 1,225 advisories across 175 lockfile dependencies with no
finding. Linux no-default test-target check and warnings-denied Clippy pass.
FreeBSD no-default library/tests, library Clippy, and the dedicated unsupported
test type-check pass with only the established unrelated test-only
`glob_files` warnings. WASI builds the dedicated unsupported target with only
the established unrelated `read_file` warning, and Node 22.22.0 actively
passes that test 1/1.

Documentation integrity covers 66 Markdown files, 462 inline links, 312
repository-relative links, and zero missing targets. The 22-file base diff is
clean, adds no unsafe Rust, Cargo metadata, dependency, or CLI source change.
A fresh locked 319,152-byte arm64 Mach-O release CLI has SHA-256
`687b6f8068ea34709c0f6d0ec536febf7d5be62c9ff8fa7b940dc48ba74f6415` and
passes exact bare, help, human-status, and JSON-status smoke paths. These
results qualify the replacement local gate only, not remote delivery, fx
equivalence, or product performance.

## Formal review cycle 2

Exact tree-identical candidate
`4f224a5447a61a76a3cdea5ced035c164240c02c`, tree
`cb75dca76eeec80dc526946c9d39d6e3da882c68`, is **GREEN** with zero findings
in all three fresh tracks. Correctness/API reran 16 direct, five engine, 15
private, one core-contract, and seven all-feature reference-host tests.
Filesystem/robustness reran the 15 private, 16 direct, and five engine tests.
Performance/concurrency reran the same 15/16/5 focused suites and
`git diff --check`. Every reviewer independently verified the exact SHA and
tree and left the worktree clean and unchanged.

This closes adversarial review for the feature behavior. The documentation-
only review seal is exempt from another adversarial cycle under the user's
explicit instruction. Exact feature CI and benchmark workflows, both exact-SHA
benchmark artifacts, fast-forward main integration, and exact main workflows
remain required.

## First feature workflow attempt and test-only remediation

Documentation seal `a03a57be82eca63a2cc471c308924608c1de6f8b` passed exact
feature benchmark workflow `32671805335`: both jobs are green and exactly two
nonexpired artifacts name and bind that SHA. Exact feature CI `32671805412`
passed formatting, Clippy, all four native Linux/macOS matrices, and dependency
policy/audit. Its quality job then remained blocked in the pre-existing
`same_engine_concurrent_create_reservation_reports_live_session` test and was
explicitly cancelled after the exact hang was proven from both that run and
the older contract run.

The fixture acquired an external session lock before the spawned create's
initial load and ran its first-call hook before reserving the scripted ID.
Linux could therefore block the initial load or let the second call consume
the first ID and wait on the lock that the test released only after that call
returned. macOS scheduling and lock semantics had masked the cycle. Exact
test-only remediation `2c771edf3d4385c0c94f2cbbee93427ea9e8b13a`, tree
`5de94a6f90d5316ab84b7f9451e51b7cc25fd6a2`, reserves the scripted step and
extracts the hook before signaling, then uses reached/release barriers instead
of filesystem-lock timing. It changes no production source or `rename_file`
behavior.

That exact remediation passes the complete replacement local gate under Rust
and Cargo 1.94.1. The formerly hanging test passes 100 consecutive focused
runs, its complete 14-test lifecycle suite, and strict focused Clippy. Full
formatting, workspace all-target/all-feature warnings-denied Clippy, workspace
tests, all-feature tests, and both doctests are green. The `rename_file`
focused totals remain 15 private, 16 direct, five engine, seven all-feature
reference-host, and one core-contract test; discovery remains 777/827 with
zero benchmarks.

All 130 Python tests pass with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.19.9, cargo-audit 0.22.2 over 1,225 advisories and
175 dependencies, Linux/FreeBSD/WASI gates, and active Node 1/1 are green.
Documentation integrity remains 66 Markdown files, 462 inline links, 312
repository-relative links, and zero missing targets. The base diff is now 23
files with no unsafe Rust, Cargo/dependency, or CLI change. A fresh locked
319,152-byte arm64 Mach-O release CLI has SHA-256
`126ecc47857cb327e3b483daecf9c50ce6b04585f4cdaed60e6f20cb9f82b107` and
passes bare, help, human-status, and JSON-status smoke paths. A tree-identical
cycle-3 candidate and three fresh same-SHA reviews remain required before the
replacement feature push.

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
