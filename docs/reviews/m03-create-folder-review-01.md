# Milestone 03 native `create_folder` review 01

Status: **CYCLE 4 PRODUCTION REVIEW GREEN; DOCUMENTATION-SEAL FINDING FIXED;
DELIVERY PENDING**

## Base and boundary

- Exact delivered base:
  `d1a5bc24112bcede8c2d12789e763a12cf44bd4a`.
- Integration branch: `agent/m03-create-folder`.
- Normative contract: [`create-folder.md`](../create-folder.md).
- Base feature CI `32685885104` and benchmark `32685885086` are green.
- Base main CI `32686210561` and benchmark `32686210659` are green.
- Both benchmark workflows retain exactly two nonexpired exact-SHA artifacts.
- Exact frozen contract commit:
  `9fab189c9c1add76a38775d08f4342c6bcc7635b`.
- Contract CI `32687614476` passed all six jobs.
- Contract benchmark `32687614442` passed both jobs and retains exactly two
  nonexpired exact-SHA artifacts.
- Exact replacement-gate remediation:
  `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`.
- Exact replacement-gate tree:
  `40eef148230a79e5d9700b5ca2bdfd0ace2f192c`.

The documentation-only contract commit was exempt from adversarial review
under the user's explicit instruction. Its green exact workflows are contract
evidence only, not implementation, delivery, performance, or fx-equivalence
evidence.

## Frozen feature

The twenty-fifth bounded slice creates one strict canonical confined directory
path and every missing parent. It freezes one `path` field, exact path and JSON
bounds, `FilesystemAccess::Create` authority, descriptor-relative no-follow
execution, recursive at-most-256 `mkdirat` calls, idempotent existing-directory
success, fixed final-nondirectory failure, requested mode `0755` with host
umask/ACL inheritance, no permission normalization, and path-only success.

The first successful or uncertain creation call is the commit boundary.
`mkdirat` is never retried, including after `EINTR`. Postcommit work retains the
created or raced-`EEXIST` suffix, freshly rewalks the public path, and attempts
bottom-up sync of every retained suffix site at no more than 257 sites and
4,112 total calls. Cancellation is ignored after the boundary and no created
prefix is rolled back. Fixed ambiguity covers uncertain creation, a hostile
umask that makes a new intermediate unopenable, moved retained parents, failed
postcommit validation, and durability failure.

The slice adds no file creation, overwrite, deletion, rollback, external path,
symlink following, content access, directory enumeration, chmod/ACL rewriting,
CLI behavior, dependency, benchmark workload, performance claim, or complete
fx-equivalence claim.

## Composed host checkpoint

The last delivered host remains byte-for-byte at ten alphabetical tools:
`copy_file`, `delete_file`, `edit_file`, `file_info`, `glob_files`,
`grep_files`, `list_files`, `read_file`, `rename_file`, and `write_file`, using
one original retained descriptor plus nine clones. Candidate source now inserts
`create_folder` immediately after `copy_file`, yields exactly eleven tools, and
uses one original plus ten identity-preserving clones in both reference-host
construction paths. This composed catalog is not delivered or integrated on
`main`.

## Composed ownership

- Core/production owns the stable existing `Create` capability evidence,
  native implementation and exports, deterministic evidence seams, retained-
  root composition, and reference-host registration; those changes are now
  composed.
- Independent evidence owns direct, private, race, engine, host, core-contract,
  unsupported-target, cancellation, permission, durability, bounds, and
  redaction tests; the focused evidence described below is now composed.
- Documentation owns the normative contract, implementation plan, maintained
  architecture/security/API/host pages, and exact-SHA lineage record.

Owners use isolated worktrees or explicitly non-overlapping files. Only the
composed integration SHA can become a formal behavior candidate.

## Required evidence

- [x] Exact tool/schema/constant/result/open-error and fixed tool-error
  contracts, including strict unknown-field rejection and stable redaction.
