# Native configuration

Status: eleventh bounded Milestone 03 slice integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`. Exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. Milestone 03 remains
`IN PROGRESS`. Exact lineage is recorded in the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).

The native configuration loader is a bounded, synchronous, read-only
`machine-god-native` authority. Core remains independent of the process
environment and filesystem, and the CLI does not invoke the loader. This
slice advances the built-in and current file schema to v2 while retaining
strict read compatibility for the exact legacy v1 object.

## Location and defaults

The loader uses the same environment snapshot and config-path resolution as
native status:

- a nonempty `XDG_CONFIG_HOME` is selected and must be absolute Unicode;
- an empty `XDG_CONFIG_HOME` falls back to a nonempty, absolute-Unicode `HOME`;
- the resolved path is `<XDG_CONFIG_HOME>/machine-god/config.json`, or
  `<HOME>/.config/machine-god/config.json` for the fallback; and
- a selected nonempty relative or non-Unicode value is invalid. Selection fails
  without trying a different environment value.

An unavailable location, including a missing or empty needed `HOME`, produces
the explicit built-in schema-v2 configuration. A resolved file that is missing
also produces this configuration:

```json
{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}
```

Invalid selected environment input is not treated as absence and fails closed.
Likewise, an inaccessible resolved path is an error rather than a reason to
silently use defaults.

## Strict schema v2

A present schema-v2 configuration is one JSON object containing exactly these
five required fields:

| Field | Required value |
| --- | --- |
| `schema_version` | JSON integer `2` |
| `permission_mode` | JSON string `"ask"` |
| `provider` | JSON string `"vercel_ai_gateway"` |
| `transport` | JSON string `"ai_gateway_http"` |
| `model` | JSON string of 1–128 visible ASCII bytes |

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

## Strict schema-v1 read compatibility

The only legacy input accepted is the exact two-field schema-v1 object:

```json
{"schema_version":1,"permission_mode":"ask"}
```

Schema-v1 unknown or duplicate fields, missing fields, wrong types or shapes,
and a permission mode other than `"ask"` remain invalid. An accepted v1 file is
projected in memory to permission mode `ask`, provider `vercel_ai_gateway`,
transport `ai_gateway_http`, and model `zai/glm-5.2`. Its observable
`schema_version()` remains `1`; it is not relabelled as v2. Loading never
rewrites, expands, or migrates the file. Consequently, a caller can distinguish
an exact legacy file from a built-in or file-backed v2 configuration while
using the same fixed declarative provider, transport, and model projection.

Every integer schema version other than `1` or `2` is unsupported. A missing,
duplicate, non-integer, or otherwise malformed schema-version field is invalid
format. Full-buffer UTF-8 validation still precedes schema dispatch.

## Public data boundary

`CONFIG_SCHEMA_VERSION` is `2`. `AI_GATEWAY_DEFAULT_MODEL` is
`"zai/glm-5.2"`, and `AI_GATEWAY_MAX_MODEL_BYTES` is `128`.
`NativeProviderKind::VercelAiGateway` has stable machine name
`vercel_ai_gateway`; `NativeTransportKind::AiGatewayHttp` has stable machine
name `ai_gateway_http`. Their `as_str` accessors return those names. They do not
imply that an optional implementation is compiled or usable in the current
build.

`NativeConfig` exposes read-only `schema_version`, `permission_mode`,
`provider`, `transport`, and `model` getters. The schema version remains the
version actually loaded, including `1` for a legacy file. Provider and
transport return the closed native enums; model returns the validated string.
`NativeConfig` and `LoadedNativeConfig` implement `Clone`, but not `Copy`,
because configuration owns its bounded model string. `NativeConfig` debug
output exposes the non-secret schema, permission, provider, and transport
fields but renders the model as `"<redacted>"`; `LoadedNativeConfig` inherits
that redaction through its nested configuration.

`ai_gateway_http` is a declarative transport selection in the configuration
schema. Its enum exists independently of the optional `ai-gateway-http` Cargo
feature and on targets where that feature's concrete Reqwest transport exports
are absent, including WebAssembly. Parsing this value therefore proves only
that configuration is valid; it does not prove transport availability,
construct a Tokio runtime, or make a network path usable.

Vercel AI Gateway credentials are fields in neither schema. The separately
integrated [`native credential discovery adapter`](ai-gateway-credentials.md)
owns its non-cloneable secret snapshot and does not put secret values into
`NativeConfig`, debug output, status output, or the configuration file.

## Bounds and filesystem boundary

The raw file limit remains 64 KiB (65,536 bytes). A file of exactly that length
can be considered for parsing; any additional byte makes it oversized. Bytes
must be valid UTF-8 and then valid strict v1 or v2 JSON. The loader retains at
most 64 KiB plus one byte while deciding whether input fits, so neither a stale
size observation nor concurrent file growth turns loading into an unbounded
read.

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path with no-follow and nonblocking behavior. It performs a preliminary
path-kind check, then authoritatively validates the opened descriptor as a
regular file before reading it. It therefore rejects a final symlink,
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
invalid model values, and unsupported permission/provider/transport values are
grouped as invalid format. Diagnostics do not reflect environment-derived
paths, configuration bytes, model values, or operating-system error text. All
listed failures fail closed.

## Relationship to status and deferred work

Native status remains a separate metadata-only observation. It still uses
final-path metadata to report config-file and state-directory states, does not
read or parse `config.json`, and reports permission mode `ask`. Existing CLI
help, version, status, error, and bare-invocation bytes remain unchanged; no CLI
command loads configuration in this slice.

Provider, transport, and model fields are declarative data only. This slice
does not instantiate `AiGatewayProvider`, select or construct an HTTP client,
create or drive a Tokio runtime, discover or attach a credential, open a
network connection, or compose any component into core or the CLI.

The separate twelfth
[`native reference-host candidate`](native-reference-host.md) consumes an
already loaded value without changing this loader. It retains the exact
`LoadedNativeConfig`: an accepted file-backed v1 therefore remains observable
with file origin and schema version `1`, while its existing fixed in-memory
provider, transport, and model projection drives composition. The candidate's
production constructor receives a separate injected credential snapshot; no
credential enters this configuration value or loader. Production implementation
and focused independent tests are composed, and three fresh adversarial tracks
are green on exact behavior SHA `5afda631`; remote delivery gates remain
pending.

Configuration mutation, a migration or rewrite command, a terminal permission
prompter and modes beyond `ask`, runtime and CLI composition, required
workspace and state-root lifecycle, the remaining native tools, CLI and
session expansion, release-binary end-to-end host evidence, and compatibility
or performance claims remain open. This slice and the twelfth composition
candidate do not complete the combined credential-and-configuration checklist
item because v2 has no bounded credential-source field. Milestone 03 remains
in progress.
