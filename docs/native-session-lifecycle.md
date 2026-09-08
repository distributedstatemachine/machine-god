# Native session lifecycle

This contract defines the Linux/macOS native reference host's durable library
boundary for creating, listing, resuming, replaying, and resetting sessions in
the current file-session schema. Callers may supply a validated core
`SessionId` to `create`, or ask the lifecycle to generate a new identity with
`create_generated`. The host allocates the `SessionIncarnationId` for every new
logical lifetime. `create_with_metadata` and `create_generated_with_metadata`
add explicitly supplied typed native metadata to that same initial publication.

The owning component is `NativeSessionLifecycle`. Its public API exposes typed
construction and operation errors plus inert `create`, `create_generated`,
`create_with_metadata`, `create_generated_with_metadata`,
`list_sessions`, `resume`, `replay`, and `reset` futures.
`NativeReferenceHost` retains it and
exposes lifecycle and session-store observation over the same shared
`FileSessionStore` instance. The engine, lifecycle component, and store
therefore use one retained state-root identity rather than reopening a path or
constructing independent stores.

The separate [`native session-listing contract`](native-session-listing.md)
defines `list_sessions`. It enumerates that same retained store root but does
not change the create, resume, replay, or reset semantics below.

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
operations never invoke those components. Its default session-ID and
incarnation sources use only OS randomness. A custom `SessionIdSource` or
`SessionIncarnationSource` is explicitly trusted host code: its implementor is
responsible for its own authority, effects, latency, allocation, internal work,
and globally unique ID contract. Neither source may be model- or
configuration-controlled merely because the lifecycle exposes an injection
boundary.

Actual lifecycle handles retain the engine's optional host resource. Operation
futures instead retain a non-owning engine requester plus their explicit store
and identity sources; merely keeping an unpolled or pending operation does not
keep that host resource alive. Create/reset candidate reservations hold only
canonical session state, not a host-lifetime lease. A returned `Session` is a
real handle and retains the resource, but a late load after the last host handle
disappears fails rather than resurrecting it. This does not undo already
committed persistence or prevent an admitted record-only observation.

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

Default composition through `NativeReferenceHost` uses a bounded operating-
system random source to allocate a session ID for `create_generated` and an
incarnation for creation and reset. It does not fall back to a timestamp,
process or thread ID, counter, model output, session ID hash, or general-purpose
deterministic generator. `with_identity_sources` may instead receive trusted
custom-host `SessionIdSource` and `SessionIncarnationSource` implementations,
whose additional authority and obligations are assigned above. Reference-host
composition always selects the OS sources and does not expose either override
as ambient, model-controlled, or configuration input.

Every default generated session identity consumes 32 bytes (256 bits) from one
OS-random draw and encodes them as `ses-` followed by exactly 64 lowercase
hexadecimal digits. This 68-byte shape is current public behavior. The result
is validated as a bounded core `SessionId` before any store operation.
`MAX_SESSION_ID_ATTEMPTS` is `8`: an existing durable identity or incompatible
live identity is treated as a collision, and generation considers at most
eight source values. Exhaustion fails closed without replacing or resuming an
existing session.

Each default incarnation carries 256 bits from a fixed-size OS random draw and
is encoded as a valid bounded `SessionIncarnationId`. Its exact textual encoding
is not a public wire or file-format promise.
`MAX_SESSION_INCARNATION_ATTEMPTS` is `8`: reset considers at most eight source
values and never accepts one equal to the currently stored incarnation. Failure
to obtain an acceptable value within that bound fails closed before publication
and never reuses the prior incarnation.

## Operation summary

Every operation returns a future. `create`, `resume`, `replay`, and `reset`
take one caller-supplied validated `SessionId`; `create_generated` obtains its
identity from the configured `SessionIdSource`. Missing `resume`, `replay`, and
`reset` are typed `NotFound` failures, not `Ok(None)`.

```rust,ignore
NativeSessionLifecycle::create_generated(
    &self,
) -> BoxFuture<'static, Result<Session, NativeSessionLifecycleError>>
```

