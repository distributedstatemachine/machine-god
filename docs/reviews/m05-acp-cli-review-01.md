# Modern ACP validation and review history

This file retains historical evidence, not live delivery status. The
[implementation plan](../implementation-plan.md) owns the current phase and gates.

## Candidate 53096f44: local validation rejection

Candidate: `53096f441f1b927e9494a91434b0b06a95975d2d`.
Base: `658f3366258cf1207904f9c2a274f32db2bb981b`.
No adversarial product-review verdict or delivery acceptance follows this entry.

The Rust 1.94.1 macOS gate passed formatting, warnings-denied Clippy, fresh
release helper construction, focused skills and ACP composition tests, serial
workspace tests, doctests, release smoke, FreeBSD/WASI compilation checks,
pinned-source drift checks, documentation policy, dependency policy and audit.
The release helper SHA-256 was
`f9c4f3ddf99f64b6c49074b78b65b569c72607d1f38aa9c59437676179da3c8e`.
A shell-wrapper failure after workspace/doctest success was traced to an RVM
`cd` hook under `set -u`; remaining checks passed using `builtin cd` without
changing the candidate, helper or test deadlines.

Linux used UID/GID 10001, no DAC-bypass capabilities, a private home, valid
passwd entry, reaping init, bash, zsh, tmux and Rust 1.94.1. Formatting, Clippy,
fresh release build, 54 focused skills tests and the composed ACP round trip
passed. The helper SHA-256 was
`3b936bdc20c17c67d69cbbae99ce84ce0449896e86da1de100461cff546c3e35`.
Default-concurrency workspace execution stopped in CLI units: 532 passed,
one failed and six existing helper fixtures were ignored. The failing
`mcp_commands_cancel_and_session_switch_record_observations_without_replay`
test hit its unchanged ten-second output deadline with an empty retained
transcript. The original failure did not identify the expected output or caller.

One unchanged focused retry and eleven unchanged full CLI unit runs passed.
These retries are diagnostic observations, not a source fix or complete gate.
Read-only tracing established that the shared session picker could silently
discard Enter received after a selectable frame was rendered but before its
flush acknowledgement. That code predates this branch. It is a concrete defect,
but neither the source trace nor the passing retries prove it caused the
recorded timeout. Input received before any selectable frame exists must still
be rejected rather than transferred to a later frame.

The separate unchanged Linux Python suite ran 269 tests in 129.293 seconds.
Its fresh-target release cleanup probe passed with the original 600-second
build and ten-second execution deadlines. The suite nevertheless failed because
Git 2.39.5 lacked `--no-lazy-fetch`, and an equal-length benchmark fixture write
could retain both its original modification and change timestamps. Instrumented
reproduction observed identical before/after timestamps on three of five runs.
This is not a successful complete Python gate. Earlier macOS cold-probe evidence
remains distinct: parent `658f3366` passed in 586.253 seconds, while candidate
`2e8283e3` exceeded the unchanged 600-second build deadline.

Raw macOS evidence is retained under `target/agent-gates/acp-connection.aN177x`;
Linux evidence is under `/cache/target/acp-53096f44.u1HjzP` in the owned validation
cache. The actual executable ACP tests cover process framing/lifecycle; the
successful composed network round trip uses the real host acquisition and stdio
owners with explicitly injected HTTP fixtures, not a live Gateway endpoint.

## Candidate 66d02faa: complete local gate, R1 review rejection

Candidate: `66d02faa881555e98a10ef239cba8f6030b5a2b2`.
Base: `658f3366258cf1207904f9c2a274f32db2bb981b`.

The complete Rust 1.94.1 local gate passed before review: Linux at normal test
concurrency, macOS serial runtime tests, formatting, warnings-denied Clippy,
fresh release helpers, focused picker/skills/MCP/ACP checks, workspace tests,
separate doctests, release smoke, FreeBSD/WASI compilation, pinned drift and
Unicode checks, documentation policy, dependency policy and audit. Linux's full
269-test Python suite and fresh-target release cleanup probe passed without
deadline changes. Its selected Git 2.55.0 supports `--no-lazy-fetch`; the
benchmark metadata-change fixture now changes its timestamp deterministically.

