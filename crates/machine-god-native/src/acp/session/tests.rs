use super::*;
use crate::acp::prompt::decode_prompt_input;
use machine_god_core::{
    ContentBlock, Message, Role, SessionIncarnationId, SessionRecord, ToolCall, ToolCallId,
    ToolName, ToolOutput,
};
use serde_json::json;

mod pure {
    use super::*;

    #[test]
    fn native_acp_origin_round_trips_without_other_match_callers() {
        let metadata = crate::NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            100,
            NativeSessionOrigin::Acp,
        )
        .unwrap();
        let map = std::collections::BTreeMap::from([(
            crate::NATIVE_SESSION_METADATA_KEY.to_owned(),
            metadata.to_value(),
        )]);
        assert_eq!(
            crate::NativeSessionMetadata::from_metadata(&map)
                .unwrap()
                .origin(),
            Some(NativeSessionOrigin::Acp)
        );
        assert_eq!(NativeSessionOrigin::Acp.as_str(), "acp");
    }

    #[test]
    fn supported_modes_are_exact_and_modern() {
        assert_eq!(parse_mode("ask").unwrap(), PermissionMode::Ask);
        assert_eq!(parse_mode("auto").unwrap(), PermissionMode::Auto);
        assert_eq!(parse_mode("yolo").unwrap(), PermissionMode::Yolo);
        for value in ["allow_always", "default", "ASK", "", "ask\n"] {
            assert!(parse_mode(value).is_err());
        }
    }

    fn record() -> SessionRecord {
        SessionRecord::empty(
            SessionId::new("acp-test").unwrap(),
            SessionIncarnationId::new("incarnation-test").unwrap(),
        )
    }

    #[test]
    fn load_history_is_incremental_and_skips_provider_only_content() {
        let mut saved = record();
        saved.messages = vec![
            Message::text(Role::System, "hidden"),
            Message::text(Role::User, "question"),
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Json {
                        value: json!({"private":"hidden"}),
                    },
                    ContentBlock::Text {
                        text: "answer".into(),
                    },
                ],
            },
        ];
        let saved = Arc::new(saved);
        let mut history = NativeAcpHistory::new(saved.clone());
        assert_eq!(Arc::strong_count(&saved), 2);
        assert_eq!(
            history.next_update().unwrap().unwrap(),
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"question"}})
        );
        assert_eq!(
            history.next_update().unwrap().unwrap()["content"]["text"],
            "answer"
        );
        assert!(history.next_update().unwrap().is_none());
        assert_eq!(saved.messages.len(), 3);
    }

    #[test]
    fn saved_tool_evidence_projects_without_execution_or_invented_success() {
        let mut saved = record();
        saved.messages = vec![
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call").unwrap(),
                        name: ToolName::new("write_file").unwrap(),
                        arguments: json!({"path":"never-written"}),
                    },
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    call_id: ToolCallId::new("call").unwrap(),
                    output: ToolOutput {
                        content: json!({"error":"denied"}),
                        is_error: true,
                    },
                }],
            },
        ];
        let mut history = NativeAcpHistory::new(Arc::new(saved));
        let call = history.next_update().unwrap().unwrap();
        assert_eq!(call["rawInput"]["path"], "never-written");
        assert_eq!(call["status"], "pending");
        let result = history.next_update().unwrap().unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["rawOutput"]["error"], "denied");
        assert!(history.next_update().unwrap().is_none());
    }

    #[test]
    fn saved_tool_json_preserves_arbitrary_numeric_spellings() {
        let exact: serde_json::Value =
            serde_json::from_str(r#"{"negative":-0,"huge":1e400,"fraction":1.2300}"#).unwrap();
        let mut saved = record();
        saved.messages = vec![Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call").unwrap(),
                        name: ToolName::new("tool").unwrap(),
                        arguments: exact.clone(),
                    },
                },
                ContentBlock::ToolResult {
                    call_id: ToolCallId::new("call").unwrap(),
                    output: ToolOutput {
                        content: exact.clone(),
                        is_error: false,
                    },
                },
            ],
        }];
        let mut history = NativeAcpHistory::new(Arc::new(saved));
        assert_eq!(
            history.next_update().unwrap().unwrap()["rawInput"].to_string(),
            exact.to_string()
        );
        assert_eq!(
            history.next_update().unwrap().unwrap()["rawOutput"].to_string(),
            exact.to_string()
        );
    }

    #[test]
    fn oversized_history_projection_fails_before_copy_and_cannot_skip_a_block() {
        let mut saved = record();
        saved.messages = vec![
            Message::text(Role::User, "\n".repeat(4 * 1024 * 1024)),
            Message::text(Role::Assistant, "must not skip"),
        ];
        let saved = Arc::new(saved);
        let mut history = NativeAcpHistory::new(saved.clone());
        assert!(matches!(history.next_update(), Err(AcpSessionError::Limit)));
        assert!(matches!(history.next_update(), Err(AcpSessionError::Limit)));
        assert_eq!(Arc::strong_count(&saved), 2);
    }
}

