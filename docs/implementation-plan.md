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
with nineteen delivered bounded slices.
The first
formal sixteenth-slice
candidate is composed through `dec98e0`, whose three review tracks were not
green. Its source and test fixes are composed in exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
green. First remote CI `32599591900` exposed the Linux removed-root gap; this
portable fix is exact behavior candidate `17f1884`, green under both executable
review tracks. Documentation seal `d3312d7` resolves its lineage finding, is
green under exact feature CI `32600292770` and benchmark evidence `32600292779`,
was fast-forwarded without force to `main`, and is green there under exact CI
`32600567094` and benchmark evidence `32600567090`.
The first
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

The eighth slice adds a native `FileSessionStore` for supported Linux and
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
I/O, locking, and sync calls on the first polling thread. The exact contract is
in [`session-store.md`](session-store.md). Its exact feature,
documentation-seal, and `main` checks are green, with evidence in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
It is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`; exact main CI run `32541315998`
and benchmark run `32541315997` are green.

The ninth integrated bounded slice defines a
native, executor-neutral `AskPermissionHandler` over an explicitly injected
`PermissionPrompter`. `AskPermissionHandler::new` accepts an owned concrete
prompter and `AskPermissionHandler::shared_prompter` accepts an
`Arc<dyn PermissionPrompter>`. The adapter forwards core's complete bounded
`PermissionRequest` by value without cloning, mutation, serialization,
truncation, revalidation, or traversal. Structured allow-once, allow-turn,
allow-session, and deny results map exactly to the corresponding core decision;
neither core nor the adapter caches a positive grant. Denial uses the fixed
reason `permission denied`. The zero-data prompt error maps fail-closed to only
`permission_prompt_failed` / `permission prompt failed`.

Authorization is inert until polled, the prompt future remains owned by the
adapter future, and drop supplies cancellation by dropping that prompt future.
The adapter detaches no work and supplies no second cancellation token, so an
injected prompter must not leave a detached approval operation behind. It owns
no terminal, UI, environment, filesystem, process, network, configuration, or
runtime authority. The exact contract is in
[`ask-permission.md`](ask-permission.md). The implementation and black-box tests
are present. Three fresh adversarial reviews have no confirmed open findings;
the documentation seal and exact feature-SHA workflows are green; and the slice
is integrated on `main` at `27e3f2b3ff170044732d9124ffb210beabcda206`.
Exact main CI run `32570197911` and benchmark run `32570197870` are green. The
complete lineage is recorded in the
[`ask permission handler review`](reviews/m03-ask-permission-review-01.md).

The tenth integrated bounded slice uses the existing native
`ai-gateway-http` and non-WASM gate. It owns a separate
`AiGatewayCredentialEnvironment` snapshot containing only `VERCEL_OIDC_TOKEN`
and `AI_GATEWAY_API_KEY`. A nonempty OIDC token has precedence over a nonempty
API key; unset and exactly empty values are absent; and any selected nonempty
non-Unicode, oversized, or malformed value fails closed without fallback.
Discovery consumes the snapshot and returns the selected source plus the
existing 1–4,096-byte RFC 6750 `AiGatewayBearerToken` without cloning it.
Errors have fixed missing, invalid-environment, and invalid-bearer categories
that retain and reflect no source or input. Accepted and retained data are
bounded, while process lookup may materialize a complete OS value before the
application can reject it. The exact contract is in
[`ai-gateway-credentials.md`](ai-gateway-credentials.md). This slice does not
add configuration credential fields or change core, the transport, or the CLI,
so the broader acquisition-and-configuration checklist item remains open.
Implementation, local gates,
and three fresh adversarial tracks are green at
`244e765713944b1bbe2ebca5bbbd02899c725e9f`. Exact feature CI run
`32573044224` and benchmark-evidence run `32573044159` are green for the
documentation seal at `ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. That
SHA is integrated on `main`; exact main CI run `32573320962` and
benchmark-evidence run `32573320937` are green. The review lineage is in the
[`credential discovery review`](reviews/m03-ai-gateway-credential-review-01.md).

The eleventh bounded slice advances the built-in native configuration and
current file schema to strict v2. Its exact five-field object has integer
`schema_version: 2`, permission mode `ask`, provider `vercel_ai_gateway`,
transport `ai_gateway_http`, and default model `zai/glm-5.2`. The exact strict
two-field v1 object remains read-compatible: it maps only in memory to those
fixed provider, transport, and model values, keeps observable schema version
`1`, and is never rewritten or migrated. Both schemas exclude credentials.
Models share the AI Gateway provider's 1–128-byte visible-ASCII validator;
config owns and redacts its model in debug output. Provider, transport, and
model selections are declarative only and do not compose the codec, optional
HTTP transport, runtime, credential, network, core, or CLI. Status remains
metadata-only and existing CLI bytes are unchanged. The exact
contract and review lineage are in
[`configuration.md`](configuration.md) and the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).
Production implementation, black-box tests, documentation, three fresh
adversarial tracks, and exact feature and `main` gates are green. The slice is
integrated on `main` at `a10f24edde80a225f89e6c7068ec035cb70f80a8`;
exact main CI run `32576876769` and benchmark-evidence run `32576876780` are
green.

The twelfth bounded library slice is Linux/macOS-only and gated on the
existing `ai-gateway-http` feature and non-WebAssembly targets. Its
`NativeReferenceHost` consumes an already validated `LoadedNativeConfig` and
composes `AiGatewayProvider` over either production `AiGatewayHttpTransport`
from an injected credential snapshot or an explicit trusted custom transport,
one shared retained workspace identity feeding exactly `list_files` and
`read_file`, the existing `FileSessionStore` over a separately supplied existing
root, and `AskPermissionHandler` over an injected `PermissionPrompter`. It uses
default `EngineLimits` and the default no-op event sink. Constructors are
synchronous: they make no network request, poll no prompt, touch no session
record, create no root or runtime, and start no background work. Production
construction opens its non-secret roots before credential discovery and token
handoff. A production HTTP request later requires a host-owned, driven Tokio
runtime.

The host retains the exact loaded configuration, including file-backed schema
v1 as observable version `1` with its fixed projected provider, transport, and
model values. Production exposes only the selected non-secret credential-source
metadata; the custom-transport authority override reports no native-discovery
source and exposes no secret getter. Fixed stage-only errors and host debug
output are redacted. The CLI remains byte-unchanged and thin. The
[`composition contract`](native-reference-host.md),
[`review record`](reviews/m03-native-reference-host-review-01.md), production
implementation, independent black-box tests, three fresh adversarial tracks,
and exact feature and `main` workflows are green. Its final delivery record is
integrated on `main` at `ac3984fb16dbab3adf86a949c7555ceca7c3e8df`;
exact feature CI run `32579779134`, feature benchmark-evidence run
`32579779123`, main CI run `32580066474`, and main benchmark-evidence run
`32580066485` are green.

