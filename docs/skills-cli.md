# Skills CLI

The interactive CLI supports human-invoked skill discovery, management and
prompt selection through explicitly supplied native capabilities.
Implementation status and delivery gates are tracked only in the
[implementation plan](implementation-plan.md). The CLI owns input decoding and
presentation; native adapters own matching, installation and queued context.
Existing model-facing [skill](skill.md) and [install_skill](install-skill.md)
tools keep their independent contracts and permission boundaries.

## Native command routing

`NativeSkillsCommand::from_str` accepts the payload after `/skills`. It is
bounded, synchronous and effect-free: it does not inspect a catalog, directory,
environment, process, credential or destination. The full payload is limited to
8,192 UTF-8 bytes before trimming or allocation. C0/C1 controls except tab are
rejected. Exact lowercase verbs use ASCII space/tab separators:

| Payload | Native operation |
| --- | --- |
| empty or `list` | `List` |
| `path` | `Path` |
| `show <selector>` | `Show` |
| `create <arguments>` | `Create` |
| `add <arguments>` or `install <arguments>` | `Install` |
| `remove <selector>` | `Remove` |

`list` and `path` accept no extra arguments. Other forms require a nonempty
remainder. Show/remove selectors are limited to 4,096 bytes. Outer ASCII
whitespace is removed; interior bytes, including Unicode and spaces, remain
unchanged. This routing layer does not interpret quoting, source URLs, locations,
installation flags or replacement consent. The corresponding native domain
adapter must validate those before admitting effects. Pasted `npx`/`bunx` text
is installer input data, never permission to execute a package manager.

All routing errors use the fixed `NativeSkillsCommandError`, displaying
`invalid native skills command`. Command debug output names only the operation,
never arguments, locations or source credentials. Programmatically constructed
enum values are likewise data: native domain adapters must still validate their
contents and obtain the appropriate owned admission.

## Explicit catalog authority

`NativeSkillCatalog` retains an ordered list of explicitly supplied
`NativeSkillRoot` values. Each root carries a retained directory, a relative
discovery location, reporting labels, provenance and link policy. Construction
validates these values without opening paths or consulting the environment.
Directory-authority labels pass the existing catalog path policy on borrowed
input before normalization or copying, including the 4,096-byte raw-path bound.
Discovery scans one child-directory level per root. Missing ordinary roots are
empty; unreadable or malformed candidates, broken links and exhausted bounds
produce diagnostics and an incomplete snapshot. Hidden skill basenames remain
eligible, except reserved `.machine-god-skill-*` transaction names under managed
roots. Compatibility roots are read-only.

Workspace/home root expansion is the composed host's responsibility, not an
ambient operation inside the catalog. Managed roots reject links. Explicitly
contained compatibility links resolve through retained descriptors and cannot
escape their captured base; absolute links must remain beneath its captured
authority label. Final `SKILL.md` files are always no-follow regular files.
Directory-name ordering is deterministic within the supplied root order.

`compose_native_skill_catalog` provides the bounded native expansion adapter.
The caller supplies retained `NativeSkillDirectoryAuthority` workspace/home
directories and the manager's unchanged `catalog_root()`. Authority construction
is inert; composition runs synchronously in an admitted worker. It never reads
the environment, reopens absolute labels, creates missing skill directories or
manufactures write authority. Discovery order is:

1. Nearest workspace then each ancestor: `skills`, `.opencode/skills`,
   `.codex/skills`, `.claude/skills`, `.agents/skills`, `.claw/skills`.
2. The explicitly supplied native managed root.
3. Explicit home compatibility: `.fx/skills`, `.config/opencode/skills`,
   `.codex/skills`, `.claude/skills`, `.agents/skills`, `.claw/skills`.

Workspace traversal stops before the captured home by descriptor identity or
path label. Otherwise it includes filesystem root, with at most 20 workspace
levels and 128 charged native stat/open attempts. Exceeding a bound fails without
a partial catalog. Descriptor-relative parent/name observations must agree with
captured labels; changed ancestry or an inconsistent home label fails instead of
silently continuing above it. Canonical labels are therefore expected for
workspace ancestry. A separately captured alias of the same home inode can stop
traversal. All compatibility roots are contained and read-only. Managed roots
must retain Managed/Reject provenance; actual write ownership still requires the
manager's original directory capability. Unsupported platforms fail without I/O.

### Composed startup

