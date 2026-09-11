use super::*;
use crate::{NativeConversation, NativeConversationError, NativeConversationTurn};
use futures_core::Stream;
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    CancellationToken, Capability, Engine, ModelEvent, ModelEventStream, ModelProvider,
    ModelRequest, PermissionRequestId, PermissionRisk, ProviderError, SessionRecord,
    SessionRevision, StopReason, ToolCallId, ToolName, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
    SessionStoreScript, SessionStoreStep,
};
use std::{
    collections::BTreeMap,
    sync::atomic::AtomicUsize,
    task::{Context, Poll},
};

struct RoutingProvider {
    contexts: Option<Arc<NativeMcpContexts>>,
    delegate: ScriptedModelProvider,
    drops: Arc<AtomicUsize>,
}
impl ModelProvider for RoutingProvider {
    fn name(&self) -> &str {
        "context-test"
    }
    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ModelEventStream, ProviderError>> {
        if let Some(contexts) = &self.contexts {
            let context = ToolContext {
                session_id: request.session_id.clone(),
                session_incarnation_id: request.session_incarnation_id.clone(),
                turn_id: request.turn_id.clone(),
                call_id: ToolCallId::new("provider-observer").unwrap(),
            };
            assert!(contexts.snapshot_for_tool(&context).unwrap().is_live());
        }
        self.delegate.stream(request, cancellation)
    }
}
impl Drop for RoutingProvider {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

struct Fixture {
    engine: Engine,
    conversation: NativeConversation,
    store: InMemorySessionStore,
    provider: ScriptedModelProvider,
    drops: Arc<AtomicUsize>,
}
fn fixture(
    name: &str,
    contexts: Option<&Arc<NativeMcpContexts>>,
    script: SessionStoreScript,
) -> Fixture {
    let id = SessionId::new(name).unwrap();
    let mut record = SessionRecord::empty(
        id.clone(),
        SessionIncarnationId::new(format!("{name}-life")).unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        crate::NATIVE_SESSION_METADATA_KEY.into(),
        crate::NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            100,
            crate::NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    let store =
        InMemorySessionStore::configured(BTreeMap::from([(id.clone(), record)]), script, 100);
    let provider = ScriptedModelProvider::new(
        "context-test",
        (0..3).map(|_| {
            ModelProviderStep::events([
                ModelEvent::TextDelta {
                    text: "answer".into(),
                },
                ModelEvent::Stop {
                    reason: StopReason::Completed,
                },
            ])
        }),
    );
    let drops = Arc::new(AtomicUsize::new(0));
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(RoutingProvider {
            contexts: contexts.cloned(),
            delegate: provider.clone(),
            drops: drops.clone(),
        })
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session).unwrap();
    // Keep enrollment separate from provider construction for duplicate tests.
    Fixture {
        engine,
        conversation,
        store,
        provider,
        drops,
    }
}
fn enrolled(name: &str, contexts: &Arc<NativeMcpContexts>, script: SessionStoreScript) -> Fixture {
    let mut fixture = fixture(name, Some(contexts), script);
    fixture.conversation = fixture.conversation.with_mcp_contexts(contexts).unwrap();
    fixture
}
fn admit(fixture: &Fixture) -> (NativeConversationTurn, ToolContext) {
    let turn = block_on(fixture.conversation.prompt("question".into(), 200)).unwrap();
    let context = ToolContext {
        session_id: fixture.conversation.id(),
        session_incarnation_id: fixture.conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    (turn, context)
}
fn complete(turn: NativeConversationTurn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(std::result::Result::is_ok), "{events:?}");
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
}

#[test]
fn construction_and_unpolled_prompt_are_inert_and_provider_observes_enrolled_turn() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("inert", &contexts, SessionStoreScript::default());
    let before = fixture.store.calls();
    drop(fixture.conversation.prompt("unpolled".into(), 200));
    assert_eq!(fixture.store.calls(), before);
    assert!(fixture.provider.requests().is_empty());
    assert!(
        contexts.routes.lock().unwrap()[0]
            .upgrade()
            .unwrap()
            .active
            .lock()
            .unwrap()
            .route
            .is_none()
    );
    let (turn, context) = admit(&fixture);
    assert!(fixture.provider.requests().is_empty());
    assert!(contexts.snapshot_for_tool(&context).unwrap().is_live());
    complete(turn);
    assert_eq!(fixture.provider.requests().len(), 1);
}

#[test]
fn absent_injection_preserves_conversation_behavior() {
    let fixture = fixture("plain", None, SessionStoreScript::default());
    complete(admit(&fixture).0);
}

