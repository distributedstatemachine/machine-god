# Milestone 03 native `ask_user_question` review ledger

Status: **CYCLE 9 REJECTED — CYCLE 10 REMEDIATION IN PROGRESS**. Formal cycle 9
rejected its exact immutable candidate with one deduplicated correctness/
lifecycle liveness finding. The historical cycle-9 source, deterministic
evidence, and complete local gate remain recorded below. No cycle-10 source,
evidence, gate, review, workflow, integration, or delivery exists yet.

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
- Formal cycle-3 candidate, **REJECTED**:
  `746e510c7d8eb93229996e74f91827f489e5bb31`, tree
  `c49221efbea66c840b333f0de0161aa686aad52f`
- Cycle-4 core component:
  `e569514028cae3b3e6d7b2ba86bf9a738b8d5210`, integrated as `4c8cff3`
- Cycle-4 native component:
  `53c05cdf5e64e9e26266b89d78c8a20a2ac160df`, integrated as `1857a3f`
- Cycle-3 finding-documentation component and exact cycle-4 behavior head:
  `b057958f950b8a2a1412ecbc83b6f452d6571a2f`, integrated at
  `cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
  `fa402acb75c6d364c41db66f6b55595aa1d0e59a`
- Formal cycle-4 candidate, **REJECTED**:
  `42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
  `b761f7b93d535a1580910f43ff509c40aa07415b`
- Cycle-5 independent evidence component:
  `ad47fcb1a6eb751e4953d84933afa1c12dddfbd7`, integrated as `bcce292`
- Cycle-5 source component:
  `80382d8f3f4df53fea867f66c53620f1d6592c6d`, integrated as `e0fd8e0`
- Cycle-4 finding-documentation component and exact cycle-5 behavior head:
  `ba53f5539b68817d2ebe920039ccb5c8303d8b34`, integrated as
  `b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
  `0b025f8e42e18006a72d89becf0e395d35c91a57`
- Direct cycle-5 cross-composition checkpoint:
  `e1947c1495c7cbdc69236b8f7ab1599dda80ca07`, tree `e63f8272`, green for all
  32 direct `ask_user_question` tests
- Formal cycle-5 candidate, **REJECTED**:
  `54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
  `54586d2256c8a3d2289b92bc9bc842eed9ce4d07`
- Cycle-5 finding-documentation component:
  `7dee2694660b3d16340f20de272c6631abdcbcef`, integrated as `e20023c`
- Cycle-6 independent evidence component:
  `b007ada85ce58727ea5d38ab810495dc68e57ef0`, integrated as `4a929c4`
- Cycle-6 source component and exact behavior head:
  `0488d71e2ca1b6b0877d5dc5e1e29ce059f1c5ff`, integrated as
  `707a794230758374fa2dab6d65eaf27449c7c477`, tree
  `1e60299e21f45079f4e8cf27468a28d1ab4fe227`
- Independent cycle-6 cross-composition candidate:
  `236dd90`, tree `94b9fdd3980a413c594538fc9222b09007518bce`, green for 34 direct
  question tests, one engine test, and native warnings-denied Clippy
