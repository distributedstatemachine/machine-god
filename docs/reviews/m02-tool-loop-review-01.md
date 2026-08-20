# Milestone 02 bounded tool-loop candidate

Status: review-cycle-four findings remediated; awaiting fresh rereview of the
fourth fix commit.

Reviewed base commit: `48b31d6e6aa32f74d4a5c4e12a21919e917cea00`.

Cycle-one fix reviewed for cycle two:
`53e9ef853444d34fa42e3f2c1866540d554df86f`.

Cycle-two fix reviewed for cycle three:
`9d6bc784dddc022835139ce033aeb8c6d999a3d4`.

Cycle-three fix reviewed for cycle four:
`f952febb87de8224df1af09e69c1e80c7ea563f9`.

Cycle-four fix commit: populated after this remediation commit. Reviewers must
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
