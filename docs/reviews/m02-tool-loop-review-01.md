# Milestone 02 bounded tool-loop candidate

Status: implementation candidate awaiting fresh adversarial review.

Exact commit: populated after the feature commit. Reviewers must review that
exact immutable commit and replace this status only after correctness/API,
security/abuse, and performance/concurrency reviews all report no findings.

## Candidate scope

- Nonzero public engine limits and checked provider-controlled byte/call/round
  counters.
- Durable user reservation, assistant/tool-result commits, prefix-checked CAS
  retries, and checked cross-round usage.
- Strict model stop/tool-call grammar, turn-wide call-ID uniqueness, registered
  tool lookup, serial permission and tool phases, and deterministic events.
- Deterministic testkit contracts for two-round behavior, denial/error recovery,
  malformed rounds, every configured budget, cancellation phases, durable
  reload, commit ordering, allocator-only conflict retry, divergence, and bad
  store revisions.

## Honest limitations for review

- `Stop` is trusted as the immediate end of a provider round. Core does not poll
  for EOF, so it cannot observe a provider that lazily produces another item
  after `Stop`; polling would hang on valid stop-then-pending providers.
- Core publishes tool JSON Schema but M02 does not validate arguments against
  it. Each tool remains responsible for exact input validation.
- Positive permission scopes are not cached. Policy is called for every tool
  invocation; a host can add identity-safe policy caching.
- The one-live-turn lease is scoped to one `Engine`. Cross-engine fencing and
  crash-safe replay of non-idempotent native tool effects remain M04 work.
- If a process crashes after a tool side effect and before its result or
  unknown-result marker commits, M02 cannot prove whether replay is safe.
- Provider-created `Value` and `String` allocations exist before core can count
  them. Core prevents unbounded accumulation and additional work after the
  configured limit, but cannot retroactively prevent the provider allocation.

## Required review evidence

Reviewers should verify terminal/cancellation races in every async phase,
observer ordering relative to commits, no execution before complete-round
validation, exact limit boundaries and checked arithmetic, transcript conflict
handling without duplication, sensitive error truncation, and absence of
runtime or ambient native authority in core.
