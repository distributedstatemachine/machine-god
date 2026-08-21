# Milestone 02 bounded tool-loop candidate

Status: review-cycle-eleven finding remediated; awaiting fresh rereview of the
eleventh fix commit.

Reviewed base commit: `48b31d6e6aa32f74d4a5c4e12a21919e917cea00`.

Cycle-one fix reviewed for cycle two:
`53e9ef853444d34fa42e3f2c1866540d554df86f`.

Cycle-two fix reviewed for cycle three:
`9d6bc784dddc022835139ce033aeb8c6d999a3d4`.

Cycle-three fix reviewed for cycle four:
`f952febb87de8224df1af09e69c1e80c7ea563f9`.

Cycle-four fix reviewed for cycle five:
`83b465cd02a143b116e9e3ac21663363f35f4fba`.

Cycle-five fix reviewed for cycle six:
`f0d6369093c12fdf51e7ab17724ec3bb67f248bb`.

Cycle-six fix reviewed for cycle seven:
`043d5fbc9e72ca5ef7e190a08eb480c396ff9dac`.

Cycle-seven fix reviewed for cycle eight:
`240fb939d274b4d77a41187eb5c0724aecd78faa`.

Cycle-eight fix reviewed for cycle nine:
`e9b20c61de0091f7f9ffc469c60f061f6bd9b23c`.

Cycle-nine fix reviewed for cycle ten:
`849fcd82ef2b69ed10cff61f7429ebab3f2075aa`.

Cycle-ten fix reviewed for cycle eleven:
`149b07b0812ea9a731fb198adcfc25195887184a`.

Cycle-eleven fix commit: populated after this remediation commit. Reviewers must
review that exact immutable commit and replace this status only after
correctness/API, security/abuse, and performance/concurrency rereviews all report
no findings.

## Candidate scope

- Nonzero public engine limits and checked provider-controlled prompt,
  transcript, session-metadata, inference-option, tool-catalog, denial-reason,
  JSON-depth, JSON-node, event, stop-detail, byte, call, and round counters.
- Durable user reservation, atomic assistant-plus-placeholder tool-call commits,
  exact in-place result replacement, prefix-checked CAS retries, and checked
  cross-round usage.
- Strict model stop/tool-call grammar, turn-wide call-ID uniqueness, registered
  tool lookup, serial permission and tool phases, and deterministic events.
- Deterministic testkit contracts for two-round behavior, denial/error recovery,
  malformed rounds, every configured budget, cancellation phases, durable
  reload, commit ordering, allocator-only conflict retry, divergence, and bad
  store revisions, interruption-safe partial rounds, provider/local cancellation
  provenance, error redaction, and final-save liveness.

## Review cycle 01 findings and remediations

1. A cancellation or infrastructure failure after a tool call was durably
   accepted could leave an assistant call without a matching result and make
   resume structurally invalid. Core now atomically commits the assistant call
   with one conservative unknown-result placeholder per call before permission
   or execution, then replaces each placeholder using an exact snapshot CAS.
   Tests cover cancellation immediately before tool readiness, partial
   multi-call completion, sink/policy/store failures, and resume without replay.
2. A provider-originated `StopReason::Cancelled` could be mistaken for local
   cancellation and bypass its pending observer. Local synthesis is now tracked
   independently and is the only path allowed to bypass observer delivery.
3. A provider could emit an unbounded sequence of empty or otherwise
   zero-progress events. Every provider stream item, including `Stop`, now
   consumes the turn-wide model-event budget; exact boundary and boundary-plus-
   one tests verify failure and lease cleanup.
4. Permission-denial reasons could be copied into durable, model-visible tool
   output. The transcript now receives a fixed generic denial; detailed policy
   reasons remain only in the host-facing `PermissionResolved` event. Secret-
   bearing regression fixtures verify absence from model requests and storage.
5. `StopReason::Other` detail could be cloned and delivered without a bound. It
   is now rejected before that work when it exceeds the configured byte limit,
   with exact-boundary coverage.
6. Prompt/transcript growth, globally ambiguous permission IDs, and raw
   provider, permission, store, or tool diagnostics were not bounded as a
   complete trust-boundary policy. Prompt bytes are checked before persistence;
   transcript count and serialized size are checked before load publication,
   each provider request, and every commit; permission IDs include turn identity
   and ordinal; untrusted diagnostics are bounded and generic at host/model
   boundaries.
7. A final provider stop was established before its required assistant save, so
   cancellation could not wake a permanently pending store. Final stops now
   remain preterminal and cancellable through the successful save. Only then is
   terminal precedence established for model-stop, sink, and completion
   delivery. A manual-poll regression verifies cancellation and lease release.

All focused post-remediation tests pass at this document's candidate state; the
required whole-workspace gates and adversarial rereview must target the final fix
commit rather than this mutable working tree.

