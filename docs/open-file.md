# Native `open_file` contract

Status: **DELIVERED**

This document records the twenty-sixth bounded Milestone 03 slice from exact
delivered base `e2ee11f2c728721d2aa93219b5fafa86ea15b0c4`. That base is green
under exact main CI `32704202572` and exact main benchmark workflow
`32704202546`. The benchmark workflow passed both jobs and retains exactly two
nonexpired exact-SHA artifacts, IDs `9511626648` and `9511745538`.

The earlier checkpoint froze documentation only and added no behavior. Its
contract commit is exempt from adversarial review under the user's instruction,
but its
exact feature CI `32707583915` subsequently passed all six jobs. Exact feature
benchmark workflow `32707583892` passed both jobs and retains exactly two
nonexpired exact-SHA artifacts, IDs `9512848704` and `9512966283`. These
workflows validate only that frozen contract checkpoint at
`6b763c4f1168963dd42087a1fdf5cf72c4212b40`; they are not implementation,
delivery, performance, or fx-equivalence evidence.

Delivered source implements the core capability, native Linux tool,
trusted launcher seam, unsupported-target behavior, tests, and twelve-tool host
composition without changing dependencies, workflows, CLI behavior, benchmark
workloads, or compatibility status. Formal review cycle 1 rejected exact
candidate `79e65c19330181955a0c341d62ef39778a18d36d`, tree
`481fd7c2968f32d3b51f82cbb46a1bd6c7edeb18`. Formal review cycle 2 rejected
exact candidate `027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`; its resource-bound, deadline,
lifecycle, test-fidelity, and frozen-contract findings are recorded below and
in the review ledger. Formal review cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`: correctness/API was green with
zero findings, while performance/concurrency and filesystem/process-lifecycle
each reported one low evidence/lifecycle gap. There were zero blocker, high,
or medium findings, zero other findings, and no production resource escape.
Candidate remediation passed the complete replacement gate in exact cycle-4
candidate `4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`. Filesystem/process-lifecycle and
performance/concurrency were green with zero findings. Correctness/API found no
production defect and one low maintained-documentation lineage drift, so cycle
4 is not green. That documentation remediation was composed into exact cycle-5
candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks rejected cycle 5
for the same low stale current-lineage wording and found zero production
defects. That documentation correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are **GREEN**
with zero findings at every severity. Exact feature workflows, delivery, and
`main` integration are complete on the seal recorded below.

`open_file` asks the fixed Linux desktop launcher to open one existing regular
file selected beneath the retained workspace root. It does not read or mutate
the file, accept a directory, URL, external path, parent traversal, or symlink,
select an application, run a shell, or prove that a graphical application
displayed the file. The tool is library-only in this slice. The product remains
Rust; Zig remains solely a pinned upstream benchmark build input.

## Public API and schema

The `machine-god-core` API adds the dedicated
`Capability::OpenFile { path: String }` variant. `machine-god-native` exports
`OPEN_FILE_TOOL_NAME`,
`OpenFileTool`, `OpenFileToolOpenError`, `OpenFileToolOpenErrorKind`, and these
limits. On Linux it additionally exports the trusted deterministic-test seam
`OpenFileLauncher`, `OpenFileLaunch`, `OpenFileLaunchRequest`, and
`OpenFileLaunchOutcome`. The production constructor always installs the fixed
system launcher; only the explicit `open_with_launcher` constructor accepts a
trusted host implementation, whose effect-free construction, cancellation,
drop, ownership, and outcome obligations are part of the trait contract.

| Public constant | Exact value |
| --- | ---: |
| `OPEN_FILE_TOOL_NAME` | `"open_file"` |
| `MAX_OPEN_FILE_PATH_BYTES` | `4,096` |
| `MAX_OPEN_FILE_PATH_COMPONENTS` | `256` |
| `MAX_OPEN_FILE_PATH_COMPONENT_BYTES` | `255` |
| `MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |
| `MAX_CONCURRENT_OPEN_FILE_LAUNCHES` | `32` |
| `OPEN_FILE_LAUNCH_TIMEOUT` | `std::time::Duration::from_secs(30)` |

