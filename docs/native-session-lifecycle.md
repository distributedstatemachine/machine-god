# Native by-ID session lifecycle

Status: delivered fifteenth bounded Milestone 03 slice. Production, fourteen
independently owned focused tests, one formal finding regression, and all three
adversarial tracks are green; the tracks agree on exact candidate `e6a3804`.
Feature record `dbba2c7` is green under exact feature CI `32594562796` and
benchmark evidence `32594562785`, and on `main` under exact CI `32594846484`
and benchmark evidence `32594846476`. This documentation-only commit is the
final delivery record; its workflows are reported at handoff. Fifteen bounded
slices are integrated. A composed sixteenth candidate extends this lifecycle
with bounded IDs-only listing; production/test composition, formal review, and
delivery evidence for that extension remain pending. Milestone 03 remains
`IN PROGRESS` because other frozen scope remains open.

This slice gives the Linux/macOS native reference host a durable, by-ID library
boundary for creating, resuming, replaying, and resetting sessions in the
current file-session schema. The caller supplies a validated core `SessionId`;
the host does not invent, enumerate, parse, or discover session IDs. The host
allocates the `SessionIncarnationId` for a new logical lifetime.

The owning component is `NativeSessionLifecycle`. Its production API exposes
typed construction and operation errors plus inert `create`, `resume`,
`replay`, and `reset` futures. `NativeReferenceHost` retains it and
exposes lifecycle and session-store observation over the same shared
`FileSessionStore` instance. The engine, lifecycle component, and store
therefore use one retained state-root identity rather than reopening a path or
constructing independent stores.

The separate [`native session-listing contract`](native-session-listing.md)
defines the candidate `list_sessions` operation. It enumerates that same
retained store root but does not change the by-ID create, resume, replay, or
reset semantics below.

## Platform and authority boundary

The standalone lifecycle API is exported on Linux and macOS independently of
the optional `ai-gateway-http` feature. Its constructors accept any caller-built
`Engine` and `Arc<FileSessionStore>` that satisfy the exact shared-allocation
invariant below. Integration through `NativeReferenceHost` retains that host's
stricter optional-HTTP, non-WebAssembly, Linux/macOS gate. Prepared-root host
composition uses the selected `machine-god` state root itself; legacy host
composition uses the explicitly supplied existing session root.

The lifecycle takes no separate path, environment, configuration, network,
terminal, process-execution, clock, or runtime authority. It does retain the
engine and file store supplied by its host, so it transitively retains the
engine's provider, permission, tool, and event-sink components; lifecycle
operations never invoke those components. Its default incarnation source uses
only OS randomness. A custom `SessionIncarnationSource` is explicitly trusted
host code: its implementor is responsible for its own authority, effects,
latency, allocation, internal work, and globally unique ID contract. It must
not be model- or configuration-controlled merely because the lifecycle exposes
an injection boundary.

Store identity is an enforced invariant, not caller documentation. Every
lifecycle constructor proves that the engine's configured session-store `Arc`
and the supplied `Arc<FileSessionStore>` are the exact same allocation before
it exposes a lifecycle value. An equivalent path, reopened descriptor,
independently constructed store, or merely equal behavior does not satisfy the
check. Mismatch returns the fixed redacted
`NativeSessionLifecycleBuildErrorKind::MismatchedSessionStore`, whose stable
name is `mismatched_session_store`, before the incarnation source or filesystem
is consulted. `NativeReferenceHost` constructs one concrete shared store and
wires that same allocation into both components; an impossible internal
mismatch maps to its existing redacted engine-construction stage.

Default production composition through `NativeReferenceHost` adds only one new
native authority: a bounded operating-system random source used to allocate an
incarnation for `create` and `reset`. It must not fall back to a timestamp,
process or thread ID, counter, model output, session ID hash, or general-purpose
deterministic generator. Standalone constructors may instead receive a trusted
custom-host source, whose additional authority and obligations are assigned
above. Reference-host composition always selects the OS source and does not
expose the override as ambient, model-controlled, or configuration input.

Each default production incarnation carries at least 128 bits from a fixed-size
OS random draw and is encoded as a valid bounded `SessionIncarnationId`. Its
exact textual encoding is not a public wire or file-format promise in this candidate.
`MAX_SESSION_INCARNATION_ATTEMPTS` is `8`: reset considers at most eight source
values and never accepts one equal to the currently stored incarnation. Failure
to obtain an acceptable value within that bound fails closed before publication
and never reuses the prior incarnation.

