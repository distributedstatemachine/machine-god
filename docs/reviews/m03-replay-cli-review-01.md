# Milestone 03 replay CLI review history

This is the compact historical review record for the contract in
[`../replay-cli.md`](../replay-cli.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh product-review scopes: correctness/API,
lifecycle/platform, and performance/resources. Any finding rejected the whole
candidate and required a complete replacement local gate plus three fresh
reviews.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `111775deb84a513798803b96d0d6b24a6e833bd0` | `810e67c7db161eca2187e3f43610747377112da3` | all three tracks rejected | Blocking FIFO input validation; invalid-byte JSON projection; artificial 16 KiB terminal-feed splits; alternate-screen cursor visibility, DEL, and F5-F7 mismatches; quadratic marker matching and suffix interning. |
| 2 | `96041cba0ca62ec31645390272d6826e6b00ce8c` | `c73675504674ae2a6e7c9dbcc98513398dfd4b86` | resources green; correctness and lifecycle rejected | Whole-frame atomic feed lost bounded mid-frame cancellation; lone C0/C1 controls diverged from pinned fx. |
| 3 | `e67f4b7bf37fb234e8e2eda106035cf2bf7a370b` | `f11c9d0b06ef65a7d74f1ff17c1ba01933497ff1` | lifecycle and resources green; correctness rejected | Short, bad-magic, and truncated-version FXTP headers collapsed into one error category. |
| 4 | `e685dc47fa8a614d0f7d580f3a3622e155b6d42f` | `abbb92de8d905fad2945744c4fda631aa5111f5a` | all three green | None. Three fresh reviewers independently reported zero actionable findings. |

## Finding disposition

The accepted replacement validates regular files without blocking on FIFOs,
preserves arbitrary byte slices in structured output, feeds each frame
atomically while checking cancellation inside bounded 16 KiB chunks, and uses
indexed/cached matching rather than repeated linear scans. Its terminal grid
matches the pinned control, key, alternate-screen, display-width, and header
error behavior covered by the rejected findings.

## Accepted candidate evidence

Cycle 4 passed the complete local gate under exact Rust and Cargo 1.94.1:
focused replay and terminal-grid tests, formatting, warnings-denied workspace
Clippy, workspace and documentation tests, repository Python tests,
documentation and pinned-compatibility checks, dependency policy and
vulnerability audit, supported and explicitly unsupported platform checks, and
fresh release-binary smoke.

The final reviewers covered literal argument grammar, parse-before-authority
behavior, FXTP framing and incomplete-tail handling, exact human/JSON/golden
and frame-artifact bytes, regular-file lifecycle behavior, cancellation,
FreeBSD/WASI surfaces, allocation and output ceilings, and asymptotic marker
and interner costs. Correctness evidence included 1,200 randomized
terminal/control differentials and 250 randomized combined output/artifact
cases against the pinned upstream revision, all byte-for-byte green.

This review seal makes no release or comparative-performance claim. The
implementation plan remains the sole live source for delivery and remote-gate
status.
