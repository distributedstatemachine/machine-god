# Milestone 03 native `write_file` review 01

Status: **FORMAL CYCLE-3 REVIEW GREEN — documentation-only seal; exact feature
workflows and delivery are pending**

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
The integration branch has composed all three without overwriting one another.
Exact candidate `119938240807f8279f83e2ace65a69706e8fcfed` completed the first
formal review cycle, and all three tracks reported **NOT GREEN**. Code and
evidence remediation is composed through `3010e6d`, and replacement exact local
gates are green at `581fe6a`. Formal-review preparation
`491496aa22aa8855717b74f6a026e8c602bb02e9` is the immediate parent of the
tree-identical exact cycle-2 candidate
`708f2d08d72d610ca387a62a4cec1f656c188a7d`. Cycle 2 is also **NOT GREEN**.
Production remediation is composed at
`9302ec3fa7d6e891fdc4a0c7bd8fe9b7cf8e427d`; exact local gates are green at
`8432c0c6b5d5955b78a882b651a5bfec76af8814`, while another fresh same-SHA cycle
remains pending.

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
following evidence through cycle-2 production remediation:

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
- [x] Deterministic target appearance, existing-target replacement, and final-
  parent postvalidation races are proven through the real production pipeline.
  Native publication proves raced-create preservation, ordinary-rename raced
  replacement, and retained-parent publication after a move outside the root.
  Staged-name replacement, eight collisions, collision preservation, and
  cleanup swap protection are also covered.
- [x] Retained owned staging residue receives one best-effort `fchmod(0600)`
  before best-effort identity-checked unlink, with the unavoidable mode-
  restoration-plus-unlink failure caveat documented and successful reset
  behavior tested.
- [x] Injected write/chmod/file-sync/rename/directory-sync failures establish
  unchanged-target precommit behavior and post-rename commit ambiguity.
- [x] Cancellation is proven through the real production pipeline during and
  after the verification phase. Traversal, final-prepublish, inert-until-poll/
  drop, engine same-poll post-effect recovery, and absence of detached work are
  covered.
- [x] Content-write and sync paths have an exact 16-interruption phase bound.
  Cumulative interleaved write interruptions, both precommit sync and
  cancellation outcomes, and real-rename postcommit ambiguity are covered.
- [x] Temporary-name entropy acquisition has a production-used finite
  cumulative partial-progress/interruption bound, cancellation checks at retry
  and exhaustion boundaries, and deterministic no-staging/no-target-effect
  evidence. Linux uses direct nonblocking `rustix` entropy with at most 16
  cumulative interruptions and 31 calls per name; the one-call macOS path is
  routed through the same checks.
- [ ] Exact six-tool alphabetical host catalog, original-plus-five-clone
  workspace identity, and the complete platform matrix are green. The catalog
  and workspace identity pass locally; native behavior is locally exercised on
  macOS, Linux and FreeBSD are cross-compiled, and the WASI unsupported-target
  test is actively executed. Exact feature CI still must exercise supported
  native Linux and macOS behavior.

## Formal gates

Before delivery, one exact composed SHA must pass local Rust 1.94.1 formatting,
warnings-denied workspace Clippy, workspace and documentation tests, repository
Python and compatibility checks, dependency policy/audit, Linux/macOS native
tests, FreeBSD/WASI checks, a clean locked release smoke, and three fresh
same-SHA adversarial tracks for correctness/API, filesystem/robustness, and
performance/concurrency. Neither cycle 1 nor cycle 2 satisfies that gate. Every
finding restarts all three tracks on a new exact behavior candidate. A later
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
- Formal-review preparation:
  `a7841c19b4b34cecf40e55d7cd001fd1547133c1`
- First exact behavior candidate:
  `119938240807f8279f83e2ace65a69706e8fcfed`
- Candidate tree lineage: `119938240807f8279f83e2ace65a69706e8fcfed` is
  tree-identical only to its immediate parent
  `a7841c19b4b34cecf40e55d7cd001fd1547133c1`. Precursor
  `072bd69eb6f73944d1db00363da0f965f09dda9f` has a different documentation
  tree and is not an exact-tree substitute for the formal candidate.
- Formal adversarial cycle 1: correctness/API **NOT GREEN**;
  filesystem/robustness **NOT GREEN**; performance/concurrency **NOT GREEN**
- Cycle-1 findings documentation:
  `016f8dff13dffbacce3010342ee5b62f0af23b82`
