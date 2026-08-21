# Security

The core is designed to have no ambient filesystem, process, environment,
credential, or network authority. Native capabilities will be supplied explicitly
by a host. When native tools land, the CLI must use permission mode `ask` by
default and unresolved noninteractive requests must fail closed. These are future
invariants until their milestone is implemented and tested.

The threat model must cover workspace escape, symlink races, command injection,
permission confusion, SSRF, secret exposure, corrupted state, denial of service,
and cancellation or shutdown races.

Every logical session lifetime has a validated, host-generated
`SessionIncarnationId` persisted in its `SessionRecord`. Core has no clock or
randomness and never derives a fallback. Reusing, resetting, or rewinding a
`SessionId` requires a new globally unique incarnation; stores reject changing
the incarnation of an existing record, and an engine rejects a different
incarnation while that session ID is live. Permission request IDs use the v2
SHA-256 preimage over length-delimited session ID, incarnation ID, turn ID, and
ordinal. This prevents an ID-keyed permission cache from replaying an allow into
a fresh logical lifetime whose turn numbering and tool-call ordinal restarted.

Benchmark CI obtains Zig only from the official Zig 0.16.0 HTTPS archive and
verifies SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extracting into a fresh fixed directory under `RUNNER_TEMP`. The workflow
then fails unless the installed executable reports version `0.16.0`. This keeps
the upstream-reference compiler outside the Rust product's dependency and
authority surfaces while binding its CI bytes without a third-party setup
action.
