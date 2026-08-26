# Top-level `models` CLI contract

Status: replacement-gated cycle-3 submission, not yet review-green or delivered
as the twenty-ninth bounded Milestone 03 slice. Exact cycle-2 behavior candidate
`2ea9d94374c4dd18f43255af785ee31088126c56`, tree
`3a948b2950d870a9cabe479bc6c3889dd5a13a3b`, passed the complete replacement
gate but three fresh tracks rejected it with a deduplicated union of one high,
one medium, and one low finding. Rejected cycle-1 candidate
`6277aa3dc26f9c485707c667f63525a2138f316b`, tree
`b5e2445ed90df000255b51c2c989d71965db1d77`, had a deduplicated union of two
medium and six low findings. The provider-neutral core contract is component
`a6c6ff333176689b0c53bcf35070e9d59afd1b28`, the bounded native catalog is
component `7c966b23d75a880a23d49e1e6ba9780e512e84b8`, and the thin CLI composition
is present in local feature commit
`e84ed2a46b1ac5fe7428414375609af562c65105`. Checked-deadline and terminal-
precedence remediation is component
`52e9b7d74f3979f7f7f55387243e96bd78773fe3`. Independent native evidence is
present at `12263afa458e48f2963ae3d0e3db5cf219f8bdf6`; deadline remediation
`02c9f86619fbdc202f5065c41090415a179316cf` raises the current focused native
total to 36 tests split across 15 provider/parser, 15 loopback HTTP, and 6
credential cases. Signal/config/WASI remediation is
`d2890c34bc628dd9ad425f5921e3816bbe1f5eef`, and dependency-topology remediation
is `06c94087e91ec298877fbe981695d2638fa1db1e`. Cycle-2 remediation accepts
arbitrary-size ignored/defaulted JSON number tokens at `9cf8c741`, replaces
blocking default DNS and repairs no-runtime composition at `8187b12`, and
fails closed on unavailable system DNS configuration with no public fallback
at `499af85`. Pre-review gate attempt `c01139811685ae73031ed6f6cbd771e4ff636714`,
tree `4ac4e5bd67b98900159aec1772f75d6003ba1d70`, was rejected because synchronous
platform DNS discovery still occurred inside timed request polling. Bounded
eager snapshot remediation is `d9922ef1`, and per-runtime absolute-name/custom-
resolver hardening is `e5248b10`. Focused provider/parser, credential, HTTP,
private resolver, CLI unit, CLI integration, and manifest evidence is now
17/17, 6/6, 16/16, 6/6, 18/18, 23/23, and 4/4. Catalog HTTP directly activates
direct `hickory-proto`, `hickory-resolver`, and `sha2` edges; Hickory resolver
remains only for bounded system-configuration parsing and its Tokio integration
is disabled. The catalog still omits generation-only direct `bytes`,
`web-fetch-http`, and Tokio's signal backend. The complete cycle-3 replacement
gate passed for exact behavior candidate
`2cecc921e48396e81ab6f434007a7ec8e3e890b5`, tree
`8c0d235355582d92aaed6fcca7c1862982494e20`, under exact Rust and Cargo 1.94.1.
Three fresh cycle-3 reviews and remote CI remain pending; no candidate is
review-green. The pinned comparison input remains fx commit
`b1774fbf6c7602b503026f96f6e960e946c692ef`. This status makes no performance,
compatibility-promotion, workflow, integration, or delivery claim.

The current isolated DNS remediation replaces Hickory's request-polled resolver
with private bounded UDP/TCP exchange. It snapshots one fallible query-ID key at
transport construction, derives IDs deterministically from an atomic sequence,
and invokes no entropy source or detached resolver task during request polling.
It raises private resolver source evidence from 6 to 14 tests. This behavior
record by itself makes no replacement-gate or review outcome claim.

The slice adds one read-only top-level command:

```text
machine-god models
machine-god models --json
```