- Formal cycle-6 candidate, **REJECTED**:
  `85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
  `fd3c5072c9473c7fe8767cc2692238eacb8a0f43`
- Cycle-6 rejection-documentation component:
  `6128f03adddfa566a8fc8f3b326fc16e927b0b05`, integrated as `1d354ff`
- Cycle-7 independent evidence component:
  `acca13c0613e12c2a20e903abbb768e87253c5b6`, integrated as `b75fc54`
- Cycle-7 source component and exact behavior head:
  `3d48ce852db57afe32601ebdd90bc8ef42d4a0fd`, integrated as
  `fbb3f5c5f40d0726b444b1ebc6f25fb1ee1fee36`, tree
  `7cee96e0701d11925360f3d1b6315f5801bbd807`
- Independent cycle-7 cross-composition candidate:
  `c0c9eb0`, tree `6fd79edfac2705e8dfe79bbe43011ab83dc4cd94`, green for formatting,
  35 direct question tests, one engine test, and native warnings-denied Clippy
- Formal cycle-7 candidate, **REJECTED**:
  `617672984fbb897f2efec63de6a05bb32db9a3db`, tree
  `f2cd844449193b46cfa1473ae21edad68664157e`
- Cycle-7 rejection-documentation component:
  `22d570286f76067971504ee2283ee40d49eab8a1`, integrated as `3650dba`
- Cycle-8 independent evidence component:
  `cf4abfd7385904ff4c32c503ff7d8f3823225032`, integrated as `5681bab`
- Cycle-8 source component and exact behavior head:
  `a1b3d231077a67a63f8984cbd3fe4f8cc2370108`, integrated as
  `d8075ffee2d6765df2ce7842300e26bb7127d52b`, tree
  `fa32564476ce6a74cd3ba09c48a4b98af602cb72`
- Independent cycle-8 cross-composition candidate:
  `01d9a06`, tree `c917dce7856e9a1736651fa01696c5ad7e42fbcb`, green for formatting,
  37 direct question tests, one engine test, and native warnings-denied Clippy
- Formal cycle-8 candidate, **REJECTED**:
  `e929b5ea7e3264c2b56066a416bc2a979a03b214`, tree
  `cfadc42814688a29c4d512e5fd91c843423821d4`
- Cycle-8 rejection-documentation component:
  `2faedc764c9cc3caa7813babed0abf0f2f867c90`, integrated as `5296dcc`
- Cycle-9 independent evidence component:
  `cf2e2207d7f298a9aa102476673d9ab33a42024c`, integrated as `ee25455`
- Cycle-9 source component and exact behavior head:
  `527e10dcc53cb609de394ac59d3fe2641ceed627`, integrated as
  `0279b8cb744b8d5cee92d2bfc263abcca60a9987`, tree
  `50b2423637fc9eb8f0cd6792874a2385ff32fd06`
- Independent disposable cycle-9 cross-composition:
  `13eccf9`, tree `56695d8d7c2daaa38355c22a04276b583b93a815`, green for formatting,
  39 direct question tests, one engine test, and native warnings-denied Clippy
- Formal cycle-9 candidate, **REJECTED**:
  `1eeab670a552bc15b5602319b0bb1ce27d2be497`, tree
  `5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`

The earlier behavior head passed its recorded local gate, but formal cycle 1
found product and evidence defects in the later immutable candidate. That
candidate remains rejected. Cycle-2 source, evidence, and cycle-1 finding docs
now compose at `c8718c6`/`c27463b`, and the complete local gate for that exact
head was green. Formal cycle 2 nevertheless rejected its later immutable
candidate. Cycle-3 source, independent evidence, and corrected documentation
now compose at `8bdc33d`/`7a342fc`; its complete replacement gate is green.
Formal cycle 3 nevertheless rejected exact candidate `746e510`/`c49221e`.
Cycle-4 core, native, and finding-documentation work compose at
`cb93bff`/`fa402acb`, whose complete exact-1.94.1 replacement local gate is
green. Formal cycle 4 nevertheless rejected exact candidate
`42ce6f0`/`b761f7b` with a deduplicated `0/0/2/1` union. Cycle-5 source,
independent race evidence, and finding docs compose at `b870731`/`0b025f8`;
its complete exact-1.94.1 local gate is green. Formal cycle 5 nevertheless
rejected exact candidate `54b1aab`/`54586d2` with a deduplicated `0/0/1/0`
union. Cycle-6 finding docs, evidence, and source compose at exact behavior head
`707a794`/`1e60299`; its complete exact-1.94.1 local gate is green. Three fresh
exact-SHA reviews rejected later candidate `85058a8`/`fd3c507` with a
deduplicated `0/0/1/1` union. Cycle-7 rejection docs, evidence, and source now
compose at exact behavior head `fbb3f5c`/`7cee96e`; its complete exact-1.94.1
local gate is green. Three fresh exact-SHA reviews rejected later candidate
`6176729`/`f2cd844` with a deduplicated `0/0/1/0` union. Cycle-8 remediation,
evidence, and source now compose at exact behavior head `d8075ff`/`fa32564`;
its complete exact-1.94.1 local gate is green. Three fresh exact-SHA reviews
rejected later candidate `e929b5e`/`cfadc42` with a deduplicated `0/0/2/0`
union. Cycle-9 rejection docs, evidence, and source now compose at exact
behavior head `0279b8c`/`50b2423`; its complete exact-1.94.1 local gate is
green. Three fresh exact-SHA reviews rejected later candidate
`1eeab67`/`5c86e62` with a deduplicated `0/0/1/0` union. Cycle-10 source,
evidence, replacement gates, formal reviews, workflows, integration, and
delivery do not exist yet.

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
The complete local gate was green. The later documentation checkpoint formed
exact immutable candidate `746e510`/`c49221e`, which three fresh read-only
product reviewers inspected across:

1. correctness/API/schema and pinned-fx boundary;
2. lifecycle/cancellation/platform/host composition; and
3. performance/concurrency/resource accounting and redaction.

Each reports blocker/high/medium/low counts and concrete evidence. Deduplicate
overlap without lowering severity. Any confirmed finding rejects the candidate;
remediation receives a new complete local gate and three fresh reviewers. Only
an exact `0/0/0/0` union may proceed to feature workflows and `main`.

## Formal cycle-3 outcome

All three read-only tracks reviewed exact candidate
`746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 1 | 2 |
| Lifecycle/cancellation/platform | 0 | 0 | 0 | 2 |
| Performance/concurrency/resources | 0 | 0 | 2 | 0 |
| Deduplicated union | 0 | 0 | 3 | 2 |

The two lifecycle lows overlap the two performance mediums, so each appears
once at medium severity in the deduplicated union. The five accepted findings
are:

- **Medium — partial public capability accessor:**
  `PreparedToolCall::capability()` panics for the valid public
  `NoAuthorityRequired` state. Public capability inspection must be total and
  optional for both authorization dispositions.
