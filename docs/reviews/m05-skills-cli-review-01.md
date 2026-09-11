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

## Review round 2 remediation

Two isolated implementation lanes repaired the grouped admission finding:

- Managed paths: `f32e83f01915e65cdba41225fb9a7de4a60dbb59`, integrated as
  `244f94f1`, bounds borrowed cwd and joined path bytes before copying or
  normalization. Absolute local and Git sources continue to ignore unused cwd,
  and cancellation keeps precedence. All 45 focused managed tests passed,
  including five new allocation, boundary, normalization and unused-cwd cases.
- Root labels: `fc40440fb34fa6187ed079e39498605fb6b63c7f`, integrated as
  `a1a76aa5`, reuses the existing catalog validator before normalization/copying.
  The shared raw-byte check now precedes UTF-8 conversion. The allocation control
  first reproduced 16,777,226 extra bytes for an 8 MiB owned label versus 8,202
  bytes for a 4,097-byte label; repaired rejection allocates zero additional
  bytes. All 20 root and 30 catalog tests passed, including exact bounds,
  normalization and invalid UTF-8 cases.

Both lanes passed exact Rust 1.94.1 strict native all-target/all-feature Clippy,
formatting and diff checks. Their clean committed worktrees were removed after
integration, preserving external logs/caches. These focused results do not
replace the complete candidate gate or fresh review cycle.

## Path-repair gate and PTY fixture diagnosis

Candidate `f24f8ef31f830d6bac4991cd57dc2324fec6e04a`, tree
`aa12cb35167190bbe2a85e7ad4f438317a0df512`, passed exact Rust 1.94.1 formatting,
strict workspace Clippy, fresh locked release builds, test precompilation,
269 Python tests (14 skips), bounded documentation, drift, dependency policy,
audit and FreeBSD/WASI checks. Linux focused tests, default-concurrency workspace,
explicit doctests and official release smoke passed with the exact clean source
and verified release hash. Build/Linux evidence remains under
`/private/tmp/mg-skills-r2-replacement.VNzMKi` and the existing SHA-prefixed Linux
evidence directory.

Mac focused CLI (44) and native skills (184) tests passed. The first workspace
run failed three `terminal_pty` cases with 2,458 native passes and 12 ignored
fixtures. Fixed no-op child probes returned no positive wait status within the
500 ms observation and separate 500 ms cleanup bounds, before inventory/PTY
helper startup. Read-only diagnosis found the same historical signature in the
[combined CLI review](m03-cli-full-review-01.md), but established no cause or
direct connection to the skills path repairs. An unchanged workspace-filtered
PTY suite passed all 26 tests, including the three failures; this was diagnostic
evidence, not remediation or gate acceptance.

One unchanged full-context retry retained separate logs under
`/private/tmp/mg-skills-r2-runtime-retry.cMHhHP`. The original three cases passed,
but `close_retry_retains_descendant_that_escaped_after_capture` failed at
`terminal_pty.rs:1992` with `ParseIntError::Empty`; 2,460 native tests passed and
12 fixtures remained ignored. The child used `std::fs::write` to create and then
populate `escape.ready`, while the parent waited only for path existence before
reading the PID. This is a concrete test-fixture publication race, separate from
the earlier unestablished admission timeouts. The required repair publishes a
complete readiness value atomically without changing process assertions,
deadlines or production behavior. Neither failed run was pushed or reviewed as
an accepted candidate; their logs were preserved without source-fix claims.

The readiness repair `3b74843e40401fb1439f3d9276fda056e357f809`, integrated as
`805a9214`, changes only the test fixture: write a private pending PID file, then
rename it to the readiness name. A deterministic regression observes the absence
of the ready path before publication and exact complete content afterward. The
regression (one test), affected lifecycle case (one test) and serial PTY module
(27 tests) passed, along with exact Rust 1.94.1 formatting, native package check
and strict all-target/all-feature Clippy. Focused diagnostics used the fresh
patched test executable with the preceding candidate's production-equivalent
release helper; they were not full-gate acceptance. External logs are retained
as `pty-ready-{regression,lifecycle,module}.log` under the shared skills evidence
parent. The clean committed repair worktree was removed after integration.
This repairs the empty-read race, not the separate child-reaping timeouts.

## Independent review round 3

Candidate `7ed64e22c4a88c50db06fe69b1a1f13bb15cf92c`, tree
`9f7e6268150324bce7fd9a271f26999ee0c8f1fd`, passed the complete replacement
local gate on macOS and Linux. Mac focused CLI (44), native skills (184), full
workspace (including 446 CLI and 2,462 native passing unit tests, with existing
ignored fixtures), explicit doctests and official release smoke passed.
Formatting, strict Clippy, fresh locked releases, test precompilation, 269 Python
tests (14 skips), documentation, drift, dependency policy/audit and FreeBSD/WASI
checks passed. Linux retained its explicit unprivileged environment and default
runtime concurrency, and finished before Mac runtime began. Source and release
hashes remained exact and unchanged. Evidence is retained under
`/private/tmp/mg-skills-pty-replacement.WpOjLY` and the SHA-prefixed Linux logs.

The correctness and lifecycle tracks used new local reviewers
`skills_review_r3_correctness` and `skills_review_r3_lifecycle`. A new resource
reviewer could not be spawned because of the host thread limit; the independent
`skills_pty_gate_diagnosis` agent performed its first full skills review instead.
It had authored no source changes and participated in no earlier skills review,
only a separate read-only PTY diagnosis. All tracks were static local reviews,
not Bugbot; they ran no builds or process fixtures. Lifecycle and resources
reported zero actionable introduced findings. Correctness reported one P2:

