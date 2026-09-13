use super::*;
use crate::{
    AskUserQuestionTool, NativeInteractivePromptBridge, NativeInteractivePromptInbox,
    NativeInteractivePromptLimits, NativeInteractivePromptView, PermissionPromptDecision,
    PermissionPrompter,
    acp::interaction::NativeAcpElicitationPresenter,
    mcp::{
        interaction::{McpClientUrlEndpoint, McpElicitationPresenter, McpElicitationPromptRequest},
        mrtr::{McpElicitationRequest, McpMrtrLimits},
        protocol::ProtocolVersion,
    },
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, Capability, PermissionRequest,
    PermissionRequestId, PermissionRisk, SessionId, SessionIncarnationId, StopReason, TokenUsage,
    Tool, ToolCall, ToolCallId, ToolContext, ToolName, ToolOutput, TurnId,
};
use serde_json::value::RawValue;
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
};

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("actual-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("actual-incarnation").unwrap(),
        turn_id: TurnId::new("actual-turn").unwrap(),
        call_id: ToolCallId::new("actual-call").unwrap(),
    }
}
fn owner() -> BackgroundOutputOwner {
    let context = context();
    BackgroundOutputOwner::new(context.session_id, context.session_incarnation_id)
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: context().call_id,
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}
fn event(payload: TurnEvent) -> EngineEvent {
    let context = context();
    EngineEvent {
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        sequence: 19,
        payload,
    }
}
fn bridge() -> (
    Arc<NativeInteractivePromptBridge>,
    NativeInteractivePromptInbox,
) {
    let (bridge, mut inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    inbox.activate(owner()).unwrap();
    (bridge, inbox)
}
fn view(inbox: &mut NativeInteractivePromptInbox) -> NativeInteractivePromptView {
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut Context::from_waker(Waker::noop())) else {
        panic!("native view");
    };
    view
}
fn poll<T>(future: &mut BoxFuture<'_, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn permission() -> PermissionRequest {
    let context = context();
    PermissionRequest {
        id: PermissionRequestId::new("permission-not-call").unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability: Capability::Tool {
            name: ToolName::new("read_file").unwrap(),
            call_id: context.call_id,
            arguments: json!({"path":"private"}),
        },
        risk: PermissionRisk::Low,
        reason: "Read private input".into(),
    }
}
fn exact(text: &str) -> Value {
    machine_god_core::json::from_str(text).unwrap()
}
fn request(text: &str) -> McpElicitationPromptRequest {
    let raw = RawValue::from_string(text.into()).unwrap();
    McpElicitationPromptRequest::new(
        context(),
        Arc::from("actual-server"),
        ToolName::new("actual-tool").unwrap(),
        Arc::new(
            McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
                .unwrap(),
        ),
    )
    .unwrap()
}
fn form() -> McpElicitationPromptRequest {
    request(
        r#"{"mode":"form","message":"Private form","requestedSchema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"n":{"type":"integer","minimum":9007199254740993}},"required":["n"]},"_meta":{"privateNumber":1e400}}"#,
    )
}

#[test]
fn text_and_thought_chunks_preserve_content_and_native_labels() {
    for (payload, expected) in [
        (
            ModelEvent::TextDelta {
                text: "line\n\u{001b}\"🙂".into(),
            },
            "agent_message_chunk",
        ),
        (
            ModelEvent::ReasoningDelta {
                text: "line\n\u{001b}\"🙂".into(),
            },
            "agent_thought_chunk",
        ),
    ] {
        let update = project_event(&event(TurnEvent::Model { event: payload }))
            .unwrap()
            .unwrap();
        assert_eq!(update["sessionUpdate"], expected);
        assert_eq!(update["content"]["text"], "line\n\u{001b}\"🙂");
        assert_eq!(update["_meta"]["machineGod"]["turnId"], "actual-turn");
        assert_eq!(
            update["_meta"]["machineGod"]["sessionIncarnationId"],
            "actual-incarnation"
        );
    }
}