`prepare_native_skills` consumes prepared roots, the captured native environment,
terminal-helper options, an owned worker scope and cancellation. Its future is
inert until polled. One worker duplicates the already retained workspace/state
descriptors, expands roots and selects optional Git authority. It returns the
original prepared roots and a service whose catalog and manager share the same
managed capability. It performs no discovery, namespace creation, process launch
or network request. Renaming the state directory does not redirect management
to a replacement path; inconsistent workspace ancestry remains an error.

Absent HOME selects no home roots. A supplied empty, relative, non-Unicode or
unavailable HOME is an error, not permission to search broader ancestors. Only
that explicit home path is opened and identity-checked. PATH is bounded to
16 KiB and 64 absolute entries; empty or relative entries are rejected. Missing
PATH or Git leaves local management available and remote Git unavailable.
Startup charges at most 128 selection I/O attempts, separately from bounded root
expansion. Individual native calls are not preempted. Dropping a polled startup
future cancels its private worker token, not the caller's token; scope completion
still retains the worker. Caller cancellation wakes the future and requests
private-operation cancellation.

The CLI captures environment values once for native roots, terminal selection
and Git selection. After signals enter the owned-setup phase, it prepares skills
on a temporary scope which it closes and joins before acquiring the full host.
Interactive preparation separately discovers the initial snapshot on that scope,
preserving incomplete-discovery diagnostics; one-shot preparation skips this
discovery. Startup failure or panic cannot abandon its admitted worker.

Metadata supports a 65,536-byte frontmatter envelope, 256-byte names and
4,096-byte decoded descriptions, including the pinned quoted and supported
block-description forms. Unknown metadata is ignored; malformed recognized
fields are errors. Basename fallback applies only when frontmatter is absent.
This is a bounded compatibility parser, not a general YAML implementation.

Snapshots retain entries, provenance, diagnostics, completeness and a generation
digest. Exact duplicate names remain separate locations. Name-only resolution
rejects ambiguity or incomplete discovery; an exact valid selection can remain
usable amid unrelated discovery failures. Query input is limited to 1,024 bytes
and 128 returned rows. Selections bind their original retained root and observed
revision. Cloning a catalog preserves that authority; independently constructing
another catalog does not recreate it.

Materialization reopens only the selected location and validates its revision
before and after reading. Discovery's revision consists of stat identity, size,
nanosecond timestamps and the observed metadata-prefix digest, not a prior
full-body digest or an atomic filesystem snapshot. A successful materialization
returns the exact complete UTF-8 file and its full SHA-256 digest. Changed,
foreign, nonregular or oversized selections fail rather than rematching by name.

Catalog limits are 128 roots, 16,384 visited entries, 1,024 candidates,
256 diagnostics, 8 MiB aggregate metadata reads, 4 MiB retained snapshot text,
4 MiB names staged for sorting a single directory, and 65,536 charged I/O
attempts. Paths are at most 4,096 bytes and 32 components, with 32 link hops;
materialized files are at most 1 MiB. Discovery/materialization are synchronous
and must run inside an admitted, host-owned worker. Cooperative cancellation
brackets native calls; it cannot preempt an individual blocking kernel call.

## Managed installation and removal

`NativeManagedSkills::open` receives an explicit existing native state root and
optional Git runner. Its writable namespace is that root's `skills` directory;
opening the adapter does not create that namespace. Other products' roots,
including fx compatibility roots, remain read-only. This human-management API
does not widen the model-facing `install_skill` tool.

Source syntax is classified before filesystem/network work. Local sources,
explicit HTTP(S)/SSH Git sources, owner/repository shorthand, skills.sh forms,
filters and pasted `npx`/`bunx` forms are supported. Pasted package-manager text
is parsed only. A two-component local path must use `./` (or an absolute path)
to distinguish it from repository shorthand. A failed local lookup never falls
back to the network. Unsupported schemes/options are errors. These are
intentional authority-preserving differences from upstream's local fallback.
Explicit install filters are checked as borrowed UTF-8 text against the 256-byte
name bound before retaining a copy; absent and empty filters remain equivalent.
Relative local sources validate the borrowed current-directory and joined path
against the 4,096-byte bound before allocating the join or normalizing its
basename. Absolute local and Git sources ignore the unused current-directory
argument. Bounded local dot/parent spelling retains its existing interpretation.

