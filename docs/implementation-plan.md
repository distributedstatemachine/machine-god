# Implementation plan

Status values: `NOT STARTED`, `IN PROGRESS`, `BLOCKED`, `COMPLETE`.

## Objective

Build a high-performance Rust 1.94.1 coding-agent engine inspired by
`vercel-labs/fx`. Local development and CI use the declared minimum toolchain
exactly. Local `+stable` is a narrowly scoped fallback for this checkout's
damaged exact-toolchain installation and is valid only while both local Rust and
Cargo report release 1.94.1 exactly.
The embeddable asynchronous engine is the primary product; the CLI is its native
reference host. Observable performance and compatibility claims require retained
evidence against the pinned upstream revision. Product claims use committed,
reviewed summaries with input and result digests. The 90-day bootstrap workflow
artifact is non-product evidence of the collection path and may expire without
weakening those durable claim records.

## Delivery workflow

Every feature uses `agent/mNN-feature-slug`, isolated subagent worktrees, local
checks, three fresh adversarial reviewers, a pushed feature branch, remote CI for
the exact SHA, and a fast-forward push to `main`. Confirmed review findings are
fixed and rereviewed until none remain. Rejected findings are documented under
`docs/reviews/`. Every iteration ends by verifying that its changes are
committed, integrated where required, and clean before safely removing its
isolated worktrees. Active or uncommitted worktrees must never be removed. CI
executes third-party actions by reviewed immutable commit,
keeps checkout credentials disabled, and grants the workflow read-only contents
permission. Python `test_*.py` files are discovered repo-wide in deterministic
order, excluding generated and checkout state under `.bench`, `.git`, and
`target`. Workspace tests execute natively on pinned Linux and macOS x86_64 and
aarch64 runner labels rather than relying on cross-compilation alone.

## Milestones

| Milestone | Deliverable | Status |
| --- | --- | --- |
| 01 | Repository, docs, CI, workspace, upstream benchmark harness, and non-product bootstrap evidence | COMPLETE |
| 02 | Provider-neutral streaming engine and deterministic testkit | COMPLETE |
| 03 | Providers, native tools, permissions, sessions, config, and CLI | IN PROGRESS |
| 04 | Security, lifecycle, concurrency, and persistence hardening | NOT STARTED |
| 05 | Skills, MCP, ACP, and subagent extensibility | NOT STARTED |
| 06 | SDK surfaces and advanced compatibility | NOT STARTED |
| 07 | Optimization, packaging evidence, and final hardening | NOT STARTED |

The thirty-fourth bounded slice, native `terminal`, is contract-frozen and
**IN PROGRESS** from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2`. Its normative boundary is
[`terminal.md`](terminal.md) and its live ledger is
[`m03-terminal-review-01.md`](reviews/m03-terminal-review-01.md). This first
slice implements only bounded foreground `exec`: fixed `/bin/sh -c`, a
descriptor-retained workspace-relative starting directory, a bounded
construction-time environment snapshot identified in the exact process
capability, separate bounded stdout/stderr capture, fail-fast concurrency,
first-poll timeout around controllable userspace phases, cancellation,
authoritative output limiting, and bounded process-group cleanup. Safe Rust
cannot preempt blocked filesystem, spawn, kernel-wait, or uninterruptible host
syscalls, nor synchronous trusted-executor poll/drop or arbitrary Waker
callbacks, so the timeout is not an unconditional wall-clock ceiling. The
production system executor's public constructors are Linux-only because safe
standard Rust exposes no descriptor-relative process cwd primitive on macOS and
unsafe Rust is forbidden. Private reference-host composition retains the non-
Linux catalog entry and fails fixed unsupported at allowed execution before cwd
lookup or spawn. It is not a sandbox and makes no PTY, durable-session, fx-
equivalence, benchmark, or product-performance claim. Formal cycle 2 rejected
its exact candidate with a deduplicated `0/2/1/2` union. Cycle-3 production,
independent evidence, and documentation remediation is composed through exact
behavior head `62d0fe17e765b86be12efe8f99c96f871b47ba0c`, tree `0e6f94b`.
Its complete exact-1.94.1 replacement gate was green. Formal cycle 3 rejected
exact candidate `3215b9e`, tree `3fafcad`, with a deduplicated `0/2/2/0` union.
Cycle-4 source, evidence, and documentation remediation is pending; its
replacement gate, fresh exact-SHA reviews, and remote delivery remain pending.

The frozen contract checkpoint is exact `79ae1b7`. Isolated host component
`ecf3e78`, production precursor `ea216db`, independent-evidence component
`785b193`, lifecycle remediation `64a4850`, configurable-system component
`13dfd28`, and expanded Linux-evidence component `ebc631e` compose locally
through exact `59b069a84e7d4dc4d76ac65520b9045603cae8af`. The composition adds the
exact process-environment capability, strict native tool, Linux system executor,
portable injected seam, deadline guardian, bounded process-group/pipe lifecycle,
fifteen-tool host catalog, 23 portable/private focused tests, and seven Linux-
only system tests. Exact precursor `ec8bbc9` passed the four required Rust gates
but exposed three stale dependency-topology assertions in the first 136-test
Python run. Exact evidence/documentation remediation `80c6ee0`, tree `027c4bc`,
requires unconditional native `sha2` for terminal environment identity and
passed the complete exact-1.94.1 replacement gate. Formal cycle 1 rejected exact
candidate `fba499e`, tree `8679917`, with a deduplicated `0/3/3/4` union.
Independent evidence `8c58c29`, contract remediation `ddccdb2`, and lifecycle/
resource remediation `3725aed` compose through exact `b03ecb0`; its complete
replacement gate is green. Formal cycle 2 rejected exact candidate `a566db2`,
tree `785c084`, with a deduplicated `0/2/1/2` union. Accepted cycle-3 semantics
publish observed output-limit authority before cleanup and preserve validated
outcome invariants; retain the originating active slot through an arbitrary
blocking Waker callback; avoid a mandatory TERM grace after ordinary foreground
exit while retaining the leader through final signal dispatch; accept literal
tilde-prefixed components such as `~cache` while rejecting exact `~`; and keep
the public timeout claim conditional around blocking syscalls, executor poll/
drop, and callbacks. Exact behavior head `62d0fe17`, tree `0e6f94b`, passes all
four required commands with 1,157 non-documentation tests and two doctests,
Python 136/8 skips, pinned-fx drift, dependency policy/audit, Linux/FreeBSD/WASI,
documentation/diff/no-unsafe, and fresh release-binary gates green. Formal
cycle 3 rejected exact candidate `3215b9e`, tree `3fafcad`, with two high and
two medium deduplicated findings covering the last overflow/deadline race and a
unified originating-slot ownership failure across injected executors, dual
callbacks, and no-Waker native thread tails. Cycle-4 remediation is pending and
does not establish a replacement candidate, green gate, review, or delivery.

The thirty-third bounded slice, native `web_search`, is **DELIVERED** from exact
delivered base
`4ba9f5afde89b9666fe9929bb81fbabcaa834334`. Its normative boundary is
[`web-search.md`](web-search.md) and its live ledger is
[`m03-web-search-review-01.md`](reviews/m03-web-search-review-01.md). Production,
independent evidence, and documentation were owned in separate isolated
worktrees and compose through exact behavior precursor
`3d2984000301e58762e0940504159aeb55b2389e`, tree
`5222c3e009e9fe440097a86fd46889d1bb2e1434`. Its complete exact-1.94.1 local
gate is green. Formal cycle 1 rejected exact candidate
`89c5ec95fb5353efcba34af6a44bc27d7b6027f7`, tree
`8d91a556f786169d42406e91e8ad2f476b7c6cf4`. Correctness/API reported
`0/1/3/2`, native effects/lifecycle `0/2/3/1`, and performance/resources
`0/0/3/0`; deduplication yields `0/2/5/2` in blocker/high/medium/low order.
Exact isolated lifecycle remediation component
`096b11c4ffed7b6b4e4419940d1cd50ef40b197f` and exact
isolated portability/bounds remediation component
`ca0b990a07c7e6659c10e91f86a2aa66ed4d3182` are composed in the current
remediation. Together they release outer shared-transport capacity before nested tool
execution, bind custom transports to their exact permission target, require an
explicit fallible deadline authority, bound every public tool constructor,
export portable contracts without HTTP/WASM gates, incrementally retain one
bounded SSE record, cap deduplication state, and redact source debug output.
Exact composed remediation precursor
`e662fa8047c5ca321d622b9b5920166804a35c27`, tree
`6c0ace98ea9931af9d16cc9fb2ade969df477d3c`, passes replacement evidence and
the complete exact-1.94.1 local gate without fallback. Formal cycle 2 rejected
exact candidate `399f5f7a14c1473d9e737d44838549ba305746de`, tree
`99a88a45fd6f0823b23fd879633784433194cf8d`: correctness/API reported
`1/1/0/0`, native effects/lifecycle `0/0/0/1`, and performance/resources
`0/0/1/0`, yielding a reported deduplicated `1/1/1/1`. Primary-source
adjudication rejects the purported blocker: the raw language-model-v4 stream
uses `result`, which the higher-level Vercel AI SDK maps to public `output`.
The accepted union is `0/1/1/1`. Remediation covers URL-standard numeric IPv4
aliases, a reachable escaping-sensitive serialized-output boundary, and stale
host-composition wording. Exact composed remediation precursor
`366cef966d7dcf1b11101a37d4493099e6f421a7`, tree
`40c05cb2999c641bc7ccbdc369fc6d9251b989b7`, passes the complete exact-1.94.1
replacement gate without fallback. Formal cycle 3 rejected exact candidate
`aef6abed174760195e712b2701e241b656733621`, tree
`5abcef3de31898e158e6c4872ee9b4131863d1b7`, with a deduplicated `1/0/2/2`.
Protocol-shape, terminal-release, numeric-host overflow, port grammar, and
documentation remediation is composed from exact isolated components
`5d45dca` and `454f8fd`. Exact composed precursor `b834205`, tree `f3557a5`,
passes the complete replacement gate. Formal cycle 4 rejected exact `cc1d3d1`,
tree `ad0c3d3`, with a deduplicated `0/0/1/1`; finish-envelope and documentation
remediation is composed from exact isolated component `dc79c8d`, tree
`e2fed70`, plus host-fixture component `9f6c474`. Exact precursor `2e9c44d`, tree
`3e25daa`, passes the complete replacement gate. Formal cycle 5 is green on
exact `782aa54`, tree `b1ba692`, with a `0/0/0/0` union. Review-exempt delivery
record `52b5885f275c9f6f4f16b378f71780c29f2ebab2`, tree
`148b358ce520645c4c4a9e4f186459e40a325572`, passed feature CI
`33023313461`, feature Benchmark evidence `33023313463`, main CI
`33023812814`, and main Benchmark evidence `33023812808`. Every run reports
that exact SHA and succeeded; both benchmark workflows retained exactly two
unexpired exact-SHA artifacts. `main` was fast-forwarded without force. Slice
33 is delivered and the delivered count is thirty-three.

The frozen tool is locally executed through core's existing strict tool loop.
It accepts required `query` plus optional mutually exclusive
`allowed_domains` / `blocked_domains`, performs bounded canonical DNS-filter
normalization, and prepares the exact configured AI Gateway `NetworkTarget`
and canonical execution arguments before the default critical-risk ask flow.
One approved execution makes at most one private required Perplexity provider-
tool request through the shared injected `Arc<dyn AiGatewayTransport>`. A
dedicated bounded native one-shot codec accepts exactly one expected
`providerExecuted: true` call, one matching final result, and a strict raw-v4
successful finish whose usage and provider metadata are validated then
discarded. The ordinary outer `AiGatewayProvider` and provider-neutral core do
not gain provider-executed advertisement or event semantics.

Public ceilings are 4,096 query bytes; 16 normalized domains and 4,096
aggregate domain bytes; ten sources; 512 title bytes; 2,048 URL bytes; 16 KiB
request; 256 KiB SSE response; 64 KiB SSE record; 256 records; 16,384 JSON
nodes; 48 KiB serialized output; one 30-second total deadline beginning before
capacity wait; default concurrency four and hard concurrency sixteen. The
4,096-byte aggregate-domain and 16 KiB worker-request caps are defense in depth:
stricter count/per-item/query limits make their exact boundaries unreachable.
JSON escaping makes the 48 KiB serialized-output ceiling reachable; replacement
evidence must admit the exact boundary and reject the first byte beyond it. The
production adapter is non-WebAssembly behind `ai-gateway-http`, and current
reference-host composition remains Linux/macOS-only. The candidate catalog has
fourteen alphabetical tools: twelve descriptor-backed workspace tools plus
rootless `web_fetch` and Gateway-backed `web_search`.

This slice deliberately adds no retry, fallback, Parallel backend, multiple
provider search use, progress, inner usage or billing, snippet/page content,
cache, artifact, `read_tool_result`, CLI command, live-provider test, benchmark
workload, product-performance result, compatibility promotion, or fx-
equivalence claim. Every implementation and review/remediation iteration must
end by verifying that its changes are committed, integrated where required,
and clean before safely removing its isolated worktrees. Active or uncommitted
worktrees are retained.

The thirty-second bounded slice, strict top-level
`session <id> [--json]`, is **DELIVERED** from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Its normative boundaries are
[`session-cli.md`](session-cli.md) and
[`native-session-inspection.md`](native-session-inspection.md), and its live
review ledger is
[`m03-session-cli-review-01.md`](reviews/m03-session-cli-review-01.md). Two
independent discovery agents audited the remaining M03 CLI and pinned-fx
surfaces and agreed this summary-only exact-ID inspection is the smallest
coherent next slice. Exact cycle-2 candidate
`1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed its complete same-SHA local
gate but is rejected. Correctness/API reported `0/0/1/2`, native boundary/
effects `0/0/1/2`, and performance/concurrency/resources `0/0/1/1`, in
blocker/high/medium/low order. Deduplication yields two medium themes—canonical-
number mismatch and residual payload-proportional allocations—and two low
themes—duplicate-key mismatch and stale maintained documentation.

The synchronized replacement contract keeps the strict CLI and six-field
projection but requires a specialized one-pass summary operation rather than
`FileSessionStore::load` or a full `SessionRecord`. It uses at most 4 KiB per
read, fixed-stack known-token scratch, canonical `serde_json::Number`
semantics, payload-sized ownership only for the two returned IDs, and a fixed-
digest duplicate tracker capped at 65,536 entries that grows entries and
buckets fallibly in proportion to unique keys and reproduces ordinary last-
value-wins metadata/nested-JSON semantics. Aggregate final decoded-tree logical-
node accounting is separately capped at 65,536; shadowed duplicate values may
increase parse work beyond that logical-node count, but the 8,651,165-byte file
ceiling bounds total parse work. The ordinary store schema is
object-only for its envelope, record, message, tool-call, and tool-output
structs and string-only for role; ordinary loading/listing and streamed
inspection reject the six noncanonical sequence/map forms consistently while
the canonical writer stays unchanged. The 8,651,165-byte file,
depth-64, node, schema, identifier, counter, and content-shape constraints are
store-owned, not engine limits. Lock wait, filesystem latency, and `EINTR`
retries remain unbounded in wall-clock time and attempt count.

The pre-remediation `1d09a0d` gate passed all four required exact-1.94.1 Rust
commands, Python 135 tests with eight expected macOS skips, exact pinned-fx
regeneration, WASI/FreeBSD target, diff/no-unsafe, and release-matrix checks.
Documentation integrity was 85 Markdown files, 146 fenced blocks, 626 parsed
links, and 81 unique repository targets with zero errors. The 4,001,712-byte
release binary had SHA-256
`e975e8a16f750188de25d8cf0eac02975643edf6730d6b3ad87d442b76ce27bb`.
Those results do not approve the rejected candidate. The cycle-2 synchronized
replacement source was composed at exact
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the complete required
exact-1.94.1 local gate without fallback. Focused 56 CLI unit, 54 CLI process,
and 21 native inspection tests are green. Python passed 135 with eight expected
skips; pinned-fx regeneration is byte-stable; WASI/FreeBSD checks are green
with only the established WASI `read_file` warning; documentation integrity is
85/147/626/81 with zero errors; and there is no manifest, lockfile, dependency,
benchmark, inventory, or unsafe-Rust delta. The 4,001,760-byte release binary
has SHA-256
`483eb60f707cadfe4b0dd10cfb65617e576488546d908f2f6811b0bfc55773cc` and its
full matrix is green, including six process differentials, an 8,650,857-byte
near-cap record, 4,097-message and 262,145-byte-metadata engine-over-default
records, and a held-lock wait of at least 500 ms. At that checkpoint, the next
formal review, remote workflows, `main` integration, and delivery were pending.
Formal cycle 3 rejected exact candidate
`9282b4044c5fb5a249598d23d098562c96850c99`, tree
`6d41f7ee6eb017dfc65d6f6623d049ac09c2966f`. Correctness/API and native
boundary/effects each reported `0/0/1/0`; performance/concurrency/resources
reported `0/0/1/1`. The deduplicated `0/0/1/1` union is a medium context-free
recursion-budget mismatch and low self-counted allocation evidence. Exact
cycle-3 remediation `af055ff3b22e157b1c42d1579b041c3cc4c05b0e`, tree
`14eafada4b3dddd62a9cb8e6077ad8f0b81753e8`, is composed. It reproduces
`serde_json` 1.0.151's 127-active-container budget with typed parent contexts
3/6/7 and exact nested-array accept/reject boundaries 123/124, 120/121,
119/120, and 119/120. Focused evidence is now 58 CLI process tests with ten
equivalence cases and 22 native inspection tests. Dev-only
`allocation-counter` 0.8.1 runs five shapes in isolated child processes; each
reports exact allocation counts 14/2/8 and byte counts
8,913,715/14/8,913,347 for total/current/maximum. The manifest/lock delta is
dev-only. The complete replacement gate is green on that exact remediation
under Rust/Cargo 1.94.1 without fallback. Python 135/8 skips, byte-stable
pinned fx, WASI/FreeBSD with only the established warning, documentation
85/147/626/81, dependency policy/license/audit, diff/inventory/no-added-unsafe,
and the release matrix are green. Exact `cargo-deny` 0.20.2 has only three
established duplicate warnings; exact `cargo-audit` 0.22.2 scanned 211
dependencies against 1,226 advisories with zero vulnerabilities. The production
normal/build graph remains 364 lines. The 4,001,760-byte release binary has
SHA-256 `d296174898938f632351bebb38449533c7db03bb3659392bea3743a02ee1619d` and
passed the 18/18 session matrix. Direct and native near-cap probes each passed
1/1. Formal cycle 4 rejected exact candidate
`df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. Correctness/API, native boundary/
effects, and performance/concurrency/resources each reported `0/0/1/0`; the
deduplicated `0/0/2/0` union is an ordinary-versus-streamed wire-form mismatch
and eager approximately 8.9 MB duplicate-tracker reservation. The replacement
must make the five stored structs object-only and `Role` string-only, grow
fixed-fingerprint tracker entries/buckets fallibly in proportion to unique
keys under the unchanged 65,536 ceiling, bound small-record allocator high-
water use, and compare long/short discarded values at equal structure.
Exact remediation `1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`, is composed and passed the
complete local gate under exact Rust/Cargo 1.94.1 without fallback. Focused 24
native and 64 CLI process tests, including 16 differentials, are green. Python
135/8 skips, byte-stable pinned fx, WASI/FreeBSD, documentation 85/147/626/81,
dependency policy/audit, unchanged production graph, diff/inventory/no-added-
unsafe, release evidence, and the native near-cap probe are green. The
3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c`.
Formal cycle 5 rejected exact candidate
`8f533cdec235660c3e17b70fc5bbd5dd0ab8c1f6`, tree
`8215fb94fa3de08841b26dd9d7c63a2ecb7e8a8d`. Correctness/API reported
`0/0/0/1`, native boundary/effects reported `0/0/0/0`, and performance/
concurrency/resources reported `0/0/0/1`; the deduplicated union is
`0/0/0/1`. The sole low is stale cross-document status: `README.md`,
`docs/README.md`, `docs/architecture.md`, `docs/cli.md`,
`docs/performance.md`, `docs/security.md`, `docs/session-store.md`, and
`docs/reviews/README.md` still described cycle 4 as pending or retained its
superseded approximately 8.9 MB allocation evidence as current. Production
review was otherwise green. Independent correctness evidence ran 312 generated
valid/boundary differentials and 1,200 randomized mutation differentials with
zero mismatch; the native track was green; and the performance allocation and
resource audit was green. That cross-document remediation is composed in exact
cycle-6 candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Correctness/API, native effects,
and performance/resources each reported `0/0/0/1`; the deduplicated `0/0/0/1`
is solely that the candidate described its own committed remediation as
pending. There was no additional production, API, native, or performance
finding. Formal cycle 7 rejected exact candidate
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`. Correctness/API and native effects
each reported `0/0/0/0`; performance/resources reported `0/0/0/1`, producing a
deduplicated `0/0/0/1` union. The sole low is imprecise resource wording:
shadowed duplicate values may parse more nodes than survive in the final tree.
The 65,536 caps apply separately to tracker entries and aggregate final decoded-
tree logical-node accounting, while the 8,651,165-byte file ceiling bounds
total parse work. Production and resources were otherwise green. The wording
correction is present in exact reviewed cycle-8 candidate
`d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
`6439863a9b7fd1720156c790fedc4798256c2b6d`. Formal cycle 8 is **GREEN**:
correctness/API, native/effects, and performance/resources each reported
`0/0/0/0`, with a deduplicated `0/0/0/0` union. Independent correctness evidence
ran 598 release-binary differentials with zero mismatches plus focused 24
native, 14 CLI unit, and 24 CLI process tests. Native reconfirmed focused 24 and
green WASI/FreeBSD checks with the established `read_file` warning. Performance
confirmed the exact recorded allocator tuples and corrected separate cap
semantics. This documentation-only result seal is review-exempt and records the
reviews of `d724b61` / `6439863`; it does not imply that the seal commit was
reviewed. Review-exempt documentation seal
`b6db9a67c070f7ef599d994c44b4a21731a004c5`, tree
`59dd628fd0552c5083449f7a31aa4241a8ecb952`, passed feature CI run
`32965947722`, feature Benchmark evidence run `32965947723`, main CI run
`32966531225`, and main Benchmark evidence run `32966531319`, all successfully
for exact `b6db9a6`. The same exact SHA is integrated on `main`; each benchmark
run retained exactly two unexpired exact-SHA artifacts. Slice 32 is delivered
and the delivered count is thirty-two. Zig builds only the pinned upstream fx
comparison input; the product remains Rust and is not written in or shipped as
Zig. This final delivery-record commit is docs-only and user-exempt from
adversarial review. Because a commit cannot contain its own future workflow
IDs, this record's exact workflow IDs will be reported at handoff.
Pinned fx has broader
`last`, `--id`, history, workspace, resume, migration, and recovery semantics,
so this slice is deliberately non-equivalent, unmeasured, and claim-ineligible.
The fixed benchmark workload is unchanged, and the production composition adds
no dependency or unsafe Rust; only dev-only allocation instrumentation is new.
No new product-performance or fx-equivalence claim is made. The delivered count
is thirty-two.

The thirty-first bounded slice, strict top-level `sessions [--json]`, is
**DELIVERED** from exact base `feaf9fa1bc6bb66544947152e2c5fe91c8cd185e`.
Its normative boundary is [`sessions-cli.md`](sessions-cli.md), and its live
review ledger is
[`m03-sessions-cli-review-01.md`](reviews/m03-sessions-cli-review-01.md).
Native process composition, CLI production, and independent evidence were
implemented in isolated non-overlapping worktrees while documentation and
integration remained coordinator-owned. The command exposes only the existing
bounded sorted session IDs and truncation bit through a safe no-create state-
root facade; it will not invent fx's workspace, summary, newest-first, or
pagination semantics. `sessions-json` becomes implemented but remains non-
equivalent, not measured, and claim-ineligible. Exact candidate
`9448738908e014c9273960d9e7dd0790e0525848`, tree
`e5342f79207af6cc41da71a280d8ca9ef3952756`, passed its complete local gate but
was rejected by fresh review cycle 1 with a deduplicated `0/0/0/3` verdict: the
process facade captured unrelated configuration state, captured `HOME` before
XDG-state precedence made it necessary, and the maintained review status was
stale. The replacement now uses a state-only first-poll reader with lazy `HOME`
fallback, carries exact injected-reader regressions, updates the maintained
status, and passed the complete replacement local gate. Exact candidate
`a52765250bf7b0e6a7224ae95cf5a4ab046ffa75`, tree
`0249dd0b97197fa37ab64d2c1baa3b818717211a`, then passed three fresh exact-SHA
cycle-2 reviews at `0/0/0/0` each and is formally green. Feature workflows,
integration, and `main` workflows completed under review-exempt seal
`b5b911621d448b11f6e68b9862896b11c4f13d78`, tree
`3e61754ef6ae82edd2ddffa78e690f5423a7eca8`: feature CI `32939742230`, feature
benchmark evidence `32939742231`, main CI `32940279028`, and main benchmark
evidence `32940279005` are green, with exactly two unexpired exact-SHA artifacts
in each benchmark run. `main` was fast-forwarded without force. No product-
performance or fx-equivalence claim is made. The delivered count is thirty-one.

