# Milestone 03 terminal `signal` review history

This is historical evidence for the closed five-signal action in
[`terminal`](../terminal.md) and its
[`native supervisor`](../background-supervisor.md). Current delivery and next
gate belong only in the [`implementation plan`](../implementation-plan.md).

## Product review and remediation

The signal implementation completed three fresh review tracks after closing
earlier findings on exact-owner authorization, process identity, cancellation,
descriptor lifetime, and resource bounds. Linux uses identity-checked process
trees and pidfds; macOS uses the original process group protected by its owned
lifecycle. Authority closes before reaping. Cancellation after committed
admission preserves the actual effect without a self-waking poll loop.

Replacement candidates bounded proc reads and descriptor retention, handled
PID namespaces and disappearing tasks, and reevaluated descriptor capacity
after both reductions and increases in the process limit. The obsolete Darwin
FFI bridge was removed. These are regression and delivery claims, not broader
pinned-fx parity or performance claims.

## CI fixture correction

| Candidate | Evidence and remediation |
| --- | --- |
| `e2dca08` | Feature CI and Benchmark passed, but exact main CI exposed a signal-snapshot fixture that scanned unrelated descendants of the parallel test harness. The correction gives the fixture its own process group and known child; production topology validation is unchanged. |
| `c048397` | Review found late cleanup ownership, incomplete adopted-child reaping, and insufficient independent snapshot/descriptor assertions. The replacement installs ownership immediately, reaps only the fixture group, and verifies exact membership and pidfd identity. |
| `72df641` | Review found an unbounded blocking readiness read and a redundant numeric-child kill. The replacement bounds nonblocking readiness by time and bytes and kills only the still-owned process group before reaping. |
| `8545ea4` | Three fresh correctness, lifecycle/platform, and performance/resource reviewers reported zero findings on the correction. No production behavior changed in this correction series. |

The accepted correction is
`8545ea48623a40d412bce4f707c70a6be991d108`. Focused Linux repetition passed
100/100; background unit and integration suites passed 76/76 and 106/106.
Exact Rust 1.94.1 formatting, warnings-denied workspace Clippy, workspace tests,
doctests, dependency policy/audit, repository Python checks, Linux container
checks, FreeBSD/WASI checks, and freshly built release-binary smoke passed.
All three fresh review worktrees were clean and removed.

Feature CI `33948779914` and Benchmark `33948779916` passed for that exact
SHA, retaining both unexpired bootstrap and upstream benchmark artifacts.
Main CI `33949137350` and Benchmark `33949136988` then passed for the same
SHA with both exact-SHA artifacts retained. The correction branch was
fast-forwarded without force; all auxiliary worktrees were clean and removed.