Release-helper SHA-256 values:

- macOS: `ccd7b531b247aaaedd7e8676dabd46c94196212a36b05da12fb96fff2e8a2164`
- Linux: `adc3b33aeee634cabfe76f86d39744aa9c97a7b41fac4ff0480e3016678cfb66`

Three newly spawned local reviewers inspected the full feature diff in isolated
exact-candidate worktrees after that gate: `acp_r1_correctness`,
`acp_r1_lifecycle` and `acp_r1_resources`. These were static direct reviews, not
Bugbot or runtime-test results. A host thread limit delayed the resource track
until the lifecycle track finished; all three reviewed the same immutable SHA.
Coordinator integration review independently identified the lifecycle defect.

Two distinct introduced findings rejected the candidate:

- P1, `ask/production/acp.rs` input admission: retaining a complete request
  rejected by backpressure prevents polling input again. A blocked output frame
  can therefore hide closed stdin indefinitely, preventing native retirement
  and the final output grace. Lifecycle and resource reviewers reported the same
  defect, not two separate findings. The coordinator reproduced it with the exact
  release CLI, 400 compact initialize requests, successfully closed pipe stdin,
  and undrained socket stdout with a 1,024-byte requested send buffer. The process
  remained live after five seconds; owned SIGTERM cleanup exited 143. An ordinary
  pipe-output attempt did not saturate and exited normally.
- P2, `acp_startup/factory.rs` and native `acp/selection/driver.rs`: native root
  preparation canonicalizes an admitted workspace, but session options and
  selection compare its canonical root against the original request spelling.
  Valid ancestor-symlink paths such as macOS `/tmp/project` are rejected. Existing
  composition fixtures canonicalize the request first and mask this mismatch.

No additional resource finding was established. ACP's 1 MiB decode/queue ceiling
does not override the engine's separately configured prompt limit; oversized
engine admission fails before turn acquisition or persistence. Neither legacy
compatibility nor explicitly deferred product categories were review requirements.
No feature push or remote acceptance occurred for this rejected candidate.

Raw evidence remains under `target/agent-gates/acp-connection.aN177x`, with macOS
`66d02faa-*` logs and the Linux copy in `linux-66d02faa.ReE8Eq`. Gate success is
regression evidence only, not delivery acceptance or an M07 performance claim.

### R1 remediation

Source components `977694ee` and `46dc27d7` address the two findings. Workspace
preparation binds the original request spelling to the exact descriptor-validated
primary scope; session options use its canonical identity. Pure constructor and
selection checks reject foreign scopes or substituted requests without reopening
paths. Native tests cover identity substitution and alias retargeting; composed
CLI coverage exercises aliased new/load/resume with inert persisted history.

The existing input worker retains one original FIFO descriptor alias and observes
requested peer hangup independently while read credit is paused. The transport
uses that observation only behind a backpressured complete frame, entering the
existing native cleanup and output-grace path. No additional reads, workers,
status-flag changes or deadline changes are introduced. Non-pipe sources retain
normal demand-gated EOF. Regressions cover direct/helper pipes, unread-byte custody,
regular files, native settlement before output grace, and a production CLI with
deterministically saturated output. Component formatting and diff checks passed;
compilation, runtime validation and fresh product reviews remain separate gates.

Candidate `ded809bb` passed both fresh release builds, formatting, Clippy,
portability and policy checks. Focused Linux input tests passed (45 plus three
existing helper fixtures ignored), as did 143 native ACP tests. Eight CLI ACP
tests passed, including aliased new/load/resume; the new disconnect test reached
actual native/input settlement but failed its exact output-grace assertion at
3.001 seconds versus 3 seconds. Tokio's pinned timer source rounds deadlines up
to millisecond ticks; pausing its already-running clock retained a fractional
offset. The regression now transfers settled state to a fresh paused runtime,
retaining channel ownership and the unchanged exact three-second assertion.
No production timeout was changed. This interrupted gate is not acceptance.