- [x] Exact/one-over 4,096-byte path, 256-component, 65,536-byte argument,
  16,384-byte result, 256-`mkdirat`, 257-site, 16-call/site, and 4,112-total
  synchronization bounds.
- [x] Effect-free preparation, exact
  `{"type":"filesystem","access":"create","path":"..."}` evidence,
  denial before lookup, direct canonical validation, and exact
  policy/execution agreement.
- [x] Shallow and deepest recursive creation, already-existing directory
  idempotence, final non-directory conflict, and rejection of every symlink or
  non-directory ancestor without outside effects.
- [x] Concurrent directory appearance and hostile-entry appearance, root and
  prefix replacement, moved retained parents, deterministic mixed-device
  identity traversal, outside sentinels, and explicit no-sandbox boundary.
  This is not privileged real-mount testing or a sandbox guarantee.
- [x] Requested `0755`, host umask/default-ACL inheritance, no chmod or ACL
  rewriting, hostile owner-bit removal, partial-prefix residue, and fixed
  ambiguity.
- [x] One attempt per missing component; no `mkdirat` retry after success,
  failure, `EEXIST`, or `EINTR`; correct EEXIST validation; fresh postcommit
  root-to-final rewalk; bottom-up best-effort sync even after earlier failure.
- [x] No sync before an effect; exact first-effect/uncertain-effect boundary;
  no rollback; precommit cancellation precedence; ignored postcommit
  cancellation; inert first poll, synchronous completion, drop, and no detached
  work.
- [x] Native macOS behavior, Linux/FreeBSD cross-target test compilation,
  Linux library warnings-denied Clippy, WASI compilation and active unsupported
  target, exact delivered ten-tool checkpoint and composed eleven-tool/ten-
  clone host, no-unsafe, dependency, compatibility, documentation, diff, and
  fresh release smoke evidence.
- [ ] Native Linux execution under exact feature CI.

Exact Rust 1.94.1 replacement-gate focused evidence is green across 17 private,
20 direct, six engine, seven reference-host, and one core-contract test.
Production preflight API and filesystem audits each reported zero findings, but
those audits are not any of the required formal correctness/API, filesystem/
robustness, or performance/concurrency tracks.

Cycle-2 evidence remediation adds a seventeenth private test that injects a
different `st_dev` for one existing component and verifies that adjacent mixed-
device identities are independently retained through initial, revalidation,
and postcommit walks. This is deterministic identity-traversal evidence, not a
privileged real mount or a sandbox guarantee. Exact remediation `f527293`, tree
`40eef14`, qualifies the complete 17-test replacement inventory.

## Historical precursor local gate

Exact implementation precursor
`ea408a1f80417475e9b08513a62e9c87b38c4e75`, tree
`7055e930accea4af645b3827b70b5343a8913888`, passes the complete local gate:

- exact Rust 1.94.1 formatting, workspace all-target/all-feature warnings-
  denied Clippy, workspace tests, and workspace doctests are green;
- focused evidence is green across 16 private, 20 direct, six engine, seven
  reference-host, and one core-contract test;
- default and all-feature test discovery finds 877 and 925 tests respectively,
  with zero benchmarks, and the workspace doctest gate runs two doctests;
- all 130 repo-wide Python tests pass with eight expected macOS-only skips;
- compatibility regeneration/checking is green against clean pinned upstream
  `vercel-labs/fx` revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact `cargo-deny` 0.20.2 policy checks pass, with only accepted duplicate-
  version warnings, and `cargo-audit` 0.22.2 checks 175 dependencies against
  1,225 locally retained advisories with zero findings;
- native macOS behavior is green; Linux and FreeBSD no-default cross-target
  test compilation and Linux warnings-denied library Clippy pass; WASI
  compilation passes and the active Node 22.22.0 unsupported-target test passes
  1/1. Native Linux execution remains pending exact feature CI;
