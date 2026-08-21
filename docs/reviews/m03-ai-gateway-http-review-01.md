# Milestone 03 native AI Gateway HTTP transport review 01

Status: **FEATURE-BRANCH GREEN — final documentation seal and `main` pending**

## Reviewed lineage

- Base: `735682fc5d861971bb862c1cef6f55607ed19645`
- Atomic feature: `791a1c9041089ce8329326801f99047898dae745`
- Adversarial remediation: `5cef51936e8f636b77b93a36657bda9b77fd99c7`
- Feature-branch evidence: `be6c6371bd829202fec50604e99e5a336aa38ed4`
- Branch: `agent/m03-ai-gateway-http`
- Toolchain: Rust and Cargo 1.94.1 exactly
- Pinned reference: fx revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`

The integrated slice adds the optional native-only Reqwest/Rustls implementation of
the existing injected `AiGatewayTransport` boundary. It pins the production
origin and path, accepts only an explicit canonical numeric-loopback plaintext
test endpoint, receives its bearer credential directly from the trusted host,
and fixes redirect, proxy, decompression, cookie, retry, timeout, concurrency,
status, error-redaction, and response-chunk policy. The transport requires a
host-owned, driven Tokio runtime with I/O and time enabled. Core and custom
injected transports remain executor-neutral, and the default CLI does not gain
HTTP dependencies or network authority.

## Reviewed behavior

- The optional `ai-gateway-http` feature is confined to
  `machine-god-native` and cfg-gated off on WebAssembly. The default build does
  not include Reqwest, Tokio, Hyper, or Rustls.
- Production can reach only the pinned HTTPS endpoint. The test constructor
  accepts canonical numeric loopback, including an explicit `:80`, and rejects
  implicit ports, user information, queries, fragments, names, alternate IP
  spellings, and non-loopback destinations.
- A bounded explicit bearer token is attached only as `Authorization`; fixed
  error and debug representations do not reveal credentials, endpoints,
  dependency diagnostics, response bodies, or response headers.
- The exact POST body and codec headers are preserved. Redirects, ambient
  proxies, cookies, response decoding, referer, user agent, and
  `Expect: 100-continue` behavior are disabled by explicit client policy.
- Application, status, and backoff retries are disabled. Hyper may recover a
  reused stale idle connection only before any request byte is written. The
  contract permits at most one peer-visible request and no replay after any
  byte may have reached the peer.
- One total deadline covers capacity waiting, upload, response-head wait, and
  response streaming. A semaphore bounds active requests, each exposed chunk
  is bounded, and the transport retains at most one dependency byte frame
  while splitting it.
- Cancellation wins same-poll races and wakes capacity, response-head, and
  response-body waiters. Dropped or cancelled work releases owned futures,
  bodies, buffers, and permits; asynchronous connection teardown proceeds only
  while the host runtime remains driven.
- Construction has no runtime or network effect. Polling transport work outside
  a current Tokio runtime returns a fixed error. A runtime without I/O or time
  drivers violates the public precondition and may panic or terminate a
  `panic=abort` process, which rustdoc and normative documentation disclose.

## Parallel implementation

Production code, black-box tests, and normative documentation were developed by
agents with non-overlapping ownership and integrated on the feature branch.
Fresh correctness/concurrency, security/abuse, and API/documentation agents
reviewed exact commits without editing them. Accepted findings were fixed and
all three reviewers then independently returned green on the remediation SHA.

## Adversarial rounds

### Round 1 — `791a1c9041089ce8329326801f99047898dae745`

Accepted findings:

- **HIGH:** catching a missing Tokio-driver panic did not uphold the documented
  error contract in a release process built with `panic=abort`. The catch was
  removed. Public rustdoc and normative/security documents now require I/O and
  time drivers and explicitly disclose possible panic or process termination
  when that precondition is violated. The no-active-runtime path remains a
  fixed redacted error.
- **MEDIUM:** the original absolute no-retry wording contradicted Hyper's
  internal recovery when a reused pooled connection is found stale before the
  request starts. The contract now precisely disables application, status, and
  backoff retries while allowing only pre-request-byte stale-idle recovery,
  with at most one peer-visible dispatch and no replay after any byte.
- **MEDIUM:** URL normalization erased an explicit loopback `:80` before
  validation, contradicting the public endpoint contract. Validation now
  checks the original lexical authority as a canonical `SocketAddr`; explicit
  `:80` is accepted, while an omitted port and noncanonical spellings remain
  rejected.
- **LOW:** the CDLA license exception was global. It is now scoped only to the
  pinned `webpki-root-certs` crate.
- **LOW:** header evidence did not explicitly exclude all identity/ambient
  state headers. The wire test now proves user-agent, referer, and cookie
  absence in addition to the existing framing and encoding assertions.

### Round 2 — `5cef51936e8f636b77b93a36657bda9b77fd99c7`

All three independent reviewers reported **GREEN**. They confirmed the runtime
precondition and abort disclosure, precise retry and peer-visibility contract,
canonical loopback parsing including explicit port 80, scoped license policy,
credential and endpoint confinement, API compatibility, cancellation and drop
behavior, resource bounds, documentation consistency, and clean feature
isolation.

## Exact local checks

The following passed on the adversarially green behavior SHA with exact
Rust/Cargo 1.94.1:

- formatting;
- workspace/all-target/all-feature Clippy with warnings denied;
- default workspace tests and all-target/all-feature workspace tests;
- workspace documentation tests;
- 20 focused native HTTP integration tests, including raw-loopback wire,
  cancellation, drop, retry, malformed-body, and runtime regressions;
- the missing-runtime regression in the release profile;
- repo-wide Python discovery: 129 run, 121 passed and 8 expected platform
  skips;
- `cargo-deny` 0.20.2 dependency policy, with only the accepted duplicate
  dependency warnings;
- `cargo-audit` 0.22.2: 1,225 advisories checked across 174 lockfile
  dependencies with no vulnerability finding;
- `wasm32-wasip1` all-feature workspace compilation, with only the pre-existing
  unrelated `read_file::check_cancellation` dead-code warning;
- pinned upstream compatibility generation against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact release build and bare CLI smoke;
- default CLI dependency-tree exclusion of Reqwest, Tokio, Hyper, and Rustls;
  and
- `git diff --check` and a clean worktree.

The local release CLI remains 319,152 bytes. This is a local regression
observation only, not retained benchmark evidence or a product-performance
claim.

## Exact remote feature-branch runs

The exact feature-branch evidence SHA
`be6c6371bd829202fec50604e99e5a336aa38ed4` passed both required remote
workflows:

- [CI run 32534687349](https://github.com/distributedstatemachine/machine-god/actions/runs/32534687349)
  completed successfully, including quality/tests, dependency policy and
  vulnerability audit, and all four native target jobs; and
- [Benchmark evidence run 32534687326](https://github.com/distributedstatemachine/machine-god/actions/runs/32534687326)
  completed successfully for both pinned-upstream and bootstrap evidence jobs.

Benchmark-workflow success validates this repository's evidence path. It does
not promote the local observation above into a product-performance or
compatibility claim.

## Remaining gates and scope

The exact feature branch is green. The documentation-seal commit that records
that result and the eventual fast-forwarded `main` SHA must still pass their own
exact remote CI and benchmark-evidence workflows. Feature-branch green is not
the final delivery seal. The benchmark workflow uses Zig only to build the
pinned upstream fx comparison target; machine-god remains a Rust product.

Milestone 03 remains in progress. This slice does not add credential discovery,
provider/CLI wiring, permission prompting, durable native sessions, broader
configuration, the remaining native tools, a compatibility claim, or a
measured product-performance claim. No package or GitHub release is authorized.
