# Security

The core is designed to have no ambient filesystem, process, environment,
credential, or network authority. Native capabilities will be supplied explicitly
by a host. When native tools land, the CLI must use permission mode `ask` by
default and unresolved noninteractive requests must fail closed. These are future
invariants until their milestone is implemented and tested.

The threat model must cover workspace escape, symlink races, command injection,
permission confusion, SSRF, secret exposure, corrupted state, denial of service,
and cancellation or shutdown races.
