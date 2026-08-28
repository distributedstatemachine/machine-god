# Native root selection and preparation

This contract defines the Linux/macOS library boundary for selecting and
retaining the workspace and state roots required by native reference-host
composition. Explicit selection is pure path derivation from caller-owned
inputs. Process selection additionally captures the ambient current directory.
Preparation is the only new root-creation authority, and it can create only a
fixed state suffix beneath an already existing selected base. Core and the CLI
do not implement environment or root-creation effects themselves; the CLI may
invoke this native boundary for an explicitly documented command.

## Selection

The public selection surface is:

```rust,ignore
NativeRootSelection::from_current_process(
    environment: &NativeEnvironment,
) -> Result<NativeRootSelection, NativeRootSelectionError>
NativeRootSelection::from_environment(
    environment: &NativeEnvironment,
    workspace_root: &Path,
) -> Result<NativeRootSelection, NativeRootSelectionError>
NativeRootSelection::workspace_root(&self) -> &Path
NativeRootSelection::state_root(&self) -> &Path
NativeRootSelectionError::kind(&self) -> NativeRootSelectionErrorKind
NativeRootSelectionErrorKind::as_str(self) -> &'static str
```

`NativeRootSelection::from_current_process(&NativeEnvironment)` has one narrow
ambient authority: it captures `std::env::current_dir()` exactly once and uses
that path as the workspace input. Its environment input remains the supplied
`NativeEnvironment` snapshot; the constructor does not reread process
environment variables. Current-directory capture failure returns the fixed
`CurrentDirectory` category before state-root selection. A captured path then
passes through the same lexical validation and state selection as
`from_environment`.

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

An accepted state base is rebuilt from its lexical components before it is
retained. This removes redundant separators and `.` decorations so the later
`O_NOFOLLOW` open applies to the actual final component; lexical `..` remains
unchanged and is resolved only by the operating system during that open.

The selected state root path is exactly
`<XDG_STATE_HOME>/machine-god`, or
`<HOME>/.local/state/machine-god` for the fallback. The resolved path is not
derived from configuration data or native status. `workspace_root()` and
`state_root()` expose the selected paths for host observation. The selection is
cloneable and equality-comparable. Its debug output is exactly
`NativeRootSelection { .. }` and reports no path or environment value.

Selection has four fixed, redacted categories:

| `NativeRootSelectionErrorKind` | Stable name | Exact `Display` |
| --- | --- | --- |
| `CurrentDirectory` | `current_directory` | `native current directory is unavailable` |
| `InvalidWorkspaceRoot` | `invalid_workspace_root` | `native workspace root selection is invalid` |
| `StateRootUnavailable` | `state_root_unavailable` | `native state root selection is unavailable` |
| `InvalidStateEnvironment` | `invalid_state_environment` | `native state environment selection is invalid` |

`CurrentDirectory` covers only failure to capture the process current
directory; it does not retain the operating-system diagnostic.
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
   every retained suffix component and, on macOS, reject any unsafe extended
   ACL or descriptor-bound ACL read failure; and
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
normalized. On macOS, the retained descriptor for the base and every existing
suffix component must have either no extended ACL or only one or more entries
whose tag is exactly `DENY`, flags are zero, and sole permission is exactly
`DELETE`, with no ACL-level flags. This recognizes the protective
`everyone deny delete` entry on ordinary macOS home directories without
accepting authority-granting policy. Any `ALLOW` or unknown tag, entry flag,
other or combined permission, ACL-level flag, malformed result, or
operating-system read failure rejects the directory.

Every directory created by preparation is requested with mode `0700`. Because
the process umask may remove even owner access bits, the just-created fixed name
is first normalized descriptor-relatively to `0700` beneath the already
effective-UID-owned, non-group/other-writable parent. It is then observed
no-follow, reopened no-follow, checked so the path observation and opened
descriptor have the same device/inode identity, and `fchmod`ed and verified at
exact `0700`. On macOS, the opened new directory must additionally satisfy the
same empty-or-exact-deny-delete ACL rule after normalization, so an inherited
authority-granting or unknown ACL fails closed.
The same-effective-UID account is the remaining trust boundary during that
normalization; Linux cannot apply `AT_SYMLINK_NOFOLLOW` to this `fchmodat`
operation. The resulting directory must be owned by the effective user ID and
still have exact `0700`. Existing components are never chmodded, chowned,
replaced, removed, stripped of ACLs, or otherwise repaired. A failure after one
or more successful creates may leave those safe fixed directories present;
preparation supplies no rollback transaction or removal authority.

