# Milestone 03 confined native `read_file` review 01

Status: **ADVERSARIAL GREEN — integrated exact remote rerun pending**

## Candidate

- Base: `8c4840dd829688a3755fe6b6f23ab92dbc2ade84`
- Adversarially green implementation: `3378d2b5c87980c1cbe8f6f352c754d7fd98a462`
- Integrated CI remediation: `4bd039750d792e3c30cbc4d3c1909c644edfb673`
- Branch: `agent/m03-read-file-reviewed`
- Toolchain: Rust and Cargo 1.94.1 exactly

The candidate is one conventional feature commit containing its behavior,
tests, implementation plan, architecture, security contract, and API guide. It
adds the first executable native tool without changing the CLI, adding a model
provider or production permission handler, completing Milestone 03, changing
the pinned fx inventory or Zig benchmark setup, or making a product performance
claim.

## Reviewed behavior

- `ReadFileTool::open` accepts an explicitly injected absolute workspace root.
  On supported Unix targets it retains a directory descriptor and returns only
  fixed, redacted construction errors.
- Root construction rebuilds the lexical path without redundant separators or
  `.` components before its final no-follow open. Decorated final symlink forms
  such as `/link/`, `/link//`, and `/link/.` therefore reject, while real
  directory forms and `/` remain valid. Root ancestors and subordinate mounts
  remain an explicit trusted-host boundary.
- The strict model-visible input is exactly one string `path`, bounded to 4,096
  UTF-8 bytes. Effect-free preflight removes `.` and repeated separators,
  rejects rooted paths, `..`, NUL and display-control characters, and returns
  the same normalized workspace-relative path in the filesystem-read
  capability and prepared execution arguments.
- Allowed execution revalidates those exact arguments and walks from the
  retained root with descriptor-relative, nonblocking, no-follow opens. Every
  ancestor must be a directory and the final descriptor must authoritatively
  be a regular file.
- Reads retain at most 8 KiB plus one overflow-detection byte, reject growth or
  initial oversize without truncation, require complete valid UTF-8, and return
  exactly `{"content":"..."}` on success.
- Direct execution checks cancellation before traversal, between component
  opens and bounded reads, and after validation. It makes no claim to preempt
  an in-flight syscall. Shared engine cancellation retains core's cancellation
  precedence and durable unknown placeholder.
- Constructor and tool failure kinds, codes, messages, retryability, `Display`,
  and redaction are documented as a complete fixed taxonomy.
- Real-engine tests prove normalized capability delivery, deny without content
  exposure or tool lifecycle events, execution only after allow, exact durable
  successful content, and preparation failure before policy or filesystem
  execution.

## Parallel implementation

Three initial isolated worktrees owned non-overlapping production/dependency,
black-box test, and documentation surfaces. Review remediation used separate
root-confinement, engine-evidence, and documentation worktrees. The staging
commits were combined into the single implementation commit above so behavior
and its documentation satisfy the repository's same-commit rule.

The production uses safe `rustix` 1.1.4 filesystem APIs. `rustix` is a
Unix-targeted dependency of `machine-god-native`; core receives no filesystem
dependency or ambient authority. The local `machine-god-testkit` dependency is
dev-only and includes both its path and explicit `0.1.0` version, so dependency
policy does not treat it as a wildcard.

## Adversarial rounds

### Round 1 — staging candidate `2e8ede05c13e3bee2e58b5ea2d58647a3f27a69a`

Three independent read-only reviewers covered filesystem security and races,
cross-crate API and evidence, and documentation and scope.

- **HIGH/MEDIUM — accepted:** passing a root ending in `/` directly to
  `open(..., O_DIRECTORY | O_NOFOLLOW)` allowed Linux and macOS to traverse a
  final symlink before no-follow applied. The same problem covered terminal
  `/.`, and the original test exercised only an undecorated symlink.
- **MEDIUM — accepted:** the concrete native tests called `prepare` and
  `execute` directly but did not compose a real `ReadFileTool` with `Engine`, a
  provider, policy, and store across deny, allow, and preparation-error paths.
