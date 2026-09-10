# Milestone 05 `background` CLI review history

This is the compact historical review record for the contract in
[`../background-cli.md`](../background-cli.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`a665289da640d69ae88d0a4a336ad90ece889086`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict command forms, stable rendering, exact record schema, duplicate and unknown field rejection, canonical stored paths, normalized configured bases, fixed error taxonomy, and pinned compatibility review. |
| Lifecycle, platform, and effects | Green | Zero findings after descriptor-relative no-follow traversal, Linux `openat2` and `O_NOATIME`, macOS `O_NOFOLLOW_ANY` and ACL policy, bounded environment reads, read-only operation, unsupported FreeBSD/WASI behavior, and native macOS tests. |
| Performance and resources | Green | Zero findings after streaming preflight, fixed depth/node/record/file/identifier/path/argument bounds, early oversized-input rejection, bounded rendering, and no-write regression review. |

## Rejected candidates and remediation

| Candidate range | Decisive findings | Replacement |
| --- | --- | --- |
| `7600463` through `2080351` | Persisted records and stored paths were too permissive, and the unsupported-platform terminal surface failed warnings-denied WASI compilation. | `09e49ed` through `2733b0c` made record decoding strict and bounded, rejected non-canonical stored paths, streamed JSON preflight, added no-follow state-base traversal, fixed the compile incompatibility, and enforced FreeBSD/WASI CI. |
| `2733b0c` through `548eff5` | Large identifier tokens were scanned before a byte bound, macOS ACL handling admitted unsafe record policy, and configured state bases did not preserve documented lexical normalization. | `6325d9a` restored bounded normalized environment bases and retained early identifier and descriptor ACL checks. |
| `6325d9a` through `8b45806` | Raw environment values lacked a pre-decode byte limit, and macOS deny-read ACL classification needed identity-stable preflight. | `a665289` bounded raw bases before decoding, added identity-checked macOS policy handling, and aligned the durable failure contract. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for the accepted
candidate. All isolated review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33454785472` and Benchmark
evidence `33454785491`, then exact-main CI `33455374424` and Benchmark evidence
`33455374426`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.

## Complete interactive feature: rejected first review candidate

The expanded interactive controls and combined legacy/terminal history were
independently reviewed at `77b6f34814887bdb1b1775c6c1d088ee1c3dea91`, tree
`b5e4207b73e16575013aec44b88464b593c2df83`, against accepted parent
`79d3d4d426aa52816ce6dd70020b7f6a0866da38`. This was not a delivery.

Before review, the exact Rust 1.94.1 local gate passed: macOS workspace
4,644 passed, 18 ignored; Linux workspace 4,650 passed, 17 ignored; explicit
workspace doctests 3 passed on each platform. Counts use top-level harness
summaries, excluding nested child-process summaries. The harnessless terminal
CLI target also exited successfully. Fresh locked release binaries, formatting,
warnings-denied Clippy, 269 repository Python tests (14 expected skips),
documentation policy, pinned compatibility/Unicode drift, dependency policy,
vulnerability audit and the three FreeBSD/WASI compile gates passed.

The Linux gate required correcting two environment-only failures: root bypassed
a permission-denial fixture, then root-owned shared temporary fixtures denied
the corrected unprivileged test user. After isolating those stale fixtures,
the complete replacement run passed with a real unprivileged NSS account,
private home, normal shell profiles and container init. Failed and successful
logs were exported and hash-verified before removing the container. No source
fix, test skip or deadline relaxation concealed either failed invocation.

Three fresh local fallback reviewers inspected clean isolated worktrees of the
same candidate; the host did not expose the named Bugbot service. Their static
reviews ran no builds or tests and produced these actionable findings:

| Track | Agent | Findings |
| --- | --- | --- |
| Correctness/API | `background_review_correctness_01` | P2: a truncated capture can open a partial URL. |
| Lifecycle/platform | `background_review_lifecycle_01` | P2: read-only profile-lock contention can discard exit output; P2: the same truncated-URL defect. |
| Performance/resources | `background_review_resources_01` | P2: the same truncated-URL defect. |

The two distinct defects reject this candidate. At
`background_commands/service.rs`, URL detection discarded the captured window's
truncation evidence: 65,515 spaces followed by
`http://localhost:3000/private` and LF could open the root URL instead.
At `terminal_registry.rs`, an exited backend fell into no-persistence cleanup
when the new read-only inspector's shared profile lock made its transaction
busy, discarding the final drained bytes. The cleanup fallback predates the
feature; routine read-only history inspection introduced the new trigger.
All three completed review worktrees were verified clean and removed.

### First-review remediation

`4ac1b8e1` carries explicit snapshot/truncated boundary evidence into URL
detection, preserving earlier delimited candidates while rejecting an
unterminated trailing one. The composed regression failed on the old behavior,
then passed twelve byte-limit, page-limit and scripted-gap scenarios covering
ordinary/oversized candidates and earlier complete URLs. Thirteen detector
tests and eight nonbrowser launcher tests also passed. Standalone default-feature
native tests additionally needed their composed-service module to use the same
feature gate as its production service; workspace feature unification had hidden
the mismatch. Warnings-denied Clippy rejected the regression adapter's fourth
boolean, so `7ad99073` represents gap injection as an optional read boundary.

The independently implemented `b9ef174e` defers known-exit cleanup on transient
profile `Busy`, retaining the backend and tail until a later bounded pump pass.
Its shared-read-only-lock regression failed before the fix and passed afterward,
proving repeated deferral, durable tail recovery and exactly-once cleanup.
All 64 registry tests passed, including retained permanent-error and shutdown
fallbacks. These are focused remediation results, not replacement feature
acceptance or authorization to merge.
