# Performance

Performance comparisons must build both projects in release modes on identical
hardware, retain raw samples, warm up before at least 30 measured runs, and report
median, p95, confidence intervals, RSS, and binary sections.

Bounded slice 34, native `terminal`, is **DELIVERED**, unmeasured, and
claim-ineligible. Its 32 KiB command, 4,096-byte/256-component cwd, bounded
256 KiB environment snapshot, 64 KiB retained raw output, 1 MiB produced-output
cutoff, 48 KiB serialized output, 120-second default deadline, default active
limit four, and hard active limit sixteen are resource ceilings, not measured
performance results. Each admitted call owns at most one deadline-guardian
thread; Linux system execution additionally owns one worker and two readers.
Once either reader observes more than 1 MiB aggregate production, fixed chunk/
post-stop read-count ceilings bound overshoot while stopping both readers
promptly. Cancellation is first; one linearized cause close makes overflow
authoritative when observed before timeout closes and makes timeout
authoritative against overflow observed later, while publishing only a valid
status/counter pair. One admitted call consumes one active slot shared without
increment by the outer call, owned request/executor, all TerminalTool-wrapped
Waker clones and callback returns, and native worker/deadline threads through
actual return.
The wrapper supplied to public injected executors needs no private counter
authority. Retained requests or Wakers, blocking callbacks, and native threads
that observed no Waker keep the same slot; later calls fail fast as busy, so OS
threads and stacks cannot accumulate outside configured capacity. An already-
exited foreground leader gets TERM and immediate final KILL while its identity
is retained, avoiding a fixed termination-grace floor for normal commands. The
first-poll deadline is
independently enforced around controllable userspace phases, but a blocked
filesystem lookup, `Command::spawn`, kernel wait, uninterruptible syscall,
synchronous executor poll/drop, or Waker callback can exceed it in wall-clock
time. These are bounded ownership and work facts, not a latency, throughput,
sandbox, or performance claim. No benchmark workload changes in this slice.
See [`terminal.md`](terminal.md).

Bounded slice 33, native `web_search`, is **DELIVERED**, unmeasured, and
claim-ineligible. It changes no benchmark workload or recorded comparison. Its
fixed ceilings are regression and denial-of-service boundaries, not performance
results: 4,096 query bytes, 16 domains and 4,096 aggregate domain bytes, ten
sources, 512 title bytes, 2,048 URL bytes, 16 KiB request, 256 KiB response,
64 KiB record, 256 records, 16,384 JSON nodes, a separately parsed 16 KiB/256-
node provider-call input, a 512-byte provider-result ID, 48 KiB serialized
output, a 30-second total deadline beginning before capacity wait, default
concurrency four, and hard concurrency sixteen. Required finish usage and
provider metadata are not retained beyond validation and share the fixed
record, response, and decoded-node ceilings.

One approved execution makes at most one Perplexity worker request with no
retry, fallback, cache, page fetch, progress stream, or detached work. The
private decoder and transport limits apply independently of core engine and
outer `AiGatewayProvider` limits. Production and independent evidence compose
through behavior precursor `3d2984000301e58762e0940504159aeb55b2389e`, whose
exact-1.94.1 local gate is green. Formal cycle 1 rejected exact `89c5ec95`, tree
`8d91a55`, with three performance-track medium findings and a deduplicated
`0/2/5/2` union. Source remediation is composed from exact isolated components
`096b11c4` and `ca0b990a`. Exact composed precursor `e662fa8`, tree `6c0ace9`,
passes the complete local gate. Exact cycle-2 remediation `366cef9`, tree
`40c05cb`, passed its complete gate, but formal cycle 3 rejected exact candidate
`aef6abe`, tree `5abcef3`, with a deduplicated `1/0/2/2`. Exact isolated
components `5d45dca` and `454f8fd` compose its remediation. Exact precursor
`b834205`, tree `f3557a5`, passes the complete replacement gate. Formal cycle 4
rejected exact `cc1d3d1`, tree `ad0c3d3`, with a deduplicated `0/0/1/1`;
exact finish-envelope remediation component `dc79c8d`, tree `e2fed70`, is
composed with host-fixture component `9f6c474`. Exact precursor `2e9c44d`, tree
`3e25daa`, passes the complete replacement gate. Formal cycle 5 is green on
exact `782aa54`, tree `b1ba692`, with a `0/0/0/0` union. Delivery record
`52b5885` passed exact feature and main CI/Benchmark workflows. No
latency, throughput, allocation
improvement, binary-size improvement, compatibility promotion, or
fx-equivalence claim follows. The
normative bounds are in [`web-search.md`](web-search.md) and status is in the
[`slice-33 ledger`](reviews/m03-web-search-review-01.md).

Current bounded slice 32 is **DELIVERED**. Cycle 4 rejected exact
`df72e084`, tree `99bf524`, with correctness/API, native effects, and
performance/resources each at `0/0/1/0`; its deduplicated `0/0/2/0` union is
the ordinary/streamed wire-form mismatch and eager
approximately 8.9 MB tracker allocation. Exact remediation `1f96c4bf`, tree
`b320f552`, makes `StoredEnvelope`, `StoredRecord`, `StoredMessage`,
`StoredToolCall`, and `StoredToolOutput` object-only and `Role` string-only,
keeps the canonical writer unchanged, and grows fixed-fingerprint tracker
storage fallibly with unique keys, with at most 65,536 tracker entries. Exact
gate-record candidate `8f533cde`, tree `8215fb94`, passed the complete exact-1.94.1
local gate without fallback: focused 24 native/64 CLI process/16 differential,
Python 135/8 skips, byte-stable pinned fx `b1774fb`, WASI/FreeBSD with only the
established `read_file` warning, docs 85/147/626/81, `cargo-deny` 0.20.2 with
three established duplicate warnings,
`cargo-audit` 0.22.2 over 211 dependencies/1,226 advisories with zero
vulnerabilities, the unchanged 364-line production graph, and diff/inventory/
no-added-unsafe evidence are green.

