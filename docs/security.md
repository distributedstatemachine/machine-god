# Security

The core has no ambient filesystem, process, environment, credential, or
network authority. Native capabilities are supplied explicitly by a host. The
first Milestone 03 native slice only snapshots config/state environment inputs
and reads final-path metadata for status. A second bounded native authority
loads configuration synchronously and read-only. A third, provider-neutral
slice adds capability-aware tool preflight without exercising native authority.
A fourth slice adds a bounded Unix-only `read_file` implementation behind that
preflight seam. A fifth adds bounded one-level Unix-only `list_files`
enumeration behind the same seam. A sixth adds a bounded AI Gateway request and
stream codec behind an injected host transport; the codec itself receives no
endpoint, credential, socket, TLS, status, retry, clock, or runtime authority.
A seventh integrated slice adds one optional native implementation of that
transport with a pinned production HTTPS endpoint and an explicitly injected
bearer token. An eighth integrated bounded slice adds a Unix file-backed
`SessionStore` beneath one explicit retained host directory descriptor. The
seventh slice's feature-branch review and exact remote runs are green and
recorded in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md).
It is integrated on `main` at
`508b0adbbe4447a85bd08f47095ae16c089c05d5`; exact main CI run `32535790803`
and benchmark run `32535790824` are green. This is not a production-ready
claim.
The eighth slice's exact feature, documentation-seal, and `main` checks are
green, with evidence retained in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
It is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`; exact main CI run `32541315998`
and benchmark run `32541315997` are green.
A ninth integrated bounded slice defines the fail-closed native
`AskPermissionHandler` over an explicitly injected `PermissionPrompter`. It
does not contain a prompt UI or change CLI behavior. Its implementation and
black-box tests, three-track adversarial review, documentation seal, and exact
feature and `main` workflows are green. It is integrated on `main` at
`27e3f2b3ff170044732d9124ffb210beabcda206`; exact main CI run `32570197911`
and benchmark run `32570197870` are green. Permission mode remains `ask`; CLI
registration and a concrete prompt remain future work. The slice's exact
security boundary and review evidence are in
[`ask-permission.md`](ask-permission.md).
A tenth integrated bounded slice adds opt-in native credential discovery
behind the existing `ai-gateway-http` and non-WASM gate. It owns a separate
secret snapshot and does not add credential authority to core, native
configuration, or the CLI. It is integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`; exact main CI run `32573320962`
and benchmark run `32573320937` are green. Its exact boundary is in
[`ai-gateway-credentials.md`](ai-gateway-credentials.md).
An eleventh integrated bounded slice advances the built-in and current native
config schema to strict v2 while keeping exact strict v1 files read-compatible
without rewrite or migration. It is integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`; exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. See
[`configuration.md`](configuration.md) and the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).
A twelfth bounded slice implements Linux/macOS-only library composition
behind the existing `ai-gateway-http` and non-WebAssembly gate. Production
implementation, independent black-box tests, three fresh adversarial tracks,
and exact feature and `main` workflows are green; its final delivery record is
`ac3984fb16dbab3adf86a949c7555ceca7c3e8df`, with exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` green. Its
boundary is in [`native-reference-host.md`](native-reference-host.md).
A thirteenth bounded slice advances native configuration to strict v3 with
one required closed, non-secret `environment` credential-source acquisition
kind. Exact v1/v2 files remain readable and gain that projection only in
memory. Production implementation, independent tests, local gates, and all
three fresh adversarial tracks are green on exact behavior SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. It is integrated on `main` at
`8755757da0da07e33af48d57f46bd9ea490b5449`; exact main CI run `32582978232`
and benchmark-evidence run `32582978286` are green. Its final delivery record is
integrated on `main` at `f840576af241c58d1e55399e66ba92f7770cd50c`;
exact feature CI run `32583585145`, feature benchmark-evidence run
`32583585148`, main CI run `32583871385`, and main benchmark-evidence run
`32583871368` are green for that exact SHA.
A fourteenth composed candidate adds explicit Linux/macOS native root selection
and narrowly bounded safe preparation. It opens and retains the
workspace before state work, may create only a fixed descriptor-relative state
suffix under an existing selected base, validates rather than repairs existing
directories, rejects root equality or ancestry, and preserves credential
discovery after retained-root preparation. Production and 14 independently
owned focused tests are present, and focused gates are green. A preliminary
audit is green but does not satisfy any of the required three fresh formal
adversarial tracks. Those formal tracks, full gates, remote workflows, and
`main` integration remain pending. Its candidate security boundary is in
[`native-root-selection.md`](native-root-selection.md).