use crate::interactive_session::tests::support;

fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn options(fixture: &support::Fixture) -> NativeInteractiveSessionOptions {
    NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        crate::NativeModelPreferences::new(
            "workspace/default",
            crate::NativeReasoningEffort::default(),
            false,
        )
        .unwrap(),
    )
    .unwrap()
}

fn input(text: &str) -> NativeAcpPrompt {
    decode_prompt_input(&json!({"prompt":[{"type":"text","text":text}]})).unwrap()
}
async fn outcome(session: &mut NativeAcpSession) -> NativeInteractiveOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        futures_util::future::poll_fn(|cx| {
            let _ = session.poll_progress(cx, 300);
            let _ = session.take_presentation();
            session.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}
async fn close(mut session: NativeAcpSession) {
    session.request_close(&session.id()).unwrap();
    assert!(matches!(
        outcome(&mut session).await,
        NativeInteractiveOutcome::Shutdown
    ));
}

#[test]
fn native_owner_is_inert_before_poll_and_binds_exact_session() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_workspace();
        let future = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::New,
            100,
        );
        drop(future);
        assert!(fixture.transport.requests().is_empty());
        let mut session = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::New,
            100,
        )
        .await
        .unwrap();
        let metadata = crate::NativeSessionMetadata::from_metadata(
            &session.runtime().record_snapshot().metadata,
        )
        .unwrap();
        assert_eq!(metadata.origin(), Some(NativeSessionOrigin::Acp));
        assert!(session.take_loaded_history().is_none());
        let other = SessionId::new("other-session").unwrap();
        assert!(matches!(
            session.enqueue(&other, input("never")),
            Err(AcpSessionError::WrongSession)
        ));
        assert!(matches!(
            session.request_cancel(&other),
            Err(AcpSessionError::WrongSession)
        ));
        assert!(matches!(
            session.request_close(&other),
            Err(AcpSessionError::WrongSession)
        ));
        assert!(matches!(
            session.set_mode(&other, "yolo"),
            Err(AcpSessionError::WrongSession)
        ));
        assert!(matches!(
            session.set_config_option(&other, "model", "other/model"),
            Err(AcpSessionError::WrongSession)
        ));
        close(session).await;
        fixture.finish();
    });
}

#[test]
fn native_turn_completion_load_and_resume_use_saved_state_without_reexecution() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_workspace();
        fixture.transport.push(support::answer());
        let mut session = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::New,
            100,
        )
        .await
        .unwrap();
        let id = session.id();
        session.set_mode(&id, "auto").unwrap();
        assert_eq!(session.mode().unwrap(), PermissionMode::Auto);
        assert_eq!(
            session.set_config_option(&id, "mode", "ask").unwrap(),
            NativeAcpConfigChange::Mode(PermissionMode::Ask)
        );
        assert_eq!(session.mode().unwrap(), PermissionMode::Ask);
        session
            .set_config_option(&id, "model", "test/selected")
            .unwrap();
        session.flush_model(&id, 110).await.unwrap();
        session.enqueue(&id, input("hello")).unwrap();
        assert!(matches!(
            session.enqueue(&id, input("second")),
            Err(AcpSessionError::Busy)
        ));
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Turn(Ok(_))
        ));
        assert_eq!(session.runtime().record_snapshot().messages.len(), 2);
        close(session).await;
        let mut loaded = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::Load(id.clone()),
            400,
        )
        .await
        .unwrap();
        assert_eq!(
            loaded.runtime().model_preferences().model(),
            "test/selected"
        );
        let mut history = loaded.take_loaded_history().unwrap();
        assert_eq!(
            history.next_update().unwrap().unwrap()["content"]["text"],
            "hello"
        );
        assert!(history.next_update().unwrap().is_some());
        assert!(history.next_update().unwrap().is_none());
        close(loaded).await;
        let mut resumed = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::Resume(id),
            500,
        )
        .await
        .unwrap();
        assert!(resumed.take_loaded_history().is_none());
        assert_eq!(fixture.transport.requests().len(), 1);
        close(resumed).await;
        fixture.finish();
    });
}

#[test]
fn native_cancellation_before_first_poll_keeps_completion_owned() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_workspace();
        let mut session = NativeAcpSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeAcpSessionSelection::New,
            100,
        )
        .await
        .unwrap();
        let id = session.id();
        session
            .enqueue(&id, input("cancel before provider"))
            .unwrap();
        assert!(session.request_cancel(&id).unwrap());
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Turn(_)
        ));
        assert!(fixture.transport.requests().is_empty());
        assert!(!session.request_cancel(&id).unwrap());
        close(session).await;
        fixture.finish();
    });
}