The 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c` and
passed 16 equivalence categories across 20 records, 12 grammar cases, missing/
no-create, held-lock, engine-over-default, and 8,650,857-byte near-cap evidence;
the native near-cap probe passed 1/1. New allocator
total/current/maximum tuples are `12/2/7` and `819/14/645` bytes for empty/
short/long text, `14/2/8` and `1,427/14/1,059` for short/long JSON, and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys. These are bounded
regression results, not comparative measurements. Cycle 5 rejected exact
`8f533cde`: correctness/API `0/0/0/1`, native effects `0/0/0/0`, and
performance/resources `0/0/0/1`, deduplicated to one low stale cross-document-
summary finding. That documentation remediation is already composed in exact
cycle-6 candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Formal cycle 6 rejected it:
correctness/API, native effects, and performance/resources each reported
`0/0/0/1`; the deduplicated `0/0/0/1` is solely that these pages described the
committed remediation as pending. There was no additional production, API,
native, or performance finding. Formal cycle 7 rejected exact
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`: correctness/API and native effects
each reported `0/0/0/0`, while performance/resources reported `0/0/0/1`; the
deduplicated union is `0/0/0/1`. The sole low corrects resource wording:
shadowed duplicate values may parse more nodes than survive in the final tree.
The 65,536 caps apply separately to tracker entries and aggregate final decoded-
tree logical-node accounting, while the 8,651,165-byte file ceiling bounds
total parse work. Production and resource behavior were otherwise green. The
wording correction is present. Formal cycle 8 is **GREEN** on exact reviewed
candidate `d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
`6439863a9b7fd1720156c790fedc4798256c2b6d`: correctness/API, native/effects,
and performance/resources each reported `0/0/0/0`, with a deduplicated
`0/0/0/0` union. Independent evidence included 598 release-binary
differentials with zero mismatches plus focused 24 native, 14 CLI unit, and 24
CLI process tests; native also reconfirmed focused 24 and green WASI/FreeBSD
checks with the established `read_file` warning; performance confirmed the
exact allocator tuples above and the corrected cap semantics. This
documentation-only result seal is review-exempt: it records review of
`d724b61` / `6439863` and does not imply that the seal commit itself was
reviewed. Review-exempt documentation seal
`b6db9a67c070f7ef599d994c44b4a21731a004c5`, tree
`59dd628fd0552c5083449f7a31aa4241a8ecb952`, passed feature CI run
`32965947722` and Benchmark evidence run `32965947723`, was integrated on
`main`, and passed main CI run `32966531225` and Benchmark evidence run
`32966531319`. All four runs succeeded for exact `b6db9a6`; each benchmark
run retained exactly two unexpired exact-SHA artifacts. Slice 32 is delivered,
and the delivered count is 32. No product-performance or fx-equivalence claim
is made. Zig is used only to build the pinned upstream fx comparison input;
`machine-god` remains a Rust product and is neither written in nor shipped as
Zig. This final delivery-record commit is docs-only and user-exempt from
adversarial review. A commit cannot contain its own future workflow IDs, so
the exact workflow IDs for this record will be reported at handoff. The
slice stays non-equivalent, unmeasured, and claim-ineligible; no product-
performance or fx-
equivalence claim is made. See the
[`live ledger`](reviews/m03-session-cli-review-01.md).

Historical slice-32 review/gate lineage through cycle 3: the
[`session` command](session-cli.md) rejected exact
cycle-2 candidate `1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`. Correctness/API reported
`0/0/1/2`, native boundary/effects `0/0/1/2`, and performance/concurrency/
resources `0/0/1/1`, in blocker/high/medium/low order. The deduplicated union
has two medium themes—noncanonical JSON-number acceptance and residual payload-
proportional allocations—and two low themes—duplicate-key mismatch and stale
maintained documentation. The pre-remediation exact gate was green, including
all four exact-1.94.1 Rust commands, Python 135 with eight expected macOS skips,
pinned-fx regeneration, target/diff/no-unsafe checks, documentation integrity
85/146/626/81, and the release matrix. Its 4,001,712-byte binary had SHA-256
`e975e8a16f750188de25d8cf0eac02975643edf6730d6b3ad87d442b76ce27bb`.
Those results are not a performance result and do not approve the rejected
candidate.

The synchronized replacement contract requires a specialized one-pass summary
parser with at most 4 KiB per read, fixed-stack known-token scratch, canonical
`serde_json::Number` semantics, and payload-sized ownership only for the two
returned IDs. Metadata and nested arbitrary JSON use fixed-size key digests in
a strictly 65,536-entry-capped tracker that replaces a repeated key's prior
logical contribution, matching ordinary last-value-wins deserialization. Final
decoded-tree logical-node accounting is separately capped at 65,536. Shadowed
duplicate values can increase total parse work beyond that logical-node count,
but the 8,651,165-byte file ceiling bounds that work. The parser never buffers
the full file, constructs a `SessionRecord`, or retains transcript/metadata
payloads. The depth-64, final decoded-tree logical-node, schema, identifier,
counter, and content-shape ceilings are store-owned limits; they do
not enforce engine-configurable/default message, serialized-transcript, or
serialized-metadata limits. Exclusive sidecar-lock wait, filesystem latency,
and `EINTR` retries remain unbounded in wall-clock time and attempt count and
synchronously block the polling and CLI thread.

Cycle-2 replacement source was composed at exact
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the complete required
exact-1.94.1 local gate without fallback. Focused evidence is green at 56 CLI
unit, 54 CLI process, and 21 native inspection tests. Python passed 135 with
eight expected skips; pinned-fx regeneration is byte-stable; WASI/FreeBSD and
85/147/626/81 documentation checks are green; and no manifest, lockfile,
dependency, benchmark, inventory, or unsafe-Rust delta exists. The 4,001,760-
byte release binary has SHA-256
`483eb60f707cadfe4b0dd10cfb65617e576488546d908f2f6811b0bfc55773cc`.
Its green matrix includes six process differentials, an 8,650,857-byte near-cap
record, 4,097-message and 262,145-byte-metadata engine-over-default records, and
a held-lock wait of at least 500 ms. These are bounded regression and gate
results, not a comparative performance result. Formal cycle 3 rejected exact
candidate `9282b404`, tree `6d41f7ee`. Correctness/API and native effects each
reported `0/0/1/0`; performance/resources reported `0/0/1/1`. The deduplicated
`0/0/1/1` union is a medium context-free recursion-budget mismatch plus low
self-counted allocation evidence. Exact composed remediation `af055ff3`, tree
`14eafad`, implements `serde_json` 1.0.151-equivalent 127-active-container
accounting with metadata/JSON-content/tool parent contexts of 3/6/7. Its exact
accepted/rejected array-depth boundaries are 123/124, 120/121, 119/120, and
119/120. Focused evidence is now 58 CLI process tests with a ten-case
equivalence subset and 22 native inspection tests.

Historical cycle-3 `allocation-counter` 0.8.1 instrumentation is dev-only and
runs each shape in an isolated child process. Empty, near-cap, number-heavy,
message-heavy, and key-heavy records each measure exactly 14 total allocations,
2 current, 8 maximum, 8,913,715 total bytes, 14 current bytes, and 8,913,347
maximum bytes. This is evidence that parser-owned allocation counts and high-
water bytes are payload-shape-independent across those five cases, not a
latency or comparative-performance claim. The dev-only dependency delta passed
policy, license, and audit checks. The complete replacement gate is green on
exact `af055ff3`/`14eafad` under Rust/Cargo 1.94.1 without fallback. Python
passed 135 tests with eight expected skips; pinned fx is byte-stable;
WASI/FreeBSD retain only the established warning; docs are 85/147/626/81 with
zero errors; and diff/inventory/no-added-unsafe checks are green. Exact
`cargo-deny` 0.20.2 passed all categories with three established duplicate
warnings. Exact `cargo-audit` 0.22.2 loaded 1,226 advisories, scanned 211
dependencies, and found zero vulnerabilities. The 364-line production normal/
build dependency graph is unchanged.

The exact 4,001,760-byte release binary has SHA-256
`d296174898938f632351bebb38449533c7db03bb3659392bea3743a02ee1619d`. Its
session matrix passed 18/18, including ten equivalence cases, held-lock
behavior, and engine-over-default records. The direct 8,650,857-byte near-cap
case passed 1/1, as did the native near-cap/allocation case. Those were
regression/gate results, not comparative measurements. At that superseded
checkpoint, cycle-4 review, remote workflows, `main` integration, and delivery
were pending. The
fixed bootstrap inventory has no `session-json` workload and is
unchanged. This slice is deliberately non-equivalent, not measured, and claim-
ineligible; no sample, comparison, threshold, compatibility promotion, product-
performance result, or fx-equivalence claim exists.

The delivered slice-30 [`doctor` command](doctor-cli.md) has explicit resource
bounds but no performance result: exactly four closed checks and one complete
human or compact JSON representation capped at 4,096 bytes including final LF.
The bootstrap `doctor-json` record moves from unimplemented to implemented with
both pinned-fx and machine-god commands `not-measured`, `equivalence: non-
equivalent`, and `claim_eligible: false`. Workload order is unchanged. At that
delivery `sessions-json` and `background-json` were unimplemented; slice 31
below supersedes only the sessions classification. The doctor distinction is
intentional because machine-god has a four-check readiness contract and
different output/status/exit semantics. No sample, comparison, threshold,
compatibility promotion, product-performance result, or fx-equivalence claim
is introduced.

The delivered slice-31 [`sessions` command](sessions-cli.md) likewise adds
resource bounds without a performance result. It preserves the native scan's
100-ID, 1,024-entry, 64 MiB aggregate-record, and per-record ceilings and adds
a 16 KiB complete serialized-output ceiling. `sessions-json` moves from
`unimplemented` to implemented with both pinned-fx and machine-god commands
`not-measured`, `equivalence: non-equivalent`, and `claim_eligible: false`.
Machine-god returns a global, ascending, IDs-only truncated observation; fx
returns workspace-aware, newest-first, rich cursor-paged summaries and has
different corruption behavior. No sample, comparison, threshold, compatibility
promotion, product-performance result, or fx-equivalence claim is introduced.
`background-json` remains unimplemented.

The delivered twenty-ninth top-level
[`models` implementation](models-cli.md) supplies resource and concurrency
budgets, not a performance claim: one checked 30-second absolute provider deadline
covers default-eight capacity waiting and at most two sequential requests.
Each HTTP call creates an attempt-local timer against that unchanged deadline;
fallback does not reset it. Accepted body, JSON work, entries, IDs, and
serialized output are independently bounded. The 256 KiB machine-god body
buffer retains no additional witness byte; the first dependency frame that
would cross the cap is rejected before append. Its catalog listing is not an
equivalence-qualified benchmark workload. Cycle-1 candidate `6277aa3`, tree
`b5e2445`, passed its local gate but was rejected with two medium and six low
findings. Deadline, signal/config/WASI, and topology remediations are composed
at `02c9f86`, `d2890c3`, and `06c9408`; the current focused native total is 36
tests. Exact cycle-2 behavior candidate `2ea9d94`, tree `3a948b2`, passed the
complete replacement gate but was rejected with one high, one medium, and one
low finding. Parser and HTTP lifecycle remediation is composed at `9cf8c74`,
`8187b12`, and `499af85`. Pre-review gate attempt `c011398`, tree `4ac4e5b`,
was rejected because synchronous system-DNS snapshot work remained inside the
timed request poll. Eager bounded snapshot and zero-cache per-runtime resolver
remediation is composed at `d9922ef` and `e5248b1`. The complete cycle-3 gate,
exact candidate `2cecc921`, tree `8c0d235`, was green, but formal review rejected
the candidate with one medium and three low findings after deduplication.
Documentation, private bounded-DNS, and Android fail-closed remediation is
composed at `f80bd056`, `b6cf4cb`, and `bd47461`. Formal cycle 4 rejected exact
candidate `57d2ac2`, tree `d30bb656`, after its complete exact-1.94.1
replacement gate. The raw overlap-deduplicated union was 0 blocker, 0 high,
1 medium, and 2 low; after prior sealed dispositions, 0 blocker, 0 high,
1 medium, and 1 low remained unresolved at verdict collection. Topology
documentation is fixed at `268d35a`; signal/output-lifecycle remediation is
integrated at exact `aa60db1`, tree `278fa365`. Exact cycle-5 candidate
`27c75f4`, tree `5e40b24`, passed the complete exact-1.94.1 replacement gate
without fallback. Three fresh formal reviews each reported 0 blocker, 0 high,
0 medium, and 0 low findings; the deduplicated union is zero and the behavior
candidate is **GREEN**. A later claim-eligible M07 comparison remains required.
Review seal `2064084`, tree `33818a4`,
passed feature benchmark-evidence run `32923421739`, while CI `32923421679`
failed solely on a test-only Linux Clippy diagnostic. Exact test-only cycle-6
replacement candidate `831d38c8`, tree `a92acc14`, passed the complete local
gate and three zero-finding formal reviews. Documentation-only delivered seal
`bacc5c3`, tree `da3183a`, passed exact feature CI `32925681006`, feature
benchmark-evidence `32925681009`, main CI `32926242609`, and main benchmark-
evidence `32926242564`. `main` was fast-forwarded without force from
`1de3b7eddf6a4d9046d48098defecf6bfa336442`, and each benchmark run retains two
unexpired exact-SHA artifacts for 90 days. This is regression and delivery-
pipeline evidence only; it makes no speed, latency, memory, binary-size-
improvement, catalog-equivalence, compatibility-promotion, product-performance,
or fx-equivalence claim. The final delivery-record commit is documentation-only
and review-exempt; its workflows will be reported at handoff rather than claimed
here.

The CLI now selects the dedicated `ai-gateway-model-catalog-http` feature
rather than the broader `ai-gateway-http` feature. Resolved topology evidence
requires the catalog's direct HTTP/TLS/runtime dependencies while excluding
generation-only direct `bytes`, `web-fetch-http`, and Tokio's signal backend.
Current remediation keeps Hickory resolver only for bounded platform-
configuration parsing and uses private bounded Tokio UDP/TCP plus direct
Hickory protocol decoding for network resolution, so no Hickory resolver task
or request-polled entropy can outlive runtime teardown. Android fails closed
before platform loading. The broader `ai-gateway-http` feature still adds
direct `bytes` and web fetch, while the CLI alone requests Tokio signal handling
for its Ctrl-C/SIGTERM composition. Release size and hash are regression
evidence only and do not establish a performance improvement.

Initial workloads cover startup, help/status/session commands, session replay,
file indexing, deterministic streaming, parallel read-only tools, cancellation,
shutdown, RSS, and binary size. Model inference time is excluded.

Bootstrap measurements are infrastructure smoke evidence only. They are not
product baselines until the representative command, persistence, runtime, and
provider paths exist. CI uploads raw exact-SHA benchmark artifacts; reviewed
summaries and checksums are committed when a milestone makes a performance claim.

## Pinned upstream harness

`benchmarks/upstream.py` is the executable, standard-library-only harness for the
pinned `vercel-labs/fx` comparison source. Run it from a clean committed
machine-god worktree:

```sh
python3 benchmarks/upstream.py
python3 benchmarks/check.py benchmarks/results/upstream-bootstrap.json \
  --expected-git-sha "$(git rev-parse HEAD)" \
  --expected-runner-class "local-$(uname -s)-$(uname -m)" \
  --fx-binary .bench/fx/zig-out/bin/fx \
  --machine-god-binary .bench/scratch/machine-target/release/machine-god
