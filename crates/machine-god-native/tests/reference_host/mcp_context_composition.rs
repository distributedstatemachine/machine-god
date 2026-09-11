use super::*;
use machine_god_native::{
    NativeConversation, NativeSessionMetadata, mcp::context::NativeMcpContexts,
};

#[test]
fn mcp_host_attachment_enrolls_actual_turns_and_absent_selection_is_unchanged() {
    for selected in [false, true] {
        let temporary = TemporaryDirectory::new("mcp-context-composition");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let contexts = Arc::new(NativeMcpContexts::new());
        let mut options =
            NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()));
        if selected {
            options = options.with_mcp_contexts(contexts.clone());
        }
        let response = b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"answer\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n";
        let transport = ScriptedTransport::new("mcp-context", [response]);
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            built_in_config(), Arc::new(transport.clone()), production_gateway_target(), prepared,
            Arc::new(AllowingPrompter::default()), inert_question_prompter(), never_deadline(), options,
        ).unwrap();
        assert_eq!(host.mcp_contexts().is_some(), selected);
        if selected {
            assert!(Arc::ptr_eq(&host.mcp_contexts().unwrap(), &contexts));
        }
        assert!(host.terminal_shutdown_completion().is_none());
        let conversation = futures_executor::block_on(NativeConversation::create(
            host.session_lifecycle(),
            NativeSessionMetadata::default(),
        ))
        .unwrap();
        let conversation = host.configure_conversation_mcp(conversation).unwrap();
        let turn = futures_executor::block_on(conversation.prompt("question".into(), 100)).unwrap();
        let context = ToolContext {
            session_id: conversation.id(),
            session_incarnation_id: conversation.incarnation_id(),
            turn_id: turn.handle().id().clone(),
            call_id: ToolCallId::new("observed-call").unwrap(),
        };
        assert!(transport.requests().is_empty());
        let snapshot = contexts.snapshot_for_tool(&context).ok();
        assert_eq!(snapshot.is_some(), selected);
        for foreign in [
            ToolContext {
                session_id: SessionId::new("foreign").unwrap(),
                ..context.clone()
            },
            ToolContext {
                session_incarnation_id: SessionIncarnationId::new("foreign").unwrap(),
                ..context.clone()
            },
            ToolContext {
                turn_id: TurnId::new("foreign").unwrap(),
                ..context.clone()
            },
        ] {
            assert!(contexts.snapshot_for_tool(&foreign).is_err());
        }
        let events = futures_executor::block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok));
        assert!(matches!(
            &events.last().unwrap().as_ref().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Completed,
                ..
            }
        ));
        assert_eq!(transport.requests().len(), 1);
        assert!(contexts.snapshot_for_tool(&context).is_err());
        if let Some(snapshot) = snapshot {
            assert!(!snapshot.is_live());
            assert!(snapshot.registry().is_err());
        }
    }
}
