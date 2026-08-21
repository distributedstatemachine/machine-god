# Deterministic testkit

`machine-god-testkit` provides provider-neutral doubles for every milestone-02
core boundary. The doubles are cloneable and thread-safe, require no async
runtime, use no sleeps or global state, and expose consistent snapshots for
assertions.

Scripts are consumed in call order. A strict script that runs out returns a
component-appropriate error with code `testkit_script_exhausted`. Recorded-call
logs default to 1,024 entries and fail with
`testkit_record_capacity_exhausted` instead of silently discarding evidence.
Constructors ending in `with_record_capacity` let tests choose a smaller or
larger explicit bound. The immutable scripts themselves are bounded by the
finite collection supplied at construction.

## Engine example

```
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    Engine, ModelEvent, PermissionDecision, PermissionGrantScope, SessionId,
    SessionIncarnationId, StopReason, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep,
    RecordingEventSink, ScriptedModelProvider, ScriptedPermissionHandler,
};

let provider = ScriptedModelProvider::new(
    "fixture",
    [ModelProviderStep::events([
        ModelEvent::TextDelta { text: "hello".into() },
        ModelEvent::Stop { reason: StopReason::Completed },
    ])],
);
let store = InMemorySessionStore::new();
let sink = RecordingEventSink::new();
let permissions = ScriptedPermissionHandler::new([
    PermissionStep::Decision(PermissionDecision::Allow {
        scope: PermissionGrantScope::Once,
    }),
]);
let engine = Engine::builder()
    .provider(provider.clone())
    .session_store(store.clone())
    .permission_handler(permissions)
    .event_sink(sink.clone())
    .build()?;

let session = engine.create_session(
    SessionId::new("docs")?,
    SessionIncarnationId::new("docs-logical-lifetime-1")?,
)?;
let turn = block_on(session.prompt("say hello"))?;
let events = block_on(turn.collect::<Vec<_>>());
assert!(events.iter().all(Result::is_ok));
assert_eq!(provider.requests()[0].request.messages.len(), 1);
assert_eq!(store.record(&session.id()).unwrap().next_turn_sequence, 2);
assert!(matches!(
    sink.events().last().unwrap().payload,
    TurnEvent::Completed { reason: StopReason::Completed, .. }
));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Doubles

- `ScriptedModelProvider` records complete requests and cancellation handles.
  Steps can fail during startup, return finite result streams, remain pending
  after a prefix, or keep startup pending until cancellation.
- `InMemorySessionStore` performs load/save and revision comparison under one
  mutex. Successful saves return and store a revision greater than both the
  current stored revision and the submitted record revision. It rejects an
  attempt to replace an existing session ID with another incarnation.
  Independent load and save scripts can pass through, fail, or remain pending.
- `RecordingEventSink` accepts all events by default. Its strict mode can accept,
  fail, or permanently backpressure each event in order. Recorded events retain
  their session incarnation, including terminal cancellation delivery.
- `ScriptedPermissionHandler` records complete requests and returns ordered
  decisions, errors, or pending futures.
- `ScriptedTool` preserves its advertised specification and the default
  source-compatible preflight, so policy sees raw `Capability::Tool` and
  execution receives the original arguments. It records the context, exact
  post-preflight JSON arguments, and cancellation handle for each invocation,
  then returns ordered outputs or errors. The recorded context includes the
  durable session incarnation needed to distinguish reset lifetimes. A pending
  tool wakes on cancellation and returns a structured cancelled error. Tests of
  normalized capabilities can implement `Tool::prepare` on a focused custom
  tool while continuing to use the scripted permission handler for inspection.

Inspection methods clone while holding only the relevant state mutex. A
poisoned mutex is recovered in the same manner as core so a deliberately
panicking test thread does not make later assertions panic. Permanently pending
steps are intended for manual polling and cancellation tests; they have no
release-by-time behavior by design.
