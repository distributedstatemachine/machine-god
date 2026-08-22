# Implementation plan

Status values: `NOT STARTED`, `IN PROGRESS`, `BLOCKED`, `COMPLETE`.

## Objective

Build a high-performance Rust 1.94.1 coding-agent engine inspired by
`vercel-labs/fx`. Local development and CI use the declared minimum toolchain
exactly. Local `+stable` is a narrowly scoped fallback for this checkout's
damaged exact-toolchain installation and is valid only while both local Rust and
Cargo report release 1.94.1 exactly.
The embeddable asynchronous engine is the primary product; the CLI is its native
reference host. Observable performance and compatibility claims require retained
evidence against the pinned upstream revision. Product claims use committed,
reviewed summaries with input and result digests. The 90-day bootstrap workflow
artifact is non-product evidence of the collection path and may expire without
weakening those durable claim records.

## Delivery workflow

Every feature uses `agent/mNN-feature-slug`, isolated subagent worktrees, local
checks, three fresh adversarial reviewers, a pushed feature branch, remote CI for
the exact SHA, and a fast-forward push to `main`. Confirmed review findings are
fixed and rereviewed until none remain. Rejected findings are documented under
`docs/reviews/`. CI executes third-party actions by reviewed immutable commit,
keeps checkout credentials disabled, and grants the workflow read-only contents
permission. Python `test_*.py` files are discovered repo-wide in deterministic
order, excluding generated and checkout state under `.bench`, `.git`, and
`target`. Workspace tests execute natively on pinned Linux and macOS x86_64 and
aarch64 runner labels rather than relying on cross-compilation alone.

## Milestones

| Milestone | Deliverable | Status |
| --- | --- | --- |
| 01 | Repository, docs, CI, workspace, upstream benchmark harness, and non-product bootstrap evidence | COMPLETE |
| 02 | Provider-neutral streaming engine and deterministic testkit | COMPLETE |
| 03 | Providers, native tools, permissions, sessions, config, and CLI | IN PROGRESS |
| 04 | Security, lifecycle, concurrency, and persistence hardening | NOT STARTED |
| 05 | Skills, MCP, ACP, and subagent extensibility | NOT STARTED |
| 06 | SDK surfaces and advanced compatibility | NOT STARTED |
| 07 | Optimization, packaging evidence, and final hardening | NOT STARTED |

Milestone 02 completion evidence is retained in the
[milestone review](reviews/m02-milestone-review.md). Milestone 03 is in progress
with seven integrated bounded slices plus an eighth adversarially green bounded
candidate. The first
provides read-only native config/state
discovery, a fixed `ask` permission-mode report, and help/version/status CLI
behavior. The second adds synchronous read-only native loading of an exact
schema-v1 `ask` config, bounded to 64 KiB with fail-closed file and content
validation. Missing or unavailable configuration uses explicit built-in
defaults; configuration mutation is not implemented. The third adds
capability-aware tool preflight: the source-compatible default preserves the
raw `Capability::Tool` request, while a tool may prepare a normalized capability
and the exact arguments that an allowed execution receives. Core bounds the
prepared arguments at the existing exact byte limit and gives capability
serialization one total byte cap equal to that limit plus 1 KiB of fixed
headroom before policy. JSON depth and node traversal applies to the prepared
arguments and only the JSON values embedded in `Tool` or `Custom` capabilities.
Preparation is synchronous, bounded, nonblocking, effect-free trusted-host work
with immediate before/after cancellation checks; its arguments may drive only
effects within the authorized capability. Preparation failure produces a
durable generic tool error without consulting policy or exercising the tool.
The fourth slice adds the first executable native tool: a Unix-hardened,
read-only `read_file` capability rooted in an absolute workspace selected and
opened explicitly by the host. Its pure preflight accepts only a strict
`{path:string}` input, bounds that UTF-8 path at 4,096 bytes, and gives policy
and execution the same normalized workspace-relative path. Allowed execution
walks retained directory descriptors without following any component symlink,
accepts only a regular file, retains at most 8 KiB plus one overflow-detection
byte, and returns only valid UTF-8. The exact contract is in
[`read-file.md`](read-file.md). The fifth slice adds `list_files`, a
Unix-hardened, read-only, one-directory enumeration rooted in an explicit
absolute host path whose directory descriptor is retained. Its pure
preflight accepts only `{}` or a sole string `path`, defaults an omitted path to
`.`, and gives policy and execution the same normalized
`Capability::Filesystem(Enumerate)` path. Allowed execution uses retained
descriptor-relative directory and no-follow traversal, reads no child content,
and returns at most 100 sorted retained entries and 16 KiB of aggregate raw
entry-name bytes plus a truncation flag. It reads only the first extra visible
entry needed to establish truncation, so a truncated subset may reflect
filesystem iteration order rather than global directory order. The exact
contract is in [`list-files.md`](list-files.md). The sixth slice adds the first
concrete `ModelProvider`: a bounded, executor-neutral Vercel AI Gateway protocol
`0.0.1` / language-model specification `4` codec over an explicitly injected
byte transport. It projects the supported core transcript into the request
shape exercised by pinned fx, strictly reconstructs text, reasoning, local tool
calls, usage and finish events from arbitrarily fragmented data-stream bytes,
independently bounds JSON nodes as well as bytes and counts, ignores unsupported
temperature and metadata after applicable structural validation, and makes one
cancellation-aware transport call after a valid request future is polled
through startup (and zero for an unpolled, pre-cancelled, or invalid request).
Empty chunks fail, bounded no-event work yields cooperatively, and cancellation
wins same-poll terminal races. The injected host retains endpoint, HTTP, TLS,
authentication, status and retry responsibility; this slice performs no
network effect itself. Its exact contract is in
[`ai-gateway.md`](ai-gateway.md). The seventh slice supplies an optional
native-only Reqwest/Rustls HTTP transport for that injected codec. It fixes the
production URL to `https://ai-gateway.vercel.sh/v3/ai/language-model`, requires
an explicitly injected bounded bearer token, accepts plaintext only through an
explicit numeric-loopback test endpoint, and fixes proxy, redirect,
decompression, cookie, retry, timeout, active-request, status and diagnostic
policy. The concrete transport is polled on a host-owned Tokio runtime; core,
the codec and custom transports remain executor-neutral. Its exact contract is
in [`ai-gateway-http.md`](ai-gateway-http.md). Exact feature-branch review and
remote-run evidence is retained in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md).
The slice is integrated on `main` at
`508b0adbbe4447a85bd08f47095ae16c089c05d5`; exact main CI run `32535790803`
and benchmark run `32535790824` are green.

