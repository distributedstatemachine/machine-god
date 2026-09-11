# Complete skills CLI review history

This is historical evidence for the complete skills CLI feature, not a live
delivery dashboard. Current phase and gates belong only in the
[implementation plan](../implementation-plan.md); durable behavior belongs in
the [skills CLI contract](../skills-cli.md).

## Initial integration rejection

Candidate `c83fb6b11b9d4f1c0a9f7113f956dd6a2a054cdd` failed two composed
terminal scenarios: affirmative skill invocation and exact duplicate-picker
invocation. Both reached a provider failure because the Gateway codec still
required exactly one user text block while core correctly appended a separate
provider-only advisory block. Command management scenarios passed.

Repair `d553ac93` updated the envelope, history prepass and user encoding to
preserve bounded ordered text parts. Five new regressions joined the existing
codec/engine suite; all 60 tests passed. The failing terminal scenarios were
added to the focused `skills_` filter and subsequently passed with a fresh
release helper. Canonical user messages remain unchanged.

## Reviewed candidate and local evidence

- Candidate: `7fed6d382068ae5e482183f02e780746092fa81a`.
- Tree: `eca2006e9aa8f7d8871e24bba128b18b81748325`.
- Base: `909c52ea91a77e0f2ad4d0d9548e33be14f38abb`.

The exact candidate passed Rust 1.94.1 formatting, workspace all-target and
all-feature warnings-denied Clippy, focused skills tests, full workspace tests,
explicit doctests and fresh locked release CLI smoke checks on macOS and Linux.
macOS runtime tests ran serially; Linux retained default test concurrency under
a real unprivileged account, private home, ordinary shells and no capabilities.
Builds completed before process-heavy runtime tests, and the two platforms'
runtime runs did not overlap. Test executables were precompiled without running
them while release builds finished. Source/tree and release hashes were verified.

The repository Python suite passed (269 tests, 14 skips), as did the bounded
documentation check, pinned-fx compatibility/Unicode drift checks, exact
dependency-policy tooling, vulnerability audit, FreeBSD/WASI compilation and
existing no-unsafe contracts. The exact clean validation worktree was used for
documentation checking; no Markdown-scanner changes were made.

Logs are retained under `/private/tmp/mg-skills-macos-replacement.MIdqfV`
and `/private/tmp/mg-skills-cli.jpLoxt/linux-gate-evidence`, with Linux logs
prefixed by the full candidate SHA. Earlier rejected-run logs remain under
`/private/tmp/mg-skills-macos-gate.J4hObK`.

## Independent review round 1

Three fresh local reviewers inspected the entire feature diff against the base,
including related core/native/CLI callers and changed durable contracts. The
named remote review service was unavailable; these were local direct reviews,
not Bugbot results. Correctness/API and lifecycle/platform each found one issue;
performance/resources found two. The first issue was independently corroborated,
giving three distinct accepted P2 findings.

| Finding | Historical source location | Evidence and required repair |
| --- | --- | --- |
| R1-1: executable resource modes lost | `skills_managed/filesystem.rs:444` | Every file was recreated as `0600`, and entries/fingerprints omitted modes. Successful local/Git installs and `create --replace` disabled executable sibling scripts. Preserve safe executable semantics through planning, verification and publication without copying special permission bits. |
| R1-2: unbounded filter copy before validation | `skills_managed/source.rs:42` | `explicit_filter.map(str::to_owned)` copied arbitrary borrowed input before the 256-byte name check. Validate the borrowed filter before copying, preserving empty-filter and error behavior. |
| R1-3: quadratic interactive query matching | `skills_picker/menu.rs:333` | Repeated prefix comparison at each byte position stalled the synchronous picker path within accepted input bounds. Use bounded linear-time ASCII-insensitive matching and avoid redundant filtering after an edit. |

R1-1 is supported by deterministic source evidence and pinned upstream
`src/builtins/skills.zig:744`, whose copy helper preserves source permissions.
The first two tracks used source review without additional process tests.

For R1-3, the resource reviewer ran an explicitly coordinated standalone
optimized Rust 1.94.1 reproduction of the exact matching loop. With 480 entries,
4,096-byte repetitive descriptions and a 4,041,600-byte snapshot charge, a
1,024-byte near-match query took 3.38–3.55 seconds; a 256-byte query took
1.16–1.48 seconds. Source and executable are retained at
`/private/tmp/mg-skills-query-r1.6X0LSL`. This measured the matching loop, not
end-to-end CLI latency, and establishes no M07 performance claim.