```

The pinned Zig toolchain is used only to build that upstream fx reference. It
is benchmark infrastructure, not a machine-god product language, runtime, or
dependency.

The harness strictly parses `benchmarks/upstream.lock`, requires the locked Zig
0.16.0 and Rust 1.94.1 toolchains, and fails rather than silently substituting a
different version. It requires nonexistent upstream and scratch paths, clones fx
into the ignored `.bench/fx` directory, fetches and checks out the full locked
commit in detached-HEAD mode, and verifies the origin, commit, disabled hook
path, and a clean checkout including ignored files. Refusing an existing path
prevents prior ignored Zig caches, build output, local Git configuration, or
hooks from contaminating a run. Use fresh `--upstream-dir` and `--scratch-dir`
values for every invocation.

Source preparation disables system/global Git configuration, hooks, interactive
authentication, and local or `ext` transports. Before cloning, the harness also
checks tracked, untracked, and ignored machine-god files. Only `.bench`,
`benchmarks/results`, and `target` are accepted as output directories; an
untracked configuration or other possible build input fails the run. It then
parses byte-exact NUL-terminated porcelain records, including the separate
source path for rename/copy entries. Literal arrows, newlines, and other unusual
filename bytes are never interpreted as display syntax and cannot redirect an
outside path into the output allowlist. It then
lists the recorded machine-god commit with `git ls-tree` and reads every accepted
regular blob with `git cat-file`. Symlinks, gitlinks, special modes, unsafe
paths, and non-UTF-8 paths fail the run. This deliberately avoids `git archive`,
whose output can be changed by committed `export-ignore` and `export-subst`
attributes. The per-file path, Git mode, object ID, size, and SHA-256 digest are
stored in a canonical manifest under scratch and checked against the exact Git
tree, the materialized files, and their modes. Builds and measurements use only
that fresh source. Changing the developer worktree after materialization cannot
change the build input, and the tree is verified again after measurement and by
the checker. The exact release builds are:

```sh
(cd .bench/fx && zig build -Doptimize=ReleaseSafe)
(
  cd .bench/scratch/machine-source
  CARGO_TARGET_DIR=../machine-target \
    cargo +1.94.1 build --locked --release -p machine-god-cli
)
```

Use `--zig /absolute/path/to/zig` when Zig 0.16.0 is not the default `zig` on the
host. `--output`, `--runner-class`, `--runs`, and `--warmup` configure evidence
without changing its classification. At least ten measured runs and one warmup
are mandatory; the defaults are 30 and 5. Fetch/tool, build, and individual
sample limits default to 300, 1200, and 10 seconds and can be changed with the
corresponding `--fetch-timeout`, `--build-timeout`, and `--sample-timeout`
options. A timeout always terminates the original process group, closes captured
pipes, and uses bounded cleanup waits. Linux CI also enables the harness as a
child subreaper and supervises immutable PID identities with pidfds and `/proc`
parent relationships. A one-time hostile preflight must prove that a child which
clears its environment, calls `setsid`, and double-forks can still be discovered,
killed, and reaped. The descendant publishes its complete decimal PID through a
private staging file and atomic rename, so the supervisor never treats a merely
created, partially written marker as ready. Execution fails closed if subreaper,
pidfd, or process-table supervision is unavailable, or if a successful command
leaks a descendant. The post-success path uses a bounded settle period to
discover and `waitpid` every adopted child, including short-lived double-fork
zombies, and accepts a run only after no supervised descendant PID remains.
Linux measurement runs fork a child which creates its process group and blocks
on a private close-on-exec gate before executing the target. While it is blocked,
the parent opens the root pidfd, registers the exit observer, and attaches the
immutable root identity with a direct, single-PID `/proc/<pid>/stat` lookup. Only
then does it start the sample clock and release the gate. Thus even
sub-millisecond exits are observed from before exec, every requested run is
retained, and no duration-dependent retry or discard can bias the distribution.
There is no periodic process-table monitor. Pidfd readability timestamps root
exit without reaping it. Full descendant discovery, settling, and reaping begin
only after that timestamp. Every process record stores the measured interval
plus separate `setup_ns`, `supervision_ns`, and `cleanup_ns` durations.

Each built benchmark binary is identified by path, canonical path, SHA-256,
size, mode, device, inode, and timestamps before measurement. On Linux its bytes
are copied once to a sealed executable memfd and every gated child uses
descriptor-based `execve`; pathname replacement cannot change the executed
bytes. Other POSIX hosts use a private executable copy. The source identity and
pinned descriptor identity are verified before and after every warmup and
sample. Evidence records both identities, and the checker compares the recorded
source identity with each supplied built binary.
The final JSON is written atomically only after validation. A full-run,
per-output exclusive lock is acquired before collection starts and held through
publication. Lock acquisition has a bounded wait, and a pre-existing lock is
never removed as stale. A failed invocation removes neither the last
successfully published evidence nor another invocation's lock.
Publication uses an exclusively created, randomly named temporary in the output
directory. The harness writes through the retained descriptor, flushes and
`fsync`s it, verifies the pathname still names the same regular-file inode,
atomically replaces the destination without following a destination symlink,
and `fsync`s the parent directory. Cleanup unlinks only the temporary identity
the harness created. The full-run lock is also released only when its pathname
still names the inode created by that invocation. Output parent directories are
created as needed, and the leaf output name is never resolved through a
pre-existing symlink.

The supervisor is initialized before a command can launch and attaches the root
as an immutable `(PID, start_time)` identity using the already opened root pidfd.
Every post-timing ancestry expansion requires the current `/proc` start time to
match the identity already recorded, while all signals and adopted-child waits
use pidfds. PID reuse therefore cannot seed or signal an unrelated process tree.
Any exception after launch—including attachment, finalization, or reaping
failures—enters the same non-throwing, bounded cleanup path, which kills the
original group and known pidfds, closes pipes, reaps, and stops supervision
before propagating the original failure.

The schema 2 artifact records both source revisions, the verified fx origin and
commit and lock checksum, clone/fetch/checkout command records, CPU model, CI
image identity and runner class, resolved tools and versions, exact build
profiles and commands, binary hashes and sizes, benchmark commands, explicit
timeouts, sanitized environments, and every raw elapsed-time and exit-code
sample. Build and source-preparation stdout and stderr are represented by
SHA-256 digests so the artifact binds their output without embedding lengthy
logs. Build and measurement processes receive only recorded allowlisted
variables; ambient `RUSTFLAGS`, Cargo profile overrides, loader injection, user
configuration, and shared Zig/Cargo build caches are not inherited.
`CARGO_HOME` and `RUSTUP_HOME` used by the machine build must exactly match the
verified tool environment. Home, temporary, Cargo cache, Cargo target, Zig
caches, manifest, and materialized source paths are all derived from and checked
against the fresh scratch directory.

Each Git, Zig, rustc, and Cargo invocation is resolved to an absolute path while
retaining proxy-symlink dispatch semantics, and is also bound to its canonical
target. Evidence records the invocation symlink identity and target plus the
target's SHA-256 digest, size, executable mode, device, inode, and
modification/change timestamps. Both identities are checked before and after its
version command and every later harness use, including each release build; a
swapped or modified tool fails the run.

The checker binds schema 2 evidence to the repository's canonical
`benchmarks/upstream.lock`, the current machine-god SHA, both exact build
commands and profiles, and both actual executable files. It recomputes each
binary's size and SHA-256 digest and verifies that the measured command names the
same path. Evidence from different `runner_class` values remains explicitly
segregated. Linux CI downloads the official
[`zig-x86_64-linux-0.16.0.tar.xz`](https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz)
archive directly into a fresh fixed directory under `RUNNER_TEMP`, verifies its
official SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extraction, and requires `zig version` to report exactly `0.16.0`. It
uses the fixed `github-ubuntu-24.04-x86_64` runner class and retains the
validated artifact for the exact workflow SHA.

The current `bootstrap-exit` workload is deliberately labeled
`non-equivalent`: fx uses its upstream-only `FX_BENCH=1` fast path, whereas the
machine-god bootstrap binary prints its identity. It checks the harness and
captures launch samples, but it cannot support a product performance claim.

Help, JSON status, and JSON doctor now exist in both binaries, but existence is not semantic
equivalence. Schema 2 records the exact `fx help` / `machine-god help` and
`fx status --json` / `machine-god status --json` commands with both
implementations set to `not-measured`, no samples, `equivalence:
non-equivalent`, and `claim_eligible: false`. It records `fx doctor --json` /
`machine-god doctor --json` with that same non-equivalent, not-measured,
claim-ineligible shape. The current machine-god status is
only read-only native configuration/runtime metadata, and neither its behavior
nor its output contract has been shown equivalent to fx. Machine-god doctor is
the strict four-check read-only contract above, not a semantic match for fx.
`sessions-json` now exists in both binaries, but existence is not semantic
equivalence. It has the same non-equivalent, not-measured, claim-ineligible
shape as the other implemented local commands. The background-task workload
remains `unimplemented`; its fx command remains unmeasured because an unpaired
result is not a comparison.
The validator fixes these distinct shapes and rejects commands, statuses,
sample fields, or eligibility that drift from them. No M03 performance or
compatibility claim is made. A later milestone must define matching semantics
and fixtures and introduce claim-eligible reviewed evidence rather than relabel
this bootstrap artifact.

The delivered sixteenth
[`native session-listing candidate`](native-session-listing.md) established the
sorted validated IDs and truncation flag. Delivered slice 31 adds its CLI
path, so `sessions-json` is no longer called unimplemented. It remains
non-equivalent and claim-ineligible, with both commands not measured and no
samples or threshold claim, because it still lacks fx's workspace-aware rich
summary and pagination semantics. The pinned Zig input remains unchanged.

The eighteenth [`glob_files` candidate](glob-files.md) likewise adds no
benchmark workload or result data. Its strict fields, enum values, and bytewise
matcher are compatibility inputs, not proof that a machine-god scenario is
equivalent to fx. Both exact modes complete the bounded traversal by contract,
but no latency, throughput, allocation, or comparative-performance result has
been measured or claimed. Existing bootstrap classifications, eligibility,
thresholds, workflows, and the pinned upstream Zig input remain unchanged.

The nineteenth [`grep_files` candidate](grep-files.md) likewise adds no
benchmark workload or result data. Its eight strict fields, literal matcher,
include-glob grammar, three modes, and structured outputs are compatibility
inputs only, not equivalence evidence. No latency, throughput, allocation, or
comparative-performance result has been measured or claimed. The candidate
starts from exact base `f6aa458bb875d6cb26565adc878703fe140916d3`; its
tree-identical integration kickoff is
`f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Production, independent tests,
and documentation are parallel, non-overlapping components. Exact production
`27eec2f` and initial test `6eaee93` components exist and initially compose
through `9057feb` and `44e33d7`; fixture fix `bdbb677` makes focused
production/test composition green. Documentation component `b04151a` produces
first fully composed behavior `42e4793`; lint fix and exact local gates are
green at `45ad91f`. All three first-cycle tracks are **NOT GREEN** on exact
`355a11a`. Remediation and exact replacement local gates are green at final
code/test precursor `275d263`. First replacement candidate `ae87bf1` is **NOT
GREEN**: performance review found a medium slashful candidate/false-DP
cancellation gap and a low unmetered slashful selected-file decision, while the
other tracks each found one low issue. Second-fix production and documentation
compose through `ac5d772`, `d672210`, `7ad0863`, and exact local-gate-green
precursor `b498ba0`; `ae87bf1` remains historically **NOT GREEN**. The
formal second replacement candidate `5aeddc1` has correctness/API and
filesystem/robustness **GREEN** with zero findings. Performance/concurrency is
**NOT GREEN**: repeated eligible files can each allocate a 204,801-byte content
buffer, yielding approximately 2,048,010,000 cumulative allocated bytes for
10,000 empty files. The observed 6.10-second review run is diagnostic only, not
a contractual timing or product-performance result. Two low findings correct
the Markdown inventory from 57 to 58 files and identify an overclaim that the
recursive dynamic-programming branch already has deterministic evidence.
Third production remediation `8777825` composes at `ab1c133`; independent
regression `dcf57ad` composes at `d7526d4`; review-findings documentation
`44afb23` composes at `f08c5f2`; lint follow-up `1f13f9a` produces exact fully
composed local-gate precursor `a8f6179`. One scan-local content buffer reads
through an 8 KiB window, grows only as far as its 204,801-byte high-water cap,
and logically resets for reuse between files. This removes per-file maximum-
buffer allocation while preserving per-file and aggregate overflow witnesses;
it is a bounded design fact, not a measured product-performance result. Both
dynamic-programming branches now route through injectable cancellation checks,
with deterministic recursive and non-recursive regressions. Exact Rust 1.94.1
formatting, warnings-denied workspace Clippy, 598 non-documentation tests plus
two doctests, 25 private native tests, 40 direct `grep_files` tests, four engine
tests, cross-target/dependency/link validation, and diff checks are green.
Compatibility/release validation is green. Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. In diagnostic allocator instrumentation,
two 10,000-file boundary scans at `5aeddc1` requested approximately
4,103,462,456 bytes and made 20,000 allocations of exactly 204,801 bytes;
`0bfe68a` requested approximately 7,459,007 bytes and made zero maximum-sized
allocations. Its maximum-plus-384 regression requested approximately 3,349,064
bytes and made one high-water allocation. Allocation and timing instrumentation
is diagnostic only, not a contract or product-performance result. Isolated
wording remediation `993b618bf78d30f6a68f3b248b572e33e4de1126` composes at
exact `f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
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
self-recorded. The corrected behavior
contract requires cancellation-aware
slashful candidate splitting at intervals of at most 1,024 bytes, checks in
both dynamic-programming branches, and one charged cancellation-checked
slashful selected-file rejection.
Existing bootstrap
classifications, eligibility, thresholds, workflows, and pinned upstream Zig
input remain unchanged; machine-god remains a Rust product.

## Milestone 02 orchestration note

The durable tool loop checks the complete transcript before each provider
request and optimistic store mutation. That work is linear in the current
serialized transcript size, and a compare-and-save retry repeats it. Message and
serialized-byte limits cap the work; the serializer aborts at the byte limit and
does not allocate a second JSON buffer. The canonical record is held behind an
immutable `Arc`: mutex-protected work copies only the record identity and small
persistence state, while full validation, serialization, equality, and deep
cloning happen after the mutex is released. An identity recheck precedes the
store CAS, retaining divergence safety without holding the session lock for up
to the 8 MiB transcript bound. Tests exercise a growing transcript and its
enforced boundary, but M02 makes no throughput or latency claim for this path. A
future representative session-replay benchmark should measure increasing
history sizes and conflict rates before an optimization claim is made.

JSON container depth and node count are validated with an explicit
iterator-frame stack. The walk is linear in visited nodes, stops at node limit
plus one on rejection, and uses O(depth) auxiliary memory rather than recursion
or a work queue proportional to all siblings. Aggregate validation reuses one
counter for a complete schema catalog, inference-metadata collection, or stored
record; individual provider arguments and results each start one counter. It
precedes recursive serialization and cloning wherever core controls the
boundary.

Configured depth may be lowered but cannot exceed the audited hard ceiling of
64. Construction rejects a larger value before catalog work rather than
clamping it. This fixed ceiling bounds the stack exposure of recursive
serialization, cloning, extension code, retained-value destruction, and other
accepted-value paths; the node and byte limits remain operator-selected resource
budgets because their traversal is iterative once depth is structurally safe.

Rejected owned values are reclaimed by consuming their arrays and maps through
the same shape of iterator stack. This costs O(actual nodes), not merely the
validation prefix, and O(actual depth) auxiliary memory; it prevents both a
recursive destructor overflow and a leak. Values still queued inside a provider
stream remain provider-owned and require a stack-safe stream destructor. These
limits also cannot recover allocation already performed while a caller built
options, a store decoded a record, a tool built a specification or result, a
provider built an event value, or a policy built its decision; those producers
require their own decode/allocation bounds.

## Slice 35 cycle-13 question resource remediation

Status: **CYCLE 13 FORMAL REVIEW GREEN — FEATURE WORKFLOWS PENDING**.

The first `ask_user_question` slice has no product-performance claim or new
benchmark workload. Its resource contract is structural: at most four
questions, 24 options, 32 KiB incoming serialized arguments, 32 KiB aggregate
rendered presentation text, 48 KiB normalized arguments, 4 KiB aggregate
complete pre-trim host answers, 16 KiB rendered answers, a 41,102-byte reachable
serialized result maximum, and a separate 48 KiB defense-in-depth result guard.
Per-field raw and rendered limits prevent one string from consuming an
aggregate budget. Every addition and encoding expansion is checked, and no
overflow path truncates or partially publishes an answer.

The normative target is linear preparation in accepted input plus ASCII-only
duplicate comparison over at most six labels per question, so its quadratic
term has a fixed 24-label ceiling. Formal cycle 1 rejected exact candidate
`6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`. The performance/resources track
reported 0 blocker, 0 high, 2 medium, and 0 low findings. First, its serialized
JSON sizing helper scanned an entire arbitrarily oversized string value or
object key before applying the remaining 32/48 KiB budget. Cycle 2 makes the
scan remaining-budget-aware and stops at the first over-limit byte. Second, the
ledger overstated exact/+1 boundary and maximum-concurrency evidence; this
overlaps the correctness evidence-overclaim finding and counts once in the
deduplicated union. The expanded direct suite now establishes those boundaries.

Terminal encoding and result construction are intended to remain linear in
their bounded inputs. Formal cycle 2 found that the submitted implementation
trimmed a host answer before checking its length, so an arbitrarily large
whitespace-only string could force an unbounded synchronous scan. Cycle 3
freezes an O(1) complete-string length check before trim: both each answer and
the aggregate complete pre-trim bytes are limited to 4,096. The tool owns one
prompt future and one permit; the limits are default concurrency one and hard
maximum eight with fail-fast saturation and no waiter queue or capacity Waker.
Exact-limit and
first-over-limit input, prepared, presentation, and answer evidence, plus
explicit one/eight/ninth-admission and independent-counter evidence, were green
in the 26-test direct suite. Its claimed exact/+1 49,152-byte result evidence
was invalid because legal inputs reach at most 41,102 serialized bytes. Cycle 3
adds exact and first-over complete pre-trim answer evidence and exact reachable
41,102-byte maximum evidence, growing the direct suite to 28. The unreachable
49,152-byte guard remains authoritative defense in depth and is not claimed as
a reachable rejection boundary. Deep and
maximum-depth drop paths,
wrong-name/canonical-prepared rejection, and unpolled/pending/drop/unwind
resource ownership are also exercised deterministically.

No timeout is claimed. Pending duration and externally allocated UI state are
properties of the injected prompter. A conforming prompter detaches no work;
the adapter starts no task, thread, timer, retry, channel, or runtime.
Historical behavior head `a76818e`, tree `f44def5`, passed its recorded local
gate, but formal cycle 1 rejected the later immutable candidate with a
deduplicated 0 blocker / 1 high / 3 medium / 3 low union. Cycle-2 evidence
`c77b336`/`0dd1128`, production `9d2e0f2`/`47e9505`, and finding docs compose
at exact `c8718c6`, tree `c27463b`; the complete exact-1.94.1 local gate is
green with 55 focused tests. Formal cycle 2 rejected exact candidate
`910d7bc`, tree `503a91f`: correctness/API reported `0/0/1/0`, lifecycle/
platform `0/0/0/2`, performance/resources `0/0/2/0`, and the deduplicated union
is `0/0/2/2`. Cycle-3 source `cf531d1`/`b7b4358`, evidence
`3e3c0c7`/`f3f6f9d`, and docs `bfdf05b` compose at exact behavior head
`8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. The complete exact-1.94.1
local gate is green with 57 focused tests, including 28 direct tests. Formal
cycle 3 rejected exact candidate `746e510c7d8eb93229996e74f91827f489e5bb31`,
tree `c49221efbea66c840b333f0de0161aa686aad52f`.
Performance/resources reported `0/0/2/0`: an arbitrarily large malformed
`Answered(Vec<String>)` is rejected by count but still destroyed synchronously,
and permit destruction precedes teardown of every prompt/cancellation waiter
and retained Waker. Lifecycle reported both at low severity; the deduplicated
union keeps their medium severity and totals `0/0/3/2` with the separate API
and documentation findings.

