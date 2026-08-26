# Milestone 03 session CLI review ledger

Status: formal cycle 7 rejected exact candidate
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`. Correctness/API and native effects
each reported `0/0/0/0`; performance/resources reported `0/0/0/1`, so the
deduplicated union is `0/0/0/1`. The sole low corrects imprecise resource
wording: shadowed duplicate values may parse more nodes than survive in the
final tree. Tracker entries and aggregate final decoded-tree logical-node
accounting have separate 65,536 caps, while the 8,651,165-byte file ceiling
bounds total parse work. Production and resources were otherwise green. The
current cycle-8 candidate contains this wording correction. Only formal cycle-8
review, remote delivery gates, integration, and delivery are pending. This is
not a review-green or delivered claim.
Historical rejected candidates and verdicts remain recorded below.
Bounded slice 32 starts from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Initial
composition was `852fec7`; focused composition-gate remediation advances the
production precursor to exact `c0c16a745943a97330223aafd4a6f6a7dce84ca6`,
tree `61bcf619fc9190a9a70ab3a9c643605c88ab1817`.

## Frozen boundary

The composed source implements strict top-level `session <id> [--json]`, the
engine-free native by-ID inspection facade, independent native/CLI evidence,
and the release-smoke workflow definition. The normative contracts are
[`docs/session-cli.md`](../session-cli.md) and
[`docs/native-session-inspection.md`](../native-session-inspection.md).

Two independent read-only discovery agents examined the remaining Milestone 03
CLI work and the pinned fx boundary. Both recommended the same summary-only
slice and independently rejected `ask`, `resume`, `replay`, `workspace`, and
parser-only slash commands as larger or semantically incomplete next steps.
Discovery agents are not formal reviewers and will not be reused for the
required post-gate review.

Implementation used isolated worktrees with non-overlapping ownership:

| Component | Owned files |
| --- | --- |
| Native production/evidence | Inspection/state capture, exports, and focused tests. |
| CLI production/unit evidence | `crates/machine-god-cli/src/main.rs`. |
| Independent evidence | CLI process tests and release smoke. |
| Documentation/integration | Contracts, maintained summaries, composition, complete gates, and review ledger. |

## Composed precursor lineage

The isolated components and their feature-branch compositions are:

- native production and focused evidence: original
  `5fa9e6075ccaf7c036e6ec794a2e430fe0b3c304`, tree
  `a6f0d247c0e3d131189fc09fcd35977d5df52a67`, integrated as
  `10a53330737ed463e31ae089dc414cef5c39f752`;
- CLI production and unit evidence: original
  `6eb275c4e1b103b8c2b99e956013cf2bf929f3f6`, tree
  `962f2a2d8529188c0861120bce8a59e309f983e5`, integrated as
  `412d63b5926cea346781dfa807402af287462a13`; and
- independent process and workflow evidence: original
  `463f7a1ebc70057d49b002634f0a235624b18634`, tree
  `48bcffa49701c7b5294437e229af8cc8c973d1cf`, integrated as
  `55f37fc62b230240898c3c859e85f2d87f166292`.

Exact initial composition precursor
`852fec7720e5714fff71d39e211deea740eac2b1`, tree
`cf0ad84945e4030fbc2c5fbfb996b2f484ed2952`, adds one production composition
fix: non-exhaustive native error categories fail closed to the CLI's
`Unavailable` category, and the help output aligns to the frozen
`Inspect a saved session` bytes.

Exact focused composition-gate remediation
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`, makes the integrated native/CLI
all-target/all-feature warnings-denied Clippy gate green and separates the
session grammar evidence into its own unit test without changing the frozen
command behavior.

Exact gate precursor `fa099f75277f7ae23a3ac220e66356c45223d1a5`, tree
`64d6a72e66b6df78bc476dadd82ce3e911644b2d`, composes production, independent
evidence, and maintained documentation and passed the complete required local
gate described below.

