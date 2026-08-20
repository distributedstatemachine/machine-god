# Milestone 02 bounded tool-loop candidate

Status: review-cycle-one findings remediated; awaiting fresh rereview of the fix
commit.

Reviewed base commit: `48b31d6e6aa32f74d4a5c4e12a21919e917cea00`.

Fix commit: populated after this remediation commit. Reviewers must review that
exact immutable commit and replace this status only after correctness/API,
security/abuse, and performance/concurrency rereviews all report no findings.

## Candidate scope

- Nonzero public engine limits and checked provider-controlled prompt,
  transcript, event, stop-detail, byte, call, and round counters.
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
- Provider-created `Value` and `String` allocations exist before core can count
  them. Core prevents unbounded accumulation and additional work after the
  configured limit, but cannot retroactively prevent the provider allocation.

## Required review evidence

Reviewers should verify terminal/cancellation races in every async phase,
observer ordering relative to commits, no execution before complete-round
validation, exact limit boundaries and checked arithmetic, transcript conflict
handling without duplication, sensitive error truncation, and absence of
runtime or ambient native authority in core.