## Operation summary

All four operations take one caller-supplied validated `SessionId` and return a
future. Missing `resume`, `replay`, and `reset` are typed `NotFound` failures,
not `Ok(None)`.

| Operation | Durable effect on success | Successful result |
| --- | --- | --- |
| `create` | Atomically creates one empty current-schema record at revision `1` with a new host-generated incarnation. | A live core `Session` for that exact durable record. |
| `resume` | No record write; loads and validates the current durable record. A present load may create the store's permanent lock sidecar. | The engine-canonical live `Session` for the stored incarnation. |
| `replay` | No record write; loads and validates one current durable snapshot. A present load may create the lock sidecar. | An owned `SessionRecord` snapshot, not a UI transcript or event stream. |
| `reset` | Atomically replaces the current record with an empty record under the same ID, a new incarnation, revision `old + 1`, and turn allocator `1`. | A live core `Session` for the newly persisted incarnation. |

No successful operation invokes the provider, transport, permission handler,
prompter, tool, or event sink. Returning a live session does not begin a turn.
Those effects remain behind a later explicit call to the core session.

## Durable create

`create` starts from an exact empty `SessionRecord`:

- the caller's `SessionId` is unchanged;
- the configured source supplies a fresh incarnation; default reference-host
  composition uses OS randomness;
- the unsaved candidate revision is the core zero sentinel;
- `next_turn_sequence` is `1`; and
- messages and metadata are empty.

The record is published through the store's atomic new-record compare-and-swap
with `expected_revision: None`. The existing `FileSessionStore` consequently
assigns revision `1`. Success is returned only after the record has passed the
store's file-sync, atomic-rename, and directory-sync sequence and the engine has
accepted the same persisted identity. A record already present for the ID maps
the new-record conflict specifically to `AlreadyExists`; it is never resumed,
reset, merged, truncated, or replaced as a convenience.

Concurrent creates for one absent ID use the same store lock and create CAS.
At most one can create that durable lifetime from the absent state; a caller
that reaches and loses the durable create CAS reports `AlreadyExists`. Within
one engine, however, a competing create may first encounter the other create's
incompatible local registry reservation and report `LiveSession` without
reaching the store CAS. The implementation never forces that reservation or
another incompatible live handle to adopt a different incarnation.

As in the underlying store, failure before rename leaves no published record,
although a permanent lock or safe temporary artifact may remain. A directory-
sync failure after rename is an `Unavailable` error with an ambiguous outcome:
the new record may already be visible. The caller must `resume` or `replay` to
reconcile before deciding what to do. Blindly retrying `create` is not a safe
generic response to that error.

## Resume

`resume` loads the durable record for the supplied ID through the exact shared
store and validates all current schema, identity, positive-revision, positive-
turn-sequence, structural JSON, byte, and configured engine limits before
returning a handle. Missing state is `NotFound`; corrupt or future-schema state
is an error and is not rewritten or skipped.

When the same ID and incarnation are already live in this host, resume returns
the engine-canonical shared state and reconciles only a valid non-regressing
durable revision. It does not publish a second local turn lease. If the ID is
locally live with an incompatible incarnation, resume reports `LiveSession`
without invalidating either identity or mutating durable state. The caller must
drop the obsolete local lifetime before the newer durable lifetime can be
resumed in that host.

Resume is not an automatic prompt, provider retry, replay, reset, or recovery
of a corrupt record. It returns the complete current durable session state from
which the caller may explicitly continue.

## Durable replay snapshot

`replay` loads and returns one owned `SessionRecord` snapshot from durable
storage. The snapshot contains exactly the stored ID, incarnation, revision,
next turn sequence, ordered provider-neutral messages, and metadata after the
file store's current-schema, fixed byte, and fixed structural validation. Replay
does not apply the composed engine's potentially smaller configured transcript,
metadata, or message-count limits because it does not publish a live session.
Its successful contents are intentionally visible to the trusted caller and may
contain user, assistant, reasoning, tool-call, and tool-result data. Error and
debug surfaces remain redacted as described below.

Replay does not:

- register or return a live `Session`;
- prefer an unpersisted in-memory handle over the store;
- reconstruct `EngineEvent` or `TurnEvent` values;
- re-emit events to the configured sink;
- reconstruct transport chunks, prompt UI state, permission decisions, or
  external tool effects; or
