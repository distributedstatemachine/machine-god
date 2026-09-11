# Owned MCP stdio transport

`machine_god_native::mcp::stdio` provides an explicitly configured Linux/macOS
process connection. Constructors and unpolled connect/submission/receive futures
perform no process, filesystem lookup or worker admission. It does not negotiate
protocols, allocate wire IDs, publish catalogs or grant tool permission.

## Launch and process ownership

`McpStdioLaunch` receives the selected trusted helper program/arguments, target
command/argv, complete selected environment, captured PATH, retained cwd file and
finite wire limits. Relative commands containing a slash and relative/empty PATH
entries resolve against that retained directory; bare commands search only the
captured PATH. No ambient PATH/environment fallback or shell command expansion is
performed. The host decides whether to select its inherited environment or a
configured environment; this adapter does not silently merge them.

Executable lookup and descriptor duplication occur on an owned worker. Lookup
requires a regular executable candidate and resolves its absolute path before
the existing bounded argv codec. Executable paths are not immutable executable
file capabilities; native filesystem changes can still cause launch failure or
replacement. The cwd itself remains descriptor-bound through the helper's
`fchdir`, including after a directory rename. The selected helper is trusted
native authority, not an arbitrary model-provided executable.

The existing captured helper has an explicit private persistent-stdin mode.
Its default captured-terminal behavior remains unchanged. Startup consumes exact
bounded fields through unbuffered reads, establishes a private session/group,
acknowledges READY, and waits for COMMIT before exec. The parent retains process
ownership before COMMIT and waits for the independent CLOEXEC exec-error receipt.
The startup socket then becomes target stdin; stdout remains a separate pipe and
target stderr is null. No unused stderr pipe or new dependency is introduced.
Startup/control acknowledgements never enter target stdout framing.

Each connection admits two collected workers through the existing bounded worker
registry: an I/O worker in its dedicated scope and an owner waiter in the parent
host scope. The dedicated scope closes after its single worker is admitted.
The owner waiter remains enrolled until that complete scope settles, including
deferred nested macOS inventory reap after the direct server child is gone.
An independent startup receipt makes connection readiness available before the
owner waiter finishes. Admission failure and owner unwind still close/collect the
dedicated scope. Closing a connection never closes unrelated host work. Host
scope closure alone does not cancel operations: hosts also cancel the explicit
lifecycle token. Dropping an uncompleted connect future cancels startup.

Close shuts down target stdin, allows up to one second for graceful exit, then
uses the existing process-group capture/kill/reap owner. Failure to finish a
bounded foreground cleanup transfers ownership into the existing reaper instead
of detaching children or reporting complete cleanup. The retained direct child,
original process group and positively captured descendants define ownership;
descendants escaping before observation are outside that existing boundary.
macOS accepts the explicit shared inventory-service helper.

## Queue, framing and submission

The non-clone connection provides independent `&self` send/receive futures so a
runtime can drain responses while awaiting writes. At most eight writes including
the active writer and two incoming frames are retained. Full write admission
rejects before submission; a full incoming queue pauses stdout admission while
retaining at most one additional 16 KiB undecoded read tail. Writes, cancellation
and deadlines remain serviced, and draining resumes decoding without requiring
the consumer to win a scheduling race against a normal response burst. Only one
receiver future is active at once. Caller-owned unpolled futures, consumed frames,
connections and runtime lineages require host-level bounds.

The worker uses nonblocking descriptors, 16 KiB reads and writes, at most two
write polls per loop, and at most a 5 ms idle observation interval. Productive
I/O does not sleep. Every loop observes connection/host cancellation and queued
request deadlines; queued tool cancellation also independently observes its live
core turn and runtime. Fixed per-loop input bounds prevent comments/empty lines
or output floods from hiding shutdown. Stderr is discarded, matching the pin.