Status resolution recognizes only the `machine-god` namespace. Empty XDG
values fall back to `HOME`; a selected nonempty relative or non-Unicode root is
invalid and cannot be bypassed through a `HOME` fallback. Missing or empty
`HOME` is unavailable. Inspection uses `symlink_metadata`, so a final symlink is
classified as the wrong kind instead of followed. It does not canonicalize,
open or parse `config.json`, create locations, or write state. These constraints
make status a bounded observation surface, not configuration authority.

Status paths are user-visible and may disclose the selected config/state
location when the user explicitly runs the command. JSON output uses JSON
escaping, and human output uses the same JSON-string encoding for paths. C0/C1
controls, Unicode line/paragraph separators, and bidirectional-formatting
controls are emitted as escapes, so environment-derived paths cannot inject
terminal lines or visually reorder the surrounding status fields. Non-UTF-8
CLI arguments are rejected as invalid. Output errors use a fixed diagnostic
rather than reflecting path or OS-error text.

Configuration loading uses the same config-location selection. In the
thirteenth slice, missing and unavailable locations yield the explicit
strict schema-v3 built-in values: permission mode `ask`, provider
`vercel_ai_gateway`, transport `ai_gateway_http`, model `zai/glm-5.2`, and the
non-secret credential-source kind `environment`. Exact legacy schema-v1 and
schema-v2 objects map in memory to that acquisition kind while remaining
observable as versions `1` and `2`. The loader never rewrites or migrates them.
An invalid selected config-location environment value does not fall back.

On the supported Unix targets exercised by Milestone 03, a present path is
opened no-follow and nonblocking. A preliminary path-kind check is followed by
authoritative opened-descriptor regularity validation before any bytes are
read. Final symlinks and non-regular entries therefore fail closed without a
FIFO open becoming an unbounded wait. Hardened open semantics for non-Unix
targets remain deferred. The loader does not canonicalize, create, or write
anything; its no-follow guarantee is for the final component and is not a claim
that the complete ancestor path is frozen.

The raw configuration bound remains 64 KiB, with at most one additional byte
retained to detect overflow or growth. Accepted bytes must be valid UTF-8 and
then one strict schema. V3 has the exact five v2 fields plus required string
`credential_source: "environment"`. V2 has exactly five fields: integer
`schema_version: 2`; strings `permission_mode: "ask"`,
`provider: "vercel_ai_gateway"`, and `transport: "ai_gateway_http"`; and a
1–128-byte visible-ASCII model and rejects `credential_source` as unknown. V1
has exactly integer `schema_version: 1` and
string `permission_mode: "ask"`. Oversize files, invalid UTF-8, malformed JSON,
unknown or duplicate fields, missing or wrong fields, unsupported versions or
enum values, and invalid models are rejected.

The model validator is shared with the AI Gateway provider, preventing config
and provider acceptance from drifting. Config owns the bounded model string and
is cloneable but not copyable. Its debug output replaces the model with
`<redacted>`, and typed failures do not echo environment-derived paths,
configuration contents, model values, or operating-system error text.
Inaccessible paths and read errors are not converted into defaults.

Bearer credential bytes are fields in none of v1, v2, or v3 and never enter
config debug output. Provider, transport, and credential-source enums contain
only public declarative identity. `NativeCredentialSourceKind::Environment`
cannot carry a token or arbitrary variable name and grants no process authority
to the loader.
In particular, `NativeTransportKind::AiGatewayHttp` exists when the optional
concrete transport is disabled and on WebAssembly; it is not evidence that an
HTTP implementation or required runtime is available. Loading valid config
does not instantiate the provider or transport, create a Tokio runtime,
discover or attach a bearer token, or perform network I/O. The non-secret
acquisition kind may appear in config debug output; the model remains redacted.

The existing status path remains metadata-only and its CLI output is
byte-stable. Configuration mutation or migration, a concrete prompt UI and
modes beyond `ask`, token fields in configuration, session lifecycle, CLI
composition and expansion, native tools other than the bounded
library-level `read_file` and `list_files`, composed release-binary end-to-end
host evidence, and compatibility or performance claims remain open. The
twelfth slice composes the existing library components only after an already
validated config value is supplied; the thirteenth slice adds validation
that its configured acquisition kind is `Environment` without changing loader
or CLI authority.

