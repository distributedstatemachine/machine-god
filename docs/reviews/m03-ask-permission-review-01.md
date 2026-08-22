# Milestone 03 native ask permission handler review 01

Status: **FEATURE REVIEW GREEN — remote and `main` gates pending**

## Reviewed lineage

- Base: `276cbf1e09139d4b6fc64f76da83e45bb30ef01b`
- Production integration: `ac28412053331b3c9e1fea2a3035f22904b28880`
- Black-box tests: `71e44788291b076f3a6e5c1969e0c33896335006`
- Candidate documentation: `bfbb76fa7d02b56aed8aa84a0815ec21ba26c3e9`
- Adversarial remediation and green feature behavior:
  `f25111fefce201a9dd3c2f581d49adeef5a07268`
- Branch: `agent/m03-ask-permission-handler`
- Toolchain: Rust and Cargo 1.94.1 exactly

The candidate adds an executor-neutral native adapter from core's existing
`PermissionHandler` contract to an explicitly injected asynchronous
`PermissionPrompter`. It adds no concrete prompt UI, CLI wiring, ambient
authority, runtime, or grant cache.

## Reviewed behavior

- `AskPermissionHandler::new` accepts an owned concrete prompter, while
  `shared_prompter` accepts an explicit `Arc<dyn PermissionPrompter>`.
- Calling `authorize` constructs an inert future. Its first poll moves the
  complete owned `PermissionRequest` into exactly one prompt future; later
  polls retain that future rather than prompting again.
- Dropping an unpolled authorization does not invoke the prompter. Dropping a
  pending authorization drops the underlying prompt future and detaches no
  adapter-owned work.
- Allow-once, allow-turn, allow-session, and deny map exactly to the existing
  core decision variants. Neither this adapter nor core caches a positive
  scope for later requests.
- Denial uses the fixed reason `permission denied`. The zero-data prompt error
  maps fail-closed to only `permission_prompt_failed` / `permission prompt
  failed`; handler debugging cannot format the injected prompter.
- Every authorization owns independent future state. The adapter contains no
  mutable request registry, blocking work, executor selection, filesystem,
  process, environment, terminal, network, configuration, or persistence
  authority.
- Core retains responsibility for validating and bounding engine-generated
  permission requests before policy. The adapter forwards that owned request
  without cloning, serializing, truncating, revalidating, or traversing it.

## Parallel implementation

Production code, independent black-box tests, and normative documentation were
implemented in separate worktrees with non-overlapping file ownership, then
cherry-picked onto the integration branch. Three fresh adversarial tracks
reviewed the exact feature commits without editing them: security/lifecycle,
API/correctness/portability, and documentation/plan/evidence.

## Adversarial rounds

### Round 1 — `bfbb76fa7d02b56aed8aa84a0815ec21ba26c3e9`

The security/lifecycle and API/correctness tracks reported **GREEN**. They
verified fail-closed mapping, request identity, scope behavior, poll/drop
ownership, absence of ambient authority or shared mutable adapter state,
object safety, Send/Sync behavior, Debug redaction, and cross-target
portability.

The documentation/evidence track found one accepted issue:

- **MEDIUM:** candidate-state wording in `ask-permission.md`, `architecture.md`,
  `security.md`, and `implementation-plan.md` said implementation and tests
  remained required even though both were already present on the exact feature
  branch. The documents now distinguish present feature-branch code/tests from
  the still-pending exact remote and `main` delivery gates.

No behavior or public API change was required.

### Round 2 — `f25111fefce201a9dd3c2f581d49adeef5a07268`

All three fresh review tracks reported **GREEN** on the remediation SHA. The
documentation reviewer confirmed the stale status was fixed across every
maintained page and that the frozen M03 ownership boundary accounts for all 22
pinned top-level commands, all planned slash-command categories, and all 26
built-in tools without silently removing scope. The other tracks confirmed the
delta was documentation-only and preserved the previously green source, tests,
security contract, and portability evidence.

No finding was rejected.

## Exact local checks

The following passed on
`f25111fefce201a9dd3c2f581d49adeef5a07268` with exact Rust/Cargo 1.94.1:

- formatting;
- locked workspace/all-target/all-feature Clippy with warnings denied;
- locked workspace tests and workspace documentation tests;
- all 9 ask-handler black-box tests, including inert construction, pending
  future drop, concurrent request distinction, exact mappings, shared trait
  objects, Send futures, and diagnostic redaction;
- repo-wide Python discovery: 129 run, 121 passed and 8 expected platform
  skips;
- `cargo-deny` dependency policy, with only the accepted duplicate `syn` and
  `windows-sys` warnings;
- `cargo-audit`: 1,225 advisories checked across 174 lockfile dependencies with
  no vulnerability finding;
- `x86_64-unknown-freebsd` no-default native-library Clippy with warnings
  denied;
- `wasm32-wasip1` no-default compilation, with only the pre-existing unrelated
  `read_file` dead-code warning;
- `aarch64-apple-darwin` no-default compilation;
- exact release CLI build and bare-binary smoke; and
- `git diff --check` and a clean worktree.

The FreeBSD all-feature attempt requires a FreeBSD C sysroot for the optional
AWS-LC dependency and is not an applicable portability gate for this
unconditional standard-library-only slice. Native Linux and macOS all-feature
coverage remains owned by remote CI.

## Pending remote gates

The exact reviewed feature SHA has not yet been pushed or exercised by remote
CI. The slice remains a candidate until the feature branch passes exact-SHA CI
and benchmark-evidence workflows, is fast-forwarded to `main`, and exact `main`
workflows pass. This section must be updated with those run IDs before the
slice is described as delivered.

## Scope

This slice does not provide a concrete terminal or graphical prompt, CLI
composition, permission modes beyond `ask`, identity-safe grant caching,
credentials, provider wiring, session commands, additional native tools, or a
compatibility or product-performance claim. Benchmark-workflow success will
validate the evidence path only. Zig remains solely a build input for the
pinned upstream fx comparison; machine-god remains a Rust product. No package
or GitHub release is authorized.
