# Milestone 03 native `vision` review history

This is the compact historical review record for the contract in
[`../vision.md`](../vision.md). Current phase, delivery, workflow, and next-gate
status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh scopes: correctness/API/documentation,
lifecycle/platform/effects, and performance/resources. Any substantive finding
rejected the complete candidate. Repeated agent prose and per-check transcripts
are omitted; the table retains the exact candidates, trees, verdicts, and
deduplicated finding themes.

## Candidate history

| Cycle | Exact candidate | Exact tree | Verdict | Deduplicated finding themes |
| ---: | --- | --- | --- | --- |
| 1 | `258305ef2a9827077fc5cd6573f78a9f78234417` | `ba803cd20cbfb7a2ffe142926ba33788707c5dc2` | rejected | The initial native tool, Gateway worker, reference-host composition, portability boundary, aggregate reads, and durable contracts required correctness, lifecycle, and resource hardening. |
| 2 | `5f8475c90d9b92dbc4f98aed89aeb93e02b90ac9` | `abd91a2fa896f71c300623ce0737fd363d39f048` | rejected | Portable failure taxonomy, Gateway allocation ceilings, filesystem confinement/revalidation, short-read handling, and evidence for those bounds remained incomplete. |
| 3 | `511bf487bfeb679bfb53b7bd11ae4fd1c3ba0626` | `d463e74ffd97a40be5082df1a07414971321180e` | rejected | Pinned lifecycle behavior, empty-output retry, exact trimming, absolute deadlines, permission/execution binding, target/reference scope, failure classification, bounded scans, scratch reuse, and related documentation needed correction. |
| 4 | `b3368c990aa15ded5f32487caa3eddc816a62328` | `22dcd2496676f45cf2f3044eef500c24fcd5c126` | rejected | Unsupported-target compilation, macOS path binding, cancellation during revalidation and publication, resource classification, exact trimming, JSON-node accounting, and feature-scope documentation remained incomplete. |
| 5 | `2f5f153db1a02289b4dc4497a249888bf335e7ed` | `3b9bf67a4682cb9ef1dc7ef354fdf1fab63caa07` | rejected | Final-teardown cancellation, empty-chunk progress, input cloning, IPv4 alias handling, focus trimming, and JSON-fence trimming needed hardening. |
| 6 | `a17e25152c5e9fa96e2cb329a6d150ac03ae7c19` | `2106e237f9a75e114bd6721b9c01b0c0e4a1362f` | rejected | Deep JSON ownership, permit/waiter teardown, cancellation after error teardown, text-delta identity, and usage-total validation were incomplete. |
| 7 | `86c50d68ba848b9ce7c7ad03adeb2570afe809a7` | `4981ccb38519398a5a7e223c0f6fb8c2bfd4191b` | rejected | The absolute deadline was not rechecked after a timeout future error, and tool/catalog plus core/security authority documentation was stale. |
| 8 | `8c82c9d3ae6fc480c975bdcf229bcf6a7f67ffbd` | `5cced76ccc7ee81c257ad0da93e406aa1607f98f` | rejected | Same-poll capacity acquisition could release before waiter teardown; public Gateway same-poll startup/stream errors needed cancellation precedence; result confidentiality wording overclaimed provider evidence. |
| 9 | `39e612d738af996230b1ebb1152c43762ba52939` | `ef293e8323f0e363ba5136b885ea84433059096f` | rejected | A Tokio reserved permit could survive without a repoll teardown path, and stream-drop cancellation needed a final check after the selected error was dismantled. |
| 10 | `888a377d9db935b29943013dc571a9f821ab9003` | `7a3cabd997c39013a40eab9899f8b12219145065` | rejected | Semantic retry could start after the deadline; AI Gateway evidence-persistence and partial-response documentation was inaccurate; usage breakdowns larger than totals were accepted. |
| 11 | `9479986562d0ed303a5d52a4b70241c3692ab75e` | `868078630f3f6c98e26187e7722432b18859e760` | rejected | Request encoding and immediately-ready response streams could consume unbounded work in one outer poll, and the documented forbidden path classes omitted control/bidirectional characters. |
| 12 | `be51be375e99846bfe03b2954bb82cccccbe35ee` | `8baa243b420384e2663bf82f03157da9b8417ee1` | rejected | Pending Gateway startup lacked an owned cancellation waiter, and unsupported-native compilation emitted warnings from supported-only vision internals. |
| 13 | `daa09ec2a6357fc269d6fbc8eb06fc8f2acf4308` | `071da62390e28c497442dba1a636ea609e85c683` | rejected | `vision_engine` integration coverage was gated on generic Unix and would execute supported-only assumptions on FreeBSD. |
| 14 | `af132783a845b5b36839dfbef968ad3e1a732970` | `82563dcff25d0f5c9219ec00f6897f5c9517871d` | rejected | The promised unsupported-native API/test was cfg-elided by no-feature FreeBSD checks, while the full HTTP feature reached AWS-LC before machine-god; no effective repository gate compiled the stub. |
| 15 | `24b9daa41bdbe3f96998e3019d94b78b404b7163` | `f94619bffafa5127f0321c3d1ec5cfb28f48ea0e` | product green; documentation seal | All three tracks reported zero findings. The narrow `vision` feature and exact FreeBSD library/integration-test Clippy gate compile the unsupported surface without HTTP, TLS, DNS, or AWS-LC dependencies. |

## Final behavior disposition

Cycle 15's exact behavior candidate passed the complete local gate with exact
Rust and Cargo 1.94.1. The gate included formatting, warnings-denied workspace
Clippy, default and all-feature workspace tests, documentation tests, repository
Python/documentation policy, pinned fx compatibility drift, exact dependency
policy and vulnerability audit, FreeBSD and WASI compilation, release-binary
`help`/`doctor`/`status` smoke, a clean diff, and zero added unsafe Rust.

The final reviewers confirmed:

- strict source/input normalization, exact composite authority, deterministic
  structured results, and reference-host integration;
- descriptor-confined Linux/macOS reads, identity revalidation, fixed
  unsupported-native construction, and platform-accurate test cfgs;
- cancellation/deadline precedence after waiter, permit, stream, worker, and
  result teardown, with no detached task or thread;
- bounded reads, batches, Base64 encoding, stream progress, JSON/evidence work,
  allocation, retries, output, and active executions; and
- a `vision`-only feature graph containing the documented Base64 and Tokio
  edges but none of the production HTTP/TLS/DNS stack.

This documentation-only seal changes no product behavior and therefore does
not start another adversarial product-review cycle. It still receives the
repository's proportionate exact-commit checks. No review or gate above makes a
measured performance, full-fx-equivalence, live-provider, or current-upstream
compatibility claim.
