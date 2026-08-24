# Milestone 03 native `rename_file` review 01

Status: **DELIVERED — final documentation record workflows pending**

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
  two exact-SHA artifacts. Contract CI `32667647822` was cancelled when a later
  feature push superseded it and is not claimed as green.
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
- Exact failed cycle-3 candidate:
  `5cc1523ebf1ba20264a80f3e703891ace58e1473` (tree
  `99b88ec8653679ca5386c9b0f1c368543f487796`).
- Exact cycle-3 remediation:
  `4cbd46f82d3553009824883de2bc243177459207` (tree
  `35f531eb867e1b08375041b3c74fcf1a650ae063`).
- Exact tree-identical cycle-4 candidate:
  `13379800ee2ee6eb6802db76c516e81dd087c62b` (tree
  `ab2bdc2b719061faa69749360fd1399177748c24`).
- All three cycle-1 tracks remain historically **NOT GREEN**. The remediation
  and its complete replacement local gate are green. All three fresh cycle-2
  tracks are **GREEN** with zero findings. The first feature workflow attempt
  exposed an unrelated pre-existing Linux test-fixture deadlock; its test-only
  remediation and complete replacement local gate are green. Cycle-3
  correctness/API is green; filesystem/robustness and performance/concurrency
  are **NOT GREEN** on the same unpinned source-inode reuse finding. Exact
  remediation passes the complete replacement local gate. All three fresh
  cycle-4 tracks are **GREEN** with zero findings. Replacement feature/main
  delivery is green on exact seal `7cb5ef9`.

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
  evidence seams, retained-root/source composition, and reference-host
  registration.
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

This closed cycle-2 adversarial review for that exact feature behavior. The
documentation-only review seal was exempt from another adversarial cycle under
the user's explicit instruction. Exact feature CI later required the unrelated
test-only remediation below, so the replacement behavior proceeded to cycle 3.

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
passes bare, help, human-status, and JSON-status smoke paths. At that checkpoint,
a tree-identical cycle-3 candidate and three fresh same-SHA reviews remained
required before the replacement feature push.

## Formal review cycle 3 and remediation local gate

Exact candidate `5cc1523ebf1ba20264a80f3e703891ace58e1473`, tree
`99b88ec8653679ca5386c9b0f1c368543f487796`, is **NOT GREEN**. Correctness/API
is green with zero findings after 16 direct, five engine, 15 private, one core,
seven host, all 14 lifecycle tests, and 100 repetitions of the formerly hanging
case. Filesystem/robustness and performance/concurrency independently found the
same medium defect: source identity retained only device/inode numbers, so an
unlink and inode reuse could make a different file pass revalidation and
postcommit comparison. The prior replacement test moved the original to a
linked name, which itself prevented reuse and did not cover that state.

Exact remediation `4cbd46f82d3553009824883de2bc243177459207`, tree
`35f531eb867e1b08375041b3c74fcf1a650ae063`, atomically opens the initial source
without following or reading it, requires a regular file by `fstat`, and keeps
the descriptor alive through postcommit comparison. Linux uses
`O_PATH | O_NOFOLLOW | O_CLOEXEC`; macOS uses the cfg-gated existing-libc
`O_EVTONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK` flags. Final-path and
published-destination identities are compared to the pinned object. One bounded
open and one constant descriptor replace the initial metadata-only inspection;
there is no content buffer, new lock, or dependency. Deterministic private
evidence unlinks the validated source while proving its retained descriptor has
zero links, moves a replacement, and requires commit ambiguity. Direct macOS
evidence records that a mode-000 source is preserved with the fixed permission
error because the kernel denies `O_EVTONLY`; Linux `O_PATH` continues to accept
that source mode.

The exact remediation passes the complete replacement gate under Rust/Cargo
1.94.1. Formatting, workspace all-target/all-feature warnings-denied Clippy,
workspace default/all-feature tests, both doctest modes, 16 private tests, 17
macOS direct tests, five engine tests, seven all-feature host tests, one core
contract, all 14 lifecycle tests, and 100 focused lifecycle repetitions are
green. Discovery is 779 default, 829 all-feature, and zero benchmarks on macOS.
The supported Linux rename targets compile and pass strict Clippy; Linux has 16
direct tests because the explicit macOS permission case is cfg-gated.

All 130 Python tests pass with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.19.9, cargo-audit 0.22.2 over 1,225 advisories and
175 dependencies, Linux/FreeBSD/WASI matrices, and active Node WASI 1/1 are
green. Only the established unrelated FreeBSD `glob_files` and WASI `read_file`
warnings remain. Documentation integrity is 66 Markdown files, 462 inline
links, 312 repository-relative links, and zero missing targets. The 23-file
base diff has no unsafe Rust, Cargo/dependency, or CLI change. A fresh locked
319,152-byte arm64 Mach-O release CLI has SHA-256
`0bdfaf3f4a1bf696030efa0120e52943f7d54a31f383440d38133067b863421c` and
passes bare, help, human-status, and JSON-status smoke paths. The exact
tree-identical cycle-4 candidate and review results are recorded next.

## Formal review cycle 4

Exact tree-identical candidate
`13379800ee2ee6eb6802db76c516e81dd087c62b`, tree
`ab2bdc2b719061faa69749360fd1399177748c24`, is **GREEN** with zero findings in
all three fresh tracks. Every reviewer independently verified the SHA/tree,
used a detached read-only worktree, and left it clean.

Correctness/API passed 16 private, 17 direct, five engine, seven host, all 61
core-contract, and all 14 lifecycle tests, plus 100 focused lifecycle
repetitions, strict native and Linux Clippy, formatting, and diff checks.
Filesystem/robustness passed the private/direct suites, exact unlink/reuse and
terminal-replacement cases, native Clippy, Linux no-default checks, and the
FreeBSD unsupported library check. It verified descriptor lifetime, hard-link
equivalence, entry-type rejection, root/parent confinement, no-replace behavior,
access errors, and durability bounds.

Performance/concurrency verified exactly one bounded non-reading source open,
one RAII descriptor, no descriptor clone, content read, lock, or unbounded
loop, and cleanup on every return path. Private/direct/engine/core/lifecycle
suites are green. Source-pin and lifecycle cases each passed 200 sequential and
64 parallel repetitions; the complete private suite passed ten low-FD and 24
parallel repetitions. At that checkpoint, replacement feature CI/benchmark,
two exact-SHA artifacts, fast-forward main integration, and exact main
workflows remained.

## Replacement delivery

Replacement documentation seal
`7cb5ef9fd04338cfe5c06b4d607e708c2bcdc620` passed exact feature CI
`32675233513` across all six jobs. Exact feature benchmark workflow
`32675233542` passed both jobs and retains exactly two nonexpired artifacts
bound to the seal SHA. `main` was fast-forwarded without force from exact prior
main `3d76f2e844312e7f3e809524cb72c1a7957975ff` to the same seal. Exact main CI
`32675562978` passed all six jobs; exact main benchmark workflow `32675562956`
passed both jobs and retains exactly two nonexpired exact-SHA artifacts. This
completes delivery of native `rename_file` as the twenty-third bounded
Milestone 03 slice.

The remaining Milestone 03 native tools, top-level CLI ownership, and composed
end-to-end boundary remain pending, so Milestone 03 is not complete. This final
delivery record is documentation-only and exempt from adversarial review under
the user's instruction. Its own exact feature CI and benchmark workflows and
exact main CI and benchmark workflows remain required after push and cannot be
self-recorded. This record makes no product-performance, fx-equivalence, or
compatibility-status promotion claim.

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
