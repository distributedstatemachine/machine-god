# Milestone 03 native `web_search` review 01

Status: **IN PROGRESS — CYCLE 4 REMEDIATION COMPOSED; REPLACEMENT GATE PENDING**

## Base and boundary

- Exact delivered base:
  `4ba9f5afde89b9666fe9929bb81fbabcaa834334`.
- Integration branch: `agent/m03-native-web-search`.
- Production branch/worktree: `agent/m03-native-web-search-prod`.
- Independent-evidence branch/worktree: `agent/m03-native-web-search-tests`.
- Documentation branch/worktree: `agent/m03-native-web-search-docs`.
- Normative contract: [`web-search.md`](../web-search.md).
- Pinned comparison reference: [`vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`](https://github.com/vercel-labs/fx/commit/b1774fbf6c7602b503026f96f6e960e946c692ef).

This is the live ledger for bounded slice 33. The contract is frozen and the
production, independent-evidence, and maintained-documentation components were
developed with non-overlapping ownership. Exact production component
`8ca0bb67166e16383b1a3c956c6140cf1d1eecba`, independent-evidence component
`beb0e2a88ca155ca0a0a692507eaeb77a5ca2f76`, and documentation component
`a8884aea0040ab42623c55972056136b68de5676` compose through exact behavior
precursor `3d2984000301e58762e0940504159aeb55b2389e`, tree
`5222c3e009e9fe440097a86fd46889d1bb2e1434`.

The exact formal cycle-1 candidate is the commit containing this kickoff
paragraph. Reviewers receive its full SHA and tree out of band, and the later
result seal records both values without claiming that the review-exempt seal
itself was reviewed. No formal verdict, feature workflow, integration, `main`
workflow, or delivery result exists at this checkpoint. The delivered count
therefore remains thirty-two and Milestone 03 remains in progress.

## Frozen candidate boundary

The candidate adds a locally executed `WebSearchTool` with required `query`
and optional mutually exclusive `allowed_domains` / `blocked_domains`. Its
effect-free preparation normalizes the complete input and supplies the exact
configured AI Gateway `NetworkTarget` plus canonical execution arguments to
core's existing critical-risk permission path.

One allowed execution issues at most one required Perplexity provider-tool
worker request over the shared injected AI Gateway transport. A dedicated
bounded native decoder—not the ordinary outer `AiGatewayProvider` decoder—
admits exactly one `providerExecuted: true` `perplexity_search` call, one
matching final result, and one successful finish. The ordinary provider codec
retains its rejection of response-side provider execution and tool results.

The fixed public limits are 4,096 query bytes, 16 normalized DNS filters and
4,096 aggregate filter bytes, ten sources, 512 title bytes, 2,048 URL bytes,
16 KiB request, 256 KiB SSE stream, 64 KiB SSE record, 256 records, 16,384 JSON
nodes, 48 KiB serialized output, a 30-second total deadline, default concurrency
four, and hard concurrency sixteen. One execution has no retry, fallback,
second backend, multiple provider use, progress, inner usage/billing, caching,
artifact, page-fetch, or detached-work behavior.

Production is non-WebAssembly behind `ai-gateway-http`; current reference-host
composition remains Linux/macOS-only. The candidate host will have fourteen
alphabetical tools: twelve descriptor-backed workspace tools plus rootless
`web_fetch` and Gateway-backed `web_search`. There is no CLI, core provider-
event, configuration-schema, benchmark-workload, generated compatibility,
product-performance, or fx-equivalence change.

## Required evidence before review

The composed exact SHA must first pass focused tests for strict input and DNS
normalization, exact permission/execution agreement, request projection,
provider-call/result identity and ordering, fragmented SSE, bounds at and over
every ceiling, redaction, timeout/cancellation precedence, drop, permit release,
outer-provider rejection preservation, host catalog ordering, and target/
feature topology. Tests use injected transports only; no live provider call is
required or claimed.

The same immutable candidate must then pass:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

On exact behavior precursor `3d2984000301e58762e0940504159aeb55b2389e`, the
focused `web_search`, engine, dedicated-codec, and reference-host selection ran
25 tests with zero failures. The four required commands above passed under
exact Rust and Cargo 1.94.1 without fallback. The all-feature workspace run,
including the feature-gated web-search tests, also passed. Repository-wide
Python ran 135 tests with eight expected platform skips and zero failures.

Pinned-fx regeneration against exact `b1774fbf6c7602b503026f96f6e960e946c692ef`
was byte-stable. FreeBSD no-default and WASI all-feature native-library checks
passed with only the established unrelated WASI `read_file` dead-code warning.
All 87 maintained Markdown files had zero missing repository-relative targets;
the base diff and metadata checks are clean, Cargo manifests and `Cargo.lock`
are unchanged, and no unsafe Rust construct was added. A freshly built locked
release CLI is 3,985,216 bytes with SHA-256
`bd7c89147f458001cc927c19b76ac75cb09ed5dca5396efc34d3933bde62a3dc` and
passes version/help smoke. Exact kickoff candidate
`89c5ec95fb5353efcba34af6a44bc27d7b6027f7`, tree
`8d91a556f786169d42406e91e8ad2f476b7c6cf4`, repeated all four required Rust
commands successfully before its formal verdict was collected.

The isolated documentation component passed `git diff --check`, a repository-
relative Markdown target check over every changed page, exact-1.94.1
`cargo fmt --all -- --check`, and exact-1.94.1 `cargo test --doc --workspace`
with two doctests passing. Those checks validate only this documentation
component; they are not a composed behavior gate, review result, integration,
or delivery claim.

## Formal review cycles

Formal cycle 1 reviewed exact candidate
`89c5ec95fb5353efcba34af6a44bc27d7b6027f7`, tree
`8d91a556f786169d42406e91e8ad2f476b7c6cf4`, in three fresh isolated read-only
tracks:

1. correctness, API, strict schema, result semantics, and compatibility;
2. native network, authorization, cancellation/drop, redaction, and platform
   boundaries; and
3. performance, concurrency, allocation, resource ceilings, and dependency/
   target topology.

Correctness/API reported `0/1/3/2`, native network/effects/lifecycle reported
`0/2/3/1`, and performance/concurrency/resources reported `0/0/3/0`. The
deduplicated union is `0/2/5/2` in blocker/high/medium/low order, so cycle 1 is
rejected.

The two high findings are shared-HTTP-capacity starvation when an outer
tool-calling stream retains its permit during nested search, and authorization
of the fixed Vercel target for an opaque custom transport that may contact a
different endpoint. The five medium themes are the public unbounded
constructor, misplaced native/HTTP feature gates around the target-neutral
seam, the driverless-Tokio abort precondition, payload-proportional decoder
copies plus post-cap dedup growth, and missing exact request/resource/
concurrency/topology evidence. The two low themes are provider source data in
`WebSearchSource` debug output and stale custom-host catalog documentation.

Every finding is confirmed. Remediation must bind target to transport,
strictly validate that target, release the outer source before tool execution,
make every public tool constructor bounded, split target-neutral contracts from
native HTTP/Tokio machinery, reject driverless runtimes without panic, frame
SSE incrementally with bounded retained state, stop post-cap dedup growth,
redact source debug output, update maintained docs, and close the exact evidence
matrix. The complete gate plus three fresh reviews repeat until the deduplicated
union is `0/0/0/0`.

Exact isolated lifecycle remediation component
`096b11c4ffed7b6b4e4419940d1cd50ef40b197f` is composed in the current
remediation. It
drops the validated outer Gateway source before queued terminal delivery and
threads explicit `NetworkTarget` / `WebSearchDeadline` authorities through all
host paths. Exact isolated portability/bounds remediation component
`ca0b990a07c7e6659c10e91f86a2aa66ed4d3182` is also composed. It
makes portable values and traits available without HTTP/WASM gates, makes both
public concrete-tool constructors bounded, strictly validates canonical
targets, replaces hidden Tokio timing with a fallible injected deadline,
incrementally frames one bounded SSE record, caps retained deduplication state,
and redacts source debug output.

Three advertised defense-in-depth ceilings cannot be reached under stricter
public limits: 16 domains of at most 253 bytes total at most 4,048 bytes; the
4,096-byte query plus at most 4,048 filter bytes and fixed worker projection
remain below 16 KiB; and ten sources of 512-byte title plus 2,048-byte URL, a
4,096-byte query, and fixed JSON overhead remain below 48 KiB. Replacement
evidence must exercise exact/+1 for every reachable boundary and retain an
explicit proof for these unreachable aggregate/request/output caps rather than
claiming impossible fixtures.

Exact composed remediation precursor
`e662fa8047c5ca321d622b9b5920166804a35c27`, tree
`6c0ace98ea9931af9d16cc9fb2ade969df477d3c`, passes all replacement evidence
under exact Rust and Cargo 1.94.1 without fallback. The focused remediation
matrix ran 88 Rust tests with zero failures. The four required commands passed;
default-feature and all-feature workspace tests passed; repository-wide Python
ran 136 tests with eight expected platform skips; FreeBSD no-default and WASI
all-feature checks passed with only the established unrelated WASI `read_file`
warning; and pinned-fx regeneration against exact `b1774fbf` was byte-stable.
All 87 maintained Markdown files had zero missing repository-relative targets.
Cargo manifests and `Cargo.lock` are unchanged, the base diff is clean, and no
unsafe Rust construct was added. The freshly built locked release binary is
3,985,216 bytes with SHA-256
`ef8751d1a50c933b42657d9b1abecb013426abc4bd8c95c89990862371b7c5b9` and
passes exact version/help smoke. Web search has no CLI command, so its
user-visible behavior is exercised through the composed release-equivalent
native host integration rather than a new binary grammar.

The formal cycle-2 candidate is the commit containing this kickoff paragraph.
Reviewers receive its exact SHA and tree out of band; the result seal will
record both values without claiming the later review-exempt documentation-only
seal was reviewed. Correctness/API, native lifecycle/effects, and performance/
resources must each report `0/0/0/0` before feature workflows may begin.

Formal cycle 2 reviewed exact candidate
`399f5f7a14c1473d9e737d44838549ba305746de`, tree
`99a88a45fd6f0823b23fd879633784433194cf8d`, in three fresh isolated read-only
tracks. Correctness/API reported `1/1/0/0`, native lifecycle/effects reported
`0/0/0/1`, and performance/resources reported `0/0/1/0`. The deduplicated
reported union is `1/1/1/1`. Finding adjudication below rejects the reported
blocker and leaves a confirmed `0/1/1/1` union, so cycle 2 remains rejected.

The reported blocker is rejected after primary-source adjudication. This
adapter consumes the raw language-model-v4 transport stream. At current Vercel
AI commit `ce6849a`, its normative
[`LanguageModelV4ToolResult`](https://github.com/vercel/ai/blob/ce6849a1832e2b900bb700a7bcc24f7436eb9e5c/packages/provider/src/language-model/v4/language-model-v4-tool-result.ts)
contains `result`, not `output`. The higher-level SDK
[`stream-language-model-call`](https://github.com/vercel/ai/blob/ce6849a1832e2b900bb700a7bcc24f7436eb9e5c/packages/ai/src/generate-text/stream-language-model-call.ts#L690-L716)
maps raw `chunk.result` to public `fullStream.output`. The reproduction in
[`vercel/ai#12178`](https://github.com/vercel/ai/issues/12178) observes that
higher-level mapped surface and was closed on 2026-08-26 after a server-side
tool-loop change; it is not evidence that the raw field changed. Replacing raw
`result` with SDK-layer `output` would create the protocol mismatch. Replacement
evidence instead preserves raw `result` and explicitly rejects mapped `output`
at this transport seam.

The high finding is that URL-standard numeric IPv4 spellings such as `127.1`,
`0177.0.0.1`, and `0x7f.0.0.1` pass the DNS-name fallback after Rust's canonical
`IpAddr` parser rejects them. Target, domain-filter, and citation validation
must recognize and reject every noncanonical URL IPv4 number spelling. The
medium finding corrects the earlier proof above: JSON escaping can expand
otherwise bounded query, title, and URL fields enough to make the 48 KiB
serialized-output ceiling reachable. Remediation must add exact and one-byte-
over output evidence and correct the contract explanation. The low finding is
the stale pending-composition label in the maintained reference-host page.

The high, medium, and low findings are confirmed. Cycle 2 remains historical
rejected evidence;
after remediation and a complete exact gate, three fresh review tracks must
evaluate one new immutable candidate and produce a `0/0/0/0` union.

Exact isolated protocol-evidence component
`3ad8ec7c256e5fffaad22ef55c61d508d960839b` preserves the production raw
`result` decoder byte-for-byte and adds the explicit raw-result/SDK-output layer
regression. Exact isolated validation component
`73441126dac5561dba5ddd39ae4da7de97c8f23b` adds dependency-free URL-Standard
IPv4 classification for targets, filters, and citations plus public execution
evidence at exactly 49,152 and 49,153 serialized output bytes. Both components
and the maintained behavior corrections are composed in the current
remediation. Exact precursor
`366cef966d7dcf1b11101a37d4493099e6f421a7`, tree
`40c05cb2999c641bc7ccbdc369fc6d9251b989b7`, passes the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. The focused decoder and
tool suites ran 28 tests with zero failures. All four required commands passed;
default-feature and all-feature workspace tests passed; repository-wide Python
ran 136 tests with eight expected platform skips; and pinned-fx regeneration
against exact `b1774fbf` was byte-stable.

FreeBSD no-default and WASI all-feature library checks passed with only the
established unrelated WASI `read_file` dead-code warning. All 87 maintained
Markdown files had zero missing repository-relative targets. Cargo manifests
and `Cargo.lock` are unchanged, the base diff is clean, and no unsafe Rust
construct was added. The freshly built locked release binary is 3,985,216 bytes
with SHA-256
`ef8751d1a50c933b42657d9b1abecb013426abc4bd8c95c89990862371b7c5b9` and
passes exact version/help smoke. Both remediation worktrees were verified
committed and clean after composition, then removed and pruned.

The formal cycle-3 candidate is the commit containing this kickoff paragraph.
Reviewers receive its exact SHA and tree out of band. Three fresh isolated
read-only tracks must independently cover correctness/API/protocol,
native effects/lifecycle/platform behavior, and performance/resources. No
feature workflow may begin unless every track and the deduplicated union report
`0/0/0/0`.

Formal cycle 3 reviewed exact candidate
`aef6abed174760195e712b2701e241b656733621`, tree
`5abcef3de31898e158e6c4872ee9b4131863d1b7`. Correctness/API/protocol reported
`1/0/1/0`, native effects/lifecycle/platform reported `0/0/0/2`, and
performance/resources reported `0/0/1/1`. The deduplicated union is `1/0/2/2`,
so cycle 3 is rejected.

The blocker confirms that raw `result` is the correct language-model-v4 field
but the inner local fixture is not a valid v4 Perplexity exchange. Official raw
calls carry stringified JSON `input`; raw results also carry `toolName` and
`isError`; and successful Perplexity results are strict `{id, results}` objects
whose entries require `title`, `url`, and `snippet`, with optional `date` and
`lastUpdated`. That candidate's decoder instead requires object input, ignores
result tool identity/error state, and admits only `{results:[{title,url}]}`. It
also rejects the permitted initial `stream-start` envelope. Remediation must
retain raw `result` and SDK-layer `output` rejection while replacing the
manufactured sequence with a bounded official-shape fixture and strict
projection.

One medium finding shows that a fully processed finish and `[DONE]` record does
not end the transport loop until the byte stream yields EOF. A logically
complete stream that remains pending therefore retains both the tool semaphore
and shared HTTP capacity until timeout. Remediation must stop after the
processed terminal chunk, while continuing to validate every same-chunk record,
and drop the stream immediately. The other medium shows that syntactically
numeric hexadecimal labels above `u64` overflow into `NotIpv4`, allowing URL-
invalid hosts such as `public.0x10000000000000000`; numeric syntax and range
failure must remain distinguishable.

The product low is URI-port grammar: `:+443` passes Rust's integer parser even
though a URL port is digits-only. Citation validation must require nonempty
ASCII digits before parsing. The documentation low is stale cycle-2-pending
status across maintained summary pages. All five deduplicated findings are
confirmed. After remediation, the complete gate and three fresh isolated
product reviews repeat on one new immutable candidate.

Exact isolated protocol/lifecycle remediation component
`5d45dca2f98793304c064941e0d8b0e951d1daa8`, tree
`1385952859ed1fa5697ffe6e67226c9f7b174c5a`, aligns the raw-v4 call/result
exchange with the official Perplexity result schema, validates and discards
nonprojected bounded fields, accepts the optional initial stream envelope, and
drops the inner byte stream as soon as a complete terminal chunk is validated.
Exact isolated URL-validation component
`454f8fdc504bbca9195e03ff2567e33439c8a77b`, tree
`ce0e2086c2440bbfbfed16935b4fc175544e19e9`, distinguishes numeric overflow
from DNS syntax and requires nonempty unsigned ASCII-decimal citation ports in
`1..=65535`. Both components, the cross-component signed-port codec regression,
and maintained behavior documentation are composed in the current remediation.
The combined focused exact-1.94.1 suites run 15 dedicated codec and 20 native
web-search tests with zero failures. The complete replacement gate and a fresh
immutable review candidate remain pending.

The first broader all-feature workspace run correctly rejected one stale
reference-host integration fixture that still emitted the pre-remediation
provider exchange. The fixture now uses stringified call input, matching raw
tool identity, official result ID, and required source snippet. Its focused
capacity-one host test and the complete all-feature rerun pass under exact Rust
1.94.1 before review kickoff.

Exact composed cycle-3 remediation precursor
`b834205494ce8363938aa1e6bf847e576bd6acbb`, tree
`f3557a56613fdf20e25d320437673629cde108cd`, passes the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. The 15-test dedicated
codec, 20-test native tool, two focused cross-layer integration regressions,
and complete default/all-feature workspaces are green. All four required
commands passed. Repository-wide Python ran 136 tests with eight expected
platform skips. FreeBSD no-default and WASI all-feature library checks passed
with only the established unrelated WASI `read_file` dead-code warning. Pinned-
fx compatibility regeneration against exact `b1774fbf` is byte-stable.

All 87 maintained Markdown files expose 661 parsed links and 500 repository-
relative targets with zero missing targets. Cargo manifests and `Cargo.lock`
are unchanged from the delivered base, the base diff is clean, and no unsafe
Rust construct was added. The freshly built locked release binary is 3,985,216
bytes with SHA-256
`ef8751d1a50c933b42657d9b1abecb013426abc4bd8c95c89990862371b7c5b9`; exact
version, help, doctor, and sessions smoke passes without creating the isolated
missing roots. The remediation worktrees were verified clean and integrated,
then removed and pruned; only the primary worktree remains.

The formal cycle-4 candidate is the commit containing this kickoff paragraph.
Reviewers receive its exact SHA and tree out of band. Three fresh isolated
read-only tracks must independently cover correctness/API/protocol, native
effects/lifecycle/platform behavior, and performance/resources. No feature
workflow may begin unless every track and the deduplicated union report
`0/0/0/0`.

Formal cycle 4 reviewed exact candidate
`cc1d3d19ffaec71ca85ff80366199173b0ef5df1`, tree
`ad0c3d353eca7b75bac67db97b9551f0e0f0184e`. Correctness/API/protocol reported
`0/0/1/0`, native effects/lifecycle/platform reported `0/0/0/0`, and
performance/resources reported `0/0/0/1`. The deduplicated union is `0/0/1/1`,
so cycle 4 is rejected.

The medium finding is an incomplete raw-v4 terminal envelope. The decoder
checks only `finishReason.unified == "stop"`, and the positive fixture therefore
omits both required `usage` and the finish reason's raw provider value. At
Vercel AI commit `ce6849a`, the official
[`LanguageModelV4StreamPart`](https://github.com/vercel/ai/blob/ce6849a1832e2b900bb700a7bcc24f7436eb9e5c/packages/provider/src/language-model/v4/language-model-v4-stream-part.ts)
requires usage on every finish, while
[`LanguageModelV4FinishReason`](https://github.com/vercel/ai/blob/ce6849a1832e2b900bb700a7bcc24f7436eb9e5c/packages/provider/src/language-model/v4/language-model-v4-finish-reason.ts)
contains both unified and raw reasons. Remediation must strictly validate and
discard the bounded official finish metadata and replace the manufactured
success fixture, with missing/mistyped regressions.

The low finding is this ledger's stale top-level cycle-3 status despite its own
cycle-4 kickoff evidence. That status is corrected above. After protocol
remediation and a complete exact gate, three fresh isolated product reviews
repeat on one new immutable candidate.

Exact isolated finish-envelope remediation component
`dc79c8d873f51663e05b3d8b7a76527574a2f16a`, tree
`e2fed7084067402ecb43b5664b942acfaefd9913`, strictly validates required raw-v4
`finishReason` and `usage`, optional provider-keyed object metadata, and every
present bounded nonnegative token counter before discarding accounting data.
Its official-shape success fixtures cover full and JSON-wire-minimal envelopes;
missing, mistyped, negative, fractional, overflowed, and unknown members fail
closed. The component's exact-1.94.1 focused codec suite ran 16 tests with zero
failures; format, all-target/all-feature check, and Clippy with `-D warnings`
also passed in its clean isolated worktree. Maintained behavior documentation is
composed with the component in the current remediation. The complete replacement
gate and one new immutable review candidate remain pending.

The first broader all-feature workspace run correctly rejected one remaining
capacity-one reference-host fixture that still emitted the pre-remediation
finish envelope. Exact isolated fixture component
`9f6c4745c789b63987ae42333cda7ab89b0515da`, tree
`e4d2ae78c0d367b6027c3140095b1363949a3a95`, aligns that nested worker finish
with the same official bounded shape. Its focused capacity-one regression passed
1/1, the dedicated codec passed 16/16, web-search engine integration passed 2/2,
and the complete all-feature workspace then passed under exact Rust 1.94.1. The
component worktree was clean after commit. The composed replacement gate restarts
on the new exact remediation commit before review kickoff.

## Worktree lifecycle

Every implementation or review/remediation iteration ends by verifying that
each worktree's changes are committed, integrated where required, and clean.
Only then is that worktree safely removed. Active or uncommitted worktrees are
retained. This invariant applies to production, independent-evidence,
documentation, reviewer, remediation, seal, and integration worktrees.

## Delivery remains pending

Only a review-green exact candidate may proceed to pushed feature workflows,
fast-forward integration without force, and exact `main` workflows. This
ledger must record the exact candidate/tree, review reports and union, feature
CI and benchmark-evidence runs, integrated SHA, main CI and benchmark-evidence
runs, and cleanup result before changing status to delivered.
