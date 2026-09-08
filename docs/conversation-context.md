# Native context preferences and summaries

`NativeContextPreferences` is a pure native value: parsing, configuration,
compaction and projection perform no filesystem, process, clock, model, network,
permission or persistence operation. The native conversation owner publishes
its preference value through an exclusive metadata transaction and passes a
projection derived from the same record revision to core prepared-turn admission.
The value itself never mutates `SessionRecord`, canonical messages or archives.

## Preference schema

The reserved metadata key is `machine_god.context_preferences`. Absence means
`first_retained_message = 0` and `max_history_turns = 0`; the latter disables
automatic compaction and preserves the current default. Schema 1 has exactly
three unsigned integer fields: `schema_version` (exactly 1),
`first_retained_message`, and `max_history_turns`. Unknown/missing fields,
unsupported versions, floats, negative values and non-scalars fail closed.
Decoding examines only this shallow entry, without cloning or traversing
unrelated metadata. `to_value` emits that exact shape.

Both preference scalars are bounded by `MAX_FILE_SESSION_BYTES`, a ceiling larger
than the maximum possible stored message/group count. The fallible
`set_max_history_turns` rejects larger values without mutation. Private fields
keep this invariant; getters expose the scalar observations. Preferences implement
`Clone`, `Default` and equality; their `Debug` omits the values. Errors have fixed,
data-free variants and diagnostics.

## Canonical groups and selection

A logical group begins at a `User` message and includes all following assistant
and tool messages until the next user message. No-input continuation therefore
extends the current group. Leading system messages remain outside group counts
and are preserved by core projection. A nonzero saved cursor must point to an
existing user boundary; stale/out-of-range indices fail rather than clamp or
repair themselves. A cut cannot discard a nonleading system message.

`force_compact` selects the last existing logical group and returns whether the
cursor changed. Empty and single-group histories are no-ops. It never truncates
history, produces a new session, or performs persistence. Repeated compaction
recomputes the summary from the actual canonical prefix, not a growing persisted
summary string.

`projection` first summarizes the explicit canonical prefix and considers that
summary one local history item, followed by retained logical groups. It then
applies the pinned automatic policy:

- Zero disables automatic compaction.
- One keeps the last group without a summary when the local history overflows.
- At least two, on overflow, keeps `min(max_history_turns - 1, 4)` recent groups
  and summarizes the removed groups, merging the existing explicit-prefix summary
  when one is present.

The returned core `SessionContextProjection` selects the canonical suffix and
carries optional advisory text; `None` means full context with no summary.
Automatic selection does not change the stored manual cursor. Core retains the
leading systems, frames summaries as untrusted assistant context, and preserves
the same projection throughout the new turn. Summary text is not a root-user
request, instruction, tool result, permission feedback, grant, or replay command.

## Deterministic summary rules

The source is pinned `vercel-labs/fx` commit
`b1774fbf6c7602b503026f96f6e960e946c692ef`,
`src/core/session/session.zig`: `forceCompaction` at 1977,
`snapshotOwnedContextHistory` at 2002, `appendCompactedPrefix` at 2027,
`compactHistory` at 2976, summary helpers at 3134–3238,
`formatToolResultEvidenceLine` at 3328, and compression/text helpers at
3396–3540. These are source mappings, not a claim that every typed upstream
history variant is already represented by the current native record.

The exact heading is `Conversation summary:` followed by the count of groups
removed in that summarization pass. Existing-prefix text is introduced by
`- Previously compacted context:`. Summary sections visit actual removed history
in order, taking the first four nonempty user texts, first three available
assistant outcomes, and first four correlated tool results. Duplicate candidates
consume those quotas before final deduplication, as upstream does.

User and assistant text collapses only SP, TAB, CR and LF and is truncated at a
complete UTF-8 boundary within 156 bytes, without ellipsis. Multiple text blocks
within one message are separated by a collapsed space. For a Rust group with
multiple assistant/tool rounds, the last nonempty assistant text is the available
outcome; its presence does not assert successful completion. Final line compression
trims those same four whitespace characters, deduplicates exact normalized lines,
and counts duplicates and budget-rejected lines as omitted. The summary contains
at most 1,200 UTF-8 bytes and 24 lines. The fixed omitted-line notice is appended
only if both budgets permit it. Despite upstream's `chars` names, these limits
measure bytes, not Unicode scalar counts.

Tool results correlate to calls within closed assistant/tool rounds. Reused IDs
in later closed rounds are allowed. Inline results report actual compact native
`ToolOutput` envelope bytes and the stored success/error flag. Explicit
`tool_result_unknown` error markers report `unknown`, never success or proof of
nonexecution. A strict native archive receipt with matching session, incarnation,
call ID and an already-reserved turn sequence contributes its advertised source
byte count, canonical handle and 96-byte collapsed preview. This is historical
receipt text, not a new archive existence/authenticity check or read authority.
Malformed or uncorrelated archive-shaped JSON remains ordinary inline evidence;
it does not advertise a trusted archive handle. Complete results and unknown
placeholders remain in canonical history and are available to ordinary archive
readers regardless of which summary lines fit.

## Input and work bounds

Public raw-record methods validate borrowed input before summary scans or JSON
serialization. The native store envelope, not generic core defaults, bounds this
work: compact `SessionRecord` bytes are at most `MAX_FILE_SESSION_BYTES`
(8,651,165), with the store's aggregate 65,536 JSON nodes and 64 container-depth
ceiling. Message and content-block prewalk ceilings are conservatively derived
from that byte limit and minimum serialized element sizes. Raw text/key-byte
accounting rejects oversized values before encoding; JSON traversal keeps one
borrowed iterator per active container and never recursively clones or drops
caller values. Serialized size is counted without an output buffer.

There is no additional 4,096-message, 8 MiB transcript or 256 KiB metadata cap:
those are generic engine defaults, not the maximum native store envelope.
Configured core limits still apply independently when the owner admits the
prepared turn. Missing calls/results, duplicate round-local IDs, orphan results,
non-result tool messages, and messages before any user group (except leading
systems) fail closed. Caller-owned rejected records remain unchanged; their owner
is responsible for safe eventual destruction of values it constructed.

## Required typed-fact integration

The current `SessionRecord` has no authoritative per-group upstream
`background_command` or `interrupted` variant and no `FileEvidence.stale` field.
This module deliberately does not synthesize background activity, interruption,
file staleness, permission feedback or root-user authority from missing assistant
text, tool names, unknown result markers or unrelated metadata. Native-owned
durable typed facts and their summary mapping remain required integration work
for the full feature; this component does not close those compatibility scenarios.

See [native conversation ownership](native-conversation.md) for owner admission
and checkpoint lifecycle, and [core API](core-api.md) for revision-pinned prepared
turns and advisory provider-only projection.
