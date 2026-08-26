# Milestone 03 `models` CLI review ledger

Status: cycle-3-rejected and locally remediated bounded slice 29. Exact
cycle-1 candidate
`6277aa3dc26f9c485707c667f63525a2138f316b`, tree
`b5e2445ed90df000255b51c2c989d71965db1d77`, passed its complete local gate and
was rejected by three fresh product-review tracks with a deduplicated union of
two medium and six low findings. Exact cycle-2 behavior candidate
`2ea9d94374c4dd18f43255af785ee31088126c56`, tree
`3a948b2950d870a9cabe479bc6c3889dd5a13a3b`, includes all cycle-1 remediation
and passed the complete replacement gate under exact Rust and Cargo 1.94.1.
Three fresh cycle-2 tracks rejected it with a deduplicated union of one high,
one medium, and one low finding. Cycle-2 remediation is locally composed.
Pre-review gate attempt `c01139811685ae73031ed6f6cbd771e4ff636714`, tree
`4ac4e5bd67b98900159aec1772f75d6003ba1d70`, was rejected by a medium DNS-
configuration lifecycle finding before formal cycle-3 review. That finding is
locally remediated. Exact cycle-3 behavior candidate
`2cecc921e48396e81ab6f434007a7ec8e3e890b5`, tree
`8c0d235355582d92aaed6fcca7c1862982494e20`, passed its complete replacement
gate under exact Rust and Cargo 1.94.1. Three fresh cycle-3 reviews rejected it
with a deduplicated union of 0 blocker, 0 high, 1 medium, and 3 low findings.
The cycle-3 findings are locally remediated, but the complete cycle-4
replacement gate and three fresh cycle-4 reviews remain pending. No candidate
is review-green. The frozen behavior contract is
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
| provider-owned deadline wakeup and pre-fallback response drop | `02c9f86619fbdc202f5065c41090415a179316cf` / `0a1ea2a1fde5d66096426caddb5260cbd6b6b13c` | cycle-1 remediation; focused provider/parser 15/15 and loopback HTTP 15/15 green |
| parent-owned signal arbitration, config-order evidence, and WASI cfg repair | `d2890c34bc628dd9ad425f5921e3816bbe1f5eef` / `ec3ef14c4ba8514d8c33185a00e7293d215b8ef9` | cycle-1 remediation; CLI unit 18/18 and integration 23/23 green |
| direct-dependency and Tokio-signal topology repair | `06c94087e91ec298877fbe981695d2638fa1db1e` / `7c3a4c4d1d29db04589a101bf5b8fd7b6ecd22ce` | cycle-1 remediation; parsed manifest/resolved tree 4/4 green |
| arbitrary-precision metadata/default handling | `9cf8c741255c8834d21090cc1e6255c1746d57ff` / `9a6052c9275ec1049ae4fdeb4bfd54c4c1033e17` | cycle-2 remediation; provider/parser 17/17 green |
| async DNS lifecycle and provider-over-HTTP no-runtime handling | `8187b12` / `b985229bffe0cf3c3ccc141c146b0041e4b35b54` | cycle-2 remediation; HTTP integration 16/16 and source lifecycle regression green |
| fail-closed system DNS configuration with no public fallback | `499af85d43738b0b138051ba4669ec27efd0ae1b` / `d96525945f34f067f969d34aa8a5712c40551c78` | cycle-2 remediation; source resolver regressions 2/2 and manifest 4/4 green |
| eager bounded DNS snapshot and fresh resolver per runtime | `d9922ef1b0173c5da6f57aa39a5c7b1c69c55346` / `df03b97d40d9b927022a518eeba5099b33160662` | pre-review gate remediation; generic-Unix bounded input and sequential-runtime evidence green |
| custom absolute-name resolver pinning | `e5248b103c5df67b733006da8210ada05d366345` / `d194034189082b1d71ae9cc69c38060ae6e8c886` | pre-review gate hardening; private resolver 6/6 and HTTP integration 16/16 green |
| cycle-3 gate record and maintained status | `2cecc921e48396e81ab6f434007a7ec8e3e890b5` / `8c0d235355582d92aaed6fcca7c1862982494e20` | exact replacement-gated cycle-3 behavior candidate; formal reviews rejected it |
| stale topology and checklist documentation remediation | `f80bd0560e49306ce56093ea667aa45a22b2c6dd` / `d6d3f354fa1eca6221d482567f9128ac6f73a3bc` | cycle-3 remediation for all three low findings |
| private bounded DNS and construction-time entropy remediation | `b6cf4cbc01ba2470b7aef77b96d5793ad95f6b0d` / `e72f7dd6b4f3306faee5d0d3fc8483c63f4fd24a` | cycle-3 medium remediation; focused resolver 14/14, HTTP 16/16, and manifest 4/4 green; complete cycle-4 gate pending |

