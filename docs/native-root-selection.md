# Native root selection and preparation

Status: candidate contract for the fourteenth bounded Milestone 03 slice.
Thirteen slices are integrated through final delivery record
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact feature CI run
`32583585145`, feature benchmark-evidence run `32583585148`, main CI run
`32583871385`, and main benchmark-evidence run `32583871368` are green for that
record. This candidate's production implementation, independent tests, three
fresh adversarial tracks, exact feature workflows, fast-forward integration,
and exact `main` workflows remain pending. Milestone 03 remains `IN PROGRESS`.

This slice adds an explicit Linux/macOS library boundary for selecting and
retaining the workspace and state roots required by native reference-host
composition. Selection is pure path derivation from caller-owned inputs.
Preparation is the only new root-creation authority, and it can create only a
fixed state suffix beneath an already existing selected base. Core and the CLI
receive no environment or root-creation authority.

## Selection

The public selection surface is:

```rust,ignore
NativeRootSelection::from_environment(
    environment: &NativeEnvironment,
    workspace_root: &Path,
) -> Result<NativeRootSelection, NativeRootSelectionError>
NativeRootSelection::workspace_root(&self) -> &Path
NativeRootSelection::state_root(&self) -> &Path
NativeRootSelectionError::kind(&self) -> NativeRootSelectionErrorKind
NativeRootSelectionErrorKind::as_str(self) -> &'static str
```

`NativeRootSelection::from_environment(&NativeEnvironment, &Path)` consumes no
ambient process state. The caller supplies the environment snapshot and the
workspace path. The workspace must be absolute and contain no lexical `..`
component. Selection rebuilds the path from its lexical components but does not
require it to exist yet, inspect it, canonicalize it, open it, or create it.
The workspace is an operating-system path and need not be Unicode; only the
environment-derived state selection has the Unicode requirement below.

State selection exactly reuses the existing state-location precedence:

- a nonempty `XDG_STATE_HOME` is selected and must be absolute Unicode;
- an empty or absent `XDG_STATE_HOME` falls back to a nonempty, absolute-Unicode
  `HOME`;
- an invalid selected nonempty `XDG_STATE_HOME` fails without falling back;
- a missing or empty needed `HOME` makes state selection unavailable; and
- a selected nonempty relative or non-Unicode `HOME` is invalid.

The selected state root path is exactly
`<XDG_STATE_HOME>/machine-god`, or
`<HOME>/.local/state/machine-god` for the fallback. The resolved path is not
derived from configuration data or native status. `workspace_root()` and
`state_root()` expose the selected paths for host observation. The selection is
cloneable and equality-comparable. Its debug output is exactly
`NativeRootSelection { .. }` and reports no path or environment value.

Selection has three fixed, redacted categories:

| `NativeRootSelectionErrorKind` | Stable name | Exact `Display` |
| --- | --- | --- |
| `InvalidWorkspaceRoot` | `invalid_workspace_root` | `native workspace root selection is invalid` |
| `StateRootUnavailable` | `state_root_unavailable` | `native state root selection is unavailable` |
| `InvalidStateEnvironment` | `invalid_state_environment` | `native state environment selection is invalid` |

`InvalidWorkspaceRoot` covers a relative path or any lexical parent component.
`StateRootUnavailable` means no nonempty XDG value or fallback `HOME` is
available. `InvalidStateEnvironment` means the selected nonempty state value is
relative or non-Unicode.

The error retains only its kind. `Debug` is exactly
`NativeRootSelectionError { kind: ... }`; `Display` is the corresponding table
entry. It has no nested source and reflects no workspace path, state path,
environment value, operating-system text, or raw error number.

## Descriptor-rooted preparation

The public preparation surface is:

```rust,ignore
PreparedNativeRoots::prepare(
    selection: NativeRootSelection,
) -> Result<PreparedNativeRoots, PreparedNativeRootsError>
PreparedNativeRoots::selection(&self) -> &NativeRootSelection
PreparedNativeRoots::workspace_root(&self) -> &Path
PreparedNativeRoots::state_root(&self) -> &Path
PreparedNativeRootsError::kind(&self) -> PreparedNativeRootsErrorKind
PreparedNativeRootsErrorKind::as_str(self) -> &'static str
```