The thirtieth bounded slice, strict top-level `doctor [--json]`, is
**DELIVERED** from exact delivered base
`f82ce46736f7bac4154da508e3b768d0b9248e15`. Its normative boundary is
[`doctor-cli.md`](doctor-cli.md), and its live ledger is
[`m03-doctor-cli-review-01.md`](reviews/m03-doctor-cli-review-01.md). Exact
candidate `15f8176b9322ef989a8c3db01bd404b79d6469fb`, tree
`8278a777dbfe375e126ce782f581182a73d1e25e`, passed the complete exact-1.94.1
local gate and three fresh review tracks with `0/0/0/0` findings in each.
Review-exempt seal `345f8125f3ffa029b7ad1df4cf3673428fcf023d`, tree
`889984990d700712978da933587a437bd13e2a71`, passed exact feature CI
`32933464234`, feature benchmark-evidence `32933464047`, main CI `32933879888`,
and main benchmark-evidence `32933879930`. `main` was fast-forwarded without
force from the delivered base; each benchmark run retains two unexpired exact-
SHA artifacts. The delivered count is thirty. Against pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`, `doctor-json` is implemented but
non-equivalent, not measured, and claim-ineligible. This record makes no
performance, compatibility-promotion, or fx-equivalence claim. The final
delivery-record commit is documentation-only and review-exempt; its own exact
feature and `main` workflows are reported at handoff rather than claimed here.

Milestone 02 completion evidence is retained in the
[milestone review](reviews/m02-milestone-review.md). Milestone 03 is in progress
with thirty-one delivered bounded slices. The twenty-seventh `web_fetch`
slice is delivered on reviewed seal
`aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
`633ddd44406e22f373962c6a2ec965eae4b9cbdb`. Exact feature CI `32874471757`,
feature benchmark-evidence `32874471812`, main CI `32875016066`, and main
benchmark-evidence `32875015892` are green; each benchmark run retains two
nonexpired exact-SHA artifacts. `main` was fast-forwarded without force from
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. Its pinned fx observation is
`b1774fbf6c7602b503026f96f6e960e946c692ef`; delivery makes no product-
performance or fx-equivalence claim. The twenty-eighth top-level
`permissions [--json]` slice is **DELIVERED**
from exact delivered base
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`. It is a read-only projection of
the already delivered ask-only native configuration and owns no rule, grant,
engine, runtime, session, credential, or network state. Its separate production,
independent-evidence, and maintained-documentation components are composed in
exact candidate `fe2475329b50e89bc069eded3eb2f398e8e1a167`, tree
`757f5169f03bf4018b4d85830cf37a2b716cd0cb`. The candidate passed its complete
local gate but formal cycle 1 reported a deduplicated 0 blocker, 0 high, 1
medium, and 2 low findings, so it is rejected. Remediation is composed from
exact production component `4cf50b6f7e7ddec00e1e251902e5b9983036dd7b`,
tree `fb4d0dc7c7501ccd3547cdf1fcc83b84623f4a08`, and documentation
component `278dd45b9e38e8912d803832c37962b84faf9fe5`, tree
`536ef22cff93d5c478600e7554b65c8442b896ab`. Exact cycle-2 candidate
`e0d590608640d7fe95f307163c99efd3e90fd2b3`, tree
`cd8919b1ff86af1b1bfbd0421a8280fc57473444`, passed the complete replacement
gate but formal review reported a deduplicated 0 blocker, 0 high, 0 medium, and
2 low findings, so it is rejected. Exact isolated native component
`fa83c6c6427028c18e1c36ba6603eb44e4102eac`, tree
`a9ac7a1c147cb2ea61c61bcbf8cb58ac407bb14f`, and documentation component
`1f8968d7592de544be3c5549c275c6bc876e62c0`, tree
`058aa738487bfce08be2d964f8a577fdc12fea09`, compose cycle-2 remediation;
feature-branch composition adds the Windows cfg mirror of the non-Unicode
regression. Exact cycle-3 candidate
`4645a0fa646d2f2bb7138cd55a8ded924d9b8992`, tree
`d1eba954a983d77f2d6d11521016dc3c8eae3735`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. Formal review reported
a deduplicated 0 blocker, 0 high, 0 medium, and 1 low finding, so it is
rejected. Correctness/API found stale lower permissions lineage in
`docs/security.md`; native config/error lifecycle and performance/CLI
portability were green with zero findings. Exact security-document components
`4ade5d839be699accaa4dac834787eab135244b2`, tree
`d2dabd20dcbd1bf0e6f3c005f715d8839e12a3f2`, and
`32236fd8b67cdc44d9deb0ee3ca5b45708560f38`, tree
`f179700fab0a38386cabda02e16d121edbb556a0`, plus ledger component
`ef1f619e72193d08ba3729042201543a10f5d838`, tree
`2368c3cd3fd209ba64a8f3a1cc3fcd5014521289`, are composed in exact cycle-4
candidate `5a6e7782fab98b57bb939f525b3a100d5d7eee1e`, tree
`90d0b750ea9bb396074211adb07e2b251e30d505`. It passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback, including 897 registered
non-documentation tests, 31 intentional child-process probes, two doctests,
focused configuration and CLI suites, pinned compatibility
and 31 generator tests, documentation integrity, dependency/unsafe checks, and
the full release-binary matrix. Formal review reported a deduplicated 0 blocker,
0 high, 0 medium, and 1 low finding, so the candidate is rejected.
Correctness/API and native config/error lifecycle were green; performance/CLI
portability found ambiguous “remote integration” wording in `docs/security.md`
that conflates composed source with pending delivery boundaries. Exact security
terminology component `2b686da95850fa6d7ae5790e1eaac19c585f3eb7`, tree
`9ceff2e2e0a5f1fd1264cf0fff4e8cf02e05b5d6`, and ledger component
`178c58002c1dcb37d411129a1547f686e4711570`, tree
`5d6050f6d1486f26e9134353f34cacf74b90c0c2`, are composed. Exact cycle-5
reviewed candidate `0b13944d19cfb33b4542d82d74c302669817c1af`, tree
`2ea72e810f07ed8ca2d4e8647fa713088477d8b5`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. It passed 897
registered non-documentation tests plus 31 child probes for 928 executions,
two doctests, focused native configuration 25+29 and CLI 6+19, pinned-fx
compatibility and all 31 generator tests, documentation integrity
76/110/548/391 with zero errors, no dependency or unsafe delta, and the full
release matrix. Its 368,944-byte release binary has SHA-256
`8756c7801285f1b09cad9a8b8ce47700a44127dec68ef2b0613e6a5dcecad45e`.
Correctness/API, native config/error lifecycle, and performance/CLI portability
each reported 0/0/0/0 in three fresh reviews; the deduplicated union is zero
and the behavior candidate is formally **GREEN**. Documentation-only green
seal `3e41cc6b90adb34d62aec21c6d03729d59ca0c1b`, tree
`bd74a96c4952c2eb1e15372f4ab716a76bba91a9`, is exempt from redundant
adversarial review. Exact feature CI `32891031065`, feature benchmark-evidence
`32891031147`, main CI `32891614025`, and main benchmark-evidence `32891614060`
are green on that exact seal SHA. `main` was fast-forwarded without force from
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`; each benchmark run retains
exactly two unexpired exact-SHA artifacts for 90 days. The delivered count is
twenty-eight and M03 remains in progress. This delivery makes no product-
performance or fx-equivalence claim. The final delivery-record commit is
documentation-only and review-exempt; its own exact feature and `main`
workflows will be reported at handoff rather than claimed here.
The twenty-ninth bounded slice is contract-frozen and **DELIVERED**. Its
cycle-4 raw overlap-deduplicated union was 0 blocker,
0 high, 1 medium, and 2 low; after prior sealed dispositions, 0 blocker, 0 high,
1 medium, and 1 low remained unresolved at verdict collection. Topology
documentation remediation is exact `268d35a`; signal/output-lifecycle
remediation is integrated at exact `aa60db15d016cf97674459a4af66318a18b762ac`,
tree `278fa365e24504452f8d111a7b08bc49e2aed164`. Exact cycle-5 candidate
`27c75f4365af92759686402574d310ada596a923`, tree
`5e40b24259d76196d573f752258c9a764b53f990`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. Three fresh formal
cycle-5 reviews each reported 0 blocker, 0 high, 0 medium, and 0 low findings;
the deduplicated union is zero. Review-exempt seal
`20640843f49faf3de1b208bc6e8ee49ff0ff9c94`, tree
`33818a48d3ef4f000789d10574ee2024de95cb29`, passed feature benchmark-
evidence run `32923421739`, but feature CI `32923421679` failed solely because
exact-1.94.1 Quality Linux Clippy rejected one test-helper `needless_continue`;
no integration occurred. Exact one-line test-only remediation and cycle-6
replacement candidate `831d38c8da72b849704ef3ab508588a9d0499c5f`, tree
`a92acc141cab42061614a2e6f0f9d1f240325e2f`, passed the complete replacement
gate. Three fresh formal cycle-6 tracks each reported 0 blocker, 0 high,
0 medium, and 0 low; the deduplicated union is zero. Review-exempt delivered
seal `bacc5c3dbc2bf094cca12102030d21f468f11e7a`, tree
`da3183a3368273c2b34324a5f33266dfe5644a0d`, passed exact feature CI
`32925681006`, feature benchmark-evidence `32925681009`, main CI `32926242609`,
and main benchmark-evidence `32926242564`. `main` was fast-forwarded without
force from `1de3b7eddf6a4d9046d48098defecf6bfa336442`; each benchmark run retains
exactly two unexpired exact-SHA artifacts for 90 days. The delivered count is
twenty-nine and M03 remains in progress. This delivery makes no product-
performance, speed, latency, memory, binary-size-improvement, catalog-
equivalence, compatibility-promotion, or fx-equivalence claim. The final
delivery-record commit is documentation-only and review-exempt; its own exact
feature and `main` workflows will be reported at handoff rather than claimed
here. Its exact top-level `models [--json]` boundary is
[`models-cli.md`](models-cli.md), and its review ledger is
[`m03-models-cli-review-01.md`](reviews/m03-models-cli-review-01.md). It starts
from exact delivered base
`1de3b7eddf6a4d9046d48098defecf6bfa336442` and the pinned fx observation
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Local provider-neutral core,
bounded native, and CLI components are
`a6c6ff333176689b0c53bcf35070e9d59afd1b28`,
`7c966b23d75a880a23d49e1e6ba9780e512e84b8`, and
`e84ed2a46b1ac5fe7428414375609af562c65105` respectively. Core owns only validated
provider-neutral available-model/access/result/error values and an object-safe
catalog trait. Native owns process credentials plus Gateway access/fallback,
deadline, parsing, bounds, ordering, and one fixed GET implementation. The CLI
remains a thin current-thread Tokio host and owns rendering/output bounds.
Exact cycle-1 candidate `6277aa3`, tree `b5e2445`, passed its complete local
gate but was rejected with two medium and six low findings. Local remediation
at `02c9f86`, `d2890c3`, and `06c9408` raises focused native evidence to 36 and
CLI unit evidence to 18. Exact cycle-2 behavior candidate
`2ea9d94374c4dd18f43255af785ee31088126c56`, tree
`3a948b2950d870a9cabe479bc6c3889dd5a13a3b`, passed the complete exact-toolchain
replacement gate but was rejected with a deduplicated union of one high, one
medium, and one low finding. Arbitrary-number, async DNS/no-runtime, and fail-
closed resolver remediation is composed at `9cf8c74`, `8187b12`, and `499af85`.
Pre-review gate attempt `c01139811685ae73031ed6f6cbd771e4ff636714`, tree
`4ac4e5bd67b98900159aec1772f75d6003ba1d70`, was rejected because synchronous
system-DNS discovery remained inside timed request polling. Bounded eager
snapshot and per-runtime absolute-name resolver remediation is composed at
`d9922ef` and `e5248b1`; private resolver evidence is now 6/6. Exact cycle-3
behavior candidate `2cecc921e48396e81ab6f434007a7ec8e3e890b5`, tree
`8c0d235355582d92aaed6fcca7c1862982494e20`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1. Formal cycle-3 correctness/API reported
0 blocker, 0 high, 0 medium, and 2 low findings; network/error lifecycle
reported 0 blocker, 0 high, 1 medium, and 1 low; performance/concurrency was
green at 0/0/0/0. The deduplicated union is 0 blocker, 0 high, 1 medium, and 3
low, so the candidate is rejected. The medium is request-poll-time Hickory RNG
whose entropy failure could panic and abort the release process. The three lows
are stale catalog topology in `docs/architecture.md`, stale catalog topology in
`docs/ai-gateway-credentials.md`, and stale slice status in the M03 checklist
below. Exact documentation remediation
`f80bd0560e49306ce56093ea667aa45a22b2c6dd`, tree
`d6d3f354fa1eca6221d482567f9128ac6f73a3bc`, corrects all three lows. Exact
native remediation `b6cf4cbc01ba2470b7aef77b96d5793ad95f6b0d`, tree
`e72f7dd6b4f3306faee5d0d3fc8483c63f4fd24a`, replaces request-polled Hickory
network resolution with custom bounded DNS and construction-time fallible
query-ID entropy. A read-only preflight before the complete cycle-4 gate then
found 0 blocker, 1 high, 0 medium, and 0 low on that exact remediation: Android
catalog construction still called Hickory `read_system_conf`, whose
uninitialized `ndk_context` panic would abort the release process. This was not
a formal cycle-4 review, and cycle 4 remained unsubmitted at that checkpoint.
Exact remediation
`bd4746197736004f3a02c4c910c569444e4653e7`, tree
`b8d8bb1ebda0c78a7f4ba353fbe2cc03d83f64f3`, makes Android fail closed through
the existing redacted unavailable result without calling a DNS or platform
loader; Apple/Windows and generic-Unix loading are unchanged. Focused exact-
toolchain checks are green for private resolver 14/14, manifest 5/5,
formatting, and warnings-denied native Clippy. The Android standard-library
target was unavailable locally, and the foreign Linux HTTP build retained its
existing `aws-lc-sys` C-sysroot blocker before product Rust. Exact cycle-4
behavior candidate `57d2ac2a3cc562763739f49642e6fdd172f036e8`, tree
`d30bb656dfe52e15858df9d4e52a301cb61da8ce`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1, but three fresh formal reviews rejected
it. Its raw overlap-deduplicated union was 0 blocker, 0 high, 1 medium, and
2 low; after prior sealed dispositions, 0 blocker, 0 high, 1 medium, and 1 low
remained unresolved at verdict collection. Exact topology documentation fix
`268d35a` and signal/output-lifecycle remediation `aa60db1`, tree `278fa365`,
are integrated. Exact cycle-5 candidate `27c75f4`, tree `5e40b24`, passed the
complete exact-1.94.1 replacement gate without fallback. Three fresh formal
reviews each reported 0 blocker, 0 high, 0 medium, and 0 low findings; their
deduplicated union is zero and the behavior candidate is **GREEN**. Exact
review seal `2064084`, tree `33818a4`, passed feature benchmark run
`32923421739`, while feature CI `32923421679` failed solely on the test-only
Linux `needless_continue` lint. Exact test-only cycle-6 replacement `831d38c8`,
tree `a92acc14`, passed the complete replacement gate and three fresh formal
reviews with a zero-finding union. Delivered seal `bacc5c3`, tree `da3183a`,
passed exact feature CI `32925681006`, feature benchmark-evidence `32925681009`,
main CI `32926242609`, and main benchmark-evidence `32926242564`. `main` was
fast-forwarded without force from `1de3b7e`; both benchmark runs retain exactly
two unexpired exact-SHA artifacts for 90 days. The delivered count is twenty-
nine and M03 remains in progress. No product-performance, size-improvement,
compatibility-promotion, catalog-equivalence, or fx-equivalence claim is made.
The retained
`web_fetch` review lineage begins with pre-review
gate record
`0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local Rust
1.94.1, integrity, dependency, baseline portability, WASI, and release-binary
gate. Formal cycle 1 is **NOT GREEN** on exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. Correctness/API reported 0
blocker, 0 high, 2 medium, and 3 low findings; lifecycle/robustness reported 0
blocker, 1 high, 3 medium, and 0 low findings; performance/concurrency reported
0 blocker, 0 high, 3 medium, and 2 low findings. Its remediation passed the
complete replacement local gate. Formal cycle 2 is **NOT GREEN** on exact
candidate `6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`. Correctness/API reported 0
blocker, 0 high, 0 medium, and 2 low findings; lifecycle/robustness reported
zero findings at every severity; performance/concurrency reported 0 blocker,
0 high, 2 medium, and 1 low. The deduplicated union is 0 blocker, 0 high, 2
medium, and 2 low findings. Exact isolated production remediation component
`6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, implements the raw DNS and resolver-
snapshot corrections. Exact composed cycle-2 remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, workflow, integration, or delivery claim;
formal candidates are identified only by exact-SHA review results. Formal
cycle 3 is **NOT GREEN** on exact candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 1 low; network/HTTP lifecycle reported
0 blocker, 0 high, 1 medium, and 1 low; performance/concurrency reported zero
findings at every severity. The deduplicated union is 0 blocker, 0 high,
2 medium, and 1 low: blocking per-query entropy inside the total deadline,
missing cancellation/deadline authority at native pre-effect boundaries
between sequential network phases, and one duplicated cancellation-waiter
finding repeated across the two non-green tracks. The exact candidate is
rejected. The corrected boundary snapshots a random query-ID seed at transport
construction, derives IDs from an atomic per-query sequence, retains fixed
hostname-unavailable state when construction prerequisites fail while allowing
literal-IP bypass, reuses one bounded cancellation waiter across permit, DNS,
HTTP, and body waits, and checks the same absolute deadline plus cancellation state
before native A, AAAA, TCP, HTTP, and body effects. The final synchronous
boundary checks both directly without another waiter. Exact isolated production
remediation component
`9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only native
`web_fetch.rs`. It implements the construction-time 32-byte key, `AtomicU32`
counter and bounded SHA-256 query-ID derivation, native-effect deadline checks
before and after each await, and one cancellation owner. Exact isolated
independent-evidence commit `3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
only native `web_fetch_http.rs`; exact 13/13 checks prove exactly one
cancellation wake, a cancelled result, and pending owned-work drop/release
across bounded and raw seams without sleep or network. This remediation record
makes no replacement-gate,
formal-review, workflow, integration, or delivery claim; formal candidates are
identified only by exact-SHA review results. Native Linux HTTP compilation
remains an
exact-CI requirement because the macOS cross-host lacks the target C sysroot.
Feature workflows are not claimed by the local gate.
Formal cycle 4 is **NOT GREEN** on exact candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 2 low findings; network/HTTP lifecycle was
green at 0/0/0/0; performance/concurrency reported 0 blocker, 0 high, 1
medium, and 0 low. The deduplicated union is 0 blocker, 0 high, 2 medium, and
2 low, so the exact candidate is rejected. The replacement must stably
deduplicate admitted A/AAAA addresses in first-seen order before HTTP-client
construction and apply the configured connect timeout to truncated-DNS TCP
replay, subordinate to cancellation and any earlier overall deadline. Exact
isolated production remediation component
`9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, changes only native
`web_fetch.rs`. Exact isolated independent-evidence commit
`408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, changes only native
`web_fetch_http.rs`; its deterministic same-poll authority regression makes no
native-DNS proof. Exact composed code/evidence precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`, contains both. This remediation
record makes no replacement-gate, formal-review outcome, candidate, workflow,
integration, or delivery claim; formal reviewer reports identify the exact
candidate they reviewed.
Exact composed cycle-4 remediation precursor
`892a52267e7ccf478e9ed567875dc95912be5412`, tree
`da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
full-workspace, focused, compatibility, dependency, portability,
documentation, diff/unsafe, and release-smoke checks are green. This gate
record makes no formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim; reviewer reports identify the
exact candidate they reviewed.
Formal cycle 5 is **NOT GREEN** on exact candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`. Correctness/API reported
0 blocker, 0 high, 0 medium, and 1 low finding; network/HTTP lifecycle reported
0 blocker, 0 high, 1 medium, and 0 low; performance/concurrency reported
0 blocker, 0 high, 0 medium, and 1 low. The timer-accounting low is duplicated
across correctness and performance, so the deduplicated union is 0 blocker,
0 high, 1 medium, and 1 low; the exact candidate is rejected. A ready DNS TCP-
connect result could escape when the configured connect deadline became due
during the same effect poll. The replacement must preserve cancellation and
outer-deadline precedence, then reject an expired connect deadline before
accepting either a ready success or error.
Exact isolated source remediation
`cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
`web_fetch.rs` and implements that boundary. Exact composed code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, or delivery claim.
Exact composed cycle-5 remediation precursor
`8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
`5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 992-test, focused, compatibility, dependency, portability, Node,
documentation, diff/unsafe, locked-release, and five-smoke checks are green.
This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.
Formal cycle 6 is **NOT GREEN** on exact candidate
`5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 0 low findings; network/HTTP lifecycle and
performance/concurrency each reported zero findings at every severity. The
deduplicated union is 0 blocker, 0 high, 1 medium, and 0 low, so the exact
candidate is rejected. The non-WASM `web-fetch-http` surface and unconditional
native query-ID hashing exceeded the direct `sha2` dependency's Linux/macOS-
only scope: FreeBSD and Windows lacked the dependency, while Rust 1.94.1's
`wasm32-wali-linux-musl` target accidentally received it through its Linux
target OS. Foreign cross-compilation stopped at the `aws-lc-sys` C-sysroot
boundary before it could expose that Rust manifest defect. Exact source
remediation `e6eba937423714bc96a7c399b4823c6deda22da7`, tree
`bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, moves the one direct `sha2` entry
to the existing `cfg(not(target_family = "wasm"))` dependency table, adds a
parsed-manifest regression, and updates the behavior documentation. Its exact
1.94.1 source checks are green. This remediation record makes no replacement-
gate, formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim.
Exact cycle-6 remediation precursor
`31e7f11870a86795f9d98831d8405714b49b989e`, tree
`68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 992-test, focused, Python, pinned-fx, dependency, portability, target-
matrix, Node, documentation, diff/unsafe, locked-release, and five-smoke checks
are green. This gate record makes no formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim;
reviewer reports identify the exact candidate they reviewed.
Formal cycle 7 is **NOT GREEN** on exact candidate
`47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 0 low findings; network/HTTP lifecycle and
performance/concurrency each reported zero findings at every severity. The
deduplicated union is 0 blocker, 0 high, 1 medium, and 0 low, so the exact
candidate is rejected. Maintained bounded-admission prose made special-use DNS
names ineligible, but the lexical denylist omitted `.arpa`; `ipv4only.arpa`,
`resolver.arpa`, and reverse descendants such as `10.in-addr.arpa` could
prepare network authority before DNS. Reviewers reported no lineage defect,
and the absence of a pre-review seal is protocol-correct. Exact root
remediation `0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
`66527a146dc87b1aa9f3b7ee99385efb8022a342`, implements the allocation-free
terminal-label correction and deterministic evidence. Isolated remediation
`8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` has the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim.
Exact cycle-7 remediation precursor
`b365cf7704bacb378b280f25d702896dcc216e0c`, tree
`8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 995-test, focused, Python, pinned-fx, dependency, portability, target-
matrix, Node, policy-evidence, documentation/link, diff/unsafe, locked-release,
and five-smoke checks are green. This gate record makes no formal-review
outcome, candidate, workflow, integration, delivery, performance, or fx-
equivalence claim; reviewer reports identify the exact candidate they reviewed.
Formal cycle 8 is **NOT GREEN** on exact candidate
`be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`. Correctness/API and network/HTTP
lifecycle each reported 0 blocker, 0 high, 1 medium, and 0 low findings;
performance/concurrency reported zero findings at every severity. The
deduplicated union is 0 blocker, 0 high, 2 medium, and 0 low, so the exact
candidate is rejected. A valid truncated UDP reply with a partial resource-
record tail could fail strict full decoding before TCP replay, while a
structurally parseable truncated reply with an invalid response tuple could
construct bounded replay before validation. Exact root remediation
`9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
`fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, validates the capped leading UDP
header and question before truncation replay while retaining strict full decode
for non-truncated UDP and TCP. Tree-identical isolated remediation
`38a36725559fe962e6f649e07cc5de154c940257` records the same correction and
deterministic evidence. This remediation record makes no replacement-gate,
formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim.
Exact cycle-8 remediation precursor
`d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
`a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 998-test, focused, Python, pinned-fx, dependency, portability, target-
matrix, Node, protocol-ordering, documentation/link, diff/unsafe, locked-
release, and five-smoke checks are green. This gate record makes no formal-
review outcome, candidate, workflow, integration, delivery, performance, or
fx-equivalence claim; reviewer reports identify the exact candidate they
reviewed.
Formal cycle 9 is **NOT GREEN** on exact candidate
`16c7287888277f14e36ce47712f017d3bb7022da`, tree
`dfe170664851281bee0324b0ed2acb97575cd49d`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 1 low; network/HTTP lifecycle reported
0 blocker, 0 high, 0 medium, and 2 low; performance/concurrency reported zero
findings at every severity. The overlapping documentation reports deduplicate,
so the union is 0 blocker, 0 high, 1 medium, and 1 low, and the exact candidate
is rejected. Strict non-truncated UDP and TCP decoding accepted a valid declared
DNS message followed by undeclared trailing bytes. Maintained summaries also
overclaimed that the count-implied minimum preceded every UDP/TCP decode,
although a validated partial `TC=1` UDP reply intentionally bypasses that full-
message check before replay. Remediation requires decoder exhaustion on the
strict complete paths, preserves the bounded partial-truncation path, and adds
exact-message, trailing-byte, and zero-replay regressions. This remediation
record makes no replacement-gate, formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim.
Exact cycle-9 remediation
`a184eba40470d210ba64e717649dc425200c0442`, tree
`094a90e357f42fda5e7de3929124d3db5b9cc937`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 1,000-test, focused 36/15/14/6 plus manifest 1/1, 131-test Python,
pinned-fx, exact dependency/audit, portability/edge-matrix, WASI/Node,
protocol-exhaustion, official-link, documentation 74/99/535/378/0, exact
diff/unsafe, locked-release, five-smoke, and no-XDG-root-creation checks are
green. The foreign all-feature native build retains the established
`aws-lc-sys` C-sysroot boundary, so native HTTP compilation remains an exact
CI obligation. This gate record makes no formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim;
reviewer reports identify the exact candidate they reviewed.
Formal cycle 10 is **NOT GREEN** on exact candidate
`698f9d924fe363014dc97971ae60781297d32c2c`, tree
`08a7d2b8af02b7cb06ec6961dc88192b93aa6658`. Correctness/API reported
0 blocker, 0 high, 0 medium, and 0 low; network/HTTP lifecycle reported
0 blocker, 0 high, 0 medium, and 0 low; performance/concurrency reported
0 blocker, 0 high, 0 medium, and 1 low. The deduplicated union is therefore
0 blocker, 0 high, 0 medium, and 1 low, so the exact candidate is rejected.
The direct non-WASM `sha2` dependency was unconditional although it is used
outside `web_fetch` only by Linux/macOS default copy/session implementations.
Focused default compilation narrowed the review's broad premise: those two
targets must retain the edge, but FreeBSD/Windows default builds unnecessarily
carried it with `web-fetch-http` disabled. Remediation uses disjoint target
declarations, retaining required Linux/macOS `sha2`, making it optional for
other non-WASM targets through `web-fetch-http`, and omitting it on WASM.
Parsed-manifest and resolved direct-tree evidence proves that matrix. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim.
Exact cycle-10 remediation
`4d5fa4f1e3a7b2c53b5fad360cf3c85a83a021aa`, tree
`ce79fce278903196858ed47de5c22b6535beb08b`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The parsed-
manifest 2/2 regression, host default check, metadata, exact direct-tree target
matrix, required formatting, all-target/all-feature warnings-denied Clippy,
workspace tests, doctests, diff check, and locked-release help smoke are green.
The 319,152-byte release binary has SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.
Formal cycle 11 is **GREEN** on exact candidate
`298be5fdf0713c76155c8fe834c667d20f0cf1f9`, tree
`8f636bd92922d9e4d1610443253e82a3c0c36268`. Three fresh isolated read-only
reviews each reported 0 blocker, 0 high, 0 medium, and 0 low for correctness/
API, network/HTTP lifecycle, and performance/portability. The deduplicated
union is zero at every severity. Reviewers independently reproduced strict DNS
decoder exhaustion and the exact Linux/macOS 1/1, FreeBSD/Windows 0/1, and
WASM 0/0 default/feature `sha2` matrix. This review seal by itself makes no
workflow, integration, delivery, performance, compatibility, or fx-equivalence
claim. Later delivery seal `aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
`633ddd44406e22f373962c6a2ec965eae4b9cbdb`, passed exact feature CI
`32874471757`, feature benchmark-evidence `32874471812`, main CI `32875016066`,
and main benchmark-evidence `32875015892`; both benchmark runs retain two
nonexpired exact-SHA artifacts. Every production and explicitly injected/custom
host contains
thirteen alphabetical tools, while its descriptor-backed workspace set remains
twelve tools using one original descriptor plus eleven clones because
`web_fetch` is rootless. The delivered count is twenty-seven and M03 remains in
progress. Native `edit_file` is delivered on
final documentation record `719a9bded86fd7ce394d482798b9064c736f43ab`.
Exact feature CI `32651168514` passed all six jobs, and feature benchmark
workflow `32651168515` passed both jobs with two nonexpired exact-SHA artifacts.
`main` was fast-forwarded without force from behavior documentation seal
`c1268fdf463e11242b7b916add70675ae91ed115` to the final record; exact main CI
`32651488265` passed all six jobs, and main benchmark workflow `32651488282`
passed both jobs with two nonexpired exact-SHA artifacts. Native `delete_file`
is delivered as the twenty-second bounded slice on replacement documentation
seal `fe56f4c57ef18f87c742340a6060dc56b91f00f9`. Exact feature CI
`32665295323` passed all six jobs, and feature benchmark workflow `32665295321`
passed both jobs with two nonexpired exact-SHA artifacts. `main` was fast-
forwarded without force from `719a9bded86fd7ce394d482798b9064c736f43ab`
to the seal; exact main CI `32665564381` passed all six jobs, and main benchmark
workflow `32665564382` passed both jobs with two nonexpired exact-SHA artifacts.
Formal cycle 1 was
**NOT GREEN** on exact candidate
`7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf`; remediation, replacement
same-SHA adversarial review, and delivery remain pending. Exact remediation
`60e81a633557bc90aca01e3579782340c7c154c9` passes the complete replacement
local gate. Tree-identical cycle-2 candidate
`88026f10ed8c194c7160a754f226241c276579fc` is **NOT GREEN**:
performance/concurrency is green, while correctness/API and filesystem/
robustness found three overlapping medium defects and correctness found one
additional low Rustdoc mismatch. Remediation and another fresh same-SHA cycle
remain pending. Exact cycle-2 remediation
`225e9617a8a8f469d663693b61cc4f9b97af8094` passes the complete replacement
local gate. Tree-identical cycle-3 candidate
`24f851d2d3db21735124729bb1b0a14adf7ae864` is **NOT GREEN** with two
low findings: validation-site `EROFS` taxonomy and missing retained hostile-
umask evidence. Exact remediation
`77884a9fceed6268cbdbec1310de3f94a9c5a230` passes the complete replacement
local gate. Tree-identical cycle-4 candidate
`0b732d2746d5c821a5294901f8b4cc641bc98530` is **NOT GREEN** across all
three tracks with the same single medium definitive-unlink-failure cancellation
finding and no others. Exact remediation
`4273de513007175be94829aef85aaaa0d09bc02c` passes the complete replacement
local gate. Tree-identical cycle-5 candidate
`8575354542803f5e8ba8faf311e7524ed87eacba` is green with zero findings in
all three fresh tracks. First documentation seal `9e2a2764420519a94e11986a758592b442faa65d`
passed exact feature benchmark `32663557187` with both jobs and two exact-SHA
artifacts, but CI `32663557182` failed only its aarch64 Linux job when the
filesystem reused an unlinked same-type test-fixture inode; the other five jobs
passed. Exact test-only remediation
`c6744ab5416fc4bde330d09f59dd507bd9991d72` passes the complete replacement
gate without changing production behavior. Tree-identical cycle-6 candidate
`9e817beb92b14ce718c9c6a2b35637fb6fa2cf7e`, tree `d63a92f`, is green with
zero findings in all three fresh tracks. Replacement seal `fe56f4c` completed
the exact feature and `main` delivery gates recorded above. The remaining
native tools, CLI ownership, and Milestone 03 completion boundary remain
pending.
The twenty-third bounded slice, native `rename_file`, is **DELIVERED** from
exact delivered base
`3d76f2e844312e7f3e809524cb72c1a7957975ff`. Its two-endpoint typed
authority, regular-file-only absent-destination semantics, one no-replace
rename boundary, bounded two-parent durability, explicit race limitations,
parallel ownership, and fresh same-SHA review protocol are frozen in
[`rename-file.md`](rename-file.md) and
[`m03-rename-file-review-01.md`](reviews/m03-rename-file-review-01.md). The
base is green under exact feature CI `32665981665`, feature benchmark
`32665981641`, main CI `32666261656`, and main benchmark `32666261525`; both
benchmark workflows retain two nonexpired exact-SHA artifacts. The frozen
contract commit is `19cad7d10a8fc885e2e70a7345fc0ba27d76872a`; its exact
benchmark workflow `32667647846` is green with both jobs and two exact-SHA
artifacts, while exact contract CI `32667647822` was cancelled when a later
feature push superseded it and is not claimed as green. Production composes at
`d8f73676fcfce2cead385fa5b36598da989abe8f`, and independent evidence
composes at `1dab9a0dfcb4ec2d204625c744171ae923cca458`. Exact composed local-gate
precursor `43847fe5fd405e8b1d28808f0495dac859ebab15`, tree `80cb9a1`, is green
with the full Rust, focused, Python, compatibility, dependency, portability,
documentation, diff, and release-smoke evidence recorded in the review. Three-
track review candidate `2bc4f9a8ad809cd38a6b7b36488b27bf9bd531f6`, tree
`44558a0e88019ad9063234642c08097b4123c5f2`, is **NOT GREEN** in all three
tracks. The consolidated findings require retained terminal replacement-race,
`EINTR`/errno, postcommit, sync-bound, late-cancellation, and moved-parent
evidence, plus a low documentation correction for final directory replacement.
Exact remediation `a3491cf8d5e6c388c896374e768794d06bf7be0b`, tree
`0b195bdf29e7873a4d77169ec4d031491b1b336a`, expands the private suite to 15
tests for that matrix, clarifies the directory race, and passes the complete
replacement local gate recorded in the review. A tree-identical cycle-2
candidate `4f224a5447a61a76a3cdea5ced035c164240c02c`, tree `cb75dca`, is green
with zero findings in all three fresh tracks. First seal `a03a57b` passed exact
feature benchmark `32671805335` with both jobs and two exact-SHA artifacts;
feature CI `32671805412` was cancelled after reproducing an unrelated
pre-existing Linux session-lifecycle fixture deadlock. Exact test-only
remediation `2c771ed`, tree `5de94a6`, passes the complete replacement local
gate without changing production or rename behavior. Tree-identical cycle-3
candidate `5cc1523`, tree `99b88ec`, was not green: correctness/API was green,
while filesystem/robustness and performance/concurrency found the same
device/inode-reuse gap. Exact remediation `4cbd46f`, tree `35f531e`, retains a
non-reading source descriptor through postcommit identity verification and
passes the complete replacement local gate. Tree-identical cycle-4 candidate
`1337980`, tree `ab2bdc2`, is green with zero findings in all three fresh
tracks. Replacement seal `7cb5ef9` passed exact feature CI `32675233513`, feature
benchmark `32675233542`, main CI `32675562978`, and main benchmark
`32675562956`; each benchmark run retains exactly two nonexpired exact-SHA
artifacts. Native `rename_file` is delivered as slice twenty-three. The
remaining native tools, CLI ownership, and Milestone 03 completion boundary
remain pending.
Final rename documentation record
`226040780eb14dd72e86d0a002dc4bf61ba2ddfc` is green under exact feature CI
`32675981622`, feature benchmark `32675981593`, main CI `32676296870`, and main
benchmark `32676296945`; both benchmark workflows retain exactly two
nonexpired exact-SHA artifacts. The twenty-fourth bounded slice, native
`copy_file`, is **DELIVERED** from that exact delivered base. Its
two-endpoint typed authority, 16 MiB binary-safe streaming bound, absent-only
atomic publication, source-stability and destination-integrity checks,
destination-parent durability, explicit race limitations, parallel ownership,
and fresh same-SHA review protocol are frozen in
[`copy-file.md`](copy-file.md) and
[`m03-copy-file-review-01.md`](reviews/m03-copy-file-review-01.md).
Documentation-only contract commit
`6021fb0d6b1cf668e1a339a2cd2f60ead8d555dd` passed exact CI `32677160680`
and benchmark workflow `32677160652`; the latter retains exactly two
nonexpired exact-SHA artifacts. Production and independently owned evidence
compose through the first formal candidate `38d0d801caf1174d6df951a03d5843d6c217eb1a`,
tree `f0eadf23e4cdfa6613f866ef5923806a8474cb0e`, which is **NOT GREEN** in all
three cycle-1 tracks. Correctness/API and filesystem/robustness share a medium
initial-stage cleanup-ownership/residue finding. Performance/concurrency found
a medium missing postcommit source-parent rewalk and low missing exact
serialization/allocation evidence. Focused checks passed, but remediation, the
complete replacement local gate, a fresh three-track same-SHA review cycle, and
delivery remained pending at that checkpoint. Exact remediation
`53f4ee947c82033a08a2ff943f23f52c475189d7`, tree
`4bdb07a30950584d71260e70e263aafcccfff710`, closes the reported gaps with
immediate post-create cleanup ownership, uniform precommit cancellation
ordering, a fresh cancellation-ignoring postcommit source-parent rewalk, and
exact serialized-bound/constant-buffer evidence. Its complete replacement gate
is green across 25 private, 24 direct, five engine, seven host, and one core
focused tests; 834/882 discovered tests with zero benchmarks; full Rust,
Python, dependency, compatibility, cross-target, active WASI, documentation,
diff, release-hash, and CLI-smoke checks. A tree-identical cycle-2 candidate,
`ad4af0c2c642cc315724a3515bacd9aa70cbe17f`, tree
`9e09fd7ba5b486847b8302629193f3e665831d81`, is green with zero findings in
all three fresh correctness/API, filesystem/robustness, and performance/
concurrency tracks. Reviewers independently verified the immutable SHA/tree in
clean detached worktrees and passed the applicable 25 private, 24 direct, five
engine, seven host, one core-contract, and focused warnings-denied Clippy
checks. Exact feature/main delivery remained pending at that checkpoint.
First feature seal `16b92ef1a409fdca78ddb86ce4ae7879b89e65d6`
passed benchmark workflow `32683596971` with exactly two nonexpired exact-SHA
artifacts, while CI `32683596986` passed four native matrices and dependency
policy/audit but failed its quality-job Clippy step on two Linux-only
`unnecessary_wraps` diagnostics for the intentionally fallible-parity ACL
shims. Exact portability remediation `bb21c7aa91554b8958c69b15c2b93dba7aed2755`,
tree `c7fe63b030cc1de468c7694ce7e0c67c86866ab8`, adds only scoped reasoned lint
allowances and changes no behavior. Its complete replacement local gate is
green, including Linux warnings-denied Clippy. Tree-identical cycle-3 candidate
`99ecdb3aa9051cd74d997c194c43c8cb496a7277`, tree
`145b3bee6976e42ade02a681fcd0d047a364cf5c`, is green with zero findings in all
three fresh tracks. Reviewers independently verified the immutable SHA/tree in
clean detached worktrees and passed the applicable Linux lint, 25 private, 24
direct, five engine, seven host, one core-contract, and FreeBSD/WASI checks.
Replacement documentation seal
`3bdd7cb36c2ef3be0ffcd0ac118adb39706c6be8` passed exact feature CI
`32684856309`, feature benchmark `32684856373`, main CI `32685192453`, and main
benchmark `32685192394`. Both benchmark runs retain exactly two nonexpired
exact-SHA artifacts. Native `copy_file` is delivered as slice twenty-four.
The twenty-fifth bounded slice, native `create_folder`, is **DELIVERED** from
exact delivered base
`d1a5bc24112bcede8c2d12789e763a12cf44bd4a`. That base is green under exact
feature CI `32685885104`, feature benchmark `32685885086`, main CI
`32686210561`, and main benchmark `32686210659`; both benchmark workflows
retain exactly two nonexpired exact-SHA artifacts. The frozen contract defines
one strict canonical confined `path`, existing `FilesystemAccess::Create`
authority, recursive missing-parent creation, idempotent existing-directory
success, no-follow descriptor-relative execution, requested mode `0755` with
host umask/ACL inheritance, a first-successful-or-uncertain-`mkdirat` commit
boundary, no retry or rollback, fresh postcommit path verification, and bounded
bottom-up durability. Exact frozen contract commit
`9fab189c9c1add76a38775d08f4342c6bcc7635b` passed all six jobs of CI
`32687614476`; benchmark workflow `32687614442` passed both jobs and retains
exactly two nonexpired exact-SHA artifacts. Candidate source implements and
exports the tool and composes eleven alphabetical tools with `create_folder`
after `copy_file`, one original retained descriptor, and ten clones. The
normative boundary and evidence plan are
[`create-folder.md`](create-folder.md) and
[`m03-create-folder-review-01.md`](reviews/m03-create-folder-review-01.md).
Exact precursor `ea408a1f80417475e9b08513a62e9c87b38c4e75`, tree
`7055e930accea4af645b3827b70b5343a8913888`, passes the complete local gate:
16 private, 20 direct, six engine, seven host, and one core-contract focused
tests; 877 default and 925 all-feature discovered tests with zero benchmarks;
full Rust and Python suites; dependency and compatibility checks; native macOS
execution; Linux/FreeBSD cross-target test compilation; Linux library Clippy;
WASI compilation and active Node 1/1; documentation, diff, release-hash, and
CLI-smoke checks.
Production preflight API and filesystem audits reported zero findings, but are
not the required formal three-track review. An immutable formal candidate,
fresh three-track review, feature delivery workflows, integration, and exact
`main` workflows remained pending at that checkpoint. Local execution evidence
was native macOS; Linux evidence was cross-target test compilation and library
Clippy, not native Linux execution. This makes no delivery, performance, or fx-
equivalence claim, and the delivered-slice count remains twenty-four.
Tree-identical formal cycle-1 candidate
`8ce899acee73a6dbcc9a80b96722df7e3ba3e9f8`, tree
`065cd190e6a1d9ef065c4d1105eefeb6e32e7583`, is **NOT GREEN**: filesystem/
robustness is green with zero findings, while correctness/API and performance/
concurrency found the same low stale-local-gate documentation inconsistency and
zero production defects. Every reported maintained passage is remediated. Exact
remediation `7bc3fb99359a12320cf1e5aa8f858c1abd0776b2`, tree
`b39bd9b5ea36e3e4b5733f8cae168770e1a9f99d`, passes the complete replacement
local gate across all Rust, focused/discovery, Python, compatibility,
dependency, Linux/FreeBSD/WASI, active Node, documentation, diff, and release-
smoke checks. Tree-identical cycle-2 candidate
`6e1f885aa1e167e902b5cda729023fd7c283895e`, tree
`ac57575c3ee300050f5a92d4cae5f507fe654002`, is **NOT GREEN**.
Correctness/API and performance/concurrency are green with zero findings.
Filesystem/robustness reported two low findings and zero production defects:
checked subordinate-mount coverage lacked deterministic changed-`st_dev`
evidence, and maintained documentation overclaimed native Linux execution.
Evidence remediation composes a deterministic mixed-device identity-chain
regression without privileged real-mount or sandbox claims and corrects current
execution evidence to native macOS plus Linux cross-target test compilation/
library Clippy only. Exact remediation
`f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate: exact Rust 1.94.1 workspace formatting, all-target/all-feature
warnings-denied Clippy, full workspace tests, and two doctests; 17 private, 20
direct, six engine, seven host, and one core-contract focused tests; 878 default
and 926 all-target/all-feature discovered tests with zero benchmarks; 130
Python tests with eight expected macOS skips; byte-identical compatibility
against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef`; `cargo-deny`
0.20.2 and `cargo-audit` 0.22.2 with zero findings across 175 dependencies and
1,225 advisories; Linux/FreeBSD cross-target checks and Linux warnings-denied
library Clippy only; WASI compilation and active Node 22.22.0 evidence 1/1;
documentation inventory 70/502/352/0; clean diff, no unsafe additions, and no
Cargo manifest, lockfile, CLI, workflow, or benchmark-workload changes; and a
fresh 319,152-byte Mach-O arm64 release binary with SHA-256
`71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`
passing bare, version, help, and inert missing-path human/JSON smokes. Native
Linux execution remains pending exact feature CI. Documentation record
`9d0bacd656d09b8ff57edfbfe7cbf701af9fef1e`, tree
`b5fb1c2b5268e46793d48be1a02611381feca7c3`, and tree-identical cycle-3
candidate `c1e572eb1ac1ac39a8a53f522e74f57fd1d4f85d` retain identical non-
documentation behavior. Cycle 3 is not green: filesystem/robustness and
performance/concurrency are green with zero findings, while correctness/API
reported one low documentation-lineage finding and zero production defects.
Exact lineage remediation `12c11baa0187f530a6c088326b869991f6f627f6`, tree
`b96575b57c2c805a845294ae16b323dd1ea4ecd2`, passes the complete replacement
gate. Documentation gate record `f6f65847a47a009b5203044ce18e6f0c4253f17a`
parents tree-identical cycle-4 candidate
`a78b693e5ce45688084fe1215073e2d859f2d438`, tree
`2b913e8d65b1da518f2c148f1b9b1b6b899e1e64`. Correctness/API and performance/
concurrency are green with zero findings. Filesystem/robustness found zero
production defects and one low stale documentation-seal sentence, corrected
under the user's explicit seal-review exemption. First feature benchmark
`32699750662` is green with exactly two nonexpired exact-SHA artifacts. First
feature CI `32699750602` has its dependency-policy and all four native Linux/
macOS jobs green, but the overall workflow is not green because Linux Quality
rejected a redundant test-only conversion from `Mode::bits()`. The evidence
trace now stores platform-native `rustix::fs::RawMode`; focused macOS, Linux
warnings-denied test-target Clippy, and FreeBSD compilation checks are green.
Exact remediation `1effcbb5fd5affa1bc23df938afc7d786e5c05ea`, tree
`b5eccb193db00b39c9e029cb3e3b472283b3e6ba`, passes the complete replacement
gate across full Rust, focused/discovery, Python, pinned-fx compatibility,
dependency, portability, documentation/diff, and release-smoke evidence.
Tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with zero
findings in all three fresh tracks. Seal `e75578b` passed exact feature CI
`32702785549`, feature benchmark `32702785574`, main CI `32703303933`, and main
benchmark `32703303931`; both benchmark runs retain exactly two nonexpired
exact-SHA artifacts. Native `create_folder` is delivered as slice twenty-five,
and the integrated host had eleven tools at that checkpoint.
The twenty-sixth bounded slice, native `open_file`, is **DELIVERED** from
the exact delivered base
`e2ee11f2c728721d2aa93219b5fafa86ea15b0c4`. That base is green under final
main CI `32704202572` and final main benchmark workflow `32704202546`; the
benchmark run retains exactly two nonexpired exact-SHA artifacts
`9511626648` and `9511745538`. The frozen library-only contract accepts one
strict canonical workspace-relative regular-file path, authorizes it through
dedicated `Capability::OpenFile { path }`, and retains no-follow workspace and
target descriptors through launch completion. Linux alone has a concrete
launcher in this slice: fixed absolute `/usr/bin/xdg-open`, never ambient
`PATH`, receives `/proc/<parent-pid>/fd/<retained-fd>` with null standard I/O
and a fixed 30-second wait. Tests use an injected launcher and never open a
desktop application. The final spawn attempt and cancellation/drop abort
transition share one serialized state gate: abort-first guarantees zero launch,
while successful spawn is the irreversible commit boundary. Postcommit failure
or timeout is fixed redacted nonretryable ambiguity because an application may
already have consumed the request. Unsupported targets fail before spawn. Drop
after spawn terminates and reaps the owned helper; normal nonreentrant cleanup
joins the worker. Before publication, drop/cancel suppresses Waker delivery,
reaps the helper, drops the request and retained descriptor, and synchronously
joins. Legal inline or blocking arbitrary Wakers may force the published
`JoinHandle` to be released to avoid self-join or a cross-thread executor-lock
cycle; only callback/final bookkeeping may outlive drop, globally bounded by a
fixed 32-launch permit retained through worker return. This narrowly replaces
the original frozen no-worker-detach invariant, which contradicted legal Waker
reentrancy/blocking. The docs-only amendment is exempt from its own adversarial
review under the owner's instruction; production remediation is not.
External paths, directories, URLs, symlinks, real macOS launching, CLI behavior,
benchmark workloads, product-performance claims, and fx-equivalence promotion
remain deferred. The normative boundary and formal-review plan are
[`open-file.md`](open-file.md) and
[`m03-open-file-review-01.md`](reviews/m03-open-file-review-01.md).
Exact frozen-contract commit `6b763c4f1168963dd42087a1fdf5cf72c4212b40`
passed all six jobs of feature CI `32707583915`. Feature benchmark workflow
`32707583892` passed both jobs and retains exactly two nonexpired exact-SHA
artifacts, IDs `9512848704` and `9512966283`. This is contract-checkpoint
evidence only; those workflows do not validate the current implementation or
delivery.
Formal cycle 1 rejected exact candidate
`79e65c19330181955a0c341d62ef39778a18d36d`, tree
`481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`. Correctness/API found a medium
missing cancellation-precedence check after failed precommit operations and a
low wait/waiter evidence mismatch. Filesystem/process-lifecycle found a medium
unserialized final spawn race and the medium inline-waker self-join defect;
performance/concurrency independently reported that same self-join defect.
Candidate remediation applies cancellation ordering uniformly, serializes the
spawn/abort gate, removes the impossible postspawn waiter-establishment state,
and avoids joins across an active wake-callback tail. Deterministic candidate
regressions cover failed-operation cancellation precedence, abort-first zero
launch at the spawn gate, forced wait failure with helper cleanup, and inline
reentrant waking without panic or deadlock. A fifth regression proves drop does
not join a published worker blocked in its arbitrary wake callback.
Formal cycle 2 rejected exact candidate
`027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`. Its findings are unbounded
pre-path serialization, acceptance after the authoritative deadline, no active
system-worker cap, a detached Waker tail retaining the request/descriptor, the
frozen no-detach contradiction, numeric-fd-reuse races in proc-closure tests, a
fake launcher that was not inert, and a forced wait-failure seam that bypassed
the real shared `Err` arm. Required replacement evidence adds a very-large path,
authoritative deadline/remaining sleep, 32-permit saturation and callback-tail
retention, exact-FD closure while the Waker is blocked, identity-aware proc
tests, an inert fake launcher, and the actual shared wait-error arm. The complete
replacement candidate reached formal cycle 3 at exact SHA
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`. Correctness/API is green with
zero findings. Performance/concurrency is not green with one low because the
deadline regression stopped at the pre-probe guard and did not exercise the
authoritative clock after `try_wait`. Filesystem/process-lifecycle is not green
with one low because the no-Waker publication gap could release an ordinary
worker tail, detaching it rather than joining it; the tail remained permit-
bounded. The helper, request, and retained descriptor were already cleaned,
and there was no
production resource escape. Cycle 3 has zero blocker, high, or medium findings
and zero other findings. Candidate remediation publishes
`notification_complete` atomically when no Waker exists and adds deterministic
after-wait-probe and no-Waker normal-join regressions. Exact cycle-4 candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, passed the complete replacement
gate. Lifecycle and performance were green with zero findings. Correctness/API
found no production defect and one low maintained-documentation lineage drift,
so cycle 4 is not green. That remediation was composed into exact cycle-5
candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks found zero
production defects and the same low remaining current-lineage wording defect,
so cycle 5 is not green. That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are **GREEN**
with zero findings at every severity. Native Linux arm64 Rust 1.94.1 evidence
passed system 14/14, direct 12/12, engine 4/4, warnings-denied Clippy, and the
repeated performance matrix 70/70. Correctness also passed core serde 1/1,
macOS active unsupported behavior 1/1, and all-feature host composition 1/1.
The full local gate is green across workspace formatting, Clippy, tests,
doctests, and no-run compilation; Python 130 with eight expected macOS skips;
pinned-fx `b1774f` compatibility; `cargo-deny` 0.20.2; `cargo-audit` 0.22.2
with zero vulnerabilities; FreeBSD; WASI compilation and active Node 1/1;
documentation; and release smokes. The fresh 319,152-byte release binary has
SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
Seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
`03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
at 6/6 and feature benchmark `32738160725` at 2/2, retaining upstream artifact
`9524219365` and bootstrap artifact `9524052760`. Exact main CI `32738798417`
passed 6/6, and main benchmark `32738798415` passed 2/2 while retaining
upstream artifact `9524461989` and bootstrap artifact `9524298408`. Feature
delivery, non-force fast-forward integration, and exact `main` workflows are
complete. Native `open_file` is delivered as bounded slice twenty-six, and the
current host has exactly twelve alphabetical tools using one retained
descriptor plus eleven identity-preserving clones. This makes no product-
performance or fx-equivalence claim. At that checkpoint, the final docs-only
record was exempt from adversarial review under the user's instruction; its own
exact feature and `main` workflows remained required and are reported below.

Final delivery-record SHA
`762d70df106d40e59b599e18b1ac5c62f678927d`, tree
`909eb320e05df4d56f5bcecf0e3655e6d761f622`, passed feature CI `32740668405`
at 6/6 and feature benchmark `32740667465` at 2/2, retaining upstream artifact
`9525188220` and bootstrap artifact `9525017236`. Main benchmark `32741322179`
passed 2/2 and retained upstream artifact `9525436660` and bootstrap artifact
`9525268460`. Main CI `32741322249` was not green: five of six jobs passed,
and Quality alone failed the exact test named by concatenating
`blocked_wake_releases_request_before_publication_and_holds_` with
`permit_until_worker_return` because the immediate `exit 0` fixture could
legitimately publish before the first poll installed `BlockingWake`. This is a
test-fixture synchronization defect, not a production behavior finding. Exact
local test-only remediation
`62c2a5349bc682079c2458ccebe9f9ea9578a3c1`, tree
`b38984441b6bb470ecb4b1c69bc9a3a9984f0bb0`, adds the existing
`before_first_wait` barrier and passed the normal native Linux arm64 exact test
100/100. Exact cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, received **GREEN** correctness/API
and filesystem/process-lifecycle reviews with zero findings. Performance/
concurrency was **NOT GREEN** with exactly two low findings and zero blocker,
high, or medium findings: the unconditional `before_first_wait` rendezvous
could hang when `Command::spawn` failed before the hook, as reproduced on
native Linux arm64 with `/tmp` mounted `noexec`; and maintained current/
operative documentation tails ended at the superseded cycle-6/handoff state.
The candidate is rejected. Exact test-only code remediation
`274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the existing
`before_spawn` barrier, reached before every spawn outcome, so Waker
registration deterministically precedes publication. The normal native Linux
arm64 exact test passed 100/100, and the `/tmp`-noexec spawn-failure case passed
1/1. Production source, public API, and manifests are unchanged. This docs
correction composes atop that commit; its SHA is pending. The full replacement
local gate and all three fresh cycle-8 tracks remain pending. This executable
test-only fix is not eligible for the documentation-only exemption. This makes
no product-performance or fx-equivalence claim.

