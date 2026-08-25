# Milestone 03 `models` CLI review ledger

Status: kickoff ledger for bounded slice 29. No implementation candidate has
been reviewed and no review track is green. The frozen contract is
[`models-cli.md`](../models-cli.md); it starts from exact delivered base
`1de3b7eddf6a4d9046d48098defecf6bfa336442` on branch
`agent/m03-models-cli`. The pinned comparison input is fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`.

This ledger records the review protocol before production code is composed. It
does not claim implementation, test, local-gate, feature-CI, benchmark,
integration, delivery, performance, compatibility, or fx-equivalence status.

## Bounded ownership

The feature must remain split across isolated, non-overlapping components:

| Component | Exclusive responsibility |
| --- | --- |
| core production | validated provider-neutral `AvailableModel`, access, result, error, and object-safe `list_models(CancellationToken)` contracts only |
| native production | fixed Gateway GET provider, process credential, access/fallback, deadline, transport, JSON validation, bounds, duplicate rejection, and ordering |
| CLI production | strict parse-before-effects, config/native composition, current-thread Tokio host, rendering/output cap, and complete output write |
| independent evidence | deterministic core/native doubles, loopback HTTP evidence, release-binary black-box coverage |
| documentation | this contract, plan and maintained summaries; never hand-edit generated compatibility files |

Core may not acquire environment, filesystem, URL, HTTP, runtime, clock,
deadline, credential, Gateway metadata, sorting, fallback, or output-rendering
authority. Native may not move product state into the CLI. CLI may not absorb
catalog parsing, sorting, fallback policy, or engine state.

## Frozen implementation assertions

Every candidate must prove all of these before review:

- grammar is exactly `models [--json]`, with repeated/extra/non-Unicode input
  rejected at parse exit 2 before any other effect;
- config loads exactly once before the two-name credential snapshot and network;
- missing credential makes one public request, while selected malformed,
  oversized, or non-Unicode input fails closed;
- production GET is exactly
  `https://ai-gateway.vercel.sh/coding-agent/v1/models`, with no environment
  override, team query/header, proxy, redirect, referer, title, cookies,
  decompression, or application retry;
- authenticated 401/403 alone causes exactly one fully anonymous fallback,
  under the original deadline, after dropping the first response and permit;
- the total 30-second bound covers the default-8/hard-32 capacity wait and both
  attempts without reset;
- body 256 KiB plus one witness, depth 32, nodes 16,384, raw entries 1,024,
  valid entries 512, ID 1–128 visible ASCII, aggregate IDs 24 KiB, and output
  64 KiB bounds are enforced at inclusive/exceeded boundaries;
- root object plus `data` array is strict, malformed/non-language entries are
  skipped only for the three documented classes, unsafe string IDs are terminal
  malformed responses, duplicate valid IDs reject the whole result, and the
  complete pinned stable ordering is deterministic;
- human and compact JSON output bytes, key order, full-list counts, public flag,
  final LF, closed redacted failure mapping, and exit 0/1/2 behavior match the
  contract;
- cancellation and same-poll precedence wake without peer progress, and drop
  releases all machine-god-owned futures, response bodies, buffers, and permits;
- the command creates no state/workspace/engine/generation provider/prompt/
  permission/session/cache/write effect; and
- production and tests add no unsafe Rust and no secret/error reflection.

## Required deterministic evidence

Focused tests must cover at least:

1. every accepted and rejected grammar form, including non-Unicode argv on
   supported targets and parse precedence over invalid config/credentials;
2. exact global help/usage bytes and human/JSON success/failure bytes;
3. config-once ordering, built-in/missing config, strict invalid config, and no
   state-root or workspace observation;
4. both credential names, empty fallthrough, precedence, missing/public,
   selected invalid fail-closed, authenticated success, 401, 403, and stripping
   all auth on fallback;
5. no fallback for public 401/403 or authenticated 3xx/429/other 4xx/5xx,
   transport, timeout, cancellation, parse, limit, duplicate, or output failure;
