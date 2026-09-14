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
