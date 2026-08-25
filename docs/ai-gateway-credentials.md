# Native AI Gateway credential discovery

Status: Milestone 03 tenth slice integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. Exact main CI run `32573320962`
and benchmark-evidence run `32573320937` are green. The delivered generation-
credential behavior remains unchanged. A local `models [--json]`
implementation now reuses the snapshot and adds a catalog-specific optional-
auth projection. Its six focused independent credential cases are present in
native evidence `12263afa458e48f2963ae3d0e3db5cf219f8bdf6`. Exact catalog
behavior candidate `2ea9d94`, tree `3a948b2`, passed its complete replacement
gate but was rejected by cycle-2 review; none of its findings changed credential
selection. Parser and HTTP lifecycle remediation is locally composed. The
pre-review cycle-3 gate attempt was rejected for request-time DNS configuration;
eager snapshot remediation does not change credential selection. The complete
cycle-3 gate passed at exact candidate `2cecc921`, tree `8c0d235`; adversarial
review, integration, and delivery remain pending.
Review details for the delivered generation behavior are in the
[`credential discovery review`](reviews/m03-ai-gateway-credential-review-01.md).

The adapter discovers one Vercel AI Gateway bearer credential from an
explicitly owned environment snapshot. It is separate from core, the CLI, and
native configuration. Bearer-token bytes are fields in no schema. The
thirteenth slice's schema v3 adds only the closed non-secret acquisition
kind `credential_source: "environment"`; exact v1/v2 files project that kind
only in memory. The only ambient lookup remains the explicitly named process-
snapshot constructor or process convenience function.

## Feature and public boundary

The API is available under either non-WASM native HTTP gate:

```text
all(
    any(
        feature = "ai-gateway-http",
        feature = "ai-gateway-model-catalog-http"
    ),
    not(target_family = "wasm")
)
```

The narrower catalog feature exposes the shared bearer/error and credential
surface without enabling generation transport exports, `web-fetch-http`,
Hickory DNS, or Moka. The existing `ai-gateway-http` feature includes the
catalog and web-fetch features and preserves its prior public behavior.

The public construction and discovery surface is:

```rust,ignore
pub const VERCEL_OIDC_TOKEN_ENV: &str = "VERCEL_OIDC_TOKEN";
pub const AI_GATEWAY_API_KEY_ENV: &str = "AI_GATEWAY_API_KEY";

AiGatewayCredentialEnvironment::new(
    vercel_oidc_token: Option<OsString>,
    ai_gateway_api_key: Option<OsString>,
) -> AiGatewayCredentialEnvironment

AiGatewayCredentialEnvironment::from_process()
    -> AiGatewayCredentialEnvironment

discover_ai_gateway_credential(environment: AiGatewayCredentialEnvironment)
    -> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError>

discover_process_ai_gateway_credential()
    -> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError>

discover_ai_gateway_catalog_credential(environment: AiGatewayCredentialEnvironment)
    -> Result<DiscoveredAiGatewayCatalogCredential, AiGatewayCredentialError>

discover_process_ai_gateway_catalog_credential()
    -> Result<DiscoveredAiGatewayCatalogCredential, AiGatewayCredentialError>
```

`DiscoveredAiGatewayCredential::source` returns the non-secret selected
`AiGatewayCredentialSource`. Its stable names are `vercel_oidc_token` and
`ai_gateway_api_key`. `into_bearer_token` consumes the result and returns the
existing `AiGatewayBearerToken`; there is no revealing token accessor.

The snapshot and discovered result do not implement `Clone`, serialization, or
equality. Discovery consumes the snapshot so the selected token moves into the
result and an unused valid fallback can be dropped without cloning either
secret. The source enum and error kind are ordinary non-secret value enums.

`DiscoveredAiGatewayCatalogCredential` is either `Authenticated` with the same
validated discovered credential or `PublicOnly`. Only the two catalog
discovery functions translate a completely missing credential into
`PublicOnly`. A selected invalid value retains the exact fail-closed behavior
below. The generation functions continue to return
`AiGatewayCredentialErrorKind::Missing`, so anonymous catalog listing does not
weaken generation or `NativeReferenceHost` construction.

## Exact source precedence

Sources are evaluated in this fixed order:

1. a nonempty `VERCEL_OIDC_TOKEN`;
2. a nonempty `AI_GATEWAY_API_KEY`; then
3. a missing-credential error.

Unset and exactly zero-length values are absent. Empty OIDC therefore falls
through to the API key. No whitespace or Unicode normalization is performed:
spaces, tabs and line endings are nonempty invalid credentials.