- invoke the provider, permission handler, tool registry, or network.

It is therefore a durable provider-neutral record snapshot, not a cinematic UI
replay. Another writer may commit after its load linearization point, so the
returned revision identifies a point-in-time value rather than a continuing
subscription. Missing state is `NotFound`.

## Reset and new incarnation

Reset reuses the caller's `SessionId` only after allocating and durably
publishing a new incarnation. It is not deletion followed by create and never
exposes a deliberate missing-record interval. For a validated current record at
revision `R`, one successful reset publishes:

```text
id                 = existing id
incarnation_id     = fresh host-generated incarnation
revision           = checked R + 1
next_turn_sequence = 1
messages           = []
metadata           = {}
```

The positive durable revision remains monotonic across reset even though the
new lifetime's turn allocator returns to `1`. `u64::MAX` cannot wrap. A checked
revision-assignment or serialization invariant failure maps to the fixed
`Engine` lifecycle category and leaves the prior record authoritative unless
the underlying store reports its documented ambiguous post-rename outcome.

Reset uses the permanent per-session advisory lock and a reset-specific atomic
compare-and-swap. The current record is loaded and validated, and its exact ID,
incarnation, and revision fence the replacement. The new envelope uses the
same bounded `0600` temporary-file, file-sync, same-directory atomic rename,
and directory-sync protocol as ordinary saves. A failure before rename leaves
the complete old record authoritative. A reader observes either the complete
old record or the complete reset record, never an intentional tombstone or a
partially serialized replacement.

Missing reset is `NotFound`. Malformed, unsupported-schema, wrong-ID,
nonregular, symlink, oversized, or otherwise corrupt state fails closed and is
not reset over. A CAS mismatch maps to `Conflict`; the lifecycle does not hide
it with an unbounded retry. Calls based on the same observed revision cannot
both replace that revision. Two distinct reset invocations may nevertheless
both succeed if they linearize sequentially, in which case each success creates
a distinct incarnation and advances the durable revision once.

An `Unavailable` result from final directory synchronization is ambiguous in
exactly the same way as an ordinary store save: the new incarnation may already
be visible. The caller must `resume` or `replay` and compare the stored
incarnation/revision before retrying. A blind retry could reset a successfully
reset session a second time and is therefore explicitly unsafe.

## Live handles and concurrency

The native lifecycle coordinates its operations with the engine's local
session registry. Reset fails as `LiveSession` before record replacement when
that host retains an incompatible incarnation, an active turn, or divergent
state for the proposed replacement. The preceding current-record load may create
the store's permanent fixed lock sidecar, and the bounded incarnation source
may already have been consulted; neither effect changes the durable record. The
lifecycle does not cancel an active turn, revoke clones, mutate a live handle in
place, or force it to adopt the new lifetime. The registry reservation also
prevents a new local handle for the ID from being published across the reset
check-and-commit window. Defensively, a nonconforming custom source that
returns the incarnation of an already registered inactive empty candidate can
reuse that matching local state; deliberate reuse violates the source trait's
globally unique ID contract and is not a supported custom-host coordination
mechanism. Even that misconfiguration cannot reuse the currently durable
incarnation through reset.

This is a process-local safety rule, not distributed revocation. Another
process may retain the old incarnation while reset succeeds under the shared
store lock. Its later save is fenced by the store's incarnation and revision
checks and fails rather than merging into the new lifetime. Work already sent
to a provider, tool, network, or other external system by another process is
not recalled by reset. The file lock coordinates only cooperating processes
using the same retained store protocol and inherits all trust boundaries in
[`session-store.md`](session-store.md).

Concurrent resume and replay calls are read-only at the record level. Local
resumes of the same incarnation converge on the canonical shared engine state;
replays remain independent owned snapshots. Create and reset use store CAS
rather than check-then-overwrite. No lifecycle operation performs session
enumeration or takes a lock spanning multiple IDs.

## Polling, cancellation, and resource bounds

Constructing a lifecycle future performs no source call, entropy read, store
call, record or lock creation, engine registration, provider work, prompt, or
background work. Dropping it before first poll is effect-free. The lifecycle
implementation and default source detach no task, thread, timer, retry loop, or
runtime work while polling. A trusted custom incarnation source is called
synchronously, but the lifecycle cannot constrain work that its implementation
performs or detaches internally; custom hosts must enforce the same bounded,
non-detaching source contract when they require those properties.