It lists the Vercel AI Gateway coding-agent model catalog. The design preserves
machine-god's existing ownership split: core owns only validated provider-
neutral available-model, access, result, error, and catalog-provider contracts;
native owns process credential acquisition and all Gateway parsing, sorting,
access/fallback, deadline, concurrency, and HTTP behavior; the CLI owns strict
parsing, thin composition, rendering/output bounds, and a current-thread Tokio
host. Nothing in this contract changes the engine's model-generation provider
or its prompt/session/tool orchestration.

## Grammar and parse-before-effects rule

The only new grammar is `models [--json]`. `--json` may occur at most once and
only in the second position. Repeated `--json`, any other flag or positional
argument, trailing arguments, and any non-Unicode argument are invalid. The
entire process argument vector is validated before configuration, environment,
clock, runtime, semaphore, credential, DNS, socket, or output effects.

An invalid invocation exits 2, writes no stdout, and uses the existing fixed
invalid-argument diagnostic with the new command in global usage:

```text
machine-god: invalid arguments
Usage: machine-god [help | --help | -h | --version | -V | models [--json] | permissions [--json] | status [--json]]
```

Both lines and the complete diagnostic end with one LF. Global help lists
`help`, `models`, `permissions`, and `status` in that order. The new row is
exactly:

```text
  models       List available models
```

No parse failure may be replaced by a configuration or credential error, even
when the environment or configuration is also invalid.

## Authority and composition sequence

A valid invocation performs this sequence exactly once unless a stated
terminal failure stops it earlier:

1. load the existing strict native configuration through
   `load_process_config()` exactly once;
2. validate that its closed provider, transport, and credential-source values
   select the built-in Gateway/environment combination;
3. snapshot and discover only `VERCEL_OIDC_TOKEN` and
   `AI_GATEWAY_API_KEY` through the existing native credential adapter;
4. synchronously snapshot bounded platform DNS configuration into the fixed
   native catalog client, then create a host-owned current-thread Tokio runtime
   with I/O and time enabled;
5. call the native provider through the provider-neutral core catalog trait
   with one cancellation token;
6. render and serialize the bounded result completely in the CLI; then
7. write the selected success or failure representation.

Configuration is read-only. A missing file or unavailable configuration
location uses the existing safe built-in schema-v3 configuration. A selected
invalid configuration fails before credential or transport access. Config is
not reloaded for the anonymous fallback.

The command does not inspect or create the state root or workspace, construct
the coding engine or generation provider, select or change the configured
generation model, create a prompt or permission request, open a session store,
read or write a session, persist a cache, or write configuration or any other
file. The catalog result is not product state and is not retained after output.

Production always uses the fixed endpoint below. It is not configurable through
configuration, a CLI argument, or a process environment variable. Deterministic
CLI unit tests may inject a catalog trait object, while native HTTP success is
tested through the explicit numeric-loopback constructor. The production and
release CLI exposes no fake-network or endpoint switch.

## Core provider-neutral boundary

`machine-god-core` owns a validated `AvailableModel`, a closed catalog-access
value, a catalog result, and one object-safe provider trait. Catalog failures
reuse core's existing redacted `ProviderError`. The trait has this public
operation:

```rust,ignore
fn list_models(
    &self,
    cancellation: CancellationToken,
) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>>;
```

The future is inert until polled. It yields an already validated, already
ordered complete result whose access says `Authenticated` or `PublicOnly`. Core's
`AvailableModel` owns only its validated model ID; Gateway type, tags, tier,
provider-rank, and release metadata do not cross the native boundary. The
result is bounded by the native provider before construction.

The core API contains no URL, credential, access request, deadline, clock,
environment-variable, Gateway-team, HTTP-header, Tokio, Reqwest, runtime,
sorting, fallback, or output-rendering policy. Core neither orchestrates calls
nor reads a clock. The CLI calls the trait once; all internal HTTP decisions
belong to the native implementation.

The implemented public core surface is `AvailableModel`, `InvalidModelId`,
`InvalidModelIdReason`, `ModelCatalog`, `ModelCatalogAccess`,
`PublicCatalogReason`, and `ModelCatalogProvider`. `AvailableModel::new`
validates one owned ID; `ModelCatalog::new` preserves its provider-supplied
order and exposes `models`, `access`, and `into_models`. The access result is
either `Authenticated` or `PublicOnly` with `NoCredential` or
`AuthenticatedCredentialRejected`. Core does not independently cap the vector;
the native implementation applies catalog bounds before constructing it.

