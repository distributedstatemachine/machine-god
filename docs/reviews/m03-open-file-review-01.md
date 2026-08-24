# Milestone 03 native `open_file` review 01

Status: **CONTRACT FROZEN; IMPLEMENTATION PENDING**

## Base and boundary

- Exact delivered base:
  `e2ee11f2c728721d2aa93219b5fafa86ea15b0c4`.
- Integration branch: `agent/m03-open-file`.
- Normative contract: [`open-file.md`](../open-file.md).
- Base main CI `32704202572` is green.
- Base main benchmark `32704202546` is green in both jobs and retains exactly
  two nonexpired exact-SHA artifacts, IDs `9511626648` and `9511745538`.

This documentation-only contract checkpoint is exempt from adversarial review
under the user's explicit instruction. Its own exact feature CI and benchmark
workflows are required after push and cannot be self-recorded. It is not
implementation, behavior, delivery, performance, or fx-equivalence evidence.

## Frozen feature

The twenty-sixth bounded slice asks one fixed Linux desktop helper to open one
strict canonical, workspace-confined, existing regular file. It freezes the
sole `{path:string}` input, exact path/JSON/result bounds, new dedicated
`Capability::OpenFile { path }` authority, retained-root descriptor-relative
no-follow lookup, final regular-file descriptor retention, and the proc target
`/proc/<machine-god-parent-pid>/fd/<retained-target-fd>`.

The production command is exactly `/usr/bin/xdg-open` with only that proc path
as its argument, fixed `/` working directory, inherited host desktop
environment, and null standard streams. Machine-god never uses a shell or
`PATH`, and no provider or model field can select the program, argv,
environment, or working directory. The trusted `xdg-open` and desktop-dispatch
boundary may consult inherited `PATH`, configuration, or host state. Linux is
the only concrete launch target. Every other target returns fixed unsupported
behavior before filesystem lookup, worker creation, or helper spawn.

Cancellation before a successful spawn wins with zero launch. Successful spawn
is the commit boundary. Cancellation after that boundary makes core drop the
execution future; its cleanup terminates and reaps the direct helper and joins
owned work without claiming rollback. Without cancellation, the 30-second
timeout decision or explicit postspawn future drop terminates and reaps the
direct helper; cleanup may extend beyond the timeout decision.
Nonzero exit, signal, timeout, wait failure, or waiter-establishment failure is
the same fixed redacted nonretryable `open_file_result_unknown`. Exit zero means
only that the direct helper accepted the request, not that a desktop application
consumed, displayed, or retained the file.

The slice adds no external path, directory, URL, selected symlink, content read,
file mutation, arbitrary process authority, macOS real launch, CLI behavior,
benchmark workload, product-performance claim, inventory promotion, or
complete fx-equivalence claim.

## Contract-only host checkpoint

The delivered host remains byte-for-byte at eleven alphabetical tools:
`copy_file`, `create_folder`, `delete_file`, `edit_file`, `file_info`,
`glob_files`, `grep_files`, `list_files`, `read_file`, `rename_file`, and
`write_file`, using one original retained descriptor plus ten clones.

Later behavior composition must insert `open_file` after `list_files` and
before `read_file`, yield exactly twelve alphabetical tools, and use one
original plus eleven identity-preserving clones. Both path-based and prepared-
root constructors must compose that same tool catalog and retained workspace
identity.

## Planned ownership

- Core/production owns the dedicated `OpenFile` capability and stable serde/
  drop evidence, native implementation and exports, deterministic launcher
  seam, retained-root composition, and reference-host registration.
- Independent evidence owns direct, private, race, engine, host, core-contract,
  unsupported-target, cancellation, process-lifecycle, timeout, drop,
  permission, bounds, and redaction tests.
- Documentation owns the normative contract, implementation plan, maintained
  architecture/security/API/host pages, and exact-SHA lineage record.

Owners use isolated worktrees or explicitly non-overlapping files. Only the
composed integration SHA can become a formal behavior candidate. This ledger
is pre-created for that future review and records no reviewer result yet.

## Required evidence

- [ ] Exact `Capability::OpenFile` API, serde JSON, exhaustive drop handling,
  native tool/schema/constants/result/open-error and fixed tool-error
  contracts, including strict unknown-field rejection and stable redaction.
