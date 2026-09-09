# Native `permissions` CLI contract

The read-only top-level command reports configured permission mode and ordered
user/local pattern rules. Native code owns configuration loading, workspace
selection, source provenance and inert-row classification. This is not a live
authorization snapshot: saved exact-action rules and runtime grants remain
explicitly unavailable, never represented as empty observed collections.

## Command grammar

The accepted forms remain:

```text
machine-god permissions
machine-god permissions --json
```

Unknown, repeated, reordered, additional or non-Unicode arguments fail before
host inspection: exit 2, empty stdout and the shared invalid-arguments diagnostic.
The exact global help/usage belongs to [the CLI contract](cli.md). Help and
identity commands do not invoke permission inspection. Interactive
`/permissions` controls remain separate from this read-only command.

## Authority and loading

`inspect_process_permissions()` calls `load_process_config()` once. The existing
strict schema-v1 through schema-v7 parser, 64 KiB input plus overflow-witness
bound, cumulative interrupted-read bound, and supported-Unix final-path
`O_NOFOLLOW`/nonblocking/regular-file checks remain unchanged. Invalid selected
configuration never falls back to defaults. Missing configuration or unavailable
configuration location retains safe built-in defaults.

The config-only environment snapshot requests `XDG_CONFIG_HOME` first and
`HOME` only for missing or empty XDG. It never requests `XDG_STATE_HOME`.
Nonempty invalid XDG never falls back to HOME. No state root, session store,
engine, credential reader, provider, network, prompter or Tokio runtime is
constructed; no directories, locks or files are created or changed.

When any workspace-local sources exist, native code captures and canonicalizes
the process CWD once, then uses the existing exact workspace-byte source
selection. Symlink aliases select the canonical workspace; Unix non-Unicode
paths retain their exact bytes without displaying a lossy path. A missing or
invalid CWD fails rather than silently reporting user rules as effective.
When no local sources exist, CWD is not observed: user rules are authoritative
for configuration selection even if the process has no usable working directory.
This bounded synchronous observation promises no filesystem deadline.

The pure `inspect_native_permissions(&LoadedNativeConfig, &Path)` adapter accepts
explicit normalized absolute workspace bytes and performs no filesystem I/O.
Its retained report contains only mode, config origin and bounded rule
projections, never unrelated settings or a raw configuration/workspace path.
Neither adapter grants execution authority.

## Sources and rows

The native report preserves user and selected-local lists independently and in
configuration order. A present empty local list shadows the user list. An absent
local list is distinct from empty; in that case the effective source is user.
The source is reported once rather than duplicating an effective list.
Unselected workspaces' lists are not displayed.

Every configured allow, ask and deny row is included. Exact `web_fetch` rows
whose patterns fail the existing canonical-domain predicate are marked inert,
with their pattern omitted. Their decision and position remain visible. Other
rows are labeled configured patterns, not claims that a live tool registry would
match them. No second permission matcher or independent allowlist policy exists.

These configured patterns are distinct from session-persisted exact-action
rules and identity-bound runtime grants. The report does not load those stores
or assert their contents. It does not mutate modes, patterns, rules or grants,
and does not reload an interactive runtime.

## Output and errors

Default human output is:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
configuration_origin: built_in_defaults
configured_rules_source: user
user_rules: 0
local_rules: absent
saved_exact_rules: unavailable
runtime_grants: unavailable
```

File-loaded configuration reports origin `file`, even when its values equal
defaults. Modes are the validated `ask`, `auto` or `yolo` value. Each human
rule line contains scope, decision, JSON-quoted category and JSON-quoted pattern,
or `[inert pattern omitted]`. Control characters cannot inject terminal lines.

Default JSON is one compact object with stable key order and a final LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"kind":"permissions","permission_mode":"ask","configuration_origin":"built_in_defaults","configured_rules":{"effective_source":"user","user":[],"local":null},"saved_exact_rules_available":false,"runtime_grants_available":false}
```

Each JSON rule has `permission`, `pattern`, `action`, and `inert` fields.
An inert pattern is `null`; local absence is `null`, while an explicit empty
shadow is `[]`. No `grants` or saved-rule array is invented. Configured rules
may intentionally contain user-authored patterns, so successful output is not a
secret-free export; unrelated settings, malformed inert patterns and debug/error
details remain redacted.

