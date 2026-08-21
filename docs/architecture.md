# Architecture

`machine-god-core` contains provider-neutral contracts and orchestration without
ambient operating-system authority. `machine-god-native` provides explicit native
capabilities. `machine-god-cli` composes them. `machine-god-testkit` provides
deterministic test doubles.

The testkit's concrete boundary doubles are documented in
[`testkit.md`](testkit.md). They are executor-neutral like core: finite scripts,
cancel-driven pending provider/tool work, and permanently pending observer or
policy steps support precise manual polling without clocks. Each double owns a
single mutex per related script-and-record state so concurrent call ordering is
linearizable and inspection returns a consistent snapshot. The in-memory store
executes comparison and mutation inside that same critical section, making its
optimistic revision behavior atomic rather than merely convenient for
single-threaded tests.

The milestone-02 public contracts are documented in
[`core-api.md`](core-api.md). Public interfaces keep model access, storage,
tools, permission policy, and event delivery behind object-safe traits. Core
uses standard futures and `futures-core::Stream`; it does not select or require
an async executor.

Milestone 03 has three bounded slices. The first two are native-host slices;
the third extends the authority-free core tool contract without adding an
executable native capability. `machine-god-native` snapshots only
`XDG_CONFIG_HOME`, `XDG_STATE_HOME`, and `HOME`, resolves namespaced config and
state paths, and inspects their final metadata for status. A separate
synchronous native authority can load the resolved config file read-only.
`machine-god-cli` remains a thin formatter for status, does not invoke the
loader, and owns no product state. The exact surfaces are documented in
[`cli.md`](cli.md) and [`configuration.md`](configuration.md).

```text
process environment -> resolved native paths
 config path --+-> final metadata -----------> status -> CLI text/JSON
               +-> bounded read-only loader -> schema-v1 config
 state path ------> final metadata -----------> status -> CLI text/JSON
```

The config and state roots resolve independently. A nonempty XDG root wins and
must be absolute Unicode; an invalid selected root fails that location without
trying `HOME`. An empty XDG value falls back to a nonempty absolute-Unicode
`HOME`. Missing or empty `HOME` makes a needed fallback unavailable. The only
paths produced are `<config-root>/machine-god/config.json` and
`<state-root>/machine-god`, with `.config` and `.local/state` inserted for the
respective `HOME` fallbacks.

Status inspection remains deliberately shallower than configuration loading. It
uses `symlink_metadata` on the final path, reports
missing/inaccessible/wrong-kind states, and treats a final symlink as
wrong-kind. It does not open, read, or parse the config file. Permission mode is
fixed to `ask`; executable native tools and permission prompting are outside
these slices. The CLI serializes paths as JSON strings even in human status so
path contents do not become terminal controls. Bare invocation keeps the
bootstrap identity contract. Help, version, status, and argument errors remain
byte-stable presentation behavior, not an engine-owned command model.

The synchronous loader resolves only the config location. An unavailable
location or missing file yields an explicit built-in schema-v1 configuration
with permission mode `ask`; invalid selected environment input and all other
load failures fail closed. A present file must be at most 64 KiB of valid UTF-8
and must be exactly a schema-v1 object with `schema_version` equal to `1` and
`permission_mode` equal to `"ask"`. Unknown, duplicate, missing, wrong-type, or
unsupported fields and values are rejected.

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path no-follow and nonblocking. A preliminary path-kind check is followed
by authoritative opened-descriptor regularity validation. The loader retains at
most the 64 KiB cap plus one byte and never writes, creates, or canonicalizes.
Hardened open semantics for non-Unix targets remain deferred. Typed diagnostics
distinguish failure classes without reflecting selected paths, file contents,
or operating-system error text. Configuration mutation, permission modes beyond
`ask`, prompting, providers, executable native tools, durable native sessions,
and CLI expansion remain deferred.

```text
                        machine-god-core
 host ---------------------------------------------------------------+
  |                                                                  |
  +-> ModelProvider ----+                                             |
  +-> SessionStore -----+-> Engine -> Session -> Turn event stream    |
  +-> PermissionHandler +                  |                          |
  +-> Tool(s) ----------+                  +-> TurnHandle/cancellation|
  +-> EventSink (optional observer)                                   |
                                                                     |
 native filesystem / process / network authority remains outside ----+
```

An engine requires explicit provider, store, and permission components. Event
observation may use the authority-free no-op sink. Validated IDs, explicit
durable session-incarnation IDs, structured component errors, optimistic session
revisions, monotonic event sequences, one-live-turn session leases, and
idempotent cancellation form the initial cross-component invariants.

