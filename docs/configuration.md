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
catalog path accepts the built-in or missing-file safe schema-v7 defaults and
strict v1/v2/v3/v4/v5/v6/v7 files, but rejects any config-load failure before credential
discovery. It does not add an endpoint, team, token, cache, or catalog field to
the configuration schema.

The configuration contract advances the built-in and current file
schema to v7 while retaining strict read compatibility for the exact legacy v1,
v2, v3, v4, v5 and v6 objects. Loading is still read-only; explicit user-default publication
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
the explicit built-in schema-v7 configuration. A resolved file that is missing
also produces this configuration:

```json
{"schema_version":7,"permission_mode":"ask","sandbox_mode":"none","permission_rules":[],"workspace_permission_rules":[],"workspace_directories":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","effort":"auto","fast_mode":false}
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

Every integer schema version other than `1`, `2`, `3`, `4`, `5`, `6`, or `7` is unsupported. A
missing, duplicate, non-integer, or otherwise malformed schema-version field is
invalid format. Full-buffer UTF-8 validation still precedes schema dispatch.

## Public data boundary

`CONFIG_SCHEMA_VERSION` is `7`. `AI_GATEWAY_DEFAULT_MODEL` is
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
must be valid UTF-8 and then valid strict v1, v2, v3, v4, v5, v6, or v7 JSON. The loader retains
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
configuration-directory path and is inert. The namespace is bounded to 4,096
raw Unix path bytes and 64 components. `load` is read-only: it retains the
nearest existing ancestor descriptor and identity, unresolved parent components,
and any existing final-directory descriptor, reads
at most 64 KiB plus one byte from a regular final no-follow `config.json`, and
returns a redacted `NativeUserConfigSnapshot`. Missing final directories/files
return explicit built-in defaults, including when configuration parents do not
exist. Unsafe or unavailable ancestors and invalid files remain errors. No
environment inference, directory creation, or lock creation occurs on load or
an observed no-op permission/workspace edit.

The directory must be owned by the effective user and private (no group/other
mode bits); macOS also rejects granting extended ACLs. Existing configuration
files must be singly linked, owned regular files without group/other write
permission. Existing ancestors retain their existing ownership/mode policy;
ordinary 0755 user parents and explicitly selected temporary parents are not
required to be private. The observed ancestor identity and subsequently opened
no-follow descendant links are revalidated, so a replaced ancestor or injected
symlink cannot redirect an already loaded snapshot.

`set_model_preferences(snapshot, preferences)` is inert until polled and
performs one bounded synchronous transaction without detached tasks. It binds
the token to its originating store instance, checks the final-directory
identity, and only after candidate validation may create the required missing
configuration parents and final directory with mode 0700 under the retained
ancestor. Newly created parent directories and their directory entries are
synced before publication continues. A failure may leave created empty
directories; it does not claim they were rolled back. It takes a private,
no-follow `.config.lock` via
nonblocking exclusive flock; contention returns `Busy`. The lock persists and
is never unlinked. Under this lock it rereads and validates the exact current
bytes; stale or foreign snapshots return `Conflict` without overwriting them.
An acquired scoped guard explicitly unlocks on every exit, retrying interrupted
lock operations. Closing the local descriptor alone is insufficient when a
concurrent spawn or descriptor duplicate retains its open-file description.
Genuine contention still returns `Busy`; locks are not externally reset or unlinked.

Only the requested model/effort/fast fields change. Existing validated provider,
transport, permission mode, sandbox preference, saved workspace directories,
global and workspace permission rules and
credential-source selections are retained, and the
explicit publication upgrades supported legacy formats to schema v7 in the
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

## Schema v6 workspace permission sources

Schema v6 retains all v5 fields and requires `workspace_permission_rules`, an
array of objects with exactly `workspace_hex` and `permission_rules`. The first
is canonical lowercase hexadecimal encoding of normalized absolute Unix path
bytes; the second uses the same ordered rule-array contract as the global
`permission_rules`. The decoded path is at most 4096 bytes, contains no NUL,
empty components, `.` or `..`, and has no trailing separator except `/` itself.
Non-UTF-8 workspace bytes are preserved losslessly. Duplicate workspace keys,
unknown/duplicate object fields, invalid hex and malformed rules are rejected.
Configuration parsing remains available independently of native runtime support;
it does not open or canonicalize any workspace path.

The entire configuration, including every workspace, rule and JSON overhead,
shares the existing 64 KiB bound. Limits are not multiplied per workspace.
Legacy v1–v5 files have no local entries in memory and retain their original
schema labels. Their new-field rejection remains strict. A successful explicit
write upgrades to the current schema; reads and unchanged permission edits never upgrade bytes.

`NativeConfig::permission_sources(workspace)` takes an already-normalized host
workspace label and returns borrowed `user`, optional `local`, and `effective`
rule lists plus `user_shadowed_by_local`. An absent local entry inherits the
global list. A present local entry replaces the whole global list, including
when local is explicitly empty; sources are never merged. The existing
`permission_rules()` getter remains the global/user list. These observations
do not grant tool or filesystem authority and do not install runtime policy.
`has_workspace_permission_rules()` reports presence of any local source,
including an explicit empty entry, without exposing its contents or path.

## Durable configured-permission edits

`NativeUserConfigStore::apply_permission_mutation` receives its exact snapshot,
the selected workspace, `User` or `Local` scope, and an explicit `Add`, `Remove`
or `Reset` storage operation. Categories and patterns must already be canonical;
the store validates bounds and surrounding whitespace but does not parse slash
commands, validate registered tool names or establish human consent. Both scopes
are stored in the same granted user configuration file, not repository files
or session metadata.

Add updates the last exact category/pattern to Allow, or appends a new rule;
an already-Allow last match is unchanged. Earlier native duplicate rules keep
their order. Remove deletes all exact matching rows, including Ask/Deny, and
retains an empty local entry when deleting its final row. Reset deletes only
Allow rows: commands select `bash`, URLs select `url`, `open_url` and
`browser_navigate`, fetch domains select `web_fetch`, and tools exclude those
categories and `*`. All selects every category. Ask/Deny rules survive reset.
If reset actually removes rows and empties a local list, the local entry is
removed and user rules become effective again. Reset on an already-empty local
list is unchanged and preserves its explicit empty shadow.

The future is inert before polling and creates no detached writer. The complete
candidate is validated and serialized before directory, lock or temp creation.
Changed edits reuse the model-default store's exact-byte CAS, nonblocking lock,
private temp, rename and directory durability checks, preserving other
workspaces and unrelated model/policy fields. Model-default edits likewise
preserve all local permission entries and saved workspace directories. Unchanged edits validate observed
store/root/bytes without creating a lock or writing anything; this is an
observation, not a reservation against future concurrent changes.

`NativeUserPermissionCommit` contains `Changed { removed_rules }` or `Unchanged`
and the resulting `LoadedNativeConfig`. Removed counts refer to actual native
rows; Add reports zero. A Changed success means durable publication was
confirmed, not that a later runtime reload succeeded. Prepublication failures
leave original bytes authoritative; failures after rename remain
`CommitAmbiguous` and require fresh observation rather than automatic retry.
Callers must preserve the durable result independently from reload/application
failures. Receipt and source Debug output do not expose workspace or rule text.

## Schema v7 saved workspace directories

Schema v7 retains all v6 fields and requires `workspace_directories`, an array
of objects with exactly `workspace_hex` and `additional_directories`. The latter
is an ordered array of records with exactly `source_hex`, `identity_hex` and
boolean `identity_canonical`. All three path fields use canonical lowercase
hexadecimal encoding of raw Unix bytes, including non-UTF-8 bytes. Each decoded
path is absolute, at most 4,096 bytes, and contains no NUL, empty components,
`.` or `..`; only `/` itself may end in a separator. Duplicate primary keys,
duplicate sources or identities within a primary, and a source or identity
equal to its primary are rejected. Unknown, missing, duplicate and wrong-type
fields remain errors. At most 16 saved identities belong to one primary; the
entire configuration still shares the 64 KiB encoded bound.

`NativeSavedWorkspaceDirectory::new(source, identity, identity_canonical)`
validates and copies bounded raw byte slices. Read-only `source_bytes`,
`identity_bytes` and `identity_canonical` getters preserve the record exactly.
`NativeConfig::saved_workspace_directories(primary)` accepts a raw-byte primary
and returns its borrowed saved slice, or an empty slice for an absent entry.
Parsing never probes, canonicalizes or opens these paths. Unavailable saved
sources remain roundtrippable; an identity-canonical flag is a retained host
observation, not a filesystem availability check or execution grant. Legacy
v1–v6 files project no saved directory entries and preserve their source schema.

## Latest-state workspace-directory publication

On Linux and macOS, `apply_workspace_directory_mutation(primary, mutation, launch_identities)`
accepts `Add(NativeSavedWorkspaceDirectory)`, `Remove(identity_bytes)` or
`Clear`. Unlike the model/permission APIs, it takes no snapshot token: a changed
operation rereads and merges the latest validated configuration under the same
nonblocking cooperative writer lock. Unrelated fields, other primaries and
concurrent same-list additions are preserved. Add appends a new identity;
an already-present identity is unchanged and retains its original source.
A reused source with a different identity is rejected, not silently retargeted.
Remove selects exact retained identity, never its current filesystem target;
an unknown identity is unchanged. Clear affects only the selected primary.
Removing the last record removes that primary's directory entry.

The caller supplies the staged surviving launch identities as bounded raw-byte
paths (empty for standalone administration and Clear). These must be normalized,
unique, nonprimary paths, with at most 16 entries. Their union with the candidate's
saved identities must fit the same 16-identity cap. The store checks this both
before acquiring publication authority and again against the latest state under
the lock, preventing concurrent saved additions from bypassing effective capacity.

The future is inert before polling and creates no detached worker. Request
validation precedes filesystem effects. An initial observational no-op creates
no directory, lock or temp and preserves bytes and schema. It is not a
reservation against later changes. If a genuinely changed preflight becomes
unchanged after lock acquisition, the lock may have been created, but no temp,
config rewrite or schema upgrade occurs. Changed publication retains the
existing descriptor, private-entry, bounded-write, exact-byte recheck, rename
and directory-sync checks. Existing foreign temp files are not removed.
The model and configured-permission APIs retain their separate exact-snapshot
CAS contracts unchanged, and their writes preserve saved directory records.

`NativeUserWorkspaceCommit` carries `before` and `after` selected saved sets,
`changed`, `loaded`, and `NativeWorkspaceCommitDurability::{Confirmed, Ambiguous}`.
The `loaded` value is the intended merged candidate, not a fresh post-commit
observation. An ambiguous receipt means replacement occurred but durability or
the final root link could not be confirmed; its retained previous/intended sets
let the owner perform a fresh reload and reconciliation rather than retrying
automatically. Errors before replacement are fixed and redacted. Receipt Debug
does not expose directory bytes. Storage does not install runtime roots, resolve
launch paths or grant tools authority; those remain explicit native host
responsibilities.
