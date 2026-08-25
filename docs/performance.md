# Performance

Performance comparisons must build both projects in release modes on identical
hardware, retain raw samples, warm up before at least 30 measured runs, and report
median, p95, confidence intervals, RSS, and binary sections.

The locally composed, not-yet-reviewed twenty-ninth top-level
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
tests. The complete replacement gate, three fresh cycle-2 reviews, exact-SHA
workflow artifacts, and a later claim-eligible M07 comparison remain required
before any speed, latency, memory, or fx-equivalence statement.

The CLI now selects the dedicated `ai-gateway-model-catalog-http` feature
rather than the broader `ai-gateway-http` feature. Resolved topology evidence
requires the catalog's direct HTTP/TLS/runtime dependencies while excluding
generation-only direct `bytes`, `web-fetch-http`, Hickory DNS, Moka, and
Tokio's signal backend. The broader `ai-gateway-http` feature still adds direct
`bytes` and web fetch, while the CLI alone requests Tokio signal handling for
its Ctrl-C/SIGTERM composition. Release size and hash are regression evidence
only and do not establish a performance improvement.

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

Help and JSON status now exist in both binaries, but existence is not semantic
equivalence. Schema 2 records the exact `fx help` / `machine-god help` and
`fx status --json` / `machine-god status --json` commands with both
implementations set to `not-measured`, no samples, `equivalence:
non-equivalent`, and `claim_eligible: false`. The current machine-god status is
only read-only native configuration/runtime metadata, and neither its behavior
nor its output contract has been shown equivalent to fx. Doctor, `sessions-json`,
and background-task workloads remain `unimplemented` for machine-god; their fx
commands also remain unmeasured because an unpaired result is not a comparison.
The validator fixes these distinct shapes and rejects commands, statuses,
sample fields, or eligibility that drift from them. No M03 performance or
compatibility claim is made. A later milestone must define matching semantics
and fixtures and introduce claim-eligible reviewed evidence rather than relabel
this bootstrap artifact.

The first formal sixteenth
[`native session-listing candidate`](native-session-listing.md) does not change
that classification. Its first formal candidate was not green; the composed
replacement is green across all three review tracks and awaits remote delivery.
Its
library result contains only sorted validated IDs and a truncation flag; it has
no CLI path and is not semantically equivalent to fx's
workspace-aware rich `sessions --json` surface. The `sessions-json` workload
therefore remains unimplemented and claim-ineligible, with no samples or
threshold claim. No benchmark definition, evidence schema, inventory, workflow,
or pinned Zig input changes in this slice.

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