The existing [NDJSON parser](mcp-runtime.md) owns framing limits: default 8 MiB,
hard maximum 16 MiB per frame, depth 64 and bounded JSON nodes. Partial EOF is a
protocol error, not clean discovery-close evidence. `receive` validates the
complete JSON-RPC envelope using that parser; malformed JSON closes the connection.
Already complete queued frames remain observable before the terminal error.
Pending writes receive explicit failure receipts, and pending receivers wake on
closure. Errors and debug representations redact commands, environment, payloads
and server diagnostics.

`receive_frame` uses the same single receiving lane and returns original bounded
JSON bytes alongside the validated envelope. Catalog/schema consumers use these
bytes instead of serializing the parsed tree, preserving exact numeric lexemes
and vendor fields. A consumed frame retains at most one wire-bounded raw buffer
in addition to its bounded decoded envelope; callers bound retained frame counts.
The existing `receive` API discards raw bytes and returns the envelope alone.

`close_observation` returns evidence only after the connection completion settles:
the terminal reason, observed clean/incomplete EOF or unclassified read end,
whether a partial decoder buffer remained, and the current count of unconsumed
complete frames. Decoder emptiness on cancellation is not observed EOF; unread
kernel/read-tail bytes remain unclassified. Queued complete frames must be drained
and validated before interpreting close evidence. Malformed queued JSON upgrades
the terminal reason to protocol failure. This observer makes no fallback decision.

`close_after_discovery_timeout` freezes new write admission and requests an owned
worker snapshot before cleanup, without manufacturing EOF or cancelling the
snapshot itself. `DiscoveryTimeoutQuiescent` denotes a distinct bounded-input
cutoff observation, not EOF. Only settled worker evidence can admit a negotiation
fallback; requesting the snapshot alone cannot. Explicit connection or host
cancellation still takes priority.

`admit_runtimes` registers at most 2,048 exact native runtime allocations after
catalog admission. It grants no permission. `submit` requires a non-clone
[`McpSubmission`](mcp-submission.md) belonging to an explicitly registered
allocation; equal names or numeric generations from another owner do not match.
Membership is checked at admission and after queue waits. The sole writer owns
the submission through its direct synchronous plaintext pipe writes and flush.
The existing guard revalidates the exact native proof/turn/runtime before every
delegation, and the final pipe adapter checks connection/host/request cancellation
and the original deadline immediately before writing.

Receipts distinguish attempted delegation, acknowledged bytes and submission
outcome; successful writing is not remote execution success. Error or cancellation
after any attempted delegation closes the connection before another frame can
follow a possible partial JSON prefix. Accepted prefixes are never restarted.
No transport-level retry or consequential request replay is provided.

`McpStdioControl` is separate typed protocol data under explicitly owned connection
authority. Its bounded constructors admit only discovery/initialize and read-only
catalog methods, initialized/cancelled notifications, or a fixed unsupported-method
error reply. They reject `tools/call`, resource reads, prompt gets and arbitrary
successful server-request replies. Application features and elicitation require
their separate native admission; no arbitrary raw-frame escape exists here.

## Compatibility and composition

Behavior follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, specifically
`src/core/mcp/mcp_runtime.zig` (`spawnStdioServer`) and
`src/core/mcp/stdio_dispatcher.zig` (serialized writes, precommit, reader and
shutdown ownership). Strict duplicate/JSON/resource admission, descriptor-bound
cwd, explicit environment authority and retained native cleanup are intentional
native boundaries. Startup is limited to 300 seconds; it does not impose that
deadline on the connected server's lifetime. Captured PATH is limited to 8 KiB
and 256 entries; target program paths to 4 KiB. The shared launch codec bounds
argv and environment as documented by its terminal contracts.
Deadlines bound controllable userspace waits, not a synchronous filesystem or
spawn syscall, arbitrary trusted waker callback, or an uninterruptible kernel
operation. Cancellation/deadline checks resume when that boundary returns.

Runtime composition remains responsible for exact connection generations, unique
wire IDs, stale/null-ID correlation, full result-shape validation, negotiation,
permission preparation, catalog refresh, bounded callback routing and CLI ownership.
Closing and awaiting an old connection precedes an admitted negotiation restart.
This component does not claim complete MCP feature acceptance or benchmark gains.
