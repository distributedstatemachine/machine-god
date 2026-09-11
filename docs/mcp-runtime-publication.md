# Native MCP runtime publication contracts

## Shared immutable executable bindings

`McpSubmissionRuntimeBinding::shared` retains shared immutable configuration and
authentication allocations and a cheap clone of the admitted schema. A catalog
with many tools does not need a separate copy of server configuration, resolved
authentication identity or schema source for each executable binding.

Both the existing copying constructor and the shared constructor enforce the same
finite logical identity bound before acquiring new storage. Shared storage does
not relax identity equality or permission evidence: exact original schema bytes,
configuration bytes, authentication bytes, server, remote tool and exposed tool
remain bound to the retained runtime allocation. Debug output remains redacted.
Publication composition must additionally bound aggregate distinct allocations;
sharing is not permission to retain an unbounded number of tools or generations.

The native runtime owner supports deferred retirement for atomic catalog swaps.
Deferral marks the old binding invalid immediately without invoking a waker.
The caller completes the returned retirement only after releasing its publication
locks. It must mark every old executable binding before exposing replacements.
Completion wakes observers outside those locks; no proof destructor, peer closure
or user interaction is performed by marking a binding invalid. Ordinary runtime
replacement and retirement preserve their existing invalidate-then-wake behavior.

## Exact HTTP head ownership

Typed HTTP request projection retains its immutable projected head allocation
through copied request, prepared submission, permission admission and final
one-shot claim. The native runtime can obtain that exact head without rebuilding
it from mutable connection configuration, current credentials, or model arguments.
The actual permission-bound wire bytes remain unchanged.

This read-only head accessor grants no destination or execution authority. The
existing concrete permission proof, runtime allocation checks, peer-minted request
lease and final plaintext writer checks remain mandatory. Legacy raw preparation
does not gain a typed projected head or access to a dynamic runtime route.

These contracts provide primitives for native publication and execution; they do
not independently discover servers, publish tools, activate MCP in a CLI, or
resolve input-consent continuations.
