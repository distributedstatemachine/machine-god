# Workspace selection and management

The bounded `workspace` command lists and manages native saved additional
directories. Configuration, descriptor authority, publication, and reconciliation
belong to the native service; the CLI only parses and presents its receipts.
Current delivery state remains in the [implementation plan](implementation-plan.md#current-delivery-state).

## Grammar and exits

```text
machine-god workspace [list | add PATH | remove PATH | clear] [--json]
```

`list` is the default. A singleton `--json` may appear anywhere after
`workspace`, including between the verb and path. Missing or extra operands,
repeated flags, unknown flags, and `--json=true` are invalid. A path beginning
with a hyphen must be spelled with an explicit relative prefix such as
`./-name`; there is no `--` separator. Path operands preserve operating-system
bytes, including non-Unicode paths where supported, with a 4,096-byte limit and
no NUL. The complete grammar is validated before acquiring native authority.

Invalid syntax emits global usage on stderr and exits `2`. Confirmed or
refreshed receipts exit `0`. Ambiguous, indeterminate, or reload-failed receipts
retain their independent facts but exit `1`; they are not blindly retried.
Operational and rendering failures also exit `1`. Human failures use stderr;
JSON failures use stdout. Output failures use
`machine-god: failed to write output\n` on stderr.

## Output and receipts

Each result identifies the canonical primary directory and generation, saved
suppression, and up to 16 additional directories. Every entry preserves source
spelling, identity, canonical-identity status, independent saved and launch
provenance, availability, and activation. Unavailable entries remain visible.

JSON paths are lossless objects: `{"text":"/path","bytes_hex":null}` for valid
Unicode, or `{"text":null,"bytes_hex":"<lowercase raw-byte hex>"}` otherwise.
Human paths use escaped quoted strings or an explicit `bytes_hex` representation.
Quotes, backslashes, terminal controls, Unicode line separators and bidirectional
formatting controls are escaped in either presentation.

Compact JSON uses keys `kind,action,primary_directory,generation,saved_suppressed,
additional_directories,saved_changed,runtime_changed,reconciliation,
reconciliation_error,launch_flag_can_restore`. Entry keys are
`source,identity,identity_canonical,saved,launch,available,active`.
The human rendering presents the same fields as labeled lines.

`saved_changed` and `runtime_changed` are independent optional booleans;
`null` means unknown, not false. Reconciliation is `cached_busy`, `refreshed`,
`confirmed`, `ambiguous_intended`, `ambiguous_before`, `indeterminate`, or
`reload_failed`. Observing intended or prior configuration after an ambiguous
publication does not confirm durability. `reconciliation_error` is null except
for a fixed redacted reload error. `launch_flag_can_restore` preserves the
native service's distinction between saved configuration and launch authority.

Both presentations are fully assembled before writing and capped at 1 MiB,
including one final LF. Entry cardinality and path bounds are checked first.
An invalid result or oversized rendering fails atomically as `ResourceLimit`.

Operational errors never reflect raw underlying messages, credentials, or paths.
Human errors are `machine-god workspace: operation failed: <Code>\n`.
JSON errors contain `kind,action,error,code`, with fixed
`"error":"workspace operation failed"`. Codes distinguish `Busy`,
`InvalidPath`, `UnknownDirectory`, `ResourceLimit`, `DuplicateRoot`,
`OverlappingState`, `Conflict`, `InvalidConfiguration`, `UnsafePath`,
`Persistence`, `Ambiguous`, `Unavailable`, and `Unsupported`.

## Native authority and effects

After parsing, explicit process root/config selection supplies the native
startup factory and workspace service. List reads actual saved configuration;
malformed settings fail rather than being ignored. Add, remove, and clear use
native owned configuration publication. Duplicate additions and no-op clears do
not rewrite settings. Top-level commands have no launch-only roots or saved
suppression. Missing configuration and session-state directories are not created
by startup or listing; mutations may create their configuration directory.
When process selection provides no settings authority, top-level listing uses
the native settings-free startup and service: it reports the validated primary
root with no saved additional roots and creates no directories. Add, remove,
and clear remain unavailable without persistence authority. This does not ignore
invalid environment selections or malformed, unreadable, or unsafe selected
configuration; those still fail rather than becoming settings-free requests.

The current-thread runtime drives the actual owned native worker scope. That
scope is closed and its full worker/collector completion is joined before
rendering either a receipt or an error. No engine, provider, reference host,
credential acquisition, network request, or detached Tokio helper is involved.

The older `NativeWorkspaceInspection` library remains a separate compatible
lexical observation API. These commands instead use descriptor-backed authority.

## Authority and remaining integration

### Descriptor authority contract

The native workspace authority primitive receives explicit root authority.
Its caller supplies an already-owned primary descriptor, an explicit
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

### Launch selection

```text
machine-god [--add-dir PATH | --add-dir=PATH]... [--no-additional-dirs] [ask ... | resume ... | <interactive-resume-options>]
```

Modifiers must precede the command. Parsing stops at the first nonmodifier;
prompt operands are not rescanned as global flags. Both additional-directory
spellings are repeatable, including canonical aliases. The next operand after
`--add-dir` is the path even if it starts with a hyphen. Suppression is singleton
and affects only saved roots, not explicit launch roots. Modifiers without a
command start a fresh interactive session. They also apply to stdin `ask`,
prompt-bearing resume and all supported interactive resume spellings. Other
top-level commands reject them rather than silently discarding the selection.
Ordinary first-argument help retains its existing no-effect precedence.

Paths retain native bytes on Unix, are nonempty, contain no NUL, and are at most
4,096 bytes. At most 64 raw additions are accepted; native preparation merges
aliases and applies the sixteen-entry scope limit. Unknown modifiers, missing
paths, duplicate suppression and unsupported command combinations fail grammar
before host configuration. Launch options do not save configuration.

Production startup first prepares its normal primary/state roots, then loads
workspace selection through `prepare_native_workspace` on a temporary owned
worker scope. That scope is closed and joined on success or failure before the
complete reference host is acquired. Existing state authority is captured after
state preparation; an earlier absent-state proof is never reused after creating
that state directory. Composition validates retained primary/state identities
against the prepared host roots. All conversations, including noninteractive
ask/resume, attach the exact workspace-context allocation before admission.
Signals latch before the first preparation worker starts and retain their first
observed identity through cleanup, including preparation failure and the later
turn-activation handoff.
The existing native settings-store private-directory checks apply during this
startup too; launch neither changes directory permissions nor falls back to
ignoring an unreadable or unsafe saved configuration.
When environment selection provides no user-settings authority, launch uses
the explicit settings-free preparation path: no saved roots, no settings-path
discovery, and the same validated launch roots. `/workspace list` still works;
add/remove/clear are unavailable because there is no persistence capability.
Fresh, resumed and reset interactive sessions keep this host selection; saved
session paths do not grant additional authority.

### Interactive management

`/workspace`, `/workspace list`, `/workspace add PATH`, `/workspace remove PATH`
and `/workspace clear` use the retained host's workspace service. The remainder
after `add` or `remove` is one path, including internal spaces; surrounding ASCII
space/tab is trimmed and shell quoting/escaping is not interpreted. There is no
interactive `--json` flag. Paths obey the same byte/NUL bound.

Acceptance retains a typed native control future bound to the exact current
runtime. No save or filesystem refresh occurs before owner progress. The native
idle-and-empty-queue lease admits mutations; a busy list returns cached authority
without I/O, while a busy mutation fails without consuming or cancelling the
queued/active turn. Started work remains owned through shutdown or a pending
transition; a failed or uncertain control rejects a pending transition and
retains the source session. Consuming the receipt is distinct from completion of
the underlying owned worker.

The slash receipt reuses the top-level human projection, including independent
saved/runtime observations, provenance, availability, reconciliation and launch
restoration facts. Its complete presentation is capped at the interactive 64 KiB
limit; failure to render does not roll back a native publication. Errors are
fixed redacted categories, never raw configuration or operating-system strings.
The next admitted turn captures the published workspace snapshot; already-taken
scopes keep their original descriptors. Top-level management alone does not
rewrite another running host's captured authority. Further tool/completion
compatibility remains governed by the individual tool contracts.

The leading-flag grammar and supported launch categories follow pinned
[`cli_surface.zig`](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/cli/cli_surface.zig).
The slash grammar follows pinned
[`app_commands.zig`](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/app/app_commands.zig),
with actions from the retained registry and pinned
[`workspace_commands.zig`](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/workspace/workspace_commands.zig).
These are scenario-level contracts, not a blanket upstream-equivalence claim.

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
`prepare_native_workspace_without_settings` preserves that same ownership and
validation contract without a settings capability or saved-directory reads.
`NativeWorkspaceService::without_settings` permits refreshed or cached listing
and rejects persistence mutations before worker admission.

At most 64 launch arguments are examined; the merged additional-root limit is
still 16. Launch paths resolve relative to the primary root and must name
existing directories. Canonical aliases merge with saved provenance before the
limit is checked. Missing saved entries remain visible and unavailable; saved
suppression does not suppress a merged launch source. Final primary and existing
state entries are opened without following symlinks; an absent state path uses
the authority's retained-prefix proof without creating it.
