# Milestone 03 native `open_file` review 01

Status: **IMPLEMENTED CANDIDATE; CYCLE 1 NOT GREEN; REMEDIATION IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `e2ee11f2c728721d2aa93219b5fafa86ea15b0c4`.
- Integration branch: `agent/m03-open-file`.
- Normative contract: [`open-file.md`](../open-file.md).
- Base main CI `32704202572` is green.
- Base main benchmark `32704202546` is green in both jobs and retains exactly
  two nonexpired exact-SHA artifacts, IDs `9511626648` and `9511745538`.

This documentation-only contract checkpoint is exempt from adversarial review
under the user's explicit instruction. Exact contract commit
`6b763c4f1168963dd42087a1fdf5cf72c4212b40` passed all six jobs of feature CI
`32707583915`. Feature benchmark workflow `32707583892` passed both jobs and
retains exactly two nonexpired exact-SHA artifacts, IDs `9512848704` and
`9512966283`. This is contract evidence only, not implementation, behavior,
delivery, performance, or fx-equivalence evidence.

## Implemented feature candidate

The twenty-sixth bounded slice asks one fixed Linux desktop helper to open one
strict canonical, workspace-confined, existing regular file. It freezes the
sole `{path:string}` input, exact path/JSON/result bounds, new dedicated
`Capability::OpenFile { path }` authority, retained-root descriptor-relative
no-follow lookup, final regular-file descriptor retention, and the proc target
`/proc/<machine-god-parent-pid>/fd/<retained-target-fd>`.

The production command is exactly `/usr/bin/xdg-open` with only that proc path
as its argument, fixed `/` working directory, inherited host desktop
environment, and null standard streams. Machine-god never uses a shell or
`PATH`, and no provider or model field can select the program, argv,
environment, or working directory. The trusted `xdg-open` and desktop-dispatch
boundary may consult inherited `PATH`, configuration, or host state. Linux is
the only concrete launch target. Every other target returns fixed unsupported
behavior before filesystem lookup, worker creation, or helper spawn.

The final spawn attempt and cancellation/drop abort transition share one
serialized state gate. Whichever transition obtains it first linearizes: abort
recorded first guarantees zero launch, while successful spawn under the gate is
the commit boundary. Postcommit cancellation makes core drop the execution
future; cleanup terminates and reaps the direct helper without claiming
rollback. Normal nonreentrant cleanup joins the worker. A valid inline waker may
reenter polling on that worker after helper reap and outcome publication; it
must drop rather than join the current thread handle. Cleanup overlapping any
still-running wake callback likewise drops the handle to avoid an executor-lock
cycle. Only that executor-controlled callback and final state update remain;
the helper is already reaped. Without cancellation, the
30-second timeout decision or explicit postspawn future drop terminates and
reaps the direct helper; cleanup may extend beyond the timeout decision.
Nonzero exit, signal, timeout, or wait failure is the same fixed redacted
nonretryable `open_file_result_unknown`. Worker creation occurs before spawn,
so no postspawn waiter-establishment failure exists. Exit zero means only that
the direct helper accepted the request, not that a desktop application consumed,
displayed, or retained the file.

The slice adds no external path, directory, URL, selected symlink, content read,
file mutation, arbitrary process authority, macOS real launch, CLI behavior,
benchmark workload, product-performance claim, inventory promotion, or
complete fx-equivalence claim.

## Candidate host composition

The delivered base host remains at eleven alphabetical tools:
`copy_file`, `create_folder`, `delete_file`, `edit_file`, `file_info`,
`glob_files`, `grep_files`, `list_files`, `read_file`, `rename_file`, and
`write_file`, using one original retained descriptor plus ten clones.

Current candidate composition inserts `open_file` after `list_files` and
before `read_file`, yields exactly twelve alphabetical tools, and uses one
original plus eleven identity-preserving clones. Both path-based and prepared-
root constructors compose that same tool catalog and retained workspace
identity. Formal review and delivery remain pending, so `main` remains at the
eleven-tool base.