The first five component commits composed rejected cycle-1 candidate `6277aa3`.
The three cycle-1 remediation commits and cycle-1 record compose rejected
cycle-2 behavior candidate `2ea9d94`. Documentation-only submission seal
`0b5976da573697b8d7cbc2c393b480559d5c3db7` recorded that immutable candidate
and is exempt from redundant adversarial review under the user's instruction.
The three cycle-2 remediation commits formed rejected pre-review gate attempt
`c011398`; the two later remediation commits and gate record compose exact
replacement-gated cycle-3 behavior candidate `2cecc921`. Documentation-only
submission seal `673773f2cf741497e39b1010e13b790af34af7f5` recorded that
immutable candidate and is exempt from redundant adversarial review under the
user's instruction. Cycle-3 review rejected the behavior candidate. Exact
documentation remediation `f80bd056` and native remediation `b6cf4cb` compose
the current cycle-4 precursor. This ledger does not claim a complete cycle-4
gate, cycle-4 candidate, green adversarial result, feature CI, benchmark,
integration, delivery, performance, compatibility promotion, or fx equivalence.

A local feature-topology refinement adds native
`ai-gateway-model-catalog-http`, makes the CLI enable only that feature, and
retains `ai-gateway-http` as the compatibility umbrella for catalog HTTP plus
direct `bytes` and `web-fetch-http`. Cycle-1 remediation removes direct `bytes`
and Tokio signal handling from native catalog HTTP; only the CLI requests the
signal feature. Cycle-2 remediation added one narrowly scoped direct Hickory
resolver with Tokio integration because Reqwest's default GAI resolver can
leave non-abortable blocking DNS work. Machine-god supplied its own system-
config-only adapter, disabled Reqwest's built-in public-resolver fallback, and
failed closed when system DNS configuration was unavailable. Pre-review gate
remediation eagerly captured a bounded configuration snapshot, built a fresh
zero-cache/hosts-disabled resolver on each active runtime, and made the fixed
hostname absolute before lookup. That behavior formed rejected cycle-3
candidate `2cecc921`.

Cycle-3 remediation keeps Hickory resolver only for bounded platform-
configuration parsing and removes its Tokio resolver integration. Private
bounded Tokio UDP/TCP exchange owns DNS network behavior, with direct
`hickory-proto` decoding and a construction-time fallible query-ID key.
A keyed atomic sequence derives request IDs with bounded SHA-256 work. Request
polling reads neither system configuration, hosts files, nor entropy and spawns
no resolver task. Generation transport/reference-host exports remain broader-
feature-only, while shared credential/bearer/TLS and catalog HTTP exports are
available under either native HTTP feature.

Current cycle-3-remediation-focused local evidence under exact Rust and Cargo
1.94.1 is green: formatting, native all-target/all-feature warnings-denied
Clippy, native all-feature tests and doctests, private resolver 14/14, loopback
HTTP 16/16, and parsed-manifest/resolved-tree 4/4 pass. The current five-target
resolved tree contains the intended catalog edges and excludes them on WASI.
These focused results do not establish the complete cycle-4 replacement gate.
The complete cycle-2 target matrix had
previously confirmed that native default, catalog-
only, compatibility-umbrella, and all-feature checks compile, and that native
and CLI WASI Preview 1 checks compile with only the established native
`read_file` dead-code warning.

