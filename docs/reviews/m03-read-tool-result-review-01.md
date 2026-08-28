# Milestone 03 native `read_tool_result` review history

This is the compact historical review record for the contract in
[`../read-tool-result.md`](../read-tool-result.md) and its conditional Gateway
projection in [`../ai-gateway.md`](../ai-gateway.md). Current delivery status is
maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh scopes: correctness/API/documentation,
lifecycle/platform/effects, and performance/resources. Any substantive finding
rejected the complete candidate. Counts and repeated agent prose are omitted
where the finding themes below preserve the actionable evidence more compactly.

## Candidate history

| Cycle | Exact candidate | Exact tree | Verdict | Deduplicated finding themes |
| ---: | --- | --- | --- | --- |
| 1 | `b538f3e43597d931a857f2c4ed91e9a11e86fccb` | `cb8bb7a3ff6fb94b04b0eccc0bc86d7ddceedccf` | rejected | Reader results could project recursively; pre-limit serialization/traversal and scan ordering were not fully bounded; reset/cancellation and allocation evidence were incomplete. |
| 2 | `850806208c29af347be729f281567842e69d6559` | `dbe4c14acc9691936b83e93f1392bb5a42547f5d` | rejected | Current-round placeholders consumed prior-result limits, deep injected records were not guarded before every boundary, and result scanning still used unbounded serde serialization. |
| 3 | `7c03c9b5decdb624eff7c9f3b0fdc04599a81dd6` | `9394e0acd427798db86d2b6e6c2c587b7dca049f` | rejected | Record/argument prepasses, reserve growth, number encoding, per-candidate source work, and recursive argument ownership retained resource defects. |
| 4 | `d46ce1730fca2789cbe7f5aa05cbcb11f8edf61b` | `01c16ab9521c7f6248713daf793bc74ac3cc1aba` | rejected | Projection advertised an unserviceable handle for a result above the reader's 64 KiB source ceiling. |
| 5 | `f0ebf8d636e69bd486f4f5831fdbe56cdfb6c164` | `ad57b7d98b037d42171b172ec20c9f3a9545cdcb` | rejected | Cancellation could arrive during final owned teardown after the last check; prepass, serializer, and record destruction allocated per high-cardinality root or candidate. |
| 6 | `62718922b0ece4e280c5d9320ed8a0fd07974509` | `d72c5bba79aae1f19f267b5a2d12fd2c5ea98e49` | rejected | Gateway construction still allocated traversal scratch and 2 KiB minimum capacity per small historical result; evergreen catalog/index documentation omitted the reader. |
| 7 | `fddd0602724534356bf79d02a100123fca4204a7` | `29cab5211205727fb6454c46c7a1593f2bd50b16` | rejected | Validation and rejection teardown still allocated per JSON root; empty assistant messages crossed the provider boundary; terminal byte-stream destruction could make cancellation ready after the last check. |
| 8 | `f535cb4c44b86ea469945da72e3e2e8794b4fde5` | `c0f4a4f78aeeaf46a767c7e87fb4daaa39cfb1cd` | product green; documentation seal | Lifecycle and performance reported `0/0/0/0`. Correctness found no code/API defect and one low stale “in-progress” label for already delivered `web_search`; this seal removes that live-status wording. |

## Final behavior disposition

Cycle 8's exact behavior candidate passed the complete local gate with exact
Rust and Cargo 1.94.1. The gate included formatting, warnings-denied workspace
Clippy, workspace and documentation tests, 141 repository Python tests, pinned
fx compatibility drift, dependency policy and vulnerability audit, FreeBSD and
WASI compilation/active behavior, release-binary smoke, clean diff, and zero
added unsafe Rust.

The final reviewers confirmed:

- strict arguments, opaque scope-bound handles, inclusive UTF-8 paging, fixed
  error collapse, and current-round exclusion;
- conditional projection only through the reader's serviceable 64 KiB ceiling,
  no recursive reader-page projection, and independent source/wire/body budgets;
- inert execution, explicit store authority, fail-fast capacity, and
  cancellation after load, record, permit, argument, and terminal transport
  teardown;
- iterative bounded validation and destruction with scratch allocation scaling
  by maximum depth rather than root count; and
- end-to-end maximum-cardinality Gateway preparation without per-root traversal
  allocation or per-result 2 KiB over-reservation.

The only cycle-8 finding changed no product behavior and is corrected in this
documentation-only seal. Under the repository's documentation-maintenance
exemption, it does not require another adversarial product-review cycle; it
still requires exact-commit checks. No review or gate above makes a measured
performance, full-fx-equivalence, or current-Gateway-compatibility claim.