Focused exact Rust/Cargo 1.94.1 evidence is green:

- 12 native session-inspection tests;
- 56 CLI unit tests;
- 46 independent CLI process tests.

The focused native/CLI all-target/all-feature warnings-denied Clippy gate is
green. The complete gate evidence is recorded below.

The fixed benchmark inventory and generated compatibility records are
unchanged. This slice remains deliberately non-equivalent, unmeasured, and
claim-ineligible. The composition adds no dependency or unsafe Rust.

Exact precursor `fa099f7` is superseded as a submission by exact cycle-1
candidate `5381d4b`. That candidate has undergone formal adversarial product
review and is rejected, not review-green or delivered. Remote workflows,
`main` integration, and delivery remain pending. The delivered count stays
thirty-one.

## Complete local gate evidence

Exact precursor `fa099f75277f7ae23a3ac220e66356c45223d1a5`, tree
`64d6a72e66b6df78bc476dadd82ce3e911644b2d`, passed all four required commands
under exact Rust/Cargo 1.94.1 without fallback:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Documentation integrity covered 85 Markdown files, 146 fenced blocks, 620
parsed links, and 81 unique repository targets with zero errors. Supplemental
gate evidence is also green:

- the complete Python discovery suite passed 135 tests with eight expected
  macOS skips;
- pinned-fx compatibility regeneration passed at exact upstream
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- native WASI no-default-feature and all-feature checks, the CLI WASI check,
  and native FreeBSD no-default-feature check passed. The only diagnostic was
  the documented pre-existing WASI `read_file` `dead_code` warning;
- diff checks and an explicit unsafe-Rust scan passed; and
- a freshly rebuilt exact-tree release binary passed success human and JSON,
  invalid-grammar-before-effects, missing-root JSON `NotFound`/no-create,
  record-immutability, private-lock, and unrelated-root checks.

Compatibility regeneration is evidence integrity only. It does not promote
this deliberately non-equivalent, unmeasured, claim-ineligible command or make
a performance or compatibility claim.

This documentation-only record changed the exact commit and tree after those
precursor results. Exact candidate
`5381d4b4dda2b609f256ec7237e0c4435b40a165`, tree
`4435bdeac6ffc1df5d5c8f68515082cd167dfc61`, then passed the exact same-SHA
gate before formal review. That gate result did not itself approve review
findings or delivery.

## Formal cycle 1 verdict

Three fresh isolated agents reviewed exact candidate `5381d4b`, tree
`4435bde`, across the required tracks. Counts are blocker/high/medium/low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Green | `0/0/0/0` |
| Native boundary/effects and portability | Rejected | `0/0/0/1` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/1/2` |

The findings are:

1. **Medium — full-record materialization.** The six-field summary reads the
   complete raw record JSON, deserializes the complete envelope, and owns a
   complete `SessionRecord` before dropping transcript and metadata content.
   Bounded final retention does not make that load summary-oriented.
2. **Low — engine-limit contract overclaim.** The store proves its own current-
   schema, file-byte, aggregate JSON depth/node, identifier, counter, and
   content-shape constraints. It does not run the engine's configurable/default
   4,096-message, 8 MiB serialized-transcript, or 256 KiB serialized-metadata
   validation. Store-valid historical or differently configured records over
   those engine limits remain inspectable. The native track's sole low and one
   performance low are this same finding.
3. **Low — latency/attempt overclaim.** Retained summary and successful
   transferred bytes/work have finite ceilings, but exclusive sidecar-lock
   acquisition, filesystem latency, and retries after `EINTR` have no wall-
   clock or attempt bound and synchronously block the polling and CLI thread.

The performance track reported these three findings: one medium and two lows.
The native track additionally reported the same engine-limit low. There are
therefore three unique findings, and deduplicating the overlapping native/
performance low yields `0/0/1/2`. Any finding rejects the exact candidate, so
`5381d4b` is rejected.

Cycle-1 documentation corrected its two normative low themes. Production
remediation then produced cycle-2 candidate `1d09a0d`, whose replacement parser
and evidence are assessed below. No remote workflow, `main` integration,
delivery, performance, compatibility-promotion, or fx-equivalence claim is
made.

## Cycle 2 pre-remediation gate evidence

Exact cycle-2 candidate `1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed all four required commands
under exact Rust/Cargo 1.94.1 without fallback:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

