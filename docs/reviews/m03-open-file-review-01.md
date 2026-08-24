# Milestone 03 native `open_file` review 01

Status: **DELIVERED**

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

## Delivered feature

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

## Delivered host composition

Before `open_file` delivery, the delivered base host contained eleven
alphabetical tools:
`copy_file`, `create_folder`, `delete_file`, `edit_file`, `file_info`,
`glob_files`, `grep_files`, `list_files`, `read_file`, `rename_file`, and
`write_file`, using one original retained descriptor plus ten clones.

The delivered composition inserts `open_file` after `list_files` and
before `read_file`, yields exactly twelve alphabetical tools, and uses one
original plus eleven identity-preserving clones. Both path-based and prepared-
root constructors compose that same tool catalog and retained workspace
identity. Formal cycle 5 rejected exact candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`, for one low documentation-lineage
finding. At that checkpoint, cycle-6 review and delivery remained pending and
`main` remained at the eleven-tool base.

Exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`, is green with zero findings in all
three fresh tracks. The twelve-tool composition is review-green but remains
delivered on `main`.

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

## Formal review cycle 3: not green

All three fresh tracks reviewed exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`. The cycle is **NOT GREEN** and
that candidate is rejected for delivery.

- Correctness/API is **GREEN** with zero findings.
- Performance/concurrency is **NOT GREEN** with one low evidence finding. The
  deterministic deadline test reached the pre-probe deadline guard and stopped
  there; it did not drive `try_wait` and then cross the deadline before the
  authoritative post-probe clock read. Replacement evidence must pause after
  the wait probe and prove an exit-zero status observed at or after the
  deadline maps to timeout rather than acceptance.
- Filesystem/process-lifecycle is **NOT GREEN** with one low lifecycle/evidence
  finding. In a publication with no registered Waker, the future could observe
  the ready outcome before notification completion became visible and release
  the worker handle, detaching the tail instead of taking the required ordinary
  postpublication join path. This tail was still globally bounded by the
  retained permit; the helper was reaped and the request and exact retained
  descriptor were already
  released. The reviewer found no production resource escape. Remediation
  requires atomic `notification_complete` publication and a deterministic
  no-Waker regression proving ready consumption joins the ordinary worker tail.

The cycle has zero blocker, high, or medium findings, zero other findings, and
no production resource escape. It does not amend or invalidate the authorized
documentation-only lifecycle amendment. Candidate remediation atomically
completes no-Waker publication and supplies both deterministic regressions.

## Formal review cycle 4: not green

All three fresh tracks reviewed exact candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, after its complete replacement
local gate passed. The cycle is **NOT GREEN** and the candidate is rejected.

- Filesystem/process-lifecycle is **GREEN** with zero findings after exact Rust
  1.94.1 Linux system 14/14, direct 12/12, engine 4/4, and warnings-denied
  Clippy evidence.
- Performance/concurrency is **GREEN** with zero findings after the same focused
  suites plus five repeated lifecycle runs, 70/70.
- Correctness/API found zero production or public-API defects and one low
  maintained-documentation lineage drift. Four cross-cutting summaries still
  described cycle-2 remediation, omitted rejected cycle 3, or incorrectly
  called the twelve-tool composition unreviewed.

Cycle 4 has zero blocker, high, or medium findings and one low documentation
finding. The maintained summaries now record exact cycle-3 SHA/tree and both
low findings, exact cycle-4 SHA/tree and its verdict, and reviewed-but-rejected
composition status.

## Formal review cycle 5: not green

All three fresh tracks reviewed exact candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. The complete replacement local
gate passed, but the cycle is **NOT GREEN** and the candidate is rejected.

- All tracks found zero production/API, lifecycle, performance/concurrency, or
  resource-bound defects.
- All tracks reported the same low maintained-documentation lineage defect.
  README still named cycle 3 as the latest rejection; the host-composition
  paragraph called the cycle-3 candidate current; and the contract and ledger
  used generic pending-review wording after four completed cycles.
- Exact native Linux arm64 Rust 1.94.1 evidence was green at system 14/14,
  direct 12/12, engine 4/4, warnings-denied Clippy, and five repeated lifecycle
  runs totaling 70/70. An amd64-under-arm64 missing-executable emulation result
  did not reproduce on native arm64 and is not a product finding.

At the cycle-5 checkpoint, the four stale passages were corrected and a fresh
correctness/API, filesystem/process-lifecycle, and performance/concurrency
cycle 6 on one immutable replacement SHA/tree remained required. No green-
review, workflow, delivery, performance, or equivalence claim was made at that
checkpoint.

## Formal review cycle 6: green

All three fresh reviewers independently verified exact candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`, in clean detached worktrees.
Correctness/API, filesystem/process-lifecycle, and performance/concurrency are
all **GREEN** with zero blocker, high, medium, low, or informational findings.