On rejected cycle-2 candidate `2ea9d94`, the resolved CLI package-name inventory
drops from 137 on exact precursor
`6431abf43d0407098672307b0a4c028bd0845e3c` to 104 in this refinement and
contains no `hickory-proto`, `hickory-resolver`, `hickory-net`, or `moka`.
An isolated locked release build is 3,635,536 bytes with SHA-256
`f1ba8bec91803de9bb79649836c61cc1263c319e72d644d3ee1fdaf8650293d9`; the
separately built exact precursor is 3,635,520 bytes with SHA-256
`21e8a922b3aa5280a12859ec22ff01db289092cad290f54a73fd60f835b4f7a9`.
These package, size, and hash values are regression/topology evidence only,
not a speed, memory, binary-size improvement, product-performance, or delivery
claim. That cycle-2 replacement gate is green but its reviews rejected the
candidate. Exact cycle-3 candidate `2cecc921` passed its replacement gate and
its reviews rejected it. The complete cycle-4 gate, cycle-4 candidate, three
fresh cycle-4 reviews, feature CI, integration, and exact `main` CI remain
pending.

## Exact cycle-2 replacement gate

Exact behavior candidate `2ea9d94374c4dd18f43255af785ee31088126c56`,
tree `3a948b2950d870a9cabe479bc6c3889dd5a13a3b`, passed the complete local gate
under exact `rustc 1.94.1 (e408947bf 2026-03-25)` and
`cargo 1.94.1 (29ea6fb6a 2026-03-24)` without fallback:

- formatting, workspace all-target/all-feature warnings-denied Clippy, all 955
  registered non-documentation tests, and both doctests passed;
- focused core 6/6, native provider/parser 15/15, credential 6/6, loopback HTTP
  15/15, CLI unit 18/18, CLI integration 23/23, and manifest 4/4 suites passed;
- 134 benchmark, compatibility-generator, and manifest tests passed with the
  expected eight macOS skips, and the pinned-fx generator check was byte-stable;
- documentation integrity covered 78 Markdown files, 127 balanced fence
  blocks, 567 inline links, and 410 repository-relative targets with zero
  missing targets;
- `cargo-deny 0.19.9` and `cargo-audit 0.22.2` passed; audit scanned 1,226
  cached advisories across 210 lockfile dependencies with zero vulnerabilities;
- host/default, narrow catalog, broad HTTP, all-feature, Linux no-feature,
  FreeBSD no-feature, and WASI native/CLI checks passed. WASI emitted only the
  established native `read_file` warning and no CLI warning;
- resolved topology keeps native catalog HTTP free of Tokio signal,
  `signal-hook-registry`, direct `bytes`, Hickory, and Moka. The CLI alone
  requests signal handling; the broad generation HTTP feature retains its
  intentional direct `bytes` and Hickory edges;
- the exact 33-file delivered-base diff is +7,360/-209, passes `git diff
  --check`, adds no unsafe Rust, and leaves workflows, benchmark workloads, and
  generated compatibility inventory unchanged; and
- a fresh locked 3,635,504-byte release binary with SHA-256
  `4d89b42cdc89b6c7feb0802a969594936dd3ed9c22399e292e5387ec43919d99`
  passed exact help, version, invalid-argument, human failure, and JSON failure
  black-box cases with zero configuration writes.

The local macOS host has no Linux C sysroot for the AWS-LC HTTP dependency, so
HTTP-feature Linux/FreeBSD evidence is dependency resolution rather than a
cross-link claim; exact remote Linux CI remains authoritative. This candidate-
local gate makes no review, workflow, integration, delivery, performance, or
fx-equivalence claim. The following documentation-only submission seal records
the evidence and is exempt from redundant adversarial review.

## Rejected cycle-3 pre-review gate attempt

Exact pre-review behavior attempt
`c01139811685ae73031ed6f6cbd771e4ff636714`, tree
`4ac4e5bd67b98900159aec1772f75d6003ba1d70`, did not pass its complete gate and
was never submitted to the three formal cycle-3 reviewers. Under exact Rust and
Cargo 1.94.1 without fallback:

- formatting, workspace all-target/all-feature warnings-denied Clippy, all 960
  registered non-documentation tests, and both doctests passed;
- focused core 6/6, provider/parser 17/17, credential 6/6, loopback HTTP 16/16,
  private resolver 2/2, CLI unit 18/18, CLI integration 23/23, and manifest 4/4
  suites passed;
- the full 134-test Python rerun passed with eight expected macOS skips after
  one fake-Git timeout case produced an initial timing-only failure and then
  passed both its isolated retry and the complete sequential rerun;
- the pinned-fx generator was byte-stable; documentation integrity covered 78
  Markdown files, 127 fence blocks, 561 inline links, and 409 repository-
  relative targets with zero errors;