The integrated `NativeReferenceHost` first rejects any loaded selection
other than `ask` / `vercel_ai_gateway` / `ai_gateway_http`. The thirteenth
slice also requires configured credential source `environment`. It then
opens the existing absolute workspace once and clones that retained descriptor so
exactly `list_files` and `read_file` share one opened directory identity. This
prevents path replacement between separate tool-construction opens from giving
the tools different roots. The same trusted-host ancestor and subordinate-mount
limits as the individual tool contracts still apply. A separately supplied
session root is retained through `FileSessionStore`; neither root is discovered
from status or configuration, selected by model input, or created. Composition
does not compare opened root identities or reject equality or ancestry. The
trusted host must select disjoint roots. If the session root equals or sits
beneath the workspace, workspace tools can reach session artifacts under their
normal bounded path rules after permission is granted.

Production construction opens both non-secret roots before it consumes the
injected credential snapshot, discovers a bearer token, and hands that token to
`AiGatewayHttpTransport`. Selection, workspace, and session-store failures
therefore occur without discovering or handing off a credential. The wrapper
retains only non-secret selected-source metadata and has no secret getter. It
retains the exact loaded configuration, including file origin and observable
schema version `1` or `2` for an accepted legacy file, while the in-memory
projection supplies its provider, transport, model, and acquisition kind.
`NativeReferenceHost::credential_source()` remains a different runtime
observation: it reports the concrete selected OIDC-token or API-key source.

The custom-transport constructor is an explicit trusted authority override. It
performs no native credential discovery or production HTTP construction and
reports `credential_source() == None`. That observation does not prove that the
custom transport is unauthenticated or secret-free. The custom transport owns
endpoint, network, authentication, status, retry, runtime, and diagnostic
policy and must return only an accepted byte stream or an already redacted
provider error under the existing injected-transport contract.

Both constructors are synchronous. Besides bounded construction and
the root opens, they make no network request, poll no prompt, load or save no
session record, create no root or runtime, and start no task, thread, timer,
retry, or other background work. `AskPermissionHandler` retains but does not
invoke the injected prompter. Later production HTTP polling still requires a
live host-owned Tokio runtime with I/O and time enabled and driven through
teardown.

Reference-host failures retain only a non-exhaustive fixed stage kind:
unsupported selection, workspace root, session store, credential, HTTP
transport, provider, or engine. Component errors, roots, config/model values,
credential bytes and source, endpoint data, prompt data, OS diagnostics, and
raw error numbers are discarded. Host debug output is fixed to
`NativeReferenceHost { .. }` and exposes no config structure or source. The
twelfth-slice behaviors are adversarially green and integrated on `main` under
exact green workflows. The schema-v3 and configured-source validation changes
are integrated through final record `f840576a`, with three fresh adversarial
tracks green on exact SHA `35ce591e` and exact final-record feature and `main`
workflows green.

The composed fourteenth candidate leaves the existing path constructors
unchanged and adds `NativeRootSelection` plus `PreparedNativeRoots`. Selection
is effect-free and consumes only an injected `NativeEnvironment` plus an
explicit absolute workspace path. Preparation opens the existing workspace
first, requires the selected `XDG_STATE_HOME` or fallback `HOME` base to exist,
and follows no
suffix symlink while it opens or creates only `machine-god` or
`.local/state/machine-god`. Newly created directories request `0700`. Existing
directories are never chmodded, chowned, replaced, or removed. The selected
base and existing intermediates must be owned by the effective UID with no
group/other write; the existing final root permits no group/other permission.
A just-created fixed name is normalized descriptor-relatively to `0700` before
the permission-requiring reopen, then identity-checked, `fchmod`ed, and verified
at exact `0700`; the same-effective-UID account is the remaining normalization
trust boundary. Opened workspace and
final state identities must be disjoint in both directions under device/inode
identity and descriptor-relative parent walking. Fixed errors and debug output
reflect no path, environment, identity, ownership, mode, or OS diagnostic.
Accepted state bases are rebuilt from lexical components before the no-follow
open, preventing a trailing slash or `/.` from moving a symlink out of the
final lookup position. A simultaneous create loser can fail closed and retry
while the winner normalizes an owner-bit-masked new directory; it never chmods
the `EEXIST` entry.
Prepared-root reference-host constructors consume the retained descriptors;
production credential discovery follows acceptance of those roots. Production
and focused tests cover this candidate behavior; formal adversarial and delivery
evidence remain pending.

