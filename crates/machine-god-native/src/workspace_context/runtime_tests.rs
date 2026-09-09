use super::*;
use futures_executor::block_on;
use futures_util::task::noop_waker;
use machine_god_core::Engine;
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};

fn runtime() -> NativeConversationRuntime {
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("test", []))
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        )
        .unwrap();
    let preferences = NativeModelPreferences::new(
        "model",
        crate::NativeReasoningEffort::parse("high").unwrap(),
        false,
    )
    .unwrap();
    NativeConversationRuntime::new(
        NativeConversation::from_session(session).unwrap(),
        preferences,
        None,
    )
    .unwrap()
}

#[test]
fn workspace_control_rejects_active_and_queued_jobs_and_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<NativeWorkspaceControlLease>();
    let runtime = runtime();
    let queued = runtime.enqueue("queued".into()).unwrap();
    assert!(matches!(
        runtime.acquire_workspace_control(),
        Err(NativeConversationRuntimeError::Busy)
    ));
    assert!(runtime.cancel_queued(queued));
    let guard = runtime.acquire_workspace_control().unwrap();
    assert!(runtime.status().active);
    assert!(matches!(
        runtime.acquire_workspace_control(),
        Err(NativeConversationRuntimeError::Busy)
    ));
    drop(guard);
    runtime.enqueue("taken".into()).unwrap();
    let turn = block_on(runtime.start_next(1)).unwrap().unwrap();
    assert!(matches!(
        runtime.acquire_workspace_control(),
        Err(NativeConversationRuntimeError::Busy)
    ));
    drop(turn);
    assert!(runtime.acquire_workspace_control().is_ok());
}

#[test]
fn workspace_control_fences_direct_conversation_admission_in_both_orders() {
    let runtime = runtime();
    let direct = block_on(runtime.conversation.prompt("direct".into(), 1)).unwrap();
    assert!(matches!(
        runtime.acquire_workspace_control(),
        Err(NativeConversationRuntimeError::Busy)
    ));
    assert!(!runtime.status().active);
    drop(direct);
    let guard = runtime.acquire_workspace_control().unwrap();
    assert!(matches!(
        block_on(runtime.conversation.prompt("blocked".into(), 2)),
        Err(NativeConversationError::Busy)
    ));
    assert!(matches!(
        block_on(
            runtime
                .conversation
                .continue_turn(InferenceOptions::default(), 2)
        ),
        Err(NativeConversationError::Busy)
    ));
    drop(guard);
    assert!(block_on(runtime.conversation.prompt("accepted".into(), 2)).is_ok());
}

#[test]
fn workspace_control_retains_quiescence_admission_until_worker_owned_drop() {
    let runtime = runtime();
    let guard = runtime.acquire_workspace_control().unwrap();
    let mut quiescence = runtime.begin_quiescence().unwrap();
    let waker = noop_waker();
    assert!(
        quiescence
            .wait_idle()
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(guard);
    block_on(quiescence.wait_idle()).unwrap();
    assert!(quiescence.selection_snapshot().is_ok());
    drop(quiescence);
    assert!(runtime.acquire_workspace_control().is_ok());
}