- **Medium — unbounded malformed-answer destruction:** `Answered(Vec<String>)`
  permits an injected host to return an arbitrarily large vector. Count
  rejection is constant-time, but synchronously destroying that rejected
  vector is unbounded and occurs in the tool future.
- **Medium — permit released before cancellation-Waker teardown:** declaration
  and destruction order can release the active-prompt permit before every
  prompt/cancellation waiter and retained Waker is torn down, allowing a new
  call to enter while arbitrary destructor callbacks from the old call still
  execute.
- **Low — contradictory public authority wording:** broad prose said
  `without_authority` requires no permission or authority. The exact claim is
  no policy-governed authority; an injected prompter separately owns its host
  interaction authority.
- **Low — stale architecture result:** the maintained architecture section
  said no cycle-3 local gate was green even though the exact cycle-3 local gate
  was recorded green before formal review rejected the later candidate.

Any nonzero finding rejects the candidate, so cycle 3 is not green despite its
complete historical local-gate evidence.

## Cycle-4 implemented remediation

Cycle 4 freezes all accepted corrections without widening the first slice:

- make the public capability accessor return an optional capability for both
  valid prepared-call dispositions, with no panic path;
- store answered values behind a privately owned container that admits zero
  through four strings only. Zero through four deliberately remains broad
  enough for execution to detect count mismatches against any legal one-to-four
  question batch, while preventing an unbounded vector destructor;
- keep the active-prompt permit alive until prompt-future and cancellation
  waiter/Waker teardown completes on return, pending drop, and unwind; and
- correct the public authority wording and stale architecture sentence.

Core `e569514`/`4c8cff3`, native `53c05cd`/`1857a3f`, and finding docs
`b057958` implement these corrections at exact behavior head `cb93bff`, tree
`fa402acb`. The focused and complete local gates below are green. Formal cycle
4 later rejected exact candidate `42ce6f0`/`b761f7b`; the local gate remains
historical regression evidence and does not approve that candidate.

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
remote-CI, benchmark, product-performance, compatibility-promotion, fx-
equivalence, integration, or delivery-completion claim. It was the historical
pre-review gate for the later exact candidate `746e510`/`c49221e`, which formal
cycle 3 rejected; it does not establish any cycle-4 gate result.

## Cycle-4 local implementation gate

Exact behavior head `cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`, passes the complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- named focused runs are green for 66 core contract, 22 testkit double, 30
  direct question, one engine, 29 affected configuration, 11 plus two root-
  selection, nine reference-host, and one reference-host lifecycle executions
  (171 total);
- repo-wide Python discovery passes 136 tests with eight intentional skips,
  and the pinned-fx drift check is green;
- `cargo deny` passes every category with three established duplicate-
  dependency warnings; `cargo audit --no-fetch` checks 1,226 advisories across
  211 dependencies with zero vulnerabilities;
- native no-default compilation, the all-feature WASI check, and warnings-
  denied no-default-feature FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; protected-input and
  exact diff checks find no Cargo-file delta and no added Rust `unsafe`; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `help`, `doctor`, and `sessions` smoke checks pass
  without creating the missing root.

User-visible prompt execution does not apply because this library-only slice
adds no CLI prompt UI. This gate is regression/delivery evidence, not a
benchmark or product-performance result. Formal cycle 4 later rejected exact
candidate `42ce6f0`/`b761f7b`; this historical gate makes no green-review,
remote-CI, benchmark-workflow, compatibility-promotion, fx-equivalence,
integration, or delivery-completion claim.

## Formal cycle-4 outcome

All three fresh read-only tracks reviewed exact candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`:

- correctness/API reported 0 blocker, 0 high, 0 medium, and 1 low finding;
- lifecycle/platform reported 0 blocker, 0 high, 1 medium, and 1 low finding;
  and
- performance/resources reported 0 blocker, 0 high, 1 medium, and 0 low
  findings.

Overlap deduplicates to 0 blocker, 0 high, 2 medium, and 1 low. The exact
candidate is rejected. The accepted findings are:

- cancellation triggered synchronously by destruction of the final registered
  cancellation Waker became observable only after the last check, so a direct
  success or error could escape;
- concurrent cancellation could move and execute the registered Waker callback
  after outer drop and permit release, allowing callback tails outside the
  configured active limit; and
- `docs/native-reference-host.md` retained stale cycle-3-pending lineage.

## Cycle-5 implemented remediation and local implementation gate

Cycle 5 freezes the following correction without widening the first slice:

- each registered or cached equivalent cancellation Waker clone retains one
  originating execution activity, and any callback moved out by cancellation
  retains that activity and permit through callback return;
- equivalent registration is cached so teardown and the final cancellation
  observation do not create an untracked Waker family;
- prompt, cancellation waiter, and cached-Waker teardown occurs under the
  execution activity on direct return, pending drop, and unwind;
- after that teardown, every direct success or error path rechecks cancellation
  before returning; and
- deterministic race evidence covers synchronous final-Waker cancellation and
  concurrent moved-callback execution beyond outer drop.

Independent evidence component
`ad47fcb1a6eb751e4953d84933afa1c12dddfbd7`, integrated as `bcce292`, and
source component `80382d8f3f4df53fea867f66c53620f1d6592c6d`, integrated as
`e0fd8e0`, compose with finding-documentation component
`ba53f5539b68817d2ebe920039ccb5c8303d8b34`, integrated as `b870731`, at exact
behavior head `b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Direct cross-composition at
`e1947c1495c7cbdc69236b8f7ab1599dda80ca07`, tree `e63f8272`, passed all 32
direct question tests.

