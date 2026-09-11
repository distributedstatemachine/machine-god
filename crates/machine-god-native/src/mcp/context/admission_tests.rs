//! Tests requiring access to the conversation's prior route owners.
use super::*;
use futures_executor::block_on;
use machine_god_core::{Engine, SessionIncarnationId, ToolCallId, TurnId};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};

#[test]
fn failed_later_permission_enrollment_unwinds_mcp_registration_before_returning() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let permissions = Arc::new(crate::NativePermissionContexts::new());
    let mut record = SessionRecord::empty(
        SessionId::new("unwind").unwrap(),
        SessionIncarnationId::new("unwind-life").unwrap(),
    );
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            100,
            crate::NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    let id = record.id.clone();
    let provider = ScriptedModelProvider::new("unwind", []);
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::from_records(
            std::collections::BTreeMap::from([(id.clone(), record)]),
        ))
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_permission_contexts(&permissions)
        .unwrap()
        .with_mcp_contexts(&contexts)
        .unwrap();
    conversation.permission_contexts.as_ref().unwrap().retire();
    let result = block_on(conversation.prompt("admission fails".into(), 200));
    assert!(matches!(
        result,
        Err(NativeConversationError::PermissionContext(_))
    ));
    let context = ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: TurnId::new("turn-1").unwrap(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    assert!(contexts.snapshot_for_tool(&context).is_err());
    assert!(!conversation.is_busy());
    assert!(!conversation.session.has_active_turn());
    assert!(provider.requests().is_empty());
    assert!(
        conversation
            .mcp_contexts
            .as_ref()
            .unwrap()
            .begin(
                &conversation.session,
                &block_on(conversation.session.prompt("fresh")).unwrap()
            )
            .is_ok()
    );
}