The thirteenth bounded slice advances the strict current native
configuration to schema v3. Its exact six-field object is the strict v2 shape
plus required non-secret `credential_source: "environment"` selection.
`NativeCredentialSourceKind::Environment`, its stable `environment` name, and
the `NativeConfig::credential_source` getter expose only that closed
acquisition kind. The built-in configuration is v3. Exact strict v1 and v2
files remain readable without rewrite or migration, retain their observable
schema versions, and project `Environment` only in memory; v2 rejects the new
field as unknown. The existing 64 KiB input cap, strict duplicate and unknown-
field failures, model bounds, and redacted diagnostics remain unchanged.

The production reference-host path validates `Environment` and then consumes
the already injected `AiGatewayCredentialEnvironment`. The loader gains no
process authority. Runtime `NativeReferenceHost::credential_source()` still
reports the concrete selected `VERCEL_OIDC_TOKEN` or `AI_GATEWAY_API_KEY`
source; the trusted custom-transport path still skips discovery and reports
`None`. No token bytes, arbitrary environment-variable names, persistence,
migration, auth-source compatibility, CLI behavior, or performance claim are
added. Production implementation, independent tests, and maintained docs are
integrated. Focused and required local gates and all three fresh adversarial tracks
are green on exact behavior/finding-fix SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. Documentation seal
`5f4deac672af85fe5c0b1be50c327ddbdd55ce9a` is feature-green under exact CI
run `32582210892` and benchmark-evidence run `32582210927`. Feature-evidence
record `8755757da0da07e33af48d57f46bd9ea490b5449` is green under exact feature
CI run `32582687145` and benchmark-evidence run `32582687169`, was fast-
forwarded without force to `main`, and is green there under exact CI run
`32582978232` and benchmark-evidence run `32582978286`. The contract and
lineage are in
[`configuration.md`](configuration.md) and the
[`configured credential-source review`](reviews/m03-configured-credential-source-review-01.md).

The thirteenth slice's final documentation-only delivery record is integrated
on `main` at `f840576af241c58d1e55399e66ba92f7770cd50c`. Exact final-record
feature CI run `32583585145`, feature benchmark-evidence run `32583585148`,
main CI run `32583871385`, and main benchmark-evidence run `32583871368` are
green for that exact SHA. Per the delivery workflow and the user's explicit
instruction, that documentation-only final record was not adversarially
reviewed after the behavior was already green.