Cycle 4 privately stores at most four answer strings in fixed four-slot storage
while still admitting zero through four entries so legal count mismatches
remain testable. It also keeps the active permit held through prompt-future and
cancellation waiter/Waker teardown on return, pending drop, and unwind,
bounding those arbitrary destructor callbacks inside the configured
concurrency slot. Core `e569514`/`4c8cff3`, native
`53c05cd`/`1857a3f`, and finding docs `b057958` compose at exact behavior head
`cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`. Its complete exact-1.94.1 local
gate is green, including 30 direct question tests and 171 named focused
executions overall. The release artifact remains 3,985,216 bytes with SHA-256
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
Formal cycle 4 rejected exact candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`. Correctness/API reported
`0/0/0/1`, lifecycle/platform `0/0/1/1`, and performance/resources
`0/0/1/0`; overlap deduplicates to `0/0/2/1`. The resource defect is that
concurrent cancellation can move the registered Waker callback out of the
waiter, after which outer drop can release capacity while the callback tail
continues. The related final-Waker teardown path can also make cancellation
observable only after the last direct-return check. Cycle 5 requires every
registered or cached equivalent cancellation Waker clone and its callback to
retain the originating permit through callback return, with prompt/waiter/
cached-Waker teardown inside the activity and a final post-teardown
cancellation check before direct returns. Cycle-5 evidence
`ad47fcb`/`bcce292`, source `80382d8`/`e0fd8e0`, and finding docs
`ba53f55`/`b870731` compose at exact behavior head
`b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Deterministic no-sleep evidence
pins both races; direct cross-composition `e1947c1`, tree `e63f8272`, and the
integrated 32-test direct suite are green. All required exact-1.94.1 and
extended gates pass, including the portability, dependency, audit, and resource
checks. The existing audited reentrant-Waker fixture is added only as a native
path dev-dependency; production normal/build dependencies and the 211-package
audit inventory are unchanged. The fresh locked release binary remains
3,985,216 bytes with SHA-256
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
These are bounded ownership and regression facts, not a benchmark or product-
performance result. Formal cycle 5 reviewed exact candidate
`54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
`54586d2256c8a3d2289b92bc9bc842eed9ce4d07`. Correctness/API and lifecycle/
platform each reported `0/0/0/0`; performance/resources reported `0/0/1/0`,
which is also the deduplicated union. The candidate is rejected on one product
resource/capacity medium: arbitrary retained activity-Waker clones can each
forward a concurrent blocking downstream callback while consuming only one
prompt slot.

Cycle 6 must introduce one activity-backed single-flight coalescing notifier.
At most one callback may be in flight, pending notifications must replay
without loss, the stale downstream target must close, and capacity must remain
held until callback and retained-clone ownership is gone. Deterministic owned-
future evidence must cover many independently retained clones.

Finding docs `7dee269`/`e20023c`, evidence `b007ada`/`4a929c4`, and source
`0488d71`/`707a794` compose at exact head
`707a794230758374fa2dab6d65eaf27449c7c477`, tree
`1e60299e21f45079f4e8cf27468a28d1ab4fe227`. The notifier's one mutex-protected
state tracks target, lifecycle, callback-in-flight, and one replay bit. Notice
bursts return after setting that bit while one callback remains in flight;
callback return performs one serialized replay if still open. Completion/drop
closes target and replay, and notifier ownership retains the permit through
every callback and retained clone.

`cloned_prompt_wakers_coalesce_blocking_callbacks_and_replay_once` wakes 16
retained clones concurrently, proves maximum callback concurrency one, and
observes exactly one replay. `completed_prompt_closes_retained_waker_delivery_until_every_clone_drops`
proves no stale callback after completion and no capacity recovery until the
blocked callback returns and final clone drops. Independent
`236dd90`/`94b9fdd3980a413c594538fc9222b09007518bce` passes direct 34, engine
one, and native Clippy. The integrated focused, all four pinned workspace,
Python 136/8, pinned drift, dependency/audit 1,226/211/zero, portability, docs
91/318/701/534/0, protected/no-unsafe, and unchanged release-smoke gates are
green. The only manifest/lock delta remains the existing audited test-only
helper and one native lock dependency-list line; production normal/build
dependencies are unchanged. Formal cycle 6 reviewed exact candidate
`85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
`fd3c5072c9473c7fe8767cc2692238eacb8a0f43`. Correctness/API reported
`0/0/0/1`; lifecycle/platform and performance/resources each reported the same
`0/0/1/0` medium; the deduplicated union is `0/0/1/1`.

