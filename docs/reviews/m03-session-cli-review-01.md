# Milestone 03 session CLI review ledger

Status: exact cycle-1 candidate
`5381d4b4dda2b609f256ec7237e0c4435b40a165`, tree
`4435bdeac6ffc1df5d5c8f68515082cd167dfc61`, passed its exact same-SHA local
gate but is rejected by formal review. Documentation remediation is recorded
here; production remediation, its complete replacement gate, three fresh
replacement reviews, remote delivery gates, and integration remain pending.
Bounded slice 32 starts from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Initial
composition was `852fec7`; focused composition-gate remediation advances the
production precursor to exact `c0c16a745943a97330223aafd4a6f6a7dce84ca6`,
tree `61bcf619fc9190a9a70ab3a9c643605c88ab1817`.

## Frozen boundary

The composed source implements strict top-level `session <id> [--json]`, the
engine-free native by-ID inspection facade, independent native/CLI evidence,
and the release-smoke workflow definition. The normative contracts are
[`docs/session-cli.md`](../session-cli.md) and
[`docs/native-session-inspection.md`](../native-session-inspection.md).

Two independent read-only discovery agents examined the remaining Milestone 03
CLI work and the pinned fx boundary. Both recommended the same summary-only
slice and independently rejected `ask`, `resume`, `replay`, `workspace`, and
parser-only slash commands as larger or semantically incomplete next steps.
Discovery agents are not formal reviewers and will not be reused for the
required post-gate review.

Implementation used isolated worktrees with non-overlapping ownership:

| Component | Owned files |
| --- | --- |
| Native production/evidence | Inspection/state capture, exports, and focused tests. |
| CLI production/unit evidence | `crates/machine-god-cli/src/main.rs`. |
| Independent evidence | CLI process tests and release smoke. |
| Documentation/integration | Contracts, maintained summaries, composition, complete gates, and review ledger. |

## Composed precursor lineage

The isolated components and their feature-branch compositions are:

- native production and focused evidence: original
  `5fa9e6075ccaf7c036e6ec794a2e430fe0b3c304`, tree
  `a6f0d247c0e3d131189fc09fcd35977d5df52a67`, integrated as
  `10a53330737ed463e31ae089dc414cef5c39f752`;
- CLI production and unit evidence: original
  `6eb275c4e1b103b8c2b99e956013cf2bf929f3f6`, tree
  `962f2a2d8529188c0861120bce8a59e309f983e5`, integrated as
  `412d63b5926cea346781dfa807402af287462a13`; and
- independent process and workflow evidence: original
  `463f7a1ebc70057d49b002634f0a235624b18634`, tree
  `48bcffa49701c7b5294437e229af8cc8c973d1cf`, integrated as
  `55f37fc62b230240898c3c859e85f2d87f166292`.

Exact initial composition precursor
`852fec7720e5714fff71d39e211deea740eac2b1`, tree
`cf0ad84945e4030fbc2c5fbfb996b2f484ed2952`, adds one production composition
fix: non-exhaustive native error categories fail closed to the CLI's
`Unavailable` category, and the help output aligns to the frozen
`Inspect a saved session` bytes.

