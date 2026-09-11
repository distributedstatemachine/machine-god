//! User text arrays preserve provider-only advisory context and existing bounds.

use super::{ScriptedTransport, bytes, expect_start_error, finish, provider, request, start};
use futures_util::StreamExt;
use machine_god_core::{
    CancellationToken, ContentBlock, Engine, Message, Role, SessionId, SessionIncarnationId,
    SessionTurnPreparation, SessionUserContext, ToolCall, ToolCallId, ToolName, ToolOutput,
};
use machine_god_native::{AiGatewayLimits, AiGatewayProvider};
use serde_json::{Value, json};
use std::sync::Arc;

fn user(texts: &[&str]) -> Message {
    Message {
        role: Role::User,
        content: texts
            .iter()
            .map(|text| ContentBlock::Text {
                text: (*text).into(),
            })
            .collect(),
    }
}

#[test]
fn separate_user_text_parts_preserve_exact_order_and_wire_representation() {
    let transport = ScriptedTransport::new([bytes(finish("stop"))]);
    let parts = [
        " canonical prompt\n",
        "---\nname: 'réview'\n---\nFull \"external\" content\n",
    ];
    drop(
        start(
            &provider(&transport),
            request(vec![user(&parts)]),
            CancellationToken::new(),
        )
        .unwrap(),
    );
    let body: Value = serde_json::from_slice(&transport.requests()[0].body).unwrap();
    assert_eq!(
        body["prompt"],
        json!([{"role":"user","content":[
            {"type":"text","text":parts[0]}, {"type":"text","text":parts[1]}
        ]}])
    );
}

#[test]
fn user_part_count_is_bounded_independently_of_tool_call_limits() {
    for count in [0, 1, 2, 64, 65] {
        let transport = ScriptedTransport::new([bytes(finish("stop"))]);
        let provider = AiGatewayProvider::with_limits(
            "provider/default",
            Arc::new(transport.clone()),
            AiGatewayLimits {
                max_tool_calls: 1,
                ..AiGatewayLimits::default()
            },
        )
        .unwrap();
        let result = start(
            &provider,
            request(vec![user(&vec![""; count])]),
            CancellationToken::new(),
        );
        let accepted = (1..=64).contains(&count);
        if accepted {
            drop(result.unwrap());
            let body: Value = serde_json::from_slice(&transport.requests()[0].body).unwrap();
            assert_eq!(
                body["prompt"][0]["content"].as_array().unwrap().len(),
                count
            );
        } else {
            assert_eq!(
                expect_start_error(result, "user part count").code,
                "gateway_invalid_history"
            );
        }
        assert_eq!(transport.requests().len(), usize::from(accepted));
    }
}

#[test]
fn all_user_parts_share_exact_encoded_body_limit_including_escaping_and_envelopes() {
    let texts = ["\"\n\\界", "\t\r\u{0}\"", ""];
    let transport = ScriptedTransport::new([bytes(finish("stop"))]);
    drop(
        start(
            &provider(&transport),
            request(vec![user(&texts), user(&texts)]),
            CancellationToken::new(),
        )
        .unwrap(),
    );
    let exact = transport.requests()[0].body.len();
    for max_request_bytes in [exact, exact - 1, 1] {
        let transport = ScriptedTransport::new([bytes(finish("stop"))]);
        let provider = AiGatewayProvider::with_limits(
            "provider/default",
            Arc::new(transport.clone()),
            AiGatewayLimits {
                max_request_bytes,
                ..AiGatewayLimits::default()
            },
        )
        .unwrap();
        let result = start(
            &provider,
            request(vec![user(&texts), user(&texts)]),
            CancellationToken::new(),
        );
        if max_request_bytes == exact {
            drop(result.unwrap());
            assert_eq!(transport.requests()[0].body.len(), exact);
        } else {
            assert_eq!(
                expect_start_error(result, "user text body bytes").code,
                "gateway_request_byte_limit"
            );
            assert!(transport.requests().is_empty());
        }
    }
}

#[test]
fn non_text_user_parts_and_multiple_system_or_tool_blocks_remain_invalid() {
    let call_id = ToolCallId::new("not-user-evidence").unwrap();
    let invalid = [
        ContentBlock::Json {
            value: json!({"untrusted":true}),
        },
        ContentBlock::ToolCall {
            call: ToolCall {
                id: call_id.clone(),
                name: ToolName::new("echo").unwrap(),
                arguments: json!({}),
            },
        },
        ContentBlock::ToolResult {
            call_id,
            output: ToolOutput::success("not permission"),
        },
    ];
    let mut cases: Vec<_> = invalid
        .into_iter()
        .flat_map(|block| {
            [
                Message {
                    role: Role::User,
                    content: vec![
                        ContentBlock::Text {
                            text: "first".into(),
                        },
                        block.clone(),
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![
                        block,
                        ContentBlock::Text {
                            text: "last".into(),
                        },
                    ],
                },
            ]
        })
        .collect();
    for role in [Role::System, Role::Tool] {
        let mut message = user(&["one", "two"]);
        message.role = role;
        cases.push(message);
    }
    for message in cases {
        let transport = ScriptedTransport::new([]);
        let error = expect_start_error(
            start(
                &provider(&transport),
                request(vec![message]),
                CancellationToken::new(),
            ),
            "invalid role or user part",
        );
        assert_eq!(error.code, "gateway_invalid_history");
        assert!(transport.requests().is_empty());
    }
}

#[test]
fn prepared_advisory_context_reaches_gateway_without_persisting_or_leaking_to_next_prompt() {
    let transport = ScriptedTransport::new([bytes(finish("stop")), bytes(finish("stop"))]);
    let engine = Engine::builder()
        .provider(provider(&transport))
        .session_store(machine_god_testkit::InMemorySessionStore::new())
        .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("user-parts").unwrap(),
            SessionIncarnationId::new("user-parts-life").unwrap(),
        )
        .unwrap();
    let advisory = "FULL EXTERNAL SKILL TEXT\nPreserve every byte.";
    let events = futures_executor::block_on(async {
        session
            .prompt_prepared(
                "canonical prompt",
                SessionTurnPreparation {
                    expected_revision: session.record().revision,
                    metadata: None,
                    context: None,
                    user_context: Some(SessionUserContext {
                        user_message_index: 0,
                        text: advisory.into(),
                    }),
                },
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(
        matches!(
            &events.last().unwrap().as_ref().unwrap().payload,
            machine_god_core::TurnEvent::Completed { .. }
        ),
        "{events:?}"
    );
    assert_eq!(
        session.record().messages[0],
        Message::text(Role::User, "canonical prompt")
    );
    let first: Value = serde_json::from_slice(&transport.requests()[0].body).unwrap();
    let parts = first["prompt"][0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], json!({"type":"text","text":"canonical prompt"}));
    assert!(parts[1]["text"].as_str().unwrap().contains(advisory));
    let events = futures_executor::block_on(async {
        session
            .prompt("next prompt")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(
        matches!(
            &events.last().unwrap().as_ref().unwrap().payload,
            machine_god_core::TurnEvent::Completed { .. }
        ),
        "{events:?}"
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(!String::from_utf8_lossy(&requests[1].body).contains(advisory.lines().next().unwrap()));
    assert!(session.record().metadata.is_empty());
}