- **MEDIUM — accepted:** the cancellation guide incorrectly grouped shared
  engine cancellation with tool errors reduced to a generic durable result.
  Core instead gives its shared cancellation token precedence and retains the
  unknown placeholder.
- **MEDIUM — accepted:** staging history separated behavior and documentation,
  contrary to the repository's same-commit rule.
- **LOW — accepted:** the guide claimed platform-prefixed paths reject, while
  supported Unix treats backslashes and Windows-looking strings as confined
  literal filenames.
- **LOW — accepted:** the normative guide called every error field fixed but
  did not enumerate the complete construction and tool error taxonomies.

The resolution rebuilds the root's lexical components before its final
no-follow open; adds decorated-symlink, real-root, filesystem-root, and Unix
backslash regressions; adds three deterministic real-engine composition tests;
corrects cancellation and Unix path wording; enumerates every public error; and
combines behavior and documentation into one feature commit.

### Round 2 — `e0bbdd419e3f3107fc95624b851ff4346daa9f58`

All three reviewers reported GREEN. They verified the root-symlink regression,
descriptor confinement, file-kind and resource bounds, cancellation and
redaction, strict capability/execution equality, real-engine evidence,
deterministic fixtures, dependency shape, documentation accuracy, scope
isolation, and the single-commit requirement.

The exact dependency-policy refresh then found one wildcard: the local testkit
dev-dependency had a path but no version. Adding explicit version `0.1.0`
changed only one manifest line and made every `cargo-deny` category pass.

### Round 3 — `3378d2b5c87980c1cbe8f6f352c754d7fd98a462`

All three fresh final-seal reviewers reported GREEN. They confirmed that the
round-2-green implementation and docs were otherwise byte-identical, the
versioned path dependency resolves to the local unpublished testkit only in
native's dev graph, no cycle or runtime dependency was introduced, all previous
security and API invariants remain intact, dependency policy is green, and the
CLI, compatibility, benchmark, performance-claim, workflow, and Zig surfaces
remain unchanged.

## Exact local checks

The following passed on the adversarially green implementation SHA using exact
Rust/Cargo 1.94.1:

- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace/all-target/all-feature tests: 234 top-level tests, plus 15
  deep-JSON child-process probes;
- workspace documentation tests: 2;
- focused native evidence: 26 unit, 18 configuration, 19 direct `read_file`,
  and 3 real-engine `read_file` tests;
- repo-wide Python discovery: 129 run, comprising 121 passed and 8 expected
  platform skips;
- the pinned upstream compatibility inventory check against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- release build and bare/help/status JSON CLI smoke checks;
- `cargo-deny check`: advisories, bans, licenses, and sources all accepted;
- `cargo audit --no-fetch`: 1,225 cached advisories checked across 39 lockfile
  dependencies with no finding;
- relative documentation links: 43 checked; and
- `git diff --check` and a clean worktree.

The stripped local release CLI remains 319,152 bytes. This is a local
regression observation only, not retained cross-platform benchmark evidence or
a product performance claim.

## Remaining gates and scope

The first sealed feature SHA, `5bf772fa271c348ff0b3d36cac81c5240c6645c1`,
passed its exact benchmark-evidence workflow but exposed a Linux-only empty PID
marker race in the repository's Python benchmark harness during CI. The
`read_file` implementation and all Rust/native jobs passed. The bounded harness
fix, deterministic regression isolation, and fresh adversarial review are
recorded in the [containment marker remediation review](m03-benchmark-containment-marker-review-01.md).
The integrated branch and its eventual fast-forwarded `main` SHA must still
pass fresh exact remote CI and benchmark-evidence workflows. The benchmark
workflow continues to use Zig only to build the pinned upstream fx comparison
target; machine-god remains a Rust product.

Milestone 03 remains in progress. Concrete providers, a production permission
prompt and modes beyond `ask`, durable native sessions, broader CLI behavior,
remaining native tools, non-Unix hardened filesystem execution, and
compatibility or product performance claims remain planned. No package or
GitHub release is authorized.
