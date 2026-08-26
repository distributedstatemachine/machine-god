# Milestone 03 sessions CLI review ledger

Status: cycle 1 rejected; the locally green cycle-2 replacement is assembled
and awaiting three fresh reviews. Bounded slice 31 starts from exact base
`feaf9fa1bc6bb66544947152e2c5fe91c8cd185e`. No formal green or delivery verdict
exists yet.

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

### Rejected cycle 1

Exact behavior candidate
`9448738908e014c9273960d9e7dd0790e0525848`, tree
`e5342f79207af6cc41da71a280d8ca9ef3952756`, passed the complete local gate
with exact Rust and Cargo 1.94.1, without fallback:

- focused native process listing 9/9, native store listing 18/18, CLI sessions
  unit 11/11, CLI sessions integration 10/10, and benchmark schema/mutation
  3/3;
- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- all workspace tests and workspace doctests;
- the complete Python discovery suite: 135 tests with 8 platform skips;
- pinned-fx compatibility generation at exact revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- native WASI and FreeBSD checks plus a CLI WASI check;
- a fresh release-binary smoke over two canonical records, exact human and
  JSON bytes, and missing-root no-create behavior; and
- documentation integrity over 82 Markdown files, 141 fenced blocks, 599
  links, and 78 repository targets with zero errors.

Three fresh, isolated, read-only agents reviewed that exact SHA:

| Track | Blocker | High | Medium | Low | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| CLI correctness/API/fx boundary | 0 | 0 | 0 | 3 | REJECTED |
| Native state/persistence/portability | 0 | 0 | 0 | 0 | GREEN |
| Performance/resource/evidence | 0 | 0 | 0 | 0 | GREEN |
| Deduplicated union | 0 | 0 | 0 | 3 | REJECTED |

The CLI track found that the process facade used the general native environment
snapshot and therefore captured unrelated `XDG_CONFIG_HOME`, captured `HOME`
even when a nonempty `XDG_STATE_HOME` was authoritative, and left this ledger
and the maintained plan at their pre-candidate state. All three findings are
accepted. The replacement will use a state-only injected environment reader,
request `HOME` only after missing or empty XDG state, add exact request-order
regressions, and update the maintained review state. Candidate `9448738` is
permanently rejected even though its other two tracks were green.

### Cycle 2 replacement submission

Native remediation component
`04dd6d9804f57413216e12c9d80a76cd9601fe30`, tree
`258afe779375860bf18aecf932f4768615b56c92`, changes only
`session_listing.rs`. The process entrypoint now captures state inputs through
an injected reader: construction remains inert; first poll requests
`XDG_STATE_HOME`; only missing or empty XDG state permits a `HOME` request; and
no branch requests `XDG_CONFIG_HOME`. Four new unit tests prove exact request
sequences for absolute, relative, non-Unicode, empty, and missing XDG values and
prove first-poll timing.

The fully composed replacement source in this commit also updates the frozen
contract and every maintained status summary. It passed the complete exact
Rust/Cargo 1.94.1 replacement gate without fallback: focused 12 native facade
unit, 9 native process, 18 native scan, 11 CLI sessions unit, and 10 CLI
sessions integration tests; format, workspace Clippy with warnings denied, all
workspace tests, and all workspace doctests; 135 Python tests with 8 platform
skips; exact pinned compatibility regeneration; native WASI and FreeBSD plus
CLI WASI checks; and a freshly built release-binary seeded/missing-root smoke.
The known WASI-only `read_file.rs` dead-code warning predates and is unrelated
to this slice.

This commit is submitted as the cycle-2 behavior candidate. Its immutable SHA
and tree are supplied to all three reviewers by the coordinator and will be
appended in the review-exempt verdict seal, because a commit cannot contain its
own hash. No green inference is made from the local gate.

Three new adversarial agents must review the replacement exact SHA on these
tracks:

1. CLI correctness, API contract, and pinned-fx compatibility boundaries;
2. native state-root, persistence, error, and portability behavior; and
3. performance, resource, concurrency, benchmark, and delivery evidence.

Any finding rejects that exact candidate. Findings, remediation commits,
replacement gates, fresh review verdicts, and exact trees will continue to be
appended here until all three tracks report zero blocker, high, medium, and low
findings.

## Delivery gate

After a green candidate, a documentation-only review seal may record the exact
verdict. The feature branch must then pass exact-SHA CI and benchmark-evidence
workflows before `main` is fast-forwarded without force. The exact integrated
`main` SHA must pass both workflows. No package publication or GitHub release is
authorized.
