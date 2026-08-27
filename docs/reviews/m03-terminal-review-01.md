# Milestone 03 native `terminal` review ledger

Status: **IN PROGRESS — CONTRACT FROZEN**

Slice 34 begins from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2`. Its normative boundary is
[`../terminal.md`](../terminal.md). It implements only bounded foreground
`exec`; every other pinned-fx terminal action remains deferred.

## Initial composition

The review-exempt frozen-contract checkpoint is exact `79ae1b7`. Exact isolated
host component `ecf3e78c4abc70bb4f3329a6f8dffa9237ff130b`, production precursor
`ea216db80cb38601da268d84dd05b962c802c5df`, independent-evidence component
`785b193e2735401ffd3966ed6ef84db637891d59`, and lifecycle remediation
`64a48504275afc0e6989cee03eeeeb8174267225` are followed by configurable-system
component `13dfd28b9f49424c5272545b7ce6ceb75f445b92` and expanded Linux-evidence
component `ebc631e136bd052a41ff104534b5ecc0d5613d93`. They compose through local
exact `59b069a84e7d4dc4d76ac65520b9045603cae8af`.

Early independent evidence found that an inline reentrant Waker makes the
literal no-thread-tail wording impossible: a worker cannot join itself from its
own callback. The accepted invariant is stronger where it matters. Every child,
original process-group member, pipe, reader, descriptor, and capacity permit is
cleaned before publication; only the resource-free notification callback tail
may self-detach. Non-self paths join. Production also remediated pre-spawn
deadline, reader-join, escaped-writer, exit/signal-range, duration, pending-
executor deadline, cancellation-precedence, and portability gaps before any
formal candidate.

Exact focused evidence is green for four private limit/deadline/outcome tests,
nineteen portable contract/lifecycle tests, two engine permission/durability
tests, one unsupported-platform test, two workspace clone/failure tests, and
eight reference-host tests. Seven real Linux process tests compile and await
Linux execution.

## Cycle 1 candidate gate

Exact precursor `ec8bbc97c23022db3f1884cac083c3cbe4460825`, tree
`afe5445ffc0bef629db82ce8a678558c470a675d`, passed the four required Rust gates
but failed the first supplemental Python run in three stale manifest assertions.
They still expected `sha2` to be optional and HTTP-feature-scoped even though
terminal now requires it for environment-snapshot identity in every native
feature topology. Exact test/documentation remediation
`80c6ee0d09c7fe5feaed96504a2f931aeb131208`, tree
`027c4bc27b90526e172b1306d0ef3700dd39a745`, corrects those assertions without
changing product Rust. Its complete replacement gate is green under exact Rust
and Cargo 1.94.1 without fallback:

- all four required commands pass: formatting, warnings-denied workspace all-
  target/all-feature Clippy, 1,147 listed non-documentation tests, and two
  doctests;
- all 136 Python tests pass with eight expected macOS skips, and regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` is byte-stable;
- exact `cargo-deny` 0.20.2 passes with the three established duplicate
  warnings, while `cargo-audit` 0.22.2 loads 1,226 advisories, scans 211 lockfile
  dependencies, and reports zero vulnerabilities;
- FreeBSD and WASI baseline checks pass, with only the established WASI
  `read_file` warning. Linux terminal test checking and warnings-denied Clippy
  pass. Foreign all-feature Linux compilation stops in `aws-lc-sys` because the
  macOS host lacks Linux C headers, before product Rust; native Linux CI remains
  authoritative;
- documentation integrity covers 89 Markdown files, 312 fence markers, 678
  parsed links, and 515 repository-relative targets with zero missing targets;
- the exact 29-file base diff is +4,484/-192, adds no unsafe Rust, and leaves
  workflows, benchmarks, generated compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes help and status smoke paths.

This evidence makes no product-performance, compatibility-promotion, or fx-
equivalence claim. Formal cycle 1 review remains pending.

## Required composition

Production, independent tests, and maintained documentation are owned in
non-overlapping isolated worktrees. Each component must be committed, verified
clean, integrated, and then removed and pruned. The composed candidate must pass
focused tests, all four exact-1.94.1 required commands, portability and
release-mode evidence before review.

Three fresh read-only adversarial product tracks review one exact immutable
candidate and tree:

1. correctness, public API, schema, capability, and engine integration;
2. native filesystem/process effects, cancellation, lifecycle, and platform
   behavior; and
3. performance, concurrency, memory/output bounds, and resource ownership.

Findings are recorded as blocker/high/medium/low. Confirmed findings are fixed
and the complete gate plus three fresh tracks repeat until each track and the
deduplicated union report `0/0/0/0`. This is ordinary terminal-agent product
review, not a cybersecurity assessment.

## Delivery gate

Only a review-green exact candidate may be pushed as the feature branch. Its
exact feature CI and Benchmark evidence SHA must pass before `main` is
fast-forwarded without force. Exact main CI and Benchmark evidence must then
pass, with the expected exact-SHA artifacts retained. No package or GitHub
release is authorized.

The final record will append exact component commits, candidate/tree, local
evidence, every review report and adjudication, workflow IDs and SHAs,
integration result, and worktree cleanup. Documentation-only result and
delivery seals follow the user's review exemption and do not restart product
review.