## Review cycle 02 findings and remediations

1. Permission request IDs used only turn identity and ordinal. Two sessions both
   issuing `turn-1` could therefore collide and an ID-keyed policy cache could
   reuse a positive decision across sessions. IDs now use RustCrypto SHA-256
   over a domain tag plus length-delimited session ID, turn ID, and ordinal,
   encoded as a fixed lowercase hexadecimal portable ID. A known-vector test,
   two-session regression, and deliberately ID-caching policy verify stable
   identity and no cross-session allow reuse.
2. Recursive session metadata, inference options, and the cached aggregate tool
   catalog were not bounded at their trust boundaries. Public nonzero limits now
   cover all three serialized values. Session metadata is checked before load
   publication, requests, and mutations; options are checked before prompt
   reservation and provider invocation; catalog descriptions and Schemas are
   checked during engine construction before caching. Exact-boundary and
   boundary-plus-one tests prove rejection before persistence or provider work.
3. Although denial reasons were kept out of the model transcript, an unbounded
   reason was cloned into the host-facing `PermissionResolved` event. Core now
   consumes and truncates it on a UTF-8 boundary before cloning or staging it,
   using a public default 4 KiB bound. The durable/model denial remains fixed and
   generic; a Unicode secret-bearing regression verifies both boundaries.
4. Provider, store, and permission messages were generic, but their hostile
   component-defined codes were still forwarded. These failures now expose only
   `provider_failed`, `store_failed`, or `permission_failed`, while retaining
   trusted store/provider kind and retryability behavior. Newline and secret
   codes are tested for provider, permission, turn-store, prompt-reservation
   store, and load-store paths.
5. Reservation and transcript mutation serialized and deep-cloned records while
   holding the shared session mutex, potentially repeating work up to the 8 MiB
   transcript limit across conflicts. Canonical records now use immutable `Arc`
   snapshots. The mutex protects only pointer/persistence capture and exact
   identity rechecks; validation, serialization, equality, and cloning occur
   outside it. Durable revision CAS still closes a race after the local recheck,
   and reconciliation retains monotonic and equal-revision-divergence rules.

The new SHA-256 dependency is registry-only RustCrypto `sha2`; dependency-policy
and advisory gates are required for the final immutable cycle-two commit.

## Review cycle 03 finding and remediation

1. Serialized-byte limits did not prevent a hostile `serde_json::Value` from
   creating enough container nesting to overflow recursive serialization or
   cloning before its byte budget was enforced. A public nonzero JSON-depth
   limit now defaults to 64 containers. A scalar root is depth zero, a root
   array or object is depth one, and every nested container increments it. The
   validator is iterative and holds one child-iterator frame per active
   container, so auxiliary memory is O(depth), not O(total nodes). It runs before
   catalog caching, prompt reservation, load publication, record
   serialization/cloning, provider invocation, permission/tool execution, and
   tool-result size counting/replacement wherever core controls the value.
   Exact and boundary-plus-one regressions cover tool Schemas, inference
   metadata with untouched provider/store, loaded record metadata and JSON
   messages, provider arguments before authorization/execution, and tool output
   after an effect while preserving the precommitted unknown-result placeholder.

## Review cycle 04 findings and remediations

1. Iterative depth validation prevented recursive serialization, but rejecting
   or abandoning a tens-of-thousands-deep owned `serde_json::Value` still ran
   its recursive destructor and could abort the process. Every core-owned
   rejection ingress now arms an ownership guard before validation or another
   fallible step. The guard replaces embedded values with `Null` and consumes
   the original arrays/maps iteratively, reclaiming all nodes in O(actual nodes)
   time and O(actual depth) auxiliary memory without a leak. Coverage includes
   builder abandonment, duplicate replacement and build errors; unpolled and
   rejected prompts; direct and conflict record loads; mutation candidates;
   yielded provider tool calls; and post-effect tool output. Eleven subprocess
   cases construct 50,000 nested arrays iteratively and prove controlled errors
   or safe abandonment without stack aborts, including cancellation observed in
   the same poll that yields a provider event or tool output. Internally queued
   stream items that have not been yielded remain provider-owned and require a
   stack-safe provider stream destructor.
2. Depth alone did not cap shallow, extremely wide JSON trees. A new public
   nonzero `max_json_nodes` limit defaults to 65,536. Every root, scalar, and
   container counts once. A single counter aggregates all tool Schemas, all
   inference metadata, or every metadata/message JSON value in one record;
   provider arguments and tool output are each bounded individually. The
   iterative walker stops after visiting limit plus one and retains only active
   ancestor iterators, never a queue of siblings. Exact aggregate-boundary and
   plus-one tests cover catalogs, inference metadata, stored records, provider
   arguments before authorization/execution, and post-effect tool output with
   its durable unknown-result placeholder.