- host/feature, Linux/FreeBSD no-feature, WASI, dependency-policy, unsafe, deny,
  and audit checks passed. Audit scanned 1,226 advisories across 211 lockfile
  dependencies with zero vulnerabilities; and
- a fresh locked 4,282,880-byte release binary with SHA-256
  `a5c27976519c887e00f219f79fb009191355fe7a5f6c611380fd5536ea0f3500`
  passed six release black-box cases with no state writes. Its 138-package
  inventory included the intended Hickory/Moka graph.

The dependency/lifecycle audit nevertheless found one **medium** issue:
`SystemHickoryResolver` initialized Hickory's system configuration lazily while
the timed Reqwest future was being polled. Hickory's platform loader is
synchronous, and generic Unix used allocation-unbounded `fs::read`; neither the
request deadline nor cancellation could preempt that nested poll. The attempt
was therefore rejected before review despite its other green checks.

Remediation `d9922ef1b0173c5da6f57aa39a5c7b1c69c55346` eagerly retains only a
validated configuration snapshot, bounds generic-Unix input to 64 KiB plus one
overflow byte with nonblocking regular-file reads, disables hosts-file and
response-cache work, and builds a fresh resolver for each active runtime.
Hardening `e5248b103c5df67b733006da8210ada05d366345` explicitly disables Reqwest's
built-in Hickory selector and resolves exactly one absolute fixed hostname,
without system search suffixes. Deterministic private evidence now passes 6/6,
including eager-once/no-request-time loading, redacted unavailable/oversized
configuration, pending lookup teardown, and a local UDP resolver reused across
two sequential current-thread runtimes. Focused loopback HTTP remains 16/16.
Those focused results did not replace the complete gate recorded below.

## Exact cycle-3 replacement gate

Exact behavior candidate `2cecc921e48396e81ab6f434007a7ec8e3e890b5`, tree
`8c0d235355582d92aaed6fcca7c1862982494e20`, passed the complete local gate
under exact `rustc 1.94.1 (e408947bf 2026-03-25)` and
`cargo 1.94.1 (29ea6fb6a 2026-03-24)` without fallback:

- formatting, workspace all-target/all-feature warnings-denied Clippy, all 964
  registered non-documentation tests, and both doctests passed with no warning;
- focused core 6/6, provider/parser 17/17, credential 6/6, loopback HTTP 16/16,
  private resolver 6/6, CLI unit 18/18, CLI integration 23/23, and manifest 4/4
  suites passed;
- all 134 Python tests passed on the first sequential run with eight expected
  macOS skips. The exact pinned-fx generator check and two external
  regenerations were byte-identical to the committed files;
- documentation integrity covered 78 Markdown files, 127 balanced fence
  blocks, 561 inline links, and 409 repository-relative targets with zero
  errors;
- host native default/catalog/broad/all-feature and CLI checks passed. Native
  default/catalog and CLI WASI Preview 1 checks passed with only the established
  native `read_file` warning. Linux and FreeBSD no-feature checks passed;
- catalog-feature Linux/FreeBSD cross checks reached `aws-lc-sys` but could not
  compile its C code without target sysroots, before machine-god product Rust.
  Target-resolved trees and cfg inspection passed; exact remote Linux CI remains
  authoritative for those HTTP-feature targets;
- the narrow resolved graph contains direct `hickory-resolver` and transitive
  Hickory protocol/network/Moka packages while excluding native direct `bytes`,
  `web-fetch-http`, Tokio signal, `signal-hook-registry`, and Reqwest's
  `hickory-dns` feature. The CLI alone requests Tokio signal handling;
- `cargo-deny 0.19.9` passed with only established allowed duplicate warnings.
  `cargo-audit 0.22.2` loaded 1,226 advisories and scanned 211 dependencies with
  zero vulnerabilities;
- the exact delivered-base diff covers 33 files at +8,524/-209, passes
  `git diff --check`, adds no unsafe Rust, and leaves workflows, benchmark
  workloads, and generated compatibility files unchanged; and
- a fresh locked 4,282,880-byte release binary with SHA-256
  `9d92285755ccdb5fd8711b4e8752802f34d359a0de0f9b18344dbab64dc24fc2`
  passed six isolated help/argument/config/public-catalog black-box cases with
  zero configuration or state writes. Its 138-package inventory contains the
  intended Hickory/Moka graph.