Human and JSON staging are capped at 512 KiB, accommodating worst-case escaping
of the bounded selected config lists and row framing. A render-bound failure
returns exit 1, empty stdout and
`machine-god permissions: could not render report` on stderr.
Configuration or required workspace observation failure returns exit 1, empty
stdout and the existing `machine-god: failed to load configuration` diagnostic.
Writing output failure returns exit 1 and
`machine-god: failed to write output`; any already-written prefix is not a
confirmed complete report. Diagnostics have a final LF and never include paths
or underlying operational details.

## Compatibility and evidence

The pinned top-level permission-discovery scenario is implemented with native
output rather than an invented live engine snapshot. The former delivered
mode-only report intentionally stated persistent-rule support was unavailable;
this combined CLI expansion replaces that field with explicit configured-rule
discovery and separate unobserved saved-rule/runtime availability.

Focused evidence covers exact defaults and grammar precedence; schemas 1–7;
all modes and decisions; source provenance; empty local shadowing; canonical
aliases and non-Unicode workspace bytes; unavailable/invalid workspace
selection; inert-pattern omission and terminal escaping; large complete bounded
output; malformed, oversized and wrong-kind configuration; fixed failure and
output-write outcomes; unchanged config bytes and absent state artifacts.
Full feature/release acceptance remains governed by the canonical
[local feature gate](implementation-plan.md#local-feature-gate).

## Native interactive `/allowlist`

The native owner parses arguments against its retained host's actual tool
registry before admitting effects. The CLI supplies an explicitly selected
`NativeUserConfigStore` and renders typed results; parsing discovers no config
location or ambient workspace. Supported forms are:

```text
/allowlist
/allowlist view [effective|local|user]
/allowlist [local|user] add command|tool|url|web-fetch-domain <pattern>
/allowlist [local|user] remove command|tool|url|web-fetch-domain <pattern>
/allowlist [local|user] reset commands|tools|urls|web-fetch-domains|all
```

Mutation scope defaults to local; reset also accepts the singular category
spellings. Verbs, scopes and kinds are ASCII-case-insensitive. Actual tool names
and the pinned historical categories remain case-sensitive. Tool targets map to
their configured permission category and `*`; `tool web_fetch` is rejected and
unquoted `tool web_search` cannot have trailing arguments. Domain targets use
the pinned canonical `domain:` spelling, lowercase DNS and one optional root
dot. Invalid URLs, wildcards, ports and zone identifiers are rejected. Pinned
DNS hyphen behavior and rejection of uppercase IPv6 hex are retained.

Quoting is deliberately not shell parsing: an initial double quote returns
bytes through the first closing double quote and ignores the rest; without a
closing quote it returns the remaining text. Backslashes and single quotes have
no escape meaning. Empty quoted input is invalid, but a nonempty quoted pattern
may normalize to an empty configured pattern after ASCII space/tab/CR/LF trim.
The request is capped at 64 KiB plus 256 bytes of syntax allowance; fully encoded
rules and the complete configuration retain the existing 64 KiB config bound.

These explicit human edits need no additional confirmation and never change
saved exact-action rules or identity-bound grants. An accepted control pins its
current runtime principal, exact supplied store, canonical host workspace and
actual host worker scope. The single native control slot accepts during active
generation, rejects conflicting controls, retains receipts through blocked
presentation and shutdown, and settles before source-changing transitions.
Disk work never runs on the driver polling thread. Its lifecycle permit remains
inside the actual worker through publication, fresh reload and cleanup even if
the response is abandoned. Response completion is distinct from the host's
full worker/thread-local cleanup join; no filesystem deadline is promised.

Views return owned bounded user/local/effective rule projections, including an
explicit empty local shadow. Native display iteration includes only Allow rows
and excludes malformed web-fetch domain rows; raw lists remain available for
warning counts. Every successful view reloads the effective runtime policy even
when displaying user or local. Changed mutations, no-op adds and zero-removal
resets reload from a fresh post-commit snapshot. A no-op remove deliberately
does not reload: its receipt has neither a source claim nor a reload attempt.

Publication is exact-byte CAS with no blind retry. Confirmed mutation receipts
remain distinct from failed or ambiguous writes and from subsequent runtime
reload failure. A confirmed save with failed reload keeps its durable outcome
and rejects any waiting source-changing transition. Reload replaces only the
configured-pattern selection for future taken jobs; captured active-job policy,
mode, sandbox preference, canonical conversation history and exact grants stay
unchanged. Host startup resolves the canonical workspace's effective source;
hosts without native permission composition reject local-source configuration
instead of silently ignoring it.