Candidate `c290db42` passed the complete Linux gate, including 269 Python tests,
both release builds, Clippy, portability and policy checks. The macOS focused
input gate failed both direct/helper pipe-disconnect regressions: with unread
bytes and a closed writer, neither observer woke within its existing deadline.
A host `select.poll` probe reproduced the cause: an empty requested event mask
reported nothing, while `POLLIN` reported `POLLIN | POLLHUP` without consuming
bytes. Apple's [XNU poll implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/sys_generic.c)
only installs a read filter when a read event is requested. The observer now
subscribes to read readiness but still interprets only hangup and errors. A
synchronous regression checks that ordinary readiness neither wakes nor settles
the observation, grants credit, consumes bytes or changes descriptor flags.
Existing direct/helper regressions retain their original deadlines and actual
worker cleanup assertions. This is a platform correctness fix, not a legacy
protocol path; this rejected candidate was not pushed.

Candidate `70b8fce3` passed the complete macOS gate, including all focused checks,
3,544 native unit tests, integration suites, doctests and fresh-release smoke.
Linux's focused checks also passed. Its full workspace run stopped in
`bounded_transport_times_out_while_queued_without_starting_an_effect`: both calls
timed out, but the fixture observed two underlying transport constructions rather
than one. The fixture and production web-fetch code are unchanged from the feature
base. Each first poll establishes its own absolute deadline, so a later-started
queued call may legitimately acquire capacity before its own deadline after the
first call expires. A real-time join does not establish simultaneous deadlines.
Independent read-only inspection confirmed deadline checks before transport
construction and retained capacity through completion; it found no production
deadline-order defect. The fixture now uses a paused clock and tests both expiry
polling orders with equal start times. A second case explicitly staggers starts
and proves admission within the remaining original budget. Both keep the 30 ms
timeout and assert exact clock, construction, active, peak and drop counts.
No production deadline or implementation changed. This interrupted Linux gate
is not acceptance; no feature push or new formal review occurred for this candidate.

## Candidate 0a3f5d2d: output-wake diagnosis

Candidate `0a3f5d2d15e16d53705ff9c7d1a9b1a400a92b89` passed the complete Linux
gate, including the corrected web-fetch cases and all 269 Python tests. macOS
passed focused web-fetch, input and native ACP tests, then stopped in the aliased
workspace new/load/resume composition fixture at its unchanged ten-second
deadline. The original diagnostic did not identify the awaited response or EOF
phase. Four unchanged isolated runs and three unchanged nine-test ACP runs passed;
these are diagnostic observations, not a source fix or complete macOS gate.

Two isolated read-only agents traced fixture/output flow and native selection.
No native selection lost wake or custody defect was established. Output inspection
did establish a separate liveness defect: queueing a new frame did not schedule
the next poll that registers its acknowledgement waiter. If input and native work
stay idle, a write acknowledgement can arrive without waking the transport.
A deterministic regression failed at that missing wake, with no input worker,
timer or incidental request event. The transport now self-wakes after queueing a
frame, as its existing flush and final-output paths already do. The regression
then passed and verified delayed write and flush acknowledgements independently.
This proves that handoff fix, not the cause of the original timed-out run.
No deadline, retry policy or protocol compatibility path changed. Both diagnosis
worktrees were clean and removed; replacement gates and fresh formal product
reviews remain separate from this diagnostic evidence.

## Candidate a1e907fb: initial preparation and network capture

Candidate `a1e907fb245ea9247608ab10e0e8e647962663e4` passed both fresh release
builds, portability/policy checks and the complete Linux gate, including 269
Python tests. macOS passed focused web-fetch, input and native ACP checks but
again hit the composed alias fixture's unchanged ten-second deadline. This time
the backtrace identified response 2, initial `session/new`, before load/resume.
The new output-wake regression and the other nine CLI ACP tests passed. This
rejects the macOS gate; the output-wake fix alone does not resolve the timeout.