Exact documentation-correction and cycle-8 candidate
`6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
`44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the complete replacement
local gate. Cycle-8 correctness/API was **NOT GREEN** with exactly two low
findings and zero blocker, high, or medium; lifecycle and
performance/concurrency were green with zero findings. The first low found no
successful-helper witness in the normal blocked-Waker fixture, whose remaining
assertions also passed after a noexec spawn failure. The second found that the
operative documentation did not identify the known cycle-8 candidate. Exact
test-only remediation `a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
`7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, factors the shared lifecycle
assertions, makes the successful helper write and verify an exact marker, and
adds a separate deterministic missing-helper spawn-failure case. Normal Linux
arm64 focused evidence is 202/202; the failure case under `/tmp` noexec is
100/100. Production source, public API, manifests, and workflows remain
unchanged. The documentation correction and exact cycle-9 candidate SHA/tree,
complete replacement local gate, and three fresh cycle-9 reviews remain
pending. This makes no product-performance or fx-equivalence claim.

Exact cycle-9 reviewed candidate
`964c59408bda1a3793978041432b84b808b474a6`, tree
`7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the complete replacement
local gate. All three fresh correctness/API, filesystem/process-lifecycle, and
performance/concurrency reviews are **GREEN** with zero blocker, high, medium,
or low findings. Rust 1.94.1 evidence includes Linux arm64 system 15/15, direct
12/12, engine 4/4, normal split cases 202/202, noexec failure 100/100,
warnings-denied Clippy, the full Rust workspace, two doctests, Python 130 with
eight expected macOS skips, pinned-fx compatibility, dependency policy/audit,
FreeBSD/WASI plus active Node 1/1, documentation checks, and five release CLI
smokes. The 319,152-byte release binary retains SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
Production source, public API, manifests, and workflows remain unchanged. The
subsequent documentation-only green seal names this reviewed candidate and is
exempt from another adversarial cycle under the user's instruction; its exact
SHA/tree and feature/main workflows remain pending. These are regression and
delivery checks, not a product-performance or fx-equivalence claim.

The first
formal sixteenth-slice
candidate is composed through `dec98e0`, whose three review tracks were not
green. Its source and test fixes are composed in exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
green. First remote CI `32599591900` exposed the Linux removed-root gap; this
portable fix is exact behavior candidate `17f1884`, green under both executable
review tracks. Documentation seal `d3312d7` resolves its lineage finding, is
green under exact feature CI `32600292770` and benchmark evidence `32600292779`,
was fast-forwarded without force to `main`, and is green there under exact CI
`32600567094` and benchmark evidence `32600567090`.
The first
provides read-only native config/state
discovery, a fixed `ask` permission-mode report, and help/version/status CLI
behavior. The second adds synchronous read-only native loading of an exact
schema-v1 `ask` config, bounded to 64 KiB with fail-closed file and content
validation. Missing or unavailable configuration uses explicit built-in
defaults; configuration mutation is not implemented. The third adds
capability-aware tool preflight: the source-compatible default preserves the
raw `Capability::Tool` request, while a tool may prepare a normalized capability
and the exact arguments that an allowed execution receives. Core bounds the
prepared arguments at the existing exact byte limit and gives capability
serialization one total byte cap equal to that limit plus 1 KiB of fixed
headroom before policy. JSON depth and node traversal applies to the prepared
arguments and only the JSON values embedded in `Tool` or `Custom` capabilities.
Preparation is synchronous, bounded, nonblocking, effect-free trusted-host work
with immediate before/after cancellation checks; its arguments may drive only
effects within the authorized capability. Preparation failure produces a
durable generic tool error without consulting policy or exercising the tool.
The fourth slice adds the first executable native tool: a Unix-hardened,
read-only `read_file` capability rooted in an absolute workspace selected and
opened explicitly by the host. Its pure preflight accepts only a strict
`{path:string}` input, bounds that UTF-8 path at 4,096 bytes, and gives policy
and execution the same normalized workspace-relative path. Allowed execution
walks retained directory descriptors without following any component symlink,
accepts only a regular file, retains at most 8 KiB plus one overflow-detection
byte, and returns only valid UTF-8. The exact contract is in
[`read-file.md`](read-file.md). The fifth slice adds `list_files`, a
Unix-hardened, read-only, one-directory enumeration rooted in an explicit
absolute host path whose directory descriptor is retained. Its pure
preflight accepts only `{}` or a sole string `path`, defaults an omitted path to
`.`, and gives policy and execution the same normalized
`Capability::Filesystem(Enumerate)` path. Allowed execution uses retained
descriptor-relative directory and no-follow traversal, reads no child content,
and returns at most 100 sorted retained entries and 16 KiB of aggregate raw
entry-name bytes plus a truncation flag. It reads only the first extra visible
entry needed to establish truncation, so a truncated subset may reflect
filesystem iteration order rather than global directory order. The exact
contract is in [`list-files.md`](list-files.md). The sixth slice adds the first
concrete `ModelProvider`: a bounded, executor-neutral Vercel AI Gateway protocol
`0.0.1` / language-model specification `4` codec over an explicitly injected
byte transport. It projects the supported core transcript into the request
shape exercised by pinned fx, strictly reconstructs text, reasoning, local tool
calls, usage and finish events from arbitrarily fragmented data-stream bytes,
independently bounds JSON nodes as well as bytes and counts, ignores unsupported
temperature and metadata after applicable structural validation, and makes one
cancellation-aware transport call after a valid request future is polled
through startup (and zero for an unpolled, pre-cancelled, or invalid request).
Empty chunks fail, bounded no-event work yields cooperatively, and cancellation
wins same-poll terminal races. The injected host retains endpoint, HTTP, TLS,
authentication, status and retry responsibility; this slice performs no
network effect itself. Its exact contract is in
[`ai-gateway.md`](ai-gateway.md). The seventh slice supplies an optional
native-only Reqwest/Rustls HTTP transport for that injected codec. It fixes the
production URL to `https://ai-gateway.vercel.sh/v3/ai/language-model`, requires
an explicitly injected bounded bearer token, accepts plaintext only through an
explicit numeric-loopback test endpoint, and fixes proxy, redirect,
decompression, cookie, retry, timeout, active-request, status and diagnostic
policy. The concrete transport is polled on a host-owned Tokio runtime; core,
the codec and custom transports remain executor-neutral. Its exact contract is
in [`ai-gateway-http.md`](ai-gateway-http.md). Exact feature-branch review and
remote-run evidence is retained in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md).
The slice is integrated on `main` at
`508b0adbbe4447a85bd08f47095ae16c089c05d5`; exact main CI run `32535790803`
and benchmark run `32535790824` are green.

The eighth slice adds a native `FileSessionStore` for supported Linux and
macOS Unix targets. A host supplies one existing absolute root whose opened
directory descriptor is retained; the store performs no environment discovery
or root creation. It maps validated session IDs through a fixed
domain-separated SHA-256 v1 layout, strictly stores one bounded versioned JSON
envelope per record, verifies the decoded ID, and implements optimistic new and
update saves with checked revision assignment. Permanent per-session advisory
lock sidecars coordinate cooperating processes. Bounded no-follow regular-file
reads and `0600` exclusive temporary writes, file sync, same-directory atomic
rename, and directory sync fail closed without repairing corrupt or nonregular
artifacts. Its futures are inert until polled and execute synchronously on the
first polling thread. Retained data and successful transfer work are capped,
but exclusive lock wait, filesystem latency, and `EINTR` retries have no wall-
clock or attempt bound. The exact contract is
in [`session-store.md`](session-store.md). Its exact feature,
documentation-seal, and `main` checks are green, with evidence in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
It is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`; exact main CI run `32541315998`
and benchmark run `32541315997` are green.

The ninth integrated bounded slice defines a
native, executor-neutral `AskPermissionHandler` over an explicitly injected
`PermissionPrompter`. `AskPermissionHandler::new` accepts an owned concrete
prompter and `AskPermissionHandler::shared_prompter` accepts an
`Arc<dyn PermissionPrompter>`. The adapter forwards core's complete bounded
`PermissionRequest` by value without cloning, mutation, serialization,
truncation, revalidation, or traversal. Structured allow-once, allow-turn,
allow-session, and deny results map exactly to the corresponding core decision;
neither core nor the adapter caches a positive grant. Denial uses the fixed
reason `permission denied`. The zero-data prompt error maps fail-closed to only
`permission_prompt_failed` / `permission prompt failed`.

Authorization is inert until polled, the prompt future remains owned by the
adapter future, and drop supplies cancellation by dropping that prompt future.
The adapter detaches no work and supplies no second cancellation token, so an
injected prompter must not leave a detached approval operation behind. It owns
no terminal, UI, environment, filesystem, process, network, configuration, or
runtime authority. The exact contract is in
[`ask-permission.md`](ask-permission.md). The implementation and black-box tests
are present. Three fresh adversarial reviews have no confirmed open findings;
the documentation seal and exact feature-SHA workflows are green; and the slice
is integrated on `main` at `27e3f2b3ff170044732d9124ffb210beabcda206`.
Exact main CI run `32570197911` and benchmark run `32570197870` are green. The
complete lineage is recorded in the
[`ask permission handler review`](reviews/m03-ask-permission-review-01.md).

The tenth integrated bounded slice uses the existing native
`ai-gateway-http` and non-WASM gate. It owns a separate
`AiGatewayCredentialEnvironment` snapshot containing only `VERCEL_OIDC_TOKEN`
and `AI_GATEWAY_API_KEY`. A nonempty OIDC token has precedence over a nonempty
API key; unset and exactly empty values are absent; and any selected nonempty
non-Unicode, oversized, or malformed value fails closed without fallback.
Discovery consumes the snapshot and returns the selected source plus the
existing 1–4,096-byte RFC 6750 `AiGatewayBearerToken` without cloning it.
Errors have fixed missing, invalid-environment, and invalid-bearer categories
that retain and reflect no source or input. Accepted and retained data are
bounded, while process lookup may materialize a complete OS value before the
application can reject it. The exact contract is in
[`ai-gateway-credentials.md`](ai-gateway-credentials.md). This slice does not
add configuration credential fields or change core, the transport, or the CLI,
so the broader acquisition-and-configuration checklist item remains open.
Implementation, local gates,
and three fresh adversarial tracks are green at
`244e765713944b1bbe2ebca5bbbd02899c725e9f`. Exact feature CI run
`32573044224` and benchmark-evidence run `32573044159` are green for the
documentation seal at `ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. That
SHA is integrated on `main`; exact main CI run `32573320962` and
benchmark-evidence run `32573320937` are green. The review lineage is in the
[`credential discovery review`](reviews/m03-ai-gateway-credential-review-01.md).

The eleventh bounded slice advances the built-in native configuration and
current file schema to strict v2. Its exact five-field object has integer
`schema_version: 2`, permission mode `ask`, provider `vercel_ai_gateway`,
transport `ai_gateway_http`, and default model `zai/glm-5.2`. The exact strict
two-field v1 object remains read-compatible: it maps only in memory to those
fixed provider, transport, and model values, keeps observable schema version
`1`, and is never rewritten or migrated. Both schemas exclude credentials.
Models share the AI Gateway provider's 1–128-byte visible-ASCII validator;
config owns and redacts its model in debug output. Provider, transport, and
model selections are declarative only and do not compose the codec, optional
HTTP transport, runtime, credential, network, core, or CLI. Status remains
metadata-only and existing CLI bytes are unchanged. The exact
contract and review lineage are in
[`configuration.md`](configuration.md) and the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).
Production implementation, black-box tests, documentation, three fresh
adversarial tracks, and exact feature and `main` gates are green. The slice is
integrated on `main` at `a10f24edde80a225f89e6c7068ec035cb70f80a8`;
exact main CI run `32576876769` and benchmark-evidence run `32576876780` are
green.

The twelfth bounded library slice is Linux/macOS-only and gated on the
existing `ai-gateway-http` feature and non-WebAssembly targets. Its
`NativeReferenceHost` consumes an already validated `LoadedNativeConfig` and
composes `AiGatewayProvider` over either production `AiGatewayHttpTransport`
from an injected credential snapshot or an explicit trusted custom transport,
one shared retained workspace identity feeding exactly `list_files` and
`read_file`, the existing `FileSessionStore` over a separately supplied existing
root, and `AskPermissionHandler` over an injected `PermissionPrompter`. It uses
default `EngineLimits` and the default no-op event sink. Constructors are
synchronous: they make no network request, poll no prompt, touch no session
record, create no root or runtime, and start no background work. Production
construction opens its non-secret roots before credential discovery and token
handoff. A production HTTP request later requires a host-owned, driven Tokio
runtime.

The host retains the exact loaded configuration, including file-backed schema
v1 as observable version `1` with its fixed projected provider, transport, and
model values. Production exposes only the selected non-secret credential-source
metadata; the custom-transport authority override reports no native-discovery
source and exposes no secret getter. Fixed stage-only errors and host debug
output are redacted. The CLI remains byte-unchanged and thin. The
[`composition contract`](native-reference-host.md),
[`review record`](reviews/m03-native-reference-host-review-01.md), production
implementation, independent black-box tests, three fresh adversarial tracks,
and exact feature and `main` workflows are green. Its final delivery record is
integrated on `main` at `ac3984fb16dbab3adf86a949c7555ceca7c3e8df`;
exact feature CI run `32579779134`, feature benchmark-evidence run
`32579779123`, main CI run `32580066474`, and main benchmark-evidence run
`32580066485` are green.

The thirteenth bounded slice advances the strict current native
configuration to schema v3. Its exact six-field object is the strict v2 shape
plus required non-secret `credential_source: "environment"` selection.
`NativeCredentialSourceKind::Environment`, its stable `environment` name, and
the `NativeConfig::credential_source` getter expose only that closed
acquisition kind. The built-in configuration is v3. Exact strict v1 and v2
files remain readable without rewrite or migration, retain their observable
schema versions, and project `Environment` only in memory; v2 rejects the new
field as unknown. The existing 64 KiB input cap, strict duplicate and unknown-
field failures, model bounds, and redacted diagnostics remain unchanged.

The production reference-host path validates `Environment` and then consumes
the already injected `AiGatewayCredentialEnvironment`. The loader gains no
process authority. Runtime `NativeReferenceHost::credential_source()` still
reports the concrete selected `VERCEL_OIDC_TOKEN` or `AI_GATEWAY_API_KEY`
source; the trusted custom-transport path still skips discovery and reports
`None`. No token bytes, arbitrary environment-variable names, persistence,
migration, auth-source compatibility, CLI behavior, or performance claim are
added. Production implementation, independent tests, and maintained docs are
integrated. Focused and required local gates and all three fresh adversarial tracks
are green on exact behavior/finding-fix SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. Documentation seal
`5f4deac672af85fe5c0b1be50c327ddbdd55ce9a` is feature-green under exact CI
run `32582210892` and benchmark-evidence run `32582210927`. Feature-evidence
record `8755757da0da07e33af48d57f46bd9ea490b5449` is green under exact feature
CI run `32582687145` and benchmark-evidence run `32582687169`, was fast-
forwarded without force to `main`, and is green there under exact CI run
`32582978232` and benchmark-evidence run `32582978286`. The contract and
lineage are in
[`configuration.md`](configuration.md) and the
[`configured credential-source review`](reviews/m03-configured-credential-source-review-01.md).

The thirteenth slice's final documentation-only delivery record is integrated
on `main` at `f840576af241c58d1e55399e66ba92f7770cd50c`. Exact final-record
feature CI run `32583585145`, feature benchmark-evidence run `32583585148`,
main CI run `32583871385`, and main benchmark-evidence run `32583871368` are
green for that exact SHA. Per the delivery workflow and the user's explicit
instruction, that documentation-only final record was not adversarially
reviewed after the behavior was already green.

The integrated fourteenth bounded slice adds explicit
Linux/macOS native root selection and safe preparation. A
`NativeRootSelection` derives an exact state root from an injected
`NativeEnvironment` and an explicit absolute workspace. A
`PreparedNativeRoots` opens and retains the workspace first, requires the
selected XDG state base or fallback `HOME` to exist, and can create only the
fixed descriptor-relative `machine-god` or `.local/state/machine-god` suffix.
New directories use private mode, existing directories are never repaired,
and ownership, write, final privacy, no-follow, root-disjointness, and
descriptor-bound macOS extended-ACL checks fail closed. New consuming
reference-host constructors preserve those retained
identities and discover production credentials only after root preparation.
Schema v3, configuration bytes, status and every CLI byte remain unchanged;
the old reference-host path constructors and `FileSessionStore::open` remain
no-create. Production is present at `050d253`, with creation-normalization fix
`7420a3a` and selected-base normalization fix `fa5119a`. Independent regression,
core-contract, and prepared-host suites are present at `85c99a8`, `85c4193`,
and `236e3d4`, covering 2, 9, and 3 focused tests respectively. The formal
portability finding is fixed by explicit fixture modes at `f5dbbca`; the formal
macOS ACL finding is fixed by descriptor-bound rejection at `8ae17db` with its
independent regression at `041c83c`. The protective macOS HOME deny-delete
compatibility regression is `bb2a856`. The focused suites are green, including
11 core tests under default and all features; its policy and Rustdoc fix is
`fa94d8a`. All three formal tracks were green on exact behavior SHA
`f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. Documentation seal `03fa9ba`
then passed all native Linux/macOS jobs and benchmark run `32588948975`, but
feature CI run `32588948956` exposed three Linux-only strict-Clippy diagnostics.
Portable lint normalization is present at `90d8f96`, with local macOS and Linux
cross-target gates green. All three final adversarial tracks are green on exact
candidate `72cf64f63e0dfa30bc1ee21d8aca16550e819c21`. Replacement documentation
seal `f08dbd9eb2da81848b8eefb2d218006a64575835` is green under exact feature CI
run `32589778343` and benchmark-evidence run `32589778374`. Feature-evidence
record `6f66b6e5972e78ba0f0ccae06b899158d99bc864` is green under exact feature CI
`32590128235` and benchmark evidence `32590128233`, was fast-forwarded without
force to `main`, and is green there under exact CI `32590429626` and benchmark
evidence `32590429592`. This documentation-only commit is the final delivery
record; its exact feature and `main` workflows are reported at handoff. The
integrated contract and review record are in
[`native-root-selection.md`](native-root-selection.md) and
[`m03-native-root-selection-review-01.md`](reviews/m03-native-root-selection-review-01.md).

The delivered fifteenth bounded slice implements a Linux/macOS native by-ID
session lifecycle over the exact `Arc<FileSessionStore>` shared by
`NativeReferenceHost` and its engine. The caller supplies each validated
`SessionId`; default reference-host composition allocates a bounded incarnation
from OS cryptographic randomness, while standalone construction may inject an
explicitly trusted custom-host source.
Every lifecycle constructor validates exact shared-store `Arc` identity and
returns fixed redacted `MismatchedSessionStore` before entropy or filesystem
effects if an engine and concrete store do not share one allocation.
Create atomically persists an empty current-schema record at revision `1`
before returning a live session. Resume loads the current durable incarnation
and converges on the engine-canonical local state. Replay returns one validated,
bounded, point-in-time `SessionRecord` from durable storage rather than
reconstructing UI or event-stream behavior.

Reset refuses a locally live incompatible lifetime before record replacement,
then uses the permanent per-ID store lock and an exact
ID/incarnation/revision CAS to atomically publish an empty record under the same
ID, a new incarnation, checked revision `old + 1`, and turn allocator `1`. Its
preceding present-record load may create the fixed lock sidecar and its bounded
incarnation source may already have been consulted. Reset has no deletion gap
and does not revoke another process's old handle or external effects; the new
incarnation fences that handle's later saves. Missing resume/replay/reset,
duplicate create, local live state, entropy failure, conflict, corruption,
unavailability, and engine/invariant failure remain typed and redacted. Futures
are inert before poll, detach no work, and inherit the store's bounded
synchronous first-poll I/O and ambiguous post-rename directory-sync boundary.
The production API and these semantics are normative in
[`native-session-lifecycle.md`](native-session-lifecycle.md). Production,
fourteen independently owned focused tests, and one formal finding regression
are integrated and green, including the allocation-address identity portability
fix. All three adversarial tracks are green together on exact candidate
`e6a3804`. Feature record `dbba2c7` is green under exact feature CI
`32594562796` and benchmark evidence `32594562785`, was fast-forwarded without
force to `main`, and is green there under exact CI `32594846484` and benchmark
evidence `32594846476`. This documentation-only commit is the final delivery
record; its workflows are reported at handoff. `list_sessions`, lifecycle
CLI commands, migration,
encryption, and non-Unix hardening are not part of this slice.

The delivered sixteenth bounded slice adds Linux/macOS library-only
`NativeSessionLifecycle::list_sessions`. Its IDs-only `NativeSessionList`
contains at most 100 sorted unique validated IDs plus `truncated`. Each call
processes or selects at most 1,024 non-dot entries plus one fetched and name-
inspected overflow witness, and accepts/decodes at most 64 MiB of aggregate
canonical record bytes plus one transient transfer byte used only to detect
concurrent growth; all non-dot names within the scan count against its budget.
Exact canonical record names are opened no-follow, locked with the existing
per-ID protocol, and
validated against the same current schema, bounds, positive counters, and
decoded-ID/digest invariant. Canonical corruption fails the whole call as
redacted `Corrupt`; enumeration, read, and lock failures are redacted
`Unavailable`. Successful listing may create private permanent lock sidecars.

`truncated` means only incomplete bounded observation. It is not pagination and
does not imply `has_more`. Canonical filenames are sorted before validation;
result and byte caps select from that sorted scanned set, while a fired raw scan
cap can make the set filesystem-iteration-dependent. Returned IDs are sorted,
but digest filename order is not a globally first ID or semantic ranking. There
is no multi-record snapshot. A candidate that vanishes before its locked read
may be omitted and may leave its private lock sidecar. A nonregular derived lock
for a still-present canonical candidate is `Corrupt`; ordinary lock I/O is
`Unavailable`. On macOS the replacement must acquire a fresh `.` descriptor
first, then validate that exact acquired descriptor's linked identity. A stable
completed rename retains identity; removal before acquisition or validation is
unavailable; and a concurrent rename/removal may conservatively be unavailable
or observe the acquired identity without creating a global snapshot. The
future is inert before poll, performs bounded synchronous work on first poll,
and detaches nothing. It consults no live registry, incarnation source,
provider, permission handler, tool, network, workspace, configuration, or
environment authority. The exact candidate contract and non-green review lineage
are in [`native-session-listing.md`](native-session-listing.md) and
[`m03-native-session-listing-review-01.md`](reviews/m03-native-session-listing-review-01.md).
Production `0accfbf` is composed as `1bffac9`, documentation `63d589c` as
`87d7de0`, and 13 initial independent tests `1b531297` as `4b4e468`, from base
`9ada4b5`. The removed-root fix and first formal candidate are `dec98e0`. All
three first review tracks were not green. Isolated acquire-first source fix
`4b8d8b0` and isolated test hardening `446b495` are composed in the replacement
candidate, with 18 focused tests and the full all-target/all-feature workspace
suite green locally. All three replacement tracks are green on exact behavior
candidate `3fa5463`; portable behavior `17f1884` and seal `d3312d7` are green
under exact feature and `main` delivery workflows.

The current schema and store contain no authoritative summary, workspace,
title, preview, language, timestamp, latest-order, or index fields. This slice
therefore adds no rich summaries, workspace/latest filter, cursor, pagination,
CLI or slash command, and makes no fx-equivalence claim. The `sessions-json`
benchmark remains unimplemented and claim-ineligible.

The delivered seventeenth bounded slice adds Linux/macOS library-only
`file_info`.
Its strict effect-free preflight accepts only a required string `path`, bounds
the requested and normalized forms to 4,096 UTF-8 bytes, applies the same
lexical confinement as `read_file`, and prepares both
`FilesystemAccess::Metadata` policy input and exact execution arguments with
the same normalized workspace-relative path. Nonempty current-directory forms
normalize to `.` so the retained root can be inspected. A distinct metadata access kind
prevents read-content authority from being inferred from this inspection.

Allowed execution starts from the explicitly retained workspace descriptor,
acquires a fresh `.` descriptor, and validates that exact acquired linked root
identity under platform-specific Linux/macOS rules. It then opens ancestors
descriptor-relatively with directory and no-follow requirements and performs
one no-follow metadata lookup for the final component without opening it; `.`
uses one final `fstat` after liveness validation. Final symlinks report link metadata; FIFO,
socket, device, and other special objects report `other` without a blocking
open. Its exact bounded output is normalized `path`, fixed `kind`, checked
nonnegative `size_bytes`, `modified` with signed Unix seconds and validated
nanoseconds, and a nullable lexical extension for regular files only. Dotfile,
trailing-dot, multi-dot, retained-root rename/removal, concurrent replacement,
single-stat snapshot, redaction, cancellation, and inert-future semantics are
normative in [`file-info.md`](file-info.md).

The delivered reference-host extension registers exactly three workspace tools:
`file_info`, `list_files`, and `read_file`; core exposes that catalog in
deterministic alphabetical order. Prepared roots transfer descriptor clones of
the same retained workspace identity so the three tools receive three
descriptor instances: the original plus two clones. Production is present at
isolated SHA `5c2d129`; independent direct
and engine tests are present at isolated SHA `ca0091c` and compose with
production at `f228c06`, where all 34 focused tests are green. Required local
gates are green through composed precursor `0973acf`. First formal candidate
`8399ec7` was not green. Isolated finding-test hardening `7f2a292` composes as
`b69ec4b`, bringing the focused total to 36 plus five private unit tests; the
two documentation findings are corrected at `9dbd188`. Replacement local gates
are green through `d445eb3`, and all three replacement tracks are green on exact
candidate `4193ecc`. Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI run
`32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four workflows report that exact seal SHA. Benchmark success
is delivery evidence only, not a product-performance claim. This
documentation-only commit is the final delivery record, is explicitly exempt
from another adversarial review after behavior was already green, and reports
its own exact workflows at handoff.
The slice adds no CLI behavior, non-Linux/macOS hardening, content or target
reading, mutation, recursion, MIME/hash/ownership/mode/ACL/xattr reporting,
extra timestamps, compatibility/equivalence claim, benchmark, or performance
claim. Its base and parallel ownership are recorded in the
[`file_info` review](reviews/m03-file-info-review-01.md).

The delivered eighteenth bounded slice adds Linux/macOS library-only `glob_files`
from exact base `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`. Strict effect-free
preflight accepts exactly
`{pattern:string,path?:string,mode?:"matches"|"count"}`, defaults path to `.`
and mode to `matches`, rejects unknown fields, and independently bounds each
requested and normalized path and pattern at 4,096 UTF-8 bytes. Path
normalization is the `file_info` rule. Pattern normalization uses `/`, collapses
repeated separators and exact `.` segments, and rejects an empty normalized,
absolute, parent-traversing, or forbidden pattern. Backslash, square brackets,
and braces are literal. The bytewise matcher gives `?` one byte, `*` zero or
more within a component, and only an exact `**` segment zero or more complete
components. Slash-free patterns match candidate basenames recursively;
slashful patterns match paths relative to the selected search root.

Successful preflight produces exact prepared arguments with both defaults
explicit and `FilesystemAccess::EnumerateRecursive` at the normalized selected
subtree. This permission kind is distinct from one-level `Enumerate`; pattern
and mode attenuate output but do not broaden or replace the recursive scan
authority. Allowed execution reacquires and validates the retained workspace
through the same fresh-`.` liveness rule as `file_info`, then performs iterative
descriptor-relative no-follow traversal. Each directory is fully read,
validated, and sorted bytewise before processing. Hidden entries are included;
regular files and final symlinks are candidates; directories are traversal-
only; specials are ignored; and symlinks are never descended through or read.
Returned candidates use full workspace-relative paths.

Both modes complete a bounded traversal of no more than 100,000 non-dot
entries, 16 MiB of aggregate raw entry-name bytes, directory traversal depth
256, and 8,388,608 aggregate matcher-work steps. A step is charged for each
slashful candidate byte inspected while splitting, pattern-segment loop visit,
dynamic-programming cell write, and invoked component-matcher transition or
trailing-star consumption. Exactly the cap is permitted; checked overflow or
the next step is a scan-limit error. The selected root is depth 0; a depth-256
directory is scanned and its
regular/symlink children remain eligible, while attempting to open a child
directory at depth 257 is a scan-limit error. Any candidate full workspace path
over 4,096 bytes is also a scan-limit error. A fired scan cap fails either mode
without partial output. Match mode returns the longest globally bytewise-sorted
prefix under 100 paths and 16 KiB of aggregate raw path bytes and sets
`truncated` exactly when an observed match is omitted; it never skips an
omitted long path to admit a later short path. Count mode returns the exact
count. A stable tree is deterministic, while concurrent scans are not
snapshots; a `NOENT` race may omit an entry and other failures fail closed under
fixed redacted errors.