| Operation | Durable effect on success | Successful result |
| --- | --- | --- |
| `create` | Atomically creates one empty current-schema record at revision `1` with a new host-generated incarnation. | A live core `Session` for that exact durable record. |
| `create_generated` | Generates a bounded random identity and applies `create`, retrying only identity collisions. | A live core `Session` for the newly generated durable record. |
| `create_with_metadata` | Atomically creates revision `1`, empty history and allocator `1` together with the validated reserved native metadata entry. | A canonical live `Session` containing that exact initial metadata. |
| `create_generated_with_metadata` | Generates an identity and applies typed-metadata creation, retrying only proven pre-publication identity collisions. | A canonical live `Session` for the newly generated durable record. |
| `resume` | No record write; loads and validates the current durable record. A present load may create the store's permanent lock sidecar. | The engine-canonical live `Session` for the stored incarnation. |
| `replay` | No record write; loads and validates one current durable snapshot. A present load may create the lock sidecar. | An owned `SessionRecord` snapshot, not a UI transcript or event stream. |
| `reset` | Atomically replaces the current record with an empty record under the same ID, a new incarnation, revision `old + 1`, and turn allocator `1`. | A live core `Session` for the newly persisted incarnation. |

No successful operation invokes the provider, transport, permission handler,
prompter, tool, or event sink. Returning a live session does not begin a turn.
Those effects remain behind a later explicit call to the core session.

## Durable create

### Native presentation metadata

`NativeSessionMetadata` is the Linux/macOS host's bounded codec for the reserved
`machine_god.native_session` metadata entry. Its schema version `2` records
optional title, current and original workspace associations, creation/update
milliseconds, language tag and explicit `cli`/`recovered`/`imported` provenance.
The provenance enum and original workspace path are separate facts. It does not change
the enclosing file-store schema or ordinary lifecycle creation below. These
helpers stage values only: serialization is not a successful persistence result.

The codec reads no clock, environment, current directory or filesystem. A host
creating metadata supplies a verified canonical workspace and observed time.
Workspace paths are normalized absolute Unix bytes, capped at 4,096 bytes and
serialized as lowercase hex without lossy UTF-8 conversion. Newly constructed
metadata records the supplied workspace as both current and original; subsequent
rebinding preserves that original. Strict schema-v1 reads remain supported, with
unknown original workspace even when a current association is known. Serialization
emits schema v2 with `origin_workspace_hex`; no read or title mutation guesses
the missing origin from a legacy current workspace. This association
does not grant filesystem authority; the native host must independently retain
and verify its actual workspace capability. Lexical validation cannot prove a
path is canonical or still names an authorized root.

Absent historical metadata stays unknown, including after a title update. No
mtime, current directory, identifier, or unrelated metadata value supplies
missing history. Creation and update timestamps are signed 64-bit milliseconds;
an update cannot regress either known timestamp. Titles trim only edge
SP/TAB/CR/LF and must then contain 1–240 UTF-8 bytes without C0 or DEL bytes.
Language values follow the pinned `ConversationLanguage.fromSlice` byte contract:
trim edge SP/TAB/CR/LF, then retain 1–24 UTF-8 bytes without inventing a BCP-47
grammar. They remain untrusted presentation data, including possible interior
controls; a terminal renderer must escape them. Validation failure leaves the
staged metadata unchanged. The reference is pinned
`src/core/shared/types.zig:1381` at the revision in the implementation plan.

Schema v1 accepts only its seven declared fields; v2 adds only the eighth
`origin_workspace_hex` field. Absent or null
optional fields mean unknown. Unknown versions, extra fields, wrong types,
untrimmed stored titles and invalid bounds fail rather than silently rewriting
future state. Decoding borrows the selected entry, inspects only its shallow
known scalar fields and allocates only bounded accepted strings/path bytes.
Unrelated metadata is neither traversed nor modified. Debug/error output is
fixed or structural and does not expose title, path or record contents.

`rename_native_session` connects title staging to the core session's exclusive
metadata transaction. Its borrowed future does nothing before polling. On first
poll it snapshots canonical state, validates the title and injected update time,
preserves other metadata keys and submits a revision-checked metadata-only save.
The returned revision means persistence succeeded; neither a display change nor
a staged value is reported as a successful rename. A live turn returns `Busy`;
a competing revision returns `Conflict` without blindly retrying the stale patch.
Unknown or invalid native metadata is not overwritten to make a rename succeed.
No provider, tool, policy prompt, clock or environment is invoked. A persistence
error can follow publication, so callers must reconcile before reporting whether
the title was saved or retrying. Error output is redacted. Full interactive
process-only title fallback and command dispatch belong to the conversation host,
not to this persistence operation.

