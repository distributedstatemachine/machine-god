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
- A first argument of `help`, `--help`, or `-h` prints the same help text and
  preempts every following argument and effect.
- `--version` and `-V` print the identity line.
- Invalid arguments are rejected before command-specific effects. They write
  the fixed global diagnostic to standard error and exit `2` unless the
  command contract below defines a command-local parse failure.
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
| `replay <tape> [options]` | Replay an FXTP terminal tape | [replay](replay-cli.md) |
| `resume <id> [--] <prompt...>` | Continue one saved session with one prompt | [resume](resume-cli.md) |
| `session <id> [--json]` | Inspect one saved session summary | [session](session-cli.md) |
| `sessions [--json]` | List bounded saved-session identities | [sessions](sessions-cli.md) |
| `status [--json]` | Report configuration and state metadata | This page |
| `workspace [list] [--json]` | Report the primary workspace | [workspace](workspace-cli.md) |

The pinned upstream inventory contains a broader command and option surface.
Unsupported forms remain invalid until their owning milestone freezes a
contract; command-name presence alone is not an equivalence claim.

## The `help` command

`help`, `--help`, and `-h` are exact first-token aliases. Once one is present
as the first argument, all remaining arguments are ignored, including unknown,
non-Unicode, and flag-looking values. Help exits `0`, writes the complete
machine-god help page with one final LF to standard output, and writes nothing
to standard error. It does not snapshot the process environment, inspect the
current directory or filesystem, load configuration or credentials, create a
runtime or engine, or use persistence or the network.

The help page is machine-god navigation, not a list of every name in the pinned
fx inventory. It contains only the commands and options whose machine-god
contracts are implemented. Its rows, summaries, ordering, and usage forms are
the exact ones in the command table above and the command-specific contracts.
The three aliases produce byte-identical output.

Pinned fx has a broader command catalog, terminal-sensitive ANSI styling,
adaptive `COLUMNS` wrapping, interactive bare invocation, additional global
flags, examples, and resources. Those presentation and product-surface details
are intentional scenario differences. This slice makes no byte-equivalence or
complete-fx-help claim.

## Identity

The identity line is:

```text
machine-god <package-version> (engine API <api-version>)
```

It performs no configuration, filesystem, credential, runtime, or network
effect.

## The `status` command

The status grammar accepts `status` followed by zero or more exact `--json`
options. Repetition is idempotent: one or many occurrences select the same JSON
output. An exact `--help` or `-h` anywhere after `status` preempts every other
status argument and effect, exits `0`, and writes exactly:

```text
machine-god status

Show configuration and runtime information

Usage:
  machine-god status [--json]

Options:
  --json  Emit machine-readable JSON instead of text
```

The transcript has one final LF. Unknown, additional, or non-Unicode status
arguments are command-local parse failures with exit `1`. Without a raw exact
`--json` anywhere in the status tail, failure writes no standard output and
writes exactly this standard-error diagnostic:

```text
usage: machine-god status [--json]
```

If the raw tail contains an exact `--json`, failure writes nothing to standard
error and writes this compact LF-terminated object to standard output:

```json
{"kind":"status","error":"invalid arguments","code":"InvalidLocalSurfaceArgs"}
```

Help and complete status parsing occur before process-environment capture or
native metadata authority. `status` and its JSON form inspect process
configuration and state-location
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

Both successful forms are fully rendered before the first output write. The
inclusive rendered-output ceiling is 65,536 bytes, including the final LF and
after worst-case JSON/control escaping. A report of exactly 65,536 bytes is
accepted. Any checked length overflow or one-byte excess exits `1`, writes no
standard output, and writes exactly:

```text
machine-god status: could not render report
```

Once a bounded report has been rendered, an output write failure exits `1` and
uses the global fixed `machine-god: failed to write output` standard-error
diagnostic. No alternate representation, configuration access, or other product
effect follows either failure.

This status remains machine-god's metadata-only native observation. Pinned fx
reports a richer runtime snapshot including model, build and update channel,
authentication, sandbox, workspace, history, grants, and agent-step limit.
Those fields and fx's configuration semantics are intentional scenario
differences here. The retained help and status benchmark workloads remain
non-equivalent, unmeasured, and claim-ineligible; this contract makes no
comparative performance claim.

## Output ownership

Every command validates its complete grammar before acquiring its native
authority. Commands assemble bounded atomic output when their contract
requires a single report. Streaming commands retain cancellation ownership
while output is live and stop promptly on output failure. The CLI does not
retain product state of its own.
