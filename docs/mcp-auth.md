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

Startup retains exact bounded HTTP authentication status and ordered
`WWW-Authenticate` bytes from connection and initial tool-catalog failures.
These are external observations, not parsed scope, destination, consent or
browser authority; no login or retry follows automatically. Lookup remains
historical after failure/cleanup. Revalidation requires the same startup source,
latest per-server observation and original owner/configuration/network/auth
lifetimes; actual profile revalidation and explicit command consent remain host
obligations. A new attempted server replaces the latest observation. Failed
retention reports `Limit`, never stale previous headers. At most 128 retained
observations (including externally held replacements), each with at most eight
headers and 16 KiB of header bytes, share the startup retained-byte budget with
candidate catalogs. Formatting never exposes challenge or server contents.

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

## Owned browser launcher

`NativeMcpBrowserLauncher` binds a retained executable, explicitly captured
environment and the host's actual `NativeOwnedWorkerScope`. Construction and
unpolled requests perform no browser or environment effects. Its separate
32 KiB HTTP(S) URL representation retains exact input bytes, rejects credentials,
missing hosts, backslashes and whitespace/control characters, and redacts debug
output. This does not widen the background server detector's 2 KiB URL bound.

The shared background/direct-child launcher path passes one URL argument without
shell interpolation, clears ambient environment, uses null standard streams and
a fixed root working directory, and revalidates the retained executable identity.
The host must protect the executable installation throughout its lifetime.
Original caller and owner cancellation are checked through worker admission,
after reaper reservation immediately before OS spawn, and through observation.
The original deadline is capped at ten seconds from
first poll; the existing background ten-second contract is unchanged. No retry
is automatic and no separate runtime or cleanup owner is created.

Clones retain one admission until actual direct-child reap. Cancellation, expiry
or dropped observation after spawn preserves owned cleanup and may report an
indeterminate handoff. A successful launcher exit is only `Opened`; a failed
exit does not prove no browser opened. Neither URL admission nor any launch
receipt is user consent, OAuth success, completion notification or retry proof.

Native interactive sessions derive their MCP launcher from the already-bound
background URL opener. Both retain the exact same executable, captured environment,
host worker scope and one active admission through direct-child reap; there is no
second launcher capture or owner. Missing or invalid optional desktop-launcher
authority disables both paths without preventing interactive startup. This inert
composition changes neither public startup options nor consent requirements.

## Interactive authentication commands

`/mcp auth NAME` selects an existing remote server and asks the user to repeat
the command with `--open`; it does not resolve OAuth secrets, load credentials,
contact OAuth endpoints or launch a browser. A disabled or failed remote server
can be selected. Stdio and unknown names are rejected. `--open` is explicit
browser consent, without a second issuer-approval dialog. An issuer mismatch
requires correcting the selected server's `oauth.issuer` and retrying.

The native command retains the actual conversation admission, selected profile
snapshot, controller reservation, original cancellation and deadline through
credential-worker completion, including abandoned observations. Authentication
commands serialize with controller startup/reload. Credential publication is
validated against the selected profile, and its receipt is separate from the
subsequent configured runtime reload. A reload failure does not undo a confirmed
credential save or establish that the server connected. `/mcp logout NAME`
reports local removal and remote revocation independently, without implicit
runtime reload. Secret-bearing authorization URLs go directly to the retained
native launcher and are never placed in recorded CLI output.

Authentication may reuse the latest failed startup's challenge bytes only after
freshly validating that observation's exact profile source. This is a new
explicit command selection, not revival of the failed startup's cancelled
authority. A changed source discards historical challenges; credential workers
still revalidate the newly selected profile before loading or publishing.

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

Controller-selected stored authentication also retains the exact native
configuration snapshot and its owner/configuration cancellation. Credential
workers revalidate that source around loading, and acquire the nonblocking
configuration lock before a refresh publication, holding it through the
credential transaction. Lock order is configuration then credentials; no lock
is held during OAuth network or human interaction. Changed or equal-byte
replaced configuration rejects publication before the credential write. A
detected noncooperative source change after credential publication remains
ambiguous, not a claim that nothing was written. This is cooperative exclusion,
not an atomic transaction against arbitrary writers ignoring both locks.
Profile-selected leases and their cancellation waiters retain the original
owner/configuration cutoff. Explicit injected `Stored` selections remain
independent of native profile selection.

