# MCP request reservation ownership

An optional peer-minted, non-clone `McpToolReservation` travels unchanged from
`McpToolRequest` through typed preparation, core admission, claim and final
writer submission. Reading its RPC ID grants no execution authority.

`with_reservation` rejects a foreign RPC ID and repeated attachment. The native
peer verifies the exact retained allocation, not merely an equal ID. A peer
retains only a weak observer, so dropping a denied, cancelled or abandoned
request makes its unsent slot reclaimable on the peer's next owned operation.
Marker destruction performs no callback, locking, I/O or remote cancellation.
Request IDs are never recycled. The explicit raw-ID API stays separate and
cannot substitute a foreign marker for a manually owned reservation.

The marker does not replace the submission registry's exact-turn reservation,
immutable schema/configuration/authentication binding, concrete native permission
proof, or final writer checks. A prepared submission can revalidate those live
preparation bindings without publishing or authorizing the request.