An unchanged diagnostic with owned-process sampling passed in 7.30 seconds.
During its initial two-second sample, 158 of 160 preparation-worker samples were
inside unconditional MCP system-DNS capture, through macOS SystemConfiguration
and CoreFoundation bundle-directory enumeration. The selection was empty. This
establishes unnecessary startup work, not the cause of the unsampled timeout.
Independent read-only inspection confirmed that the admitted MCP transport
requirements were not passed to the host factory. The diagnosis checkout was
clean and removed; raw sample and gate logs are retained with local evidence.

Remediation derives a bounded, non-secret transport requirement while decoding
the selection and passes it through new/load/resume preparation. Empty/stdio
selections capture no network inputs; literal HTTP endpoints retain normal TLS
and fresh entropy without system DNS; hostname HTTP retains system capture.
Profile capture, required-peer readiness and fail-closed network admission remain
unchanged. No legacy fallback, fixture-only network suppression or deadline
change is part of this correction. Replacement validation and fresh full-feature
reviews remain required before delivery.

## Candidate dd7b210a: complete local gate and R2 rejection

Candidate `dd7b210a9d697b156ebfd166302e914e9419f07c` passed the complete exact
Rust 1.94.1 Linux/macOS local gate: fresh locked release helpers, Clippy,
portability, dependency/policy checks, focused regressions, workspace tests,
doctests and release smoke. Linux passed 269 Python tests and 3,549 native unit
tests; macOS passed 3,551 native unit tests. The seven transport-aware capture
regressions passed on both platforms. macOS's composed alias fixture passed at
its unchanged ten-second deadline; that result does not prove the cause of the
previous unsampled timeout. One Linux focused web-fetch invocation initially
omitted `--all-features` and ran zero tests; a corrected invocation passed all
15 before macOS runtime started. The zero-test invocation is not acceptance
evidence. Raw logs and helper hashes are retained with local gate evidence.

A fresh correctness/API reviewer used the local direct-review fallback because
the host supplied no Bugbot reviewer. It rejected the candidate with two P2
findings, independently confirmed by the coordinator:

- Native `/model` changed live preferences but never emitted modern
  `config_option_update`, leaving clients' advertised model configuration stale.
- Production `session/list` compared the client's original workspace spelling
  literally with canonical stored metadata, so an ancestor alias accepted by
  new/load/resume could omit those same sessions from a scoped list.

Fresh lifecycle/platform and resources/performance tracks did not start because
of the host's agent-thread limit; neither is a zero-finding result. The rejected
candidate was not pushed. All three allocated review worktrees were clean and
removed, including the two unused checkouts.

The replacement work adds original-principal, bounded complete configuration
updates before command results and prompt completion, reflecting actual live
preferences independently of persistence outcome. Alias resolution is explicit
native work on the existing owned list worker, with descriptive literal filters
for deleted workspaces and no host preparation or workspace creation. Under the
user's modern-only policy, the superseded Session Modes wire method and duplicate
projection are also removed; this scope alignment is not a third review finding.
Native `ask`/`auto`/`yolo` remain available through `session/set_config_option`.
The replacement requires the full exact gate and three new reviewers; prior
gate success or unstarted review tracks cannot accept changed source.

## Candidate f30e6616: newly published dependency advisory

Candidate `f30e6616c57d5da80a769996429056f31c64b9ea` integrated both R2 fixes,
modern-only wire cleanup and their regression tests/contracts. Exact Rust 1.94.1
formatting and warnings-denied full Clippy passed on macOS and Linux. The isolated
component worktree was verified integrated, clean and removed. Before runtime
tests began, cargo-deny rejected locked rustls 0.23.39 for
[RUSTSEC-2026-0285](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc),
published September 14, 2026. The upstream advisory names 0.23.45 as patched.
The coordinator interrupted only the identified owned release-build jobs; both
pipelines settled with status 130. These interrupted builds are not acceptance.
No runtime tests, formal R3 reviews or remote pushes occurred for this candidate.
The correction raises the workspace rustls minimum and lockfile to 0.23.45;
there is no advisory exception or policy relaxation. The replacement requires
fresh release helpers and the complete gate before three new product reviewers.