The integrated focused gate is green:

- `cargo +1.94.1 fmt --all -- --check`;
- 32 direct `ask_user_question` tests and one engine test;
- nine all-feature reference-host tests and one reference-host session-
  lifecycle test;
- native all-target/all-feature Clippy with warnings denied; and
- six native-manifest tests.

Exact behavior head `b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`, also passes the complete gate
under Rust and Cargo 1.94.1 exactly without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- repo-wide Python discovery passes 136 tests with eight intentional skips,
  and pinned compatibility regeneration is byte-stable;
- `cargo deny` passes with only the established duplicate-dependency warnings;
  `cargo audit --no-fetch` loads 1,226 advisories, checks 211 dependencies, and
  reports zero vulnerabilities;
- native no-default compilation, the all-feature WASI library check, and
  warnings-denied no-default FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file::check_cancellation` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets;
- the exact diff is clean, protected `.github`, benchmark, and compatibility
  inputs are unchanged, and no Rust `unsafe` is added; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `--help`, `doctor --json`, and `sessions --json` smoke
  checks pass without creating files.

The Cargo change is explicit and development-only: native tests add the
existing audited `machine-god-reentrant-waker-test` path dev-dependency. Its
package already existed in `Cargo.lock`; only the native dependency-list lock
line changed. The production normal/build dependency graph remains unchanged,
and the audit inventory remains 211 dependencies. This gate is regression and
delivery evidence, not a benchmark, product-performance, compatibility-
promotion, fx-equivalence, integration, or delivery-completion claim. Formal
cycle 5 later rejected the exact candidate recorded below, so this historical
gate does not approve it.

## Formal cycle-5 outcome and cycle-6 remediation target

All three fresh read-only tracks reviewed exact candidate
`54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
`54586d2256c8a3d2289b92bc9bc842eed9ce4d07`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 0 | 0 |
| Lifecycle/cancellation/platform | 0 | 0 | 0 | 0 |
| Performance/concurrency/resources | 0 | 0 | 1 | 0 |
| Deduplicated union | 0 | 0 | 1 | 0 |

Any nonzero finding rejects the candidate, so cycle 5 is not green. The one
accepted medium is a product resource/capacity defect, not a cybersecurity
finding: arbitrary retained activity-Waker clones can independently forward
concurrent blocking downstream callbacks while all of those callbacks consume
only one configured prompt slot.

Cycle 6 must replace independent forwarding with one activity-backed,
single-flight coalescing notifier. At most one downstream callback may be in
flight; notifications arriving while it runs must be replayed without loss;
the stale downstream target must be closed; and configured capacity must remain
held until callback and retained-clone ownership is gone. Deterministic
owned-future evidence must exercise many independently retained clones and
concurrent blocking downstream callbacks. The remediation must then pass a new
complete local gate and three fresh exact-SHA review agents.

## Cycle-6 implemented remediation and local implementation gate

Cycle 6 implements the accepted resource correction without widening the first
slice. One activity-backed notifier is shared by the prompt and cancellation
futures. Its mutex-protected state owns the current downstream target, open/
closed lifecycle, callback-in-flight bit, and one coalesced replay bit. A notice
that arrives during a callback sets the replay bit and returns; callback return
serially replays once against the current target when still open. Arbitrary
target cloning and destruction occur outside the state lock. Binding ignores
the notifier's own supplied Waker, and prompt completion or outer-future drop
closes the target and clears replay before prompt destruction can emit a stale
notice. The notifier retains the originating permit through every retained
clone, target destruction, and in-flight callback return.

Finding documentation `7dee2694660b3d16340f20de272c6631abdcbcef`, integrated
as `e20023c`, independent evidence
`b007ada85ce58727ea5d38ab810495dc68e57ef0`, integrated as `4a929c4`, and
source `0488d71e2ca1b6b0877d5dc5e1e29ce059f1c5ff` compose at exact behavior
head `707a794230758374fa2dab6d65eaf27449c7c477`, tree
`1e60299e21f45079f4e8cf27468a28d1ab4fe227`. Independent cross-composition
candidate `236dd90`, tree `94b9fdd3980a413c594538fc9222b09007518bce`, is green
for 34 direct question tests, one engine test, and native warnings-denied
Clippy.

Two deterministic owned-future regressions establish the new boundary:

- `cloned_prompt_wakers_coalesce_blocking_callbacks_and_replay_once` wakes 16
  independently retained clones concurrently, proves only one downstream
  callback is ever in flight, and observes exactly one replay after release;
