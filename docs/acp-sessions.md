# Native ACP sessions

The native ACP session facade wraps `NativeInteractiveSession`; it does not
create an alternative conversation engine or give product ownership to CLI I/O.
The caller injects the verified host, workspace options and authoritative
ephemeral MCP selection. Opening is inert before its first poll.

`acp::prompt` owns provider-independent typed prompt data, bounded decoding and
data-free `AcpPromptError` diagnostics without an HTTP feature dependency. On
Linux and macOS, resource readers, provider-only context and the generic native
conversation FIFO are also available with default features. They still require
explicit workspace and worker authority. The concrete ACP session, selection,
command and connection facade retains the `ai-gateway-http` feature boundary of
its existing `NativeInteractiveSession` and `NativeReferenceHost` composition;
decoding or queuing a resource prompt does not enable that host or HTTP transport.

New sessions persist `acp` provenance. Load and resume select exact native IDs
and reuse native checked adoption. Load exposes an incremental immutable
checkpoint cursor; resume does not replay history. Neither operation executes
saved tools. System messages and provider-only JSON are excluded from editor
history; user/assistant text and recorded tool-call/result evidence are projected
one bounded update at a time without copying the entire transcript.

The facade admits one prompt at a time and rejects foreign session IDs.
Interaction custody uses the session's incarnation-bearing principal. A
streamed event is not a completion receipt: only the native turn outcome follows
owned checkpoint and history finalization. Cancellation and close must keep
polling the native owner until its outcome settles. Close retires live ownership
without deleting durable history.

## Selected session lifecycle

`NativeAcpSelectionOwner` owns one current session and one bounded pending
selection. Requests store inert intent; polling invokes an explicitly captured
host factory with the admitted selection's pure network requirement. Empty or
stdio selections do not capture system DNS; hostname HTTP selections still do.
Each prepared host has its own permission registry, ephemeral MCP
runtime and workers. The connection bridge is shared, but only the committed
host's registry is activated for client permission requests.

The admitted `cwd` spelling is bound during owned preparation to the exact
descriptor-checked primary workspace scope used by host composition. Ancestor
aliases such as macOS `/tmp` and `/var` are accepted without requiring clients
to supply their canonical spelling. Session options and persisted metadata use
the host's canonical identity. Pure prepared-host validation requires that exact
retained primary allocation, and selection separately matches the original
request spelling. Neither validation reopens a path; a foreign scope, substituted
request or same-spelling replacement cannot borrow the binding. Native root
preparation still rejects a symlink in the final workspace component.

New, load and resume first validate the candidate host and ready its complete
authoritative MCP selection. The old prompt is then cancelled and driven through
its exact checkpoint and finalization. Only afterward does reversible native
quiescence admit candidate creation or adoption. In particular, same-ID load and
resume cannot read a stale pre-cancellation checkpoint. Loaded tool JSON preserves
its exact arbitrary-precision numeric representation; history remains inert.
Fresh creation and exact-ID prepare/adopt execute their blocking identity and
store work on the exact host-owned worker scope, not the connection polling
thread. Accepted preparation retains its lifecycle lease through the result even
if a response wrapper is abandoned; async composition stays on the owner driver.

Candidate opening and a final readiness check precede irreversible retirement of
the old runtime. A rejected candidate before retirement restores old admission;
its receipt separately reports whether candidate persistence may already have
occurred. After any candidate opening attempt, failure cleanup retains old
quiescence while an owned worker revalidates the old exact durable ID,
incarnation and revision. A same-ID workspace rebind, missing or unavailable
checkpoint, or another revision change yields an indeterminate fenced result,
not an assertion that the stale old runtime is usable. The controlled read does
not wait on a foreign writer's lock. Retirement or cleanup uncertainty also
fences further mutation and yields an
explicit indeterminate receipt, never a claim that a destructively retired old
selection remains usable. Successful selection follows old native, MCP and actual
worker settlement. Completion is observed asynchronously without admitting a
new observer worker, so collector capacity or thread-admission failure cannot
turn incomplete retired-host cleanup into connection closure. The old principal's
finalized turn outcome is retained separately so the connection can settle its
outstanding prompt response and drain
old completion notices before publishing the new selection.

Cancellation of a selected prompt remains available during candidate preparation
and does not cancel that candidate. Connection shutdown cancels pending selection
intent while continuing to poll already accepted factory, startup, adoption and
cleanup futures. Exact model-save receipts remain drainable during shutdown.
Close validates the selected session ID and reports completed live-resource
retirement, not merely acceptance of a close request. Dropping the owner is only a
last-resort cutoff; it manufactures no successful cleanup receipt.

## Prompt resource context

Typed prompt decoding preserves the order of text and embedded text resources,
including embedded resource URI labels. It bounds joined canonical text to
1 MiB, content blocks to 4096 and each URI to 4096 bytes. URI-only resources can
accompany nonempty canonical text: their targets are separate advisory inputs,
not invented user messages. Binary and image blocks remain explicitly unsupported.
Decoding itself performs no filesystem, network, environment or editor operation.

Only absolute local `file:` URIs with empty authority are eligible instruction
targets. Percent decoding is strict UTF-8; traversal, control/NUL bytes,
backslashes, remote authorities, queries and fragments are ineligible. Normalized
targets are deduplicated and capped at 64. Invalid/over-limit targets retain at
most 32 bounded omission records plus a count for additional omitted records;
embedded text remains intact even when its URI is ineligible.

