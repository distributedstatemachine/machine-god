# Milestone 03 native `ask_user_question` review history

This is the compact historical review record for the contract in
[`../ask-user-question.md`](../ask-user-question.md). Current delivery status is
maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).
Git history retains the verbose per-agent reports, component lineage, local
gate transcripts, workflow IDs, and delivery records that this file replaces.

Counts are ordered blocker/high/medium/low. Any nonzero deduplicated union
rejected a candidate and required remediation plus three fresh review tracks.

## Candidate history

| Cycle | Exact candidate | Exact tree | Correctness | Lifecycle | Performance | Union | Verdict |
| ---: | --- | --- | ---: | ---: | ---: | ---: | --- |
| 1 | `6c54ec3bf2c23983f14b0a4edeac723321a97900` | `bea90245a559e8e223cc5bb45e0ddfa15e426ee6` | `0/0/2/2` | `0/1/0/1` | `0/0/2/0` | `0/1/3/3` | rejected |
| 2 | `910d7bc84cfd7800fb4daf9ab8537bf269027896` | `503a91f334156dbcf2470560b9bb456c3491fd3d` | `0/0/1/0` | `0/0/0/2` | `0/0/2/0` | `0/0/2/2` | rejected |
| 3 | `746e510c7d8eb93229996e74f91827f489e5bb31` | `c49221efbea66c840b333f0de0161aa686aad52f` | `0/0/1/2` | `0/0/0/2` | `0/0/2/0` | `0/0/3/2` | rejected |
| 4 | `42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb` | `b761f7b93d535a1580910f43ff509c40aa07415b` | `0/0/0/1` | `0/0/1/1` | `0/0/1/0` | `0/0/2/1` | rejected |
| 5 | `54b1aab5660e90096b95518bde4ebffb93f28fa6` | `54586d2256c8a3d2289b92bc9bc842eed9ce4d07` | `0/0/0/0` | `0/0/0/0` | `0/0/1/0` | `0/0/1/0` | rejected |
| 6 | `85058a8aa88fab6912d9313f1ce71e2778cc937f` | `fd3c5072c9473c7fe8767cc2692238eacb8a0f43` | `0/0/0/1` | `0/0/1/0` | `0/0/1/0` | `0/0/1/1` | rejected |
| 7 | `617672984fbb897f2efec63de6a05bb32db9a3db` | `f2cd844449193b46cfa1473ae21edad68664157e` | `0/0/0/0` | `0/0/1/0` | `0/0/0/0` | `0/0/1/0` | rejected |
| 8 | `e929b5ea7e3264c2b56066a416bc2a979a03b214` | `cfadc42814688a29c4d512e5fd91c843423821d4` | `0/0/0/0` | `0/0/1/0` | `0/0/1/0` | `0/0/2/0` | rejected |
| 9 | `1eeab670a552bc15b5602319b0bb1ce27d2be497` | `5c86e624cf3c0e6d521382c377a9ed9b0500ee5b` | `0/0/1/0` | `0/0/1/0` | `0/0/0/0` | `0/0/1/0` | rejected |
| 10 | `4ea1c1f5be3586ce9bee696b12c4120dc2a72018` | `78e781ffd7b03aafdf295ae79f4090120971c248` | `0/0/1/0` | `0/0/1/0` | `0/0/0/0` | `0/0/2/0` | rejected |
| 11 | `b1d454ba21d2a380a4198bb1253c4cb1bc34d4a6` | `26d90d8ec3924f6b7e12617506d5275ae32ec00b` | `0/0/0/1` | `0/0/1/0` | `0/0/1/0` | `0/0/2/1` | rejected |
| 12 | `3dec7a2f073fa85479af19765b03b06cdfd9da8c` | `c34d20a45f70b82652bf78df9653f39399d7fc6d` | `0/0/1/1` | `0/0/1/2` | `0/0/1/0` | `0/0/1/2` | rejected |
| 13 | `a4f1bb91c00064e0ceb6975e1c9e7b4a09b1ff95` | `72a0303d81f34586f93775050ad70257cc2da551` | `0/0/0/0` | `0/0/0/0` | `0/0/0/0` | `0/0/0/0` | **GREEN** |

Overlapping reports were counted once at their highest confirmed severity. In
particular, cycle 1's evidence finding, cycle 2's result-bound finding, cycle
3's lifecycle/resource findings, cycle 6's replay finding, cycle 9's liveness
finding, and cycle 12's release-evidence finding were cross-track duplicates.

## Rejected findings and dispositions