The integrated file-session slice does not consume those status-derived state
paths.
The host explicitly supplies one existing absolute root. On supported Linux and
macOS Unix targets, `FileSessionStore::open` opens the final component
no-follow, verifies the resulting descriptor is a directory, and retains it.
The store performs no environment lookup, root discovery or creation, session
listing, deletion, reset, or arbitrary child-path access. Root ownership,
permissions, quotas, ancestor resolution, and filesystem behavior remain in the
trusted host boundary. Hardened non-Unix support is deferred.

For each validated `SessionId`, lowercase SHA-256 of the ASCII domain separator
`machine-god:file-session:v1:` plus the ID's UTF-8 bytes selects fixed flat
`.json`, `.lock`, and `.tmp` names. This keeps raw IDs out of ordinary filenames
and prevents ID bytes from becoming path syntax, but it does not hide a
guessable ID or record contents and does not provide authentication or
confinement. Descriptor-relative operations under the retained root provide the
path boundary. Load also verifies the exact decoded record ID, so a misplaced
record or theoretical digest collision fails as corrupt state rather than being
merged.

The strict compact schema-v1 envelope is bounded to
`MAX_FILE_SESSION_BYTES` (`8_651_165`), enough for every record under default
`EngineLimits`. Loads retain at most one extra byte to detect exact
overflow, then reject invalid UTF-8, malformed or unknown structure, unsupported
versions, zero revisions, wrong IDs, and over-limit data. The record and
permanent lock are opened no-follow and authoritatively required by `fstat` to
be regular files; special files cannot turn a read into FIFO, device, socket,
or directory access. Missing loads create no artifact. Corrupt and nonregular
artifacts are preserved rather than repaired, replaced, unlinked, or migrated.
The store iteratively enforces core's default aggregate JSON bounds of 64
container levels and 65,536 nodes for direct callers before serialization and
after decode; the file-size ceiling remains a separate resource bound.

Every save is serialized under the byte cap before publication. A permanent
regular no-follow lock sidecar provides exclusive advisory coordination
for cooperating store instances and processes. It is retained so cooperating
processes do not split across recreated lock inodes. A process that ignores the
lock, or an actor able to remove or replace the sidecar, remains outside that
coordination guarantee. Under the exclusive lock, new and update saves compare
the exact stored revision, reject incarnation changes, and assign the next
revision with checked arithmetic. They write through an exclusively created,
no-follow, authoritatively regular `0600` temporary file, synchronize that file,
atomically rename it over the record in the retained directory, and synchronize
the directory.

Before rename, failure preserves the old authoritative record. After rename,
directory-sync failure is necessarily ambiguous: the new record may be visible
but not proven durable, so a caller must load and reconcile rather than blindly
retrying a possibly completed operation. Atomic publication is one complete old
or new record on supported local filesystems honoring the assumed Unix advisory
lock, rename, and sync semantics. It is not a cross-record transaction, a
defense against noncooperating writers, an NFS guarantee, or a promise that
every sudden-power-loss/full-system failure mode preserves the last acknowledged
version. The implementation requests `fsync`; on macOS it does not request
`F_FULLFSYNC` and does not claim a successful save reached physical media.
Record contents are plaintext; encryption, authentication, secure
erasure, key management, migration, reset, and listing remain deferred.

Load and save futures are inert until first poll and spawn no task, thread,
timer, or runtime work. First poll performs bounded synchronous filesystem I/O,
advisory locking, file sync, and directory sync inline. Retained data and
successful transfer work are bounded. Advisory locking, byte transfers, and
`fsync` retry `EINTR`; other interrupted operations use the fixed error
taxonomy. Retry attempts and wall-clock duration are not bounded. Those calls
can block the polling executor thread, and dropping the future cannot interrupt
a synchronous call already in progress. Fixed errors never reflect session
IDs, hashes, roots, child paths,
record bytes, parser diagnostics, OS text, or raw error numbers. The complete
contract and fixed taxonomy are in
[`session-store.md`](session-store.md).

The ask-handler slice adds no ambient authority to core or native. The host
must inject either an owned `PermissionPrompter` or an explicitly shared
`Arc<dyn PermissionPrompter>`. The adapter never reads terminal input, writes
terminal output, inspects environment or configuration, accesses a file or
process, contacts a network, selects an executor, or persists a grant. Such
authority and its security controls belong behind the injected prompter.

For the engine path, core bounds the prepared capability and arguments and
constructs the complete auditable request before the adapter is called. The
adapter forwards that owned request exactly once and does not clone, mutate,
serialize, truncate, revalidate, or traverse it. This preserves the identity,
session incarnation, turn, capability, critical-risk hint, and fixed reason
that core already exposed through `PermissionRequested`; a presentation layer
must not silently authorize a shortened or reconstructed request.