The medium rejects the current replay loop. Every notice during callback
delivery sets `pending_replay`. A replay callback that self-notifies therefore
rearms another loop iteration; one original wake can consume unbounded
synchronous callback work even though maximum callback concurrency remains one.
Cycle 7 must make coalescing observation-aware and hold callback count to a
constant bound under continual self-notification. It must still replay a notice
without loss when an outer observation occurs before the new notice.
Deterministic finite-budget evidence must prove both paths before a replacement
gate and three fresh reviews.

The low is the stale cycle-5 local-gate status in the three operative opening
summaries. The rejection-doc checkpoint aligned their then-current status; this
cycle-7 checkpoint advanced all ten operative regions to its then-current
local-gate status with a focused 10-current/zero-stale result.

Cycle-7 rejection docs `6128f03`/`1d354ff`, evidence `acca13c`/`b75fc54`, and
source `3d48ce8`/`fbb3f5c` compose at exact behavior head
`fbb3f5c5f40d0726b444b1ebc6f25fb1ee1fee36`, tree
`7cee96e0701d11925360f3d1b6315f5801bbd807`. Entry to delivery clears observed
and pending; pre-observation notices coalesce into the in-flight callback; bind
marks observation; only a later notice earns one replay; replay clears
observation; close/panic clears both. Maximum callback concurrency remains one,
retained activity holds capacity, and no foreign Waker work occurs under lock.