The multi-round turn loop is an executor-neutral future polled inline by the
`Turn` stream. A one-event acknowledgement gate connects it to observer
delivery: orchestration cannot advance past a nonterminal event until the sink
accepts that event and the caller receives it. No task, channel, timer, or
runtime-specific primitive is required. Provider, store, policy, and tool
futures stay owned by that orchestration frame, so cancellation or drop tears
down the in-flight phase without detached work. Immutable tool specifications
are cached in deterministic name order when the engine is built and cloned into
each provider request.

Durability divides each loop into explicit phases:

```text
user+turn reservation -> model
model tool calls -> atomic assistant + N unknown placeholders commit
                 -> effect-free tool preflight -> bounds -> permission
                 -> allowed tool -> exact in-place result replacement -> model
model final answer -> assistant commit -> terminal events
```

Tool preflight is a provider-neutral transformation before policy. The
source-compatible `Tool::prepare` default returns the provider's original JSON
arguments with the existing raw `Capability::Tool`. A tool may instead return a
`PreparedToolCall` containing a normalized capability and replacement arguments.
Preparation is synchronous trusted-host code and must be deterministic,
bounded, nonblocking, and free of external effects. Core checks cancellation
immediately before and after the call; it cannot interrupt preparation in
flight. Core validates the prepared arguments at the exact configured tool
argument-byte limit, including their JSON depth and node bounds. For a prepared
capability, depth and node traversal covers only JSON values embedded in its
`Tool` or `Custom` variant. Every variant is also serialized as a whole under
one total byte cap of the configured argument limit plus 1 KiB. That fixed 1
KiB is headroom within the total capability cap, not a separately metered
envelope. Core then presents the prepared capability to policy and passes
exactly the prepared arguments to `Tool::execute` only after policy allows the
request.

The trusted tool must ensure those arguments can drive only effects contained
by the exact capability that policy authorized. Filesystem, process, and network
implementations must not reinterpret normalized arguments into a broader path,
command, or destination. This obligation keeps authorization and execution
about the same normalized operation without giving core semantic knowledge of
tool JSON or ambient operating-system authority.

A preparation error consults no permission handler and starts no tool. It
replaces the already-durable unknown placeholder with the same fixed generic
tool-error result used for an execution error, then permits the next model round
to recover. Permission request identity, critical risk, fixed reason, event
ordering, cancellation precedence, and the absence of core-side grant caching
are unchanged. The cancellation claim covers the immediate checks around the
synchronous preflight; preparation itself must not block because it cannot be
cancelled while running.

The transcript prefix is the optimistic-merge boundary. Allocator and metadata
changes may advance across a retry while messages remain identical; any message
change is divergence and fails closed. This preserves external allocator work
without guessing how concurrent conversation suffixes should be ordered.
Prompt, inference-option, session-metadata, tool-catalog, and complete-transcript
message/serialized-byte limits are checked before their corresponding build,
store, or model boundary. Every committed call therefore has exactly one result
message even if cancellation, an infrastructure error, or a process interruption
prevents its real result from replacing the conservative placeholder. The next
model round is not requested until all placeholders in the current round have
been replaced.

Every reachable `serde_json::Value` in these boundaries also passes an iterative
container-depth and node-count walk before recursive serialization or deep
cloning. A scalar root is depth zero and a root container is depth one; every
root, scalar, and container counts as one node. One node budget aggregates the
entire tool-schema catalog, all inference metadata, or all record metadata and
message JSON. Provider arguments and tool results are independently bounded.
The walk stores only the iterator for each active ancestor, never all pending
siblings, making its auxiliary memory proportional to configured depth and
stopping at node limit plus one. Provider tool arguments are rejected before
policy or execution. A tool result is checked after the effect but before
serialization and durable replacement; an over-depth or over-node result leaves
the precommitted unknown placeholder and terminates without replay.

The configurable depth is subordinate to the public hard safety ceiling
`MAX_SAFE_JSON_DEPTH` (64). Engine construction rejects a larger configuration
before catalog traversal or any provider, store, or policy call and never
clamps it. Iterative validation alone cannot make an accepted 50,000-level tree
safe for subsequent recursive serialization, cloning, retention, extension
components, and ordinary destruction, so this is a structural invariant rather
than an operator-tunable resource budget.

Owned rejection paths replace each hostile `Value` with `Null` and consume the
original through an iterative child-iterator stack before the surrounding
object is dropped. This applies to builder abandonment, duplicate replacement
and build errors; dropped unpolled prompts; direct and conflict record loads;
mutation candidates; yielded provider calls; and tool output after an effect.
Cancellation-aware poll boundaries drain a just-ready yielded provider event,
conflict-loaded record, or tool output before honoring cancellation.
Normally yielded provider events are guarded before model-event counting, so a
limit or counter failure also drains their arguments iteratively.
Reclamation is O(actual nodes) time with O(actual depth) auxiliary memory and
does not leak rejected trees. An item still queued inside a provider's stream
has not crossed the core ownership boundary, so stack-safe destruction of that
internal queue remains the provider's responsibility.