The integrated fourteenth bounded slice adds explicit
Linux/macOS native root selection and safe preparation. A
`NativeRootSelection` derives an exact state root from an injected
`NativeEnvironment` and an explicit absolute workspace. A
`PreparedNativeRoots` opens and retains the workspace first, requires the
selected XDG state base or fallback `HOME` to exist, and can create only the
fixed descriptor-relative `machine-god` or `.local/state/machine-god` suffix.
New directories use private mode, existing directories are never repaired,
and ownership, write, final privacy, no-follow, root-disjointness, and
descriptor-bound macOS extended-ACL checks fail closed. New consuming
reference-host constructors preserve those retained
identities and discover production credentials only after root preparation.
Schema v3, configuration bytes, status and every CLI byte remain unchanged;
the old reference-host path constructors and `FileSessionStore::open` remain
no-create. Production is present at `050d253`, with creation-normalization fix
`7420a3a` and selected-base normalization fix `fa5119a`. Independent regression,
core-contract, and prepared-host suites are present at `85c99a8`, `85c4193`,
and `236e3d4`, covering 2, 9, and 3 focused tests respectively. The formal
portability finding is fixed by explicit fixture modes at `f5dbbca`; the formal
macOS ACL finding is fixed by descriptor-bound rejection at `8ae17db` with its
independent regression at `041c83c`. The protective macOS HOME deny-delete
compatibility regression is `bb2a856`. The focused suites are green, including
11 core tests under default and all features; its policy and Rustdoc fix is
`fa94d8a`. All three formal tracks were green on exact behavior SHA
`f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. Documentation seal `03fa9ba`
then passed all native Linux/macOS jobs and benchmark run `32588948975`, but
feature CI run `32588948956` exposed three Linux-only strict-Clippy diagnostics.
Portable lint normalization is present at `90d8f96`, with local macOS and Linux
cross-target gates green. All three final adversarial tracks are green on exact
candidate `72cf64f63e0dfa30bc1ee21d8aca16550e819c21`. Replacement documentation
seal `f08dbd9eb2da81848b8eefb2d218006a64575835` is green under exact feature CI
run `32589778343` and benchmark-evidence run `32589778374`. Feature-evidence
record `6f66b6e5972e78ba0f0ccae06b899158d99bc864` is green under exact feature CI
`32590128235` and benchmark evidence `32590128233`, was fast-forwarded without
force to `main`, and is green there under exact CI `32590429626` and benchmark
evidence `32590429592`. This documentation-only commit is the final delivery
record; its exact feature and `main` workflows are reported at handoff. The
integrated contract and review record are in
[`native-root-selection.md`](native-root-selection.md) and
[`m03-native-root-selection-review-01.md`](reviews/m03-native-root-selection-review-01.md).

The delivered fifteenth bounded slice implements a Linux/macOS native by-ID
session lifecycle over the exact `Arc<FileSessionStore>` shared by
`NativeReferenceHost` and its engine. The caller supplies each validated
`SessionId`; default reference-host composition allocates a bounded incarnation
from OS cryptographic randomness, while standalone construction may inject an
explicitly trusted custom-host source.
Every lifecycle constructor validates exact shared-store `Arc` identity and
returns fixed redacted `MismatchedSessionStore` before entropy or filesystem
effects if an engine and concrete store do not share one allocation.
Create atomically persists an empty current-schema record at revision `1`
before returning a live session. Resume loads the current durable incarnation
and converges on the engine-canonical local state. Replay returns one validated,
bounded, point-in-time `SessionRecord` from durable storage rather than
reconstructing UI or event-stream behavior.

Reset refuses a locally live incompatible lifetime before record replacement,
then uses the permanent per-ID store lock and an exact
ID/incarnation/revision CAS to atomically publish an empty record under the same
ID, a new incarnation, checked revision `old + 1`, and turn allocator `1`. Its
preceding present-record load may create the fixed lock sidecar and its bounded
incarnation source may already have been consulted. Reset has no deletion gap
and does not revoke another process's old handle or external effects; the new
incarnation fences that handle's later saves. Missing resume/replay/reset,
duplicate create, local live state, entropy failure, conflict, corruption,
unavailability, and engine/invariant failure remain typed and redacted. Futures
are inert before poll, detach no work, and inherit the store's bounded
synchronous first-poll I/O and ambiguous post-rename directory-sync boundary.
The production API and these semantics are normative in
[`native-session-lifecycle.md`](native-session-lifecycle.md). Production,
fourteen independently owned focused tests, and one formal finding regression
are integrated and green, including the allocation-address identity portability
fix. All three adversarial tracks are green together on exact candidate
`e6a3804`. Feature record `dbba2c7` is green under exact feature CI
`32594562796` and benchmark evidence `32594562785`, was fast-forwarded without
force to `main`, and is green there under exact CI `32594846484` and benchmark
evidence `32594846476`. This documentation-only commit is the final delivery
record; its workflows are reported at handoff. `list_sessions`, lifecycle
CLI commands, migration,
encryption, and non-Unix hardening are not part of this slice.

The delivered sixteenth bounded slice adds Linux/macOS library-only
`NativeSessionLifecycle::list_sessions`. Its IDs-only `NativeSessionList`
contains at most 100 sorted unique validated IDs plus `truncated`. Each call
processes or selects at most 1,024 non-dot entries plus one fetched and name-
inspected overflow witness, and accepts/decodes at most 64 MiB of aggregate
canonical record bytes plus one transient transfer byte used only to detect
concurrent growth; all non-dot names within the scan count against its budget.
Exact canonical record names are opened no-follow, locked with the existing
per-ID protocol, and
validated against the same current schema, bounds, positive counters, and
decoded-ID/digest invariant. Canonical corruption fails the whole call as
redacted `Corrupt`; enumeration, read, and lock failures are redacted
`Unavailable`. Successful listing may create private permanent lock sidecars.

`truncated` means only incomplete bounded observation. It is not pagination and
does not imply `has_more`. Canonical filenames are sorted before validation;
result and byte caps select from that sorted scanned set, while a fired raw scan
cap can make the set filesystem-iteration-dependent. Returned IDs are sorted,
but digest filename order is not a globally first ID or semantic ranking. There
is no multi-record snapshot. A candidate that vanishes before its locked read
may be omitted and may leave its private lock sidecar. A nonregular derived lock
for a still-present canonical candidate is `Corrupt`; ordinary lock I/O is
`Unavailable`. On macOS the replacement must acquire a fresh `.` descriptor
first, then validate that exact acquired descriptor's linked identity. A stable
completed rename retains identity; removal before acquisition or validation is
unavailable; and a concurrent rename/removal may conservatively be unavailable
or observe the acquired identity without creating a global snapshot. The
future is inert before poll, performs bounded synchronous work on first poll,
and detaches nothing. It consults no live registry, incarnation source,
provider, permission handler, tool, network, workspace, configuration, or
environment authority. The exact candidate contract and non-green review lineage
are in [`native-session-listing.md`](native-session-listing.md) and
[`m03-native-session-listing-review-01.md`](reviews/m03-native-session-listing-review-01.md).
Production `0accfbf` is composed as `1bffac9`, documentation `63d589c` as
`87d7de0`, and 13 initial independent tests `1b531297` as `4b4e468`, from base
`9ada4b5`. The removed-root fix and first formal candidate are `dec98e0`. All
three first review tracks were not green. Isolated acquire-first source fix
`4b8d8b0` and isolated test hardening `446b495` are composed in the replacement
candidate, with 18 focused tests and the full all-target/all-feature workspace
suite green locally. All three replacement tracks are green on exact behavior
candidate `3fa5463`; portable behavior `17f1884` and seal `d3312d7` are green
under exact feature and `main` delivery workflows.

The current schema and store contain no authoritative summary, workspace,
title, preview, language, timestamp, latest-order, or index fields. This slice
therefore adds no rich summaries, workspace/latest filter, cursor, pagination,
CLI or slash command, and makes no fx-equivalence claim. The `sessions-json`
benchmark remains unimplemented and claim-ineligible.

The delivered seventeenth bounded slice adds Linux/macOS library-only
`file_info`.
Its strict effect-free preflight accepts only a required string `path`, bounds
the requested and normalized forms to 4,096 UTF-8 bytes, applies the same
lexical confinement as `read_file`, and prepares both
`FilesystemAccess::Metadata` policy input and exact execution arguments with
the same normalized workspace-relative path. Nonempty current-directory forms
normalize to `.` so the retained root can be inspected. A distinct metadata access kind
prevents read-content authority from being inferred from this inspection.

Allowed execution starts from the explicitly retained workspace descriptor,
acquires a fresh `.` descriptor, and validates that exact acquired linked root
identity under platform-specific Linux/macOS rules. It then opens ancestors
descriptor-relatively with directory and no-follow requirements and performs
one no-follow metadata lookup for the final component without opening it; `.`
uses one final `fstat` after liveness validation. Final symlinks report link metadata; FIFO,
socket, device, and other special objects report `other` without a blocking
open. Its exact bounded output is normalized `path`, fixed `kind`, checked
nonnegative `size_bytes`, `modified` with signed Unix seconds and validated
nanoseconds, and a nullable lexical extension for regular files only. Dotfile,
trailing-dot, multi-dot, retained-root rename/removal, concurrent replacement,
single-stat snapshot, redaction, cancellation, and inert-future semantics are
normative in [`file-info.md`](file-info.md).

The delivered reference-host extension registers exactly three workspace tools:
`file_info`, `list_files`, and `read_file`; core exposes that catalog in
deterministic alphabetical order. Prepared roots transfer descriptor clones of
the same retained workspace identity so the three tools receive three
descriptor instances: the original plus two clones. Production is present at
isolated SHA `5c2d129`; independent direct
and engine tests are present at isolated SHA `ca0091c` and compose with
production at `f228c06`, where all 34 focused tests are green. Required local
gates are green through composed precursor `0973acf`. First formal candidate
`8399ec7` was not green. Isolated finding-test hardening `7f2a292` composes as
`b69ec4b`, bringing the focused total to 36 plus five private unit tests; the
two documentation findings are corrected at `9dbd188`. Replacement local gates
are green through `d445eb3`, and all three replacement tracks are green on exact
candidate `4193ecc`. Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI run
`32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four workflows report that exact seal SHA. Benchmark success
is delivery evidence only, not a product-performance claim. This
documentation-only commit is the final delivery record, is explicitly exempt
from another adversarial review after behavior was already green, and reports
its own exact workflows at handoff.
The slice adds no CLI behavior, non-Linux/macOS hardening, content or target
reading, mutation, recursion, MIME/hash/ownership/mode/ACL/xattr reporting,
extra timestamps, compatibility/equivalence claim, benchmark, or performance
claim. Its base and parallel ownership are recorded in the
[`file_info` review](reviews/m03-file-info-review-01.md).