Source-backed lifecycle inspection confirmed that construction eagerly retains
only the validated snapshot, request polling calls neither system configuration
nor hosts-file readers, the custom absolute-name resolver replaces every
Reqwest default path, and each request owns a fresh active-runtime Hickory
resolver. Deterministic local DNS succeeded across two separately created and
dropped current-thread runtimes. This gate makes no adversarial-review,
workflow, integration, delivery, product-performance, or fx-equivalence claim.

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
| 1 | `6277aa3dc26f9c485707c667f63525a2138f316b` / `b5e2445ed90df000255b51c2c989d71965db1d77` | 0 blocker, 0 high, 0 medium, 4 low | 0 blocker, 0 high, 2 medium, 1 low | 0 blocker, 0 high, 2 medium, 3 low | 0 blocker, 0 high, 2 medium, 6 low after deduplication | **REJECTED** |
| 2 | `2ea9d94374c4dd18f43255af785ee31088126c56` / `3a948b2950d870a9cabe479bc6c3889dd5a13a3b` | 0 blocker, 0 high, 0 medium, 1 low | 0 blocker, 1 high, 1 medium, 0 low | 0 blocker, 0 high, 1 medium, 1 low | 0 blocker, 1 high, 1 medium, 1 low after deduplication | **REJECTED** |
| 3 | `2cecc921e48396e81ab6f434007a7ec8e3e890b5` / `8c0d235355582d92aaed6fcca7c1862982494e20` | 0 blocker, 0 high, 0 medium, 2 low | 0 blocker, 0 high, 1 medium, 1 low | 0 blocker, 0 high, 0 medium, 0 low | 0 blocker, 0 high, 1 medium, 3 low after deduplication | **REJECTED** |
| 4 | pending / pending | pending | pending | pending | pending | **NOT SUBMITTED** |

### Cycle 1 rejected findings and remediation

1. **Medium — signal readiness was not authoritative.** The current-thread CLI
   spawned signal tasks without proving their first poll, then aborted them as
   soon as the provider returned. A signal or registration failure ready during
   the provider's terminal synchronous work could lose to success. Remediation
   `d2890c34bc628dd9ad425f5921e3816bbe1f5eef` creates and primes parent-owned
   listener futures before provider-future creation, checks signals before and
   after a ready provider poll, cancels before drop, and adds deterministic
   same-poll/registration/drop evidence without real signals or sleeps.
2. **Medium — a pending injected transport could defeat the total deadline.**
   The provider raced only cancellation and `get`; its wall-clock deadline was
   rechecked only when another future woke. Remediation
   `02c9f86619fbdc202f5065c41090415a179316cf` adds the transport's required
   independently polled `wait_until(deadline)` authority, cancellation-first
   precedence, and a deterministic permanently-pending request wake/drop
   regression.
3. **Low — the rejected authenticated response remained owned across public
   fallback.** Remediation `02c9f86619fbdc202f5065c41090415a179316cf`
   explicitly drops that response before the cancellation/deadline checks and
   second call; the first production permit was already request-future-owned.
4. **Low — closed SIGTERM waiting was silently ignored.** Remediation
   `d2890c34bc628dd9ad425f5921e3816bbe1f5eef` maps `recv() == None` to the same
   fixed signal-wait failure as registration failure and proves it through the
   injected signal source.
5. **Low — config-once and exact downstream ordering lacked direct evidence.**
   Remediation `d2890c34bc628dd9ad425f5921e3816bbe1f5eef` routes production through one
   config, credential, transport/list sequence and adds a deterministic trace
   proving one call each plus config- and credential-terminal short-circuiting.
6. **Low — the recorded WASI result omitted two CLI warnings.** Remediation
   `d2890c34bc628dd9ad425f5921e3816bbe1f5eef` cfg-scopes native-only failure
   variants and the provider classifier. Exact focused WASI CLI checks now emit
   only the established native `read_file` warning.
7. **Low — Tokio signal and direct `bytes` dependencies were mis-scoped.**
   Remediation `06c94087e91ec298877fbe981695d2638fa1db1e` moves Tokio signal activation
   from the workspace dependency to the CLI and direct `bytes` activation from
   catalog HTTP to the broader generation feature. Parsed-manifest and resolved-
   tree evidence rejects regression.