Review provenance: `skills_review_r1_correctness`, `skills_review_r1_lifecycle`
and `skills_review_r1_resources`. The candidate was rejected despite green
local tests and was not pushed or merged. All three unchanged review worktrees
were removed after their reviews finished; review and benchmark evidence was
retained.

## Review round 1 remediation

The fixes used three isolated, non-overlapping implementation worktrees:

- R1-1: `3929ab20eec2a4966cda9dd2c93198116f35e5af`, integrated as
  `55bc9cac`, captures modes in exact fingerprints, publishes private normalized
  modes while preserving execute bits, and preserves original backup modes on
  rollback. All 35 focused managed tests passed, including seven new mode
  regressions; exact Rust 1.94.1 strict native Clippy and formatting passed.
- R1-2: `f23a372960b096bb10b5ec3789dffa7ec873a0e6`, integrated as
  `5b75acca`, validates borrowed filters before copying. The allocation regression
  first reproduced 8,388,615 allocated bytes for an invalid 8 MiB filter versus
  264 bytes for a 257-byte filter; both now allocate equally within a 4 KiB bound.
  Five new tests and two existing parser tests passed, as did exact Rust 1.94.1
  strict native Clippy and formatting.

- R1-3: `c2cc6d2b07ead77a9850c3b0cede832dde8388cc`, integrated as
  `46338f0e`, preprocesses each bounded picker query once with a linear-time
  ASCII-insensitive matcher and avoids a redundant CLI cursor transition after
  actual edits. Native cursor/frame semantics remain unchanged. All 22 focused
  native picker tests and six CLI adapter tests passed; exact Rust 1.94.1 strict
  native/CLI Clippy and formatting passed after fixing two new style warnings.
  Deterministic comparison-count and exhaustive small-byte-string regressions
  cover worst-case work and preserved matching semantics. The final optimized
  production-matcher diagnostic took 1.985–2.112 ms for the same 1,024-byte query
  and 480-entry workload; this is not an end-to-end or M07 claim.

Mode and filter logs remain under `/private/tmp/mg-skills-cli.jpLoxt` with
`r1-modes-` and `r1-filter-` prefixes. Their clean committed worktrees were removed
after cherry-pick integration; external logs and build caches were retained.
The query worktree was likewise removed after integration. Its external
diagnostics remain under `/private/tmp/mg-skills-query-r1.6X0LSL`, including
`query-linear-accepted.log` for the final committed matcher.

## Independent review round 2

Candidate `b84afebc18b94ab12eb985d2fc0cf55a5ee0d3d5`, tree
`90ef2cb8a454ba120b3030a7570a94824a1a86b7`, retained the same base. Its complete
replacement local gate passed on macOS and Linux before fresh reviews began.
Exact Rust 1.94.1 formatting, strict Clippy, fresh locked release builds,
focused CLI (44) and native skills (175) tests, full workspace tests, explicit
doctests and official release smoke checks passed. The macOS workspace included
446 CLI and 2,452 native passing unit tests, with existing ignored fixture
entries; Linux retained default concurrency under its unprivileged environment.
The Python suite passed 269 tests with 14 platform skips. Documentation policy,
upstream drift, dependency policy/audit and FreeBSD/WASI checks also passed.
Linux runtime and smoke finished before macOS runtime began.

Mac evidence is retained under `/private/tmp/mg-skills-r1-replacement.hgx1ji`;
Linux evidence is candidate-SHA-prefixed under the existing Linux evidence
directory. Both sources remained clean, and fresh release hashes were verified
before/after runtime and smoke. This was regression evidence, not an M07 claim.

Fresh local reviewers `skills_review_r2_correctness`,
`skills_review_r2_lifecycle`, and `skills_review_r2_resources` inspected the full
feature. Correctness/API and lifecycle/platform each reported zero actionable
introduced findings. Resource review reported one grouped P2 finding:

| Finding | Historical source location | Evidence and required repair |
| --- | --- | --- |
| R2-1: oversized paths copied before admission bounds | `skills_roots.rs:76`, `skills_managed/planning.rs:118` | Root-authority construction normalized/cloned an arbitrary owned label before its 4,096-byte rejection. Relative local installation joined/normalized an arbitrary borrowed cwd repeatedly before the same bound. Reject oversized borrowed path representations before additional allocation, preserving bounded normalization and ignoring unused cwd for absolute local/Git sources. |

The finding was supported by static public-API call paths, not a new runtime
reproduction. A separate `.git*` resource-exclusion lead was ruled out because
the pinned upstream uses the same prefix exclusion. All three reviews were
local, not Bugbot, and performed no builds or process tests. Their clean
unchanged worktrees were removed after review. The candidate was rejected and
was not pushed or merged.
