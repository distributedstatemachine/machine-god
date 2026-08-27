# Milestone 03 native `ask_user_question` review ledger

Status: **CYCLE 3 LOCAL GATE GREEN — FORMAL REVIEW PENDING**. The cycle-3 tree
is ready for immutable same-SHA formal review. No cycle-3 formal-review or
remote-workflow result is claimed; integration and delivery remain pending.

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
- Cycle-2 independent evidence component:
  `c77b336a378b349f51eaddc60cb342f805fd7e21`, integrated as
  `0dd1128b914b00f15a17be3cbf2b6f7edccf605b`
- Cycle-2 production component:
  `9d2e0f234fd96beb2b2ce5b7dd5a6c123905fbf6`, integrated as
  `47e9505f463b5ca9f4f418198022a4805757621b`
- Cycle-1 finding documentation and exact composed behavior head:
  `c8718c60ead54b4e66916cecb1d382c1e8f82934`, tree
  `c27463b76607ae048363327e163c2077e296b898`
- Formal cycle-2 candidate, **REJECTED**:
  `910d7bc84cfd7800fb4daf9ab8537bf269027896`, tree
  `503a91f334156dbcf2470560b9bb456c3491fd3d`
- Cycle-3 production component:
  `cf531d1692a2946442a37e2049507369c4e12b5c`, integrated as
  `b7b4358525ce1f8864e501a8176b8c3fbdf3790e`
- Cycle-3 independent evidence component:
  `3e3c0c7ea06131adaeca027053c677a670f1a09b`, integrated as `f3f6f9d`
- Cycle-3 documentation component:
  `bfdf05b6db1a343c8b4ab15cad98476986a77552`, integrated at exact behavior
  head `8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
  `7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`

The earlier behavior head passed its recorded local gate, but formal cycle 1
found product and evidence defects in the later immutable candidate. That
candidate remains rejected. Cycle-2 source, evidence, and cycle-1 finding docs
now compose at `c8718c6`/`c27463b`, and the complete local gate for that exact
head was green. Formal cycle 2 nevertheless rejected its later immutable
candidate. Cycle-3 source, independent evidence, and corrected documentation
now compose at `8bdc33d`/`7a342fc`; its complete replacement gate is green.
Three fresh exact-SHA reviews, exact feature workflows, fast-forward
integration, and exact `main` workflows remain required.

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
  an option. Bounded free-form answers support an `Other` path. The per-answer
  and aggregate raw bounds measure complete host-returned strings before trim
  or scanning.
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
| Complete pre-trim host answer / aggregate | 4,096 / 4,096 bytes |
| Aggregate rendered answers | 16,384 bytes |
| Reachable serialized result maximum | 41,102 bytes |
| Serialized result defense-in-depth guard | 49,152 bytes |
| Default/hard active prompts | 1 / 8, fail-fast |

These bounds measure distinct stages. The incoming serialized ceiling is
checked before traversal; raw question/option fields after ASCII trim; complete
host-answer lengths before trim or scan; rendering while terminal encoding;
presentation as the sum of rendered display strings; and normalized arguments
and result as compact full JSON serialization. Legal inputs reach at most
41,102 result bytes; the larger 49,152-byte guard is defense in depth. No stage
truncates.

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

## Cycle-2 implemented remediation and local evidence

Cycle 2 implements every accepted remediation and locally proves it at exact
composed head `c8718c6`, tree `c27463b`:

- the final cancellation check is adjacent to prompter invocation after all
  request cloning and other intervening work;
- direct prepared execution reconstructs and validates a canonical incoming
  preimage under the 32 KiB input and per-field raw limits;
- serialized string-value and object-key sizing consumes a remaining byte
  budget and stops as soon as the applicable ceiling is exceeded;
- successful output intentionally inserts `answer` then `question` independent
  of dependency map features;
- pinned-fx wording, all four reference-host signatures, and the exact
  sixteen-tool alphabetical catalog are corrected; and
- the 26 direct tests cover exact and first-over input, prepared,
  presentation, and answer limits; every documented terminal class;
  default-one and maximum-eight capacity, fail-fast ninth admission and
  independent counters; deep and maximum-depth drop paths; wrong-name and
  canonical-prepared rejection; and inert/unpolled, pending-drop, cancellation,
  and unwind resource ownership. Its claimed 49,152-byte exact/+1 result
  evidence was later rejected because that guard is unreachable under legal
  inputs.

The full focused gate is 55 tests: 26 direct, one engine, 15 configuration,
three root-selection, nine reference-host, and one reference-host lifecycle
test. This is local regression/delivery evidence, not formal review, product
performance, compatibility promotion, or fx equivalence.

User-visible execution through the fresh release binary is not applicable to
this library-only slice because no CLI prompt UI is added. Reference-host
engine evidence with deterministic injected provider, permission handler, and
question prompter is required instead. A later composed release-host slice owns
interactive CLI evidence.

## Formal cycle-2 outcome

All three read-only tracks reviewed exact candidate
`910d7bc84cfd7800fb4daf9ab8537bf269027896`, tree
`503a91f334156dbcf2470560b9bb456c3491fd3d`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 1 | 0 |
| Lifecycle/cancellation/platform | 0 | 0 | 0 | 2 |
| Performance/concurrency/resources | 0 | 0 | 2 | 0 |
| Deduplicated union | 0 | 0 | 2 | 2 |

The result-bound evidence medium was reported by correctness and performance
and counts once in the union. The accepted findings are:

- **Medium — unbounded pre-trim answer scan:** answer validation trims a
  host-returned string before applying the 4,096-byte limit. An arbitrarily
  large whitespace-only response can therefore force unbounded synchronous
  scanning before rejection.
- **Medium — unreachable result-bound evidence:** the focused test labeled as
  exact and first-over 49,152-byte result evidence cannot reach that guard
  under the other legal limits. The exact reachable maximum is 41,102 bytes;
  49,152 remains a defense-in-depth guard, not a reachable rejection boundary.
- **Low — authority wording:** `docs/README.md` said preparation requires no
  external authority. The correct claim is no policy-governed authority; the
  injected prompter separately owns its host interaction authority.
- **Low — review lineage:** `docs/reviews/README.md` still called completed
  cycle-2 production, evidence, and local gates pending, while
  `docs/native-reference-host.md` placed pre-slice-35 green SHAs beside current
  signatures without stating that those historical reviews did not cover the
  later question parameter or sixteen-tool catalog.

Any nonzero finding rejects the candidate, so cycle 2 is not green despite its
earlier local-gate evidence.

## Cycle-3 implemented remediation and checkpoint

Cycle 3 implements every accepted cycle-2 correction:

- check each complete host-returned answer and the aggregate complete answer
  bytes against 4,096 before ASCII trimming or any character scan;
- only then trim, reject empty answers, terminal-safe encode, and construct the
  ordered result;
- replace the unreachable result-guard exact/+1 claim with evidence for the
  exact reachable 41,102-byte maximum while retaining the 49,152-byte guard as
  defense in depth; and
- correct the authority summary, review index, and historical reference-host
  lineage scope.

Isolated production `cf531d1`/`b7b4358`, independent evidence
`3e3c0c7`/`f3f6f9d`, and documentation `bfdf05b` compose at exact behavior head
`8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. The 28-test direct suite proves
the complete pre-trim per-answer and aggregate exact/first-over boundaries and
the exact reachable 41,102-byte result maximum. The larger 49,152-byte guard is
authoritative defense in depth but unreachable under the other legal limits.
The complete local gate is green, so this tree is ready for immutable same-SHA
review by three fresh read-only product reviewers:

1. correctness/API/schema and pinned-fx boundary;
2. lifecycle/cancellation/platform/host composition; and
3. performance/concurrency/resource accounting and redaction.

Each reports blocker/high/medium/low counts and concrete evidence. Deduplicate
overlap without lowering severity. Any confirmed finding rejects the candidate;
remediation receives a new complete local gate and three fresh reviewers. Only
an exact `0/0/0/0` union may proceed to feature workflows and `main`. No
cycle-3 formal-review result exists yet.

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

## Cycle-2 local implementation gate

Exact composed behavior head `c8718c60ead54b4e66916cecb1d382c1e8f82934`,
tree `c27463b76607ae048363327e163c2077e296b898`, passes the complete
local gate under exact Rust and Cargo 1.94.1 without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- focused evidence is green for 26 direct tool tests, one engine test, and the
  affected 15 configuration, three root-selection, nine reference-host, and
  one reference-host lifecycle tests (55 total);
- repo-wide Python discovery passes 136 tests with eight intentional skips,
  and the pinned-fx drift check is green;
- `cargo deny` passes every category with the three established duplicate-
  dependency warnings; `cargo audit` checks 1,226 advisories across 211
  dependencies with zero vulnerabilities;
- native no-default compilation, the all-feature WASI check, and
  warnings-denied no-default-feature FreeBSD Clippy are green; WASI emits only
  the established unrelated `read_file` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; exact diff checks
  find no Cargo-file delta and no added unsafe Rust; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `help`, `doctor`, and `sessions` smoke checks pass.

User-visible prompt execution does not apply because this library-only slice
adds no CLI prompt UI. This gate is
regression/delivery evidence; it makes no formal-review, remote-CI, benchmark,
product-performance, compatibility-promotion, fx-equivalence, integration, or
delivery-completion claim. Replacement reviewer reports must identify the
later exact immutable candidate they inspect.

## Cycle-3 local implementation gate

Exact behavior head `8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`, passes the complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- focused evidence is green for 28 direct tool tests, one engine test, and the
  affected 15 configuration, three root-selection, nine reference-host, and
  one reference-host lifecycle tests (57 total);
- repo-wide Python discovery passes 136 tests with eight intentional skips,
  and the pinned-fx drift check is green;
- `cargo deny` passes every category with the established duplicate-dependency
  warnings; `cargo audit --no-fetch` checks 1,226 advisories across 211
  dependencies with zero vulnerabilities;
- native no-default compilation, the all-feature WASI check, and warnings-
  denied no-default-feature FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; protected and exact
  diff checks find no Cargo-file delta and no added unsafe Rust; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `help`, `doctor`, and `sessions` smoke checks pass
  without creating the missing root.

User-visible prompt execution does not apply because this library-only slice
adds no CLI prompt UI. This gate is regression/delivery evidence; it makes no
formal-review, remote-CI, benchmark, product-performance, compatibility-
promotion, fx-equivalence, integration, or delivery-completion claim. The tree
is ready for immutable same-SHA formal review.

## Deferred and nonclaim record

This slice does not implement approval escalation, a CLI/TUI, timeouts,
background prompts, persistent prompt state, durable terminal work, `vision`,
`read_tool_result`, Milestone 05 surfaces, benchmark workloads, compatibility
promotion, product-performance results, or fx equivalence. No package or
GitHub release is authorized.
