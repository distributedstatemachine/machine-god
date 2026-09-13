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

Prompt decoding accepts text and embedded text resources, preserving their order
and resource URI labels. It bounds the joined text to 1 MiB, content blocks to
4096, and individual URI labels to 4096 bytes. A resource URI is descriptive;
decoding never reads local files, fetches remote resources or delegates to editor
filesystem/terminal APIs. URI-only, binary and image resources are explicitly
unsupported instead of silently disappearing from a submitted prompt.

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