The delivered eighteenth bounded slice adds Linux/macOS library-only `glob_files`
from exact base `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`. Strict effect-free
preflight accepts exactly
`{pattern:string,path?:string,mode?:"matches"|"count"}`, defaults path to `.`
and mode to `matches`, rejects unknown fields, and independently bounds each
requested and normalized path and pattern at 4,096 UTF-8 bytes. Path
normalization is the `file_info` rule. Pattern normalization uses `/`, collapses
repeated separators and exact `.` segments, and rejects an empty normalized,
absolute, parent-traversing, or forbidden pattern. Backslash, square brackets,
and braces are literal. The bytewise matcher gives `?` one byte, `*` zero or
more within a component, and only an exact `**` segment zero or more complete
components. Slash-free patterns match candidate basenames recursively;
slashful patterns match paths relative to the selected search root.

Successful preflight produces exact prepared arguments with both defaults
explicit and `FilesystemAccess::EnumerateRecursive` at the normalized selected
subtree. This permission kind is distinct from one-level `Enumerate`; pattern
and mode attenuate output but do not broaden or replace the recursive scan
authority. Allowed execution reacquires and validates the retained workspace
through the same fresh-`.` liveness rule as `file_info`, then performs iterative
descriptor-relative no-follow traversal. Each directory is fully read,
validated, and sorted bytewise before processing. Hidden entries are included;
regular files and final symlinks are candidates; directories are traversal-
only; specials are ignored; and symlinks are never descended through or read.
Returned candidates use full workspace-relative paths.

Both modes complete a bounded traversal of no more than 100,000 non-dot
entries, 16 MiB of aggregate raw entry-name bytes, directory traversal depth
256, and 8,388,608 aggregate matcher-work steps. A step is charged for each
slashful candidate byte inspected while splitting, pattern-segment loop visit,
dynamic-programming cell write, and invoked component-matcher transition or
trailing-star consumption. Exactly the cap is permitted; checked overflow or
the next step is a scan-limit error. The selected root is depth 0; a depth-256
directory is scanned and its
regular/symlink children remain eligible, while attempting to open a child
directory at depth 257 is a scan-limit error. Any candidate full workspace path
over 4,096 bytes is also a scan-limit error. A fired scan cap fails either mode
without partial output. Match mode returns the longest globally bytewise-sorted
prefix under 100 paths and 16 KiB of aggregate raw path bytes and sets
`truncated` exactly when an observed match is omitted; it never skips an
omitted long path to admit a later short path. Count mode returns the exact
count. A stable tree is deterministic, while concurrent scans are not
snapshots; a `NOENT` race may omit an entry and other failures fail closed under
fixed redacted errors.

