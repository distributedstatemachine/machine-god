# Milestone 01 upstream benchmark review 01

Reviewed candidates: `f487585e7bdbfb8c82edd7c18e7725d9f7a0e556`
and `59bcdea81885895ea7c752563c31b88a4ac40422`.

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
  source, archive, target, Cargo, and Zig cache path from the fresh scratch
  directory. Hostile substitutions for each path are rejected.
- The machine build still read its source from the developer worktree after the
  recorded revision was checked, leaving a time-of-check/time-of-use gap. The
  harness now creates a fresh Git archive of the recorded object ID, safely
  extracts it into scratch, records its Git tree, archive checksum, and canonical
  source-tree checksum, and builds and measures only that immutable snapshot.
  Both the collector and checker re-hash the materialized source. A regression
  test mutates the original worktree during a build-like read and confirms that
  the recorded source remains the only input.
- Timeout cleanup could wait forever when a detached descendant retained a
  captured pipe. All cleanup waits and pipe closure are now bounded. Linux CI
  gives each run a random inherited token, discovers same-user descendants via
  `/proc` even after `setsid` or a double fork, stops and kills them, and fails
  closed if that containment facility is unavailable or a supposedly successful
  command leaks a descendant. Regressions cover both a detached pipe holder and
  Linux token containment.
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
timeouts, Linux descendant containment, and stale evidence.

Local end-to-end validation used the checksum-verified Zig 0.16.0 macOS aarch64
toolchain and a new isolated checkout. Clone, exact detached checkout, origin,
and disabled-hook verification succeeded, but this host terminated the fx
ReleaseSafe compilation before it emitted a binary. The harness correctly left
no evidence file. The pinned Ubuntu CI job is therefore the remaining
environment for complete fx build and artifact validation.

No findings were rejected.