`reentrant_prompt_wake_before_outer_repoll_has_constant_callback_work` rejects
the cycle-6 base after 65 callbacks exceed budget 64; the fix returns after one.
`cloned_prompt_wakers_replay_once_after_outer_repoll_observes_the_burst` proves
one lossless replay after observation. Cross-composition `c0c9eb0`/`6fd79ed`
passes formatting, direct 35, engine one, and native Clippy. Focused, all four
pinned, Python 136/8, compatibility, deny/audit 1,226/211/zero, portability,
docs 91/318/701/534/0, status 10/0, protected/no-unsafe, and unchanged release-
smoke gates are green. The exact Cargo delta remains the audited dev-only test
fixture and one native lock list line; production graph is unchanged. Formal
cycle 7 reviewed exact `617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`. Correctness/API and performance/
resources each reported `0/0/0/0`; lifecycle/platform reported `0/0/1/0`;
the deduplicated union is `0/0/1/0`, so the candidate is rejected.

The accepted medium is a product lifecycle/resource-ordering defect. The
notifier selects replay B before dropping prior target A. If A's destructor
panics, `notifying` remains wedged; if it reentrantly closes or replaces the
lane, stale B can still be delivered. Cycle 8 must drop A before selecting a
replay, catch and settle drop unwind plus lane flags, and admit only the then-
current replay after successful destruction. Deterministic tests must prove
panic recovery and reentrant-close suppression before a replacement gate and
three fresh reviews. Analogous preexisting terminal code is outside this
bounded slice and is not claimed fixed.

