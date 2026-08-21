# Milestone 03 confined native `list_files` review 01

Status: **ADVERSARIAL GREEN — exact remote rerun pending**

## Candidate

- Base: `472e10c622ed2183c3e488d930529d7391c0ded7`
- Atomic feature: `80e4db6a7151e5bec4575b2f556f7ac5a0c2ac42`
- Platform/error remediation: `d1e961d30165fb32bb666ee379a3899dbba95660`
- Adversarially green behavior: `97c52c72f8080610cf4a70563df70f547bbdf75c`
- Branch: `agent/m03-list-files-reviewed`
- Toolchain: Rust and Cargo 1.94.1 exactly

The candidate adds one bounded library-level native tool together with its
tests, implementation plan, architecture, security contract, and API guide.
It does not wire the CLI to an engine, add a provider or production permission
handler, complete Milestone 03, change the pinned fx inventory or Zig benchmark
setup, or make a compatibility or product-performance claim.

## Reviewed behavior

- `ListFilesTool::open` accepts an explicitly injected absolute workspace root.
  On supported Linux and macOS targets it retains a directory descriptor and
  returns only fixed, redacted construction errors. Other targets return the
  stable unsupported-platform category without acquiring filesystem authority.
- Root construction removes redundant separators and `.` components before its
  final no-follow open. Model-selected traversal starts only from that retained
  descriptor and opens every component as a no-follow directory, so selected
  component symlinks and non-directories fail closed.
- Effect-free preflight accepts exactly `{}` or a sole string `path`, defaults
  omission to `.`, bounds input to 4,096 UTF-8 bytes, rejects rooted paths,
  `..`, control and bidirectional-formatting characters, and places the same
  normalized path in the filesystem-enumerate capability and execution args.
- Allowed execution enumerates one level without recursively opening children,
  reading contents, following entry symlinks, applying ignore rules, or
  discovering a workspace. Entry kinds come from the directory record and are
  fixed to `file`, `directory`, `symlink`, or `other`.
- Every observed visible name must be safe UTF-8. The retained subset is bounded
  to 100 entries and 16 KiB of aggregate name bytes. Only that subset is sorted;
  one additional native-order entry establishes truncation, and the contract
  makes no global ordering or snapshot claim.
- The independent worst-case calculation is exact: structured content is at
  most 44,101 serialized bytes and the complete `ToolOutput` is at most 44,130
  bytes, below core's default 64 KiB result bound.
- Direct execution checks cooperative cancellation around traversal and entry
  reads. Constructor, preparation, traversal, enumeration, name-validation,
  and cancellation failures use the documented fixed redacted taxonomy.
- Real-engine tests prove exact normalized capability delivery, denial before
  enumeration, allow before execution, exact durable success, and preparation
  failure before policy or filesystem access.

## Parallel implementation

Three isolated worktrees owned non-overlapping production, black-box/engine
test, and documentation surfaces. Their staging changes were combined into one
atomic feature commit so behavior and documentation landed together. Three
fresh read-only reviewers then covered filesystem confinement and errors,
API/tests/documentation contracts, and performance/portability/scope.

## Adversarial rounds

### Round 1 — `80e4db6a7151e5bec4575b2f556f7ac5a0c2ac42`

- **MEDIUM — accepted:** `Dir::read_from` can return `EACCES` or `EPERM` after
  access to a retained root is revoked, but every stream-construction failure
  was classified as retryable `list_files_read_failed`. The resolution maps
  those access errors to the documented nonretryable
  `list_files_permission_denied` result and adds a mode-enforcement regression.
- **MEDIUM — accepted:** implementation cfgs selected every Rust `unix` target,
  while rustix does not expose `DirEntry::file_type` on every such platform.
  The resolution selects the documented Linux/macOS targets consistently and
  routes every other target through the stable unsupported implementation.
- The performance/portability/scope reviewer otherwise reported GREEN after
  independently checking bounds, rustix directory-stream semantics, declared
  target APIs, truncated-order wording, and absence of CLI, dependency,
  workflow, benchmark, pinned-fx, or Zig drift.

### Round 2 — `d1e961d30165fb32bb666ee379a3899dbba95660`

The security and contract reviewers reported GREEN on the access-error and
platform remediations. The cross-target reviewer found one remaining issue:

- **LOW — accepted:** `check_cancellation` remained unconditional and was dead
  code on the unsupported path, so exact FreeBSD Clippy with warnings denied
  failed even though ordinary fallback compilation succeeded. The helper now
  uses the same Linux/macOS cfg as its callers.

### Round 3 — `97c52c72f8080610cf4a70563df70f547bbdf75c`

All three final-seal reviewers reported GREEN. They confirmed exact native and
unsupported cfg symmetry, the fixed permission taxonomy and redaction,
unchanged confinement/cancellation/resource behavior, 26 focused tests, native
warnings-denied Clippy, and warnings-denied FreeBSD fallback compilation.

## Exact local checks

The following passed on the adversarially green behavior SHA with exact
Rust/Cargo 1.94.1:

- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace tests: 262 top-level tests plus 15 deep-JSON child-process probes;
- workspace documentation tests: 2;
- focused `list_files` evidence: 23 direct and 3 real-engine tests;
- warnings-denied `machine-god-native` library Clippy for
  `x86_64-unknown-freebsd`, proving the unsupported fallback;
- repo-wide Python discovery: 129 run, comprising 121 passed and 8 expected
  platform skips;
- the pinned upstream compatibility inventory check against exact fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact release build and bare/help/status JSON CLI smoke checks;
- `cargo-deny` 0.20.2: advisories, bans, licenses, and sources all accepted;
- `cargo-audit` 0.22.2 with `--no-fetch`: 1,225 cached advisories checked across
  39 lockfile dependencies with no finding;
- relative documentation links: 54 checked; and
- `git diff --check` and a clean worktree.

The stripped local release CLI remains 319,152 bytes. This is a local regression
observation only, not retained cross-platform benchmark evidence or a product
performance claim.

## Remaining gates and scope

The sealed branch and its eventual fast-forwarded `main` SHA must each pass
fresh exact remote CI and benchmark-evidence workflows. The benchmark workflow
continues to use Zig only to build the pinned upstream fx comparison target;
machine-god remains a Rust product.

Milestone 03 remains in progress. Concrete providers, a production permission
prompt and modes beyond `ask`, durable native sessions, broader CLI behavior,
remaining native tools, non-Linux/macOS hardened filesystem execution, and
compatibility or product-performance claims remain planned. No package or
GitHub release is authorized.
