# Milestone 03 native `ask_user_question` review ledger

Status: **CONTRACT FROZEN — implementation, exact candidate, review, and
delivery pending**.

## Frozen lineage

- Exact delivered base:
  `5846799b665d62fc8301b33520da5cda33e850b3`
- Base tree: `758d37b140fb57ee14817016abd7bc5b4d80eb71`
- Pinned fx comparison:
  `b1774fbf6c7602b503026f96f6e960e946c692ef`
- Integration branch: `agent/m03-ask-user-question`
- Documentation component branch: `agent/m03-ask-docs`
- Normative contract: [`../ask-user-question.md`](../ask-user-question.md)

The contract component is review-exempt only as a planning checkpoint. It is
not implementation evidence, a behavior candidate, a formal review result, or
delivery. Production, independent evidence, composition, the complete local
gate, three fresh exact-SHA adversarial tracks, exact feature workflows,
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

## Required implementation evidence

Focused evidence must establish at least:

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

After production, evidence, and docs compose and the complete exact-1.94.1
replacement gate is green, freeze one immutable candidate SHA/tree. Spawn three
fresh read-only product reviewers in isolated clean worktrees:

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

## Deferred and nonclaim record

This slice does not implement approval escalation, a CLI/TUI, timeouts,
background prompts, persistent prompt state, durable terminal work, `vision`,
`read_tool_result`, Milestone 05 surfaces, benchmark workloads, compatibility
promotion, product-performance results, or fx equivalence. No package or
GitHub release is authorized.
