# Native configuration

Milestone 03 includes a bounded, read-only native configuration loader. It is a
synchronous `machine-god-native` authority: core remains independent of the
process environment and filesystem, and the CLI does not invoke the loader.

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
the explicit built-in configuration. A resolved file that is missing also
produces that configuration:

```json
{"schema_version":1,"permission_mode":"ask"}
```

Invalid selected environment input is not treated as absence and fails closed.
Likewise, an inaccessible resolved path is an error rather than a reason to
silently use defaults.

## Schema v1

A present configuration is one JSON object containing exactly these two
required fields:

| Field | Required value |
| --- | --- |
| `schema_version` | JSON integer `1` |
| `permission_mode` | JSON string `"ask"` |

Unknown or duplicate fields, missing fields, wrong JSON types or shapes,
unsupported schema versions, and permission modes other than `ask` are errors.
There is no forward-compatible field ignoring or value coercion in this slice.
AI Gateway credentials are not schema-v1 fields. The separate opt-in
[`native credential discovery adapter`](ai-gateway-credentials.md) owns a
dedicated non-cloneable secret snapshot and does not put secret values into
`NativeConfig`, status output, or the configuration file.

The raw file limit is 64 KiB (65,536 bytes). A file of exactly that length can
be considered for parsing; any additional byte makes it oversized. Bytes must
be valid UTF-8 and then valid schema-v1 JSON. The loader retains at most 64 KiB
plus one byte while deciding whether the input fits, so neither a stale size
observation nor concurrent file growth turns loading into an unbounded read.

## Filesystem boundary

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path with no-follow and nonblocking behavior. It performs a preliminary
path-kind check, then authoritatively validates the opened descriptor as a
regular file before reading it. It therefore rejects a final symlink,
directory, FIFO, socket, device, or other non-regular entry and does not block
opening a hostile FIFO. Hardened open semantics for non-Unix targets are not
part of this milestone's supported contract.

Loading never writes configuration, creates files or directories, or
canonicalizes the selected path. It does not claim to validate or freeze the
entire ancestor path: on supported Unix targets, the bounded guarantee in this
slice is final-component no-follow plus descriptor regularity.

Errors are typed so callers can distinguish invalid environment input,
open/read failure, invalid file kind, size overflow, invalid format, and an
unsupported schema version. Invalid UTF-8, malformed JSON, schema-shape errors,
and unsupported permission modes are grouped as invalid format. Diagnostics do
not reflect environment-derived paths, configuration bytes, or operating-system
error text. All listed failures fail closed.

## Relationship to status and deferred work

Native status remains a separate metadata-only observation. It still uses
final-path metadata to report config-file and state-directory states, does not
read or parse `config.json`, and reports permission mode `ask`. Existing CLI
help, version, status, error, and bare-invocation bytes remain unchanged; no CLI
command loads configuration in this slice.

Configuration mutation, credential fields, permission prompting and modes
beyond `ask`, concrete providers and tools, durable native sessions, broader
CLI behavior, and
compatibility or performance claims remain deferred. This loader is not a
claim that Milestone 03 is complete.