### Durable workspace rebinding

`rebind_native_session_workspace(session, expected_revision, workspace, now_ms)`
is a borrowed inert future for an explicit association change. The host supplies
the verified canonical root and observed time; the adapter performs only lexical
path validation and the core session's exclusive metadata transaction, without
opening a root or consulting a provider, tool, environment, or clock. It does not
grant root authority or change the process directory.

First poll requires the exact canonical revision. A real change saves schema-v2
current workspace and update time together with one typed previous/next event in
`machine_god.workspace_rebindings`. Original workspace, provenance, creation time,
title, language, transcript, turn allocator, checkpoint, preferences, and unrelated
metadata remain unchanged. An unknown previous association is recorded as null;
unknown original association stays unknown. The event records injected `at_ms`
and `previous_revision`, not an invented historical timestamp or revision.

History schema v1 has exactly `schema_version` and `entries`. Each event has
exactly `previous_workspace_hex`, `next_workspace_hex`, `at_ms`, and
`previous_revision`. Paths use the same bounded lossless hex codec. Decoding
rejects wrong types, unknown/missing fields, invalid paths, no-change events,
disconnected previous/next paths, regressing times and non-increasing revisions.
Rebinding additionally checks the first event against known creation time and
the last event against current workspace, update time, and revision. At most 64
events are retained; the next change returns
`HistoryLimit` without discarding an event. Existing core/store metadata byte,
node and structural limits may reject a smaller history first.

An unchanged workspace returns the unchanged revision, without a write, time
update, schema upgrade, or event. It still uses `Session::check_metadata_revision`
to acquire the exclusive lease, reconcile any uncertain publication, and check
the expected canonical revision. This is a checked observation, not a lock
against future cross-process writes. Busy and stale no-ops do not report success.

Invalid history returns redacted `InvalidHistory`; other input, busy, conflict,
host and persistence outcomes use `NativeSessionMetadataMutationError`. A pending
change excludes turns and other edits. Dropping it releases the lease and drops
owned store work, but cannot undo an already published save. Failed/dropped saves
force core reconciliation before subsequent metadata work; callers must inspect
the confirmed revision/history rather than blindly retrying an uncertain change.
The pinned previous/current check and update behavior comes from
`src/core/session/session_store.zig:3153` and the `workspace_rebound` reducer in
`src/core/session/session_event.zig:924` at the implementation-plan revision.

### Empty-record publication

`create` starts from an exact empty `SessionRecord`:

- the caller's `SessionId` is unchanged;
- the configured source supplies a fresh incarnation; default reference-host
  composition uses OS randomness;
- the unsaved record revision is the core zero sentinel;
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

`create_generated` obtains one validated identity from its configured
`SessionIdSource` and applies the same durable-create protocol. It retries only
`AlreadyExists` and `LiveSession`, because those categories prove an identity
collision before this call publishes a new record. Every other lifecycle error
is returned immediately. A source failure returns `SessionIdSource` and does
not fall back to a weaker identity scheme. Eight collisions return
`SessionIdExhausted`; the lifecycle neither performs a ninth source call nor
reflects any collided identity. Collision attempts can perform the bounded
record lookup, lock-sidecar, incarnation-source, and local-registry work of
`create`, but they never overwrite, resume, merge, truncate, or delete the
colliding session.

As in the underlying store, failure before rename leaves no published record,
although a permanent lock or safe temporary artifact may remain. A directory-
sync failure after rename is an `Unavailable` error with an ambiguous outcome:
the new record may already be visible. The caller must `resume` or `replay` to
reconcile before deciding what to do. Blindly retrying `create` is not a safe
generic response to that error.

### Atomic initial native metadata