- `completed_prompt_closes_retained_waker_delivery_until_every_clone_drops`
  completes and drops the prompt while a callback is blocked, proves a stale
  clone cannot deliver another callback, and proves capacity remains exhausted
  until that callback returns and the last retained clone is dropped.

The integrated focused gate is green for exact-1.94.1 formatting; 34 direct
question tests; one question-engine test; nine all-feature reference-host
tests; one reference-host lifecycle test; six native-manifest tests; and native
all-target/all-feature warnings-denied Clippy.

Exact behavior head `707a794230758374fa2dab6d65eaf27449c7c477`, tree
`1e60299e21f45079f4e8cf27468a28d1ab4fe227`, also passes the complete pinned
gate under Rust and Cargo 1.94.1 exactly without fallback:

- all four required formatting, workspace all-target/all-feature warnings-
  denied Clippy, workspace test, and workspace doctest commands are green;
- repo-wide Python passes 136 tests with eight intentional skips, and pinned
  compatibility regeneration is byte-stable;
- `cargo deny` passes with only the established duplicate-dependency warnings;
  `cargo audit --no-fetch` loads 1,226 advisories, checks 211 dependencies, and
  reports zero vulnerabilities;
- native no-default compilation, the all-feature WASI library check, and
  warnings-denied no-default FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file::check_cancellation` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets;
- the exact diff is clean, protected `.github`, benchmark, and compatibility
  inputs are unchanged, and no Rust `unsafe` is added; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `--help`, `doctor --json`, and `sessions --json` smoke
  checks pass without creating files.

The authorized manifest/lock delta remains development-only: native tests use
the existing audited `machine-god-reentrant-waker-test` path dependency, and
only one native dependency-list line changes in `Cargo.lock`. The production
normal/build dependency graph remains unchanged, and audit still covers 211
dependencies. This checkpoint is regression and delivery evidence, not a
benchmark, product-performance, compatibility-promotion, fx-equivalence,
formal-review, integration, or delivery-completion claim. Formal cycle 6 later
rejected the exact candidate recorded below, so this historical gate does not
approve it.

## Formal cycle-6 outcome and cycle-7 remediation target

All three fresh read-only tracks reviewed exact candidate
`85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
`fd3c5072c9473c7fe8767cc2692238eacb8a0f43`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 0 | 1 |
| Lifecycle/cancellation/platform | 0 | 0 | 1 | 0 |
| Performance/concurrency/resources | 0 | 0 | 1 | 0 |
| Deduplicated union | 0 | 0 | 1 | 1 |

The lifecycle and performance tracks reported the same medium, so it counts
once in the union. Any nonzero finding rejects the candidate.

- **Medium — unbounded synchronous replay amplification:** every reentrant
  notice sets `pending_replay`, including a notice emitted by the replayed
  downstream callback itself. The notifier loop can therefore call an
  adversarial callback repeatedly in one original `wake` call as each callback
  rearms its successor. A prompt slot then permits unbounded synchronous work.
  Cycle 7 must make replay observation-aware: retain constant bounded callback
  count for a callback that continually self-notifies, while losslessly
  scheduling one replay only when an outer observation is followed by a new
  notice. Deterministic finite-budget evidence must prove both properties
  before a new complete gate and three fresh reviews.
- **Low — stale operative opening status:** the opening summaries at
  `README.md:9`, `docs/README.md:3`, and `docs/architecture.md:3` still said
  cycle 5 was local-gate green while their later text said cycle 6 was local-
  gate green. Cycle 7 must correct every operative opening to the current
  rejection/remediation state and add a focused status-consistency scan that
  fails on the superseded cycle-5 local-gate opening.

## Cycle-7 implemented remediation and local implementation gate

Cycle 7 makes replay observation-aware without widening the first slice. Entry
to callback delivery clears observed and pending state. Notices before an outer
observation coalesce into the callback already in flight. An outer bind marks
that the callback has been observed; only a later notice earns one serialized
replay. Starting the replay clears observation again. Close and panic clear
both observation and pending replay. Downstream callback concurrency remains at
most one; retained Wakers and in-flight callbacks retain the originating permit;
and no arbitrary downstream Waker clone, drop, or callback runs while the
notifier state lock is held.

Rejection documentation `6128f03adddfa566a8fc8f3b326fc16e927b0b05`, integrated
as `1d354ff`, independent evidence
`acca13c0613e12c2a20e903abbb768e87253c5b6`, integrated as `b75fc54`, and
source `3d48ce852db57afe32601ebdd90bc8ef42d4a0fd` compose at exact behavior
head `fbb3f5c5f40d0726b444b1ebc6f25fb1ee1fee36`, tree
`7cee96e0701d11925360f3d1b6315f5801bbd807`. Independent cross-composition
`c0c9eb0`, tree `6fd79edfac2705e8dfe79bbe43011ab83dc4cd94`, is green for
formatting, 35 direct question tests, one engine test, and native warnings-
denied Clippy.

Deterministic evidence establishes both sides of the observation boundary:

- `reentrant_prompt_wake_before_outer_repoll_has_constant_callback_work`
  rejects the cycle-6 base after it executes 65 callbacks against a finite
  budget of 64, while cycle 7 performs one callback; and
