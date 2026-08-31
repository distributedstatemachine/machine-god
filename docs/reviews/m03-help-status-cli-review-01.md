# M03 help/status CLI review 01

This historical ledger seals the accepted help/status CLI behavior candidate.
Current delivery state and workflow IDs remain exclusively in
[`docs/implementation-plan.md`](../implementation-plan.md).

## Accepted candidate

- Behavior SHA: `42e2885ea01a6d1813297ae241f628043920aade`
- Feature CI: `33346735499` (`GREEN`)
- Feature Benchmark evidence: `33346735516` (`GREEN`)
- Delivered seal SHA: `90197707dd75bee2010d9bbab8223821166a48a8`
- Delivered-main CI: `33347258058` (`GREEN`)
- Delivered-main Benchmark evidence: `33347258029` (`GREEN`)
- Retained artifacts:
  behavior-candidate and delivered-main bootstrap plus pinned-upstream JSON for
  their respective exact SHAs.

## Delivered behavior

- The thin CLI owns byte-stable global help aliases and bounded human/JSON
  `status` parsing, output, diagnostics, and output-failure behavior.
- Native runtime-status inspection composes bounded configuration, credential,
  workspace, permission, sandbox, session, and build provenance observations
  without creating product roots or performing network effects.
- The pinned-upstream regression harness measures equivalent status help and
  authenticated status JSON workloads while keeping all results
  claim-ineligible until M07 thresholds apply.
- Zig remains an ephemeral checksum-pinned input used only to build pinned fx.
  It is outside the checkout and is neither a machine-god implementation
  language nor a product runtime dependency.
- CI classifies eligible documentation-only changes before heavy jobs. The
  documentation policy and aggregate gates always run; Rust, platform,
  dependency-audit, and benchmark-evidence jobs run only for behavior-affecting
  changes, with classification failures failing closed to the full gate.

## Review history

Review cycles 1 through 17 rejected intermediate candidates. Their remediations
closed status-schema and effect-order drift; evidence provenance and publication
races; bounded cache, process, signal, cursor, archive, and descriptor cleanup;
process-group identity and confirmed reaping; launch-exception precedence; exact
WASI compatibility; and a numeric-descriptor reuse defect. Every rejection
restarted the complete local gate and all three independent review tracks.

Cycle 18 reviewed the exact accepted SHA in three clean isolated worktrees:

| Track | Findings | Evidence |
| --- | ---: | --- |
| Correctness/API | 0 | Post-close interruption with immediate descriptor reuse preserved the replacement, failed the launch closed, and reaped the anchor; 61 provisioning tests passed. |
| Lifecycle/platform | 0 | 60/60 signal and cleanup stress cases plus 30 real supervisor runs restored the inherited mask exactly and kept descriptors stable. |
| Performance/resources/integration | 0 | Real normal/failure probes found bounded wrapper overhead and no persistent process/cache residue; evidence binding, help/status, and CI classification gates passed. |

## Accepted gates

The exact Rust and Cargo 1.94.1 local gate passed formatting, warnings-denied
workspace Clippy, workspace tests, and doctests. The repository Python matrix
passed 253 tests with 14 expected host-platform skips. Three warnings-denied
WASI builds and the exact FreeBSD unsupported-surface Clippy command passed;
pinned compatibility regeneration was byte-stable; documentation policy,
no-added-unsafe, dependency policy, and vulnerability audit passed. A freshly
built release binary passed isolated help and deterministic status smoke, and a
fresh 30-run pinned-upstream collection validated against the behavior SHA.

Remote CI then passed all Linux/macOS x86_64/aarch64, quality, documentation,
FreeBSD, dependency, and aggregate jobs. Both benchmark evidence jobs validated
and uploaded their exact-SHA artifacts before the aggregate Benchmark gate
passed.