The exact tool description is
`Open one existing regular file within the configured workspace in the desktop default application`.
The exact `path` property description is
`Workspace-relative regular-file path to open`.

The exact input schema is:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative regular-file path to open"
    }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

`path` is a required string with no default. Unknown fields are invalid. The
requested and canonical path are each capped at 4,096 UTF-8 bytes. Shape and
borrowed path length are checked before any complete-value serialization, so a
hostile over-bound path cannot cause unbounded pre-path serialization inside
the tool. Only an already path-bounded value may be measured against the
65,536-byte serialized-argument cap. The
canonical path contains at most 256 components, and every component contains
at most 255 UTF-8 bytes. Complete requested and prepared JSON values are each
capped at 65,536 serialized bytes. Direct execution revalidates the exact
shape, bounds, and canonical representation.

Only a byte-for-byte canonical relative spelling is accepted. Components are
separated by one `/`. Empty components, repeated separators, a leading `./`,
any exact `.` or `..` component, and a trailing separator are ambiguous and
reject rather than normalize. Empty input, canonical `.`, a root path,
absolute paths, `~`-prefixed paths, and parent traversal reject. C0/C1 control
characters and the exact Unicode set U+061C, U+200E, U+200F, U+2028 through
U+202E, and U+2066 through U+2069 reject. Unicode is otherwise neither
normalized nor case-folded; its accepted UTF-8 byte spelling is identity.
Backslash and space remain literal Linux filename characters.

Construction accepts one injected absolute workspace directory. On Linux it
lexically removes redundant separators and exact `.` components from that
host-selected root, opens the final root component no-follow, requires a real
directory, and retains its descriptor. Model input cannot select or reopen the
root. Other targets return the fixed unsupported construction result; any
target-independent execution seam also returns unsupported before filesystem
lookup, worker creation, or helper spawn.

Construction errors retain only their kind:

| Kind | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native open_file is unsupported on this platform` |
| `InvalidRoot` | `native open_file workspace root is invalid` |
| `InvalidFileType` | `native open_file workspace root is not a directory` |
| `Unavailable` | `native open_file workspace root is unavailable` |

`Display` and `Debug` never retain the requested path, canonical path, injected
root, process ID, descriptor number, helper command, environment, operating-
system text, or raw error number.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs no filesystem lookup, metadata read, descriptor open,
`/proc` access, environment read, worker creation, or process launch.
Successful preparation retains exactly the accepted canonical path and returns:

```text
Capability::OpenFile {
    path: canonical_path,
}
```

Its stable serialized permission input is exactly:

```json
{"type":"open_file","path":"canonical/path"}
```

Policy and allowed execution receive the same canonical path. Denial or failed
preparation has no filesystem or process effect. This capability authorizes
only the bounded retained-root lookup and one fixed launcher attempt described
below. It grants no general `FilesystemAccess::Read`, content access, metadata
enumeration, mutation, external path, arbitrary `Capability::Process`, shell,
program, argument, environment, or working-directory authority.

The dedicated variant is required because opening a desktop application is not
a content read and the existing general `Capability::Process` would expose
broader authority than the model-selected path. Core owns its stable serde
shape, exhaustive internal drop handling, and permission evidence. Native code
owns all filesystem and launcher effects.

## Retained-target validation

Allowed execution is concrete only on Linux. The injected root pathname is
never reopened as authority. Before any worker or helper process exists, one
call performs this sequence:

1. Check cancellation, acquire `.` descriptor-relatively from the retained
   root, and validate that exact acquired workspace identity as a linked
   directory.
2. Walk every ancestor from that descriptor using directory, no-follow,
   nonblocking, and close-on-exec opens. Each retained ancestor is the only base
   for the next component. A symlink or non-directory ancestor fails closed.
3. Open the final component descriptor-relatively, no-follow, nonblocking, and
   close-on-exec. Require the retained final descriptor to identify a linked
   regular file. A directory, symlink, FIFO, socket, device, or other special
   object rejects without launching.
4. Derive the launcher target solely from trusted process state as
   `/proc/<machine-god-parent-pid>/fd/<retained-target-fd>`, using unsigned
   decimal PID and descriptor spellings without leading zeroes. Validate that
   the proc descriptor entry is available, then make the final pre-spawn
   cancellation check.

At most 256 model-selected components are opened. The tool never reads file
content, resolves a selected symlink, or constructs the helper target from raw
model bytes. The retained target descriptor closes on exec and remains owned by
machine-god until the helper has been waited for, terminated, or reaped.

Replacing an unopened component may change which regular file is selected at
the authorized canonical path. Once its final descriptor is retained, later
rename, unlink, or pathname replacement cannot redirect the helper target.
An unlinked retained target may still be accepted through its descriptor. This
is path authority at execution time, not an inode promise from preparation
time and not a continued-path-existence promise after return.

## Fixed Linux launcher protocol

The production launcher is exactly `/usr/bin/xdg-open`; machine-god never
discovers it through `PATH`. No shell is invoked by machine-god. The exact
two-element argument vector is:

```text
["/usr/bin/xdg-open", "/proc/<parent-pid>/fd/<target-fd>"]
```

Standard input, output, and error are all connected to the null device. The
helper's working directory is fixed to `/`. It inherits the host process
environment needed for the desktop session unchanged, but neither provider nor
model input can select, add, remove, or rewrite a program, argument,
environment entry, or working directory. The absolute launcher installation,
`/proc` mount, desktop-session environment, and default-application behavior
are trusted host boundaries. `xdg-open` and downstream desktop dispatch may
themselves consult inherited `PATH`, configuration, or other host state.

The launcher boundary is injected for deterministic tests. Constructing the
tool or execution future does not call that boundary. First poll performs
preflight execution work, and no worker or helper is started before the final
pre-spawn cancellation check. A production system-launch future then acquires
one process-global launch permit immediately before worker creation. Exactly
`MAX_CONCURRENT_OPEN_FILE_LAUNCHES = 32` permits exist. Saturation is a
retryable precommit launcher-unavailable result with zero new worker or helper;
it does not wait, queue, or consume an unbounded thread. The acquired permit
moves into the worker and is retained through request/descriptor and helper
cleanup, outcome publication, the arbitrary Waker callback, final notification
bookkeeping, and worker return. The worker's final spawn attempt and the
cancellation/drop abort transition share one serialized state gate. Whichever
transition obtains that gate first linearizes: abort recorded first guarantees
zero launch; a successful spawn while the worker owns the gate commits the
effect. A successful spawn immediately starts a monotonic fixed 30-second
deadline. Every wait probe is followed by an authoritative monotonic-clock
read. Even if that probe reports exit zero, acceptance is permitted only while
that read is strictly before the deadline; at or after the deadline, timeout
wins. Each polling sleep is
`min(5ms, deadline.saturating_duration_since(now))`, so no full polling interval
is added after the remaining budget. Exit status zero accepted within that
boundary means only that the helper accepted the request; it does not prove
that another application started, retained access to the proc path, rendered
the file, or remained running.

Success is exactly:

```json
{"path":"canonical/relative/path"}
```

The helper path, PID, descriptor, launcher status, and environment are never
returned. The complete `ToolOutput` is defensively capped at 16,384 serialized
bytes.

## Commit, cancellation, timeout, and drop

The final helper-spawn attempt and cancellation/drop abort transition share one
serialized state gate. If cancellation or drop records abort through that gate
first, it wins and guarantees zero helper launches. If the worker owns the gate
and successfully spawns first, that spawn wins and commits. Execution checks
before root acquisition, before and after every retained open or validation
operation whether that operation succeeds or fails, immediately before entering
the spawn gate, and after a failed spawn. A failed spawn is therefore precommit;
cancellation observed around that failure takes precedence, otherwise it
returns the fixed retryable launcher-unavailable error.

Successful helper spawn is the commit boundary. From that instant, machine-god
cannot prove that the desktop open request had no effect. Cancellation after
that boundary causes the execution future's cleanup path to terminate and reap
the direct helper. It normally joins its owned worker after notification has
finished. It cannot claim rollback or
relabel the committed effect as precommit cancellation. The engine's existing
turn cancellation remains authoritative and may discard the tool-level result.
Without cancellation, the tool waits for the helper until exit or the fixed
30-second timeout decision and retains the target descriptor throughout.

Exit zero observed by a wait probe and confirmed by its authoritative
post-probe clock read strictly before the deadline returns success. A status
observed at or after the deadline, nonzero exit, signal termination, timeout,
or wait failure returns the same fixed redacted, nonretryable
`open_file_result_unknown` error. On timeout or wait failure the owned helper is
terminated and reaped before return. The timeout decision occurs at 30 seconds;
synchronous termination, reap, and normal nonreentrant worker join may extend
past that deadline. Worker creation occurs before spawn, so there is no
postspawn waiter-establishment state or error.

The execution future is inert until first poll. Every trusted fake launcher
used as evidence is inert as well: invoking its launch constructor records no
request and performs no effect until the returned launch future is first
polled. Dropping either future before first poll has no filesystem, request-
recording, thread, or process effect.

Before outcome publication, cancellation or drop suppresses the registered
Waker, synchronously terminates and reaps any helper, drops the request and its
retained target descriptor, joins the worker, and releases its permit. Helper
reap and request/descriptor drop are required before any outcome is published.
After publication, the normal nonreentrant completion/drop path synchronously
joins the worker. A valid arbitrary Waker may instead repoll inline on the
worker, or block while another thread drops the future. Joining in either
overlap would self-join or form an executor-lock cycle, so that path releases
the `JoinHandle`; the callback and final notification bookkeeping may outlive
future drop. At that point the helper is reaped, the request and retained
descriptor are dropped, and only callback/final bookkeeping remains. The
system-launch permit is nevertheless retained until that callback completes
and the worker returns, so these tails are globally bounded to 32. No owned
helper process is detached. A desktop application
independently started by `xdg-open` is outside the owned helper lifecycle and
cannot be rolled back by this tool.

### Cycle-2 lifecycle contract amendment

The preceding rule replaces only the original frozen contract checkpoint
`6b763c4f1168963dd42087a1fdf5cf72c4212b40` clause that no owned worker could
ever be detached and that every drop synchronously joined it. Legal executor
Wakers may repoll inline or block on executor-owned locks, making that absolute
worker-join invariant contradictory: joining can self-deadlock or create a
cross-thread lock cycle. The amended invariant keeps synchronous join for all
unpublished work and the normal published path, permits only the bounded
postpublication callback/bookkeeping tail above, and requires the fixed permit
to cover its complete lifetime. No other frozen authority, confinement,
helper-reap, result, or delivery boundary is reopened. This documentation-only
contract amendment is exempt from its own adversarial review under the owner's
instruction; replacement production and evidence remain subject to a complete
fresh three-track same-SHA cycle.

## Fixed tool errors

All failures are fixed and redacted:

| Code | Kind | Retryable | Exact message |
| --- | --- | --- | --- |
| `open_file_invalid_arguments` | `InvalidInput` | no | `open_file arguments are invalid` |
| `open_file_invalid_path` | `InvalidInput` | no | `open_file path is invalid` |
| `open_file_unsupported_platform` | `Unavailable` | no | `native open_file is unsupported on this platform` |
| `open_file_not_found` | `Unavailable` | no | `requested file is unavailable` |
| `open_file_permission_denied` | `PermissionDenied` | no | `requested file cannot be opened` |
| `open_file_path_rejected` | `PermissionDenied` | no | `requested file path is not confined` |
| `open_file_not_regular_file` | `Execution` | no | `requested path is not a regular file` |
| `open_file_unavailable` | `Unavailable` | yes | `requested file is unavailable` |
| `open_file_launcher_unavailable` | `Unavailable` | yes | `native file launcher is unavailable` |
| `open_file_result_unknown` | `Execution` | no | `requested file open status is uncertain` |
| `open_file_cancelled` | `Cancelled` | no | `open_file execution was cancelled` |

`open_file_not_found`, confinement rejection, nonregular-target rejection, root
or proc unavailability, and launcher spawn failure are precommit and guarantee
zero launch. Raw path bytes, root, PID, descriptor, launcher argv, environment,
exit status, signal, timeout detail, wait diagnostic, operating-system text,
and errno are never retained by public errors. Engine-facing non-cancellation
failures remain the delivered generic durable tool-error surface.

## Races and host boundary

Retained no-follow descriptors prevent replacement of already-opened ancestors
or the final target from redirecting later steps. They do not make the
workspace a filesystem transaction or sandbox. An actor can replace a
component before its lookup, remove the retained file, or change file contents
through another descriptor. The launcher is authorized for the retained file
identity selected at the canonical path; the tool promises neither stable
contents nor a pathname that remains present.

`xdg-open` and any desktop application it starts are host programs outside the
provider-neutral core. They may inspect file metadata or content, consult host
configuration, communicate with a desktop session, and outlive the owned
helper. The tool controls only its retained descriptor and direct helper. The
contract is not a sandbox guarantee and makes no claim about third-party
application behavior.

## Host composition and compatibility boundary

Before `open_file` delivery, the delivered base reference host contained
exactly
the eleven alphabetical tools: `copy_file`, `create_folder`, `delete_file`,
`edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`,
`read_file`, `rename_file`, and `write_file`, using the original retained
descriptor plus ten identity-preserving clones.

The delivered composition inserts `open_file` after
`list_files` and before `read_file`, yielding exactly twelve alphabetical tools
and using one original retained descriptor plus eleven identity-preserving
clones. Both path-based and prepared-root reference-host constructors compose
the same catalog and retained workspace identity. Formal cycle 5 rejected exact
candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`, for one low documentation-lineage
finding, so this composition is reviewed but not delivered; `main` remains at
eleven tools.

Exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`, is green with zero findings in all
three fresh tracks. The twelve-tool composition is therefore review-green but
is now delivered on `main`.

Pinned fx at `b1774fbf6c7602b503026f96f6e960e946c692ef` uses the same tool
name and required `path` field, marks the operation approval-required,
side-effecting, and reversible, and launches `xdg-open` on Linux or `open` on
macOS. It resolves workspace-relative and policy-approved external paths,
discovers the launcher by program name, passes an absolute pathname, captures
helper output, waits without this explicit 30-second contract, returns text,
and supports directories as existing targets.

Machine-god intentionally narrows that behavior to a strict confined existing
regular file, Linux-only fixed absolute launcher, retained-descriptor proc
target, null stdio, bounded wait, owned drop cleanup, dedicated authority,
structured result, and fixed redacted errors. External paths, symlink following,
directories, macOS launch, PATH lookup, and equivalence promotion remain
deferred. Zig is benchmark input only.

## Formal review cycle 3: not green

All three fresh tracks reviewed exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`. The cycle is **NOT GREEN** and
that candidate is rejected for delivery.

- Correctness/API is **GREEN** with zero findings.
- Performance/concurrency is **NOT GREEN** with one low evidence finding. The
  deterministic deadline regression stopped at the pre-probe deadline guard,
  so it did not exercise the authoritative clock read after `try_wait` returns.
  Replacement evidence must pause after the wait probe and prove an exit-zero
  status observed at or after the deadline is rejected as timeout.