- at exact precursor `ea408a1`, 70 Markdown files contain 501 links, including
  351 relative file links, with zero missing relative targets; this maintained
  local-gate record adds one valid relative link, so its current inventory is
  70 files, 502 links, 352 relative file links, and zero missing targets;
- the feature diff adds no unsafe code, dependency, lockfile, CLI, workflow, or
  benchmark-workload change, and the feature worktree is clean; and
- a fresh locked release build produces a 319,152-byte binary with SHA-256
  `71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`;
  exact bare/version/help aliases and inert missing-path human/JSON status
  smokes are green.

At that checkpoint, local success was not delivery, product-performance, or fx-
equivalence evidence. The required immutable same-SHA formal review and remote
delivery gates remained pending.

## Exact local gate before formal review

The composed candidate must first pass focused create-folder private, direct,
engine, host, core-contract, and unsupported-target suites. Then run the exact
Rust 1.94.1 repository gate:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Also require the repository's Python, compatibility, dependency-policy,
dependency-audit, cross-target, active unsupported-target, Markdown-link,
clean-diff, no-unsafe, and freshly built release-binary smoke checks. Record all
exact counts, hashes, versions, expected skips, and any valid exact-toolchain
fallback. Local success is not delivery or performance evidence.

## Formal same-SHA review protocol

After the complete local exact-SHA gate, create a tree-identical behavior
candidate and start three fresh reviewers against that same immutable SHA and
tree:

1. correctness/API;
2. filesystem/robustness;
3. performance/concurrency.

Each reviewer independently verifies the SHA/tree in a clean detached worktree
and runs the applicable focused evidence. Every confirmed finding is fixed, the
complete local gate is rerun, and all three tracks restart with fresh reviewers
on one replacement SHA. Repeat until every track is green with zero findings.

Only then may a documentation seal be pushed for exact feature CI and benchmark
workflows. After those pass for that exact SHA, fast-forward `main` without
force and require exact main CI and benchmark workflows. Each claimed benchmark
run must retain the expected nonexpired exact-SHA artifacts. Documentation-only
seal and final delivery-record commits are exempt from another adversarial
cycle, but their exact workflows remain required.

## Formal review cycle 1

Tree-identical candidate
`8ce899acee73a6dbcc9a80b96722df7e3ba3e9f8`, tree
`065cd190e6a1d9ef065c4d1105eefeb6e32e7583`, is **NOT GREEN**. All three fresh
reviewers independently verified that exact SHA and tree in clean detached
worktrees.

- Correctness/API found one low maintained-documentation inconsistency: the
  repository README and the `create_folder` contract, core API, and security
  pages retained stale complete-local-gate-pending language. It found zero
  production or public-API defects and passed 16 private, 20 direct, six
  engine, seven host, one core-contract, formatting, and focused warnings-
  denied Clippy checks.
- Filesystem/robustness is green with zero findings. It passed 16 private, 20
  direct, six engine, native warnings-denied Clippy, Linux and FreeBSD checks,
  WASI compilation, and the active Node 22.22.0 unsupported-target test 1/1.
- Performance/concurrency found the same low stale-documentation inconsistency
  and zero production performance, concurrency, bounded-work, or resource
  defects. It passed 16 private, 20 direct, six engine, seven host, the exact
  clone-count test, and native warnings-denied Clippy.

The confirmed finding is remediated across every reported maintained passage:
they now identify exact precursor `ea408a1`, tree `7055e93`, as complete-local-
gate green and leave only formal review, delivery, and `main` integration
pending. Production code, API, dependencies, CLI behavior, workflows, and
benchmark workloads are unchanged. A complete replacement local gate and all
three fresh replacement reviewers remain required.

First documentation-remediation commit `f24f98f`, tree `bcd7938`, passed the
required Rust, focused, Python, compatibility, dependency, cross-target, active
WASI, diff, and release-smoke checks. Its documentation shard found that the
historical precursor link counts above needed explicit lineage after the local-
gate record added its own review link. This clarification changes no code or
behavior; a complete exact replacement gate therefore remained required before
cycle 2.

