# Milestone 03 native `create_folder` review 01

Status: **BEHAVIOR COMPOSED; FULL LOCAL GATE PENDING**

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
- [ ] Concurrent directory appearance and hostile-entry appearance, root and
  prefix replacement, moved retained parents, subordinate mounts, outside
  sentinels, and explicit no-sandbox boundary.
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
- [ ] Linux/macOS behavior, FreeBSD/WASI compile coverage, active unsupported
  target, exact delivered ten-tool checkpoint and composed eleven-tool/ten-
  clone host, no-unsafe, dependency, compatibility, documentation, diff, and
  fresh release smoke evidence.

Exact Rust 1.94.1 focused evidence is green across 16 private, 20 direct, six
engine, seven reference-host, and one core-contract test. Native warnings-
denied Clippy is green. Supported macOS/Linux checks and FreeBSD/WASI check
compilation are green. Production preflight API and filesystem audits each
reported zero findings, but those audits are not any of the required formal
correctness/API, filesystem/robustness, or performance/concurrency tracks.

The complete workspace and doctest gate, repo-wide Python, dependency policy
and audit, compatibility, remaining portability/unsupported behavior,
documentation, clean-diff/no-unsafe, and fresh release-binary checks remain
pending. The unchecked rows require evidence beyond the focused composition.

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

## Current verdict

**BEHAVIOR COMPOSED; FULL LOCAL GATE PENDING.** Production implementation,
exports, focused evidence, and eleven-tool/ten-clone host composition are
present. No immutable behavior-candidate SHA, required formal three-track
review, feature delivery workflow, fast-forward integration, exact `main`
workflow, delivery, product-performance, or fx-equivalence claim exists at
this checkpoint. The delivered-slice count remains twenty-four.