## Candidate 89620b56: alias fixture correction

Candidate `89620b5674510d73d7543ac9c22531f390d3dc42` passed exact dependency
policy/audit, compatibility, Unicode, documentation and Linux build checks.
Linux's fresh release helper and workspace/all-feature test compilation finished;
macOS passed full Clippy but had not finished its gate. Three filesystem-only
alias tests then ran on Linux without process-heavy fixtures: two passed, and
the combined-predicate case returned no rows. Its records had neither titles nor
canonical user previews, but the query searched for their ID prefix. The catalog
intentionally does not search IDs, so this was an invalid fixture expectation,
not an alias resolver defect. The correction supplies canonical user messages
for the existing preview-search predicate. Production filtering, query bounds,
assertions and deadlines remain unchanged. No full runtime gate, formal R3 review
or push accepted this candidate; the corrected exact commit requires the full
replacement gate. An in-flight build of unchanged release code can populate
the compilation cache but cannot substitute for that new exact-commit gate.

## Candidate c8e12295: local acceptance and unavailable R3 reviewers

Candidate `c8e122950bc080d7a26eaeeff04a28b40afdc33f` passed the complete exact
Rust 1.94.1 replacement gate with freshly selected locked release helpers.
Linux ran its normal test concurrency: 3,555 native unit tests passed, with 11
existing ignored fixtures, and all workspace integrations, doctests and 269
repository Python tests passed. macOS ran serially after Linux settled: 3,557
native unit tests passed, with 12 existing ignored fixtures, followed by all
workspace integrations, separate doctests and the exact CI release smoke.
Both platforms passed focused ACP, alias-listing, input, picker and skills
regressions. Formatting, full warnings-denied Clippy, dependency policy/audit,
pinned drift/Unicode, portable compilation and documentation checks passed.
The exact-commit clean guards passed; logs and helper hashes are retained locally.

After that gate, two direct attempts to create a fresh correctness/API reviewer
failed with `agent thread limit reached`. A nonauthor preparation agent attempted
one fresh child reviewer through the supported nested-spawn API and received the
same error. No new reviewer was created and no formal R3 review took place;
the coordinator's bounded source inspection is not a substitute. The unused
isolated review checkout was verified at the exact candidate, clean, and removed.
No source fix, relaxed gate, remote push or delivery followed this capacity error.

## Candidate c8e12295: restored R3 review and rejection

After capacity was restored, three new isolated read-only direct-review agents
examined exact `c8e122950bc080d7a26eaeeff04a28b40afdc33f` against delivered
main `658f3366258cf1207904f9c2a274f32db2bb981b`. Correctness/API and
performance/resources each reported zero actionable introduced findings.
Lifecycle/platform reported two P2 findings, rejecting the whole candidate:

- Cancelling an ACP prompt during native resource preparation settled as the
  typed resource cancellation error, but output mapped it to JSON-RPC `-32603`
  instead of the required cancelled stop reason. The related EOF drain also
  classified this expected cancellation as a native failure.
- With a selectable frame awaiting flush, one chunk containing Tab then an edit
  or Escape could clear and immediately re-arm a skill selection. A queued ACK
  then selected the skill and discarded the remaining input as stale. The
  session picker's deferred Enter had the same input-custody issue.

These were static full-feature reviews, not Bugbot or independent platform test
runs. The coordinator confirmed both paths. Two isolated implementation workers
supplied precise settled-outcome classification with pure tests and persistent
mixed-chunk revocation with actual-driver held-ACK regressions for skills/picker.
The coordinator added protocol-boundary cancellation-before-poll, held-reader
settlement and EOF regressions using the existing test-only native reader seam.
Unrelated errors, real worker custody, editor identity and deadlines remain intact.
Component formatting and diff checks passed; those checks alone do not establish
runtime acceptance. This replacement requires the complete local gate and three
new reviewers before any feature push or delivery.

## Replacement preflight: cancellation fixture lifetime