- Filesystem/process-lifecycle is **NOT GREEN** with one low lifecycle/evidence
  finding. In the no-Waker publication gap, the completion flag was not yet
  visible when the future observed a ready outcome, so ordinary cleanup could
  detach the worker tail rather than take the required normal join path.
  The tail remained globally permit-bounded and the helper, launch request, and
  retained target descriptor were already cleaned; the reviewers found no
  production resource escape. Remediation requires atomic
  `notification_complete` publication and a deterministic no-Waker regression
  proving ready consumption takes the normal join path.

The cycle has zero blocker, high, or medium findings, zero other findings, and
no production resource escape. These findings do not weaken the authorized
documentation-only lifecycle amendment, but they reject this implementation
candidate. Candidate remediation now atomically completes no-Waker publication
and supplies both deterministic regressions.

## Formal review cycle 4: not green

All three fresh tracks reviewed exact candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`. The complete replacement local
gate was green, but the cycle is **NOT GREEN** and that candidate is rejected
for delivery.

- Filesystem/process-lifecycle is **GREEN** with zero findings.
- Performance/concurrency is **GREEN** with zero findings.
- Correctness/API found no production or public-API defect and reported one low
  maintained-documentation lineage drift: `architecture.md`, `core-api.md`,
  `native-reference-host.md`, and `security.md` still described cycle-2
  remediation, omitted rejected cycle 3, or called the composition unreviewed.

Cycle 4 has zero blocker, high, or medium findings and one low documentation
finding. The four maintained summaries now record the exact cycle-3 lineage,
its two low findings, the cycle-4 remediation and verdict, and reviewed-but-
rejected composition status.

## Formal review cycle 5: not green

All three fresh tracks reviewed exact candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. The complete replacement gate was
green, but the cycle is **NOT GREEN** and that candidate is rejected.

- Correctness/API, filesystem/process-lifecycle, and performance/concurrency
  found zero production, API, lifecycle, performance, concurrency, or resource-
  bound defects.
- All three tracks reported the same low maintained-documentation lineage
  defect: README still called cycle 3 the latest rejection, the host-
  composition paragraph called the cycle-3 candidate current, and the contract
  and ledger used generic pending-review wording after four completed cycles.
- Exact native Linux arm64 Rust 1.94.1 evidence was green at system 14/14,
  direct 12/12, engine 4/4, warnings-denied Clippy, and five repeated lifecycle
  runs totaling 70/70.

At the cycle-5 checkpoint, the stale passages were corrected and a fresh
three-track cycle 6 on one immutable replacement SHA/tree remained required;
no green-review claim was made at that checkpoint.

## Formal review cycle 6: green

All three fresh tracks reviewed exact candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. Correctness/API,
filesystem/process-lifecycle, and performance/concurrency are **GREEN** with
zero findings at every severity.

Native Linux arm64 Rust 1.94.1 evidence passed system 14/14, direct 12/12,
engine 4/4, and warnings-denied Clippy. Performance/concurrency repeated its
focused lifecycle matrix for 70/70 passes. Correctness/API additionally passed
core serde 1/1, macOS active unsupported behavior 1/1, and all-feature host
composition 1/1.

The exact candidate also passes workspace formatting, warnings-denied Clippy,
tests, doctests, and no-run compilation; 130 Python tests with eight expected
macOS skips; byte-identical compatibility against pinned fx `b1774f`; and
`cargo-deny` 0.20.2 plus `cargo-audit` 0.22.2 with zero findings. FreeBSD
compilation, WASI compilation and active Node evidence 1/1, documentation
checks, and release smokes are green. The fresh 319,152-byte release binary has
SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.

Seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
`03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
at 6/6 and feature benchmark `32738160725` at 2/2. That benchmark retains
upstream artifact `9524219365` and bootstrap artifact `9524052760`. Exact main
CI `32738798417` passed 6/6, and main benchmark `32738798415` passed 2/2 while
retaining upstream artifact `9524461989` and bootstrap artifact `9524298408`.
Feature delivery, non-force fast-forward integration, and exact `main`
workflows are complete. The current host has exactly twelve alphabetical tools
using one retained descriptor plus eleven identity-preserving clones. This
makes no product-performance or fx-equivalence claim. At that checkpoint, the
final docs-only record was exempt from adversarial review under the user's
instruction; its own exact feature and `main` workflows remained required and
are reported below.

