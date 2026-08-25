# Milestone 03 native `web_fetch` review 01

Status: **IN PROGRESS — CYCLE 8 REJECTED**

## Base and boundary

- Exact delivered base:
  `a56ff350c2aace1dc22cb14c269aee89d399cd8e`.
- Integration branch: `agent/m03-web-fetch`.
- Normative contract: [`web-fetch.md`](../web-fetch.md).
- Pinned comparison reference: [`vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`](https://github.com/vercel-labs/fx/commit/b1774fbf6c7602b503026f96f6e960e946c692ef).

This is the live review ledger for the proposed twenty-seventh bounded slice.
Production, independently owned evidence, host composition, and a complete
pre-review local gate exist. Formal cycle 1 rejected exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. Its remediation passed a complete
replacement local gate. Formal cycle 2 rejected exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`. Exact composed cycle-2 remediation
precursor `1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate. This gate record establishes no formal-review outcome, workflow,
integration, delivery, compatibility, or product-performance claim; formal
candidates are identified only by exact-SHA review results. Milestone 03
remains in progress with twenty-six delivered slices.
Formal cycle 3 rejected exact candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`. Correctness/API reported 0
blocker, 0 high, 1 medium, and 1 low; network/HTTP lifecycle reported 0
blocker, 0 high, 1 medium, and 1 low; performance/concurrency reported zero
findings at every severity. The deduplicated union is 0 blocker, 0 high,
2 medium, and 1 low. Exact isolated production remediation component
`9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only
`crates/machine-god-native/src/web_fetch.rs`. Exact isolated
independent-evidence commit `3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
only `crates/machine-god-native/tests/web_fetch_http.rs`. This remediation
record makes no replacement-gate, formal-review, workflow, integration,
delivery, compatibility, or product-performance claim; formal candidates are
identified only by exact-SHA review results.
Formal cycle 4 rejected exact candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 2 low findings; network/HTTP lifecycle was
green at 0/0/0/0; performance/concurrency reported 0 blocker, 0 high, 1
medium, and 0 low. The deduplicated union is 0 blocker, 0 high, 2 medium, and
2 low. The exact candidate is rejected. Exact isolated production remediation
component `9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, changes only native
`web_fetch.rs`. Exact isolated independent-evidence commit
`408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, changes only native
`web_fetch_http.rs`. Exact composed code/evidence precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`, contains both. This remediation
record makes no replacement-gate, formal-review outcome, candidate, workflow,
integration, delivery, compatibility, or product-performance claim. Formal
reviewer reports identify the exact candidate they reviewed.
Exact composed cycle-4 remediation precursor
`892a52267e7ccf478e9ed567875dc95912be5412`, tree
`da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed.
Formal cycle 5 rejected exact candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`. Correctness/API reported
0 blocker, 0 high, 0 medium, and 1 low finding; network/HTTP lifecycle reported
0 blocker, 0 high, 1 medium, and 0 low; performance/concurrency reported
0 blocker, 0 high, 0 medium, and 1 low. The repeated timer-accounting low
deduplicates across correctness and performance. The union is 0 blocker,
0 high, 1 medium, and 1 low, so the exact candidate is rejected.
Exact isolated source remediation
`cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
`web_fetch.rs`. Exact composed code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, compatibility, or product-performance claim.
Exact composed cycle-5 remediation precursor
`8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
`5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Formal cycle 6
rejected exact candidate `5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`, with a deduplicated union of
0 blocker, 0 high, 1 medium, and 0 low. Exact source remediation
`e6eba937423714bc96a7c399b4823c6deda22da7`, tree
`bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, corrects the target dependency
scope and adds parsed-manifest evidence. This remediation record makes no
replacement-gate, formal-review outcome, candidate, workflow, integration,
delivery, compatibility, product-performance, or fx-equivalence claim.
Exact cycle-6 remediation precursor
`31e7f11870a86795f9d98831d8405714b49b989e`, tree
`68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed.
Formal cycle 7 rejected exact candidate
`47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`, with a deduplicated union of
0 blocker, 0 high, 1 medium, and 0 low. Exact root remediation
`0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
`66527a146dc87b1aa9f3b7ee99385efb8022a342`, rejects `.arpa` infrastructure
names before network authority. Tree-identical isolated remediation
`8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` records the same correction. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, compatibility, product-performance, or fx-
equivalence claim.
Exact cycle-7 remediation precursor
`b365cf7704bacb378b280f25d702896dcc216e0c`, tree
`8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed.
Formal cycle 8 rejected exact candidate
`be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`, with a deduplicated union of
0 blocker, 0 high, 2 medium, and 0 low. Exact root remediation
`9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
`fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, validates a capped leading DNS
response tuple before truncated-UDP replay. Tree-identical isolated remediation
`38a36725559fe962e6f649e07cc5de154c940257` records the same correction and
deterministic evidence. This remediation record makes no replacement-gate,
formal-review outcome, candidate, workflow, integration, delivery,
compatibility, product-performance, or fx-equivalence claim.
Exact cycle-8 remediation precursor
`d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
`a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed.

## Frozen candidate boundary

The candidate is a cfg-gated, non-WASM, rootless `WebFetchTool` behind
`web-fetch-http`; `ai-gateway-http` includes that feature. It accepts only a
sole `{url:string}`, trims its boundary, bounds the canonical ASCII URL at
2,000 bytes, upgrades `http` to canonical `https`, rejects credentials, strips
fragments, and admits only a public multi-label DNS name or strict public IP
literal. Special-use `.alt` names and trailing-dot numeric IPv4 spellings are
rejected. An explicit default HTTPS port canonicalizes to an omitted port;
only an explicit non-default port is preserved. Effect-free preflight must
produce exact `Capability::Network` policy/execution agreement. Existing core
policy presents network authority as `Critical` and the default path stays
`Ask`.

Allowed execution performs one fixed-header, no-auth Reqwest HTTP/1 GET with no
proxy, retry, referer, cookie, automatic redirect, or decompression behavior.
Every one of at most 32 DNS answers must be public. After validation, the
accepted set is stably deduplicated in first-seen A-then-AAAA order and pinned
to the connection. Defaults are eight active calls, a 10-second connect bound,
a 60-second total bound, an inclusive 24 KiB body bound, and a 56 KiB
serialized-result bound; active calls can never exceed 32. The connect bound
applies to HTTP connects and truncated-DNS TCP replay, with DNS TCP connect
subordinate to cancellation and any earlier overall deadline. Cancellation and
drop release the response and permit and own no machine-god worker. One outer
bounded wrapper retains the acquired permit through transport, rendering,
serialized-result validation, and the final cancellation/deadline boundary.
Its cancellation future and one reusable outer machine-god invocation-deadline
sleep cover bounded permit, DNS, HTTP, and body waits; the final synchronous
boundary directly checks the token/deadline without another waiter. Each
truncated A or AAAA DNS TCP replay additionally owns one short-lived configured
connect-timeout sleep, so one invocation owns at most two sequential DNS replay
sleeps. Reqwest/Hyper may own bounded HTTP connection-attempt timers. The outer
sleep is allocated once; each DNS replay sleep is allocated once when that
replay begins. None resets or extends the outer absolute deadline.
Construction is runtime-independent. Polling production requires a current
host-owned Tokio runtime with I/O and time enabled; no current handle returns
`RuntimeRequired`, while a current driverless runtime violates the documented
`# Panics` precondition and may terminate a release process. The one outer
machine-god Tokio invocation-deadline sleep and one cancellation waiter are
reused across bounded permit, DNS, HTTP, and body waits. The final synchronous
boundary checks the token/deadline directly without a second waiter. The native
transport checks cancellation and that same absolute deadline before each
effect transition between A, AAAA, TCP
replay, HTTP dispatch, and body work.

Native production-transport construction synchronously snapshots the first
UDP nameserver from host system resolver configuration and one random query-ID
seed outside invocation timing and without requiring Tokio. It retains those
prerequisites or fixed unavailable state until reconstruction. Hostname
execution uses the stored nameserver and derives query IDs from the seed plus
an atomic per-query sequence, while an admitted public IP literal bypasses
failed hostname prerequisites. The invocation performs no blocking per-query
entropy and sends bounded rooted A then AAAA queries on owned Tokio sockets,
using TCP only once when a UDP response is truncated. A
one-byte UDP overflow witness enforces the inclusive 4 KiB message cap; TCP
length outside 12 through 4,096 bytes is rejected before allocation, and a
still-truncated TCP response is invalid. One raw predecode helper gates both
UDP and TCP before Hickory: the header is at least 12 bytes, `QDCOUNT == 1`,
`ANCOUNT <= 39`, `NSCOUNT <= 128`, `ARCOUNT <= 128`, aggregate resource records
are at most 128, and the actual payload satisfies the checked count-implied
minimum of `12 + 5 * questions + 11 * resource_records`. Strict query tuple,
Internet class, rooted CNAME-chain, terminal owner, 32-address, and every-
address-public checks apply. There is no
libc lookup, cache, retry, search suffix, resolver thread, or spawned resolver
task. The admitted set feeds a fresh pinned Reqwest client using a process-wide
cached Rustls root configuration and fixed HTTP/1.1 ALPN, so roots are not
reparsed per invocation.

Only 2xx and identity encoding succeed. Text, JSON, XML, JavaScript, bounded
raw HTML, complete-bounded-body missing-MIME classification, model-unsafe-text
rejection, and metadata-only
binary results follow the normative contract. Every result starts with the
upstream-untrusted warning and includes query-redacted URL, status, normalized
effective MIME, content kind, and `cache_hit: false`. An absent declaration is
reported as inferred `text/plain` or `application/octet-stream`, not an
absence marker. Errors are fixed and redacted.

Every production and explicitly injected/custom candidate host has thirteen
alphabetical tools, but only its twelve workspace-backed tools use the original
retained descriptor plus eleven clones. `web_fetch` is rootless. There is no
CLI change.

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

## Composed implementation and complete local gate

The non-overlapping components now compose locally:

- production commit `8b2a66993989fcdc67ab7d42f9d3e6a6858a9cfe`
  supplies cfg/public exports and bounded URL/DNS/HTTP/result behavior;
- independent commit `9825a890cf1a21a8585b29aff537dde611508517`
  supplies core serde plus direct and engine evidence;
- independent commit `a09dbc7915a16478ae5a4a70aa177ea718539b49`
  supplies deterministic production-construction, runtime, cancellation, and
  redaction evidence; and
- the integration branch wires the rootless tool into the shared production and
  explicitly injected/custom composition paths; each has exactly thirteen
  alphabetical tools while retaining twelve descriptor-backed workspace tools.

Focused exact Rust 1.94.1 evidence is green: 11 private, 13 direct, five
engine, three production-boundary, seven host, and 65 core-contract tests, plus
warnings-denied all-target/all-feature native Clippy. One independent-test
compile shadow and its warnings-denied reference signature were corrected
without weakening an assertion.

Exact pre-review local-gate record
`0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed. The worktree was clean and
exact Rust/Cargo 1.94.1 was used without fallback. This successful gate does
not override the later non-green formal review.

The four repository-required commands passed. The default workspace lists 881
tests; the extended all-target/all-feature workspace lists 961 tests and its
complete run also passed. A fresh locked release CLI is a 319,152-byte arm64
Mach-O with SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
Its bare, version, help, inert human-status, and inert JSON-status exercises
passed without changing the CLI surface.

Two consecutive uncontended CI-style Python discovery runs passed 130 tests
with eight expected macOS skips. An earlier run overlapped a full dependency-
tool compilation and transiently failed two timing-sensitive POSIX tests; both
passed focused and in both replacement full runs. Pinned fx checkout
`b1774fbf6c7602b503026f96f6e960e946c692ef` was clean and compatibility
regeneration passed. Exact `cargo-deny` 0.20.2 and cached `cargo-audit` 0.22.2
passed. All 74 Markdown files, 529 inline links, and 377 relative file links
passed target, fence, whitespace, and final-newline checks. The 19-file diff
passed whitespace review and changes no CLI source, workflow, benchmark,
`Cargo.lock`, or generated compatibility inventory.

Linux and FreeBSD baseline libraries plus all 53 integration targets
type-checked, their baseline libraries passed warnings-denied Clippy, and the
host-native standalone feature type-checked all three web-fetch targets. The
WASI all-target/all-feature workspace check, dependency exclusion, API-doc cfg
audit, and retained Node 22.22.0 active unsupported test all passed. Unsafe Rust
remains forbidden and absent. Standalone HTTP cross-compilation stopped in
third-party `aws-lc-sys` before machine-god HTTP Rust compiled because the
macOS cross-host lacks Linux/FreeBSD C sysroot headers. Exact native Linux CI
must therefore supply Linux HTTP evidence; fully proving FreeBSD HTTP behavior
requires a FreeBSD host or suitable sysroot.

The formal cycle-1 SHA and tree are recorded below. That candidate is rejected.

## Formal cycle 1 — not green

Three fresh agents independently inspected exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`, in isolated detached worktrees.
Every track is **NOT GREEN** because the workflow treats a finding at any
severity as candidate rejection.

### Correctness and public API

Counts: **0 blocker, 0 high, 2 medium, 3 low**.

- **Medium — driverless-runtime abort:** checking only for a current Tokio
  handle does not prove that its time and I/O drivers are enabled. Constructing
  the deadline sleep can therefore panic instead of returning the documented
  fixed `RuntimeRequired` failure.
- **Medium — missing `.alt` exclusion:** the lexical public-name predicate
  admits the special-use `.alt` suffix despite the public-DNS-only contract.
- **Low — trailing-dot numeric admission:** a raw IPv4 spelling such as
  `8.8.8.8.` can canonicalize to an accepted address even though ambiguous
  numeric spellings are supposed to fail closed.
- **Low — missing-MIME contract mismatch:** rendering emits inferred
  `text/plain` or `application/octet-stream`, while the maintained contract
  described a fixed absence value.
- **Low — explicit-default-port mismatch:** canonicalization strips explicit
  HTTPS port 443, while the maintained contract said every accepted explicit
  port was preserved.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 1 high, 3 medium, 0 low**.

- **High — driverless-runtime abort:** the same active-handle check permits a
  current runtime without enabled time or I/O drivers, allowing production
  execution to abort rather than fail through the typed redacted taxonomy.
- **Medium — non-abortable libc DNS continuation:** cancelling or timing out
  Tokio `lookup_host` can drop its future while host `getaddrinfo` work remains
  active in Tokio's blocking pool. The request permit and documented total
  deadline can then end before that DNS work, contradicting the claimed
  lifetime boundary.
- **Medium — permit and deadline end before rendering:** the production
  transport releases its permit and completes its deadline when it returns a
  `WebFetchResponse`; MIME normalization, UTF-8/control validation, result
  allocation, and serialization occur later in `WebFetchTool`.
- **Medium — missing deterministic production HTTP lifecycle evidence:** the
  production-boundary suite did not exercise the real Reqwest success,
  redirect, encoding, body-stream, DNS-pinning, and TLS-hostname paths. Its
  injected transport coverage could not establish those production claims.

### Performance and concurrency

Counts: **0 blocker, 0 high, 3 medium, 2 low**.

- **Medium — permit and deadline end before rendering:** the same early
  release permits result classification and serialization to execute outside
  both the advertised active-call cap and total deadline.
- **Medium — per-wait timer allocation:** every bounded permit, DNS, HTTP, and
  response-chunk await allocates a new boxed Tokio sleep even though the
  invocation has one absolute deadline.
- **Medium — missing deterministic production concurrency/timing evidence:**
  tests did not establish the production eight/default and 32/hard saturation
  behavior, queued-deadline behavior, or permit release across success,
  cancellation, timeout, error, and drop.
- **Low — repeated trust-store setup:** every invocation clones the retained
  certificate vector into a fresh client builder, adding avoidable per-request
  TLS configuration work.
- **Low — stale candidate state:** the ledger still said that an immutable
  candidate was pending after `3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`
  already existed.

### Consolidated union and disposition

After deduplicating overlaps, cycle 1 has **0 blocker, 1 high, 6 medium, and 5
low** findings. The union is: driverless-runtime failure, `.alt` admission,
trailing-dot numeric admission, missing-MIME wording, explicit-default-port
wording, host libc DNS continuation beyond the permit/deadline, permit/deadline
release before rendering, missing deterministic production HTTP evidence,
per-wait timer allocation, missing deterministic production concurrency/timing
evidence, repeated per-invocation trust-store setup, and stale candidate-state
wording.

Exact candidate `3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c` is rejected and must never be
used as delivery evidence. Documentation now rejects `.alt` and trailing-dot
numeric hosts, defines inferred fixed MIME values, describes default-port
canonicalization accurately, records this candidate, and exposes the
driver-enabled Tokio `# Panics` precondition rather than promising typed driver
detection. The replacement boundary removes libc DNS in favor of bounded owned
Tokio socket queries, reuses one outer machine-god invocation-deadline sleep
rather than allocating another outer sleep per wait, retains the permit through
render/final validation, and reuses one process-wide Rustls root configuration.
Subordinate connection-attempt timer accounting was incomplete at that
checkpoint and is corrected in cycle 5 below. At that checkpoint,
documentation composition, a replacement SHA/tree, the complete local gate,
and three fresh same-SHA review tracks remained pending. Their later exact
results are recorded below.

## Cycle-1 remediation and replacement gate

Isolated production implementation commit
`bc4c06806685cdd7cf25015f364500f86d2554e7`, tree
`fcb6d561620539351912308bcf820dc7835013f4`, implements the corrected host
validation, outer lifetime owner, direct DNS, and cached TLS boundaries. It
rejects `.alt` and trailing-dot numeric hosts; holds one permit, cancellation
waiter, and deadline sleep through rendering and final validation; replaces
libc resolution with bounded direct A/AAAA socket exchanges; reuses one cached
root-parsed Rustls configuration; and documents the Tokio driver `# Panics`
precondition in the public API. Synchronous query-ID entropy runs under the
held permit and spawns or detaches no work. Direct DNS retains a one-byte UDP
overflow witness, preallocation TCP length rejection, and rejection of a TCP
answer that remains truncated.

Production tip `0c8c76935a6e3ca392e58b2aa9c375f88221f41f`, tree
`d96c13c853424325a688631dfea25c504bb62250`, adds a private
permit-through-render/final-boundary proof without changing public behavior.

Independent evidence tip `c3dc6a00da22738b6840fc2bc66840dc735eee6f`,
tree `558140e5ac31f6f8f2cd7d15064681b53e7fd39b`, is parented directly by
`0c8c76935a6e3ca392e58b2aa9c375f88221f41f`. It adds the independently owned
maximum-result, active-cap, queue, deadline, success/error/cancellation/drop,
render-lifetime, and runtime-boundary matrix. Exact composed focused evidence
is green: 14/14 direct, 11/11 HTTP lifecycle, 5/5 engine, and 17/17 private
tests, plus formatting, native all-target/all-feature warnings-denied Clippy,
clean diff, and clean status.

The components and documentation correction are composed through `8006846`. A
first portability correction at `2be69c8` excluded `rcgen` from WASI, but the
expanded baseline cross-target gate then proved that the non-WASM
dev-dependency still pulled `aws-lc-sys` into Linux and FreeBSD no-feature
all-target builds. Exact correction
`5a7960f6e728bf5681e91a411710b4c24dbd6991`, tree
`f1ed559f0328b8eda721b7b28bcb6fcdb95367b2`, removes `rcgen` and its seven
lock-only packages. A deterministic P-256 `example.com` DER certificate and
PKCS#8 key, valid through 2126, keep the production Rustls verifier, root,
address pinning, SNI, hostname, and fixed-header test path intact.

That exact code-and-test precursor passed the complete replacement local gate
with Rust/Cargo 1.94.1 and no fallback. The four repository-required commands,
the complete 976-test all-target/all-feature run, 881-test default inventory,
and two doctests passed. Focused evidence is 17 private, 14 direct, 11 HTTP
lifecycle, and five engine tests. The fresh release CLI is byte-identical at
319,152 bytes with SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`; bare,
version, help, inert human status, and inert JSON status passed.

Two consecutive CI-style Python runs each passed 130 tests with eight expected
macOS skips. Pinned-fx compatibility regeneration passed at exact upstream
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Exact cargo-deny 0.20.2 passed
with three allowed duplicate warnings. Cached cargo-audit 0.22.2 passed against
1,225 advisories and 209 dependencies. All 74 Markdown files, 99 fence blocks,
527 links, and 378 relative targets passed; unsafe Rust remains absent and
forbidden. The exact 21-file diff is clean and changes no CLI source, workflow,
benchmark, generated compatibility inventory, or compatibility status.

Linux and FreeBSD workspace all-target no-feature checks and native-library
warnings-denied Clippy passed. WASI all-target check and test compilation passed
with and without all features; its dependency trees exclude `rcgen`, `time`,
and `yasna`. Node 22.22.0 actively ran the all-feature unsupported-target test
1/1. Established cfg-only cross-target warnings remain outside the
warnings-denied library gates. Native Linux HTTP execution remains an exact-CI
requirement because this macOS cross-host lacks the target C sysroot.

The tree-identical formal cycle-2 candidate and review outcome are recorded
below. The precursor's green local gate does not override that later rejection.

## Formal cycle 2 — not green

Three fresh agents independently inspected exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`, in isolated detached worktrees.
Any finding rejects the candidate, so cycle 2 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 0 medium, 2 low**.

- **Low — stale candidate state:** maintained current-state passages still said
  that exact-tree checks, an immutable cycle-2 marker, and formal replacement
  review were pending after this candidate already existed and the reviews had
  begun.
- **Low — missing-MIME prefix mismatch:** the normative contract said only a
  bounded prefix was classified, while implementation applies model-safe UTF-8
  classification to the complete bounded response body.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-2 candidate.

### Performance and concurrency

Counts: **0 blocker, 0 high, 2 medium, 1 low**.

- **Medium — DNS header-count capacity amplification:** the 4 KiB DNS wire cap
  did not prevent untrusted header counts from influencing decoder allocation
  capacity before a raw-record-count bound was enforced.
- **Medium — synchronous resolver configuration inside the total deadline:**
  each hostname invocation performed a blocking system resolver-configuration
  read after starting its advertised absolute deadline. That synchronous work
  could neither be interrupted nor made subject to the Tokio deadline.
- **Low — stale candidate state:** the same maintained current-state passages
  did not name the exact already-existing cycle-2 candidate and review.

### Consolidated union and disposition

After deduplicating the repeated stale-state finding, cycle 2 has **0 blocker,
0 high, 2 medium, and 2 low** findings. Exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9` is rejected and must never be used
as delivery evidence.

Exact isolated production remediation component
`6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, changes only
`crates/machine-god-native/src/web_fetch.rs`. It snapshots and retains the
system-configuration result synchronously at production-transport construction,
outside invocation timing and without Tokio. Hostname execution uses the stored
first UDP nameserver or the same fixed, retryable unavailable result until
transport reconstruction; an admitted public IP literal needs no nameserver and
bypasses snapshot failure. Missing-MIME
classification covers the complete bounded body. The raw DNS predecode
contract requires a 12-byte header, exactly one question, at most 39 answers,
at most 128 authority and additional records individually, at most 128
aggregate resource records, and the checked count-implied minimum payload
length before either UDP or TCP decoding. The 39 answers cover 32 admitted
addresses plus seven CNAME links. A TCP advertised frame must be 12 through
4,096 bytes before allocation. The isolated production component alone makes
no replacement-gate or green-review claim.

## Cycle-2 remediation replacement gate

Exact composed remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The four
repository-required commands passed, as did the complete 976-test all-target/
all-feature workspace run. Focused evidence passed 21 private, 14 direct, 11
production HTTP lifecycle, and five engine tests.

The first CI-style Python discovery run overlapped the root all-feature Cargo
compilation and completed 130 tests with eight expected macOS skips and two
failures in 63.198 seconds. Both failures returned the fixed 2.0-second command-
timeout diagnostic instead of the expected diagnostic:
`test_schema_one_git_output_overflow_cleans_up_publication` and
`test_schema_one_repository_head_rejects_git_path_replacement`. After that
contention ended, two consecutive complete replacement runs passed 130 tests
with eight expected macOS skips in 60.041 and 39.346 seconds. Exact pinned-fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility regeneration with
`--check` passed.

Dependency policy passed under `cargo-deny` 0.20.2 with three allowed duplicate
warnings. Cached `cargo-audit` 0.22.2 `--no-fetch` passed against 1,225
advisories and all 209 locked packages. The `Cargo.lock` delta from the
delivered base adds 34 packages and removes none; `rcgen`, `time`, and `yasna`
are absent. The WASI dependency trees exclude those packages and `aws-lc-sys`.

Linux, FreeBSD, and WASI portability gates passed. Each WASI no-run variant
produced 61 executable artifacts, and Node 22.22.0 actively passed the retained
unsupported-target test. The two established FreeBSD cfg-only warnings and six
established cfg-only warnings per WASI variant remain outside the warnings-
denied library gates. All 74 Markdown files, 99 fence blocks, 527 inline links,
and 378 relative targets passed with zero errors.

The exact 21-file diff is clean. CLI source, workflows, benchmark workloads,
and generated compatibility data are byte-identical to the delivered base.
Changed Rust adds no unsafe code; the nine existing unsafe constructs remain
only in an unchanged excluded ADR fixture. The locked release build produced a
fresh 319,152-byte arm64 Mach-O binary with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`.
Bare, version, help, inert human-status, and inert JSON-status exercises all
exited zero with exact stdout plus LF, empty stderr, and output sizes 33, 33,
289, 140, and 192 bytes respectively.

This gate record makes no formal-review outcome, workflow, integration, or
delivery claim; formal candidates are identified only by exact-SHA review
results.

## Formal cycle 3 — not green

Three fresh agents independently inspected exact candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 3 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 1 medium, 1 low**.

- **Medium — blocking per-query entropy inside the total deadline:** hostname
  execution synchronously obtained a fresh random DNS query ID after the
  invocation's absolute deadline began. The blocking platform call had no
  await boundary at which the Tokio deadline could interrupt it, so the
  advertised total bound was not authoritative over all invocation work.
- **Low — duplicated cancellation waiter:** the outer bounded invocation and
  native transport each constructed a waiter for the same cancellation token.
  This contradicted the maintained one-waiter boundary and performed repeated
  per-invocation cancellation-future setup.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 1 medium, 1 low**.

- **Medium — missing authority at native pre-effect phase boundaries:** the
  outer race could drop pending native work, but the native sequence did not
  itself recheck cancellation and the same absolute deadline before advancing
  from completed A work to AAAA, into truncated-answer TCP replay, into HTTP
  dispatch, or into later response-body reads. When one phase completed
  immediately, a cancellation or deadline that became authoritative between
  effects could lose the next-effect boundary.
- **Low — duplicated cancellation waiter:** this track independently reported
  the same second cancellation future created inside native execution.

### Performance and concurrency

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-3 candidate.

### Consolidated union and disposition

After deduplicating the repeated waiter finding, cycle 3 has **0 blocker,
0 high, 2 medium, and 1 low** findings. Exact candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`, is rejected and must never be used
as delivery evidence.

The corrected boundary snapshots one random query-ID seed synchronously during
production-transport construction, outside invocation timing. An atomic per-
query sequence derives each A/AAAA ID without a blocking per-query entropy
call. A failed seed or resolver snapshot leaves hostname execution at the same
fixed, retryable unavailable result until reconstruction, while an admitted
literal IP bypasses those hostname-only prerequisites. The outer bounded
invocation owns one outer cancellation waiter and one reusable outer invocation-
deadline sleep for permit/DNS/HTTP/body waits. Subordinate connection-attempt
timer accounting is corrected in cycle 5 below. Native execution receives the
same token and absolute deadline and checks both before A, AAAA, TCP replay,
HTTP dispatch, and response-body effects,
including immediately completing phase transitions. The final synchronous
boundary checks both directly and creates no second waiter.

Exact isolated production remediation component
`9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only
`crates/machine-god-native/src/web_fetch.rs`. Its `QueryIdSequence` holds the
construction-snapshotted 32-byte key and an `AtomicU32`; each bounded SHA-256
derivation hashes that key with one counter value and selects the first 16
digest bits. The private carried deadline is excluded from canonical request
identity and debug output. The shared native effect helper checks cancellation
and that deadline immediately before and after every awaited native effect,
while only the bounded permit/DNS/HTTP/body wait owner holds a cancellation
future. The final synchronous boundary directly checks state without one.

Exact isolated independent-evidence commit
`3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on a tree-identical
integration of production component `9abef298` and changes only
`crates/machine-god-native/tests/web_fetch_http.rs`. Its exact 13/13 focused
checks require exactly one cancellation wake followed by a cancelled result
and pending owned-work drop/release through both the bounded and raw transport
seams. They use neither sleeps nor network operations.

This remediation record makes no replacement-gate, formal-review, workflow,
integration, delivery, compatibility, or product-performance claim; formal
candidates are identified only by exact-SHA review results.

## Cycle-3 remediation replacement gate

Exact composed remediation precursor
`78e6f4dcb4d49fd8ccf112e64350b745f622ca7f`, tree
`1fc16e8f7792c3001ba5f4b4a0c112778d2cf30c`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The four
repository-required commands and the complete all-target/all-feature workspace
run passed. Focused evidence passed 26 private, 14 direct, 13 production HTTP
lifecycle, and five engine tests.

The CI-style Python discovery run passed 130 tests with eight expected macOS
skips in 40.358 seconds. Compatibility regeneration with `--check` passed
against clean pinned fx revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Dependency policy passed under
`cargo-deny` 0.20.2 with three allowed duplicate warnings. Cached
`cargo-audit` 0.22.2 `--no-fetch` checked 1,225 advisories and 209 locked
dependencies with zero vulnerabilities.

Linux and FreeBSD baseline libraries plus all 53 integration targets
type-checked. FreeBSD emitted only its two established cfg-only warnings. Each
WASI no-run variant produced 61 executable artifacts and only its six
established cfg-only warnings. Node 22.22.0 actively passed the retained WASI
unsupported-target test 1/1. Documentation integrity covered 74 Markdown
files, 99 fence blocks, 530 inline links, and 378 relative targets with zero
errors.

At the code/evidence precursor, the exact whole-feature diff from the delivered
base covered 21 files with 6,490 insertions and 21 deletions. The cycle-3
production/test delta was 523 insertions and 102 deletions. CLI source,
workflows, benchmark workloads, and generated compatibility data were
unchanged, and changed Rust added no unsafe code. A fresh root release build
produced a 319,152-byte arm64 Mach-O binary with SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`;
bare, version, help, inert human-status, and inert JSON-status smokes passed.

This replacement gate makes no formal-review, workflow, integration, delivery,
performance, or fx-equivalence claim. Formal reviewer reports identify the
exact candidate they reviewed.

## Formal cycle 4 — not green

Three fresh agents independently inspected exact candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 4 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 1 medium, 2 low**.

- **Medium — DNS TCP replay omits the configured connect timeout:** a
  truncated UDP answer could enter `TcpStream::connect` under cancellation and
  the 60-second overall deadline, but not the advertised configured 10-second
  connect bound. A stalled resolver connection could therefore consume the
  remaining invocation budget instead of the smaller connect budget.
- **Low — incomplete custom-host contract:** maintained composition wording
  described the production host as thirteen tools without stating that the
  explicitly injected/custom transport constructors reach the same shared
  composition. Every such path has thirteen alphabetical tools; exactly twelve
  workspace-backed tools use one original retained descriptor plus eleven
  clones, and rootless `web_fetch` uses none.
- **Low — stale current-candidate state:** maintained current/operative prose
  said a new exact candidate remained pending after exact cycle-4 candidate
  `af043dc860ab88941df1385543a92c3d9880beed` already existed and the reviews
  were running.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-4 candidate.

### Performance and concurrency

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — repeated destinations amplify connection attempts:** the combined
  A/AAAA vector preserved duplicate public addresses before constructing the
  pinned Reqwest client. Overlapping or repeated DNS answers could therefore
  create repeated connection attempts instead of one stable first-seen
  destination sequence.

### Consolidated union and disposition

Cycle 4 has **0 blocker, 0 high, 2 medium, and 2 low** deduplicated findings.
Exact candidate `af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`, is rejected and must never be used
as delivery evidence.

The replacement contract stably deduplicates fully validated admitted
addresses in first-seen A-then-AAAA order before HTTP-client construction. A
truncated-DNS TCP connect uses the configured connect timeout subordinate to
cancellation and any earlier overall deadline. Maintained host documentation
states the shared production and explicitly injected/custom composition shape,
and time-sensitive pending-candidate wording is removed.

Exact isolated production remediation component
`9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, changes only
`crates/machine-god-native/src/web_fetch.rs`. Its exact checks passed 29/29
private, 14/14 direct, 13/13 production HTTP, and 5/5 engine tests, plus native
all-target/all-feature tests, formatting, and warnings-denied Clippy. It
implements stable first-seen destination deduplication and makes configured
connect timeout, cancellation, and any earlier overall deadline authoritative
over truncated-DNS TCP connect.

Exact isolated independent-evidence commit
`408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, changes only
`crates/machine-god-native/tests/web_fetch_http.rs`. Its deterministic
same-poll authority regression brings the HTTP suite to 14/14 with formatting
and warnings-denied Clippy green. This injected seam does not establish the
native DNS-specific corrections; their proof remains in production's private
tests.

Exact composed code/evidence precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`, contains both components. This
remediation record makes no replacement-gate, formal-review outcome,
candidate, workflow, integration, delivery, compatibility, or product-
performance claim; formal reviewer reports identify the exact candidate they
reviewed.

## Cycle-4 remediation replacement gate

Exact composed remediation precursor
`892a52267e7ccf478e9ed567875dc95912be5412`, tree
`da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The four
repository-required commands below passed, as did the complete 991-test
all-target/all-feature workspace run. Focused evidence passed 29 private, 14
direct, 14 production HTTP, and five engine tests.

The CI-style Python run passed 130 tests with eight expected macOS skips in
39.386 seconds. Compatibility regeneration with `--check` passed against clean
pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`. Dependency
policy passed under exact `cargo-deny` 0.20.2 with the three allowed duplicate
warnings for `core-foundation`, `cpufeatures`, and `syn`. Exact `cargo-audit`
0.22.2 `--no-fetch` checked 1,225 cached advisories and 209 locked dependencies
with zero vulnerabilities.

Linux portability passed with zero warnings. FreeBSD passed with its two
established cfg-only warnings. Each WASI variant produced 61 executable
artifacts and only its six established cfg-only warnings. Node 22.22.0 actively
passed the retained unsupported-target test 1/1. Documentation integrity
covered 74 Markdown files, 99 fence blocks, 530 inline links, and 378 relative
targets with zero errors.

The exact whole-feature diff from delivered base covers 21 files with 7,641
insertions and 32 deletions. The cycle-4 delta covers 10 files with 728
insertions and 71 deletions. CLI source, workflows, benchmark workloads, and
generated compatibility data are byte-unchanged, and changed Rust adds no
unsafe construct. A locked isolated release build produced a 319,152-byte
binary with SHA-256
`3ac3557269798c42fefaa39fd44d0f7fd7374fbe64da7c3afe3b029cdc87dcf1`;
bare, version, help, inert human-status, and inert JSON-status smokes all passed.

This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.

## Formal cycle 5 — not green

Three fresh agents independently inspected exact candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 5 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 0 medium, 1 low**.

- **Low — incomplete timer inventory:** maintained claims described one
  invocation deadline sleep without disclosing the additional configured
  connect-timeout sleep owned by each truncated-DNS TCP replay or the bounded
  connection-attempt timers Reqwest/Hyper may own.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — same-poll DNS TCP-connect deadline escape:**
  `await_native_connect_with_waiter` polled its configured connect-timeout
  sleep before the TCP connect future, but a ready connect result rechecked
  only cancellation and the outer invocation deadline. If the absolute connect
  deadline became due during that same effect poll, a late success could be
  accepted and a late error could later map as unavailable rather than timeout.

### Performance and concurrency

Counts: **0 blocker, 0 high, 0 medium, 1 low**.

- **Low — incomplete timer inventory:** this is the same maintained-
  documentation finding reported by correctness/API, not a second unique
  defect.

### Consolidated union and disposition

Cycle 5 has **0 blocker, 0 high, 1 medium, and 1 low** deduplicated findings.
Exact candidate `81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`, is rejected and must never be used
as delivery evidence.

The replacement must retain an absolute configured connect deadline and, after
a ready connect poll, apply cancellation and outer-deadline precedence before
rejecting an expired connect deadline and accepting either success or error.
The maintained timer inventory is exactly one reusable outer machine-god
invocation-deadline sleep; one additional short-lived configured connect-
timeout sleep for each truncated A or AAAA DNS TCP replay, at most two
sequential DNS replay sleeps per invocation; and any bounded Reqwest/Hyper HTTP
connection-attempt timers. The outer sleep is allocated once; each DNS replay
sleep is allocated once when that replay begins. None resets or extends the
outer absolute deadline.

Exact isolated source remediation
`cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only
`crates/machine-god-native/src/web_fetch.rs`. It retains one absolute connect
deadline and, after a ready effect result, applies cancellation and outer-
deadline precedence before rejecting an expired connect deadline and accepting
either success or error. Its focused checks passed 30/30 private, 14/14 direct,
14/14 production HTTP, and 5/5 engine tests, native all-target/all-feature
tests, formatting, and warnings-denied Clippy. Exact composed code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, compatibility, or product-performance claim;
formal reviewers identify only the exact candidate they inspect.

## Cycle-5 remediation replacement gate

Exact composed remediation precursor
`8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
`5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four repository-required commands below
passed, as did the complete 992-test all-target/all-feature workspace run.
Focused evidence passed 30 private, 14 direct, 14 production HTTP, and five
engine tests.

The CI-style Python run passed 130 tests with eight expected macOS skips in
84.454 seconds. Compatibility regeneration with `--check` passed against clean
pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`. Dependency
policy passed under exact `cargo-deny` 0.20.2 with the three established
duplicate warnings for `core-foundation`, `cpufeatures`, and `syn`. Exact
`cargo-audit` 0.22.2 `--no-fetch` checked 1,225 cached advisories and 209 locked
dependencies with zero vulnerabilities.

Linux portability passed with zero warnings. FreeBSD passed with its two
established cfg-only warnings. The default and all-feature WASI variants each
produced 61 executable artifacts and only their six established cfg
diagnostics. Node 22.22.0 actively passed the retained unsupported-target test
1/1. Documentation integrity covered 74 Markdown files, 99 fence blocks, 530
inline links, and 378 relative targets with zero errors.

The exact whole-feature diff from delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` covers 21 files with 8,233
insertions and 32 deletions. The cycle-5 replacement delta from rejected
candidate `81b963ad5a2033fb2295f7325a28fba6b66197d5` covers nine files with 542
insertions and 42 deletions. CLI source, workflows, benchmark workloads,
generated compatibility data, Cargo manifests, and the lockfile are unchanged
in that delta, and changed Rust adds no unsafe construct. A locked isolated
release build produced a 319,152-byte arm64 Mach-O binary with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`. Bare,
version, help, inert missing-path human-status, and inert missing-path JSON-
status smokes all passed with empty stderr; the XDG missing roots were not
created.

This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.

## Formal cycle 6 — not green

Three fresh agents independently inspected exact candidate
`5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 6 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — direct dependency does not cover the exported target surface:**
  `web-fetch-http` and `web_fetch.rs` are exported and documented for every
  non-WASM target, and native DNS query-ID derivation uses `sha2`
  unconditionally. The direct dependency covered only Linux and macOS, leaving
  FreeBSD and Windows without it. It also accidentally covered Rust 1.94.1's
  `wasm32-wali-linux-musl` target because that WASM-family target reports Linux
  as its target OS. The macOS cross-host's `aws-lc-sys` C-sysroot failure
  stopped compilation before this missing Rust dependency could be exposed.

Correctness reran the 30-private/14-direct/14-production-HTTP/5-engine focused
evidence and inventoried 992 all-target/all-feature tests on the exact
candidate.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-6 candidate.

### Performance and concurrency

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-6 candidate.

### Consolidated union and disposition

Cycle 6 has **0 blocker, 0 high, 1 medium, and 0 low** deduplicated findings.
Exact candidate `5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`, is rejected and must never be used
as delivery evidence.

The replacement must put the one direct `sha2` dependency in the existing
`cfg(not(target_family = "wasm"))` table shared by the `web-fetch-http`
dependencies. Manifest evidence must parse the TOML and reject aliases,
duplicates or overlapping placements, optional/version drift, and target-scope
drift.

Exact source remediation `e6eba937423714bc96a7c399b4823c6deda22da7`, tree
`bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, moves the direct `sha2` entry into
that non-WASM table. The stdlib-`tomllib` regression in
`tests/test_native_manifest.py` parses the manifest and rejects a `sha2` alias,
duplicate or overlapping placement, an optional or versioned specification,
and scope drift. The normative behavior document records the corrected target
boundary without changing the feature's non-WASM contract.

Exact Rust and Cargo 1.94.1 source checks passed: 30/30 private, 14/14 direct,
14/14 production HTTP, and 5/5 engine tests; formatting; full warnings-denied
Clippy; workspace tests and doctests; and 131 Python tests with eight expected
macOS skips. Direct Linux and FreeBSD dependency-tree probes include native
`sha2` and HTTP edges, while the WALI tree includes neither. Diff checks are
clean.

This remediation record makes no replacement-gate, formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim.

## Cycle-6 remediation replacement gate

Exact remediation precursor
`31e7f11870a86795f9d98831d8405714b49b989e`, tree
`68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four repository-required commands below
passed, as did the complete 992-test all-target/all-feature workspace run.
Focused evidence passed 30 private, 14 direct, 14 production HTTP, and five
engine tests.

The CI-style Python run passed 131 tests, including the parsed-manifest
regression, with eight expected macOS skips in 44.974 seconds. Compatibility
regeneration with `--check` passed against clean pinned fx revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Dependency policy passed under
exact `cargo-deny` 0.20.2 with the three established duplicate warnings for
`core-foundation`, `cpufeatures`, and `syn`. Exact `cargo-audit` 0.22.2
`--no-fetch` checked 1,225 cached advisories and 209 locked dependencies with
zero vulnerabilities.

Linux passed with zero warnings. The standard FreeBSD check passed with its two
established cfg-only warnings. The macOS cross-host stopped at the
`aws-lc-sys` foreign C-sysroot boundary, so this record makes no FreeBSD HTTP-
compilation claim beyond that point. Each WASI variant produced 61 executable
artifacts and only its six established warnings. Node 22.22.0 actively passed
the retained unsupported-target test 1/1. Target-matrix dependency trees
contain direct native HTTP and `sha2` edges for Linux, macOS, FreeBSD, and
Windows. They omit both edges for WASI Preview 1, WASI Preview 2, wasm64, and
WALI. `Cargo.lock` is the identical blob
`a798f7aefef7e9e043c0ed96ea205c35d0ed6b63`, containing exactly one `sha2`
0.10.9 and 209 packages.

Documentation integrity covered 74 Markdown files, 99 fence blocks, 530
inline links, and 378 relative targets with zero errors. The exact whole-
feature diff from delivered base `a56ff350c2aace1dc22cb14c269aee89d399cd8e`
covers 22 files with 8,558 insertions and 35 deletions. The cycle-6 replacement
delta from rejected candidate `5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`
covers five files with 235 insertions and nine deletions. CLI source, workflows,
benchmark workloads, generated compatibility data, the root Cargo manifest,
and `Cargo.lock` are unchanged in that delta. The native Cargo manifest changes
intentionally, and changed Rust adds no unsafe construct.

A locked isolated release build produced a 319,152-byte arm64 Mach-O binary
with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`. Bare,
version, help, inert missing-path human-status, and inert missing-path JSON-
status smokes all passed with empty stderr. Human and JSON status checks against
missing XDG roots created neither root.

This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.

## Formal cycle 7 — not green

Three fresh agents independently inspected exact candidate
`47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 7 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — `.arpa` infrastructure names bypass lexical admission:** the
  bounded contract made special-use names ineligible, but the lexical denylist
  omitted `.arpa`. Names such as `ipv4only.arpa`, `resolver.arpa`, and reverse-
  name descendants such as `10.in-addr.arpa` could therefore prepare a network
  capability before the DNS answer-set boundary ran.

Correctness reran the exact candidate's 30-private/14-direct/14-production-
HTTP/5-engine focused suites plus the parsed-manifest test 1/1. The exact
target-matrix evidence was also green.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-7 candidate; its focused transport, cancellation, deadline, and
lifecycle evidence reported no finding.

### Performance and concurrency

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-7 candidate; its deterministic admission, saturation, and stress
evidence reported no finding.

### Consolidated union and disposition

Cycle 7 has **0 blocker, 0 high, 1 medium, and 0 low** deduplicated findings.
Exact candidate `47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`, is rejected and must never be used
as delivery evidence.

The candidate is the tree-identical formal freeze following the exact cycle-6
replacement-gate lineage. Reviewers reported no lineage finding. The absence
of a separate pre-review seal is protocol-correct.

The replacement must perform an allocation-free ASCII-case-insensitive exact
final-label suffix predicate, include `.arpa`, and preserve label boundaries so
ordinary example domains remain eligible. The `.arpa` terminal label subsumes
the older explicit `home.arpa` case. Preparation must reject the denied names
without consulting policy or transport.

Exact root remediation `0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
`66527a146dc87b1aa9f3b7ee99385efb8022a342`, implements the final-label
predicate and records the exact bounded behavior. Isolated remediation
`8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` has the same tree. The predicate
uses ASCII-case-insensitive equality without allocating, adds `.arpa`, and
retains existing terminal labels; `home.arpa` is subsumed. `example.com`,
`example.net`, `example.org`, and label-bounded names such as
`resolver.arpa.example.com` remain eligible.

Deterministic private, direct, and engine regressions prove `.arpa` and reverse-
name rejection before policy or transport, including zero transport calls and
polls. Exact Rust and Cargo 1.94.1 source checks passed 31/31 private, 15/15
direct, 14/14 production HTTP, and 6/6 engine tests; formatting; full workspace
warnings-denied Clippy; the complete native all-target/all-feature run; and
clean diff checks.

This remediation record makes no replacement-gate, formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim.

## Cycle-7 remediation replacement gate

Exact remediation precursor
`b365cf7704bacb378b280f25d702896dcc216e0c`, tree
`8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four repository-required commands below
passed, as did the complete 995-test all-target/all-feature workspace run.
Focused evidence passed 31 private, 15 direct, 14 production HTTP, and six
engine tests plus the parsed-manifest test 1/1.

The CI-style Python run passed 131 tests with eight expected macOS skips in
49.059 seconds. Compatibility regeneration with `--check` passed against clean
pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`. Dependency
policy passed under exact `cargo-deny` 0.20.2 with the three established
duplicate warnings for `core-foundation`, `cpufeatures`, and `syn`. Exact
`cargo-audit` 0.22.2 `--no-fetch` checked 1,225 cached advisories and 209 locked
dependencies with zero vulnerabilities.

The Linux baseline checked all 53 integration targets with zero warnings, and
warnings-denied library Clippy passed. The FreeBSD baseline and Clippy passed
with only its two established cfg-only warning groups. The macOS cross-host
stopped at the `aws-lc-sys` foreign C-sysroot boundary, so this record makes no
FreeBSD HTTP-compilation claim beyond that point. Each WASI variant produced
61 test executables and only its six established diagnostics. Node 22.22.0
actively passed the retained unsupported-target test 1/1.

The native dependency matrix has 14 direct edges on Linux, 15 on macOS, 14 on
FreeBSD, and 12 on Windows; every native tree includes `reqwest` and `sha2`.
WASI Preview 1, WASI Preview 2, wasm64, and WALI have 4, 4, 4, and 6 direct
edges respectively and include neither dependency. The 209-package lockfile
contains exactly one `sha2` 0.10.9 entry.

The exact allocation-free 11-entry final-label predicate passed `.arpa`
negative cases and example-domain positive cases. Effect-free engine evidence
proved rejection before policy or transport. All five official contract links
returned HTTP 200. Documentation integrity covered 74 Markdown files, 99 fence
blocks, 535 inline links, and 378 relative targets with zero errors.

The exact whole-feature diff from delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` covers 22 files with 8,999
insertions and 35 deletions. The cycle-7 replacement delta from rejected
candidate `47c6a2e7b209aec29f30fd7f7413f55ea039202f` covers six files with 342
insertions and 20 deletions. CLI source, workflows, benchmark workloads,
generated compatibility data, the root and native Cargo manifests, and
`Cargo.lock` are unchanged in that delta. Changed Rust adds no unsafe
construct.

A locked isolated release build produced a 319,152-byte arm64 Mach-O binary
with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`. Bare,
version, help, inert missing-path human-status, and inert missing-path JSON-
status smokes all passed with empty stderr. Human and JSON status checks against
missing XDG roots created neither root.

This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.

## Formal cycle 8 — not green

Three fresh agents independently inspected exact candidate
`be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`, in isolated clean worktrees. Any
finding rejects the candidate, so cycle 8 is **NOT GREEN**.

### Correctness and public API

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — valid partial truncated UDP replies fail before replay:** a valid
  `TC=1` reply whose resource-record wire tail was incomplete failed either the
  strict full-message decoder or its section-count-implied minimum-length check
  before the implementation reached bounded TCP replay.

### Network/HTTP lifecycle and robustness

Counts: **0 blocker, 0 high, 1 medium, 0 low**.

- **Medium — invalid truncated replies can construct replay before
  validation:** a structurally parseable `TC=1` reply with a wrong ID, QR bit,
  opcode, RCODE, question name or type, or class could construct bounded TCP
  replay before those response fields were validated.

### Performance and concurrency

Counts: **0 blocker, 0 high, 0 medium, 0 low**. This track is **GREEN** on the
exact cycle-8 candidate; its bounded admission, deadline, cancellation, and
deterministic stress evidence reported no finding.

Candidate-focused evidence passed 31/31 private, 15/15 direct, 14/14
production HTTP, and 6/6 engine tests plus the parsed-manifest test 1/1.

### Consolidated union and disposition

Cycle 8 has **0 blocker, 0 high, 2 medium, and 0 low** deduplicated findings.
Exact candidate `be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`, is rejected and must never be used
as delivery evidence.

The replacement must retain the raw bounded UDP message, validate the response
header and complete expected question before treating truncation as replay
authority, and accept a validated partial resource-record tail only for the
truncated-UDP path. Non-truncated UDP and all TCP responses must remain strict
full decodes, and a truncated TCP response must be rejected.

Exact root remediation `9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
`fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, implements that boundary. Exact
isolated remediation `38a36725559fe962e6f649e07cc5de154c940257` has the same
tree. UDP receive retains at most 4,096 raw bytes. Capped `Header::read` and
`Query::read` parsing validates the expected response tuple and question before
honoring `TC=1`. A validated truncated reply may ignore an incomplete resource-
record tail, cross a fresh cancellation/deadline authority boundary, and
lazily perform one bounded same-nameserver TCP replay. Non-truncated UDP and
all TCP responses still require strict full decoding; a still-truncated TCP
response is rejected.

Deterministic evidence proves one replay for a valid partial truncated reply
and zero replay for mismatched, malformed, cap-breaching, non-truncated, and
authority-boundary-rejected replies. It also proves rejection of a still-
truncated TCP response. Exact Rust and Cargo 1.94.1 source checks passed 34/34
private, 15/15 direct, 14/14 production HTTP, and 6/6 engine tests; formatting;
workspace all-target/all-feature warnings-denied Clippy; the complete native
all-target/all-feature run; workspace tests and doctests; a locked release
build and help smoke; and clean diff checks.

This remediation record makes no replacement-gate, formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim.

## Cycle-8 remediation replacement gate

Exact remediation precursor
`d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
`a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four repository-required commands below
passed, as did the complete 998-test all-target/all-feature workspace run.
Focused evidence passed 34 private, 15 direct, 14 production HTTP, and six
engine tests plus the parsed-manifest test 1/1.

The CI-style Python run passed 131 tests with eight expected macOS skips in
102.142 seconds. Compatibility regeneration with `--check` passed against
clean pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`.
Dependency policy passed under exact `cargo-deny` 0.20.2 with the three
established duplicate warnings for `core-foundation`, `cpufeatures`, and `syn`.
Exact `cargo-audit` 0.22.2 `--no-fetch` checked 1,225 cached advisories and 209
locked dependencies with zero vulnerabilities.

The Linux baseline passed with zero warnings, and warnings-denied library
Clippy passed. The FreeBSD baseline and Clippy passed with only its two
established cfg-only warning groups. The macOS cross-host stopped at the
`aws-lc-sys` foreign C-sysroot boundary, so this record makes no FreeBSD HTTP-
compilation claim beyond that point. Each WASI variant produced 61 test
executables and only its six established diagnostics. Node 22.22.0 actively
passed the retained unsupported-target test 1/1.

The native dependency matrix has 14 direct edges on Linux, 15 on Apple, 14 on
FreeBSD, and 12 on Windows; every native tree includes `reqwest` and `sha2`.
WASI Preview 1, WASI Preview 2, and wasm64 each have four direct edges, while
WALI has six; all four include neither dependency. The 209-package lockfile
contains exactly one `sha2` 0.10.9 entry.

Protocol-ordering evidence retains at most 4,096 raw UDP bytes and enforces the
wire/count caps. Capped `Header::read` plus `Query::read` validates the expected
response tuple and question before truncation replay. A valid partial `TC=1`
reply performs exactly one replay. All 13 invalid-response cases plus
cancellation/deadline authority-boundary failures perform zero replay. Non-
truncated UDP and TCP retain strict full decoding. All five official contract
links returned HTTP 200. Documentation integrity covered 74 Markdown files, 99
fence blocks, 535 inline links, and 378 relative targets with zero errors.

The exact whole-feature diff from delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` covers 22 files with 9,635
insertions and 35 deletions. The cycle-8 replacement delta from rejected
candidate `be418af26317da8fa5de77c45c926311475d7ff6` covers four files with 561
insertions and 54 deletions. Cargo manifests, `Cargo.lock`, CLI source,
workflows, benchmark workloads, and generated compatibility data are unchanged
in that delta. Changed Rust adds no unsafe construct.

A locked isolated release build produced a 319,152-byte arm64 Mach-O binary
with SHA-256
`869d871571ac6502ca5da7ebdbdbf26450870492234d8f226463bfa6aaf68051`. Bare,
version, help, inert unavailable-path human-status, and inert unavailable-path
JSON-status smokes all passed with empty stderr, final LFs, and exact stdout
sizes 33, 33, 289, 140, and 192 bytes. Separate human and JSON checks against
missing XDG roots created neither root.

This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.

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

Current state: formal cycles 1, 2, 3, 4, 5, 6, 7, and 8 are **NOT GREEN** on
their exact recorded SHA/tree pairs. Exact cycle-8 candidate
`be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`, is rejected with a deduplicated
union of 0 blocker, 0 high, 2 medium, and 0 low. Exact root remediation
`9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
`fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, and tree-identical isolated
remediation `38a36725559fe962e6f649e07cc5de154c940257` validate the bounded leading
UDP response tuple before truncated replay while preserving strict non-
truncated and TCP decoding. This remediation record makes no replacement-gate,
formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim. Exact cycle-8 remediation precursor
`d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
`a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed. Exact cycle-7 candidate
`47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`, is rejected with a deduplicated
union of 0 blocker, 0 high, 1 medium, and 0 low. Exact root remediation
`0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
`66527a146dc87b1aa9f3b7ee99385efb8022a342`, and tree-identical isolated
remediation `8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` reject `.arpa`
infrastructure names before policy or transport. This remediation record makes
no replacement-gate, formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim. Exact cycle-7 remediation
precursor `b365cf7704bacb378b280f25d702896dcc216e0c`, tree
`8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed. Exact cycle-6 candidate
`5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`, is rejected with a deduplicated
union of 0 blocker, 0 high, 1 medium, and 0 low. Exact source remediation
`e6eba937423714bc96a7c399b4823c6deda22da7`, tree
`bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, corrects the target dependency
scope and adds parsed-manifest evidence. This remediation record makes no
replacement-gate, formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim. Exact cycle-6 remediation
precursor `31e7f11870a86795f9d98831d8405714b49b989e`, tree
`68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed. Exact cycle-5 candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`, is rejected with a deduplicated
union of 0 blocker, 0 high, 1 medium, and 1 low. Exact isolated source
remediation `cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
`web_fetch.rs` and composes at exact code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` with the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim. Exact
composed cycle-5 remediation precursor
`8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
`5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed. Exact
cycle-2 remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passed its complete replacement
local gate before formal cycle 3. Exact cycle-3 candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`, is rejected with a deduplicated
union of 0 blocker, 0 high, 2 medium, and 1 low. Exact isolated
production remediation component
`9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only native
`web_fetch.rs`. Exact isolated independent-evidence commit
`3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
only native `web_fetch_http.rs`. Exact composed remediation precursor
`78e6f4dcb4d49fd8ccf112e64350b745f622ca7f`, tree
`1fc16e8f7792c3001ba5f4b4a0c112778d2cf30c`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate makes
no formal-review, workflow, integration, delivery, performance, or fx-
equivalence claim. Exact cycle-4 candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`, is rejected with a deduplicated
union of 0 blocker, 0 high, 2 medium, and 2 low. Exact isolated production
remediation component `9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, and isolated independent-
evidence commit `408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, compose through exact precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`. This remediation record makes no
replacement-gate, formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim; formal reviewer reports
identify the exact candidate they reviewed.
Exact composed cycle-4 remediation precursor
`892a52267e7ccf478e9ed567875dc95912be5412`, tree
`da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim; reviewer reports identify the exact
candidate they reviewed.
