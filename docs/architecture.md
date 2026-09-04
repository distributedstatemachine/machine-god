# Architecture

Machine-god is an embeddable, provider-neutral coding-agent engine with an
explicit native host. The engine is the primary product; native effects and the
CLI are replaceable adapters around it. Current delivery scope lives only in
the [implementation plan](implementation-plan.md#current-delivery-state).

## Crate ownership

| Crate | Owns | Must not own |
| --- | --- | --- |
| `machine-god-core` | Model, session, permission, tool, event, cancellation, identifier, and orchestration contracts | Ambient filesystem, process, environment, credential, clock, randomness, network, or executor authority |
| `machine-god-darwin-proc` | Safe fixed-buffer access to the narrowly audited Darwin process identity and lineage ABI | Product policy, allocation from kernel counts, process mutation, or non-macOS effects |
| `machine-god-native` | Operating-system, persistence, network, credential, root, prompt, and native-tool effects | Provider-neutral product state or CLI presentation policy |
| `machine-god-cli` | Argument parsing, process environment capture for a command, runtime driving where required, rendering, exit codes, and stdout/stderr writes | Engine state, provider protocol, permission policy, persistence schema, or tool behavior |
| `machine-god-testkit` | Deterministic providers, stores, permission handlers, tools, event sinks, and fixtures | Production effects |

The production dependency direction is `machine-god-cli` to
`machine-god-native` to `machine-god-core`; on macOS only, native also depends
on `machine-god-darwin-proc`. The testkit depends on core. Core does not depend
on the other workspace crates.

Unsafe Rust remains forbidden in core, native, CLI, and testkit. The sole
production exception is the private Darwin FFI module narrowed and audited by
[ADR 0003](decisions/0003-bounded-darwin-process-query-ffi.md); its crate
denies unsafe code everywhere else.

## Core composition

`EngineBuilder` requires an explicit `ModelProvider`, `SessionStore`, and
`PermissionHandler`. There are no permissive hidden defaults. An `EventSink` is
observational and defaults to `NoopEventSink`; tools are registered explicitly
under validated names. Public extension traits are object-safe, `Send`, and
`Sync`, and use boxed standard futures plus `futures-core::Stream`, leaving the
core executor-independent.

The engine holds provider-neutral values only. Native paths, file descriptors,
tokens, sockets, processes, runtime handles, and clocks never enter core. A
capability contains the normalized identity policy needs to decide an effect,
not the authority needed to perform it.

## Turn and tool data flow

A normal turn follows this direction:

1. The host supplies a validated session, prompt, inference options, and a
   cancellation token.
2. Core validates configured byte, count, depth, and node limits before calling
   the provider.
3. The provider streams text, reasoning, usage, stop, and local tool-call
   events. Core validates ordering and aggregate limits.
4. Core resolves each tool by validated name and calls its synchronous,
   effect-free `prepare` method.
5. Preparation returns canonical execution arguments and either an exact
   policy capability or the narrow explicit no-authority disposition.
6. Authority-bearing calls are presented to the injected permission handler.
   Only an allow decision reaches execution.
7. The tool receives exactly the prepared arguments, executes through its
   injected/native authority, and returns a bounded result.
8. Core records the durable tool result and, within round limits, supplies the
   updated transcript to the provider.
9. The session store persists revisioned state through its explicit contract;
   observers receive bounded events but cannot alter execution.

Preparation is trusted host code, but it is required to be bounded,
nonblocking, and effect-free. `PreparedToolCall::without_authority` is reserved
for tools whose prepared execution genuinely needs no policy-governed
authority; it does not skip argument validation, cancellation, tool lifecycle
events, result validation, persistence, or the next model round.

## State and identity

Core validates session, incarnation, turn, tool-call, and permission-request
identifiers. Native persistence maps those values into its own versioned,
bounded storage format. The native reference host shares the same concrete
`Arc<FileSessionStore>` with the engine and `NativeSessionLifecycle`, so create,
resume, replay, reset, listing, and inspection do not silently address a second
store allocation.

Session incarnation and revision checks fence stale cooperating handles, but do
not claim global snapshots or revocation of already running external effects.
The [session store](session-store.md) and native lifecycle documents own the
exact durability, locking, and race contracts.

## Native effect boundaries

`machine-god-native` converts explicit host inputs into concrete capabilities:

- retained workspace and state descriptors for confined filesystem work;
- fixed process configuration and retained cwd identity for terminal work;
- a host-owned background supervisor that composes provider-neutral admission
  with durable records and exact process-group ownership;
- bounded credentials and fixed endpoints for AI Gateway access;
- injected permission and question prompters for human interaction;
- host-owned runtime, clock, deadline, and transport implementations where
  asynchronous effects require them.

Native tools repeat strict input validation on direct execution. Their
preflight capability and execution arguments must agree exactly, so a direct
caller cannot widen authority by bypassing the model-facing path. Each tool
contract documents its platform support, resource ceilings, irreversible
boundary, cancellation order, redacted failures, and unavoidable host races.

The [native reference host](native-reference-host.md) is the maintained example
of full composition. It validates the selected configuration, retains roots,
constructs the provider and transports, installs prompt adapters and the shared
store, and registers the bounded tool catalog. Construction performs no model
request, prompt poll, or session-record operation; individual constructors may
perform their documented synchronous root, credential, environment, entropy,
or resolver-configuration acquisition.

## CLI boundary

The CLI parses the complete command before command-specific effects, constructs
only the native facade needed by that command, and fully renders bounded output
before the first success write. It owns exit codes and fixed diagnostics, but
does not own model, session, permission, or tool semantics.

The current CLI contains read-only configuration, diagnostic, model-catalog,
and session-inspection surfaces. Interactive agent UI and the remaining command
inventory are tracked by the implementation plan rather than inferred from the
library composition.

## Invariants

1. Core gains no ambient authority.
2. Authority is explicit, normalized, and decided before a native effect.
3. Policy sees the same canonical identity execution revalidates and uses.
4. Cancellation never implies rollback across an irreversible external effect.
5. All retained model-controlled data and traversal work have explicit bounds.
6. Errors and debug output expose stable categories, not secrets or arbitrary
   external diagnostics.
7. Provider-specific wire behavior remains native; core sees provider-neutral
   events and values.
8. The CLI remains a thin host and may not become a second owner of product
   state.
9. Tests use deterministic injected seams; production effects remain behind
   their native contracts.
10. Compatibility and performance are claimed only from retained,
    scenario-equivalent evidence against the pinned upstream revision.

## Platform and feature shape

Core and the testkit are portable. Native exports are feature- and target-gated
according to the authority they require. The complete AI Gateway reference host
is compiled for Linux and macOS, non-WebAssembly targets, with
`ai-gateway-http`. Some portable native contracts expose an injected seam on
more targets while their production system implementation is narrower; for
example, the terminal system executor is Linux-only. The process-local
background supervisor has production adapters for both Linux and macOS; macOS
uses the fixed safe-Rust inherited-descriptor helper described by its contract.
The contract for each component is authoritative.

Skills, MCP, ACP, subagents, SDK surfaces, advanced compatibility, optimization,
and packaging belong to later milestones. They must extend these boundaries
without moving ambient authority into core or product state into the CLI.
