# Performance

Performance comparisons must build both projects in release modes on identical
hardware, retain raw samples, warm up before at least 30 measured runs, and report
median, p95, confidence intervals, RSS, and binary sections.

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