Only structured allow-once, allow-turn, allow-session, and deny values cross
back into the adapter. The first three map to the matching core scopes but are
not cached by either the adapter or core. Deny maps to the fixed host-facing
reason `permission denied`. Prompt infrastructure failure carries no source
data in the zero-data `PermissionPromptError` and maps only to the fixed
`permission_prompt_failed` / `permission prompt failed` core error. It cannot
be treated as approval or ordinary denial.

Authorization is inert until polled and the adapter detaches no work. Dropping
an unpolled future never calls the prompter; dropping a pending future drops the
underlying prompt future. There is no separate cancellation token or revocation
message at this boundary. A prompter must therefore retain prompt work in its
returned future or perform its own drop cleanup; detaching an approval request
that can outlive that future violates the contract. Full polling, redaction,
and deferred-scope requirements are in
[`ask-permission.md`](ask-permission.md).

Tool preflight closes the representation gap between a model's raw JSON call
and the operation presented to permission policy. The source-compatible default
retains the existing raw `Capability::Tool` and arguments. A tool may instead
prepare a normalized `Capability` and replacement arguments, and an allowed
execution receives exactly those prepared arguments. Preparation is an
effect-free validation and normalization step implemented by trusted host code.
It must be deterministic, synchronous, bounded, and nonblocking, and it must not
open a path, start a process, contact a network destination, mutate state, or
exercise any other unapproved authority. Core checks cancellation immediately
before and after preparation returns but cannot interrupt it in flight.

Core applies its resource bounds before policy or execution. Prepared arguments
retain the exact configured tool-argument byte limit and their JSON depth and
node bounds. Capability depth and node traversal covers only JSON values
embedded in `Capability::Tool` or `Capability::Custom`; every capability variant
is separately serialized as a whole under one total byte cap of
`max_tool_argument_bytes + 1024`. The fixed 1 KiB is headroom within that total
cap, not a separately metered envelope or payload allowance. An invalid or
oversized prepared value fails before either boundary. A tool-reported
preparation error is reduced to a fixed generic durable tool error, does not
consult policy, and causes no tool effect. Policy still sees a fresh request
with the existing critical risk, fixed reason, and deterministic permission ID;
core still does not cache positive grant scopes.

The prepared arguments are not a second grant of authority. A trusted tool may
use them only for effects contained by the exact prepared capability that
policy allowed. Native filesystem, process, and network implementations must not
reinterpret them into a broader path, command, or destination. This slice
supplies that authorization seam. Its first native consumer is `read_file`. The
host injects one absolute workspace root, and Unix construction opens and
retains the final root directory without following a final symlink. Before that
open it rebuilds the host root from lexical path components, removing redundant
separators and `.` components throughout while preserving `..`, without
canonicalizing ancestors or resolving symlinks. The resulting removal of
terminal separators and terminal `.` components ensures that `/root-link/` and
`/root-link/.` cannot bypass final-component no-follow; the equivalent forms of
a real directory remain accepted. Provider input is exactly an object with one
UTF-8 `path` string bounded to 4,096 bytes. Effect-free preflight performs only
strict decoding and lexical normalization; it removes `.` components,
collapses repeated separators, and rejects `/`-rooted paths, `..`, forbidden
control or line/paragraph-separator or bidirectional-formatting characters, or
an empty normalized path before policy. On supported Unix, backslash and
Windows-looking prefixes are ordinary confined filename bytes, not path
separators or prefixes.
Policy receives
`FilesystemAccess::Read` for exactly the normalized workspace-relative path
that execution receives.

Allowed execution starts from the retained root descriptor and opens every path
component descriptor-relatively with no-follow semantics. It retains stable
ancestor descriptors during traversal, opens nonblocking, and authoritatively
checks that the final descriptor is a regular file before reading. A symlink in
any model-selected component, a directory, FIFO, socket, device, or other
non-regular final entry fails closed. Path replacement after a component is
opened cannot redirect later lookup through a different ancestor; replacement
of the final pathname after open cannot change which descriptor is read.
Replacement by another ordinary file before its open may change the bytes at
the same authorized path, so this is path authorization, not an immutable file
identity or snapshot guarantee.