The native context reader receives an explicit retained workspace scope and
owned worker scope. Its future is inert before polling and checks cancellation
before routing, opens and reads. Materialization retains the exact FIFO admission
lease on the owned worker. The native ACP enqueue path charges text, inference
options, resource paths and omission sources against the shared bounded queue,
with its 1 MiB ACP text limit; ordinary CLI prompts keep their native limit.
No instructions are read while queued. The first-polled take selects the exact
workspace, permission and model observations, then materializes outside queue
locks while retaining that admission lease through cancellation or abandonment.
A target outside
the admitted active roots or inside excluded state is omitted. Descriptor-relative
opens reject symlinks and nonregular targets; target-file bytes are not read into
the prompt. The reader gathers only `AGENTS.md` in the primary root and the
applicable admitted target ancestors, root first, with shared directories
deduplicated. No global/home lookup or unrelated directory scan is implied.

Target depth is capped at 32 components, distinct instruction directories at
128, each instruction file at 16 KiB and combined instruction text/framing at
60 KiB. Missing instruction files are ordinary absence; invalid, unreadable,
binary or oversized instructions produce explicit omissions. Whole instructions
are omitted rather than silently truncated. A bounded provider-visible omission
summary accompanies the materialized context. Retained descriptors prevent path
replacement from redirecting the snapshot; cancellation aborts materialization.

Materialized instructions use a separate `NativeResourcePromptContext`, not
skill identity or canonical user text. Its versioned
`machine_god.resource_prompt_context` metadata contains inert bounded bytes tied
to the exact turn sequence and first user-message index. Continuation decoding
rejects foreign checkpoints, oversized text and malformed versions without
reading resources again. Host integration composes this provider-only data with
other native user context under the shared 65,536-byte core limit; it never
grants tools or permission provenance from instruction text.
Only provider requests receive the materialized instruction blocks. Canonical
user history and permission-review provenance retain the original prompt text.
Cancelled/failed turns keep exact inert context for continuation; successful
finalization and fresh prompts clear the prior context. Resource and skill
metadata identities stay separate, and their combined provider budget is
validated both at turn admission and when inspecting saved checkpoints.

## Native slash commands

ACP routes recognized local slash commands before provider prompt admission.
Invalid or unsupported local commands are explicit rejections, not model input;
ordinary text and slash-prefixed paths containing another path separator remain
ordinary prompts. Unknown slash words are rejected. Command routing
uses the native slash grammar, with its 65,536-byte input bound, and does not
read attached resource targets or change canonical user history.

`available_commands_update` advertises only the same-session native subset:
`help`, `status`, `model`, `compact` and `undo`; `permissions` and effective-view
`allowlist` require current permission ownership; `models` requires an injected
catalog, and `fast` requires the selected model's advertised support. `skills`
lists an explicitly injected native skills service. `mcp` reports ephemeral
publication state and supports existing human resource/prompt feature operations
through that selected runtime. No profile MCP fallback, mutation, authentication
or logout command is exposed. Skills installation and management, persistent
allowlist edits, user-default model saves, terminal UI operations and
identity-changing slash commands are not advertised. Session new/load/resume
remain the dedicated ACP lifecycle methods; deferred product categories stay
deferred. The pinned ACP advertisement is reference data, not evidence that its
slash entries have handlers.

Model and effort selection and fast toggling affect the exact native session;
the receipt distinguishes accepted preference generation from saved, unchanged,
deferred or not-started persistence. Compact, undo, skills discovery and MCP
feature commands use the existing owned native control lane. One accepted
command and its exact control ID/incarnation retain custody through cancellation
and shutdown. Cancellation is an intent, never a claim that an already completed
effect was undone. A mismatched completion leaves both the pending command and
the supplied foreign receipt untouched. A late cancel after native receipt
drainage does not create new cancellation intent. An uncertain undo explicitly requires
manual inspection; generic native failures report unconfirmed effects and never
request automatic retry. The connection can annotate observed cancellation using
the exact principal; this metadata operation cannot cancel native work itself.
Model-save and command-control receipts
have separate facade custody; selection retirement waits for both to drain.

Completed commands produce an agent text update with a structured
`command_result` extension, never a fabricated model turn or tool-call ID. The
result retains the underlying native receipt until presentation releases it.
Updates are bounded to 64 KiB, structured data to 32 KiB and catalog projections
to 128 rows, with explicit omission counts and bounded text previews. MCP JSON
numbers preserve their exact admitted representation. Large individual MCP
response data is explicitly omitted rather than eagerly copied; this does not
change its native operation receipt. Debug and error diagnostics omit inputs,
paths, results and nested native errors. The connection emits the command update
before settling the original prompt RPC, using cancellation status independently
from native effect failure.
When a command changes the actual live model, a complete bounded
`config_option_update` for its original principal precedes that command result
and prompt response. Accepted live preferences can survive a failed or cancelled
save; the configuration update reports those preferences, while the separate
command receipt reports persistence honestly. Unchanged, read-only or rejected
commands with no live configuration change emit no configuration update.

## Session configuration

Permission modes are the native `ask`, `auto` and `yolo` selections. Changes
affect future taken jobs, not a running turn or persisted permission rules.
Modern `configOptions` is the only wire configuration interface; there is no
older `session/set_mode` method or duplicate `modes` projection. The `mode`
configuration option uses the same session policy. The `model`
configuration option changes the same runtime's model preferences;
its acceptance generation is separate from session-only persistence. It never
writes user-default configuration. Session saves use the existing owned native
control lane: dropping a response wrapper does not discard an accepted save.
Listing delegates to the host's bounded
native catalog and opaque cursor. The production factory resolves existing
workspace aliases on its owned list worker, without preparing a host or creating
directories. A missing path (including an ancestor that is no longer a directory)
retains its literal filter so canonical history remains listable after workspace
deletion; other resolution failures are errors. This workspace scope is a
descriptive observation, not filesystem authority or a stable identity binding.