6. exact production request method/origin/path/headers and absence of body,
   team, query, proxy, redirect, referer, title, compression, cookie, and retry;
7. numeric IPv4 and IPv6 loopback test endpoints plus every rejected endpoint
   class;
8. each inclusive resource limit and its first rejected value, including a
   lying/absent `Content-Length`, chunked body overflow, depth/node accounting,
   aggregate ID bytes, and final-LF output accounting;
9. root/data structural defects, ignored fields still consuming JSON budget,
   exact three skip classes, terminal unsafe string IDs, metadata defaults,
   duplicate IDs, ASCII-insensitive type/tag/tier matching, case-sensitive
   provider prefixes, all release/ID comparator branches, and stable total
   ordering;
10. capacity contention, same absolute deadline across fallback, cancellation
    at each pending phase, same-poll cancellation/deadline precedence, drop
    before dispatch, drop during body, and permit release; and
11. a freshly built release binary for parser/config/error/help and other
    environment-independent black-box cases, plus native numeric-loopback
    success evidence proving production has no endpoint-injection or process-
    environment override.

Tests must use deterministic doubles or numeric-loopback servers. They must not
contact the production Gateway, depend on external DNS, sleep to establish
ordering, contain a live credential, or rely on the developer's environment.

## Local gate before each review cycle

Use exact Rust and Cargo 1.94.1 and record every command and count:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Run focused core-contract/native/CLI tests first. Then run the pinned compatibility
generator check and tests without hand-editing generated artifacts, repository
documentation integrity, dependency/license/audit policy, unsafe and diff
checks, target/feature matrix, and a locked fresh release build. Exercise all
user-visible cases through that fresh `target/release/machine-god`, recording
its size and SHA-256 only as regression evidence.

The repository's exact-toolchain fallback rule applies: floating stable is
acceptable locally only if both tools report release 1.94.1 exactly, and the
fallback must be recorded. A newer stable is not evidence.

## Fresh adversarial tracks

After every behavior candidate, spawn three fresh read-only reviewers against
the same exact SHA and tree:

1. **correctness/API/compatibility** — grammar, ownership, thin core trait,
   config/credential order, native fallback state machine, response semantics,
   native sorting, CLI outputs, pinned-source fidelity, and intentional
   differences;
2. **network/security/error lifecycle** — endpoint/header confinement, TLS,
   redirect/proxy/retry rejection, secret and dependency-error redaction,
   body/JSON bounds, cancellation/drop, and no additional authority; and
3. **performance/concurrency/portability** — total deadline, capacity bound,
   allocation/work amplification, output construction, same-poll races, runtime
   ownership, target/feature topology, and absence of product-performance
   claims.

Any blocker, high, medium, or low finding rejects the exact candidate. Resolve
the union in production/tests/docs as appropriate, rerun the complete
replacement gate, commit a new candidate, and use three fresh reviewers. Do not
declare a track or the slice green from a prior SHA, an author's self-review, or
a documentation-only statement.

## Candidate and finding register

| Cycle | Exact candidate/tree | Correctness/API | Network/security | Performance/concurrency | Union | Status |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | pending | pending | pending | pending | pending | **NOT REVIEWED** |

No findings or rejected rationales exist yet. Add every finding verbatim enough
to identify its evidence, severity, affected invariant, resolution commit, and
replacement proof. Preserve rejected candidates as history rather than
rewriting them as green.

## Remote and integration boundary

Only after a zero-finding exact candidate and complete replacement local gate:

1. commit the documentation seal if behavior is unchanged;
2. push the feature branch without force;
3. wait for exact-SHA CI and benchmark-evidence workflows;
4. verify the required jobs and retained exact-SHA artifacts;
5. fast-forward `main` without force;
6. wait for exact-`main` CI and benchmark-evidence workflows; and
7. record those run IDs and exact SHAs in this ledger and maintained summaries.

The benchmark workflow is delivery evidence only. Slice 29 cannot make a
product-performance, speedup, memory, latency, compatibility-promotion, or fx-
equivalence claim.