#[test]
fn tool_projection_preserves_exact_numbers_and_only_observed_status() {
    let arguments = exact(r#"{"integer":9007199254740993,"negative":-0,"large":1e400}"#);
    let pending = project_event(&event(TurnEvent::Model {
        event: ModelEvent::ToolCall {
            call: call("terminal", arguments.clone()),
        },
    }))
    .unwrap()
    .unwrap();
    assert_eq!(pending["toolCallId"], "actual-call");
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending["kind"], "execute");
    assert_eq!(pending["rawInput"], arguments);
    let started = project_event(&event(TurnEvent::ToolStarted {
        call: call("terminal", arguments.clone()),
    }))
    .unwrap()
    .unwrap();
    assert_eq!(started["status"], "in_progress");
    assert_eq!(started["sessionUpdate"], "tool_call_update");
    let finished = project_event(&event(TurnEvent::ToolFinished {
        call_id: context().call_id,
        output: ToolOutput {
            content: arguments.clone(),
            is_error: true,
        },
    }))
    .unwrap()
    .unwrap();
    assert_eq!(finished["status"], "failed");
    assert_eq!(finished["rawOutput"], arguments);
    let text = finished["content"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("1e400") && text.contains("-0"));
}

#[test]
fn terminal_engine_and_provider_events_never_finalize_prompt() {
    for payload in [
        TurnEvent::Completed {
            reason: StopReason::Completed,
            usage: TokenUsage::default(),
        },
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Completed,
            },
        },
        TurnEvent::Failed {
            component: "private".into(),
            code: "private".into(),
            message: "private".into(),
            retryable: false,
        },
        TurnEvent::Started,
    ] {
        assert!(project_event(&event(payload)).unwrap().is_none());
    }
}

#[test]
fn oversized_escaped_and_invalid_constructed_numbers_are_rejected() {
    for text in [
        "x".repeat(ACP_MAX_FRAME_BYTES),
        "\0".repeat(ACP_MAX_FRAME_BYTES / 5),
    ] {
        assert_eq!(
            project_event(&event(TurnEvent::Model {
                event: ModelEvent::TextDelta { text }
            })),
            Err(ProjectionError::Limit)
        );
    }
    let arguments = Value::Number(serde_json::Number::from_string_unchecked(
        "0,\"injected\":true".into(),
    ));
    assert_eq!(
        project_event(&event(TurnEvent::ToolStarted {
            call: call("read_file", arguments)
        })),
        Err(ProjectionError::Limit)
    );
}

#[test]
fn initialization_advertises_modern_native_surface_only() {
    let value = initialize_result();
    assert_eq!(value["protocolVersion"], 1);
    assert!(
        value["agentCapabilities"]["mcpCapabilities"]
            .get("sse")
            .is_none()
    );
    assert!(value["agentCapabilities"].get("fs").is_none());
    assert!(
        value["agentCapabilities"]["sessionCapabilities"]
            .get("remove")
            .is_none()
    );
}

#[test]
fn projection_rejects_deep_or_wide_constructed_values_before_clone() {
    let mut deep = Value::Null;
    for _ in 0..protocol::ACP_MAX_JSON_DEPTH {
        deep = Value::Array(vec![deep]);
    }
    let wide = Value::Array(vec![Value::Null; protocol::ACP_MAX_JSON_NODES]);
    for arguments in [deep, wide] {
        assert_eq!(
            project_event(&event(TurnEvent::ToolStarted {
                call: call("read_file", arguments)
            })),
            Err(ProjectionError::Limit)
        );
    }
}

#[test]
fn permissions_require_actual_call_and_map_always_to_volatile_session() {
    let (bridge, mut inbox) = bridge();
    let mut pending = bridge.prompt(permission());
    assert!(poll(&mut pending).is_pending());
    let view = view(&mut inbox);
    assert!(project_prompt(&view, None).is_err());
    assert!(project_permission(&view, &call("terminal", json!({}))).is_err());
    let wire = project_permission(&view, &call("read_file", json!({"path":"private"}))).unwrap();
    assert_eq!(wire.method, "session/request_permission");
    assert_eq!(wire.params["toolCall"]["toolCallId"], "actual-call");
    assert_eq!(wire.params["options"].as_array().unwrap().len(), 3);
    assert!(!format!("{wire:?}").contains("private"));
    let response = decode_reply(
        &view,
        &json!({"outcome":{"outcome":"selected","optionId":"allow_always"}}),
    )
    .unwrap();
    inbox.reply(view.token(), response).unwrap();
    assert_eq!(
        futures_executor::block_on(pending),
        Ok(PermissionPromptDecision::AllowSession)
    );
}