The eighth candidate adds a native `FileSessionStore` for supported Linux and
macOS Unix targets. A host supplies one existing absolute root whose opened
directory descriptor is retained; the store performs no environment discovery
or root creation. It maps validated session IDs through a fixed
domain-separated SHA-256 v1 layout, strictly stores one bounded versioned JSON
envelope per record, verifies the decoded ID, and implements optimistic new and
update saves with checked revision assignment. Permanent per-session advisory
lock sidecars coordinate cooperating processes. Bounded no-follow regular-file
reads and `0600` exclusive temporary writes, file sync, same-directory atomic
rename, and directory sync fail closed without repairing corrupt or nonregular
artifacts. Its futures are inert until polled but execute bounded synchronous
I/O, locking, and sync calls on the first polling thread. The exact candidate
contract is in [`session-store.md`](session-store.md). Its exact feature-branch
review, CI, and benchmark evidence are green, with evidence in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
The final documentation seal and `main` integration remain pending.

These slices do not complete Milestone 03. Permission prompting and modes
beyond `ask`, credential discovery and broader configuration, durable native
session migration, encryption, reset and listing, provider/CLI wiring,
workspace discovery, recursive/glob/grep and
mutation tools, non-Unix
hardened workspace construction, the remaining executable native tools, and
compatibility or performance claims remain planned. Existing CLI bytes,
benchmark evidence, workflows, and Zig inputs are unchanged; the provider is
explicitly scoped to a pinned wire shape and makes no current-protocol or full
fx-equivalence claim. Help and status remain claim-ineligible and unmeasured in
bootstrap evidence.

## Release gates

- Formatting, Clippy with warnings denied, workspace tests on all four native
  target runners, doc tests, repo-wide Python unit tests, dependency policy, and
  vulnerability audit pass.
- Deterministic end-to-end tests pass on Linux and macOS, x86_64 and aarch64.
- Three equivalent local workloads beat pinned fx by at least 20%, no other
  equivalent workload regresses more than 5%, Linux local command startup is at
  most 2 ms, and the stripped Linux x86_64 binary is at most 7.8 MiB.
- Safety, permission, correctness, and resource-bound invariants cannot be
  weakened to meet performance targets.

## Authorization and stop conditions

The coordinator is authorized to commit and push branches and `main` to
`distributedstatemachine/machine-god`. It is not authorized to publish packages
or GitHub releases. Continue fixing ordinary implementation, review, benchmark,
and CI failures until green. Stop only for missing external authority, unavailable
required credentials/runners, irreproducible upstream behavior, or a conflict
between a performance goal and a security invariant.