8. **Low — maintained candidate/gate provenance was stale.** This cycle-1
   record preserves the exact rejected SHA/tree, all three raw track counts,
   their deduplicated union, and the remediation lineage. The exact cycle-2
   candidate and replacement-gate evidence are now recorded above; its three
   fresh reviews remain explicitly pending.

Every cycle-1 finding remains remediated. Cycle 1 remains rejected history.

### Cycle 2 rejected findings and remediation

1. **High — provider-over-HTTP missing-runtime handling panicked.** The concrete
   `wait_until` future constructed a Tokio timer before the provider could poll
   `get` and receive its typed `RuntimeRequired` result. Remediation `8187b12`
   probes the current handle before timer construction and leaves the deadline
   waiter inert outside Tokio, so provider composition reaches the fixed typed
   request error without panic. The provider-over-concrete regression is green.
2. **Medium — default GAI DNS could outlive deadline and runtime teardown.**
   Reqwest's resolver used non-abortable blocking `getaddrinfo`; cancellation or
   deadline could drop the request while runtime shutdown still waited on DNS.
   Remediation `8187b12` installs asynchronous Hickory resolution and proves
   pending lookup drop, permit restoration, deadline/cancellation completion,
   and current-thread teardown. Follow-up `499af85` replaces Reqwest's built-in
   Google fallback with a private system-config-only adapter that fails closed
   and redacted; no default GAI or public-resolver path remains.
3. **Low — standards-valid out-of-range JSON numbers bypassed metadata
   defaults.** Default numeric conversion rejected a large integer or `1e400`
   before `released` could default to zero or ignored metadata could be skipped.
   Remediation `9cf8c741255c8834d21090cc1e6255c1746d57ff` uses borrowed raw values and a
   bounded linear structural pass, preserving exact depth/node/cancellation/
   deadline limits while defaulting or ignoring non-`i64` metadata. Exact
   arbitrary-number, node, and depth regressions are green.

All cycle-2 findings are locally remediated. Exact cycle-3 candidate `2cecc921`,
tree `8c0d235`, passed the complete replacement gate and then was rejected by
its three fresh reviews. Cycle 2 remains rejected history.

### Cycle 3 rejected findings and remediation

1. **Medium — request polling could invoke fallible Hickory entropy and abort.**
   Hickory's network resolver lazily called `rand::rng()`/`SysRng` while the
   first lookup future was polled. Entropy failure could panic, and the release
   profile's aborting panic strategy would bypass the provider's timer,
   cancellation, and fixed redacted error mapping. Native remediation
   `b6cf4cbc01ba2470b7aef77b96d5793ad95f6b0d`, tree
   `e72f7dd6b4f3306faee5d0d3fc8483c63f4fd24a`, keeps Hickory only for bounded
   configuration parsing and replaces its network resolver with private bounded
   UDP/TCP exchange. A fallible 32-byte key is captured at construction and a
   keyed atomic sequence derives IDs with bounded SHA-256 work. Entropy failure
   is retained as fixed unavailable state; request polling invokes no entropy
   source or detached resolver task. Deterministic private resolver evidence is
   14/14, including entropy-failure/no-packet behavior and sequential runtimes.
2. **Low — `docs/architecture.md` described obsolete catalog topology.** Exact
   documentation remediation
   `f80bd0560e49306ce56093ea667aa45a22b2c6dd`, tree
   `d6d3f354fa1eca6221d482567f9128ac6f73a3bc`, now distinguishes the narrow
   catalog's bounded DNS/configuration graph from the generation-only surface.
3. **Low — `docs/ai-gateway-credentials.md` described obsolete catalog
   topology.** The same exact documentation remediation records the current
   provider-neutral credential boundary without falsely excluding the catalog's
   DNS/configuration dependencies.
4. **Low — the bottom M03 checklist stopped at rejected cycle 1.** The same
   exact documentation remediation records rejected cycle 2, the rejected
   pre-review attempt, the cycle-3 gate and rejection, and the next pending
   replacement/review boundary. This ledger commits the exact raw cycle-3
   counts and deduplicated union.

All cycle-3 findings are locally remediated by exact commits `f80bd056` and
`b6cf4cb`. Their composition is only a cycle-4 precursor: the complete cycle-4
replacement gate, immutable cycle-4 candidate, and three fresh cycle-4 reviews
remain pending. Cycle 3 remains rejected history, and no candidate is review-
green.

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