Integrated `e59727766f9013078ef3c2171fc6306410ce8fb0` first hit a
warnings-denied Clippy test-style error; `7817055d743b6bbc0596778f120112ed5750cfd1`
names the token-usage default explicitly. The latter passed Linux build checks,
both platforms' full Clippy, dependency policy/audit and documentation/drift checks.
Its Linux cancellation preflight passed EOF but timed out both explicit-cancel
fixtures at the unchanged 20-second bound. No full runtime or review acceptance
followed. A retained reviewer was reused for read-only diagnosis, not fresh review.

Both failing fixtures retained an `Arc<NativeConversationRuntime>` while awaiting
shutdown. That handle retains the core session's host resource; the resource's
drop closes the terminal worker scope that ACP cleanup awaits. The passing EOF
fixture retained no such handle. The correction drops the two observation handles
after their assertions and before shutdown. It changes neither production
cancellation nor deadlines. The changed candidate must rerun focused tests and
the complete replacement gate; an older in-flight build is cache preparation,
not evidence for the changed commit.

## Candidate eb57d916: pipe-observation fixture correction

Candidate `eb57d9165b1dc054de40262be034eb6e2262bf25` passed both exact platform
build gates, fresh release checks, portable compilation and policy checks.
Linux passed six cancellation/outcome tests and fifteen neighboring skills/picker
acknowledgement tests. An initial neighboring-test filter matched zero tests;
that invocation was not acceptance and was replaced with the correct module
filters and positive test-count checks. The full runtime pipeline then passed
native ACP, CLI ACP, catalog and HTTP focused checks but stopped in input tests:
45 passed, three existing helper entrypoints were ignored, and
`pipe_peer_readiness_is_not_disconnect_or_read_credit` failed its immediate
notification assertion. Workspace, Python and macOS runtime gates did not run.

An isolated implementation worker found the fixture's one-observation assumption
exceeded the production observation contract. A local writer drop does not prove
all kernel writers have closed, and the observer deliberately retries interrupted
polls. The failed syscall and any temporary inherited writer were not captured;
no specific concurrency cause is claimed. The test-only correction retains an
explicit duplicate writer to assert non-closure, then releases it and observes
actual closure using the existing five-second helper. No-credit, unread-byte,
descriptor-flag and wake assertions remain, with no production or deadline change.
Component formatting and diff checks passed. The replacement requires focused
tests, the complete exact gate and three fresh reviews; no push accepted this
candidate. Its logs remain retained, not overwritten by the replacement.

## Candidate 7229b042: runtime failures and acquisition sampling

Candidate `7229b042fd233b402144254e01bc0f8683c04f14` integrated the pipe fixture
correction. Both platform build gates, fresh release helpers, portable lint and
policy checks passed. Linux focused suites passed, but the first workspace run
failed the tmux shared-bootstrap matrix at its commandless write deadline:
3,560 native tests passed, one failed and 11 existing helper fixtures were ignored.
The recorded pending stage was `write-paste`, with empty command input/output.
An isolated matrix passed in 2.86 seconds; a narrower package-only concurrent
native diagnostic passed 3,012 tests in 45.18 seconds. Read-only driver inspection
found no actionable defect: repeated writes drive the pending operation, and
the shared command deadline is checked before output/exit observation. Neither
the logs nor passing diagnostics establish the initial timeout's cause.

One unchanged full Linux replacement passed all focused suites, 3,561 native
unit tests with 11 existing ignores, workspace integrations, doctests and 269
Python tests. The native unit suite took 48.79 seconds. The original failed log
is retained separately; this retry is not a source-fix claim.

The serial macOS gate then failed
`aliased_workspace_new_load_resume_list_preserve_canonical_checkpoint` while
awaiting a response: 548 CLI unit tests passed, one failed and six existing helper
fixtures were ignored in 116.96 seconds. An isolated case passed in 7.11 seconds
and the paired composition tests in 3.11 seconds. Read-only native/CLI diagnosis
found no established lost wake or retained-owner shutdown cycle. One unchanged
full macOS replacement reproduced the failure (548/one/six in 98.01 seconds);
its backtrace identifies response 2, initial `session/new`, before alias
reselection. Neither run reached the native workspace, doctest or release-smoke
stages. These failures reject local acceptance despite the Linux pass.

