# Milestone 03 native `web_fetch` review 01

Status: **IN PROGRESS — CYCLE 1 NOT GREEN**

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
`1378b02e92973ab15fbf4623138a643b70057f33`. This record establishes no green
review, workflow, integration, delivery, compatibility, or product-performance
claim. Milestone 03 remains in progress with twenty-six delivered slices.

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
Every one of at most 32 DNS answers must be public and the accepted set must be
pinned to the connection. Defaults are eight active calls, a 10-second connect
bound, a 60-second total bound, an inclusive 24 KiB body bound, and a 56 KiB
serialized-result bound; active calls can never exceed 32. Cancellation and
drop release the response and permit and own no machine-god worker. One outer
bounded wrapper owns the permit, cancellation future, and reused deadline sleep
through transport, rendering, serialized-result validation, and the final
cancellation/deadline boundary.
Construction is runtime-independent. Polling production requires a current
host-owned Tokio runtime with I/O and time enabled; no current handle returns
`RuntimeRequired`, while a current driverless runtime violates the documented
`# Panics` precondition and may terminate a release process. One Tokio deadline
sleep is reused throughout each invocation.

After permit acquisition, hostname resolution reads one UDP nameserver from
host system resolver configuration. The invocation sends bounded rooted A then
AAAA queries on owned Tokio sockets, using TCP only once when a UDP response is
truncated. Query IDs use synchronous platform entropy under the permit. A
one-byte UDP overflow witness enforces the inclusive 4 KiB message cap; TCP
length is rejected before allocation above that cap, and a still-truncated TCP
response is invalid. Strict query tuple, Internet class, rooted CNAME-chain,
terminal owner, 32-address, and every-address-public checks apply. There is no
libc lookup, cache, retry, search suffix, resolver thread, or spawned resolver
task. The admitted set feeds a fresh pinned Reqwest client using a process-wide
cached Rustls root configuration and fixed HTTP/1.1 ALPN, so roots are not
reparsed per invocation.

Only 2xx and identity encoding succeed. Text, JSON, XML, JavaScript, bounded
raw HTML, missing-MIME sniffing, model-unsafe-text rejection, and metadata-only
binary results follow the normative contract. Every result starts with the
upstream-untrusted warning and includes query-redacted URL, status, normalized
effective MIME, content kind, and `cache_hit: false`. An absent declaration is
reported as inferred `text/plain` or `application/octet-stream`, not an
absence marker. Errors are fixed and redacted.

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

## Composed implementation and complete local gate

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
Tokio socket queries, reuses one invocation deadline sleep, retains the permit
through render/final validation, and reuses one process-wide Rustls root
configuration. Exact production and independent-evidence components are
recorded below; documentation composition, the replacement SHA/tree, its
complete local gate, and three fresh same-SHA review tracks remain pending.

## Cycle-1 remediation in progress

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

These components do not form a documentation-complete replacement candidate
and have not passed the complete replacement gate or formal rereview.
Integration with this documentation, the immutable replacement SHA/tree, its
complete gate, and three fresh same-SHA tracks remain pending.

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

Current state: the pre-review local gate was green, but formal cycle 1 is
**NOT GREEN** on the exact SHA/tree above. The isolated production component is
`0c8c76935a6e3ca392e58b2aa9c375f88221f41f`, tree
`d96c13c853424325a688631dfea25c504bb62250`; focused-green evidence tip is
`c3dc6a00da22738b6840fc2bc66840dc735eee6f`, tree
`558140e5ac31f6f8f2cd7d15064681b53e7fd39b`. Documentation composition and a
complete replacement local gate are pending. No replacement review has
started, and feature workflows, fast-forward integration, exact `main`
workflows, and delivery remain pending.
