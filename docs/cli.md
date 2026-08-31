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
| `background [last\|<id>] [--json]` | Inspect bounded persisted background history | [background](background-cli.md) |
| `doctor [--json]` | Run bounded local health checks | [doctor](doctor-cli.md) |
| `models [--json]` | List the bounded AI Gateway model catalog | [models](models-cli.md) |
| `permissions [--json]` | Report configured permission mode | [permissions](permissions-cli.md) |
| `replay <tape> [options]` | Replay an FXTP terminal tape | [replay](replay-cli.md) |
| `resume <id> [--] <prompt...>` | Continue one saved session with one prompt | [resume](resume-cli.md) |
| `session <id> [--json]` | Inspect one saved session summary | [session](session-cli.md) |
| `sessions [--json]` | List bounded saved-session identities | [sessions](sessions-cli.md) |
| `status [--json]` | Report the effective local runtime snapshot | This page |
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
native runtime-status authority. A valid invocation loads the bounded strict
native configuration, discovers only the configured environment credential
source, and canonicalizes the current workspace. It does not construct the
engine, create directories, write configuration or state, start a session, or
contact a provider. The human form uses the following exact field order, with
one LF-terminated line per present field:

```text
[status] model=<configured-model>
[status] update_channel=stable
[status] build_channel=stable
[status] build_revision=<compiled-revision> # omitted when unavailable
[status] auth=<credential-source-or-missing>
[status] auth_refreshable=false
[status] auth_help=Machine God needs access to Vercel AI Gateway. Set VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY. # missing auth only
[status] permission_mode=ask
[status] sandbox=none
[status] workspace=<canonical-current-directory>
[status] history_turns=0
[status] session_permission_grants=0
[status] agent_step_limit=8
```

The comments above describe conditional lines and are not output. The model
and permission mode come from the loaded configuration, including built-in
defaults when the configuration file is absent. The build and update channels
are `stable`. A compile-time `MACHINE_GOD_BUILD_REVISION` containing one to 12
ASCII hexadecimal characters supplies the human `build_revision` line;
otherwise that line is omitted. `status --json` writes one compact object
followed by LF in this exact key order:

```text
{"kind":"status","model":"<configured-model>","update_channel":"stable","build_channel":"stable","build_revision":"<compiled-revision-or-empty>","auth":"<credential-source-or-missing>","auth_refreshable":false,"auth_help":"Machine God needs access to Vercel AI Gateway. Set VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY.","permission_mode":"ask","sandbox":"none","workspace":"<canonical-current-directory>","history_turns":0,"session_permission_grants":0,"agent_step_limit":8}
```

The JSON `build_revision` key is always present and is the empty string when no
revision was compiled in. `auth` is exactly `VERCEL_OIDC_TOKEN`,
`AI_GATEWAY_API_KEY`, or `missing`. A nonempty valid `VERCEL_OIDC_TOKEN` takes
precedence over a valid `AI_GATEWAY_API_KEY`; tokens are never rendered.
`auth_help` has the exact branded text shown above and is present only while
authentication is missing. Environment credentials are not refreshable, so
`auth_refreshable` is always `false` in this bounded native host.

Status describes a fresh, non-session runtime boundary. Its effective sandbox
is `none`, its history and session-grant counts are zero, and its agent step
limit is eight. The workspace is the Unicode canonical current directory.
Quotes, backslashes, C0/C1 controls and DEL, Unicode line and paragraph
separators, and Unicode bidirectional-formatting controls in rendered string
fields are escaped. The configured model retains its existing 128-byte bound,
and the canonical Unicode workspace is limited to 4,096 UTF-8 bytes. Every
rendered value must also fit the inclusive report-output bound below.

Invalid configuration, a selected invalid credential, a non-Unicode or
unavailable current directory, or another runtime-snapshot inspection failure
exits `1`, writes no standard output, and writes exactly this redacted standard
error diagnostic:

```text
machine-god status: could not inspect runtime
```

Inspection reads at most the existing 65,536-byte configuration-file ceiling.
A missing configuration file selects the built-in runtime defaults without
creating it. Status does not inspect or initialize the state root. It performs
no product write or network request.

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

The command-specific help transcript is the bounded common `status --help`
scenario with product-name normalization. Top-level help remains an honest
capability-aware machine-god index rather than a claim of full pinned-fx
catalog parity. This contract makes no comparative performance claim.

## Output ownership

Every command validates its complete grammar before acquiring its native
authority. Commands assemble bounded atomic output when their contract
requires a single report. Streaming commands retain cancellation ownership
while output is live and stop promptly on output failure. The CLI does not
retain product state of its own.