## Review cycle 05 finding and remediation

1. A yielded provider event crossed into core, but the model-event counter was
   incremented and checked before its JSON ownership guard was armed. A deep
   tool-call event at event limit plus one could therefore take the early limit
   return and run recursive `Value::drop`. Core now wraps every successful
   provider event in the first expression after `next_model_item` completes,
   before presence, overflow, limit, stop, or variant handling. A twelfth
   50,000-level subprocess case sends one benign event followed by the deep
   tool call under an event limit of one; it terminates with
   `model_event_limit`, performs no permission or tool work, and does not abort.
   The corresponding audit found no other raw JSON-bearing post-await value
   with an early return before its guard: ready tool output and cancellable
   conflict loads are guarded inside their poll wrappers, while direct loads,
   prompt values, builder Schemas, and mutation candidates arm guards before
   validation or fallible branching.

## Review cycle 06 finding and remediation

1. `max_json_depth` was nonzero but otherwise unrestricted. A host could set it
   to 50,000, the iterative validator would accept a matching tree, and later
   recursive `serde_json` serialization, cloning, retention, extension code, or
   ordinary destruction could overflow the process stack. The public
   `MAX_SAFE_JSON_DEPTH` structural ceiling is now fixed at the audited default
   of 64. `EngineBuilder::build` returns the dedicated stable
   `JsonDepthLimitExceedsSafeMaximum` error before catalog validation,
   serialization, caching, or runtime component calls whenever the configured
   value is higher; it never clamps. The tool-map ownership guard is armed
   first, so this early error still drains hostile builder Schemas iteratively.
   Boundary coverage accepts 64 and rejects 65 with the exact public error and
   message. A thirteenth subprocess case configures 50,000 with a matching deep
   Schema, observes the controlled build error, proves no provider/store/policy
   call, and exits without aborting.
2. The configuration audit found no other knob that lifts a structural stack
   safety invariant. Once depth is at most 64, node, byte, message, event, call,
   round, text, reasoning, result, and denial-reason maxima control
   operator-selected time or memory exposure; their counting/traversal and
   arithmetic are iterative or checked. Large values may intentionally permit
   correspondingly large resource consumption, but do not authorize deeper
   recursion. Producer-private construction and queued-value destruction remain
   the documented external responsibility.

## Review cycle 07 finding and remediation

1. Permission request IDs bound the session ID, turn ID, and ordinal, but a host
   could delete or reset durable state under the same session ID while retaining
   an ID-caching permission handler. Turn numbering and ordinal would restart,
   recreating an old request ID and replaying an allow into a new tool call.
   Every session record now requires a validated, caller-supplied
   `SessionIncarnationId`; core supplies no clock-, randomness-, or
   session-ID-derived fallback. Stores preserve it and reject mutation, while
   the live registry refuses to merge the same session ID with a different
   incarnation. `ModelRequest` and `PermissionRequest` expose it for audit, and
   permission IDs now use the v2 SHA-256 preimage over length-delimited session
   ID, incarnation ID, turn ID, and ordinal. Known vectors cover differences
   across sessions, incarnations, and turns. A shared ID-caching policy across
   two fresh stores proves that the second logical lifetime receives a new
   decision and cannot replay the first allow; load, serialization, collision,
   and missing-field regressions enforce the durable contract.

## Review cycle 08 findings and remediations

1. `ToolContext` omitted the durable session incarnation. A reset lifetime could
   therefore repeat the same session, turn, and call IDs, causing a tool's
   idempotency or audit key to collide with an earlier invocation. Tool contexts
   now carry the captured record incarnation alongside those IDs.
2. `EngineEvent` likewise omitted the incarnation, so event-sink deduplication
   or audit could merge two otherwise identical event sequences across reset
   lifetimes. Every staged event now carries the incarnation, including
   workflow terminal events, locally synthesized cancellation, and cancellation
   that replaces a pending observer delivery. A two-lifetime regression uses
   fresh engines and stores with identical session, turn, call, and sequence IDs
   and proves tool contexts, returned events, sink-recorded events, and their
   serialized forms remain distinct. Focused cancellation regressions cover
   both local terminal construction paths.

## Review cycle 09 findings and remediations

1. `Turn::poll_delivery` returned component-supplied `EventSinkError` codes and
   messages unchanged. A hostile sink could expose secrets, newlines, or
   unbounded diagnostics through the public error. Core now drops those strings
   and returns only `event_sink_failed` / `event sink failed`. Regressions use
   large secret-bearing values and verify the stable public fields plus `Display`
   and `Debug`; another sink cancels and returns a ready hostile error in the
   same poll, proving cancellation precedence without diagnostic leakage.