- Filesystem/process-lifecycle passed native Linux arm64 Rust 1.94.1 system
  14/14, direct 12/12, engine 4/4, and warnings-denied Clippy.
- Performance/concurrency passed the same focused Linux matrix and five
  repeated lifecycle runs totaling 70/70.
- Correctness/API passed core serde 1/1, macOS active unsupported behavior 1/1,
  and all-feature host composition 1/1 in addition to its focused checks.

The complete exact-candidate local gate is green: workspace formatting,
all-target/all-feature warnings-denied Clippy, workspace tests, workspace
doctests, and no-run compilation; 130 Python tests with eight expected macOS
skips; byte-identical compatibility against pinned fx `b1774f`; `cargo-deny`
0.20.2; and `cargo-audit` 0.22.2 with zero vulnerabilities. FreeBSD
compilation, WASI compilation and active Node unsupported behavior 1/1,
documentation checks, diff checks, and release smokes are green. The freshly
built release binary is 319,152 bytes with SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.

Seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
`03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
at 6/6 and feature benchmark `32738160725` at 2/2. That benchmark retains
upstream artifact `9524219365` and bootstrap artifact `9524052760`. Exact main
CI `32738798417` passed 6/6, and main benchmark `32738798415` passed 2/2 while
retaining upstream artifact `9524461989` and bootstrap artifact `9524298408`.
Feature delivery, non-force fast-forward integration, and exact `main`
workflows are complete. Native `open_file` is delivered as bounded slice
twenty-six in the twelve-tool/eleven-clone host. This makes no product-
performance or fx-equivalence claim. At that checkpoint, the final docs-only
record was exempt from adversarial review under the user's instruction; its own
exact feature and `main` workflows remained required and are reported below.

Final delivery-record SHA
`762d70df106d40e59b599e18b1ac5c62f678927d`, tree
`909eb320e05df4d56f5bcecf0e3655e6d761f622`, passed feature CI `32740668405`
at 6/6 and feature benchmark `32740667465` at 2/2, retaining upstream artifact
`9525188220` and bootstrap artifact `9525017236`. Main benchmark `32741322179`
passed 2/2 and retained upstream artifact `9525436660` and bootstrap artifact
`9525268460`. Main CI `32741322249` was not green: five of six jobs passed,
and Quality alone failed the exact test named by concatenating
`blocked_wake_releases_request_before_publication_and_holds_` with
`permit_until_worker_return` because the immediate `exit 0` fixture could
legitimately publish before the first poll installed `BlockingWake`. This is a
test-fixture synchronization defect, not a production behavior finding. Exact
local test-only remediation
`62c2a5349bc682079c2458ccebe9f9ea9578a3c1`, tree
`b38984441b6bb470ecb4b1c69bc9a3a9984f0bb0`, adds the existing
`before_first_wait` barrier and passed the normal native Linux arm64 exact test
100/100. Exact cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, received **GREEN** correctness/API
and filesystem/process-lifecycle reviews with zero findings. Performance/
concurrency was **NOT GREEN** with exactly two low findings and zero blocker,
high, or medium findings: the unconditional `before_first_wait` rendezvous
could hang when `Command::spawn` failed before the hook, as reproduced on
native Linux arm64 with `/tmp` mounted `noexec`; and maintained current/
operative documentation tails ended at the superseded cycle-6/handoff state.
The candidate is rejected. Exact test-only code remediation
`274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the existing
`before_spawn` barrier, reached before every spawn outcome, so Waker
registration deterministically precedes publication. The normal native Linux
arm64 exact test passed 100/100, and the `/tmp`-noexec spawn-failure case passed
1/1. Production source, public API, and manifests are unchanged. This docs
correction composes atop that commit; its SHA is pending. The full replacement
local gate and all three fresh cycle-8 tracks remain pending. This executable
test-only fix is not eligible for the documentation-only exemption. This makes
no product-performance or fx-equivalence claim.

Exact documentation-correction and cycle-8 candidate
`6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
`44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the complete replacement
local gate. Cycle-8 correctness/API was **NOT GREEN** with exactly two low
findings and zero blocker, high, or medium; lifecycle and
performance/concurrency were green with zero findings. The first low found no
successful-helper witness in the normal blocked-Waker fixture, whose remaining
assertions also passed after a noexec spawn failure. The second found that the
operative documentation did not identify the known cycle-8 candidate. Exact
test-only remediation `a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
`7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, factors the shared lifecycle
assertions, makes the successful helper write and verify an exact marker, and
adds a separate deterministic missing-helper spawn-failure case. Normal Linux
arm64 focused evidence is 202/202; the failure case under `/tmp` noexec is
100/100. Production source, public API, manifests, and workflows remain
unchanged. The documentation correction and exact cycle-9 candidate SHA/tree,
complete replacement local gate, and three fresh cycle-9 reviews remain
pending. This makes no product-performance or fx-equivalence claim.