Exact remediation `7bc3fb99359a12320cf1e5aa8f858c1abd0776b2`, tree
`b39bd9b5ea36e3e4b5733f8cae168770e1a9f99d`, passes the complete replacement
local gate. Fresh exact evidence includes:

- required Rust 1.94.1 formatting, workspace all-target/all-feature warnings-
  denied Clippy, workspace tests, and two workspace doctests;
- 16 private, 20 direct, six engine, seven reference-host, and one core-
  contract focused tests; 877 default and 925 all-target/all-feature discovered
  tests with zero benchmarks;
- 130 Python tests with eight expected platform skips, clean byte-identical
  pinned-fx regeneration, `cargo-deny` 0.20.2, and `cargo-audit` 0.22.2 with
  zero findings across 175 dependencies and 1,225 advisories;
- Linux and FreeBSD cross-target test compilation and Linux warnings-denied
  library Clippy, WASI test compilation, and active Node 22.22.0 unsupported-
  target evidence 1/1;
- 70 Markdown files, 502 links, 352 repository-relative links, and zero missing
  targets; clean diff, zero unsafe additions, and no Cargo, lockfile, CLI,
  workflow, or benchmark-workload change; and
- a fresh 319,152-byte locked release binary with SHA-256
  `71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`
  passing exact bare/version/help and inert human/JSON status smoke paths.

This local gate is not delivery, product-performance, or fx-equivalence
evidence. A tree-identical replacement candidate and all three fresh cycle-2
tracks remain pending.

## Formal review cycle 2 and evidence remediation

Tree-identical candidate
`6e1f885aa1e167e902b5cda729023fd7c283895e`, tree
`ac57575c3ee300050f5a92d4cae5f507fe654002`, is **NOT GREEN**.

- Correctness/API is **GREEN** with zero findings.
- Performance/concurrency is **GREEN** with zero findings.
- Filesystem/robustness is **NOT GREEN** with two low findings and zero
  production defects.

The first low finding is an evidence gap: the checked subordinate-mount row had
no deterministic proof that adjacent traversed components may retain different
device identities. Evidence remediation adds private test
`adjacent_mixed_device_identities_are_retained_across_every_walk_phase`, which
injects a changed `st_dev` for the first existing component, observes that
component once during each initial, revalidation, and postcommit identity walk,
and preserves the expected one-create and bottom-up sync sequence. This proves
mixed-device identity traversal through the deterministic evidence boundary. It
does not perform a privileged real mount and does not establish a filesystem-
sandbox guarantee.

The second low finding is maintained-documentation overclaim: the exact local
evidence ran native macOS behavior, while Linux evidence was cross-target test
compilation and warnings-denied library Clippy. The maintained contract,
architecture, API, security, host, plan, README, and this review now state that
native Linux execution remains pending exact feature CI. Historical exact-gate
counts remain unchanged; the composed remediation inventory is now 17 private,
20 direct, six engine, seven reference-host, and one core-contract test.

These changes remediate both low findings without changing production code,
public API, authority, dependency, CLI, workflow, benchmark workload, or the
no-sandbox boundary. They do not retroactively make cycle 2 green. Exact
remediation `f527293`, tree `40eef14`, now passes the complete replacement local
gate; three fresh same-SHA cycle-3 tracks remain required.

## Exact replacement local gate after cycle 2

Exact remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate. Fresh exact evidence is:

- exact Rust 1.94.1 workspace formatting, all-target/all-feature warnings-
  denied Clippy, full workspace tests, and two workspace doctests;
- 17 private, 20 direct, six engine, seven reference-host, and one core-
  contract focused tests;
- 878 default and 926 all-target/all-feature discovered tests, with zero
  benchmarks;