## Implemented ownership

- Core/production owns the dedicated `OpenFile` capability and stable serde/
  drop evidence, native implementation and exports, deterministic launcher
  seam, retained-root composition, and reference-host registration.
- Independent evidence owns direct, private, race, engine, host, core-contract,
  unsupported-target, cancellation, process-lifecycle, timeout, drop,
  permission, bounds, and redaction tests.
- Documentation owns the normative contract, implementation plan, maintained
  architecture/security/API/host pages, and exact-SHA lineage record.

Production, host, direct/engine/unsupported evidence, and documentation were
composed from explicitly non-overlapping ownership. Only the final composed SHA
can become a formal behavior candidate.

## Formal review cycle 1: not green

All three tracks reviewed exact candidate
`79e65c19330181955a0c341d62ef39778a18d36d`, tree
`481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`. The cycle is **NOT GREEN** and
that candidate is rejected for delivery.

- Correctness/API reported one medium production defect: failed precommit
  filesystem operations and validation failures could map their filesystem
  error without the promised cancellation check afterward. Remediation captures
  every precommit operation/validation result, checks cancellation whether it
  succeeded or failed, and only then maps the result; a cancelled-token/error
  regression exercises the shared ordering helper.
- Correctness/API also reported one low evidence mismatch: maintained
  documentation checked wait failure without a deterministic seam and claimed
  an impossible postspawn waiter-establishment failure even though the worker
  exists before spawn. Remediation removes the nonexistent state and adds a
  deterministic forced-wait-failure candidate regression that requires fixed
  `result_unknown` and no live direct helper afterward.
- Filesystem/process-lifecycle reported one medium spawn race: the final abort
  check released its mutex before `Command::spawn`, allowing cancellation/drop
  to win state while the worker still launched. Remediation serializes the
  abort transition and spawn attempt under one gate and adds a barrier-based
  regression proving abort that wins the gate leaves no launch marker.
- Filesystem/process-lifecycle and performance/concurrency independently
  reported the same medium reentrant-waker defect: waking on the worker thread
  could synchronously repoll/drop and self-join that worker. Remediation avoids
  self-join after helper reap and outcome publication, limits the unjoined work
  to the wake callback's tail, and adds an inline-waker regression proving ready
  acceptance without panic or deadlock.
- Remediation preflight then found the broader cross-thread form: a wake callback
  blocked on the future could deadlock with another thread dropping that future
  while joining the worker. The replacement cleanup suppresses notification
  before joining unpublished work, but never joins a published worker while its
  arbitrary wake callback remains active. A barrier regression proves overlapping
  drop returns before the callback is released; the helper is already reaped.

These are remediation-candidate changes, not green formal-review evidence. The
complete replacement local gate and three completely fresh tracks on one exact
replacement SHA/tree remain required.

## Required evidence

- [ ] Exact `Capability::OpenFile` API, serde JSON, exhaustive drop handling,
  native tool/schema/constants/result/open-error and fixed tool-error
  contracts, including strict unknown-field rejection and stable redaction.
- [ ] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds.
- [ ] Empty/root/dot, absolute, tilde, parent, repeated/trailing separator,
  dot-component, control, line/paragraph-separator, bidirectional, and
  over-bound rejection; no Unicode normalization or case folding.
- [ ] Effect-free preparation, exact
  `{"type":"open_file","path":"..."}` evidence, denial before lookup,
  canonical direct execution, exact policy/execution agreement, and no general
  filesystem-read or process authority.
- [ ] Fresh retained-root validation, descriptor-relative no-follow traversal,
  final linked regular-file requirement, directory/symlink/FIFO/socket/device
  rejection, and absence of content reads.
- [ ] Root, ancestor, and final replacement; rename and unlink after retention;
  mixed-device traversal; proc-entry availability; outside sentinels; and the
  explicit host-boundary/no-sandbox semantics.