#[test]
fn exact_tool_and_permission_lookup_share_registry_and_reject_foreign_fields() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("lookup", &contexts, SessionStoreScript::default());
    let (_turn, context) = admit(&fixture);
    let request = PermissionRequest {
        id: PermissionRequestId::new("permission").unwrap(),
        session_id: context.session_id.clone(),
        session_incarnation_id: context.session_incarnation_id.clone(),
        turn_id: context.turn_id.clone(),
        capability: Capability::Tool {
            name: ToolName::new("mcp_test").unwrap(),
            call_id: context.call_id.clone(),
            arguments: serde_json::json!({}),
        },
        risk: PermissionRisk::Low,
        reason: "lookup only".into(),
    };
    assert!(Arc::ptr_eq(
        &contexts
            .snapshot_for_tool(&context)
            .unwrap()
            .registry()
            .unwrap(),
        &contexts
            .snapshot_for_permission(&request)
            .unwrap()
            .registry()
            .unwrap()
    ));
    for field in 0..3 {
        let mut foreign = context.clone();
        match field {
            0 => foreign.session_id = SessionId::new("foreign").unwrap(),
            1 => foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap(),
            _ => foreign.turn_id = TurnId::new("foreign").unwrap(),
        }
        assert_eq!(
            contexts.snapshot_for_tool(&foreign).unwrap_err(),
            NativeMcpContextError::Unavailable
        );
    }
}

#[test]
fn separate_engines_do_not_share_registry_even_with_equal_turn_and_call_ids() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let first = enrolled("first", &contexts, SessionStoreScript::default());
    let second = enrolled("second", &contexts, SessionStoreScript::default());
    let (first_turn, first_context) = admit(&first);
    let (_second_turn, second_context) = admit(&second);
    assert_eq!(first_context.turn_id, second_context.turn_id);
    let first_snapshot = contexts.snapshot_for_tool(&first_context).unwrap();
    let second_snapshot = contexts.snapshot_for_tool(&second_context).unwrap();
    assert!(!Arc::ptr_eq(
        &first_snapshot.registry().unwrap(),
        &second_snapshot.registry().unwrap()
    ));
    drop(first_turn);
    assert!(!first_snapshot.is_live());
    assert!(second_snapshot.is_live());
}

#[test]
fn duplicate_live_identity_across_engines_is_rejected() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let _first = enrolled("duplicate", &contexts, SessionStoreScript::default());
    let second = fixture("duplicate", Some(&contexts), SessionStoreScript::default());
    assert_eq!(
        second
            .conversation
            .with_mcp_contexts(&contexts)
            .unwrap_err(),
        NativeConversationError::McpContext(NativeMcpContextError::Duplicate)
    );
    assert_eq!(contexts.routes.lock().unwrap().len(), 1);
}

#[test]
fn snapshot_and_observer_do_not_own_engine_or_keep_registration_alive() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("ownership", &contexts, SessionStoreScript::default());
    let (turn, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    let registry = snapshot.registry().unwrap();
    let observer = snapshot.cancelled();
    let drops = fixture.drops.clone();
    drop(turn);
    drop(fixture);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert!(registry.revalidate().is_err());
    assert!(!snapshot.is_live());
    block_on(observer);
}

#[test]
fn conversation_drop_retires_retained_turn_and_snapshot() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("drop-owner", &contexts, SessionStoreScript::default());
    let (turn, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    drop(fixture.conversation);
    assert!(!snapshot.is_live());
    assert!(snapshot.registry().is_err());
    block_on(snapshot.cancelled());
    drop(turn);
}

#[test]
fn cancellation_rejects_route_before_registration_drop() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("cancel", &contexts, SessionStoreScript::default());
    let (turn, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    assert!(turn.handle().cancel());
    assert!(!snapshot.is_live());
    assert!(contexts.snapshot_for_tool(&context).is_err());
    block_on(snapshot.cancelled());
}

#[test]
fn completion_and_continuation_use_fresh_turn_registries() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("continue", &contexts, SessionStoreScript::default());
    let (first, first_context) = admit(&fixture);
    let old = contexts.snapshot_for_tool(&first_context).unwrap();
    let registry = old.registry().unwrap();
    drop(first);
    let second = block_on(fixture.conversation.continue_turn(Default::default(), 300)).unwrap();
    let context = ToolContext {
        turn_id: second.handle().id().clone(),
        ..first_context.clone()
    };
    let fresh = contexts.snapshot_for_tool(&context).unwrap();
    assert_ne!(context.turn_id, first_context.turn_id);
    assert!(!Arc::ptr_eq(&registry, &fresh.registry().unwrap()));
    assert!(contexts.snapshot_for_tool(&first_context).is_err());
    assert!(!old.is_live());
    complete(second);
    assert!(!fresh.is_live());
}

#[test]
fn route_is_retired_before_pending_durable_finalization() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled(
        "finalize",
        &contexts,
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Pending,
            ]),
            ..Default::default()
        },
    );
    let (turn, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    let mut turn = Box::pin(turn);
    let waker = noop_waker();
    loop {
        match turn.as_mut().poll_next(&mut Context::from_waker(&waker)) {
            Poll::Ready(Some(Ok(event))) => {
                assert!(!matches!(event.payload, TurnEvent::Completed { .. }))
            }
            Poll::Pending => break,
            result => panic!("unexpected {result:?}"),
        }
    }
    assert!(fixture.conversation.is_busy());
    assert!(!snapshot.is_live());
    assert!(contexts.snapshot_for_tool(&context).is_err());
    block_on(snapshot.cancelled());
}