- 130 repo-wide Python tests with eight expected macOS skips;
- byte-identical compatibility regeneration against clean pinned fx revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact `cargo-deny` 0.20.2 policy checks and `cargo-audit` 0.22.2 with zero
  findings across 175 dependencies and 1,225 advisories;
- Linux and FreeBSD cross-target checks and Linux warnings-denied library
  Clippy only; WASI compilation and active Node 22.22.0 unsupported-target
  evidence 1/1. Native Linux execution remains pending exact feature CI;
- 70 Markdown files, 502 links, 352 relative file links, and zero missing
  targets;
- clean diff, no unsafe additions, and no Cargo manifest, lockfile, CLI,
  workflow, or benchmark-workload changes; and
- a fresh 319,152-byte Mach-O arm64 release binary with SHA-256
  `71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`,
  passing exact bare, version, help, and inert missing-path human/JSON smokes.

The next candidate must preserve this exact non-documentation behavior.
Documentation-only gate record `9d0bacd656d09b8ff57edfbfe7cbf701af9fef1e`,
tree `b5fb1c2b5268e46793d48be1a02611381feca7c3`, records the result without
changing non-documentation files. Cycle-3 candidate
`c1e572eb1ac1ac39a8a53f522e74f57fd1d4f85d` is tree-identical to that
documentation record and retains non-documentation behavior identical to
remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`. This local result is not
delivery, `main` integration, product-performance, or fx-equivalence evidence.

## Formal review cycle 3 and lineage remediation

All three fresh formal tracks reviewed exact cycle-3 candidate
`c1e572eb1ac1ac39a8a53f522e74f57fd1d4f85d`, tree
`b5fb1c2b5268e46793d48be1a02611381feca7c3`, from clean detached worktrees:

- Correctness/API is **NOT GREEN** with one LOW documentation/evidence finding
  and zero production/API defects. The prior paragraph incorrectly required
  the whole candidate tree to equal pre-record remediation tree `40eef148`,
  even though documentation-only gate record `9d0bacd`, tree `b5fb1c2`, is the
  candidate's exact tree-identical parent. The corrected lineage above now
  distinguishes whole-tree identity from unchanged non-documentation behavior.
- Filesystem/robustness is green with zero findings and zero production or
  evidence defects. It passed the 17 private, 20 direct, and six engine tests;
  native warnings-denied Clippy; Linux/FreeBSD cross-target checks; Linux
  warnings-denied library Clippy; WASI compilation; and active Node 22.22.0
  unsupported-target evidence 1/1.
- Performance/concurrency is green with zero findings and zero production or
  evidence defects. It passed the 17 private, 20 direct, six engine, seven host,
  exact ten-clone, 256-component, and 4,112-sync-bound evidence plus native
  warnings-denied all-target/all-feature Clippy.

Cycle 3 is not green because every track must report zero findings. All three
tracks found zero production defects. This documentation-only lineage
remediation does not change production, tests, API, authority, dependencies,
CLI behavior, workflows, or benchmark workloads.

## Exact replacement gate after cycle 3

Exact lineage remediation `12c11baa0187f530a6c088326b869991f6f627f6`, tree
`b96575b57c2c805a845294ae16b323dd1ea4ecd2`, passes the complete replacement
gate. Fresh exact evidence is unchanged in scope and includes exact Rust 1.94.1
workspace formatting, warnings-denied all-target/all-feature Clippy, full
workspace tests, two doctests, focused 17/20/6/7/1, discovery 878/926/0, Python
130 with eight expected macOS skips, byte-identical pinned-fx compatibility,
`cargo-deny` 0.20.2, `cargo-audit` 0.22.2 with zero findings across 175
dependencies and 1,225 advisories, Linux/FreeBSD cross-target checks, Linux
warnings-denied library Clippy, WASI compilation and active Node 22.22.0 1/1,
documentation 70/502/352/0, clean diff/no unsafe/no forbidden changes, and the
fresh 319,152-byte Mach-O arm64 release SHA
`71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`
passing bare/version/help/inert human-and-JSON smokes. Native Linux execution
remains pending exact feature CI. Documentation-only gate record
`f6f65847a47a009b5203044ce18e6f0c4253f17a` is the parent of tree-identical
cycle-4 candidate `a78b693e5ce45688084fe1215073e2d859f2d438`; both have tree
`2b913e8d65b1da518f2c148f1b9b1b6b899e1e64` and retain non-documentation
behavior identical to `12c11ba` and `f527293`.

## Formal review cycle 4 and exempt seal correction

All three entirely fresh formal tracks reviewed exact cycle-4 candidate
`a78b693e5ce45688084fe1215073e2d859f2d438`, tree
`2b913e8d65b1da518f2c148f1b9b1b6b899e1e64`, from clean detached worktrees:

- Correctness/API is green with zero BLOCKER, HIGH, MEDIUM, or LOW findings and
  zero production/API or evidence/documentation defects. It passed 17 private,
  20 direct, six engine, seven host, one core-contract, formatting, and native/
  core warnings-denied Clippy evidence and independently checked pinned-fx and
  candidate/remediation lineage.
- Performance/concurrency is green with zero findings of every severity and
  zero production, resource, evidence, or documentation defects. It passed 17
  private, 20 direct, six engine, seven host, one core-contract, exact ten-clone,
  256-component, 4,112-sync-bound, and native warnings-denied Clippy evidence.
- Filesystem/robustness found zero production or filesystem-behavior defects and
  one LOW documentation/evidence finding at the prior current-verdict sentence:
  the gate record and cycle-4 candidate were described as pending even though
  `f6f6584` already directly parents tree-identical `a78b693`. Its 17 private,
  20 direct, six engine, seven host, one core-contract, native warnings-denied
  Clippy, Linux/FreeBSD cross-target, Linux warnings-denied library Clippy, WASI
  compile, and active Node 22.22.0 evidence 1/1 were green.

The low finding is solely a stale documentation-gate self-record introduced
before the empty candidate existed; every production review scope has zero
findings. This sentence and the current verdict below correct it by recording
the exact `f6f6584` parent and `a78b693` candidate relationship. Under the
user's explicit instruction that documentation seal commits do not need a
separate adversarial review, this documentation-only seal correction does not
start a cycle-5 review. It changes no production, tests, API, authority,
dependencies, CLI behavior, workflows, or benchmark workloads.

## Current verdict

**CYCLE 4 PRODUCTION REVIEW GREEN; DOCUMENTATION-SEAL FINDING FIXED; DELIVERY
PENDING.** Production
implementation, exports, eleven-tool/ten-clone host composition, and the 17-
private/20-direct/6-engine/7-host/1-core-contract evidence inventory are
composed. Cycle 2 found zero production defects, its two low findings are
remediated, and exact remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`,
tree `40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate. Documentation record `9d0bacd`, tree `b5fb1c2`, and tree-identical
cycle-3 candidate `c1e572e` retain identical non-documentation behavior. Cycle
3 is not green only for one low documentation-lineage finding; filesystem and
performance are green, and all three tracks found zero production defects.
Exact lineage remediation `12c11ba`, tree `b96575b`, passes the complete
replacement gate. Documentation gate record `f6f6584` parents tree-identical
cycle-4 candidate `a78b693`, tree `2b913e8`. Cycle 4 found zero production
defects. Correctness/API and performance/concurrency are green with zero
findings; filesystem/robustness found only the low stale documentation-seal
sentence corrected here under the user's explicit seal-review exemption. Native
Linux execution remains pending exact feature CI. Feature delivery workflow,
fast-forward integration, exact `main` workflow, delivery, product-performance,
and fx-equivalence claims also remain pending. The delivered-slice count remains
twenty-four.