#[test]
fn malformed_permission_replies_do_not_settle_the_native_request() {
    let (bridge, mut inbox) = bridge();
    let mut pending = bridge.prompt(permission());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    for invalid in [
        json!({"outcome":{"outcome":"selected","optionId":"allow_forever"}}),
        json!({"outcome":{"outcome":"cancelled","optionId":"allow_once"}}),
        json!({"outcome":{"outcome":"selected","optionId":"allow_once","save":true}}),
        json!({"action":"accept"}),
        json!({"outcome":"selected"}),
    ] {
        assert!(decode_reply(&prompt, &invalid).is_err());
        assert!(poll(&mut pending).is_pending());
    }
    inbox
        .reply(
            prompt.token(),
            decode_reply(&prompt, &json!({"outcome":{"outcome":"cancelled"}})).unwrap(),
        )
        .unwrap();
    assert_eq!(
        futures_executor::block_on(pending),
        Ok(PermissionPromptDecision::Deny)
    );
}

#[test]
fn typed_reply_validation_never_replaces_native_freshness_checks() {
    let (bridge, mut inbox) = bridge();
    let mut pending = bridge.prompt(permission());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    let result = json!({"outcome":{"outcome":"selected","optionId":"allow_once"}});
    inbox.activate(owner()).unwrap();
    // Syntax/schema remain valid, but the original token cannot acquire a new scope.
    let answer = decode_reply(&prompt, &result).unwrap();
    assert!(inbox.reply(prompt.token(), answer).is_err());
}

