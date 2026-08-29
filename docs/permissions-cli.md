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
configuration. Valid strict schema-v1, schema-v2, and schema-v3 files report the
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

Deferred work includes modes beyond `ask`, persistent rule schemas, identity-
safe grants, allowlists, sandbox policy, mutation, interactive `/permissions`,
and live session-grant introspection. Those authority-bearing surfaces belong
to the frozen M04 or later boundary, not the thin CLI.

## Required evidence

Independent tests must cover:

- exact grammar, updated global help/usage, non-Unicode arguments, exit codes,
  standard streams, JSON key order, and final LF;
- missing and unavailable configuration defaults plus valid v1/v2/v3 files;
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