- `cloned_prompt_wakers_replay_once_after_outer_repoll_observes_the_burst`
  proves that an outer re-poll observes the first burst and one later notice is
  delivered as exactly one serialized replay.

The integrated focused gate is green for exact-1.94.1 formatting; 35 direct
question tests; one question-engine test; nine all-feature reference-host
tests; one reference-host lifecycle test; six native-manifest tests; and native
all-target/all-feature warnings-denied Clippy.

Exact behavior head `fbb3f5c5f40d0726b444b1ebc6f25fb1ee1fee36`, tree
`7cee96e0701d11925360f3d1b6315f5801bbd807`, also passes the complete pinned
gate under Rust and Cargo 1.94.1 exactly without fallback:

- all four required formatting, workspace all-target/all-feature warnings-
  denied Clippy, workspace test, and workspace doctest commands are green;
- repo-wide Python passes 136 tests with eight intentional skips, and pinned
  compatibility regeneration is byte-stable;
- `cargo deny` passes with only the established duplicate-dependency warnings;
  `cargo audit --no-fetch` loads 1,226 advisories, checks 211 dependencies, and
  reports zero vulnerabilities;
- native no-default compilation, the all-feature WASI library check, and
  warnings-denied no-default FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file::check_cancellation` dead-code warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; all ten operative
  status regions are current with zero stale openings;
- the exact diff is clean, protected `.github`, benchmark, and compatibility
  inputs are unchanged, and no Rust `unsafe` is added; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `--help`, `doctor --json`, and `sessions --json` smoke
  checks pass without creating files.

The authorized manifest/lock delta remains development-only: native tests use
the existing audited `machine-god-reentrant-waker-test` path dependency, and
only one native dependency-list line changes in `Cargo.lock`. The production
normal/build dependency graph remains unchanged, and audit still covers 211
dependencies. This checkpoint is regression and delivery evidence, not a
benchmark, product-performance, compatibility-promotion, fx-equivalence,
formal-review, integration, or delivery-completion claim. Formal cycle 7 later
rejected the exact candidate recorded below, so this historical gate does not
approve it.

## Formal cycle-7 outcome and cycle-8 remediation target

All three fresh read-only tracks reviewed exact candidate
`617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 0 | 0 |
| Lifecycle/cancellation/platform | 0 | 0 | 1 | 0 |
| Performance/concurrency/resources | 0 | 0 | 0 | 0 |
| Deduplicated union | 0 | 0 | 1 | 0 |

Any nonzero finding rejects the candidate, so cycle 7 is not green. The one
accepted medium is a product lifecycle defect in replay-target destruction.
The notifier selects replay target B before dropping the prior target A. If
A's destructor panics, unwinding occurs before the lane can settle and leaves
`notifying` wedged. If A's destructor instead reentrantly closes the notifier or
installs a replacement, the already selected B can still receive stale
delivery despite that intervening lifecycle transition.

Cycle 8 must drop A before selecting any replay target. Drop unwind must be
caught, the lane flags must be settled on every unwind path, and only after A
has been destroyed successfully may the notifier admit the then-current target
for replay. Deterministic evidence must cover panic recovery and suppression of
replay after A's destructor reentrantly closes the notifier. The replacement
must pass a new complete gate and three fresh exact-SHA reviews.

The native `terminal` notifier contains analogous preexisting destructor/
replay ordering, but it is outside this bounded slice-35 change and review.
Nothing in this cycle-8 target claims to fix, approve, or deliver that separate
terminal path.

## Cycle-8 implemented remediation and local implementation gate

Cycle 8 destroys callback target A under `catch_unwind` outside the notifier
lock while retaining the lane and originating activity. Only after A has been
destroyed successfully does replay arbitration inspect the then-current open/
closed lifecycle, pending notice, and target. A destructor's reentrant close or
replacement therefore wins before replay selection. A callback panic or target-
drop panic clears the lane flags; when both occur, the callback panic wins
deterministically. Callback concurrency remains at most one, capacity remains
owned through the lane/activity, and no foreign callback, clone, drop, or Waker
work executes under the notifier lock.

Cycle-7 rejection documentation
`22d570286f76067971504ee2283ee40d49eab8a1`, integrated as `3650dba`,
independent evidence `cf4abfd7385904ff4c32c503ff7d8f3823225032`, integrated
as `5681bab`, and source `a1b3d231077a67a63f8984cbd3fe4f8cc2370108`
compose at exact behavior head `d8075ffee2d6765df2ce7842300e26bb7127d52b`,
tree `fa32564476ce6a74cd3ba09c48a4b98af602cb72`. Independent cross-
composition `01d9a06`, tree `c917dce7856e9a1736651fa01696c5ad7e42fbcb`,
is green for formatting, 37 direct question tests, one engine test, and native
warnings-denied Clippy.

Deterministic evidence freezes both rejected-base failures and their recovery:

- `replay_target_drop_panic_clears_lane_for_a_fresh_notification` observes
  fresh target B at zero callbacks on the rejected base because `notifying`
  stays wedged, while cycle 8 clears the lane and delivers the fresh notice;