The CLI constructs the native HTTP transport with the optional validated
credential and constructs the native provider with a matching access mode. The
provider chooses exactly one of these internal access paths:

- no credential: one `Public` call;
- valid credential: one `Authenticated` call; or
- an `Authenticated` call returning the native, status-derived 401/403 class,
  followed by exactly one `Public` call.

The native fallback uses the same absolute deadline and cancellation token. It
does not carry an authorization value or any other authenticated-only metadata.
The first response and its permit are dropped before the public call begins.
There is no fallback or retry for cancellation, timeout, capacity failure, 3xx,
429, other 4xx, 5xx, DNS/TLS/transport failure, invalid JSON, a semantic limit,
a duplicate ID, output overflow, or any other error. A public first request is
never retried. The core trait is not a retry interface.

Native parses, bounds, validates, rejects duplicates, and sorts the Gateway result,
then constructs validated core `AvailableModel` values and records the final
access in the core result. The CLI derives presentation fields from that final
access and renders them. Output construction is not part of core or native.

## Credential selection and authentication

Credential discovery retains the exact existing native contract:

1. the first nonempty `VERCEL_OIDC_TOKEN`;
2. otherwise the first nonempty `AI_GATEWAY_API_KEY`;
3. otherwise no credential.

Unset and exactly empty values are absent. Empty OIDC falls through to the API
key. A selected nonempty value must be Unicode and a valid 1–4,096-byte RFC
6750 `b64token`. A selected non-Unicode, malformed, or oversized value fails
closed and does not fall through to a lower-priority value or make an anonymous
request. Credential failures retain and display no source name or value.

A request is authenticated only by:

```text
Authorization: Bearer <validated-token>
```

Machine-god has no team-selection source in this slice. It sends no team query
parameter and no `x-vercel-ai-gateway-team` header. Anonymous fallback strips
the complete authorization header; it does not reuse an authenticated request
object whose header is merely overwritten.

The native credential module publicly exposes
`DiscoveredAiGatewayCatalogCredential`,
`discover_ai_gateway_catalog_credential`, and
`discover_process_ai_gateway_catalog_credential` in addition to the existing
generation discovery API. Only the catalog functions map a completely missing
credential to `PublicOnly`. `discover_ai_gateway_credential` and
`discover_process_ai_gateway_credential`, including generation/reference-host
composition that uses them, continue to return the existing `Missing` error.

## Fixed native Gateway GET provider

Production issues `GET` with no body to exactly:

```text
https://ai-gateway.vercel.sh/coding-agent/v1/models
```

The client uses Rustls with the repository's pinned WebPKI roots and HTTP/1.1.
It sends fixed `Accept: application/json`, `Accept-Encoding: identity`, and
`User-Agent: machine-god/<package-version>` headers plus authorization only for
authenticated access. Machine-god adds no team header or query, proxy, cookie,
referer, title, request-body content type, or endpoint-selection header.
Dependency-required `Host` and wire-framing headers are allowed. Thus the
application-selected header set is exactly `Accept`, `Accept-Encoding`,
`User-Agent`, and optionally `Authorization`.

Redirect following, proxy discovery/use, cookies, automatic decompression,
application retry, status retry, backoff, and referer generation are disabled.
Every 3xx is terminal and maps through the closed `Unavailable` presentation;
`Location` is not followed,
retained, or reflected. Only HTTP 200 has a body consumed. A non-200 body,
reason phrase, headers, endpoint, dependency diagnostic, and operating-system
diagnostic are never used in public error text. Authenticated 401/403 is the
sole typed signal eligible for the one public fallback described above.

The sole alternate endpoint is an explicit native test constructor. It accepts
plain HTTP only for a canonical numeric IPv4 address in `127.0.0.0/8` or
canonical bracketed IPv6 `::1`, an explicit nonzero port, and an absolute path.
It rejects hostnames including `localhost`, non-loopback addresses, alternate
or encoded IP spellings, user information, query, fragment, HTTPS alternates,
and all non-HTTP schemes. Endpoint input is at most 2,048 ASCII bytes.
This constructor is not reachable from process environment, configuration, or
release CLI parsing.