The slice extends the reference host to exactly four alphabetical workspace
tools: `file_info`, `glob_files`, `list_files`, and `read_file`. Prepared roots
distribute the original retained descriptor plus three clones of the same
identity. Its public constants, constructor errors, result shapes, cancellation
boundaries, compatibility inputs and deliberate differences, and deferred scope
are normative in [`glob-files.md`](glob-files.md). Parallel production,
independent-test, and documentation ownership is recorded in the
[`glob_files` review](reviews/m03-glob-files-review-01.md). Isolated components,
composed behavior, and local-gate lineage are recorded there: production
`a5d1399`, schema correction `f1584f5`, independent tests `948994d`, and
documentation `f2f0fc1` compose through initial local-gate precursor `60070d8`.
The first formal review at `1f5de6a` found a high unmetered matcher-work defect.
Production fix `f3fd13b` and independent regression `825bbd3` compose through
`aba2821`; the public-cap assertion and replacement local gates are green at
exact code-and-test head `4171a4a8811a98888b7e4e161281a1216564746f`.
All three replacement adversarial tracks are green on exact behavior SHA
`523df85822a27102d7e7100e274e3bad7b25494f`. Documentation seal and integrated
`main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI
`32610950593`, feature benchmark evidence `32610950594`, main CI `32611208411`,
and main benchmark evidence `32611208415`; all four report that exact seal SHA.
Benchmark success is delivery evidence only, not a product-performance claim.
Final documentation record `f6aa458bb875d6cb26565adc878703fe140916d3`
passed exact feature CI `32611623653` and benchmark evidence `32611623655`.
GitHub did not materialize workflows for its first `main` event, so
tree-identical non-behavior grep kickoff marker
`f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` passed exact feature CI
`32612424382` and benchmark evidence `32612424383`, was fast-forwarded to
`main`, and passed exact main CI `32612662260` and benchmark evidence
`32612662203`. Per the user's instruction, these documentation-only/tree-
identical records are exempt from another adversarial cycle after behavior is
green. The slice adds no
CLI behavior, external-path access, ignore or Git/subprocess behavior, content
read, mutation, dependency, benchmark workload, product-performance claim, or
fx-equivalence claim.

The delivered nineteenth bounded slice adds Linux/macOS library-only `grep_files`
from exact base `f6aa458bb875d6cb26565adc878703fe140916d3`. Strict effect-free
preflight owns all eight pinned field names: required literal `pattern` plus
optional `path`, `include`, `case_insensitive`, `mode`, `head_limit`, `offset`,
and `context_lines`. It rejects unknown or mistyped values and prepares the
explicit defaults `.`, `null`, `false`, `matches`, `100`, `0`, and `0`.
Policy receives the distinct `FilesystemAccess::SearchContent` capability at
the normalized selected path, which may resolve after approval to a regular
file or directory. Search authority is separate from `Read`, `Metadata`,
`Enumerate`, and `EnumerateRecursive`.

Allowed execution reacquires the retained workspace under the delivered linked-
root liveness rule. Selected ancestors, the selected file/directory, traversed
directories, and candidate regular files use descriptor-relative no-follow,
close-on-exec, and nonblocking operations with authoritative opened-type checks.
Traversal is iterative, includes hidden entries, fully validates and bytewise
sorts each directory, and bounds every complete descendant path before
allocation, entry-kind handling, or include matching. Stable special objects
are skipped; a raced nonblocking special open is rejected before read, and no
symlink is followed. Optional include filtering is compiled once per call,
uses the delivered bytewise glob grammar, charges complete parse and match
work, and does not prune directory traversal. Fixed literal pattern-table work
is charged before selected-root resolution. Selected-file filtering occurs
after no-follow stat classification and before content open. A slashful
selected-file rejection consumes one charged cancellation-checked include
decision. An excluded selected file consumes fixed pattern-table and include
work but no candidate, content-byte, or per-file matching work; an included file
opens and is revalidated before those latter budgets. No ignore, Git,
subprocess, external-path, or ambient discovery behavior is added.

Content matching is literal and worst-case linear, with exact byte comparison
or ASCII-only case folding and one result per matching LF-delimited line. A
candidate is eligible text only when its complete observed content is at most
204,800 bytes, valid UTF-8, and NUL-free. Oversized and non-text candidates are
skipped with disclosed aggregate statistics; other candidate failures fail the
complete call. Match excerpts are UTF-8-safe, at most 4,096 bytes, and contain
the complete first matched substring. One scan-local content buffer reads
through an 8 KiB window, grows only to the 204,801-byte per-file high-water
ceiling, and logically resets for reuse between files without exposing stale
bytes. Actual aggregate and per-file overflow witnesses remain charged.
Requested before/after context comes from the same validated logical file view,
and per-record `context_truncated` distinguishes budgeted context omission from
top-level page incompleteness.

Each mode either completes the same bounded scan or fails without partial
output under exact 4,096-byte input/path limits, head 100, offset 67,108,864,
context 5, 100,000 entries, 16 MiB name bytes, 10,000 candidates, 64 MiB
aggregate content, 8,388,608 include compile/match steps, 268,435,456 literal-match
steps, depth 256, 8 KiB aggregate result paths, 8 KiB aggregate result text,
and a 48 KiB serialized `ToolOutput`. `matches` and `files_with_matches`
return deterministic bounded pages with exact totals, scan statistics,
`next_offset`, and list-completeness `truncated`; `count` returns exact eligible-
text matching-line/file totals without pagination fields. Every emitted
`next_offset` is reusable under the accepted bound. Cancellation is checked at
fixed intervals while indexing lines and at every serialized-output trimming
iteration. Slashful candidate splitting checks at intervals of at most 1,024
candidate bytes, and both recursive and non-recursive dynamic-programming
branches route through the scan-local injectable cancellation checker.

The candidate extends reference-host composition to exactly five alphabetical
workspace tools: `file_info`, `glob_files`, `grep_files`, `list_files`, and
`read_file`. Prepared roots distribute the original retained descriptor plus
four clones of one identity. The complete provider schema, authority,
descriptor, matcher, eligibility, result, error, cancellation, race, and
deferred contract is normative in [`grep-files.md`](grep-files.md). Parallel
production, independent-test, and documentation ownership and the review gates
are recorded in the
[`grep_files` review](reviews/m03-grep-files-review-01.md). Integration kickoff
marker `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` is tree-identical to the exact
base. Exact production component
`27eec2f3c25ffecd1ba8ff3c0a4fe0129dbeeac3` and initial independent-test
component `6eaee93398de8fbf6e87e77cf4d3e7de56e2a8cb` exist. Initial integration
heads are `9057feb24fd3f24657148ca8e78198b88c9dbab4` after production and
`44e33d7e24c6650a1e375cd095eb9efae31f4e78` after the tests. Reference-host
fixture fix and focused production/test head
`bdbb677161322e249aea95a12bfb1b2169ff5b48` is green. The documentation
component is `b04151a7d958875118eebddd67526d74e2ea9526`, producing first fully
composed behavior candidate `42e4793b27902da7390dc54ef6bedb169da7e1bc`.
Lint fix and exact local gates are green at
`45ad91fa2689250c47c79d2105f5e3c261cea638`. All three first-cycle formal
tracks are **NOT GREEN** on exact candidate
`355a11a6055b0053dff80e71011d7633e8a6ce97`. Remediation and exact replacement
local gates are green at final code/test precursor
`275d263dd3c7981e66f6a0f90f3779c271eb4cc3`. First replacement candidate
`ae87bf1454b1527b2e55ed5e517c21fd7410c980` is **NOT GREEN** with one low
correctness ordering finding, one low filesystem evidence-wording finding, and
medium-plus-low performance/cancellation findings. Second production remediation
`ac5d7726411744e4f85344edf966d26a3cdb0a26` composes at `d672210`; second
documentation remediation `7ad0863885d28b7b7a1d6f89d35f525cdd2dd3fa`
produces fully composed exact local-gate-green precursor
`b498ba06fa808dc9453a7644727cf8166b6f8e87`. Formal second replacement
candidate `5aeddc1b4cb210b00cb967b938db8d5232062916` has correctness/API and
filesystem/robustness **GREEN** with zero findings. Performance/concurrency is
**NOT GREEN** with one medium repeated 204,801-byte buffer-allocation
amplification finding and two low findings: the Markdown inventory is 58 rather
than 57 files, and deterministic recursive-DP evidence was overclaimed. The
third remediation supplies injected-checker routing and a deterministic
recursive regression. Third production remediation
`8777825b1b8b8c97dd4eb4bb31c0d8dbed9a7741` composes at `ab1c133`; independent
regression `dcf57ad35150b86c84a3f6c1127d9e379f3840fc` composes at `d7526d4`;
review-findings documentation `44afb232f2b8418c0b61eec7d1dab46bbe8e3667`
composes at `f08c5f2`; lint follow-up `1f13f9ae04ee3307d13a363ed28b156d7ee2421f`
produces exact fully composed local-gate precursor
`a8f61794ee5e279558856220b5789526b908015a`. Exact Rust 1.94.1 formatting,
warnings-denied workspace Clippy, 598 non-documentation tests plus two doctests,
25 private native tests, 40 direct `grep_files` tests, four engine tests, and
diff checks are green. Exact a8f cross-target/dependency/link and compatibility/
release validators are green. Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. Isolated wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
green, and its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
in all three fresh tracks: correctness/API, filesystem/robustness, and
performance/concurrency. Exact-SHA formatting, warnings-denied workspace
Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25 private
tests, 40 direct `grep_files` tests, four engine tests, and the 58/420/270/0
documentation inventory are green. All historical findings are closed,
including the attempted-read-window storage wording. This documentation-only
seal is exempt from another adversarial review under the user's explicit
instruction. Documentation seal `0f48806310882caf3c668c72fe1b9d211cae744b`
is feature-green: CI run `32623585346` passed all six jobs and benchmark-
evidence run `32623585349` passed both jobs and artifacts, all for exact `0f`.
`main` was fast-forwarded without force from `f6ab594` to exact `0f`. Main CI
run `32623904784` is **GREEN** for exact `0f`: all six jobs and every step
passed without reruns. Main benchmark-evidence run `32623904800` is **GREEN**
on attempt 1 for exact `0f`: both jobs and every step passed, with two valid
non-expired exact-SHA artifacts retained. The `grep_files` slice is delivered;
the remaining native tools remain pending.
This final delivery record is documentation-only and exempt from adversarial
review; its own exact remote workflows are required after push and cannot be
self-recorded.
The maintained documentation must compose into the
exact behavior SHA reviewed by all three adversarial tracks. A later
documentation-only seal or delivery record is exempt from another adversarial
cycle under the user's instruction but still requires exact feature and `main`
workflows.

The candidate adds no regex, Unicode case folding, binary search, alternate
encoding, symlink-target search, context reread, CLI behavior, benchmark
workload, compatibility-status change, product-performance claim, or fx-
equivalence claim. Zig remains only the pinned upstream benchmark build input;
the product remains Rust.

The delivered twentieth bounded Milestone 03 slice is native `write_file`, frozen in
exact contract commit `3ee52fd8393bfb86f11048eaa6c624bd18a78798` from exact
delivered base `bc042536eb3a40d75ccf4d1fe52032b31defac04`.
Exact contract-only feature CI `32626410935` and benchmark-evidence run
`32626410931` are green; they are not implementation or final-feature evidence.
Effect-free preparation accepts exactly required UTF-8 `path` and `content`
strings, rejects unknown fields, independently caps the normalized path at
4,096 bytes and 256 components, raw content at 49,152 bytes, and serialized
arguments at 65,536 bytes, and prepares exactly
`FilesystemAccess::Write` for the canonical workspace-relative path. Empty and
NUL-containing content are valid. Direct execution revalidates the same strict
canonical shape, so neither provider nor direct use can widen approved
authority.

Allowed Linux/macOS execution operates only beneath the retained workspace
descriptor and requires every parent to exist. It no-follow walks parents,
records parent and target identity, rejects symlink and special targets, and
never opens or reads existing target content. It stages into one exclusive,
initially private `0600`, same-parent regular file chosen within eight high-
entropy name attempts; writes and cancellation checks use at most 8 KiB chunks.
The staged
file receives exact `0644` ordinary rwx bits for creation or the initially
observed replacement target's `st_mode & 0o777`, then is file-synced,
identity-revalidated, and published atomically at the pathname. Missing-target
creation must use `renameat_with(..., RenameFlags::NOREPLACE)` and fail closed
when unavailable; replacement uses ordinary same-parent rename. A final parent
sync completes durability. Success is exactly normalized `path` plus
`bytes_written`.

The irreversible boundary is rename. Precommit failures leave the target name
unchanged and use best-effort identity-checked temporary cleanup, with the
portable metadata-check-to-unlink race explicitly disclosed. Formal cycle 2
found that exact candidate `708f2d0` can leave retained owned residue carrying
its already-applied final mode rather than private `0600`, so the earlier
private-residue statement is not a satisfied candidate claim. After rename,
the tool completes parent sync and returns success or
nonretryable commit ambiguity rather than its own cancellation error. Creation
cannot clobber a raced target, but replacement is not inode compare-and-swap:
the final validation-to-rename race and a final-parent move outside the
workspace remain documented limitations. The slice claims neither perfect
identity-safe cleanup nor adversarial concurrent-rename confinement.

Production, independent tests, and maintained documentation had separate
owners and are composed on `agent/m03-write-file`. Exact production component
`e9b3ad8` composes as `c0d555b`; exact independent-test component `59a06a3`
composes as `c4c5ce6`; exact documentation component `9285fe9` composes as
`de46c3e`. Retained-root fixture correction `8509933`, deterministic seam
hardening component `1d30ff9` composed as `a9a7c99`, and core same-poll recovery
regression `8b5847f` close the first preformal evidence gaps. Full-pipeline
fault and phase evidence `c7fdef2` covers both mode stages, write, staged sync,
create/replace rename, staged-name tampering, traversal and final-prepublish
cancellation, real-rename parent-sync ambiguity, and device rejection. Exact
local precursor gates are green at `072bd69`: formatting, warnings-denied
workspace Clippy, 651 workspace tests, two doctests, focused native and engine
suites, Python and pinned-compatibility gates, dependency policy/audit, native
macOS execution, Linux and FreeBSD cross-compilation, WASI compilation with
active unsupported-target execution, documentation links, diff checks, and a
freshly built release CLI smoke. Exact candidate
`119938240807f8279f83e2ace65a69706e8fcfed` is tree-identical only to its
immediate parent `a7841c19b4b34cecf40e55d7cd001fd1547133c1`; precursor
`072bd69eb6f73944d1db00363da0f965f09dda9f` has a different documentation tree.

All three first-cycle formal tracks are **NOT GREEN** on exact candidate
`119938240807f8279f83e2ace65a69706e8fcfed`. Confirmed findings are high/medium
unbounded `EINTR` retries in write and sync helpers; medium missing
deterministic real-pipeline proofs for target appearance, existing-target
replacement, and final-parent postvalidation races; medium missing real-
pipeline verification-phase cancellation evidence; and low stale platform,
local-gate, and lineage claims in maintained documentation. Documentation
correction `016f8df` and code/evidence remediation `3010e6d` close those
findings with an exact cumulative 16-interruption phase bound, native
postvalidation race tests, and post-verification cancellation/cleanup evidence.
Exact replacement local gates are green at `581fe6a`: 655 workspace tests, two
doctests, focused native/engine suites, warnings-denied workspace Clippy,
Python/compatibility/dependency gates, cross-target and active WASI checks,
documentation/diff/no-unsafe checks, and a fresh release CLI smoke. Three fresh
same-SHA review tracks then inspected exact candidate
`708f2d08d72d610ca387a62a4cec1f656c188a7d`, which is tree-identical only to
its immediate formal-review preparation parent
`491496aa22aa8855717b74f6a026e8c602bb02e9`. Correctness/API is **GREEN** with
zero findings. Filesystem/robustness is **NOT GREEN** with two medium findings:
retained staged inode mode can remain more permissive than `0600`, and Linux
entropy acquisition can retry partial reads or interruptions without a finite
work/cancellation bound. Performance/concurrency is **NOT GREEN** with the same
medium entropy finding. No behavior-green SHA is claimed.

Production remediation is composed at exact
`9302ec3fa7d6e891fdc4a0c7bd8fe9b7cf8e427d`. Linux entropy acquisition now
uses direct `rustix` `getrandom` with `NONBLOCK`, at most 16 cumulative `EINTR`
results and 31 calls per 16-byte name including partial progress, and
cancellation checks before and after every call. `ENOSYS`, `EPERM`, and `EAGAIN`
fail closed as retryable `write_file_unavailable` rather than invoke a fallback
or block. The pinned macOS `getrandom` 0.4.3 path makes one `getentropy` call for
the 16-byte request and routes through the same bounds. Cleanup now makes one
best-effort `fchmod(0600)` call on the held, unpublished staged descriptor
before the existing identity-checked best-effort unlink. If mode restoration
and unlink both fail, residue can retain final mode bits.

Exact clean remediation precursor
`8432c0c6b5d5955b78a882b651a5bfec76af8814` passes the complete local gate
under Rust and Cargo 1.94.1. Formatting, workspace/all-target/all-feature
warnings-denied Clippy, workspace tests, two doctests, 30 private module tests
including 28 in the supported-platform submodule, 25 direct tests, and five
engine tests are green. Discovery reports 611 default-feature and 660 all-
feature tests with zero benchmarks. Linux, FreeBSD, and active WASI cross-gates;
cargo-deny 0.19.9; cargo-audit 0.22.2 `--no-fetch` over 1,225 advisories and 175
dependencies with zero findings; the sequential 129-test Python gate; clean
pinned-fx compatibility; the 60/429/279/0 documentation inventory; fresh
isolated locked release smoke; and diff/no-unsafe/clean checks are green. The
first Python attempt incorrectly overlapped the LTO release build and produced
one two-second timeout; its isolated 1/1 retry and the full sequential rerun are
green, proving validation-method contention rather than a product failure.

These exact local gates resolve both cycle-2 findings for formal-review
preparation, but do not retroactively make cycle 2 green or establish a new
behavior candidate. All three fresh tracks must review the same next exact SHA.
Exact gate record `9a09172ac40d7ec09ebb9fa7a4e4e21f12b2a632`
retains that evidence. Exact candidate
`db78c6407c4f603f18e2839a8a291f2de33e579c` is tree-identical to its immediate
formal-preparation parent `5ed38f3c61d3f29677f41c0b4a41468616a59c7e`.
All three fresh correctness/API, filesystem/robustness, and performance/
concurrency tracks are **GREEN** with zero findings on that same SHA.
Filesystem/robustness reran 30 private, 25 direct, and five engine tests under
Rust 1.94.1 exactly. Exact-candidate formatting, workspace warnings-denied
Clippy, workspace tests, two doctests, and diff/clean checks are green. Behavior-
green SHA is `db78c640`. Documentation seal
`bdd27ec969769d94c78efdab8e07cbd6b600ca3f` passed exact feature CI
`32633372927` across all six jobs and feature benchmark run `32633372982` across
both jobs with two nonexpired exact-SHA artifacts. `main` was fast-forwarded
without force from `bc042536eb3a40d75ccf4d1fe52032b31defac04` to the seal;
exact main CI `32633639774` passed all six jobs and main benchmark run
`32633639763` passed both jobs with two nonexpired exact-SHA artifacts. The
native `write_file` slice is delivered. This documentation-only seal is exempt
from further adversarial review. The remaining M03 native tools, CLI ownership,
and composed end-to-end boundary remain pending.

Evidence must cover exact schema and
limits, normalization and policy agreement, create/replace and atomic
visibility, exact modes under hostile umask, missing parents, symlink and
special rejection, retained-root changes, all eight temporary collisions,
cleanup and target/parent race seams, every precommit fault, post-rename sync
ambiguity, cancellation through the final precommit check, engine recovery,
the exact six-tool alphabetical reference host, Linux/macOS execution,
FreeBSD/WASI compilation, and active unsupported behavior. Three fresh
correctness/API, filesystem/robustness, and performance/concurrency agents must
all report green on the same replacement behavior SHA; every finding restarts
all three tracks. Exact seal feature CI executes supported native Linux and
macOS behavior green. Documentation-only seal and delivery commits are exempt
from a new adversarial cycle under the user's instruction. The complete normative
contract and kickoff lineage are in [`write-file.md`](write-file.md) and the
[`write_file` review](reviews/m03-write-file-review-01.md). The slice adds no
parent creation, external-path access, target-content read, CLI behavior,
benchmark workload, product-performance claim, or fx-equivalence claim. The
product remains Rust; Zig remains only the pinned upstream fx benchmark input.

The twenty-first bounded Milestone 03 slice, native `edit_file`, is
**DELIVERED** from exact delivered base
`242adfed4be717baf7cd07275aae40ec8a3637f6`. Contract commit
`bb0c381f8b044f7849bef80cc482034e1dd57ecf` is green under exact CI
`32634883133` and benchmark workflow `32634883139`. Production and independent
evidence are now composed through exact local-gate precursor
`31ec79e000589c4fb34599be4aad4f90ea33974f`. Documentation
`b1210f395a25bc59590c3b4b0164fac56e96bca0` and formal preparation
`3934d9d26ced78d5164e9ff2620c44ebb6480dd1` produced exact cycle-1 candidate
`8fdb67892f34a0fbfbb90a54e8eda982159813bf`, which was not green. Production
and independent remediation evidence are locally composed through exact
precursor `482d33c0bc586ff594d5b0decc58de347cb9243e`. Remediation documentation and
review preparation produced exact cycle-2 candidate
`f84bac87f472fc851eca670764657e5a31ce0256`, which was also not green.
Cycle-2 production and independent evidence are composed at exact behavior SHA
`ab6841388838384e27e6299151d50bb83d2ec46e`. Cycle-4 remediation composes at
`d0d188b39290a50f7f10d7e4665cf694abdfc460`; documentation-only parent
`12470d9e6c4c9301a0eeaef34e01a1ab31c84d07` and tree-identical freeze marker
`78d6fd7e0c42ec97f4f176e8378ab774c25893ca` produce the exact reviewed
candidate. Documentation seal `c1268fdf463e11242b7b916add70675ae91ed115`
completed exact feature and `main` delivery gates recorded below.

The frozen strict effect-free input is exactly required UTF-8 `path`,
`old_string`, and `new_string`, with no unknown fields. Path normalization is
the delivered `write_file` rule and independently caps requested and canonical
paths at 4,096 bytes and 256 components. `old_string` is nonempty and must
differ from `new_string`; old and new are independently capped at 49,152 raw
bytes, complete serialized arguments at 65,536 bytes, and the existing
preimage and resulting postimage independently at 49,152 bytes. NUL is valid,
empty replacement is valid, I/O and construction chunks are at most 8,192
bytes, and success is exactly normalized path plus resulting
`bytes_written`.

Core has distinct `FilesystemAccess::Edit`, serialized `edit`, because one
allowed edit must read the complete existing preimage before deriving and
atomically replacing it. Reusing `Write` would contradict `write_file`'s
explicit no-target-read authority. Preparation remains effect-free, so it
cannot pre-read the file or compute pinned fx's preapproval diff. Policy sees
only the normalized path and edit authority. Preview-bearing authorization is
deferred rather than moving ambient filesystem authority into core.

Allowed execution requires a valid-UTF-8 existing Linux/macOS regular file,
uses a cancellation-metered worst-case-linear prefix-table matcher, counts
overlapping occurrences as ambiguous, and stops after the second. The public
matcher-work cap is 393,216 charged comparison/transition steps, with
cancellation at bounded intervals. Exactly one occurrence constructs one
checked postimage; zero, multiple, oversize, invalid-UTF-8, or changing inputs
fail without publication.

The native effect reuses the hardened retained-descriptor pattern: linked-root
reacquisition, existing no-follow parents, initial bounded stable target read,
one private same-parent stage selected within eight entropy names, at most 16
cumulative interrupted results per I/O/sync phase, and at most 16 interruptions
and 31 native entropy calls per 16-byte name including partial progress. The
stage stays `0600` throughout the long parent/target rewalk and complete target
reread. On macOS, creation immediately clears and verifies the held stage has
no inherited ACL flags or entries before any write; mode `0600` is not treated
as sufficient by itself. Complete mutation-sensitive staged-path,
held-descriptor postimage, and empty-ACL checks follow the deterministic race
boundaries. Only near publication does the stage receive the original target's
nine ordinary rwx bits; content and metadata are synced and completely
reverified, including the empty macOS ACL, immediately before the final
cancellation check and atomic rename. After rename, tool cancellation is
ignored while the retained staged descriptor is stably reread for exact bytes,
identity, type, size, mode, metadata, and empty macOS ACL and the published path
is rechecked. Parent sync is always attempted, even when verification fails;
either failure is nonretryable commit ambiguity. Precommit cleanup makes the
delivered best-effort held-descriptor `fchmod(0600)` and identity-checked unlink
attempts with the same disclosed race and directly evidenced dual-failure
residue caveat.

This is not inode compare-and-swap isolation: a target or retained parent can
change after the last validation, and a moved retained parent can receive
publication. Private mode plus the empty macOS ACL narrows staged access but
cannot exclude the same UID or root. Final-mode and check-to-rename windows
remain, and writers can mutate the file after post-rename verification.
Replacing the inode preserves only
ordinary rwx bits and breaks the edited pathname away from other hard links;
ownership, ACLs, xattrs, timestamps, and other metadata are not preserved. The
slice adds no missing-file insertion, parent creation, external path, binary/alternate-encoding
editing, regex, patch/range/multi-edit, append, symlink-target access, CLI
behavior, non-Linux/macOS hardening, benchmark change, performance claim, or fx
equivalence.

Production, independent tests, and maintained documentation had separate,
non-overlapping owners. Production owned core `Edit` authority, native behavior,
exports, prepared-root/reference-host wiring, and any narrow package-private
staging extraction while preserving every delivered `write_file` behavior.
Independent tests owned public direct, engine, host, portability, fault, race,
bound, cancellation, and unsupported-target evidence. Documentation owns the
normative contract, plan/index status, and lineage record. Composition registers
exactly seven alphabetical workspace tools: `edit_file`, `file_info`,
`glob_files`, `grep_files`, `list_files`, `read_file`, and `write_file`, backed
by one retained workspace descriptor plus six identity-preserving clones.

The composed precursor passes 25 private production-helper, 23 direct, five
engine, and seven reference-host focused tests. Formatting, workspace all-
target/all-feature warnings-denied Clippy, workspace tests, and two doctests are
green; discovery inventories 665 default-feature tests, 714 all-feature tests,
and zero benchmarks. The repository harness first failed honestly at 129 tests
because its benchmark compatibility helper sorted paths while Git supplies
source order. Correction component
`e4eec3cac30ec923c19fa53a81e5b6ba9b81cfae`, integrated as `31ec79e`, preserves
Git order; the final harness is green at 130 tests, comprising 122 passes and
eight expected macOS skips. Pinned-fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, cargo-deny 0.20.2,
cargo-audit 0.22.2, Linux/FreeBSD/WASI gates, Node's active WASI unsupported
test 1/1, locked arm64 Mach-O release smoke, documentation links, no-unsafe,
whole-feature diff, and clean-worktree checks are green. These are local
precursor results, not compatibility promotion, performance, equivalence,
formal candidate, review, or delivery approval.

Cycle-1 correctness/API was **GREEN** with zero findings. Filesystem/robustness
was **NOT GREEN** with a high same-inode/same-size staged-mutation false-success
finding and a low incomplete-mandatory-evidence finding. Performance/
concurrency was **NOT GREEN** with the same high finding. Production remediation
`578ef3cf2061568d02a160fbe7a498203880b9e9` composes as
`013016f276e023838ffe7ddf8a79121a3ee463a1`; independent test component
`59471147817ed7520513fdf51041ec24c822bfe3` is red on the failed candidate for
three corruption windows and composes green as
`482d33c0bc586ff594d5b0decc58de347cb9243e`. Focused remediation suites pass 30
private, 24 direct, five engine, and seven reference-host tests. They directly
cover both precommit same-inode/same-size staged mutation windows,
postpublication held-inode corruption with parent sync still attempted, hostile-
umask mode preservation, final-verification cancellation, and moved retained-
parent publication.

Cycle-2 candidate `f84bac87f472fc851eca670764657e5a31ce0256` confirmed the
cycle-1 corruption fix. Correctness/API and performance/concurrency were each
**GREEN** with zero findings. Filesystem/robustness was **NOT GREEN** with a high
macOS inherited-allow-ACL stage/publication defect and a low incomplete
deterministic fault/cancellation evidence finding. Production component
`22197389a521095132c02125726dbe67fbf06d1b` clears and verifies the empty ACL
before write and revalidates it after final sync and publication; it composes as
`7900d97269341a9b8a46bcdcdb987279bc168e4d`. Independent component
`65d40d99f4e026834a05778029800fa703c9379e` uses the generic evidence seams to
add a phase matrix for traversal, initial/revalidation/staged/published reads,
staged creation, final-sync corruption, post-real-rename cancellation, and
cleanup mode/unlink dual failure. Exact composed behavior
`ab6841388838384e27e6299151d50bb83d2ec46e` passes 39 private, 24 direct, five
engine, and seven reference-host focused tests. This is local cycle-2
remediation evidence, not cycle-3 review, delivery, performance, or equivalence
approval.

Exact cycle-3 candidate `da1537b229393007101264cd7bc8fd12ee393a3d` has
correctness/API and performance/concurrency **GREEN** with zero findings.
Filesystem/robustness is **NOT GREEN** only for four low evidence gaps: the
actual RAII `Drop` dual-failure path, the true final-verification-to-rename
cancellation boundary, ACL clear/read/nonempty phase mapping, and call-ordinal
descriptor `fstat` selection. It reported no production atomicity defect.

Production component `985b232731883f7a5c18f8f7cbba56dbedfc7c6e` and
independent-test component `3cbb8956477af30cf5f8d63f118e597793267efc`
compose through `1b9ffca9031c61625420279569670c1c80d2d750` and exact
integrated remediation `d0d188b39290a50f7f10d7e4665cf694abdfc460`.
Production routes the actual generic staged-file `Drop` through statically
dispatched cleanup evidence, adds the checkpoint immediately after final staged
verification, and exposes ACL clear/read-empty defaults while retaining native
operations and separate `fstat` calls. Independent evidence adds ordinal-aware
fault ranges and four regressions for late descriptor faults, ACL outcomes, the
true final cancellation boundary, and actual RAII dual-failure cleanup. Exact
focused totals are 43 private, 24 direct, five engine, and seven reference-host
`edit_file` tests; `write_file` remains green at 30 private, 25 direct, and five
engine tests.

All three fresh correctness/API, filesystem/robustness, and performance/
concurrency tracks are **GREEN** with zero findings on exact cycle-4 candidate
`78d6fd7e0c42ec97f4f176e8378ab774c25893ca`. They confirmed closure of the
four cycle-3 evidence gaps in the actual generic RAII cleanup path, the true
final verification-to-rename cancellation boundary, phase- and ordinal-exact
ACL outcomes with postpublication parent sync, and intended ordinal-aware
initial, revalidation, late staged, and published descriptor `fstat` calls.

The exact candidate is green under Rust and Cargo 1.94.1 formatting, workspace
all-target/all-feature warnings-denied Clippy, workspace tests, and workspace
doctests. Focused `edit_file` totals are 43/24/5/7; delivered `write_file`
totals are 30/25/5. Discovery inventories 684 default-feature tests, 733 all-
feature tests, and zero benchmarks. The Python harness passes 130 tests with
eight expected macOS skips; pinned-fx revision
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, cargo-deny 0.20.2,
cargo-audit 0.22.2, Linux/FreeBSD/WASI gates, and Node's active unsupported
test 1/1 are green. Documentation integrity covers 62 Markdown files, 437
inline links, and 287 repository-relative links with zero missing targets. The
base diff is clean and contains zero unsafe Rust. A freshly built locked arm64
Mach-O release CLI has SHA-256
`66d1db86666764b68f79bcb5eb01a6413aab9f27d795a81b27152aa0c24add9d` and
passes bare, help, and status smoke paths.

Documentation seal `c1268fdf463e11242b7b916add70675ae91ed115` passed exact
feature CI `32650254095` across all six jobs and feature benchmark workflow
`32650254086` across both jobs with two nonexpired exact-SHA artifacts. `main`
was fast-forwarded without force from
`242adfed4be717baf7cd07275aae40ec8a3637f6` to the seal. Exact main CI
`32650593685` passed all six jobs; main benchmark workflow `32650593703` passed
both jobs and retains two nonexpired exact-SHA artifacts. Native `edit_file` is
delivered as bounded Milestone 03 slice twenty-one.

The remaining native tools, CLI ownership, and composed end-to-end boundary
remain pending, so Milestone 03 is not complete. This final delivery record is
documentation-only and exempt from adversarial review under the user's
instruction. Final documentation record
`719a9bded86fd7ce394d482798b9064c736f43ab` passed exact feature CI
`32651168514`, feature benchmark workflow `32651168515`, main CI `32651488265`,
and main benchmark workflow `32651488282`; both benchmark workflows retain two
nonexpired exact-SHA artifacts. This records no compatibility-status promotion,
product-performance, or fx-equivalence claim. The normative boundary and
delivery lineage are in
[`edit-file.md`](edit-file.md) and the
[`edit_file` review](reviews/m03-edit-file-review-01.md). Zig remains solely the
pinned upstream benchmark build input; the product implementation remains Rust.

The twenty-second bounded Milestone 03 slice, native `delete_file`, is
**IN PROGRESS** from exact delivered base
`719a9bded86fd7ce394d482798b9064c736f43ab`. That base is green under feature
CI `32651168514` across all six jobs and feature benchmark workflow
`32651168515` across both jobs with two nonexpired exact-SHA artifacts. `main`
was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to the exact base and is green
under main CI `32651488265` across all six jobs and main benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

The strict effect-free input is exactly required `path: string`, with no
unknown fields. It uses the delivered mutation-path normalization and
independently caps requested and canonical UTF-8 paths at 4,096 bytes, the
canonical path at 256 components, serialized arguments at 65,536 bytes, and
serialized results at 16,384 bytes. A canonical `.` is rejected, so the
workspace root cannot be removed. Successful preflight prepares exactly
`FilesystemAccess::Delete` for the canonical path, and direct use revalidates
the same strict shape. Success is exactly the normalized path, with no kind or
count.

Allowed Linux/macOS execution deletes exactly one existing confined regular
file or empty directory. It does not recurse, follow a symlink, read file
content, enumerate a directory, create anything, or access an external path.
It reacquires and validates the linked retained root, no-follow/nonblocking
walks existing parents, and records the final parent and target identity and
exact regular-file or directory type. A complete second root/parent walk must
revalidate that parent identity and target identity/type. The final
cancellation check occurs immediately after final validation and immediately
before exactly one `unlinkat`, with empty flags for a regular file and
`REMOVEDIR` for a directory. The call is never retried: `EINTR` is nonretryable
commit ambiguity because deletion may already have happened.

After successful deletion, later tool cancellation is ignored while the
retained parent is synced with at most 16 cumulative interruptions. An
interrupted ambiguous delete also receives a best-effort bounded parent sync,
but remains ambiguous. A nonempty directory is reported through its fixed
nonretryable category without prior enumeration. There is no staging, temporary
name, content buffer, ACL, chmod, rename, or cleanup protocol.

Portable `unlinkat` is not pathname compare-and-swap. A replacement installed
after the final target check and before the syscall can be the entry deleted.
For a validated regular file, empty-flag `unlinkat` may remove any replacement
non-directory entry, including a regular file, symlink, FIFO, or Unix-domain
socket; it never follows a replacement symlink, and unrelated referents and
sentinels remain untouched. File/directory boundary replacements fail through
the flags mismatch, while a validated directory may still be replaced by and
remove another empty directory. A retained parent moved outside the public
workspace path can still receive the descriptor-relative deletion. Other hard
links and already open descriptors survive, and a concurrent actor can
recreate the pathname after success. These limits are normative disclosures
rather than stronger concurrent-isolation or final-entry-type claims.

Production implementation/wiring, independent tests, and maintained
documentation have non-overlapping owners. Production must route statically
dispatched evidence through the real pipeline for root/intermediate opens,
ordinal `fstat`/`statat`, post-initial and post-final validation checkpoints,
actual `unlinkat` flags/outcome, the checkpoint after a real delete, and parent
sync. Evidence must cover schema and limits, effect-free policy agreement,
files/empty directories, all rejected types and paths, complete revalidation,
retained-root/parent and final-entry races, one-call interruption ambiguity,
bounded sync, cancellation/drop/engine recovery, the exact eight-tool
alphabetical host and one additional descriptor clone, Linux/macOS behavior,
FreeBSD/WASI compilation, and active unsupported-target behavior.

Exact clean precursor `5e340155f9a38b81a2812942d6ad0a796164beb5` passes the
complete local gate under Rust and Cargo 1.94.1. Focused totals are 19 default-
feature and 20 all-feature private tests, 19 direct tests, five engine tests,
and seven reference-host tests. Formatting, workspace all-target/all-feature
warnings-denied Clippy, workspace tests, and two doctests are green. Discovery
inventories 728 default-feature tests, 778 all-feature tests, and zero
benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, cargo-deny 0.20.2,
cargo-audit 0.22.2, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity is 64/445/295/0. The base diff is clean,
adds no unsafe Rust, and changes neither Cargo metadata nor CLI source. A fresh
locked arm64 Mach-O release CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`
and passes bare, help, and status smoke paths. Formal cycle 1 reviewed exact
candidate `7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf`; all three tracks
reported **NOT GREEN**. The four unique findings cover requested-path work
precedence, retained-root permission taxonomy, macOS cancellation around the
post-`EPERM` diagnostic metadata operation, and the under-specified portable
file-class replacement race. Remediation and a fresh three-track same-SHA
cycle are in progress. Exact remediation
`60e81a633557bc90aca01e3579782340c7c154c9` passes the complete replacement
local gate: focused totals are 22/23 private, 20 direct, five engine, and seven
host; discovery is 732/782 with zero benchmarks; workspace Rust, Python 130,
pinned compatibility, dependency, Linux/FreeBSD/WASI and active Node,
64/445/295/0 documentation, no-unsafe/diff, and fresh release-smoke gates are
green. Neither precursor, failed candidate, nor remediation local gate
establishes delivery, compatibility promotion, performance, or fx-equivalence
approval. Tree-identical cycle-2 candidate
`88026f10ed8c194c7160a754f226241c276579fc` is performance/concurrency
**GREEN** with zero findings but correctness/API and filesystem/robustness
**NOT GREEN**. Three overlapping medium findings cover failed-operation
cancellation precedence, complete-identity macOS `EPERM` diagnosis, and
non-root revalidation permission taxonomy; correctness adds one low public-
Rustdoc mismatch. Exact cycle-2 remediation
`225e9617a8a8f469d663693b61cc4f9b97af8094` passes the complete replacement
local gate: focused totals are 28/29 private, 20 direct, five engine, seven
host, and one core contract; discovery is 738/788 with zero benchmarks; full
workspace, Python 130, compatibility, dependency, portability, documentation,
diff, and fresh release-smoke gates are green. Another fresh tree-identical
review cycle produced exact candidate
`24f851d2d3db21735124729bb1b0a14adf7ae864`: performance/concurrency is
green with zero findings; correctness/API is not green only for low validation-
site `EROFS` taxonomy; filesystem/robustness is not green only for low missing
hostile-umask evidence and found no production defect. Mapping/matrix and
isolated child-process evidence remediations compose in exact
`77884a9fceed6268cbdbec1310de3f94a9c5a230`, which passes the complete local
gate: focused 28/29 private, 21 direct, five engine, seven host, one core;
discovery 739/789 with zero benchmarks; workspace, Python, compatibility,
dependency, portability, docs, diff, and fresh release-smoke gates green.
Tree-identical cycle-4 candidate
`0b732d2746d5c821a5294901f8b4cc641bc98530` is **NOT GREEN** in all
three tracks for the same single medium finding: definitive non-`EINTR` unlink
failures skipped post-syscall cancellation precedence. Exact remediation
`4273de513007175be94829aef85aaaa0d09bc02c` checks cancellation after every
definitive failed delete and before mapping, and its ten-case file/directory
errno matrix requires an exact cancelled result, intact targets and sentinels,
one correctly flagged delete call, and zero syncs. Complete replacement gates
are green: 29/30 private, 21 direct plus hostile-umask child, five engine,
seven host, one core, 740/790 discovery including two doctests and zero
benchmarks; workspace, Python 130, compatibility, dependency, portability,
64/445/295/0 documentation, +6,582/-63 clean diff/no-unsafe/no-Cargo/no-CLI,
and fresh release-smoke evidence pass. The 319,152-byte arm64 Mach-O CLI has
SHA-256 `126ecc47857cb327e3b483daecf9c50ce6b04585f4cdaed60e6f20cb9f82b107`.
Optional all-feature Linux cross remains blocked only by the host C sysroot in
`aws-lc-sys`, before product Rust. Formal cycle 5 reviewed exact candidate
`8575354542803f5e8ba8faf311e7524ed87eacba`, tree
`13f28f2a687960e17cd4061c849a0bae17604ae7`; correctness/API,
filesystem/robustness, and performance/concurrency are all **GREEN** with zero
findings. Clean detached worktrees and focused exact Rust 1.94.1 reruns covered
29/30 private, 21 direct including hostile umask, five engine, seven host, one
core, reference-host composition, and relevant FreeBSD/WASI gates. Crate
content is identical to remediation `4273de5`. Exact feature and `main`
delivery workflows remained pending at that point. First documentation seal `9e2a276` passed
exact benchmark workflow `32663557187` with both jobs and two exact-SHA
artifacts. Exact CI `32663557182` passed five jobs but failed aarch64 Linux when
its same-type replacement fixture allowed immediate inode reuse. Test-only
remediation `c6744ab5416fc4bde330d09f59dd507bd9991d72` retains an open handle
to the original unlinked file until revalidation, changes no production
behavior, and passes the complete replacement gate on exact tree `2ac83ee`:
workspace and two doctests, 29/30 private, 21 direct, five engine, seven host,
one core, 740/790 discovery with zero benchmarks, Python 130, compatibility,
dependency, portability, docs 64/445/295/0, clean 16-file +6,766/-62 diff with
no unsafe/Cargo/CLI changes, and fresh release-smoke evidence are green. A fresh
319,152-byte arm64 Mach-O CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`.
Optional Linux all-feature cross remains a host C-sysroot limitation before
product Rust. Because the change is executable test evidence rather than
documentation-only, exact tree-identical cycle-6 candidate
`9e817beb92b14ce718c9c6a2b35637fb6fa2cf7e`, tree `d63a92f`, received three
fresh reviews. Correctness/API, filesystem/robustness, and performance/
concurrency are all green with zero findings. The repaired identity regression
passed 500/500 sequential stress in two tracks and 64 parallel invocations in
the performance track; reviewers confirmed bounded RAII cleanup, Linux/macOS
inode pinning, zero production overhead, byte-identical production behavior,
and all applicable focused/portable gates. Replacement documentation seal
`fe56f4c57ef18f87c742340a6060dc56b91f00f9` passed feature CI
`32665295323` across all six jobs and benchmark workflow `32665295321` across
both jobs with two exact-SHA artifacts. It was fast-forwarded without force from
prior main `719a9bded86fd7ce394d482798b9064c736f43ab`; main CI `32665564381`
passed all six jobs and benchmark workflow `32665564382` passed both jobs with
two nonexpired exact-SHA artifacts. Native `delete_file` is delivered as
bounded Milestone 03 slice twenty-two. The docs-only final record's own exact
feature and `main` workflows remain required after push and cannot be self-
recorded.

After exact local gates, three fresh correctness/API, filesystem/robustness,
and performance/concurrency agents must review the same behavior SHA. Every
finding is fixed and restarts all three tracks until each reports **GREEN** with
zero findings. Documentation-only contract, seal, and delivery commits are
exempt from their own adversarial cycle under the user's instruction but still
require exact feature and `main` workflows. The complete normative boundary and
kickoff review are in [`delete-file.md`](delete-file.md) and the
[`delete_file` review](reviews/m03-delete-file-review-01.md).

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` was observed
to accept a regular file and empty directory but has broader pathname-based
behavior. This slice deliberately adds no recursion, symlink following, root
removal, content read, enumeration, creation, external path, CLI change, new
dependency, compatibility promotion, benchmark workload, product-performance
claim, or fx-equivalence claim. Exact implementation, same-SHA review, feature
workflows, fast-forward integration, and `main` workflows are green, increasing
the delivered-slice count to twenty-two without completing Milestone 03.

### Delivered bounded slice 27: native `web_fetch`

The twenty-seventh slice is **DELIVERED** on reviewed seal
`aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
`633ddd44406e22f373962c6a2ec965eae4b9cbdb`, from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. Exact feature CI `32874471757`,
feature benchmark-evidence `32874471812`, main CI `32875016066`, and main
benchmark-evidence `32875015892` are green; both benchmark runs retain two
nonexpired exact-SHA artifacts. The comparison checkout is pinned to
`vercel-labs/fx` revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`. The complete normative boundary
and live review protocol are [`web-fetch.md`](web-fetch.md) and
[`m03-web-fetch-review-01.md`](reviews/m03-web-fetch-review-01.md). The upstream
surface is an observation, not a compatibility, product-performance, or
equivalence claim.

The implementation work is split across isolated, non-overlapping worktrees:

- production owns cfg/public exports, URL/DNS/HTTP/result behavior, and
  rootless reference-host registration;
- independent evidence owns public/direct/private/engine/host tests, hostile
  network fixtures, cancellation/drop, limits, and redaction; and
- documentation owns this frozen contract, maintained summaries, and the
  exact-SHA review ledger.

Those components compose only after their own focused checks. No agent may
revert another component, edit the generated compatibility inventory, add a CLI
surface, change workflows or benchmark workloads, or make a product-performance
claim as part of this slice.

`WebFetchTool` is rootless and available only on non-WASM targets behind the
optional `web-fetch-http` feature; existing `ai-gateway-http` includes it. Its
strict model input is solely `{url:string}`. Effect-free preflight trims the
boundary, accepts at most 2,000 bytes of canonical ASCII URL, upgrades `http`
to `https`, rejects credentials, strips fragments, admits only a public
multi-label DNS name or strict public IP literal, and returns one exact
canonical URL paired with `Capability::Network`. Special-use `.alt` names and
trailing-dot numeric IPv4 spellings are rejected. An explicit default HTTPS
port canonicalizes to no port field; only an explicit non-default port is
retained. Policy and execution must agree on the HTTPS scheme, host, and
effective port. Existing core behavior keeps that network capability
`Critical`; the default policy path remains `Ask`.

