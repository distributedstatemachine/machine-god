# Milestone 03 `models` CLI review ledger

Status: pending-review ledger for bounded slice 29. Local implementation and
35 focused independent native tests exist, but the complete local candidate
gate remains pending. No exact candidate has been submitted to adversarial
review, and no review track is green. The frozen behavior contract is
[`models-cli.md`](../models-cli.md); work started from exact delivered base
`1de3b7eddf6a4d9046d48098defecf6bfa336442`. The pinned comparison input is fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`.

The locally composed lineage is:

| Component | Exact commit/tree | Current status |
| --- | --- | --- |
| core contract and focused contract tests | `a6c6ff333176689b0c53bcf35070e9d59afd1b28` / `1ab98b4416ba4371e508ce65264bc7517c2dc851` | implemented locally |
| native provider, credential projection, parser, sort, and HTTP transport | `7c966b23d75a880a23d49e1e6ba9780e512e84b8` / `8c324bee5a52f2269b6f795c100f8aa5c86149b2` | implemented locally |
| CLI parser, current-thread host, rendering, and CLI tests | `e84ed2a46b1ac5fe7428414375609af562c65105` / `78e107e6e75e91667e1669681b48f8f7fa61ce61` | implemented locally |
| native checked-deadline and terminal-precedence remediation | `52e9b7d74f3979f7f7f55387243e96bd78773fe3` / `56358767fcbf1ab216db0f1b8b8f4a550eb6c864` | implemented locally |
| independent native evidence, sourced from `219f6a71a766e9b833a98f236cfbc3aaff292cd5` | `12263afa458e48f2963ae3d0e3db5cf219f8bdf6` / `9b6241cb023aa0f42b808b32dbee3afecefc3d01` | 35 focused tests green: 14 provider/parser, 15 loopback HTTP, 6 credential |

These component commits are not a gated review candidate. The focused evidence
does not substitute for the complete workspace/candidate gate. This ledger
does not claim that complete gate, an adversarial result, feature CI, benchmark,
integration, delivery, performance, compatibility promotion, or fx equivalence.

A local feature-topology refinement is also present but is not yet a gated
candidate. It adds native `ai-gateway-model-catalog-http`, makes the CLI enable
only that feature, and retains `ai-gateway-http` as the compatibility umbrella
for catalog HTTP plus `web-fetch-http`. Parsed-manifest and resolved-tree
evidence must prove the narrow feature's required direct dependencies and the
absence of `web-fetch-http`, Hickory DNS, and Moka before review. Generation
transport/reference-host exports remain broader-feature-only, while shared
credential/bearer/TLS and catalog HTTP exports are available under either
native HTTP feature.

Refinement-focused local evidence under exact Rust and Cargo 1.94.1 is green:
the parsed-manifest/resolved-tree suite passes 3/3; native default, catalog-
only, compatibility-umbrella, and all-feature checks compile; the catalog-only
provider/parser, credential, and loopback HTTP suites pass 14/14, 6/6, and
15/15; and the existing compatibility-umbrella library plus generation HTTP
and credential suites pass 286/286, 20/20, 8/8, 2/2, and 3/3. Workspace all-
target/all-feature warnings-denied Clippy is green. Default and catalog-only
WASI Preview 1 checks both compile with the same established `read_file`
dead-code warning, so the narrower feature adds no WASM diagnostic.

The resolved CLI package-name inventory drops from 137 on exact precursor
`6431abf43d0407098672307b0a4c028bd0845e3c` to 104 in this refinement and
contains no `hickory-proto`, `hickory-resolver`, `hickory-net`, or `moka`.
An isolated locked release build is 3,635,536 bytes with SHA-256
`f1ba8bec91803de9bb79649836c61cc1263c319e72d644d3ee1fdaf8650293d9`; the
separately built exact precursor is 3,635,520 bytes with SHA-256
`21e8a922b3aa5280a12859ec22ff01db289092cad290f54a73fd60f835b4f7a9`.
These package, size, and hash values are regression/topology evidence only,
not a speed, memory, binary-size improvement, product-performance, or delivery
claim. The complete slice gate, three fresh adversarial reviews, feature CI,
integration, and exact `main` CI remain pending.

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
  decompression, or application retry; fixed headers are
  `Accept: application/json`, `Accept-Encoding: identity`, and the package-
  version machine-god user agent, plus authorization only for authenticated
  access;
- authenticated 401/403 alone causes exactly one fully anonymous fallback,
  under the original deadline, after dropping the first response and permit;
- the total 30-second bound covers the default-8/hard-32 capacity wait and both
  attempts without reset; the same absolute deadline reaches both calls, while
  each call owns attempt-local cancellation/timer waiters rather than one outer
  sleep spanning fallback;
- body 256 KiB retained with a crossing frame rejected before append, depth 32,
  nodes 16,384, raw entries 1,024,
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
  deadline construction is checked, and cancellation/deadline are rechecked on
  Serde/trailing failures and around sort/final model construction;
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
6. exact production request method/origin/path/headers, including the machine-
   god package-version user agent, and absence of body,
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
| 1 | pending after complete local gate | pending | pending | pending | pending | **NOT REVIEWED** |

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
