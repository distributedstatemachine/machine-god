# Milestone 01 upstream benchmark review 01

Reviewed candidates: `f487585e7bdbfb8c82edd7c18e7725d9f7a0e556`,
`59bcdea81885895ea7c752563c31b88a4ac40422`, and
`d5713d7ce40d7f809d9ca19d5bcffbdcda7dfd69`.

Adversarial correctness, security, CI, and performance review confirmed these
findings and the branch resolves them as follows:

- Schema 2 trusted self-reported provenance. Validation now binds the artifact
  to the canonical `benchmarks/upstream.lock` path, bytes, repository, commit,
  and Zig version; exact release profiles, build commands, output paths, and
  measurement commands are structural requirements.
- Binary metadata was not rechecked. The checker now requires both built
  executables, resolves their paths, and recomputes file size and SHA-256 before
  accepting evidence.
- Upstream worktrees and ignored compiler caches could be reused. Each run now
  requires nonexistent checkout and scratch paths, disables Git hooks and
  system/global configuration, forbids local and `ext` transports, and verifies
  a clean detached checkout including ignored files before building.
- Machine-god's cleanliness check ignored untracked inputs. It now inspects
  tracked, untracked, and ignored state and permits only the `.bench`,
  `benchmarks/results`, and `target` output trees.
- External commands had no wall-clock bounds. Fetch/tool, build, and per-sample
  timeouts are explicit, configurable, recorded, and terminate the spawned
  process group. Evidence is written atomically only after validation, and a
  failed run removes stale output at the requested path.
- Builds and measurements inherited ambient flags, configuration, loader
  injection, and caches. They now receive recorded allowlisted environments,
  fresh Cargo and Zig build caches, a fresh home and temporary directory, and no
  ambient Rust flags, Cargo profile overrides, or loader variables.
- The schema did not completely bind build caches and Rust tool state. It now
  requires the machine build's `CARGO_HOME` and `RUSTUP_HOME` to equal the
  verified tool environment, and derives and validates every home, temporary,
  source, manifest, target, Cargo, and Zig cache path from the fresh scratch
  directory. Hostile substitutions for each path are rejected.
- The machine build still read its source from the developer worktree after the
  recorded revision was checked, leaving a time-of-check/time-of-use gap. The
  harness now materializes the recorded Git object tree into scratch, records a
  canonical manifest and source-tree checksum, and builds and measures only that snapshot.
  Both the collector and checker re-hash the materialized source. A regression
  test mutates the original worktree during a build-like read and confirms that
  the recorded source remains the only input.
- Timeout cleanup could wait forever when a detached descendant retained a
  captured pipe. All cleanup waits and pipe closure are now bounded, and a
  regression covers a detached pipe holder.
- Environment-token discovery remained fail-open because a hostile child could
  clear its environment or make `/proc/*/environ` unreadable. Linux containment
  now combines unconditional original-process-group termination with a verified
  child-subreaper, pidfds, parent-relationship tracking, and explicit reaping.
  A hostile double-fork preflight must succeed before commands launch; it and the
  regressions clear the token and do not read descendant environments.
- Post-command containment scans were included in elapsed samples. The end
  timestamp is now taken immediately after `communicate` returns; a deliberately
  delayed scan regression proves the recorded sample excludes cleanup overhead.
- Successful-command finalization ignored known zombies, allowing a short-lived
  detached grandchild to remain adopted after the run was accepted. A bounded
  settle pass now repeatedly discovers and reaps all known adopted children and
  asserts that no descendant PID or zombie remains, without moving the timing
  boundary. A Linux-only close-pipes/double-fork regression exercises this path.
- Exceptions raised after `Popen`, including supervision and finalization
  failures, could bypass cleanup. The supervisor is now created before launch,
  and every later exception enters a non-throwing bounded cleanup that kills the
  process group and known pidfds, closes pipes, reaps, and stops the monitor.
  Injected constructor, monitor, and finalizer failures verify that no hostile
  child survives.
- Ancestry expansion seeded numeric PIDs from old records without rechecking
  process start time, so PID reuse could attach unrelated descendants. Root and
  discovered identities are immutable `(PID, start_time)` pairs, expansion uses
  only currently matching pairs, and signaling/reaping uses pidfds. A synthetic
  reuse regression confirms that neither a recycled root nor parent seeds a
  child.
- Git archives honored committed export attributes, so the recorded Git tree did
  not uniquely determine materialized inputs. Materialization now uses canonical
  `ls-tree` and `cat-file` operations, rejects links and special modes, and binds
  every file path, Git mode, object ID, byte count, and digest. Hostile
  `export-ignore` and `export-subst` attributes cannot alter the snapshot.
- Resolved tool paths could change after version checks. Each absolute invocation
  path and its canonical target are now recorded, preserving Rustup proxy
  dispatch while binding both filesystem identities and target content before
  and after every use. Regressions swap a discovery symlink and mutate its former
  target during execution.
- Host provenance did not identify the processor or CI image class. Evidence now
  includes CPU model, runner architecture, CI image identity, and an explicit
  runner class that the validator binds consistently.
- The pinned-upstream harness was not exercised by CI. The benchmark workflow
  now runs on feature branches and main, installs Zig 0.16.0 through an immutable
  action commit, checks the exact workflow SHA and both binaries, and retains the
  schema 2 artifact under a runner-specific name.

Hostile tests cover lock/revision substitution, altered build profiles and
commands, binary/measurement path substitution, post-recording binary changes,
ambient compiler flags, runner-class changes, ignored configuration inputs,
preexisting upstream checkouts, every scratch/cache/tool-state path, original
worktree mutation after snapshotting, process-group and detached-descendant
timeouts, Linux descendant containment without readable environments, delayed
containment scans, Git export attributes and link modes, mutable tool paths, and
successful-command zombie reaping, injected lifecycle failures, PID reuse, and
stale evidence.

Local end-to-end validation used the checksum-verified Zig 0.16.0 macOS aarch64
toolchain and a new isolated checkout. Clone, exact detached checkout, origin,
and disabled-hook verification succeeded, but this host terminated the fx
ReleaseSafe compilation before it emitted a binary. The harness correctly left
no evidence file. The pinned Ubuntu CI job is therefore the remaining
environment for complete fx build and artifact validation.

No findings were rejected.