- `replay_target_drop_close_suppresses_selected_replay_and_retains_capacity`
  observes one stale B delivery on the rejected base, while cycle 8 lets A's
  destructor close win, suppresses B, and retains prompt capacity through
  destruction.

The integrated focused gate is green for exact-1.94.1 formatting; 37 direct
question tests; one question-engine test; nine all-feature reference-host
tests; one reference-host lifecycle test; six native-manifest tests; and native
all-target/all-feature warnings-denied Clippy.

Exact behavior head `d8075ffee2d6765df2ce7842300e26bb7127d52b`, tree
`fa32564476ce6a74cd3ba09c48a4b98af602cb72`, also passes the complete pinned
gate under Rust and Cargo 1.94.1 exactly without fallback:

- all four required formatting, workspace all-target/all-feature warnings-
  denied Clippy, workspace test, and workspace doctest commands are green;
- repo-wide Python passes 136 tests with eight intentional skips, and pinned
  compatibility regeneration is byte-stable;
- `cargo deny` passes with only the established duplicate-dependency warnings;
  `cargo audit --no-fetch` loads 1,226 advisories, checks 211 dependencies, and
  reports zero vulnerabilities;
- native no-default compilation, the all-feature WASI library check, and
  warnings-denied no-default FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file::check_cancellation` warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; all ten operative
  status regions are current with zero stale openings;
- the exact diff is clean, protected `.github`, benchmark, and compatibility
  inputs are unchanged, and no Rust `unsafe` is added; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `--help`, `doctor --json`, and `sessions --json` smoke
  checks pass without creating files.

The authorized manifest/lock delta remains development-only: native tests use
the existing audited reentrant-Waker path fixture, only one native dependency-
list line changes in `Cargo.lock`, and the production normal/build dependency
graph remains unchanged. This checkpoint is regression and delivery evidence,
not a benchmark, product-performance, compatibility-promotion, fx-equivalence,
formal-review, integration, or delivery-completion claim. Three fresh exact-SHA
formal reviews later rejected the exact candidate below; this historical gate
does not approve it.

## Formal cycle-8 outcome and cycle-9 remediation target

All three fresh read-only tracks reviewed exact candidate
`e929b5ea7e3264c2b56066a416bc2a979a03b214`, tree
`cfadc42814688a29c4d512e5fd91c843423821d4`:

| Track | Blocker | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| Correctness/API/schema | 0 | 0 | 0 | 0 |
| Lifecycle/cancellation/platform | 0 | 0 | 1 | 0 |
| Performance/concurrency/resources | 0 | 0 | 1 | 0 |
| Deduplicated union | 0 | 0 | 2 | 0 |

The two mediums are distinct, so both remain in the union. Any nonzero finding
rejects the candidate.

- **Medium — secondary panic payload can override the promised primary:** when
  both the callback and target A's destructor panic, cycle 8 selects the
  callback panic as primary. However, destruction of the captured secondary
  target-drop panic payload can itself panic and replace that promised primary
  during unwind. Cycle 9 must preserve the callback panic by safely suppressing
  or forgetting the secondary payload. Deterministic marker evidence must prove
  the primary identity, lane cleanup, capacity retention, and fresh delivery
  after recovery.
- **Medium — re-poll/re-notify replay amplification:** a callback can
  synchronously re-poll the outer future, mark the callback observed, and then
  re-notify. Every replay can repeat that sequence, so one activation executes
  257 callbacks against a finite budget of 256. Cycle 9 must cap each explicit
  notify activation at the initial callback plus at most one replay, retain any
  residual pending notice for a later explicit activation, keep callback
  concurrency at most one, retain capacity, and add deterministic large-budget
  evidence for the bound and later delivery.

Any analogous native `terminal` path is preexisting and outside this bounded
slice-35 remediation. Nothing here claims that separate path is fixed. No
cycle-9 source, evidence, gate, review, workflow, integration, or delivery
result is claimed at this rejection checkpoint.

## Cycle-9 implemented remediation and local implementation gate

Cycle 9 bounds one explicit notify activation to the initial callback plus at
most one serialized replay. A replay-generated post-observation notice remains
pending when the lane is released and is consumed only by a later explicit
activation. Close and panic clear the lane, callback concurrency remains at
most one, target A is destroyed before replay arbitration, and every retained
lane, callback, target, and activity keeps the prompt permit/capacity owned.

When callback execution and target drop both panic, the callback panic remains
primary. Cycle 9 intentionally forgets the opaque secondary target-drop panic
payload so destroying that payload cannot override the promised primary. A
single target-drop panic is still propagated. No foreign callback, clone, drop,
or Waker work runs under the notifier lock.

Cycle-8 rejection documentation
`2faedc764c9cc3caa7813babed0abf0f2f867c90`, integrated as `5296dcc`,
independent evidence `cf2e2207d7f298a9aa102476673d9ab33a42024c`, integrated
as `ee25455`, and source `527e10dcc53cb609de394ac59d3fe2641ceed627`
compose at exact behavior head `0279b8cb744b8d5cee92d2bfc263abcca60a9987`,
tree `50b2423637fc9eb8f0cd6792874a2385ff32fd06`. Independent disposable cross-
composition `13eccf9`, tree `56695d8d7c2daaa38355c22a04276b583b93a815`,
is green for formatting, 39 direct question tests, one engine test, and native
warnings-denied Clippy.

Deterministic evidence establishes both corrections:

- `callback_panic_precedes_panicking_replay_target_payload_drop` marks both
  panic paths and proves the callback marker survives as primary while lane
  cleanup, capacity retention, and a fresh later delivery remain correct; and
- `one_notify_activation_has_one_replay_and_leaves_residual_pending_work`
  rejects base `e929b5e` after 257 callbacks exhaust a budget of 256, while
  cycle 9 performs two callbacks in the first activation and reaches four total
  only after a later explicit activation consumes more residual pending work.

The focused gate is green under exact Rust and Cargo 1.94.1 without
fallback: formatting; 39 direct question tests; one question-engine test; nine
all-feature reference-host tests; one reference-host lifecycle test; six
native-manifest tests; and native all-target/all-feature warnings-denied
Clippy.

Exact behavior head `0279b8cb744b8d5cee92d2bfc263abcca60a9987`, tree
`50b2423637fc9eb8f0cd6792874a2385ff32fd06`, also passes all four required
exact-1.94.1 workspace formatting, all-target/all-feature warnings-denied
Clippy, test, and doctest commands without fallback.

The extended gate is green:

- repo-wide Python passes 136 tests with eight intentional skips, and pinned
  compatibility regeneration is byte-stable;
- `cargo deny` passes with only the established `core-foundation`,
  `cpufeatures`, and `syn` duplicate-dependency warnings;
  `cargo audit --no-fetch` loads 1,226 advisories, checks 211 dependencies, and
  reports zero vulnerabilities;
- native no-default compilation, the all-feature WASI library check, and
  warnings-denied no-default FreeBSD Clippy are green; WASI emits only the
  established unrelated `read_file::check_cancellation` warning;
- documentation integrity reports 91 Markdown files, 318 fence markers, 701
  parsed links, 534 local links, and zero missing targets; all ten operative
  status regions are current with zero stale openings;
- the exact diff is clean, protected `.github`, benchmark, and compatibility
  inputs are unchanged, and no Rust `unsafe` is added; and
- a fresh locked release binary is 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `--help`, `doctor --json`, and `sessions --json` smoke
  checks pass without creating files.

The authorized manifest delta remains development-only: the native test
fixture adds one line in `machine-god-native/Cargo.toml`, and only the native
dependency-list line changes in `Cargo.lock`. The production normal/build
dependency graph remains unchanged. This checkpoint is regression and release-
smoke evidence, not a benchmark, product-performance, compatibility-promotion, fx-
equivalence, formal-review, integration, or delivery-completion claim. Three
fresh exact-SHA formal reviews and both feature and `main` workflow gates remain
pending.

## Formal cycle-9 outcome

Three fresh review tracks examined exact immutable candidate
`1eeab670a552bc15b5602319b0bb1ce27d2be497`, tree
`5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`:

- correctness/API: `0 blocker / 0 high / 1 medium / 0 low`;
- lifecycle/platform: `0 blocker / 0 high / 1 medium / 0 low`;
- performance/resources: `0 blocker / 0 high / 0 medium / 0 low`; and
- deduplicated union: `0 blocker / 0 high / 1 medium / 0 low`.

The correctness and lifecycle reports describe the same product liveness
defect. Once one explicit activation has spent its initial-callback-plus-one-
replay budget, a legal wake emitted after the replay poll is recorded only in
`pending_after_observation`. Notify then releases the lane without scheduling a
downstream callback. Only an unrelated later explicit notify consumes that
pending work. The committed
`one_notify_activation_has_one_replay_and_leaves_residual_pending_work`
regression advances by manually invoking `retained_wakers[2]`, so it proves
retention but not autonomous progress.

A self-waking prompt or cancellation transition whose wake is the last external
activity can therefore remain `Pending` indefinitely. This slice has no
timeout, so no independent deadline eventually repairs the missed schedule.
The candidate is rejected.

Cycle 10 must ensure every wake emitted after its corresponding poll schedules
progress without requiring unrelated activity. It must retain bounded,
nonrecursive delivery, callback single-flight, established panic/target-drop
ordering, and prompt-permit ownership. A deferred or trampoline dispatcher may
be required; alternatively, a public contract redesign needs an explicit and
justified progress rule. No cycle-10 source, independent evidence, local gate,
formal review, workflow, integration, or delivery result is claimed.

## Deferred and nonclaim record

This slice does not implement approval escalation, a CLI/TUI, timeouts,
background prompts, persistent prompt state, durable terminal work, `vision`,
`read_tool_result`, Milestone 05 surfaces, benchmark workloads, compatibility
promotion, product-performance results, or fx equivalence. No package or
GitHub release is authorized.