#[test]
fn capacity_is_bounded_and_dead_owners_release_slots() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let mut fixtures: Vec<_> = (0..MAX_NATIVE_MCP_CONTEXT_SESSIONS)
        .map(|index| {
            enrolled(
                &format!("capacity-{index}"),
                &contexts,
                SessionStoreScript::default(),
            )
        })
        .collect();
    let extra = fixture("overflow", Some(&contexts), SessionStoreScript::default());
    assert_eq!(
        extra.conversation.with_mcp_contexts(&contexts).unwrap_err(),
        NativeConversationError::McpContext(NativeMcpContextError::Limit)
    );
    drop(fixtures.pop());
    let _replacement = enrolled("replacement", &contexts, SessionStoreScript::default());
    assert_eq!(
        contexts.routes.lock().unwrap().len(),
        MAX_NATIVE_MCP_CONTEXT_SESSIONS
    );
}

#[test]
fn retirement_wakes_reentrant_lookup_after_unpublishing_and_unlocking() {
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    for retire_conversation in [false, true] {
        let contexts = Arc::new(NativeMcpContexts::new());
        let fixture = enrolled("reentrant", &contexts, SessionStoreScript::default());
        let (turn, context) = admit(&fixture);
        let snapshot = contexts.snapshot_for_tool(&context).unwrap();
        let routes = contexts.clone();
        let owner = routes.routes.lock().unwrap()[0].upgrade().unwrap();
        let (waker, observed) = reentrant_waker(Callback::Wake, move || {
            assert!(routes.routes.try_lock().is_ok());
            assert!(owner.active.try_lock().is_ok());
            assert!(routes.snapshot_for_tool(&context).is_err());
        });
        let mut observer = snapshot.cancelled();
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        if retire_conversation {
            drop(fixture.conversation);
        } else {
            drop(turn);
        }
        assert!(observed.calls() > 0);
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
}

#[test]
fn explicit_router_drop_retires_without_owning_engine() {
    let contexts = Arc::new(NativeMcpContexts::new());
    // No provider-held router: dropping the sole strong handle really retires it.
    let mut fixture = fixture("router-drop", None, SessionStoreScript::default());
    fixture.conversation = fixture.conversation.with_mcp_contexts(&contexts).unwrap();
    let (_turn, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    drop(contexts);
    assert!(!snapshot.is_live());
    block_on(snapshot.cancelled());
}

#[test]
fn lifecycle_retirement_removes_idle_route_and_prevents_readmission() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled("lifecycle", &contexts, SessionStoreScript::default());
    let gate = crate::conversation_lifecycle::LifecycleGate::new();
    fixture.conversation.bind_lifecycle(&gate).unwrap();
    let mut guard = gate.begin_quiescence().unwrap();
    guard.try_retire().unwrap();
    fixture.conversation.retire_lifecycle_routes();
    assert!(contexts.routes.lock().unwrap().is_empty());
    assert!(block_on(fixture.conversation.prompt("retired".into(), 200)).is_err());
    assert!(fixture.provider.requests().is_empty());
}

#[test]
fn retired_owner_drop_cannot_remove_replacement_for_same_real_session() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = enrolled(
        "replacement-owner",
        &contexts,
        SessionStoreScript::default(),
    );
    let (first, context) = admit(&fixture);
    let snapshot = contexts.snapshot_for_tool(&context).unwrap();
    let old_owner = contexts.routes.lock().unwrap()[0].upgrade().unwrap();
    old_owner.retire();
    drop(first);
    let session = block_on(fixture.engine.load_session(context.session_id.clone()))
        .unwrap()
        .unwrap();
    let replacement = NativeConversation::from_session(session)
        .unwrap()
        .with_mcp_contexts(&contexts)
        .unwrap();
    let second = block_on(replacement.continue_turn(Default::default(), 300)).unwrap();
    let new_context = ToolContext {
        turn_id: second.handle().id().clone(),
        ..context.clone()
    };
    assert_ne!(new_context.turn_id, context.turn_id);
    drop(fixture.conversation);
    drop(old_owner);
    assert!(contexts.snapshot_for_tool(&new_context).unwrap().is_live());
    assert!(contexts.snapshot_for_tool(&context).is_err());
    assert!(!snapshot.is_live());
}

#[test]
fn registration_cannot_reenroll_same_core_turn_after_guard_drop() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let fixture = fixture("same-turn", None, SessionStoreScript::default());
    let session = block_on(fixture.engine.load_session(fixture.conversation.id()))
        .unwrap()
        .unwrap();
    let owner = contexts.register(&session).unwrap();
    let turn = block_on(session.prompt("actual core turn")).unwrap();
    let registration = owner.begin(&session, &turn).unwrap();
    assert!(matches!(
        owner.begin(&session, &turn),
        Err(NativeMcpContextError::Duplicate)
    ));
    drop(registration);
    assert!(matches!(
        owner.begin(&session, &turn),
        Err(NativeMcpContextError::Duplicate)
    ));
    assert!(fixture.provider.requests().is_empty());
}