Install planning includes root and nested skills, metadata-name/basename filters,
deterministic destination selection and collision checks, including filesystem
aliases. Git URL query text never becomes a destination basename. Hidden names
are permitted except the case-insensitive reserved `.machine-god-skill-*`
prefix. Create generates a native template and preserves sibling resources when
replacing an existing `SKILL.md`. Removal captures one exact managed basename;
advertised-name resolution belongs to the native command facade.

Plans retain bounded immutable content and exact destination revisions. Local
and completed remote preparation do not leave a disk staging tree while waiting
for consent. Default `NoReplace` cannot overwrite an existing destination.
Explicit outer `--replace` authorizes only the plan's exact observed replacement
revisions; pasted `-y`, `--yes` or global flags are not replacement consent.
Changed destinations reject the stale plan rather than transferring consent to
a new occupant.

On Unix, installed regular files retain their source execute bits with private
owner read/write permissions (`0600 | (source_mode & 0111)`). Generated `SKILL.md`
files use `0600`; staged and published directories use `0700`. Group/other
read/write and special permission bits are not copied. Replacing `SKILL.md`
preserves sibling resources' execute bits. Exact observed modes participate in
local-source revalidation, destination revisions and rollback checks; publication
checks the deliberately normalized planned modes.

Publication uses a private advisory lock, bounded staging/backup and rollback.
Per-item receipts distinguish installed, replaced, removed, failed, rolled back,
indeterminate and not attempted. Failure after publication is not proof that
nothing changed. Uncertain publication or cleanup residue stops the batch and
preserves recovery evidence. Opaque recovery IDs are reporting-only private
transaction basenames, never cleanup authority. Callers must retain partial
receipts separately from any later catalog refresh.

Management limits include 64 selected skills, 4,096 entries, depth 32,
4,096-byte paths, 1 MiB aggregate relative names, 1 MiB per file and 64 MiB
aggregate file bytes per normal traversal/staging phase. Each such phase charges
at most 16,384 work steps; a path-open step may include up to 32 descriptor-relative
opens, so this is not a raw syscall count. Cleanup separately allows 8,194 entries,
2 MiB names and 65,536 steps to cover both staged and backup trees. Lock waiting
is limited to two seconds and 201 attempts; kernel-call latency remains separate.
Directory enumeration charges both iteration and buffer refills, including
bounded interrupted-call retries, and checks cancellation after native returns.
Repeated walks use independently opened descriptor-relative directory cursors.

Remote preparation injects `NativeSkillGitRunner` with the exact precreated
private clone directory, cancellation, a 120-second deadline, 64 KiB output limit
and 256 MiB clone-size rejection/watchdog threshold. The latter is not a hard
host-disk quota: polling cannot prevent writes between observations. The runner
must retain its directory lease through actual owned-process cleanup, including
deferred cleanup after an error. The production adapter is a separate composed
native capability; the storage API does not execute a package manager or shell.

`SystemNativeSkillGitRunner` is the Linux/macOS production adapter. Construction
validates an explicitly selected absolute Git executable, captured-exec helper
and allowlisted environment without starting a process. Execution uses direct
arguments, shallow single-branch cloning, no tags or recursive submodules,
disabled hooks/templates and only HTTP, HTTPS or SSH transport. System/global
Git configuration and terminal credential prompting are disabled. The selected
environment may retain ordinary home, locale, agent-socket, certificate and proxy
settings; arbitrary Git or loader configuration is rejected and never logged.

The existing helper READY/COMMIT protocol binds the retained working directory
and cleanup lease before Git can run. Cleanup retains the original process group
and positively observed members through actual reaping, including deferred
cleanup; it does not claim ownership of descendants that escaped before any
observation. Explicit macOS inventory uses the bounded authenticated helper.
Before COMMIT, loss of direct-child wait authority permanently stops numeric-PID
signaling and observation and retains the bounded admission ticket and directory
lease. That unresolved ownership is not reported as successful cleanup.
Clone-size checks run initially, finally and at bounded polling intervals, with
16,384 entries, depth 32 and two million charged traversal steps. The size limit
remains a rejection threshold, not a filesystem quota.

