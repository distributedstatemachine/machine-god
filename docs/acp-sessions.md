# Native ACP sessions

The native ACP session facade wraps `NativeInteractiveSession`; it does not
create an alternative conversation engine or give product ownership to CLI I/O.
The caller injects the verified host, workspace options and authoritative
ephemeral MCP selection. Opening is inert before its first poll.

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
host factory. Each prepared host has its own permission registry, ephemeral MCP
runtime and workers. The connection bridge is shared, but only the committed
host's registry is activated for client permission requests.

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
worker settlement. The old principal's finalized turn outcome is retained
separately so the connection can settle its outstanding prompt response and drain
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

## Session configuration

Permission modes are the native `ask`, `auto` and `yolo` selections. Changes
affect future taken jobs, not a running turn or persisted permission rules.
The `mode` configuration option uses the same session policy. The `model`
configuration option changes the same runtime's model preferences;
its acceptance generation is separate from session-only persistence. It never
writes user-default configuration. Session saves use the existing owned native
control lane: dropping a response wrapper does not discard an accepted save.
Listing delegates to the host's bounded
native catalog and opaque cursor; workspace scope is a descriptive filter,
not filesystem authority.