macOS ACL inspection is bound directly to each retained descriptor, not to its
original path or a `/dev/fd` pathname. The product crate remains free of unsafe
Rust and exact-pins `calcifer-macos-acl` 0.1.0 as a target-macOS-only narrow FFI
boundary around the native descriptor API. Its crates.io checksum is retained
in `Cargo.lock`; it has no normal dependencies. The dependency's complete
published source remains subject to dependency policy and vulnerability gates.
ACL validation is a point-in-time
preparation check; the same user or a privileged process can still mutate an
accepted directory later.

Concurrent preparers never widen an existing directory or treat `EEXIST` as
proof that they created it. A caller that loses the create race while the winner
is still normalizing owner access may fail closed and retry; the API does not
promise that every simultaneous caller succeeds on its first attempt under an
owner-bit-masking umask. This preserves the rule that an `EEXIST` entry is
validated as existing and is never chmodded or repaired by the losing caller.

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
covers an opened base or suffix directory that violates ownership, permission,
or macOS extended-ACL rules, including failure to read its descriptor-bound ACL.
`OverlappingRoots` covers descriptor-proven equality or ancestry.

Each error retains only its kind. `Debug` is exactly
`PreparedNativeRootsError { kind: ... }`; `Display` is the corresponding table
entry. It has no nested source. Paths, environment values, ownership and mode
details, operating-system diagnostics, and raw error numbers are not retained
or reflected. These selection and preparation types are exported only on Linux
and macOS; other targets receive no public runtime API for this boundary.

## Reference-host composition

`NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots` and
`NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots`
constructors consume one `PreparedNativeRoots`. They transfer its retained
workspace authority, cloning its descriptor only as required by the registered
workspace-rooted tools, and transfer its separately retained state-root identity
to `FileSessionStore` without reopening either path. Core exposes the composed
tools in deterministic alphabetical catalog order. The selected `machine-god`
state root itself is the store root; no extra `sessions` suffix is inserted.
The loaded configuration, permission prompter, credential snapshot or trusted
custom transport, engine limits, and event sink retain their documented
meanings.

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

If transferring the prepared workspace descriptor into a tool cannot clone it,
the reference-host `WorkspaceRoot` stage is returned. The session store has
already accepted the retained state descriptor during preparation, so
prepared-root composition does not reopen a session path.

The existing path constructors
`NativeReferenceHost::compose_ai_gateway_http` and
`NativeReferenceHost::compose_with_ai_gateway_transport` remain supported:
both require an existing absolute workspace and an existing absolute session
root, create nothing, and continue assigning disjointness to their trusted
caller. Every composition path receives an explicit `WebSearchDeadline`;
custom transport paths also receive the canonical `NetworkTarget` that core
authorizes for search. These inputs do not change root identity or creation
semantics. `FileSessionStore::open` likewise remains
an explicit existing-root, no-create constructor. Safe creation is available
only through `PreparedNativeRoots::prepare`; the consuming host constructors
only consume the result.

## Relationship to status, configuration, and CLI

Configuration remains strict schema v3 with the exact same built-in and file
bytes. Root selection is not a configuration field and neither the loader nor
core gains environment, filesystem, or creation authority.

Native status remains a metadata-only observation of the same resolved state
path. It does not call `NativeRootSelection`, prepare roots, open directory
descriptors, or create a missing directory.

The one-shot `machine-god ask` host captures one `NativeEnvironment` snapshot,
passes it to `NativeRootSelection::from_current_process`, prepares the result,
and consumes the prepared roots during reference-host composition. Its ambient
current-directory read is therefore owned by this native boundary rather than
core. Root selection and preparation occur only after the complete prompt has
passed CLI validation. Other commands do not acquire root-preparation authority
merely by observing status or configuration.

## Deferred scope

This boundary does not implement session lifecycle operations, allocate session
or incarnation identities, change the session record schema, migrate or encrypt
state, add cleanup or repair authority, or supply a terminal prompter. Those
behaviors remain owned by their respective native components. Workspace tools
consume cloned prepared descriptors but add no root-selection authority or
behavior of their own.
