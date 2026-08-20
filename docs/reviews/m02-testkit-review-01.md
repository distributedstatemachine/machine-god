# Milestone 02 deterministic testkit review 01

Candidate commit: recorded after implementation verification.

Scope: `machine-god-testkit` doubles, dependency wiring, documentation, and
tests. Review must cover correctness/API, security/abuse, and
performance/concurrency with fresh agents against the committed candidate.

Pre-review invariants:

- scripts and retained observations have finite construction-time or explicit
  record bounds;
- exhaustion and capacity failures are visible and component-appropriate;
- no test double uses a clock, sleep, async runtime, global state, ambient
  authority, or unsafe Rust;
- store compare-and-swap is atomic and successful revisions increase;
- captured provider/tool cancellation handles remain inspectable;
- inspection clones consistent snapshots and all mutex poison is recovered.

Findings and resolutions will be appended by the milestone coordinator after
the candidate commit is reviewed.

During integration testing, the testkit exposed a core defect: delivery of
`ModelEvent::Usage` updated the accumulator but did not restore the provider
stream state, so a turn ended instead of consuming the later provider stop. The
coordinator expanded this feature's scope for the minimal state restoration.
Focused core and full testkit regressions now require the usage event, later
stop, terminal completion carrying the latest usage, and released session lease.