`PreparedNativeRoots::prepare(NativeRootSelection)` is synchronous. On
supported Linux and macOS targets it performs these ordered operations:

1. open the selected existing absolute workspace without following its final
   component, require the opened object to be a directory, and retain that
   descriptor;
2. open the selected existing absolute state base without following its final
   component and require a directory;
3. beneath that retained base descriptor, walk or create only the selected
   fixed suffix, opening every existing or newly created component relative to
   its parent without following symlinks;
4. validate the ownership and permission rules for the selected state base and
   every retained suffix component; and
5. compare the retained workspace and final state-root identities and reject
   equality or either ancestor relationship.

The state base is `XDG_STATE_HOME` on the XDG path and `HOME` on fallback. It
must already exist; preparation never creates, replaces, repairs, or chmods it.
The only creatable components are the single XDG suffix `machine-god`, or the
three fallback suffix components `.local`, `state`, and `machine-god`. The
component names are constants, never configuration, environment, CLI, or model
input. `..`, alternate suffixes, and arbitrary child creation are unavailable.

The workspace remains an explicitly trusted host selection. Preparation
requires it to open as a directory but imposes no effective-UID ownership or
private-mode rule on it. The individual workspace-tool ancestor and subordinate-
mount trust limits remain unchanged.

The selected base and every existing fixed intermediate must be directories
owned by the process's effective user ID with no group-or-other write bit
(`mode & 0o022 == 0`). The existing final `machine-god` directory must meet
those rules and be private from group and other entirely
(`mode & 0o077 == 0`). User permission bits are not repaired or otherwise
normalized.

Every directory created by preparation is made with mode `0700`, reopened
no-follow, checked so the no-follow path observation and opened descriptor have
the same device/inode identity, then `fchmod`ed to exact `0700` so a restrictive
umask cannot leave a different final mode. The resulting directory must be
owned by the effective user ID and still have exact `0700`. Existing components
are never chmodded, chowned, replaced, removed, or otherwise repaired. A
failure after one or more successful creates may leave those safe fixed
directories present; preparation supplies no rollback transaction or removal
authority.

Preparation performs its filesystem operations synchronously on the caller's
thread. It creates no future, runtime, task, thread, timer, or background work
and has no cancellation boundary once called. The suffix walk is fixed at one
XDG component or three fallback components; identity ancestry checks walk
descriptor parents until the filesystem root. Retained data is bounded by the
selected paths and descriptors, but filesystem calls and parent traversal may
block for an operating-system-dependent duration.

`PreparedNativeRoots` retains the opened workspace and final state-root
descriptors. `workspace_root()` and `state_root()` return the selected paths,
while the descriptors—not later path lookup—are the authorities handed to
workspace tools and `FileSessionStore`. Replacing either path after successful
preparation does not redirect those retained components. `selection()` exposes
the non-authoritative selected paths. The prepared value is not cloneable, and
its debug output is exactly `PreparedNativeRoots { .. }`, exposing no path,
descriptor, device, inode, ownership, or mode.

Equality and ancestry rejection is mandatory rather than a trusted-host
precondition. Device/inode identity and descriptor-relative `..` traversal are
used instead of lexical path comparison. A workspace equal to or above the
selected state base is rejected before any suffix creation. After the final
state root is retained, equality and ancestry are checked in both directions.
A final state root equal to, below, or above the workspace is therefore rejected
before composition can expose session artifacts through workspace tools. This
does not turn either tree into a sandbox against the trusted host, ancestor
replacement before an open, subordinate mount changes, or a filesystem that
violates the assumed descriptor semantics.

Preparation has five fixed, redacted categories:

