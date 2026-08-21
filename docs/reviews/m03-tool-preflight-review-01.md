# Milestone 03 capability-aware tool preflight review 01

Status: **ADVERSARIAL GREEN — exact remote CI pending**

## Candidate

- Base: `7374d807659eded924956a06854f25ec78734bd5`
- Adversarially green implementation: `c22d10eda72b75ff23cbd7244df589966e8a3cfa`
- Branch: `agent/m03-tool-preflight`
- Toolchain: Rust and Cargo 1.94.1 exactly

The candidate adds a provider-neutral, effect-free tool-preflight boundary to
core and a reusable scripted prepared-tool fixture to the testkit. It does not
add an executable native tool or change the CLI, native configuration loader,
provider implementations, compatibility inventory, benchmark workloads, or
product performance claims. Milestone 03 remains in progress.

## Reviewed behavior

- Add the source-compatible, object-safe `Tool::prepare` method. Its default
  preserves the registered tool name, provider call ID, provider arguments,
  raw `Capability::Tool`, and exact execution arguments.
- Permit a trusted tool to return a `PreparedToolCall` containing the exact
  normalized capability shown to policy and the exact JSON arguments delivered
  to execution after policy allows that capability.
- Require preparation to be synchronous, bounded, nonblocking, deterministic,
  and effect-free. Core checks cancellation immediately before and after the
  call; it does not claim to preempt preparation in flight.
- Require an implementation's later effects to remain within the authority of
  the prepared capability and to interpret its prepared arguments consistently
  with that authorized operation.
- Validate prepared arguments at the existing JSON depth, node, and exact
  serialized-byte limits. Traverse depth and nodes for JSON values embedded in
  `Tool` and `Custom` capabilities, and serialize every complete capability
  under one total byte cap of `max_tool_argument_bytes + 1024`.
- Preserve exact existing authorization identity, critical risk, fixed reason,
  provider order, placeholder durability, denial behavior, and absence of
  core-side grant caching.
- Convert preparation errors into the existing generic durable tool-error
  result without consulting policy or starting execution, so the model may
  recover in a later round without receiving trusted-host diagnostics.
- Reclaim deeply nested prepared JSON iteratively on cancellation, rejection,
  denial, error, and normal ownership exits.
- Supply a bounded `ScriptedPreparedTool` whose preparation and execution
  scripts, recordings, capacity, exhaustion behavior, poison recovery, and
  snapshots share one synchronized state.

## Parallel implementation

Three isolated worktrees initially owned non-overlapping core, testkit, and
documentation changes. Follow-up commits preserve review and remediation
history:

- `de7cf15` — freeze the public preflight contract and milestone boundary;
- `cb1de50` — add core preparation, validation, policy, execution, error, and
  stack-safe ownership behavior;
- `71412aa` — add the scripted prepared-tool fixture;
- `cb09e25` — document the fixture and examples;
- `8ba7312` — clarify bounded synchronous preflight and authority obligations;
- `269c64e` — add fixture concurrency, poison, exhaustion, and capacity tests;
- `8e51214` — preserve the exact raw-argument boundary and add the fixed
  capability headroom;
- `77383b8` — align the cross-crate oversized-capability regression;
- `16b66c7` — precisely scope capability depth, node, and byte bounds; and
- `c22d10e` — make fixture linearization and partial-snapshot evidence
  deterministic.

## Adversarial rounds

### Round 1 — `cb09e2511850b7221908f9f1ea8151a6418c605e`

Three cross-ownership, read-only reviews covered core correctness and public API,
testkit concurrency and evidence, and documentation/security boundaries.

- **HIGH — accepted:** core applied `max_tool_argument_bytes` to the complete
  default `Capability::Tool` envelope after already accepting raw arguments at
  that exact limit. Wrapper fields could therefore reject a previously valid
  legacy invocation.
- **MEDIUM — accepted:** documentation could be read as claiming cancellation
  interrupts synchronous preflight, although core can only check immediately
  before and after the call.
- **MEDIUM — accepted:** the new testkit evidence did not directly exercise
  concurrent snapshots, poisoned-state recovery, script exhaustion, or exact
  record-capacity boundaries independently for preparation and execution.
- **MEDIUM — accepted:** the public API did not state the trusted tool's
  normative obligation to keep execution authority within the prepared
  capability and interpret the prepared arguments consistently with it.

