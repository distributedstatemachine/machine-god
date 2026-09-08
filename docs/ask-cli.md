# One-shot `ask` command

`machine-god ask` runs one bounded, noninteractive prompt through the native
reference host and native conversation runtime. It does not introduce an
interactive UI or command-line permission-mode override.

## Grammar

The accepted form is:

```text
machine-god ask [--] <prompt...>
```

One or more Unicode prompt arguments are joined with one ASCII space. A single
`--` ends option recognition, allowing a prompt part that begins with `-`.
`--` is not part of the prompt. There are no other accepted `ask` options in
this grammar, including no process-model override.

The complete joined prompt must:

- contain at least one non-space, non-tab, non-CR, non-LF byte;
- contain no NUL byte; and
- contain at most 256 KiB of UTF-8, matching the default core prompt bound.

Join accounting is checked before allocation. Missing, empty, non-Unicode,
oversized, extra-invalid, or unsupported-option input is rejected by the
global invalid-arguments contract with exit `2`. Parsing and prompt validation
finish before configuration, current-directory, state, credential, runtime,
session, or network effects. Standard input is never read.

## Native composition

On Linux and macOS, a valid request first starts an owned signal guardian and
then:

1. captures the process-native environment;
2. loads the strict native configuration;
3. captures the current workspace, then selects and prepares identity-checked
   workspace and state roots;
4. validates one inference credential, then borrows it to load the rich model
   catalog over a host-owned current-thread Tokio runtime with I/O and time
   enabled, awaiting a completed cache observation;
5. consumes that same credential into the production reference host with
   complete terminal, shared undo, conversation-model routing, and native file
   observation and native permission allocations;
6. creates one fresh durable native conversation using a bounded random-identity
   operation, with the verified selected workspace, explicit current Unix time
   in milliseconds, and `Cli` origin in its initial metadata; and
7. enqueues exactly one prompt in `NativeConversationRuntime` and drives its
   admitted turn through native checkpoint finalization.

Root preparation may create only the private fixed state suffix described by
[native root selection](native-root-selection.md), and it occurs before
credential discovery. Missing or invalid inference credentials stop startup
before any catalog request; the anonymous-listing credential path is not used
to admit inference. Catalog and inference reuse one validated acquisition,
without a second credential lookup. Invalid grammar creates nothing. Once
session creation succeeds, the session remains durable even if the provider or
turn later fails.

Catalog loading completes before terminal-host acquisition, while the signal
guardian remains in setup mode. A pending catalog therefore cannot delay setup
signal exit or leave terminal workers awaiting cleanup. A completed failed
catalog observation permits inference without advertised capabilities; it does
not discard requested model settings. Catalog transport/backend construction
failure remains an operational setup failure. The catalog uses the existing
bounded provider/cache contract, including its authenticated rejection fallback
and shared request deadline, and prints no catalog output here. Shared host
preparation retains the acquired cache alongside the conversation resources,
so a long-lived CLI owner can reuse its authenticated provider without another
credential lookup. One-shot ask does not initiate a later refresh.

The native runtime begins with the configuration's requested model, effort, and
fast-mode preferences. Admission persists those requested values in the session
and pins effective controls for the taken job. Only explicitly advertised
effort/fast controls reach the Gateway; unavailable or unsupported capabilities
omit those wire controls without rewriting the requested preferences. Ordinary
`ask` does not write user defaults. Provider context selection and continuation
checkpoints belong to the [native conversation](native-conversation.md), not
CLI-owned product state.

The host's nine file-history adapters and each created or resumed conversation
share one `NativeConversationObservations` allocation. Attachment happens before
runtime admission and an attachment failure stops setup. Native code correlates
observations and publishes durable history; the CLI neither copies observation
state nor writes history metadata itself.

Before admission, the CLI attaches the host's exact permission controller and
review context to the conversation. The controller enforces configured mode,
patterns, saved rules and final file approvals. The owned Tokio runtime also
drives the dedicated automatic reviewer. On macOS the constructor worker
explicitly attempts to retain the fixed system sandbox executable. Missing
authority remains missing: a later Os launch fails instead of running unconfined;
None and Yolo require no sandbox executable.

Targets outside Linux and macOS fail through one fixed unsupported operational
path without importing or attempting the complete reference-host composition.

The constructor worker captures the account shell and full environment once,
then explicitly supplies the CLI executable as the private terminal helper.
An available tmux executable is selected from that frozen PATH. The resulting
terminal tool exposes all twelve actions with shared lossless input/result
archives; ordinary transcript limits remain unchanged.
Library embeddings are not assumed to implement the CLI's private helper modes.
After the turn succeeds, fails, or unwinds, the constructor worker drops the
host and waits for its terminal worker scope to settle before returning the
command outcome. Settlement includes collected worker joins and transferred
child reaping, not consumption of tool-result futures. Other hosts' workers do
not delay this wait; the async poll thread performs no blocking join.
Once a turn is active, the guardian remains in turn-forwarding mode and the
turn signal receiver stays owned through settlement, including operation errors
and unwinds. A first signal arriving during cleanup remains deliverable and
determines the outcome; an already accepted turn signal keeps precedence.
Only after native workers join may the worker enter final handling or ask the
guardian to finish a stalled output path. The signal receiver retains the first
observed signal independently of the turn future. If an error or unwind loses
output-progress state, that
signal still requests final exit after settlement, so a blocked borrowed writer
cannot trap the command in final handling. Without a signal, errors and unwinds
retain the ordinary fixed operational-failure diagnostic.

