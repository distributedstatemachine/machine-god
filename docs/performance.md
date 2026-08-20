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
killed, and reaped. Execution fails closed if subreaper, pidfd, or process-table
supervision is unavailable, or if a successful command leaks a descendant. The
recorded sample time stops immediately when the command's captured streams
close, before any post-run containment scan. The final JSON is written atomically
only after validation; failure removes the named evidence output rather than
leaving a partial or stale artifact.

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

Each Git, Zig, rustc, and Cargo executable is resolved through symlinks to one
canonical absolute path. Evidence records its SHA-256 digest, size, executable
mode, device, inode, and modification/change timestamps. The identity is checked
before and after its version command and every later harness use, including each
release build; a swapped or modified tool fails the run.

The checker binds schema 2 evidence to the repository's canonical
`benchmarks/upstream.lock`, the current machine-god SHA, both exact build
commands and profiles, and both actual executable files. It recomputes each
binary's size and SHA-256 digest and verifies that the measured command names the
same path. Evidence from different `runner_class` values remains explicitly
segregated. CI uses the immutable pinned Zig setup action for 0.16.0 and the
fixed `github-ubuntu-24.04-x86_64` runner class, then retains the validated
artifact for the exact workflow SHA.

The current `bootstrap-exit` workload is deliberately labeled
`non-equivalent`: fx uses its upstream-only `FX_BENCH=1` fast path, whereas the
machine-god bootstrap binary prints its identity. It checks the harness and
captures launch samples, but it cannot support a product performance claim.
Help, status, doctor, session-list, and background-task workloads are recorded
as `unimplemented` for machine-god and are not timed on fx in isolation. Once
both products expose matching semantics and fixtures, a later milestone must
introduce a claim-eligible schema and reviewed workload definition rather than
relabel this bootstrap evidence.
