# Milestone 01 completion evidence

Milestone 01's deliverables are complete at
`b766188999468d281da9bb02e2494c9f187c4ae2`. This document records the
integrated feature commits and their retained remote checks. `COMPLETE` is the
status of the milestone deliverables; it does not waive the delivery workflow's
review and exact-SHA CI gates for a later documentation or integration commit.

## Scope and outcomes

Milestone 01 established the Rust 1.94.1 workspace, documentation and
architecture boundaries, pinned-action CI, dependency-policy and vulnerability
checks, a generated compatibility inventory for the pinned upstream fx commit,
and a reproducible upstream benchmark-evidence path. In particular:

- bootstrap and evidence-validation work completed at
  `3eb3a646950e4a6d85f19a5af18a3616a8bf17f3`;
- CI and developer-experience hardening completed at
  `bd37cab345dd0965b4c0b19140783ca73802b655`;
- compatibility inventory generation completed at
  `954b702f4a22fe1f86b49a0a0d7ddc8371a253aa`; and
- the upstream benchmark harness completed at
  `b766188999468d281da9bb02e2494c9f187c4ae2`.

Git ancestry confirms each of those commits is integrated into the Milestone 01
tip. The later core-contracts candidate
`49bbb168e33023dda16f7e2f5002474df8965fd4` is the base of this documentation
change only because Milestone 02 work has already begun. It is not a Milestone
01 deliverable and is not evidence for Milestone 01 completion.

Feature-level adversarial-review history is retained in the
[bootstrap](m01-bootstrap-review-01.md),
[compatibility](m01-compatibility-review-01.md), and
[upstream benchmark](m01-upstream-benchmark-review-01.md) reports.

## Remote evidence

All entries below completed successfully for the exact listed commit.

| Feature | Ref | Workflow | Run |
| --- | --- | --- | --- |
| Bootstrap | feature branch | CI | [32357945652](https://github.com/distributedstatemachine/machine-god/actions/runs/32357945652) |
| Bootstrap | `main` | CI | [32358262968](https://github.com/distributedstatemachine/machine-god/actions/runs/32358262968) |
| Bootstrap | `main` | Benchmark evidence | [32358263026](https://github.com/distributedstatemachine/machine-god/actions/runs/32358263026) |
| CI and DX | feature branch | CI | [32360949805](https://github.com/distributedstatemachine/machine-god/actions/runs/32360949805) |
| CI and DX | `main` | CI | [32361407467](https://github.com/distributedstatemachine/machine-god/actions/runs/32361407467) |
| CI and DX | `main` | Benchmark evidence | [32361407518](https://github.com/distributedstatemachine/machine-god/actions/runs/32361407518) |
| Compatibility inventory | feature branch | CI | [32372092445](https://github.com/distributedstatemachine/machine-god/actions/runs/32372092445) |
| Compatibility inventory | `main` | CI | [32372679735](https://github.com/distributedstatemachine/machine-god/actions/runs/32372679735) |
| Compatibility inventory | `main` | Benchmark evidence | [32372679705](https://github.com/distributedstatemachine/machine-god/actions/runs/32372679705) |
| Upstream benchmark | feature branch | CI | [32378079710](https://github.com/distributedstatemachine/machine-god/actions/runs/32378079710) |
| Upstream benchmark | feature branch | Benchmark evidence | [32378079777](https://github.com/distributedstatemachine/machine-god/actions/runs/32378079777) |
| Upstream benchmark | `main` | CI | [32378774285](https://github.com/distributedstatemachine/machine-god/actions/runs/32378774285) |
| Upstream benchmark | `main` | Benchmark evidence | [32378774221](https://github.com/distributedstatemachine/machine-god/actions/runs/32378774221) |

The bootstrap feature-branch CI and first `main` runs cover commit
`3eb3a646950e4a6d85f19a5af18a3616a8bf17f3`; the CI/DX runs cover
`bd37cab345dd0965b4c0b19140783ca73802b655`; the compatibility runs cover
`954b702f4a22fe1f86b49a0a0d7ddc8371a253aa`; and the upstream benchmark runs
cover `b766188999468d281da9bb02e2494c9f187c4ae2`.

## Known limitations

- This milestone establishes scaffolding, inventory, and evidence collection;
  it does not provide a usable agent engine. Engine delivery begins in
  Milestone 02.
- Inventory entries describe upstream compatibility targets, not implemented
  machine-god behavior.
- Benchmark artifacts retained by GitHub Actions are bootstrap evidence and may
  expire after 90 days. Durable product performance claims still require
  committed, reviewed summaries with input and result digests.
- Zig 0.16.0 is used only to build the pinned upstream fx reference. The
  machine-god product and workspace are written in Rust.
- Benchmark CI downloads Zig from the official
  [`zig-x86_64-linux-0.16.0.tar.xz`](https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz)
  archive and verifies official SHA-256 digest
  `70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
  before extraction. It does not depend on a third-party Zig setup action.
- Package publication and GitHub releases remain outside the authorized scope.

## Milestone review gate

Review and CI attest immutable commits from outside the commit they attest. A
candidate cannot append its own exact-SHA results without changing that SHA and
invalidating the results. The coordinator therefore records fresh
correctness/API, security/abuse, and performance/concurrency review results in
the orchestration record, requires remote feature-branch checks for that same
immutable SHA, and merges only after all gates pass. Evidence-only documents
such as this one cite earlier reviewed SHAs and retained workflow runs; they do
not claim to attest their own commit. The resulting exact `main` SHA receives
fresh CI and benchmark-evidence runs after the fast-forward push.

## Completion-candidate operability review

An operability review of
`19ca28e85655614c5756cd3a680b78b5cc8fa5ca` found three issues:

- benchmark CI used a third-party Node-based Zig setup action instead of binding
  the official compiler archive directly;
- the documented instruction to append exact review and CI results to the
  reviewed commit was self-invalidating because the append changes its SHA; and
- “benchmark baseline” overstated evidence that is explicitly non-product
  bootstrap evidence.

This follow-up replaces the action with a direct official Zig 0.16.0 HTTPS
download, verifies SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extraction, and asserts the exact installed version. It also defines the
external immutable-SHA gate above and consistently calls the Milestone 01 output
a benchmark harness with non-product bootstrap evidence. These are resolutions
of findings against `19ca28e`; this record makes no claim about final review of
the follow-up commit that contains it.
