# Native MCP stdio startup factories

`NativeMcpStdioStartup` turns an exact enabled stdio `McpServerConfig` snapshot
into a `NativeMcpStdioFactory` implementing the existing peer launch-factory
contract. Native owns the selected trusted helper/entrypoint, complete captured
parent environment, retained cwd `Arc<File>` and wire limits. Constructors and
factory calls do not read environment, inspect paths/descriptors, resolve an
executable, duplicate cwd, admit a worker or start a process.

## Frozen configuration and environment

Each factory retains the exact immutable `Arc<McpServerConfig>`, including
required/startup/operation/restart policy. It does not reinterpret negotiation or
restart policy. Disabled and non-stdio configurations cannot create a stdio
factory. Command and argument ordering, empty arguments and literal shell-looking
bytes remain unchanged; there is no shell invocation or variable interpolation.

Environment behavior follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`
`src/builtins/mcp.zig` (`parseSelectedEnvironment`, `parseEnvironment`) and
`src/core/mcp/mcp_runtime.zig` (stdio connection and `spawnStdioServer`):

- Empty configured environment selects the complete captured parent environment.
- Nonempty configured environment replaces it with only the configured entries.
- `environment` precedence over `env` is already resolved by the config codec.
- Values, including empty strings and `$NAME`/`${NAME}`, remain literal.

No missing variable is filled from ambient state. The existing native environment
codec validates and encodes the selected complete environment once: at most 512
entries, 1 KiB per key, 16 KiB per value and 256 KiB aggregate key/value bytes,
with no duplicate keys, NUL or `=` in a key. The independent config limits also
apply to explicit server entries. A replacement is bounded independently of the
parent snapshot, not charged as an accidental merged environment.

## Executable lookup is separate from child PATH

The exact [Zig 0.16.0 stdlib](https://codeberg.org/ziglang/zig/src/tag/0.16.0/lib/std/Io/Threaded.zig)
used by the upstream pin selects `spawnPosix` lookup PATH from its captured parent
environment, independently of the supplied child `environ_map`. Native follows
that distinction. A configured child `PATH` is passed literally to the child but
does not replace the lookup PATH used to locate the server executable.

Absent captured parent PATH selects the pinned fixed
`/usr/local/bin:/bin/:/usr/bin`, not an ambient lookup. Empty PATH segments are
removed as in the pinned tokenizer; an all-empty value gives no bare-name lookup.
Relative nonempty segments resolve against the retained cwd. Unix PATH bytes,
including non-UTF-8 segments, are retained without lossy conversion. The existing
launch codec still requires the final resolved executable spelling to be UTF-8;
unrepresentable candidates fail under that established boundary, not through
rewriting PATH bytes. Lookup admits at most 8 KiB and 256 original PATH segments.

Only the [stdio launch worker](mcp-stdio.md) performs executable/cwd resolution
after polling. The existing low-level public `McpStdioLaunch::new` retains its
explicit `Option<String>` API and its existing empty-segment behavior; native
startup applies the pinned empty-segment normalization before constructing its
internal immutable template. Both paths use the same byte-preserving resolver.

## Shared helper and restart ownership

On macOS, startup explicitly selects one inert inventory-service registration
before creating any factory. Repeated registration on the same startup authority
is rejected. Every server factory and negotiation restart shares the same helper
allocation and inventory registration. Creating a factory or cloning a launch
does not register or start another service. The existing helper owner retains
actual service startup, query, lease and cleanup behavior.

Cloned launch templates share helper, command, argv, validated environment and
encoded frame, lookup PATH and cwd allocations. No repeated environment encoding,
configuration/schema parsing or executable lookup occurs during factory cloning.
The exact retained snapshot can outlive the startup builder without recapturing
ambient state. A separately selected startup authority remains independent.

The existing peer drives negotiation/fallback and waits for old connection
cleanup before an admitted restart. The transport owns worker enrollment,
deadlines, cancellation, child lifetime and retained cleanup; the factory adds no
task, finalizer or detached work. Debug/errors omit commands, arguments,
environment, helper paths and cwd. Production host/CLI composition and the full
feature gates remain in the [implementation plan](implementation-plan.md).

## Observed configured startup

`McpStdioPeer::connect_observed` accepts explicit startup timeout, absolute outer
deadline and a synchronous fallible completion observer. It returns the ready
peer and selected attempt deadline; initial tools catalog loading shares that
remaining budget. Complete-startup retries, including catalog failures, belong
to the composer under the configured `restart_limit` (0–255), not an additional
peer retry loop. Application requests are never replayed by startup.

Configured timeouts are positive and at most `u32::MAX` milliseconds. Deadline
addition is checked and bounded by an outer deadline when the caller selects one. The
existing `connect` interfaces retain their 300-second admission behavior.
`McpPeerTimer::now` defaults to native monotonic time; explicit hosts can provide
the same `Instant` domain alongside their existing injected timer. Child-side
process deadlines still use native monotonic time.

The startup composer retains the selected `McpPeerLifetime` on each ready peer.
`OwnerControlled` adds no expiry; `Until` can only narrow an existing expiry.
Catalog, feature, proof-bearing call and runtime admission reject expired peers
before transport effects. Their request deadlines are constrained before queue
admission, so the owned transport also enforces expiry on queued writes and
partial suffixes. Completing startup does not discard that policy.

`McpStdioPeer::connect_configured_observed` omits an overall deadline while
retaining the full finite configured budget for each admitted attempt. It does
not implement an infinite attempt or a far-future timestamp. Explicit-deadline
callers retain their existing outer cutoff. Cancelled owner admission fails
before the launch factory or observation callback. Native inter-attempt cleanup
has a separate 30-second bound when no overall deadline was selected; legacy
disconnect/relaunch also remains inside that legacy attempt's budget. Unsettled
cleanup rejects restart and retains the existing worker/completion ownership.

The pinned `connectionAttemptControl` gives modern discovery one configured
attempt, then modern-to-legacy fallback a fresh attempt. All subsequent legacy
version retries share that legacy deadline; they do not each reset it. The
outer deadline and cancellation remain live during cleanup and fallback.
Positive, settled old-connection evidence is still required before reopening;
malformed success and ambiguous application operations do not authorize it.

The launch companion invokes its observer on first poll after creating an inert
child scope and before admitting a host worker, resolving paths or spawning a
process. It holds no transport mutex during the callback. Returning false rejects
with capacity; the unadmitted scope closes on rejection, cancellation, host
admission failure or callback unwind. An observed completion therefore remains
settleable even if startup never returns a peer. Once admitted, existing host
ownership retains child and deferred-reap cleanup after cancellation or dropped
futures. Observation adds no worker, waiter task or detached finalizer.

Observers must use a finite receipt ledger, pruning only positively complete
entries. Each fallback is observed separately; a full ledger rejects before new
effects. Hosts must wait for previous observed cleanup before full-startup
retries and keep the finite ledger across all 256 configured attempts. Observation is
cleanup data, not execution authority or proof of remote effect reversal.