The implemented native catalog module publicly exports the fixed provider
name, all catalog resource constants, `AiGatewayModelCatalogAccessMode`,
`AiGatewayModelCatalogRequestAccess`, the injected
`AiGatewayModelCatalogTransport` trait and its fixed response/error types, and
`AiGatewayModelCatalogProvider`. The optional non-WASM
`ai-gateway-model-catalog-http` surface additionally exports
`AiGatewayModelCatalogHttpEndpoint`,
`AiGatewayModelCatalogHttpLimits`, `AiGatewayModelCatalogHttpTransport`, their
fixed construction error, and the endpoint/time/capacity/chunk constants.
`AiGatewayModelCatalogHttpTransport::new` selects production/default limits;
`with_endpoint_and_limits` accepts only a validated production or numeric-
loopback endpoint and validated limits. The broader `ai-gateway-http` feature
includes this catalog feature and the separate `web-fetch-http` feature, while
the CLI selects only the catalog feature. Construction performs no request;
production construction eagerly reads platform DNS configuration exactly once,
before the provider computes its operation deadline. Numeric-loopback test
construction performs no DNS discovery. Polling `get` requires a current Tokio
runtime with I/O and time enabled.
The injected transport contract also requires `wait_until(deadline)` to retain
and wake a deadline future independently from `get`; the provider polls both,
so a conforming request future cannot remain pending past the shared catalog
deadline.

The production resolver retains only an immutable validated configuration
snapshot, construction-time query-ID sequence, or fixed unavailable states.
Request polling cannot rediscover system configuration or entropy. Generic Unix
other than Apple and Android accepts
the ordinary `/etc/resolv.conf` symlink only when pre-open, opened-descriptor,
and final metadata are regular and at most 64 KiB; nonblocking close-on-exec
reading retains at most one additional overflow byte and has finite read and
interruption bounds. Apple and Windows use their synchronous platform API
during construction and then reject a snapshot over 32 nameservers, 32 search
domains, 8 KiB of DNS names, 64 server connections, or the fixed option bounds.
Those platform APIs may allocate their result before post-validation. Android
does not call Hickory's platform API because it requires initialized NDK process
context and may panic when that context is absent. Android instead retains a
fixed unavailable configuration state, so the production catalog hostname
fails closed through the redacted `Transport` result without a DNS request.

Each Reqwest resolution runs private bounded Tokio UDP/TCP exchange from the
snapshot. A construction-keyed `AtomicU32` sequence derives 16-bit query IDs
with bounded SHA-256 work; request polling calls no entropy source and spawns no
resolver task. A and AAAA run concurrently without detachment. Each family
tries configured nameservers in bounded concurrent batches and attempt order,
with one absolute configured timeout per server exchange. Configured TCP-only,
TCP-on-error, recursion, and trusted-negative behavior is retained; unsupported
case-randomization or avoided-local-port sets fail closed. UDP truncation can
replay only over configured TCP. Each response is at most 4 KiB, exhaustively
decoded, and validates its ID, opcode, class, absolute question, response code,
section counts, CNAME chain/cycle bound, and at most 32 stable first-seen
addresses. A CNAME chain may continue across responses but never exceeds seven
links. Lookup sockets and timers remain owned by the active request/runtime and
drop on cancellation, deadline, request drop, or runtime teardown. Reqwest's
hostname is normalized to one terminal dot before lookup, so the production
host is an absolute FQDN and configured search suffixes are never queried.
Client construction explicitly
disables Reqwest's built-in Hickory selection before installing the custom
resolver, including under dependency feature unification. There is no default
GAI, Google, or other public-resolver fallback. Snapshot failure is exposed
only through the fixed redacted catalog `Transport` result.

## Time, concurrency, body, and JSON bounds

All bounds below are normative and independent. Reaching an inclusive maximum
is allowed; observing one unit beyond it rejects the whole invocation.