| Finding | Historical source location | Evidence and required repair |
| --- | --- | --- |
| R3-1: copy exclusion suppresses nested discovery | `skills_managed/filesystem.rs:360`, `skills_managed/planning.rs:158` | Source traversal skipped every `.git*` entry before finding `SKILL.md`, so `.github/skills/review/SKILL.md` yielded `NoMatches`. Pinned upstream discovery walks these ancestors, selects `review`, then applies `.git*` exclusions only while copying that selected skill's contents. Separate the stages while retaining finite bounds and exact source revalidation. |

This is narrower than the previously ruled-out resource-copy lead: excluding
`.github` resources inside an already selected skill is intentional pinned
behavior, but excluding ancestors before selection is not. The candidate was
rejected despite its green local gate and was not pushed or merged. All completed
clean review worktrees were removed, preserving their evidence.

## Nested discovery remediation

Native repair `70444d649e3a068d998f6b86e043d045a6741a9e`, integrated as
`9d0b46be`, separates ancestor inventory from selected-resource capture. Inventory
reads candidate files one at a time and retains a digest of paths, metadata and
candidate bytes. Selected trees keep the existing copy exclusions and payload
bounds; ordinary overlapping selections share immutable bytes. Independent
bounded read phases preserve the full selected payload allowance. Local plans
revalidate the inventory and exact selected tree fingerprints; completed Git
plans retain immutable payloads without retaining a consent-time clone.

All 53 focused managed tests passed, including eight discovery regressions for
hidden ancestors, copy exclusions, stale revisions, non-following traversal,
overlap sharing and exact 64 MiB selected capacity. Exact Rust 1.94.1 strict
native all-target/all-feature Clippy, formatting and diff checks passed. Logs
remain under `/private/tmp/mg-skills-cli.jpLoxt` as
`r3-discovery-focused-final.log` and `r3-discovery-clippy-accepted.log`.

CLI regression `3fdc2105597147acb60d68b91c077655cefabae6`, integrated as
`2feaad26`, exercises `/skills install` for `.github/skills/review`, receipt and
refresh, ordinary resources, excluded `.git*` resources, unchanged source and
zero inference. The fresh composed-interactive test executable against the
unrepaired backend failed as expected with `NoMatches`; formatting, precompilation
and strict CLI lint passed. This harness uses a production-run test child with
injected boundaries, not a direct invocation of the shipped CLI. Its green
integrated run and separate fresh-release smoke belong to the replacement gate.
The `r3-cli-discovery-{build,clippy,red}.log` evidence is retained. Both clean
committed repair worktrees were removed after integration; these focused results
do not replace full-feature validation or fresh independent review.

## Independent review round 4

Candidate `b6760d701bdac14507e2c69f5d475378791b3857`, tree
`b7505273433fc088c9ee3a80d59d82fa6df647b6`, passed the complete replacement
local macOS/Linux gate. Mac focused CLI (45), native skills (192), workspace
(447 CLI and 2,470 native passing unit tests, existing ignored fixtures),
integrations, explicit doctests and official fresh-release smoke passed.
Formatting, strict Clippy, fresh locked releases, all test precompilation,
269 Python tests (14 skips), drift, documentation, dependency policy/audit and
FreeBSD/WASI checks passed. Linux retained its unprivileged environment and
default runtime concurrency; all builds finished first, and Linux runtime and
smoke finished before Mac runtime. Exact source and release hashes stayed
unchanged. Logs remain under `/private/tmp/mg-skills-r3-replacement.WnRwB1` and
the SHA-prefixed Linux evidence directory. The previously expected-red composed
CLI regression passed with the integrated discovery backend.

Three newly spawned local reviewers, `skills_review_r4_correctness`,
`skills_review_r4_lifecycle` and `skills_review_r4_resources`, inspected the full
feature against parent `909c52ea91a77e0f2ad4d0d9548e33be14f38abb`. They performed
read-only source/contract/test inspection, not builds, fixtures, remote checks
or Bugbot service execution. Lifecycle and resources reported zero actionable
findings. Correctness established one P2:

| Finding | Historical source location | Evidence and required repair |
| --- | --- | --- |
| R4-1: non-EOF metadata prefix accepted as complete | `skills_catalog/io.rs:305`, `skills_catalog/discovery.rs:253` | A valid header whose closing dashes end exactly at byte 16,384 is accepted before its following newline is read. Discovery records a different body offset from full materialization, so the unchanged skill fails as `StaleSelection`. Require a complete newline or actual EOF at the prefix boundary without widening header/read bounds. |

The candidate was rejected despite its green local gate and was not pushed or
merged. All three clean completed review worktrees were removed; retained logs
remain historical evidence, not a claim that the outstanding finding is fixed.

The completion repair `11dea0fbc903013a2a2a6f04dce6f431db5c0b7a`, integrated as
`34c1c242`, changes only the catalog prefix-completion predicate: a detected
closing delimiter needs its terminating newline or an actual zero-byte EOF
read. Shared parsing, read loop, budgets, public APIs and error policy remain
unchanged. The public discover/resolve/materialize regression first reproduced
`StaleSelection` against the unchanged baseline, then passed with the fix.
All 36 catalog tests passed, including six new tests covering short reads,
LF/CRLF splits and complete chunk-end delimiters, continued delimiter-like lines,
actual EOF at the exact 64 KiB header limit, and over-limit newline rejection
without an advertised selection. Strict exact Rust 1.94.1 native
all-target/all-feature Clippy, formatting and diff checks passed. Evidence is
retained as `r4-prefix-{expected-red,focused,clippy}.log` under the shared skills
evidence parent. The clean committed repair worktree was removed after
integration. Full replacement validation and fresh reviews remain required.
