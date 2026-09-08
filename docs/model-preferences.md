# Native model preferences

`NativeModelPreferences` is a pure, bounded value and metadata codec. It does
not load configuration, fetch catalogs, write user settings or session records,
queue prompts, or change an active turn. `NativeModelSnapshot` captures its
requested and effective values for an admitted job; the
[native conversation owner](native-conversation.md) explicitly persists session
preferences and installs these snapshots on prepared turns. Its native runtime
owns queued/current selection and deferred session writes. User-default
persistence and full slash-command integration remain separate.

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

## Immutable job snapshots

`NativeModelSnapshot::new(preferences, capabilities)` owns a bounded copy of the
requested settings and resolves effective Gateway controls once. It borrows no
mutable catalog or runtime state. `preferences` exposes the requested value by
shared reference; `apply_to` installs the captured model and effective controls
on inference options. Token limits, temperature and unrelated metadata remain
unchanged, and any replaced reserved JSON tree is destroyed iteratively.
Unsupported effort/fast requests survive in the requested value for persistence,
but replace stale effective controls with automatic effort and fast off.

The queue owner must rewrite pending jobs when selection changes and capture a
snapshot when taking a job. Already-taken jobs retain their snapshot across all
provider rounds. An explicit paused continuation is a new job using current
selection, not the interrupted turn's old model/effort. Recovery route, effective
fast downgrade and attempt-budget facts are separate from model preferences;
this snapshot does not implement that recovery policy. These distinctions follow
pinned `src/core/agent/worker_runtime.zig:728`, `:1006`, `:1161`, `:1180`,
`:1194`, `:4034`, `src/main.zig:1221`, `:1250`, and
`src/core/agent/runtime/orchestrator.zig:2563`.

These rules follow pinned revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`: effort definitions in
`src/core/shared/types.zig:1561`, effective controls in
`src/core/config/model_capabilities.zig:192`, direct selection and fast behavior
in `src/core/session/session_commands.zig:685` and `:760`, and picker defaults
in `src/core/app/input_completion_runtime.zig:1138`.

## Pure model-query resolution

`resolve_model_query(query, catalog)` returns the selected catalog ID's exact
bytes, or `None`. The caller owns cache/fetch ordering and raw-query fallback;
this helper performs none of those effects. An empty query does not match.
The query shares the native slash-input bound of 65,536 UTF-8 bytes, not the
1,024-byte durable model-ID limit or the interactive picker's separate 256-byte
buffer. Long queries and queries containing controls can still select a valid
catalog ID through token matching. Validation as a durable ID belongs only to
raw-query fallback; UI output must escape the original untrusted query.
Before matching, the helper checks the existing 512-entry and 24 KiB aggregate
ID bounds even for an early exact match or empty query. The only matching-time
allocation is one bounded copy of the selected ID.

ASCII-case-insensitive exact equality wins first, retaining the first catalog
spelling. Otherwise the first highest positive fuzzy score wins ties:

| Match | Score |
| --- | --- |
| Case-sensitive prefix or ASCII-case-insensitive slash-delimited suffix | 120 + min(query bytes, 50) |
| Other ASCII-case-insensitive substring | 100 + min(query bytes, 50) |
| Every token present, with at least two tokens | 80 + 5 × token count |
| Byte subsequence | 40 + min(matched bytes, 20) |
| Any token present | 20 |
| No match | 0 |

Only space, hyphen, slash and underscore split tokens; at most sixteen nonempty
tokens are used. Non-ASCII bytes remain opaque: no Unicode case folding,
normalization, character scoring or invented provider/model grammar occurs.
The pinned pure resolver does not trim edge spaces, and neither does this
helper. Its caller should pass the routed payload: the pinned slash router
separately trims SP/TAB (`src/core/slash_commands/command_router.zig:100`).
An unmatched query remains the caller's original string; callers must validate
it as a model ID before accepting it as a persistent preference.

Scoring and exact-first resolution follow pinned
`src/core/session/session_commands.zig:1489` and `:1142`; caller-owned
cache/fetch/raw-fallback ordering is at `:1010`. Token scanning uses a fixed
sixteen-slot borrowed array, and matching does not infer model capabilities.
