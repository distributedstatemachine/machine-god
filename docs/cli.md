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
form is line-oriented; the JSON form is one compact LF-terminated object.
Paths are presentation values for this command and are escaped in JSON.

## Output ownership

Every command validates its complete grammar before acquiring its native
authority. Commands assemble bounded atomic output when their contract
requires a single report. Streaming commands retain cancellation ownership
while output is live and stop promptly on output failure. The CLI does not
retain product state of its own.
