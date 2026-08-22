# Command-line interface

The `machine-god` binary is the thin native reference host for the embeddable
engine. This page defines the exact Milestone 03 config/status slice. The
commands inspect process environment and filesystem metadata only; they do not
parse configuration, create directories, write files, or start the engine.
The separate [`native configuration schema-v3 candidate`](configuration.md)
does not change that boundary or any CLI byte documented below. Its production
implementation, independent tests, local gates, and all three adversarial tracks
are green on exact behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`.
Exact feature CI run `32582210892` and benchmark-evidence run `32582210927` are
green on documentation seal SHA `5f4deac672af85fe5c0b1be50c327ddbdd55ce9a`.
Fast-forward integration and exact `main` delivery remain pending. Provider,
transport, model, and credential-source config fields remain invisible to this
metadata-only surface.

## Accepted invocations

The accepted argument forms are exactly:

```text
machine-god
machine-god help
machine-god --help
machine-god -h
machine-god --version
machine-god -V
machine-god status
machine-god status --json
```

Bare invocation, `--version`, and `-V` write this exact identity to stdout:

```text
machine-god 0.1.0 (engine API 1)
```

The output ends in one LF. Bare invocation intentionally remains the original
identity behavior.

`help`, `--help`, and `-h` write this exact stdout, including the final LF:

```text
machine-god 0.1.0
Embeddable coding-agent engine

Usage:
  machine-god
  machine-god help
  machine-god status [--json]

Commands:
  help      Show this help
  status    Show configuration and runtime information

Options:
  -h, --help       Show this help
  -V, --version    Show version
```

## Status output

`machine-god status` writes four lines with a final LF:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
config_file: state=<state> path=<JSON-string-or-null>
state_directory: state=<state> path=<JSON-string-or-null>
```

Even in human output, a present path is encoded as a JSON string. Quotes,
backslashes, C0/C1 controls, Unicode line/paragraph separators, and Unicode
bidirectional-formatting controls are escaped. An unresolved path is the
unquoted token `null`.

`machine-god status --json` writes one compact JSON object in this fixed key
order, followed by one LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"permission_mode":"ask","config_file":{"path":null,"state":"unavailable"},"state_directory":{"path":null,"state":"unavailable"}}
```

The example shows unavailable paths. The exact structural form is:

```text
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"permission_mode":"ask","config_file":{"path":<JSON-string-or-null>,"state":"<state>"},"state_directory":{"path":<JSON-string-or-null>,"state":"<state>"}}
```

Config-file state is one of `file`, `missing`, `not_file`, `inaccessible`,
`unavailable`, or `invalid_environment`. State-directory state is one of
`directory`, `missing`, `not_directory`, `inaccessible`, `unavailable`, or
`invalid_environment`. A valid resolved path is reported even when its state is
missing, inaccessible, or the wrong kind. The path is `null` only for
`unavailable` or `invalid_environment`.

Permission mode is always `ask` in this slice. It reports the native host's
fixed safe default; no permission prompt or permission-gated native tool is
implemented here. Status does not load a legacy v1 or v2 config or a candidate
current v3 config and therefore does not report its observable schema version,
provider, transport, model, or credential source.

## Config and state locations

Configuration and state use only the `machine-god` namespace:

- a nonempty `XDG_CONFIG_HOME` selects
  `$XDG_CONFIG_HOME/machine-god/config.json`;
- otherwise, a nonempty `HOME` selects
  `$HOME/.config/machine-god/config.json`;
- a nonempty `XDG_STATE_HOME` selects `$XDG_STATE_HOME/machine-god`;
- otherwise, a nonempty `HOME` selects
  `$HOME/.local/state/machine-god`.

An empty XDG value is treated as absent and falls back to `HOME`. If a selected
nonempty XDG root is relative or not Unicode, that location is
`invalid_environment`; the CLI does not fall back to `HOME`. A selected
nonempty `HOME` must likewise be absolute Unicode. Absent or empty `HOME` makes
a fallback location `unavailable`.

Inspection calls `symlink_metadata` on each final path. A final symlink is not
followed: a config symlink reports `not_file`, and a state-directory symlink
reports `not_directory`. The command does not canonicalize paths, parse
`config.json`, follow a final symlink, create missing locations, or write any
state.

## Errors and exit status

Valid invocations exit zero after writing their output. Any other argument
sequence, including a non-UTF-8 argument, writes no stdout, exits 2, and writes
this exact stderr with a final LF:

```text
machine-god: invalid arguments
Usage: machine-god [help | --help | -h | --version | -V | status [--json]]
```

An output-write failure exits 1 and uses this fixed diagnostic on stderr:

```text
machine-god: failed to write output
```