The complete Python discovery suite passed 135 tests with eight expected macOS
skips. Pinned-fx compatibility regeneration passed at exact upstream
`b1774fbf6c7602b503026f96f6e960e946c692ef`; native WASI no-default/all-
feature, CLI WASI, native FreeBSD no-default, diff, and explicit unsafe-Rust
checks passed. Documentation integrity covered 85 Markdown files, 146 fenced
blocks, 626 parsed links, and 81 unique repository targets with zero errors.

The freshly rebuilt 4,001,712-byte exact-tree release binary had SHA-256
`e975e8a16f750188de25d8cf0eac02975643edf6730d6b3ad87d442b76ce27bb`. Its
matrix passed human and JSON success, invalid grammar before effects, missing-
root JSON `NotFound`/no-create, record immutability, private-lock permissions,
and unrelated-root isolation. These are pre-remediation gate results only.
They do not resolve the formal findings or make `1d09a0d` review-green.

## Formal cycle 2 verdict

Three fresh isolated agents reviewed exact candidate `1d09a0d`, tree
`72a63c0`. Counts are blocker/high/medium/low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Rejected | `0/0/1/2` |
| Native boundary/effects and portability | Rejected | `0/0/1/2` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/1/1` |

The overlapping reports deduplicate to four themes:

1. **Medium — canonical-number mismatch.** The specialized parser accepts
   arbitrary JSON number spellings that canonical `serde_json::Number` parsing
   rejects, so it can summarize a record that ordinary `FileSessionStore::load`
   reports as corrupt.
2. **Medium — residual payload-proportional allocations.** Known tokens and
   discarded payload strings still allocate by payload length, and metadata-key
   ownership grows with payload bytes. A streaming read buffer alone does not
   establish summary-oriented auxiliary memory.
3. **Low — duplicate-key mismatch.** The parser rejects duplicate top-level
   metadata keys and does not reproduce nested arbitrary-JSON last-value-wins
   behavior and final-tree node accounting used by ordinary deserialization.
4. **Low — stale maintained documentation.** Maintained summaries described the
   prior cycle and did not identify `1d09a0d` or its current gate/review state.

Any finding rejects the whole exact candidate. The deduplicated verdict is
therefore two medium and two low findings, and `1d09a0d` is rejected.

## Synchronized replacement contract

The next replacement must preserve the CLI grammar, output, redaction, root,
lock, and no-record-mutation behavior while replacing the internal parser
contract as follows:

- inspect through a specialized one-pass summary operation rather than
  `FileSessionStore::load` or a full `SessionRecord`;
- use one fixed 4 KiB input buffer and fixed-stack scratch for known field,
  variant, and role tokens;
- accept and reject numbers with canonical `serde_json::Number` semantics;
- retain payload-sized ownership only for the two returned ID strings;
- stream-discard transcript strings, metadata keys/values, tool payloads, and
  arbitrary JSON scalars after validation;
- use fixed-size key digests in a 65,536-entry-capped duplicate tracker so
  metadata and nested arbitrary JSON match ordinary last-value-wins semantics,
  including replacement of a repeated key's prior logical node contribution,
  with separate 65,536 final decoded-tree logical-node accounting; and
- reproduce `serde_json` 1.0.151's parse-time 127-active-container recursion
  budget including typed parent containers, independently of the final-tree
  depth limit.

The 8,651,165-byte file ceiling, depth-64 and final-tree 65,536-node aggregate
JSON ceilings, identifier/counter/content-shape rules, and duplicate tracking
are store-owned persistence limits. They do not prove or enforce the engine's
configurable/default message, serialized-transcript, or serialized-metadata
limits. Likewise, finite parser memory and transferred bytes do not bound
exclusive lock wait, filesystem latency, or `EINTR` retries; those have no
wall-clock or attempt ceiling and synchronously block the polling thread.

The cycle-2 synchronized contract was composed at exact source
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the exact same-SHA gate
recorded below. Formal cycle 3 nevertheless rejected the exact tree because its
recursion accounting and allocation evidence did not satisfy the refined
contract above. It is not delivered.

## Cycle 2 remediation gate record

Exact replacement source `f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, passed all four required commands
under exact Rust/Cargo 1.94.1 without fallback: workspace formatting, workspace
all-target/all-feature warnings-denied Clippy, workspace tests, and workspace
doctests.