Bounded OS sampling of the existing CLI test executable followed, without code,
deadline or scheduling changes. A full CLI diagnostic passed 549 tests with six
existing ignores in 118.57 seconds. Three isolated captured-output samples passed
in 7.19, 2.95 and 3.28 seconds. Sample 1 attributes 1,671 of 1,683 acquisition-worker
samples to `WebFetchTool::new` → system nameserver discovery → macOS system
configuration/CoreFoundation bundle-directory enumeration while the client waits
for initial session creation. Samples 2 and 3 show the same acquisition path.
These are passing traces, not samples of either failed event; they establish
unnecessary eager startup work, not its exclusive responsibility for the failures.

The scoped remediation defers only web-fetch resolver configuration for complete
hosts onto their existing owned worker scope. The first admitted hostname fetch
shares one result, including failure; literal IPs bypass discovery. TLS and
query-ID seed setup, DNS destination policy and invocation deadlines remain.
Cancelled waiters do not restart discovery or abandon an already-started worker.
Standalone transports without a supplied scope retain synchronous capture.
Deterministic lifecycle tests and durable contracts accompany this change; the
complete replacement gate and three fresh reviews remain required before push.

## Candidate 085c24f2: local acceptance and R4 rejection

Candidate `085c24f28583e5b17924a2830801aca604dc34d5` integrated deferred resolver
capture and five deterministic regressions. The complete required Rust 1.94.1
local gate passed: Linux native units 3,566/zero failures/11 existing ignores,
macOS native units 3,568/zero/12, CLI units 549/zero/six on each platform,
focused suites, workspace integrations, doctests, 269 Python tests, release
smoke, pinned drift/generation checks, documentation, dependency policy/audit
and required FreeBSD/WASI portability lint. Native unit durations were 52.53
seconds on Linux and 921.64 seconds on macOS; no M07 performance claim follows.
An additional FreeBSD all-features compilation probe failed in `aws-lc-sys`
because the cross-compiler lacked FreeBSD C headers. That supplementary probe
did not establish Rust compatibility and is not reported as passing.

Three fresh read-only R4 agents reviewed the full branch against
`658f3366258cf1207904f9c2a274f32db2bb981b` in isolated exact-SHA worktrees.
Correctness/API reported zero actionable findings. Lifecycle/platform found one
P2, independently confirmed by performance/resources on follow-up, superseding
that track's initial zero-finding report. This is one shared finding, not two.

`acp/selection/cleanup.rs` started a new unscoped worker to join a retired host.
Capacity or thread-admission failure could return an incomplete receipt whose
worker completions were retained but never observed again. Selection closure
ignored them; CLI finalization joined only its separate input/factory scopes and
could exit before retired host cleanup settled. This rejects the candidate despite
the passing gate. Replace observer-worker admission with direct asynchronous
completion, test actual settlement and failure paths, and repeat the complete
gate with three fresh reviewers. No feature push or remote acceptance occurred.

The repair adds infallible asynchronous observation of the existing scope's
closed-and-settled predicate, without a new worker, polling timer or admission
budget. Independent notification registrations retain actual collector/TLS/reap
completion and contain caller-waker panics. Blocking self-wait rejection remains.
ACP retirement awaits this observation before retaining any cleanup receipt;
closure also checks retained completion handles. A thread-local, test-only
unscoped-admission rejection exercises close and EOF while a real host worker
remains held, without saturating the process-wide collector. Primitive waiter
and wake/drop regressions accompany it. These component changes require the
replacement gate; they are not a new acceptance record.

## Candidate f4ce3682: replacement gate, R5 and feature acceptance

Candidate `f4ce36823885b9a73c7ee9b350047d2a6f721410` contains the observer-free
shutdown repair, twelve primitive completion regressions and the composed
close/EOF regression under rejected unscoped admission. Its final follow-up
renames test notification counters and applies lint-required punctuation;
it does not change the repair's behavior.

