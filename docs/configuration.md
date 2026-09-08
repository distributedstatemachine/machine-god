# Native configuration

The native configuration loader is a bounded, synchronous, read-only
`machine-god-native` authority. Core remains independent of the process
environment and filesystem. After complete command-local argument validation,
the implemented `machine-god status [--json]` path invokes the loader exactly
once as part of native runtime-status inspection. The implemented
`machine-god permissions [--json]` path also invokes it exactly once after
complete argument validation and observes only permission mode.

The [`models [--json]` implementation](models-cli.md) invokes this loader exactly
once after complete argument validation. It validates the closed provider,
transport, and credential-source selections before native credential or
network access, never reloads configuration for public fallback, never changes
the configured generation model, and never writes or migrates the file. That
catalog path accepts the built-in or missing-file safe schema-v5 defaults and
strict v1/v2/v3/v4/v5 files, but rejects any config-load failure before credential
discovery. It does not add an endpoint, team, token, cache, or catalog field to
schema v5.

The configuration contract advances the built-in and current file
schema to v5 while retaining strict read compatibility for the exact legacy v1,
v2, v3 and v4 objects. Loading is still read-only; explicit user-default publication
is a separately granted native effect described below.

## Location and defaults

The loader resolves only the configuration portion of an injected environment
snapshot:

- a nonempty `XDG_CONFIG_HOME` is selected and must be absolute Unicode;
- an empty `XDG_CONFIG_HOME` falls back to a nonempty, absolute-Unicode `HOME`;
- the resolved path is `<XDG_CONFIG_HOME>/machine-god/config.json`, or
  `<HOME>/.config/machine-god/config.json` for the fallback; and
- a selected nonempty relative or non-Unicode value is invalid. Selection fails
  without trying a different environment value.

The retained legacy `NativeStatus` metadata API snapshots `XDG_CONFIG_HOME`,
`XDG_STATE_HOME`, and `HOME` because it reports both config and state location
metadata. It is not the snapshot used by the CLI status command. The
configuration loader used by `NativeRuntimeStatus` and by permissions requests
`XDG_CONFIG_HOME` first and requests `HOME` only when XDG is missing or empty;
it never requests `XDG_STATE_HOME`. A nonempty XDG value decides selection
whether it is valid, relative, or non-Unicode, so that path neither reads nor
falls back to `HOME`.

An unavailable location, including a missing or empty needed `HOME`, produces
the explicit built-in schema-v5 configuration. A resolved file that is missing
also produces this configuration:

```json
{"schema_version":5,"permission_mode":"ask","sandbox_mode":"none","permission_rules":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","effort":"auto","fast_mode":false}
```

Invalid selected environment input is not treated as absence and fails closed.
Likewise, an inaccessible resolved path is an error rather than a reason to
silently use defaults.

## Strict schema v3

Schema v3 remains an accepted legacy format; it projects `effort: auto` and
`fast_mode: false` without changing its source schema label or file bytes.

A present schema-v3 configuration is one JSON object containing exactly these
six required fields:

| Field | Required value |
| --- | --- |
| `schema_version` | JSON integer `3` |
| `permission_mode` | JSON string `"ask"` |
| `provider` | JSON string `"vercel_ai_gateway"` |
| `transport` | JSON string `"ai_gateway_http"` |
| `model` | JSON string of 1–1024 UTF-8 bytes under the shared core model-ID validator |
| `credential_source` | JSON string `"environment"` |

Empty models, ASCII C0 (`0x00`–`0x1f`) or DEL (`0x7f`) bytes, leading or trailing
SP/TAB/CR/LF, and values longer than 1024 UTF-8 bytes are invalid. Interior
spaces, non-ASCII text, and Unicode C1 characters are accepted unchanged.
Validation is byte-based, not Unicode `is_control` or Unicode trimming. The
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

Every integer schema version other than `1`, `2`, `3`, `4`, or `5` is unsupported. A
missing, duplicate, non-integer, or otherwise malformed schema-version field is
invalid format. Full-buffer UTF-8 validation still precedes schema dispatch.

## Public data boundary

`CONFIG_SCHEMA_VERSION` is `5`. `AI_GATEWAY_DEFAULT_MODEL` is
`"zai/glm-5.2"`, and `AI_GATEWAY_MAX_MODEL_BYTES` aliases core's
`MAX_MODEL_ID_BYTES` (`1024`). The shared `validate_model_id` contract matches
the pinned settings and durable-session model validators.
`NativeProviderKind::VercelAiGateway` has stable machine name
`vercel_ai_gateway`; `NativeTransportKind::AiGatewayHttp` has stable machine
name `ai_gateway_http`; and `NativeCredentialSourceKind::Environment` has stable
machine name `environment`. Their `as_str` accessors return those names. They
do not imply that an optional implementation is compiled or usable in the
current build.