Focused exact-SHA evidence is green for 56 CLI unit tests, 54 independent CLI
process tests, and 21 native inspection tests. The complete Python discovery
suite passed 135 tests with eight expected macOS skips. Compatibility
regeneration is byte-stable against pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Native WASI default/all-feature,
CLI WASI, and native FreeBSD checks passed; the only target diagnostic is the
established WASI `read_file` warning. Documentation integrity covered 85
Markdown files, 147 fenced blocks, 626 parsed links, and 81 unique repository
targets with zero errors.

The replacement introduces no Cargo manifest, lockfile, dependency, benchmark,
generated inventory, or unsafe-Rust delta. The fixed benchmark workload and
classification inventory remain unchanged.

The freshly built 4,001,760-byte exact-tree release binary has SHA-256
`483eb60f707cadfe4b0dd10cfb65617e576488546d908f2f6811b0bfc55773cc`. Its
complete release matrix is green, including the established human/JSON,
grammar-before-effects, no-create, record-immutability, private-lock, and root-
isolation cases plus six process differentials, an 8,650,857-byte near-cap
record, a 4,097-message record over the engine default, a 262,145-byte metadata
record over the engine default, and an exclusive held-lock wait of at least
500 ms.

These checks establish the exact cycle-2 replacement local gate only. They do
not override the formal cycle-3 rejection recorded next.

## Formal cycle 3 verdict