Native production-transport construction synchronously snapshots the first UDP
nameserver from host resolver configuration and one random query-ID seed
outside invocation timing and without Tokio. Hostname execution retains a
fixed, retryable unavailable result when either prerequisite failed, without
rereading configuration or entropy until tool reconstruction; an admitted
public IP literal bypasses both prerequisites. A hostname receives one rooted
Internet-class A and then AAAA query through invocation-owned Tokio sockets,
with one TCP replay only for a truncated UDP answer. Query IDs derive from the
construction seed and an atomic per-query sequence, so execution performs no
blocking per-query entropy work and detaches no work. A one-byte UDP
overflow witness enforces the inclusive 4 KiB message cap. Before Hickory
decoding on UDP or TCP, the raw header must be at least 12 bytes, have exactly
one question, no more than 39 answers, no more than 128 authority or additional
records individually, and no more than 128 resource records across those three
sections. UDP then minimally decodes and validates the header/question tuple.
Only a valid `TC=1` reply may bypass the count-implied minimum and complete
resource-record decode to authorize one bounded replay. Non-truncated UDP and
every TCP reply must satisfy the checked count-implied minimum `12 + 5 *
questions + 11 * resource_records`, decode the complete declared message, and
leave no undeclared trailing bytes. A TCP frame length outside 12 through 4,096
bytes is rejected before allocation, and a still-truncated TCP answer is
invalid.
Response tuple, rooted CNAME owner chain, and terminal address ownership are
validated. There is no libc lookup, cache, retry, search suffix, resolver
thread, or spawned resolver task. The combined result accepts at most 32 DNS
answers only when every answer is public, then stably deduplicates the set in
first-seen A-then-AAAA order before pinning it to the connection while retaining
the canonical hostname for HTTP/TLS. Every returned address counts toward the
32-answer cap. A process-wide cached Rustls configuration
contains the pinned roots and fixed HTTP/1.1 ALPN; each invocation clones it
without reparsing roots into a fresh pinned Reqwest client. The tool performs at
most one HTTP/1 GET with fixed no-auth headers. Proxies, retries, referer,
cookies, automatic redirect following, and decompression are disabled. Only
2xx and identity encoding succeed. A 3xx is rejected so any later destination
requires fresh preflight and permission; the original `NetworkTarget` never
authorizes another host.

Default concurrency is eight with a hard maximum of 32. HTTP connects and
truncated-DNS TCP replay connects are bounded at 10 seconds, and the complete
operation, beginning before permit acquisition, is bounded at 60 seconds. A
DNS TCP connect uses the configured connect timeout subordinate to cancellation
and any earlier overall deadline. The inclusive response-body limit is 24 KiB and the independent
serialized-result limit is 56 KiB. Cancellation/drop releases the response and
permit on every path; the tool owns no machine-god worker. Production
construction is runtime-independent. Polling requires a current host-owned
Tokio runtime with I/O and time enabled: no handle returns fixed
`RuntimeRequired`, while a current driverless runtime violates the documented
`# Panics` precondition and may terminate a release process. Exactly one outer
machine-god invocation-deadline sleep and one cancellation future are reused
across bounded permit, DNS, HTTP, and body waits. Each truncated A or AAAA DNS
TCP replay may additionally own one short-lived configured connect-timeout
sleep, for at most two sequential DNS replay sleeps per invocation;
Reqwest/Hyper may own bounded HTTP connection-attempt timers. The outer sleep
is allocated once; each DNS replay sleep is allocated once when that replay
begins. None resets or extends the outer absolute deadline. The final
synchronous boundary checks token/deadline state directly without a second
waiter. The
native transport checks cancellation state and that same absolute deadline at
pre-effect boundaries between A, AAAA, TCP replay, HTTP dispatch, and body
work, including immediately completing phase transitions. The outer
permit survives transport completion, rendering, serialized-result validation,
and the final cancellation/deadline boundary. Text, JSON, XML, and
JavaScript are eligible bounded text. HTML remains bounded raw untrusted text.
Binary is metadata-only with no persistence. Missing MIME is classified from
the complete bounded body and reported as effective `text/plain` or
`application/octet-stream`; model-unsafe text is rejected.

Every successful output begins with a fixed upstream-untrusted warning and
includes query-redacted canonical URL, status, MIME, content kind, and
`cache_hit: false`. All failures use fixed redacted categories. This slice
deliberately defers cache behavior, binary artifact storage,
`read_tool_result`, progress/completion side channels, HTML-to-Markdown,
compression, every redirect-following policy, private/authenticated targets,
CLI changes, compatibility promotion, benchmark changes, product-performance
claims, and fx-equivalence claims. Pinned-upstream same-site/optional-`www`
redirects and default-safe admission are explicitly not copied because one
`NetworkTarget`/`Ask` decision cannot authorize a second host; all network
authority remains `Critical`.

Production and independently owned focused tests now compose locally: 11
private, 13 direct, five engine, three production-boundary, seven host, and 65
core-contract tests are green, together with warnings-denied native
all-target/all-feature Clippy. The complete exact Rust 1.94.1 local gate and
release-binary regression exercise passed on exact pre-review record
`0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`. The full default workspace lists
881 tests and the all-target/all-feature workspace lists 961. Three fresh
adversarial agents reviewed exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`, for correctness/API,
network/HTTP lifecycle and robustness, and performance/concurrency. Cycle 1 is
**NOT GREEN**. Correctness reported 0 blocker/0 high/2 medium/3 low; lifecycle
reported 0 blocker/1 high/3 medium/0 low; performance reported 0 blocker/0
high/3 medium/2 low. The deduplicated union is the driverless-runtime abort,
`.alt` and trailing-dot IPv4 admission, missing-MIME and default-port wording,
libc DNS continuation beyond permit/deadline, permit/deadline release before
rendering, missing deterministic production HTTP and concurrency evidence,
per-wait timer allocation, repeated per-invocation trust-store setup, and stale
candidate-state wording. Fixes require a new immutable SHA, the complete
replacement local gate, and three fresh review tracks; repeat until all three
report zero findings. Only a review-green exact SHA may run feature workflows,
fast-forward `main` without force, and run exact `main` workflows. Isolated
production remediation is exact component
`0c8c76935a6e3ca392e58b2aa9c375f88221f41f`, tree
`d96c13c853424325a688631dfea25c504bb62250`; exact focused-green evidence tip
is `c3dc6a00da22738b6840fc2bc66840dc735eee6f`, tree
`558140e5ac31f6f8f2cd7d15064681b53e7fd39b`. Its replacement local gate passed,
but formal cycle 2 rejected exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`. Correctness/API reported
0/0/0/2; lifecycle/robustness was green at 0/0/0/0; performance/concurrency
reported 0/0/2/1. The deduplicated union is 0 blocker, 0 high, 2 medium, and 2
low findings: raw DNS count capacity, synchronous per-invocation resolver
configuration, stale candidate-state prose, and missing-MIME prefix wording.
Exact isolated production remediation component
`6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, implements the corrected boundary.
Exact composed cycle-2 remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The four required
commands, complete 976-test all-target/all-feature workspace run, focused
21-private/14-direct/11-HTTP/5-engine inventory, Python replacements,
compatibility, dependency, portability, documentation, diff, and release-smoke
checks are green. This gate record makes no formal-review outcome, workflow,
integration, or delivery claim; formal candidates are identified only by
exact-SHA review results.
Formal cycle 3 rejected exact candidate
`16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`. Correctness/API reported
0/0/1/1; lifecycle/robustness reported 0/0/1/1; performance/concurrency was
green at 0/0/0/0. The deduplicated union is 0 blocker, 0 high, 2 medium, and
1 low. Blocking per-query entropy ran inside the total deadline; native
pre-effect transitions between A, AAAA, TCP replay, HTTP dispatch, and body
work lacked authoritative cancellation/deadline checks; and correctness plus
lifecycle repeated the low duplicated-cancellation-waiter finding. The
corrected boundary uses a construction-snapshotted random query-ID seed with an
atomic per-query sequence, fixed hostname-unavailable construction state with
literal-IP bypass, one bounded waiter for permit/DNS/HTTP/body waits, and the
same absolute deadline at every native pre-effect boundary. The final
synchronous boundary checks the token/deadline directly without a second
waiter. Exact isolated production
remediation component `9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only native
`web_fetch.rs` and implements the 32-byte key, `AtomicU32`/bounded SHA-256 ID
derivation, carried pre/post-effect deadline checks, and one cancellation
owner. Exact isolated independent-evidence commit
`3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
only `web_fetch_http.rs`; its 13/13 focused checks prove exactly one
cancellation wake, a cancelled result, and pending owned-work drop/release for
bounded and raw seams without sleep or network. This remediation record makes no
replacement-gate, formal-review, workflow, integration, or delivery claim;
formal candidates are identified only by exact-SHA review results.
Formal cycle 4 rejected exact candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`. Correctness/API reported
0/0/1/2; lifecycle/robustness was green at 0/0/0/0; performance/concurrency
reported 0/0/1/0. The deduplicated union is 0 blocker, 0 high, 2 medium, and
2 low: DNS TCP replay omitted the configured connect timeout, maintained
custom-host composition wording was incomplete, current-candidate prose was
stale, and repeated addresses reached pinned client construction without
stable first-seen deduplication. The replacement contract corrects all four
boundaries. Exact isolated production remediation component
`9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, changes only native
`web_fetch.rs`. Exact isolated independent-evidence commit
`408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, changes only native
`web_fetch_http.rs` and adds one deterministic same-poll authority regression,
but makes no native-DNS proof. Exact composed code/evidence precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`, contains both. This remediation
record makes no replacement-gate, formal-review outcome, candidate, workflow,
integration, or delivery claim; formal reviewer reports identify the exact
candidate they reviewed.
Exact composed cycle-4 remediation precursor
`892a52267e7ccf478e9ed567875dc95912be5412`, tree
`da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The four required
commands, complete 991-test all-target/all-feature run, and focused 29-private/
14-direct/14-HTTP/5-engine suites are green. Python passed 130 with eight
expected macOS skips in 39.386 seconds; pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility,
exact `cargo-deny` 0.20.2 with three allowed duplicate warnings, and
`cargo-audit` 0.22.2 `--no-fetch` across 1,225 advisories and 209 dependencies
with zero vulnerabilities are green. Linux passed with zero warnings; FreeBSD
passed with its two established warnings; each WASI variant produced 61
artifacts with six established warnings; and Node 22.22.0 ran actively at 1/1.
Documentation is
74/99/530/378/0. The whole 21-file diff is +7,641/-32 and cycle 4 is 10 files
at +728/-71; CLI, workflow, benchmark, and generated-compatibility bytes are
unchanged and no unsafe construct was added. The locked isolated 319,152-byte
release has SHA-256
`3ac3557269798c42fefaa39fd44d0f7fd7374fbe64da7c3afe3b029cdc87dcf1` and all
five exact smokes pass. This gate record makes no formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim; reviewer reports identify the exact candidate they reviewed.
Formal cycle 5 rejected exact candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`. Correctness/API reported
0/0/0/1; network/HTTP lifecycle reported 0/0/1/0; performance/concurrency
reported 0/0/0/1. The timer-accounting low is duplicated across correctness
and performance, so the union is 0 blocker, 0 high, 1 medium, and 1 low. The
exact candidate is rejected. The medium finding is a same-poll DNS TCP-connect
deadline escape. The corrected timer contract owns one reusable outer machine-
god invocation-deadline sleep, up to two sequential short-lived DNS replay
connect-timeout sleeps, and bounded Reqwest/Hyper HTTP connection-attempt
timers; no progress or subordinate timer resets the outer absolute deadline.
The source replacement must reapply cancellation and outer-deadline precedence,
then reject an expired connect deadline before accepting either a ready
success or error.
Exact isolated source remediation
`cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
`web_fetch.rs` and implements that ordering. Exact composed code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, or delivery claim.
Exact composed cycle-5 remediation precursor
`8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
`5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four required commands, complete 992-test
all-target/all-feature run, and focused 30-private/14-direct/14-HTTP/5-engine
suites are green. Python passed 130 tests with eight expected macOS skips in
84.454 seconds; pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef`
compatibility, exact `cargo-deny` 0.20.2 with the three established duplicate
warnings for `core-foundation`, `cpufeatures`, and `syn`, and exact
`cargo-audit` 0.22.2 `--no-fetch` across 1,225 advisories and 209 dependencies
with zero vulnerabilities are green. Linux passed with zero warnings; FreeBSD
passed with its two established cfg-only warnings; both the default and all-
feature WASI variants produced 61 executable artifacts with six established
cfg diagnostics each; and Node 22.22.0 ran the active unsupported-target check
at 1/1. Documentation is 74/99/530/378/0. The whole diff from exact delivered
base `a56ff350c2aace1dc22cb14c269aee89d399cd8e` is 21 files at +8,233/-32;
the cycle-5 replacement delta from rejected candidate `81b963ad` is nine files
at +542/-42. CLI source, workflows, benchmark workloads, generated
compatibility data, Cargo manifests, and the lockfile are unchanged in that
cycle-5 delta, and it adds no unsafe construct. The locked isolated arm64
Mach-O release is 319,152 bytes with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`; all five
exact smokes pass with empty stderr, and the inert status smokes do not create
their missing XDG roots. This gate record makes no formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim; reviewer reports identify the exact candidate they reviewed.
Formal cycle 6 rejected exact candidate
`5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
`a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`. Correctness/API reported
0/0/1/0; network/HTTP lifecycle and performance/concurrency were each green at
0/0/0/0. The deduplicated union is 0 blocker, 0 high, 1 medium, and 0 low; the
exact candidate is rejected. Correctness reran focused 30-private/14-direct/
14-HTTP/5-engine evidence and inventoried 992 all-target/all-feature tests. It
found that exported and documented `web-fetch-http` covers every non-WASM
target and `web_fetch.rs` uses `sha2` unconditionally for native DNS query-ID
derivation, while the direct dependency covered only Linux and macOS. FreeBSD
and Windows therefore lacked the dependency. Conversely,
`wasm32-wali-linux-musl` accidentally received it because Rust 1.94.1 reports
Linux as its target OS even though it belongs to the WASM target family. The
macOS cross-host's `aws-lc-sys` foreign C-sysroot failure occurred before this
Rust dependency could be compiled, so that cross-target attempt did not prove
the manifest boundary. The lifecycle and performance tracks each reported
zero findings at every severity on the exact candidate.
Exact source remediation `e6eba937423714bc96a7c399b4823c6deda22da7`, tree
`bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, moves the one direct `sha2`
dependency into the existing `cfg(not(target_family = "wasm"))` table. The
stdlib-`tomllib` manifest regression rejects a `sha2` alias, duplicate or
overlapping placement, optional or versioned specification, and target-scope
drift; the behavior documentation records the corrected boundary. Source
checks passed under exact Rust and Cargo 1.94.1: focused 30/14/14/5, formatting,
full warnings-denied Clippy, workspace tests and doctests, and the 131-test
Python run with eight expected macOS skips. Direct Linux and FreeBSD dependency-
tree probes include the native `sha2` and HTTP edges; the WALI tree includes
neither. Diff checks are clean. This remediation record makes no replacement-
gate, formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim.
Exact cycle-6 remediation precursor
`31e7f11870a86795f9d98831d8405714b49b989e`, tree
`68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four required commands, complete 992-test
all-target/all-feature run, and focused 30-private/14-direct/14-HTTP/5-engine
suites are green. Python passed 131 tests, including the parsed-manifest
regression, with eight expected macOS skips in 44.974 seconds. Pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, exact
`cargo-deny` 0.20.2 with the three established duplicate warnings, and exact
`cargo-audit` 0.22.2 `--no-fetch` across 1,225 advisories and 209 dependencies
with zero vulnerabilities are green. Linux passed with zero warnings. The
standard FreeBSD check passed with its two established cfg-only warnings; the
cross-host stopped at the `aws-lc-sys` foreign C-sysroot boundary, so no
FreeBSD HTTP compilation beyond that point is claimed. Each WASI variant
produced 61 executable artifacts and six established warnings, and Node
22.22.0 actively passed 1/1. Target-matrix dependency trees contain direct
native HTTP and `sha2` edges for Linux, macOS, FreeBSD, and Windows and omit
them for WASI Preview 1, WASI Preview 2, wasm64, and WALI. `Cargo.lock` remains
the identical blob `a798f7aefef7e9e043c0ed96ea205c35d0ed6b63`, with exactly one
`sha2` 0.10.9 and 209 packages. Documentation is 74/99/530/378/0.
The whole diff from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` is 22 files at +8,558/-35; the
cycle-6 replacement delta from rejected candidate `5b2b5f6` is five files at
+235/-9. That delta leaves CLI source, workflows, benchmark workloads,
generated compatibility data, the root Cargo manifest, and `Cargo.lock`
unchanged; it intentionally changes the native Cargo manifest and adds no
unsafe construct. The locked isolated arm64 Mach-O release is 319,152 bytes
with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`; all five
exact smokes pass, and status does not create missing XDG roots. This gate
record makes no formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim; reviewer reports identify the
exact candidate they reviewed.
Formal cycle 7 rejected exact candidate
`47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
`cc28286fa9bb8ac25cf05c2606a8cf9682872a52`. Correctness/API reported
0/0/1/0; network/HTTP lifecycle and performance/concurrency were each green at
0/0/0/0. The deduplicated union is 0 blocker, 0 high, 1 medium, and 0 low; the
exact candidate is rejected. The maintained bounded admission contract said
special-use DNS names were ineligible, but the lexical denylist omitted the
infrastructure suffix `.arpa`. As a result, `ipv4only.arpa`, `resolver.arpa`,
and reverse-name descendants such as `10.in-addr.arpa` could prepare a network
capability before DNS. Candidate-focused evidence passed 30 private, 14 direct,
14 production HTTP, five engine, and one parsed-manifest test. The exact
target-matrix, lifecycle/cancellation, and deterministic stress/saturation
evidence was green in the two zero-finding tracks. The candidate is the tree-
identical formal freeze after the exact cycle-6 gate lineage; reviewers found
no lineage defect, and the lack of a separate pre-review seal is protocol-
correct.
Exact root remediation `0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
`66527a146dc87b1aa9f3b7ee99385efb8022a342`, and isolated remediation
`8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` with the same tree replace suffix
allocation with an allocation-free ASCII-case-insensitive exact final-label
suffix predicate and add `.arpa`; it subsumes the older explicit `home.arpa`
case.
The exact bounded behavior document now lists the terminal-label policy, while
`example.com`, `example.net`, `example.org`, and label-bounded names such as
`resolver.arpa.example.com` remain eligible. Deterministic private, direct, and
engine evidence proves rejection before policy or transport. Source checks
passed under exact Rust and Cargo 1.94.1: 31 private, 15 direct, 14 production
HTTP, and six engine tests; formatting; full workspace warnings-denied Clippy;
the complete native all-target/all-feature run; and clean diffs. This
remediation record makes no replacement-gate, formal-review outcome, candidate,
workflow, integration, delivery, performance, or fx-equivalence claim.
Exact cycle-7 remediation precursor
`b365cf7704bacb378b280f25d702896dcc216e0c`, tree
`8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four required commands, complete 995-test
all-target/all-feature run, and focused 31-private/15-direct/14-HTTP/6-engine
suites plus the parsed-manifest test 1/1 are green. Python passed 131 tests with
eight expected macOS skips in 49.059 seconds. Pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, exact
`cargo-deny` 0.20.2 with the three established duplicate warnings for
`core-foundation`, `cpufeatures`, and `syn`, and exact `cargo-audit` 0.22.2
`--no-fetch` across 1,225 advisories and 209 dependencies with zero
vulnerabilities are green. The Linux baseline checked all 53 integration
targets with zero warnings, and warnings-denied library Clippy passed. The
FreeBSD baseline and Clippy passed with only its two established cfg-only
warning groups; the cross-host stopped at the `aws-lc-sys` foreign C-sysroot
boundary, so no later FreeBSD HTTP compilation is claimed. Each WASI variant
produced 61 test executables and six established diagnostics, and Node 22.22.0
actively passed 1/1. Native direct-edge counts are Linux 14, macOS 15, FreeBSD
14, and Windows 12, each with `reqwest` and `sha2`; WASI Preview 1, WASI
Preview 2, wasm64, and WALI count 4, 4, 4, and 6 respectively, with neither
edge. The 209-package lockfile contains exactly one `sha2` 0.10.9.
The exact allocation-free 11-entry final-label predicate, `.arpa` negative cases,
example-domain positive cases, and effect-free engine evidence are green. All
five official contract links returned HTTP 200. Documentation is
74/99/535/378/0. The whole diff from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` is 22 files at +8,999/-35; the
cycle-7 replacement delta from rejected candidate `47c6a2e` is six files at
+342/-20. That delta leaves CLI source, workflows, benchmark workloads,
generated compatibility data, the root and native Cargo manifests, and
`Cargo.lock` unchanged and adds no unsafe construct. The locked isolated arm64
Mach-O release is 319,152 bytes with SHA-256
`eed6f30ecbf19dc0c7dea498547e2562600745ed6f42561a589076083128e0e4`; all five
exact smokes pass, and status does not create missing XDG roots. This gate
record makes no formal-review outcome, candidate, workflow, integration,
delivery, performance, or fx-equivalence claim; reviewer reports identify the
exact candidate they reviewed.
Formal cycle 8 rejected exact candidate
`be418af26317da8fa5de77c45c926311475d7ff6`, tree
`4c0d054ef1a5231d4347524bb99a7bb93eceda67`. Correctness/API reported
0/0/1/0; network/HTTP lifecycle reported 0/0/1/0; performance/concurrency was
green at 0/0/0/0. The deduplicated union is 0 blocker, 0 high, 2 medium, and
0 low, so the exact candidate is rejected. The correctness finding is that a
valid `TC=1` UDP response with a partial resource-record wire tail failed the
strict full decoder or its section-count-implied minimum-length check before
the implementation could replay over TCP. The lifecycle finding is that a
structurally parseable `TC=1` UDP response with the wrong ID, QR bit, opcode,
RCODE, question name or type, or class could construct bounded TCP replay
before validation. Candidate-focused evidence passed 31 private, 15 direct,
14 production HTTP, and six engine tests plus the parsed-manifest test 1/1.
The exact bounded admission, deadline, cancellation, and stress evidence was
green in the zero-finding performance track.
Exact root remediation `9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
`fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, and tree-identical isolated
remediation `38a36725559fe962e6f649e07cc5de154c940257` retain at most 4,096 raw UDP
bytes and use capped `Header::read` plus `Query::read` parsing to validate the
expected response tuple and question before honoring truncation. A validated
partial truncated reply may ignore an incomplete resource-record tail, cross a
fresh authority boundary, and lazily perform one bounded same-nameserver TCP
replay. Non-truncated UDP and all TCP replies retain strict full decoding; a
still-truncated TCP reply is rejected. Deterministic evidence proves exactly
one replay for a valid partial truncated reply and zero replay for mismatched,
malformed, cap-breaching, non-truncated, or boundary-rejected replies.
Source checks passed under exact Rust and Cargo 1.94.1: 34 private, 15 direct,
14 production HTTP, and six engine tests; formatting; workspace all-target/
all-feature warnings-denied Clippy; the complete native all-target/all-feature
run; workspace tests and doctests; a locked release build and help smoke; and
clean diff checks. This remediation record makes no replacement-gate, formal-
review outcome, candidate, workflow, integration, delivery, performance, or
fx-equivalence claim.
Exact cycle-8 remediation precursor
`d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
`a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
local gate under exact rustc 1.94.1 (`e408947bf`) and Cargo 1.94.1
(`29ea6fb6a`) without fallback. The four required commands, complete 998-test
all-target/all-feature workspace run, and focused 34-private/15-direct/14-HTTP/
6-engine suites plus the parsed-manifest test 1/1 are green. Python passed 131
tests with eight expected macOS skips in 102.142 seconds. Pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` regeneration, exact `cargo-deny`
0.20.2 with the three established duplicate warnings for `core-foundation`,
`cpufeatures`, and `syn`, and exact `cargo-audit` 0.22.2 `--no-fetch` across
1,225 advisories and 209 dependencies with zero vulnerabilities are green.
The Linux baseline passed with zero warnings, and warnings-denied library
Clippy passed. The FreeBSD baseline and Clippy passed with only its two
established cfg-only warning groups; the cross-host stopped at the `aws-lc-sys`
foreign C-sysroot boundary, so no later FreeBSD HTTP compilation is claimed.
Each WASI variant produced 61 test executables and six established diagnostics,
and Node 22.22.0 actively passed 1/1. Native direct-edge counts are Linux 14,
Apple 15, FreeBSD 14, and Windows 12, each with `reqwest` and `sha2`; WASI
Preview 1, WASI Preview 2, and wasm64 each count four, while WALI counts six,
and all four include neither dependency. The 209-package lockfile contains
exactly one `sha2` 0.10.9.
Protocol-ordering evidence retains at most 4,096 raw UDP bytes and enforces the
wire/count caps. Capped `Header::read` plus `Query::read` validates the response
tuple and question before truncation replay. A valid partial `TC=1` reply
replays exactly once; all 13 invalid-response cases plus authority-boundary
failures replay zero times. Non-truncated UDP and TCP remain strict full
decodes. All five official contract links returned HTTP 200. Documentation is
74/99/535/378/0. The whole diff from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` is 22 files at +9,635/-35; the
cycle-8 replacement delta from rejected candidate `be418af` is four files at
+561/-54. That delta leaves Cargo manifests, `Cargo.lock`, CLI source,
workflows, benchmark workloads, and generated compatibility data unchanged and
adds no unsafe construct. The locked isolated 319,152-byte arm64 Mach-O release
has SHA-256
`869d871571ac6502ca5da7ebdbdbf26450870492234d8f226463bfa6aaf68051`; the
five exact 33/33/289/140/192-byte smokes pass with empty stderr and final LFs,
and status does not create missing XDG roots. This gate record makes no formal-
review outcome, candidate, workflow, integration, delivery, performance, or
fx-equivalence claim; reviewer reports identify the exact candidate they
reviewed.
Formal cycle 9 rejected exact candidate
`16c7287888277f14e36ce47712f017d3bb7022da`, tree
`dfe170664851281bee0324b0ed2acb97575cd49d`. Correctness/API reported
0/0/1/1; network/HTTP lifecycle reported 0/0/0/2; performance/concurrency was
green at 0/0/0/0. The two lifecycle lows overlap the correctness trailing-wire
and documentation findings; after severity reconciliation the deduplicated
union is 0 blocker, 0 high, 1 medium, and 1 low. The exact candidate is
rejected. Non-truncated UDP and TCP used a full-message API that accepted a
valid declared DNS prefix without proving input exhaustion. Maintained summary
prose also overclaimed the count-implied minimum as universal, despite the
intentional validated partial-`TC=1` UDP exception.
The remediation retains universal raw wire and header/count caps. Only a
validated truncated UDP reply bypasses the count-implied/full-message path to
authorize one bounded replay. Non-truncated UDP and every TCP reply require the
count-implied minimum, `BinDecoder` plus `Message::read`, explicit decoder
exhaustion, and full response validation. Deterministic evidence accepts exact
complete messages, rejects a complete TCP-decoded message with undeclared
trailing bytes, and rejects non-truncated UDP trailing bytes without replay.
This remediation record makes no replacement-gate, formal-review outcome,
candidate, workflow, integration, delivery, performance, or fx-equivalence
claim.
Exact cycle-9 remediation
`a184eba40470d210ba64e717649dc425200c0442`, tree
`094a90e357f42fda5e7de3929124d3db5b9cc937`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
complete 1,000-test, focused 36/15/14/6 plus manifest 1/1, Python, pinned-fx,
exact dependency/audit, portability/edge-matrix, WASI/Node, strict DNS wire
exhaustion, official-link, documentation 74/99/535/378/0, exact diff/unsafe,
locked-release, and five-smoke checks are green. This gate makes no formal-
review outcome, candidate, workflow, integration, delivery, performance, or
fx-equivalence claim; reviewer reports identify the exact candidate they
reviewed.
Formal cycle 10 rejected exact candidate
`698f9d924fe363014dc97971ae60781297d32c2c`, tree
`08a7d2b8af02b7cb06ec6961dc88192b93aa6658`. Correctness/API was green at
0/0/0/0; network/HTTP lifecycle was green at 0/0/0/0; performance/concurrency
reported 0/0/0/1. The deduplicated union is 0 blocker, 0 high, 0 medium, and
1 low. The exact candidate is rejected because the direct non-WASM `sha2`
dependency remained unconditional. Focused compilation narrowed the review's
broad premise: Linux/macOS default copy/session implementations independently
require it, while FreeBSD/Windows default builds do not and unnecessarily
carried the edge with `web-fetch-http` disabled.
The remediation uses disjoint target declarations. Linux/macOS retain required
default `sha2`; other non-WASM targets declare it optional and activate it from
`web-fetch-http`; WASM omits it. The parsed-manifest regression requires those
placements and the feature edge. Resolved depth-one trees prove Linux/macOS
default and feature presence, FreeBSD/Windows default absence and feature
presence, and WASM absence. This remediation record makes no replacement-gate,
formal-review outcome, candidate, workflow, integration, delivery,
performance, or fx-equivalence claim.
Exact cycle-10 remediation
`4d5fa4f1e3a7b2c53b5fad360cf3c85a83a021aa`, tree
`ce79fce278903196858ed47de5c22b6535beb08b`, passed the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. Focused manifest,
host-default, metadata, and target-tree checks plus every required workspace
command, diff check, and a locked-release help smoke are green. The release
binary is 319,152 bytes with SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
This gate record makes no formal-review outcome, candidate, workflow,
integration, delivery, performance, or fx-equivalence claim; reviewer reports
identify the exact candidate they reviewed.
Formal cycle 11 is **GREEN** on exact candidate
`298be5fdf0713c76155c8fe834c667d20f0cf1f9`, tree
`8f636bd92922d9e4d1610443253e82a3c0c36268`. Correctness/API, network/HTTP
lifecycle, and performance/portability each reported 0/0/0/0 in three fresh
isolated read-only reviews, so the deduplicated union is zero. This review seal
by itself makes no workflow, integration, delivery, performance, compatibility,
or fx-equivalence claim. Later reviewed seal
`aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
`633ddd44406e22f373962c6a2ec965eae4b9cbdb`, passed exact feature CI
`32874471757`, feature benchmark-evidence `32874471812`, main CI `32875016066`,
and main benchmark-evidence `32875015892`; both benchmark runs retain two
nonexpired exact-SHA artifacts. `main` was fast-forwarded without force from
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`.
Every production and explicitly injected/custom candidate host has thirteen
alphabetical tools, while the descriptor-backed set remains twelve with one
original plus eleven clones.
M03 remains **IN PROGRESS** with thirty delivered slices. The delivery
evidence makes no product-performance or fx-equivalence claim.

### Delivered bounded slice 28: top-level `permissions`

The implemented twenty-eighth slice starts from exact delivered base
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`, tree
`e1508d91edfb524df470cdf7c9b3112c4d145e4a`. Its normative boundary and review
protocol are [`permissions-cli.md`](permissions-cli.md) and
[`m03-permissions-cli-review-01.md`](reviews/m03-permissions-cli-review-01.md).
The only new grammar is `permissions [--json]`. Global help lists `help`,
`permissions`, and `status` in that order; the permissions row is exactly
`permissions  Show the permission mode and rules`, with the two leading help
indent spaces documented in [`cli.md`](cli.md).

After complete argument validation, the thin CLI calls the existing byte-
bounded, read-only `load_process_config()` exactly once and reports only the
validated ask-mode configuration plus explicit unsupported/unavailable
persistent-rule and runtime-grant state. Missing or unavailable configuration
uses built-in defaults. Every invalid configuration category fails closed with
exit 1, empty
stdout, and one fixed redacted diagnostic. The slice constructs no engine,
provider, transport, credential source, prompt, session store, state root, or
runtime, and performs no write or network effect.

Production owns only `machine-god-cli/src/main.rs`; independent evidence owns
only `machine-god-cli/tests/cli.rs`; documentation owns the contract and
maintained summaries. Those separate components implement the frozen behavior
and are composed in this feature change. They were produced from isolated
worktrees and may not add dependencies or hand-edit generated compatibility
artifacts.

Exact candidate `fe2475329b50e89bc069eded3eb2f398e8e1a167`, tree
`757f5169f03bf4018b4d85830cf37a2b716cd0cb`, passed the complete local gate
under Rust and Cargo 1.94.1 without fallback. Focused CLI evidence is six unit
and 19 integration tests. Formatting, warnings-denied workspace all-target/all-
feature Clippy, workspace tests, and workspace doctests are green. The pinned
compatibility generator check and its 31 tests are green. A fresh 368,944-byte
release binary has SHA-256
`3900c435bb108056f7916764a2b9542e479368a6b9166a499eae06d8f9b0dba3` and
passes human, JSON, valid-config no-rewrite, invalid-config redaction, parse-
precedence, help, and no-create smokes. The slice adds no dependency or unsafe
Rust. This is candidate-local evidence, not delivery evidence.

Formal cycle 1 reviewed that exact SHA/tree. Correctness/API and native config/
error lifecycle are each **GREEN** at 0 blocker, 0 high, 0 medium, and 0 low.
Performance/CLI portability is **NOT GREEN** at 0 blocker, 0 high, 1 medium,
and 2 low; the deduplicated union is the same. The medium finding is unbounded
retry work when configuration reads repeatedly return `Interrupted`. The low
findings are the config-only command's unnecessary `XDG_STATE_HOME` snapshot
and stale pre-review gate/candidate summaries. The stale-summary low is process
evidence, not a production defect, and this remediation documentation corrects
it. Any finding rejects the exact candidate.

Exact production remediation `4cf50b6f7e7ddec00e1e251902e5b9983036dd7b`,
tree `fb4d0dc7c7501ccd3547cdf1fcc83b84623f4a08`, and documentation
component `278dd45b9e38e8912d803832c37962b84faf9fe5`, tree
`536ef22cff93d5c478600e7554b65c8442b896ab`, are composed in this remediation
change. The replacement retries the first 15 cumulative `Interrupted` results,
maps the 16th to the existing fixed `Unreadable` result, and proves both
boundaries with a deterministic injected reader. Configuration loading and
permissions never request `XDG_STATE_HOME`; status retains its separate
`XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`HOME` snapshot.

Exact cycle-2 candidate `e0d590608640d7fe95f307163c99efd3e90fd2b3`, tree
`cd8919b1ff86af1b1bfbd0421a8280fc57473444`, passed the complete replacement
gate under Rust and Cargo 1.94.1 without fallback. Formatting, warnings-denied
workspace all-target/all-feature Clippy, workspace tests, and workspace
doctests are green. Focused native-configuration and CLI checks, the pinned
compatibility generator check, and all 31 generator tests are green. A fresh
368,944-byte release binary has SHA-256
`9ff974588808823a0419b150bd9b30a016cc377f8bf84f0c5aac2a14035784fe` and
passes the release-binary smoke matrix. The candidate adds no dependency or
unsafe Rust. This is candidate-local evidence, not delivery evidence.

Formal cycle 2 reviewed that exact SHA/tree. Correctness/API and native config/
error lifecycle are each **GREEN** at 0 blocker, 0 high, 0 medium, and 0 low.
Performance/CLI portability is **NOT GREEN** at 0 blocker, 0 high, 0 medium,
and 2 low; the deduplicated union is the same. One low is eager `HOME`
observation and allocation when nonempty `XDG_CONFIG_HOME` already decides
selection. The other is the permissions contract's unqualified no-follow claim
despite hardened non-Unix opening remaining deferred. Any finding rejects the
exact candidate.

Exact isolated native remediation `fa83c6c6427028c18e1c36ba6603eb44e4102eac`,
tree `a9ac7a1c147cb2ea61c61bcbf8cb58ac407bb14f`, and documentation component
`1f8968d7592de544be3c5549c275c6bc876e62c0`, tree
`058aa738487bfce08be2d964f8a577fdc12fea09`, are composed. Feature-branch
composition adds the Windows cfg mirror of the non-Unicode regression. The
replacement reads
`XDG_CONFIG_HOME` first for config loading and permissions,
reads `HOME` only when XDG is missing or empty, and never reads
`XDG_STATE_HOME`. A nonempty valid, invalid-relative, or non-Unicode XDG value
neither reads nor falls back to `HOME`. Maintained final-path `O_NOFOLLOW`,
nonblocking, no-follow, and descriptor-regularity guarantees are explicitly
supported-Unix behavior; hardened non-Unix opening remains deferred.

Exact cycle-3 candidate `4645a0fa646d2f2bb7138cd55a8ded924d9b8992`,
tree `d1eba954a983d77f2d6d11521016dc3c8eae3735`, passed the complete
replacement local gate under exact Rust and Cargo 1.94.1 without fallback. The
four required workspace commands, focused native-configuration and CLI suites,
pinned-fx `b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility generator
check and all 31 generator tests, and the 76-Markdown/110-fence/391-relative-
target documentation check are green with zero errors. The candidate adds no
dependency or unsafe Rust. Its freshly built 368,944-byte release binary has
SHA-256 `c1ffef35588516903217aeefad173fcf55daa58cec9f6ebcae37a70140e56cc6` and
passes the release-binary smoke matrix.

Formal cycle 3 rejected that exact candidate. Correctness/API reported 0
blocker, 0 high, 0 medium, and 1 low and is **NOT GREEN**; native config/error
lifecycle and performance/CLI portability each reported 0/0/0/0 and are
**GREEN**. The deduplicated union is 0/0/0/1. The sole finding is stale lower
permissions lineage in `docs/security.md`. The security prose and ledger
remediation are composed in exact cycle-4 candidate
`5a6e7782fab98b57bb939f525b3a100d5d7eee1e`, tree
`90d0b750ea9bb396074211adb07e2b251e30d505`.

That candidate passed another complete replacement gate under exact Rust and
Cargo 1.94.1 without fallback. The four required commands passed: 897
registered non-documentation tests and 31 intentional child-process probes
produced 928 non-documentation executions, and two doctests are green. Focused
native configuration passed 25 private and 29 public tests, and focused CLI
passed six unit and 19 integration
tests. The generator check against pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` and all 31 generator tests passed.
Documentation integrity covered 76 Markdown files, 110 fenced code blocks, and
391 repository-relative targets with zero errors. The candidate adds no
dependency or unsafe Rust. Its fresh 368,944-byte release binary has SHA-256
`c1ffef35588516903217aeefad173fcf55daa58cec9f6ebcae37a70140e56cc6` and
passed the full release-binary smoke matrix.

Formal cycle 4 rejected that exact candidate. Correctness/API and native
config/error lifecycle each reported 0/0/0/0 and are **GREEN**. Performance/CLI
portability reported 0 blocker, 0 high, 0 medium, and 1 low and is **NOT
GREEN**; the deduplicated union is 0/0/0/1. The sole low is ambiguous “remote
integration” wording in two `docs/security.md` permissions-lineage passages.
It conflates the already composed feature-branch source with exact feature
workflows, non-force main integration, exact `main` workflows, and delivery,
all of which remain pending. Terminology remediation is composed from exact
components named above.

Exact cycle-5 candidate `0b13944d19cfb33b4542d82d74c302669817c1af`,
tree `2ea72e810f07ed8ca2d4e8647fa713088477d8b5`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. The four required
commands passed: 897 registered non-documentation tests plus 31 intentional
child-process probes produced 928 non-documentation executions, and two
doctests are green. Focused native configuration passed 25 private and 29
public tests; focused CLI passed six unit and 19 integration tests. The
generator check against pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` and all 31 generator tests passed.
Documentation integrity covered 76 Markdown files, 110 fenced blocks, 548
inline links, and 391 repository-relative targets with zero errors. The
candidate adds no dependency or unsafe Rust. Its fresh 368,944-byte release
binary has SHA-256
`8756c7801285f1b09cad9a8b8ce47700a44127dec68ef2b0613e6a5dcecad45e` and
passed the full release-binary matrix.

