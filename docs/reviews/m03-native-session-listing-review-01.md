# Milestone 03 native session-listing review 01

Status: **COMPOSED CANDIDATE — FORMAL REVIEW AND DELIVERY PENDING**

This record reserves the review lineage for the sixteenth bounded Milestone 03
slice. The candidate is being composed from exact integrated base
`9ada4b5429b89138cc18aec2b8e88e610b1df2fb`. No production SHA, independent-test
SHA, composed-candidate SHA, review result, local-gate result, remote workflow,
or delivery SHA is recorded here until that evidence exists.

Per the user's instruction, a later documentation-only seal does not require a
new adversarial review after the behavior candidate itself is green. That does
not waive the formal review required for the production and independently owned
test composition.

## Candidate boundary

The proposed Linux/macOS library slice adds
`NativeSessionLifecycle::list_sessions`, returning an IDs-only
`NativeSessionList` and `truncated`. Its normative contract is
[`native-session-listing.md`](../native-session-listing.md).

The fixed ceilings are 100 returned IDs, 1,024 visible scanned entries, and
64 MiB of aggregate canonical record bytes. Every visible entry consumes scan
budget. Exact canonical candidates use the current strict file-session schema,
same no-follow regular-file validation, per-ID locks, decoded-ID/digest check,
and redacted `Corrupt` or `Unavailable` failures. A nonregular derived lock for
a present canonical record is corrupt; ordinary lock I/O is unavailable.
Candidate filenames are sorted before validation, and only a fired raw scan cap
makes candidate selection filesystem-iteration-dependent. Returned IDs are
sorted and unique. Truncation means only incomplete bounded observation, not
pagination or a globally ranked ID prefix.

The slice adds no multi-record snapshot, live-registry/source/provider/tool or
network authority, rich summaries, workspace/latest/cursor semantics, CLI
surface, compatibility claim, benchmark implementation, or product-performance
claim. Its future is inert before polling, performs bounded synchronous work on
first poll, and detaches no effects. Successful validation may create private
`0600` permanent lock sidecars.

## Required independent evidence

Before the behavior can be called green, independently owned tests must cover:

- empty, sorted, unique and boundary-sized successful results;
- every scan, aggregate-byte and returned-ID truncation edge, including
  filesystem-iteration dependence only when the raw scan cap fires;
- ignored but scan-counted unrelated, lock, temporary and noncanonical names;
- exact canonical recognition, strict schema and decoded-ID/digest validation;
- symlink and nonregular rejection, oversized and corrupt records, redacted
  failures, and unavailable enumeration/read/lock paths;
- omission when a candidate vanishes before its locked read, including the
  allowed permanent lock sidecar, plus corrupt nonregular derived locks for
  still-present candidates;
- use of the same per-ID lock protocol and private sidecar creation;
- concurrent cooperating writer behavior without a multi-record snapshot
  claim; and
- unpolled inertness, synchronous first-poll behavior, no detached work, and
  absence of provider, registry, source, prompt, tool, network and workspace
  effects.

## Formal review tracks

The following fresh reviews remain pending on one exact composed behavior SHA:

1. Correctness, public API, bounds, portability, and independent-test coverage.
2. Filesystem safety, corruption behavior, redaction, locking, concurrency, and
   resource-abuse resistance.
3. Performance/concurrency semantics, documentation consistency, compatibility
   non-claims, and evidence integrity.

Every confirmed finding must be fixed and rereviewed until all three tracks are
green on the same exact behavior commit. The repository's focused and required
Rust 1.94.1 local gates, exact feature-branch CI and benchmark-evidence
workflows, fast-forward integration, and exact `main` workflows also remain
pending.

## Scope remaining after this slice

Composition of this slice completes the combined root plus native
create/list/resume/replay/reset library checklist item. Milestone 03 remains in
progress because the remaining native tool set, top-level CLI and slash-command
ownership, and composed freshly built release-binary end-to-end evidence remain
open. The machine-god `sessions-json` benchmark is still unimplemented and
claim-ineligible, and Zig remains benchmark-only.
