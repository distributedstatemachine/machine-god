# Skills CLI components

This document describes native components supporting human-invoked skills.
Implementation status and delivery gates are tracked only in the
[implementation plan](implementation-plan.md). The native component APIs below
do not by themselves register an interactive CLI command or install authority.
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
The host must reset bindings on handoff or synchronization failure. A chosen
insertion has already updated native state: apply its precise edit to the matching
composer exactly once, without echoing it back as a second edit.

The picker can traverse all 1,024 catalog candidates, exposing at most 128 rows
per view with an absolute selected index and window offset. The host sanitizes
display data and visibly reports incomplete discovery. Draft and selection bounds
match invocation planning (256 KiB, 16 selections and 64 KiB retained selection
text); failing edits are atomic. The picker performs no discovery, materialization,
permission granting or filesystem effects.
