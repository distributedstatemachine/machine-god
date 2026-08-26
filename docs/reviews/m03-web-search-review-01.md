# Milestone 03 native `web_search` review 01

Status: **IN PROGRESS — CYCLE 1 KICKOFF; REVIEW PENDING**

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
passes version/help smoke. The exact kickoff candidate must repeat the four
required Rust commands before its formal verdict is collected.

The isolated documentation component passed `git diff --check`, a repository-
relative Markdown target check over every changed page, exact-1.94.1
`cargo fmt --all -- --check`, and exact-1.94.1 `cargo test --doc --workspace`
with two doctests passing. Those checks validate only this documentation
component; they are not a composed behavior gate, review result, integration,
or delivery claim.

## Formal review cycles

Formal cycle 1 is opening on the exact kickoff commit. After that immutable SHA
repeats the four required Rust commands, three fresh isolated read-only tracks
must review the same candidate:

1. correctness, API, strict schema, result semantics, and compatibility;
2. native network, authorization, cancellation/drop, redaction, and platform
   boundaries; and
3. performance, concurrency, allocation, resource ceilings, and dependency/
   target topology.

Any finding rejects that exact candidate. Confirmed findings are remediated and
the complete gate plus three fresh reviews repeat until the deduplicated union
is `0/0/0/0` in blocker/high/medium/low order.

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
