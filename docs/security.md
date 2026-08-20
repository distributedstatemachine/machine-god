# Security

The core has no ambient filesystem, process, environment, credential, or network
authority. Native capabilities are supplied explicitly by a host. The CLI enables
native tools with permission mode `ask` by default; unresolved noninteractive
requests fail closed.

The threat model must cover workspace escape, symlink races, command injection,
permission confusion, SSRF, secret exposure, corrupted state, denial of service,
and cancellation or shutdown races.

