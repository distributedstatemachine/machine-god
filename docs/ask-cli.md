# One-shot `ask` command

`machine-god ask` runs one bounded, noninteractive prompt through the native
reference host. This first slice owns the prompt-to-provider-to-session path;
it does not introduce an interactive shell or a broader permission mode.

## Grammar

The accepted form is:

```text
machine-god ask [--] <prompt...>
```

One or more Unicode prompt arguments are joined with one ASCII space. A single
`--` ends option recognition, allowing a prompt part that begins with `-`.
`--` is not part of the prompt. There are no other accepted `ask` options in
this slice.

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
4. composes the production AI Gateway reference host over one host-owned
   current-thread Tokio runtime with I/O and time enabled;
5. creates one fresh durable session using a bounded native random-identity
   operation; and
6. runs exactly one prompt turn to a terminal engine event.

Root preparation may create only the private fixed state suffix described by
[native root selection](native-root-selection.md), and it occurs before
credential discovery because the prepared-root reference-host constructor
consumes roots before constructing the transport. Invalid grammar creates
nothing. Once session creation succeeds, the session remains durable even if
the provider or turn later fails.

Targets outside Linux and macOS fail through one fixed unsupported operational
path without importing or attempting the complete reference-host composition.

## Noninteractive authority

The command never prompts on standard input. Every permission-gated native
capability receives a per-request denial from the host adapter. The rootless
`ask_user_question` tool receives its fixed unavailable outcome. The model may
continue after either result, but neither path grants authority or starts
detached interaction.

`--auto`, `--yolo`, `--prompt-permissions`, persistent grants, and additional
permission modes belong to later permission hardening. Images, JSON, quiet or
TTY presentation, no-save operation, resume, replay, and recovery flags also
remain outside this slice.

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

- A completed turn exits `0` after all preceding text bytes are written.
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
- During configuration, root preparation, host/session setup, final diagnostic
  presentation, or command finalization, the signal guardian exits with the
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
cleanup. Otherwise the scoped turn worker joins before final presentation.

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
- The host current-thread runtime, its scoped worker, the signal guardian and
  its current-thread runtime, four capacity-one output/signal/control channels,
  signal listeners, provider stream, permission or question future, tool
  future, turn lease, and output borrow remain owned by the command and are not
  detached.

This is scenario compatibility with the pinned upstream `ask` entry point, not
full option, presentation, persistence-mode, or performance equivalence.
