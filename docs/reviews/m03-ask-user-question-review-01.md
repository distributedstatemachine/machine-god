# Milestone 03 native `ask_user_question` review ledger

Status: **IMPLEMENTED — complete local gate green; exact review candidate,
formal adversarial review, remote workflows, integration, and delivery
pending**.

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

The exact behavior head implements and independently exercises the frozen
slice. It is not yet an immutable formal-review candidate, a review result, or
delivery. Three fresh exact-SHA adversarial tracks, exact feature workflows,
fast-forward integration, and exact `main` workflows remain required.

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

## Implemented evidence coverage

Focused direct, engine, and composition evidence establishes:

- strict root, nested object, required field, type, count, and unknown-field
  failures, including the dedicated `permission_request_id` rejection;
- exact raw, rendered, aggregate-presentation, normalized-argument, answer, and
  serialized-result boundaries plus the first value beyond each reachable
  boundary;
- ASCII-only trim, post-render duplicate-label comparison (including escaped
  control/literal collisions), Unicode non-folding, empty-description
  normalization, and exact terminal-safe C0/DEL/C1/U+061C/bidi encoding;
- no-authority preparation preserving every ordinary engine validation,
  cancellation, event, durability, placeholder, result-size, and recovery path
  while emitting no permission events and invoking no permission handler;
- inert unpolled futures, first-poll prompt invocation exactly once, prompt
  future ownership, pending drop cleanup, no detached work, and permit release;
- default-one and explicit-eight concurrency, fail-fast ninth admission, no
  capacity queue or Waker retention, recovery after completion/drop, and
  independent tool-instance counters;
- answer count/order, bounded non-label free-form success, deterministic JSON
  key/order/escaping, explicit cancellation and noninteractive sentinels;
- cancellation precedence before admission, before prompt, and against every
  ready outcome; and
- error, `Debug`, and host-composition redaction on native, FreeBSD, and WASI
  compile paths, with active portable behavior where the repository can run it.

User-visible execution through the fresh release binary is not applicable to
this library-only slice because no CLI prompt UI is added. Reference-host
engine evidence with deterministic injected provider, permission handler, and
question prompter is required instead. A later composed release-host slice owns
interactive CLI evidence.

## Formal review plan

Production and independent evidence now compose and the complete exact-1.94.1
local gate is green. The next step is to freeze one immutable candidate
SHA/tree and spawn three fresh read-only product reviewers in isolated clean
worktrees:

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

Exact behavior head `a76818e7779f8d306cdb0101903236d5e755488f`, tree
`f44def500e9f2d598c81f97abf46f62e59ef1ce3`, passes the complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- focused evidence is green for 20 direct tool tests, one engine test, and the
  affected 15 configuration, three root-selection, nine reference-host, and
  one reference-host lifecycle tests (49 total);
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

This gate makes no formal-review, CI, benchmark, performance, compatibility,
fx-equivalence, integration, or delivery claim. Reviewer reports must identify
the later exact immutable candidate they inspect.

## Deferred and nonclaim record

This slice does not implement approval escalation, a CLI/TUI, timeouts,
background prompts, persistent prompt state, durable terminal work, `vision`,
`read_tool_result`, Milestone 05 surfaces, benchmark workloads, compatibility
promotion, product-performance results, or fx equivalence. No package or
GitHub release is authorized.
