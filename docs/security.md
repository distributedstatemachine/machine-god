# Security boundaries

Machine-god is an ordinary coding-agent product. This document defines the
trust, authority, confinement, redaction, and lifecycle boundaries needed to
run model-selected work predictably; it is not a claim that the agent is a
sandbox or a security-analysis tool. Current delivery state is maintained only
in the [implementation plan](implementation-plan.md#current-delivery-state).

## Trust model

The model transcript, model events, tool arguments, provider responses,
workspace contents, network content, persisted records, environment values,
and operating-system diagnostics are untrusted data. They may be malformed,
large, racy, or crafted to cross a boundary.

The embedding host chooses and therefore trusts concrete providers, stores,
permission handlers, prompt adapters, roots, transports, runtimes, clocks, and
system executors. Machine-god validates their data at its public boundaries,
but cannot make a hostile injected implementation safe or preempt arbitrary
synchronous code inside it.

An operator approval grants the exact capability shown to policy. It does not
turn the resulting effect into a sandboxed operation, guarantee rollback, or
remove authority already available to the operating-system account.

## Authority model

`machine-god-core` has no ambient filesystem, process, environment, credential,
clock, randomness, or network authority. `EngineBuilder` receives every
authority-bearing component explicitly.

Before normal tool execution, the tool performs bounded, synchronous,
effect-free preparation. Preparation normalizes model input into:

- exact canonical arguments that allowed execution receives; and
- a capability identifying the filesystem path and access kind, process and
  environment identity, network target, or indivisible ordered path set plus
  destination that policy must decide.

The durable `memory` tool uses an exact custom capability containing its
canonical action and fact, if present. All memory actions require policy
approval because they observe or mutate state shared across sessions. Its
state-root descriptor is explicit native authority and is never discovered by
core or inferred from model input.

The workspace-local `skill` tool requests one exact filesystem-read capability
for `skills/<name>/<resource>`. Its byte offset narrows only the returned range
and does not broaden the authorized object. Loaded text remains untrusted opaque
workspace content; it cannot grant installation, process, network, persistence,
MCP, subagent, or additional filesystem authority.

The local `install_skill` tool requests one exact custom capability containing
the canonical source and managed destination. Approval is required before it
observes either path. It admits only a bounded no-follow tree with a regular
UTF-8 `SKILL.md`, stages private copies, and atomically publishes one absent
destination. Installed bytes remain untrusted and grant no execution or
additional authority.

The injected `mcp_features` tool uses no ambient MCP, process, network, or
filesystem authority. Its sole host-interaction interface is an explicitly
injected read-only `McpFeatureAuthority`. That trusted boundary must admit the
exact server-qualified action and identity before any underlying effect and
revalidate the same live authority generation before returning. Resource and
prompt content is marked untrusted with `authority: "none"`; it cannot grant a
permission, authorize a later tool, or override user instructions. Production
MCP transports and authentication remain deferred.

The provider-neutral `subagent` tool similarly uses no ambient child provider,
executor, filesystem, process, network, permission, clock, task, or persistence
authority. Its sole child-computation seam is an explicitly injected
`SubagentAuthority`. The authority receives only a bounded name and prompt,
the parent call's bounded structural session/incarnation/turn/call identifiers,
and a cancellation token. Those identifiers support admission and attribution;
they are not session, transcript, store, engine, or permission handles. The
authority does not receive the parent transcript, grants,
prepared capabilities, dynamic tools, tool catalog, recursive `subagent`
visibility, or model/effort/permission/notification overrides. Completed text
is stamped `trust: "untrusted_child"` and `authority: "none"`; it cannot grant
permission or authority to a later call.

Authority-bearing capabilities pass through the injected permission handler.
An error is never approval. The native ask adapter maps prompt failure to a
fixed denial-class error and does not cache grants.

Core validates every complete capability within one serialized-byte envelope
before policy. JSON depth and node traversal applies only to an embedded JSON
value; typed path, identity, target, and composite path-plus-target fields have
no separate payload budget. For `vision`, one approval covers both the exact
workspace paths disclosed and their exact provider destination; neither half
is independently sufficient. See the [`vision` contract](vision.md).

The explicit no-authority preparation disposition is a narrow trust assertion,
not an optimization inferred from model input. It is used only when the
prepared execution needs no policy-governed authority. The question tool uses
it because the injected `QuestionPrompter` owns the host-interaction authority;
the tool cannot recursively request permission or escalate through
`permission_request_id`. The current `vision` attachment-ID path also uses it:
attachment storage is not implemented, so the path deterministically returns
unavailable records without reading the filesystem or contacting a provider.

## Filesystem confinement

Supported native workspace tools begin from an absolute root selected by the
host and retained as a directory descriptor. They use descriptor-relative,
no-follow traversal and revalidate types and identities at the boundaries
specified by each contract. Model-supplied paths are strict, bounded,
workspace-relative forms; preparation and direct execution apply the same
normalization.

The `skill` reader additionally fixes the leading `skills` directory and one
validated skill-name component, admits the complete selected regular file
within a fixed byte limit, validates all bytes as UTF-8, and returns bounded
pages without interpreting Markdown, YAML, frontmatter, references, or code.

The `install_skill` writer rejects managed-source overlap and all links or
special entries. It retains source descriptors, copies into a private staged
tree, and uses one no-replace rename as its commit boundary. It does not fetch,
unpack, parse, or execute skill content.

This prevents ordinary lexical escape and symlink traversal, but it is not a
general filesystem sandbox. Portable pathname operations are not atomic
compare-and-swap primitives. A cooperating or same-account process may rename
or replace entries between checks; a retained directory moved elsewhere can
still receive descriptor-relative work. Mutation contracts identify their
irreversible syscall, post-commit ambiguity, durability steps, cleanup limits,
and remaining race windows.

Root selection validates its existing directory chain's ownership, private
mode, no-follow acquisition, and supported macOS ACL conditions. The session
store's path seam retains an existing no-follow directory; before that same
descriptor is granted to the background writer, reference-host composition
read-only applies the background store's owner-private mode and supported macOS
ACL validation. These components create only documented fixed suffixes or
per-record lock artifacts. Missing-root read-only CLI paths do not create a
root.

## Process boundary

The terminal tool authorizes a fixed shell, canonical starting directory,
command, and bounded environment identity before execution. Foreground `exec`
uses the construction snapshot. Noninteractive `start` authorizes the validated
workspace-relative cwd bound to the retained workspace identity and uses the
background supervisor's fixed environment digest. Only after approval does
execution privately derive the absolute canonical cwd used for persistence and
process launch. Reference-host construction retains those identities but
creates no background namespace or worker cohort. The first permitted, polled
`start` performs one shared initialization; its fixed, redacted success or
failure is reused, and cancelled waiters cannot trigger a second reconciliation
or cohort. The retained descriptor constrains the starting directory only.
After approval, `/bin/sh -c` can exercise every authority available to that
process account; terminal execution is explicitly not a sandbox.

The Linux system executor owns bounded pipes, a process group, cleanup signals,
and child reaping. Cancellation, timeout, output overflow, and drop attempt the
documented bounded cleanup sequence. Safe Rust cannot impose a universal
wall-clock ceiling on blocked kernel calls, filesystem operations, process
spawn/wait, executor poll/drop, or arbitrary Waker callbacks. Descendants that
escape the process group or credential domain are outside the containment
claim. The full limits and ambiguity rules are in [terminal.md](terminal.md).

## Network and credential boundary

Network tools prepare a canonical scheme, host, and effective port before the
permission decision. Redirects that would change authority are not followed
under the original approval. Native transports use fixed production endpoints,
explicit bounded credentials, disabled proxy/cookie/referer behavior, bounded
status/body parsing, and fixed error categories according to their contracts.

Credential discovery reads only the named supported sources. A selected
nonempty higher-priority value that is invalid fails closed; it does not fall
back to a lower-priority credential or anonymous access unless a contract
defines an explicit authentication-result fallback. Token bytes are moved into
the transport and are not exposed through host getters, results, errors, or
debug output. Custom transports are trusted authority overrides and must be
paired with their exact network target.

Fetched pages, search results, titles, URLs, snippets, and provider-returned
text remain untrusted reference material. Receiving them does not grant another
tool capability or permission decision.

## Persistence boundary

The native store uses a versioned bounded envelope, validated identifiers,
revision/incarnation checks, permanent per-session advisory lock sidecars,
no-follow regular-file reads, private temporary files, same-directory atomic
publication, and directory synchronization. These coordinate cooperating
processes and fence stale saves; they do not provide a global multi-record
snapshot, encryption, record authentication, hostile-process exclusion, or
secure erasure.

The native `memory` tool separately owns a fixed versioned document and
permanent advisory lock under an identity-preserving clone of the retained
state root. Its nonblocking lock fails busy instead of waiting on a cooperating
operation; save and clear use the atomic-publication and post-commit ambiguity
boundary in the [memory contract](memory.md). Memory state and session records
are not one transaction.

Store futures are inert until polled and currently perform synchronous native
work on the polling thread. Lock waits, filesystem latency, and documented
interrupted-call retries may be unbounded in wall-clock time. Session listing
and inspection return bounded projections and never expose transcript content
through the CLI.

## Validation and resource safety

Core and native boundaries limit retained bytes, item counts, JSON depth and
nodes, rounds, events, tool calls, concurrency, and serialized outputs. JSON
validation occurs before recursive serialization or cloning. Tool contracts
also bound path components, directory visits, matching work, content reads,
DNS/SSE records, response bodies, process output, prompt callbacks, and other
effect-specific work.

Bounds limit application-controlled work and retention. They do not promise
that the operating system, allocator, DNS resolver, TLS stack, injected
implementation, or kernel performs no additional bounded or blocking work.
The [performance overview](performance.md) distinguishes structural bounds from
measured claims.

Foreground subagent execution adds independent fail-fast limits of four active
children globally and two for one parent turn. Exhaustion performs no authority
call and creates no waiter or queue. The tool creates no task, thread, timer,
watcher, persistent child, or detached cleanup tail; an injected implementation
that does so is outside the core contract and remains trusted host code.

## Cancellation, drop, and panic

Futures own their in-flight work unless a contract explicitly documents a
bounded native worker tail. Dropping an unpolled future performs no effect.
Cancellation is checked before authority-bearing phases and at contract-defined
intervals during bounded work. Once an irreversible effect succeeds or becomes
ambiguous, cancellation cannot truthfully report rollback; the tool completes
its durability/cleanup boundary and returns success or a fixed commit-ambiguity
error.

For `subagent`, cancellation also races the injected authority future and wins
over a ready success or failure observed in the same poll. The losing future is
dropped before active-child capacity is released, and no partial child text is
published.

Waker callbacks are arbitrary foreign code. Native adapters do not invoke them
while holding internal locks, serialize callback delivery where required, and
retain capacity until the associated callback or worker activity settles.
Question prompting has a lifetime-wide finite callback budget and closes stale
Waker delivery before releasing capacity. Closed retained Wakers are inert.

Release builds use `panic = "unwind"` so audited cleanup and panic-precedence
paths can settle state. Suppressed opaque panic payloads whose destruction could
replace a selected primary panic are intentionally forgotten in the narrowly
documented paths. This is a bounded leak chosen to preserve process and state
integrity; it must not be generalized without review.

## Redaction

Public errors use closed kinds, codes, and fixed messages. Debug output is
structural. It must not reflect secrets, paths, environment values, commands,
provider payloads, HTTP bodies, record content, or dependency/operating-system
diagnostics unless a specific user-facing contract explicitly includes a
bounded value.

Terminal-safe encoding is applied before host presentation of model-controlled
question text. CLI success output is assembled within a fixed cap before the
first write, preventing partial success records. Output-write failures use fixed
diagnostics.

## Deferred hardening

The following remain explicit future work rather than implied guarantees:

- permission modes and durable grant policy beyond ask-only decisions;
- encrypted/authenticated persistence, key management, and secure erasure;
- hardened non-Unix workspace and store construction;
- a true process sandbox or stronger descendant containment;
- private/authenticated web destinations and redirect authorization; and
- remote or packaged skill discovery/installation, production MCP transport
  and authentication, extension/ACP authority, persistent/background subagent
  management, and SDK authority models.

Each subsystem contract is normative for its exact platform, effect, limits,
and race semantics.