| Cycle | Deduplicated finding themes | Remediation disposition |
| ---: | --- | --- |
| 1 | Adjacent cancellation gap; direct execution widened prepared raw limits; oversized strings/keys were fully scanned before the remaining budget; incomplete boundary evidence; overstated fx answer parity; incidental map-key ordering; stale reference-host signatures/catalog. | Added the adjacent pre-invocation check, canonical-preimage validation, remaining-budget serialization, complete boundary regressions, intentional answer-then-question insertion, scoped parity wording, and the sixteen-tool host contract. |
| 2 | Host answers were trimmed before their 4 KiB bound; the 49,152-byte result guard was falsely treated as reachable; authority and lineage wording was stale. | Bounded complete answers before trim/scan, proved the reachable 41,102-byte maximum, retained 49,152 only as defense in depth, and scoped no-authority to policy authority. |
| 3 | Public capability inspection could panic; malformed hosts could return an unbounded answer vector; capacity could release before cancellation-Waker teardown; broad authority and architecture wording was stale. | Made capability inspection optional and total, introduced a fixed four-slot answer container, and kept prompt/waiter/Waker teardown under activity ownership before capacity release. |
| 4 | Final registered-Waker destruction could expose cancellation after the last check; a concurrently moved callback could outlive permit release; host lineage was stale. | Added the post-teardown cancellation check and activity-backed callback/Waker ownership through callback return. |
| 5 | Retained Waker clones could independently run concurrent blocking callbacks while consuming one prompt slot. | Introduced one activity-backed single-flight coalescing notifier with stale-target close, lossless replay, and retained activity ownership. |
| 6 | A self-notifying replay could synchronously amplify one wake without bound; operative summaries were stale. | Made replay observation-aware, bounded pre-observation reentry, preserved a later observed notice, and synchronized historical summaries. |
| 7 | Replay target B was selected before old target A dropped, so A's panic could wedge the lane and A's reentrant close could permit stale B delivery. | Dropped A outside the lock before replay arbitration, settled every unwind path, and let the then-current lifecycle win. |
| 8 | A secondary panic payload could replace the promised callback primary; re-poll/re-notify could execute 257 callbacks in one activation. | Intentionally forgot suppressed opaque payloads and bounded an activation to an initial callback plus one replay while retaining pending work. |
| 9 | Residual pending work needed an unrelated later notify, so a final self-wake or cancellation wake could remain pending forever. | Added autonomous, serialized, nonrecursive wake progress and a terminal delivery-exhaustion path. |
| 10 | Queue-only executors reset the per-notify budget and bypassed the 256 limit; cleanup payload destruction could replace a selected panic or abort. | Moved the 256 budget into prompt-lifetime state and centralized panic selection with explicit forgetting of every nonselected opaque payload. |
| 11 | Prompt failure kind/code prose was wrong; release used panic abort despite unwind-based settlement; a forgotten payload owning a Waker could permanently retain capacity. | Corrected `Execution` versus error code, changed release panic handling to unwind, and detached capacity from closed Waker identity using state and local callback/close guards. |
| 12 | The release probe never entered a cleanup-panic conflict; clone-capacity prose described the superseded design; global release-profile prose still said abort. | Replaced the probe with ordinary and ambient in-close target-drop cases using the supplied Waker, corrected capacity ownership, and aligned release-unwind wording. |

The cycle 7 review noted an analogous preexisting `terminal` notifier ordering;
it remained outside the bounded `ask_user_question` review. No row above is a
claim about that separate tool.

## Final green disposition

Cycle 13 reviewed exact immutable candidate
`a4f1bb91c00064e0ceb6975e1c9e7b4a09b1ff95`, tree
`72a0303d81f34586f93775050ad70257cc2da551`. Correctness/API/evidence,
lifecycle/platform/concurrency, and performance/resources each reported
`0/0/0/0`; the deduplicated union was `0/0/0/0`.

The three reviewers validated:

- final-target destruction occurs inside notifier close;
- ordinary prompt-`Drop` and ambient primaries retain precedence;
- the secondary panic payload owns the supplied Waker, its destructor-control
  case panics, and product cleanup suppresses and intentionally forgets it;
- stale wakes remain zero and fresh capacity is available while closed Waker
  identities are still retained;
- the 256-callback limit applies to the complete prompt lifetime, including
  queue-only delivery; and
- schema, byte, authority, platform, cancellation, redaction, and other
  resource boundaries remain intact under the normative contract.

All findings from cycles 1 through 12 are resolved in that reviewed tree. This
history makes no product-performance, compatibility, or fx-equivalence claim.
