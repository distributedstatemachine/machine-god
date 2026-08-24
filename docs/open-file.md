# Native `open_file` contract

Status: **IMPLEMENTED CANDIDATE; FORMAL REVIEW PENDING**

This document defines the twenty-sixth bounded Milestone 03 candidate from exact
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

Current candidate source now implements the core capability, native Linux tool,
trusted launcher seam, unsupported-target behavior, tests, and twelve-tool host
composition without changing dependencies, workflows, CLI behavior, benchmark
workloads, or compatibility status. It has not yet completed formal review,
exact feature workflows, delivery, or `main` integration.

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
requested and canonical path are each capped at 4,096 UTF-8 bytes. The
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
pre-spawn cancellation check. A successful spawn begins a monotonic fixed
30-second wait. Exit status zero means only that the helper accepted the
request; it does not prove that another application started, retained access to
the proc path, rendered the file, or remained running.

Success is exactly:

```json
{"path":"canonical/relative/path"}
```

The helper path, PID, descriptor, launcher status, and environment are never
returned. The complete `ToolOutput` is defensively capped at 16,384 serialized
bytes.

## Commit, cancellation, timeout, and drop

Cancellation observed before a successful helper spawn wins and guarantees
zero helper launches. Execution checks before root acquisition, before and
after every retained open or validation operation, immediately before spawn,
and after a failed spawn. A failed spawn is therefore precommit; cancellation
observed around that failure takes precedence, otherwise it returns the fixed
retryable launcher-unavailable error.

Successful helper spawn is the commit boundary. From that instant, machine-god
cannot prove that the desktop open request had no effect. Cancellation after
that boundary causes the execution future's cleanup path to terminate and reap
the direct helper and join its owned worker. It cannot claim rollback or
relabel the committed effect as precommit cancellation. The engine's existing
turn cancellation remains authoritative and may discard the tool-level result.
Without cancellation, the tool waits for the helper until exit or the fixed
30-second timeout decision and retains the target descriptor throughout.

Exit zero within the bound returns success. Nonzero exit, signal termination,
timeout, or wait failure returns the same fixed redacted, nonretryable
`open_file_result_unknown` error. On timeout the owned helper is terminated and
reaped before return. The timeout decision occurs at 30 seconds; synchronous
termination, reap, and worker join may extend past that deadline. Any failure
to establish the owned waiter after a successful helper spawn is also
postcommit: machine-god terminates and reaps the helper and returns
`open_file_result_unknown`.

The execution future is inert until first poll. Dropping it before first poll
has no filesystem, thread, or process effect. Dropping it after successful
spawn synchronously signals the owned waiter, terminates and reaps the helper,
joins the owned worker, closes the retained target descriptor, and returns no
claim about whether the desktop request took effect. No owned child or worker
thread is detached on success, failure, cancellation, timeout, or drop. A
desktop application independently started by `xdg-open` is outside the owned
helper lifecycle and cannot be rolled back by this tool.

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

The delivered base reference host remains exactly
the eleven alphabetical tools: `copy_file`, `create_folder`, `delete_file`,
`edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`,
`read_file`, `rename_file`, and `write_file`, using the original retained
descriptor plus ten identity-preserving clones.

Current candidate composition inserts `open_file` after
`list_files` and before `read_file`, yielding exactly twelve alphabetical tools
and using one original retained descriptor plus eleven identity-preserving
clones. Both path-based and prepared-root reference-host constructors compose
the same catalog and retained workspace identity. This composition is not yet
reviewed or delivered; `main` remains at eleven tools.

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

## Candidate evidence and remaining gates

- [x] Exact core variant/serde/drop contract, native exports, constants,
  descriptions, strict schema, construction taxonomy, result, errors, and
  redaction.
- [ ] Exact and one-over 4,096-byte requested/canonical path, 256-component,
  255-byte component, 65,536-byte argument, and 16,384-byte result bounds.
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
- [x] Missing launcher and spawn failure before commit; nonzero, signal,
  timeout, wait, and waiter-establishment failure after commit; exact fixed
  retryability and `result_unknown` classification.
- [x] Inert until poll; cancellation before spawn with zero launch; successful-
  spawn boundary; postspawn cancellation through the engine's existing drop
  path; 30-second timeout decision plus complete cleanup; pre-poll and
  postspawn drop; terminate/reap/join; no detached owned child or worker;
  concurrent-call isolation.
- [x] Candidate macOS active unsupported behavior, Linux cross-target warnings-
  denied compilation, exact twelve-tool/eleven-clone composition, and no new
  dependency, workflow, CLI, benchmark, or unsafe-Rust source.
- [ ] Exact composed-SHA native Linux execution, FreeBSD/WASI and active WASI,
  dependency, pinned-compatibility, documentation, clean-diff, and freshly
  built release-binary evidence; then three green formal review tracks and the
  exact remote delivery workflows.

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
exact workflows remain required. The pre-created pending ledger is
[`m03-open-file-review-01.md`](reviews/m03-open-file-review-01.md).

## Deferred scope

External, absolute, home-relative, and parent-traversing paths; directories;
URLs; symlink targets; content reads; file mutation; arbitrary process
authority; shell execution; model-selected programs, arguments, environment,
or working directories; PATH lookup; macOS or other non-Linux real launch;
CLI ownership; new benchmark workloads; product-performance claims; inventory
promotion; and complete fx equivalence remain outside this slice.
Formal review, exact feature workflows, integration, and delivery remain
pending; current implementation and local results are candidate evidence only.