- [ ] Exact absolute `/usr/bin/xdg-open`, two-element argv, PID/fd decimal proc
  target, fixed `/` cwd, inherited host environment, null stdio, zero shell/PATH
  lookup by machine-god, trusted downstream dispatch, target-descriptor
  lifetime, and no model-controlled launch fields.
- [ ] Missing launcher and every spawn failure as retryable precommit
  unavailable with zero launch; exit-zero helper acceptance without application
  consumption/display claims.
- [ ] Nonzero, signal, timeout, and deterministically forced wait failure as
  fixed redacted nonretryable `result_unknown`; timeout/wait failure
  terminates and reaps the helper; no nonexistent waiter-establishment state.
- [ ] Inert construction/future until poll; cancellation/drop and spawn
  serialized through one linearization gate; abort-first zero launch;
  successful-spawn commit; postspawn engine cancellation through drop; pre-poll
  and postspawn drop; 30-second timeout decision followed by terminate/reap;
  normal nonreentrant worker join; safe overlapping wake-callback tail without
  self-join or cross-thread join cycles; blocked-waker drop regression;
  concurrent-call isolation.
- [ ] Native Linux behavior, macOS/FreeBSD/WASI compilation and active
  unsupported-target behavior, exact delivered eleven-tool checkpoint and
  candidate twelve-tool/eleven-clone host, no-unsafe, dependency,
  compatibility, documentation,
  diff, and fresh release-binary smoke evidence.

## Exact local gate before formal review

The composed candidate must first pass focused open-file private, direct,
engine, host, core-contract, launcher, process-lifecycle, and unsupported-target
suites. Then run the exact Rust 1.94.1 repository gate:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Also require the repository's Python, compatibility, dependency-policy,
dependency-audit, native Linux execution, native macOS unsupported-target
execution, cross-target, active WASI unsupported-target, Markdown-link, clean-
diff, no-unsafe, and freshly built release-binary smoke checks. Record all
exact counts, hashes, versions, expected skips, and any valid exact-toolchain
fallback. Real-launch tests must use a controlled fake or injected launcher;
CI must not open a desktop application. Local success is not delivery or
performance evidence.

## Formal same-SHA review protocol

After the complete local exact-SHA gate, create a tree-identical behavior
candidate and start three fresh reviewers against that same immutable SHA and
tree:

1. correctness/API;
2. filesystem/process-lifecycle robustness;
3. performance/concurrency.

Each reviewer independently verifies the SHA/tree in a clean detached worktree
and runs the applicable focused evidence. Every confirmed finding is fixed, the
complete local gate is rerun, and all three tracks restart with fresh reviewers
on one replacement SHA. Repeat until every track is green with zero findings.

Only then may a documentation seal be pushed for exact feature CI and benchmark
workflows. After those pass for that exact SHA, fast-forward `main` without
force and require exact main CI and benchmark workflows. Each claimed benchmark
run must retain the expected nonexpired exact-SHA artifacts. Documentation-only
seal and final delivery-record commits are exempt from another adversarial
cycle, but their exact workflows remain required. No package or release
publication is authorized by this review.

## Current verdict

**IMPLEMENTED CANDIDATE; CYCLE 1 NOT GREEN; REMEDIATION IN PROGRESS.** Exact
base main and frozen-contract feature CI and benchmark evidence is green.
Cycle 1 rejected exact candidate `79e65c19330181955a0c341d62ef39778a18d36d`,
tree `481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`, with the findings and candidate
remediations above. Candidate source now
contains the core variant, native tool, trusted launcher seam, direct/private/
engine/unsupported evidence, and twelve-tool/eleven-clone host composition with
no dependency, workflow, CLI, benchmark, or compatibility-status change. The
complete replacement exact-SHA gate, three fresh green review tracks, feature
workflows, integration, main workflows, delivery, product-performance, and fx-
equivalence claims remain pending.