The slice extends the reference host to exactly four alphabetical workspace
tools: `file_info`, `glob_files`, `list_files`, and `read_file`. Prepared roots
distribute the original retained descriptor plus three clones of the same
identity. Its public constants, constructor errors, result shapes, cancellation
boundaries, compatibility inputs and deliberate differences, and deferred scope
are normative in [`glob-files.md`](glob-files.md). Parallel production,
independent-test, and documentation ownership is recorded in the
[`glob_files` review](reviews/m03-glob-files-review-01.md). Isolated components,
composed behavior, and local-gate lineage are recorded there: production
`a5d1399`, schema correction `f1584f5`, independent tests `948994d`, and
documentation `f2f0fc1` compose through initial local-gate precursor `60070d8`.
The first formal review at `1f5de6a` found a high unmetered matcher-work defect.
Production fix `f3fd13b` and independent regression `825bbd3` compose through
`aba2821`; the public-cap assertion and replacement local gates are green at
exact code-and-test head `4171a4a8811a98888b7e4e161281a1216564746f`.
All three replacement adversarial tracks are green on exact behavior SHA
`523df85822a27102d7e7100e274e3bad7b25494f`. Documentation seal and integrated
`main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI
`32610950593`, feature benchmark evidence `32610950594`, main CI `32611208411`,
and main benchmark evidence `32611208415`; all four report that exact seal SHA.
Benchmark success is delivery evidence only, not a product-performance claim.
Final documentation record `f6aa458bb875d6cb26565adc878703fe140916d3`
passed exact feature CI `32611623653` and benchmark evidence `32611623655`.
GitHub did not materialize workflows for its first `main` event, so
tree-identical non-behavior grep kickoff marker
`f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` passed exact feature CI
`32612424382` and benchmark evidence `32612424383`, was fast-forwarded to
`main`, and passed exact main CI `32612662260` and benchmark evidence
`32612662203`. Per the user's instruction, these documentation-only/tree-
identical records are exempt from another adversarial cycle after behavior is
green. The slice adds no
CLI behavior, external-path access, ignore or Git/subprocess behavior, content
read, mutation, dependency, benchmark workload, product-performance claim, or
fx-equivalence claim.

The delivered nineteenth bounded slice adds Linux/macOS library-only `grep_files`
from exact base `f6aa458bb875d6cb26565adc878703fe140916d3`. Strict effect-free
preflight owns all eight pinned field names: required literal `pattern` plus
optional `path`, `include`, `case_insensitive`, `mode`, `head_limit`, `offset`,
and `context_lines`. It rejects unknown or mistyped values and prepares the
explicit defaults `.`, `null`, `false`, `matches`, `100`, `0`, and `0`.
Policy receives the distinct `FilesystemAccess::SearchContent` capability at
the normalized selected path, which may resolve after approval to a regular
file or directory. Search authority is separate from `Read`, `Metadata`,
`Enumerate`, and `EnumerateRecursive`.

Allowed execution reacquires the retained workspace under the delivered linked-
root liveness rule. Selected ancestors, the selected file/directory, traversed
directories, and candidate regular files use descriptor-relative no-follow,
close-on-exec, and nonblocking operations with authoritative opened-type checks.
Traversal is iterative, includes hidden entries, fully validates and bytewise
sorts each directory, and bounds every complete descendant path before
allocation, entry-kind handling, or include matching. Stable special objects
are skipped; a raced nonblocking special open is rejected before read, and no
symlink is followed. Optional include filtering is compiled once per call,
uses the delivered bytewise glob grammar, charges complete parse and match
work, and does not prune directory traversal. Fixed literal pattern-table work
is charged before selected-root resolution. Selected-file filtering occurs
after no-follow stat classification and before content open. A slashful
selected-file rejection consumes one charged cancellation-checked include
decision. An excluded selected file consumes fixed pattern-table and include
work but no candidate, content-byte, or per-file matching work; an included file
opens and is revalidated before those latter budgets. No ignore, Git,
subprocess, external-path, or ambient discovery behavior is added.

Content matching is literal and worst-case linear, with exact byte comparison
or ASCII-only case folding and one result per matching LF-delimited line. A
candidate is eligible text only when its complete observed content is at most
204,800 bytes, valid UTF-8, and NUL-free. Oversized and non-text candidates are
skipped with disclosed aggregate statistics; other candidate failures fail the
complete call. Match excerpts are UTF-8-safe, at most 4,096 bytes, and contain
the complete first matched substring. One scan-local content buffer reads
through an 8 KiB window, grows only to the 204,801-byte per-file high-water
ceiling, and logically resets for reuse between files without exposing stale
bytes. Actual aggregate and per-file overflow witnesses remain charged.
Requested before/after context comes from the same validated logical file view,
and per-record `context_truncated` distinguishes budgeted context omission from
top-level page incompleteness.

Each mode either completes the same bounded scan or fails without partial
output under exact 4,096-byte input/path limits, head 100, offset 67,108,864,
context 5, 100,000 entries, 16 MiB name bytes, 10,000 candidates, 64 MiB
aggregate content, 8,388,608 include compile/match steps, 268,435,456 literal-match
steps, depth 256, 8 KiB aggregate result paths, 8 KiB aggregate result text,
and a 48 KiB serialized `ToolOutput`. `matches` and `files_with_matches`
return deterministic bounded pages with exact totals, scan statistics,
`next_offset`, and list-completeness `truncated`; `count` returns exact eligible-
text matching-line/file totals without pagination fields. Every emitted
`next_offset` is reusable under the accepted bound. Cancellation is checked at
fixed intervals while indexing lines and at every serialized-output trimming
iteration. Slashful candidate splitting checks at intervals of at most 1,024
candidate bytes, and both recursive and non-recursive dynamic-programming
branches route through the scan-local injectable cancellation checker.

The candidate extends reference-host composition to exactly five alphabetical
workspace tools: `file_info`, `glob_files`, `grep_files`, `list_files`, and
`read_file`. Prepared roots distribute the original retained descriptor plus
four clones of one identity. The complete provider schema, authority,
descriptor, matcher, eligibility, result, error, cancellation, race, and
deferred contract is normative in [`grep-files.md`](grep-files.md). Parallel
production, independent-test, and documentation ownership and the review gates
are recorded in the
[`grep_files` review](reviews/m03-grep-files-review-01.md). Integration kickoff
marker `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` is tree-identical to the exact
base. Exact production component
`27eec2f3c25ffecd1ba8ff3c0a4fe0129dbeeac3` and initial independent-test
component `6eaee93398de8fbf6e87e77cf4d3e7de56e2a8cb` exist. Initial integration
heads are `9057feb24fd3f24657148ca8e78198b88c9dbab4` after production and
`44e33d7e24c6650a1e375cd095eb9efae31f4e78` after the tests. Reference-host
fixture fix and focused production/test head
`bdbb677161322e249aea95a12bfb1b2169ff5b48` is green. The documentation
component is `b04151a7d958875118eebddd67526d74e2ea9526`, producing first fully
composed behavior candidate `42e4793b27902da7390dc54ef6bedb169da7e1bc`.
Lint fix and exact local gates are green at
`45ad91fa2689250c47c79d2105f5e3c261cea638`. All three first-cycle formal
tracks are **NOT GREEN** on exact candidate
`355a11a6055b0053dff80e71011d7633e8a6ce97`. Remediation and exact replacement
local gates are green at final code/test precursor
`275d263dd3c7981e66f6a0f90f3779c271eb4cc3`. First replacement candidate
`ae87bf1454b1527b2e55ed5e517c21fd7410c980` is **NOT GREEN** with one low
correctness ordering finding, one low filesystem evidence-wording finding, and
medium-plus-low performance/cancellation findings. Second production remediation
`ac5d7726411744e4f85344edf966d26a3cdb0a26` composes at `d672210`; second
documentation remediation `7ad0863885d28b7b7a1d6f89d35f525cdd2dd3fa`
produces fully composed exact local-gate-green precursor
`b498ba06fa808dc9453a7644727cf8166b6f8e87`. Formal second replacement
candidate `5aeddc1b4cb210b00cb967b938db8d5232062916` has correctness/API and
filesystem/robustness **GREEN** with zero findings. Performance/concurrency is
**NOT GREEN** with one medium repeated 204,801-byte buffer-allocation
amplification finding and two low findings: the Markdown inventory is 58 rather
than 57 files, and deterministic recursive-DP evidence was overclaimed. The
third remediation supplies injected-checker routing and a deterministic
recursive regression. Third production remediation
`8777825b1b8b8c97dd4eb4bb31c0d8dbed9a7741` composes at `ab1c133`; independent
regression `dcf57ad35150b86c84a3f6c1127d9e379f3840fc` composes at `d7526d4`;
review-findings documentation `44afb232f2b8418c0b61eec7d1dab46bbe8e3667`
composes at `f08c5f2`; lint follow-up `1f13f9ae04ee3307d13a363ed28b156d7ee2421f`
produces exact fully composed local-gate precursor
`a8f61794ee5e279558856220b5789526b908015a`. Exact Rust 1.94.1 formatting,
warnings-denied workspace Clippy, 598 non-documentation tests plus two doctests,
25 private native tests, 40 direct `grep_files` tests, four engine tests, and
diff checks are green. Exact a8f cross-target/dependency/link and compatibility/
release validators are green. Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. Isolated wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
green, and its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
in all three fresh tracks: correctness/API, filesystem/robustness, and
performance/concurrency. Exact-SHA formatting, warnings-denied workspace
Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25 private
tests, 40 direct `grep_files` tests, four engine tests, and the 58/420/270/0
documentation inventory are green. All historical findings are closed,
including the attempted-read-window storage wording. This documentation-only
seal is exempt from another adversarial review under the user's explicit
instruction. Documentation seal `0f48806310882caf3c668c72fe1b9d211cae744b`
is feature-green: CI run `32623585346` passed all six jobs and benchmark-
evidence run `32623585349` passed both jobs and artifacts, all for exact `0f`.
`main` was fast-forwarded without force from `f6ab594` to exact `0f`. Main CI
run `32623904784` is **GREEN** for exact `0f`: all six jobs and every step
passed without reruns. Main benchmark-evidence run `32623904800` is **GREEN**
on attempt 1 for exact `0f`: both jobs and every step passed, with two valid
non-expired exact-SHA artifacts retained. The `grep_files` slice is delivered;
the remaining native tools remain pending.
This final delivery record is documentation-only and exempt from adversarial
review; its own exact remote workflows are required after push and cannot be
self-recorded.
The maintained documentation must compose into the
exact behavior SHA reviewed by all three adversarial tracks. A later
documentation-only seal or delivery record is exempt from another adversarial
cycle under the user's instruction but still requires exact feature and `main`
workflows.

The candidate adds no regex, Unicode case folding, binary search, alternate
encoding, symlink-target search, context reread, CLI behavior, benchmark
workload, compatibility-status change, product-performance claim, or fx-
equivalence claim. Zig remains only the pinned upstream benchmark build input;
the product remains Rust.

The twentieth bounded Milestone 03 slice is native `write_file`, frozen in
exact contract commit `3ee52fd8393bfb86f11048eaa6c624bd18a78798` from exact
delivered base `bc042536eb3a40d75ccf4d1fe52032b31defac04`.
Exact contract-only feature CI `32626410935` and benchmark-evidence run
`32626410931` are green; they are not implementation or final-feature evidence.
Effect-free preparation accepts exactly required UTF-8 `path` and `content`
strings, rejects unknown fields, independently caps the normalized path at
4,096 bytes and 256 components, raw content at 49,152 bytes, and serialized
arguments at 65,536 bytes, and prepares exactly
`FilesystemAccess::Write` for the canonical workspace-relative path. Empty and
NUL-containing content are valid. Direct execution revalidates the same strict
canonical shape, so neither provider nor direct use can widen approved
authority.

Allowed Linux/macOS execution operates only beneath the retained workspace
descriptor and requires every parent to exist. It no-follow walks parents,
records parent and target identity, rejects symlink and special targets, and
never opens or reads existing target content. It stages into one exclusive,
initially private `0600`, same-parent regular file chosen within eight high-
entropy name attempts; writes and cancellation checks use at most 8 KiB chunks. The staged
file receives exact `0644` ordinary rwx bits for creation or the initially
observed replacement target's `st_mode & 0o777`, then is file-synced,
identity-revalidated, and published atomically at the pathname. Missing-target
creation must use `renameat_with(..., RenameFlags::NOREPLACE)` and fail closed
when unavailable; replacement uses ordinary same-parent rename. A final parent
sync completes durability. Success is exactly normalized `path` plus
`bytes_written`.

The irreversible boundary is rename. Precommit failures leave the target name
unchanged and use best-effort identity-checked temporary cleanup, with the
portable metadata-check-to-unlink race explicitly disclosed. Formal cycle 2
found that exact candidate `708f2d0` can leave retained owned residue carrying
its already-applied final mode rather than private `0600`, so the earlier
private-residue statement is not a satisfied candidate claim. After rename,
the tool completes parent sync and returns success or
nonretryable commit ambiguity rather than its own cancellation error. Creation
cannot clobber a raced target, but replacement is not inode compare-and-swap:
the final validation-to-rename race and a final-parent move outside the
workspace remain documented limitations. The slice claims neither perfect
identity-safe cleanup nor adversarial concurrent-rename confinement.

Production, independent tests, and maintained documentation had separate
owners and are composed on `agent/m03-write-file`. Exact production component
`e9b3ad8` composes as `c0d555b`; exact independent-test component `59a06a3`
composes as `c4c5ce6`; exact documentation component `9285fe9` composes as
`de46c3e`. Retained-root fixture correction `8509933`, deterministic seam
hardening component `1d30ff9` composed as `a9a7c99`, and core same-poll recovery
regression `8b5847f` close the first preformal evidence gaps. Full-pipeline
fault and phase evidence `c7fdef2` covers both mode stages, write, staged sync,
create/replace rename, staged-name tampering, traversal and final-prepublish
cancellation, real-rename parent-sync ambiguity, and device rejection. Exact
local precursor gates are green at `072bd69`: formatting, warnings-denied
workspace Clippy, 651 workspace tests, two doctests, focused native and engine
suites, Python and pinned-compatibility gates, dependency policy/audit, native
macOS execution, Linux and FreeBSD cross-compilation, WASI compilation with
active unsupported-target execution, documentation links, diff checks, and a
freshly built release CLI smoke. Exact candidate
`119938240807f8279f83e2ace65a69706e8fcfed` is tree-identical only to its
immediate parent `a7841c19b4b34cecf40e55d7cd001fd1547133c1`; precursor
`072bd69eb6f73944d1db00363da0f965f09dda9f` has a different documentation tree.

All three first-cycle formal tracks are **NOT GREEN** on exact candidate
`119938240807f8279f83e2ace65a69706e8fcfed`. Confirmed findings are high/medium
unbounded `EINTR` retries in write and sync helpers; medium missing
deterministic real-pipeline proofs for target appearance, existing-target
replacement, and final-parent postvalidation races; medium missing real-
pipeline verification-phase cancellation evidence; and low stale platform,
local-gate, and lineage claims in maintained documentation. Documentation
correction `016f8df` and code/evidence remediation `3010e6d` close those
findings with an exact cumulative 16-interruption phase bound, native
postvalidation race tests, and post-verification cancellation/cleanup evidence.
Exact replacement local gates are green at `581fe6a`: 655 workspace tests, two
doctests, focused native/engine suites, warnings-denied workspace Clippy,
Python/compatibility/dependency gates, cross-target and active WASI checks,
documentation/diff/no-unsafe checks, and a fresh release CLI smoke. Three fresh
same-SHA review tracks then inspected exact candidate
`708f2d08d72d610ca387a62a4cec1f656c188a7d`, which is tree-identical only to
its immediate formal-review preparation parent
`491496aa22aa8855717b74f6a026e8c602bb02e9`. Correctness/API is **GREEN** with
zero findings. Filesystem/robustness is **NOT GREEN** with two medium findings:
retained staged inode mode can remain more permissive than `0600`, and Linux
entropy acquisition can retry partial reads or interruptions without a finite
work/cancellation bound. Performance/concurrency is **NOT GREEN** with the same
medium entropy finding. No behavior-green SHA is claimed.

The next remediation remains pending. Linux entropy acquisition is intended to
use direct `rustix` `getrandom` with `NONBLOCK`, one cumulative partial-
progress/interruption bound, and cancellation at retry and exhaustion
boundaries. `ENOSYS`, `EPERM`, and `EAGAIN` will fail closed as retryable
`write_file_unavailable` rather than invoke a fallback or block. The pinned
macOS `getrandom` 0.4.3 path uses one `getentropy` call for the 16-byte request.
Cleanup is intended to make one best-effort `fchmod(0600)` call on the retained
held staged descriptor before the existing identity-checked best-effort unlink.
If mode restoration and unlink both fail, residue can retain final mode bits.
These descriptions are pending design requirements, not completed behavior.
After production remediation and exact local gates compose into a new behavior
candidate, all three fresh tracks must review that same SHA again.

Evidence must cover exact schema and
limits, normalization and policy agreement, create/replace and atomic
visibility, exact modes under hostile umask, missing parents, symlink and
special rejection, retained-root changes, all eight temporary collisions,
cleanup and target/parent race seams, every precommit fault, post-rename sync
ambiguity, cancellation through the final precommit check, engine recovery,
the exact six-tool alphabetical reference host, Linux/macOS execution,
FreeBSD/WASI compilation, and active unsupported behavior. Three fresh
correctness/API, filesystem/robustness, and performance/concurrency agents must
all report green on the same replacement behavior SHA; every finding restarts
all three tracks. Exact feature CI must still execute supported native Linux and
macOS behavior. Documentation-only seal and delivery commits are exempt from a
new adversarial cycle under the user's instruction. The complete normative
contract and kickoff lineage are in [`write-file.md`](write-file.md) and the
[`write_file` review](reviews/m03-write-file-review-01.md). The slice adds no
parent creation, external-path access, target-content read, CLI behavior,
benchmark workload, product-performance claim, or fx-equivalence claim. The
product remains Rust; Zig remains only the pinned upstream fx benchmark input.

### Milestone 03 completion boundary

The nineteen delivered slices do not complete Milestone 03.
The following checklist is the frozen M03 boundary; changing ownership requires
an explicit plan change in a reviewed commit rather than silently deferring a
gate:

- [x] Integrate and retain evidence for slices one through eight.
- [x] Integrate the ninth ask-handler slice under its exact contract and
  retain feature, adversarial-review, exact-SHA CI, and `main` evidence.
- [x] Integrate the tenth credential-discovery slice under its exact
  contract and retain feature, adversarial-review, exact-SHA CI, and `main`
  evidence.
- [x] Integrate the eleventh native host configuration schema-v2 slice under
  its exact contract and retain feature, adversarial-review, exact-SHA CI, and
  `main` evidence.
- [x] Compose a useful native reference-host path through an explicitly selected
  provider and transport, session store, permission handler and prompter, and
  registered tools. The CLI stays a thin host and owns no product state.
  Implementation, independent tests, composed adversarial review, exact
  feature-SHA gates, fast-forward integration, and exact `main` gates are green.
- [x] Add bounded, redacted credential acquisition and the configuration fields
  required by that composition. Source precedence, missing/invalid behavior,
  size limits, and secret non-reflection must be normative and tested; core
  receives no ambient credential or configuration authority. The thirteenth
  slice supplies the bounded non-secret selection. Implementation, independent
  tests, all three fresh adversarial tracks, exact feature gates, fast-forward
  integration, and exact `main` gates are green.
- [x] Add explicit workspace/state-root selection and safe required-root
  creation, plus native create, list, resume, replay, and reset session
  lifecycle behavior for the current schema. A reset under a reused session ID
  must allocate a new incarnation before reuse. The delivered fourteenth slice
  is limited to the root-selection and safe-creation sub-boundary. The
  delivered fifteenth slice implements by-ID create, resume, durable-record replay,
  and reset/new-incarnation behavior with fourteen independently owned focused
  tests plus one formal finding regression, with all three adversarial tracks
  green on exact candidate `e6a3804`; exact feature and `main` workflows are
  green through record `dbba2c7`. The delivered sixteenth slice supplies the
  remaining bounded IDs-only native listing functional scope. Production and
  13 initial independent tests are composed through first formal candidate
  `dec98e0`, whose three first review tracks were not green. The replacement
  source fix and 18-test hardened suite from `4b8d8b0` and `446b495` are
  composed in exact behavior candidate
  `3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
  green. Portable behavior `17f1884` is green under both executable reviews.
  Documentation seal `d3312d7` passed exact feature CI `32600292770` and
  benchmark evidence `32600292779`, was fast-forwarded without force to `main`,
  and passed exact main CI `32600567094` and benchmark evidence `32600567090`.