`NativeConfig` exposes read-only `schema_version`, `permission_mode`, `sandbox_mode`,
`permission_rules`,
`provider`, `transport`, `model`, `credential_source`, `effort`, `fast_mode`, and
complete typed `model_preferences` getters. The schema
version remains the version actually loaded, including `1` or `2` for a legacy
file. Provider, transport, and credential source return closed native enums;
model returns the validated string.
`NativeConfig` and `LoadedNativeConfig` implement `Clone`, but not `Copy`,
because configuration owns its bounded model string and ordered rules. `NativeConfig` debug
output exposes the non-secret schema, permission, provider, transport, and
credential-source fields but renders the model and permission rules as `"<redacted>"`;
`LoadedNativeConfig` inherits that redaction through its nested configuration.

`ai_gateway_http` is a declarative transport selection in the configuration
schema. Its enum exists independently of the optional `ai-gateway-http` Cargo
feature and on targets where that feature's concrete Reqwest transport exports
are absent, including WebAssembly. Parsing this value therefore proves only
that configuration is valid; it does not prove transport availability,
construct a Tokio runtime, or make a network path usable.

Vercel AI Gateway credential bytes are fields in no schema. The separate
[`native credential discovery adapter`](ai-gateway-credentials.md)
owns its non-cloneable secret snapshot and does not put secret values into
`NativeConfig`, debug output, status output, or the configuration file.

## Bounds and filesystem boundary

The raw file limit remains 64 KiB (65,536 bytes). A file of exactly that length
can be considered for parsing; any additional byte makes it oversized. Bytes
must be valid UTF-8 and then valid strict v1, v2, v3, v4, or v5 JSON. The loader retains
at most 64 KiB plus one byte while deciding whether input fits, so neither a
stale size observation nor concurrent file growth turns loading into an
unbounded retained buffer. The read loop retries the first 15 cumulative
`Interrupted` results and maps the 16th to the fixed `Unreadable` error. Partial
progress does not reset the count, and an over-reported read maps to
`Unreadable`.

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

The retained `NativeStatus` API remains a separate metadata-only observation.
It uses final-path metadata to report config-file and state-directory states,
does not read or parse `config.json`, and reports permission mode `ask`. The CLI
does not use that legacy snapshot for `status`.

After successful status parsing, the CLI delegates to `NativeRuntimeStatus`.
That native inspection loads the strict configuration exactly once, classifies
the selected environment credential without retaining or reporting its secret
bytes, and canonicalizes the current directory as the bounded workspace. It
does not inspect a state root, invoke `NativeRootSelection`, prepare any root,
create or modify a file or directory, construct a runtime, or use the network.
The separate permissions command continues to load configuration once after
valid permissions parsing.

Provider, transport, model, and credential-source fields are declarative data
only. `environment` tells the production reference-host constructor which
already injected acquisition adapter is compatible with this configuration; it
does not read the process environment. Configuration loading does not instantiate
`AiGatewayProvider`, select or construct an HTTP client, create or drive a
Tokio runtime, discover or attach a credential, open a network connection, or
compose any component into core or the CLI.

The [`native reference host`](native-reference-host.md) consumes an
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

The [`native root-selection contract`](native-root-selection.md) derives a state
root from the same injected snapshot but does not change configuration or use
the loader. Its preparation authority remains independent of this read-only
surface; loading never rewrites built-in or file-backed configuration.

An independent migration or rewrite command, a terminal permission
prompter and runtime mode enforcement, runtime composition, session lifecycle, the
remaining native tools, remaining CLI and session expansion, release-binary
end-to-end host evidence, and compatibility or performance claims remain
outside this configuration contract.

## Strict schema v4 read compatibility

Schema v4 has exactly the six v3 fields plus required `effort` (string) and
`fast_mode` (boolean). Model validation is unchanged. Effort uses the shared
[`NativeReasoningEffort`](model-preferences.md) contract: `auto`, `adaptive`,
and `default` are case-insensitive automatic aliases; named efforts preserve
case and contain 1–64 ASCII letters, digits, hyphens, underscores or periods.
Fast mode is the requested preference, not a claim that the selected model
supports it. Legacy v1/v2/v3 inputs project automatic effort and disabled fast
mode in memory while retaining their exact loaded schema versions. Strict
unknown-field, duplicate-field and shape rejection remains unchanged.