Three fresh isolated agents reviewed exact candidate
`9282b4044c5fb5a249598d23d098562c96850c99`, tree
`6d41f7ee6eb017dfc65d6f6623d049ac09c2966f`. Counts are blocker/high/medium/
low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Rejected | `0/0/1/0` |
| Native boundary/effects and portability | Rejected | `0/0/1/0` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/1/1` |

The overlapping reports deduplicate to `0/0/1/1`:

1. **Medium — context-free recursion-budget mismatch.** The streamed parser's
   arbitrary-JSON recursion budget did not include the typed envelope, record,
   message, content, call, or output containers already active at that parse
   site. It could therefore accept a deeply nested value that ordinary
   `serde_json` deserialization rejects before last-value-wins shadowing and
   final-tree depth accounting.
2. **Low — self-counted allocation evidence.** The prior evidence asserted
   bounded parser-owned structures from counters maintained by the parser
   itself. It did not observe allocations through the real allocator and could
   not independently establish payload-shape-independent allocation behavior.

Any finding rejects the entire candidate, so `9282b404` is not review-green.

## Cycle 3 remediation

Exact remediation `af055ff3b22e157b1c42d1579b041c3cc4c05b0e`, tree
`14eafada4b3dddd62a9cb8e6077ad8f0b81753e8`, reproduces `serde_json` 1.0.151's
127 simultaneously active array/object limit. Typed contexts consume three
parent containers before metadata JSON, six before a JSON content value, and
seven before tool-call arguments or tool-result content. Exact ordinary-store,
listing, and inspection equivalence evidence accepts/rejects nested-array
depths at 123/124 for metadata, 120/121 for JSON content, 119/120 for tool-call
arguments, and 119/120 for tool-result content, including a deeply nested value
later shadowed by a duplicate key.

Focused evidence is now 58 CLI process tests, including ten equivalence cases,
and 22 native inspection tests. Real `allocation-counter` 0.8.1 instrumentation
is a dev-only native-crate dependency and runs each measured shape in its own
child process. Empty, near-cap, number-heavy, message-heavy, and key-heavy
records all report exactly `count_total=14`, `count_current=2`, `count_max=8`,
`bytes_total=8913715`, `bytes_current=14`, and `bytes_max=8913347`. The Cargo
manifest and lockfile delta is solely this dev dependency; dependency policy,
license, and vulnerability-audit checks are green. Production dependencies,
benchmark workloads, generated compatibility inventory, and unsafe Rust are
unchanged.

## Cycle 3 remediation gate record

Exact remediation `af055ff3b22e157b1c42d1579b041c3cc4c05b0e`, tree
`14eafada4b3dddd62a9cb8e6077ad8f0b81753e8`, passed all four required commands
under exact Rust/Cargo 1.94.1 without fallback. The complete Python discovery
suite passed 135 tests with eight expected macOS skips. Compatibility
regeneration is byte-stable against pinned fx `b1774fb`. Native WASI default/
all-feature, CLI WASI, and native FreeBSD checks passed with only the
established WASI `read_file` warning. Documentation integrity covered 85
Markdown files, 147 fenced blocks, 626 parsed links, and 81 unique repository
targets with zero errors.

Exact `cargo-deny` 0.20.2 passed every category with the three established
duplicate-version warnings. Exact `cargo-audit` 0.22.2 loaded 1,226 advisories,
scanned 211 dependencies, and found zero vulnerabilities. The sole manifest/
lock delta is crates.io `allocation-counter` 0.8.1 as an MIT/Apache dev-only
dependency. The production normal/build graph remains unchanged at 364 lines.
Diff, generated-inventory, and no-added-unsafe checks are green; benchmark
workloads and generated compatibility bytes are unchanged.

The freshly built 4,001,760-byte exact-tree release binary has SHA-256
`d296174898938f632351bebb38449533c7db03bb3659392bea3743a02ee1619d`. Its
release-session matrix passed 18/18, including ten exact ordinary-store/
listing/session equivalence cases, held-lock behavior, and engine-over-default
records. The direct 8,650,857-byte near-cap case passed 1/1. The native near-
cap/allocation case also passed 1/1 and retained the five-shape exact allocation
tuple above.

These checks establish the complete exact-remediation local gate only. They do
not override the formal cycle-4 rejection recorded next.

## Formal cycle 4 verdict

Three fresh isolated agents reviewed exact candidate
`df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. Counts are blocker/high/medium/
low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Rejected | `0/0/1/0` |
| Native boundary/effects and portability | Rejected | `0/0/1/0` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/1/0` |

The overlapping reports deduplicate to `0/0/2/0`:

1. **Medium — ordinary-versus-streamed wire-form mismatch.** Ordinary
   serde-derived storage deserialization accepted positional sequences for
   `StoredEnvelope`, `StoredRecord`, `StoredMessage`, `StoredToolCall`, and
   `StoredToolOutput`, plus an externally tagged unit-variant map for `Role`.
   The streamed inspection parser required the canonical object forms and role
   string, so it reported `Corrupt` for records accepted by the ordinary store.
2. **Medium — eager duplicate-tracker reservation.** Every inspection reserved
   approximately 8.9 MB for the duplicate tracker up front, including empty
   and small records. The five allocation shapes therefore demonstrated a
   fixed large allocation, not auxiliary memory proportional to the unique keys
   actually encountered.

Any finding rejects the entire candidate, so `df72e084` is not review-green.

## Cycle 4 remediation contract

The durable ordinary-store schema becomes explicitly object-only for
`StoredEnvelope`, `StoredRecord`, `StoredMessage`, `StoredToolCall`, and
`StoredToolOutput`, and string-only for `Role`. The canonical writer is
unchanged. Ordinary loading/listing and streamed inspection must reject each of
the five positional struct sequences and the externally tagged unit-role map
consistently.

The fixed-fingerprint duplicate tracker retains a strict 65,536-entry ceiling,
while aggregate final decoded-tree logical-node accounting is separately capped
at 65,536. Entries and buckets must grow fallibly in proportion to unique keys
actually encountered rather than reserving maximum capacity for every
inspection.
Replacement allocation evidence must bound empty/small-record allocator
high-water use and compare long and short discarded values at equal structural
shape. A near-cap input alone does not establish the small-record bound.

## Cycle 4 remediation and complete local gate

The isolated native remediation component is exact
`65012426ee9e174c15ededac7c0c95f9f496b2cf`; independent CLI differential
evidence is exact `225fee4af82c24ea226db185801a7fe397407593`. Feature-branch
composition records CLI evidence at
`7c191b8a742f820368dd8f25e4f7e6f3026a87e0`, cycle-4 documentation at
`c73467ea74e563c2c89db38603bc6b9d8f11aaf1`, and native production/evidence at
exact remediation `1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`.

Focused evidence is green for 24 native inspection tests and 64 CLI process
tests, including 16 differential cases.
All four required commands passed on exact `1f96c4b` under Rust/Cargo 1.94.1
without fallback:

- `cargo +1.94.1 fmt --all -- --check`;
- `cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo +1.94.1 test --workspace`; and
- `cargo +1.94.1 test --doc --workspace`.

The complete Python suite passed 135 tests with eight expected macOS skips.
Pinned-fx `b1774fb` regeneration is byte-stable. WASI and FreeBSD checks are
green with only the established WASI `read_file` warning. Documentation
integrity is 85 Markdown files, 147 fenced blocks, 626 parsed links, and 81
unique repository targets with zero errors. Exact `cargo-deny` 0.20.2 passed
with only three established duplicate-version warnings. Exact `cargo-audit`
0.22.2 loaded 1,226 advisories, scanned 211 dependencies, and found zero
vulnerabilities. The production normal/build graph is unchanged at 364
normalized lines; `allocation-counter` remains dev-only. Diff, generated-
inventory, and no-added-unsafe checks are green.

The freshly built 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c`. Release-
binary evidence passed 16 equivalence categories across 20 records, 12 grammar
cases, missing-root/no-create, an exclusive held-lock wait of at least 500 ms,
engine-over-default records, and the 8,650,857-byte near-cap record. The direct
native near-cap probe passed 1/1.