Formal cycle 5 reviewed that exact SHA/tree. Correctness/API, native
config/error lifecycle, and performance/CLI portability each reported
0 blocker, 0 high, 0 medium, and 0 low findings and are **GREEN**. The
deduplicated union is zero at every severity, so the behavior candidate is
formally green. Documentation-only green seal
`3e41cc6b90adb34d62aec21c6d03729d59ca0c1b`, tree
`bd74a96c4952c2eb1e15372f4ab716a76bba91a9`, is exempt from redundant
adversarial review under the user's instruction. Exact feature CI `32891031065`
and feature benchmark-evidence `32891031147` are green on the exact seal SHA.
`main` was fast-forwarded without force from
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`; exact main CI `32891614025` and
main benchmark-evidence `32891614060` are green on the same SHA. Each benchmark
run retains exactly two unexpired exact-SHA artifacts for 90 days. The delivered
count is twenty-eight and M03 remains in progress. The combined top-level CLI
inventory remains planned, and this slice makes no benchmark-workload, product-
performance, compatibility-promotion, or fx-equivalence claim. The final
delivery-record commit is documentation-only and review-exempt; its own exact
feature and `main` workflows will be reported at handoff rather than claimed here.

### Delivered bounded slice 29: top-level `models`

The twenty-ninth slice adds only `models [--json]` from exact base
`1de3b7eddf6a4d9046d48098defecf6bfa336442`. Its normative behavior and output
boundary is [`models-cli.md`](models-cli.md); its adversarial record is
[`m03-models-cli-review-01.md`](reviews/m03-models-cli-review-01.md). The
contract is frozen and local core/native/CLI implementation exists in exact
components `a6c6ff333176689b0c53bcf35070e9d59afd1b28`,
`7c966b23d75a880a23d49e1e6ba9780e512e84b8`, and
`e84ed2a46b1ac5fe7428414375609af562c65105`. Checked-deadline and terminal-
precedence remediation is `52e9b7d74f3979f7f7f55387243e96bd78773fe3`.
Independent native evidence `12263afa458e48f2963ae3d0e3db5cf219f8bdf6`
initially contained 35 focused tests (14 provider/parser, 15 loopback HTTP, and 6
credential), sourced independently from `219f6a71a766e9b833a98f236cfbc3aaff292cd5`.
The local feature-topology refinement adds the dedicated non-WASM
`ai-gateway-model-catalog-http` gate and makes the CLI select it without
activating direct generation `bytes`, `web-fetch-http`, or Tokio's signal
backend. Catalog HTTP uses Hickory only for bounded system-configuration
parsing; private bounded Tokio UDP/TCP exchange owns DNS network behavior, so
no non-abortable GAI lookup or Hickory resolver task can outlive runtime
teardown. Only the CLI requests signal handling. Production construction
eagerly retains one bounded validated system-DNS snapshot and one fallible
32-byte query-ID key before the provider deadline exists. Generic Unix bounds
`/etc/resolv.conf` to 64 KiB plus one overflow byte through nonblocking close-
on-exec regular-file reads; Apple, Android, and Windows use their platform
snapshot APIs and then apply the same structural bounds. Request polling
performs no configuration, hosts-file, or entropy read and spawns no resolver
task. A keyed atomic sequence derives IDs with bounded SHA-256 work, and the
lookup uses only active-runtime-owned sockets and timers, so no runtime-backed
state is reused across sequential current-thread runtimes. Reqwest's built-in
Hickory path is explicitly disabled before the custom resolver is installed,
and the lookup name is made an absolute single-dot FQDN so platform search
suffixes are not queried. The existing `ai-gateway-http` feature still adds
direct `bytes` and includes catalog HTTP plus `web-fetch-http`, so its delivered
generation/reference-host behavior is preserved. Exact cycle-1
candidate `6277aa3dc26f9c485707c667f63525a2138f316b`, tree
`b5e2445ed90df000255b51c2c989d71965db1d77`, passed the complete local gate and
was rejected by three fresh tracks with a deduplicated union of two medium and
six low findings. Provider deadline/response-lifetime remediation
`02c9f86619fbdc202f5065c41090415a179316cf`, signal/config/WASI remediation
`d2890c34bc628dd9ad425f5921e3816bbe1f5eef`, and dependency-topology remediation
`06c94087e91ec298877fbe981695d2638fa1db1e` are composed locally. Focused native
evidence is now 36 tests (15 provider/parser, 15 loopback HTTP, 6 credential),
and CLI unit evidence is 18 tests. Exact cycle-2 behavior candidate
`2ea9d94374c4dd18f43255af785ee31088126c56`, tree
`3a948b2950d870a9cabe479bc6c3889dd5a13a3b`, passed the complete replacement
gate but was rejected with one high, one medium, and one low finding. Parser
remediation `9cf8c74`, async DNS/no-runtime remediation `8187b12`, and fail-
closed system-resolver remediation `499af85` are composed locally. Provider/
parser evidence is now 17 tests, HTTP integration is 16, and private resolver
lifecycle evidence was 2. Pre-review gate attempt
`c01139811685ae73031ed6f6cbd771e4ff636714`, tree
`4ac4e5bd67b98900159aec1772f75d6003ba1d70`, was rejected when dependency
inspection found synchronous system-DNS discovery inside the timed request
poll. Bounded eager snapshot remediation `d9922ef` and custom absolute-name/
per-runtime resolver hardening `e5248b1` raise private resolver evidence to 6.
Exact cycle-3 behavior candidate
`2cecc921e48396e81ab6f434007a7ec8e3e890b5`, tree
`8c0d235355582d92aaed6fcca7c1862982494e20`, passed its complete replacement
gate. Formal cycle-3 review rejected it: correctness/API reported 0/0/0/2,
network/error lifecycle reported 0/0/1/1, and performance/concurrency reported
0/0/0/0. The deduplicated union is 0 blocker, 0 high, 1 medium, and 3 low.
Documentation remediation `f80bd0560e49306ce56093ea667aa45a22b2c6dd`, tree
`d6d3f354fa1eca6221d482567f9128ac6f73a3bc`, corrects the three stale topology/
status passages. Native remediation
`b6cf4cbc01ba2470b7aef77b96d5793ad95f6b0d`, tree
`e72f7dd6b4f3306faee5d0d3fc8483c63f4fd24a`, eliminates Hickory's lazy
request-poll RNG path through private bounded DNS and fallible construction-
time entropy. A read-only preflight before the complete cycle-4 gate found
0 blocker, 1 high, 0 medium, and 0 low on that exact remediation. Android
catalog construction could still invoke Hickory `read_system_conf`; without an
initialized `ndk_context`, that loader panics and the release profile aborts.
This was not a formal cycle-4 review, so cycle 4 remained unsubmitted at that
checkpoint. Exact
remediation `bd4746197736004f3a02c4c910c569444e4653e7`, tree
`b8d8bb1ebda0c78a7f4ba353fbe2cc03d83f64f3`, makes Android catalog DNS fail
closed and redacted without a DNS or platform-loader call while leaving Apple/
Windows and generic-Unix loading unchanged. Exact focused resolver 14/14,
manifest 5/5, formatting, and warnings-denied native Clippy checks are green.
The Android standard-library target was unavailable locally, and foreign Linux
HTTP compilation retained the existing `aws-lc-sys` C-sysroot blocker before
product Rust. Exact cycle-4 behavior candidate
`57d2ac2a3cc562763739f49642e6fdd172f036e8`, tree
`d30bb656dfe52e15858df9d4e52a301cb61da8ce`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. Focused evidence is
green at core 6/6, provider/parser 17/17, credential 6/6, HTTP 16/16, private
resolver 14/14, CLI unit 18/18, CLI integration 23/23, and manifest 5/5. The
complete gate passed formatting, warnings-denied workspace Clippy, 972 non-doc
Rust tests, two doctests, the first sequential 135-test Python run with eight
expected macOS skips, byte-stable pinned-fx regeneration, documentation
integrity 78/127/567/410/0, dependency policy/audit with zero vulnerabilities,
the honest host/WASI/foreign-target matrix, the exact 33-file +9,890/-209 diff,
and six fresh release-binary black-box cases. The 3,818,944-byte locked release
has SHA-256
`caf993ea44007431f16b5a0b7383865e4bead46337e2e9c711a8a9174812bd7c`,
resolves 139 normal packages, and returned 228 live public models without a
state write. Three fresh formal cycle-4 reviews **REJECTED** the exact
candidate. The raw overlap-deduplicated union was 0 blocker, 0 high, 1 medium,
and 2 low; after prior sealed dispositions, 0 blocker, 0 high, 1 medium, and
1 low remained unresolved at verdict collection. Exact documentation fix
`268d35a447d6ceb29b37b0c5f789ad589e4792e4` closes the remaining topology low.
Exact signal/output-lifecycle remediation
`aa60db15d016cf97674459a4af66318a18b762ac`, tree
`278fa365e24504452f8d111a7b08bc49e2aed164`, closes the medium in locally
integrated behavior. Exact cycle-5 behavior candidate
`27c75f4365af92759686402574d310ada596a923`, tree
`5e40b24259d76196d573f752258c9a764b53f990`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback:

- focused core 6/6, provider/parser 17/17, credential 6/6, loopback HTTP 16/16,
  private resolver 14/14, CLI unit 22/22, CLI integration 23/23, and parsed-
  manifest/resolved-tree 5/5 suites passed;
- formatting, warnings-denied workspace all-target/all-feature Clippy, 976
  non-documentation Rust tests, and both workspace doctests passed;
- the Python suite first encountered one fake-Git timing timeout; its focused
  retry and complete sequential rerun passed all 135 tests with eight expected
  macOS skips;
- regeneration against pinned fx
  `b1774fbf6c7602b503026f96f6e960e946c692ef` was byte-stable, and
  documentation integrity covered 78 Markdown files, 127 fenced blocks, 567
  inline links, and 410 repository-relative targets with zero errors;
- `cargo-deny 0.19.9` passed with only the established duplicate warnings, and
  `cargo-audit 0.22.2 --no-fetch` loaded 1,226 advisories, scanned 210
  dependencies, and found zero vulnerabilities;
- the host default, narrow catalog, broad HTTP, all-feature, and CLI checks,
  WASI native default/catalog and CLI checks, and Linux/FreeBSD no-feature
  checks passed. Resolved target trees for macOS, Linux, FreeBSD, Windows,
  Android, and WASI have the intended direct edges;
- catalog HTTP cross-compilation for Linux and FreeBSD stopped in `aws-lc-sys`
  because the macOS host lacks the foreign C standard-library/sysroot headers,
  before product Rust; exact remote Linux CI remains authoritative;
- workflows, benchmark workloads, pinned-fx generator inputs, and generated
  compatibility data are unchanged; and
- a fresh locked 3,852,144-byte `target/release/machine-god` with SHA-256
  `9babe527b1f5836b53e3aeaf932b24bb6a67b187bc4bbaba69a17d9d7c8eba4f`
  resolved 139 normal packages and passed six black-box cases plus the release
  signal regressions. The live public catalog returned 228 models without a
  configuration or state write.

Three fresh formal cycle-5 reviews bound that exact candidate and each reported
0 blocker, 0 high, 0 medium, and 0 low findings:

- correctness/API/compatibility exercised 90 focused checks, doctests, release-
  binary behavior, WASI, and pinned-fx regeneration evidence;
- lifecycle/network/portability exercised private DNS 14/14, loopback HTTP
  16/16, all three signal regressions in debug and release, WASI, and FreeBSD;
  and
- performance/concurrency/resources exercised CLI unit 22/22, provider/parser
  17/17, loopback HTTP 16/16, private DNS 14/14, manifest/resolved-tree 5/5,
  and the bounded signal tests.

The deduplicated union is zero, so exact candidate `27c75f4`, tree `5e40b24`,
is formally **GREEN**. At that approval checkpoint, exact feature workflows,
fast-forward integration, and exact `main` workflows remained pending; the
slice had not reached CI-green, integrated, or delivered status. These checks
make no product-performance, speed,
latency, memory, binary-size-improvement, catalog-equivalence, or fx-equivalence
claim.

Review-exempt cycle-5 green seal
`20640843f49faf3de1b208bc6e8ee49ff0ff9c94`, tree
`33818a48d3ef4f000789d10574ee2024de95cb29`, was pushed to the feature branch.
Feature benchmark-evidence run `32923421739` passed both the pinned-upstream
and bootstrap jobs. Feature CI `32923421679` was not green: its sole failure
was exact-Rust-1.94.1 Quality Linux Clippy rejecting `needless_continue` at the
preexisting native catalog-DNS test helper's old line 2233. No integration to
`main` occurred. Exact one-line test-only remediation
`831d38c8da72b849704ef3ab508588a9d0499c5f`, tree
`a92acc141cab42061614a2e6f0f9d1f240325e2f`, replaces that redundant control-
flow spelling without changing product source or behavior.

That exact cycle-6 replacement candidate passed the complete local gate under
exact Rust and Cargo 1.94.1. All four required workspace commands are green,
with 976 non-documentation Rust tests and two doctests. Private resolver
evidence is 14/14, and the sequential repository Python suite is green at 135
tests with eight expected macOS skips. Documentation integrity, dependency
policy, and vulnerability audit are green. Exact Linux warnings-denied lint
proof is green; the optional foreign catalog-HTTP cross-build retains only the
known `aws-lc-sys` missing-C-sysroot boundary before product Rust. A fresh
release-product comparison is normalized-identical to the cycle-5 reviewed
product and passes six release-binary black boxes, the live 228-model public
catalog case, and the signal regressions.

Three fresh formal cycle-6 correctness/API/compatibility, lifecycle/network/
portability, and performance/concurrency/resources tracks bound exact candidate
`831d38c8`, tree `a92acc14`. Each reported 0 blocker, 0 high, 0 medium, and
0 low findings; the deduplicated union is zero. The replacement is formally
**REVIEW-GREEN**. Documentation-only delivered seal
`bacc5c3dbc2bf094cca12102030d21f468f11e7a`, tree
`da3183a3368273c2b34324a5f33266dfe5644a0d`, is exempt from redundant
adversarial review under the user's instruction. Exact feature CI `32925681006`
and feature benchmark-evidence `32925681009` are green on that exact seal SHA.
`main` was fast-forwarded without force from
`1de3b7eddf6a4d9046d48098defecf6bfa336442`; exact main CI `32926242609` and
main benchmark-evidence `32926242564` are green on the same SHA. Each benchmark
run retains exactly two unexpired exact-SHA artifacts for 90 days. The delivered
count is twenty-nine and M03 remains in progress. This delivery makes no
product-performance, speed, latency, memory, binary-size-improvement, catalog-
equivalence, compatibility-promotion, or fx-equivalence claim. The final
delivery-record commit is documentation-only and review-exempt; its own exact
feature and `main` workflows will be reported at handoff rather than claimed
here.

Parsing is strict and completes before effects. Repeated `--json`, every extra
or unknown argument, and non-Unicode input fail at exit 2. A valid command loads
configuration exactly once before credential and transport access. The
existing environment credential precedence is retained: a missing credential
selects public catalog access, while a selected non-Unicode, malformed, or
oversized credential fails closed without lower-priority or anonymous fallback.
Machine-god has no team source, so it sends no team query or header.

Core owns validated provider-neutral available-model/access/result/error values
and only an object-safe `list_models(CancellationToken)` trait; it receives no
access request or deadline and owns no clock, fallback, sorting, Gateway
metadata, or output rendering. Native owns the separate production GET to
exactly
`https://ai-gateway.vercel.sh/coding-agent/v1/models`, with only an explicitly
injected numeric-loopback HTTP test endpoint. The release endpoint is not
environment-overridable. The thin CLI constructs and drives a current-thread
Tokio runtime with I/O and time enabled. An authenticated 401/403 alone receives
exactly one fully anonymous retry under the original absolute deadline; no
other result and no public request retries. Redirects, proxies, cookies,
decompression, referer/title/team metadata, and application retry are disabled.
The catalog request identifies itself with fixed
`User-Agent: machine-god/<package-version>` in addition to
`Accept: application/json`, `Accept-Encoding: identity`, and optional bearer
authorization.

The total catalog budget is a checked 30 seconds including capacity wait, with
active attempts defaulting to 8 and hard-limited to 32. The same absolute deadline is
passed unchanged to both possible transport calls. The provider independently
polls the transport's required deadline authority, and each HTTP call creates
its own attempt-local cancellation waiter and timer against that deadline
rather than sharing one outer sleep. Independent inclusive caps are
256 KiB retained response body, JSON depth 32, JSON nodes
16,384, 1,024 raw `data` entries, 512 valid language entries, 1–128 visible-
ASCII bytes per model ID, 24 KiB aggregate accepted ID bytes, and 64 KiB
serialized output including its final LF. A strict root object and `data` array
are required. Only non-object entries, objects lacking a string ID, and entries
with an explicit ASCII-case-insensitive non-language type are skipped. A string
ID that is not safe visible ASCII in the exact length bound is a terminal
malformed response. Duplicate valid IDs and global structural/resource defects
also reject the whole response.

All accepted IDs use native's pinned full-catalog total ordering: tool-use
first, then ASCII-case-insensitive tier rank, exact case-sensitive provider ID-
prefix rank, release descending, and ID ascending.
Human success is `[models] N available` plus one ` - <id>` line per ID, with a
separate empty-catalog line and machine-god-specific public-catalog explanation.
JSON success is compact, has fixed key order
`kind,count,shown_count,more_count,private_models_hidden,ids`, contains the full
list, prepends no name/version fields, and ends in LF. Valid-command failures
exit 1 through the closed redacted `AuthenticationRejected`, `Cancelled`,
`MalformedResponse`, `ResourceLimit`, or `Unavailable` presentation map. The
command creates no state/workspace/engine/generation-provider/prompt/permission/
session/cache/write effect.

Production, independent evidence, and documentation must use isolated,
non-overlapping ownership. Each exact behavior candidate must pass focused and
complete exact-1.94.1 local gates, then three fresh correctness/API,
network/security, and performance/concurrency adversarial reviews. Any finding
rejects the candidate and requires remediation, the complete replacement gate,
and three fresh reviewers. Only a zero-finding exact candidate may proceed
through feature exact-SHA workflows, a non-force fast-forward, and exact `main`
workflows. Benchmark evidence is delivery evidence only.

### Delivered bounded slice 30: top-level `doctor`

The thirtieth slice is contract-frozen from exact delivered base
`f82ce46736f7bac4154da508e3b768d0b9248e15` and is **DELIVERED**. It adds
only strict `doctor [--json]`. Exact cycle-1 candidate `761bf0b` passed its
complete local gate but was rejected by one medium and two deduplicated low
product-review findings: real process inspection order, partial-write evidence,
and complete native classification/build-mode evidence. Isolated remediations
`04444d9`, `3de641f`, and `1e5bf28` are composed and their focused exact-1.94.1
gate was green before cycle-2 composition. Cycle-1 reviewers did not approve
that replacement.

Exact cycle-2 candidate `408e6ff` then passed its complete replacement gate.
Two fresh reviewers were zero-finding, but the performance/resource reviewer
reported one low evidence gap: no test fixed the `Ok(0)`/`WriteZero` stdout
boundary. That finding rejected the entire candidate. Isolated remediation
`5a125b6` covers both human and JSON zero-progress writes. Exact cycle-3
candidate `15f8176`, tree `8278a77`, passed its complete exact-1.94.1
replacement gate without fallback. Three new same-SHA review tracks each
reported `0/0/0/0`, so the deduplicated union is zero and the behavior
candidate is formally **GREEN**. Neither prior reviewer set was reused.
Review-exempt seal `345f812`, tree `8899849`, passed feature CI `32933464234`,
feature benchmark-evidence `32933464047`, main CI `32933879888`, and main
benchmark-evidence `32933879930`. `main` was fast-forwarded without force from
the delivered base; both benchmark runs retain exactly two unexpired exact-SHA
artifacts. The delivered count is thirty and M03 remains in progress.

Parsing completes before effects. Accepted forms are exactly `doctor` and
`doctor --json`; every other, repeated, extra, reordered, or non-Unicode
argument fails at exit 2. A valid invocation always reports exactly four checks
in order: `config`, `credential`, `state`, `platform`. Each status is one of
`ok`, `warn`, or `fail`. Missing configuration selects built-in defaults and is
`warn`; strict file-backed configuration is `ok`; every existing fixed redacted
configuration error is `fail`. A valid selected OIDC token or API key is `ok`;
missing or invalid generation credentials are `fail`. A ready state directory
is `ok`, missing state is `warn`, and every other state classification is
`fail`. Linux/macOS are `ok`; other targets report platform `fail`.

Human output is one `[doctor] ok=N warn=N fail=N` summary followed by four
`[status] name: detail` lines. Compact JSON fixes top-level key order
`kind,ok_count,warn_count,fail_count,checks` and check-object order
`name,status,detail`. Each count is `0..=4`, their sum is exactly four, and they
match the check list. Both outputs end in LF and are built before the first
stdout write under an inclusive 4,096-byte cap. Closed detail strings expose no
path or arbitrary external input. Any complete report, including diagnostic
`fail` checks, exits 0. Render failure exits 1 with exact
`machine-god doctor: could not render report\n`; stdout write failure exits 1
with existing exact `machine-god: failed to write output\n`.

The command is read-only and no-create. It inspects only strict configuration,
the two fixed credential sources, selected state-directory metadata, and
compile-time platform support. It performs no configuration rewrite or root
preparation; constructs no engine, provider, transport, runtime, session, or
workspace; selects no model; starts no process; and makes no network request.
It outputs no network, process, runtime, session, workspace, model, or path
state. Fixed failures disclose no secret or operating-system error.

The bootstrap workload order remains `bootstrap-exit`, `help`, `status-json`,
`doctor-json`, `sessions-json`, `background-json`. Doctor moves only from
unimplemented to implemented/non-equivalent/not-measured/claim-ineligible.
Sessions and background remain unimplemented. Pinned fx commit
`b1774fbf6c7602b503026f96f6e960e946c692ef` has a different diagnostic
inventory, schema, and exit boundary; this slice makes no semantic-equivalence
or product-performance claim.

### Delivered bounded slice 31: top-level `sessions`

The thirty-first slice is contract-frozen from exact delivered base
`feaf9fa1bc6bb66544947152e2c5fe91c8cd185e`. It adds only strict
`sessions [--json]`; every extension, reordered/repeated flag, extra or non-
Unicode argument is rejected before environment or persistence effects. The
normative boundary is [`sessions-cli.md`](sessions-cli.md), and the live ledger
is [`m03-sessions-cli-review-01.md`](reviews/m03-sessions-cli-review-01.md).
Exact candidate `9448738908e014c9273960d9e7dd0790e0525848`, tree
`e5342f79207af6cc41da71a280d8ca9ef3952756`, passed the complete local gate and
was rejected by cycle 1 with `0/0/0/3` deduplicated findings. Two accepted lows
require state-only process environment capture with lazy `HOME` fallback; the
third was this stale candidate summary. The replacement implements the narrow
reader and exact request-sequence tests, updates the maintained summaries, and
passed the complete replacement local gate. Exact candidate
`a52765250bf7b0e6a7224ae95cf5a4ab046ffa75`, tree
`0249dd0b97197fa37ab64d2c1baa3b818717211a`, passed three fresh exact-SHA
cycle-2 tracks at `0/0/0/0` each and is formally green. Feature workflows,
integration, and `main` workflows passed under review-exempt seal `b5b9116`,
tree `3e61754`: feature CI `32939742230`, feature benchmark evidence
`32939742231`, main CI `32940279028`, and main benchmark evidence `32940279005`
are green. Both benchmark runs retain exactly two unexpired exact-SHA artifacts,
and `main` was fast-forwarded without force. The delivered count is thirty-one
and M03 remains in progress.

The command exposes the delivered bounded native session-ID scan through a new
engine-free native process facade. It selects only the machine-god state
namespace, safely walks an existing root under the established ownership,
mode, no-follow, and macOS ACL policy, and creates no missing root. A genuinely
absent selected base or fixed suffix is an empty, non-truncated result. Invalid,
unsafe, inaccessible, symlink, and wrong-kind roots fail with fixed redacted
categories. The command does not construct the full reference host, workspace
tools, config, credential source, provider, transport, permission handler,
engine, or runtime. The existing scan may create a missing permanent private
`0600` per-record lock sidecar while validating a canonical candidate; it never
creates, repairs, rewrites, migrates, or deletes a record.

Success returns no more than 100 distinct IDs in strict ascending lexical order
plus `truncated`. Human output uses an empty `no saved sessions` line or a
counted header and one ID row, with an explicit resource-limit line whenever
truncated. Compact JSON fixes key order `kind,count,truncated,sessions`; each
session object currently contains only `id`. Both representations end in one LF,
are assembled before the first success write, and fit under an inclusive 16 KiB
cap. Truncation is successful incomplete observation, not `has_more`, a cursor,
or pagination. Closed operational categories are `Corrupt`, `Unavailable`,
`Unsupported`, and `ResourceLimit`; neither channel reflects external details.

Pinned fx instead defaults to the current workspace, supports `--all`,
`--limit`, and `--cursor`, orders rich summaries newest-first, paginates, and
skips/reports corrupt records. Current machine-god records have no
authoritative workspace, title, preview, language, time, history-length, or
display-rank fields. This slice therefore stays deliberately non-equivalent.
The `sessions-json` benchmark record moves from `unimplemented` only to
implemented/non-equivalent/not-measured/claim-ineligible. It gains no samples,
threshold, performance claim, or compatibility promotion. Workload order and
the pinned upstream input stay fixed; `background-json` remains unimplemented.

Production and evidence work is assigned to three isolated, non-overlapping
worktrees: native facade/root/store sharing, CLI production, and independent
native/CLI/benchmark/CI evidence. Documentation and integration stay with the
coordinator. After composition, focused checks and the complete exact Rust
1.94.1 gate must pass before three fresh correctness/API, native persistence/
portability, and performance/resource reviewers inspect the same exact SHA.
Any finding rejects that candidate and requires remediation, the complete
replacement gate, and three new reviewers. Discovery agents are not reviewers.
Only a zero-finding candidate may proceed to exact feature workflows, a non-
force fast-forward, and exact `main` workflows.

### In-progress bounded slice 32: top-level `session`

The thirty-second slice is in progress from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Exact cycle-2 candidate
`1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, is rejected. Its cycle-2
replacement was composed at exact `f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`,
tree `86971aca0f78e637de55d2a79eda64e88bff8734`; formal cycle 3 rejected exact
candidate `9282b4044c5fb5a249598d23d098562c96850c99`, tree
`6d41f7ee6eb017dfc65d6f6623d049ac09c2966f`. Cycle-3 remediation was composed
at exact `af055ff3b22e157b1c42d1579b041c3cc4c05b0e`, tree
`14eafada4b3dddd62a9cb8e6077ad8f0b81753e8`; formal cycle 4 rejected exact
candidate `df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. The slice adds only strict
`session <id>` and `session <id> --json`. The ID is parsed through the core
portable contract before effects, while exact tokens `last`, `--id`, and
`--json` are reserved in ID position. Every upstream-style selector, reordered
or repeated flag, extra/non-Unicode argument, or invalid ID fails before state
environment or filesystem access. The normative contracts are
[`session-cli.md`](session-cli.md) and
[`native-session-inspection.md`](native-session-inspection.md); evidence is
tracked in
[`m03-session-cli-review-01.md`](reviews/m03-session-cli-review-01.md).

Native captures only `XDG_STATE_HOME` and lazy-fallback `HOME` on first poll,
opens the already-existing validated machine-god hierarchy, and projects only
ID, incarnation ID, positive revision, positive next turn sequence, message
count, and top-level metadata-entry count. The synchronized replacement must
inspect the exact current-schema record in one forward pass through a fixed
4 KiB input buffer without calling `FileSessionStore::load` or constructing a
full `SessionRecord`. Known tokens use fixed-stack scratch; only the two
returned IDs retain payload-sized strings. Canonical `serde_json::Number`
parsing and a fixed-digest entry-capped duplicate tracker must match
ordinary last-value-wins metadata/nested-JSON semantics. Tracker entries and
buckets must grow fallibly in proportion to unique keys and are capped at
65,536. Aggregate final decoded-tree logical-node accounting is separately
capped at 65,536. Shadowed duplicate values may increase total parse work beyond
that logical-node count, but the 8,651,165-byte file ceiling bounds that work.
The ordinary-store schema is object-only for its envelope,
record, message, tool-call, and tool-output structs and string-only for role;
ordinary loading/listing and streamed inspection must reject all six
noncanonical sequence/map forms consistently while the canonical writer stays
unchanged. Parse-time recursion
must independently reproduce `serde_json` 1.0.151's 127-active-container
budget, including typed parent contexts of 3 for metadata, 6 for JSON content,
and 7 for tool-call/result JSON. These file-byte,
aggregate JSON depth/node, identifier, counter, and content-shape constraints
are store-owned. Inspection does not invoke engine validation and does not
enforce its configurable/default 4,096-message, 8 MiB serialized-transcript, or
256 KiB serialized-metadata limits. Missing hierarchy or record is no-create
`NotFound`. Existing record inspections may create only the file store's
permanent private `0600` lock sidecar. No transcript, metadata key/value,
engine, provider, credential, configuration, permission, workspace, network,
runtime, resume, replay, migration, recovery, or `.fx` authority crosses this
boundary.

Human output has one header and five fixed structural rows. Compact JSON fixes
key order
`kind,id,incarnation_id,revision,next_turn_sequence,message_count,metadata_entry_count`.
Both modes end in one LF, are completely assembled before writing, and fit
under an inclusive 4,096-byte cap. Successful transferred bytes/work and the
retained summary are finite, but exclusive sidecar-lock wait, filesystem
latency, and retries after `EINTR` have no wall-clock or attempt ceiling and
synchronously block the polling and CLI thread. Closed presentation failures
are `NotFound`, `Corrupt`, `Unavailable`, `Unsupported`, and `ResourceLimit`;
no external detail is reflected. There is no matching bootstrap `session-json`
workload, so the fixed inventory and its classifications remain unchanged. The
feature is intentionally non-equivalent, not measured, and claim-ineligible.

Native production/focused evidence, CLI production/unit evidence, and
independent process/workflow evidence were built in isolated, non-overlapping
worktrees and composed as `10a5333`, `412d63b`, and `55f37fc`. Composition
makes non-exhaustive native categories fail closed to `Unavailable` and aligns
help bytes to `Inspect a saved session`. The initial focused exact-1.94.1
evidence was green for 12 native inspection tests, 56 CLI unit tests, 46
independent CLI process tests, and native/CLI all-target/all-feature warnings-
denied Clippy. Focused composition-gate remediation was exact
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`. Focused, Python, pinned-fx,
WASI/FreeBSD, documentation, diff/no-unsafe, and complete exact-tree release-
binary gates are green. The fixed benchmark inventory remains unchanged, and
no new dependency or unsafe Rust is introduced.

Exact cycle-2 candidate `1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed the complete required local
gate under exact Rust/Cargo 1.94.1 plus Python 135/8 skips, pinned-fx,
WASI/FreeBSD, diff/no-unsafe, 85/146/626/81 documentation-integrity, and release-
matrix checks. Its 4,001,712-byte release binary had SHA-256
`e975e8a16f750188de25d8cf0eac02975643edf6730d6b3ad87d442b76ce27bb`.
Formal cycle 2 rejected it: correctness/API reported `0/0/1/2`, native
boundary/effects `0/0/1/2`, and performance/concurrency/resources `0/0/1/1`.
The deduplicated union has two medium themes (canonical-number mismatch and
residual payload-proportional allocations) and two low themes (duplicate-key
mismatch and stale maintained docs).

Exact cycle-2 remediation source
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, passed the complete required local
gate under exact Rust/Cargo 1.94.1 without fallback. Focused evidence is green
for 56 CLI unit, 54 CLI process, and 21 native inspection tests. Python is green
at 135 with eight expected skips; pinned-fx `b1774fb` regeneration is byte-
stable; WASI/FreeBSD checks are green with only the established WASI
`read_file` warning; and documentation integrity is 85/147/626/81 with zero
errors. The exact replacement adds no manifest, lockfile, dependency,
benchmark, inventory, or unsafe-Rust delta. Its 4,001,760-byte release binary
has SHA-256
`483eb60f707cadfe4b0dd10cfb65617e576488546d908f2f6811b0bfc55773cc` and its
matrix is green, including six process differentials, an 8,650,857-byte near-
cap record, 4,097-message and 262,145-byte-metadata engine-over-default records,
and an exclusive held-lock wait of at least 500 ms. Formal cycle 3 rejected
exact candidate `9282b404`, tree `6d41f7ee`: correctness/API and native
boundary/effects each reported `0/0/1/0`, while performance/concurrency/
resources reported `0/0/1/1`. The deduplicated `0/0/1/1` findings are a medium
context-free recursion-budget mismatch and low self-counted allocation
evidence.

Exact remediation source `af055ff3b22e157b1c42d1579b041c3cc4c05b0e`, tree
`14eafada4b3dddd62a9cb8e6077ad8f0b81753e8`, implements the parent-aware 127-
container budget above. Exact ordinary-store/listing/session equivalence cases
accept/reject nested arrays at 123/124 for metadata, 120/121 for JSON content,
and 119/120 for tool-call and tool-result JSON. Focused evidence is green at 58
CLI process tests, including ten equivalence cases, and 22 native inspection
tests. Dev-only `allocation-counter` 0.8.1 instrumentation runs each of five
shapes in its own child process. Every shape reports exact allocation
total/current/maximum counts 14/2/8 and byte counts
8,913,715/14/8,913,347. The manifest and lockfile change solely for that dev
dependency. The complete replacement gate is green on exact `af055ff3`/
`14eafad` under Rust/Cargo 1.94.1 without fallback. Python passed 135 with eight
expected skips; pinned-fx `b1774fb` regeneration is byte-stable; WASI/FreeBSD
checks passed with only the established WASI `read_file` warning; and
documentation integrity is 85/147/626/81 with zero errors. Exact `cargo-deny`
0.20.2 passed all categories with three established duplicate warnings. Exact
`cargo-audit` 0.22.2 loaded 1,226 advisories, scanned 211 dependencies, and
found zero vulnerabilities. Crates.io `allocation-counter` 0.8.1 is dev-only
MIT/Apache, while the production normal/build dependency graph remains
unchanged at 364 lines. Diff, inventory, and no-added-unsafe checks are green.

The exact 4,001,760-byte release binary has SHA-256
`d296174898938f632351bebb38449533c7db03bb3659392bea3743a02ee1619d`. Its
session matrix passed 18/18, including ten equivalence cases, held-lock
behavior, and engine-over-default records. The direct 8,650,857-byte near-cap
case and native near-cap/allocation case each passed 1/1. Formal cycle 4
rejected exact candidate `df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. The three track counts are
`0/0/1/0`, `0/0/1/0`, and `0/0/1/0`; the deduplicated union is `0/0/2/0`.
Ordinary serde accepted positional sequences for five stored structs and an
externally tagged unit-role map that streamed inspection rejected. The
duplicate tracker also eagerly reserved approximately 8.9 MB even for empty
records. The replacement must enforce object-only stored structs and a string-
only role consistently, fallibly grow fixed-fingerprint entries/buckets in
proportion to unique keys under the unchanged ceiling, bound empty/small-record
allocator high-water use, and compare long and short values at equal structure.
Exact isolated components `65012426ee9e174c15ededac7c0c95f9f496b2cf`
for native remediation and `225fee4af82c24ea226db185801a7fe397407593`
for CLI differentials compose through `7c191b8`, `c73467e`, and exact
remediation `1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`. The complete local gate passed
under exact Rust/Cargo 1.94.1 without fallback. Focused 24 native and 64 CLI
process tests, including 16 differentials, are green. Python 135/8 skips,
byte-stable pinned fx, WASI/FreeBSD, docs 85/147/626/81, dependency policy/
audit, unchanged 364-line production graph, diff/inventory/no-added-unsafe,
release evidence, and the native near-cap probe are green. The 3,985,216-byte
release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c`.
Allocator evidence gives equal `12/2/7` allocation counts and `819/14/645`
bytes for empty/short/long text, equal `14/2/8` and `1,427/14/1,059` for
short/long JSON, and `35/2/9` plus `2,228,435/14/1,606,083` for 5,000 keys, in
total/current/maximum order. Formal cycle 5 rejected exact candidate
`8f533cdec235660c3e17b70fc5bbd5dd0ab8c1f6`, tree
`8215fb94fa3de08841b26dd9d7c63a2ecb7e8a8d`: correctness/API reported
`0/0/0/1`, native boundary/effects `0/0/0/0`, and performance/concurrency/
resources `0/0/0/1`, deduplicated to `0/0/0/1`. The sole low is stale status
across the maintained summary pages listed in the review ledger; production
review was otherwise green. Independent correctness evidence completed 312
generated valid/boundary and 1,200 randomized mutation differentials with zero
mismatch, while the native and performance allocation/resource tracks were
green. Cross-document status remediation is composed in exact cycle-6 candidate
`5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Correctness/API, native effects,
and performance/resources each reported `0/0/0/1`; the deduplicated `0/0/0/1`
is solely the self-pending remediation wording. There was no additional
production, API, native, or performance finding. Formal cycle 7 rejected exact
candidate `399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`: correctness/API and native effects
each reported `0/0/0/0`, while performance/resources reported `0/0/0/1`; the
deduplicated union is `0/0/0/1`. The sole low corrects the node/work-cap wording;
production and resources were otherwise green. Formal cycle 8 is green on exact
reviewed candidate `d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
`6439863a9b7fd1720156c790fedc4798256c2b6d`: every track and the deduplicated
union reported `0/0/0/0`. The review-exempt documentation result seal records
that reviewed candidate and does not imply its own commit was reviewed. Exact
seal `b6db9a67c070f7ef599d994c44b4a21731a004c5`, tree
`59dd628fd0552c5083449f7a31aa4241a8ecb952`, passed successful feature CI/
benchmark runs `32965947722`/`32965947723` and main CI/benchmark runs
`32966531225`/`32966531319`; each benchmark retained two unexpired exact-SHA
artifacts. Slice 32 is delivered and the delivered count is thirty-two.