| `PreparedNativeRootsErrorKind` | Stable name | Exact `Display` |
| --- | --- | --- |
| `WorkspaceRoot` | `workspace_root` | `native workspace root preparation failed` |
| `StateBase` | `state_base` | `native state base preparation failed` |
| `StateRoot` | `state_root` | `native state root preparation failed` |
| `UnsafeStateDirectory` | `unsafe_state_directory` | `native state directory is unsafe` |
| `OverlappingRoots` | `overlapping_roots` | `native workspace and state roots overlap` |

`WorkspaceRoot` covers failure to open and retain the workspace. `StateBase`
covers failure to open or inspect the required existing state base. `StateRoot`
covers a fixed suffix open/create/identity operation. `UnsafeStateDirectory`
covers an opened base or suffix directory that violates ownership or permission
rules. `OverlappingRoots` covers descriptor-proven equality or ancestry.

Each error retains only its kind. `Debug` is exactly
`PreparedNativeRootsError { kind: ... }`; `Display` is the corresponding table
entry. It has no nested source. Paths, environment values, ownership and mode
details, operating-system diagnostics, and raw error numbers are not retained
or reflected. These selection and preparation types are exported only on Linux
and macOS; other targets receive no public runtime API for this candidate.

## Reference-host composition

The new
`NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots` and
`NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots`
constructors consume one `PreparedNativeRoots`. They transfer its retained
workspace identity to exactly `list_files` and `read_file` and its separately
retained state-root identity to `FileSessionStore` without reopening either
path. The selected `machine-god` state root itself is the store root; no extra
`sessions` suffix is inserted. The existing loaded configuration, permission
prompter, production credential snapshot or trusted custom transport, default
engine limits, and no-op event sink retain their existing meanings.

The consuming constructors remain under the existing `ai-gateway-http`, non-
WebAssembly, Linux/macOS composition gate. Root selection and preparation do
not themselves require the optional HTTP feature, but are exported only on
Linux and macOS.

Root preparation is an explicit step before either consuming constructor is
called; it opens and retains the workspace before the state base and suffix.
The consuming constructor validates the loaded configuration before converting
the already prepared roots into tools and a store. Production composition
performs credential discovery and bearer-token handoff only after both roots
have been retained, validated, proven disjoint, and accepted for composition.
Root failure therefore does not discover or hand off a credential. The custom
transport path still performs no native credential discovery or production HTTP
construction.

If transferring the prepared workspace descriptor into the two tools cannot
clone it, the existing reference-host `WorkspaceRoot` stage is returned. The
session store has already accepted the retained state descriptor during
preparation, so prepared-root composition does not reopen a session path.

The existing path constructors
`NativeReferenceHost::compose_ai_gateway_http` and
`NativeReferenceHost::compose_with_ai_gateway_transport` remain supported and
unchanged: both require an existing absolute workspace and an existing absolute
session root, create nothing, and continue assigning disjointness to their
trusted caller. `FileSessionStore::open` likewise remains an explicit existing-
root, no-create constructor. Safe creation is available only through
`PreparedNativeRoots::prepare`; the new host constructors only consume the
result.

## Relationship to status, configuration, and CLI

Configuration remains strict schema v3 with the exact same built-in and file
bytes. Root selection is not a configuration field and neither the loader nor
core gains environment, filesystem, or creation authority.

Native status remains a metadata-only observation of the same resolved state
path. It does not call `NativeRootSelection`, prepare roots, open directory
descriptors, or create a missing directory. `machine-god`, help, version,
human status, JSON status, invalid-argument, and write-error bytes remain exact
and unchanged. No CLI command invokes root preparation.

## Deferred scope

This root sub-boundary does not implement session create, list, resume, replay,
or reset. It does not allocate a session ID or a new incarnation for reset,
change the session record schema, migrate or encrypt state, add a cleanup or
repair command, supply a terminal prompter, add native tools, or expand CLI
ownership. The combined root-and-session-lifecycle checklist item therefore
remains unchecked even after this candidate's root sub-boundary is delivered.

The slice makes no compatibility, upstream-equivalence, or product-performance
claim and does not change the pinned fx inventory, benchmark workloads,
workflows, or schema. Zig remains solely the pinned upstream benchmark build
input; machine-god remains a Rust product.
