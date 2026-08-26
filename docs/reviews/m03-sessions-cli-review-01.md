# Milestone 03 sessions CLI review ledger

Status: pre-review. Bounded slice 31 is under implementation from exact base
`feaf9fa1bc6bb66544947152e2c5fe91c8cd185e`. No formal behavior candidate or
green verdict exists yet.

## Frozen boundary

The reviewed unit will be strict top-level `sessions [--json]`, its engine-free
native process-listing facade, independent native and release-binary evidence,
the claim-ineligible benchmark classification change, and the maintained
documentation. The normative contract is
[`docs/sessions-cli.md`](../sessions-cli.md).

Implementation is split into isolated, non-overlapping worktrees:

| Component | Owned files |
| --- | --- |
| Native production | Native session-list facade, root-policy extraction, store/lifecycle sharing, and exports. |
| CLI production | `crates/machine-god-cli/src/main.rs`. |
| Independent evidence | Native/CLI integration tests, benchmark schema/tests, and CI release smoke. |
| Documentation/integration | Contract, plan, maintained summaries, composition, and gates. |

Three read-only discovery agents independently audited the pinned fx contract,
the native composition boundary, and the evidence/benchmark surface. They are
not formal reviewers and will not be reused for the required post-gate review.

## Candidate and review cycles

No candidate has been submitted. After the fully composed exact Rust 1.94.1
gate passes, three fresh adversarial agents will review:

1. CLI correctness, API contract, and pinned-fx compatibility boundaries;
2. native state-root, persistence, error, and portability behavior; and
3. performance, resource, concurrency, benchmark, and delivery evidence.

Any finding rejects that exact candidate. Findings, remediation commits,
replacement gates, fresh review verdicts, and exact trees will be appended
here until all three tracks report zero blocker, high, medium, and low findings.

## Delivery gate

After a green candidate, a documentation-only review seal may record the exact
verdict. The feature branch must then pass exact-SHA CI and benchmark-evidence
workflows before `main` is fast-forwarded without force. The exact integrated
`main` SHA must pass both workflows. No package publication or GitHub release is
authorized.
