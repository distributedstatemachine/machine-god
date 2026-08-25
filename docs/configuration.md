# Native configuration

Status: integrated contract for the thirteenth bounded Milestone 03 slice.
Thirteen slices are integrated. Production, independent tests, focused and
required local gates, and all three fresh adversarial tracks are green on exact
behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. The slice is integrated on
`main` through final delivery record
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact final-record feature CI
run `32583585145`, feature benchmark-evidence run `32583585148`, main CI run
`32583871385`, and main benchmark-evidence run `32583871368` are green.
Milestone 03 remains `IN PROGRESS`. Full lineage is recorded in the
[`configured credential-source review`](reviews/m03-configured-credential-source-review-01.md).

The native configuration loader is a bounded, synchronous, read-only
`machine-god-native` authority. Core remains independent of the process
environment and filesystem. `machine-god status` remains metadata-only and
does not invoke the loader. The implemented
`machine-god permissions [--json]` path invokes it exactly once after complete
argument validation and observes only permission mode. Exact permissions
cycle-5 reviewed candidate `0b13944d19cfb33b4542d82d74c302669817c1af`,
tree `2ea72e810f07ed8ca2d4e8647fa713088477d8b5`, passed its complete exact-
1.94.1 replacement gate without fallback and all three fresh reviews at
0/0/0/0. The gate included 928 non-documentation executions, two doctests,
focused native configuration 25+29 and CLI 6+19, pinned-fx compatibility and
31 generator tests, documentation integrity 76/110/548/391 with zero errors,
no dependency or unsafe delta, and the full release matrix. Its 368,944-byte
binary has SHA-256
`8756c7801285f1b09cad9a8b8ce47700a44127dec68ef2b0613e6a5dcecad45e`.
The behavior candidate is **GREEN**. Documentation-only seal
`3e41cc6b90adb34d62aec21c6d03729d59ca0c1b`, tree
`bd74a96c4952c2eb1e15372f4ab716a76bba91a9`, is exempt from another
adversarial cycle. Exact feature CI `32891031065`, feature benchmark-evidence
`32891031147`, main CI `32891614025`, and main benchmark-evidence `32891614060`
are green on that exact seal SHA. `main` was fast-forwarded without force from
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`; each benchmark run retains
exactly two unexpired exact-SHA artifacts for 90 days. Permissions is delivered
as slice twenty-eight, while M03 remains in progress. This delivery makes no
product-performance or fx-equivalence claim. The final delivery-record commit
is documentation-only and review-exempt; its own exact feature and `main`
workflows will be reported at handoff rather than claimed here.

The locally composed twenty-ninth
[`models [--json]` implementation](models-cli.md) invokes this loader exactly
once after complete argument validation. It validates the closed provider,
transport, and credential-source selections before native credential or
network access, never reloads configuration for public fallback, never changes
the configured generation model, and never writes or migrates the file. That
catalog path accepts the built-in or missing-file safe schema-v3 defaults and
strict v1/v2/v3 files, but rejects any config-load failure before credential
discovery. It does not add an endpoint, team, token, cache, or catalog field to
schema v3. Focused independent native evidence is present at
`12263afa458e48f2963ae3d0e3db5cf219f8bdf6`. Exact cycle-1 candidate `6277aa3`,
tree `b5e2445`, was rejected; config-once/order remediation is composed at
`d2890c3`. Exact cycle-2 behavior candidate `2ea9d94`, tree `3a948b2`, passed
the complete replacement gate. Fresh cycle-2 review, integration, and delivery
remain pending. The historical configuration green status above does not make
this new composition green.

This configuration slice advances the built-in and current file
schema to v3 while retaining strict read compatibility for the exact legacy v1
and v2 objects.

## Location and defaults

The loader resolves only the configuration portion of an injected environment
snapshot:

- a nonempty `XDG_CONFIG_HOME` is selected and must be absolute Unicode;
- an empty `XDG_CONFIG_HOME` falls back to a nonempty, absolute-Unicode `HOME`;
- the resolved path is `<XDG_CONFIG_HOME>/machine-god/config.json`, or
  `<HOME>/.config/machine-god/config.json` for the fallback; and
- a selected nonempty relative or non-Unicode value is invalid. Selection fails
  without trying a different environment value.

Native status still snapshots `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, and `HOME`
because it reports both config and state metadata. The rejected cycle-1
permissions candidate unnecessarily used that general snapshot even though its
loader consumed only configuration inputs. The composed config-loading and
permissions snapshot requests `XDG_CONFIG_HOME` and `HOME` and never requests
`XDG_STATE_HOME`, but cycle 2 found that `HOME` is read and stored eagerly. The
composed replacement reads `XDG_CONFIG_HOME` first and reads `HOME` only when
XDG is missing or empty. A nonempty XDG value decides selection whether it is
valid, relative, or non-Unicode, so that path neither reads nor falls back to
`HOME`. Status retains the general three-value snapshot. The cycle-3 and
cycle-4 gates and review tracks confirmed this behavior; cycle 4 was rejected
only for the separate ambiguous delivery terminology described above. Exact
cycle 5 passed its complete replacement gate and all three fresh reviews with
zero findings.