- [ ] Complete the M03 native tool set: `list_files`, `glob_files`,
  `grep_files`, `read_file`, `write_file`, `edit_file`, `delete_file`,
  `rename_file`, `copy_file`, `create_folder`, `file_info`, `open_file`,
  `web_fetch`, `web_search`, `terminal`, `ask_user_question`, `vision`, and
  `read_tool_result`. Every authority-bearing tool requires normalized
  preflight, exact policy/execution agreement, resource bounds, redacted
  diagnostics, cancellation/drop tests, and platform scope stated before
  integration. The delivered seventeenth slice supplies only `file_info`; production
  and 34 focused tests are present and green at code-and-test head `f228c06`,
  with review hardening bringing the focused total to 36 plus five private unit
  tests at `b69ec4b`. The first formal candidate was not green; replacement
  local gates are green through `d445eb3`, and all three replacement tracks are
  green on exact candidate `4193ecc`. Seal and integrated SHA
  `60dd54f273afc7e62fb4b3cc1fb1a347d739998b` is green under exact feature CI
  `32605071080` on successful retry attempt 2, feature benchmark evidence
  `32605071063`, main CI `32606050292`, and main benchmark evidence
  `32606050294`; all four report that exact seal SHA. The other listed native
  tools remain incomplete, so this combined item stays unchecked. The
  delivered eighteenth `glob_files` slice has a frozen contract from base
  `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`; production, 39 focused
  integration tests, five private unit tests, documentation, composition, and
  initial local gates were green at `60070d8`, but the first formal review at
  `1f5de6a` found a high unmetered matcher-work defect. The checked matcher-
  budget fix, independent both-mode regression, 40 focused integration tests,
  nine private tests, and replacement local gates are green at exact code-and-
  test head `4171a4a8811a98888b7e4e161281a1216564746f`. All three same-SHA replacement
  adversarial tracks are green on exact behavior SHA `523df858`. Documentation
  seal and integrated `main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4`
  passed exact feature CI `32610950593`, feature benchmark evidence
  `32610950594`, main CI `32611208411`, and main benchmark evidence
  `32611208415`. Final documentation record `f6aa458` passed exact feature CI
  `32611623653` and benchmark evidence `32611623655`; tree-identical kickoff
  marker `f6ab594` supplied the missing exact `main` handoff evidence under
  feature CI `32612424382`, feature benchmark `32612424383`, main CI
  `32612662260`, and main benchmark `32612662203`. The nineteenth
  `grep_files` candidate is frozen from exact base `f6aa458`; its parallel
  production, independent tests, and maintained documentation must compose into
  one exact behavior candidate with all three formal review tracks green.
  Exact production `27eec2f` and initial independent-test `6eaee93` components
  exist and initially compose through `9057feb` and `44e33d7`; fixture fix
  `bdbb677` makes focused production/test composition green. Documentation
  component `b04151a` produces first fully composed behavior `42e4793`; lint
  fix and exact local gates are green at `45ad91f`. All three first-cycle tracks
  are **NOT GREEN** on exact `355a11a`. Remediation and exact replacement local
  gates are green at final code/test precursor `275d263`. First replacement
  candidate `ae87bf1` remains historically **NOT GREEN** across all three tracks.
  Second-fix production and documentation compose through `ac5d772`, `d672210`,
  `7ad0863`, and exact local-gate-green precursor `b498ba0`. Formal second
  replacement candidate `5aeddc1` has correctness/API and
  filesystem/robustness **GREEN** with zero findings and performance/concurrency
  **NOT GREEN** with one medium allocation-amplification finding and two low
  documentation/evidence findings. Third remediation composes through
  `8777825`, `ab1c133`, `dcf57ad`, `d7526d4`, `44afb23`, `f08c5f2`, and
  `1f13f9a` at exact fully composed local-gate precursor `a8f6179`. Exact Rust
  1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation
  tests plus two doctests, 25 private native tests, 40 direct tests, four engine
  tests, and diff checks are green. Exact a8f cross-target/dependency/link and
  compatibility/release validators are green. Formal third-cycle candidate
  `0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
  **GREEN** with zero findings. Correctness/API and performance/concurrency are
  **NOT GREEN** only for the same LOW documentation contract mismatch;
  reviewers confirmed zero production defects. Isolated wording remediation
  `993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
  `f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
  green, and its behavior tree remains `a8f6179` except for documentation.
  Formal fourth-cycle exact behavior SHA
  `8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero
  findings in all three fresh tracks: correctness/API, filesystem/robustness,
  and performance/concurrency. Exact-SHA formatting, warnings-denied workspace
  Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25
  private tests, 40 direct tests, four engine tests, and the 58/420/270/0
  documentation inventory are green. All historical findings are closed,
  including the attempted-read-window storage wording. This documentation-only
  seal is exempt from another adversarial review under the user's explicit
  instruction. Documentation seal `0f48806` is feature-green: CI run
  `32623585346` passed all six jobs and benchmark-evidence run `32623585349`
  passed both jobs and artifacts, all for exact `0f`. `main` was fast-forwarded
  without force from `f6ab594` to exact `0f`. Main CI run `32623904784` is
  **GREEN** for exact `0f`: all six jobs and every step passed without reruns.
  Main benchmark-evidence run `32623904800` is **GREEN** on attempt 1 for exact
  `0f`: both jobs and every step passed, with two valid non-expired exact-SHA
  artifacts retained. The `grep_files` slice is delivered; the remaining native
  tools remain pending. This final delivery
  record is documentation-only and exempt from adversarial review; its own exact
  remote workflows are required after push and cannot be self-recorded. The
  twentieth `write_file` contract is frozen at exact `3ee52fd` from exact base
  `bc042536`. Production `e9b3ad8`, independent tests `59a06a3`, and maintained
  documentation `9285fe9` compose through `c0d555b`, `c4c5ce6`, and `de46c3e`;
  fixture correction `8509933`, deterministic seam hardening `a9a7c99`, and
  core same-poll recovery regression `8b5847f` are present; full-pipeline fault
  and phase evidence is `c7fdef2`. Exact local gates are green at composed
  precursor `072bd69`. Formal-review preparation is `a7841c1`; exact first
  candidate `119938240807f8279f83e2ace65a69706e8fcfed` is tree-identical only to
  that immediate parent, while `072bd69` has a different documentation tree.
  All three first-cycle tracks are **NOT GREEN**. Documentation correction
  `016f8df` and code/evidence remediation `3010e6d` close the unbounded `EINTR`,
  real-pipeline target/parent race, and verification-phase cancellation
  findings. Replacement exact local gates are green at `581fe6a`. Exact cycle-2
  candidate `708f2d08d72d610ca387a62a4cec1f656c188a7d` is tree-identical only
  to immediate formal-review preparation parent
  `491496aa22aa8855717b74f6a026e8c602bb02e9`. Correctness/API is **GREEN** with
  zero findings; filesystem/robustness is **NOT GREEN** with medium private-
  residue-mode and unbounded-Linux-entropy findings; and performance/concurrency
  is **NOT GREEN** with the same medium entropy finding. The production-used
  bounded entropy path and best-effort held-descriptor `0600` reset, including
  the restoration-failure caveat, remain pending. No behavior-green or
  replacement claim is made; a new exact candidate must repeat all three fresh
  tracks. Local
  evidence is native macOS, Linux and
  FreeBSD cross-compilation, and active WASI unsupported-target execution;
  exact feature CI native Linux/macOS remains pending. A later documentation-
  only seal or delivery record remains exempt from adversarial review under the
  user's instruction. The
  remaining native tools are
  incomplete, so this pending slice does not change the combined checkbox.