Each issued lease retains its selected authorization clock and an immutable
monotonic hard expiry mapped from the persisted wall-clock deadline. Wall-clock
rollback cannot extend that issued lease; current forward wall-clock expiry can
reject access earlier. `expires_at` reports that hard deadline in the selected
authorization clock's domain, which must not be compared with an unrelated
injected clock. The no-expiry sentinel remains unbounded by token time, not by
generation or profile authority. Unrepresentable finite deadlines fail before
credential publication rather than becoming unbounded leases.
The bounded identity slot retains one exact credential issuance with its
generation. Reacquiring equal credentials reuses that original deadline;
wall-clock rollback cannot postpone refresh by issuing another lease. Changed
stored credentials or a successful credential publication select a new issuance
and retire the prior generation, with cancellation outside coordinator locks.
`refresh_due` checks the original generation/profile and the pinned 60-second
skew, returning true even after expiry; expired `access_token` calls are rejected.
`cancelled_owned` observes the original cutoff and hard expiry using the selected
timer only when polled. It creates no detached watcher and does not change the
meaning of the original generation cancellation token.

One service serializes authorization/refresh for each identity and retains at
most 128 transient identities, 128 live selection owners and 128 pending
operation reservations, including retired
operations and worker results that have not been consumed. Async credential
loads and publications run on the injected actual host worker scope; OAuth and
browser futures stay on the existing caller runtime. `status_owned` provides
worker-owned read-only status. Synchronous `status` is only for an explicitly
selected caller worker, not an async polling thread.

Logout cuts off the exact live incarnation on first poll, then waits for older
admitted operations before deleting credentials. `retire` is likewise an
acknowledged async operation: it cuts off authority immediately, but success
proves that older admitted publications have finished. Retirement introduces no
store mutation or OAuth request of its own. A publication admitted before the
cutoff may finish and retain its actual durability outcome, but its returned
lease cannot supply credentials after retirement. Same-identity admission stays
busy while retired predecessor work remains unresolved; cancellation does not
release a running worker's reservation early. No coordinator mutex is held
during filesystem I/O or while invoking cancellation wakeups.
Successful refresh retires the previous lease. Typed invalidation events and
lease cancellation observers let the runtime retire the matching executable
allocations. Hooks and cancellation wakeups run outside coordinator locks.
Separate processes share file CAS, not an in-memory generation coordinator.
Native stored-auth startup retains an exact service-and-identity selection
before loading credentials, including a `Missing` result. At most 64 identities
belong to one startup; the transient service cap permits bounded old-plus-new
configuration overlap without increasing the persisted file's 64-entry cap.
Simultaneous selections of the same identity share custody. Rejected or
abandoned candidates release only their own selections; successful controller
publication releases the previous configuration's selections after cutover.
Removal and controller close release those selections even when historical
startup or publication receipts remain retained. Batch cleanup also releases
completed peers' selections. An explicit auth/logout command retains its own
selection through the actual profile worker and returned lease custody.

The last selection's drop cuts off only its matching live slot and notifies
outside coordinator locks. It performs no file mutation or OAuth request and
does not claim retirement acknowledgement: pending workers and their results
keep retired slots charged until actual completion. Subsequent admission and
cleanup observations prune settled slots. The public async `retire` operation
remains available when an explicit acknowledgement is needed. A live overlapping
selection prevents unrelated candidate failure from retiring its identity.

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

Every exchange uses the shorter selected overall deadline or 30 seconds. The
five-minute interactive callback wait begins after successful browser handoff;
discovery, registration and consent do not consume that window. Accepted callback
I/O receives its own at-most-30-second budget, and the subsequent token exchange
uses its own network budget, not the callback-wait cutoff. The caller's original
overall deadline and cancellation constrain every phase. Cancellation or dropping
a polled future releases its owned socket and listener without replay.
Constructors and unpolled futures remain inert.
Constructing the service or an unpolled operation starts no worker. Once polled,
persistence jobs retain their admission and result custody through the selected
host worker collector, including when the caller drops its future. An admitted
publication is not selected away when cancellation races its completion, and
an accepted local logout outcome is not erased by later revocation cancellation.
Filesystem syscall latency is not a hard wall-clock deadline guarantee.

`close` is an immediate service admission/credential cutoff, not a completion
claim. `settle` uses an independent cleanup token and selected-clock deadline to
observe pending auth operations and worker bodies; interrupted observation leaves
the same obligations available through `cleanup_status` and later settlement.
Neither operation closes or joins the shared host worker scope. The actual host
separately performs its final worker join, including thread-local destruction and
collector cleanup, before releasing host resources.

Debug/display omit endpoint, issuer, identity, challenge, callback and credential
contents. Owned secret strings and serialized store buffers are overwritten on
release where practical; this is best effort, not comprehensive zeroization of
parser temporaries, allocator history, HTTP copies or caller-owned values.