2. The correctness review framed a gap between return from the final
   `commit_message` await and `emitter.establish_terminal`. That exact
   interleaving is rejected: a ready await resumes inside the same workflow poll
   and terminal establishment is synchronous, so the outer `Turn` cannot poll
   cancellation between those instructions. The adjacent boundary audit did
   confirm a real gap inside the generic cancellation-aware save poll: a store
   could durably persist, request cancellation, and return `Ready(Ok)` in one
   poll, causing the wrapper to discard the success as cancellation. The final
   assistant commit now uses a specialized precedence mode. Cancellation still
   wins before polling, while the save is pending, or with a ready error; a
   ready success is reconciled and followed immediately by terminal
   establishment. A store regression persists the final answer, cancels from
   that ready-success poll, and proves the provider `Stop` and `Completed`
   outcome remain intact. The existing pending-final-save cancellation
   regression remains green. Placeholder and tool-result commits retain their
   original cancellation behavior.

## Review cycle 10 findings and remediations

1. The cycle-nine final-save helper received an already-created store future.
   A `SessionStore::save` implementation can perform work eagerly, so it could
   durably persist and request cancellation synchronously before returning a
   ready successful future; the helper's initial cancellation check would then
   discard that success without polling. Final-save construction now lives
   inside the specialized helper. It checks cancellation immediately before
   invoking `save`, unconditionally polls the new future once, and lets a ready
   success from construction or that first poll win. Pending or ready-error
   results still yield to cancellation, and every later poll checks cancellation
   before touching the previously pending future. A new eager-store regression
   joins the existing poll-time-ready-success and pending-save cancellation
   tests; all three prove their respective precedence boundary. Placeholder and
   tool-result save paths remain unchanged.
2. `Engine`'s `Debug` implementation called the provider-controlled `name`
   method and formatted its returned string. Debug logging could therefore run
   hostile provider code, panic, block, or expose secrets. It now reports only
   fixed structural state: `has_provider: true` and the registered tool count.
   A provider whose name call is observable and panics with a sentinel proves
   formatting neither invokes the method nor leaks the sentinel, while an exact
   assertion retains a stable useful debug shape.

## Review cycle 11 finding and remediation

1. `SessionRegistry::session_state` held the entries mutex while validating an
   upgraded live state. On an incarnation conflict, another thread could drop
   the original handle after the upgrade, leaving the validating thread's local
   `Arc` as the last owner. Error unwinding would then drop `SessionState` and
   its `SessionRegistration` before the entries guard, causing cleanup to relock
   the same non-reentrant mutex and deadlock. The registry now explicitly drops
   the entries guard immediately after `Weak::upgrade`, retains the strong
   state, and performs identity validation outside the lock. A deterministic
   `cfg(test)` hook clones its optional barrier while holding only the hook mutex,
   releases that mutex, and waits only after the entries lock is gone. The
   regression pauses there, drops the original owner, releases the sole-owner
   conflict path, requires its result through a bounded channel receive, and
   then proves the registry remains usable through load, convergent create, and
   last-handle drop. Under the old ordering, the worker would deadlock before it
   could send the conflict result.

## Honest limitations for review

- `Stop` is trusted as the immediate end of a provider round. Core does not poll
  for EOF, so it cannot observe a provider that lazily produces another item
  after `Stop`; polling would hang on valid stop-then-pending providers.
- Core publishes tool JSON Schema but M02 does not validate arguments against
  it. Each tool remains responsible for exact input validation.
- Positive permission scopes are not cached. Policy is called for every tool
  invocation; a host can add identity-safe policy caching.
- The one-live-turn lease is scoped to one `Engine`. Cross-engine fencing and
  exactly-once recovery of non-idempotent native tool effects remain M04 work.
- If a process crashes after a tool side effect and before its precommitted
  unknown-result placeholder is replaced, M02 cannot prove whether the side
  effect completed. The placeholder prevents automatic replay, but M02 cannot
  recover the result or provide exactly-once external effects.
- Owned `Value` and `String` allocations exist before core can count or traverse
  them when a caller constructs prompt options, a store decodes records, a tool
  creates specifications or results, a provider creates event values, or a
  policy creates decisions and reasons. Core prevents its controlled recursive
  serialization/cloning and additional effects after the configured boundaries,
  but cannot retroactively prevent those producer allocations. Each producer
  needs complementary decode and construction limits.
- Core can iteratively reclaim a provider event only after `poll_next` yields
  ownership of it. If a turn stops early, any values retained in the provider
  stream's private queue are destroyed by that provider's `Drop` implementation,
  which must itself be stack-safe or bounded during decoding/construction.

## Required review evidence

Reviewers should verify terminal/cancellation races in every async phase,
observer ordering relative to commits, no execution before complete-round
validation, exact limit boundaries and checked arithmetic, transcript conflict
handling without duplication, sensitive error truncation, and absence of
runtime or ambient native authority in core.
