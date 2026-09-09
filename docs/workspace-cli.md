# Top-level workspace command

The bounded `workspace` command reports the process's primary workspace as a
read-only lexical snapshot. It does not manage additional directories or
construct the native reference host. Current delivery state and gate evidence
remain only in the
[implementation plan](implementation-plan.md#current-delivery-state); this page
defines durable behavior.

## Grammar and exits

The only accepted invocations are:

```text
machine-god workspace
machine-god workspace --json
machine-god workspace list
machine-god workspace list --json
```

`list` is the default action. The singleton `--json` flag must be final.
`--json list`, repeated flags, `--json=true`, `add`, `remove`, `clear`, path
operands, extra arguments, and non-Unicode arguments are invalid. The complete
grammar is validated before current-directory or any other native authority is
acquired.

Invalid syntax writes the one global usage diagnostic to standard error,
writes no standard output, and exits `2`. Success writes only standard output
and exits `0`. An operational or rendering failure exits `1`. Human mode writes
its fixed diagnostic to standard error with empty standard output. JSON mode
writes one compact error object to standard output with empty standard error.
A stdout failure uses the existing exact
`machine-god: failed to write output\n` standard-error diagnostic and exits
`1`.

The closed, redacted presentation categories are:

| Category | Meaning |
| --- | --- |
| `Unavailable` | Current-directory capture failed, or the captured path is non-Unicode, relative, or contains a lexical parent component. |
| `ResourceLimit` | The path, returned snapshot invariant, or complete serialized output exceeds its bound. |

The human failure is exactly
`machine-god workspace: could not inspect workspace: <Category>\n`. JSON
failure fixes key order `kind,error,code` and is exactly
`{"kind":"workspace","error":"could not inspect workspace: <Category>","code":"<Category>"}\n`.
Neither mode reflects a path, environment value, operating-system diagnostic,
raw error number, or underlying error text.

## Successful output

The result has one primary directory. It is the accepted lexical path returned
by the single process current-directory capture, not a canonical path, retained
descriptor, filesystem identity, or promise that the directory still exists.
The UTF-8 path is at most 4,096 bytes.

Human success is exactly these two LF-terminated lines:

```text
[workspace] primary="<absolute-path>"
[workspace] additional_directories=unsupported
```

`<absolute-path>` is a JSON string, including its quotes. Quotes, backslashes,
C0/C1 controls and DEL, Unicode line and paragraph separators, and Unicode
bidirectional-formatting controls are escaped. The path is intentional public
output in both modes.

Compact JSON fixes key order
`kind,action,primary_directory,additional_directories_supported,additional_directories`:

```json
{"kind":"workspace","action":"list","primary_directory":"<absolute-path>","additional_directories_supported":false,"additional_directories":[]}
```

`action` is always `list`. The `false` value and empty array describe the
currently supported command surface; they do not claim that an upstream or
foreign saved-directory configuration was inspected. Both representations are
assembled completely before their first byte is written, end with exactly one
LF, and are capped at 32,768 bytes including that LF. A violated result
invariant or output cap fails atomically as `ResourceLimit`; partial success
output is never intentionally emitted.

## Native authority and effects

After parsing, the native workspace inspection boundary synchronously calls
`std::env::current_dir()` exactly once. The returned path must be nonempty,
Unicode, absolute, contain no lexical `ParentDir` component, and satisfy the
4,096-byte UTF-8 limit. Inspection performs lexical validation only.

The command does not read process environment variables, configuration, state,
credentials, session records, or directory metadata. It does not inspect,
canonicalize, open, create, remove, rename, or write a filesystem object; load
or prepare native roots; construct an engine, provider, transport, runtime, or
reference host; prompt; or use the network. In particular, missing, empty,
relative, non-Unicode, or otherwise invalid `HOME`, `XDG_CONFIG_HOME`, and
`XDG_STATE_HOME` values cannot affect this command and no configuration or state
path is created.

This boundary is separate from `NativeRootSelection::from_current_process`.
Root selection also selects a state root and therefore has authority and
failure modes that a primary-workspace observation does not need. The workspace
snapshot grants no filesystem or tool authority to later operations.

## Deferred surface

### Descriptor authority contract

The native workspace authority primitive is separate from the lexical command
above. Its caller supplies an already-owned primary descriptor, an explicit
state identity, and an owned state descriptor when that directory exists.
An absent state directory instead retains a validated nearest-existing-prefix
descriptor and canonical projected exclusion path; it creates nothing and does
not exclude unrelated siblings beneath that prefix. Omitting the descriptor for
an existing target is rejected. Blocking preparation validates
those identities and opens additional roots; it never discovers a current
directory, expands a home directory, or creates directories. Hosts must execute
preparation and refresh on their owned native worker scope.

Immutable generation snapshots retain the actual descriptors. A replacement is
bound to its originating manager and base generation; only the admitted native
owner may publish it. Foreign and stale preparations fail without changing the
current snapshot. Existing snapshots and resolved routes retain their old
descriptors across publication, root renames, and manager destruction.
New preparations revalidate the primary and exclusion descriptor identities;
renaming or replacing the retained exclusion ancestor fails without rebinding.
If an absent state target appears, preparation fails until the owner obtains
explicit existing-state authority instead of silently switching the proof.

At most 16 additional entries are retained, including unavailable and suppressed
entries. Each retains its source spelling, absolute identity, canonical-identity
flag, and independent saved/launch provenance. Availability is separate from
activation: an available root is active when it is a launch source or is saved
without saved suppression. A combined saved/launch entry remains active during
saved suppression. Duplicate primary/additional identities and descriptor aliases
are rejected; saved/launch merging occurs before authority preparation.

Canonical identities refresh against the observed identity, not the original
source spelling. A retargeted identity becomes unavailable, never authority for
the new target. An unavailable provisional source may acquire its first canonical
identity when it becomes available. The owner must carry the resulting snapshot's
updated source records forward. Missing entries remain visible and can reactivate
when their retained identity returns. Final root symlinks are not followed;
ancestor resolution is checked against the opened directory's identity.

Paths preserve non-Unicode bytes, have a 4,096-byte bound, and reject NUL and
parent traversal. Pure route selection sends relative paths to the primary root;
absolute paths select primary first, then the first matching active additional
root in entry order, using component boundaries. State roots and their ancestors,
descendants, and descriptor aliases cannot become workspace authority. A route
retains its root descriptor, identity, generation, and normalized relative path.
This is lexical routing, not descendant symlink validation: consuming tools and
preparers must still walk descendants relative to that descriptor using their
existing no-follow confinement checks. This primitive alone does not change CLI
grammar or extend any tool's authority.

### Remaining command surface

The following pinned-upstream behavior remains intentionally unsupported:

- `workspace add PATH`, `workspace remove PATH`, `workspace clear`, durable
  additional-directory configuration, reconciliation, saved suppression, and
  the upstream additional-directory limit;
- global `--add-dir` and `--no-additional-dirs` options;
- availability, activation, and source flags for additional directories;
- interactive `/workspace` behavior and its slash-command category;
- extending tool authority, indexing, search, or completion across additional
  roots; and
- canonical filesystem identity, descriptor retention, or a compatibility-
  equivalence claim for the broader upstream workspace manager.

## Native workspace operation ownership

The native workspace service is shared by administrative and interactive
workspace commands. It receives one fixed descriptor authority, an explicit
user-config store and the actual host-owned worker scope. Construction and
unpolled operation futures perform no filesystem work. Each service admits one
operation at a time. Interactive mutations additionally hold the exact accepted
runtime's idle-and-empty-queue lease through staging, publication, reconciliation
and worker cleanup; abandoning the response does not release that ownership.
A list requested while busy returns an explicitly cached scope without I/O.
An idle list refreshes retained-root availability without editing configuration.

Add resolves an existing directory relative to the primary root, validates state
exclusion before publication and saves its canonical identity. Adding a launch
root promotes its saved provenance. Remove matches retained source or identity
before resolving the input, so missing or retargeted saved sources remain
removable. Remove and clear drop both saved and launch provenance, while keeping
the launch suppression setting. Runtime-only changes need not create config.
Saved and launch identities merge before the sixteen-root bound is applied;
unavailable and suppressed entries still count. A stale full runtime may stage
the requested add independently and let the latest locked config decide capacity.
The shared startup/reload merger accepts at most 64 raw launch observations,
deduplicates canonical aliases, and resolves provisional saved sources only once.
Later refreshes retain the acquired identity even if its old source is retargeted.
Each scope retains the exact saved record that produced an observed source.
Capacity checks may collapse its canonical launch alias only while that entire
record still matches both preflight and locked latest configuration. Changed or
new records count conservatively; proofless manually constructed scopes do not
infer provenance from a matching path spelling. Matching observations also pin
post-save reload to the accepted identity. This bounded alias evidence is never
written to configuration or session metadata.

Confirmed publication is followed by a fresh config read and authority rebuild,
preserving other writers' latest saved roots and the operation's surviving launch
roots. A failed rebuild leaves the old runtime authority installed and retains
the confirmed save observation. An ambiguous publication accepts only its
observed intended or previous saved set; any other set remains indeterminate.
These reconciliation observations never establish directory durability.
Receipts distinguish known saved/runtime changes from unknown outcomes and
identify removed launch roots that command-line flags can restore on restart.
Previously taken immutable scopes retain their exact original descriptors.

## Owned workspace startup

`prepare_native_workspace` receives explicit root selection, native settings
store, launch paths, saved suppression, and a worker scope. Its future is inert
until polled. The owned worker validates bounded inputs, observes saved settings,
merges sources and retains primary/additional descriptors plus state-exclusion
authority. It does not discover environment or credentials, start a provider,
create state/configuration directories, or publish settings. The caller must
close and settle the actual worker scope even when a response is abandoned.

At most 64 launch arguments are examined; the merged additional-root limit is
still 16. Launch paths resolve relative to the primary root and must name
existing directories. Canonical aliases merge with saved provenance before the
limit is checked. Missing saved entries remain visible and unavailable; saved
suppression does not suppress a merged launch source. Final primary and existing
state entries are opened without following symlinks; an absent state path uses
the authority's retained-prefix proof without creating it.
