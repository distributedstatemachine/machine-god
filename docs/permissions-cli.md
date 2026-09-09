# Native `permissions` CLI contract

The read-only top-level command reports the validated native permission mode and
the availability of persistent rules and runtime grants. It does not construct
an engine or complete the broader permission-management surface.

## Command grammar

The only accepted forms are:

```text
machine-god permissions
machine-god permissions --json
```

`--json` is accepted exactly once and only after `permissions`. Unknown,
reordered, repeated, additional, or non-Unicode arguments fail through the
existing invalid-arguments boundary: exit code 2, empty standard output, and
this exact diagnostic on standard error, including the final LF:

```text
machine-god: invalid arguments
Usage: machine-god [help | --help | -h | --version | -V | permissions [--json] | status [--json]]
```

Argument validation completes before configuration is inspected.

The general `help`, `--help`, and `-h` output lists `permissions` between
`help` and `status`, with this exact command row:

```text
  permissions  Show the permission mode and rules
```

The global one-line invalid-argument usage includes
`permissions [--json]` before `status [--json]`. The complete exact help and
usage transcripts are maintained in [`cli.md`](cli.md). The command does not add
command-local help or a `/permissions` interactive slash command.

## Authority and loading

After successful parsing, `permissions` calls `load_process_config()` exactly
once and observes only the validated `NativeConfig::permission_mode()`. The
loader's retained bytes remain bounded to 64 KiB plus one overflow witness. It
is synchronous, read-only, and redacted on failure. On supported Unix targets,
the selected final configuration path is opened with `O_NOFOLLOW` and
nonblocking behavior, then authoritatively required to be regular. Hardened
opening on non-Unix targets remains deferred. The reader allows the first 15
cumulative interrupted results to retry and returns the existing fixed
`Unreadable` failure on the 16th. Partial progress does not reset the count,
and an over-reported read fails as `Unreadable`.

A missing file or unavailable configuration location uses the safe built-in
configuration. Valid strict schema-v1, schema-v2, schema-v3 and schema-v4 files report the
same currently supported `ask` mode without rewriting any byte. An invalid
selected environment, wrong file type, unreadable or oversized file, malformed
configuration, or unsupported schema version fails closed with exit code 1,
empty standard output, and exactly:

```text
machine-god: failed to load configuration
```

The diagnostic does not disclose the error kind, path, configuration content,
model, provider, transport, credential source, or operating-system detail.

The command performs no state-root filesystem metadata access and creates no
state root. Its config-only environment snapshot requests `XDG_CONFIG_HOME`
first, reads `HOME` only when XDG is missing or empty, and never requests
`XDG_STATE_HOME`. A nonempty valid, invalid-relative, or non-Unicode XDG value
never reads or falls back to `HOME`. Status retains its separate
`XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`HOME` snapshot. Neither command constructs
an engine, provider, transport, credential source, permission prompter, session
store, or Tokio runtime; reads a credential; makes a network request; prompts;
persists a rule; or caches a grant.

## Exact output

Human output is exactly:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
persistent_rules: unsupported
runtime_grants: unavailable
```

JSON output is one compact object with stable key order and one final LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"kind":"permissions","permission_mode":"ask","persistent_rules_supported":false,"runtime_grants_available":false}
```

`persistent_rules_supported: false` means the command does not expose or manage
identity-safe persistent policy. `runtime_grants_available: false` means the
read-only command has no live engine or permission-handler snapshot. Neither
field asserts that an unobserved collection is empty.

If writing successful output fails, the existing output boundary returns exit
code 1 and the fixed `machine-god: failed to write output` diagnostic. No partial
success is claimed.

## Compatibility and deferrals

Pinned fx exposes a scenario named `permissions [--json]`. Machine-god aligns
with that read-only discovery scenario but intentionally uses its own exact
output and supports only validated mode `ask`. The combined top-level CLI
compatibility surface remains planned; this command does not promote the
generated inventory.

This read-only top-level surface does not mutate modes, configured patterns,
saved exact rules, grants or sandbox policy. Native interactive allowlist
ownership is specified below; it does not change the top-level output contract.

## Required evidence

Independent tests must cover:

- exact grammar, updated global help/usage, non-Unicode arguments, exit codes,
  standard streams, JSON key order, and final LF;
- missing and unavailable configuration defaults plus valid v1/v2/v3/v4 files;
- invalid environment, symlink/wrong-kind, unreadable, oversized, malformed,
  and unsupported-version failures with fixed redaction;
- byte-identical configuration files and absence of newly created config/state
  roots;
- a cumulative 16-`Interrupted` read limit with deterministic injected-reader
  success after up to 15 interruptions and fixed `Unreadable` failure on the
  16th;
- config-only process snapshots that read `XDG_CONFIG_HOME` first, read `HOME`
  only for missing or empty XDG, never read `XDG_STATE_HOME`, and do not read
  or fall back to `HOME` for nonempty valid, invalid-relative, or non-Unicode
  XDG;
- supported-Unix final-path `O_NOFOLLOW`, nonblocking, and authoritative
  regularity behavior without a hardened non-Unix claim;
- unchanged identity/version/status behavior except the intentional global
  help and invalid-usage additions; and
- freshly built release-binary human, JSON, invalid-config, no-create, and
  no-rewrite smokes.

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
