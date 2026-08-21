# Milestone 03 bounded native configuration loading review 01

Status: **ADVERSARIAL GREEN — exact remote CI pending**

## Candidate

- Base: `8abc233d5d9e5a4585882a938b9693df097be8c6`
- Adversarially green candidate: `d4de949f9e29373e394bed3162c3d2101063af1a`
- Configuration implementation: `c06b5160b5a837a78c6cc588fd322591def05da9`
- Branch: `agent/m03-native-config-load`
- Toolchain: Rust and Cargo 1.94.1 exactly

The candidate adds a synchronous, read-only native configuration loader. It
does not complete Milestone 03 and does not change the CLI, core engine,
testkit, compatibility inventory, benchmark workloads, or product performance
claims.

## Reviewed behavior

- Resolve only the existing machine-god config location from an owned
  `NativeEnvironment` snapshot.
- Return explicit schema-v1, permission-mode `ask` built-in defaults when the
  location or resolved file is absent.
- Fail closed for an invalid selected environment root, inaccessible file,
  final symlink, non-regular file, input above 64 KiB, invalid UTF-8, malformed
  or trailing JSON, invalid schema shape, unsupported permission mode, or
  unsupported schema version.
- Retain at most the 64 KiB limit plus one byte while detecting growth.
- On supported Unix targets, open the final component no-follow and
  nonblocking, then authoritatively validate the opened descriptor as regular.
- Keep public error diagnostics independent of selected paths, file contents,
  and operating-system error strings.
- Never create, write, canonicalize, start the engine, or reread the process
  environment when an injected snapshot is supplied.

The exact schema-v1 file is:

```json
{"schema_version":1,"permission_mode":"ask"}
```

## Parallel implementation

Three isolated worktrees owned non-overlapping changes:

- runtime API, bounded file handling, parsing, and manifests;
- black-box native integration tests; and
- configuration, architecture, security, and implementation-plan docs.

Their commits were integrated without squashing so ownership remains auditable:

- `782435f` — configuration documentation and plan;
- `6609f07` — native loader runtime;
- `cfceb81` — black-box native tests; and
- `9764367` — documentation error-taxonomy correction from coordinator review.

## Adversarial rounds

### Round 1 — `9764367fd0eea96a6cb15fd1bf782f6c5776def4`

Three read-only reviews covered API/schema correctness,
filesystem/resource/concurrency safety, and docs/tests/evidence boundaries.

- **MEDIUM — accepted:** `UnsupportedSchemaVersion` was reached only after the
  complete schema-v1 body had deserialized as v1. A realistic future schema
  with new fields or modes, and an integer outside `u64`, was reported as
  `InvalidFormat`.
- **MEDIUM — accepted:** documentation stated final-component no-follow and
  nonblocking behavior without scoping it to Unix, although those atomic open
  flags are Unix-only. It also failed to acknowledge the preliminary pathname
  kind check before authoritative descriptor validation.
- The filesystem/resource reviewer otherwise reported GREEN.

Resolution `e25c6551480775a17a586750039889d18f2c7fab` added a borrowed raw
schema-version envelope, classified arbitrary-size signed integer versions
before v1-only fields, retained strict v1 parsing, added public and private
regressions, and corrected the platform wording. Hardened non-Unix open
semantics remain explicitly deferred.

### Round 2 — `e25c6551480775a17a586750039889d18f2c7fab`

- The filesystem/resource and docs/tests/evidence reviewers reported GREEN.
- **MEDIUM — accepted:** Serde's ignored-value path for unknown future fields
  could skip invalid UTF-8. A schema-v2 document containing invalid bytes in an
  ignored string was therefore reported as `UnsupportedSchemaVersion`, contrary
  to the documented `InvalidFormat` precedence.

Resolution `c06b5160b5a837a78c6cc588fd322591def05da9` added an
allocation-free full-buffer UTF-8 validation before version dispatch and added
private and black-box regressions.

### Round 3 — `c06b5160b5a837a78c6cc588fd322591def05da9`

