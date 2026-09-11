# Native MCP authorization

`mcp::auth` owns OAuth discovery, dynamic registration, PKCE, approved browser
callbacks, credential persistence, refresh and logout. It follows the protocol
behavior in pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef`
`mcp_auth.zig` and `mcp_auth_store.zig`. The
[implementation plan](implementation-plan.md) remains the sole live ledger.

## Explicit authority

Configuration is built from an admitted remote configuration, captured secret
environment lookup and selected authentication identity. Constructors perform
no filesystem, environment, network or browser effects. A host injects the
private native profile directory, secure entropy, monotonic timer, wall-clock
milliseconds, per-URL destination/trust admission and browser presenter/launcher.

Every protected-resource, issuer-metadata, registration, token and revocation
URL is independently admitted. Resource headers and bearer credentials are
never forwarded to discovery. Client authentication goes only to its selected
token/revocation endpoint. HTTPS and explicit-port loopback HTTP are supported;
a public resource cannot direct OAuth to plaintext loopback. Redirects are not
followed, and consequential POSTs are not automatically retried. The existing
[owned HTTP connector](mcp-http.md) supplies TLS verification, cancellation,
framing and bounded response ownership. OAuth GET/JSON/form controls are private
native constructors, not a public raw HTTP or tool-submission bypass.

## Discovery and authorization

Protected-resource metadata uses an explicit challenge URL or path-specific
then origin well-known locations. Authorization metadata supports OAuth and
OpenID well-known locations, including issuer paths. Resource coverage, exact
issuer equality and S256 support are admitted before browser authorization.
Configured clients, HTTPS client-metadata URLs and dynamic native registration
are supported. Token authentication implements `none`, `client_secret_basic`
and `client_secret_post` with percent-encoded form values.

Scope selection preserves the prior scope, then challenge/configured/resource
scope priority, deduplication and supported `offline_access`. Each scope has at
most 64 tokens of 256 bytes. Runtime challenge sequences own the pinned maximum
of two scope reauthorizations; this service executes one explicit authorization.

Native owns an ephemeral `127.0.0.1` callback listener before registration fixes
its redirect URI. A presenter must approve before the launcher is called.
The callback must be GET `/callback`, with bounded headers, exact state and the
required/returned issuer. Duplicate query parameters, invalid percent escapes,
wrong state and wrong issuer fail without token exchange. Entropy supplies a
48-byte PKCE verifier source and independent 32-byte state source. No listener,
browser process or refresh worker is detached by this service; the injected
launcher remains responsible for any process it starts.

OAuth documents are limited to 256 KiB, URLs to 4 KiB, secrets to 16 KiB and
callback headers to 16 KiB. Browser URL output is bounded to 32 KiB. JSON uses
core's exact-number, duplicate-rejecting decoder. Token expiry accepts only
nonnegative signed-64-bit integer seconds, saturates arithmetic and defaults
to no expiry. Refresh uses the pinned 60-second skew and retains an omitted
refresh token, scope or token type.
Expired credentials without a refresh token require explicit reauthorization;
they are not reported as missing credentials or bypassed anonymously.

## Credentials and generations

The explicitly selected directory owns `mcp-credentials.json`, with separate
`.mcp-credentials.lock` and `.mcp-credentials.tmp` names. The store is private,
bounded to 1 MiB and 64 entries, and reuses descriptor-relative source/inode
observations, nonblocking cooperative writer locks, atomic replacement and
independent durability receipts. Missing read-only observations create nothing;
malformed selected files are errors. Native storage is not an fx-file import,
settings-schema extension or ambient macOS keychain selection. Both supported
platforms use this explicitly selected native file backend.

Credentials are partitioned by the exact endpoint, including query, and a
digest of selected client/OAuth/authentication configuration and captured secret.
The stored resource, issuer and registered client remain part of the admitted
record. Equal-byte file replacement by another inode conflicts. A stale refresh
cannot overwrite a changed cooperative store observation.

One service serializes authorization/refresh for each identity. Logout removes
the exact live incarnation and cancels its credential generation before storage
or network effects; an older operation cannot republish after that retirement.
Successful refresh retires the previous lease. Typed invalidation events and
lease cancellation observers let the runtime retire the matching executable
allocations. Hooks and cancellation wakeups run outside coordinator locks.
Separate processes share file CAS, not an in-memory generation coordinator.
Runtime reload/removal calls local-only `retire` for superseded identities to
release their slots without deleting persisted credentials or contacting OAuth.

Logout reports local unchanged/removed/ambiguous/failed separately from remote
confirmed/unsupported/ambiguous/not-attempted. A local write failure is not
hidden by successful remote revocation. Refresh and access tokens are revoked
separately when available; a receipt never asserts that remote work was undone.
If logout retires an in-flight authorization or refresh, the aggregate remote
receipt remains ambiguous even when revocation of known tokens is acknowledged:
an already-submitted exchange may have issued an unknown replacement token.
Removing the last entry publishes an empty native document rather than deleting
the credential namespace. Secure erasure and keychain/encrypted persistence
remain outside this native file contract.

## Cancellation and cleanup

Every exchange uses the shorter selected deadline or 30 seconds. Interactive
authorization uses at most five minutes, and accepted callback I/O at most
30 seconds. Cancellation or dropping a polled future releases its owned socket
and listener without replay. Constructors and unpolled futures remain inert.
The bounded synchronous filesystem transaction starts no detached worker;
filesystem syscall latency is not a hard wall-clock deadline guarantee.

Debug/display omit endpoint, issuer, identity, challenge, callback and credential
contents. Owned secret strings and serialized store buffers are overwritten on
release where practical; this is best effort, not comprehensive zeroization of
parser temporaries, allocator history, HTTP copies or caller-owned values.