## Noninteractive authority

The command never prompts on standard input. Native configured and saved policy
is evaluated first. Proven built-in bypasses and permitted Auto/Yolo actions may
execute; any remaining human-prompt requirement receives a per-request denial.
Auto review returning Ask or failing denies for replanning without a human
fallback. The rootless
`ask_user_question` tool receives its fixed unavailable outcome. The model may
continue after either result, but neither path grants authority or starts
detached interaction.

`--auto`, `--yolo` and `--prompt-permissions` are not exposed by this grammar;
permission mode comes from validated native configuration. Images, JSON, quiet or TTY
presentation, no-save operation, resume, replay, and recovery flags also remain
outside this one-shot form.

## Presentation and exits

Only assistant `TextDelta` payload bytes are written to standard output, in
event order and without terminal styling, forced newline, or buffering the
complete answer. Reasoning, usage, lifecycle events, session identities,
permission details, tool calls/results, and provider diagnostics are not
printed. A successful empty answer therefore writes no bytes. When a started
turn reaches a terminal path and no output operation has failed, the command
explicitly flushes acknowledged output. Failures before a turn or output bridge
exists schedule no standard-output operation and therefore no flush. A write or
flush failure is an output failure unless an already observed signal has
precedence; a failed writer is not retried. A writer panic is not converted into
an apparently recoverable output failure.

- A completed turn exits `0` after native checkpoint finalization succeeds and
  all preceding text bytes are written. A finalization failure cannot become a
  successful `Completed` event.
- Invalid grammar exits `2` with the global invalid-arguments diagnostic.
- Configuration, root, credential, composition, session, provider, engine,
  terminal-event, and runtime failures exit `1` with one fixed redacted
  `machine-god ask` diagnostic.
- Standard-output failure cancels or drops the owned turn, exits `1`, and uses
  the existing fixed output diagnostic.
- During a live turn, `SIGINT` and `SIGTERM` request cancellation, keep driving
  owned work to terminal cleanup, and exit `130` and `143` respectively. Once
  one signal is accepted, later signals are coalesced until cleanup finishes;
  the first accepted signal determines the exit.
- During configuration, root preparation, catalog loading, host/session setup,
  final diagnostic presentation, or command finalization, the signal guardian exits with the
  same signal code. A blocking setup operation or saturated standard-error
  writer therefore cannot swallow the signal.

Synchronous standard-output work stays on the calling thread. The host runtime
and turn run on one scoped worker and exchange one owned output item at a time
over capacity-one work and acknowledgement channels. A separately owned
current-thread signal runtime registers before valid-request effects and uses
capacity-one signal and control channels. It switches from setup handling to
turn forwarding only after a concrete cancellable turn exists, then stays live
through diagnostics and final process exit. Output or setup backpressure
therefore cannot stop signal observation or leave Tokio's installed Unix signal
handler without an active receiver. If signal registration is only partially
successful, request effects do not start: the command takes the fixed
operational-failure path while retaining every installed listener through its
diagnostic and final exit.

After a turn signal, an outstanding write and any following flush share one
absolute 100 ms post-cleanup acknowledgement deadline; the flush cannot restart
the grace period. If the borrowed writer remains blocked, the guardian exits
the process with the signal code only after the turn has reached terminal
cleanup and the explicit terminal host's native workers have joined. The output
grace does not replace or shorten host settlement. Otherwise the scoped turn
worker joins before final presentation.

Partial assistant bytes already written before a later operational failure are
not retracted. No failure text may include the prompt, a credential, a path,
provider data, tool data, operating-system diagnostics, or a session identity.

## Resource and lifecycle bounds

- Prompt bytes: 256 KiB, including inserted join spaces.
- Assistant text: the core engine's default 1 MiB cumulative bound.
- Provider rounds, events, tool calls, tool results, transcript bytes, and
  permission reasons retain the default engine bounds.
- Fresh session-ID generation and collision retries are bounded in the native
  lifecycle API; OS randomness is acquired only after its future is polled.
- Catalog acquisition retains the [model catalog](models-cli.md) and
  [cache](model-catalog-cache.md) bounds. Only a completed observation is used
  for admission; the CLI does not treat a loading cache's retained data as ready.
- The host current-thread runtime, its scoped worker, the signal guardian and
  its current-thread runtime, four capacity-one output/signal/control channels,
  signal listeners, provider stream, permission or question future, tool
  future, turn lease, and output borrow remain owned by the command and are not
  detached.

This is scenario compatibility with the pinned upstream `ask` entry point, not
full option, presentation, persistence-mode, or performance equivalence.