All three reviewers reported GREEN with no actionable findings:

- API/schema: future fields and modes, arbitrary-size signed integer versions,
  duplicate version keys, wrong types, floats/exponents, trailing bytes,
  malformed JSON, deep future input, v1 strictness, and invalid UTF-8
  precedence;
- filesystem/resources: environment selection, symlink and replacement races,
  Unix open flags, special files, descriptor validation, bounded growth reads,
  redaction, cleanup, and absence of writes; and
- docs/evidence: platform scoping, public-contract accuracy, dependency policy,
  CLI byte stability, plan boundaries, and absence of compatibility or
  performance overclaims.

### Seal follow-up — `c73831ef83b5a102f2ab55be9e6ea3906cf5511f`

The documentation seal corrected the Python result accounting to 129 tests
run, comprising 121 passed and 8 expected skips. During that exact-SHA seal, a
reviewer also reproduced a pre-existing scheduling race in
`test_replacement_during_sample_cannot_change_executed_bytes`: the replacement
thread could swap the public invocation path before the measured pinned child
had started. This was accepted as a test-reliability finding even though the
configuration implementation and benchmark workload were unchanged.

Resolution `a6a321905ca51c1c998bb451930a1ca533361a6a` replaced the fixed 50 ms
delay with a child-written startup marker. The replacement thread now waits a
bounded three seconds for pinned execution to begin, while the existing
post-sample identity check and `good` result assertion continue to prove that
the sampled pinned bytes ran and that the public path was replaced.

### Round 4 — `a6a321905ca51c1c998bb451930a1ca533361a6a`

- The runtime reviewer reported GREEN.
- **MEDIUM — accepted:** startup and result marker paths were interpolated into
  POSIX shell text without shell quoting. A temporary-directory path containing
  whitespace or shell metacharacters could therefore make the regression fail
  spuriously or create a stray file.

Resolution `d4de949f9e29373e394bed3162c3d2101063af1a` applies POSIX
`shlex.quote` to both paths in the Linux `sh -c` command and the generated
non-Linux POSIX script.

### Round 5 — `d4de949f9e29373e394bed3162c3d2101063af1a`

All three reviewers reported GREEN with no actionable findings. They verified
the bounded startup handshake, process and thread cleanup, the retained pinned
byte and identity-swap assertions, Linux and non-Linux POSIX quoting, and scope
isolation from product code, manifests, evidence, workloads, and claims. The
focused regression passed 10 consecutive runs in a normal environment and 10
more with a temporary-directory path containing spaces, a single quote, a
dollar sign, and a semicolon. Two parallel full Python-suite runs also passed
during the synchronization review.

## Exact local checks

The following passed on the adversarially green candidate SHA using exact
Rust/Cargo 1.94.1:

- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace/all-target/all-feature tests: 193 top-level tests, including 24
  native unit and 18 native black-box configuration tests;
- workspace documentation tests: 2;
- repo-wide Python discovery: 129 run, comprising 121 passed and 8 expected
  platform skips;
- release build and bare/help/status JSON CLI smoke checks;
- `cargo-deny check`: advisories, bans, licenses, and sources all accepted;
- `cargo audit --no-fetch`: 1,225 cached advisories checked across 33 lockfile
  dependencies with no finding;
- relative documentation links: 33 checked; and
- `git diff --check` and a clean worktree.

The stripped local release CLI remained 319,152 bytes. This is a local
regression observation only, not retained cross-platform benchmark evidence or
a product performance claim.

## Remaining gates and scope

The feature branch and its eventual fast-forwarded `main` SHA must still pass
their exact remote CI and benchmark-evidence workflows. The benchmark workflow
continues to use Zig only to build the pinned upstream fx comparison target;
the machine-god product remains Rust.

Configuration mutation, modes beyond `ask`, permission prompting, concrete
providers and executable tools, durable native sessions, broader CLI behavior,
and compatibility/performance claims remain planned. Milestone 03 therefore
remains `IN PROGRESS`.
