# Milestone 05 `subagent` review history

This is the compact historical review record for the contract in
[`../subagent.md`](../subagent.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`ba52dbfeb7d05b67ee314934905abee7e0c35ffc`, tree
`b31b79e92acebef4c18cff921038f529a3c88c68`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict create-shape decoding, NUL and byte bounds, fixed error taxonomy, structural context identities, canonical preparation, typed execution, pinned behavior, host composition, testkit, and durable-contract review. |
| Lifecycle, platform, and effects | Green | Zero findings after inert-before-poll authority and fixture review, effect-free preparation, same-poll cancellation precedence, wake/drop and permit release, fail-fast admission, inert native defaults, injected constructors, macOS smoke paths, FreeBSD, and WASI review. |
| Performance and resources | Green | Zero findings after raw and serialized JSON bounds, escaping amplification, iterative rejected-value destruction, output bounds, allocation passes, global/per-parent contention, mutex/static state, cancellation/wakers, and testkit capacity review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `0ffa79f` | The scripted authority recorded and consumed before first poll; input resource failures used the execution error class; security omitted structural context identities; and preparation documentation claimed a typed request too early. | `5a03a16` moved authority work into the future, separated input and execution resource failures, and corrected both durable contracts. |
| `5a03a16` | The core API omitted structural context identities, while core and native review fixtures still mutated state before their returned futures were polled. | `c6806e1` documented the identities as non-authoritative and made every fixture future inert with direct unpolled regressions. |
| `c6806e1` | The testkit overview overpromised string error codes for the kind-only subagent boundary, and the input contract omitted its NUL exclusion. | `ba52dbf` documented the exact `Failed`/`ResourceLimit` mappings and the no-NUL input rule. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for each replacement.
All isolated gate and review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33440115994` and Benchmark
evidence `33440115963`, then exact-main CI `33440894118` and Benchmark evidence
`33440894154`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.

## Complete managed-agent feature: first reviewed candidate

Candidate `437a19b06e862b973716b6f344ef052a03185fc4` was reviewed against
`6d6364c6ee3b1540505749b10b2527df39ddf5be` after its complete local gate.
This is a rejected product candidate, not a delivery or performance claim.

Exact Rust 1.94.1 Linux and macOS formatting, warnings-denied workspace Clippy,
fresh locked release builds, workspace and doc tests passed. Linux ran with
default concurrency under unprivileged UID 10001 and a reaping init; macOS
process tests ran serially. Focused production-helper input, managed/MCP PTY and
agent-driver tests passed on both platforms. Repository Python tests (269),
documentation policy, upstream/Unicode drift, dependency deny/audit, FreeBSD/WASI
checks and both Apple ABI compilations plus the native arm64 ABI probe passed.

Logs remain under `/tmp/mg-managed-implementation.V0ZGg1/`, notably
`managed-full-runtime-linux-437a19b0-r2.log`,
`managed-full-runtime-macos-437a19b0.log`, and
`managed-focused-macos-437a19b0.log`. The initial Linux invocation failed before
tests because the container lacked `rg`; the corrected diagnostic used `grep`.
The initial audit invocation omitted its `audit` subcommand and was corrected.
Both failed invocations remain retained; neither is a product-test failure.

The preceding macOS release-PTY failure on `e00da357` was traced to XNU setting
the shared TTY descriptor's `FWASWRITTEN` bookkeeping bit on its first output
write. Candidate `437a19b0` ignores only that macOS bit when comparing status
flags; all other bits remain exact. New regressions cover every status bit and
the real first-write/reopen sequence. Diagnostic logs and rejected runtime
evidence remain retained; temporary diagnostic source was removed.

Three fresh general review agents inspected the entire feature, read-only,
without executing tests or performance measurements:

| Track | Reviewer | Result |
| --- | --- | --- |
| Correctness/API | `m65_r1_correctness` | P2: production never writes the typed events selected by event inspection; receipt sequences identify journal pages without public events. |
| Lifecycle/platform | `m65_r1_lifecycle` | P1: abandoned shared MCP refresh can retain a private peer in the admission cohort while cleanup waits for that cohort. P2: ordinary foreground preparation/enrollment drops candidate custody and its reservation after cleanup failure instead of fencing. |
| Performance/resources | `m65_r1_resources` | No concrete introduced findings; reviewed budgets, scheduling/dependencies, notices, paging, undo, blocked output and weak ownership. |

These findings reject the candidate. Passing local tests did not establish
coverage of the identified scenarios. No remote delivery was attempted for it.

The remediation combines isolated event-history and foreground-custody work with
principal-owned abandoned-admission settlement. Event mutations and receipts now
share durable typed evidence, including truthful suppressed/duplicate milestone
records. Ordinary foreground failures use the retained settlement/fence path;
the public opening failure carries any untransferred startup owner. MCP admission
settlement drives original preparation without closing a persistent child's
publication, and excludes idle or already-transferred turn cohorts. Added
regressions cover command-level event paging/recovery, exact worker custody,
cleanup timeouts, public failure ownership, and non-emitting milestone rendering.
These implementation changes still require replacement local gates and reviews;
the rejected candidate's passing tests do not verify them.

Coordinator inspection of `7b04dcf2` found that the initial repair inferred
abandoned preparation from outstanding worker tickets and could miss a retained
refresh before worker admission. Both platform Clippy checks passed, but the
subsequent test builds were intentionally interrupted before runtime tests.
The replacement consumes explicit preparation custody once per admission,
including worker-free cancellation, without affecting later idle controls.