Exact cycle-9 reviewed candidate
`964c59408bda1a3793978041432b84b808b474a6`, tree
`7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the complete replacement
local gate. All three fresh correctness/API, filesystem/process-lifecycle, and
performance/concurrency reviews are **GREEN** with zero blocker, high, medium,
or low findings. Rust 1.94.1 evidence includes Linux arm64 system 15/15, direct
12/12, engine 4/4, normal split cases 202/202, noexec failure 100/100,
warnings-denied Clippy, the full Rust workspace, two doctests, Python 130 with
eight expected macOS skips, pinned-fx compatibility, dependency policy/audit,
FreeBSD/WASI plus active Node 1/1, documentation checks, and five release CLI
smokes. The 319,152-byte release binary retains SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
Production source, public API, manifests, and workflows remain unchanged. The
subsequent documentation-only green seal names this reviewed candidate and is
exempt from another adversarial cycle under the user's instruction; its exact
SHA/tree and feature/main workflows remain pending. These are regression and
delivery checks, not a product-performance or fx-equivalence claim.

## Required evidence

- [x] Exact `Capability::OpenFile` API, serde JSON, exhaustive drop handling,
  native tool/schema/constants/result/open-error and fixed tool-error
  contracts, including strict unknown-field rejection and stable redaction.
- [x] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds;
  very-large hostile path rejection before complete-value serialization.
- [x] Empty/root/dot, absolute, tilde, parent, repeated/trailing separator,
  dot-component, control, line/paragraph-separator, bidirectional, and
  over-bound rejection; no Unicode normalization or case folding.
- [x] Effect-free preparation, exact
  `{"type":"open_file","path":"..."}` evidence, denial before lookup,
  canonical direct execution, exact policy/execution agreement, and no general
  filesystem-read or process authority.
- [x] Fresh retained-root validation, descriptor-relative no-follow traversal,
  final linked regular-file requirement, directory/symlink/FIFO/socket/device
  rejection, and absence of content reads.
- [x] Root, ancestor, and final replacement; rename and unlink after retention;
  mixed-device traversal; proc-entry availability; outside sentinels; and the
  explicit host-boundary/no-sandbox semantics. Proc closure evidence must track
  exact descriptor identity rather than only a reusable numeric fd.
- [x] Exact absolute `/usr/bin/xdg-open`, two-element argv, PID/fd decimal proc
  target, fixed `/` cwd, inherited host environment, null stdio, zero shell/PATH
  lookup by machine-god, trusted downstream dispatch, target-descriptor
  lifetime, and no model-controlled launch fields.
- [x] Missing launcher and every spawn failure as retryable precommit
  unavailable with zero launch; exit-zero helper acceptance without application
  consumption/display claims.
- [x] Nonzero, signal, timeout, and a deterministically injected failure through
  the actual shared `try_wait` `Err` arm as fixed redacted nonretryable
  `result_unknown`; timeout/wait failure terminates and reaps the helper; no
  nonexistent waiter-establishment state.
- [x] Inert production and fake launcher construction/future until poll;
  cancellation/drop and spawn
  serialized through one linearization gate; abort-first zero launch;
  successful-spawn commit; postspawn engine cancellation through drop; pre-poll
  and postspawn drop; authoritative post-probe `now < deadline` acceptance,
  at/after-deadline timeout through a deterministic pause after `try_wait`,
  remaining-duration sleep, then terminate/reap;
  unpublished Waker suppression plus synchronous join; helper reap and exact
  request/descriptor closure before publication; atomic
  `notification_complete` publication and deterministic no-Waker normal
  published-worker join;
  safe overlapping inline/blocking wake-callback tail without self-join or
  cross-thread join cycles; blocked-waker exact-FD-identity regression; exactly
  32 active system launches with saturation producing precommit unavailable and
  zero new worker/helper; permit retention through callback completion;
  concurrent-call isolation.
- [x] Native Linux behavior, macOS/FreeBSD/WASI compilation and active
  unsupported-target behavior, exact delivered eleven-tool checkpoint and
  candidate twelve-tool/eleven-clone host, no-unsafe, dependency,
  compatibility, documentation,
  diff, and fresh release-binary smoke evidence.
- [x] Exact feature workflows, non-force fast-forward integration, and exact
  `main` workflows for the twelve-tool delivery seal.

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

**DELIVERED.** Exact
base main and frozen-contract feature CI and benchmark evidence is green.
Cycle 1 rejected exact candidate `79e65c19330181955a0c341d62ef39778a18d36d`,
tree `481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`, with the findings and candidate
remediations above. Cycle 2 rejected exact candidate
`027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`, for the eight resource-bound,
deadline, concurrency/lifecycle, frozen-contract, and test-fidelity findings
recorded above. The narrow worker-tail contract amendment is documentation-only
and exempt from its own adversarial cycle; production remediation is not.
Cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`. Correctness/API is green with
zero findings. Performance/concurrency reported one low missing authoritative
post-`try_wait` deadline regression. Filesystem/process-lifecycle reported one
low no-Waker normal-join gap requiring atomic `notification_complete`
publication. The cycle has zero blocker, high, or medium findings, zero other
findings, and no production resource escape. The candidate is rejected. Cycle
4 then reviewed exact candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`. Lifecycle and performance were
green with zero findings. Correctness/API found no production defect and one
low stale maintained-documentation lineage finding. That remediation was
composed into cycle-5 candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks found zero
production defects and the same low remaining current-lineage wording defect.
That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are green with
zero findings at every severity.

Delivered source contains the core variant, native tool, trusted launcher seam,
direct/private/engine/unsupported evidence, and twelve-tool/eleven-clone host
composition with no dependency, workflow, CLI, benchmark, or compatibility-
status change. The exact seal, feature workflows, non-force fast-forward
integration, and exact `main` workflows recorded above deliver the twelve-tool
host using one retained descriptor plus eleven identity-preserving clones.
This final docs-only record is exempt from adversarial review under the user's
instruction but still required its own exact feature and `main` workflows,
which were to be reported at that delivery checkpoint.

Subsequent exact cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, is **NOT GREEN** and rejected.
Correctness/API and filesystem/process-lifecycle are green with zero findings.
Performance/concurrency reported exactly two low findings and zero blocker,
high, or medium findings: the unconditional `before_first_wait` rendezvous
could hang if `Command::spawn` failed before reaching the hook, reproduced on
native Linux arm64 with `/tmp` mounted `noexec`; and maintained current/
operative tails stopped at the superseded cycle-6/handoff state. Exact
test-only code remediation `274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`,
tree `865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the
existing `before_spawn` barrier, reached before every spawn outcome, so Waker
registration deterministically precedes publication. The normal native Linux
arm64 exact test passed 100/100, and the `/tmp`-noexec spawn-failure case passed
1/1. Production source, public API, and manifests are unchanged. This docs
correction composes atop that commit; its SHA is pending. The full replacement
local gate and all three fresh cycle-8 tracks remain pending. The executable
test-only fix is not eligible for the documentation-only exemption. No
product-performance or fx-equivalence claim is made.