Cycle-8 docs `22d5702`/`3650dba`, evidence `cf4abfd`/`5681bab`, and source
`a1b3d23`/`d8075ff` compose at exact behavior head
`d8075ffee2d6765df2ce7842300e26bb7127d52b`, tree
`fa32564476ce6a74cd3ba09c48a4b98af602cb72`. A is destroyed under
`catch_unwind` outside the lock while lane/activity ownership is retained.
Only afterward does arbitration read the current lifecycle, pending bit, and
target. Destructor close/replacement wins; callback or target-drop panic clears
the lane; callback panic wins if both occur; no foreign work runs under lock;
and maximum callback concurrency remains one.

`replay_target_drop_panic_clears_lane_for_a_fresh_notification` records fresh
B=0 on the rejected base's wedged lane, then proves the fix recovers and
delivers. `replay_target_drop_close_suppresses_selected_replay_and_retains_capacity`
records one stale B delivery on the base, then proves close suppression and
capacity retention. Cross-composition `01d9a06`/`c917dce` passes formatting,
direct 37, engine one, and native Clippy. Focused, all four pinned, Python
136/8, compatibility, deny/audit 1,226/211/zero, portability, docs
91/318/701/534/0, status 10/0, diff/protected/no-unsafe, and unchanged release-
smoke gates are green. The exact Cargo delta remains the audited dev-only
fixture and one native lock list line; the production graph is unchanged.
Formal cycle 8 reviewed exact candidate
`e929b5ea7e3264c2b56066a416bc2a979a03b214`, tree
`cfadc42814688a29c4d512e5fd91c843423821d4`. Correctness/API reported
`0/0/0/0`; lifecycle/platform and performance/resources each reported
`0/0/1/0`; the distinct mediums produce a deduplicated `0/0/2/0` union.

One medium is panic/lane integrity: destruction of a captured secondary target-
drop panic payload can itself panic and override the promised primary callback
panic. Cycle 9 must suppress or forget that secondary payload and use marker
evidence to prove primary identity, lane recovery, capacity retention, and
fresh delivery.

The resource medium is unbounded replay work under synchronous re-poll/re-
notify. Every callback can mark itself observed and earn another replay, so one
activation executes 257 callbacks for budget 256. Cycle 9 must cap each explicit
notify activation at the initial callback plus at most one replay, retain
residual pending work for later explicit activation, preserve maximum callback
concurrency one and capacity ownership, and add deterministic large-budget
evidence. Analogous preexisting terminal code is outside this bounded slice and
is not claimed fixed.

Cycle-9 docs `2faedc7`/`5296dcc`, evidence `cf2e220`/`ee25455`, and source
`527e10d`/`0279b8c` compose at exact behavior head
`0279b8cb744b8d5cee92d2bfc263abcca60a9987`, tree
`50b2423637fc9eb8f0cd6792874a2385ff32fd06`. One explicit notify activation is
initial plus at most one replay. Residual post-observation pending work survives
lane release for later activation; close/panic clears; concurrency stays one;
A drops before arbitration; and capacity remains retained. Dual panic forgets
the opaque secondary payload to preserve callback-primary precedence; a single
target-drop panic propagates.

`callback_panic_precedes_panicking_replay_target_payload_drop` proves primary
marker identity plus lane/capacity/fresh delivery.
`one_notify_activation_has_one_replay_and_leaves_residual_pending_work` rejects
base `e929b5e` at 257 callbacks for budget 256; cycle 9 performs two callbacks,
then reaches four total only after later activation while residual work
decrements. Cross-composition `13eccf9`/`56695d8` passes formatting, direct 39,
engine one, and native Clippy. Focused, all four pinned exact-1.94.1 commands,
Python 136/8, compatibility, deny/audit 1,226/211/zero, portability, docs
91/318/701/534/0, status 10/0, diff/protected/no-unsafe, and unchanged release-
smoke gates are green. The authorized Cargo delta remains the dev-only fixture
line plus one native lock list line; production normal/build dependencies are
unchanged. Formal cycle 9 rejected exact candidate
`1eeab670a552bc15b5602319b0bb1ce27d2be497`, tree
`5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`. Correctness/API and lifecycle/
platform each reported the same `0/0/1/0` medium, performance/resources
reported `0/0/0/0`, and the deduplicated union is `0/0/1/0`.

After the initial callback plus one replay budget is exhausted, a legal wake
after the replay poll remains only in `pending_after_observation`. Releasing
the lane schedules no downstream callback; only unrelated later explicit
notify activity consumes the work. The committed regression manually invokes
`retained_wakers[2]`, so a self-waking prompt or cancellation transition whose
wake is last may remain `Pending` indefinitely. No timeout exists.