Exact focused composition-gate remediation
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`, makes the integrated native/CLI
all-target/all-feature warnings-denied Clippy gate green and separates the
session grammar evidence into its own unit test without changing the frozen
command behavior.

Exact gate precursor `fa099f75277f7ae23a3ac220e66356c45223d1a5`, tree
`64d6a72e66b6df78bc476dadd82ce3e911644b2d`, composes production, independent
evidence, and maintained documentation and passed the complete required local
gate described below.

Focused exact Rust/Cargo 1.94.1 evidence is green:

- 12 native session-inspection tests;
- 56 CLI unit tests;
- 46 independent CLI process tests.

The focused native/CLI all-target/all-feature warnings-denied Clippy gate is
green. The complete gate evidence is recorded below.

The fixed benchmark inventory and generated compatibility records are
unchanged. This slice remains deliberately non-equivalent, unmeasured, and
claim-ineligible. The composition adds no dependency or unsafe Rust.

Exact precursor `fa099f7` is superseded as a submission by exact cycle-1
candidate `5381d4b`. That candidate has undergone formal adversarial product
review and is rejected, not review-green or delivered. Remote workflows,
`main` integration, and delivery remain pending. The delivered count stays
thirty-one.

## Complete local gate evidence

Exact precursor `fa099f75277f7ae23a3ac220e66356c45223d1a5`, tree
`64d6a72e66b6df78bc476dadd82ce3e911644b2d`, passed all four required commands
under exact Rust/Cargo 1.94.1 without fallback:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Documentation integrity covered 85 Markdown files, 146 fenced blocks, 620
parsed links, and 81 unique repository targets with zero errors. Supplemental
gate evidence is also green:

- the complete Python discovery suite passed 135 tests with eight expected
  macOS skips;
- pinned-fx compatibility regeneration passed at exact upstream
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- native WASI no-default-feature and all-feature checks, the CLI WASI check,
  and native FreeBSD no-default-feature check passed. The only diagnostic was
  the documented pre-existing WASI `read_file` `dead_code` warning;
- diff checks and an explicit unsafe-Rust scan passed; and
- a freshly rebuilt exact-tree release binary passed success human and JSON,
  invalid-grammar-before-effects, missing-root JSON `NotFound`/no-create,
  record-immutability, private-lock, and unrelated-root checks.

Compatibility regeneration is evidence integrity only. It does not promote
this deliberately non-equivalent, unmeasured, claim-ineligible command or make
a performance or compatibility claim.

This documentation-only record changed the exact commit and tree after those
precursor results. Exact candidate
`5381d4b4dda2b609f256ec7237e0c4435b40a165`, tree
`4435bdeac6ffc1df5d5c8f68515082cd167dfc61`, then passed the exact same-SHA
gate before formal review. That gate result did not itself approve review
findings or delivery.

## Formal cycle 1 verdict

Three fresh isolated agents reviewed exact candidate `5381d4b`, tree
`4435bde`, across the required tracks. Counts are blocker/high/medium/low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Green | `0/0/0/0` |
| Native boundary/effects and portability | Rejected | `0/0/0/1` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/1/2` |

The findings are:

1. **Medium — full-record materialization.** The six-field summary reads the
   complete raw record JSON, deserializes the complete envelope, and owns a
   complete `SessionRecord` before dropping transcript and metadata content.
   Bounded final retention does not make that load summary-oriented.
2. **Low — engine-limit contract overclaim.** The store proves its own current-
   schema, file-byte, aggregate JSON depth/node, identifier, counter, and
   content-shape constraints. It does not run the engine's configurable/default
   4,096-message, 8 MiB serialized-transcript, or 256 KiB serialized-metadata
   validation. Store-valid historical or differently configured records over
   those engine limits remain inspectable. The native track's sole low and one
   performance low are this same finding.
3. **Low — latency/attempt overclaim.** Retained summary and successful
   transferred bytes/work have finite ceilings, but exclusive sidecar-lock
   acquisition, filesystem latency, and retries after `EINTR` have no wall-
   clock or attempt bound and synchronously block the polling and CLI thread.

The performance track reported these three findings: one medium and two lows.
The native track additionally reported the same engine-limit low. There are
therefore three unique findings, and deduplicating the overlapping native/
performance low yields `0/0/1/2`. Any finding rejects the exact candidate, so
`5381d4b` is rejected.

This documentation commit corrects the two normative low themes but neither
implements nor claims the production optimization. Production remediation,
the complete replacement exact-SHA local gate, and a new three-agent review
cycle are pending. No remote workflow, `main` integration, delivery,
performance, compatibility-promotion, or fx-equivalence claim is made.

## Required replacement adversarial product review

After production and documentation remediation are composed and the replacement
commit passes its exact same-SHA local gate, three fresh isolated read-only
agents must review that replacement exact commit and tree:

1. correctness/API, including CLI grammar, output/error behavior, and the
   pinned-fx boundary;
2. native boundary/effects, including state roots, persistence, races, errors,
   and portability; and
3. performance/concurrency/resources, including benchmark classification and
   evidence completeness.

Every report must end with exact counts for blocker, high, medium, and low
findings. Any finding rejects the entire candidate. Remediation must be
composed, the complete replacement gate rerun, and three new agents must review
the replacement exact SHA. No prior discovery, implementation, review, or
remediation agent may approve its own work or be reused in a later review
cycle. Only a deduplicated `0/0/0/0` candidate is formally green.

## Delivery gate

A documentation-only review seal may record returned zero-finding verdicts
without another product review. The sealed feature SHA must pass CI and
benchmark-evidence workflows before `main` is fast-forwarded without force.
The exact integrated `main` SHA must pass both workflows, and each benchmark
run must retain exactly two unexpired exact-SHA artifacts. No package
publication or GitHub release is authorized.