Subsequent exact cycle-8 candidate
`6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
`44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the full local gate.
Correctness/API rejected it with exactly two low findings and zero blocker,
high, or medium: no successful-helper witness in the normal fixture, and stale
operative lineage that did not name this candidate. Lifecycle and
performance/concurrency were green at zero findings. Exact remediation
`a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
`7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, adds distinct successful-marker
and deterministic spawn-failure tests around shared lifecycle assertions;
normal Linux arm64 evidence is 202/202 and the noexec failure case is 100/100.
Production source, public API, manifests, and workflows remain unchanged. The
documentation correction/cycle-9 candidate, full replacement gate, and three
fresh cycle-9 reviews remain pending. This makes no product-performance or
fx-equivalence claim.

Exact cycle-9 candidate `964c59408bda1a3793978041432b84b808b474a6`, tree
`7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the full replacement gate.
All three fresh correctness/API, filesystem/process-lifecycle, and
performance/concurrency reviews are green with zero findings at every
severity. Production source, public API, manifests, and workflows remain
unchanged. A documentation-only green seal naming that reviewed candidate is
exempt from another adversarial cycle; its exact SHA/tree and feature/main
workflows remain pending. This makes no product-performance or fx-equivalence
claim.

Reviewer-owned Rust 1.94.1 Linux arm64 evidence independently confirms the
split fixture. Correctness/API passed system 15/15, normal split cases 202/202,
and noexec failure 100/100. Lifecycle passed the same focused counts plus full
system-suite stress 10/10; its negative control ran the success-marker test
under noexec and observed prompt marker failure rather than a timeout or false
success. Performance/concurrency passed normal success 1,000/1,000, normal
missing-helper 1,000/1,000, noexec missing-helper 1,000/1,000, paired changed
tests 2,000/2,000, 0.25-CPU focused stress 1,500/1,500, parallel system
matrices 300/300, and isolated serial matrices 150/150. These invocations were
externally bounded. This is concurrency-regression evidence, not a product-
performance comparison.