Only a deduplicated `0/0/0/0` exact SHA may proceed through feature
workflows, integration, and exact `main` workflows. Exact reviewed candidate
`d724b61`, tree `6439863`, is review-green; seal `b6db9a6`, tree `59dd628`,
completed those delivery gates. With slice 32 delivered, bounded slice 33 has
returned to the remaining native tools through the frozen `web_search`
contract rather than expanding the CLI inspection surface. Its cycle-1
remediation remains pending as recorded above.
Every review/remediation iteration must end with committed/integrated worktrees
verified clean and safely removed; active or uncommitted worktrees are retained.

### Milestone 03 completion boundary

The thirty-two delivered slices do not complete Milestone 03.
The following checklist is the frozen M03 boundary; changing ownership requires
an explicit plan change in a reviewed commit rather than silently deferring a
gate:

- [x] Integrate and retain evidence for slices one through eight.
- [x] Integrate the ninth ask-handler slice under its exact contract and
  retain feature, adversarial-review, exact-SHA CI, and `main` evidence.
- [x] Integrate the tenth credential-discovery slice under its exact
  contract and retain feature, adversarial-review, exact-SHA CI, and `main`
  evidence.
- [x] Integrate the eleventh native host configuration schema-v2 slice under
  its exact contract and retain feature, adversarial-review, exact-SHA CI, and
  `main` evidence.
- [x] Compose a useful native reference-host path through an explicitly selected
  provider and transport, session store, permission handler and prompter, and
  registered tools. The CLI stays a thin host and owns no product state.
  Implementation, independent tests, composed adversarial review, exact
  feature-SHA gates, fast-forward integration, and exact `main` gates are green.
- [x] Add bounded, redacted credential acquisition and the configuration fields
  required by that composition. Source precedence, missing/invalid behavior,
  size limits, and secret non-reflection must be normative and tested; core
  receives no ambient credential or configuration authority. The thirteenth
  slice supplies the bounded non-secret selection. Implementation, independent
  tests, all three fresh adversarial tracks, exact feature gates, fast-forward
  integration, and exact `main` gates are green.
- [x] Add explicit workspace/state-root selection and safe required-root
  creation, plus native create, list, resume, replay, and reset session
  lifecycle behavior for the current schema. A reset under a reused session ID
  must allocate a new incarnation before reuse. The delivered fourteenth slice
  is limited to the root-selection and safe-creation sub-boundary. The
  delivered fifteenth slice implements by-ID create, resume, durable-record replay,
  and reset/new-incarnation behavior with fourteen independently owned focused
  tests plus one formal finding regression, with all three adversarial tracks
  green on exact candidate `e6a3804`; exact feature and `main` workflows are
  green through record `dbba2c7`. The delivered sixteenth slice supplies the
  remaining bounded IDs-only native listing functional scope. Production and
  13 initial independent tests are composed through first formal candidate
  `dec98e0`, whose three first review tracks were not green. The replacement
  source fix and 18-test hardened suite from `4b8d8b0` and `446b495` are
  composed in exact behavior candidate
  `3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
  green. Portable behavior `17f1884` is green under both executable reviews.
  Documentation seal `d3312d7` passed exact feature CI `32600292770` and
  benchmark evidence `32600292779`, was fast-forwarded without force to `main`,
  and passed exact main CI `32600567094` and benchmark evidence `32600567090`.
- [ ] Complete the M03 native tool set: `list_files`, `glob_files`,
  `grep_files`, `read_file`, `write_file`, `edit_file`, `delete_file`,
  `rename_file`, `copy_file`, `create_folder`, `file_info`, `open_file`,
  `web_fetch`, `web_search`, `terminal`, `ask_user_question`, `vision`, and
  `read_tool_result`. Every authority-bearing tool requires normalized
  preflight, exact policy/execution agreement, resource bounds, redacted
diagnostics, cancellation/drop tests, and platform scope stated before
integration. Slice 33 `web_search` is delivered through exact record
`52b5885`; its feature and main CI/Benchmark workflows are green. Slice 34
contract-freezes the bounded foreground-`exec` subset of `terminal` from that
exact base. Cycle 1 rejected exact candidate `fba499e` with a deduplicated
`0 blocker / 3 high / 3 medium / 4 low` product-finding union. Independent
evidence `8c58c29`, contract remediation `ddccdb2`, and lifecycle/resource
remediation `3725aed` compose through exact `b03ecb0`; all isolated remediation
worktrees are removed and pruned. The complete exact-1.94.1 replacement gate is
green with 1,150 non-documentation Rust tests, two doctests, 136 Python tests,
pinned-fx regeneration, dependency policy/audit, Linux/FreeBSD/WASI checks,
documentation/diff invariants, and fresh release-binary smoke. Three fresh
cycle-2 tracks rejected exact candidate `a566db2`, tree `785c084`, with a
deduplicated `0 blocker / 2 high / 1 medium / 2 low` union covering
deadline/output arbitration, bounded notification tails, normal-exit latency,
tilde grammar, and public Rustdoc. Every finding is accepted; cycle-3
documentation `2bf62a3`, evidence `6a15438`, and production `44f13ef`
components plus exact deadline-priority follow-up `62d0fe1` are composed. Every
isolated worktree was cleaned after integration. The exact behavior head
`62d0fe17`, tree `0e6f94b`, passes the complete exact-1.94.1 replacement gate:
24 portable external terminal tests, nine private terminal tests, 1,157 total
non-documentation Rust tests, two doctests, Python 136/8 skips, byte-stable
pinned fx, dependency policy/audit, Linux/FreeBSD/WASI, 89/312/678/515
documentation integrity, unchanged protected inputs, no added unsafe Rust, and
fresh release smoke are green. Formal cycle 3 rejected exact candidate
`3215b9e`, tree `3fafcad`: track counts `0/1/0/1`, `0/1/1/1`, and `0/0/1/0`
deduplicate to `0 blocker / 2 high / 2 medium / 0 low`. Accepted cycle-4
remediation must close the final overflow/deadline publication interleave and
share one originating admission slot through the outer execution, injected and
built-in Waker families, callback returns, and native thread completion. Every
cycle-3 review worktree was removed and pruned. Replacement composition, its
complete gate, three fresh reviews, and remote delivery remain pending, so this
combined item stays unchecked. The delivered
  seventeenth slice supplies only `file_info`; production
  and 34 focused tests are present and green at code-and-test head `f228c06`,
  with review hardening bringing the focused total to 36 plus five private unit
  tests at `b69ec4b`. The first formal candidate was not green; replacement
  local gates are green through `d445eb3`, and all three replacement tracks are
  green on exact candidate `4193ecc`. Seal and integrated SHA
  `60dd54f273afc7e62fb4b3cc1fb1a347d739998b` is green under exact feature CI
  `32605071080` on successful retry attempt 2, feature benchmark evidence
  `32605071063`, main CI `32606050292`, and main benchmark evidence
  `32606050294`; all four report that exact seal SHA. The other listed native
  tools remain incomplete, so this combined item stays unchecked. The
  delivered eighteenth `glob_files` slice has a frozen contract from base
  `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`; production, 39 focused
  integration tests, five private unit tests, documentation, composition, and
  initial local gates were green at `60070d8`, but the first formal review at
  `1f5de6a` found a high unmetered matcher-work defect. The checked matcher-
  budget fix, independent both-mode regression, 40 focused integration tests,
  nine private tests, and replacement local gates are green at exact code-and-
  test head `4171a4a8811a98888b7e4e161281a1216564746f`. All three same-SHA replacement
  adversarial tracks are green on exact behavior SHA `523df858`. Documentation
  seal and integrated `main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4`
  passed exact feature CI `32610950593`, feature benchmark evidence
  `32610950594`, main CI `32611208411`, and main benchmark evidence
  `32611208415`. Final documentation record `f6aa458` passed exact feature CI
  `32611623653` and benchmark evidence `32611623655`; tree-identical kickoff
  marker `f6ab594` supplied the missing exact `main` handoff evidence under
  feature CI `32612424382`, feature benchmark `32612424383`, main CI
  `32612662260`, and main benchmark `32612662203`. The nineteenth
  `grep_files` candidate is frozen from exact base `f6aa458`; its parallel
  production, independent tests, and maintained documentation must compose into
  one exact behavior candidate with all three formal review tracks green.
  Exact production `27eec2f` and initial independent-test `6eaee93` components
  exist and initially compose through `9057feb` and `44e33d7`; fixture fix
  `bdbb677` makes focused production/test composition green. Documentation
  component `b04151a` produces first fully composed behavior `42e4793`; lint
  fix and exact local gates are green at `45ad91f`. All three first-cycle tracks
  are **NOT GREEN** on exact `355a11a`. Remediation and exact replacement local
  gates are green at final code/test precursor `275d263`. First replacement
  candidate `ae87bf1` remains historically **NOT GREEN** across all three tracks.
  Second-fix production and documentation compose through `ac5d772`, `d672210`,
  `7ad0863`, and exact local-gate-green precursor `b498ba0`. Formal second
  replacement candidate `5aeddc1` has correctness/API and
  filesystem/robustness **GREEN** with zero findings and performance/concurrency
  **NOT GREEN** with one medium allocation-amplification finding and two low
  documentation/evidence findings. Third remediation composes through
  `8777825`, `ab1c133`, `dcf57ad`, `d7526d4`, `44afb23`, `f08c5f2`, and
  `1f13f9a` at exact fully composed local-gate precursor `a8f6179`. Exact Rust
  1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation
  tests plus two doctests, 25 private native tests, 40 direct tests, four engine
  tests, and diff checks are green. Exact a8f cross-target/dependency/link and
  compatibility/release validators are green. Formal third-cycle candidate
  `0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
  **GREEN** with zero findings. Correctness/API and performance/concurrency are
  **NOT GREEN** only for the same LOW documentation contract mismatch;
  reviewers confirmed zero production defects. Isolated wording remediation
  `993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
  `f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
  green, and its behavior tree remains `a8f6179` except for documentation.
  Formal fourth-cycle exact behavior SHA
  `8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero
  findings in all three fresh tracks: correctness/API, filesystem/robustness,
  and performance/concurrency. Exact-SHA formatting, warnings-denied workspace
  Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25
  private tests, 40 direct tests, four engine tests, and the 58/420/270/0
  documentation inventory are green. All historical findings are closed,
  including the attempted-read-window storage wording. This documentation-only
  seal is exempt from another adversarial review under the user's explicit
  instruction. Documentation seal `0f48806` is feature-green: CI run
  `32623585346` passed all six jobs and benchmark-evidence run `32623585349`
  passed both jobs and artifacts, all for exact `0f`. `main` was fast-forwarded
  without force from `f6ab594` to exact `0f`. Main CI run `32623904784` is
  **GREEN** for exact `0f`: all six jobs and every step passed without reruns.
  Main benchmark-evidence run `32623904800` is **GREEN** on attempt 1 for exact
  `0f`: both jobs and every step passed, with two valid non-expired exact-SHA
  artifacts retained. The `grep_files` slice is delivered; the remaining native
  tools remain pending. This final delivery
  record is documentation-only and exempt from adversarial review; its own exact
  remote workflows are required after push and cannot be self-recorded. The
  twentieth `write_file` contract is frozen at exact `3ee52fd` from exact base
  `bc042536`. Production `e9b3ad8`, independent tests `59a06a3`, and maintained
  documentation `9285fe9` compose through `c0d555b`, `c4c5ce6`, and `de46c3e`;
  fixture correction `8509933`, deterministic seam hardening `a9a7c99`, and
  core same-poll recovery regression `8b5847f` are present; full-pipeline fault
  and phase evidence is `c7fdef2`. Exact local gates are green at composed
  precursor `072bd69`. Formal-review preparation is `a7841c1`; exact first
  candidate `119938240807f8279f83e2ace65a69706e8fcfed` is tree-identical only to
  that immediate parent, while `072bd69` has a different documentation tree.
  All three first-cycle tracks are **NOT GREEN**. Documentation correction
  `016f8df` and code/evidence remediation `3010e6d` close the unbounded `EINTR`,
  real-pipeline target/parent race, and verification-phase cancellation
  findings. Replacement exact local gates are green at `581fe6a`. Exact cycle-2
  candidate `708f2d08d72d610ca387a62a4cec1f656c188a7d` is tree-identical only
  to immediate formal-review preparation parent
  `491496aa22aa8855717b74f6a026e8c602bb02e9`. Correctness/API is **GREEN** with
  zero findings; filesystem/robustness is **NOT GREEN** with medium private-
  residue-mode and unbounded-Linux-entropy findings; and performance/concurrency
  is **NOT GREEN** with the same medium entropy finding. Exact production
  remediation `9302ec3fa7d6e891fdc4a0c7bd8fe9b7cf8e427d` supplies direct
  nonblocking Linux entropy with cumulative 16-interruption/31-call bounds,
  the bounded one-call macOS path, cancellation before and after each entropy
  call, and one best-effort held-descriptor `fchmod(0600)` before cleanup. If
  mode restoration and unlink both fail, residue can retain final bits. Focused
  checks pass 28 supported-submodule private tests, 109 native library tests,
  25 direct tests, formatting, workspace warnings-denied Clippy, and the Linux
  cross-check. Exact clean remediation precursor
  `8432c0c6b5d5955b78a882b651a5bfec76af8814` then passes the complete local
  Rust 1.94.1 gate: 30 total private module tests, 25 direct tests, five engine
  tests, 611 default/660 all-feature discovered tests, two doctests, zero
  benchmarks, cross/dependency/Python/compatibility/documentation/release/diff
  checks, and a clean worktree. The isolated Python retry and sequential full
  rerun establish that its earlier LTO-overlapped two-second timeout was
  validation-method contention, not a product failure. No behavior-green or
  replacement claim arose from cycle 2 alone. Exact local-gate record
  `9a09172ac40d7ec09ebb9fa7a4e4e21f12b2a632` is retained. Its immediately
  following formal-preparation and tree-identical exact candidate
  `db78c6407c4f603f18e2839a8a291f2de33e579c` are now **GREEN** with zero
  findings in all three fresh tracks. Candidate fmt, workspace Clippy/tests, two
  doctests, and diff/clean checks are green. Behavior-green SHA is `db78c640`.
  Documentation seal `bdd27ec969769d94c78efdab8e07cbd6b600ca3f` is green
  under feature CI `32633372927` and benchmark evidence `32633372982`, was fast-
  forwarded without force from `bc042536` to `main`, and is green there under
  CI `32633639774` and benchmark evidence `32633639763`; each benchmark run has
  two nonexpired exact-SHA artifacts. Native `write_file` is delivered. This
  docs-only seal is exempt from further adversarial review. Local
  evidence is native macOS, Linux and
  FreeBSD cross-compilation, and active WASI unsupported-target execution;
  exact feature CI native Linux/macOS is green on the seal. This final delivery
  record is documentation-only and exempt from adversarial review under the
  user's instruction; its own exact feature and `main` workflows are required
  after push and cannot be self-recorded. The
  twenty-first `edit_file` slice is **DELIVERED** from exact delivered base
  `242adfed4be717baf7cd07275aae40ec8a3637f6`. Its strict effect-free contract,
  distinct `FilesystemAccess::Edit` authority, exact-one overlapping matcher,
  393,216-step public work cap, bounded existing-file atomic-replacement
  protocol, parallel ownership, and fresh same-SHA review requirements are
  frozen in [`edit-file.md`](edit-file.md) and
  [`m03-edit-file-review-01.md`](reviews/m03-edit-file-review-01.md). Contract
  `bb0c381f8b044f7849bef80cc482034e1dd57ecf` is green under exact CI
  `32634883133` and benchmark workflow `32634883139`. Production and independent
  evidence are composed through exact local-gate precursor
  `31ec79e000589c4fb34599be4aad4f90ea33974f`; focused suites, the Rust 1.94.1
  workspace gate, 130-test repository harness, pinned-fx compatibility,
  dependency, portability, documentation, release-smoke, no-unsafe, diff, and
  clean-worktree checks are green. Exact cycle-1 candidate
  `8fdb67892f34a0fbfbb90a54e8eda982159813bf` was not green: correctness/API
  was green with zero findings; filesystem/robustness had a high staged-
  mutation false-success finding and a low incomplete-evidence finding; and
  performance/concurrency had the same high finding. Production remediation
  `578ef3cf2061568d02a160fbe7a498203880b9e9` composes as
  `013016f276e023838ffe7ddf8a79121a3ee463a1`, and
  independent test component `59471147817ed7520513fdf51041ec24c822bfe3`
  composes as exact precursor
  `482d33c0bc586ff594d5b0decc58de347cb9243e`. Its three corruption regressions
  are red on the failed candidate and green on the fix; focused totals are 30
  private, 24 direct, five engine, and seven reference-host tests. Hostile-
  umask and moved-retained-parent evidence are now direct. Exact cycle-2
  candidate `f84bac87f472fc851eca670764657e5a31ce0256` confirmed that
  corruption fix; correctness/API and performance/concurrency were green with
  zero findings, while filesystem/robustness was not green with a high macOS
  inherited-ACL defect and low incomplete deterministic-evidence finding.
  Production component `22197389a521095132c02125726dbe67fbf06d1b` clears and
  verifies an empty macOS stage ACL before write and revalidates it after final
  sync and publication. Independent component
  `65d40d99f4e026834a05778029800fa703c9379e` exercises the generic evidence
  seams across traversal, all four logical read phases, staging, final sync,
  post-real-rename cancellation, and cleanup dual failure. Exact composed
  remediation behavior `ab6841388838384e27e6299151d50bb83d2ec46e` passes 39
  private, 24 direct, five engine, and seven reference-host focused tests.
  Exact cycle-3 candidate `da1537b229393007101264cd7bc8fd12ee393a3d` is
  correctness/API and performance/concurrency green with zero findings;
  filesystem/robustness is not green only for four low evidence gaps and
  reports no production atomicity defect. Source `985b232` and tests `3cbb895`
  compose through `1b9ffca` and exact integrated remediation `d0d188b`, with
  actual generic RAII cleanup evidence, the after-final-verification checkpoint,
  ACL evidence methods, ordinal-aware fault ranges, and four regressions.
  Focused `edit_file` totals are 43/24/5/7; delivered `write_file` regressions
  remain green at 30/25/5. Exact tree-identical candidate `78d6fd7` is green
  with zero findings in all three fresh cycle-4 tracks. The exact Rust 1.94.1,
  684/733-test discovery, 130-test Python, pinned-fx, dependency,
  Linux/FreeBSD/WASI and active Node, documentation, no-unsafe/diff, and fresh
  release-smoke gates are green. Documentation seal `c1268fd` passed feature CI
  `32650254095` and benchmark workflow `32650254086` with two exact-SHA
  artifacts, then was fast-forwarded without force from `242adfe` to `c1268fd`
  on `main`. It is green there under CI `32650593685` and benchmark workflow
  `32650593703` with two nonexpired exact-SHA artifacts. Native `edit_file` is
  delivered.
  Documentation-only commits are exempt from a separate adversarial cycle under
  the user's instruction. Final documentation record `719a9bd` is green under
  exact feature CI `32651168514`, feature benchmark `32651168515`, main CI
  `32651488265`, and main benchmark `32651488282`; both benchmark workflows
  retain two nonexpired exact-SHA artifacts. The remaining native tools are
  incomplete, so delivery of this slice does not change the combined checkbox
  or complete Milestone 03. The twenty-second `delete_file` slice is
  **IN PROGRESS** from exact delivered base
  `719a9bded86fd7ce394d482798b9064c736f43ab`. Its strict path-only `Delete`
  authority, one-file-or-empty-directory descriptor protocol, non-retried
  `unlinkat`, bounded postcommit parent sync, fixed redacted errors, disclosed
  final different-entry race, eight-tool host composition, parallel ownership,
  evidence matrix, and fresh same-SHA review protocol are frozen in
  [`delete-file.md`](delete-file.md) and
  [`m03-delete-file-review-01.md`](reviews/m03-delete-file-review-01.md).
  Contract commit `78ed6292386f86e5807bcf72591d6cb5d9f45c45` is green under
  exact feature CI `32652361712` and benchmark workflow `32652361692` with two
  exact-SHA artifacts. Implementation and independent evidence compose through
  exact local-gate precursor `5e340155f9a38b81a2812942d6ad0a796164beb5`;
  focused, workspace, Python, compatibility, dependency, portability,
  documentation, no-unsafe/diff, and release-smoke gates are green. Exact
  cycle-1 candidate `7c6f7ee` is **NOT GREEN** across all three fresh tracks,
  with four unique findings covering requested-path work precedence, root
  permission taxonomy, macOS diagnostic-metadata cancellation, and the
  portable final non-directory replacement boundary. Exact remediation
  `60e81a6` passes the complete replacement local gate with 22/23 private, 20
  direct, five engine, seven host, 732/782 discovery, workspace, Python,
  compatibility, dependency, portability, documentation, diff, and release-
  smoke evidence green. Tree-identical cycle-2 candidate `88026f1` is
  performance/concurrency green but correctness/API and filesystem/robustness
  not green with three overlapping medium defects and one additional low
  Rustdoc mismatch. Exact remediation `225e961` passes the complete replacement
  local gate with 28/29 private, 20 direct, five engine, seven host, one core
  contract, 738/788 discovery, workspace, Python, compatibility, dependency,
  portability, documentation, diff, and release-smoke evidence green. Another
  exact cycle-3 candidate `24f851d` is performance/concurrency green but
  correctness/API and filesystem/robustness not green with one low finding
  each: validation-site `EROFS` taxonomy and missing hostile-umask regression.
  Exact remediation `77884a9` passes complete local gates with 28/29 private,
  21 direct, five engine, seven host, one core, 739/789 discovery, workspace,
  Python, compatibility, dependency, portability, documentation, diff, and
  release-smoke evidence green. Tree-identical cycle-4 candidate `0b732d2` is
  not green across all three tracks for the same single medium definitive-
  unlink-failure cancellation finding. Exact remediation `4273de5` passes the
  complete replacement gate with 29/30 private, 21 direct plus hostile-umask
  child, five engine, seven host, one core, 740/790 discovery including two
  doctests and zero benchmarks, workspace, Python, compatibility, dependency,
  portability, documentation, diff/no-unsafe/no-Cargo/no-CLI, and fresh
  release-smoke evidence green. Its ten-case definitive-failure matrix proves
  post-syscall cancellation precedence while preserving the success/`EINTR`
  commit boundary. Tree-identical cycle-5 candidate `8575354`, tree `13f28f2`,
  is green with zero findings in all three fresh correctness/API, filesystem/
  robustness, and performance/concurrency tracks. Feature delivery and `main`
  delivery remained pending at that point. First seal `9e2a276` passed benchmark workflow
  `32663557187` with two artifacts but CI `32663557182` failed only aarch64
  Linux after same-type test-fixture inode reuse. Test-only remediation
  `c6744ab`, tree `2ac83ee`, retains the original unlinked handle through
  revalidation and passes complete Rust, policy, portability, docs, diff, and
  release-smoke gates without changing production behavior. Tree-identical
  cycle-6 candidate `9e817be`, tree `d63a92f`, is green with zero findings in
  all three fresh tracks; 500/500 sequential and 64 parallel repaired-fixture
  stress checks pass. Replacement seal `fe56f4c` passed exact feature CI
  `32665295323`, feature benchmark `32665295321`, main CI `32665564381`, and
  main benchmark `32665564382`; both benchmark workflows retain two nonexpired
  exact-SHA artifacts. Native `delete_file` is delivered as slice twenty-two.
  The combined native-tool checkbox stays unchecked because the remaining
  native tools and M03 ownership are incomplete. The twenty-third
  `rename_file` slice is **DELIVERED** from exact base
  `3d76f2e`.
  It is frozen as one existing confined regular file moved to an absent
  destination through exact two-endpoint `FilesystemRename` policy input,
  existing no-follow parents, one `NOREPLACE` rename, postcommit identity
  verification, and bounded one- or two-parent sync. It deliberately adds no
  overwrite, directory move, parent creation, external path, content read,
  copy/delete fallback, CLI behavior, new dependency, benchmark workload,
  performance claim, or fx-equivalence claim. Contract `19cad7d`, production
  composition `d8f7367`, and independent-evidence composition `1dab9a0` are
  present. Exact composed local-gate precursor `43847fe`, tree `80cb9a1`, is
  green. Exact cycle-1 candidate `2bc4f9a`, tree `44558a0`, is **NOT GREEN**
  in all three fresh tracks because retained terminal race, `EINTR`/errno,
  postcommit, sync-bound, late-cancellation, and moved-parent evidence was
  incomplete; a low documentation finding also required the final directory-
  replacement race to be explicit. Exact remediation `a3491cf`, tree
  `0b195bd`, expands the private suite to 15 tests for the requested matrix,
  clarifies that race, and passes the complete replacement local gate. Tree-
  identical cycle-2 candidate `4f224a5`, tree `cb75dca`, is green with zero
  findings in all three fresh tracks. First seal `a03a57b` passed exact feature
  benchmark `32671805335`; feature CI `32671805412` reproduced an unrelated
  pre-existing Linux session-lifecycle fixture deadlock. Exact test-only
  remediation `2c771ed`, tree `5de94a6`, passes the complete replacement local
  gate without changing production. Tree-identical cycle-3 candidate
  `5cc1523`, tree `99b88ec`, was not green because two fresh tracks found the
  unpinned device/inode-reuse race. Exact remediation `4cbd46f`, tree
  `35f531e`, pins the source with a non-reading descriptor through commit
  verification and passes the complete replacement local gate. A tree-identical
  cycle-4 candidate `1337980`, tree `ab2bdc2`, is green with zero findings in
  all three fresh tracks. Exact replacement feature/main delivery gates are
  green under seal `7cb5ef9`: feature CI `32675233513`, feature benchmark
  `32675233542`, main CI `32675562978`, and main benchmark `32675562956`, with
  exactly two nonexpired exact-SHA artifacts retained by each benchmark run.
  Native `rename_file` is delivered as slice twenty-three under
  [`rename-file.md`](rename-file.md).
  The twenty-fourth `copy_file` slice is **DELIVERED** from exact
  delivered base `226040780eb14dd72e86d0a002dc4bf61ba2ddfc`. It is frozen as
  one existing confined regular file copied without source mutation to one
  absent confined destination through exact two-endpoint `FilesystemCopy`
  policy input, bounded binary streaming into a private destination-local
  stage, source and staged-content verification, one `NOREPLACE` publication,
  postcommit destination verification, and bounded destination-parent sync.
  It deliberately adds no overwrite, parent creation, directory copy, symlink
  following, external path, full-content allocation, CLI behavior, new
  dependency, benchmark workload, performance claim, or fx-equivalence claim.
  Contract commit `6021fb0` is green under exact CI `32677160680` and
  benchmark workflow `32677160652`, with two exact-SHA artifacts. Production
  component `9ab8d90` and maintained documentation component `622b9d4` compose
  at exact local-gate precursor `622b9d4bfd9e3bbbe34165f5dd64c5b2bf7996d4`.
  The Rust 1.94.1 gate is green across 20 private, 24 direct, five engine, seven
  host, and one core focused tests; 829/877 discovered tests with zero
  benchmarks; workspace formatting, warnings-denied Clippy, default and all-
  feature tests, and two doctests; Python 130 with eight expected macOS skips;
  pinned-fx compatibility; cargo-deny 0.20.2 and cargo-audit 0.22.2 over 175
  dependencies; Linux/FreeBSD/WASI and active Node 1/1 checks; documentation
  68/483/333/0; clean diff/no-unsafe/no-Cargo/no-CLI checks; and a fresh release
  CLI with SHA-256
  `1e8c5aefd32ab12f201c1527b38f86ef31463c80be1f75a4901f9e00930f3c24`
  passing bare/help/status smoke paths. Exact tree-identical cycle-1 candidate
  `38d0d801caf1174d6df951a03d5843d6c217eb1a`, tree
  `f0eadf23e4cdfa6613f866ef5923806a8474cb0e`, is **NOT GREEN** in all three
  tracks despite passing reviewer-focused checks. Correctness/API and
  filesystem/robustness share one medium finding: cancellation or initial
  metadata rejection after successful exclusive stage creation can precede
  cleanup-guard ownership and leave the tool-owned stage pathname behind.
  Performance/concurrency found a medium postcommit source-parent rewalk defect:
  source validation can reuse the prepublication retained parent instead of
  freshly resolving the requested path from the retained root. That track also
  found low missing exact serialized-bound and source-size-independent
  allocation evidence. Production and evidence remediation composes at exact
  `53f4ee947c82033a08a2ff943f23f52c475189d7`, tree
  `4bdb07a30950584d71260e70e263aafcccfff710`. It establishes immediate
  cleanup-guard ownership after stage creation, check-before/raw-call/check-
  after cancellation ordering for precommit native operations, a fresh
  cancellation-ignoring postcommit source-parent rewalk, exact serialized
  guards, and one reusable 64-KiB buffer observed across every streaming phase
  for empty, one-byte, and exact-limit inputs. The complete replacement local
  gate is green across 25 private, 24 direct, five engine, seven host, and one
  core focused tests; 834/882 discovered tests with zero benchmarks; full Rust
  and Python suites; dependency, compatibility, Linux/FreeBSD/WASI, active Node
  1/1, documentation, diff, release-hash, and CLI-smoke checks. Exact tree-
  identical cycle-2 candidate
  `ad4af0c2c642cc315724a3515bacd9aa70cbe17f`, tree
  `9e09fd7ba5b486847b8302629193f3e665831d81`, is green with zero findings in
  all three fresh correctness/API, filesystem/robustness, and performance/
  concurrency tracks. The reviewers independently verified the immutable SHA
  and tree in clean detached worktrees. Correctness/API passed 25 private, 24
  direct, five engine, seven host, and one core-contract test. Filesystem/
  robustness passed 25 private and 24 direct tests. Performance/concurrency
  passed 25 private, 24 direct, five engine, and focused warnings-denied Clippy.
  Exact feature/main delivery gates remained pending at that checkpoint under
  [`copy-file.md`](copy-file.md). This documentation-only review seal is exempt
  from its own adversarial cycle. First feature seal
  `16b92ef1a409fdca78ddb86ce4ae7879b89e65d6` passed benchmark workflow
  `32683596971` with exactly two nonexpired exact-SHA artifacts. CI
  `32683596986` passed all four native target matrices and dependency policy/
  audit, but its quality job failed warnings-denied Clippy before tests on two
  Linux-only `unnecessary_wraps` diagnostics for the no-op ACL shims. Exact
  portability remediation `bb21c7aa91554b8958c69b15c2b93dba7aed2755`, tree
  `c7fe63b030cc1de468c7694ce7e0c67c86866ab8`, adds two scoped, reasoned lint
  allowances to preserve the shared fallible macOS/Linux interface without any
  behavior, test, dependency, or CLI change. Its complete replacement local
  gate is green across Linux and macOS warnings-denied Clippy; full workspace,
  focused, doctest, Python, compatibility, dependency, Linux/FreeBSD/WASI,
  active Node, documentation, diff, and release-smoke evidence. Exact tree-
  identical cycle-3 candidate
  `99ecdb3aa9051cd74d997c194c43c8cb496a7277`, tree
  `145b3bee6976e42ade02a681fcd0d047a364cf5c`, is green with zero findings in
  all three fresh tracks. Correctness/API passed 25 private, 24 direct, five
  engine, seven host, one core-contract, Linux lint, and FreeBSD/WASI checks.
  Filesystem/robustness passed 25 private, 24 direct, and Linux lint checks.
  Performance/concurrency passed 25 private, 24 direct, five engine, and Linux
  lint checks. All independently confirmed the allowances are interface-parity
  metadata only and preserve the complete cycle-2 behavior. Replacement
  documentation seal `3bdd7cb36c2ef3be0ffcd0ac118adb39706c6be8` is green under
  exact feature CI `32684856309`, feature benchmark `32684856373`, main CI
  `32685192453`, and main benchmark `32685192394`, with exactly two nonexpired
  exact-SHA artifacts retained by each benchmark run. Native `copy_file` is
  delivered as slice twenty-four under [`copy-file.md`](copy-file.md).
  The twenty-fifth `create_folder` slice is **DELIVERED** from exact
  delivered base
  `d1a5bc24112bcede8c2d12789e763a12cf44bd4a`. Base feature CI `32685885104`,
  feature benchmark `32685885086`, main CI `32686210561`, and main benchmark
  `32686210659` are green; both benchmark workflows retain exactly two
  nonexpired exact-SHA artifacts. The strict `{path:string}` contract binds
  policy and execution to one canonical confined path through existing
  `FilesystemAccess::Create`, recursively creates missing directory components,
  treats an existing final directory as idempotent success, and rejects every
  symlink, non-directory ancestor, and external path. At most 256 `mkdirat`
  calls request mode `0755`; effective permissions and ACLs honor host
  umask/inheritance without later normalization. No creation call is retried,
  the first successful or uncertain call commits, cancellation is ignored
  afterward, and no prefix is rolled back. A fresh public-path rewalk and
  bottom-up durability use at most 257 sites, 16 calls per site, and 4,112
  total sync calls. Exact frozen contract commit
  `9fab189c9c1add76a38775d08f4342c6bcc7635b` passed all six jobs of CI
  `32687614476`; benchmark workflow `32687614442` passed both jobs and retains
  exactly two nonexpired exact-SHA artifacts. Candidate source composes
  `create_folder` after `copy_file` for eleven tools and one original plus ten
  clones. Exact precursor `ea408a1`, tree `7055e93`, passes the complete local
  gate across 16 private, 20 direct, six engine, seven host, and one core-
  contract focused tests; 877 default and 925 all-feature discovered tests with
  zero benchmarks; full Rust and Python suites; dependency and compatibility;
  native macOS execution; Linux/FreeBSD cross-target test compilation; Linux
  library Clippy; WASI compilation and active Node 1/1; documentation, diff,
  release-hash, and CLI-smoke checks. Production preflight API and filesystem
  audits found no issues but are not the required formal three-track review. A
  frozen candidate SHA, fresh same-SHA review, feature workflows, integration,
  and main workflows remained pending at that checkpoint under
  [`create-folder.md`](create-folder.md) and
  [`m03-create-folder-review-01.md`](reviews/m03-create-folder-review-01.md).
  Exact cycle-1 candidate `8ce899a`, tree `065cd19`, is not green because two
  tracks found the same low stale-local-gate documentation inconsistency;
  filesystem/robustness is green and all tracks found zero production defects.
  Every reported passage is fixed. Exact remediation `7bc3fb9`, tree `b39bd9b`,
  passes the complete replacement local gate across all Rust, focused/
  discovery, Python, compatibility, dependency, Linux/FreeBSD/WASI, active
  Node, documentation, diff, and release-smoke checks. Tree-identical cycle-2
  candidate `6e1f885aa1e167e902b5cda729023fd7c283895e`, tree
  `ac57575c3ee300050f5a92d4cae5f507fe654002`, is not green. Correctness/API
  and performance/concurrency are green with zero findings. Filesystem/
  robustness reported two low findings and zero production defects: checked
  subordinate-mount coverage lacked deterministic changed-`st_dev` evidence,
  and maintained documentation overclaimed native Linux execution. Evidence
  remediation composes deterministic mixed-device identity traversal without
  privileged real-mount or sandbox claims and corrects current execution
  evidence to native macOS plus Linux cross-target test compilation/library
  Clippy only. Exact remediation
  `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
  `40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
  local gate with exact Rust 1.94.1 full-workspace formatting, warnings-denied
  all-target/all-feature Clippy, tests, and two doctests; focused totals
  17/20/6/7/1; discovery totals 878/926/0; Python 130 with eight expected macOS
  skips; byte-identical pinned-fx compatibility; `cargo-deny` 0.20.2 and
  `cargo-audit` 0.22.2 with zero findings across 175 dependencies and 1,225
  advisories; Linux/FreeBSD cross-target checks and Linux warnings-denied
  library Clippy only; WASI compilation and active Node 22.22.0 evidence 1/1;
  documentation 70/502/352/0; clean diff/no unsafe/no Cargo manifest/lockfile/
  CLI/workflow/benchmark-workload changes; and the fresh 319,152-byte Mach-O
  arm64 release SHA
  `71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`
  with bare/version/help/inert human-and-JSON smokes. Documentation record
  `9d0bacd`, tree
  `b5fb1c2`, and tree-identical cycle-3 candidate `c1e572e` retain identical
  non-documentation behavior. Cycle 3 is not green: filesystem/robustness and
  performance/concurrency are green with zero findings, while correctness/API
  reported one low documentation-lineage finding and zero production defects.
  Exact lineage remediation `12c11ba`, tree `b96575b`, passes the complete
  replacement gate. Gate record `f6f6584` parents tree-identical cycle-4
  candidate `a78b693`, tree `2b913e8`. Correctness/API and performance/
  concurrency are green with zero findings. Filesystem/robustness found zero
  production defects and one low stale documentation-seal sentence, corrected
  under the user's seal-review exemption. First feature benchmark
  `32699750662` is green with exactly two nonexpired exact-SHA artifacts. First
  feature CI `32699750602` has its dependency-policy and all four native Linux/
  macOS jobs green, but the overall workflow is not green because Linux Quality
  rejected the test-only `u32::from(Mode::bits())` trace conversion. The trace
  now stores platform-native `rustix::fs::RawMode`; focused macOS, Linux
  warnings-denied test-target Clippy, and FreeBSD compilation checks are green.
  Exact remediation `1effcbb5fd5affa1bc23df938afc7d786e5c05ea`, tree
  `b5eccb193db00b39c9e029cb3e3b472283b3e6ba`, passes the complete replacement
  gate across full Rust, focused/discovery, Python, pinned-fx compatibility,
  dependency, portability, documentation/diff, and release-smoke evidence.
  Tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with
  zero findings in all three fresh tracks. Seal `e75578b` passed exact feature
  CI `32702785549`, feature benchmark `32702785574`, main CI `32703303933`, and
  main benchmark `32703303931`; both benchmark runs retain exactly two
  nonexpired exact-SHA artifacts. Feature delivery and `main` integration are
  complete. It adds no product-performance or fx-equivalence claim, so the
  combined native-tool checkbox remains open while the delivered count becomes
  twenty-five.
  The twenty-sixth native `open_file` slice is **DELIVERED** from exact
  delivered base `e2ee11f2`. Final base
  main CI `32704202572` and benchmark `32704202546` are green; the benchmark
  retains exactly two nonexpired exact-SHA artifacts `9511626648` and
  `9511745538`. Its sole strict `{path:string}` names one workspace-confined
  existing regular file and produces dedicated
  `Capability::OpenFile { path }` authority. Linux launch uses only fixed
  `/usr/bin/xdg-open` and a `/proc/<parent-pid>/fd/<retained-fd>` target while
  the descriptor stays retained; it never resolves the model-selected file
  through ambient `PATH` itself or a mutable workspace pathname. Null standard I/O,
  an injected evidence boundary, a 30-second helper wait, fixed redacted
  results, a serialized spawn/cancellation/drop gate, and owned-helper cleanup
  behavior are normative. Abort-first guarantees zero launch; successful spawn
  commits. Prepublication drop/cancel suppresses waking, reaps the helper, drops
  the request/descriptor, and synchronously joins; normal postpublication
  cleanup joins too. An overlapping inline or blocking arbitrary Waker may
  release the `JoinHandle` to avoid deadlock, leaving only callback/final
  bookkeeping after helper/request cleanup. A fixed global 32-launch permit is
  retained through callback completion and worker return, bounding that tail.
  This documentation-only amendment replaces the frozen absolute no-worker-
  detach clause because legal Waker reentrancy/blocking made it contradictory;
  it is exempt from its own adversarial review under the owner's instruction.
  Other targets are
  unsupported before spawn. External
  paths, directories, URLs, symlinks, a concrete macOS launcher, CLI changes,
  benchmark changes, performance claims, and fx-equivalence promotion are
  deferred. Candidate production, independent direct/private/engine/
  unsupported evidence, and exact twelve-tool/eleven-clone host composition
  are present without dependency, workflow, CLI, benchmark, or compatibility-
  status changes. Formal cycle 1 rejected exact candidate `79e65c1`, tree
  `481fd7c`, for failed-operation cancellation precedence, an unserialized
  spawn/abort race, inline-waker self-join, and a wait/waiter evidence mismatch.
  Candidate remediation and five deterministic regressions were present.
  Formal cycle 2 rejected exact candidate `027ba3367eb0853fec828ed0900398c7b7458e71`,
  tree `9002e8f137d5ed2352cd620db6145da2339cdb2c`, for unbounded pre-path
  serialization, post-deadline acceptance, no active worker cap, a detached
  Waker tail retaining request/FD ownership, the frozen no-detach contradiction,
  numeric-proc-fd test races, a non-inert fake launcher, and a forced wait seam
  that bypassed the actual shared `Err` arm. Required evidence now includes a
  very-large path, authoritative deadline/remaining sleep, active 32-permit
  saturation, exact-FD closure under a blocked Waker, identity-aware proc tests,
  inert fake launch, and the actual shared wait-error arm. Formal cycle 3
  rejected exact candidate
  `6815843ac2c8d7731ca6554e5a84772351def850`, tree
  `4a479b51ebdba49afb81a6827f1381d01ed75e52`. Correctness/API is green with
  zero findings. Performance/concurrency reported one low because the deadline
  regression stopped before exercising the authoritative post-`try_wait`
  clock. Filesystem/process-lifecycle reported one low because a no-Waker
  publication gap could detach a normal tail instead of joining it, although
  the tail remained permit-bounded and the helper, request, and retained
  descriptor were already cleaned. The cycle has zero blocker, high, or medium
  findings, zero other findings, and no production resource escape. Candidate
  remediation atomically publishes `notification_complete` when no Waker exists
  and adds deterministic after-wait-probe deadline and no-Waker normal-join
  regressions. Exact cycle-4 candidate
  `4632162f8d3f323fce65263ec92f0802d9416121`, tree
  `ab1ecebe1680813614db3682f505e5de0fc31cfc`, passed the complete replacement
  local gate. Lifecycle and performance were green with zero findings;
  correctness/API found no production defect and one low stale maintained-
  documentation lineage finding. That remediation was composed into exact
  cycle-5 candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
  `90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks found zero
  production defects and the same low remaining current-lineage wording defect.
  That correction is composed in exact cycle-6 candidate
  `b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
  `07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
  filesystem/process-lifecycle, and performance/concurrency tracks are green
  with zero findings at every severity. Native Linux arm64 Rust 1.94.1 passed
  system 14/14, direct 12/12, engine 4/4, warnings-denied Clippy, and repeated
  performance evidence 70/70. Correctness passed core serde 1/1, macOS active
  unsupported behavior 1/1, and all-feature host composition 1/1. The full
  local gate, including Python 130 with eight expected macOS skips, pinned-fx
  `b1774f`, dependency, FreeBSD/WASI, active Node 1/1, documentation, and the
  319,152-byte release binary with SHA-256
  `4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`, is
  green. Seal and integrated `main` SHA
  `a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
  `03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
  at 6/6 and feature benchmark `32738160725` at 2/2, retaining upstream
  artifact `9524219365` and bootstrap artifact `9524052760`. Main CI
  `32738798417` passed 6/6, and main benchmark `32738798415` passed 2/2 while
  retaining upstream artifact `9524461989` and bootstrap artifact `9524298408`.
  Feature delivery, non-force fast-forward integration, and exact `main`
  workflows are complete under
  [`open-file.md`](open-file.md) and
  [`m03-open-file-review-01.md`](reviews/m03-open-file-review-01.md). The
  documentation-only contract checkpoint and final docs-only record are exempt
  from their own adversarial cycles under the user's instruction. At that
  checkpoint, the final record's own exact feature and `main` workflows still
  required reporting. Final delivery record
  `762d70df106d40e59b599e18b1ac5c62f678927d`, tree
  `909eb320e05df4d56f5bcecf0e3655e6d761f622`, passed exact feature CI and
  benchmark plus exact main benchmark, but main CI was not green because the
  immediate `exit 0` fixture could publish before installing `BlockingWake`.
  Exact cycle-7 candidate `ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
  `f8a681db319f0a89e21f38e7f9f8c474c270452b`, is rejected: correctness/API
  and lifecycle are green with zero findings; performance/concurrency is not
  green with exactly two low findings and zero blocker, high, or medium
  findings. The unconditional `before_first_wait` rendezvous could hang on a
  pre-hook `Command::spawn` failure, reproduced on native Linux arm64 with
  `/tmp` mounted `noexec`; and maintained operative/current tails stopped at
  the superseded cycle-6/handoff state. Exact test-only code remediation
  `274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
  `865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the
  existing `before_spawn` barrier, reached before every spawn outcome, and
  passes normal native Linux arm64 exact testing 100/100 plus the `/tmp`-
  noexec spawn-failure case 1/1. Production source, public API, and manifests
  are unchanged. The docs correction composes atop that commit with SHA
  pending. The full replacement local gate and all three fresh cycle-8 tracks
  remain pending. The executable test-only fix is not eligible for the docs
  exemption. This makes no product-performance or fx-equivalence claim.
  cycle-8 candidate `6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
  `44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the full local gate.
  Correctness/API rejected it with exactly two low findings and zero blocker,
  high, or medium: no successful-helper witness in the normal fixture and
  operative lineage that did not name the known candidate. Lifecycle and
  performance/concurrency were green at zero findings. Exact remediation
  `a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
  `7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, adds distinct successful-marker
  and deterministic spawn-failure tests around shared lifecycle assertions.
  Normal Linux arm64 evidence is 202/202 and the noexec failure case is 100/100.
  Production source, public API, manifests, and workflows remain unchanged.
  The documentation correction/cycle-9 candidate, full replacement gate, and
  three fresh cycle-9 reviews remain pending. This makes no product-performance
  or fx-equivalence claim. Exact cycle-9 candidate
  `964c59408bda1a3793978041432b84b808b474a6`, tree
  `7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the full replacement
  gate. All three fresh correctness/API, filesystem/process-lifecycle, and
  performance/concurrency reviews are green with zero findings at every
  severity. Production source, public API, manifests, and workflows remain
  unchanged. A documentation-only green seal naming that reviewed candidate
  is exempt from another adversarial cycle; its exact SHA/tree and feature/main
  workflows remain pending. This makes no product-performance or
  fx-equivalence claim. Exact contract commit `6b763c4` passed
  all six feature CI jobs in `32707583915`; feature benchmark `32707583892`
  passed both jobs and retains exactly two nonexpired exact-SHA artifacts,
  `9512848704` and `9512966283`. Those runs establish only the frozen contract
  checkpoint, not implementation or delivery. Native `open_file` is delivered
  as slice twenty-six. The current host has exactly twelve alphabetical tools
  using one retained descriptor plus eleven identity-preserving clones. This
  makes no product-performance or fx-equivalence claim. The combined native-
  tool checkbox stays open because the remaining named tools are incomplete.
  The delivered twenty-seventh `web_fetch` slice started from exact base
  `a56ff350c2aace1dc22cb14c269aee89d399cd8e` under
  [`web-fetch.md`](web-fetch.md). It is rootless, so its production and
  explicitly injected/custom candidate hosts each have thirteen tools without
  changing the twelve-tool original-plus-eleven-clone workspace descriptor set.
  Production and independent tests are composed. Pre-review
  gate record `0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
  `5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local gate.
  Formal cycle 1 rejected exact candidate
  `3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
  `1378b02e92973ab15fbf4623138a643b70057f33`, with findings in every track.
  Cycle-1 remediation, independent evidence, and the static-fixture portability
  correction are composed through exact replacement precursor
  `5a7960f6e728bf5681e91a411710b4c24dbd6991`, tree
  `f1ed559f0328b8eda721b7b28bcb6fcdb95367b2`. Its complete exact Rust 1.94.1
  replacement gate is green across required/full/focused Rust tests, Python,
  pinned-fx compatibility, dependency policy/audit, Linux/FreeBSD/WASI plus
  active Node, documentation integrity, diff/unsafe checks, and release-binary
  smoke. Formal cycle 2 rejected exact candidate
  `6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
  `6dc095e796b70fa5964e2d9a24163d75667e1c7a`. Correctness/API reported
  0/0/0/2, lifecycle/robustness was green at 0/0/0/0, and performance/
  concurrency reported 0/0/2/1; the deduplicated union is 0 blocker, 0 high, 2
  medium, and 2 low. Exact isolated production remediation component
  `6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
  `490f628caa20449c3db96069b34356b0117b7ae4`, implements the raw DNS and resolver-
  snapshot corrections. Exact composed cycle-2 remediation precursor
  `1a78f6437eb17f646bdd11337464c949beea49f0`, tree
  `b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. This gate
  record makes no formal-review outcome, workflow, integration, or delivery
  claim; formal candidates are identified only by exact-SHA review results.
  Formal cycle 3 rejected exact candidate
  `16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
  `0440e1eda3cad5ba1a4138bbd0808622de285420`. Correctness/API and lifecycle/
  robustness each reported 0 blocker, 0 high, 1 medium, and 1 low finding;
  performance/concurrency reported zero findings. The deduplicated union is
  0 blocker, 0 high, 2 medium, and 1 low: blocking per-query entropy within
  the total deadline, missing cancellation/deadline authority at native
  pre-effect phase boundaries, and one duplicated cancellation-waiter finding
  repeated across two tracks. The corrected boundary snapshots query-ID
  randomness at construction and derives IDs through an atomic sequence,
  retains fixed hostname-unavailable state with literal-IP bypass, reuses one
  bounded waiter for permit/DNS/HTTP/body waits, and checks the same absolute
  deadline plus cancellation state before A, AAAA, TCP, HTTP, and body effects.
  The final synchronous boundary checks both directly without a second waiter.
  Exact isolated production
  remediation component `9abef298352ea3d9517543c384d9703b949cda75`, tree
  `b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only native
  `web_fetch.rs` and implements that boundary with a 32-byte key, `AtomicU32`
  counter, bounded SHA-256 derivation, carried pre/post-effect deadline checks,
  and one cancellation owner. Exact isolated independent-evidence commit
  `3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
  `f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
  only `web_fetch_http.rs`; exact 13/13 checks prove exactly one cancellation
  wake, a cancelled result, and pending owned-work drop/release across bounded
  and raw seams without sleep or network.
  This remediation record makes no replacement-gate, formal-review, workflow,
  integration, or delivery
  claim; formal candidates are identified only by exact-SHA review results.
  Exact composed cycle-3 remediation precursor
  `78e6f4dcb4d49fd8ccf112e64350b745f622ca7f`, tree
  `1fc16e8f7792c3001ba5f4b4a0c112778d2cf30c`, passes the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. The four
  required commands, complete all-target/all-feature run, focused 26-private/
  14-direct/13-production-HTTP/5-engine suites, Python 130 with eight expected
  macOS skips in 40.358 seconds, pinned-fx check, dependency, Linux/FreeBSD/
  WASI and active Node 1/1, documentation, diff/no-unsafe, fresh release-hash,
  and five CLI-smoke gates are green. The detailed ledger records 53 native
  integration targets, two established FreeBSD warnings, 61 artifacts and six
  established warnings per WASI variant, Markdown 74/99/530/378/0, the 21-file
  code/evidence diff at +6,490/-21, and the cycle-3 source/test delta at
  +523/-102. CLI, workflow, benchmark, and generated-compatibility bytes are
  unchanged. The 319,152-byte release SHA is
  `4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
  This gate makes no formal-review, workflow, integration, delivery,
  performance, or fx-equivalence claim. Formal cycle 4 rejected exact candidate
  `af043dc860ab88941df1385543a92c3d9880beed`, tree
  `095bac47e4db4001b9010b4f66b46202c620dfaa`. Correctness/API reported
  0/0/1/2, lifecycle/robustness was green at 0/0/0/0, and performance/
  concurrency reported 0/0/1/0. The deduplicated union is 0 blocker, 0 high,
  2 medium, and 2 low: DNS TCP replay omitted the configured connect timeout;
  custom-host composition wording and current-candidate prose were incomplete
  or stale; and repeated addresses reached client construction without stable
  first-seen deduplication. The replacement requires stable address
  deduplication before client construction and bounds DNS TCP connect by its
  configured timeout subordinate to cancellation and any earlier overall
  deadline. Exact isolated production remediation component
  `9d793035422cd449c9160c7fccd62221382b5ac5`, tree
  `87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, and isolated independent-
  evidence commit `408e33ec07171988a8f78ee6175adac16532e966`, tree
  `6172f1092561fb06316836f1b7f789db038a4a57`, compose through exact precursor
  `d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
  `56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`. The evidence commit adds a
  deterministic same-poll authority regression but makes no native-DNS proof.
  This remediation record makes no replacement-gate, formal-review outcome,
  candidate, workflow, integration, or delivery claim; formal reviewer reports
  identify the exact candidate they reviewed.
  Exact composed cycle-4 remediation precursor
  `892a52267e7ccf478e9ed567875dc95912be5412`, tree
  `da2d72a2c843e9acadeb529d5127b83cc40ec9b7`, passes the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 991-test, focused 29/14/14/5, Python, pinned-fx, dependency,
  portability, Node, documentation, diff/unsafe, locked release, and five-smoke
  checks are green. This gate record makes no formal-review outcome, candidate,
  workflow, integration, delivery, performance, or fx-equivalence claim;
  reviewer reports identify the exact candidate they reviewed.
  Formal cycle 5 rejected exact candidate
  `81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
  `f5ede2e70637f5cd8ab373c9dfc893189dd5775c`. Correctness/API reported
  0/0/0/1, network/HTTP lifecycle reported 0/0/1/0, and performance/
  concurrency reported 0/0/0/1. The repeated timer-accounting low deduplicates
  across correctness and performance, leaving 0 blocker, 0 high, 1 medium, and
  1 low. The exact candidate is rejected. Its DNS TCP-connect helper could
  accept a ready success or error after the configured connect deadline became
  due during that same effect poll. The replacement must preserve cancellation
  and outer-deadline precedence, then reject the expired connect deadline. The
  maintained timer contract owns exactly one reusable outer machine-god
  invocation-deadline sleep, at most two sequential short-lived DNS replay
  connect-timeout sleeps, and bounded Reqwest/Hyper HTTP connection-attempt
  timers; none resets or extends the outer absolute deadline.
  Exact isolated source remediation
  `cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
  `8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
  `web_fetch.rs` and implements the corrected ordering. Exact composed code
  precursor `d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. This
  remediation record makes no replacement-gate, formal-review outcome,
  candidate, workflow, integration, or delivery claim.
  Exact composed cycle-5 remediation precursor
  `8687898ee19b55fa44864af5f27f7fae8ec3d97e`, tree
  `5d8224eb8afcd297ed53e30909c3d037524f00ba`, passed the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 992-test, focused 30/14/14/5, Python, pinned-fx, dependency,
  portability, Node, documentation 74/99/530/378/0, exact diff/unsafe, locked
  release, and five-smoke checks are green. This gate record makes no formal-
  review outcome, candidate, workflow, integration, delivery, performance, or
  fx-equivalence claim; reviewer reports identify the exact candidate they
  reviewed.
  Formal cycle 6 rejected exact candidate
  `5b2b5f6a50c49d42a6fb3abeaa93b31b12757e95`, tree
  `a802d93eeeac9473d2251ae1d31f6a87f1f87a9f`. Correctness/API reported
  0/0/1/0; lifecycle and performance were each green at 0/0/0/0. The union is
  0 blocker, 0 high, 1 medium, and 0 low, so the exact candidate is rejected.
  The non-WASM HTTP surface and unconditional query-ID hashing exceeded the
  Linux/macOS-only direct `sha2` dependency scope, leaving FreeBSD and Windows
  uncovered while accidentally including WALI. Exact source remediation
  `e6eba937423714bc96a7c399b4823c6deda22da7`, tree
  `bc0a0ddddaa43da5027b1041a9353ce1f6b17184`, moves the dependency to the
  non-WASM table, adds parsed-manifest evidence, and updates behavior docs.
  Exact-1.94.1 focused, full Rust, 131-test Python, Linux/FreeBSD/WALI tree, and
  diff checks are green. This remediation record makes no replacement-gate,
  formal-review outcome, candidate, workflow, integration, delivery,
  performance, or fx-equivalence claim.
  Exact cycle-6 remediation precursor
  `31e7f11870a86795f9d98831d8405714b49b989e`, tree
  `68f5f333e116bd020300943cdfdd74588f95494c`, passed the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 992-test, focused 30/14/14/5, 131-test Python, pinned-fx,
  dependency, portability/target-matrix, Node, documentation 74/99/530/378/0,
  exact diff/unsafe, locked release, five-smoke, and no-XDG-root-creation checks
  are green. This gate record makes no formal-review outcome, candidate,
  workflow, integration, delivery, performance, or fx-equivalence claim;
  reviewer reports identify the exact candidate they reviewed.
  Formal cycle 7 rejected exact candidate
  `47c6a2e7b209aec29f30fd7f7413f55ea039202f`, tree
  `cc28286fa9bb8ac25cf05c2606a8cf9682872a52`. Correctness/API reported
  0/0/1/0; lifecycle and performance were each green at 0/0/0/0. The union is
  0 blocker, 0 high, 1 medium, and 0 low, so the exact candidate is rejected.
  The lexical special-use denylist omitted `.arpa`, allowing infrastructure
  and reverse-name descendants to prepare network capability before DNS.
  Reviewers found no lineage defect, and no pre-review seal was required.
  Exact root remediation `0c9d8b3c9c0283c7a297c6a4b53ca716a3ed04ae`, tree
  `66527a146dc87b1aa9f3b7ee99385efb8022a342`, and tree-identical isolated
  remediation `8fd1f89c5d4ac716d83fa16994ec01d8d4d1eda3` implement the allocation-free
  final-label suffix predicate, bounded docs, and no-policy/no-transport
  regressions.
  Exact-1.94.1 focused 31/15/14/6, formatting, full workspace Clippy, native
  all-target/all-feature, and diff checks are green. This remediation record
  makes no replacement-gate, formal-review outcome, candidate, workflow,
  integration, delivery, performance, or fx-equivalence claim.
  Exact cycle-7 remediation precursor
  `b365cf7704bacb378b280f25d702896dcc216e0c`, tree
  `8a1cecf85ee3ad66e98930aca8f58f208f0ffb07`, passed the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 995-test, focused 31/15/14/6 plus manifest 1/1, 131-test Python,
  pinned-fx, dependency, portability/edge-matrix, Node, allocation-free policy,
  official-link, documentation 74/99/535/378/0, exact diff/unsafe, locked
  release, five-smoke, and no-XDG-root-creation checks are green. This gate
  record makes no formal-review outcome, candidate, workflow, integration,
  delivery, performance, or fx-equivalence claim; reviewer reports identify
  the exact candidate they reviewed.
  Formal cycle 8 rejected exact candidate
  `be418af26317da8fa5de77c45c926311475d7ff6`, tree
  `4c0d054ef1a5231d4347524bb99a7bb93eceda67`. Correctness/API and lifecycle
  each reported 0/0/1/0; performance was green at 0/0/0/0. The deduplicated
  union is 0 blocker, 0 high, 2 medium, and 0 low. Valid partial truncated UDP
  replies could fail strict decode before replay, while invalid but parseable
  truncated replies could construct TCP replay before response validation.
  Exact root remediation `9d3a6ece0cb5a997203b9c63174e0212ce411059`, tree
  `fd65684e526b955c4d8c6f922fd6a162e3cfb39a`, and tree-identical isolated
  remediation `38a36725559fe962e6f649e07cc5de154c940257` add capped leading response-
  tuple validation before one lazy replay, keep non-truncated UDP and TCP
  strict, and add deterministic replay-count and boundary evidence.
  Exact-1.94.1 focused 34/15/14/6, formatting, full workspace Clippy, native
  all-target/all-feature, workspace tests/doctests, locked release/help, and
  diff checks are green. This remediation record makes no replacement-gate,
  formal-review outcome, candidate, workflow, integration, delivery,
  performance, or fx-equivalence claim.
  Exact cycle-8 remediation precursor
  `d6cdaccca217ddd8ca30a12c6f8153acd3aea52d`, tree
  `a5b2b9c9f607c7346d81220bba5e131d5ec4851d`, passed the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 998-test, focused 34/15/14/6 plus manifest 1/1, 131-test Python,
  pinned-fx, dependency, portability/edge-matrix, Node, protocol-ordering,
  official-link, documentation 74/99/535/378/0, exact diff/unsafe, locked-
  release, five-smoke, and no-XDG-root-creation checks are green. This gate
  record makes no formal-review outcome, candidate, workflow, integration,
  delivery, performance, or fx-equivalence claim; reviewer reports identify
  the exact candidate they reviewed.
  Formal cycle 9 rejected exact candidate
  `16c7287888277f14e36ce47712f017d3bb7022da`, tree
  `dfe170664851281bee0324b0ed2acb97575cd49d`. Correctness/API reported
  0/0/1/1; lifecycle reported 0/0/0/2; performance was green at 0/0/0/0. The
  deduplicated union is 0 blocker, 0 high, 1 medium, and 1 low. Complete non-
  truncated UDP and TCP decoding accepted undeclared trailing wire bytes, and
  maintained summaries overclaimed the count-implied minimum as universal
  despite the validated partial-truncation exception. Remediation requires
  decoder exhaustion on complete paths, retains bounded partial `TC=1` replay,
  and adds exact-message, trailing-byte, and zero-replay regressions. This
  remediation record makes no replacement-gate, formal-review outcome,
  candidate, workflow, integration, delivery, performance, or fx-equivalence
  claim.
  Exact cycle-9 remediation
  `a184eba40470d210ba64e717649dc425200c0442`, tree
  `094a90e357f42fda5e7de3929124d3db5b9cc937`, passed the complete replacement
  local gate under exact Rust and Cargo 1.94.1 without fallback. Required,
  complete 1,000-test, focused 36/15/14/6 plus manifest 1/1, Python, pinned-fx,
  exact dependency/audit, portability/edge-matrix, WASI/Node, strict-wire,
  official-link, documentation 74/99/535/378/0, exact diff/unsafe, locked-
  release, five-smoke, and no-XDG-root-creation checks are green. This gate
  record makes no formal-review outcome, candidate, workflow, integration,
  delivery, performance, or fx-equivalence claim; reviewer reports identify
  the exact candidate they reviewed.
  Formal cycle 10 rejected exact candidate
  `698f9d924fe363014dc97971ae60781297d32c2c`, tree
  `08a7d2b8af02b7cb06ec6961dc88192b93aa6658`. Correctness/API and lifecycle
  were green at 0/0/0/0; performance reported 0/0/0/1. The deduplicated union
  is 0 blocker, 0 high, 0 medium, and 1 low. Focused compilation narrowed the
  broad finding: Linux/macOS default copy/session hashing requires `sha2`, but
  FreeBSD/Windows default builds unnecessarily carried it. Remediation retains
  the Linux/macOS edge, makes it optional through `web-fetch-http` on other
  non-WASM targets, omits it on WASM, and adds manifest plus target-matrix tree
  regressions. This remediation record makes no replacement-gate, formal-
  review outcome, candidate, workflow, integration, delivery, performance, or
  fx-equivalence claim.
  Exact cycle-10 remediation
  `4d5fa4f1e3a7b2c53b5fad360cf3c85a83a021aa`, tree
  `ce79fce278903196858ed47de5c22b6535beb08b`, passed the complete exact-1.94.1
  replacement gate without fallback: focused manifest/default/metadata/tree
  checks, all required workspace commands, diff check, and locked-release help
  smoke are green. This gate record makes no formal-review outcome, candidate,
  workflow, integration, delivery, performance, or fx-equivalence claim.
  Formal cycle 11 is **GREEN** on exact candidate
  `298be5fdf0713c76155c8fe834c667d20f0cf1f9`, tree
  `8f636bd92922d9e4d1610443253e82a3c0c36268`. Three fresh correctness/API,
  lifecycle, and performance/portability tracks each reported 0/0/0/0. This
  review seal by itself makes no workflow, integration, delivery, performance,
  compatibility, or fx-equivalence claim. Reviewed delivery seal
  `aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
  `633ddd44406e22f373962c6a2ec965eae4b9cbdb`, passed exact feature CI
  `32874471757`, feature benchmark-evidence `32874471812`, main CI
  `32875016066`, and main benchmark-evidence `32875015892`; both benchmark runs
  retain two nonexpired exact-SHA artifacts. `main` was fast-forwarded without
  force from `a56ff350c2aace1dc22cb14c269aee89d399cd8e`. The delivered count is
  twenty-seven. This makes no product-performance or fx-equivalence claim.