- Cycle-1 code and evidence remediation:
  `3010e6d883b5f894083c6925b2b4412b8102b750`
- Remediated local-gate precursor:
  `581fe6aa9a4190ba8cc303371e02af5aba68a5a1`
- Replacement formal-review preparation:
  `491496aa22aa8855717b74f6a026e8c602bb02e9`
- Exact cycle-2 replacement candidate:
  `708f2d08d72d610ca387a62a4cec1f656c188a7d`
- Cycle-2 candidate tree lineage:
  `708f2d08d72d610ca387a62a4cec1f656c188a7d` is tree-identical only to its
  immediate parent `491496aa22aa8855717b74f6a026e8c602bb02e9`. Remediated
  local-gate precursor `581fe6aa9a4190ba8cc303371e02af5aba68a5a1` has a
  different documentation tree and is not an exact-tree substitute.
- Formal adversarial cycle 2: correctness/API **GREEN** with zero findings;
  filesystem/robustness **NOT GREEN** with two medium findings;
  performance/concurrency **NOT GREEN** with one medium finding
- Cycle-2 findings documentation:
  `5e7e61a1da8aa39f31126d3c474dba9880d3a4b1`
- Pending-remediation specification:
  `526aa4abddb4aa004a1623d328ae5ea1af241473`
- Those two documentation-only commits are exempt from adversarial review under
  the user's instruction
- Cycle-2 production remediation:
  `9302ec3fa7d6e891fdc4a0c7bd8fe9b7cf8e427d`
- Cycle-2 remediation local-gate precursor:
  `8432c0c6b5d5955b78a882b651a5bfec76af8814`
- Exact cycle-2 remediation gate record:
  `9a09172ac40d7ec09ebb9fa7a4e4e21f12b2a632`
- Cycle-2 remediation resolution: both confirmed findings are closed by exact
  production and deterministic evidence locally; historical cycle 2 remains
  **NOT GREEN** and cycle 3 is required
- Exact cycle-3 behavior candidate:
  `db78c6407c4f603f18e2839a8a291f2de33e579c`
- Cycle-3 candidate tree lineage: exact candidate `db78c640` is tree-identical
  to immediate formal-review preparation parent `5ed38f3c`
- Behavior-green SHA: `db78c6407c4f603f18e2839a8a291f2de33e579c`
- Formal adversarial cycle 3: correctness/API **GREEN** zero findings;
  filesystem/robustness **GREEN** zero findings; performance/concurrency
  **GREEN** zero findings
- Documentation seal: this documentation-only commit; exempt from further
  adversarial review under the user's instruction
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

The preformal evidence work closed several earlier gaps before the first formal
candidate:

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

Formal cycle 1 subsequently found remaining bounded-progress, real-pipeline
race, real-pipeline verification-cancellation, and documentation-evidence
defects. No preformal result is promoted into delivery evidence.

## Formal adversarial cycle 1

All three fresh tracks inspected exact candidate
`119938240807f8279f83e2ace65a69706e8fcfed` and returned **NOT GREEN**. The
confirmed findings are:

- **High/medium:** write and sync helpers can retry `EINTR` without a finite
  attempt or work bound, contradicting the bounded synchronous execution
  contract and weakening cancellation responsiveness under repeated
  interruption.
- **Medium:** deterministic evidence does not yet drive target appearance,
  existing-target replacement, or final-parent postvalidation identity races
  through the real production pipeline.
- **Medium:** verification-phase cancellation is not yet exercised through the
  real production pipeline.
- **Low:** the maintained documents conflated the local-gate precursor with the
  later formal-candidate tree and overstated platform evidence. This record now
  distinguishes local native macOS execution, Linux/FreeBSD cross-compilation,
  active WASI execution, and the still-pending exact feature-CI Linux/macOS
  native matrix.

Documentation correction `016f8df` and code/evidence remediation `3010e6d`
close every confirmed cycle-1 finding. The write path has one cumulative
16-interruption budget despite partial progress; precommit file sync and
postcommit parent sync each have their own 16-interruption budget. Precommit
exhaustion preserves the target and cleans the stage, cancellation on the final
interruption wins, and postcommit exhaustion after a real rename is
nonretryable ambiguity with the new bytes live.