The reader retains at most 8 KiB plus one byte to detect overflow or growth and
never truncates a successful result. It accepts only valid UTF-8 and returns
exactly `{content}`; successful content is intentionally model-visible and
therefore also present in the durable tool result and observer event. Failures
use the fixed constructor and tool-error tables in
[`read-file.md`](read-file.md). Operating-system error numbers may select among
those fixed categories, but no OS text, workspace path, requested path, or file
bytes enter their display, debug, code, or message. Only non-cancellation
preparation and execution errors become core's generic durable tool-error
result. A direct `Tool::execute` observes cancellation as the fixed
`read_file_cancelled` error; when the engine owns the shared token, engine
cancellation wins, terminates the turn, and leaves the unknown placeholder
rather than writing a generic tool error. Cancellation is checked before
traversal, between bounded reads, and after content validation. It closes
retained per-call descriptors and buffers but cannot interrupt an individual
open, metadata, or read syscall already in flight.

`list_files` uses the same authorization and confinement boundary for
`FilesystemAccess::Enumerate`. Strict effect-free preflight accepts only `{}` or
a sole string `path`, defaults omission to `.`, applies the 4,096-byte bound and
the same absolute, parent, control, and bidirectional-character rejection, and
gives policy and execution the exact same normalized path. Backslash and space
are literal Unix filename characters. No preflight filesystem effect occurs.

Allowed enumeration opens every selected component relative to the retained
root with directory and no-follow requirements. It never follows a selected or
entry symlink and never opens a child, resolves an entry target, reads content,
recurses, applies ignore rules, accesses an external path, or discovers a
workspace. It skips only `.` and `..`. Entry kinds come directly from the
directory entry type; an unknown type is `other`. Every observed visible name,
including a truncation witness, must be safe valid UTF-8 or the call fails with
a fixed redacted error.

Enumeration retains no more than 100 entries and 16 KiB of aggregate raw name
bytes. It reads the first extra visible entry needed to prove truncation and
then stops, sorting only what it retained. This limits retained memory and
syscall work after a bound is crossed, but a truncated selection can reflect
filesystem iteration order. It is not a whole-directory ordering or snapshot
claim. At the independent maximums, JSON escaping and fixed entry syntax remain
within 44,130 serialized bytes including core's fixed `ToolOutput` envelope,
below the default 64 KiB result limit.

Failures use the fixed constructor and tool-error tables in
[`list-files.md`](list-files.md). No root, requested path, entry name,
operating-system text, or error number enters the fixed public diagnostic.
Cancellation is checked before traversal, between component opens, before
enumeration, between entry reads, and after result validation and sorting. It
cannot preempt one open or directory-read syscall already in flight and spawns
no detached work.

These tools provide descriptor-rooted confinement of model-selected path
components, not a claim that an untrusted host is sandboxed. The host's
resolution of ancestor components leading to an injected root path and mount
points visible beneath a retained directory are trusted inputs. Hardened
non-Unix workspace construction and traversal remain deferred. The normative
surfaces are [`read-file.md`](read-file.md) and
[`list-files.md`](list-files.md).

The injected-transport AI Gateway provider preserves network authority at an
explicit trusted-host boundary. `AiGatewayProvider` accepts only an owned body,
fixed protocol/model/session metadata and a transport object. It cannot select
a URL, resolve DNS, open a socket, configure a proxy, negotiate TLS, read a
credential, interpret an HTTP status, follow a redirect or schedule a retry.
The host's `AiGatewayTransport` owns each of those decisions and must return
only the byte stream of an accepted response or a redacted `ProviderError`.

Consequently, SSRF controls belong where the endpoint is selected and
resolved, not in this codec. A production transport must constrain schemes,
origins, redirects, proxy routing and resolved addresses according to its host
policy, including DNS rebinding and link-local/private address considerations.
Secret controls also remain there: credentials must be attached outside the
request body and fixed codec headers, must not enter debug or error text, and
must be scoped to the selected origin. Authentication, rate-limit, availability
and other status classification must occur before returning a successful byte
stream. The host likewise decides whether an operation is safe to retry; the
codec calls the transport at most once, and exactly once only after a valid
request future is polled through startup. It never retries a possibly delivered
model request.

The optional native `ai-gateway-http` transport is one intentionally narrower
host policy. Production requests can target only
`https://ai-gateway.vercel.sh/v3/ai/language-model`. The alternate
`AiGatewayHttpEndpoint::loopback_http` constructor is explicitly for
deterministic tests and accepts plaintext HTTP only for a numeric IPv4 address
in `127.0.0.0/8` or IPv6 `::1`, with an explicit port and absolute path. It
requires that port to be nonzero and rejects user information, queries,
fragments, hostnames and alternate or encoded IP forms; it does not turn
arbitrary private networks or non-loopback addresses into a production
configuration surface. Endpoint text is bounded to
2,048 ASCII bytes. Redirects and proxies are disabled, so
the bearer credential cannot be redirected or proxy-routed by this client.

