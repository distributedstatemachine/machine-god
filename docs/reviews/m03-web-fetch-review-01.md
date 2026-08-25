# Milestone 03 native `web_fetch` review 01

Status: **IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `a56ff350c2aace1dc22cb14c269aee89d399cd8e`.
- Integration branch: `agent/m03-web-fetch`.
- Normative contract: [`web-fetch.md`](../web-fetch.md).
- Pinned comparison reference: [`vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`](https://github.com/vercel-labs/fx/commit/b1774fbf6c7602b503026f96f6e960e946c692ef).

This is the review plan for the proposed twenty-seventh bounded slice. It does
not establish implementation, test, local-gate, review, workflow, integration,
delivery, compatibility, or performance evidence. Milestone 03 remains in
progress with twenty-six delivered slices.

## Frozen candidate boundary

The candidate is a cfg-gated, non-WASM, rootless `WebFetchTool` behind
`web-fetch-http`; `ai-gateway-http` includes that feature. It accepts only a
sole `{url:string}`, trims its boundary, bounds the canonical ASCII URL at
2,000 bytes, upgrades `http` to canonical `https`, rejects credentials, strips
fragments, and admits only a public multi-label DNS name or strict public IP
literal. Effect-free preflight must produce exact `Capability::Network`
policy/execution agreement. Existing core policy presents network authority as
`Critical` and the default path stays `Ask`.

Allowed execution performs one fixed-header, no-auth Reqwest HTTP/1 GET with no
proxy, retry, referer, cookie, automatic redirect, or decompression behavior.
Every one of at most 32 DNS answers must be public and the accepted set must be
pinned to the connection. Defaults are eight active calls, a 10-second connect
bound, a 60-second total bound, an inclusive 24 KiB body bound, and a 56 KiB
serialized-result bound; active calls can never exceed 32. Cancellation and
drop release the response and permit and own no machine-god worker.

Only 2xx and identity encoding succeed. Text, JSON, XML, JavaScript, bounded
raw HTML, missing-MIME sniffing, model-unsafe-text rejection, and metadata-only
binary results follow the normative contract. Every result starts with the
upstream-untrusted warning and includes query-redacted URL, status, MIME,
content kind, and `cache_hit: false`. Errors are fixed and redacted.

The candidate host will have thirteen alphabetical tools, but only its twelve
workspace tools use the original retained descriptor plus eleven clones.
`web_fetch` is rootless. There is no CLI change.

## Explicit deferrals and deviations

This slice adds no cache, binary artifact store, `read_tool_result` integration,
progress/completion side channel, HTML-to-Markdown conversion, compression,
redirect following, private/authenticated target, CLI change, benchmark
workload, product-performance claim, compatibility-inventory promotion, or
fx-equivalence claim.

Pinned upstream same-site/optional-`www` redirects and default-safe admission
are deliberately not copied. One `NetworkTarget` and one `Ask` decision cannot
authorize another host; every network call remains `Critical` and requires the
normal permission path.

## Independent ownership

Production and test ownership must remain non-overlapping and compose only on
the review candidate:

| Component | Required owner and evidence |
| --- | --- |
| Production | Core capability agreement, native URL/DNS/HTTP/result implementation, cfg gates, and rootless host registration. |
| Independent evidence | Public API, direct/private/engine/host tests; hostile URLs and DNS answers; HTTP wire behavior; byte/time/concurrency/cancellation/drop bounds; output and redaction. |
| Documentation | Normative contract, maintained architecture/API/host/security summaries, implementation plan, and this ledger. |

No owner may weaken a frozen bound to make its own tests pass. The coordinator
must verify the composed diff contains no unauthorized CLI, workflow,
benchmark, generated compatibility, or inventory change.

## Composed implementation and pending local gate

The non-overlapping components now compose locally:

- production commit `8b2a66993989fcdc67ab7d42f9d3e6a6858a9cfe`
  supplies cfg/public exports and bounded URL/DNS/HTTP/result behavior;
- independent commit `9825a890cf1a21a8585b29aff537dde611508517`
  supplies core serde plus direct and engine evidence;
- independent commit `a09dbc7915a16478ae5a4a70aa177ea718539b49`
  supplies deterministic production-construction, runtime, cancellation, and
  redaction evidence; and
- the integration branch wires the rootless tool into the exact thirteen-tool
  host while retaining twelve descriptor-backed workspace tools.

Focused exact Rust 1.94.1 evidence is green: 11 private, 13 direct, five
engine, three production-boundary, seven host, and 65 core-contract tests, plus
warnings-denied all-target/all-feature native Clippy. One independent-test
compile shadow and its warnings-denied reference signature were corrected
without weakening an assertion.

The following are still pending:

- exact Rust 1.94.1 formatting, warnings-denied Clippy, workspace tests, and
  doctests as one complete immutable-candidate gate;
- dependency, target, documentation, diff, and release-binary checks; and
- one immutable, exact-SHA candidate for formal review.

The local gate must include the repository-required commands:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Any allowed exact-1.94.1 stable fallback must be recorded. A newer floating
stable is not evidence for this gate. User-visible behavior must also be
exercised through a freshly built `target/release/machine-god`; no CLI change is
expected, so that exercise must prove existing host behavior remains intact.

## Formal review protocol

After the complete local gate, three fresh agents must independently review the
same immutable candidate SHA and tree:

1. **Correctness/API** — schema strictness, URL canonicalization, exact
   capability/prepared-input agreement, cfg/public exports, MIME/result shape,
   bounds, error taxonomy, and maintained-doc agreement.
2. **Network/HTTP lifecycle and robustness** — public-target/DNS admission,
   answer-set pinning, TLS hostname preservation, fixed headers, redirect and
   encoding rejection, no proxy/retry/credential leakage, body streaming,
   cancellation/drop, redaction, and no machine-god worker lifetime.
3. **Performance/concurrency** — eight-default/32-hard admission, permit
   lifetime and saturation, 10-second connect and 60-second total deadlines,
   24 KiB inclusive body handling, 56 KiB serialized results, bounded DNS and
   sniff work, allocation amplification, repeated race evidence, and prompt
   cleanup.

Each report must list blocker, high, medium, and low findings explicitly. Any
finding at any severity makes that cycle **NOT GREEN** and rejects the SHA.
After remediation, rerun the complete replacement local gate and assign three
fresh tracks to the replacement SHA. Repeat until all three report zero
findings. Review prose is evidence only when it names the exact reviewed SHA;
results from different candidates cannot be combined.

## Remote delivery protocol

Only a review-green exact candidate may run feature CI and required evidence
workflows. Their exact SHA and every required job must be green before the
branch is eligible for a non-force fast-forward to `main`. Exact `main` CI and
required evidence workflows must then pass for the integrated SHA. Record all
SHAs, trees, run IDs, attempts, jobs, and retained artifacts here without
turning regression or size evidence into a product-performance claim.

Current state: production, independent evidence, and host composition are
present and focused-green. The complete local gate, all three formal reviews,
feature workflows, fast-forward integration, exact `main` workflows, and
delivery are pending.