Isolated allocator evidence reports total/current/maximum allocations and bytes.
Empty, short-text, and long-text records each report `12/2/7` allocations and
`819/14/645` bytes. Short-JSON and long-JSON records each report `14/2/8` and
`1,427/14/1,059`. The 5,000-key record reports `35/2/9` and
`2,228,435/14/1,606,083`. Equal-structure short/long shapes therefore preserve
allocation counts and high-water bytes independent of discarded payload length,
while empty/small records no longer reserve the former maximum capacity.

These results establish only the exact remediation's complete local gate.
The formal cycle-5 rejection is recorded next. No remote workflow, `main`
integration, delivery, compatibility-promotion, product-performance, or fx-
equivalence claim is made.

## Formal cycle 5 verdict

Three fresh isolated agents reviewed exact candidate
`8f533cdec235660c3e17b70fc5bbd5dd0ab8c1f6`, tree
`8215fb94fa3de08841b26dd9d7c63a2ecb7e8a8d`. Counts are blocker/high/medium/
low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Rejected | `0/0/0/1` |
| Native boundary/effects and portability | Green | `0/0/0/0` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/0/1` |

The overlapping reports deduplicate to `0/0/0/1`:

1. **Low — stale maintained summary pages.** `README.md`, `docs/README.md`,
   `docs/architecture.md`, `docs/cli.md`, `docs/performance.md`,
   `docs/security.md`, `docs/session-store.md`, and `docs/reviews/README.md`
   still described fresh cycle-4 review as pending, described cycle 4 as the
   latest rejection, or presented the superseded approximately 8.9 MB eager-
   reservation evidence as the current allocation result.

This is a cross-document status finding, not a production-behavior finding.
Independent correctness evidence exercised 312 generated valid/boundary
differentials and 1,200 randomized mutation differentials with zero mismatch.
The native boundary/effects and portability track was green. The performance
track's allocation and resource audit was also green apart from the overlapping
stale-documentation finding.

Any finding rejects the entire candidate, so `8f533cd` is not review-green.
Cross-document remediation synchronizes the maintained summary pages while
preserving the cycle-4 contract and complete-gate evidence above. It is
composed in exact candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`,
tree `d2fec0815b60c61368298e7f4f0d7bef0fc2e097`, assessed next.

