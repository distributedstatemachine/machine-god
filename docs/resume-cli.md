# Explicit-session `resume` command

`machine-god resume` continues one existing durable session with one bounded,
noninteractive prompt. It exposes the native lifecycle's current-schema resume
operation through the same streaming request path as [`ask`](ask-cli.md); it is
not a session picker, interactive shell, replay, reset, migration, or recovery
command.

## Grammar

The only accepted form is:

```text
machine-god resume <id> [--] <prompt...>
```

`<id>` is one explicit session ID: 1-128 ASCII bytes containing only letters,
digits, `-`, `_`, `.`, or `:`, and with a first byte other than `-`. The core
`SessionId` alphabet itself remains unchanged; rejecting a command-position ID
that begins with `-` is an intentional CLI parser tightening because that token
is option-like. The exact token `last` is also reserved for possible future
selection behavior and cannot name a resume target. One or more Unicode prompt
arguments follow the ID and are joined with one ASCII space into exactly one
prompt. A single `--` after the ID ends option recognition and permits the first
prompt part to begin with `-`; it is not part of the prompt.

The joined prompt must contain at least one byte other than space, tab, CR, or
LF, contain no NUL byte, and contain at most 256 KiB of UTF-8. Join accounting
is checked before allocation. Missing or invalid IDs, missing or blank prompts,
non-Unicode input, oversized prompts, misplaced delimiters, extra-invalid
arguments, and unsupported options use the global invalid-arguments diagnostic
and exit `2`.

Grammar and prompt validation complete before configuration, current-directory,
state-root, credential, runtime, session-store, or network effects. Standard
input is never read. There is no implicit `last` session, picker, alias,
`--resume`, `--resume-last`, `--continue`, short flag, JSON form, or interactive
mode in this slice.

## Native composition and authority

On Linux and macOS, a valid request starts the same owned signal guardian used
by `ask`, then captures the process-native environment, loads the current
strict configuration, captures the current workspace, prepares the selected
workspace and state roots, and composes the production AI Gateway reference
host. It resumes `<id>` through that host's retained
`NativeSessionLifecycle`, then runs exactly one prompt turn on the returned
engine-canonical session.

Resume accepts only a valid current-schema record under the state root selected
for this invocation. Missing, corrupt, future-schema, incompatible-incarnation,
or otherwise unloadable state fails; the command does not import, migrate,
reset, repair, skip, or rewrite it as a convenience. The session ID is never
selected from a directory listing, and this path does not allocate a new
session identity.

The stored transcript is provider-neutral. The continued turn deliberately
uses the provider, model, transport, credentials, engine limits, permission
adapter, tool catalog, current workspace, and other authority from the current
invocation's configuration and composed host. It does not resurrect historical
credentials, configuration, workspace authority, pending prompt UI, permission
decisions, or external tool effects from the durable record.

As with `ask`, this noninteractive host denies every permission-gated native
capability per request and gives `ask_user_question` its fixed unavailable
outcome. Neither path reads standard input, grants authority, or starts detached
interaction.

Targets other than Linux and macOS return the fixed operational failure for
valid grammar without attempting the complete native reference-host
composition. This is deterministic unsupported behavior, not partial support.

## Streaming, durability, and exits

Assistant text uses the `ask` command's output bridge and lifecycle contract.
Only `TextDelta` payload bytes are written to standard output, in event order,
without styling, a forced newline, or whole-answer buffering. Reasoning, usage,
lifecycle events, the session ID, provider diagnostics, permission details,
tool calls, and tool results are not printed. Successful terminal completion
flushes acknowledged output.

- A completed turn exits `0` after all preceding assistant bytes are written.
- Invalid grammar exits `2` with the global invalid-arguments diagnostic.
- Configuration, root, credential, composition, session-load, provider,
  engine, terminal-event, and runtime failures exit `1` with the fixed redacted
  diagnostic `machine-god resume: request failed` followed by LF.
- Standard-output failure cancels or drops the owned turn, exits `1`, and uses
  the same fixed output diagnostic as `ask`.
- `SIGINT` and `SIGTERM` use the `ask` signal guardian, cleanup, first-signal
  precedence, and exit codes `130` and `143`. The same 100 ms absolute
  post-cleanup acknowledgement deadline applies to an outstanding write and
  any following flush.

Partial assistant bytes already acknowledged before a later failure are not
retracted. No diagnostic may include the prompt, session ID, credential, path,
provider data, tool data, configuration value, or operating-system detail.

A turn persists through the core session contract. Once the resumed turn has
started, its user message may already be durably appended before a later
provider, engine-output, standard-output, signal, or final presentation
failure. Such a failure does not roll back the durable user turn or any
earlier committed transcript prefix. The next explicit inspection or resume
observes whatever current record successfully committed.

The command makes no cross-process serialization claim. Process-local engine
state converges same-incarnation resumes only within one composed host, while
the file store's advisory lock and compare-and-swap fence individual durable
operations. Another cooperating process may load the same revision and begin
work concurrently. Prompt reservation handles a conflict with at most 32
reload-and-retry attempts: a loser may reconcile a same-incarnation user-message
prefix committed by another process, append its own prompt, and send that
combined prefix to its provider. A later assistant commit whose transcript has
diverged fails closed instead of merging assistant results. The CLI neither
holds a process-wide session lease for the complete provider turn nor excludes
processes that ignore the store protocol.

## Resource and compatibility boundary

The prompt, assistant text, provider rounds, events, tool calls/results,
transcript, metadata, and structural JSON retain the bounds documented for
[`ask`](ask-cli.md), the [core engine](core-api.md), and the
[session lifecycle](native-session-lifecycle.md). The initial lifecycle resume
loads at most one current-schema record within the file-store cap. Subsequent
prompt reservation and assistant persistence use the core store contract;
compare-and-swap validation may reread the bounded record, and reservation may
perform the bounded conflict retries described above. The command may create
the record's permanent lock sidecar. It adds no directory enumeration or
ID-generation retry. It reuses `ask`'s bounded scoped worker and owned signal
guardian, so no thread or task remains detached. Semantic provider and conflict
retries are bounded, but the inherited file store has no attempt or wall-clock
ceiling for advisory-lock acquisition, filesystem latency, or retries after
`EINTR`.

The pinned upstream fx surface accepts implicit-last selection, aliases,
recording, and interactive continuation forms. Machine-god intentionally
implements only the explicit-ID, one-prompt scenario above. Matching the
observable ability to continue a selected session is scenario compatibility;
it is not grammar, option, presentation, persistence-format, concurrency, or
performance equivalence.