## Schema v5 permission preferences

Schema v5 requires all eight v4 fields plus `sandbox_mode` and
`permission_rules`. `permission_mode` accepts exactly `ask`, `auto`, or `yolo`;
`sandbox_mode` accepts exactly `os` or `none`. Their typed accessors return
`PermissionMode::{Ask, Auto, Yolo}` and `NativeSandboxMode::{Os, None}` with
matching `as_str` names. Defaults are `ask` and `none`, matching pinned
`sandbox.zig::backendFromConfig` when no sandbox setting exists. An explicit
schema-v5 `os` remains `os`; permission mode does not rewrite the preference.

`permission_rules` is an ordered JSON array, including an empty array. Each
entry has exactly three required string fields: `permission`, `pattern`, and
`action`; action accepts only `allow`, `ask`, or `deny`. Unknown or duplicate
fields, nulls, wrong types, and missing fields are rejected. The existing
`NativeConfiguredPermissionRules` value preserves order and repeated rules;
its constructors trim only SP/TAB/CR/LF around permission and pattern, reject
an empty permission, and retain empty patterns. Its bounded evaluator uses
last-match semantics; these configured patterns are not saved exact-action
rules or persisted grants. Debug output never reveals pattern contents.

Schemas 1–4 remain strictly ask-only and reject both new fields. They project
`sandbox_mode: none` and empty rules in memory without rewriting their bytes or
changing their observed version. Provider and model defaults are unchanged.

These fields are preferences, not enforcement or execution authority. Loading
`os` does not establish an active OS sandbox, and loading `auto`, `yolo`, or
an allow rule does not itself approve or execute a tool. Runtime policy and
platform enforcement remain separate native responsibilities.

The 64 KiB bound covers the entire configuration, including every rule and
JSON escaping, not just the rule array. Current-schema serialization is also
bounded: increasing model preferences beyond the complete encoded limit
returns `InvalidConfig(TooLarge)` before creating a publication temp or replacing
the original file. Exactly 64 KiB remains accepted.

## Durable user model-default publication

On Linux and macOS, `NativeUserConfigStore::new` receives an explicit absolute
configuration-directory path and is inert. `load` is read-only: it retains an
existing parent descriptor and any existing final-directory descriptor, reads
at most 64 KiB plus one byte from a regular final no-follow `config.json`, and
returns a redacted `NativeUserConfigSnapshot`. Missing final directories/files
return explicit built-in defaults; unavailable parents and invalid files are
errors. No environment inference or recursive parent creation occurs.

The directory must be owned by the effective user and private (no group/other
mode bits); macOS also rejects granting extended ACLs. Existing configuration
files must be singly linked, owned regular files without group/other write
permission. Ancestor components are not recursively frozen: operations retain
the granted parent descriptor, so replacement of its pathname never redirects
an already loaded snapshot to a different directory.

`set_model_preferences(snapshot, preferences)` is inert until polled and
performs one bounded synchronous transaction without detached tasks. It binds
the token to its originating store instance, checks the final-directory
identity, and may create only the missing final directory with mode 0700 under
the retained parent. It takes a private, no-follow `.config.lock` via
nonblocking exclusive flock; contention returns `Busy`. The lock persists and
is never unlinked. Under this lock it rereads and validates the exact current
bytes; stale or foreign snapshots return `Conflict` without overwriting them.

Only the requested model/effort/fast fields change. Existing validated provider,
transport, permission mode, sandbox preference, ordered permission rules and
credential-source selections are retained, and the
explicit publication upgrades supported legacy formats to schema v5 in the
same `config.json`. Malformed or future configurations are never overwritten.
Publication exclusively creates `.config.tmp` with mode 0600, writes and fsyncs
it, rechecks entry identities and current bytes, renames it atomically, and
fsyncs the retained directory. Existing temp artifacts are never adopted or
deleted. A failed pre-rename publication leaves the original configuration
authoritative; an error after replacement is `CommitAmbiguous`, requiring a
fresh observation rather than automatic retry. Only owned unpublished temps
are cleaned up after ordinary write failures. Cooperative writers serialize
through the persistent lock; arbitrary same-user actors replacing entries
outside this protocol are not granted a general transactional guarantee.

This user-default target is independent of session preference persistence.
Callers must report the two outcomes separately, matching pinned
`src/core/app/app_session_runtime.zig::commitRuntimePreferences` and
`src/core/session/session_commands.zig` settings-result reporting. A user-file
failure does not imply runtime selection or session persistence failed, and a
session failure must not suppress an explicitly requested user-default attempt.