| Resource | Exact bound |
| --- | --- |
| total catalog operation | 30 seconds |
| active native catalog attempts | default 8, configured range 1–32 |
| accepted HTTP 200 body | 262,144 bytes (256 KiB) |
| machine-god-retained overflow witness | none; the first frame that would exceed the cap is rejected before append |
| JSON nesting depth | 32 containers, including the root |
| JSON value nodes | 16,384, including root, keys' values, array entries, and nested values |
| raw `data` array entries | 1,024 |
| accepted language-model entries | 512 |
| one accepted model ID | 1–128 bytes, all in ASCII `0x21`–`0x7e` |
| aggregate accepted ID bytes | 24,576 bytes (24 KiB) |
| serialized stdout value | 65,536 bytes (64 KiB), including final LF |

On the first poll of `list_models`, native uses `checked_add` to compute one
absolute deadline before permit acquisition; an unrepresentable deadline maps
to `ResourceLimit` before transport. The 30 seconds include native concurrency-
permit waiting, both possible sequential HTTP attempts, response-head and body
waits, JSON decoding, entry validation, duplicate detection, and sorting. No
phase and no anonymous fallback resets or extends it. CLI rendering follows a
bounded native result and has its independent 64 KiB output cap; it does not extend the
network deadline or enter core.

The provider passes that same absolute deadline unchanged to both possible
transport calls. For each call it polls the transport's required
`wait_until(deadline)` authority independently from `get` and from its own
cancellation waiter. The concrete HTTP `get` additionally owns an attempt-local
cancellation waiter and Tokio sleep for the earlier of its configured per-
attempt timeout and that shared absolute deadline. Therefore fallback creates
a fresh set of per-call waiters, but it neither recomputes nor extends the
provider deadline. There is no single outer Tokio sleep spanning both calls.

One semaphore permit covers one active HTTP attempt through response drop.
There are never two active attempts for one invocation. The authenticated
permit is released before anonymous fallback waits for capacity. Construction
rejects zero or more than 32 configured permits; production uses exactly 8.

The body reader retains no more than 262,144 bytes. An oversized declared
`Content-Length` is rejected before reading; absent or in-range length is not
trusted as proof that a response fits. The first dependency-provided data frame
that would cross the cap is rejected before any bytes from that frame are
appended. Reqwest/Hyper may transiently materialize that frame outside the
machine-god buffer. Automatic
decompression is disabled, so the cap applies to received representation
bytes. Invalid UTF-8 is invalid JSON. One linear structural pass over the
already bounded body enforces depth and node budgets before semantic field
decoding and without converting JSON numbers to a machine numeric type. Each
standards-valid number is exactly one node regardless of integer or exponent
magnitude. Semantic decoding borrows bounded raw values rather than allocating
arbitrary-precision numbers. Trailing non-whitespace bytes are rejected.

## Gateway response and entry validation

An accepted response is exactly one top-level JSON object with a `data` member
whose value is an array. Missing `data`, a non-array `data`, a non-object root,
invalid JSON, duplicate top-level `data`, or a raw array longer than 1,024
rejects the response. Other top-level fields may be ignored while still
counting toward the global JSON budgets.

Each raw `data` value is inspected independently. Exactly three classes are
skipped: a non-object value, an object with a missing or non-string `id`, and an
object whose string `type` is not ASCII-case-insensitively `language`. An absent
or non-string `type` is accepted as the pinned language default. Once a string
ID reaches validation, an empty, longer-than-128-byte, non-ASCII, space, or
control-containing ID is a terminal `MalformedResponse`; an unsafe ID is never
silently skipped. Unknown fields and malformed optional metadata map only to
the documented defaults. Repeating any recognized entry field (`id`, `type`,
`released`, or `tags`) is terminal `MalformedResponse`. Nested ignored data
still counts toward the global JSON budgets.

