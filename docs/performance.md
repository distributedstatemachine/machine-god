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
  --expected-git-sha "$(git rev-parse HEAD)"
```

The harness strictly parses `benchmarks/upstream.lock`, requires the locked Zig
0.16.0 and Rust 1.94.1 toolchains, and fails rather than silently substituting a
different version. It clones or fetches fx into the ignored `.bench/fx`
directory, checks out the full locked commit in detached-HEAD mode, verifies the
origin, commit, and clean checkout, and then runs these builds:

```sh
(cd .bench/fx && zig build -Doptimize=ReleaseSafe)
cargo +1.94.1 build --locked --release -p machine-god-cli
```

Use `--zig /absolute/path/to/zig` when Zig 0.16.0 is not the default `zig` on the
host. `--upstream-dir`, `--fixture-home`, `--output`, `--runs`, and `--warmup`
also allow isolated invocations without changing the source or evidence model.
At least ten measured runs and one warmup are mandatory; the defaults are 30 and
5.

The schema 2 artifact records both source revisions, the verified fx origin and
commit, clone/fetch/checkout command records, host details, resolved tools and
versions, build profiles and commands, binary hashes and sizes, benchmark
commands and non-secret environment overrides, and every raw elapsed-time and
exit-code sample. Build and source-preparation stdout and stderr are represented
by SHA-256 digests so the artifact binds their output without embedding lengthy
logs. The benchmarked processes inherit the runner environment; only the
deliberate non-secret overrides are retained, so comparisons used for release
claims must also run on controlled identical workers.

The current `bootstrap-exit` workload is deliberately labeled
`non-equivalent`: fx uses its upstream-only `FX_BENCH=1` fast path, whereas the
machine-god bootstrap binary prints its identity. It checks the harness and
captures launch samples, but it cannot support a product performance claim.
Help, status, doctor, session-list, and background-task workloads are recorded
as `unimplemented` for machine-god and are not timed on fx in isolation. Once
both products expose matching semantics and fixtures, a later milestone must
introduce a claim-eligible schema and reviewed workload definition rather than
relabel this bootstrap evidence.
