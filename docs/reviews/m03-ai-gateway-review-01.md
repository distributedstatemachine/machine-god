# Milestone 03 injected-transport AI Gateway review 01

Status: **ADVERSARIAL GREEN — exact remote runs pending**

## Candidate

- Base: `0905a67b2c55d3839b667506cbd4017ab425778c`
- Atomic feature: `93692cd608f5bf96a21b797d755482ee8da234eb`
- Ownership, bounds, and compatibility remediation:
  `33f3043465392cb97eb2d77805b6eed5585fa998`
- Stream lifecycle and reconciliation remediation:
  `9811d958495fa5b1988532c386813916d0462e36`
- Adversarially green behavior:
  `19573ee6f5780fff7b2852cfc2247e453944ebf7`
- Branch: `agent/m03-ai-gateway-provider`
- Toolchain: Rust and Cargo 1.94.1 exactly
- Pinned reference: fx revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`

The candidate adds the first concrete `ModelProvider`: a bounded,
executor-neutral Vercel AI Gateway protocol `0.0.1` / language-model
specification `4` codec in `machine-god-native`. An injected transport owns all
HTTP, endpoint, TLS, authentication, status, timeout, redirect, and retry
effects. The current CLI does not construct the provider or acquire network
authority.

## Reviewed behavior

- Construction fixes a 1–128-byte visible-ASCII default model, injected
  transport, and explicit independent limits. A request may supply an override
  under the same model rule. Unsupported temperature and metadata are omitted;
  metadata JSON is still safely bounded and reclaimed.
- Request projection accepts the documented provider-neutral text and local
  tool transcript, emits only pinned `prompt`, `tools`, `toolChoice`, and
  optional `maxOutputTokens` fields, and supplies the exact documented headers.
  Cheap count and shape checks precede aggregate depth/node traversal.
- The streaming decoder accepts arbitrary chunk fragmentation, LF/CRLF pinned
  `data: ` records, strict JSON without duplicate keys, text/reasoning deltas,
  local tool calls, usage, and one validated finish. Empty chunks fail and one
  non-event source chunk consumes at most one outer poll.
- Streamed tool arguments are bounded and parsed once. Changed provisional IDs
  use a bounded canonical index keyed by tool name and structural input,
  including reordered object keys and signed floating zero. Ambiguity fails.
  An explicit exact-ID final input is authoritative over valid, malformed, or
  unfinished provisional input; finalized tombstones safely absorb bounded
  late delta/end records.
- Request JSON ownership is guarded synchronously before the returned future
  exists. Unpolled, cancelled, count-rejected, shape-rejected, and depth/node
  rejected requests iteratively drain every current owned JSON surface with
  O(depth) auxiliary state.
- Cancellation wins documented same-poll startup and source terminal races.
  The response stream retains its cancellation waiter only across `Pending`,
  so ready events, errors, stop, and end do not retain or spuriously wake an
  inactive poller.
- Codec errors and debug output are fixed and redacted. Trusted injected
  transport errors pass through unchanged under the documented requirement
  that the transport redact them first. No detached task, thread, timer, retry,
  socket, credential lookup, URL, clock, or async runtime is introduced.

## Parallel implementation

Production code, black-box/real-engine tests, and documentation were initially
developed in parallel by agents with non-overlapping ownership and combined in
one atomic feature commit. Every remediation round used fresh read-only
protocol, reliability, and performance/documentation reviewers against an exact
commit. No reviewer edited the candidate it assessed.

## Adversarial rounds

### Round 1 — `93692cd608f5bf96a21b797d755482ee8da234eb`

Accepted findings:

- **HIGH:** a deeply nested request `serde_json::Value` could recursively drop
  when an unpolled future or rejected request was destroyed. A synchronous
  request guard now drains all owned JSON iteratively and disarms only after
  every tree satisfies the safe depth/node limits. Four subprocess modes cover
  unpolled, depth-rejected, message-count-rejected, and content-count-rejected
  teardown.
- **HIGH:** pinned changed provisional/final tool-call IDs were rejected. The
  decoder now reconciles a unique ended same-name, structurally equal input and
  rejects ambiguity.
- **HIGH:** `poll_next` could loop forever over an always-ready empty/no-op
  source. Empty chunks now fail, while a non-event chunk self-wakes and yields
  after one source poll.
- **MEDIUM:** temperature and metadata were rejected despite core requiring
  unsupported optional inference controls to be ignored. They are now omitted
  after applicable structural validation.
- **MEDIUM:** cancellation could lose same-poll races to transport errors or
  EOF. Every ready startup/source result is followed by a cancellation check
  before interpretation.
- **MEDIUM:** broad JSON validation used O(nodes) scratch and had no node cap;
  final argument size was measured only after a full allocation. Validation now
  uses iterator frames and an independent 262,144-node budget, strict response
  parsing counts before retention, and response arguments use bounded writers.
- **MEDIUM:** each historical tool result could independently retain the full
  request allowance before final body rejection. All serialized results now
  share one cumulative projection budget.
- **LOW:** transport-call documentation incorrectly included unpolled, invalid,
  and pre-cancelled requests. The contract now says at most once, and exactly
  once only after a valid future is polled through startup.

### Round 2 — `33f3043465392cb97eb2d77805b6eed5585fa998`

Accepted findings:

- **MEDIUM:** ready and terminal stream outcomes retained the cancellation
  future's cloned waker. The waiter is now optional, retained only across
  `Pending`, with counting-waker regressions for nonterminal, stop, and error
  outcomes.
- **MEDIUM:** same-ID explicit final arguments incorrectly had to equal ended
  provisional arguments. Explicit final input is now authoritative while the
  ID/name relationship remains validated.
- **MEDIUM:** JSON traversal preceded cheap message/tool limits and lacked
  cancellation checks around non-JSON outer loops. A borrowed envelope gate now
  rejects count/model/content-shape failures before traversal, whose outer loops
  also check cancellation.
- **MEDIUM:** changed-ID reconciliation reparsed and scanned every same-name
  candidate for every final call. Ended inputs are parsed once and stored in a
  bounded canonical nested index with logarithmic lookup and exact cleanup.
- **LOW:** the pinned regression used distinct names rather than the adversarial
  same-name, multi-key, reordered-object shape. The exact structural shape is
  now covered.
- **LOW:** architecture/security still said the transport was always called
  once, and the normative guide understated the model restriction. All docs now
  state at-most-once invocation and the exact model byte rule; construction and
  override boundary tests cover it.

### Round 3 — `9811d958495fa5b1988532c386813916d0462e36`

The reliability and performance/documentation reviewers were green. The
protocol reviewer found and the implementation accepted:

- **HIGH:** malformed-ended provisional input failed before a later exact-ID
  authoritative final, and final-before-end made later delta/end records
  unmatched. Invalid ended input is now a fixed marker; an exact final may
  replace it or unfinished input, and a bounded tombstone absorbs later
  delta/end records. Invalid fallback and unresolved state still fail closed.
- **MEDIUM:** serialized canonical bytes distinguished `-0.0` from `0.0` even
  though the pinned structural comparison treats them as equal. Canonical keys
  now normalize signed floating zero, with a changed-ID regression.

### Round 4 — `19573ee6f5780fff7b2852cfc2247e453944ebf7`

All three fresh seal reviewers reported **GREEN**. They confirmed authoritative
exact-ID correction, malformed/final-before-end handling, bounded tombstones,
invalid fallback rejection, reordered and signed-zero structural matching,
index cleanup/ambiguity, request teardown, cancellation and waker lifecycle,
poll fairness, independent resource limits, redaction, Rust 1.94.1 portability,
documentation accuracy, and scope containment.

## Exact local checks

The following passed on the adversarially green behavior SHA with exact
Rust/Cargo 1.94.1:

- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace tests: 299 top-level tests plus 19 deep-JSON child-process probes;
- workspace documentation tests: 2;
- focused AI Gateway evidence: 35 direct codec tests and 2 real-engine tests;
- warnings-denied `machine-god-native` library Clippy for
  `x86_64-unknown-freebsd`;
- `wasm32-wasip1` native-library compilation, with only the pre-existing
  unrelated `read_file::check_cancellation` dead-code warning;
- repo-wide Python discovery: 129 run, comprising 121 passed and 8 expected
  platform skips;
- pinned upstream compatibility inventory check against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact release build and bare/help/status JSON CLI smoke checks;
- `cargo-deny` 0.20.2: advisories, bans, licenses, and sources accepted;
- `cargo-audit` 0.22.2 with `--no-fetch`: 1,225 cached advisories checked across
  39 lockfile dependencies with no finding;
- 61 relative documentation links;
- unchanged workflow, benchmark, compatibility-inventory, and CLI code from the
  base; and
- `git diff --check` and a clean worktree.

The stripped local release CLI remains 319,152 bytes. This is a local regression
observation only, not retained benchmark evidence or a product-performance
claim.

## Remaining gates and scope

The documentation seal commit, feature branch, and eventual fast-forwarded
`main` SHA must pass their exact remote CI and benchmark-evidence workflows.
The benchmark workflow continues to use Zig only to build the pinned upstream
fx comparison target; machine-god remains a Rust product.

Milestone 03 remains in progress. This slice does not add a native HTTP
transport, credentials, CLI/provider wiring, a production permission prompt,
durable native sessions, broader configuration, the remaining native tools,
non-Linux/macOS hardened filesystem execution, a compatibility claim, or a
measured product-performance claim. No package or GitHub release is authorized.