The shared `FileSessionStore` performs bounded synchronous I/O and advisory
locking on the polling thread. Once one of those calls is executing, dropping
the outer future cannot preempt it or roll back a completed rename. The host
does not add a Tokio runtime or move the work to a blocking pool. A caller that
must preserve executor responsiveness must arrange a suitable polling context.

With the default OS source, per-call retained application data is bounded by:

- one validated `SessionId`, at most 128 bytes;
- one bounded incarnation and one fixed-size random draw for create, or at most
  `MAX_SESSION_INCARNATION_ATTEMPTS` (`8`) such draws for reset;
- one current-schema record within `MAX_FILE_SESSION_BYTES` (`8_651_165`) plus
  the store's single overflow-witness byte while reading;
- the store's 64-level and 65,536-node aggregate JSON ceilings; and
- the composed engine's configured transcript, metadata, message-count, and
  structural limits before a record becomes a live session. Replay remains
  bounded by the fixed store limits above and does not register engine state.

Create and reset serialize only an empty record. Resume and replay do not read
past the file-store cap. Each attempt touches only the fixed record, lock, and
temporary names derived from one ID and performs no directory enumeration.
Successful byte-transfer and default-source work are bounded; advisory-lock
waits, filesystem latency, synchronization latency, and the store's `EINTR`
retries have no wall-clock bound. A custom source owns any additional resource
or wall-clock bounds required by its trusted host.

## Errors and redaction

The lifecycle error kind is non-exhaustive. Its stable behavioral categories
are:

| Kind | Meaning |
| --- | --- |
| `AlreadyExists` | Atomic create found an existing durable record. |
| `NotFound` | Resume, replay, or reset found no record. |
| `LiveSession` | A locally live incompatible lifetime prevents the requested operation. |
| `IncarnationSource` | The configured source failed, or reset exhausted its eight attempts without a value distinct from the durable incarnation. OS entropy failure is the default production case. |
| `Conflict` | A stored revision/incarnation CAS changed before the requested update. |
| `Corrupt` | The current file, schema, identity, counters, type, or bounds are invalid. |
| `Unavailable` | Store I/O, locking, rename, or synchronization failed, possibly after publication. |
| `Engine` | Core validation/registration failed, or the store reported a non-I/O invariant such as revision exhaustion or serialization failure. |

The production public method signatures and fixed display strings implement
this contract. The mapping is normative: atomic create conflict becomes
`AlreadyExists`; missing
load becomes `NotFound`; store CAS mismatch remains `Conflict`; store corrupt
remains `Corrupt`; store I/O and save-ambiguous remain `Unavailable`; store
`Other` and core protocol/validation failures become `Engine`; engine
incarnation conflict becomes `LiveSession`; and entropy failure becomes
`IncarnationSource`.

The construction-time `MismatchedSessionStore` failure is separate from these
operation errors. It cannot be deferred until the first create, resume, replay,
or reset and retains no nested source or authority detail.

No lifecycle error or debug representation retains or reflects a session ID,
incarnation, revision, message, metadata, record bytes, digest, root or child
path, random bytes, configuration, provider data, parser diagnostic,
operating-system error text, or raw error number. Retry guidance is category
and operation specific. In particular, `Unavailable` does not imply that
retrying create or reset is safe because publication may already have happened.

## Current schema and deferred scope

All operations accept only the currently supported file-session schema v1 and
current core `SessionRecord` invariants. They do not import, migrate, rewrite,
quarantine, or reset over legacy, future, corrupt, encrypted, or authenticated
formats. Migration, explicit legacy import, encryption, record authentication,
key management, secure erasure, stronger lifecycle concurrency hardening, and
non-Unix support remain assigned to Milestone 04.

This delivered slice does not itself implement `list_sessions`, deletion,
automatic cleanup, session-ID generation, CLI
`session`/`sessions`/`resume`/`replay` commands, a UI replay, or CLI byte
changes. The composed sixteenth candidate adds only bounded IDs-only
library-level listing under the separate
[`native session-listing contract`](native-session-listing.md). After that
composition the combined M03 root plus create/list/resume/replay/reset
functional checklist item is complete; production/test composition, formal
review, and delivery evidence for listing remain pending.

The slice makes no compatibility, upstream-equivalence, or product-performance
claim and changes no benchmark workload or workflow. Zig remains solely the
pinned upstream fx benchmark build input; `machine-god` remains a Rust product.
