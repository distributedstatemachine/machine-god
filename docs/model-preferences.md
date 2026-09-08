# Native model preferences

`NativeModelPreferences` is a pure, bounded value and metadata codec. It does
not load configuration, fetch catalogs, write user settings or session records,
queue prompts, or change an active turn. Runtime owners must explicitly persist
changes and freeze their per-turn snapshots; this module makes no durability or
full slash-command completion claim.

The reserved `machine_god.model_preferences` metadata entry has exactly four
required fields:

```json
{"schema_version":1,"model":"zai/glm-5.2","effort":"auto","fast_mode":false}
```

Absent metadata returns `None`, not manufactured historical defaults. Explicit
`Default` uses the built-in Gateway model, automatic effort and requested fast
off. Decoding borrows the entry, checks fixed fields directly, never clones or
recursively visits untrusted values, and ignores unrelated metadata. Unknown
versions, unknown/missing fields, wrong types and invalid values fail with fixed,
redacted errors. All preference and capability debug output is redacted.

Model validation uses the shared [model-ID contract](ai-gateway.md): 1–1024
UTF-8 bytes, no ASCII C0/DEL or edge SP/TAB/CR/LF. Values are not normalized.
`NativeReasoningEffort::parse` normalizes ASCII-case-insensitive `auto`,
`adaptive`, and `default` to automatic. Other names contain 1–64 ASCII
alphanumeric, hyphen, underscore or dot bytes and retain their exact case.
`as_named` returns `None` for automatic; `label` returns canonical `auto` or
the opaque name. Named efforts are not a fixed low/medium/high enum.

`NativeModelCapabilities` copies at most sixteen already validated named
efforts, preserving order and duplicates; automatic is implicit and cannot be
an advertised named entry. Fast support is an explicit boolean. Neither model
IDs nor tags imply either control. Effective preferences borrow the requested
value, include reasoning only on an exact advertised match, and enable fast
only when both requested and supported. Requested values are not overwritten
by this projection.

Direct model changes preserve requested effort and fast. Fast toggling always
allows an existing true value to become false; unsupported false-to-true leaves
all preferences unchanged and returns `Unsupported`. Picker-default selection
sets automatic effort when reasoning choices exist and enables fast when
supported, preserving unsupported controls. Invalid model selection leaves all
preferences unchanged. Explicit picker-choice orchestration belongs to the
runtime owner, not this default-selection helper.

These rules follow pinned revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`: effort definitions in
`src/core/shared/types.zig:1561`, effective controls in
`src/core/config/model_capabilities.zig:192`, direct selection and fast behavior
in `src/core/session/session_commands.zig:685` and `:760`, and picker defaults
in `src/core/app/input_completion_runtime.zig:1138`.