An unavailable location, including a missing or empty needed `HOME`, produces
the explicit built-in schema-v3 configuration. A resolved file that is missing
also produces this configuration:

```json
{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}
```

Invalid selected environment input is not treated as absence and fails closed.
Likewise, an inaccessible resolved path is an error rather than a reason to
silently use defaults.

## Strict schema v3

A present schema-v3 configuration is one JSON object containing exactly these
six required fields:

| Field | Required value |
| --- | --- |
| `schema_version` | JSON integer `3` |
| `permission_mode` | JSON string `"ask"` |
| `provider` | JSON string `"vercel_ai_gateway"` |
| `transport` | JSON string `"ai_gateway_http"` |
| `model` | JSON string of 1–128 visible ASCII bytes |
| `credential_source` | JSON string `"environment"` |

The model byte range is `0x21` through `0x7e`, inclusive. Empty models, spaces,
controls, non-ASCII text, and values longer than 128 bytes are invalid. The
built-in model is the exact string `"zai/glm-5.2"`; a valid file may select any
model satisfying the same bounded validator used by `AiGatewayProvider` for
its default model and request-level model override.

Unknown or duplicate fields, missing fields, wrong JSON types or shapes,
unsupported field values, and invalid model values are errors. There is no
field ignoring, coercion, alias, case folding, or schema-specific fallback.
JSON object field order and insignificant JSON whitespace do not alter the
decoded object.

`credential_source` is a closed, non-secret acquisition-kind selection. It
does not contain a token, select an arbitrary environment-variable name, or
grant the loader process-environment authority.

## Strict schema-v1 and schema-v2 read compatibility

The exact two-field schema-v1 object remains accepted:

```json
{"schema_version":1,"permission_mode":"ask"}
```

The exact five-field schema-v2 object also remains accepted:

```json
{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}
```

Its model may be any value accepted by the same bounded validator as v3.
Schema-v1 and schema-v2 unknown or duplicate fields, missing fields, wrong
types or shapes, and unsupported values remain invalid. In particular, a v1
or v2 object containing `credential_source` is rejected as an unknown-field
error; accepting that field requires explicit schema version `3`.

An accepted v1 file is projected in memory to permission mode `ask`, provider
`vercel_ai_gateway`, transport `ai_gateway_http`, model `zai/glm-5.2`, and
credential source `environment`. An accepted v2 file retains its validated
model and gains only the same in-memory `environment` credential-source
projection. Their observable `schema_version()` values remain `1` and `2`
respectively; neither is relabelled as v3. Loading never rewrites, expands, or
migrates either file.

Every integer schema version other than `1`, `2`, or `3` is unsupported. A
missing, duplicate, non-integer, or otherwise malformed schema-version field is
invalid format. Full-buffer UTF-8 validation still precedes schema dispatch.

## Public data boundary

`CONFIG_SCHEMA_VERSION` is `3`. `AI_GATEWAY_DEFAULT_MODEL` is
`"zai/glm-5.2"`, and `AI_GATEWAY_MAX_MODEL_BYTES` is `128`.
`NativeProviderKind::VercelAiGateway` has stable machine name
`vercel_ai_gateway`; `NativeTransportKind::AiGatewayHttp` has stable machine
name `ai_gateway_http`; and `NativeCredentialSourceKind::Environment` has stable
machine name `environment`. Their `as_str` accessors return those names. They
do not imply that an optional implementation is compiled or usable in the
current build.

`NativeConfig` exposes read-only `schema_version`, `permission_mode`,
`provider`, `transport`, `model`, and `credential_source` getters. The schema
version remains the version actually loaded, including `1` or `2` for a legacy
file. Provider, transport, and credential source return closed native enums;
model returns the validated string.
`NativeConfig` and `LoadedNativeConfig` implement `Clone`, but not `Copy`,
because configuration owns its bounded model string. `NativeConfig` debug
output exposes the non-secret schema, permission, provider, transport, and
credential-source fields but renders the model as `"<redacted>"`;
`LoadedNativeConfig` inherits that redaction through its nested configuration.

`ai_gateway_http` is a declarative transport selection in the configuration
schema. Its enum exists independently of the optional `ai-gateway-http` Cargo
feature and on targets where that feature's concrete Reqwest transport exports
are absent, including WebAssembly. Parsing this value therefore proves only
that configuration is valid; it does not prove transport availability,
construct a Tokio runtime, or make a network path usable.