Final delivery-record SHA
`762d70df106d40e59b599e18b1ac5c62f678927d`, tree
`909eb320e05df4d56f5bcecf0e3655e6d761f622`, passed feature CI `32740668405`
at 6/6 and feature benchmark `32740667465` at 2/2, retaining upstream artifact
`9525188220` and bootstrap artifact `9525017236`. Main benchmark `32741322179`
passed 2/2 and retained upstream artifact `9525436660` and bootstrap artifact
`9525268460`. Main CI `32741322249` was not green: five of six jobs passed,
and Quality alone failed the exact test named by concatenating
`blocked_wake_releases_request_before_publication_and_holds_` with
`permit_until_worker_return` because the immediate `exit 0` fixture could
legitimately publish before the first poll installed `BlockingWake`. This is a
test-fixture synchronization defect, not a production behavior finding. Exact
local test-only remediation
`62c2a5349bc682079c2458ccebe9f9ea9578a3c1`, tree
`b38984441b6bb470ecb4b1c69bc9a3a9984f0bb0`, adds the existing
`before_first_wait` barrier and passed the normal native Linux arm64 exact test
100/100. Exact cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, received **GREEN** correctness/API
and filesystem/process-lifecycle reviews with zero findings. Performance/
concurrency was **NOT GREEN** with exactly two low findings and zero blocker,
high, or medium findings: the unconditional `before_first_wait` rendezvous
could hang when `Command::spawn` failed before the hook, as reproduced on
native Linux arm64 with `/tmp` mounted `noexec`; and maintained current/
operative documentation tails ended at the superseded cycle-6/handoff state.
The candidate is rejected. Exact test-only code remediation
`274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the existing
`before_spawn` barrier, reached before every spawn outcome, so Waker
registration deterministically precedes publication. The normal native Linux
arm64 exact test passed 100/100, and the `/tmp`-noexec spawn-failure case passed
1/1. Production source, public API, and manifests are unchanged. This docs
correction composes atop that commit; its SHA is pending. The full replacement
local gate and all three fresh cycle-8 tracks remain pending. This executable
test-only fix is not eligible for the documentation-only exemption. This makes
no product-performance or fx-equivalence claim.

## Evidence and delivery gates

- [x] Exact core variant/serde/drop contract, native exports, constants,
  descriptions, strict schema, construction taxonomy, result, errors, and
  redaction.
- [x] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds;
  a much larger hostile path proves rejection occurs before complete-value
  serialization or another path-proportional copy.
- [x] Rejection of empty/root/dot, absolute, tilde, parent, repeated/trailing
  separator, dot-component, control, line/paragraph-separator, bidirectional,
  and over-bound paths; byte-for-byte canonical policy/execution agreement.
- [x] Effect-free preparation, exact
  `{"type":"open_file","path":"..."}` authority, denial before lookup,
  direct canonical revalidation, and absence of general process authority.
- [x] Retained-root liveness, no-follow ancestor/final traversal, regular-file-
  only enforcement, every symlink/special/directory rejection, root/prefix/
  final replacement, unlink, rename, mixed-device traversal, and outside
  sentinels.
- [x] Controlled Linux-only source evidence for exact `/usr/bin/xdg-open` and
  two-element proc-fd argv, fixed `/` cwd,
  inherited host environment, null stdio, no machine-god shell/PATH or model-
  selected launch field, retained target descriptor, trusted downstream host
  dispatch, and exit-zero acceptance semantics.
- [x] Identity-aware proc-entry closure evidence tracks the exact descriptor
  identity rather than treating reuse of the same numeric fd as continued
  ownership.
- [x] Missing launcher and spawn failure before commit; nonzero, signal,
  timeout, and a wait seam that drives the actual shared `try_wait` `Err` arm
  after commit; exact fixed retryability, `result_unknown`, helper reap, and no
  impossible postspawn waiter-establishment claim.
- [x] Inert production and fake launcher futures until first poll; a
  deterministic spawn-gate barrier proving cancellation
  that wins the serialized gate has zero launch; successful-spawn boundary;
  postspawn cancellation through the engine's existing drop path; 30-second
  authoritative post-probe deadline decision, remaining-budget sleep, and
  post-deadline exit-zero rejection through a deterministic pause after the
  wait probe; pre-poll and postspawn drop; terminate/reap; atomic
  `notification_complete` publication and a deterministic no-Waker normal
  join; inline reentrant-waker completion
  without self-join panic or deadlock; overlapping blocked-waker drop without a
  cross-thread join cycle; exact request/descriptor identity closed before
  publication while that Waker remains blocked; 32 active system launches
  saturate with the next call precommit unavailable and zero new worker/helper;
  permits remain charged through blocked callback completion; concurrent-call
  isolation.
- [x] Candidate macOS active unsupported behavior, Linux cross-target warnings-
  denied compilation, exact twelve-tool/eleven-clone composition, and no new
  dependency, workflow, CLI, benchmark, or unsafe-Rust source.
- [x] Composed-remediation native Linux execution, FreeBSD/WASI and active WASI,
  dependency, pinned-compatibility, documentation, clean-diff, and freshly
  built release-binary evidence.
- [x] Three green formal review tracks on one immutable cycle-6 SHA/tree.
- [x] Exact feature workflows, fast-forward delivery, and exact `main`
  workflows.

## Review and delivery protocol

After candidate source and independently owned evidence compose, run the complete
local gate on one exact SHA. Create a tree-identical candidate and start three
fresh reviewers against that same immutable SHA and tree:

1. correctness/API;
2. filesystem/process-lifecycle robustness;
3. performance/concurrency.

Every confirmed finding is fixed, the complete local gate is rerun, and all
three tracks restart with fresh reviewers on one replacement SHA. Repeat until
all three tracks report zero findings. Then push the feature seal, require its
exact CI and benchmark workflows, fast-forward `main` without force, and
require exact `main` CI and benchmark workflows. Documentation-only seal and
delivery-record commits are exempt from another adversarial cycle, but their
exact workflows remain required. The review ledger is
[`m03-open-file-review-01.md`](reviews/m03-open-file-review-01.md).

## Deferred scope

External, absolute, home-relative, and parent-traversing paths; directories;
URLs; symlink targets; content reads; file mutation; arbitrary process
authority; shell execution; model-selected programs, arguments, environment,
or working directories; PATH lookup; macOS or other non-Linux real launch;
CLI ownership; new benchmark workloads; product-performance claims; inventory
promotion; and complete fx equivalence remain outside this slice.
Formal cycle-6 review is green on the exact SHA/tree recorded above. Exact
feature workflows, integration, and delivery are complete on the seal recorded
above. Native `open_file` remains delivered as bounded slice twenty-six.
Subsequent cycle-7 test-reliability candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, is rejected with two low
performance/concurrency findings and zero blocker, high, or medium findings;
correctness/API and lifecycle are green at zero findings. Exact test-only code
remediation `274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, uses the existing `before_spawn`
barrier and leaves production source, public API, and manifests unchanged. Its
docs correction SHA, full replacement local gate, and all three fresh cycle-8
tracks remain pending. This makes no product-performance or fx-equivalence
claim.
