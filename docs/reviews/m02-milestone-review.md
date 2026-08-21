# Milestone 02 completion evidence

Milestone 02's provider-neutral streaming engine and deterministic testkit are
complete at the bounded tool-loop candidate
`c90499b9198d7fb4a426c54249a5061c6df9dbe1`. This document records the
integrated feature commits, retained checks, and honest boundary of that claim.

## Scope and outcomes

Milestone 02 delivered:

- provider-neutral, executor-independent model, storage, permission, event, and
  tool contracts at
  `49bbb168e33023dda16f7e2f5002474df8965fd4`;
- deterministic providers, stores, policies, tools, event sinks, scripts, and
  fixtures at `b3b74e64d649f8557d174180a55c6b624d6d0460`; and
- a bounded durable multi-round tool loop with explicit authority, optimistic
  persistence, cancellation and terminal precedence, interruption-safe
  placeholders, validated incarnation identity, resource limits, and
  adversarial regressions at
  `c90499b9198d7fb4a426c54249a5061c6df9dbe1`.

Feature-level findings and their remediations remain in the
[core-contracts](m02-core-contracts-review-01.md),
[testkit](m02-testkit-review-01.md), and
[bounded tool-loop](m02-tool-loop-review-01.md) reports.

## Verification evidence

The exact bounded tool-loop candidate passed the Rust 1.94.1 formatting gate,
Clippy across all workspace targets and features with warnings denied, all 137
top-level workspace tests plus all 13 deep-JSON subprocess cases, and both
workspace documentation tests. The locked release CLI build completed and the
fresh binary reported `machine-god 0.1.0 (engine API 1)`. Dependency policy and
advisory checks passed; the audit covered 33 dependencies with no known
vulnerabilities. The repo-wide Python benchmark and compatibility suite passed
76 tests, with 8 expected macOS skips for Linux-specific cases.

Each feature candidate also completed its exact remote CI and benchmark-evidence
workflows successfully:

| Feature candidate | Workflow | Run |
| --- | --- | --- |
| Core contracts `49bbb168e33023dda16f7e2f5002474df8965fd4` | CI | [32391677911](https://github.com/distributedstatemachine/machine-god/actions/runs/32391677911) |
| Core contracts `49bbb168e33023dda16f7e2f5002474df8965fd4` | Benchmark evidence | [32391677886](https://github.com/distributedstatemachine/machine-god/actions/runs/32391677886) |
| Testkit `b3b74e64d649f8557d174180a55c6b624d6d0460` | CI | [32397258414](https://github.com/distributedstatemachine/machine-god/actions/runs/32397258414) |
| Testkit `b3b74e64d649f8557d174180a55c6b624d6d0460` | Benchmark evidence | [32397258296](https://github.com/distributedstatemachine/machine-god/actions/runs/32397258296) |
| Bounded tool loop `c90499b9198d7fb4a426c54249a5061c6df9dbe1` | CI | [32464211010](https://github.com/distributedstatemachine/machine-god/actions/runs/32464211010) |
| Bounded tool loop `c90499b9198d7fb4a426c54249a5061c6df9dbe1` | Benchmark evidence | [32464210997](https://github.com/distributedstatemachine/machine-god/actions/runs/32464210997) |

The final cycle-twelve adversarial review of the exact bounded tool-loop
candidate was GREEN in all required categories: correctness/API,
security/abuse, and performance/resource/concurrency.

## Deferred work and limitations

- Milestone 03 remains `NOT STARTED`. Concrete model providers, native tools,
  permission implementations, durable session backends, configuration, and a
  useful CLI host are not part of this milestone.
- Milestone 04 remains `NOT STARTED`. Cross-engine or cross-process fencing,
  exactly-once recovery for non-idempotent effects, and broader lifecycle,
  concurrency, persistence, and security hardening remain deferred.
- M02 advertises tool JSON Schema but does not validate arguments against it;
  tools retain responsibility for exact input validation.
- The engine's current performance evidence verifies the harness and bounded
  behavior. It does not yet establish the final product speedup targets or make
  the project production-ready.

The bounded tool-loop report retains the complete list of trust-boundary and
producer-owned allocation limitations.

## Milestone evidence seal

The exact Milestone 02 documentation candidate
`af87f73c37ad52fd4d91ef82c4cdccd78fea6036` received fresh external review with
no actionable findings:

- correctness/API: GREEN;
- security/abuse: GREEN; and
- performance/resource/evidence: GREEN.

The same immutable candidate completed both remote workflows successfully:

| Workflow | Run |
| --- | --- |
| CI | [32465808810](https://github.com/distributedstatemachine/machine-god/actions/runs/32465808810) |
| Benchmark evidence | [32465808681](https://github.com/distributedstatemachine/machine-god/actions/runs/32465808681) |

This later evidence-only seal records those external results. It changes no
code, milestone scope, or previously reviewed claim, and does not claim that the
seal commit reviewed itself.
