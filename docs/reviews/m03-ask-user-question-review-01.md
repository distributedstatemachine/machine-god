# Milestone 03 native `ask_user_question` review ledger

Status: **CYCLE 1 REJECTED — CYCLE 2 REMEDIATION IN PROGRESS**. Replacement
gate, replacement review, remote workflows, integration, and delivery are
pending.

## Frozen lineage

- Exact delivered base:
  `5846799b665d62fc8301b33520da5cda33e850b3`
- Base tree: `758d37b140fb57ee14817016abd7bc5b4d80eb71`
- Pinned fx comparison:
  `b1774fbf6c7602b503026f96f6e960e946c692ef`
- Integration branch: `agent/m03-ask-user-question`
- Normative contract: [`../ask-user-question.md`](../ask-user-question.md)
- Core no-authority component:
  `de1ce26b5c99a526aadec3f527746b18d581d832`, tree
  `fac2038cbd71bbe7077e9265833e08bac289c04e`
- Frozen contract component:
  `13cd366c92f7a692f4ae83f6fb8c28033e152de8`, tree
  `938116d47163b8e5799ea49f304988c012009c8f`
- Contract correction:
  `399f9608550e96c385fd49ecceca6bdad6d3edd5`, tree
  `b25cf4cc067820cf1239dbf68f6e2399922aed5b`
- Native production component:
  `b24a673a05529104d0f0b612e0168aecc2c69dfa`, tree
  `47713c8b316d7385e460bffef450893f71c95551`
- Independent evidence and exact current behavior head:
  `a76818e7779f8d306cdb0101903236d5e755488f`, tree
  `f44def500e9f2d598c81f97abf46f62e59ef1ce3`
- Formal cycle-1 candidate, **REJECTED**:
  `6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
  `bea90245a559e8e223cc5bb45e0ddfa15e426ee6`

The earlier behavior head passed its recorded local gate, but formal cycle 1
found product and evidence defects in the later immutable candidate. That
candidate is rejected and must not be described as green. Cycle-2 source,
evidence, and documentation remediation, a complete replacement gate, three
fresh exact-SHA reviews, exact feature workflows, fast-forward integration,
and exact `main` workflows remain required.

## Frozen first-slice decisions

- Ordinary questions only; any `permission_request_id` field is rejected with
  a fixed deferred-feature error.
- Strict root/question/option objects reject unknown fields and wrong types.
- One to four ordered questions each contain two to six ordered options.
- ASCII-edge trim, exact terminal-safe encoding, and ASCII-only
  case-insensitive label deduplication precede prompt invocation.
- Optional descriptions must be strings; trimmed empty descriptions become
  absent.
- Answers must exactly match the question count and order, but need not match
  an option. Bounded free-form answers support an `Other` path.
- Preparation explicitly requires no policy-governed authority. Core must skip
  permission-ID construction, permission events, and the permission handler
  only for this trusted explicit disposition; the injected prompter separately
  owns its host interaction authority.
- The rootless injected `QuestionPrompter` owns interaction. The tool owns its
  future, detaches no work, sets no timeout, and has fail-fast bounded
  concurrency.
- Cancellation, user cancellation, noninteractive use, host failure, invalid
  host output, and resource exhaustion have fixed redacted behavior.
- The portable library seam is unconditional; current complete reference-host
  composition remains Linux/macOS-only under its existing feature gates.

## Exact frozen limits

| Resource | Bound |
| --- | ---: |
| Incoming serialized arguments | 32,768 bytes |
| Questions/options | 4 / 6 per question, 24 total options |
| Raw/rendered question | 1,024 / 4,096 bytes |
| Raw/rendered label | 128 / 512 bytes |
| Raw/rendered description | 512 / 2,048 bytes |
| Aggregate rendered presentation | 32,768 bytes |
| Serialized normalized arguments | 49,152 bytes |
| Raw answer / aggregate raw answers | 4,096 / 4,096 bytes |
| Aggregate rendered answers | 16,384 bytes |
| Serialized result | 49,152 bytes |
| Default/hard active prompts | 1 / 8, fail-fast |

These bounds measure distinct stages. The incoming serialized ceiling is
checked before traversal; raw fields after ASCII trim; rendering while terminal
encoding; presentation as the sum of rendered display strings; normalized
arguments and result as compact full JSON serialization. No stage truncates.

## Parallel ownership

The coordinator assigned non-overlapping isolated worktrees:

- core no-authority orchestration seam and its provider-neutral tests;
- native production, public prompt seam, and reference-host composition;
- independent direct/engine/composition evidence; and
- this normative contract and maintained documentation.

Agents must not edit another owner's files or revert integrated work. Each
iteration must end committed and clean before its worktree is safely removed.
Active or uncommitted worktrees remain in place.

## Formal cycle-1 outcome

All three read-only tracks reviewed exact candidate
`6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 2 | 2 |
| Lifecycle/cancellation/platform | 0 | 1 | 0 | 1 |
| Performance/concurrency/resources | 0 | 0 | 2 | 0 |
| Deduplicated union | 0 | 1 | 3 | 3 |

The evidence-overclaim medium was reported in both correctness and
performance and counts once in the union. The following findings are accepted:

- **High — adjacent cancellation:** execution checks cancellation, clones up
  to 16 KiB of question presentation data, and then invokes the prompter
  without another adjacent check. Cancellation becoming observable during
  that work can still permit UI invocation.
