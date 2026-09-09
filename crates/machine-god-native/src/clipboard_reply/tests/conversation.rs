use super::*;
use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationError,
    NativeConversationHistory, NativeConversationRuntime, NativeHistoryState,
    NativeModelPreferences, NativeReasoningEffort, NativeSessionMetadata, NativeSessionOrigin,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    Engine, ModelEvent, SessionRevision, SessionStoreError, SessionStoreErrorKind, StopReason,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
    SessionStoreScript, SessionStoreStep,
};
use std::collections::BTreeMap;

fn store_fixture() -> (InMemorySessionStore, SessionId) {
    let mut initial = (*record(vec![])).clone();
    initial.revision = SessionRevision(1);
    initial.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            100,
            NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    let id = initial.id.clone();
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(id.clone(), initial)]),
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "fixture",
                    "fixture finalization failure",
                    false,
                )),
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
        100,
    );
    (store, id)
}

#[test]
fn saved_final_reply_survives_failed_native_finalization_reload_and_interruption() {
    let (store, id) = store_fixture();
    let provider = ScriptedModelProvider::new(
        "clipboard",
        [ModelProviderStep::events([
            ModelEvent::TextDelta {
                text: "saved **reply**\n".into(),
            },
            ModelEvent::Stop {
                reason: StopReason::Completed,
            },
        ])],
    );
    let engine = Engine::builder()
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(store.clone())
        .provider(provider)
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id.clone())).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session.clone()).unwrap();
    let before = conversation.record_snapshot();
    assert!(Arc::ptr_eq(&before, &session.record_snapshot()));
    let turn = block_on(conversation.prompt("question".into(), 200)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    let final_snapshot = conversation.record_snapshot();
    assert!(!Arc::ptr_eq(&before, &final_snapshot));
    assert!(before.messages.is_empty());
    assert_eq!(&*selected(final_snapshot), "saved **reply**\n");
    assert_eq!(
        NativeConversationHistory::from_record(&store.record(&id).unwrap())
            .unwrap()
            .group(0)
            .unwrap()
            .state(),
        NativeHistoryState::Running
    );
    drop(conversation);
    drop(session);
    drop(engine);

    let engine = Engine::builder()
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(store.clone())
        .provider(ScriptedModelProvider::new("reload", []))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session.clone()).unwrap();
    let pinned = NativeClipboardReplySelection::new(conversation.record_snapshot());
    // Real native admission marks the recovered previous attempt Interrupted.
    // The new turn is dropped before any provider request or partial reply.
    let next = block_on(conversation.prompt("next user request".into(), 300)).unwrap();
    drop(next);
    let recovered = conversation.record_snapshot();
    assert_eq!(
        NativeConversationHistory::from_record(&recovered)
            .unwrap()
            .group(0)
            .unwrap()
            .state(),
        NativeHistoryState::Interrupted
    );
    assert_eq!(&*selected(recovered.clone()), "saved **reply**\n");
    assert_eq!(&*selected(pinned.record), "saved **reply**\n");
    let runtime = NativeConversationRuntime::new(
        conversation,
        NativeModelPreferences::new("model", NativeReasoningEffort::default(), false).unwrap(),
        None,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &runtime.record_snapshot(),
        &session.record_snapshot()
    ));
    assert!(Arc::ptr_eq(&runtime.record_snapshot(), &recovered));
}