Cycle-10 docs `216c3b4`/`895c9d4`, final evidence
`74a849791e311759630d0204d692190a39da279c`/`5e46f56` (superseding
`5cbd9b0`), and source `b0433648b1c836a8db6151f64b461196830fea92` compose at
exact behavior head `72e8e75ba2490d4dfa0f680d9dca0b4e10a0401a`, tree
`5405180e5b3b4b59c4d7e712f614bdbc958a9d75`; disposable final composition
`a8acbf4`/`54124807ac991cc93dc15db28bad21ac8e2a19ae` passes formatting/direct
41.

The serialized nonrecursive lane has `Open`, `DeliveryResourceExhausted`, and
`Closed` states and an exact per-activation downstream cap of 256. Calls 1-255
advance normal observation-aware wakes. Continuation after poll 255 records
sticky exhaustion before terminal callback 256; its outer poll checks
cancellation first and otherwise returns the existing redacted nonretryable
prompt-failed error. Exhaustion suppresses retained binds/wakes until close.
Short residual/cancellation chains now advance autonomously through callback 3.
No thread, queue, dependency, public API, or lock-held foreign Waker operation
is added; established close/panic/A-drop/panic-ordering/permit rules remain.

`observed_residual_wakes_progress_without_an_unrelated_activation` proves base-
two to autonomous-three short progress;
`continuously_rewaking_prompt_stops_at_the_delivery_limit_with_redacted_error`
proves base-two to exact terminal-256 continuous exhaustion; and
`cancellation_in_the_residual_wake_window_progresses_and_closes_delivery`
proves base-two/new-three cancellation-first close.
Focused direct 41, all required exact-1.94.1 commands, and extended/release-
smoke gates are green. Formal cycle 10 rejected exact candidate
`4ea1c1f5be3586ce9bee696b12c4120dc2a72018`, tree
`78e781ffd7b03aafdf295ae79f4090120971c248`. Correctness/API and lifecycle/
platform each reported `0/0/1/0`, performance/resources reported `0/0/0/0`,
and the distinct union is `0/0/2/0`.

The resource defect is a queued-executor budget bypass. Because
`callbacks_started` is notify-local and a queue-only callback returns after
enqueue, the lane clears `Open` before the queued `Pending` poll self-wakes.
Each wake starts again at one, so 256 is never reached and capacity remains
occupied indefinitely. Existing evidence covers synchronous reentrant callback
execution only. Cycle 11 needs one prompt-lifetime budget with queue-driven
exact-bound and cancellation evidence.

The separate cleanup defect can replace the primary panic or double-panic abort
when discarded/nonselected opaque cleanup payloads destruct during unwind.
Cycle 11 must forget all suppressed payloads and prove primary identity, no
abort, lane close, and capacity recovery. Cycle-11 docs `d839d18`/`f8342db`,
evidence `3692d19608900ecc39c6babe8f90e06ea9cc3821`/`adf2b93`, and source
`83dd836bd684dac90e5b161087eded1a04b336d6` compose at exact behavior head
`b8b721a065f4b14f5f3678a22ee5b0bd2267ca2f`, tree
`46721503429685e5feb8e4ac33f74e865acf0c2a`; disposable composition
`49b1186c51d3873fb9e329eb38a0c792d6646850`/`40d474d` passes direct 46 and
all focused portability checks.

The state-owned prompt-lifetime budget calls `begin_callback` before every
queued, synchronous, or external callback. Calls 1-255 are ordinary; sticky
exhaustion precedes terminal callback 256, cancellation wins, later delivery is
suppressed, and panic does not refund budget. One nonrecursive serialized lane
keeps concurrency at most one. Central cleanup precedence forgets every
suppressed/nonselected payload before selected resume. Four tests plus a
subprocess child correct base callback-256 Pending, callback-257 cancellation,
primary replacement, and `SIGABRT` while proving lane/capacity recovery.

Direct 46, all required exact-1.94.1 commands, extended/release-smoke gates,
and the unchanged dev-only fixture/lock delta are green. Formal cycle 11
rejected exact candidate `b1d454ba21d2a380a4198bb1253c4cb1bc34d4a6`, tree
`26d90d8ec3924f6b7e12617506d5275ae32ec00b`. Correctness/API reported
`0/0/0/1`, lifecycle/platform `0/0/1/0`, performance/resources `0/0/1/0`,
and the distinct union is `0/0/2/1`.

The product-resource medium is a capacity leak: a forgotten opaque panic
payload can retain the supplied prompt Waker, whose `ActivityWake` owns the
active permit for its entire `Arc` lifetime. Cycle 12 must use a state-held
permit and admitted callback guard; close removes state capacity only after
target teardown while its local guard preserves ordering. Forgotten closed
Wakers must retain no capacity, with ordinary/ambient retained-Waker recovery
evidence. Cycle-12 docs `5047d40`/`e582331`, source `0684f3e`/`54d0af0`, and
evidence `87d175b` compose at exact behavior head
`696dccfa84b9ce0a57ca4f764a6f05aefedb39f3`, tree
`f8734a8815b424f07d59f668f5ccd2a59319a8b1`; disposable source/evidence
composition is `8378a47`/`522d0a4`.

Release panic is `unwind`, and an independent release-product probe emits exact
34-byte `primary-caught\ncapacity-recovered\n`. The permit lives in detachable
state. Admitted callbacks retain local guards; close retains its guard through
target teardown and then releases capacity, so closed forgotten Waker identities
retain none. Ordinary and ambient retained-Waker payload evidence records fresh
admission under the test profile. Direct 46, manifest eight, all exact/extended/
portability gates,
docs/status 91/318/701/534/0 and 10/0, and the 4,481,664-byte release/smokes pass.
Formal cycle 12 rejected exact candidate
`3dec7a2f073fa85479af19765b03b06cdfd9da8c`, tree
`c34d20a45f70b82652bf78df9653f39399d7fc6d`. Correctness/API reported
`0/0/1/1`, lifecycle/platform `0/0/1/2`, performance/resources `0/0/1/0`, and
the deduplicated union is `0/0/1/2`. The shared resource/evidence medium is that
the release probe uses only a prompt-poll panic and `NoopWake`; it does not
exercise a target-drop secondary payload retaining the supplied Waker, stale-
lane suppression, or capacity recovery while closed clones stay retained.
Cycle-12 rejection docs `70c929f15d345431b4673f799a29b2b45eee2c5d`/
`f74ebaf` and cycle-13 evidence `c9f9535892441cc6b0f4a99f115365f10a7c8426`,
integrated as `c252620f55eb75edbb1f771950200168671ef0f3`/`a921449`, replace
the release evidence and rename one stale test without changing production
source, API, dependencies, or resource limits. The optimized two-case probe
preserves ordinary prompt-drop and ambient primaries, creates a target-drop
secondary payload owning the supplied Waker, proves its destructor panics in a
control, suppresses/forgets it in product, and records two target drops, two
secondary callbacks, zero stale wakes, and two fresh admissions. Exact stdout
is 193 bytes and stderr is empty. The complete exact local gate is green,
including a fresh 4,481,664-byte CLI release with SHA-256
`a568e58e07b02a3b9739f1210794ad698faa8c6aec9933247150e19fa67799b4`.
Formal cycle 13 reviewed exact `a4f1bb91c00064e0ceb6975e1c9e7b4a09b1ff95`/
`72a0303`: correctness/API/evidence, lifecycle/platform/concurrency,
performance/resources, and union each reported `0/0/0/0`. Reviewers validated
the real target-drop path, both primaries, secondary Waker ownership/control/
suppression/forget, stale-zero/fresh-capacity ordering, lifetime-256 and all
other resource bounds, and current unwind wording. All prior findings are
resolved. This review-exempt docs seal makes no behavior, performance,
equivalence, or delivery claim. Feature workflows, integration, `main`
workflows, and delivery remain pending. See
[`ask-user-question.md`](ask-user-question.md).
