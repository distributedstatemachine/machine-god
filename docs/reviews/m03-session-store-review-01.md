# Milestone 03 native file session store review 01

Status: **DELIVERY GREEN — integrated on `main`**

## Reviewed lineage

- Base: `508b0adbbe4447a85bd08f47095ae16c089c05d5`
- Atomic feature: `c4b52c1f2a33af22eb3b59d7b54594753ebc5694`
- Adversarial remediation: `2fa3fa74891189901affb10f8e0adc6f1c1f45f9`
- Feature-branch evidence: `3f9496ee4b6716a0b607cd492f4c8427cc43b3c6`
- Documentation seal and `main` delivery:
  `8f7b47db9580b14570bf9fb55763858f71a81271`
- Branch: `agent/m03-native-session-store`
- Toolchain: Rust and Cargo 1.94.1 exactly
- Pinned reference: fx revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`

The integrated slice adds a bounded Linux/macOS file implementation of core's
injected `SessionStore` boundary. The host explicitly selects and prepares the
root. The store retains a no-follow directory descriptor, derives fixed child
names from a domain-separated session-ID digest, strictly validates bounded
schema-v1 records, and performs per-record optimistic updates under a permanent
advisory lock. Core retains no ambient filesystem authority and the CLI gains
no session command or wiring.

## Reviewed behavior

- Construction accepts only an existing absolute directory on Linux and macOS,
  retains the opened descriptor, performs no environment discovery or root
  creation, and returns a fixed unsupported-platform error elsewhere.
- Session IDs map to fixed SHA-256-derived record, lock, and temporary names.
  Every loaded record is checked against the requested ID; hashing is a naming
  measure, not encryption or confinement.
- Records use a strict compact schema-v1 envelope capped at 8,651,165 bytes.
  Typed duplicate, unknown, missing, malformed, zero-counter, wrong-version,
  wrong-ID, over-depth, and over-node state fails closed.
- Missing loads create no artifacts. For present loads and structurally valid
  saves, after the permanent sidecar is safely opened, verified, and exclusively
  locked, its advisory lock is held through record reads or filesystem/CAS
  processing. Record and temporary-file access uses bounded no-follow,
  nonblocking, post-open-verified regular files.
- Save implements atomic compare-and-swap for cooperating processes, assigns a
  checked revision greater than both stored and candidate revisions, preserves
  incarnation, and never publishes a partial record.
- Temporary files are exclusively created and forced to exact mode `0600`,
  synchronized, renamed within the retained directory, and followed by a
  directory sync. A post-rename sync failure is explicitly ambiguous and not
  safely retryable.
- Stale regular temporary files are recovered under lock. Symlink, directory,
  FIFO, socket, device, and other nonregular artifacts are preserved and fail
  closed through the fixed redacted taxonomy.
- Futures are inert before polling and perform bounded-data synchronous I/O on
  their polling thread. Advisory-lock waits and interrupted retries can take
  unbounded wall time; the host must isolate them from latency-sensitive async
  executor threads.

## Parallel implementation

Production code, black-box and engine-integration tests, and normative
documentation were developed by agents with non-overlapping ownership and
integrated on the feature branch. Fresh correctness/concurrency,
security/abuse, and API/documentation agents reviewed exact commits without
editing them. Accepted findings were fixed and all three tracks independently
returned green on the remediation SHA.

## Adversarial rounds

### Round 1 — `c4b52c1f2a33af22eb3b59d7b54594753ebc5694`

Accepted findings:

- **MEDIUM:** the normative contract said interrupted I/O was retried, while
  temporary-file and directory `fsync` calls did not retry `EINTR`. A common
  retry helper now covers advisory locking, byte transfers, and both sync
  boundaries. A focused regression proves repeated interruption is retried and
  other errors return immediately; the documentation now precisely limits the
  retry statement and discloses unbounded retry duration.
- **MEDIUM:** unsupported targets acquired new dead-code and unused-import
  warnings, causing warnings-denied cross-target Clippy to fail. Unix-only
  implementation details are now cfg-gated while the portable unsupported API
  remains available. Exact FreeBSD warnings-denied Clippy and WebAssembly
  checks pass.

No reviewer found a remaining correctness, concurrency, confinement,
resource-bound, schema, API, or CLI-scope defect in the behavior after these
remediations.

### Round 2 — `2fa3fa74891189901affb10f8e0adc6f1c1f45f9`

All three independent reviewers reported **GREEN**. They confirmed compare-and-
swap and revision behavior, schema and byte bounds, stack-safe JSON handling,
descriptor-relative no-follow confinement, artifact classification, exact
temporary-file mode, interrupted-sync handling, unsupported-target portability,
fixed diagnostics, API/docs consistency, and unchanged CLI and Zig scope.

### Documentation seal — `9f4ec99e3f891d32223161c0d68e36b7f5594638`

Accepted findings:

- **MEDIUM:** `core-api.md` retained the pre-review status and incorrectly said
  adversarial review was pending. It now links this evidence and distinguishes
  the green behavior/local gates from the pending feature-branch remote and
  `main` gates.
- **LOW:** the initial reviewed-behavior summary said all saves acquire the
  advisory lock, although candidates can fail before or during sidecar open,
  verification, or lock acquisition. It now conditions the held-lock claim on
  successful acquisition and states the protected processing interval.

Both findings concern evidence precision only and do not change product code or
the adversarially green behavior SHA.

## Exact local checks

The following passed on the adversarially green behavior SHA with exact
Rust/Cargo 1.94.1:

- formatting;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace/all-target/all-feature tests and workspace documentation tests;
- 30 native library tests, 19 black-box file-store tests, and 2 store/engine
  integration tests, including process-level compare-and-swap, hostile artifact,
  exact-mode, byte-cap, strict-schema, and interruption regressions;
- repo-wide Python discovery: 129 run, 121 passed and 8 expected platform
  skips;
- `cargo-deny` dependency policy, with only the accepted duplicate `syn` and
  `windows-sys` warnings;
- `cargo-audit`: 1,225 advisories checked across 174 lockfile dependencies with
  no vulnerability finding;
- `x86_64-unknown-freebsd` warnings-denied Clippy for the native crate;
- `wasm32-wasip1` all-feature compilation, with only the pre-existing unrelated
  `read_file::check_cancellation` dead-code warning;
- pinned upstream compatibility generation against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact release build and bare CLI smoke; and
- `git diff --check` and a clean worktree.

The local release CLI remains 319,152 bytes. This is a local regression
observation only, not retained benchmark evidence or a product-performance
claim.

## Exact remote feature-branch runs

The exact feature-branch evidence SHA
`3f9496ee4b6716a0b607cd492f4c8427cc43b3c6` passed both required remote
workflows:

- [CI run 32540570112](https://github.com/distributedstatemachine/machine-god/actions/runs/32540570112)
  completed successfully, including quality/tests, dependency policy and
  vulnerability audit, and all four native target jobs; and
- [Benchmark evidence run 32540570125](https://github.com/distributedstatemachine/machine-god/actions/runs/32540570125)
  completed successfully for both pinned-upstream and bootstrap evidence jobs.

Benchmark-workflow success validates this repository's evidence path. It does
not promote the local observation above into a product-performance or
compatibility claim.

## Exact remote documentation-seal runs

The exact documentation-seal SHA
`8f7b47db9580b14570bf9fb55763858f71a81271` passed both required remote
feature-branch workflows:

- [CI run 32541009906](https://github.com/distributedstatemachine/machine-god/actions/runs/32541009906)
  completed successfully for all six jobs; and
- [Benchmark evidence run 32541009865](https://github.com/distributedstatemachine/machine-god/actions/runs/32541009865)
  completed successfully for both evidence jobs.

## Exact remote `main` runs

`main` was fast-forwarded without a merge commit to exact SHA
`8f7b47db9580b14570bf9fb55763858f71a81271`. Its independently triggered
required workflows passed:

- [CI run 32541315998](https://github.com/distributedstatemachine/machine-god/actions/runs/32541315998)
  completed successfully for quality/tests, dependency policy and audit, and
  all four native target jobs; and
- [Benchmark evidence run 32541315997](https://github.com/distributedstatemachine/machine-god/actions/runs/32541315997)
  completed successfully for pinned-upstream and bootstrap evidence.

## Delivery status and scope

The exact behavior, feature evidence, documentation seal, and fast-forwarded
`main` delivery are green. Benchmark-workflow success validates the evidence
path but does not establish a product-performance or compatibility claim. The
workflow uses Zig only to build the pinned upstream fx comparison target;
machine-god remains a Rust product.

Milestone 03 remains in progress. This slice does not add credential discovery,
provider/CLI wiring, permission prompting, session listing/reset/deletion,
migration, encryption, non-Unix hardening, a compatibility claim, or a measured
product-performance claim. No package or GitHub release is authorized.