Canonical session state stores its record behind an immutable `Arc`. Reservation
and transcript mutation capture only that cheap identity and persistence bit
under the session mutex, then serialize and deep-clone outside the critical
section. Immediately before starting a store compare-and-save, core reacquires
the mutex and requires the exact record identity and persistence bit to match;
otherwise it retries from the new snapshot. A state change after that recheck is
still caught by the durable revision CAS. Reconciliation similarly performs
whole-record comparisons outside the mutex and uses pointer identity to recheck
before a constant-time state update.

A live turn owns the session lease and provider cancellation signal as one
lifecycle unit. Destruction signals cancellation before removing waiters and
releasing the lease; terminal completion has already released that unit, so its
later destructor is cleanup-only. Out-of-band observer or delivery-state failure
before a terminal provider outcome follows the same cancel-before-release rule,
preventing dropped streams from orphaning retained provider work.
Terminal establishment is the cancellation precedence boundary. A final model
stop remains preterminal through its assistant-message save so cancellation can
wake and release a turn blocked on persistence. If eager save construction
requests cancellation and returns a future whose first poll succeeds, or that
first poll itself requests cancellation and succeeds, durable success is
authoritative: core checked cancellation immediately before construction,
always gives the new future that one poll, and then runs reconciliation and
terminal establishment synchronously before the workflow yields to the outer
turn. A pending future receives cancellation prechecks on every later poll.
After the successful boundary, the stop
retains its pending observer delivery and final reason even if cancellation
races afterward. Provider failures and missing stops establish their terminal
outcome when accepted because they have no final assistant commit.
Provider startup, stream, persistence, policy, tool, and observer delivery polls
include a post-poll cancellation observation before their result is
interpreted. Cancellation observed there establishes the terminal outcome first
while the turn remains preterminal, except for that narrow ready successful
final-save boundary; pending or ready-error final saves remain cancellable. Only
a locally synthesized cancellation bypasses observer delivery; a
provider-originated cancelled stop follows normal durability and observer
ordering.
Cancellation treats wakers as user-controlled callback objects: cloning happens
before locking, registry mutation only moves values, and superseded, removed, or
drained wakers are dropped or invoked after unlocking.

Each `Engine` owns a weak session-state registry keyed by `SessionId`. All
create/load races inside that engine converge on one in-memory record and active
turn flag only if the persisted `SessionIncarnationId` also matches. A collision
between the same live session ID and a different incarnation fails rather than
merging logical lifetimes; a live turn itself keeps the state alive if its
originating session handle is dropped. This is an in-process coordination
boundary, not a distributed lease. Registry access uses one requested-ID
`BTreeMap` lookup rather than scanning all live sessions. The
last owner removes its weak entry during state destruction only when pointer
identity still matches, so dead keys are reclaimed without an old destructor
removing a concurrently installed replacement. Registry lookup holds the
entries mutex only through weak-reference upgrade or
new-state insertion. Existing-state identity validation runs after unlocking
while the upgraded strong reference preserves lifetime. An incarnation conflict
can thus drop the last state owner and reenter registration cleanup without
self-deadlocking on the entries mutex. Independent engines and
processes coordinate durable turn-number allocation through the session store's
optimistic revision contract. Loaded records reconcile strictly and
monotonically: corrupt sequences, stale revisions, and equal-revision divergence
are protocol errors, and completion of an older in-flight save cannot replace a
newer canonical record. Successful-save reconciliation also rejects divergent
records at the same revision. Session stores preserve a host-generated globally
unique incarnation for the entire logical record lifetime and reject a save
that changes it. Reset, rewind, or reuse of a session ID requires the host to
rotate the incarnation; core neither guesses legacy values nor acquires
randomness or clock authority. Model requests, permission requests, tool
contexts, and engine events carry that incarnation. The permission-request v2
digest binds it alongside session, turn, and ordinal identity to prevent an
ID-cached allow from crossing a reset; tool idempotency and event-sink
deduplication can use the same durable lifetime identity. Intrinsic load
validation precedes registry
publication, preventing a concurrent handle from retaining invalid persisted
state even when the originating load returns an error. Revision zero is an
in-memory unsaved sentinel only; persisted loads and conflict reloads require a
positive optimistic-concurrency revision. Missing conflict reloads may change
persistence status only with an exact snapshot comparison under the session-state
lock, while record revisions stay monotonic independently of that status flag.
The durable turn allocator is monotonic independently of both fields: a higher
revision cannot authorize a lower `next_turn_sequence`. Higher revisions may
advance conversation messages and metadata only after passing that allocator
guard, while equal revisions require whole-record identity.

Diagnostic formatting is also an authority boundary. `Engine::fmt` emits only
fixed structural state (`has_provider` and tool count); it never invokes the
provider's `name` method or copies provider-controlled text.