Language classification follows the pinned catalog shape described above.
Release metadata that is absent or not an integer fitting signed 64-bit maps to
`0`; this includes standards-valid integers and exponents outside the signed
64-bit range. Such numbers remain valid ignored/defaulted values in `tags`,
unknown top-level or entry fields, and non-object raw `data` entries rather
than making the response malformed. Tags that are absent or not an array
provide no capability; a string tag equal to `tool-use`
under ASCII case folding sets the tool-use bit. Other tag values are ignored.
Tier and provider ranks derive only from the already validated ID; there are no
separate provider or tier wire fields.

After skipped entries are removed, more than 512 valid language entries or more
than 24,576 aggregate ID bytes rejects the whole response. Two valid entries
with byte-identical IDs reject the whole response; duplicates are not silently
removed. ID comparison and all final tie-breaking are bytewise and locale-
independent. Skipping only the three enumerated structural/non-language entry
classes is intentional, but an unsafe string ID or any global structural,
resource, or duplicate-ID defect is terminal.

## Stable full-catalog ordering

The command never applies an interactive picker limit. Every accepted ID is
shown. Sorting is stable and compares these keys in order:

1. tool-use capable entries first;
2. tier rank ascending, using ASCII-case-insensitive ID substring matching in
   this exact precedence: `preview` or `beta` rank 4; `haiku`, `mini`, or
   `lite` rank 3; `flash` rank 2; `opus`, `sonnet`, `gpt-5`, `o1`, `o3`,
   `o4`, `pro`, or `grok-4` rank 0; all others rank 1;
3. provider rank ascending, using these exact case-sensitive ID prefixes:
   `anthropic/` 0, `openai/` 1, `google/` 2, `xai/` 3, `deepseek/` 4,
   `meta/` 5, `mistral/` 6, `alibaba/` 7, and every other prefix 8;
4. release integer descending; then
5. model ID ascending bytewise.

The explicit tier precedence above matters: the first matching tier group wins.
Type, tag, and tier matching are ASCII-case-insensitive; provider-prefix
matching is deliberately case-sensitive like the pinned comparator. The
comparator is total because the validated ID is the final key and duplicate IDs
were rejected.

## Success output

Human output for a nonempty catalog is exactly:

```text
[models] N available
 - <id-1>
 - <id-2>
```

There is one ID line per accepted entry in stable order. `N` is the decimal
number of accepted IDs. An empty catalog writes exactly:

```text
[models] no models returned by gateway
```

If the successful result is public because no credential was present, append:

```text
[models] Using the public model catalog; set VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY to include private models.
```

If an authenticated 401/403 caused successful public fallback, append instead:

```text
[models] Gateway authentication was rejected; showing the public model catalog.
```

No explanation is appended after authenticated success. These messages are
machine-god-specific and do not advertise an unsupported fx login command.
Every human result has exactly one final LF and no stderr.

JSON success is one compact object with this exact key order and one final LF:

```json
{"kind":"models","count":2,"shown_count":2,"more_count":0,"private_models_hidden":false,"ids":["provider/a","provider/b"]}
```

`count` and `shown_count` both equal the complete accepted ID count;
`more_count` is always zero; `ids` contains the complete sorted list; and
`private_models_hidden` is true for either successful public path and false for
authenticated success. JSON success has no stderr. The complete selected
representation, including its LF, must fit 64 KiB before the first stdout
write. There is no partial success or truncation path.

## Failures, redaction, and exit status

Every valid-command failure exits 1. Before any successful stdout bytes are
written, human mode writes no stdout and one redacted line to stderr:

```text
machine-god models: could not list models: <detail>
```

JSON mode writes no stderr and this exact compact shape to stdout, followed by
one LF:

```json
{"kind":"models","error":"could not list models: <detail>","code":"<code>"}
```

The mapping is closed and exact:

| Internal terminal class | `<detail>` | `<code>` |
| --- | --- | --- |
| authentication rejected | `AuthenticationRejected` | `AuthenticationRejected` |
| cancellation | `the request was cancelled` | `Cancelled` |
| malformed response, including structural/duplicate defects | `MalformedResponse` | `MalformedResponse` |
| any resource bound, including deadline, capacity/body/JSON/entry/ID/output overflow | `ResourceLimit` | `ResourceLimit` |
| rate, Gateway, transport, configuration, credential, runtime, and every other failure | `Unavailable` | `Unavailable` |

