# Milestone 03 native `open_file` review 01

Status: **IMPLEMENTED CANDIDATE; CYCLE 2 NOT GREEN; REMEDIATION IN PROGRESS**

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

The production system launcher admits at most
`MAX_CONCURRENT_OPEN_FILE_LAUNCHES = 32` active launches. Permit acquisition is
precommit immediately before worker creation; saturation is retryable
unavailable with no new worker or helper. The permit remains owned through
request/descriptor and helper cleanup, outcome publication, arbitrary Waker
execution, final notification state, and worker return. The final spawn attempt
and cancellation/drop abort transition share one
serialized state gate. Whichever transition obtains it first linearizes: abort
recorded first guarantees zero launch, while successful spawn under the gate is
the commit boundary. Postcommit cancellation makes core drop the execution
future; cleanup terminates and reaps the direct helper without claiming
rollback. Before publication, cancellation/drop suppresses Waker delivery,
reaps any helper, drops the request and retained descriptor, and synchronously
joins the worker. Helper reap and request/descriptor drop precede publication.
Normal postpublication cleanup joins the worker. A valid arbitrary Waker may
instead repoll inline on that worker or remain blocked while another thread
drops the future. Joining in those overlaps would self-join or create an
executor-lock cycle, so the `JoinHandle` is released; only that callback and
final notification bookkeeping may outlive drop, and their still-owned permit
bounds such tails globally to 32. Without cancellation, the 30-second deadline
starts immediately after successful spawn. A wait probe is accepted only when
its following authoritative clock read is strictly before the deadline, and
sleep is capped to the remaining budget. Explicit postspawn future drop
terminates and reaps the direct helper; cleanup may extend beyond the timeout
decision.
Nonzero exit, signal, timeout, or wait failure is the same fixed redacted
nonretryable `open_file_result_unknown`. Worker creation occurs before spawn,
so no postspawn waiter-establishment failure exists. Exit zero means only that
the direct helper accepted the request, not that a desktop application consumed,
displayed, or retained the file.

This lifecycle wording is a narrow documentation-only amendment to frozen
contract commit `6b763c4f1168963dd42087a1fdf5cf72c4212b40`. It replaces that
checkpoint's absolute no-worker-detach/every-drop-joins clause because legal
inline or blocking executor Wakers make it contradictory with deadlock-free
cleanup. It does not relax helper reap, request/descriptor closure, authority,
or confinement. Per the owner's instruction the amendment is exempt from its
own adversarial review; implementation and evidence are not exempt.

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

## Formal review cycle 2: not green

All three fresh tracks reviewed exact candidate
`027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`. The cycle is **NOT GREEN** and
that candidate is rejected for delivery. Consolidated formal findings are:

- Preflight measured the complete serialized input before enforcing the path
  byte bound, permitting unbounded path-proportional serialization work.
  Remediation must inspect shape and borrowed path length first and retain a
  very-large-path regression proving no complete serialization precedes
  rejection.
- The wait loop could accept an exit-zero status found by a probe after the
  fixed deadline. Remediation makes the post-probe monotonic clock authoritative:
  acceptance requires `now < deadline`, while `now >= deadline` is timeout, and
  every sleep is capped to the remaining duration.
- System launches had no active worker cap, so concurrent calls could create an
  unbounded number of threads. Remediation adds exactly 32 process-global
  system-launch permits, returns retryable precommit unavailable on saturation
  with zero new worker/helper, and holds each permit through arbitrary Waker
  completion and worker return.
- A postpublication wake tail could outlive future drop while still retaining
  the launch request and target descriptor. Remediation requires helper reap
  and request/descriptor drop before outcome publication. A blocked-waker
  regression must verify closure by exact descriptor identity while the wake
  callback is still blocked.
- The implemented detached wake tail contradicted the original frozen
  `6b763c4f1168963dd42087a1fdf5cf72c4212b40` absolute no-worker-detach
  invariant. That invariant is impossible alongside legal executor Wakers that
  repoll inline or block while another thread drops the future. The narrow
  amendment above requires synchronous Waker suppression/join before
  publication, normal join after publication, and permits only a callback/
  notification tail globally bounded by the retained 32-launch permit. The
  docs-only amendment is exempt from its own adversarial review under the
  owner's instruction.