Resolutions `8ba731265f0ca5dd8426a019791189736128cb50`,
`269c64e04d580eef01f644c1e50e9894d0079630`, and
`8e51214eb1eb6cab301505c6e844e38ed347f7e7` strengthened Rustdoc and guides,
added direct fixture evidence, retained the exact configured limit for prepared
arguments, and gave complete capability serialization an overflow-safe fixed 1
KiB headroom. Exact and plus-one real-engine regressions cover both limits.

The combined workspace gate then caught one stale integration assertion that
still expected a capability below the new total cap to fail. Resolution
`77383b8135cd3b234c895a1f4d2e0ae7975304ca` moved that oversized test beyond the
documented derived cap.

### Round 2 — `77383b8135cd3b234c895a1f4d2e0ae7975304ca`

- **MEDIUM — accepted:** the concurrency regression could observe only final
  state and discarded the unique result of each scripted step, so it did not
  prove a partial snapshot or bind record order to script-step order.
- **MEDIUM — accepted:** the API guide described JSON depth and node validation
  as applying to every complete typed capability. The implementation applies
  those traversals to embedded `serde_json::Value` fields and applies the whole
  serialized-byte cap to every capability variant. The guide also described 1
  KiB as an envelope allowance without making clear that it is headroom inside
  one total cap.
- The third reviewer otherwise reported GREEN.

Resolution `16b66c789a734595dcc37ba308bfebd41ca28c3c` corrected architecture,
API, security, and plan wording. Resolution
`c22d10eda72b75ff23cbd7244df589966e8a3cfa` replaced scheduler-luck sampling
with two gated worker waves. It proves an exact partial snapshot before the
second wave can run, retains every call's unique returned preparation or output,
and maps those outcomes by call ID to the fixture's recorded order. A split or
reordered record-to-step assignment is therefore detected without assuming
thread scheduling order.

### Round 3 — `c22d10eda72b75ff23cbd7244df589966e8a3cfa`

All three reviewers reported GREEN with no actionable findings. They verified:

- default source compatibility and object safety;
- normalized policy and execution inputs, denial, grant, and durable-error
  paths;
- immediate before/after cancellation checks and explicitly non-preemptible
  synchronous preparation;
- exact prepared-argument and derived complete-capability boundaries;
- embedded-JSON-only depth and node traversal wording;
- iterative deep-value ownership and rejection paths;
- deterministic barrier counts, partial and final snapshots, unique call IDs,
  record-to-step ordinal mapping, exact remaining counts, poison recovery,
  exhaustion, and independent capacity; and
- absence of native, CLI, provider, benchmark, compatibility, generated, or
  performance-claim changes.

The focused gated-wave regression also passed 20 consecutive runs during this
round.

## Exact local checks

The following passed on the adversarially green implementation SHA using exact
Rust/Cargo 1.94.1:

- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace/all-target/all-feature tests: 210 top-level tests, plus 15
  deep-JSON child-process probes;
- workspace documentation tests: 2;
- repo-wide Python discovery: 129 run, comprising 121 passed and 8 expected
  platform skips;
- the pinned upstream compatibility inventory check against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- release build and bare/help/status JSON CLI smoke checks;
- `cargo-deny check`: advisories, bans, licenses, and sources all accepted;
- `cargo audit --no-fetch`: 1,225 cached advisories checked across 33 lockfile
  dependencies with no finding;
- relative documentation links: 36 checked; and
- `git diff --check` and a clean worktree.

The stripped local release CLI remained 319,152 bytes. This is a local
regression observation only, not retained cross-platform benchmark evidence or
a product performance claim.

## Remaining gates and scope

The feature branch and its eventual fast-forwarded `main` SHA must still pass
their exact remote CI and benchmark-evidence workflows. The benchmark workflow
continues to use Zig only to build the pinned upstream fx comparison target;
machine-god remains a Rust product.

The first native `read_file` tool is the next bounded slice and must use this
preflight seam so policy and execution describe the same normalized path.
Workspace confinement, symlink-safe filesystem access, concrete providers,
permission prompting and modes beyond `ask`, durable native sessions, broader
configuration and CLI behavior, remaining native tools, and compatibility or
performance claims remain planned. No package or GitHub release is authorized.