- **Medium — prepared-call preimage:** direct prepared execution admits a
  printable 4,096-byte question that ordinary preparation rejects at the
  1,024-byte raw limit, so direct execution widens the preparation boundary.
- **Medium — remaining-budget scan:** serialized JSON sizing scans a complete
  arbitrarily oversized string value or object key before applying the
  remaining 32/48 KiB budget.
- **Medium — evidence overclaim:** the ledger claimed exact and first-over
  input/prepared/presentation/result boundaries, complete terminal classes,
  default-one/maximum-eight/ninth-admission behavior, independent counters,
  and deep/wrong-name/unpolled resource paths that the focused suite did not
  fully establish.
- **Low — pinned-fx wording:** parity was overstated. The shared property is
  only that an answer need not match an option label; machine-god separately
  trims, bounds, rejects empty answers, and terminal-safe encodes them.
- **Low — output representation:** the candidate inserts `question` before
  `answer` and relies on the current lexical map implementation to serialize
  `answer` first, rather than intentionally expressing answer-then-question
  insertion independent of feature unification.
- **Low — reference-host documentation:** all four maintained constructor
  signatures omitted `question_prompter`, and the composition prose still
  described a fifteen-tool catalog instead of the actual sixteen-tool catalog.

## Cycle-2 remediation and evidence plan

Cycle 2 is in progress and is not green. Planned remediation is:

- add the final adjacent cancellation check immediately before prompter
  invocation, after all request cloning or other intervening work;
- make direct prepared execution prove that normalized arguments have a valid
  incoming preimage under the 32 KiB input and per-field raw limits;
- make serialized string-value and object-key sizing consume a remaining byte
  budget and stop scanning as soon as the applicable ceiling is exceeded;
- construct successful output with intentional `answer` then `question`
  insertion independent of map implementation or dependency features;
- correct the pinned-fx statement and all four reference-host signatures and
  document the exact sixteen-tool alphabetical catalog; and
- add deterministic exact-limit and first-over-limit input, prepared,
  presentation, and result tests; complete terminal-class coverage; explicit
  one/eight/ninth-admission and independent-counter evidence; and the missing
  deep, wrong-name, and unpolled resource-path tests.

Until those tests are integrated and pass on a replacement candidate, the
earlier suite establishes only the cases its assertions directly exercise. It
does not establish the broader exact/+1 or maximum-concurrency claims listed
above.

User-visible execution through the fresh release binary is not applicable to
this library-only slice because no CLI prompt UI is added. Reference-host
engine evidence with deterministic injected provider, permission handler, and
question prompter is required instead. A later composed release-host slice owns
interactive CLI evidence.

## Replacement review plan

After cycle-2 remediation passes a new complete exact-1.94.1 local gate, freeze
a new immutable candidate SHA/tree and spawn three fresh read-only product
reviewers in isolated clean worktrees:

1. correctness/API/schema and pinned-fx boundary;
2. lifecycle/cancellation/platform/host composition; and
3. performance/concurrency/resource accounting and redaction.

Each reports blocker/high/medium/low counts and concrete evidence. Deduplicate
overlap without lowering severity. Any confirmed finding rejects the candidate;
remediation receives a new complete local gate and three fresh reviewers.
Only an exact `0/0/0/0` union may proceed to feature workflows and `main`.

## Required gates

Run focused checks first and then, under Rust and Cargo 1.94.1 exactly:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

The complete gate also retains repo-wide Python discovery, byte-stable pinned-
fx regeneration, dependency policy and audit, supported cross-target checks,
documentation integrity, diff/protected-input/no-added-unsafe checks, and
applicable release-binary smokes. Gate success is regression/delivery evidence,
not a product-performance or fx-equivalence claim.

## Local implementation gate

Historical behavior head `a76818e7779f8d306cdb0101903236d5e755488f`, tree
`f44def500e9f2d598c81f97abf46f62e59ef1ce3`, passes the complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- the recorded focused commands ran 20 direct tool tests, one engine test, and
  the affected 15 configuration, three root-selection, nine reference-host,
  and one reference-host lifecycle tests (49 total), but cycle 1 identified
  the semantic coverage gaps listed above;
- repo-wide Python discovery passes 136 tests with eight intentional skips,
  and the pinned-fx drift check is green;
- `cargo deny` passes every category with the three established duplicate-
  dependency warnings; `cargo audit` checks 1,226 advisories across 211
  dependencies with zero vulnerabilities;
- the all-feature WASI check and warnings-denied no-default-feature FreeBSD
  Clippy check are green;
- documentation integrity reports `91/318/699/532/0`; exact diff checks find
  no Cargo-file delta and no added unsafe Rust; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated-root `help`, `doctor`, and `sessions` smoke checks pass.

This historical gate makes no positive claim about rejected candidate
`6c54ec3`, formal review, CI, benchmark, performance, compatibility,
fx-equivalence, integration, or delivery. Replacement reviewer reports must
identify the later exact immutable candidate they inspect.

## Deferred and nonclaim record

This slice does not implement approval escalation, a CLI/TUI, timeouts,
background prompts, persistent prompt state, durable terminal work, `vision`,
`read_tool_result`, Milestone 05 surfaces, benchmark workloads, compatibility
promotion, product-performance results, or fx equivalence. No package or
GitHub release is authorized.
