# Milestone 03 terminal `start` review history

This is the compact historical review record for the background-start addition
to [`../terminal.md`](../terminal.md). Current delivery, workflow, and next-gate
status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

The slice begins from exact delivered base
`61703e91d318f24439af1507f69353f8160291d3`. It adds only bounded,
noninteractive `action: "start"` through the delivered background supervisor.
Persistent handles, read/write/wait/control actions, interactive terminals, and
restart-safe control remain outside this slice.

## Rejected candidates and remediations

Every finding rejected its complete candidate. After each remediation, the
complete exact Rust 1.94.1 replacement gate ran again and three fresh review
tracks restarted without reusing an earlier green result.

| Cycle | Exact candidate | Findings and replacement |
| ---: | --- | --- |
| 19 | `c4b2b41407236f95b6047792a975e8d6bb4aeee0` | Permission identity was not retained as the workspace-relative cwd across workspace moves, and eager reference-host supervisor construction exercised state-root authority before `start`. Exact remediations `9353672af0b86e7410fd4167ca1d060efe261308` and `29f166f1d1cae40d1d83dc9b6d6d891cfa3030d3` bind permission to the retained workspace and initialize the supervisor lazily. |
| 20 | `0befe154b827f75599cedb230e0d584a0b6acf92` | The combined absolute cwd bound was not rejected before permission, and worker resources could be admitted as partial cohorts while public or otherwise unsafe background state roots were insufficiently rejected. Exact remediations `9e3784143474348aae336f6070fa4df46e166366` and `ccbc96fd14006fcb4d227cd41fcdbfaf10e724b2` move the bound into preflight, make direct and lazy cohort reservation atomic, and validate the state root read-only before supervisor effects. |
| 21 | `01a9729abafb1ae3605881071f8fe167819c5686` | Correctness found that the core API document described all private non-Linux terminal behavior as unsupported despite macOS background start. Lifecycle found that the security document called the permission cwd absolute even though authorization deliberately retains the workspace-relative cwd and derives the private canonical absolute cwd only after approval. Exact remediation `dd398320a7e225c1e64d414ed1260d465ddcd1a3` aligns both contracts. Performance reported zero findings. |
| 22 | `502d06375731d91abe9afb4e6fbb19f432660a52` | Correctness found that the revised core API still called background start universally unsupported outside Linux/macOS, contradicting the public Unix injected `TerminalBackgroundStarter` seam. Exact remediation `b8d255b0c1bd8dba42d92d7d843b98964daf4096` limits the platform statement to production/reference-host composition and preserves the injected seam. Lifecycle and performance reported zero findings. |
| 23 | `8e09477f64375d1e8f83b7fb8c43cb7adf241b4c` | All three review tracks reported zero findings, and review seal `c0c915fd6fc9b7142a0b76fc6e2b1c5cd4803b91` passed exact feature CI and Benchmark evidence. After the non-force fast-forward, exact-main Benchmark passed with both artifacts, but exact-main CI rejected the candidate: Linux x86 native testing observed `terminal_busy` when the deadline wake callback still legitimately retained its admission slot. The green Benchmark run did not override the failed required CI gate. |
| 24 | `4d427c071ca997545c439d47f5476eb478e139d6` | A yield-only capacity-recovery loop fixed the scheduler race but performance review rejected its unbounded rapid request reconstruction and ability to mask slow reclamation under the outer two-second assertion. Exact replacement `7433cc0bcb663a96e1bd0cc6efff320391df24ec` adds a 256-attempt ceiling, passive one-millisecond backoff, exact busy classification, and explicit executor admission/drop accounting. |

## Accepted candidate

Three fresh cycle-25 adversarial tracks reviewed exact replacement
`7433cc0bcb663a96e1bd0cc6efff320391df24ec`, tree
`24b9bf4ba516f3d19ef3621e34c0dd077484c81e`, against the failed exact-main
candidate. All three reported **zero findings**:

- correctness, API, compatibility, schema, permissions, error mapping, and
  durable contracts;
- cancellation, launch and persistence ordering, retained cwd identity,
  cleanup, platform adapters, state-root validation, and lifecycle races; and
- CPU, memory, file-descriptor, process, blocking-worker, admission, I/O, and
  amplification bounds plus performance-claim accuracy.

The correctness reviewer explicitly matched the retryable `terminal_busy`
classification, terminal schema, permission identity, API seam, error mapping,
and [`../terminal.md`](../terminal.md) callback-tail ownership contract. The
lifecycle reviewer rechecked deadline activity ownership, cancellation and lazy
initialization precedence, irreversible-release behavior, and Linux/macOS
platform boundaries. The performance reviewer verified that a 257th busy
result fails, each retry sleeps passively, busy attempts do not enter the
executor, exact two-call/two-drop accounting remains mandatory, and roughly
1.9 seconds of failed reclamation cannot hide under the outer assertion.

Focused exact-1.94.1 reviewer evidence included 100 consecutive correctness
executions, the blocked callback-tail and lazy-background lifecycle tests, and
500 consecutive performance executions. The latter completed in 2.66 seconds
with 0.25 seconds user CPU and 0.37 seconds system CPU.

All three detached review worktrees remained clean at the exact candidate and
were removed and pruned after review.

## Exact local evidence

The final replacement at
`7433cc0bcb663a96e1bd0cc6efff320391df24ec` passed the complete local gate under
exact Rust and Cargo 1.94.1 without fallback:

- `cargo fmt --all -- --check`;
- warnings-denied workspace, all-target, all-feature Clippy;
- every workspace test and doctest, including 607 native unit tests, 41
  terminal tests, five terminal-engine tests, 43 background-process tests, and
  19 reference-host tests;
- all 87 repository Python tests selected by `tests/test_*.py`;
- documentation policy with 126 Markdown files, 386 fence markers, 447
  relative links, 124 unique targets, and zero errors;
- pinned-fx compatibility regeneration against the retained upstream checkout;
- dependency policy and vulnerability audit, with 1,239 cached advisories and
  211 lockfile dependencies scanned; the established duplicate dependency
  warnings remained allowlisted;
- warnings-denied FreeBSD and WASI Clippy coverage for their supported and
  explicitly unsupported native surfaces;
- clean diff checks and no added unsafe Rust; and
- a fresh locked release binary exercising exact version, help, canonical
  status, and empty background JSON from an isolated workspace without creating
  ambient config, state, or home roots.

This evidence is regression and delivery evidence only. It does not promote a
performance comparison or claim broader pinned-fx equivalence.

## Remote acceptance boundary

This review seal records completed local and adversarial replacement evidence.
Delivery still requires exact feature-branch CI and artifact-producing
Benchmark success, a non-force fast-forward of `main`, and exact-main CI and
artifact-producing Benchmark success. Those live workflow results belong only
in the implementation plan.