## Formal cycle 6 verdict

Three fresh isolated agents reviewed exact candidate
`5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Counts are blocker/high/medium/
low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Rejected | `0/0/0/1` |
| Native boundary/effects and portability | Rejected | `0/0/0/1` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/0/1` |

The overlapping reports deduplicate to `0/0/0/1`:

1. **Low — self-pending remediation wording.** Candidate `5332d6a` is the
   committed cross-document status remediation, but its maintained current
   pages still mislabeled that remediation and cycle-6 review as future work.
   Only fresh cycle-7 review, remote workflows, `main` integration, and
   delivery should remain pending.

There is no additional production, API, native, performance, resource, or
compatibility finding. Any finding rejects the entire candidate, so `5332d6a`
is not review-green. The wording remediation produced exact cycle-7 candidate
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`, assessed next.

## Formal cycle 7 verdict

Three fresh isolated agents reviewed exact candidate
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`. Counts are blocker/high/medium/
low:

| Track | Verdict | Counts |
| --- | --- | --- |
| Correctness/API and pinned-fx boundary | Green | `0/0/0/0` |
| Native boundary/effects and portability | Green | `0/0/0/0` |
| Performance/concurrency/resources and evidence | Rejected | `0/0/0/1` |

The reports deduplicate to `0/0/0/1`:

1. **Low — imprecise node/work-cap wording.** The native inspection contract
   said aggregate arbitrary-JSON work was strictly capped at 65,536 nodes, but
   values later shadowed by duplicate keys can require more parse work than the
   final decoded tree retains. The 65,536 limits apply separately to tracker
   entries and aggregate final decoded-tree logical-node accounting. The
   8,651,165-byte file ceiling bounds total parse work, including shadowed
   values.

There is no production-behavior, API, native-effect, compatibility, allocation,
or other resource finding. Any finding rejects the entire candidate, so
`399e75e` is not review-green. The current cycle-8 candidate contains this
wording correction. Only formal cycle-8 review, remote workflows, `main`
integration, and delivery are pending. Do not claim cycle 8 green before those
reviews report `0/0/0/0` on one exact SHA. No prior discovery, implementation,
review, or remediation agent may approve its own work or be reused in a later
review cycle. Product-performance and fx-equivalence claims remain absent.

After each review/remediation iteration, committed and integrated worktrees
must be verified clean and then safely removed; active or uncommitted worktrees
must never be deleted. After this slice is delivered, bounded development
returns to completing the remaining Milestone 03 native tools rather than
expanding the CLI inspection surface.

## Delivery gate

A documentation-only review seal may record returned zero-finding verdicts
without another product review. The sealed feature SHA must pass CI and
benchmark-evidence workflows before `main` is fast-forwarded without force.
The exact integrated `main` SHA must pass both workflows, and each benchmark
run must retain exactly two unexpired exact-SHA artifacts. No package
publication or GitHub release is authorized.