`create_with_metadata(id: SessionId, metadata: NativeSessionMetadata)` and
`create_generated_with_metadata(metadata: NativeSessionMetadata)` return the same
`BoxFuture<'static, Result<Session, NativeSessionLifecycleError>>` as ordinary
creation. The input is the private-field bounded native codec, not an arbitrary
recursive JSON value or caller-supplied metadata map. Default codec fields remain
unknown; creation does not supply a clock, current directory, title, or provenance
that the caller did not provide.

These methods publish exactly one initial record containing only the reserved
`machine_god.native_session` metadata entry, with revision `1`, empty messages,
and allocator `1`. They never first write an empty record and then perform a
metadata update. The store validates the typed codec and performs the same
new-record CAS, file sync, atomic rename, and directory sync as ordinary create.
Existing `create`, `create_generated`, and `reset` still publish empty metadata;
the strict empty-record store predicate is not broadened.

Before metadata creation consults the store or incarnation source, its shallow
scalar codec output is checked against configured engine metadata-byte and JSON-
node limits, plus the two-byte empty-transcript requirement. Rejection is fixed
`Engine` and publishes no record or lock. Generated creation may already have
consumed one session-ID source value; validation failure is not a collision and
does not retry. The native map has one JSON container level and does not add
arbitrary JSON traversal authority.

The existing weak canonical reservation remains exactly empty and grants no
host-lifetime vote. After publication, canonical loading reconciles the initial
revision and metadata before returning the real session handle. An immediate
replay and a fresh engine resume observe that same revision-one metadata. A late
host closure cannot resurrect the host. If the post-publication canonical load
finds an incompatible live lifetime, metadata creation returns `Conflict`, not
a pre-publication `LiveSession` collision; generated creation must not allocate
another ID after that successful publication. Other unknown or ambiguous
publication errors likewise return without retry. The eight-attempt collision
bound and source-failure behavior remain unchanged.

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
returns the incarnation of an already registered inactive empty record can
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
background work. This includes `create_generated`: its session-ID source is not
called until the returned future is polled. Dropping any lifecycle future
before first poll is effect-free. The lifecycle implementation and default
sources detach no task, thread, timer, retry loop, or runtime work while
polling. A trusted custom identity source is called synchronously, but the
lifecycle cannot constrain work that its implementation performs or detaches
internally; custom hosts must enforce the same bounded, non-detaching source
contract when they require those properties.

The shared `FileSessionStore` performs bounded synchronous I/O and advisory
locking on the polling thread. Once one of those calls is executing, dropping
the outer future cannot preempt it or roll back a completed rename. The host
does not add a Tokio runtime or move the work to a blocking pool. A caller that
must preserve executor responsiveness must arrange a suitable polling context.

With the default OS source, per-call retained application data is bounded by:

- one validated `SessionId`, at most 128 bytes, per create attempt;
- for `create_generated`, at most `MAX_SESSION_ID_ATTEMPTS` (`8`) fixed-size
  32-byte random draws and generated IDs;
- one bounded incarnation and one fixed-size random draw for create, or at most
  `MAX_SESSION_INCARNATION_ATTEMPTS` (`8`) such draws for reset;
- one current-schema record within `MAX_FILE_SESSION_BYTES` (`8_651_165`) plus
  the store's single overflow-witness byte while reading;
- the store's 64-level and 65,536-node aggregate JSON ceilings; and
- the composed engine's configured transcript, metadata, message-count, and
  structural limits before a record becomes a live session. Replay remains
  bounded by the fixed store limits above and does not register engine state.

Ordinary create and reset serialize only an empty record. Typed-metadata create
adds only the bounded native codec entry to the empty history. Resume and replay do not read
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
| `SessionIdSource` | The configured generated-ID source failed before supplying a validated identity. OS entropy failure is the default case. |
| `SessionIdExhausted` | All eight generated identities collided with an existing durable or live session. |
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
incarnation conflict becomes `LiveSession`; incarnation entropy failure becomes
`IncarnationSource`; generated-ID source failure becomes `SessionIdSource`; and
eight generated-ID collisions become `SessionIdExhausted`.

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

The lifecycle does not implement deletion, automatic cleanup, a UI replay, or
CLI presentation. IDs-only enumeration is defined by the separate
[`native session-listing contract`](native-session-listing.md). Top-level
session commands decide which lifecycle operation to expose and remain
responsible for their own parsing, output, and exit contracts.