The complete required Rust 1.94.1 local gate passed on both platforms. Linux
native units passed 3,579 tests with zero failures and 11 existing ignores in
53.04 seconds; macOS passed 3,581 with zero failures and 12 existing ignores in
908.94 seconds. CLI units passed 549 with zero failures and six existing ignores
on each platform (2.63 seconds Linux, 47.80 seconds macOS). Both platforms passed
the twelve completion regressions, close/EOF regression and 157 focused ACP
native tests, followed by the full workspace integrations and doctests.

Formatting, warnings-denied workspace Clippy, fresh locked release helpers,
required FreeBSD/WASI lint, dependency policy/audit, pinned drift and Unicode
checks, documentation policy, 269 Python tests and fresh-release smoke passed.
Supplementary standalone web-fetch compilation was not a strict-lint claim.
The earlier supplementary FreeBSD all-features failure remains recorded above;
the required portability gate passed without changing that probe's scope.

Three fresh local adversarial reviewers inspected the complete branch against
`658f3366258cf1207904f9c2a274f32db2bb981b` in isolated exact-SHA worktrees:

| Track | Fresh reviewer | Actionable introduced findings |
| --- | --- | ---: |
| Correctness/API | `acp_r5_correctness` | 0 |
| Lifecycle/platform | `acp_r5_lifecycle` | 0 |
| Performance/resources | `acp_r5_resources` | 0 |

Each track inspected related contracts, callers and regression sources without
relying on prior zero-finding reviews. These were independent static local
reviews, not Bugbot, dynamic race reproduction or independent reruns of the
coordinator's gates. The resources reviewer started after a completed track
freed capacity; no repair author was reused. All review worktrees were checked
clean at the candidate and removed. No M07 performance claim follows.

Feature CI `34915965303` and Benchmark evidence `34915965295` succeeded for the
exact candidate. All ten expected CI jobs passed, including all four native
Linux/macOS architecture jobs; all four Benchmark jobs passed. The benchmark
run retained matching, nonempty, unexpired artifacts `10376106781` (bootstrap)
and `10376412517` (pinned upstream), both expiring on 2026-12-14. Their names and
workflow-run metadata matched the full candidate SHA. Main was then advanced
from the reviewed base to this candidate by fast-forward without force. This
records local, review and feature-branch acceptance only; the implementation
plan owns subsequent main acceptance and delivery state.

## Exact-main catalog fixture failure and replacement

Main CI `34918146254` rejected `f4ce36823885b9a73c7ee9b350047d2a6f721410`:
the Intel macOS native job failed
`timeout_fixture_does_not_accept_a_malformed_complete_head_as_peer_close` in
`crates/machine-god-native/tests/ai_gateway_model_catalog_http.rs`. Its client
`shutdown(Shutdown::Both).unwrap()` returned OS error 57, `NotConnected`;
the integration suite reported 19 passed and one failed. The other substantive
CI jobs passed; the aggregate correctly failed. Main Benchmark `34918146228`
passed and retained nonempty, unexpired exact-main bootstrap `10377067210` and
upstream `10377262031` artifacts. Benchmark success does not accept failed CI.

The fixture file was unchanged by the ACP branch and last changed in
`ad346115db8ec56bf0e27f2fbff1495c9bc1ff81`. Complete malformed-head parsing
returns `Other("malformed request header")` immediately and drops the accepted
socket, racing the client's redundant shutdown. The isolated repair component
`7fe005b8b33151f5514528f7d80d10802e3ac1c9` joins the server while the peer is
still open, then drops the peer and asserts both the error kind and exact
diagnostic. Rejection therefore cannot pass as peer EOF, timeout or another
error. Actual worker joining is retained; deadlines and production code are
unchanged. Adjacent partial/empty-head cases require peer closure before their
worker can finish and are unchanged. Component formatting and diff checks
passed; this record does not assert replacement runtime or remote acceptance.