Vercel AI Gateway credential bytes are fields in no schema. The separately
integrated [`native credential discovery adapter`](ai-gateway-credentials.md)
owns its non-cloneable secret snapshot and does not put secret values into
`NativeConfig`, debug output, status output, or the configuration file.

## Bounds and filesystem boundary

The raw file limit remains 64 KiB (65,536 bytes). A file of exactly that length
can be considered for parsing; any additional byte makes it oversized. Bytes
must be valid UTF-8 and then valid strict v1, v2, or v3 JSON. The loader retains
at most 64 KiB plus one byte while deciding whether input fits, so neither a
stale size observation nor concurrent file growth turns loading into an
unbounded retained buffer. Exact permissions cycle 1 found that the candidate's
read loop retries every `Interrupted` result without a cumulative work bound.
The composed replacement retries the first 15 cumulative interrupted results and
maps the 16th to the existing fixed `Unreadable` error, with deterministic
injected-reader evidence for both boundaries. Partial progress does not reset
the count, and an over-reported read maps to `Unreadable`. The complete cycle-2
replacement gate and review confirmed these bounds; that rejected candidate's
two lows concerned lazy environment access and target-qualified documentation.
The complete cycle-3 and cycle-4 gates and all fresh tracks confirmed the
remediated loader bounds and environment-selection behavior; cycle 4 was
rejected only for the separate ambiguous delivery terminology in
`security.md`. Exact cycle 5 passed its complete replacement gate and all three
fresh review tracks at 0/0/0/0.

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path with `O_NOFOLLOW` and nonblocking behavior. It performs a
preliminary path-kind check, then authoritatively validates the opened
descriptor as a regular file before reading it. It therefore rejects a final
symlink,
directory, FIFO, socket, device, or other non-regular entry and does not block
opening a hostile FIFO. Hardened open semantics for non-Unix targets are not
part of this milestone's supported contract.

Loading never writes configuration, creates files or directories, or
canonicalizes the selected path. It does not claim to validate or freeze the
entire ancestor path: on supported Unix targets, the bounded guarantee remains
final-component no-follow plus descriptor regularity.

Errors remain typed so callers can distinguish invalid environment input,
open/read failure, invalid file kind, size overflow, invalid format, and an
unsupported schema version. Invalid UTF-8, malformed JSON, schema-shape errors,
invalid model values, and unsupported permission/provider/transport/credential-
source values are grouped as invalid format. Diagnostics do not reflect
environment-derived paths, configuration bytes, model values, or operating-
system error text. All listed failures fail closed.

## Relationship to status and deferred work

Native status remains a separate metadata-only observation. It still uses
final-path metadata to report config-file and state-directory states, does not
read or parse `config.json`, and reports permission mode `ask`. Existing CLI
version, status, and bare-invocation bytes remain unchanged. The original
configuration slice added no CLI command; the separate permissions slice
intentionally changes help and invalid-usage bytes and loads configuration once
after valid permissions parsing.

Provider, transport, model, and credential-source fields are declarative data
only. `environment` tells the production reference-host constructor which
already injected acquisition adapter is compatible with this configuration; it
does not read the process environment. This slice does not instantiate
`AiGatewayProvider`, select or construct an HTTP client, create or drive a
Tokio runtime, discover or attach a credential, open a network connection, or
compose any component into core or the CLI.

The separate twelfth
[`native reference-host slice`](native-reference-host.md) consumes an
already loaded value without changing this loader. It retains the exact
`LoadedNativeConfig`: accepted file-backed v1 and v2 values therefore remain
observable with their exact origins and schema versions while their in-memory
`environment` projection drives the same production composition path. That
constructor validates `NativeCredentialSourceKind::Environment` and consumes a
separately injected `AiGatewayCredentialEnvironment`; the config loader never
calls `from_process`. Runtime `NativeReferenceHost::credential_source()` still
reports the concrete selected OIDC-token or API-key source, not the configured
acquisition kind. The trusted custom-transport constructor skips native
discovery and reports `None` as before.

The separate fourteenth composed
[`native root-selection candidate`](native-root-selection.md) derives a state
root from the same injected snapshot but does not change configuration or use
the loader. Its preparation authority remains independent of this read-only
surface; schema-v3 built-in and file bytes are unchanged.

Configuration mutation, a migration or rewrite command, a terminal permission
prompter and modes beyond `ask`, runtime composition, CLI composition beyond
the read-only permissions projection, session lifecycle, the remaining native
tools, remaining CLI and session expansion, release-binary end-to-end host
evidence, and compatibility or performance claims remain open. The
combined credential-and-configuration
checklist item is complete after implementation, independent tests, three green
adversarial tracks, exact feature workflows, fast-forward integration, and
exact `main` workflows all passed. Milestone 03 remains in progress.