- [ ] Complete the M03 top-level CLI ownership from the pinned inventory:
  `help`, `ask`, `status`, `permissions`, `models`, `doctor`, `session`,
  `sessions`, `resume`, `replay`, and `workspace`. M03 also owns the pinned
  slash-command categories `general`, `session`, `model`, `security`, and
  `workspace`. Observable compatibility is scenario-based; command names may
  remain intentional differences when documented. Strict `doctor [--json]` is
  delivered as slice 30 from exact base `f82ce467`. Exact candidate
  `15f8176`, tree `8278a77`, passed the complete exact-1.94.1 gate and three
  fresh reviews at `0/0/0/0` each. Review-exempt seal `345f812`, tree
  `8899849`, passed exact feature CI `32933464234`, feature benchmark-evidence
  `32933464047`, main CI `32933879888`, and main benchmark-evidence
  `32933879930`; `main` was fast-forwarded without force. It adds exactly four
  ordered read-only checks and no authority-bearing repair behavior. The
  delivered count before sessions was thirty, and this combined item stays
  unchecked. Strict `sessions [--json]` is delivered as bounded slice 31
  from exact base `feaf9fa`. Exact candidate `9448738`, tree `e5342f7`, passed
  its complete local gate but cycle 1 rejected it with `0/0/0/3` deduplicated
  findings. The replacement remediation and complete local gate are green;
  exact candidate `a527652`, tree `0249dd0`, passed three fresh exact-SHA
  reviews at `0/0/0/0` each. Seal `b5b9116`, tree `3e61754`, passed feature CI
  `32939742230`, feature benchmark `32939742231`, main CI `32940279028`, and
  main benchmark `32940279005`; `main` was fast-forwarded without force. The
  delivered count is thirty-one. Strict `session <id> [--json]` is the
  delivered bounded slice 32 from exact delivered base `6e687b6`. Exact
  cycle-2 candidate `1d09a0d`, tree `72a63c0`, passed its complete exact-
  1.94.1 local gate plus Python 135/8 skips, pinned-fx, WASI/FreeBSD, diff/no-
  unsafe, 85/146/626/81 docs-integrity, and release-matrix checks. Its
  4,001,712-byte binary had SHA-256
  `e975e8a16f750188de25d8cf0eac02975643edf6730d6b3ad87d442b76ce27bb`.
  Formal cycle 2 rejected it: the three counts were `0/0/1/2`, `0/0/1/2`, and
  `0/0/1/1`, deduplicated to canonical-number and residual-allocation medium
  themes plus duplicate-key and stale-docs low themes. The synchronized
  replacement requires one-pass fixed-4-KiB streaming, canonical
  `serde_json::Number` semantics, fixed-stack known tokens, only two retained
  ID strings, and a fixed-digest 65,536-entry-capped last-value-wins duplicate
  tracker with separate 65,536 final decoded-tree logical-node accounting.
  Exact remediation source `f4dbe3d`, tree `86971ac`, passed the
  complete required exact-1.94.1 local gate without fallback. Focused 56 CLI
  unit, 54 CLI process, and 21 native tests; Python 135/8 skips; byte-stable
  pinned fx; WASI/FreeBSD; docs 85/147/626/81; no-delta; and the 4,001,760-byte
  release SHA/matrix are green. Formal cycle 3 rejected exact candidate
  `9282b404`, tree `6d41f7ee`: track counts `0/0/1/0`, `0/0/1/0`, and
  `0/0/1/1` deduplicate to a medium context-free recursion-budget mismatch and
  low self-counted allocation evidence. Exact remediation `af055ff3`, tree
  `14eafad`, implements `serde_json` 1.0.151-equivalent 127-active-container
  accounting with typed parent contexts 3/6/7 and exact nested-array boundaries
  123/124, 120/121, 119/120, and 119/120. Focused evidence is 58 CLI process
  tests with ten equivalence cases and 22 native inspection tests. Real dev-
  only allocator instrumentation is child-process-isolated; five shapes all
  report exact allocation counts 14/2/8 and byte counts
  8,913,715/14/8,913,347. The complete exact-remediation gate is green without
  fallback: required Rust, Python 135/8, pinned fx, WASI/FreeBSD, docs
  85/147/626/81, dependency policy/license/audit, unchanged 364-line production
  graph, diff/inventory/no-added-unsafe, exact release hash, 18/18 session
  matrix, and both 1/1 near-cap probes passed. Formal cycle 4 rejected exact
  candidate `df72e084`, tree `99bf524`: all three tracks reported `0/0/1/0`,
  deduplicated to `0/0/2/0`. Ordinary serde accepted five positional stored-
  struct sequences plus an externally tagged unit-role map that the streamed
  parser rejected, while the duplicate tracker eagerly reserved approximately
  8.9 MB even for an empty record. The replacement contract makes the five
  structs object-only and role string-only, keeps the canonical writer
  unchanged, grows fixed-fingerprint entries/buckets fallibly with unique keys
  under a 65,536-entry ceiling, and requires small-record high-water plus
  equal-structure long/short-value allocation evidence. Exact remediation
  `1f96c4b`, tree `b320f55`, passed the complete exact-1.94.1 local gate without
  fallback. Focused 24 native and 64 CLI process tests with 16 differentials,
  Python 135/8 skips, pinned fx, targets, docs 85/147/626/81, dependency/diff/
  inventory/no-added-unsafe, release, allocation, and native near-cap evidence
  are green. Its 3,985,216-byte release binary has SHA-256
  `c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c`.
  Formal cycle 5 rejected exact candidate `8f533cd`, tree `8215fb9`.
  Correctness/API, native boundary/effects, and performance/concurrency/
  resources reported `0/0/0/1`, `0/0/0/0`, and `0/0/0/1`, respectively,
  deduplicated to `0/0/0/1`. The sole low is stale cycle/allocation status in
  the maintained summary pages listed in the review ledger; production review
  was otherwise green. Independent correctness evidence completed 312
  generated valid/boundary and 1,200 randomized mutation differentials with
  zero mismatch, and the native and performance allocation/resource tracks
  were green. Cross-document status remediation is composed in exact cycle-6
  candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
  `d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Correctness/API, native effects,
  and performance/resources each reported `0/0/0/1`; the deduplicated
  `0/0/0/1` is solely the self-pending remediation wording. No additional
  production, API, native, or performance finding exists. Formal cycle 7
  rejected exact candidate `399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
  `d056b96ef8361e841c936c5f61c138de913b5fff`: correctness/API and native effects
  each reported `0/0/0/0`, while performance/resources reported `0/0/0/1`,
  deduplicated to `0/0/0/1`. The sole low corrects node/work-cap wording:
  tracker entries and aggregate final decoded-tree logical nodes have separate
  65,536 caps, while the 8,651,165-byte file ceiling bounds total parse work,
  including shadowed values. Production/resources were otherwise green. Formal
  cycle 8 is green on exact reviewed candidate
  `d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
  `6439863a9b7fd1720156c790fedc4798256c2b6d`: all three tracks and the union
  reported `0/0/0/0`. The review-exempt documentation result seal records that
  reviewed SHA/tree only. Exact seal `b6db9a6`, tree `59dd628`, passed feature
  CI/benchmark runs `32965947722`/`32965947723` and main CI/benchmark runs
  `32966531225`/`32966531319`; both benchmark runs retained two unexpired exact-
  SHA artifacts. Slice 32 is delivered and the delivered count is 32. It
  remains limited to six structural fields and makes no compatibility or
  performance claim. With its delivery complete, bounded work returns to the
  remaining native tools rather than further CLI inspection.
  `models [--json]` is
  delivered as bounded slice 29. Exact cycle-2 candidate
  `2ea9d94374c4dd18f43255af785ee31088126c56` passed its replacement gate but
  was rejected. Pre-review gate attempt
  `c01139811685ae73031ed6f6cbd771e4ff636714` was also rejected and remediated.
  Exact cycle-3 candidate `2cecc921e48396e81ab6f434007a7ec8e3e890b5`
  passed the complete replacement gate, but formal cycle-3 review rejected that
  SHA with a deduplicated union of 0 blocker, 0 high, 1 medium, and 3 low
  findings. Exact documentation remediation `f80bd056`, tree `d6d3f35`, and
  custom bounded-DNS remediation `b6cf4cb`, tree `e72f7dd`, are composed. A
  read-only preflight before the complete cycle-4 gate found 0 blocker, 1 high,
  0 medium, and 0 low on `b6cf4cb`: Android's Hickory system loader can panic
  without initialized NDK context and abort the release process. This was not a
  formal cycle-4 review; cycle 4 remained unsubmitted at that checkpoint.
  Exact remediation
  `bd4746197736004f3a02c4c910c569444e4653e7`, tree `b8d8bb1`, makes Android
  fail closed and redacted without DNS/platform-loader access while leaving
  Apple/Windows and generic Unix unchanged. Focused exact resolver 14/14,
  manifest 5/5, formatting, and warnings-denied native Clippy checks are green;
  the Android standard-library target was unavailable locally and foreign
  Linux HTTP compilation retained the existing `aws-lc-sys` C-sysroot blocker
  before product Rust. Exact cycle-4 candidate `57d2ac2`, tree `d30bb656`,
  passed the complete exact-1.94.1 replacement gate: focused 6/17/6/16/14/18/
  23/5, 972 non-doc Rust tests, two doctests, Python 135, pinned-fx and docs
  integrity, dependency/audit, portability, diff, and release-binary evidence
  are green. Three fresh formal cycle-4 reviews rejected it. The raw overlap-
  deduplicated union was 0 blocker, 0 high, 1 medium, and 2 low; after prior
  sealed dispositions, 0 blocker, 0 high, 1 medium, and 1 low remained
  unresolved at verdict collection. Exact topology fix `268d35a` and exact
  signal/output-lifecycle remediation `aa60db1`, tree `278fa365`, are locally
  integrated. Exact cycle-5 candidate `27c75f4`, tree `5e40b24`, passed the
  complete exact-1.94.1 replacement gate with focused 6/17/6/16/14/22/23/5,
  976 non-doc Rust tests, two doctests, Python 135, pinned-fx and docs integrity,
  dependency/audit, target-matrix, protected-input, release-binary, and signal-
  regression evidence green. Three fresh formal cycle-5 correctness/API/
  compatibility, lifecycle/network/portability, and performance/concurrency/
  resources reviews each reported 0 blocker, 0 high, 0 medium, and 0 low
  findings; the deduplicated union is zero. Review seal `2064084`, tree
  `33818a4`, passed feature benchmark run `32923421739`, but feature CI
  `32923421679` failed solely on exact-1.94.1 Quality Linux's test-only
  `needless_continue` diagnostic, so no integration occurred. Exact test-only
  cycle-6 replacement `831d38c8`, tree `a92acc14`, passed the complete
  replacement gate and three fresh reviews at 0 blocker, 0 high, 0 medium, and
  0 low in every track. Delivered seal `bacc5c3`, tree `da3183a`, passed exact
  feature CI `32925681006`, feature benchmark-evidence `32925681009`, main CI
  `32926242609`, and main benchmark-evidence `32926242564`; `main` was fast-
  forwarded without force from `1de3b7e`, and each benchmark run retains two
  unexpired exact-SHA artifacts for 90 days. This combined item remains
  unchecked because the rest of the M03 CLI inventory is pending.
- [ ] Retain deterministic end-to-end evidence for the composed host with fake
  provider/prompt/network boundaries, exercise user-visible behavior through a
  freshly built release binary, resolve three fresh adversarial reviews, pass
  every required local and remote exact-SHA gate, and update the compatibility
  inventory/status without making a performance claim.

Ownership beyond that boundary is also fixed:

| Owner | Explicitly assigned work |
| --- | --- |
| M04 | Permission modes and identity-safe grant policy beyond `ask`; session schema migration and explicit legacy import; encryption, record authentication, key management, and secure erasure; persistence/lifecycle concurrency hardening; hardened non-Unix workspace and store construction. |
| M05 | Skills, MCP, ACP, and subagent infrastructure; top-level `acp`, `background`, and `teams`; extension/agent slash commands; built-in `memory`, `semantic_search`, `skill`, `install_skill`, `subagent`, `mcp_search_tools`, `mcp_select_tool`, and `mcp_features`. |
| M06 | SDK surfaces and remaining advanced compatibility, including top-level `pr`, `issue`, `login`, `logout`, `setup`, `credits`, `usage`, and `upgrade`, plus pinned account, media, product, and appearance slash-command categories. |
| M07 | Claim-eligible performance comparison, threshold enforcement, optimization, packaging evidence, and final hardening. Earlier milestones retain regression/size evidence needed by CI but make no product performance claim. |

Existing CLI bytes, benchmark evidence, workflows, and Zig inputs are unchanged
by the tenth through fifteenth slices and the delivered sixteenth through
twentieth slices; Zig remains only the pinned
upstream benchmark build input, not a machine-god product language or runtime
dependency. The provider is explicitly scoped to a pinned wire shape and makes
no current-protocol or full fx-equivalence claim. Help and status remain
claim-ineligible and unmeasured in bootstrap evidence.

## Release gates

- Formatting, Clippy with warnings denied, workspace tests on all four native
  target runners, doc tests, repo-wide Python unit tests, dependency policy, and
  vulnerability audit pass.
- Deterministic end-to-end tests pass on Linux and macOS, x86_64 and aarch64.
- Three equivalent local workloads beat pinned fx by at least 20%, no other
  equivalent workload regresses more than 5%, Linux local command startup is at
  most 2 ms, and the stripped Linux x86_64 binary is at most 7.8 MiB.
- Safety, permission, correctness, and resource-bound invariants cannot be
  weakened to meet performance targets.

## Authorization and stop conditions

The coordinator is authorized to commit and push branches and `main` to
`distributedstatemachine/machine-god`. It is not authorized to publish packages
or GitHub releases. Continue fixing ordinary implementation, review, benchmark,
and CI failures until green. Stop only for missing external authority, unavailable
required credentials/runners, irreproducible upstream behavior, or a conflict
between a performance goal and a security invariant.