A selected nonempty source fails closed if it is non-Unicode, oversized, or
malformed. Discovery does not fall through to a lower-priority valid value in
that case. A valid selected source succeeds without making an invalid
lower-priority source observable. An injected snapshot owns its values and is
not affected by later process-environment changes; independent snapshots share
no global cache.

The two names and their order match the pinned fx comparison revision. This is
not a claim of full fx compatibility or current Vercel behavior.

## Validation and failures

Unicode values pass through the existing `AiGatewayBearerToken::new` validator.
The exact accepted shape remains 1–4,096 UTF-8 bytes of RFC 6750 `b64token`:
one or more ASCII letters, digits, `-`, `.`, `_`, `~`, `+`, or `/`, followed
only by optional trailing `=` padding. Exactly 4,096 bytes is accepted;
4,097 bytes is rejected. Leading or internal padding, data after padding,
controls, whitespace, non-ASCII text and header-injection bytes are rejected.

Failures have only these stable categories and messages:

| `AiGatewayCredentialErrorKind` | Exact display text |
| --- | --- |
| `Missing` | `AI Gateway credential is missing` |
| `InvalidEnvironment` | `AI Gateway credential environment is invalid` |
| `InvalidBearerToken` | `AI Gateway bearer token is invalid` |

`InvalidEnvironment` means the selected nonempty value is not Unicode.
`InvalidBearerToken` groups the selected Unicode value's syntax and size
failures. Errors retain no source, environment value, token bytes, operating-
system diagnostic, or validator error. Display, debug, and error-source output
are fixed and non-reflecting. Discovery prints nothing.

## Bounds and secret lifetime

The snapshot converts supplied values into absent, invalid, or validated-token
states and does not retain raw invalid input. At most two validated 4,096-byte
tokens can be retained before precedence is resolved. Discovery consumes that
snapshot, moves the selected token into the result, and drops any unused valid
fallback. The bearer token keeps its existing best-effort byte clearing on
invalid construction and drop.

These are accepted-data, retained-data, move-without-clone, and non-reflection
guarantees. They are not locked-memory, allocator-history, process-environment,
HTTP-header, or comprehensive zeroization guarantees. In particular,
`std::env::var_os` may materialize the complete operating-system value before
machine-god can reject a value larger than 4,096 bytes. The application does
not claim that process lookup itself reads only 4,097 bytes.

## Authority and deferred scope

Injected discovery reads no process state. `from_process` and
`discover_process_ai_gateway_credential` read only the two documented names;
they perform no file, keychain, prompt, terminal, network, or configuration
access. Core receives no ambient credential authority. The HTTP transport still
receives an explicit validated bearer token and retains its existing endpoint,
origin, redirect, proxy, status, timeout, and cancellation policy.

This integrated adapter does not store or rotate credentials, write environment
variables, add a setup command, select a generation model, or itself make a
network request. The local CLI catalog composition explicitly calls the new
process catalog convenience function after config validation; that host use
does not give this adapter transport authority. The thirteenth slice
[`native configuration schema-v3 contract`](configuration.md) adds a
declarative non-secret acquisition-kind field; it adds no bearer value,
arbitrary environment name, or ambient lookup and does not invoke this adapter.
The separate twelfth
[`native reference-host slice`](native-reference-host.md) implements a
production library constructor that consumes an explicitly injected snapshot
and retains only the selected source metadata. Its implementation, independent
tests, three fresh adversarial tracks, and exact feature and `main` workflows
are green; its final delivery record is `ac3984fb`. Its custom-
transport authority override skips discovery
and reports no native-discovery source. The production constructor validates
configured `Environment` and then consumes its already injected snapshot;
`NativeReferenceHost::credential_source()` still reports the concrete selected
OIDC-token or API-key source. The broader M03 credential-and-configuration
checklist item is complete after the thirteenth slice passed every local,
adversarial, feature, integration, and exact `main` gate. It is integrated at
`8755757d`; all three adversarial tracks are green on exact behavior SHA
`35ce591e`.
The delivered generation adapter's adversarial, feature-branch,
documentation-seal, and `main` gates are green at the exact lineage recorded in
its review. The new catalog projection and CLI composition are explicitly
outside that historical green claim; benchmark workloads remain unchanged.
Zig remains only a build input for the
pinned upstream benchmark; machine-god remains a Rust product.

## Delivery evidence

Production, independent black-box tests, required local checks, and three fresh
adversarial tracks are green. Exact feature CI run `32573044224` and
benchmark-evidence run `32573044159` are green for the documentation seal at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. The branch was fast-forwarded
without force to `main`; exact main CI run `32573320962` and benchmark-evidence
run `32573320937` are green for that same SHA.