Already admitted prompt text uses the checkpoint-bound continuation contract in
[native conversation](native-conversation.md#checkpoint-bound-skill-context).

## Native command service

`NativeSkillsService` combines an explicit catalog with optional managed-write
authority. Construction is inert; synchronous execution requires an admitted
host-owned worker. Every command is validated before effects, including directly
constructed command variants. The caller supplies the current directory and
cancellation token; the service never reads ambient home or environment values.

List returns catalog data. Show returns a menu query and optional exact focus,
not skill contents. Missing names produce a not-found notice; ambiguous or
incomplete discovery cannot invent a unique name selection. Path requires the
managed adapter. Create and install derive replacement consent solely from the
outer `--replace` flag and the plan's exact destination revisions. Mutation
receipts remain separate from any caller-requested later refresh; partial,
rolled-back, unattempted or uncertain batches are not successful controls.

Removal resolves only managed entries by advertised name, basename or exact
location. Name-only removal rejects incomplete discovery. Compose the catalog
using `NativeManagedSkills::catalog_root()` so discovery and mutation retain the
same directory capability. Equal path labels, or independent opens of the same
inode, do not recreate that identity. Removal rejects foreign capabilities before
preparation, then materializes the original selection before committing the
captured destination revision. It cannot transfer a stale name selection to a
replacement occupant. Managed recovery identifiers remain reporting evidence.

## Exact invocation planning

`NativeSkillInvocationPlan::resolve` is effect-free. The caller supplies the
exact prompt, an observed catalog snapshot and already prompt/span-bound picker
selections. The planner additionally validates every explicit selection's
authority and revision against that snapshot, including duplicates, before
cloning anything. Explicit selections retain their caller order; automatic
matches follow in catalog order, deduplicated by location. The originating
catalog must remain available for later exact materialization.

Automatic matching follows the pinned leading forms: `$name` or `/name` after
ASCII whitespace, or an initial affirmative `use`/`apply`/`activate`/`invoke`/`run`
request naming a skill, with optional `Please` and `the`. Initial quoted forms,
negations and incidental later mentions do not match. Sigils compare ASCII case
insensitively and otherwise preserve bytes; their continuation boundary excludes
only ASCII alphanumeric, underscore and hyphen bytes. Natural references use
the pinned ASCII-word normalization, not Unicode case folding. Consequently,
`$reviewé` can match `review`, and `Please-use review skill` is accepted. These
are compatibility rules, not a general natural-language intent classifier.

Exact duplicate advertised names are excluded from automatic matching. Distinct
case variants or names that normalize alike may both match; neither becomes an
arbitrary precedence winner. Arbitrary inline `$` mentions require explicit
picker bindings rather than automatic sentence scanning.

Incomplete discovery suppresses all automatic selection while preserving valid
explicit selections. `automatic_matching_incomplete()` must be visibly reported
by the composed host; an empty automatic result is not proof of absence or
uniqueness. No context is read or authority inferred by this planner.

Prompts are limited to 256 KiB, explicit and resulting selections to 16, and
aggregate retained selection text to 64 KiB. Incoming count/bytes are checked
before deduplication. Result bounds are checked before selection cloning;
failure is atomic. Materialized prompt context has its own independent limits.

### FIFO admission and materialization

`NativeConversationRuntime::enqueue_with_skills` resolves the exact prompt,
snapshot and explicit selections without effects. It checks each selection
against the originating catalog capability before retaining the bounded plan;
the snapshot itself is not retained in the queue. Selection bytes count with
prompt bytes against the existing 4 MiB queue budget. The interactive facade
obtains the service's catalog and host worker scope, returning a queued identifier
and the incomplete-matching flag. Missing skills authority fails without retaining
input; ordinary `enqueue` remains a separate API.

After FIFO admission, materialization runs outside the runtime mutex on an owned
worker. The exact runtime lease, captured policy and workspace travel with that
worker until reads finish, even when admission or its response is dropped.
Selections are revalidated and read in order; failure never rematches a name,
silently omits a skill or falls back to an empty context. Full text, names,
locations and advisory separators must all fit the 65,536-byte context budget.
Context remains provider-only and cannot change canonical user text or grants.

Cancellation can be retained before the first admission poll or before a core
turn handle exists. Worker and core cancellation are invoked outside runtime
locks; later handle publication still receives an earlier request. Continuation
uses its saved inert context without a fresh invocation plan or catalog scan.

## Effect-free picker and draft identity

`NativeSkillPicker` retains a bounded draft mirror, UTF-8 cursor, exact selection
spans and an optional catalog-backed menu. The host supplies actual edit ranges;
matching prefixes or suffixes between two drafts cannot establish which repeated
token was edited. Edits intersecting a binding invalidate it. Unaffected spans
shift with the actual edit, and uncertain token adjacency invalidates bindings
rather than rematching names. Cursor moves invalidate frames, not selections.

Inline completion replaces the captured token prefix through the cursor and
preserves surrounding text. Menu completion inserts at the captured cursor.
Unicode and spaces in names remain exact. Added separators keep neighboring word
text distinct without placing unnecessary spaces before whitespace or punctuation.
Bindings cover the inserted `$name`, excluding any separator.

Frames carry opaque draft-owner/revision, menu-owner/revision and catalog
generation identity. Selection requires acknowledgement of the exact current
frame. Close or Escape invalidates frames while retaining the draft and valid
bindings; reset creates a fresh owner and clears bindings even for identical text.
An exact command-service selection can focus its matching row without resolving
duplicate names. Successful focus invalidates the previous acknowledgement;
foreign, changed or filtered-out selections leave the existing frame unchanged.
Presentation changes such as resize invalidate the frame without changing its
query, selected row or draft, so an older output acknowledgement cannot select.
The host must reset bindings on handoff or synchronization failure. A chosen
insertion has already updated native state: apply its precise edit to the matching
composer exactly once, without echoing it back as a second edit.

The picker can traverse all 1,024 catalog candidates, exposing at most 128 rows
per view with an absolute selected index and window offset. The host sanitizes
display data and visibly reports incomplete discovery. Draft and selection bounds
match invocation planning (256 KiB, 16 selections and 64 KiB retained selection
text); failing edits are atomic. The picker performs no discovery, materialization,
permission granting or filesystem effects.

Picker substring queries use ASCII-only case-insensitive byte matching. Each
filter preprocesses the bounded query once and performs linear work in scanned
name, description and location bytes, including repetitive prefixes. The CLI
does not repeat a cursor transition already applied by an exact edit receipt;
explicit native cursor transitions still invalidate frames.

The CLI menu projection is separately effect-free. It returns one escaped,
64 KiB-bounded frame with at most 128 physical rows, 480 projected bytes per
line, an exact frame identity and a selectable-visibility flag. The viewport
follows the selected absolute row within the native window. Each entry has a
name/description preview and a separate basename/location preview; the footer
states that previews may be clipped. Row numbers distinguish observed entries
without claiming lossless display of long locations. Incomplete-discovery
warnings take priority over ordinary content. Terminals narrower than 16 columns
or shorter than five rows (six with a warning) display a resize notice and cannot
acknowledge a selectable frame. Output height counts actual lines below the
anchor and no trailing newline is emitted.

## Interactive input and refresh

`/skills` and `/skills list` open the observed catalog menu; `show` applies the
native query and exact focus without displaying a skill body. Not-found results
do not open a menu. A separate query editor is limited to 1,024 UTF-8 bytes and
preserves the original draft/cursor. Inline `$` editing instead uses the ordinary
draft and replaces only the exact native token span. Arrow keys move selection;
Enter or Tab selects only after the corresponding visible frame has been fully
written and flushed. A separate newly received Enter submits the resulting draft.

Input retains its first-received draft epoch and query/draft editor identity
through partial UTF-8, escape sequences, paste and buffered remainders. An old
editor's bytes cannot be reassigned to its replacement. Choosing advances the
input epoch without discarding the newly created native binding; a coalesced
Tab/Enter chunk cannot select and silently submit the changed draft. Resize and
navigation invalidate prior frame acknowledgements. Tiny or hidden frames cannot
authorize selection. Escape closes the menu and preserves the original draft
and valid bindings; idle Ctrl-C clears the draft, including a suspended slash-menu
draft. Submission, modal ownership changes and session transitions reset binding
ownership so it cannot transfer to another prompt.

Catalog control results are applied only to their exact request and current
session owner. Every managed batch receipt invalidates the old invocation
snapshot and closes its menu, including partial or uncertain batches. Its outcome
and recovery evidence remain available independently of refresh. Only after the
receipt is flushed does a separate owned List request refresh the snapshot,
without reopening a menu. A failed refresh cannot reuse the pre-mutation
snapshot. Retained explicit bindings are revalidated against any later snapshot,
never rebound by name.

Production interactive startup supplies the first snapshot before accepting
prompts. A host with skills authority but no initial or refreshed snapshot
visibly rejects prompt queueing until discovery succeeds. Incomplete snapshots
still allow valid explicit selections, but suppress automatic matching. That
warning occupies its own bounded pending presentation slot, survives an occupied
notice and is retained into final presentation on shutdown. It cannot replace a
control receipt or cause notices to exceed their output limit.