#[test]
fn form_schema_metadata_and_accepted_number_are_lossless() {
    let (bridge, mut inbox) = bridge();
    let mut pending = bridge.present(form(), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    let wire = project_prompt(&prompt, None).unwrap();
    assert_eq!(wire.reply_kind, NativeAcpReplyKind::Form);
    assert_eq!(wire.params["sessionId"], "actual-session");
    assert_eq!(wire.params["toolCallId"], "actual-call");
    assert!(wire.params["requestedSchema"].get("$schema").is_none());
    assert_eq!(
        wire.params["_meta"]["privateNumber"]
            .as_number()
            .unwrap()
            .as_str(),
        "1e400"
    );
    let result = exact(r#"{"action":"accept","content":{"n":9007199254740993.0}}"#);
    let answer = decode_reply(&prompt, &result).unwrap();
    inbox.reply(prompt.token(), answer).unwrap();
    let answer = futures_executor::block_on(pending).unwrap();
    assert!(answer.canonical_json().get().contains("9007199254740993.0"));
}

#[test]
fn malformed_schema_and_action_results_are_rejected_before_inbox_reply() {
    let (bridge, mut inbox) = bridge();
    let mut pending = bridge.present(form(), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    for invalid in [
        json!({"action":"accept","content":{"n":9007199254740992_u64}}),
        json!({"action":"accept","content":{}}),
        json!({"action":"accept","content":{"n":"9007199254740993"}}),
        json!({"action":"cancel","content":{}}),
        json!({"action":"decline","extra":true}),
        json!({"action":"future"}),
    ] {
        assert!(decode_reply(&prompt, &invalid).is_err());
        assert!(poll(&mut pending).is_pending());
    }
}

#[test]
fn escaped_mcp_reply_obeys_native_serialized_answer_ceiling() {
    let (bridge, mut inbox) = bridge();
    let request = request(
        r#"{"message":"Text","requestedSchema":{"type":"object","properties":{"text":{"type":"string"}}}}"#,
    );
    let mut pending = bridge.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    let result = json!({"action":"accept","content":{"text":"\0".repeat(24*1024)}});
    assert!(matches!(
        decode_reply(&prompt, &result),
        Err(ProjectionError::Limit)
    ));
    assert!(poll(&mut pending).is_pending());
}

#[test]
fn schema_projection_preserves_properties_named_like_annotations() {
    let (bridge, mut inbox) = bridge();
    let request = request(
        r#"{"message":"Fields","requestedSchema":{"type":"object","properties":{"$schema":{"type":"string"},"enumNames":{"type":"string"}},"required":["$schema","enumNames"]}}"#,
    );
    let mut pending = bridge.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    let wire = project_prompt(&prompt, None).unwrap();
    let properties = &wire.params["requestedSchema"]["properties"];
    assert!(properties.get("$schema").is_some());
    assert!(properties.get("enumNames").is_some());
    let response = json!({"action":"accept","content":{"$schema":"a","enumNames":"b"}});
    assert!(decode_reply(&prompt, &response).is_ok());
}

#[test]
fn human_feature_projection_never_invents_tool_call_authority() {
    let (bridge, mut inbox) = bridge();
    let source = form();
    let request = McpElicitationPromptRequest::new_human_feature(
        owner(),
        Arc::from("server"),
        crate::McpFeatureAction::ResourceRead,
        source.request().clone(),
    )
    .unwrap();
    let mut pending = bridge.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let wire = project_prompt(&view(&mut inbox), None).unwrap();
    assert_eq!(wire.params["sessionId"], "actual-session");
    assert!(wire.params.get("toolCallId").is_none());
}

#[test]
fn urls_require_host_registration_and_never_copy_remote_scope_or_id() {
    let (bridge, mut inbox) = bridge();
    let presenter = NativeAcpElicitationPresenter::new(bridge.clone());
    presenter.activate(owner());
    let request = request(
        r#"{"mode":"url","message":"Private URL","url":"https://example.test/authorize","sessionId":"foreign","toolCallId":"foreign"}"#,
    );
    let completion = presenter.register(&request).unwrap();
    let id = presenter.id_for(&request).unwrap();
    let mut pending = bridge.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    assert!(project_prompt(&prompt, None).is_err());
    let wire = project_prompt(&prompt, Some(id)).unwrap();
    assert_eq!(wire.reply_kind, NativeAcpReplyKind::Url);
    assert_eq!(wire.params["elicitationId"], id.to_string());
    assert_eq!(wire.params["sessionId"], "actual-session");
    assert_eq!(wire.params["toolCallId"], "actual-call");
    assert!(decode_reply(&prompt, &json!({"action":"accept","content":{}})).is_err());
    inbox
        .reply(
            prompt.token(),
            decode_reply(&prompt, &json!({"action":"accept"})).unwrap(),
        )
        .unwrap();
    assert!(futures_executor::block_on(pending).is_ok());
    drop(completion);
    let raw=RawValue::from_string(r#"{"mode":"url","message":"Private","url":"https://example.test/authorize","elicitationId":"peer-supplied"}"#.into()).unwrap();
    assert!(
        McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
            .is_err()
    );
}

#[test]
fn ordinary_questions_use_native_provenance_and_accept_free_text() {
    let (bridge, mut inbox) = bridge();
    let tool = AskUserQuestionTool::shared_prompter(bridge);
    let arguments = json!({"questions":[{"question":"Which path?","options":[{"label":"One"},{"label":"Two"}]}]});
    let arguments = tool
        .prepare(call("ask_user_question", arguments))
        .unwrap()
        .arguments()
        .clone();
    let mut pending = tool.execute(context(), arguments, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let prompt = view(&mut inbox);
    let wire = project_prompt(&prompt, None).unwrap();
    assert_eq!(wire.reply_kind, NativeAcpReplyKind::Question);
    assert_eq!(wire.params["toolCallId"], "actual-call");
    assert_eq!(
        wire.params["requestedSchema"]["properties"]["question_1"]["title"],
        "Which path?"
    );
    for invalid in [
        json!({"action":"accept","content":{}}),
        json!({"action":"accept","content":{"question_1":" "}}),
        json!({"action":"accept","content":{"question_1":"x","other":"y"}}),
        json!({"action":"accept","content":{"question_1":"x".repeat(4097)}}),
    ] {
        assert!(decode_reply(&prompt, &invalid).is_err());
    }
    let result = json!({"action":"accept","content":{"question_1":"A different route"}});
    inbox
        .reply(prompt.token(), decode_reply(&prompt, &result).unwrap())
        .unwrap();
    assert!(futures_executor::block_on(pending).is_ok());
}