Native-publish pipeline tests now prove raced-create preservation, ordinary-
rename replacement of a postvalidation racer, publication into a retained
parent moved outside the configured workspace, and cancellation after staged
and target verification with zero publish calls plus exact-name cleanup.
Replacement local gates are green at `581fe6a`; later documentation prepares
the replacement review at `491496a`, whose immediate tree-identical marker is
exact candidate `708f2d0`. Cycle 2 did not approve that marker, so no green
behavior SHA is claimed.

## Formal adversarial cycle 2

All three fresh tracks inspected exact candidate
`708f2d08d72d610ca387a62a4cec1f656c188a7d`. Their exact results are:

- Correctness/API: **GREEN**, zero findings.
- Filesystem/robustness: **NOT GREEN**, with two medium findings. First, final
  mode is applied before publication but cleanup does not restore the held
  staged descriptor to `0600`; cancellation, rename failure, or staged-identity
  disagreement can therefore retain an owned inode with `0644` or an observed
  target mode as permissive as `0777`, contradicting the private-residue claim.
  Second, Linux temporary-name entropy delegates to a backend that can retry
  partial reads or `EINTR` without the feature's finite work and cancellation
  bound.
- Performance/concurrency: **NOT GREEN**, with the same medium Linux entropy
  finding. Exact exhaustion, cumulative interruption accounting, cancellation
  precedence on the final interruption, and proof of no target or staging
  effect remain missing.

No green or replacement claim follows from cycle 2. Production remediation
`9302ec3fa7d6e891fdc4a0c7bd8fe9b7cf8e427d` uses direct Linux `rustix`
`getrandom` with `NONBLOCK`, at most 16 cumulative `EINTR` results and 31 calls
per 16-byte name including partial progress, and cancellation checks before and
after every call. `ENOSYS`, `EPERM`, and `EAGAIN` fail closed as retryable
`write_file_unavailable` rather than invoke a fallback or block. The pinned
macOS `getrandom` 0.4.3 path makes one `getentropy` call for the 16-byte request
and routes through the same bounds. Cleanup now makes one best-effort
`fchmod(0600)` call on the held, unpublished staged descriptor before the
existing identity-checked best-effort unlink. If mode restoration and unlink
both fail, residue can retain its final mode.

Focused remediation checks are green: 28 private `write_file` tests, 109 native
library tests, 25 direct integration tests, formatting, workspace/all-target/
all-feature warnings-denied Clippy, and the Linux cross-check. These focused
results preceded the complete exact local gate now recorded at `8432c0c`. They
do not establish a new exact behavior candidate or a green cycle 3. Three fresh
agents must repeat all tracks on that same new SHA.

## Initial local gate results

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
- Linux no-default native cross-target Clippy passes with warnings denied;
  FreeBSD
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
evidence. They were recorded at `072bd69eb6f73944d1db00363da0f965f09dda9f`;
that commit has a different documentation tree from the formal candidate.
Candidate `119938240807f8279f83e2ace65a69706e8fcfed` is tree-identical only to
its immediate parent `a7841c19b4b34cecf40e55d7cd001fd1547133c1` and failed
formal cycle 1 as recorded above.

## Cycle-1 remediation local gate results

Exact composed remediation precursor
`581fe6aa9a4190ba8cc303371e02af5aba68a5a1` is green under Rust and Cargo
1.94.1 exactly:

- formatting and workspace/all-target/all-feature warnings-denied Clippy pass;
- the workspace all-target/all-feature inventory and gate pass 655 tests plus
  two doctests;
- focused evidence passes 27 private `write_file` tests, 25 direct integration
  tests, five real-engine tests, seven reference-host tests, and three prepared-
  root tests;
- the repository Python gate runs 129 tests: 121 pass and eight expected macOS
  skips, with zero failures or errors;
- the clean pinned fx checkout at
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes compatibility-inventory
  generation check;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 checks 1,225 cached advisories over 175
  dependencies with zero findings;
- Linux no-default native cross-target Clippy passes with warnings denied;
  FreeBSD no-default library/tests compile and library Clippy passes with only
  two pre-existing test-only `glob_files` warnings; WASI builds the dedicated
  unsupported-target test with only the pre-existing `read_file` dead-code
  warning, and Node's WASI runner actively passes that exact test 1/1;
- a fresh locked release build passes and produces an arm64 Mach-O CLI with
  SHA-256 `57025124a24a636e7fc9639ca5a0f53c75690d3610667d11cc87f2eaeda7f494`;
  executing that binary directly prints the exact version/API line and exits
  zero; and
- 60 Markdown files contain 429 inline links, including 279 repository-relative
  links with zero missing; whole-feature diff checks pass, no unsafe Rust was
  added, and the exact precursor worktree is clean.

