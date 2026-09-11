use super::*;
use futures_executor::block_on;
use futures_util::{FutureExt, StreamExt, task::noop_waker};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use std::task::{Context, Poll};

fn conversation(runtime: &NativeMcpRuntime) -> (Engine, NativeConversation) {
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new(
            "fixture",
            (0..3).map(|_| {
                ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }])
            }),
        ))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("pins").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
        .unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_mcp_contexts(&runtime.contexts)
        .unwrap();
    (engine, conversation)
}
fn turn(conversation: &NativeConversation) -> (NativeConversationTurn, ToolContext) {
    let turn = block_on(conversation.prompt("requested change".into(), 1)).unwrap();
    let context = ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    (turn, context)
}

#[test]
fn discovering_does_not_pin_and_replacement_requires_new_exact_turn() {
    let runtime = standalone();
    let (_engine, conversation) = conversation(&runtime);
    let (first_turn, context) = turn(&conversation);
    let native = runtime.contexts.snapshot_for_tool(&context).unwrap();
    assert!(
        runtime
            .for_turn(&native.registry().unwrap())
            .unwrap()
            .is_none()
    );
    assert!(runtime.state.lock().unwrap().turns.is_empty());
    runtime
        .publish(candidate(&runtime, "calendar", &["one"], Arc::default()))
        .unwrap();
    let first = runtime
        .for_turn(&native.registry().unwrap())
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &runtime
            .for_turn(&native.registry().unwrap())
            .unwrap()
            .unwrap()
    ));
    runtime
        .publish(candidate(&runtime, "calendar", &["one"], Arc::default()))
        .unwrap();
    assert!(runtime.for_turn(&native.registry().unwrap()).is_err());
    assert!(
        block_on(first_turn.collect::<Vec<_>>())
            .iter()
            .all(std::result::Result::is_ok)
    );
    let (second_turn, context) = turn(&conversation);
    let second = runtime.contexts.snapshot_for_tool(&context).unwrap();
    let next = runtime
        .for_turn(&second.registry().unwrap())
        .unwrap()
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &next));
    assert_eq!(
        runtime.state.lock().unwrap().turns.len(),
        1,
        "closed pins pruned despite retained native snapshot"
    );
    drop(second_turn);
}

#[test]
fn bounded_serial_acquisition_cancels_and_releases_capacity_without_polling_timers() {
    let runtime = standalone_with_limits(NativeMcpRuntimeLimits {
        max_pending_operations: 2,
        ..Default::default()
    });
    let (_engine, conversation) = conversation(&runtime);
    let (_turn, context) = turn(&conversation);
    let native = runtime.contexts.snapshot_for_tool(&context).unwrap();
    let prepared = candidate(&runtime, "calendar", &["one"], Arc::default());
    let server = prepared.publication.servers[0].clone();
    runtime.publish(prepared).unwrap();
    let cancellation = CancellationToken::new();
    let held = block_on(server.acquire(&native, &cancellation)).unwrap();
    let queued_cancel = CancellationToken::new();
    let mut queued = Box::pin(server.acquire(&native, &queued_cancel));
    assert!(matches!(
        queued.poll_unpin(&mut Context::from_waker(&noop_waker())),
        Poll::Pending
    ));
    assert!(matches!(
        block_on(server.acquire(&native, &cancellation)),
        Err(NativeMcpRuntimeError::Limit)
    ));
    queued_cancel.cancel();
    assert!(matches!(
        block_on(queued),
        Err(NativeMcpRuntimeError::Cancelled)
    ));
    drop(held);
    assert_eq!(server.pending.load(Ordering::Acquire), 0);
    assert!(block_on(server.acquire(&native, &cancellation)).is_ok());
    runtime.close();
    assert!(block_on(server.acquire(&native, &cancellation)).is_err());
}

#[test]
fn cancelled_retirement_keeps_cleanup_owned_until_explicit_drain() {
    let runtime = standalone();
    let first = candidate(&runtime, "calendar", &["one"], Arc::default());
    let old = Arc::downgrade(&first.publication.servers[0]);
    runtime.publish(first).unwrap();
    runtime
        .publish(candidate(&runtime, "calendar", &["two"], Arc::default()))
        .unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        block_on(runtime.drain_retired(Instant::now() + Duration::from_secs(1), cancelled))
            .is_err()
    );
    assert!(old.upgrade().is_some());
    let receipts = block_on(runtime.drain_retired(
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(old.upgrade().is_none());
}