The trusted host must pass the bearer value directly through
`AiGatewayBearerToken::new`. The transport itself performs no environment,
file, keychain, interactive, CLI or broader configuration lookup. The value
must be a
1–4,096-byte RFC 6750 bearer `b64token`: one or more ASCII letters, digits,
`-`, `.`, `_`, `~`, `+`, or `/`, followed only by optional trailing `=`
padding. It is attached only as the `Authorization: Bearer` header. Its display
surface does not reveal the value, its debug representation and all transport
diagnostics are redacted, and no dependency text is forwarded. This is a
non-reflection guarantee, not a secure-memory-erasure claim; the injecting host
remains responsible for credential acquisition, lifetime, rotation and origin
scope.

The tenth slice supplies one separate optional acquisition path. An owned
snapshot contains only `VERCEL_OIDC_TOKEN` and `AI_GATEWAY_API_KEY`; a nonempty
OIDC token has precedence, an exactly empty value is absent, and any selected
nonempty invalid value fails closed without fallback. Non-Unicode selection is
distinct from malformed or oversized Unicode bearer input. Results move the
existing bearer type without cloning, and errors retain no value, source, OS
diagnostic, or validator text. Debug and display surfaces are non-reflecting.
At most two validated 4 KiB tokens are retained before selection, but process
lookup may materialize a larger OS value before rejection. Drop clearing is
best-effort and does not cover environment storage, allocator history, HTTP
header copies, or other dependency internals. The adapter does not persist,
rotate, log, print, transmit, or configure a credential by itself.

The Reqwest client disables redirects, proxy use, automatic response
decompression, cookies, retry and referer generation and fixes HTTP/1. A
semaphore bounds active requests, defaulting to 16 with a validated range of
1–64; its permit remains owned by the response stream. Same-endpoint idle
connections may be reused. A connection attempt defaults to 30 seconds and is
bounded above by 5 minutes. The complete request/response stream defaults to 10 minutes and
is bounded above by 1 hour; both durations must be positive and the connect
timeout cannot exceed the total. The total deadline begins before semaphore
acquisition and covers capacity waiting. Dependency frames are split into
public chunks of at most 64 KiB by default; a host may choose 1 byte through 1
MiB. This public chunk cap does not bound Hyper's internal frame/read
allocation, and the codec still applies its
independent chunk, record, undecoded-buffer, total-byte and record-count limits.
The fixed POST request carries the codec headers and JSON body plus the injected
authorization header, fixed `Accept: text/event-stream`, and fixed
`Accept-Encoding: identity`. No response header, error body or dependency
diagnostic is reflected through the provider error.

Only HTTP 200 is accepted as a stream. Status handling is closed and exact:
401/403 are non-retryable authentication errors; 429 is retryable rate
limiting; 408, 425 and every 5xx status are retryable unavailability; all other
4xx statuses are non-retryable invalid requests; and 3xx or any remaining
non-200 status is a non-retryable protocol error. A generic network failure is
conservatively retryable transport failure, while a recognized TLS failure is
non-retryable transport failure. The mapping uses only the status or fixed
failure class and never consumes an error body for diagnosis. Application,
status, and backoff retries are disabled. Hyper may recover a stale reused
connection only before writing request bytes; at most one peer-visible request
is dispatched, and a possibly delivered request is never replayed.

TLS uses Reqwest's Rustls backend with the pinned `webpki-root-certs` dataset;
default Reqwest features and native-root loading are disabled. The repository
explicitly permits that certificate dataset's `CDLA-Permissive-2.0` license.
This makes trust deterministic
and avoids silently inheriting a machine-wide trust store, at the deliberate
cost that enterprise interception roots and private certificate authorities in
the operating-system store are not trusted. Updating trust follows the pinned
Rust dependency rather than an immediate OS trust-store update. The feature is
unavailable and its exports are cfg-gated on WebAssembly targets.

The same core cancellation token reaches the Reqwest work. Cancellation before
dispatch prevents a request; cancellation while active-request capacity, the
upload, response head, or body is pending wakes the wrapper and drops the owned
in-flight operation and semaphore permit.
Dropping a transport future or returned byte stream performs the same ownership
teardown. Machine-god owns no internal runtime and detaches no producer task,
retry, or timer. Reqwest/Hyper owns connection-dispatch tasks on the host
runtime. The concrete transport must be polled inside a live host-owned Tokio
runtime with I/O and time enabled, and that runtime must remain driven through
asynchronous socket teardown. No active runtime handle produces a fixed
redacted failure. A handle without I/O or time violates the API precondition;
Tokio may panic and the abort-on-panic release profile may terminate the host.
Cancellation
and drop bound owned work but do not promise that the operating system, remote
peer or dependency can prove whether already-transmitted bytes were delivered.
The normative transport contract is
[`ai-gateway-http.md`](ai-gateway-http.md).