These results close the replacement local gate. They do not make cycle 1 green,
do not replace native Linux/macOS feature CI, and do not count as formal
replacement review. Later preparation `491496a` and its immediate tree-identical
marker freeze exact cycle-2 candidate `708f2d0`, which did not pass all three
tracks as recorded above.

## Cycle-2 remediation local gate results

Exact clean remediation precursor
`8432c0c6b5d5955b78a882b651a5bfec76af8814` is green under Rust and Cargo
1.94.1 exactly:

- formatting, workspace/all-target/all-feature warnings-denied Clippy,
  workspace tests, and two doctests pass;
- workspace discovery reports 611 default-feature tests, 660 all-feature tests,
  and zero benchmarks;
- focused evidence passes 30 private `write_file` module tests, including all
  28 supported-platform-submodule tests, 25 direct integration tests, and five
  engine tests;
- Linux no-default native cross-target Clippy passes with warnings denied;
- FreeBSD no-default library and tests compile with exactly two pre-existing
  test-only `glob_files` warnings, and its library Clippy passes with warnings
  denied;
- WASI compiles the `write_file` unsupported-target case and Node actively
  passes it 1/1, with one pre-existing `read_file` warning;
- cargo-deny 0.19.9 passes with only the accepted `syn` and `windows-sys`
  duplicate warnings; cargo-audit 0.22.2 `--no-fetch` checks 1,225 advisories
  over 175 dependencies with zero findings;
- the first Python run was incorrectly overlapped with the LTO release build
  and had one two-second timeout; the isolated retry passes 1/1 and the full
  sequential rerun is green across 129 tests, with 121 passes and eight expected
  macOS skips. This is validation-method contention, not a product failure;
- a fresh clean checkout of pinned fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef` passes the compatibility
  generation `--check`;
- 60 Markdown files contain 429 inline links and 279 repository-relative links
  with zero missing;
- a fresh isolated locked release build produces an arm64 Mach-O binary with
  SHA-256 `f82465e2ac522c3a59b158b8e7bc26e2d369710c2931f8d7483e1fcea4931794`,
  and executing that exact binary emits
  `machine-god 0.1.0 (engine API 1)` and exits zero; and
- whole-feature diff checks pass, no unsafe Rust was added, and the exact
  precursor worktree is clean.

These local gates resolve the two cycle-2 findings in the composed remediation
evidence. They do not retroactively make cycle 2 green, replace the required
three-track cycle 3, prove the native Linux/macOS feature-CI matrix, or establish
remote delivery. A later documentation-only seal or delivery record remains
exempt from another adversarial cycle under the user's instruction, while its
exact feature and `main` workflows remain required.

## Formal adversarial cycle 3

Exact gate record `9a09172ac40d7ec09ebb9fa7a4e4e21f12b2a632`
retains the complete precursor evidence. Exact behavior candidate
`db78c6407c4f603f18e2839a8a291f2de33e579c` is tree-identical to immediate
formal-review preparation parent
`5ed38f3c61d3f29677f41c0b4a41468616a59c7e`. All three fresh tracks returned
**GREEN** with zero findings on that same exact SHA:

- correctness/API: **GREEN**, zero findings;
- filesystem/robustness: **GREEN**, zero findings, with 30 private module tests,
  25 direct tests, and five engine tests rerun under Rust 1.94.1 exactly; and
- performance/concurrency: **GREEN**, zero findings.

Exact-candidate formatting, workspace/all-target/all-feature warnings-denied
Clippy, workspace tests, two doctests, and diff/clean checks are green. The full
precursor gate at `8432c0c` and exact gate record `9a09172` remain retained,
including the initial LTO-overlapped Python timeout and the isolated plus full
sequential green reruns that establish validation-method contention rather than
a product failure. The behavior-green SHA is exactly `db78c640`. Feature CI,
benchmark evidence, fast-forward `main`, and exact `main` workflows remain
pending. This seal changes documentation only and is exempt from further
adversarial review under the user's instruction.

## Explicit nonclaims

This contract adds no CLI behavior, parent creation, external-path access,
symlink-target mutation, target-content read, append/patch operation,
ownership/ACL/xattr preservation, non-Linux/macOS hardened execution,
benchmark workload, compatibility-status change, fx-equivalence claim, or
product-performance claim. Zig remains only the pinned fx benchmark build
input; machine-god and this tool remain Rust.