- [ ] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds.
- [ ] Empty/root/dot, absolute, tilde, parent, repeated/trailing separator,
  dot-component, control, line/paragraph-separator, bidirectional, and
  over-bound rejection; no Unicode normalization or case folding.
- [ ] Effect-free preparation, exact
  `{"type":"open_file","path":"..."}` evidence, denial before lookup,
  canonical direct execution, exact policy/execution agreement, and no general
  filesystem-read or process authority.
- [ ] Fresh retained-root validation, descriptor-relative no-follow traversal,
  final linked regular-file requirement, directory/symlink/FIFO/socket/device
  rejection, and absence of content reads.
- [ ] Root, ancestor, and final replacement; rename and unlink after retention;
  mixed-device traversal; proc-entry availability; outside sentinels; and the
  explicit host-boundary/no-sandbox semantics.
- [ ] Exact absolute `/usr/bin/xdg-open`, two-element argv, PID/fd decimal proc
  target, fixed `/` cwd, inherited host environment, null stdio, zero shell/PATH
  lookup by machine-god, trusted downstream dispatch, target-descriptor
  lifetime, and no model-controlled launch fields.
- [ ] Missing launcher and every spawn failure as retryable precommit
  unavailable with zero launch; exit-zero helper acceptance without application
  consumption/display claims.
- [ ] Nonzero, signal, timeout, wait, and waiter-establishment outcomes as fixed
  redacted nonretryable `result_unknown`; timeout terminates/reaps the helper.
- [ ] Inert construction/future until poll; cancellation before spawn with zero
  launch; successful-spawn commit; postspawn engine cancellation through drop;
  pre-poll drop and postspawn drop; 30-second timeout decision followed by
  terminate/reap/join; no detached owned helper/thread; concurrent-call
  isolation.
- [ ] Native Linux behavior, macOS/FreeBSD/WASI compilation and active
  unsupported-target behavior, exact eleven-tool checkpoint and future twelve-
  tool/eleven-clone host, no-unsafe, dependency, compatibility, documentation,
  diff, and fresh release-binary smoke evidence.

## Exact local gate before formal review

The composed candidate must first pass focused open-file private, direct,
engine, host, core-contract, launcher, process-lifecycle, and unsupported-target
suites. Then run the exact Rust 1.94.1 repository gate:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Also require the repository's Python, compatibility, dependency-policy,
dependency-audit, native Linux execution, native macOS unsupported-target
execution, cross-target, active WASI unsupported-target, Markdown-link, clean-
diff, no-unsafe, and freshly built release-binary smoke checks. Record all
exact counts, hashes, versions, expected skips, and any valid exact-toolchain
fallback. Real-launch tests must use a controlled fake or injected launcher;
CI must not open a desktop application. Local success is not delivery or
performance evidence.

## Formal same-SHA review protocol

After the complete local exact-SHA gate, create a tree-identical behavior
candidate and start three fresh reviewers against that same immutable SHA and
tree:

1. correctness/API;
2. filesystem/process-lifecycle robustness;
3. performance/concurrency.

Each reviewer independently verifies the SHA/tree in a clean detached worktree
and runs the applicable focused evidence. Every confirmed finding is fixed, the
complete local gate is rerun, and all three tracks restart with fresh reviewers
on one replacement SHA. Repeat until every track is green with zero findings.

Only then may a documentation seal be pushed for exact feature CI and benchmark
workflows. After those pass for that exact SHA, fast-forward `main` without
force and require exact main CI and benchmark workflows. Each claimed benchmark
run must retain the expected nonexpired exact-SHA artifacts. Documentation-only
seal and final delivery-record commits are exempt from another adversarial
cycle, but their exact workflows remain required. No package or release
publication is authorized by this review.

## Current verdict

**CONTRACT FROZEN; IMPLEMENTATION PENDING.** Exact base main CI and benchmark
evidence is green and the normative decisions above are closed. No open-file
core variant, native source, tests, dependency change, twelve-tool host
composition, behavior candidate, adversarial result, feature workflow,
integration, main workflow, delivery, product-performance, or fx-equivalence
claim exists at this checkpoint.
