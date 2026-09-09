# Command-line interface

`machine-god` is the thin native reference host for the embeddable Rust
engine. The CLI owns argument parsing, process exit codes, and presentation;
provider-neutral orchestration remains in `machine-god-core`, while process,
filesystem, persistence, credential, and network effects remain in
`machine-god-native`.

Current delivery state and gate evidence are maintained only in the
[implementation plan](implementation-plan.md#current-delivery-state).

## Global behavior

Command implementations live in command-specific CLI modules. The entrypoint
retains global grammar and private helper dispatch, and routes through named,
borrowed host dependencies so tests can replace one host without rebuilding
unrelated command wiring. Constructing that dependency set performs no effects.

Doctor, models, session inspection, sessions, status, workspace and background
share byte-bounded output staging. Each command keeps its own byte ceiling,
initial capacity, escaping and error mapping; rendering completes before success
output is written. A rejected append leaves its existing prefix unchanged.
Model signal ownership remains alive through output and cleanup.

- With no arguments, `machine-god` opens a fresh interactive session on Linux/macOS.
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

Linux/macOS process helpers use exact private single-argument modes, outside
normal command parsing and help. PTY/startup-marker protocol failures exit
`125` without rendering protocol data; a successful startup marker exits `0`.
Additional arguments do not activate a helper.

Interactive stdin uses the [native input adapter](interactive-input.md), including
the exact private input-helper dispatch before configuration or signal setup.
Ordinary shared TTY/pipe input never authorizes changing the shell's status flags.
The interactive host separately owns raw termios and restores the captured
settings after stopping and joining input readers, before final signal exit or
the native-free output tail. External signals retain shutdown ownership; raw
Ctrl-C cancels first and requests exit on a second press within three seconds.
Ctrl-D deletes forward in a draft, exits with an empty idle composer, and is
ignored with an empty composer during active/queued work. Physical EOF never
submits a draft and is an abnormal input closure.

The UTF-8 composer retains at most 262,144 bytes and supports cursor movement,
home/end, deletion and atomic bracketed paste. Paste line endings normalize to
LF; invalid or oversized paste is drained through its closing marker without
executing embedded control actions or changing the previous draft. Invalid
ordinary UTF-8, NUL, or oversized input retains the valid draft and rejects through the next Enter;
that Enter does not submit a truncated prefix. Submitted prompt identity comes
from the first received byte, including buffered answers across modal pages.

A bounded single-row viewport uses the native screen's pinned Unicode display
units. It never truncates the underlying draft. Columns and rows come from one
size observation on the retained output TTY, initially and through owned reads
after coalesced resize notifications. Subscription precedes the initial read;
unavailable or zero dimensions fail explicitly without guessed defaults.
The native columns-only API remains available and accepts zero rows, while
full-dimensions observation validates both axes. Reads share one admission lane
and stay owned through actual worker completion even if their futures are dropped.
Draft rendering pauses during model streaming
unless a human prompt is active, keeping model deltas contiguous. Terminal
controls in draft text are escaped. Bracketed-paste mode is enabled through the
acknowledged output lane and disabled in its final tail; output failure or a
signal-forced stalled-output exit cannot promise that display cleanup reached
the terminal. Termios restoration and native joins do not depend on that tail.

## Commands

| Command | Purpose | Contract |
| --- | --- | --- |
| `help` | Show command help | This page |
| No arguments | Start a fresh interactive session | This page |
| `ask [--] [<prompt...>]` | Run one noninteractive request from argv or whole stdin | [ask](ask-cli.md) |
| `background [last\|<id>] [--json]` | Inspect bounded persisted background history | [background](background-cli.md) |
| `doctor [--json]` | Run bounded local health checks | [doctor](doctor-cli.md) |
| `models [--json]` | List the bounded AI Gateway model catalog | [models](models-cli.md) |
| `permissions [--json]` | Report configured permission mode | [permissions](permissions-cli.md) |
| `replay <tape> [options]` | Replay an FXTP terminal tape | [replay](replay-cli.md) |
| `resume <id> [--] <prompt...>` | Continue one saved session with one prompt | [resume](resume-cli.md) |
| `resume [last\|<id>]` | Resume latest or an exact saved session interactively | This page |
| `session <id> [--json]` | Inspect one saved session summary | [session](session-cli.md) |
| `sessions [--all] [--limit <1-100>] [--cursor <cursor>] [--json]` | Page rich saved-session summaries for the current workspace or all workspaces | [sessions](sessions-cli.md) |
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
adaptive `COLUMNS` wrapping, additional global
flags, examples, and resources. Those presentation and product-surface details
are intentional scenario differences. This slice makes no byte-equivalence or
complete-fx-help claim.

## Interactive ownership

Bare startup creates a fresh native session; `resume` and `resume last` select
latest through the validated native catalog, while `resume <id>` selects that
exact record. Interactive startup sends no fabricated initial prompt. It retains
one complete host, credential and completed model-catalog cache across turns.
The catalog can fall back without inventing model capabilities. The noninteractive
`ask` and prompt-bearing `resume` paths retain their separate contracts.
Both stdin and stdout must be TTYs for production interactive startup; failure
exits `1` with a fixed diagnostic before configuration, credentials or sessions
are acquired. The explicitly supplied native pipe adapter is not permission to
treat piped bare input as an upstream-compatible interactive prompt.

The raw UTF-8 composer described above serves both prompts and commands through
one input reader. Permission choices distinguish once, turn,
session and deny; session permission is not saved-rule confirmation. Ordinary
questions present ordered numbered options plus `other <answer>` and cancellation.
Answers require the exact native prompt token and acknowledged question page.
Already-received chunks and partial drafts retain their original binding; they
cannot be retargeted to a replacement prompt or next question page. This does
not claim timestamps or provenance for unread bytes still in the kernel.

Initial resume displays a single canonical record snapshot; confirmed session
installation replaces that projection with the destination's history. Unchanged
or rejected transitions do not manufacture a new history. Projection neither
starts inference nor re-executes tools, and never uses the compacted provider
context as a substitute for canonical history. Historical user/assistant text is
escaped and streamed in bounded chunks; raw system instructions and tool-result
details are not printed as ordinary transcript text. Tool call/result summaries
describe recorded identities and outcomes, not fresh execution receipts.
Native-decoded terminal exec/start requests additionally show their saved command
and requested CWD; an omitted CWD remains unknown, not today's working directory.
Snapshot construction validates the bounded reserved native observation ledger.
After the transcript, typed cards show recorded file status, stale flags and
destinations, plus saved background log paths/URLs with current liveness unknown.
Exact source-bound edit calls show escaped requested old/new fragments, not a
full-file diff or proof of a successful mutation. Invalid observation metadata is
suppressed with a fixed notice; unknown tool payloads stay collapsed. No card
opens files, retrieves archives, probes processes or supplies execution authority.
History traversal and card output yield after bounded work. Native progress, modal answers and
shutdown remain independent of history output acknowledgements. Unsent obsolete
history is discarded on confirmed replacement or shutdown, while already-written
bytes cannot be retracted. Typed current lifecycle/save receipts remain retained.

Native session ownership drives admitted turns, accepted saves, transitions and
shutdown independently of stdout acknowledgements. Typed save and lifecycle
receipts stay separate from disposable streaming text. Session retirement is
not full-host cleanup: the CLI drops native ownership and joins actual input and
terminal workers before waiting for final presentation. Signals during acquired
host startup or active work latch until cleanup; post-cleanup output retains the
existing signal-exit behavior. The CLI never treats a save error as rollback or
automatically repeats an uncertain operation.

The complete slash-command, picker, workspace and policy feature remains governed
by the implementation plan; command names in the native catalog alone do not
establish CLI support. `/help` describes the handlers actually wired in this host.

The wired handlers include `/help`, `/status`, `/version`, `/quit` (`/exit`),
`/cancel`, `/clear`, `/new`, `/reset`, argumentless `/resume` (picker), `/continue`,
`/rename <title>`, `/compact` and argumentless `/undo` and `/copy`. Policy selection uses
`/permissions [ask|auto|yolo|reset]` and `/sandbox [os|none]`. Model controls are
`/models`, `/model [id-or-query|effort <name>|save|save-default]` and `/fast`.
Configured persistent rule editing uses `/allowlist`, with native parsing,
explicit user-settings authority, and typed save/reload receipts as described
below. It is separate from exact-action session grants.
Model/effort/fast changes request native session and explicitly injected
user-default saves; their independent receipts distinguish accepted, deferred,
saved and failed targets. Without injected defaults authority only the session
target is available; `save-default` never discovers an ambient store.
Ordinary prompts retain their 256 KiB bound independently of the 64 KiB slash
envelope. `/cancel` requests owned turn cancellation without replacing its
session or discarding untaken prompts; an accepted save settles first.

`/undo` requests the latest cooperating file inverse from the complete host's
shared native tracker, including during an active response. It does not cancel
that response, delete messages, rewrite history or fabricate tool events. The
CLI performs no inverse filesystem work itself. Native serializes the request
with cooperating mutations and retains its ownership through shutdown.

The typed receipt distinguishes an empty tracker, a restored workspace-relative
path and removal of a newly created path. Paths use the bounded terminal-safe
encoder; an unrenderable oversized receipt fails presentation, not a truncated
success. Busy, rejected authority, changed tracked state, resource exhaustion,
unavailable filesystem work, cancellation and both non-undoable preimage reasons
remain distinct failures, never “Nothing to undo” or generic saved-publication
reload advice. An ambiguous result warns that effects may be partial and recovery
artifacts are retained; no automatic retry or rollback is promised. As with
other controls, the exact native receipt stays owned until output acknowledgement,
including blocked output and the native-free shutdown tail. New controls wait
while an earlier receipt is pending.

`/copy` captures the current canonical transcript when accepted and asks native
to select the newest nonempty assistant reply without tool calls. It preserves
the saved text bytes, not rendered terminal output or an uncommitted stream.
An empty history reports “No assistant reply to copy.” without invoking a
clipboard backend. Normal successful clipboard completion reports “Copied to
clipboard.”; unavailable authority, cancellation, limits and process failures
report “Failed to copy to clipboard.” without echoing the payload or environment.
Clipboard failure alone does not turn a successful conversation into a failure.

The blocking startup host resolves only `pbcopy` on macOS or `xclip` on Linux
from its frozen PATH, with at most 512 search entries and bounded path spelling,
and supplies a retained executable and frozen environment. Native constructs
the actual clipboard operation in the complete host's worker scope. Missing
clipboard support does not prevent interactive startup. No fallback clipboard
protocol or shell command is constructed; process behavior is defined by the
[native clipboard capability](clipboard.md).

Clipboard selection/process work has an independent polling lane: it does not
cancel or delay model generation or durable controls. A session transition or
shutdown cancels pending copy work without retargeting its original snapshot.
The CLI retains its typed receipt through blocked output and final flush
acknowledgement; another copy waits for that receipt, but ordinary prompts do
not. Started clipboard work settles before conversion to the native-free output
tail, and full host completion still joins its real worker and child cleanup
before the tail is presented.

### Persistent allowlist

`/allowlist` and `/allowlist view [effective|local|user]` display persistent Allow
rules in grouped, terminal-safe form. Native also reloads the effective runtime
policy for every view. An explicit empty local list shadows user rules. Malformed
web-fetch rules remain inert and contribute to the displayed warning count.

Mutations accept `[local|user] add|remove command|tool|url|web-fetch-domain <pattern>`
or `[local|user] reset commands|tools|urls|web-fetch-domains|all`; the default scope
is local. The native parser validates actual registered tool names and the pinned
historical categories, quoting and domain conventions. The CLI does not perform
configuration discovery, rule parsing, filesystem publication or policy matching.
The settings store is the same explicitly injected authority used by model-default
saves. Missing authority fails without changing settings or runtime policy.

An explicit human slash command needs no second confirmation prompt. Native
accepts edits during generation, but already taken jobs keep their earlier policy
snapshot. Future taken jobs use a successfully reloaded effective selection.
Mode, sandbox preference and exact-action grants are not reset by allowlist edits.
Removing a missing rule is an unchanged result and does not reload runtime policy;
other successful mutations, including no-op add and reset, reload effective rules.

The renderer distinguishes settings saved, unchanged, publication uncertain and
runtime reload failed. A saved-settings receipt remains visible if reloading fails;
there is no rollback claim or automatic retry. Rule groups preserve first-appearance
and pattern order, including duplicate entries. Display uses an independent bounded
buffer sized for terminal-safe expansion of the complete 64 KiB configuration,
then the normal 4 KiB output chunks; valid rule lists are not silently truncated.
The native control lane pins the accepted session and workspace through settlement.
Typed results remain retained through blocked output and native-free shutdown, and
new controls wait for earlier receipt acknowledgement.

The native storage and mutation contract is maintained in
[configuration](configuration.md) and [permissions](permissions-cli.md).

### Session picker

`-r` opens the current-workspace picker before allocating any writable session.
In an existing interactive session, argumentless `/resume` opens the same scope;
Cmd/Super+R opens all workspaces. Ctrl-R is not an alias. Opening refuses a
nonempty draft, active response or queued work without cancelling it. Current
session rows and records without conversation history are excluded. Managed
child-session persistence retains its M05 ownership; terminal/background jobs
are not treated as persisted child-session relationships.

The picker observes real terminal rows and requests `clamp(rows - 7, 10, 100)`
summaries per page. Native performs cancellable catalog scanning on the complete
host's owned worker scope; advisory lock contention is distinct from invalid
records. Initial-page caches are separate for current/all scopes. Reopening
shows cached rows immediately while refreshing; a failed refresh retains rows.
Incomplete scans and skipped invalid records remain visible, not fabricated
complete results. At most 1,024 compact rows are retained in the active view.

Search is a trimmed, 256-byte ASCII-case-insensitive substring match over loaded
titles, workspace paths and previews, not session IDs or unseen pages. Missing
titles display `Untitled session`; saved origin workspace is a display fallback,
not newly acquired authority. Absent activity times remain unknown. Pagination
uses the last unfiltered native row, even when every displayed row is excluded
or fails the search. Load-more selection and approaching the last matching row
request another page; newly arriving matching rows become the selection.

Up/Down and Ctrl-K/Ctrl-J move without wrapping; Tab/Shift-Tab switch scope while
preserving the query. Shift-Enter does not insert a search newline. Escape has a
50 ms standalone-key disambiguation window, distinct from split CSI and paste
sequences. It closes an in-session picker without changing the active session;
at startup it explicitly requests a fresh session. Ctrl-C retains its ordinary
clear/exit-arm behavior, and empty Ctrl-D exits without creating a session.

Only an exact acknowledged picker frame can select a row. Input already
received keeps its original chunk identity across opening, query changes,
scope changes and startup handoff. Selection carries the observed session ID,
incarnation and revision; it cannot silently retarget a replaced or revised
record. Missing/stale/open failures keep the picker available for retry. A typed
busy, changed or missing selection rejection alone does not make an otherwise
successful interactive session exit as failed. Its receipt remains visible through
blocked output and shutdown; unrelated failures and uncertain outcomes still fail.
This does not promise rollback of an association already published during preparation.
Successful
selection uses the normal native resume transition and canonical history replay,
not a provider prompt or repeated tool effects. Terminal-safe width/byte-bounded
labels and owned output acknowledgements also apply to picker presentation.

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
fields are escaped. The configured model has a 1,024 UTF-8-byte bound,
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

The reusable canonical input framer (not the production raw composer) retains one logical line of at most
262,144 bytes, separately from the native adapter's 4,096-byte raw chunk. It
preserves split UTF-8, accepts LF/CRLF, validates UTF-8 and NUL at frame completion,
and flushes a partial line at EOF once. A bare CR remains content. Oversize input
reports once, discards through LF without growth, and resumes at the next line.
Emitted strings retain only their logical byte capacity. Framing does not
acquire stdin, route answers or change the command grammar described above;
slash routing applies its separate bound only to slash submissions.

Every command validates its complete grammar before acquiring its native
authority. Commands assemble bounded atomic output when their contract
requires a single report. Streaming commands retain cancellation ownership
while output is live and stop promptly on output failure. The CLI does not
retain product state of its own.