- [ ] Complete the M03 top-level CLI ownership from the pinned inventory:
  `help`, `ask`, `status`, `permissions`, `models`, `doctor`, `session`,
  `sessions`, `resume`, `replay`, and `workspace`. M03 also owns the pinned
  slash-command categories `general`, `session`, `model`, `security`, and
  `workspace`. Observable compatibility is scenario-based; command names may
  remain intentional differences when documented.
- [ ] Retain deterministic end-to-end evidence for the composed host with fake
  provider/prompt/network boundaries, exercise user-visible behavior through a
  freshly built release binary, resolve three fresh adversarial reviews, pass
  every required local and remote exact-SHA gate, and update the compatibility
  inventory/status without making a performance claim.

Ownership beyond that boundary is also fixed:

| Owner | Explicitly assigned work |
| --- | --- |
| M04 | Permission modes and identity-safe grant policy beyond `ask`; session schema migration and explicit legacy import; encryption, record authentication, key management, and secure erasure; persistence/lifecycle concurrency hardening; hardened non-Unix workspace and store construction. |
| M05 | Skills, MCP, ACP, and subagent infrastructure; top-level `acp`, `background`, and `teams`; extension/agent slash commands; built-in `memory`, `semantic_search`, `skill`, `install_skill`, `subagent`, `mcp_search_tools`, `mcp_select_tool`, and `mcp_features`. |
| M06 | SDK surfaces and remaining advanced compatibility, including top-level `pr`, `issue`, `login`, `logout`, `setup`, `credits`, `usage`, and `upgrade`, plus pinned account, media, product, and appearance slash-command categories. |
| M07 | Claim-eligible performance comparison, threshold enforcement, optimization, packaging evidence, and final hardening. Earlier milestones retain regression/size evidence needed by CI but make no product performance claim. |

Existing CLI bytes, benchmark evidence, workflows, and Zig inputs are unchanged
by the tenth through fifteenth slices, the delivered sixteenth through
eighteenth slices, and the nineteenth candidate; Zig remains only the pinned
upstream benchmark build input, not a machine-god product language or runtime
dependency. The provider is explicitly scoped to a pinned wire shape and makes
no current-protocol or full fx-equivalence claim. Help and status remain
claim-ineligible and unmeasured in bootstrap evidence.

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
