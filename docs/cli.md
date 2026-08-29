# Command-line interface

`machine-god` is the thin native reference host for the embeddable Rust
engine. The CLI owns argument parsing, process exit codes, and presentation;
provider-neutral orchestration remains in `machine-god-core`, while process,
filesystem, persistence, credential, and network effects remain in
`machine-god-native`.

Current delivery state and gate evidence are maintained only in the
[implementation plan](implementation-plan.md#current-delivery-state).

## Global behavior

- With no arguments, `machine-god` prints its identity line.
- `help`, `--help`, and `-h` print the same help text.
- `--version` and `-V` print the identity line.
- Invalid arguments are rejected before command-specific effects, write a
  fixed diagnostic to standard error, and exit `2`.
- Successful commands exit `0`. Operational or output failures exit `1`
  unless a command contract defines a signal exit.
- `--json` is command-local. It is not a global option and is accepted only by
  commands whose linked contract defines it.
- Diagnostics are bounded and redact configuration values, credentials,
  prompts, paths, provider payloads, tool arguments, and operating-system
  details unless a command contract explicitly makes a value public output.

## Commands

| Command | Purpose | Contract |
| --- | --- | --- |
| `help` | Show command help | This page |
| `ask [--] <prompt...>` | Run one noninteractive request | [ask](ask-cli.md) |
| `doctor [--json]` | Run bounded local health checks | [doctor](doctor-cli.md) |
| `models [--json]` | List the bounded AI Gateway model catalog | [models](models-cli.md) |
| `permissions [--json]` | Report configured permission mode | [permissions](permissions-cli.md) |
| `resume <id> [--] <prompt...>` | Continue one saved session with one prompt | [resume](resume-cli.md) |
| `session <id> [--json]` | Inspect one saved session summary | [session](session-cli.md) |
| `sessions [--json]` | List bounded saved-session identities | [sessions](sessions-cli.md) |
| `status [--json]` | Report configuration and state metadata | This page |

The pinned upstream inventory contains a broader command and option surface.
Unsupported forms remain invalid until their owning milestone freezes a
contract; command-name presence alone is not an equivalence claim.

## Identity

The identity line is:

```text
machine-god <package-version> (engine API <api-version>)
```

It performs no configuration, filesystem, credential, runtime, or network
effect.

## The `status` command

`status` and `status --json` inspect process configuration and state-location
metadata without loading configuration contents, constructing the engine,
creating directories, starting a runtime, or contacting a provider. The human
form is exactly four LF-terminated lines in this field order:

```text
machine-god <package-version> (engine API <api-version>)
permission_mode: ask
config_file: state=<config-state> path=<JSON-string-or-null>
state_directory: state=<state-directory-state> path=<JSON-string-or-null>
```

The angle-bracketed version values are the compiled package version and the
supported core API version. `status --json` writes one compact object followed
by LF, with this exact nested key order:

```text
{"name":"machine-god","version":"<package-version>","engine_api_version":<api-version>,"permission_mode":"ask","config_file":{"path":<JSON-string-or-null>,"state":"<config-state>"},"state_directory":{"path":<JSON-string-or-null>,"state":"<state-directory-state>"}}
```

Config state is exactly one of `file`, `missing`, `not_file`, `inaccessible`,
`unavailable`, or `invalid_environment`. State-directory state is exactly one
of `directory`, `missing`, `not_directory`, `inaccessible`, `unavailable`, or
`invalid_environment`. A resolved path is present even when it is missing,
inaccessible, or the wrong kind; only `unavailable` and
`invalid_environment` have a `null` path. Status reports the fixed native
permission default `ask` and does not parse configuration, so it does not
report configured provider, transport, model, credential source, or schema.

Present paths use JSON-string encoding in both forms. Quotes, backslashes,
C0/C1 controls and DEL, Unicode line and paragraph separators, and Unicode
bidirectional-formatting controls are escaped; unresolved paths use the
unquoted token `null` in human output and JSON `null` in JSON output.

Config and state locations are resolved independently. A nonempty
`XDG_CONFIG_HOME` selects
`$XDG_CONFIG_HOME/machine-god/config.json`; otherwise a nonempty `HOME`
selects `$HOME/.config/machine-god/config.json`. A nonempty `XDG_STATE_HOME`
selects `$XDG_STATE_HOME/machine-god`; otherwise a nonempty `HOME` selects
`$HOME/.local/state/machine-god`. Empty XDG values are absent and fall back to
`HOME`. Every selected root must be absolute Unicode. A nonempty invalid XDG
root produces `invalid_environment` for its location and never falls back to
`HOME`; an invalid selected `HOME` does the same. Missing or empty `HOME` makes
a location that needs it `unavailable`.

Inspection uses no-follow metadata for each final path. A final config symlink
is `not_file`, and a final state-directory symlink is `not_directory`; neither
target is followed. The command does not canonicalize paths, read or parse the
configuration file, create missing paths or ancestors, write state, construct
an engine, start a runtime, discover credentials, or use the network.

## Output ownership

Every command validates its complete grammar before acquiring its native
authority. Commands assemble bounded atomic output when their contract
requires a single report. Streaming commands retain cancellation ownership
while output is live and stop promptly on output failure. The CLI does not
retain product state of its own.
