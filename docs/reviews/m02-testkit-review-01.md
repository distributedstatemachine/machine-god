# Milestone 02 deterministic testkit review 01

Status: complete. Integrated candidate:
`b3b74e64d649f8557d174180a55c6b624d6d0460`.

Scope: `machine-god-testkit` doubles, dependency wiring, documentation, and
tests. The review covered correctness/API, security/abuse, and
performance/concurrency against the committed candidate.

Retained invariants and regression coverage:

- scripts and retained observations have finite construction-time or explicit
  record bounds;
- exhaustion and capacity failures are visible and component-appropriate;
- no test double uses a clock, sleep, async runtime, global state, ambient
  authority, or unsafe Rust;
- store compare-and-swap is atomic and successful revisions increase;
- captured provider/tool cancellation handles remain inspectable;
- inspection clones consistent snapshots and all mutex poison is recovered.

During integration testing, the testkit exposed one core defect: delivery of
`ModelEvent::Usage` updated the accumulator but did not restore the provider
stream state, so a turn ended instead of consuming the later provider stop. The
coordinator expanded this feature's scope for the minimal state restoration.
Focused core and full testkit regressions now require the usage event, later
stop, terminal completion carrying the latest usage, and released session lease.
The fix is integrated into the later Milestone 02 tool-loop candidate and the
retained regression remains green.

The exact testkit candidate completed both remote workflows successfully:

| Workflow | Run |
| --- | --- |
| CI | [32397258414](https://github.com/distributedstatemachine/machine-god/actions/runs/32397258414) |
| Benchmark evidence | [32397258296](https://github.com/distributedstatemachine/machine-god/actions/runs/32397258296) |