Machine-god supplies no fx referer, title or user agent and therefore does not
misrepresent the caller's identity. The codec supplies no authorization, team
or endpoint metadata. The optional HTTP transport adds exactly the explicitly
injected authorization value plus its fixed accept headers; a custom host
transport may add its own values only under that host's identity and disclosure
policy. The model, session ID and complete encoded prompt are necessarily
disclosed to the selected transport. Tool descriptions,
schemas, arguments and serialized results may be part of that prompt and must
be treated as sensitive model input.

Untrusted response bytes are accepted only through independent request, chunk,
record, undecoded-buffer, total-response, record-count, message, tool,
streamed-tool-input, final-tool-call and per-call argument limits, plus an
aggregate request JSON-node limit and a fresh node limit for each decoded
response or serialized-argument JSON tree. Strict UTF-8, JSON and
supported-event schema validation rejects malformed, conflicting, duplicate,
provider-executed, incomplete and post-finish state. A final identity can
replace a differing provisional identity only through one unambiguous
same-name, structurally equal explicit-input match. Canonical lookup normalizes
signed floating zero and invalid provisional JSON is retained only as a fixed
marker. An authoritative exact-ID final may replace that marker or unfinished
input, after which bounded delta/end records for its tombstone are ignored.
Bounded blank, comment, non-data and unknown-event records are no-ops.
Partial tool inputs are never emitted. `[DONE]` or EOF without one valid finish
is a failure, so truncation cannot be reported as a normal stop. Provider error
events and a unified `error` finish are redacted protocol failures rather than
model-visible text.

An empty successful response chunk is rejected. A nonempty chunk that produces
no event consumes at most one source item in a poll before the codec schedules
another poll and yields. A malicious always-ready source therefore cannot hold
an executor thread by returning an unlimited sequence of comments, ignored
fields, unknown events, or partial records in one poll.

Construction, request, decoder, bound and cancellation errors use fixed
categories without reflecting prompts, tool values, response records, model
bytes, endpoint data, credentials or transport-controlled diagnostics. Provider
and request debug output is similarly structural. A supplied transport owns the
sanitization of its own error kind, code, message and retryability before the
codec receives it.

Cancellation is checked around request construction and transport startup and
between response chunks, records and yielded events. The provider registers a
cancellation wakeup rather than waiting for a new hostile or stalled response
chunk. The transport receives the same cancellation token and is responsible
for waking and tearing down its pending I/O. When cancellation and a terminal
provider result become ready in the same poll, cancellation wins. Dropping
provider work drops its owned future, byte stream, buffers and partial tool
state. Ready decoder outcomes first deregister their cancellation waiter, so an
inactive poller's waker is neither retained nor spuriously woken later. All
request-side JSON, including ignored metadata, is depth/node checked
before the owned request leaves its guard. An early-rejected, cancelled or
dropped guarded request drains those values with an iterative child stack,
preventing deep hostile input from triggering recursive JSON teardown. Accepted
values are within the safe depth ceiling. No codec task, timer, thread or retry
is detached. A blocking or cancellation-insensitive injected transport violates
its host-side contract and cannot be repaired by the codec. The exact boundary
is documented in [`ai-gateway.md`](ai-gateway.md).

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
`ToolContext` and `EngineEvent` carry the same incarnation so tool idempotency
keys and event-sink deduplication or audit keys cannot collide across resets
whose session, turn, call, and event sequence identifiers repeat.

Event sinks are untrusted diagnostic producers. A sink failure crosses the
public API only as `event_sink_failed` / `event sink failed`; the original code
and message are dropped, including when cancellation races a ready sink error.
Engine debug formatting likewise never calls `ModelProvider::name` or includes
provider-controlled text; it exposes only fixed structural fields.

Benchmark CI obtains Zig only from the official Zig 0.16.0 HTTPS archive and
verifies SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extracting into a fresh fixed directory under `RUNNER_TEMP`. The workflow
then fails unless the installed executable reports version `0.16.0`. This keeps
the upstream-reference compiler outside the Rust product's dependency and
authority surfaces while binding its CI bytes without a third-party setup
action.
