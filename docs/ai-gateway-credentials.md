# Native AI Gateway credential discovery

Status: Milestone 03 tenth slice integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. Exact main CI run `32573320962`
and benchmark-evidence run `32573320937` are green. This does not mean that the
CLI makes model requests today. Review details are in the
[`credential discovery review`](reviews/m03-ai-gateway-credential-review-01.md).

The adapter discovers one Vercel AI Gateway bearer credential from an
explicitly owned environment snapshot. It is separate from core, the CLI, and
native configuration. Credentials are fields in neither the exact legacy v1
schema nor the integrated current v2 schema. The only ambient lookup is the
explicitly named process-snapshot constructor or process convenience function.

## Feature and public boundary

The API is available only under the same native gate as the existing HTTP
transport:

```text
all(feature = "ai-gateway-http", not(target_family = "wasm"))
```

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
```

`DiscoveredAiGatewayCredential::source` returns the non-secret selected
`AiGatewayCredentialSource`. Its stable names are `vercel_oidc_token` and
`ai_gateway_api_key`. `into_bearer_token` consumes the result and returns the
existing `AiGatewayBearerToken`; there is no revealing token accessor.

The snapshot and discovered result do not implement `Clone`, serialization, or
equality. Discovery consumes the snapshot so the selected token moves into the
result and an unused valid fallback can be dropped without cloning either
secret. The source enum and error kind are ordinary non-secret value enums.

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
variables, add a setup command, compose a provider into the CLI, select a model,
or make a network request. The eleventh integrated
[`native configuration schema-v2 slice`](configuration.md) adds only
declarative provider, transport, and model data; it does not add a credential
field or invoke this adapter. The separate twelfth
[`native reference-host candidate`](native-reference-host.md) implements a
production library constructor that consumes an explicitly injected snapshot
and retains only the selected source metadata. Its focused composed tests are
green, while adversarial and delivery gates remain pending. Its custom-
transport authority override skips discovery
and report no native-discovery source. The broader M03 credential-and-
configuration checklist item remains unchecked even after that candidate is
delivered because configuration still has no bounded credential-source field.
This adapter's adversarial, feature-branch, documentation-seal, and `main` gates
are green at the exact lineage recorded in its review. Existing CLI bytes and
benchmark workloads are unchanged. Zig remains only a build input for the
pinned upstream benchmark; machine-god remains a Rust product.

## Delivery evidence

Production, independent black-box tests, required local checks, and three fresh
adversarial tracks are green. Exact feature CI run `32573044224` and
benchmark-evidence run `32573044159` are green for the documentation seal at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. The branch was fast-forwarded
without force to `main`; exact main CI run `32573320962` and benchmark-evidence
run `32573320937` are green for that same SHA.