The public distinction is semantically useful but fully redacted. An
authenticated 401/403 is not terminal when its one public fallback can begin;
`AuthenticationRejected` is emitted only when the final result is that class.
An initial public-only 401/403 has no fallback and maps directly to that same
closed failure presentation.

Internal typed errors remain available for deterministic tests and the single
fallback decision. Output never reflects a token, source selection, model
value from configuration, endpoint, response body or header, model ID from a
rejected response, path, OS error, dependency error, numeric HTTP status,
status reason, or redirect location.

If writing either a completed success or completed JSON failure itself fails,
the process exits 1 and makes a best-effort write of the existing fixed stderr
diagnostic `machine-god: failed to write output\n`. No second network request is
made. The fixed failure representations are inherently far below the 64 KiB
success-rendering cap and do not traverse that renderer.

## Cancellation and drop

Catalog futures do no work until polled. Native checks cancellation before
permit acquisition, request dispatch, each response-body read, JSON decoding,
bounded entry processing, and each native-effect transition. A cancellation
that is ready in the same poll as capacity, HTTP, body, or provider completion
wins. Deadline expiry has the same pre-acceptance rule after cancellation
precedence. On a Serde or trailing-input failure, native rechecks cancellation
and deadline before accepting the parser classification. It also checks both
before and after sorting, during each final `AvailableModel` construction, and
again immediately before returning the completed result. The CLI then renders
that result, and core performs no clock, sorting, or output work.

Dropping or cancelling an attempt drops its owned Reqwest request/response,
body buffer, and semaphore permit. Dropping the one trait future between the
two native attempts prevents anonymous fallback. The native future owns one
provider cancellation waiter and one separately polled deadline authority for
the currently active transport call, while each HTTP call owns its attempt-
local cancellation waiter and timer described above. All are dropped with that
call; fallback creates new per-call waiters against the unchanged deadline.
Machine-god spawns no catalog worker, producer, retry/backoff, or detached task.
Reqwest/Hyper connection dispatch and its bounded connection timers remain
owned by the host runtime. The CLI keeps that current-thread runtime driven
through request completion and teardown.
Cancellation/drop cannot recall bytes already sent or prove what the peer
received.

The CLI registers Ctrl-C cancellation on every supported native target and
SIGTERM cancellation on Unix. Signal registration or wait failure maps to the
closed `Unavailable` presentation; closure of the SIGTERM stream is a wait
failure. Both parent-owned listener futures are created and polled once before
the provider future is created. One parent-owned poll loop checks the listeners
before the provider and rechecks them after a ready provider poll, so a ready
signal or wait failure wins that poll. A received signal cancels and drops the
provider future before the command returns. Listener futures are dropped before
rendering; no signal task is spawned or detached.

## Intentional pinned-fx differences and deferred scope

The command mirrors the pinned fx command name, full-catalog ordering, success
shapes, and single authenticated-401/403 public fallback. These are deliberate
compatibility inputs, not a claim of general fx equivalence. Machine-god
intentionally differs by:

- rejecting repeated `--json` and every extra/non-Unicode argument;
- loading its strict config once before credential/network effects;
- failing closed on a selected invalid credential rather than treating it as
  absence;
- omitting fx login, team selection, team query/header, referer, title, and fx
  identity while sending `machine-god/<package-version>` as its own user agent;
- pinning production origin/path with only a numeric-loopback test seam;
- rejecting redirects, proxies, decompression, cookies, and retries;
- bounding time, concurrency, body, JSON work, entry counts, ID bytes, and
  serialized output;
- rejecting duplicate valid IDs and global structural/resource defects; and
- exposing only redacted fixed operational diagnostics.

This slice adds no picker, model selection, generation call, catalog cache,
offline catalog, account/team/login flow, pagination, streaming catalog,
configuration schema field, SDK surface, or compatibility promotion. It makes
no product-performance, speedup, latency, memory, or fx-equivalence claim.
Benchmarks and remote workflow evidence remain later gates for the exact
implemented candidate.