- Proc-descriptor closure tests watched only the numeric fd path; descriptor
  number reuse could make a closed descriptor look live. Replacement evidence
  must compare the exact retained descriptor identity and distinguish numeric
  reuse from continued ownership.
- The recording fake launcher performed request-observation work in its launch
  constructor, contradicting the trusted seam's inert-until-first-poll
  contract. Replacement fake launchers must remain observationally inert until
  their returned future is polled.
- The forced wait-failure test seam branched around the real shared `try_wait`
  `Err` match arm. Replacement evidence must inject the failure through the
  same wait-result path used by the system call and prove fixed uncertainty plus
  helper reap.

These are rejected-candidate findings, not green remediation evidence. A
complete replacement exact-SHA local gate and three completely fresh review
tracks on one immutable replacement SHA/tree remain required.

## Required evidence

- [ ] Exact `Capability::OpenFile` API, serde JSON, exhaustive drop handling,
  native tool/schema/constants/result/open-error and fixed tool-error
  contracts, including strict unknown-field rejection and stable redaction.
- [ ] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds;
  very-large hostile path rejection before complete-value serialization.
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
  explicit host-boundary/no-sandbox semantics. Proc closure evidence must track
  exact descriptor identity rather than only a reusable numeric fd.
- [ ] Exact absolute `/usr/bin/xdg-open`, two-element argv, PID/fd decimal proc
  target, fixed `/` cwd, inherited host environment, null stdio, zero shell/PATH
  lookup by machine-god, trusted downstream dispatch, target-descriptor
  lifetime, and no model-controlled launch fields.
- [ ] Missing launcher and every spawn failure as retryable precommit
  unavailable with zero launch; exit-zero helper acceptance without application
  consumption/display claims.
- [ ] Nonzero, signal, timeout, and a deterministically injected failure through
  the actual shared `try_wait` `Err` arm as fixed redacted nonretryable
  `result_unknown`; timeout/wait failure terminates and reaps the helper; no
  nonexistent waiter-establishment state.
- [ ] Inert production and fake launcher construction/future until poll;
  cancellation/drop and spawn
  serialized through one linearization gate; abort-first zero launch;
  successful-spawn commit; postspawn engine cancellation through drop; pre-poll
  and postspawn drop; authoritative post-probe `now < deadline` acceptance,
  at/after-deadline timeout, remaining-duration sleep, then terminate/reap;
  unpublished Waker suppression plus synchronous join; helper reap and exact
  request/descriptor closure before publication; normal published worker join;
  safe overlapping inline/blocking wake-callback tail without self-join or
  cross-thread join cycles; blocked-waker exact-FD-identity regression; exactly
  32 active system launches with saturation producing precommit unavailable and
  zero new worker/helper; permit retention through callback completion;
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

**IMPLEMENTED CANDIDATE; CYCLE 2 NOT GREEN; REMEDIATION IN PROGRESS.** Exact
base main and frozen-contract feature CI and benchmark evidence is green.
Cycle 1 rejected exact candidate `79e65c19330181955a0c341d62ef39778a18d36d`,
tree `481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`, with the findings and candidate
remediations above. Cycle 2 rejected exact candidate
`027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`, for the eight resource-bound,
deadline, concurrency/lifecycle, frozen-contract, and test-fidelity findings
recorded above. The narrow worker-tail contract amendment is documentation-only
and exempt from its own adversarial cycle; production remediation is not.
Candidate source contains the core variant, native tool, trusted launcher seam,
direct/private/engine/unsupported evidence, and twelve-tool/eleven-clone host
composition with no dependency, workflow, CLI, benchmark, or compatibility-
status change. The complete replacement exact-SHA gate, three fresh green
review tracks, feature workflows, integration, main workflows, delivery,
product-performance, and fx-equivalence claims remain pending.
